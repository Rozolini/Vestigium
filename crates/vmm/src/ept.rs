use std::ffi::c_void;
use windows::core::Result;
use windows::Win32::System::Hypervisor::{
    WHvMapGpaRange, WHV_MAP_GPA_RANGE_FLAGS, WHV_PARTITION_HANDLE,
};

/// Manages Extended Page Tables (EPT) for guest memory isolation.
pub struct EptManager {
    partition: WHV_PARTITION_HANDLE,
}

impl EptManager {
    /// Initializes the EPT manager for the specified partition.
    pub fn new(partition: WHV_PARTITION_HANDLE) -> Self {
        Self { partition }
    }

    /// Maps a host memory region into the guest physical address (GPA) space.
    ///
    /// # Safety
    /// `host_address` must point to a valid, page-aligned memory region
    /// of at least `size` bytes and remain valid while mapped.
    pub unsafe fn map_gpa_range(
        &self,
        guest_address: u64,
        host_address: *mut c_void,
        size: u64,
        flags: WHV_MAP_GPA_RANGE_FLAGS,
    ) -> Result<()> {
        WHvMapGpaRange(self.partition, host_address, guest_address, size, flags)?;

        Ok(())
    }
}
