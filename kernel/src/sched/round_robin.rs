extern crate alloc;
use alloc::collections::VecDeque;

use super::process::{Task, TaskState};
use super::traits::{Scheduler, TaskId};

pub struct RoundRobinScheduler {
    queue: VecDeque<Task>,
    current: Option<Task>,
    ticks_remaining: usize,
    next_id: u64,
    /// Scratch RSP for the boot context (has no Task struct).
    boot_rsp: u64,
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
