//! Virtio block device driver (virtio spec 5.2).
//!
//! Implements a synchronous (polling) virtio-blk driver over a split virtqueue.
//! Discovers the device via PCI, negotiates features, and provides
//! `read_block`/`write_block` through the `BlockDevice` trait.

use core::ptr;
use core::sync::atomic::{fence, Ordering};

use crate::drivers::pci;
use crate::drivers::traits::{BlockDevice, Driver, IoError};
use crate::mm::addr::{phys_to_virt, PhysAddr, VirtAddr};
use crate::mm::frame;
use crate::sync::{Once, Spinlock};

use super::virtqueue::{SplitVirtqueue, VirtqBuf};

/// Red Hat's virtio vendor ID.
const VIRTIO_VENDOR_ID: u16 = 0x1AF4;
/// Transitional virtio-blk device ID.
const VIRTIO_BLK_DEVICE_ID_LEGACY: u16 = 0x1001;
/// Modern virtio-blk device ID.
const VIRTIO_BLK_DEVICE_ID_MODERN: u16 = 0x1042;

/// Virtio PCI capability ID (vendor-specific).
const PCI_CAP_VENDOR: u8 = 0x09;

/// Virtio PCI capability types.
const VIRTIO_PCI_CAP_COMMON_CFG: u8 = 1;
const VIRTIO_PCI_CAP_NOTIFY_CFG: u8 = 2;
const VIRTIO_PCI_CAP_ISR_CFG: u8 = 3;
const VIRTIO_PCI_CAP_DEVICE_CFG: u8 = 4;

/// Virtio device status bits.
const VIRTIO_STATUS_ACKNOWLEDGE: u8 = 1;
const VIRTIO_STATUS_DRIVER: u8 = 2;
const VIRTIO_STATUS_DRIVER_OK: u8 = 4;
const VIRTIO_STATUS_FEATURES_OK: u8 = 8;

/// Virtio feature bit: device conforms to virtio 1.x spec.
const VIRTIO_F_VERSION_1: u64 = 1 << 32;

/// Block request types.
const VIRTIO_BLK_T_IN: u32 = 0;
const VIRTIO_BLK_T_OUT: u32 = 1;

/// Sector size in bytes.
const SECTOR_SIZE: usize = 512;

/// Max poll iterations before timeout.
const POLL_TIMEOUT: u64 = 100_000_000;

/// Layout of the virtio PCI common configuration MMIO region.
#[repr(C)]
struct VirtioPciCommonCfg {
    device_feature_select: u32,
    device_feature: u32,
    driver_feature_select: u32,
    driver_feature: u32,
    msix_config: u16,
    num_queues: u16,
    device_status: u8,
    config_generation: u8,
    queue_select: u16,
    queue_size: u16,
    queue_msix_vector: u16,
    queue_enable: u16,
    queue_notify_off: u16,
    queue_desc: u64,
    queue_avail: u64,
    queue_used: u64,
}

/// Block device configuration (spec 5.2.4).
#[repr(C)]
struct VirtioBlkConfig {
    /// Number of 512-byte sectors.
    capacity: u64,
}

/// Block request header submitted to the device.
#[repr(C)]
struct VirtioBlkReqHeader {
    type_: u32,
    reserved: u32,
    sector: u64,
}

/// Parsed virtio PCI capability pointers.
struct VirtioPciCaps {
    /// MMIO pointer to common cfg.
    common_cfg: *mut VirtioPciCommonCfg,
    /// MMIO pointer to notification area base.
    notify_base: *mut u8,
    /// Multiplier for per-queue notification offset.
    notify_off_multiplier: u32,
    /// MMIO pointer to ISR status.
    isr_cfg: *mut u8,
    /// MMIO pointer to device-specific config.
    device_cfg: *mut u8,
}

