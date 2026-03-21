//! Split virtqueue implementation (virtio spec 2.7).
//!
//! A split virtqueue consists of three physically-contiguous regions:
//! the descriptor table, the available ring, and the used ring.
//! This is the simplest virtqueue format and is supported by all virtio devices.

use core::ptr;
use core::sync::atomic::{fence, Ordering};

use crate::mm::addr::{phys_to_virt, virt_to_phys, PhysAddr, VirtAddr};
use crate::mm::frame;
use crate::mm::page::{pages_to_order, PAGE_SIZE};

// ---------------------------------------------------------------------------
// Descriptor and ring structures (virtio spec 2.7.5 - 2.7.8)
// ---------------------------------------------------------------------------

/// Virtqueue descriptor (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VirtqDesc {
    /// Physical address of the buffer.
    pub addr: u64,
    /// Length of the buffer in bytes.
    pub len: u32,
    /// Descriptor flags.
    pub flags: u16,
    /// Index of the next descriptor in the chain.
    pub next: u16,
}

/// Descriptor flag: buffer continues via the `next` field.
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Descriptor flag: buffer is device-writable (otherwise device-readable).
pub const VIRTQ_DESC_F_WRITE: u16 = 2;

/// Available ring header.
#[repr(C)]
struct VirtqAvail {
    flags: u16,
    idx: u16,
    // Followed by ring[queue_size]: u16
}

/// Used ring element.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct VirtqUsedElem {
    /// Index of the start of the used descriptor chain.
    id: u32,
    /// Total bytes written into the descriptor chain.
    len: u32,
}

/// Used ring header.
#[repr(C)]
struct VirtqUsed {
    flags: u16,
    idx: u16,
    // Followed by ring[queue_size]: VirtqUsedElem
}

// ---------------------------------------------------------------------------
// Split virtqueue
// ---------------------------------------------------------------------------

/// A split virtqueue with descriptor table, available ring, and used ring.
///
/// # Safety
///
/// The raw pointers inside are kernel-allocated DMA pages. Access is
/// serialized by the caller (wrapped in a `Spinlock`).
pub struct SplitVirtqueue {
    /// Number of descriptors in the queue.
    size: u16,
    /// Virtual pointer to the descriptor table.
    desc: *mut VirtqDesc,
    /// Virtual pointer to the available ring.
    avail: *mut VirtqAvail,
    /// Virtual pointer to the used ring.
    used: *mut VirtqUsed,
    /// Physical address of the descriptor table.
    desc_phys: u64,
    /// Physical address of the available ring.
    avail_phys: u64,
    /// Physical address of the used ring.
    used_phys: u64,
    /// Head of the free descriptor list.
    free_head: u16,
    /// Number of free descriptors remaining.
    num_free: u16,
    /// Last-seen used ring index (for detecting new completions).
    last_used_idx: u16,
}

// SAFETY: All internal pointers are kernel-allocated DMA pages. Access is
// serialized by the caller (wrapped in a Spinlock).
unsafe impl Send for SplitVirtqueue {}

/// A buffer segment for a descriptor chain.
pub struct VirtqBuf {
    /// Physical address of the buffer.
    pub phys: u64,
    /// Length in bytes.
    pub len: u32,
    /// If true, the device may write to this buffer.
    pub device_writable: bool,
}

impl SplitVirtqueue {
    /// Allocate and initialize a split virtqueue of the given size.
    ///
    /// Returns `None` if memory allocation fails.
    pub fn new(queue_size: u16) -> Option<Self> {
        let size = queue_size as usize;

        // Calculate sizes for each region.
        let desc_size = size * 16; // sizeof(VirtqDesc) = 16
        let avail_size = 6 + 2 * size; // flags(2) + idx(2) + ring(2*size) + used_event(2)
        let used_size = 6 + 8 * size; // flags(2) + idx(2) + ring(8*size) + avail_event(2)

        // Virtio spec alignment: available ring must be at desc_size aligned to 2,
        // used ring must be at (desc+avail) aligned to 4. We allocate them in
        // separate page-aligned regions for simplicity.
        let desc_pages = desc_size.div_ceil(PAGE_SIZE);
        let avail_pages = avail_size.div_ceil(PAGE_SIZE);
        let used_pages = used_size.div_ceil(PAGE_SIZE);

        let desc_phys = frame::alloc_pages(pages_to_order(desc_pages))?;
        let avail_phys = frame::alloc_pages(pages_to_order(avail_pages))?;
        let used_phys = frame::alloc_pages(pages_to_order(used_pages))?;

        let desc_virt = phys_to_virt(desc_phys);
        let avail_virt = phys_to_virt(avail_phys);
        let used_virt = phys_to_virt(used_phys);

        // Zero all memory.
        unsafe {
            ptr::write_bytes(desc_virt.as_mut_ptr::<u8>(), 0, desc_pages * PAGE_SIZE);
            ptr::write_bytes(avail_virt.as_mut_ptr::<u8>(), 0, avail_pages * PAGE_SIZE);
            ptr::write_bytes(used_virt.as_mut_ptr::<u8>(), 0, used_pages * PAGE_SIZE);
        }

        // Initialize free descriptor chain: each descriptor's `next` points to i+1.
        let desc_ptr: *mut VirtqDesc = desc_virt.as_mut_ptr();
        for i in 0..queue_size {
            unsafe {
                let d = &mut *desc_ptr.add(i as usize);
                d.next = if i + 1 < queue_size { i + 1 } else { 0xFFFF };
            }
        }

        Some(Self {
            size: queue_size,
            desc: desc_ptr,
            avail: avail_virt.as_mut_ptr(),
            used: used_virt.as_mut_ptr(),
            desc_phys: desc_phys.as_u64(),
            avail_phys: avail_phys.as_u64(),
            used_phys: used_phys.as_u64(),
            free_head: 0,
            num_free: queue_size,
            last_used_idx: 0,
        })
    }

