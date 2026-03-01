//! dlmalloc-style allocator backed by `sbrk`.
//!
//! Based on Doug Lea's malloc algorithm — the same design used by glibc's
//! ptmalloc2.  Implements boundary-tag coalescing, binned free lists (small
//! bins for exact sizes, large bins for ranges), chunk splitting, and a
//! wilderness (top) chunk for efficient heap growth.
//!
//! Simplified from the full dlmalloc: no mmap fallback, no per-thread
//! arenas, no fastbins.  These can be added later.

use core::alloc::Layout;
use core::cell::UnsafeCell;
use core::ptr;

use super::Allocator;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum alignment for all allocations (8 bytes on 64-bit).
const ALIGNMENT: usize = 8;

/// Chunk header overhead: one `usize` for the size field.
const CHUNK_OVERHEAD: usize = core::mem::size_of::<usize>();

/// Minimum chunk size: header + forward pointer + backward pointer + prev_size footer.
/// This ensures free chunks have room for the doubly-linked list pointers.
const MIN_CHUNK_SIZE: usize = 4 * core::mem::size_of::<usize>();

/// Number of small bins.  Bin `i` holds chunks of exactly `(i + 2) * ALIGNMENT` bytes.
/// Covers chunk sizes from `MIN_CHUNK_SIZE` (32) to `(NSMALLBINS + 1) * ALIGNMENT` (264).
const NSMALLBINS: usize = 32;

/// Small bin threshold: chunks smaller than this use exact-size small bins.
const SMALLBIN_MAX: usize = (NSMALLBINS + 1) * ALIGNMENT;

/// Number of large (tree) bins.  Each covers a power-of-two range.
const NLARGEBINS: usize = 32;

/// Total number of bins.
const NBINS: usize = NSMALLBINS + NLARGEBINS;

/// Minimum heap growth via sbrk (128 KiB).
const MIN_SBRK_SIZE: usize = 128 * 1024;

// ---------------------------------------------------------------------------
// Chunk flags (stored in low bits of the size field)
// ---------------------------------------------------------------------------

/// The previous chunk (lower address) is in use.  When clear, the prev_size
/// field at the end of the previous chunk is valid and can be used to find
/// the previous chunk's header for coalescing.
const PREV_IN_USE: usize = 0x1;

/// This chunk is currently allocated.
const THIS_IN_USE: usize = 0x2;

/// Mask to extract the actual chunk size (clear flag bits).
const SIZE_MASK: usize = !(PREV_IN_USE | THIS_IN_USE);

// ---------------------------------------------------------------------------
// Chunk header
// ---------------------------------------------------------------------------

/// Header for a heap chunk.
///
/// Layout in memory:
/// ```text
/// [prev_size]   <- only valid when previous chunk is free
/// [size|flags]  <- chunk header (this pointer)
/// [payload...]  <- returned to caller (or fd/bk pointers when free)
/// ```
#[repr(C)]
struct Chunk {
    size: usize,
}

impl Chunk {
    #[inline]
    fn chunk_size(&self) -> usize {
        self.size & SIZE_MASK
    }

    #[inline]
    fn is_in_use(&self) -> bool {
        self.size & THIS_IN_USE != 0
    }

    #[inline]
    fn prev_in_use(&self) -> bool {
        self.size & PREV_IN_USE != 0
    }

    #[inline]
    fn payload(&self) -> *mut u8 {
        unsafe { (self as *const Self as *mut u8).add(CHUNK_OVERHEAD) }
    }

    #[inline]
    unsafe fn from_payload(ptr: *mut u8) -> *mut Chunk {
        unsafe { ptr.sub(CHUNK_OVERHEAD) as *mut Chunk }
    }

    #[inline]
    unsafe fn next_chunk(&self) -> *mut Chunk {
        unsafe {
            let base = self as *const Self as *const u8;
            base.add(self.chunk_size()) as *mut Chunk
        }
    }

    #[inline]
    unsafe fn prev_chunk(&self) -> *mut Chunk {
        unsafe {
            let base = self as *const Self as *const u8;
            let prev_size = self.prev_size();
            base.sub(prev_size) as *mut Chunk
        }
    }

    #[inline]
    unsafe fn prev_size(&self) -> usize {
        unsafe {
            let ptr = (self as *const Self as *const usize).sub(1);
            *ptr
        }
    }

    /// Write the size footer at the end of this (free) chunk.
    #[inline]
    unsafe fn set_footer(&self) {
        unsafe {
            let size = self.chunk_size();
            let footer = (self as *const Self as *mut usize)
                .add(size / core::mem::size_of::<usize>())
                .sub(1);
            *footer = size;
        }
    }

    #[inline]
    unsafe fn fd(&self) -> *mut Chunk {
        unsafe { *(self.payload() as *const *mut Chunk) }
    }