/// Resolve a virtio PCI capability to a virtual MMIO address.
fn resolve_cap_addr(addr: pci::PciAddress, cap_offset: u8) -> (*mut u8, u32) {
    let bar_idx = pci::config_read8(addr.bus, addr.device, addr.function, cap_offset + 4);
    let bar = pci::parse_bar(addr, bar_idx);
    let pci::Bar::Memory { base_addr: bar_base, .. } = bar else {
        return (ptr::null_mut(), 0);
    };

    let cap_bar_offset =
        pci::config_read32(addr.bus, addr.device, addr.function, cap_offset + 8) as u64;
    let cap_length =
        pci::config_read32(addr.bus, addr.device, addr.function, cap_offset + 12);

    let phys = PhysAddr::new(bar_base + cap_bar_offset);
    let virt = phys_to_virt(phys);
    (virt.as_mut_ptr(), cap_length)
}

/// Discover all virtio PCI capabilities for a device.
fn discover_caps(addr: pci::PciAddress) -> Option<VirtioPciCaps> {
    let mut caps = VirtioPciCaps {
        common_cfg: ptr::null_mut(),
        notify_base: ptr::null_mut(),
        notify_off_multiplier: 0,
        isr_cfg: ptr::null_mut(),
        device_cfg: ptr::null_mut(),
    };

    pci::walk_capabilities(addr, |cap_id, cap_offset| {
        if cap_id != PCI_CAP_VENDOR {
            return false;
        }

        let cfg_type =
            pci::config_read8(addr.bus, addr.device, addr.function, cap_offset + 3);

        match cfg_type {
            VIRTIO_PCI_CAP_COMMON_CFG => {
                let (ptr, _) = resolve_cap_addr(addr, cap_offset);
                caps.common_cfg = ptr.cast();
            }
            VIRTIO_PCI_CAP_NOTIFY_CFG => {
                let (ptr, _) = resolve_cap_addr(addr, cap_offset);
                caps.notify_base = ptr;
                // notify_off_multiplier is at capability offset + 16.
                caps.notify_off_multiplier = pci::config_read32(
                    addr.bus,
                    addr.device,
                    addr.function,
                    cap_offset + 16,
                );
            }
            VIRTIO_PCI_CAP_ISR_CFG => {
                let (ptr, _) = resolve_cap_addr(addr, cap_offset);
                caps.isr_cfg = ptr;
            }
            VIRTIO_PCI_CAP_DEVICE_CFG => {
                let (ptr, _) = resolve_cap_addr(addr, cap_offset);
                caps.device_cfg = ptr;
            }
            _ => {}
        }
        false // keep scanning
    });

    if caps.common_cfg.is_null() || caps.notify_base.is_null() || caps.device_cfg.is_null() {
        return None;
    }

    Some(caps)
}

/// A virtio block device.
struct VirtioBlkInner {
    /// The request virtqueue.
    vq: SplitVirtqueue,
    /// MMIO pointer to common cfg.
    common_cfg: *mut VirtioPciCommonCfg,
    /// MMIO pointer to notification area.
    notify_base: *mut u8,
    /// Notify offset multiplier.
    notify_off_multiplier: u32,
    /// Per-queue notify offset.
    queue_notify_off: u16,
    /// Disk capacity in sectors.
    capacity: u64,
    /// Bounce buffer (one page) for DMA transfers.
    bounce_phys: PhysAddr,
    /// Virtual address of the bounce buffer.
    bounce_virt: VirtAddr,
    /// Request header buffer (physical).
    hdr_phys: PhysAddr,
    /// Request header buffer (virtual).
    hdr_virt: VirtAddr,
}

// SAFETY: All internal pointers are MMIO or kernel-allocated pages. Access is
// serialized via the outer Spinlock.
unsafe impl Send for VirtioBlkInner {}

/// Public virtio block device handle.
pub struct VirtioBlkDevice {
    inner: Spinlock<VirtioBlkInner>,
}

// SAFETY: Access serialized via Spinlock.
unsafe impl Sync for VirtioBlkDevice {}

impl VirtioBlkDevice {
    /// Write a value to the common cfg MMIO region.
    fn write_status(cfg: *mut VirtioPciCommonCfg, status: u8) {
        unsafe {
            ptr::write_volatile(&mut (*cfg).device_status, status);
        }
    }

