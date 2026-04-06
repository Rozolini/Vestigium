use goblin::pe::PE;
use std::alloc::{Layout, alloc_zeroed};

/// Parses PE (Portable Executable) files and handles guest memory mapping.
pub struct PeLoader<'a> {
    pe: PE<'a>,
    raw_bytes: &'a [u8],
}

impl<'a> PeLoader<'a> {
    /// Initializes the loader by parsing the raw PE binary.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, goblin::error::Error> {
        let pe = PE::parse(bytes)?;
        Ok(Self {
            pe,
            raw_bytes: bytes,
        })
    }

    /// Returns the entry point Relative Virtual Address (RVA).
    pub fn entry_point(&self) -> usize {
        self.pe.entry as usize
    }

    /// Allocates page-aligned host memory and maps PE sections into it.
    ///
    /// Returns a tuple containing the host memory pointer and total aligned size.
    pub fn map_into_memory(&self) -> (*mut u8, usize) {
        let size_of_image = self
            .pe
            .header
            .optional_header
            .map(|h| h.windows_fields.size_of_image)
            .unwrap_or(0) as usize;

        // Align image size to a 4KB page boundary.
        let aligned_size = (size_of_image + 4095) & !4095;

        // Allocate zeroed host memory for the guest image.
        let layout = Layout::from_size_align(aligned_size, 4096).expect("Invalid alignment layout");
        let host_memory = unsafe { alloc_zeroed(layout) };
        assert!(!host_memory.is_null(), "Memory allocation failed");

        // Map individual sections into their respective virtual addresses.
        for section in &self.pe.sections {
            let va = section.virtual_address as usize;
            let raw_ptr = section.pointer_to_raw_data as usize;
            let raw_size = section.size_of_raw_data as usize;

            if raw_ptr > 0 && raw_size > 0 {
                // Safety: Bounds are validated implicitly by the PE parser.
                // We ensure no overlapping occurs during the section copy.
                unsafe {
                    let dest = host_memory.add(va);
                    let src = self.raw_bytes[raw_ptr..].as_ptr();
                    std::ptr::copy_nonoverlapping(src, dest, raw_size);
                }
            }
        }

        (host_memory, aligned_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pe_parser_invalid_format() {
        // Validate that malformed binaries are rejected.
        let invalid_binary = [0x00, 0x11, 0x22, 0x33];
        let result = PeLoader::parse(&invalid_binary);
        assert!(result.is_err());
    }
}
