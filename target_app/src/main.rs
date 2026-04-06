#![no_std]
#![no_main]

use core::hint::black_box;

// Unsynchronized shared state for data race simulation.
static mut COUNTER: u64 = 0;

// Triggers a VM exit via VMCALL to yield execution to the hypervisor scheduler.
#[inline(never)]
fn sched_yield(thread_id: u64) {
    unsafe {
        core::arch::asm!(
        "vmcall",
        in("rax") 0x1, // Hypercall ID: Yield
        in("rcx") thread_id, // Current thread ID
        );
    }
}

// Performs a non-atomic increment interrupted by a forced context switch.
#[inline(always)]
unsafe fn execute_slice(thread_id: u64) {
    let temp = unsafe { COUNTER };
    sched_yield(thread_id);
    unsafe { COUNTER = temp + 1 };
}

/// Bare-metal entry point.
/// Simulates concurrent execution and evaluates deterministic interleaving.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Simulate two threads interlacing execution.
    for _ in 0..100 {
        unsafe {
            execute_slice(0);
            execute_slice(1);
        }
    }

    // Safely read the final value after all simulated threads complete.
    let final_val = unsafe { COUNTER };
    black_box(final_val);

    // Halt execution and pass the final counter value to the hypervisor via RAX.
    loop {
        unsafe {
            core::arch::asm!("hlt", in("rax") final_val);
        }
    }
}

// Halts the virtual processor on panic.
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}
