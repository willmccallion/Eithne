pub mod context;
pub mod process;
pub mod round_robin;
pub mod thread;
pub mod traits;
pub mod user;

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

/// Spawn a user-mode process from an ELF binary.
#[cfg(target_arch = "x86_64")]
pub fn spawn_user_from_elf(elf_data: &[u8]) -> Option<TaskId> {
    let sched = SCHEDULER.get()?;
    let mut guard = sched.lock();
    let id = guard.alloc_id();
    let task = user::create_user_task_from_elf(id, "user_init", elf_data)?;
    crate::println!("[sched] spawned user task {:?} '{}'", id, task.name);
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
        Some((new_rsp, kstack_top, cr3)) => {
            activate_task(kstack_top, cr3);
            new_rsp
        }
        None => 0,
    }
}

/// Mark the current task as Dead and yield.
///
/// Called from `sys_exit`.  The scheduler will never run the task again.
/// This function does not return.
pub fn exit_current() -> ! {
    {
        let sched = SCHEDULER.get().expect("scheduler not init");
        sched.lock().kill_current();
    }
    yield_cpu();
    loop {
        crate::arch::halt_once();
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
        activate_task(p.new_kstack_top, p.new_cr3);
        unsafe { yield_now(p.old_rsp_ptr, p.new_rsp) };
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Activate the incoming task: switch address space and update SYSCALL_KSTACK_TOP.
///
/// Must be called with the scheduler lock dropped (address space activation
/// touches CR3; SYSCALL_KSTACK_TOP write is a plain store).
#[inline]
fn activate_task(kstack_top: u64, cr3: u64) {
    // Update SYSCALL_KSTACK_TOP so that the next SYSCALL instruction from
    // user mode lands on this task's kernel stack.
    crate::arch::set_syscall_stack(kstack_top);

    // Switch page tables if this task has its own address space.
    if cr3 != 0 {
        unsafe {
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack));
        }
    }
}
