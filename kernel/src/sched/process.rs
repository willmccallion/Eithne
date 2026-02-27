use super::context::TaskStack;
use super::traits::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Dead,
}

pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    /// RSP pointing at the task's `InterruptFrame` on its kernel stack.
    pub saved_rsp: u64,
    pub stack: TaskStack,
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
