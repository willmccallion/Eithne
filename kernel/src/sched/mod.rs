pub mod context;
pub mod fd;
pub mod process;
pub mod round_robin;
pub mod thread;
pub mod traits;
pub mod user;

pub use fd::FdTable;
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

/// Run a closure with mutable access to the current task's file descriptor table.
///
/// The scheduler lock is held for the duration of the closure. The closure
/// must NOT call back into the scheduler (no blocking, yielding, etc.).
pub fn with_current_fd_table<F, R>(f: F) -> Result<R, crate::syscall::traits::SyscallError>
where
    F: FnOnce(&mut FdTable) -> Result<R, crate::syscall::traits::SyscallError>,
{
    let sched = SCHEDULER
        .get()
        .ok_or(crate::syscall::traits::SyscallError::IoError)?;
    let mut guard = sched.lock();
    let fdt = guard
        .current_fd_table()
        .ok_or(crate::syscall::traits::SyscallError::InvalidArgument)?;
    f(fdt)
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

/// Mark the current task as Blocked and yield the CPU, but only if
/// `should_block` returns true when called with interrupts disabled.
///
/// Used by blocking syscalls (e.g. `sys_read`) when there is no data
/// available. The task will be woken by `wake_reader()`.
///
/// The condition check + block + switch are all done under the scheduler's
/// IRQ-disabling lock. Because interrupts are disabled while the lock is
/// held, no IRQ can sneak in between the condition check and the state
/// change — this closes the race where an IRQ pushes data and calls
/// `wake_reader()` before the task is actually marked Blocked.
pub fn block_current_if(should_block: impl FnOnce() -> bool) {
    let pending = {
        let sched = match SCHEDULER.get() {
            Some(s) => s,
            None => return,
        };
        let mut guard = sched.lock();
        // Re-check with interrupts disabled: if data arrived between the
        // caller's check and us acquiring the lock, don't block.
        if !should_block() {
            return;
        }
        guard.block_current();
        guard.prepare_switch()
    };

    if let Some(p) = pending {
        activate_task(p.new_kstack_top, p.new_cr3);
        unsafe { context::yield_now(p.old_rsp_ptr, p.new_rsp) };
    }
}

/// Wake the first blocked task. Called from the serial RX IRQ handler.
///
/// Also sets NEED_RESCHED so that `check_resched` switches to the woken
/// task on the very next interrupt return, rather than waiting for a timer tick.
pub fn wake_reader() {
    let sched = match SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };
    if sched.lock().wake_blocked() {
        NEED_RESCHED.store(true, Ordering::Release);
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
