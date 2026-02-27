use super::process::Task;

/// Scheduler trait.
///
/// Determines which task runs next. Implementations provide different
/// scheduling policies (round-robin, priority-based, CFS-like).
pub trait Scheduler {
    /// Add a task to the scheduler's run queue.
    fn add_task(&mut self, task: Task);

    /// Switch to the next runnable task.  Returns immediately (without
    /// switching) if the run queue is empty.
    fn pick_next(&mut self);

    /// Voluntarily yield the CPU — moves the current task to the back of
    /// the run queue and switches to the next task.
    fn yield_cpu(&mut self);

    /// Called on each timer tick.  Decrements the time slice and triggers a
    /// context switch when it expires.
    fn tick(&mut self);

    /// Remove a task from the scheduler by ID.  If the task is currently
    /// running it is marked Dead and removed on the next switch.
    fn remove_task(&mut self, id: TaskId);
}

/// Unique identifier for a scheduled task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);
