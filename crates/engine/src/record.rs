use crate::runner::VmRunner;
use serde::{Deserialize, Serialize};
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use trace::format::{EventType, TraceEvent};
use vmm::vcpu::Vcpu;
use windows::Win32::System::Hypervisor::{
    WHV_REGISTER_NAME, WHV_REGISTER_VALUE, WHvX64RegisterRax, WHvX64RegisterRbx, WHvX64RegisterRcx,
    WHvX64RegisterRdx, WHvX64RegisterRip,
};
use windows::core::Result;

/// Manages 4KB page-aligned memory buffers required by the hypervisor.
pub struct AlignedMemory {
    ptr: *mut u8,
    size: usize,
    layout: Layout,
}

impl AlignedMemory {
    /// Allocates zero-initialized, page-aligned memory.
    pub fn new(size: usize) -> Self {
        let layout = Layout::from_size_align(size, 4096).expect("Invalid alignment layout");
        let ptr = unsafe { alloc_zeroed(layout) };
        assert!(!ptr.is_null(), "Memory allocation failed");
        Self { ptr, size, layout }
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.ptr
    }

    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    pub fn len(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.size == 0
    }
}

/// Ensures proper deallocation of the aligned memory buffer.
impl Drop for AlignedMemory {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr, self.layout);
        }
    }
}

// Custom serialization logic for raw memory buffers.
impl Serialize for AlignedMemory {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let slice = unsafe { std::slice::from_raw_parts(self.ptr, self.size) };
        serializer.serialize_bytes(slice)
    }
}

impl<'de> Deserialize<'de> for AlignedMemory {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        let mut mem = AlignedMemory::new(bytes.len());
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mem.as_mut_ptr(), bytes.len());
        }
        Ok(mem)
    }
}

/// Handles serialization of WHV_REGISTER_VALUE C-unions.
mod register_serde {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(
        regs: &[WHV_REGISTER_VALUE],
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let byte_regs: Vec<[u8; 16]> = regs
            .iter()
            .map(|r| unsafe { std::mem::transmute_copy::<WHV_REGISTER_VALUE, [u8; 16]>(r) })
            .collect();
        byte_regs.serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Vec<WHV_REGISTER_VALUE>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let byte_regs: Vec<[u8; 16]> = Vec::deserialize(deserializer)?;
        Ok(byte_regs
            .into_iter()
            .map(|b| unsafe { std::mem::transmute_copy::<[u8; 16], WHV_REGISTER_VALUE>(&b) })
            .collect())
    }
}

/// Represents a point-in-time snapshot of the guest VM state.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub memory_regions: HashMap<u64, AlignedMemory>,
    #[serde(with = "register_serde")]
    pub registers: Vec<WHV_REGISTER_VALUE>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self::new()
    }
}

impl Snapshot {
    /// Initializes an empty snapshot.
    pub fn new() -> Self {
        Self {
            memory_regions: HashMap::new(),
            registers: Vec::new(),
        }
    }

    /// Captures a specific guest physical memory region into the snapshot.
    ///
    /// # Safety
    /// `host_ptr` must be a valid pointer to an initialized memory region of at least `size` bytes.
    pub unsafe fn capture_region(&mut self, gpa: u64, host_ptr: *const u8, size: usize) {
        let mut buffer = AlignedMemory::new(size);
        unsafe {
            std::ptr::copy_nonoverlapping(host_ptr, buffer.as_mut_ptr(), size);
        }
        self.memory_regions.insert(gpa, buffer);
    }

    /// Captures the specified vCPU register states.
    pub fn capture_registers(&mut self, vcpu: &Vcpu, names: &[WHV_REGISTER_NAME]) -> Result<()> {
        let mut values = vec![WHV_REGISTER_VALUE::default(); names.len()];
        vcpu.get_registers(names, &mut values)?;
        self.registers = values;
        Ok(())
    }

