//! Buddy frame allocator.
//!
//! O(1) allocation and deallocation of physical page frames using the same
//! buddy system algorithm as Linux's `mm/page_alloc.c`. Free blocks are
//! tracked in per-order doubly-linked lists stored intrusively in the free
//! pages themselves. Coalescing uses an XOR bitmap for O(1) buddy lookup.

use core::ptr;

use super::addr::PhysAddr;
use super::boot_info::BootMemoryInfo;
use super::page::{self, MAX_ORDER, PAGE_SIZE};
use super::traits::FrameAllocator;
use crate::sync::{Once, Spinlock};

/// Intrusive doubly-linked list node written into free pages.
/// Minimum page size is 4 KiB, so there's always room for two pointers.
#[repr(C)]
struct FreeBlock {
    next: *mut Self,
    prev: *mut Self,
}

/// Head of a circular doubly-linked free list for one buddy order.
struct FreeList {
    head: *mut FreeBlock,
    count: usize,
}

// SAFETY: FreeBlock pointers are only accessed under the global allocator lock.
unsafe impl Send for FreeList {}

impl FreeList {
    const fn empty() -> Self {
        Self {
            head: ptr::null_mut(),
            count: 0,
        }
    }

    #[allow(clippy::inline_always)]
    #[inline(always)]
    const fn is_empty(&self) -> bool {
        self.head.is_null()
    }

    /// Push a block (by physical address) onto the front of this list.
    fn push(&mut self, addr: u64) {
        let block = addr as *mut FreeBlock;
        unsafe {
            if self.head.is_null() {
                (*block).next = block;
                (*block).prev = block;
            } else {
                let old_head = self.head;
                let tail = (*old_head).prev;
                (*block).next = old_head;
                (*block).prev = tail;
                (*old_head).prev = block;
                (*tail).next = block;
            }
        }
        self.head = block;
        self.count += 1;
    }

    /// Pop a block from the front. Returns the physical address.
    fn pop(&mut self) -> u64 {
        debug_assert!(!self.head.is_null());
        let block = self.head;
        unsafe {
            if (*block).next == block {
                self.head = ptr::null_mut();
            } else {
                let next = (*block).next;
                let prev = (*block).prev;
                (*next).prev = prev;
                (*prev).next = next;
                self.head = next;
            }
        }
        self.count -= 1;
        block as u64
    }

    /// Remove a specific block from anywhere in the list. O(1) because
    /// we have prev/next pointers.
    fn remove(&mut self, addr: u64) {
        let block = addr as *mut FreeBlock;
        unsafe {
            if (*block).next == block {
                // Only element
                self.head = ptr::null_mut();
            } else {
                let next = (*block).next;
                let prev = (*block).prev;
                (*next).prev = prev;
                (*prev).next = next;
                if self.head == block {
                    self.head = next;
                }
            }
        }
        self.count -= 1;
    }
}

pub struct BuddyAllocator {
    /// One free list per order (0..MAX_ORDER).
    free_lists: [FreeList; MAX_ORDER],

    /// XOR bitmap for buddy coalescing.
    ///
    /// For order k, bit index = bitmap_offsets[k] + (pfn >> (k+1)).
    /// The bit is the XOR of the two buddies' allocation states:
    ///   0 = both allocated or both free (cannot coalesce)
    ///   1 = exactly one is free (can coalesce when the other frees)
    bitmap: &'static mut [u8],

    /// Starting bit offset in the bitmap for each order.
    bitmap_offsets: [usize; MAX_ORDER],

    /// Total free pages (order-0 equivalent).
    free_pages: usize,
}

// SAFETY: Only accessed under a Spinlock.
unsafe impl Send for BuddyAllocator {}

