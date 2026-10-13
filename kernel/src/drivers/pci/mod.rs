//! PCI bus enumeration and configuration space access.
//!
//! Uses legacy I/O port mechanism (ports `0xCF8`/`0xCFC`). This works on all
//! x86_64 platforms including QEMU. ECAM/MMIO configuration is not needed.

extern crate alloc;
use alloc::vec::Vec;

use crate::sync::{Once, Spinlock};

/// I/O port for PCI configuration address register.
const CONFIG_ADDRESS: u16 = 0x0CF8;
/// I/O port for PCI configuration data register.
const CONFIG_DATA: u16 = 0x0CFC;

// PCI configuration space register offsets.
/// Vendor ID register (16-bit).
const PCI_VENDOR_ID: u8 = 0x00;
/// Device ID register (16-bit, at offset 2).
const PCI_DEVICE_ID: u8 = 0x02;
/// Revision / class / subclass / prog-if (32-bit).
const PCI_CLASS_REVISION: u8 = 0x08;
/// Header type register.
const PCI_HEADER_TYPE: u8 = 0x0E;
/// First BAR register.
const PCI_BAR0: u8 = 0x10;
/// Interrupt line register.
const PCI_INTERRUPT_LINE: u8 = 0x3C;
/// Multi-function bit in header type register.
const PCI_HEADER_MULTIFUNCTION: u8 = 0x80;
/// Mask to extract header type (bits 6:0).
const PCI_HEADER_TYPE_MASK: u8 = 0x7F;
/// Invalid vendor ID (device not present).
const PCI_VENDOR_NONE: u16 = 0xFFFF;

/// Location of a device on the PCI bus.
#[derive(Debug, Clone, Copy)]
pub struct PciAddress {
    /// Bus number (0-255).
    pub bus: u8,
    /// Device number (0-31).
    pub device: u8,
    /// Function number (0-7).
    pub function: u8,
}

/// Parsed PCI device information.
#[derive(Debug)]
pub struct PciDevice {
    /// Bus/device/function address.
    pub address: PciAddress,
    /// Vendor ID.
    pub vendor_id: u16,
    /// Device ID.
    pub device_id: u16,
    /// Class code.
    pub class_code: u8,
    /// Subclass code.
    pub subclass: u8,
    /// Programming interface byte.
    pub prog_if: u8,
    /// Header type (0 = standard, 1 = bridge, 2 = cardbus).
    pub header_type: u8,
    /// Raw BAR values (6 for type-0 headers).
    pub bars: [u32; 6],
    /// Interrupt line.
    pub interrupt_line: u8,
}

/// A parsed Base Address Register.
#[derive(Debug, Clone, Copy)]
pub enum Bar {
    /// Memory-mapped BAR.
    Memory {
        /// Physical base address.
        base_addr: u64,
        /// Region size in bytes.
        size: u64,
        /// Whether the region is prefetchable.
        prefetchable: bool,
    },
    /// I/O port BAR.
    Io {
        /// Port base address.
        base_addr: u32,
        /// Region size in bytes.
        size: u32,
    },
    /// BAR is not present or zero.
    None,
}

/// Build the PCI config address for a given bus/device/function/offset.
const fn config_addr(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    (1u32 << 31)
        | ((bus as u32) << 16)
        | ((device as u32) << 11)
        | ((function as u32) << 8)
        | ((offset as u32) & 0xFC)
}

/// Read a 32-bit value from PCI configuration space.
pub fn config_read32(bus: u8, device: u8, function: u8, offset: u8) -> u32 {
    let addr = config_addr(bus, device, function, offset);
    unsafe {
        crate::arch::x86_64::io::outl(CONFIG_ADDRESS, addr);
        crate::arch::x86_64::io::inl(CONFIG_DATA)
    }
}

/// Write a 32-bit value to PCI configuration space.
pub fn config_write32(bus: u8, device: u8, function: u8, offset: u8, value: u32) {
    let addr = config_addr(bus, device, function, offset);
    unsafe {
        crate::arch::x86_64::io::outl(CONFIG_ADDRESS, addr);
        crate::arch::x86_64::io::outl(CONFIG_DATA, value);
    }
}

