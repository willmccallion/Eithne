//! IRQ-safe spinlock.
//!
//! Identical to `Spinlock` except that it disables hardware interrupts while
//! the lock is held and restores the previous interrupt state on drop.
//! Use this for any data that is accessed from both thread and interrupt context.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// Read RFLAGS and clear the interrupt-enable bit (CLI), returning the old value.
#[inline]
fn irq_save() -> u64 {
    let rflags: u64;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {rflags}",
            "cli",
            rflags = out(reg) rflags,
            options(nomem),
        );
    }
    rflags
}

/// Restore the interrupt-enable bit from a previously saved RFLAGS value.
#[inline]
fn irq_restore(rflags: u64) {
    // Only re-enable interrupts if they were enabled before we disabled them.
    if rflags & (1 << 9) != 0 {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
    }
}

pub struct IrqSpinlock<T: ?Sized> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Sync for IrqSpinlock<T> {}
unsafe impl<T: ?Sized + Send> Send for IrqSpinlock<T> {}

impl<T> IrqSpinlock<T> {
    pub const fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn lock(&self) -> IrqSpinlockGuard<'_, T> {
        let saved = irq_save();
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.locked.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }
        IrqSpinlockGuard {
            lock: self,
            saved_rflags: saved,
        }
    }
}

pub struct IrqSpinlockGuard<'a, T: ?Sized> {
    lock: &'a IrqSpinlock<T>,
    saved_rflags: u64,
}

impl<T: ?Sized> Deref for IrqSpinlockGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for IrqSpinlockGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for IrqSpinlockGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        irq_restore(self.saved_rflags);
    }
}