    /// Read the current device status.
    fn read_status(cfg: *mut VirtioPciCommonCfg) -> u8 {
        unsafe { ptr::read_volatile(&(*cfg).device_status) }
    }

    /// Notify the device that new buffers are available on a queue.
    fn notify(inner: &VirtioBlkInner) {
        let offset = u32::from(inner.queue_notify_off) * inner.notify_off_multiplier;
        // SAFETY: notify_base is an MMIO region; offset is device-provided and aligned.
        #[allow(clippy::cast_ptr_alignment)]
        unsafe {
            let doorbell = inner.notify_base.add(offset as usize).cast::<u16>();
            ptr::write_volatile(doorbell, 0); // queue index 0
        }
    }

    /// Perform a block I/O operation (read or write) using polling.
    fn do_io(
        inner: &mut VirtioBlkInner,
        lba: u64,
        buf: &mut [u8],
        is_write: bool,
    ) -> Result<(), IoError> {
        if buf.len() != SECTOR_SIZE {
            return Err(IoError::BufferTooSmall);
        }
        if lba >= inner.capacity {
            return Err(IoError::InvalidOperation);
        }

        // Set up request header.
        let hdr = VirtioBlkReqHeader {
            type_: if is_write {
                VIRTIO_BLK_T_OUT
            } else {
                VIRTIO_BLK_T_IN
            },
            reserved: 0,
            sector: lba,
        };
        // Write header fields individually for volatile-correctness.
        let hdr_ptr = inner.hdr_virt.as_mut_ptr::<VirtioBlkReqHeader>();
        unsafe {
            ptr::write_volatile(&mut (*hdr_ptr).type_, hdr.type_);
            ptr::write_volatile(&mut (*hdr_ptr).reserved, hdr.reserved);
            ptr::write_volatile(&mut (*hdr_ptr).sector, hdr.sector);
        }

        // For writes, copy data to bounce buffer.
        if is_write {
            unsafe {
                ptr::copy_nonoverlapping(
                    buf.as_ptr(),
                    inner.bounce_virt.as_mut_ptr(),
                    SECTOR_SIZE,
                );
            }
        }

        // Status byte at the end of the bounce buffer page.
        let status_phys = inner.bounce_phys.offset(SECTOR_SIZE as u64);
        let status_virt = inner.bounce_virt.offset(SECTOR_SIZE as u64);
        // Set status to 0xFF so we can detect completion.
        unsafe {
            ptr::write_volatile(status_virt.as_mut_ptr::<u8>(), 0xFF);
        }

        // Build 3-descriptor chain: header, data, status.
        let bufs = [
            VirtqBuf {
                phys: inner.hdr_phys.as_u64(),
                len: core::mem::size_of::<VirtioBlkReqHeader>() as u32,
                device_writable: false,
            },
            VirtqBuf {
                phys: inner.bounce_phys.as_u64(),
                len: SECTOR_SIZE as u32,
                device_writable: !is_write,
            },
            VirtqBuf {
                phys: status_phys.as_u64(),
                len: 1,
                device_writable: true,
            },
        ];

        let Some(_head) = inner.vq.push(&bufs) else {
            return Err(IoError::DeviceNotReady);
        };

        // Notify the device.
        Self::notify(inner);

        // Poll for completion.
        let mut polls = 0u64;
        loop {
            if let Some((_id, _len)) = inner.vq.pop_used() {
                break;
            }
            polls += 1;
            if polls >= POLL_TIMEOUT {
                return Err(IoError::Timeout);
            }
            core::hint::spin_loop();
        }

        // Check status byte.
        let status = unsafe { ptr::read_volatile(status_virt.as_ptr::<u8>()) };
        if status != 0 {
            return Err(IoError::HardwareError);
        }

        // For reads, copy data from bounce buffer.
        if !is_write {
            unsafe {
                ptr::copy_nonoverlapping(
                    inner.bounce_virt.as_ptr::<u8>(),
                    buf.as_mut_ptr(),
                    SECTOR_SIZE,
                );
            }
        }

        Ok(())
    }
}