/// Read a 16-bit value from PCI configuration space.
pub fn config_read16(bus: u8, device: u8, function: u8, offset: u8) -> u16 {
    let val32 = config_read32(bus, device, function, offset);
    ((val32 >> ((offset & 2) * 8)) & 0xFFFF) as u16
}

/// Read an 8-bit value from PCI configuration space.
pub fn config_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    let val32 = config_read32(bus, device, function, offset);
    ((val32 >> ((offset & 3) * 8)) & 0xFF) as u8
}

/// Parse a BAR from a PCI device, determining its type, base address, and size.
pub fn parse_bar(addr: PciAddress, bar_index: u8) -> Bar {
    let offset = 0x10 + bar_index * 4;
    let original = config_read32(addr.bus, addr.device, addr.function, offset);

    if original == 0 {
        return Bar::None;
    }

    // Write all-ones to determine size.
    config_write32(addr.bus, addr.device, addr.function, offset, 0xFFFF_FFFF);
    let size_mask = config_read32(addr.bus, addr.device, addr.function, offset);
    // Restore original value.
    config_write32(addr.bus, addr.device, addr.function, offset, original);

    if size_mask == 0 {
        return Bar::None;
    }

    if original & 1 != 0 {
        // I/O BAR
        let base = original & !0x3;
        let size = !(size_mask & !0x3).wrapping_add(1);
        Bar::Io {
            base_addr: base,
            size,
        }
    } else {
        // Memory BAR
        let prefetchable = original & 0x8 != 0;
        let is_64bit = (original >> 1) & 0x3 == 2;
        let base_low = (original & !0xF) as u64;

        let (base_addr, size) = if is_64bit {
            let offset_hi = 0x10 + (bar_index + 1) * 4;
            let original_hi = config_read32(addr.bus, addr.device, addr.function, offset_hi);
            config_write32(
                addr.bus,
                addr.device,
                addr.function,
                offset_hi,
                0xFFFF_FFFF,
            );
            let size_hi = config_read32(addr.bus, addr.device, addr.function, offset_hi);
            config_write32(addr.bus, addr.device, addr.function, offset_hi, original_hi);

            let base = base_low | ((original_hi as u64) << 32);
            let combined_mask =
                ((size_mask & !0xF) as u64) | ((size_hi as u64) << 32);
            let sz = (!combined_mask).wrapping_add(1);
            (base, sz)
        } else {
            let sz = (!(size_mask & !0xF) as u64).wrapping_add(1) & 0xFFFF_FFFF;
            (base_low, sz)
        };

        Bar::Memory {
            base_addr,
            size,
            prefetchable,
        }
    }
}

/// Scan all PCI buses and return discovered devices.
fn enumerate() -> Vec<PciDevice> {
    let mut devices = Vec::new();

    for bus in 0u16..=255 {
        for device in 0u8..32 {
            let vendor = config_read16(bus as u8, device, 0, PCI_VENDOR_ID);
            if vendor == PCI_VENDOR_NONE {
                continue;
            }
            let header_type = config_read8(bus as u8, device, 0, PCI_HEADER_TYPE);
            let max_func = if header_type & PCI_HEADER_MULTIFUNCTION != 0 { 8 } else { 1 };

            for function in 0..max_func {
                let vendor = config_read16(bus as u8, device, function, PCI_VENDOR_ID);
                if vendor == PCI_VENDOR_NONE {
                    continue;
                }
                let device_id = config_read16(bus as u8, device, function, PCI_DEVICE_ID);
                let class_sub = config_read32(bus as u8, device, function, PCI_CLASS_REVISION);
                let class_code = (class_sub >> 24) as u8;
                let subclass = (class_sub >> 16) as u8;
                let prog_if = (class_sub >> 8) as u8;
                let ht = config_read8(bus as u8, device, function, PCI_HEADER_TYPE) & PCI_HEADER_TYPE_MASK;
                let interrupt_line = config_read8(bus as u8, device, function, PCI_INTERRUPT_LINE);

                let mut bars = [0u32; 6];
                if ht == 0 {
                    let mut i = 0;
                    while i < 6 {
                        let offset = PCI_BAR0 + (i as u8) * 4;
                        bars[i] = config_read32(bus as u8, device, function, offset);
                        // Skip the high half of 64-bit BARs.
                        if bars[i] & 1 == 0 && (bars[i] >> 1) & 3 == 2 {
                            i += 1;
                            if i < 6 {
                                bars[i] = config_read32(
                                    bus as u8,
                                    device,
                                    function,
                                    PCI_BAR0 + (i as u8) * 4,
                                );
                            }
                        }
                        i += 1;
                    }
                }

                devices.push(PciDevice {
                    address: PciAddress {
                        bus: bus as u8,
                        device,
                        function,
                    },
                    vendor_id: vendor,
                    device_id,
                    class_code,
                    subclass,
                    prog_if,
                    header_type: ht,
                    bars,
                    interrupt_line,
                });
            }
        }
    }
    devices
}

