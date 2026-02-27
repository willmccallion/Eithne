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
    /// User-mode address space.  None for kernel threads.
    #[cfg(target_arch = "x86_64")]
    pub aspace: Option<crate::arch::x86_64::KernelAddressSpace>,
    /// Top of this task's kernel stack; used to update SYSCALL_KSTACK_TOP on switch.
    pub kstack_top: u64,
}

impl Task {
    pub fn new(id: TaskId, name: &'static str, entry_fn: fn() -> !) -> Option<Self> {
        let stack = TaskStack::new_for_fn(entry_fn)?;
        let initial_rsp = stack.initial_rsp;
        let kstack_top = stack.kstack_top;
        Some(Self {
            id,
            state: TaskState::Ready,
            saved_rsp: initial_rsp,
            kstack_top,
            stack,
            name,
            #[cfg(target_arch = "x86_64")]
            aspace: None,
        })
    }
}
