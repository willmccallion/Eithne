/// COM1 serial port (0x3F8) for early debug output.
const COM1: u16 = 0x3F8;

/// Initialize COM1 serial port.
pub fn init() {
    unsafe {
        // Disable interrupts
        outb(COM1 + 1, 0x00);
        // Enable DLAB (set baud rate divisor)
        outb(COM1 + 3, 0x80);
        // Set divisor to 1 (115200 baud)
        outb(COM1, 0x01);
        outb(COM1 + 1, 0x00);
        // 8 bits, no parity, one stop bit
        outb(COM1 + 3, 0x03);
        // Enable FIFO, clear, 14-byte threshold
        outb(COM1 + 2, 0xC7);
        // IRQs enabled, RTS/DSR set
        outb(COM1 + 4, 0x0B);
    }
}

/// Write a single byte to COM1.
fn write_byte(byte: u8) {
    unsafe {
        // Wait for transmit buffer to be empty
        while (inb(COM1 + 5) & 0x20) == 0 {
            core::hint::spin_loop();
        }
        outb(COM1, byte);
    }
}

/// Write a string to COM1.
pub fn write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            write_byte(b'\r');
        }
        write_byte(byte);
    }
}

#[inline]
unsafe fn outb(port: u16, val: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") val,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
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