/// Walk the PCI capability list, calling `f` for each capability.
///
/// `f` receives `(cap_id, cap_offset)` and returns `true` to stop iteration.
pub fn walk_capabilities(
    addr: PciAddress,
    mut f: impl FnMut(u8, u8) -> bool,
) {
    // Check if capabilities list is present (Status register bit 4).
    let status = config_read16(addr.bus, addr.device, addr.function, 0x06);
    if status & (1 << 4) == 0 {
        return;
    }

    let mut offset = config_read8(addr.bus, addr.device, addr.function, 0x34) & !0x3;
    while offset != 0 {
        let cap_id = config_read8(addr.bus, addr.device, addr.function, offset);
        if f(cap_id, offset) {
            return;
        }
        offset = config_read8(addr.bus, addr.device, addr.function, offset + 1) & !0x3;
    }
}

static PCI_DEVICES: Once<Spinlock<Vec<PciDevice>>> = Once::new();

/// Initialize PCI bus enumeration. Call once during boot.
pub fn init() {
    let _ = PCI_DEVICES.call_once(|| {
        let devices = enumerate();
        for dev in &devices {
            crate::println!(
                "[pci] {:02x}:{:02x}.{} vendor={:#06x} device={:#06x} class={:02x}:{:02x}",
                dev.address.bus,
                dev.address.device,
                dev.address.function,
                dev.vendor_id,
                dev.device_id,
                dev.class_code,
                dev.subclass,
            );
        }
        crate::println!("[pci] {} device(s) found", devices.len());
        Spinlock::new(devices)
    });
}

/// Find the first PCI device matching a vendor/device ID pair.
pub fn find_device(vendor_id: u16, device_id: u16) -> Option<PciAddress> {
    let devices = PCI_DEVICES.get()?.lock();
    for dev in &*devices {
        if dev.vendor_id == vendor_id && dev.device_id == device_id {
            return Some(dev.address);
        }
    }
    None
}

/// Return the highest physical address used by any memory BAR.
///
/// This is used to determine how much physical address space the direct map
/// must cover so that MMIO regions are accessible via `phys_to_virt`.
pub fn max_bar_address() -> u64 {
    let Some(lock) = PCI_DEVICES.get() else {
        return 0;
    };
    let devices = lock.lock();
    let mut max = 0u64;
    for dev in &*devices {
        let mut i = 0;
        while i < 6 {
            let bar = dev.bars[i];
            if bar == 0 || bar & 1 != 0 {
                // Empty or I/O BAR — skip.
                i += 1;
                continue;
            }
            let is_64bit = (bar >> 1) & 3 == 2;
            let base_low = u64::from(bar & !0xF);
            if is_64bit && i + 1 < 6 {
                let base = base_low | (u64::from(dev.bars[i + 1]) << 32);
                // Round up generously — BAR regions are at least a page.
                max = max.max(base.saturating_add(0x10_0000));
                i += 2;
            } else {
                max = max.max(base_low.saturating_add(0x10_0000));
                i += 1;
            }
        }
    }
    max
}

/// Find the first PCI device matching a vendor ID with device ID in a range.
pub fn find_device_range(vendor_id: u16, dev_lo: u16, dev_hi: u16) -> Option<PciAddress> {
    let devices = PCI_DEVICES.get()?.lock();
    for dev in &*devices {
        if dev.vendor_id == vendor_id && dev.device_id >= dev_lo && dev.device_id <= dev_hi {
            return Some(dev.address);
        }
    }
    None
}
