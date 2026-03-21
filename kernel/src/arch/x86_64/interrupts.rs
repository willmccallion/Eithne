//! IRQ handling for x86_64.
//!
//! Manages the 8259 PIC for hardware interrupt routing.

use super::idt::InterruptFrame;
use super::io::{inb, io_wait, outb};
use core::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// 8259 PIC I/O ports
// ---------------------------------------------------------------------------

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

const ICW1_INIT: u8 = 0x10;
const ICW1_ICW4: u8 = 0x01;
const ICW4_8086: u8 = 0x01;
const PIC_EOI: u8 = 0x20;

/// PIC remapping offsets.
pub const PIC1_OFFSET: u8 = 32;
pub const PIC2_OFFSET: u8 = 40;

// ---------------------------------------------------------------------------
// IRQ line numbers (master PIC = 0-7, slave PIC = 8-15)
// ---------------------------------------------------------------------------

const IRQ_TIMER: u8 = 0;
const IRQ_KEYBOARD: u8 = 1;
const IRQ_COM1: u8 = 4;

/// PS/2 keyboard data port.
const PS2_DATA_PORT: u16 = 0x60;

/// Timer tick counter.
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// PIC initialization
// ---------------------------------------------------------------------------

/// Initialize and remap the 8259 PIC to vectors 32-47.
pub fn init_pic() {
    crate::println!(
        "[pic] remapping to vectors {}-{}",
        PIC1_OFFSET,
        PIC2_OFFSET + 7
    );

    unsafe {
        // ICW1: start init sequence
        outb(PIC1_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();
        outb(PIC2_CMD, ICW1_INIT | ICW1_ICW4);
        io_wait();

        // ICW2: vector offsets
        outb(PIC1_DATA, PIC1_OFFSET);
        io_wait();
        outb(PIC2_DATA, PIC2_OFFSET);
        io_wait();

        // ICW3: master/slave wiring
        outb(PIC1_DATA, 0x04); // slave on IRQ2
        io_wait();
        outb(PIC2_DATA, 0x02); // cascade identity = 2
        io_wait();

        // ICW4: 8086 mode
        outb(PIC1_DATA, ICW4_8086);
        io_wait();
        outb(PIC2_DATA, ICW4_8086);
        io_wait();

        // Build initial IRQ mask: unmask timer, keyboard, and COM1.
        let mut mask: u8 = 0xFF;
        mask &= !(1 << IRQ_TIMER);
        mask &= !(1 << IRQ_KEYBOARD);
        mask &= !(1 << IRQ_COM1);
        outb(PIC1_DATA, mask);
        io_wait();
        outb(PIC2_DATA, 0xFF); // slave: all masked
        io_wait();
    }

    crate::println!(
        "[pic] IRQ{} (timer), IRQ{} (kbd), IRQ{} (COM1) unmasked",
        IRQ_TIMER,
        IRQ_KEYBOARD,
        IRQ_COM1
    );
}

// ---------------------------------------------------------------------------
// EOI and masking
// ---------------------------------------------------------------------------

/// Send End-of-Interrupt to the PIC(s).
pub fn send_eoi(irq: u8) {
    unsafe {
        if irq >= 8 {
            outb(PIC2_CMD, PIC_EOI);
        }
        outb(PIC1_CMD, PIC_EOI);
    }
}

/// Unmask an IRQ line (0-15).
pub fn unmask_irq(irq: u8) {
    unsafe {
        if irq < 8 {
            let mask = inb(PIC1_DATA);
            outb(PIC1_DATA, mask & !(1 << irq));
        } else {
            let mask = inb(PIC2_DATA);
            outb(PIC2_DATA, mask & !(1 << (irq - 8)));
            // Ensure cascade (IRQ2) is unmasked on master
            let m = inb(PIC1_DATA);
            outb(PIC1_DATA, m & !(1 << 2));
        }
    }
}

/// Mask an IRQ line (0-15).
pub fn mask_irq(irq: u8) {
    unsafe {
        if irq < 8 {
            let mask = inb(PIC1_DATA);
            outb(PIC1_DATA, mask | (1 << irq));
        } else {
            let mask = inb(PIC2_DATA);
            outb(PIC2_DATA, mask | (1 << (irq - 8)));
        }
    }
}

// ---------------------------------------------------------------------------
// IRQ dispatch
// ---------------------------------------------------------------------------

/// Called from the IDT dispatcher for vectors 32-47.
pub fn irq_handler(irq: u8, _frame: &InterruptFrame) {
    match irq {
        IRQ_TIMER => {
            let _ = TIMER_TICKS.fetch_add(1, Ordering::Relaxed);
            // Signal the scheduler; the actual switch happens in check_resched
            // at the isr_common exit path, after the full InterruptFrame is on
            // the stack and the Rust handler has returned.
            crate::sched::timer_tick();
        }
        IRQ_KEYBOARD => {
            let scancode = unsafe { inb(PS2_DATA_PORT) };
            crate::drivers::kbd::handle_scancode(scancode);
        }
        IRQ_COM1 => {
            // Drain the COM1 FIFO into the serial input buffer.
            while let Some(byte) = super::serial::read_byte() {
                crate::drivers::serial::push_byte(byte);
            }
            // Wake the blocked serial reader if one is waiting.
            let tid = crate::drivers::serial::SERIAL_READER_TID.swap(0, Ordering::AcqRel);
            if tid != 0 {
                crate::sched::wake_serial_reader(tid);
            }
        }
        _ => {
            crate::println!("[irq] unhandled IRQ {}", irq);
        }
    }

    send_eoi(irq);
}

/// Return the current timer tick count.
pub fn timer_ticks() -> u64 {
    TIMER_TICKS.load(Ordering::Relaxed)
}
