# Vestigium

Vestigium is a deterministic record and replay engine for x86_64 Windows. By intercepting and simulating non-deterministic events (syscalls, `rdtsc`, `cpuid`, thread scheduling), it provides exact memory and register state reproduction to facilitate concurrency debugging.

---

## Architecture

The system is built on the Windows Hypervisor Platform (WHPX) and is divided into distinct operational phases:

### 1. Interception Foundation (`vmm`)
- **Hardware Isolation:** Utilizes WHPX to isolate the target process, enforcing strict Ring 3 control.


- **VMExit Handling:** Configures Extended Page Tables (EPT) and VMExit interceptions for `syscall`, `cpuid`, and `rdtsc` instructions.


- **Instruction Counting:** Employs Performance Monitoring Counters (PMC) to precisely track thread progress.

### 2. Record Engine (`engine/record`)
- **State Snapshots:** Captures an initial memory snapshot of the mapped environment and the starting register context.


- **Event Telemetry:** Intercepts and logs syscall results (return codes and modified memory buffers) into a highly-compressed binary trace.


- **Scheduling Fixation:** Records thread context switch points pinned to the exact executed instruction count.

### 3. Replay Engine (`engine/replay`)
- **State Restoration:** Restores the process entirely from the initial memory snapshot.


- **Execution Simulation:** Blocks real syscall execution, instead injecting recorded data from the trace directly into registers and memory.


- **Deterministic Interleaving:** Forces thread context switches strictly according to the recorded instruction counter, perfectly mirroring the original OS scheduler's behavior.

---

## Getting Started

### Prerequisites

* Windows 10/11 Pro/Enterprise with the **Windows Hypervisor Platform** feature enabled.


* Rust toolchain (stable).


* Rust nightly (strictly for Miri verification).

### Quick Start (Payload Execution)

Vestigium includes a pre-configured `no_std` target template (`target_app`), eliminating the need for complex MSVC linker configurations.

### 1. Clone the repository

```bash
git clone https://github.com/Rozolini/Vestigium.git
cd Vestigium
```

### 2. Modify the payload

Open `target_app/src/main.rs`. The environment is already set up for bare-metal execution. Replace the default demo payload with your custom logic inside the `_start` function:

```rust
#![no_std]
#![no_main]

use core::arch::asm;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        // Trigger engine interception
        asm!("rdtsc", out("rax") _, out("rdx") _);

        loop {
            asm!("hlt");
        }
    }
}
```

### 3. Build and Execute

Compile the workspace and execute the payload through the Vestigium engine:

```bash
cargo build --release
cargo run --release --bin cli -- "target\release\vestigium_test.exe"
```
---

## Testing & Verification

Due to the strict requirements for deterministic execution, Vestigium relies on 
a multi-tiered automated verification pipeline.

### 1. Unit & Integration Testing

Validates trace serialization/deserialization, PE file parsing, isolated I/O syscalls,
memory allocation, and VMExit memory injections.

```bash
cargo test --workspace
```

### 2. Exhaustive Concurrency Testing (Loom)

Simulates thread interleavings and memory barrier executions to mathematically prove the 
absence of data races in the engine's state machine.

```PowerShell
$env:RUSTFLAGS="--cfg loom"; cargo test --workspace; $env:RUSTFLAGS=""
```

### 3. Undefined Behavior Detection (Miri)

Validates memory safety and strict pointer rules across the engine's 
internal structures (WHPX FFI is mocked during this phase).

```bash
cargo +nightly miri test --workspace
```

### 4. End-to-End (E2E) Determinism Verification

Compiles a bare-metal payload with an intentional data race, records its execution, 
and replays it to assert a 100% memory and register state match.

```PowerShell
# 1. Build the target test payload
cargo build --bin vestigium_test --release

# 2. Run the E2E record/replay pipeline
cargo test test_phase4_e2e_record_replay_pipeline --release
```

---

## License

This project is licensed under the MIT License. See the [LICENSE](./LICENSE) file for details.







