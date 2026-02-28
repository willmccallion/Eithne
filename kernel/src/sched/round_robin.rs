extern crate alloc;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use super::process::{Task, TaskState};
use super::traits::{Scheduler, TaskId};

/// Lightweight zombie record: task resources have been freed, only the exit
/// status is kept until the parent calls waitpid.
struct Zombie {
    id: TaskId,
    parent: TaskId,
    status: i32,
}

pub struct RoundRobinScheduler {
    queue: VecDeque<Task>,
    current: Option<Task>,
    ticks_remaining: usize,
    next_id: u64,
    /// Scratch RSP for the boot context (has no Task struct).
    boot_rsp: u64,
    /// Dead tasks awaiting waitpid from their parent.
    zombies: Vec<Zombie>,
}

const TIMESLICE: usize = 2;

/// Returned by `prepare_switch`; caller must drop the lock before using it.
pub struct PendingSwitch {
    /// Where to write the outgoing task's saved RSP.
    pub old_rsp_ptr: *mut u64,
    /// Incoming task's saved RSP (points at an InterruptFrame).
    pub new_rsp: u64,
    /// Incoming task's kernel stack top (for SYSCALL_KSTACK_TOP).
    pub new_kstack_top: u64,
    /// Incoming task's page-table CR3 (0 = keep current / kernel thread).
    pub new_cr3: u64,
}

// SAFETY: old_rsp_ptr points into a Task in the VecDeque heap or boot_rsp.
unsafe impl Send for PendingSwitch {}

