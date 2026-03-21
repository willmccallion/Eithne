use core::sync::atomic::{AtomicU64, Ordering};

/// A physical memory address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct PhysAddr(u64);

/// A virtual memory address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct VirtAddr(u64);

impl PhysAddr {
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Align address up to the given power-of-two alignment.
    pub const fn align_up(self, align: u64) -> Self {
        Self((self.0 + align - 1) & !(align - 1))
    }

    /// Align address down to the given power-of-two alignment.
    pub const fn align_down(self, align: u64) -> Self {
        Self(self.0 & !(align - 1))
    }

    /// Check if the address is aligned to the given power-of-two alignment.
    pub const fn is_aligned(self, align: u64) -> bool {
        self.0 & (align - 1) == 0
    }

    pub const fn offset(self, offset: u64) -> Self {
        Self(self.0 + offset)
    }
}

impl VirtAddr {
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }

    pub const fn align_up(self, align: u64) -> Self {
        Self((self.0 + align - 1) & !(align - 1))
    }

    pub const fn align_down(self, align: u64) -> Self {
        Self(self.0 & !(align - 1))
    }

    pub const fn is_aligned(self, align: u64) -> bool {
        self.0 & (align - 1) == 0
    }

    pub const fn offset(self, offset: u64) -> Self {
        Self(self.0 + offset)
    }

    pub const fn as_ptr<T>(self) -> *const T {
        self.0 as *const T
    }

    pub const fn as_mut_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }
}

/// Offset added to physical addresses to get their kernel direct-map virtual
/// address. Zero during early boot (identity map); set to `KERNEL_VIRT_BASE`
/// by `mm::set_higher_half_offset` after the new CR3 is loaded.
static PHYS_OFFSET: AtomicU64 = AtomicU64::new(0);

/// Switch `phys_to_virt` / `virt_to_phys` to use the higher-half offset.
///
/// Must be called immediately after `arch::init_higher_half` loads the new
/// CR3.  All subsequent allocations and pointer dereferences will use the
/// direct-map region.
pub fn set_higher_half_offset(offset: u64) {
    PHYS_OFFSET.store(offset, Ordering::Release);
}

/// Convert physical to virtual address via the kernel direct-map region.
///
/// During early boot this is an identity map (offset = 0).
/// After `set_higher_half_offset` it adds `KERNEL_VIRT_BASE`.
#[allow(clippy::inline_always)]
#[inline(always)]
pub fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new(phys.as_u64() + PHYS_OFFSET.load(Ordering::Acquire))
}

/// Convert virtual to physical address.
///
/// Inverse of `phys_to_virt`. Only valid for the kernel direct-map region.
#[allow(clippy::inline_always)]
#[inline(always)]
pub fn virt_to_phys(virt: VirtAddr) -> PhysAddr {
    PhysAddr::new(virt.as_u64() - PHYS_OFFSET.load(Ordering::Acquire))
}

impl From<u64> for PhysAddr {
    fn from(addr: u64) -> Self {
        Self(addr)
    }
}

impl From<u64> for VirtAddr {
    fn from(addr: u64) -> Self {
        Self(addr)
    }
}

impl core::fmt::Display for PhysAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PhysAddr({:#x})", self.0)
    }
}

impl core::fmt::Display for VirtAddr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "VirtAddr({:#x})", self.0)
    }
}