impl BuddyAllocator {
    /// Initialize the buddy allocator from parsed UEFI memory info.
    ///
    /// 1. Calculate bitmap size based on actual physical memory extent.
    /// 2. Steal pages from the first large-enough region for the bitmap.
    /// 3. Populate free lists from all usable regions.
    pub fn init(boot_info: &BootMemoryInfo) -> Self {
        let max_pfn = (boot_info.max_phys_addr.as_u64() / PAGE_SIZE as u64) as usize;

        // Calculate bitmap layout
        let mut total_bitmap_bits: usize = 0;
        let mut bitmap_offsets = [0usize; MAX_ORDER];
        for (order, offset) in bitmap_offsets.iter_mut().enumerate() {
            *offset = total_bitmap_bits;
            let pairs = max_pfn / (2 << order);
            total_bitmap_bits += pairs;
        }
        let bitmap_bytes = total_bitmap_bits.div_ceil(8);
        let bitmap_pages = bitmap_bytes.div_ceil(PAGE_SIZE);

        // Find a region large enough to hold the bitmap
        let mut bitmap_region_idx = usize::MAX;
        let mut bitmap_start = 0u64;
        for (i, region) in boot_info.usable_regions().iter().enumerate() {
            if region.page_count as usize >= bitmap_pages {
                bitmap_region_idx = i;
                bitmap_start = region.start.as_u64();
                break;
            }
        }
        assert!(
            bitmap_region_idx != usize::MAX,
            "no region large enough for buddy bitmap"
        );

        // Zero-initialize bitmap (identity-mapped, write directly)
        let bitmap_ptr = bitmap_start as *mut u8;
        unsafe {
            ptr::write_bytes(bitmap_ptr, 0, bitmap_pages * PAGE_SIZE);
        }
        let bitmap = unsafe { core::slice::from_raw_parts_mut(bitmap_ptr, bitmap_bytes) };

        let mut alloc = Self {
            free_lists: [
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
                FreeList::empty(),
            ],
            bitmap,
            bitmap_offsets,
            free_pages: 0,
        };

        // Add all usable regions to the free lists
        for (i, region) in boot_info.usable_regions().iter().enumerate() {
            let mut start = region.start.as_u64();
            let end = region.end().as_u64();

            // Skip the pages we stole for the bitmap
            if i == bitmap_region_idx {
                start += (bitmap_pages as u64) * (PAGE_SIZE as u64);
            }

            if start >= end {
                continue;
            }

            alloc.add_region(start, end);
        }

        alloc
    }

    /// Add a contiguous physical region to the free lists.
    ///
    /// Maximizes use of higher orders by choosing the largest naturally-aligned
    /// block that fits within the remaining region at each step.
    fn add_region(&mut self, start: u64, end: u64) {
        let mut addr = start;

        while addr < end {
            // Find the largest order where:
            // 1. addr is naturally aligned to that order's block size
            // 2. The block fits within the remaining region
            let mut order = 0;
            while order + 1 < MAX_ORDER {
                let next_size = (PAGE_SIZE << (order + 1)) as u64;
                if addr & (next_size - 1) != 0 {
                    break; // not aligned for next order
                }
                if addr + next_size > end {
                    break; // would exceed region
                }
                order += 1;
            }

            let block_size = (PAGE_SIZE << order) as u64;
            if addr + block_size > end {
                // Current order doesn't fit — shouldn't normally happen given
                // the loop above, but handle gracefully
                if order == 0 {
                    break;
                }
                order -= 1;
            }

            self.free_list_push(addr, order);
            self.free_pages += page::order_to_pages(order);
            addr += (PAGE_SIZE << order) as u64;
        }
    }

    /// Allocate 2^order contiguous pages. Returns the physical base address.
    pub fn alloc_pages(&mut self, order: usize) -> Option<PhysAddr> {
        debug_assert!(order < MAX_ORDER);

        // Find the smallest available order >= requested
        let mut current_order = order;
        while current_order < MAX_ORDER {
            if !self.free_lists[current_order].is_empty() {
                break;
            }
            current_order += 1;
        }
        if current_order >= MAX_ORDER {
            return None; // OOM
        }

        // Pop block from this order's free list
        let block_addr = self.free_list_pop(current_order);

        // Split down to the requested order
        while current_order > order {
            current_order -= 1;
            // The upper buddy half goes back on the free list
            let buddy_addr = block_addr + (PAGE_SIZE << current_order) as u64;
            self.free_list_push(buddy_addr, current_order);
        }

        self.free_pages -= page::order_to_pages(order);
        Some(PhysAddr::new(block_addr))
    }

