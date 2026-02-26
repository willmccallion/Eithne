use super::process::Task;
use super::traits::{Scheduler, TaskId};

/// Simple round-robin scheduler (stub).
///
/// Each task gets an equal time slice. Tasks are cycled in FIFO order.
pub struct RoundRobinScheduler {
    // TODO:
    // - queue: VecDeque<Task> (needs alloc, or a static ring buffer)
    // - current: Option<Task>
    // - ticks_remaining: usize
}

impl RoundRobinScheduler {
    pub fn new() -> Self {
        Self {}
    }
}

impl Scheduler for RoundRobinScheduler {
    fn add_task(&mut self, _task: Task) {
        // TODO: push to run queue
    }

    fn pick_next(&mut self) -> Option<Task> {
        // TODO: pop from front of queue
        None
    }

    fn yield_cpu(&mut self) {
        // TODO: move current task to back of queue, pick next
    }

    fn tick(&mut self) {
        // TODO: decrement time slice, preempt if expired
    }

    fn remove_task(&mut self, _id: TaskId) {
        // TODO: remove task from queue
    }
}
