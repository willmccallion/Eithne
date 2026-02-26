//! Parse UEFI memory map into a kernel-internal representation.
//!
//! The UEFI memory map uses variable-sized descriptors backed by pool memory.
//! We copy the relevant data into a fixed-size static array so it can be used
//! after UEFI boot services memory is reclaimed.

use super::addr::PhysAddr;
use super::page::PAGE_SIZE;

/// Maximum number of usable memory regions we expect from UEFI.
/// Typical systems report 20-80 regions; 128 is generous.
const MAX_MEMORY_REGIONS: usize = 128;

/// A single contiguous region of usable physical memory.
#[derive(Debug, Clone, Copy)]
pub struct MemoryRegion {
    pub start: PhysAddr,
    pub page_count: u64,
}

impl MemoryRegion {
    pub const EMPTY: Self = Self {
        start: PhysAddr::new(0),
        page_count: 0,
    };

    pub const fn end(&self) -> PhysAddr {
        PhysAddr::new(self.start.as_u64() + self.page_count * PAGE_SIZE as u64)
    }

    pub const fn size_bytes(&self) -> u64 {
        self.page_count * PAGE_SIZE as u64
    }
}

/// Parsed boot memory information, stored in a fixed-size array (no heap).
pub struct BootMemoryInfo {
    pub regions: [MemoryRegion; MAX_MEMORY_REGIONS],
    pub region_count: usize,
    pub total_pages: u64,
    pub max_phys_addr: PhysAddr,
}

impl BootMemoryInfo {
    /// Parse a UEFI `MemoryMapOwned` into our internal representation.
    ///
    /// Extracts all usable regions (CONVENTIONAL, BOOT_SERVICES_CODE/DATA),
    /// skipping the first 1 MiB to avoid legacy BIOS areas.
    pub fn from_uefi_mmap(mmap: &uefi::mem::memory_map::MemoryMapOwned) -> Self {
        use uefi::boot::MemoryType;
        use uefi::mem::memory_map::MemoryMap;

        let mut info = Self {
            regions: [MemoryRegion::EMPTY; MAX_MEMORY_REGIONS],
            region_count: 0,
            total_pages: 0,
            max_phys_addr: PhysAddr::new(0),
        };

        for desc in mmap.entries() {
            // Only use CONVENTIONAL memory for now. BOOT_SERVICES_CODE/DATA
            // pages are physically free after exit_boot_services(), but UEFI
            // may have already unmapped them from its page tables — writing
            // to them would page-fault. We can reclaim them later once we
            // set up our own page tables.
            let usable = desc.ty == MemoryType::CONVENTIONAL;

            if !usable || desc.page_count == 0 {
                continue;
            }

            let start = desc.phys_start;
            let end = start + desc.page_count * PAGE_SIZE as u64;

            // Skip below 1 MiB (legacy BIOS area, real-mode IVT, etc.)
            if end <= 0x10_0000 {
                continue;
            }
            let clamped_start = if start < 0x10_0000 { 0x10_0000 } else { start };
            let clamped_pages = (end - clamped_start) / PAGE_SIZE as u64;

            if clamped_pages == 0 || info.region_count >= MAX_MEMORY_REGIONS {
                continue;
            }

            info.regions[info.region_count] = MemoryRegion {
                start: PhysAddr::new(clamped_start),
                page_count: clamped_pages,
            };
            info.region_count += 1;
            info.total_pages += clamped_pages;

            let region_end = PhysAddr::new(clamped_start + clamped_pages * PAGE_SIZE as u64);
            if region_end.as_u64() > info.max_phys_addr.as_u64() {
                info.max_phys_addr = region_end;
            }
        }

        info
    }

    pub fn usable_regions(&self) -> &[MemoryRegion] {
        &self.regions[..self.region_count]
    }
}