impl Driver for VirtioBlkDevice {
    fn name(&self) -> &'static str {
        "virtio-blk"
    }

    fn init(&mut self) -> Result<(), IoError> {
        Ok(()) // Already initialized during probe.
    }

    fn reset(&mut self) -> Result<(), IoError> {
        let inner = self.inner.lock();
        Self::write_status(inner.common_cfg, 0);
        Ok(())
    }
}

impl BlockDevice for VirtioBlkDevice {
    fn read_block(&self, lba: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let mut inner = self.inner.lock();
        Self::do_io(&mut inner, lba, buf, false)
    }

    fn write_block(&self, lba: u64, buf: &[u8]) -> Result<(), IoError> {
        let mut inner = self.inner.lock();
        // do_io takes &mut [u8] for both read/write; for writes we copy in anyway.
        // Use a stack copy to satisfy the signature.
        let mut tmp = [0u8; SECTOR_SIZE];
        let len = buf.len().min(SECTOR_SIZE);
        tmp[..len].copy_from_slice(&buf[..len]);
        Self::do_io(&mut inner, lba, &mut tmp, true)
    }

    fn block_size(&self) -> usize {
        SECTOR_SIZE
    }

    fn block_count(&self) -> u64 {
        self.inner.lock().capacity
    }
}

