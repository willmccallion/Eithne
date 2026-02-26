//! Sleeping mutex (stub).
//!
//! Unlike [`Spinlock`](super::Spinlock), this mutex will put the calling
//! thread to sleep when contended — but it requires a working scheduler.
//!
//! Until the scheduler is implemented, use `Spinlock` instead.

pub struct Mutex<T: ?Sized> {
    _data: T,
}

// TODO: implement once sched::Scheduler is available
// - Lock should call sched::yield_cpu() in a loop instead of spinning
// - Unlock should wake one waiting thread
