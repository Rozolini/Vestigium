use clap::{Parser, Subcommand};
use engine::loader::PeLoader;
use engine::record::{AlignedMemory, RecordEngine, Snapshot};
use engine::replay::ReplayEngine;
use engine::runner::VmRunner;
use std::fs;
use std::path::PathBuf;
use trace::format::TraceEvent;
use vmm::ept::EptManager;
use vmm::paging::PageTable;
use vmm::partition::VmPartition;
use vmm::vcpu::Vcpu;
use windows::Win32::System::Hypervisor::{
    WHV_MAP_GPA_RANGE_FLAGS, WHV_REGISTER_VALUE, WHV_RUN_VP_EXIT_CONTEXT, WHvX64RegisterCr0,
    WHvX64RegisterCr3, WHvX64RegisterCr4, WHvX64RegisterRax, WHvX64RegisterRflags,
    WHvX64RegisterRip, WHvX64RegisterRsp,
};

/// CLI interface for the Vestigium hypervisor.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Supported execution modes.
#[derive(Subcommand)]
enum Commands {
    /// Executes the target binary and records deterministic events.
    Record {
        #[arg(short, long)]
        target: PathBuf,

        #[arg(short, long, default_value = "trace.bin")]
        output: PathBuf,
    },
    /// Replays a previously recorded execution trace.
    Replay {
        #[arg(short, long)]
        trace: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Record { target, output } => {
            println!("Recording target: {:?}", target);

            // Parse and map the target executable.
            let binary_data = fs::read(target).expect("Failed to read target binary");
            let loader = PeLoader::parse(&binary_data).expect("Failed to parse PE file");
            let (host_ptr, size) = loader.map_into_memory();

            // Initialize VM partition and memory manager.
            let mut partition = VmPartition::new().expect("Failed to create VM partition");
            partition
                .configure_interceptions()
                .expect("Failed to configure interceptions");

            let ept = EptManager::new(partition.as_raw());
            let base_gpa: u64 = 0x100000;

            // Map target binary into guest physical memory.
            unsafe {
                ept.map_gpa_range(
                    base_gpa,
                    host_ptr as *mut _,
                    size as u64,
                    WHV_MAP_GPA_RANGE_FLAGS(0x7),
                )
                .expect("Failed to map target memory");
            }

            // Allocate and map the guest stack.
            let stack_size = 0x10000;
            let mut stack_memory = AlignedMemory::new(stack_size);
            let stack_base_gpa = base_gpa + ((size as u64 + 0xFFF) & !0xFFF);

            unsafe {
                ept.map_gpa_range(
                    stack_base_gpa,
                    stack_memory.as_mut_ptr() as *mut _,
                    stack_size as u64,
                    WHV_MAP_GPA_RANGE_FLAGS(0x7),
                )
                .expect("Failed to map stack memory");
            }

            // Map a null page to trap null pointer dereferences.
            let mut null_page = AlignedMemory::new(0x1000);
            unsafe {
                ept.map_gpa_range(
                    0x0,
                    null_page.as_mut_ptr() as *mut _,
                    0x1000,
                    WHV_MAP_GPA_RANGE_FLAGS(0x7),
                )
                .expect("Failed to map null page");
            }

            // Set up identity mapping for 64-bit long mode.
            let pt_gpa: u64 = 0x200000;
            let page_table = PageTable::create_identity_mapping(pt_gpa);

            unsafe {
                ept.map_gpa_range(
                    pt_gpa,
                    page_table.host_ptr as *mut _,
                    page_table.size as u64,
                    WHV_MAP_GPA_RANGE_FLAGS(0x7),
                )
                .expect("Failed to map page tables");
            }

            // Captures the current memory layout for the snapshot.
            let capture_memory_layout = || -> Snapshot {
                let mut snap = Snapshot::new();
                unsafe {
                    snap.capture_region(0x0, null_page.as_ptr(), 0x1000);
                    snap.capture_region(base_gpa, host_ptr, size);
                    snap.capture_region(stack_base_gpa, stack_memory.as_ptr(), stack_size);
                    snap.capture_region(pt_gpa, page_table.host_ptr, page_table.size);
                }
                snap
            };

            let mut snapshot = capture_memory_layout();

            println!(
                "Target mapped at GPA {:#x}. Entry RVA: {:#x}",
                base_gpa,
                loader.entry_point()
            );

            // Initialize vCPU and configure initial register state.
            let vcpu = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU");
            vcpu.setup_long_mode(pt_gpa)
                .expect("Failed to setup long mode");

            let entry_gpa = base_gpa + loader.entry_point() as u64;
            let stack_gpa = stack_base_gpa + stack_size as u64;

            let names = [WHvX64RegisterRip, WHvX64RegisterRsp];
            let values = [
                WHV_REGISTER_VALUE { Reg64: entry_gpa },
                WHV_REGISTER_VALUE { Reg64: stack_gpa },
            ];
            vcpu.set_registers(&names, &values)
                .expect("Failed to set RIP/RSP registers");

            let capture_names = [
                WHvX64RegisterRip,
                WHvX64RegisterRsp,
                WHvX64RegisterRflags,
                WHvX64RegisterCr0,
                WHvX64RegisterCr3,
                WHvX64RegisterCr4,
            ];
            snapshot
                .capture_registers(&vcpu, &capture_names)
                .expect("Failed to capture initial registers");

            // Execute the binary and record deterministic events.
            let mut record_engine = RecordEngine::new(&partition, &vcpu);
            println!("Starting VM execution (Record Mode)...");
            record_engine.run().expect("Record engine execution failed");

            // Capture final state hash for E2E verification.
            let mut final_snapshot = capture_memory_layout();
            final_snapshot
                .capture_registers(&vcpu, &capture_names)
                .expect("Failed to capture final registers");

            let final_hash = final_snapshot.hash_state();
            println!("Record final state hash: 0x{:016X}", final_hash);

            let recorded_events = record_engine.finalize();
            println!(
                "Execution completed. Captured {} events.",
                recorded_events.len()
            );
            println!("Trace destination: {:?}", output);

            // Serialize and save the trace archive.
            let archive = (&snapshot, &recorded_events, final_hash);
            let encoded = bincode::serialize(&archive).expect("Failed to serialize trace archive");
            fs::write(output, encoded).expect("Failed to write trace to disk");
            println!("Trace successfully saved.");
        }
        Commands::Replay { trace } => {
            println!("Replaying trace: {:?}", trace);

            // Load and deserialize the trace archive.
            let archive_data = fs::read(trace).expect("Failed to read trace file");
            let (mut snapshot, events, expected_hash): (Snapshot, Vec<TraceEvent>, u64) =
                bincode::deserialize(&archive_data).expect("Failed to deserialize trace archive");

            println!(
                "Loaded snapshot with {} memory regions and {} events.",
                snapshot.memory_regions.len(),
                events.len()
            );

            // Initialize the replay VM partition.
            let mut partition = VmPartition::new().expect("Failed to create VM partition");
            partition
                .configure_interceptions()
                .expect("Failed to configure interceptions");

            let vcpu = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU");

            // Initialize 64-bit environment before restoring state.
            let pt_gpa: u64 = 0x200000;
            vcpu.setup_long_mode(pt_gpa)
                .expect("Failed to setup long mode");

            let mut replay_engine = ReplayEngine::new(&partition, 0);
            replay_engine.register_vcpu(0, &vcpu);

            let restore_names = [
                WHvX64RegisterRip,
                WHvX64RegisterRsp,
                WHvX64RegisterRflags,
                WHvX64RegisterCr0,
                WHvX64RegisterCr3,
                WHvX64RegisterCr4,
            ];

            // Restore initial memory and register state.
            replay_engine
                .restore_snapshot(&snapshot, &restore_names)
                .expect("Failed to restore snapshot");
            println!("Snapshot successfully restored. Ready for execution.");

            // Sequentially inject recorded events.
            println!("Replaying {} events...", events.len());
            for event in &events {
                replay_engine
                    .replay_event(event)
                    .expect("Failed to replay event");
            }

            println!("Trace replay finished. Executing remaining instructions...");

            let runner = VmRunner::new(&partition, &vcpu);
            handle_vm_exit(&vcpu, runner.run());

            // Capture final state and verify determinism.
            let capture_names = [
                WHvX64RegisterRip,
                WHvX64RegisterRsp,
                WHvX64RegisterRflags,
                WHvX64RegisterCr0,
                WHvX64RegisterCr3,
                WHvX64RegisterCr4,
            ];
            snapshot
                .capture_registers(&vcpu, &capture_names)
                .expect("Failed to capture final registers");

            let replay_hash = snapshot.hash_state();
            println!("Expected hash: 0x{:016X}", expected_hash);
            println!("Replay hash:   0x{:016X}", replay_hash);

            assert_eq!(
                replay_hash, expected_hash,
                "E2E VERIFICATION FAILED: State mismatch!"
            );
            println!("E2E VERIFICATION SUCCESS: 100% deterministic execution confirmed.");
        }
    }
}

/// Evaluates the final VM exit status.
fn handle_vm_exit(vcpu: &Vcpu, result: windows::core::Result<WHV_RUN_VP_EXIT_CONTEXT>) {
    match result {
        Ok(exit_ctx) => {
            let reason_code = exit_ctx.ExitReason.0;
            if reason_code == 1 {
                let mem = unsafe { exit_ctx.Anonymous.MemoryAccess };
                println!("FATAL: EPT Memory Access Violation!");
                println!("Target GPA: 0x{:X}", mem.Gpa);
                println!("Target GVA: 0x{:X}", mem.Gva);
            } else if reason_code == 8 {
                let names = [WHvX64RegisterRax];
                let mut values = [WHV_REGISTER_VALUE::default()];
                if vcpu.get_registers(&names, &mut values).is_ok() {
                    println!("Guest halted (Replay). Verification RAX: 0x{:X}", unsafe {
                        values[0].Reg64
                    });
                } else {
                    println!("Guest halted execution (HLT), but failed to read RAX.");
                }
            } else {
                println!("Execution stopped. Reason code: {}", reason_code);
            }
        }
        Err(e) => println!("Execution failed: {:?}", e),
    }
}
