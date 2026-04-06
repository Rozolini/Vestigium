use serde::{Deserialize, Serialize};

/// Represents intercepted non-deterministic events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventType {
    /// Captures system call results and associated memory modifications.
    Syscall {
        rax: u64,
        memory_writes: Vec<(u64, Vec<u8>)>,
    },
    /// Captures the cycle count returned by RDTSC.
    Rdtsc { rax: u64, rdx: u64 },
    /// Captures architectural state returned by CPUID.
    Cpuid {
        eax: u32,
        ebx: u32,
        ecx: u32,
        edx: u32,
    },
    /// Identifies a thread context switch.
    ContextSwitch { next_thread_id: u32 },
}

/// A single recorded event mapped to its exact execution timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    /// The retired instruction count at the time of interception.
    pub instruction_count: u64,
    pub event: EventType,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_serialization() {
        // Validate binary serialization and deserialization of trace events.
        let original_event = TraceEvent {
            instruction_count: 15420,
            event: EventType::Syscall {
                rax: 0x100,
                memory_writes: vec![(0x5000, vec![0xDE, 0xAD, 0xBE, 0xEF])],
            },
        };

        let encoded: Vec<u8> =
            bincode::serialize(&original_event).expect("Failed to serialize trace event");

        let decoded: TraceEvent =
            bincode::deserialize(&encoded).expect("Failed to deserialize trace event");

        assert_eq!(original_event, decoded);
    }
}
