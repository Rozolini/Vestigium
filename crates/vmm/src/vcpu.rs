use windows::core::Result;
use windows::Win32::System::Hypervisor::{
    WHvCreateVirtualProcessor, WHvDeleteVirtualProcessor, WHvGetVirtualProcessorRegisters,
    WHvSetVirtualProcessorRegisters, WHV_PARTITION_HANDLE, WHV_REGISTER_NAME, WHV_REGISTER_VALUE,
};

/// Architectural MSRs for Performance Monitoring Counters (PMCs).
pub const MSR_IA32_FIXED_CTR0: u32 = 0x309; // Instructions retired counter
pub const MSR_IA32_FIXED_CTR_CTRL: u32 = 0x38D; // Fixed counter control

/// Represents a virtual CPU within a WHP partition.
pub struct Vcpu {
    partition: WHV_PARTITION_HANDLE,
    index: u32,
}

impl Vcpu {
    /// Allocates and initializes a vCPU instance.
    pub fn new(partition: WHV_PARTITION_HANDLE, index: u32) -> Result<Self> {
        unsafe {
            WHvCreateVirtualProcessor(partition, index, 0)?;
        }
        Ok(Self { partition, index })
    }

    /// Returns the zero-based index of the vCPU.
    #[inline]
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Retrieves the values of specified architectural registers.
    pub fn get_registers(
        &self,
        names: &[WHV_REGISTER_NAME],
        values: &mut [WHV_REGISTER_VALUE],
    ) -> Result<()> {
        assert_eq!(names.len(), values.len());
        unsafe {
            WHvGetVirtualProcessorRegisters(
                self.partition,
                self.index,
                names.as_ptr(),
                names.len() as u32,
                values.as_mut_ptr(),
            )?;
        }
        Ok(())
    }

    /// Updates the values of specified architectural registers.
    pub fn set_registers(
        &self,
        names: &[WHV_REGISTER_NAME],
        values: &[WHV_REGISTER_VALUE],
    ) -> Result<()> {
        assert_eq!(names.len(), values.len());
        unsafe {
            WHvSetVirtualProcessorRegisters(
                self.partition,
                self.index,
                names.as_ptr(),
                names.len() as u32,
                values.as_ptr(),
            )?;
        }
        Ok(())
    }

    /// Configures control registers and segments to transition the vCPU into x86_64 Long Mode.
    pub fn setup_long_mode(&self, pml4_gpa: u64) -> Result<()> {
        use windows::Win32::System::Hypervisor::*;

        // 64-bit code segment
        let cs = WHV_X64_SEGMENT_REGISTER {
            Limit: 0xFFFFFFFF,
            Selector: 0x08,
            Anonymous: WHV_X64_SEGMENT_REGISTER_0 { Attributes: 0xA09B },
            ..Default::default()
        };

        // 64-bit data segment
        let ds = WHV_X64_SEGMENT_REGISTER {
            Limit: 0xFFFFFFFF,
            Selector: 0x10,
            Anonymous: WHV_X64_SEGMENT_REGISTER_0 { Attributes: 0xC093 },
            ..Default::default()
        };

        // Task state segment
        let tr = WHV_X64_SEGMENT_REGISTER {
            Limit: 0xFFFF,
            Selector: 0x18,
            Anonymous: WHV_X64_SEGMENT_REGISTER_0 { Attributes: 0x008B },
            ..Default::default()
        };

        // Local descriptor table
        let ldtr = WHV_X64_SEGMENT_REGISTER {
            Anonymous: WHV_X64_SEGMENT_REGISTER_0 { Attributes: 0x0000 },
            ..Default::default()
        };

        let names = [
            WHvX64RegisterCr3,
            WHvX64RegisterCr4,
            WHvX64RegisterEfer,
            WHvX64RegisterCr0,
            WHvX64RegisterCs,
            WHvX64RegisterDs,
            WHvX64RegisterEs,
            WHvX64RegisterFs,
            WHvX64RegisterGs,
            WHvX64RegisterSs,
            WHvX64RegisterTr,
            WHvX64RegisterLdtr,
            WHvX64RegisterRflags,
        ];

        let values = [
            WHV_REGISTER_VALUE { Reg64: pml4_gpa },   // CR3 points to PML4
            WHV_REGISTER_VALUE { Reg64: 0x00000620 }, // CR4: PAE and OSFXSR enabled
            WHV_REGISTER_VALUE { Reg64: 0x00000500 }, // EFER: LME and LMA enabled
            WHV_REGISTER_VALUE { Reg64: 0x80000031 }, // CR0: PG, PE, NE enabled
            WHV_REGISTER_VALUE { Segment: cs },
            WHV_REGISTER_VALUE { Segment: ds },
            WHV_REGISTER_VALUE { Segment: ds },
            WHV_REGISTER_VALUE { Segment: ds },
            WHV_REGISTER_VALUE { Segment: ds },
            WHV_REGISTER_VALUE { Segment: ds },
            WHV_REGISTER_VALUE { Segment: tr },
            WHV_REGISTER_VALUE { Segment: ldtr },
            WHV_REGISTER_VALUE { Reg64: 0x00000002 }, // RFLAGS: Reserved bit 1 set
        ];

        self.set_registers(&names, &values)
    }
}

impl Drop for Vcpu {
    fn drop(&mut self) {
        unsafe {
            let _ = WHvDeleteVirtualProcessor(self.partition, self.index);
        }
    }
}
