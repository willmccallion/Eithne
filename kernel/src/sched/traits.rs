use super::process::Task;

/// Scheduler trait.
///
/// Determines which task runs next. Implementations can provide
/// different scheduling policies (round-robin, priority-based, CFS-like).
pub trait Scheduler {
    /// Add a task to the scheduler's run queue.
    fn add_task(&mut self, task: Task);

    /// Select the next task to run. Returns None if no tasks are runnable.
    fn pick_next(&mut self) -> Option<Task>;

    /// Voluntarily yield the CPU to the scheduler.
    fn yield_cpu(&mut self);

    /// Called on each timer tick to update scheduling state.
    fn tick(&mut self);

    /// Remove a task from the scheduler by its ID.
    fn remove_task(&mut self, id: TaskId);
}

/// Unique identifier for a scheduled task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);
