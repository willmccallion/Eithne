//! PCI bus enumeration and configuration (stub).
//!
//! PCI is the primary bus for discovering hardware on x86_64.
//! On ARM/RISC-V, PCIe (or device tree) serves a similar role.

// TODO: PCI configuration space read/write (x86: I/O ports 0xCF8/0xCFC)
// TODO: Bus enumeration: scan all buses/devices/functions
// TODO: Vendor/device ID matching → driver binding
// TODO: BAR (Base Address Register) parsing
// TODO: MSI/MSI-X interrupt setup
