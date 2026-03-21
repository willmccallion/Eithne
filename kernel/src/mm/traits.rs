use super::addr::{PhysAddr, VirtAddr};

/// Flags controlling page permissions and attributes.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy)]
pub struct PageFlags {
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
    pub user_accessible: bool,
    pub cache_disable: bool,
}

impl PageFlags {
    pub const KERNEL_RW: Self = Self {
        readable: true,
        writable: true,
        executable: false,
        user_accessible: false,
        cache_disable: false,
    };

    pub const KERNEL_RX: Self = Self {
        readable: true,
        writable: false,
        executable: true,
        user_accessible: false,
        cache_disable: false,
    };

    pub const USER_RW: Self = Self {
        readable: true,
        writable: true,
        executable: false,
        user_accessible: true,
        cache_disable: false,
    };

    pub const USER_RX: Self = Self {
        readable: true,
        writable: false,
        executable: true,
        user_accessible: true,
        cache_disable: false,
    };
}

#[derive(Debug)]
pub enum MapError {
    FrameAllocationFailed,
    AlreadyMapped,
    InvalidAddress,
}

#[derive(Debug)]
pub enum UnmapError {
    NotMapped,
    InvalidAddress,
}

/// Physical frame allocator.
///
/// Manages allocation and deallocation of physical memory in page-sized
/// (or multi-page) units. Implementations must be safe to use behind a lock.
pub trait FrameAllocator {
    /// Allocate a single physical frame (4 KiB). Returns None if OOM.
    fn alloc_frame(&mut self) -> Option<PhysAddr>;

    /// Deallocate a single physical frame.
    fn dealloc_frame(&mut self, frame: PhysAddr);

    /// Allocate 2^order contiguous physical pages.
    fn alloc_pages(&mut self, order: usize) -> Option<PhysAddr>;

    /// Deallocate 2^order contiguous physical pages.
    fn dealloc_pages(&mut self, addr: PhysAddr, order: usize);

    /// Return the number of free frames (order-0 equivalent) available.
    fn free_frame_count(&self) -> usize;
}

/// Virtual → physical page mapper.
///
/// Each architecture provides its own implementation that manipulates
/// the hardware page tables (x86_64 4-level, ARM TTBR, RISC-V Sv48).
pub trait PageMapper {
    /// Map a virtual address to a physical address with given flags.
    fn map(&mut self, virt: VirtAddr, phys: PhysAddr, flags: PageFlags) -> Result<(), MapError>;

    /// Unmap a virtual address, returning the physical address it was mapped to.
    fn unmap(&mut self, virt: VirtAddr) -> Result<PhysAddr, UnmapError>;

    /// Translate a virtual address to its physical mapping, if one exists.
    fn translate(&self, virt: VirtAddr) -> Option<PhysAddr>;
}

/// High-level address space management.
///
/// An AddressSpace owns a set of page mappings (one per process, plus the kernel).
pub trait AddressSpace {
    /// Switch the CPU to use this address space.
    fn activate(&self);

    /// Map a region of virtual memory.
    fn map_region(
        &mut self,
        virt_start: VirtAddr,
        phys_start: PhysAddr,
        size: usize,
        flags: PageFlags,
    ) -> Result<(), MapError>;

    /// Unmap a region of virtual memory.
    fn unmap_region(&mut self, virt_start: VirtAddr, size: usize) -> Result<(), UnmapError>;
}
