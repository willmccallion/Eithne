use super::traits::TaskId;

/// Task state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Ready,
    Running,
    Blocked,
    Dead,
}

/// A schedulable task (kernel thread or user process).
pub struct Task {
    pub id: TaskId,
    pub state: TaskState,
    // TODO:
    // - address_space: Option<AddressSpace>  (None for kernel threads)
    // - kernel_stack: VirtAddr
    // - saved_context: ArchContext  (arch-specific register state)
    // - priority: u8
    // - name: [u8; 32]
}

impl Task {
    pub fn new(id: TaskId) -> Self {
        Self {
            id,
            state: TaskState::Ready,
        }
    }
}
