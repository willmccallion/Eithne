use super::addr::PhysAddr;
use super::traits::FrameAllocator;

/// Bitmap-based physical frame allocator (stub).
///
/// Tracks free/used frames using a bitmap where each bit represents
/// one 4 KiB physical frame.
pub struct BitmapFrameAllocator {
    // TODO:
    // - bitmap: &'static mut [u8]
    // - total_frames: usize
    // - free_frames: usize
    // - next_search: usize (optimization: start searching from last allocation)
}

impl BitmapFrameAllocator {
    /// Initialize from a UEFI memory map.
    ///
    /// Marks all usable memory regions as free and reserved regions as used.
    pub fn init_from_memory_map() -> Self {
        // TODO: parse UEFI memory map, set up bitmap
        Self {}
    }
}

impl FrameAllocator for BitmapFrameAllocator {
    fn alloc_frame(&mut self) -> Option<PhysAddr> {
        // TODO: scan bitmap for a free bit, mark it used, return address
        None
    }

    fn dealloc_frame(&mut self, _frame: PhysAddr) {
        // TODO: clear the corresponding bit in the bitmap
    }

    fn free_frame_count(&self) -> usize {
        // TODO: count zero bits in bitmap
        0
    }
}
