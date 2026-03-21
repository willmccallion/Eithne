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

/// Write a 16-bit value to an I/O port.
#[inline]
pub unsafe fn outw(port: u16, val: u16) {
    unsafe {
        core::arch::asm!(
            "out dx, ax",
            in("dx") port,
            in("ax") val,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Read a 16-bit value from an I/O port.
#[inline]
pub unsafe fn inw(port: u16) -> u16 {
    let val: u16;
    unsafe {
        core::arch::asm!(
            "in ax, dx",
            in("dx") port,
            out("ax") val,
            options(nomem, nostack, preserves_flags),
        );
    }
    val
}

/// Write a 32-bit value to an I/O port.
#[inline]
pub unsafe fn outl(port: u16, val: u32) {
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") port,
            in("eax") val,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Read a 32-bit value from an I/O port.
#[inline]
pub unsafe fn inl(port: u16) -> u32 {
    let val: u32;
    unsafe {
        core::arch::asm!(
            "in eax, dx",
            in("dx") port,
            out("eax") val,
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
