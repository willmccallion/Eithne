//! Slab-based kernel heap allocator.
//!
//! Small allocations (8–2048 bytes) use per-size-class slab caches with
//! O(1) alloc/free. Large allocations (> PAGE_SIZE) go directly to the
//! buddy frame allocator. This is a simplified version of Linux's SLUB.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use super::addr::PhysAddr;
use super::frame;
use super::page::PAGE_SIZE;
use crate::sync::Spinlock;

/// Size classes: power-of-two from 8 to 2048.
const NUM_SIZE_CLASSES: usize = 9;
const SIZE_CLASSES: [usize; NUM_SIZE_CLASSES] = [8, 16, 32, 64, 128, 256, 512, 1024, 2048];

/// Find the size class index for a layout, or None if too large for slabs.
#[inline(always)]
fn size_class_index(layout: Layout) -> Option<usize> {
    let size = layout.size().max(layout.align());
    // Binary search through size classes
    let mut i = 0;
    while i < NUM_SIZE_CLASSES {
        if SIZE_CLASSES[i] >= size {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Free object node stored inline in unused slab slots.
/// Minimum size class is 8 bytes, which fits a single pointer.
#[repr(C)]
struct FreeObject {
    next: *mut FreeObject,
}

/// Per-size-class slab cache.
struct SlabCache {
    free_list: *mut FreeObject,
    object_size: usize,
}

// SAFETY: Only accessed under per-size-class Spinlock.
unsafe impl Send for SlabCache {}

impl SlabCache {
    const fn new(object_size: usize) -> Self {
        Self {
            free_list: ptr::null_mut(),
            object_size,
        }
    }

    /// Allocate a new slab page and thread all its objects into the free list.
    fn grow(&mut self) -> bool {
        let page = match frame::alloc_frame() {
            Some(p) => p,
            None => return false,
        };

        let base = page.as_u64() as *mut u8;
        let objects_per_page = PAGE_SIZE / self.object_size;

        // Thread objects backwards so the first object is at the head
        // (better cache locality for the next alloc).
        for i in (0..objects_per_page).rev() {
            let obj = unsafe { base.add(i * self.object_size) as *mut FreeObject };
            unsafe {
                (*obj).next = self.free_list;
            }
            self.free_list = obj;
        }

        true
    }

    fn alloc(&mut self) -> *mut u8 {
        if self.free_list.is_null() && !self.grow() {
            return ptr::null_mut();
        }

        let obj = self.free_list;
        unsafe {
            self.free_list = (*obj).next;
        }
        obj as *mut u8
    }

    fn dealloc(&mut self, ptr: *mut u8) {
        let obj = ptr as *mut FreeObject;
        unsafe {
            (*obj).next = self.free_list;
        }
        self.free_list = obj;
    }
}

/// Kernel heap allocator with per-size-class slab caches.
///
/// Each size class has its own `Spinlock`, so allocations of different
/// sizes never contend with each other.
pub struct KernelHeapAllocator {
    caches: [Spinlock<SlabCache>; NUM_SIZE_CLASSES],
}

impl KernelHeapAllocator {
    pub const fn new() -> Self {
        Self {
            caches: [
                Spinlock::new(SlabCache::new(8)),
                Spinlock::new(SlabCache::new(16)),
                Spinlock::new(SlabCache::new(32)),
                Spinlock::new(SlabCache::new(64)),
                Spinlock::new(SlabCache::new(128)),
                Spinlock::new(SlabCache::new(256)),
                Spinlock::new(SlabCache::new(512)),
                Spinlock::new(SlabCache::new(1024)),
                Spinlock::new(SlabCache::new(2048)),
            ],
        }
    }
}

unsafe impl GlobalAlloc for KernelHeapAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if let Some(idx) = size_class_index(layout) {
            // Small: slab cache
            self.caches[idx].lock().alloc()
        } else if layout.size() <= PAGE_SIZE {
            // Fits in one page but doesn't match a slab size class
            match frame::alloc_frame() {
                Some(addr) => addr.as_u64() as *mut u8,
                None => ptr::null_mut(),
            }
        } else {
            // Large: round up to pages, use buddy allocator
            let pages = layout.size().div_ceil(PAGE_SIZE);
            let order = pages.next_power_of_two().trailing_zeros() as usize;
            match frame::alloc_pages(order) {
                Some(addr) => addr.as_u64() as *mut u8,
                None => ptr::null_mut(),
            }
        }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if let Some(idx) = size_class_index(layout) {
            self.caches[idx].lock().dealloc(ptr);
        } else if layout.size() <= PAGE_SIZE {
            frame::dealloc_frame(PhysAddr::new(ptr as u64));
        } else {
            let pages = layout.size().div_ceil(PAGE_SIZE);
            let order = pages.next_power_of_two().trailing_zeros() as usize;
            frame::dealloc_pages(PhysAddr::new(ptr as u64), order);
        }
    }
}

#[global_allocator]
static ALLOCATOR: KernelHeapAllocator = KernelHeapAllocator::new();
