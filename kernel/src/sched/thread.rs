//! Kernel thread helpers.
//!
//! High-level wrappers around `sched::spawn` for common kernel background tasks.

/// Spawn a named kernel thread.
///
/// This is a thin alias for `crate::sched::spawn`.  Use it from modules that
/// want to start background work without importing the full scheduler.
pub fn spawn_kernel_thread(
    name: &'static str,
    entry_fn: fn() -> !,
) -> Option<crate::sched::TaskId> {
    crate::sched::spawn(name, entry_fn)
}
