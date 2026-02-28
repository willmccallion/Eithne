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

/// Run a closure with the scheduler lock held (used by fork which needs
/// direct access to the current task's fields).
pub fn with_scheduler<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut RoundRobinScheduler) -> R,
{
    let sched = SCHEDULER.get()?;
    Some(f(&mut sched.lock()))
}

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

/// The TaskId of the first user process (init). Orphaned children are
/// reparented here.
const INIT_TASK_ID: TaskId = TaskId(2);

/// TaskId of the currently running task.
pub fn current_task_id() -> Option<TaskId> {
    let sched = SCHEDULER.get()?;
    sched.lock().current_id()
}

/// Run a closure with mutable access to the current task.
///
/// The scheduler lock is held for the duration of the closure.
pub fn with_current_task<F, R>(f: F) -> Result<R, crate::syscall::traits::SyscallError>
where
    F: FnOnce(&mut process::Task) -> Result<R, crate::syscall::traits::SyscallError>,
{
    let sched = SCHEDULER
        .get()
        .ok_or(crate::syscall::traits::SyscallError::IoError)?;
    let mut guard = sched.lock();
    let task = guard
        .current_task_mut()
        .ok_or(crate::syscall::traits::SyscallError::InvalidArgument)?;
    f(task)
}

/// Add a pre-built Task to the scheduler queue (used by fork).
pub fn add_task(task: Task) {
    if let Some(sched) = SCHEDULER.get() {
        sched.lock().add_task(task);
    }
}

/// Allocate a new TaskId.
pub fn alloc_id() -> Option<TaskId> {
    let sched = SCHEDULER.get()?;
    Some(sched.lock().alloc_id())
}

/// Mark the current task as Dead and yield (backwards compatibility).
pub fn exit_current() -> ! {
    exit_current_with_code(0);
}

/// Mark the current task as Dead with the given exit code and yield.
///
/// Encodes the exit code as a standard Unix wait status `(code & 0xFF) << 8`,
/// drains file descriptors (so pipe refcounts are decremented while we can
/// still safely call into the pipe subsystem), reparents children to init,
/// wakes the parent if blocked, then yields.  Does not return.
pub fn exit_current_with_code(code: i32) -> ! {
    use crate::sched::fd::FdKind;

    /// Standard Unix encoding: normal exit status = `(code & 0xFF) << 8`.
    const EXIT_STATUS_SHIFT: i32 = 8;
    const EXIT_CODE_MASK: i32 = 0xFF;

    let status = (code & EXIT_CODE_MASK) << EXIT_STATUS_SHIFT;

    // Phase 1: Under the scheduler lock, drain all fds and mark the task Dead.
    let fds_to_close = {
        let sched = SCHEDULER.get().expect("scheduler not init");
        let mut guard = sched.lock();

        // Drain all open fds so we can close them outside the lock.
        let fds = guard
            .current_task_mut()
            .and_then(|t| t.fd_table.as_mut())
            .map(|fdt| fdt.drain_all())
            .unwrap_or_default();

        // Record exit status.
        guard.kill_current_with_status(status);

        // Reparent all children to init.
        if let Some(my_id) = guard.current_id() {
            guard.reparent_children(my_id, INIT_TASK_ID);
        }

        // Wake the parent if it's blocked (waiting in waitpid).
        if let Some(parent_id) = guard.current_parent() {
            guard.wake_task(parent_id);
        }

        fds
    };
    // Scheduler lock is dropped.

    // Phase 2: Close each fd outside the scheduler lock. Pipe closes may
    // wake blocked readers/writers (which acquires the scheduler lock).
    for kind in fds_to_close {
        match kind {
            FdKind::Pipe(h) => crate::ipc::pipe::pipe_close(h),
            FdKind::Vfs(h) => {
                let _ = crate::fs::vfs::close(h);
            }
        }
    }

    yield_cpu();
    loop {
        crate::arch::halt_once();
    }
}

/// Block until a child exits, then return `(child_pid, wait_status)`.
///
/// If `child_pid` is `u64::MAX`, waits for any child.
/// Returns `Err(NoChildren)` if the caller has no children.
pub fn wait_for_child(child_pid: u64) -> Result<(u64, i32), crate::syscall::traits::SyscallError> {
    use crate::syscall::traits::SyscallError;

    loop {
        let should_block = {
            let sched = SCHEDULER.get().ok_or(SyscallError::IoError)?;
            let mut guard = sched.lock();

            let my_id = guard.current_id().ok_or(SyscallError::IoError)?;

            // Try to collect a zombie child.
            if let Some((tid, status)) = guard.collect_zombie(my_id, child_pid) {
                return Ok((tid.0, status));
            }

            // No zombie — check if we have any children at all.
            if !guard.has_children(my_id) {
                return Err(SyscallError::NoChildren);
            }

            // Block this task; it will be woken when a child exits.
            guard.block_current();
            guard.prepare_switch()
        };

        if let Some(p) = should_block {
            activate_task(p.new_kstack_top, p.new_cr3);
            unsafe { context::yield_now(p.old_rsp_ptr, p.new_rsp) };
        }
        // After waking, loop to re-check for zombies.
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

/// Wake the specific task blocked on serial read. Called from the serial
/// RX and keyboard IRQ handlers.
///
/// Also sets NEED_RESCHED so that `check_resched` switches to the woken
/// task on the very next interrupt return, rather than waiting for a timer tick.
pub fn wake_serial_reader(tid: u64) {
    let sched = match SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };
    if sched.lock().wake_task(TaskId(tid)) {
        NEED_RESCHED.store(true, Ordering::Release);
    }
}

/// Wake a task blocked on a pipe read or write. Called from `pipe_close`,
/// `pipe_read`, and `pipe_write` after releasing the PIPES lock.
pub fn wake_pipe_task(tid: u64) {
    let sched = match SCHEDULER.get() {
        Some(s) => s,
        None => return,
    };
    if sched.lock().wake_task(TaskId(tid)) {
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
