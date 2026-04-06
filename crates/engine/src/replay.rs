use crate::record::Snapshot;
use crate::runner::VmRunner;
use std::collections::HashMap;
use trace::format::{EventType, TraceEvent};
use vmm::ept::EptManager;
use vmm::partition::VmPartition;
use vmm::vcpu::Vcpu;
use windows::Win32::System::Hypervisor::{
    WHV_MAP_GPA_RANGE_FLAGS, WHV_REGISTER_NAME, WHV_REGISTER_VALUE, WHvX64RegisterRax,
    WHvX64RegisterRbx, WHvX64RegisterRcx, WHvX64RegisterRdx, WHvX64RegisterRip,
};
use windows::core::Result;

/// Manages the deterministic replay of a recorded execution trace.
pub struct ReplayEngine<'a> {
    partition: &'a VmPartition,
    vcpus: HashMap<u32, &'a Vcpu>,
    active_vcpu_id: u32,
}

impl<'a> ReplayEngine<'a> {
    /// Initializes the replay engine with the target partition and initial vCPU ID.
    pub fn new(partition: &'a VmPartition, initial_vcpu_id: u32) -> Self {
        Self {
            partition,
            vcpus: HashMap::new(),
            active_vcpu_id: initial_vcpu_id,
        }
    }

    /// Registers a vCPU instance for replay execution.
    pub fn register_vcpu(&mut self, id: u32, vcpu: &'a Vcpu) {
        self.vcpus.insert(id, vcpu);
    }

    /// Returns a reference to the currently active vCPU.
    pub fn active_vcpu(&self) -> &'a Vcpu {
        self.vcpus
            .get(&self.active_vcpu_id)
            .expect("Active vCPU not found")
    }

    /// Returns the ID of the currently active vCPU.
    pub fn active_vcpu_id(&self) -> u32 {
        self.active_vcpu_id
    }

    /// Restores the initial guest memory and register state from a snapshot.
    pub fn restore_snapshot(
        &self,
        snapshot: &Snapshot,
        register_names: &[WHV_REGISTER_NAME],
    ) -> Result<()> {
        let ept = EptManager::new(self.partition.as_raw());
        let flags = WHV_MAP_GPA_RANGE_FLAGS(0x7);

        for (&gpa, data) in &snapshot.memory_regions {
            unsafe {
                ept.map_gpa_range(gpa, data.as_ptr() as *mut _, data.len() as u64, flags)?;
            }
        }

        if !snapshot.registers.is_empty() {
            self.active_vcpu()
                .set_registers(register_names, &snapshot.registers)?;
        }

        Ok(())
    }

    /// Executes the VM until the next intercepted event, verifying and injecting recorded state.
    pub fn replay_event(&mut self, event: &TraceEvent) -> Result<()> {
        loop {
            let runner = VmRunner::new(self.partition, self.active_vcpu());
            let exit_ctx = runner.run()?;
            let reason = exit_ctx.ExitReason.0;

            match reason {
                1 | 8 => {
                    // WHvRunVpExitReasonX64Halt
                    let names = [WHvX64RegisterRax];
                    let mut values = [WHV_REGISTER_VALUE::default()];
                    self.active_vcpu().get_registers(&names, &mut values)?;
                    println!("Guest halted (Replay). Verification RAX: 0x{:X}", unsafe {
                        values[0].Reg64
                    });
                    break;
                }
                4097 => {
                    // WHvRunVpExitReasonX64Cpuid
                    if matches!(event.event, EventType::Cpuid { .. }) {
                        self.inject_event(event)?;

                        let instruction_length = exit_ctx.VpContext._bitfield & 0x0F;
                        self.advance_rip(instruction_length)?;
                        break;
                    }
                }
                4099 => {
                    // WHvRunVpExitReasonX64Rdtsc
                    if matches!(event.event, EventType::Rdtsc { .. }) {
                        self.inject_event(event)?;

                        let instruction_length = exit_ctx.VpContext._bitfield & 0x0F;
                        self.advance_rip(instruction_length)?;
                        break;
                    }
                }
                4 => {
                    // WHvRunVpExitReasonX64Callout
                    // Intercept VMCALL used for context switching.
                    if let EventType::ContextSwitch { next_thread_id } = &event.event {
                        let names = [WHvX64RegisterRcx];
                        let mut values = [WHV_REGISTER_VALUE::default()];
                        self.active_vcpu().get_registers(&names, &mut values)?;

                        let current_thread_id = unsafe { values[0].Reg64 } as u32;
                        if current_thread_id == *next_thread_id {
                            self.inject_event(event)?;
                            // VMCALL instruction is exactly 3 bytes long.
                            self.advance_rip(3)?;
                            break;
                        } else {
                            println!(
                                "Replay mismatch: Expected thread {}, got {}",
                                next_thread_id, current_thread_id
                            );
                            break;
                        }
                    } else {
                        println!("Replay mismatch: Unexpected VMCALL execution.");
                        break;
                    }
                }
                _ => {
                    println!(
                        "Execution stopped. Unhandled VM exit reason during replay: {}",
                        reason
                    );
                    break;
                }
            }
        }

        Ok(())
    }

    /// Injects recorded state (registers/memory) into the active vCPU to ensure determinism.
    pub fn inject_event(&mut self, event: &TraceEvent) -> Result<()> {
        let vcpu = self.active_vcpu();

        match &event.event {
            EventType::Syscall { rax, memory_writes } => {
                let names = [WHvX64RegisterRax];
                let values = [WHV_REGISTER_VALUE { Reg64: *rax }];
                vcpu.set_registers(&names, &values)?;

                for (gpa, data) in memory_writes {
                    self.write_guest_memory(*gpa, data)?;
                }
            }
            EventType::Rdtsc { rax, rdx } => {
                let names = [WHvX64RegisterRax, WHvX64RegisterRdx];
                let values = [
                    WHV_REGISTER_VALUE { Reg64: *rax },
                    WHV_REGISTER_VALUE { Reg64: *rdx },
                ];
                vcpu.set_registers(&names, &values)?;
            }
            EventType::Cpuid { eax, ebx, ecx, edx } => {
                let names = [
                    WHvX64RegisterRax,
                    WHvX64RegisterRbx,
                    WHvX64RegisterRcx,
                    WHvX64RegisterRdx,
                ];
                let values = [
                    WHV_REGISTER_VALUE { Reg64: *eax as u64 },
                    WHV_REGISTER_VALUE { Reg64: *ebx as u64 },
                    WHV_REGISTER_VALUE { Reg64: *ecx as u64 },
                    WHV_REGISTER_VALUE { Reg64: *edx as u64 },
                ];
                vcpu.set_registers(&names, &values)?;
            }
            EventType::ContextSwitch { next_thread_id: _ } => {
                // Cooperative scheduler operates on a single vCPU (ID 0).
                // No vCPU switch is required here.
            }
        }

        Ok(())
    }

    /// Writes data to guest physical memory.
    fn write_guest_memory(&self, _gpa: u64, _data: &[u8]) -> Result<()> {
        // TODO: Implement guest memory writing via EPT
        Ok(())
    }

    /// Advances the guest instruction pointer (RIP) past the intercepted instruction.
    fn advance_rip(&self, instruction_length: u8) -> Result<()> {
        let names = [WHvX64RegisterRip];
        let mut values = [WHV_REGISTER_VALUE::default()];
        self.active_vcpu().get_registers(&names, &mut values)?;

        unsafe {
            values[0].Reg64 += instruction_length as u64;
        }

        self.active_vcpu().set_registers(&names, &values)?;
        Ok(())
    }
}
