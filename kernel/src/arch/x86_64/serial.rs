//! COM1 serial port driver (0x3F8).
//!
//! Provides early debug output and, once interrupts are enabled, RX input.

use super::io::{inb, outb};

// ---------------------------------------------------------------------------
// Port base and register offsets
// ---------------------------------------------------------------------------

const COM1: u16 = 0x3F8;

/// Offsets from the COM port base address.
const REG_DATA: u16 = 0;    // RBR (read) / THR (write) — also DLL when DLAB=1
const REG_IER: u16 = 1;     // Interrupt Enable Register — also DLH when DLAB=1
const REG_FCR: u16 = 2;     // FIFO Control Register (write)
const REG_LCR: u16 = 3;     // Line Control Register
const REG_MCR: u16 = 4;     // Modem Control Register
const REG_LSR: u16 = 5;     // Line Status Register

// ---------------------------------------------------------------------------
// Line Status Register bits
// ---------------------------------------------------------------------------

const LSR_DATA_READY: u8 = 1 << 0;
const LSR_THR_EMPTY: u8 = 1 << 5;

// ---------------------------------------------------------------------------
// Interrupt Enable Register bits
// ---------------------------------------------------------------------------

const IER_RX_AVAILABLE: u8 = 1 << 0;

// ---------------------------------------------------------------------------
// Line Control Register bits
// ---------------------------------------------------------------------------

const LCR_DLAB: u8 = 0x80;   // Divisor Latch Access Bit
const LCR_8N1: u8 = 0x03;    // 8 data bits, no parity, 1 stop bit

// ---------------------------------------------------------------------------
// FIFO Control Register bits
// ---------------------------------------------------------------------------

const FCR_ENABLE_FIFO: u8 = 0x01;
const FCR_CLEAR_RX: u8 = 0x02;
const FCR_CLEAR_TX: u8 = 0x04;
const FCR_TRIGGER_14: u8 = 0xC0; // 14-byte trigger level

// ---------------------------------------------------------------------------
// Modem Control Register bits
// ---------------------------------------------------------------------------

const MCR_DTR: u8 = 0x01;
const MCR_RTS: u8 = 0x02;
const MCR_OUT2: u8 = 0x08;   // required for PIC-routed interrupts

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialize COM1 serial port at 115200 baud, 8N1.
pub fn init() {
    unsafe {
        // Disable all interrupts during setup.
        outb(COM1 + REG_IER, 0x00);
        // Enable DLAB to set baud rate divisor.
        outb(COM1 + REG_LCR, LCR_DLAB);
        // Divisor = 1 → 115200 baud (low byte, then high byte).
        outb(COM1 + REG_DATA, 0x01);
        outb(COM1 + REG_IER, 0x00);
        // 8 bits, no parity, one stop bit (clears DLAB).
        outb(COM1 + REG_LCR, LCR_8N1);
        // Enable FIFO, clear both FIFOs, 14-byte trigger threshold.
        outb(COM1 + REG_FCR, FCR_ENABLE_FIFO | FCR_CLEAR_RX | FCR_CLEAR_TX | FCR_TRIGGER_14);
        // Assert DTR, RTS, and OUT2 (OUT2 gates PIC interrupts).
        outb(COM1 + REG_MCR, MCR_DTR | MCR_RTS | MCR_OUT2);
    }
}

/// Write a single byte to COM1, blocking until the transmit buffer is empty.
fn write_byte(byte: u8) {
    unsafe {
        while (inb(COM1 + REG_LSR) & LSR_THR_EMPTY) == 0 {
            core::hint::spin_loop();
        }
        outb(COM1 + REG_DATA, byte);
    }
}

/// Write a string to COM1, inserting `\r` before each `\n`.
pub fn write_str(s: &str) {
    for byte in s.bytes() {
        if byte == b'\n' {
            write_byte(b'\r');
        }
        write_byte(byte);
    }
}

/// Enable the COM1 Received Data Available interrupt.
///
/// Must be called after the PIC and IDT are initialized, since the RX
/// interrupt fires on IRQ4 (vector 36).
pub fn enable_rx_interrupt() {
    unsafe {
        let ier = inb(COM1 + REG_IER);
        outb(COM1 + REG_IER, ier | IER_RX_AVAILABLE);
    }
}

/// Read a byte from COM1 if data is available.
///
/// Checks LSR Data Ready bit and returns the byte from the receive
/// buffer, or `None` if no data is pending.
pub fn read_byte() -> Option<u8> {
    unsafe {
        if inb(COM1 + REG_LSR) & LSR_DATA_READY != 0 {
            Some(inb(COM1 + REG_DATA))
        } else {
            None
        }
    }
}
