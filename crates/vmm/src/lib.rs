pub mod ept; // Extended Page Tables (EPT) management for guest memory isolation.
pub mod paging; // Guest page table structures for x86_64 long mode.
pub mod partition; // Windows Hypervisor Platform (WHP) partition lifecycle.
pub mod vcpu; // Virtual CPU state and execution control.
#[cfg(not(miri))]
#[cfg(test)]
mod tests {
    use crate::ept::EptManager;
    use crate::partition::VmPartition;
    use crate::vcpu::Vcpu;
    use std::alloc::{alloc, dealloc, Layout};
    use windows::Win32::System::Hypervisor::{
        WHvX64RegisterRip, WHV_MAP_GPA_RANGE_FLAGS, WHV_REGISTER_VALUE,
    };

    #[test]
    fn test_phase1_interception_foundation() {
        // 1. Initialize a new WHP partition.
        let mut partition = VmPartition::new().expect("Failed to create WHPX partition");

        // 2. Configure VMExit interceptions for deterministic event capture.
        partition
            .configure_interceptions()
            .expect("Failed to configure VMExit interceptions");

        // 3. Initialize the first vCPU for architectural state control.
        let vcpu = Vcpu::new(partition.as_raw(), 0).expect("Failed to create vCPU");

        // 4. Validate EPT (Extended Page Tables) mapping capability.
        let ept = EptManager::new(partition.as_raw());

        // WHPX requires host memory pointers to be 4KB page-aligned for EPT mapping.
        let layout = Layout::from_size_align(4096, 4096).expect("Invalid layout");
        let host_ptr = unsafe { alloc(layout) };

        // Define mapping permissions: Read (0x1) | Write (0x2).
        let flags = WHV_MAP_GPA_RANGE_FLAGS(0x3);

        // Map a guest physical address (GPA) to the allocated host memory.
        unsafe {
            ept.map_gpa_range(0x1000, host_ptr as _, 4096, flags)
                .expect("Failed to map EPT memory");
        }

        // 5. Validate vCPU register access (GET/SET) via the hypervisor.
        let names = [WHvX64RegisterRip];
        let mut values = [WHV_REGISTER_VALUE::default()];
        vcpu.get_registers(&names, &mut values)
            .expect("Failed to read vCPU registers");

        // Release allocated host memory.
        unsafe {
            dealloc(host_ptr, layout);
        }
    }
}
