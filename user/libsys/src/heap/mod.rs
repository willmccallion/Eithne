//! User-space memory allocator framework.
//!
//! Provides a trait-based allocator system so different implementations
//! (dlmalloc, bump, etc.) can be swapped at compile time.  The chosen
//! allocator is registered as the `#[global_allocator]` to back the
//! `alloc` crate (Vec, String, Box, etc.).

pub mod bump;
pub mod dlmalloc;

use core::alloc::{GlobalAlloc, Layout};

/// Trait for user-space heap allocators.
///
/// Implementations must be safe to use from a single-threaded context.
/// Thread-safety can be added via internal locking if needed.
pub trait Allocator {
    /// Allocate a block of memory satisfying the given layout.
    ///
    /// Returns a pointer to the allocated region, or null on failure.
    /// The returned memory is not required to be zeroed.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8;

    /// Deallocate a previously allocated block.
    ///
    /// `ptr` must have been returned by a prior call to `alloc` or `realloc`
    /// with a compatible layout.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout);

    /// Grow or shrink a previously allocated block.
    ///
    /// The default implementation allocates a new block, copies the data,
    /// and frees the old block.
    unsafe fn realloc(&self, ptr: *mut u8, old_layout: Layout, new_size: usize) -> *mut u8 {
        unsafe {
            let new_layout = Layout::from_size_align_unchecked(new_size, old_layout.align());
            let new_ptr = self.alloc(new_layout);
            if !new_ptr.is_null() {
                let copy_size = old_layout.size().min(new_size);
                core::ptr::copy_nonoverlapping(ptr, new_ptr, copy_size);
                self.dealloc(ptr, old_layout);
            }
            new_ptr
        }
    }
}

/// Bridges our `Allocator` trait to Rust's `GlobalAlloc` interface.
///
/// Wraps a concrete allocator type and delegates all allocation requests.
struct GlobalAllocBridge<A: Allocator>(A);

unsafe impl<A: Allocator> GlobalAlloc for GlobalAllocBridge<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { self.0.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.0.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        unsafe { self.0.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: GlobalAllocBridge<dlmalloc::DlMalloc> =
    GlobalAllocBridge(dlmalloc::DlMalloc::new());
