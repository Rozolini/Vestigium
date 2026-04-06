use std::mem::size_of;
use windows::core::Result;
use windows::Win32::System::Hypervisor::{
    WHvCreatePartition, WHvDeletePartition, WHvPartitionPropertyCodeExtendedVmExits,
    WHvPartitionPropertyCodeProcessorCount, WHvSetPartitionProperty, WHvSetupPartition,
    WHV_EXTENDED_VM_EXITS, WHV_PARTITION_HANDLE, WHV_PARTITION_PROPERTY,
};

/// Manages the lifecycle of a Windows Hypervisor Platform (WHP) partition.
pub struct VmPartition {
    handle: WHV_PARTITION_HANDLE,
}

impl VmPartition {
    /// Creates and initializes a new hardware-accelerated VM partition.
    pub fn new() -> Result<Self> {
        let handle = unsafe { WHvCreatePartition()? };

        // Allocate multiple vCPUs to support thread interleaving scenarios.
        let property = WHV_PARTITION_PROPERTY { ProcessorCount: 4 };

        unsafe {
            WHvSetPartitionProperty(
                handle,
                WHvPartitionPropertyCodeProcessorCount,
                &property as *const _ as *const _,
                size_of::<WHV_PARTITION_PROPERTY>() as u32,
            )?;

            WHvSetupPartition(handle)?;
        }

        Ok(Self { handle })
    }

    /// Configures VM exits to intercept non-deterministic instructions.
    pub fn configure_interceptions(&mut self) -> Result<()> {
        // Explicitly set bits to intercept CPUID (Bit 0), Exceptions (Bit 2), and RDTSC (Bit 3).
        let exits = WHV_EXTENDED_VM_EXITS {
            AsUINT64: (1 << 0) | (1 << 2) | (1 << 3),
        };

        let property = WHV_PARTITION_PROPERTY {
            ExtendedVmExits: exits,
        };

        unsafe {
            WHvSetPartitionProperty(
                self.handle,
                WHvPartitionPropertyCodeExtendedVmExits,
                &property as *const _ as *const _,
                size_of::<WHV_PARTITION_PROPERTY>() as u32,
            )?;
        }

        Ok(())
    }

    /// Returns the underlying WHP partition handle.
    #[inline]
    pub fn as_raw(&self) -> WHV_PARTITION_HANDLE {
        self.handle
    }
}

impl Drop for VmPartition {
    fn drop(&mut self) {
        if self.handle.0 != 0 {
            unsafe {
                let _ = WHvDeletePartition(self.handle);
            }
        }
    }
}
