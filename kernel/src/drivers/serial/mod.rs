//! Serial console input buffer.
//!
//! Provides a kernel-global ring buffer that the COM1 RX interrupt handler
//! fills and `sys_read` drains.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::sync::{IrqSpinlock, Once};

const BUF_SIZE: usize = 256;

struct InputBuffer {
    buf: [u8; BUF_SIZE],
    head: usize, // next write position
    tail: usize, // next read position
    count: usize,
}

impl InputBuffer {
    const fn new() -> Self {
        Self {
            buf: [0; BUF_SIZE],
            head: 0,
            tail: 0,
            count: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        if self.count >= BUF_SIZE {
            return; // drop if full
        }
        self.buf[self.head] = byte;
        self.head = (self.head + 1) % BUF_SIZE;
        self.count += 1;
    }

    fn pop(&mut self) -> Option<u8> {
        if self.count == 0 {
            return None;
        }
        let byte = self.buf[self.tail];
        self.tail = (self.tail + 1) % BUF_SIZE;
        self.count -= 1;
        Some(byte)
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }
}

static SERIAL_INPUT: Once<IrqSpinlock<InputBuffer>> = Once::new();

/// Task ID of the task blocked waiting for serial input, or 0 if none.
/// The IRQ handler checks this to know which specific task to wake.
pub static SERIAL_READER_TID: AtomicU64 = AtomicU64::new(0);

pub fn init() {
    SERIAL_INPUT.call_once(|| IrqSpinlock::new(InputBuffer::new()));
}

/// ASCII ETX — sent by the terminal when the user presses Ctrl+C.
const CTRL_C: u8 = 0x03;

/// Push a byte into the input buffer. Called from the IRQ handler and
/// the PS/2 keyboard driver.
///
/// If the byte is Ctrl+C (0x03), sends SIGINT to the task currently
/// blocked on serial read instead of buffering the byte.
pub fn push_byte(byte: u8) {
    if byte == CTRL_C {
        let tid = SERIAL_READER_TID.load(Ordering::Acquire);
        if tid != 0 {
            crate::sched::send_signal_to(
                crate::sched::traits::TaskId(tid),
                crate::ipc::signal::SIGINT,
            );
        }
        return;
    }
    if let Some(buf) = SERIAL_INPUT.get() {
        buf.lock().push(byte);
    }
}

/// Read up to `max` bytes from the input buffer into `out`.
/// Returns the number of bytes read (0 if empty).
pub fn read(out: &mut [u8], max: usize) -> usize {
    let buf = match SERIAL_INPUT.get() {
        Some(b) => b,
        None => return 0,
    };
    let mut guard = buf.lock();
    let mut count = 0;
    let limit = max.min(out.len());
    while count < limit {
        match guard.pop() {
            Some(byte) => {
                // Use write_volatile to ensure the store reaches memory and isn't
                // elided by the optimiser.  The destination may be a user-space
                // buffer whose physical page is only reachable through the current
                // page table; a plain store could theoretically be reordered or
                // eliminated.
                unsafe { core::ptr::write_volatile(&mut out[count], byte) };
                count += 1;
            }
            None => break,
        }
    }
    count
}

/// Check if the input buffer is empty.
pub fn is_empty() -> bool {
    match SERIAL_INPUT.get() {
        Some(buf) => buf.lock().is_empty(),
        None => true,
    }
}
