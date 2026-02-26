/// RISC-V UART driver (stub).
///
/// QEMU virt machine provides a 16550-compatible UART at 0x1000_0000.

pub fn write_str(s: &str) {
    // TODO: implement 16550 MMIO writes
    let _ = s;
}
