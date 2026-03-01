//! Simple bump allocator backed by `sbrk`.
//!
//! Allocates by advancing the program break forward.  Deallocation is a
//! no-op — memory is only reclaimed when the process exits.  This is the
//! simplest possible allocator, useful as a baseline or for programs that
//! allocate but never free.

use core::alloc::Layout;

use super::Allocator;

/// A bump allocator that grows the heap via `sbrk` and never reuses freed
/// memory.
#[allow(dead_code)]
pub struct BumpAllocator;

#[allow(dead_code)]
impl BumpAllocator {
    pub const fn new() -> Self {
        Self
    }
}

impl Allocator for BumpAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();

        // Get current break.
        let current = crate::syscall::sys_sbrk(0) as usize;
        if current == 0 {
            return core::ptr::null_mut();
        }

        // Align upward.
        let aligned = (current + align - 1) & !(align - 1);
        let padding = aligned - current;
        let total = padding + size;

        // Extend the break.
        let base = crate::syscall::sys_sbrk(total as isize);
        if base.is_null() {
            return core::ptr::null_mut();
        }

        (base as usize + padding) as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // No-op: memory is reclaimed on process exit.
    }
}
