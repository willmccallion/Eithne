pub mod context;
pub mod process;
pub mod round_robin;
pub mod thread;
pub mod traits;

pub use traits::{Scheduler, TaskId};

use crate::sync::{IrqSpinlock, Once};
use context::switch_context;
use process::Task;
use round_robin::RoundRobinScheduler;

// ---------------------------------------------------------------------------
// Global scheduler instance
// ---------------------------------------------------------------------------

static SCHEDULER: Once<IrqSpinlock<RoundRobinScheduler>> = Once::new();

/// Initialize the global scheduler.  Must be called after `mm::init()`.
pub fn init() {
    SCHEDULER.call_once(|| IrqSpinlock::new(RoundRobinScheduler::new()));
    crate::println!("[sched] round-robin scheduler initialized");
}

/// Spawn a kernel thread with the given name and entry function.
pub fn spawn(name: &'static str, entry_fn: fn() -> !) -> Option<TaskId> {
    let sched = SCHEDULER.get()?;
    let mut guard = sched.lock();
    let id = guard.alloc_id();
    let task = Task::new(id, name, entry_fn)?;
    crate::println!("[sched] spawned task {:?} '{}'", id, name);
    guard.add_task(task);
    Some(id)
}

/// Perform a context switch to the next runnable task.
///
/// Locks the scheduler only long enough to update queue state, then drops
/// the lock **before** calling `switch_context` so IRQs are re-enabled and
/// the incoming task can immediately acquire the lock if needed.
fn do_switch() {
    let pending = {
        // Lock scope: update state, get RSP pair, drop lock.
        let sched = match SCHEDULER.get() {
            Some(s) => s,
            None => return,
        };
        sched.lock().prepare_switch()
    };

    if let Some(p) = pending {
        // Lock is released here; interrupts are re-enabled by IrqSpinlock drop.
        unsafe { switch_context(p.old_rsp_ptr, p.new_rsp) };
    }
}

/// Voluntarily yield the CPU to the next runnable task.
pub fn yield_cpu() {
    do_switch();
}
