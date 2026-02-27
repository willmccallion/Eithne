extern crate alloc;
use alloc::collections::VecDeque;

use super::process::{Task, TaskState};
use super::traits::{Scheduler, TaskId};

/// Simple preemptive round-robin scheduler.
///
/// Each task gets `TIMESLICE` timer ticks before being preempted.
pub struct RoundRobinScheduler {
    /// Tasks waiting to run.
    queue: VecDeque<Task>,
    /// The task currently executing (None only before the first switch).
    current: Option<Task>,
    /// Remaining ticks for the current task.
    ticks_remaining: usize,
    /// Sequence counter for issuing `TaskId`s.
    next_id: u64,
    /// Scratch RSP storage for the boot context (has no Task struct).
    boot_rsp: u64,
}

/// Default time slice: 2 PIT ticks ≈ 110 ms at the default 18.2 Hz rate.
const TIMESLICE: usize = 2;

/// Values needed to call `switch_context` after the scheduler lock is dropped.
pub struct PendingSwitch {
    /// Where `switch_context` writes the outgoing task's saved RSP.
    pub old_rsp_ptr: *mut u64,
    /// The incoming task's saved RSP to switch to.
    pub new_rsp: u64,
}

// SAFETY: old_rsp_ptr either points into a Task in the scheduler queue
// (stable because VecDeque heap memory doesn't move during a switch) or
// into the scheduler's own `boot_rsp` field.  Either way it outlives the
// switch_context call.
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

    /// Prepare a context switch to the next queued task.
    ///
    /// Updates all scheduler state and returns the RSP pair needed for
    /// `switch_context`.  Returns `None` if the queue is empty.
    ///
    /// **The caller must drop the scheduler lock before calling
    /// `switch_context` with the returned `PendingSwitch`.**
    pub fn prepare_switch(&mut self) -> Option<PendingSwitch> {
        let mut next = self.queue.pop_front()?;
        next.state = TaskState::Running;
        self.ticks_remaining = TIMESLICE;
        let new_rsp = next.saved_rsp; // read before next is moved into self.current

        let old_rsp_ptr: *mut u64 = if let Some(mut prev) = self.current.take() {
            if prev.state == TaskState::Running {
                prev.state = TaskState::Ready;
            }
            self.queue.push_back(prev);
            // SAFETY: VecDeque stores its elements on the heap; push_back does
            // not relocate existing entries, so this pointer stays valid until
            // the task is removed from the queue.
            let rsp_ref: &mut u64 = &mut self.queue.back_mut().unwrap().saved_rsp;
            rsp_ref as *mut u64
        } else {
            &mut self.boot_rsp as *mut u64
        };

        self.current = Some(next);
        Some(PendingSwitch { old_rsp_ptr, new_rsp })
    }

    /// Decrement the timeslice and return true if it has expired.
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