    /// Computes a deterministic hash of the captured VM state.
    /// Used for end-to-end determinism verification.
    pub fn hash_state(&self) -> u64 {
        let mut hasher = DefaultHasher::new();

        // Hash registers deterministically.
        for reg in &self.registers {
            unsafe { reg.Reg64 }.hash(&mut hasher);
        }

        // Hash memory regions in a stable order (sorted by GPA).
        let mut gpas: Vec<&u64> = self.memory_regions.keys().collect();
        gpas.sort_unstable();

        for gpa in gpas {
            gpa.hash(&mut hasher);
            let mem = self.memory_regions.get(gpa).unwrap();
            let slice = unsafe { std::slice::from_raw_parts(mem.as_ptr(), mem.len()) };
            slice.hash(&mut hasher);
        }

        hasher.finish()
    }
}

/// Sequentially records VM events for deterministic replay.
#[derive(Serialize, Deserialize)]
pub struct TraceLogger {
    events: Vec<TraceEvent>,
}

impl Default for TraceLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceLogger {
    /// Initializes an empty event trace logger.
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Appends a new trace event to the log.
    pub fn log_event(&mut self, event: TraceEvent) {
        self.events.push(event);
    }

    /// Returns a slice of all recorded events.
    pub fn events(&self) -> &[TraceEvent] {
        &self.events
    }
}

/// Orchestrates VM execution and state recording.
pub struct RecordEngine<'a> {
    vcpu: &'a Vcpu,
    logger: TraceLogger,
    runner: VmRunner<'a>,
}

impl<'a> RecordEngine<'a> {
    /// Initializes the recording engine with the target partition and vCPU.
    pub fn new(partition: &'a vmm::partition::VmPartition, vcpu: &'a Vcpu) -> Self {
        Self {
            vcpu,
            logger: TraceLogger::new(),
            runner: VmRunner::new(partition, vcpu),
        }
    }

    /// Runs the VM execution loop, intercepting and recording non-deterministic events.
    pub fn run(&mut self) -> Result<()> {
        let mut instruction_count = 0;

        loop {
            let exit_ctx = self.runner.run()?;
            let reason = exit_ctx.ExitReason.0;

            match reason {
                1 => {
                    // WHvRunVpExitReasonMemoryAccess
                    // Fatal EPT violation.
                    let mem = unsafe { exit_ctx.Anonymous.MemoryAccess };
                    println!("FATAL: EPT Memory Access Violation at GPA: 0x{:X}", mem.Gpa);
                    break;
                }
                8 => {
                    // WHvRunVpExitReasonX64Halt
                    // Guest executed HLT. Execution finished.
                    let names = [WHvX64RegisterRax];
                    let mut values = [WHV_REGISTER_VALUE::default()];
                    self.vcpu.get_registers(&names, &mut values)?;

                    // Expecting 0x42 to confirm deterministic execution
                    println!("Guest halted (Record). Verification RAX: 0x{:X}", unsafe {
                        values[0].Reg64
                    });
                    break;
                }
                4097 => {
                    // WHvRunVpExitReasonX64Cpuid
                    // Intercept CPUID, inject deterministic values, and log the event.
                    let sim_eax = 0xAAAA_AAAA;
                    let sim_ebx = 0xBBBB_BBBB;
                    let sim_ecx = 0xCCCC_CCCC;
                    let sim_edx = 0xDDDD_DDDD;

                    let names = [
                        WHvX64RegisterRax,
                        WHvX64RegisterRbx,
                        WHvX64RegisterRcx,
                        WHvX64RegisterRdx,
                    ];
                    let values = [
                        WHV_REGISTER_VALUE {
                            Reg64: sim_eax as u64,
                        },
                        WHV_REGISTER_VALUE {
                            Reg64: sim_ebx as u64,
                        },
                        WHV_REGISTER_VALUE {
                            Reg64: sim_ecx as u64,
                        },
                        WHV_REGISTER_VALUE {
                            Reg64: sim_edx as u64,
                        },
                    ];
                    self.vcpu.set_registers(&names, &values)?;

                    self.logger.log_event(TraceEvent {
                        instruction_count,
                        event: EventType::Cpuid {
                            eax: sim_eax,
                            ebx: sim_ebx,
                            ecx: sim_ecx,
                            edx: sim_edx,
                        },
                    });

                    instruction_count += 1;
                    let instruction_length = exit_ctx.VpContext._bitfield & 0x0F;
                    self.advance_rip(instruction_length)?;
                }
                4099 => {
                    // WHvRunVpExitReasonX64Rdtsc
                    // Intercept RDTSC, inject deterministic cycle count, and log the event.
                    let sim_rax = 0x1000;
                    let sim_rdx = 0x0;

                    let names = [WHvX64RegisterRax, WHvX64RegisterRdx];
                    let values = [
                        WHV_REGISTER_VALUE { Reg64: sim_rax },
                        WHV_REGISTER_VALUE { Reg64: sim_rdx },
                    ];
                    self.vcpu.set_registers(&names, &values)?;

                    self.logger.log_event(TraceEvent {
                        instruction_count,
                        event: EventType::Rdtsc {
                            rax: sim_rax,
                            rdx: sim_rdx,
                        },
                    });

                    instruction_count += 1;
                    let instruction_length = exit_ctx.VpContext._bitfield & 0x0F;
                    self.advance_rip(instruction_length)?;
                }
                4 => {
                    // WHvRunVpExitReasonX64Callout
                    // Intercept VMCALL used for cooperative context switching.
                    let names = [WHvX64RegisterRcx];
                    let mut values = [WHV_REGISTER_VALUE::default()];
                    self.vcpu.get_registers(&names, &mut values)?;

                    let thread_id = unsafe { values[0].Reg64 } as u32;
                    self.logger.log_event(TraceEvent {
                        instruction_count,
                        event: EventType::ContextSwitch {
                            next_thread_id: thread_id,
                        },
                    });

                    instruction_count += 1;
                    // VMCALL instruction is exactly 3 bytes long
                    self.advance_rip(3)?;
                }

                _ => {
                    println!("Execution stopped. Unhandled VM exit reason: {}", reason);
                    break;
                }
            }
        }

        Ok(())
    }