    /// Physical address of the descriptor table.
    pub const fn desc_phys(&self) -> u64 {
        self.desc_phys
    }

    /// Physical address of the available ring.
    pub const fn avail_phys(&self) -> u64 {
        self.avail_phys
    }

    /// Physical address of the used ring.
    pub const fn used_phys(&self) -> u64 {
        self.used_phys
    }

    /// Push a descriptor chain onto the available ring.
    ///
    /// `bufs` describes each segment of the chain. Returns the head descriptor
    /// index (for matching against used ring completions), or `None` if there
    /// aren't enough free descriptors.
    pub fn push(&mut self, bufs: &[VirtqBuf]) -> Option<u16> {
        if bufs.is_empty() || self.num_free < bufs.len() as u16 {
            return None;
        }

        let head = self.free_head;

        for (i, buf) in bufs.iter().enumerate() {
            let idx = if i == 0 { head } else { self.free_head };
            let desc = unsafe { &mut *self.desc.add(idx as usize) };

            desc.addr = buf.phys;
            desc.len = buf.len;
            desc.flags = if buf.device_writable {
                VIRTQ_DESC_F_WRITE
            } else {
                0
            };

            if i + 1 < bufs.len() {
                desc.flags |= VIRTQ_DESC_F_NEXT;
                desc.next = unsafe { (*self.desc.add(idx as usize)).next };
                // Advance free_head to next free descriptor.
                self.free_head = desc.next;
            } else {
                // Last in chain: consume this descriptor from free list.
                let next_free = desc.next;
                desc.next = 0;
                desc.flags &= !VIRTQ_DESC_F_NEXT;
                self.free_head = next_free;
            }

            self.num_free -= 1;
        }

        // Add head to the available ring.
        let avail_idx = unsafe { ptr::read_volatile(&(*self.avail).idx) };
        let ring_idx = (avail_idx % self.size) as usize;
        // SAFETY: avail ring is page-aligned; offset 4 + ring_idx*2 is u16-aligned.
        #[allow(clippy::cast_ptr_alignment)]
        let ring_entry =
            unsafe { (self.avail as *mut u8).add(4 + ring_idx * 2) as *mut u16 };
        unsafe {
            ptr::write_volatile(ring_entry, head);
        }

        // Memory barrier: ensure descriptor writes are visible before we update idx.
        fence(Ordering::Release);

        unsafe {
            ptr::write_volatile(&mut (*self.avail).idx, avail_idx.wrapping_add(1));
        }

        Some(head)
    }

    /// Pop a completed request from the used ring.
    ///
    /// Returns `Some((head_desc_id, bytes_written))` if a completion is
    /// available, `None` otherwise. Reclaims all descriptors in the chain.
    pub fn pop_used(&mut self) -> Option<(u16, u32)> {
        fence(Ordering::Acquire);

        let used_idx = unsafe { ptr::read_volatile(&(*self.used).idx) };
        if self.last_used_idx == used_idx {
            return None;
        }

        let ring_idx = (self.last_used_idx % self.size) as usize;
        // SAFETY: used ring is page-aligned; offset 4 + ring_idx*8 is u64-aligned.
        #[allow(clippy::cast_ptr_alignment)]
        let elem_ptr =
            unsafe { (self.used as *mut u8).add(4 + ring_idx * 8) as *mut VirtqUsedElem };
        // Read fields individually for volatile-correctness.
        let elem_id = unsafe { ptr::read_volatile(&(*elem_ptr).id) };
        let elem_len = unsafe { ptr::read_volatile(&(*elem_ptr).len) };

        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        // Reclaim descriptors in the chain.
        let mut idx = elem_id as u16;
        loop {
            let desc = unsafe { &*self.desc.add(idx as usize) };
            let has_next = desc.flags & VIRTQ_DESC_F_NEXT != 0;
            let next = desc.next;

            // Push back onto free list.
            unsafe {
                (*self.desc.add(idx as usize)).next = self.free_head;
            }
            self.free_head = idx;
            self.num_free += 1;

            if has_next {
                idx = next;
            } else {
                break;
            }
        }

        Some((elem_id as u16, elem_len))
    }
}