    #[inline]
    unsafe fn bk(&self) -> *mut Chunk {
        unsafe { *((self.payload() as *const *mut Chunk).add(1)) }
    }

    #[inline]
    unsafe fn set_fd(&mut self, fd: *mut Chunk) {
        unsafe { *(self.payload() as *mut *mut Chunk) = fd }
    }

    #[inline]
    unsafe fn set_bk(&mut self, bk: *mut Chunk) {
        unsafe { *((self.payload() as *mut *mut Chunk).add(1)) = bk }
    }
}

// ---------------------------------------------------------------------------
// Bin sentinel node
// ---------------------------------------------------------------------------

/// Sentinel node for a bin's doubly-linked free list.
///
/// When a bin is empty, `fd` and `bk` both point to the sentinel itself.
#[repr(C)]
struct BinHead {
    fd: *mut Chunk,
    bk: *mut Chunk,
}

impl BinHead {
    const fn empty() -> Self {
        Self {
            fd: ptr::null_mut(),
            bk: ptr::null_mut(),
        }
    }

    unsafe fn init(&mut self) {
        let self_as_chunk = self as *mut BinHead as *mut Chunk;
        self.fd = self_as_chunk;
        self.bk = self_as_chunk;
    }

    unsafe fn is_empty(&self) -> bool {
        let self_as_chunk = self as *const BinHead as *const Chunk;
        self.fd as *const Chunk == self_as_chunk
    }

    fn as_chunk_ptr(&mut self) -> *mut Chunk {
        self as *mut BinHead as *mut Chunk
    }
}

// ---------------------------------------------------------------------------
// Allocator state
// ---------------------------------------------------------------------------

struct DlMallocState {
    bins: [BinHead; NBINS],
    smallmap: u32,
    largemap: u32,
    /// The "top" (wilderness) chunk at the heap frontier.
    top: *mut Chunk,
    top_size: usize,
    initialized: bool,
}

impl DlMallocState {
    const fn new() -> Self {
        Self {
            bins: [const { BinHead::empty() }; NBINS],
            smallmap: 0,
            largemap: 0,
            top: ptr::null_mut(),
            top_size: 0,
            initialized: false,
        }
    }

    unsafe fn init(&mut self) {
        if self.initialized {
            return;
        }
        for bin in self.bins.iter_mut() {
            unsafe { bin.init() };
        }
        self.initialized = true;
    }

    // -----------------------------------------------------------------------
    // Bin index computation
    // -----------------------------------------------------------------------

    #[inline]
    fn small_index(size: usize) -> usize {
        (size / ALIGNMENT) - 2
    }

    #[inline]
    fn large_index(size: usize) -> usize {
        let mut idx = 0usize;
        let mut s = size >> 8;
        while s > 1 && idx < NLARGEBINS - 1 {
            s >>= 1;
            idx += 1;
        }
        idx
    }

    // -----------------------------------------------------------------------
    // Free list operations
    // -----------------------------------------------------------------------

    unsafe fn insert_chunk(&mut self, chunk: *mut Chunk) {
        unsafe {
            let size = (*chunk).chunk_size();

            if size < SMALLBIN_MAX {
                let idx = Self::small_index(size);
                let head = self.bins[idx].as_chunk_ptr();
                let old_fd = self.bins[idx].fd;
                (*chunk).set_fd(old_fd);
                (*chunk).set_bk(head);
                (*old_fd).set_bk(chunk);
                self.bins[idx].fd = chunk;
                self.smallmap |= 1 << idx;
            } else {
                let idx = NSMALLBINS + Self::large_index(size);
                let head = self.bins[idx].as_chunk_ptr();

                // Walk to find insertion point (sorted by size, smallest first).
                let mut cur = self.bins[idx].fd;
                while cur != head && (*cur).chunk_size() < size {
                    cur = (*cur).fd();
                }

                let prev = (*cur).bk();
                (*chunk).set_fd(cur);
                (*chunk).set_bk(prev);
                (*prev).set_fd(chunk);
                (*cur).set_bk(chunk);
                self.largemap |= 1 << (idx - NSMALLBINS);
            }
        }
    }

