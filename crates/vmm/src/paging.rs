use std::alloc::{alloc_zeroed, Layout};

/// Manages identity-mapped page tables required for x86_64 Long Mode.
pub struct PageTable {
    pub pml4_gpa: u64,
    pub host_ptr: *mut u8,
    pub size: usize,
}

impl PageTable {
    /// Creates a 1:1 identity mapping for the first 1GB of guest memory using 2MB huge pages.
    pub fn create_identity_mapping(base_gpa: u64) -> Self {
        // Allocate 3 contiguous 4KB pages: PML4, PDPT, and PD (12KB total).
        let size = 4096 * 3;
        let layout = Layout::from_size_align(size, 4096).expect("Invalid alignment");
        let host_ptr = unsafe { alloc_zeroed(layout) };

        let pml4 = host_ptr as *mut u64;
        let pdpt = unsafe { host_ptr.add(4096) } as *mut u64;
        let pd = unsafe { host_ptr.add(8192) } as *mut u64;

        unsafe {
            // Link PML4 to PDPT and PDPT to PD with Present and R/W flags (0x3).
            *pml4 = (base_gpa + 4096) | 0x3;
            *pdpt = (base_gpa + 8192) | 0x3;

            // Map 512 entries of 2MB each (1GB total) using the Page Size flag (0x83).
            for i in 0..512 {
                *pd.add(i) = ((i as u64) * 0x200000) | 0x83;
            }
        }

        Self {
            pml4_gpa: base_gpa,
            host_ptr,
            size,
        }
    }
}
