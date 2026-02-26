//! x86_64 port-mapped I/O helpers.

/// Write a byte to an I/O port.
#[inline]
pub unsafe fn outb(port: u16, val: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") val,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Read a byte from an I/O port.
#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") port,
            out("al") val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

/// Small delay for I/O operations (writes to port 0x80).
#[inline]
pub unsafe fn io_wait() {
    unsafe { outb(0x80, 0) }
}
