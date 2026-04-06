#![no_std]
#![no_main]

use core::hint::black_box;

// Conditionally compile PanicInfo to prevent unused import warnings during tests.
#[cfg(not(test))]
use core::panic::PanicInfo;

/// Bare-metal entry point for the end-to-end test payload.
/// Designed to execute non-deterministic instructions for hypervisor interception.
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Trigger VM exit to capture the simulated RDTSC value.
    let rax: u64;
    unsafe {
        core::arch::asm!("rdtsc", out("rax") rax, out("rdx") _);
    }

    // Trigger VM exit to capture the simulated CPUID value.
    let cpuid_eax: u32;

    // Temporary 64-bit register for RBX preservation (LLVM inline assembly requirement).
    let mut _ebx_out: u64;
    unsafe {
        core::arch::asm!(
        "xchg rbx, {tmp}",
        "cpuid",
        "xchg rbx, {tmp}",
        inout("eax") 0 => cpuid_eax,
        out("ecx") _,
        out("edx") _,
        tmp = out(reg) _ebx_out,
        );
    }

    // Verify deterministic state injection from the hypervisor.
    // Expected values: CPUID EAX = 0xAAAA_AAAA, RDTSC RAX = 0x1000.
    let result = if cpuid_eax == 0xAAAA_AAAA && rax == 0x1000 {
        0x42
    } else {
        0xDEAD
    };

    // Prevent compiler optimization of the verification logic.
    black_box(result);

    // Halt execution. The hypervisor verifies the final state.
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}

/// Minimal panic handler for the no_std environment.
#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}

#[cfg(miri)]
#[unsafe(no_mangle)]
fn miri_start(_argc: isize, _argv: *const *const u8) -> isize {
    0
}
