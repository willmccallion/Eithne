//! Block device subsystem.
//!
//! Currently contains:
//! - Virtio block device driver (for QEMU)

pub mod virtio_blk;
pub mod virtqueue;

/// Initialize all block device drivers.
pub fn init() {
    virtio_blk::init();
}
