pub mod context;
pub mod process;
pub mod round_robin;
pub mod thread;
pub mod traits;

pub use traits::{Scheduler, TaskId};

use core::sync::atomic::{AtomicBool, Ordering};

use crate::sync::{IrqSpinlock, Once};
use context::yield_now;
use process::Task;
use round_robin::RoundRobinScheduler;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static SCHEDULER: Once<IrqSpinlock<RoundRobinScheduler>> = Once::new();

/// Set by the timer IRQ when the current timeslice expires.
/// Cleared and acted on by `check_resched` at the `isr_common` exit.
pub static NEED_RESCHED: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn init() {
    SCHEDULER.call_once(|| IrqSpinlock::new(RoundRobinScheduler::new()));
    crate::println!("[sched] round-robin scheduler initialized");
}

pub fn spawn(name: &'static str, entry_fn: fn() -> !) -> Option<TaskId> {
    let sched = SCHEDULER.get()?;
    let mut guard = sched.lock();
    let id = guard.alloc_id();
    let task = Task::new(id, name, entry_fn)?;
    crate::println!("[sched] spawned task {:?} '{}'", id, name);
    guard.add_task(task);
    Some(id)
}

/// Called from the timer IRQ handler each tick.
pub fn timer_tick() {
    let sched = match SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };
    if sched.lock().tick_check() {
        NEED_RESCHED.store(true, Ordering::Release);
    }
}

/// Called from the `isr_common` exit path (before `iretq`).
///
/// `interrupted_rsp` points at the `InterruptFrame` on the current kernel stack.
///
/// Returns the new task's RSP if a switch should happen, or 0 for no switch.
/// `isr_common` loads the returned value into RSP and falls into the
/// pop-registers + `iretq` sequence, resuming the new task.
#[unsafe(no_mangle)]
pub extern "C" fn check_resched(interrupted_rsp: u64) -> u64 {
    if !NEED_RESCHED.swap(false, Ordering::AcqRel) {
        return 0;
    }
    let sched = match SCHEDULER.get() {
        Some(s) => s,
        None => return 0,
    };
    match sched.lock().prepare_switch_preempt(interrupted_rsp) {
        Some((new_rsp, _)) => new_rsp,
        None => 0,
    }
}

/// Cooperatively yield the CPU.
///
/// Builds a synthetic `InterruptFrame` on the current stack, saves it as the
/// current task's RSP, then resumes the next task via `iretq`.
pub fn yield_cpu() {
    let pending = {
        let sched = match SCHEDULER.get() {
            Some(s) => s,
            None => return,
        };
        sched.lock().prepare_switch()
    };

    if let Some(p) = pending {
        // yield_now saves our InterruptFrame into *old_rsp_ptr, loads new_rsp,
        // and resumes the next task.  When we are scheduled back, execution
        // returns from yield_now (at the .Lyield_resume ret instruction).
        unsafe { yield_now(p.old_rsp_ptr, p.new_rsp) };
    }
}