impl VirtioBlkDevice {
    /// Probe and initialize a virtio-blk PCI device.
    pub fn probe() -> Option<Self> {
        // Find a virtio-blk device on the PCI bus.
        let addr = pci::find_device(VIRTIO_VENDOR_ID, VIRTIO_BLK_DEVICE_ID_LEGACY)
            .or_else(|| pci::find_device(VIRTIO_VENDOR_ID, VIRTIO_BLK_DEVICE_ID_MODERN))?;

        crate::println!(
            "[virtio-blk] found device at {:02x}:{:02x}.{}",
            addr.bus,
            addr.device,
            addr.function,
        );

        // Enable PCI bus mastering (bit 2 of Command register).
        let cmd = pci::config_read16(addr.bus, addr.device, addr.function, 0x04);
        pci::config_write32(
            addr.bus,
            addr.device,
            addr.function,
            0x04,
            u32::from(cmd | 0x06), // bus master + memory space enable
        );

        // Discover virtio capabilities.
        let caps = discover_caps(addr)?;
        let cfg = caps.common_cfg;

        // 1. Reset.
        Self::write_status(cfg, 0);
        while Self::read_status(cfg) != 0 {
            core::hint::spin_loop();
        }

        // 2. Acknowledge.
        Self::write_status(cfg, VIRTIO_STATUS_ACKNOWLEDGE);

        // 3. Driver.
        Self::write_status(cfg, VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

        // 4. Feature negotiation.
        // Read device features (select page 0 and 1).
        unsafe {
            ptr::write_volatile(&mut (*cfg).device_feature_select, 0);
        }
        fence(Ordering::SeqCst);
        let _feat_lo = unsafe { ptr::read_volatile(&(*cfg).device_feature) };

        unsafe {
            ptr::write_volatile(&mut (*cfg).device_feature_select, 1);
        }
        fence(Ordering::SeqCst);
        let _feat_hi = unsafe { ptr::read_volatile(&(*cfg).device_feature) };

        // Accept VIRTIO_F_VERSION_1 (bit 32 = page 1 bit 0).
        unsafe {
            ptr::write_volatile(&mut (*cfg).driver_feature_select, 0);
            ptr::write_volatile(&mut (*cfg).driver_feature, 0);
            ptr::write_volatile(&mut (*cfg).driver_feature_select, 1);
            ptr::write_volatile(&mut (*cfg).driver_feature, 1); // bit 0 of page 1 = bit 32
        }

        // 5. Features OK.
        Self::write_status(
            cfg,
            VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER | VIRTIO_STATUS_FEATURES_OK,
        );
        fence(Ordering::SeqCst);

        if Self::read_status(cfg) & VIRTIO_STATUS_FEATURES_OK == 0 {
            crate::println!("[virtio-blk] device refused features");
            return None;
        }

        // 6. Read device config.
        // SAFETY: device_cfg points to an MMIO region with natural alignment.
        #[allow(clippy::cast_ptr_alignment)]
        let dev_cfg = caps.device_cfg.cast_const().cast::<VirtioBlkConfig>();
        let capacity = unsafe { ptr::read_volatile(&(*dev_cfg).capacity) };
        crate::println!(
            "[virtio-blk] capacity: {} sectors ({} MiB)",
            capacity,
            capacity * SECTOR_SIZE as u64 / (1024 * 1024),
        );

        // 7. Set up virtqueue 0 (requestq).
        unsafe {
            ptr::write_volatile(&mut (*cfg).queue_select, 0);
        }
        fence(Ordering::SeqCst);

        let max_queue_size = unsafe { ptr::read_volatile(&(*cfg).queue_size) };
        let queue_size = max_queue_size.min(256); // Cap at 256 for sanity.

        let vq = SplitVirtqueue::new(queue_size)?;

        unsafe {
            ptr::write_volatile(&mut (*cfg).queue_size, queue_size);
            ptr::write_volatile(&mut (*cfg).queue_desc, vq.desc_phys());
            ptr::write_volatile(&mut (*cfg).queue_avail, vq.avail_phys());
            ptr::write_volatile(&mut (*cfg).queue_used, vq.used_phys());
            ptr::write_volatile(&mut (*cfg).queue_enable, 1);
        }

        let queue_notify_off = unsafe { ptr::read_volatile(&(*cfg).queue_notify_off) };

        // Allocate bounce buffer (1 page) and request header buffer.
        let bounce_phys = frame::alloc_frame()?;
        let bounce_virt = phys_to_virt(bounce_phys);
        let hdr_phys = frame::alloc_frame()?;
        let hdr_virt = phys_to_virt(hdr_phys);

        // 8. Driver OK.
        Self::write_status(
            cfg,
            VIRTIO_STATUS_ACKNOWLEDGE
                | VIRTIO_STATUS_DRIVER
                | VIRTIO_STATUS_FEATURES_OK
                | VIRTIO_STATUS_DRIVER_OK,
        );

        crate::println!("[virtio-blk] device initialized, queue_size={}", queue_size);

        Some(Self {
            inner: Spinlock::new(VirtioBlkInner {
                vq,
                common_cfg: cfg,
                notify_base: caps.notify_base,
                notify_off_multiplier: caps.notify_off_multiplier,
                queue_notify_off,
                capacity,
                bounce_phys,
                bounce_virt,
                hdr_phys,
                hdr_virt,
            }),
        })
    }
}

static VIRTIO_BLK: Once<VirtioBlkDevice> = Once::new();

/// Initialize the virtio block device. Call after `pci::init()`.
pub fn init() {
    if let Some(dev) = VirtioBlkDevice::probe() {
        let _ = VIRTIO_BLK.call_once(|| dev);
    } else {
        crate::println!("[virtio-blk] no device found");
    }
}

/// Read a block from the virtio block device.
pub fn read_block(lba: u64, buf: &mut [u8]) -> Result<(), IoError> {
    let dev = VIRTIO_BLK.get().ok_or(IoError::DeviceNotReady)?;
    dev.read_block(lba, buf)
}

/// Write a block to the virtio block device.
pub fn write_block(lba: u64, buf: &[u8]) -> Result<(), IoError> {
    let dev = VIRTIO_BLK.get().ok_or(IoError::DeviceNotReady)?;
    dev.write_block(lba, buf)
}

/// Get the capacity in sectors, or 0 if no device.
pub fn capacity() -> u64 {
    VIRTIO_BLK
        .get()
        .map_or(0, |dev| dev.inner.lock().capacity)
}
