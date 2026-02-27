//! IRQ handling for x86_64.
//!
//! Manages the 8259 PIC for hardware interrupt routing.

use super::idt::InterruptFrame;
use super::io::{inb, io_wait, outb};
use core::sync::atomic::{AtomicU64, Ordering};

// 8259 PIC I/O ports
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

/// Timer tick counter.
static TIMER_TICKS: AtomicU64 = AtomicU64::new(0);

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

        // Mask all except IRQ0 (timer)
        outb(PIC1_DATA, 0xFE); // bit 0 clear = IRQ0 unmasked
        io_wait();
        outb(PIC2_DATA, 0xFF); // all masked
        io_wait();
    }

    crate::println!("[pic] IRQ0 (timer) unmasked");
}

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

/// Called from the IDT dispatcher for vectors 32-47.
pub fn irq_handler(irq: u8, _frame: &InterruptFrame) {
    match irq {
        0 => {
            let ticks = TIMER_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
            if ticks.is_multiple_of(100) {
                crate::println!("[timer] {} ticks", ticks);
            }
            // Signal the scheduler; the actual switch happens in check_resched
            // at the isr_common exit path, after the full InterruptFrame is on
            // the stack and the Rust handler has returned.
            crate::sched::timer_tick();
        }
        1 => {
            let scancode = unsafe { inb(0x60) };
            crate::println!("[kbd] scancode: {:#x}", scancode);
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
