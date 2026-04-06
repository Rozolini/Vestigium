pub mod loader; // PE file parsing and memory mapping.
pub mod record; // Deterministic execution recording and state snapshots.
pub mod replay; // Trace replay and state injection.
pub mod runner; // VM execution loop and exit handling.

#[cfg(not(miri))]
#[cfg(test)]
mod tests {
    use crate::record::{Snapshot, TraceLogger};
    use crate::replay::ReplayEngine;
    use trace::format::{EventType, TraceEvent};
    use vmm::partition::VmPartition;
    use vmm::vcpu::Vcpu;
    use windows::Win32::System::Hypervisor::{WHV_REGISTER_VALUE, WHvX64RegisterRax};

    #[test]
    fn test_phase2_record_engine() {
        // Validate basic memory snapshot capture and event logging.
        let mut snapshot = Snapshot::new();
        let dummy_memory = [0x90, 0x90, 0xCC];

        unsafe {
            snapshot.capture_region(0x1000, dummy_memory.as_ptr(), dummy_memory.len());
        }
        assert_eq!(snapshot.memory_regions.get(&0x1000).unwrap().len(), 3);

        let mut logger = TraceLogger::new();

        logger.log_event(TraceEvent {
            instruction_count: 150,
            event: EventType::Syscall {
                rax: 0,
                memory_writes: vec![(0x2000, vec![0xAA, 0xBB])],
            },
        });

        let events = logger.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].instruction_count, 150);
    }

    #[test]
    fn test_phase3_replay_engine_injection() {
        // Verify register state injection during replay.
        let mut partition = VmPartition::new().expect("Failed to create partition");
        partition.configure_interceptions().unwrap();
        let vcpu = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU");

        let mut replay = ReplayEngine::new(&partition, 0);
        replay.register_vcpu(0, &vcpu);

        let event = TraceEvent {
            instruction_count: 0,
            event: EventType::Syscall {
                rax: 0xDEADBEEF,
                memory_writes: vec![],
            },
        };

        replay.inject_event(&event).expect("Failed to inject event");

        let names = [WHvX64RegisterRax];
        let mut values = [WHV_REGISTER_VALUE::default()];
        vcpu.get_registers(&names, &mut values)
            .expect("Failed to read registers");

        unsafe {
            assert_eq!(values[0].Reg64, 0xDEADBEEF);
        }
    }

    #[test]
    fn test_phase3_replay_engine_interleaving() {
        // Ensure context switches do not alter the underlying hardware vCPU in cooperative mode.
        let mut partition = VmPartition::new().expect("Failed to create partition");
        partition.configure_interceptions().unwrap();

        let vcpu0 = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU 0");

        let mut replay = ReplayEngine::new(&partition, 0);
        replay.register_vcpu(0, &vcpu0);

        assert_eq!(replay.active_vcpu_id(), 0);

        let event = TraceEvent {
            instruction_count: 500,
            event: EventType::ContextSwitch { next_thread_id: 1 },
        };

        replay
            .inject_event(&event)
            .expect("Failed to inject context switch");

        // Hardware vCPU ID remains 0 in cooperative scheduling.
        assert_eq!(replay.active_vcpu_id(), 0);
    }

    #[test]
    fn test_phase4_e2e_record_replay_pipeline() {
        // End-to-end validation of the record and replay pipeline.
        use std::alloc::{Layout, alloc_zeroed};
        use vmm::paging::PageTable;
        use windows::Win32::System::Hypervisor::WHvX64RegisterRip;

        // RECORD PHASE
        let mut snapshot = Snapshot::new();

        let layout = Layout::from_size_align(4096, 4096).unwrap();
        let host_memory = unsafe { alloc_zeroed(layout) };

        unsafe {
            // Write basic x64 instructions: RDTSC, HLT
            *host_memory.add(0) = 0x0F;
            *host_memory.add(1) = 0x31;
            *host_memory.add(2) = 0xF4;
            snapshot.capture_region(0x1000, host_memory, 4096);
        }

        // Map page tables for 64-bit long mode execution.
        let pt_gpa = 0x2000;
        let page_table = PageTable::create_identity_mapping(pt_gpa);
        unsafe {
            snapshot.capture_region(pt_gpa, page_table.host_ptr, page_table.size);
        }

        let mut logger = TraceLogger::new();

        logger.log_event(TraceEvent {
            instruction_count: 0,
            event: EventType::Rdtsc {
                rax: 0x1111,
                rdx: 0x0,
            },
        });

        logger.log_event(TraceEvent {
            instruction_count: 0,
            event: EventType::ContextSwitch { next_thread_id: 1 },
        });

        let recorded_events = logger.events().to_vec();

        // REPLAY PHASE
        let mut partition = VmPartition::new().expect("Failed to create partition");
        partition.configure_interceptions().unwrap();

        let vcpu0 = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU 0");

        // Initialize 64-bit environment and set instruction pointer.
        vcpu0.setup_long_mode(pt_gpa).unwrap();
        let names = [WHvX64RegisterRip];
        let values = [WHV_REGISTER_VALUE { Reg64: 0x1000 }];
        vcpu0.set_registers(&names, &values).unwrap();

        let mut replay = ReplayEngine::new(&partition, 0);
        replay.register_vcpu(0, &vcpu0);

        replay
            .restore_snapshot(&snapshot, &[])
            .expect("Failed to restore snapshot");

        for event in &recorded_events {
            replay.replay_event(event).expect("Failed to replay event");
        }

        // VERIFICATION
        let names = [WHvX64RegisterRax];
        let mut values = [WHV_REGISTER_VALUE::default()];
        vcpu0.get_registers(&names, &mut values).unwrap();

        unsafe {
            assert_eq!(values[0].Reg64, 0x1111);
        }

        // Verify that hardware vCPU remains 0.
        assert_eq!(replay.active_vcpu_id(), 0);
    }
}
