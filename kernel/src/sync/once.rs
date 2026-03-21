use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU8, Ordering};

const UNINIT: u8 = 0;
const RUNNING: u8 = 1;
const COMPLETE: u8 = 2;

/// A synchronization primitive for one-time initialization.
pub struct Once<T> {
    state: AtomicU8,
    data: UnsafeCell<Option<T>>,
}

unsafe impl<T: Send + Sync> Sync for Once<T> {}
unsafe impl<T: Send> Send for Once<T> {}

impl<T> Once<T> {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(UNINIT),
            data: UnsafeCell::new(None),
        }
    }

    /// Initialize the value exactly once. Subsequent calls return the existing value.
    pub fn call_once(&self, f: impl FnOnce() -> T) -> &T {
        if self.state.load(Ordering::Acquire) == COMPLETE {
            return unsafe { (*self.data.get()).as_ref().unwrap_unchecked() };
        }

        if self
            .state
            .compare_exchange(UNINIT, RUNNING, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            unsafe {
                *self.data.get() = Some(f());
            }
            self.state.store(COMPLETE, Ordering::Release);
        } else {
            // Another thread is initializing — spin until complete.
            while self.state.load(Ordering::Acquire) != COMPLETE {
                core::hint::spin_loop();
            }
        }
        unsafe { (*self.data.get()).as_ref().unwrap_unchecked() }
    }

    /// Get the value if initialized, None otherwise.
    pub fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::Acquire) == COMPLETE {
            unsafe { (*self.data.get()).as_ref() }
        } else {
            None
        }
    }
}
