use std::mem::size_of;
use vmm::partition::VmPartition;
use vmm::vcpu::Vcpu;
use windows::Win32::System::Hypervisor::{WHV_RUN_VP_EXIT_CONTEXT, WHvRunVirtualProcessor};
use windows::core::Result;

/// Encapsulates the execution loop for a virtual CPU.
pub struct VmRunner<'a> {
    partition: &'a VmPartition,
    vcpu: &'a Vcpu,
}

impl<'a> VmRunner<'a> {
    /// Initializes the runner with the target partition and vCPU.
    pub fn new(partition: &'a VmPartition, vcpu: &'a Vcpu) -> Self {
        Self { partition, vcpu }
    }

    /// Executes the virtual processor until a VM exit occurs.
    /// Returns the context detailing the reason for the exit.
    pub fn run(&self) -> Result<WHV_RUN_VP_EXIT_CONTEXT> {
        let mut exit_context = WHV_RUN_VP_EXIT_CONTEXT::default();
        let context_size = size_of::<WHV_RUN_VP_EXIT_CONTEXT>() as u32;

        // Invokes the Windows Hypervisor Platform API to run the vCPU.
        unsafe {
            WHvRunVirtualProcessor(
                self.partition.as_raw(),
                self.vcpu.index(),
                &mut exit_context as *mut _ as *mut _,
                context_size,
            )?;
        }

        Ok(exit_context)
    }
}
