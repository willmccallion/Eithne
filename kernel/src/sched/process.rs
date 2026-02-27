use super::context::TaskStack;
use super::traits::TaskId;

/// Task state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Dead,
}

/// A schedulable kernel task (kernel thread).
pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    /// Saved kernel stack pointer. Updated by `switch_context` on every switch.
    pub saved_rsp: u64,
    /// Owns the allocated stack memory. Dropped when the task is destroyed.
    pub stack: TaskStack,
    /// Human-readable name for debugging.
    pub name: &'static str,
}

impl Task {
    pub fn new(id: TaskId, name: &'static str, entry_fn: fn() -> !) -> Option<Self> {
        let stack = TaskStack::new_for_fn(entry_fn)?;
        let initial_rsp = stack.initial_rsp;
        Some(Self {
            id,
            state: TaskState::Ready,
            saved_rsp: initial_rsp,
            stack,
            name,
        })
    }
}
