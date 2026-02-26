/// PL011 UART driver for AArch64 (stub).
///
/// The PL011 is the standard UART on ARM platforms (QEMU virt machine uses it).
/// Base address on QEMU virt: 0x0900_0000.

pub fn write_str(s: &str) {
    // TODO: implement PL011 MMIO writes
    let _ = s;
}