impl RoundRobinScheduler {
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            current: None,
            ticks_remaining: TIMESLICE,
            next_id: 1,
            boot_rsp: 0,
            zombies: Vec::new(),
        }
    }

    pub fn alloc_id(&mut self) -> TaskId {
        let id = TaskId(self.next_id);
        self.next_id += 1;
        id
    }

    fn task_cr3(task: &Task) -> u64 {
        #[cfg(target_arch = "x86_64")]
        if let Some(ref aspace) = task.aspace {
            return aspace.cr3().as_u64();
        }
        0
    }

    /// Pop the first `Ready` task from the queue, skipping any `Blocked` tasks.
    fn pop_next_ready(&mut self) -> Option<Task> {
        let pos = self
            .queue
            .iter()
            .position(|t| t.state == TaskState::Ready)?;
        self.queue.remove(pos)
    }

    /// Cooperative switch: caller will write outgoing RSP via `old_rsp_ptr`.
    pub fn prepare_switch(&mut self) -> Option<PendingSwitch> {
        let mut next = self.pop_next_ready()?;
        next.state = TaskState::Running;
        self.ticks_remaining = TIMESLICE;
        let new_rsp = next.saved_rsp;
        let new_kstack_top = next.kstack_top;
        let new_cr3 = Self::task_cr3(&next);

        let old_rsp_ptr: *mut u64 = if let Some(mut prev) = self.current.take() {
            if prev.state == TaskState::Dead {
                // Convert to zombie before dropping resources.
                let id = prev.id;
                let parent = prev.parent.unwrap_or(TaskId(0));
                let status = prev.exit_code.unwrap_or(0);
                prev.aspace = None;
                prev.fd_table = None;
                self.zombies.push(Zombie { id, parent, status });
                drop(prev);
                &mut self.boot_rsp as *mut u64
            } else {
                if prev.state == TaskState::Running {
                    prev.state = TaskState::Ready;
                }
                self.queue.push_back(prev);
                &mut self.queue.back_mut().unwrap().saved_rsp as *mut u64
            }
        } else {
            &mut self.boot_rsp as *mut u64
        };

        self.current = Some(next);
        Some(PendingSwitch {
            old_rsp_ptr,
            new_rsp,
            new_kstack_top,
            new_cr3,
        })
    }

    /// Preemptive switch: outgoing RSP is supplied directly.
    /// Returns (new_rsp, new_kstack_top, new_cr3) or None if no ready task.
    pub fn prepare_switch_preempt(&mut self, interrupted_rsp: u64) -> Option<(u64, u64, u64)> {
        let mut next = self.pop_next_ready()?;
        next.state = TaskState::Running;
        self.ticks_remaining = TIMESLICE;
        let new_rsp = next.saved_rsp;
        let new_kstack_top = next.kstack_top;
        let new_cr3 = Self::task_cr3(&next);

        if let Some(mut prev) = self.current.take() {
            if prev.state == TaskState::Dead {
                // Convert to zombie before dropping resources.
                let id = prev.id;
                let parent = prev.parent.unwrap_or(TaskId(0));
                let status = prev.exit_code.unwrap_or(0);
                prev.aspace = None;
                prev.fd_table = None;
                self.zombies.push(Zombie { id, parent, status });
                drop(prev);
            } else {
                if prev.state == TaskState::Running {
                    prev.state = TaskState::Ready;
                }
                prev.saved_rsp = interrupted_rsp;
                self.queue.push_back(prev);
            }
        }

        self.current = Some(next);
        Some((new_rsp, new_kstack_top, new_cr3))
    }

    /// Access the current task's file descriptor table mutably.
    ///
    /// Returns `None` if there is no current task or it has no fd table
    /// (kernel threads).
    pub fn current_fd_table(&mut self) -> Option<&mut super::fd::FdTable> {
        self.current.as_mut().and_then(|t| t.fd_table.as_mut())
    }

    /// Mark the currently running task as Dead so it won't be re-queued.
    pub fn kill_current(&mut self) {
        if let Some(ref mut cur) = self.current {
            cur.state = TaskState::Dead;
        }
    }

    /// Mark the currently running task as Blocked.
    ///
    /// The task will be re-queued but skipped by `pop_next_ready()` until
    /// `wake_blocked()` sets it back to `Ready`.
    pub fn block_current(&mut self) {
        if let Some(ref mut cur) = self.current {
            cur.state = TaskState::Blocked;
        }
    }

    /// Wake the first `Blocked` task in the queue by setting it to `Ready`.
    ///
    /// Returns `true` if a task was woken.
    pub fn wake_blocked(&mut self) -> bool {
        for task in self.queue.iter_mut() {
            if task.state == TaskState::Blocked {
                task.state = TaskState::Ready;
                return true;
            }
        }
        false
    }

    /// Decrement timeslice; return true when expired.
    pub fn tick_check(&mut self) -> bool {
        if self.ticks_remaining > 0 {
            self.ticks_remaining -= 1;
        }
        self.ticks_remaining == 0
    }

    // ------------------------------------------------------------------
    // Process management (fork / exit / wait)
    // ------------------------------------------------------------------

    /// TaskId of the currently running task.
    pub fn current_id(&self) -> Option<TaskId> {
        self.current.as_ref().map(|t| t.id)
    }

    /// Parent TaskId of the currently running task.
    pub fn current_parent(&self) -> Option<TaskId> {
        self.current.as_ref().and_then(|t| t.parent)
    }

    /// Mark the current task as Dead with the given wait status.
    pub fn kill_current_with_status(&mut self, status: i32) {
        if let Some(ref mut cur) = self.current {
            cur.state = TaskState::Dead;
            cur.exit_code = Some(status);
        }
    }

    /// Convert the current Dead task into a zombie record.
    ///
    /// Frees the task's address space, FD table, and kernel stack, keeping
    /// only the PID, parent, and exit status for waitpid.
    pub fn reap_current_to_zombie(&mut self) {
        if let Some(ref mut cur) = self.current {
            if cur.state != TaskState::Dead {
                return;
            }
            let id = cur.id;
            let parent = cur.parent.unwrap_or(TaskId(0));
            let status = cur.exit_code.unwrap_or(0);

            // Drop expensive resources while we still own the task.
            cur.aspace = None;
            cur.fd_table = None;

            self.zombies.push(Zombie { id, parent, status });
        }
    }

    /// Wake a specific Blocked task by TaskId (used to wake a parent waiting
    /// on waitpid).
    pub fn wake_task(&mut self, id: TaskId) -> bool {
        for task in self.queue.iter_mut() {
            if task.id == id && task.state == TaskState::Blocked {
                task.state = TaskState::Ready;
                return true;
            }
        }
        false
    }

    /// Collect (reap) a zombie child matching `parent` and optional `child_pid`.
    ///
    /// If `child_pid` is `u64::MAX`, any zombie child of `parent` is accepted.
    /// Returns `(child_tid, wait_status)` and removes the zombie.
    pub fn collect_zombie(&mut self, parent: TaskId, child_pid: u64) -> Option<(TaskId, i32)> {
        let pos = self
            .zombies
            .iter()
            .position(|z| z.parent == parent && (child_pid == u64::MAX || z.id.0 == child_pid))?;
        let z = self.zombies.swap_remove(pos);
        Some((z.id, z.status))
    }

    /// Check whether `parent` has any living children (in queue or current)
    /// or zombie children.
    pub fn has_children(&self, parent: TaskId) -> bool {
        if self.zombies.iter().any(|z| z.parent == parent) {
            return true;
        }
        if self.queue.iter().any(|t| t.parent == Some(parent)) {
            return true;
        }
        if let Some(ref cur) = self.current {
            if cur.parent == Some(parent) {
                return true;
            }
        }
        false
    }

    /// Reparent all children (alive + zombie) of `old_parent` to `new_parent`.
    pub fn reparent_children(&mut self, old_parent: TaskId, new_parent: TaskId) {
        for task in self.queue.iter_mut() {
            if task.parent == Some(old_parent) {
                task.parent = Some(new_parent);
            }
        }
        for z in self.zombies.iter_mut() {
            if z.parent == old_parent {
                z.parent = new_parent;
            }
        }
    }

    /// Find any task by ID (current or queued) and return a mutable reference.
    pub fn find_task_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        if let Some(ref mut cur) = self.current {
            if cur.id == id {
                return Some(cur);
            }
        }
        self.queue.iter_mut().find(|t| t.id == id)
    }

    /// Mutable access to the current task (used by fork/exec).
    pub fn current_task_mut(&mut self) -> Option<&mut Task> {
        self.current.as_mut()
    }

    /// Immutable access to the current task.
    pub fn current_task(&self) -> Option<&Task> {
        self.current.as_ref()
    }
}

impl Scheduler for RoundRobinScheduler {
    fn add_task(&mut self, task: Task) {
        self.queue.push_back(task);
    }
    fn pick_next(&mut self) {}
    fn yield_cpu(&mut self) {}
    fn tick(&mut self) {}

    fn remove_task(&mut self, id: TaskId) {
        self.queue.retain(|t| t.id != id);
        if let Some(ref mut cur) = self.current
            && cur.id == id
        {
            cur.state = TaskState::Dead;
        }
    }
}