    /// Deallocate 2^order contiguous pages starting at addr.
    pub fn dealloc_pages(&mut self, addr: PhysAddr, order: usize) {
        let mut addr = addr.as_u64();
        let mut current_order = order;

        // Try to coalesce with buddies up the order chain
        while current_order < MAX_ORDER - 1 {
            let was_set = self.toggle_buddy_bit(addr, current_order);

            if was_set {
                // Bit went 1→0: buddy is also free → coalesce
                let buddy_addr = addr ^ ((PAGE_SIZE << current_order) as u64);
                self.free_lists[current_order].remove(buddy_addr);

                // Merged block starts at the lower of the two addresses
                addr &= !((PAGE_SIZE << current_order) as u64);
                current_order += 1;
            } else {
                // Bit went 0→1: buddy is allocated → stop
                break;
            }
        }

        self.free_list_push(addr, current_order);
        self.free_pages += page::order_to_pages(order);
    }

    /// Toggle the XOR buddy bit for the pair containing `addr` at `order`.
    /// Returns the *previous* value of the bit.
    #[allow(clippy::inline_always)]
    #[inline(always)]
    fn toggle_buddy_bit(&mut self, addr: u64, order: usize) -> bool {
        let pfn = (addr / PAGE_SIZE as u64) as usize;
        let pair_index = pfn >> (order + 1);
        let bit_index = self.bitmap_offsets[order] + pair_index;
        let byte_index = bit_index / 8;
        let bit_offset = bit_index % 8;
        let mask = 1u8 << bit_offset;

        let prev = self.bitmap[byte_index] & mask != 0;
        self.bitmap[byte_index] ^= mask;
        prev
    }

    /// Push a block onto a free list and toggle its buddy bit.
    fn free_list_push(&mut self, addr: u64, order: usize) {
        let _ = self.toggle_buddy_bit(addr, order);
        self.free_lists[order].push(addr);
    }

    /// Pop a block from a free list and toggle its buddy bit.
    fn free_list_pop(&mut self, order: usize) -> u64 {
        let addr = self.free_lists[order].pop();
        let _ = self.toggle_buddy_bit(addr, order);
        addr
    }
}

impl FrameAllocator for BuddyAllocator {
    fn alloc_frame(&mut self) -> Option<PhysAddr> {
        self.alloc_pages(0)
    }

    fn dealloc_frame(&mut self, frame: PhysAddr) {
        self.dealloc_pages(frame, 0);
    }

    fn alloc_pages(&mut self, order: usize) -> Option<PhysAddr> {
        Self::alloc_pages(self, order)
    }

    fn dealloc_pages(&mut self, addr: PhysAddr, order: usize) {
        Self::dealloc_pages(self, addr, order);
    }

    fn free_frame_count(&self) -> usize {
        self.free_pages
    }
}

static FRAME_ALLOCATOR: Once<Spinlock<BuddyAllocator>> = Once::new();

/// Initialize the global frame allocator. Must be called once during boot.
pub fn init(boot_info: &BootMemoryInfo) {
    let _ = FRAME_ALLOCATOR.call_once(|| Spinlock::new(BuddyAllocator::init(boot_info)));
}

/// Allocate a single 4 KiB frame.
pub fn alloc_frame() -> Option<PhysAddr> {
    FRAME_ALLOCATOR.get()?.lock().alloc_pages(0)
}

/// Deallocate a single 4 KiB frame.
pub fn dealloc_frame(frame: PhysAddr) {
    if let Some(alloc) = FRAME_ALLOCATOR.get() {
        alloc.lock().dealloc_pages(frame, 0);
    }
}

/// Allocate 2^order contiguous pages.
pub fn alloc_pages(order: usize) -> Option<PhysAddr> {
    FRAME_ALLOCATOR.get()?.lock().alloc_pages(order)
}

/// Deallocate 2^order contiguous pages.
pub fn dealloc_pages(addr: PhysAddr, order: usize) {
    if let Some(alloc) = FRAME_ALLOCATOR.get() {
        alloc.lock().dealloc_pages(addr, order);
    }
}

/// Return the number of free pages.
pub fn free_page_count() -> usize {
    FRAME_ALLOCATOR
        .get()
        .map_or(0, |a| a.lock().free_pages)
}
