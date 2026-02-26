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