    unsafe fn unlink_chunk(&mut self, chunk: *mut Chunk) {
        unsafe {
            let fd = (*chunk).fd();
            let bk = (*chunk).bk();
            (*fd).set_bk(bk);
            (*bk).set_fd(fd);

            let size = (*chunk).chunk_size();
            if size < SMALLBIN_MAX {
                let idx = Self::small_index(size);
                if self.bins[idx].is_empty() {
                    self.smallmap &= !(1 << idx);
                }
            } else {
                let idx = NSMALLBINS + Self::large_index(size);
                if self.bins[idx].is_empty() {
                    self.largemap &= !(1 << (idx - NSMALLBINS));
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Coalescing
    // -----------------------------------------------------------------------

    unsafe fn coalesce(&mut self, chunk: *mut Chunk) -> (*mut Chunk, usize) {
        unsafe {
            let mut p = chunk;
            let mut size = (*p).chunk_size();

            // Coalesce with previous chunk (lower address).
            if !(*p).prev_in_use() {
                let prev = (*p).prev_chunk();
                let prev_size = (*prev).chunk_size();
                self.unlink_chunk(prev);
                p = prev;
                size += prev_size;
            }

            // Coalesce with next chunk (higher address).
            let next = (*p).next_chunk();
            if next != self.top && !(*next).is_in_use() {
                let next_size = (*next).chunk_size();
                self.unlink_chunk(next);
                size += next_size;
            } else if next == self.top {
                // Merge into the top chunk.
                size += self.top_size;
                self.top = p;
                self.top_size = size;
                (*p).size = size | PREV_IN_USE;
                return (p, size);
            }

            // Write the merged chunk header.
            let prev_flag = if p == chunk && (*chunk).prev_in_use() {
                PREV_IN_USE
            } else {
                0
            };
            (*p).size = size | prev_flag;
            (*p).set_footer();

            // Clear PREV_IN_USE in the next chunk's header.
            let next_after = (*p).next_chunk();
            if next_after != self.top {
                (*next_after).size &= !PREV_IN_USE;
            }

            (p, size)
        }
    }

    // -----------------------------------------------------------------------
    // Splitting
    // -----------------------------------------------------------------------

    unsafe fn use_chunk(&mut self, chunk: *mut Chunk, needed: usize) -> *mut u8 {
        unsafe {
            let size = (*chunk).chunk_size();
            let remainder = size - needed;

            if remainder >= MIN_CHUNK_SIZE {
                // Split: use the front, free the remainder.
                (*chunk).size = needed | THIS_IN_USE | ((*chunk).size & PREV_IN_USE);

                let rest = (chunk as *mut u8).add(needed) as *mut Chunk;
                (*rest).size = remainder | PREV_IN_USE;
                (*rest).set_footer();

                let after_rest = (*rest).next_chunk();
                if after_rest == self.top {
                    // Remainder becomes the new top chunk.
                    self.top = rest;
                    self.top_size = remainder;
                    (*rest).size = remainder | PREV_IN_USE;
                    return (*chunk).payload();
                }
                (*after_rest).size &= !PREV_IN_USE;
                self.insert_chunk(rest);
            } else {
                // Use the whole chunk.
                (*chunk).size |= THIS_IN_USE;
                let next = (*chunk).next_chunk();
                (*next).size |= PREV_IN_USE;
            }

            (*chunk).payload()
        }
    }

    // -----------------------------------------------------------------------
    // Heap growth
    // -----------------------------------------------------------------------

    unsafe fn grow_heap(&mut self, needed: usize) -> bool {
        let grow = needed.max(MIN_SBRK_SIZE);
        let base = crate::syscall::sys_sbrk(grow as isize);
        if base.is_null() {
            return false;
        }

        if self.top.is_null() {
            // First allocation: the whole sbrk region is the top chunk.
            let chunk = base as *mut Chunk;
            unsafe { (*chunk).size = grow | PREV_IN_USE };
            self.top = chunk;
            self.top_size = grow;
        } else {
            // Extend the existing top chunk.
            self.top_size += grow;
            unsafe {
                (*self.top).size = self.top_size | ((*self.top).size & PREV_IN_USE);
            }
        }

        true
    }

    // -----------------------------------------------------------------------
    // Core allocation
    // -----------------------------------------------------------------------

    unsafe fn malloc(&mut self, mut size: usize) -> *mut u8 {
        unsafe {
            self.init();

            // Compute the chunk size we need.
            size = size.max(core::mem::size_of::<usize>() * 2);
            size = (size + CHUNK_OVERHEAD + ALIGNMENT - 1) & !(ALIGNMENT - 1);
            if size < MIN_CHUNK_SIZE {
                size = MIN_CHUNK_SIZE;
            }

            // 1. Check small bins for exact fit.
            if size < SMALLBIN_MAX {
                let idx = Self::small_index(size);
                if self.smallmap & (1 << idx) != 0 {
                    let chunk = self.bins[idx].fd;
                    self.unlink_chunk(chunk);
                    return self.use_chunk(chunk, size);
                }

                // Check larger small bins.
                let map = self.smallmap & !((1 << (idx + 1)) - 1);
                if map != 0 {
                    let bit = map.trailing_zeros() as usize;
                    let chunk = self.bins[bit].fd;
                    self.unlink_chunk(chunk);
                    return self.use_chunk(chunk, size);
                }
            }

            // 2. Check large bins for best fit.
            if self.largemap != 0 {
                let start_idx = if size < SMALLBIN_MAX {
                    0
                } else {
                    Self::large_index(size)
                };
                let map = self.largemap & !((1 << start_idx) - 1);
                if map != 0 {
                    let bit = map.trailing_zeros() as usize;
                    let bin_idx = NSMALLBINS + bit;
                    let head = self.bins[bin_idx].as_chunk_ptr();

                    // Walk sorted list for best fit (first chunk >= size).
                    let mut cur = self.bins[bin_idx].fd;
                    while cur != head {
                        if (*cur).chunk_size() >= size {
                            self.unlink_chunk(cur);
                            return self.use_chunk(cur, size);
                        }
                        cur = (*cur).fd();
                    }

                    // Check next bins.
                    let map2 = self.largemap & !((1 << (bit + 1)) - 1);
                    if map2 != 0 {
                        let bit2 = map2.trailing_zeros() as usize;
                        let bin_idx2 = NSMALLBINS + bit2;
                        let chunk = self.bins[bin_idx2].fd;
                        self.unlink_chunk(chunk);
                        return self.use_chunk(chunk, size);
                    }
                }
            }

            // 3. Try the top chunk (wilderness).
            if !self.top.is_null() && self.top_size >= size + MIN_CHUNK_SIZE {
                return self.alloc_from_top(size);
            }

            // 4. Need more memory from the OS.
            if !self.grow_heap(size + MIN_CHUNK_SIZE) {
                return ptr::null_mut();
            }

            // Retry from top.
            if self.top_size >= size + MIN_CHUNK_SIZE {
                return self.alloc_from_top(size);
            }

            ptr::null_mut()
        }
    }

    /// Carve a chunk from the top (wilderness) chunk.
    unsafe fn alloc_from_top(&mut self, size: usize) -> *mut u8 {
        unsafe {
            let chunk = self.top;
            let old_top_size = self.top_size;

            (*chunk).size = size | THIS_IN_USE | PREV_IN_USE;

            let new_top = (chunk as *mut u8).add(size) as *mut Chunk;
            let new_top_size = old_top_size - size;
            (*new_top).size = new_top_size | PREV_IN_USE;
            self.top = new_top;
            self.top_size = new_top_size;

            (*chunk).payload()
        }
    }

    // -----------------------------------------------------------------------
    // Core deallocation
    // -----------------------------------------------------------------------

    unsafe fn free(&mut self, ptr: *mut u8) {
        if ptr.is_null() {
            return;
        }

        unsafe {
            let chunk = Chunk::from_payload(ptr);

            // Mark as free.
            (*chunk).size &= !THIS_IN_USE;
            (*chunk).set_footer();

            // Coalesce with neighbors.
            let (merged, _size) = self.coalesce(chunk);

            // If the merged chunk didn't become the top chunk, insert into a bin.
            if merged != self.top {
                self.insert_chunk(merged);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public allocator type
// ---------------------------------------------------------------------------

/// A dlmalloc-style allocator.
///
/// Thread-safety: uses `UnsafeCell` internally.  Currently single-threaded
/// only.  Add a spinlock wrapper for multi-threaded use.
pub struct DlMalloc {
    state: UnsafeCell<DlMallocState>,
}

// SAFETY: eithne user processes are currently single-threaded.
unsafe impl Send for DlMalloc {}
unsafe impl Sync for DlMalloc {}

impl DlMalloc {
    pub const fn new() -> Self {
        Self {
            state: UnsafeCell::new(DlMallocState::new()),
        }
    }
}

impl Allocator for DlMalloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let state = unsafe { &mut *self.state.get() };
        let size = layout.size().max(layout.align());
        unsafe { state.malloc(size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        let state = unsafe { &mut *self.state.get() };
        unsafe { state.free(ptr) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, _old_layout: Layout, new_size: usize) -> *mut u8 {
        let state = unsafe { &mut *self.state.get() };

        let chunk = unsafe { Chunk::from_payload(ptr) };
        let old_chunk_size = unsafe { (*chunk).chunk_size() };
        let old_usable = old_chunk_size - CHUNK_OVERHEAD;

        // If the new size fits in the existing chunk, keep it.
        let needed = (new_size + CHUNK_OVERHEAD + ALIGNMENT - 1) & !(ALIGNMENT - 1);
        let needed = needed.max(MIN_CHUNK_SIZE);
        if needed <= old_chunk_size {
            return ptr;
        }

        // Allocate new, copy, free old.
        let new_ptr = unsafe { state.malloc(new_size) };
        if !new_ptr.is_null() {
            let copy_size = old_usable.min(new_size);
            unsafe {
                ptr::copy_nonoverlapping(ptr, new_ptr, copy_size);
                state.free(ptr);
            }
        }
        new_ptr
    }
}