    /// Advances the guest instruction pointer (RIP) past the intercepted instruction.
    fn advance_rip(&self, instruction_length: u8) -> Result<()> {
        let names = [WHvX64RegisterRip];
        let mut values = [WHV_REGISTER_VALUE::default()];
        self.vcpu.get_registers(&names, &mut values)?;

        unsafe {
            values[0].Reg64 += instruction_length as u64;
        }

        self.vcpu.set_registers(&names, &values)?;
        Ok(())
    }

    /// Consumes the engine and returns the recorded trace events.
    pub fn finalize(self) -> Vec<TraceEvent> {
        self.logger.events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::{deserialize, serialize};

    #[cfg(not(miri))] // Bypasses Miri to prevent unused import warning.
    use windows::Win32::System::Hypervisor::WHV_REGISTER_VALUE;

    #[test]
    fn test_aligned_memory_serde() {
        // Validate custom Bincode serialization for AlignedMemory buffers.
        let mut mem = AlignedMemory::new(4096);
        let test_data = b"Deterministic execution test data";

        unsafe {
            std::ptr::copy_nonoverlapping(test_data.as_ptr(), mem.as_mut_ptr(), test_data.len());
        }

        let serialized_data = serialize(&mem).expect("Failed to serialize AlignedMemory");
        let deserialized_mem: AlignedMemory =
            deserialize(&serialized_data).expect("Failed to deserialize");

        assert_eq!(mem.len(), deserialized_mem.len());

        let deserialized_slice = unsafe {
            std::slice::from_raw_parts(deserialized_mem.as_ptr(), deserialized_mem.len())
        };
        assert_eq!(&deserialized_slice[..test_data.len()], test_data);
    }

    #[test]
    #[cfg(not(miri))]
    fn test_snapshot_registers_serde() {
        // Validate custom Bincode serialization for C-union register structures.
        let mut snapshot = Snapshot::new();
        snapshot.registers.push(WHV_REGISTER_VALUE {
            Reg64: 0xDEADBEEFCAFEBABE,
        });

        let serialized_data = serialize(&snapshot).expect("Failed to serialize Snapshot");
        let deserialized_snap: Snapshot =
            deserialize(&serialized_data).expect("Failed to deserialize");

        assert_eq!(snapshot.registers.len(), deserialized_snap.registers.len());
        assert_eq!(unsafe { snapshot.registers[0].Reg64 }, unsafe {
            deserialized_snap.registers[0].Reg64
        });
    }
}
