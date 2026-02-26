/// System call result type.
pub type SyscallResult = Result<usize, SyscallError>;

/// System call error codes.
#[derive(Debug, Clone, Copy)]
pub enum SyscallError {
    InvalidSyscall,
    InvalidArgument,
    PermissionDenied,
    NotFound,
    IoError,
    NoMemory,
    WouldBlock,
}

/// System call handler trait.
///
/// The kernel's syscall dispatch table maps syscall numbers to handlers.
/// Each handler validates arguments and performs the requested operation.
pub trait SyscallHandler {
    /// Handle a system call.
    ///
    /// `num` is the syscall number, `args` contains up to 6 arguments
    /// (passed in registers by the arch-specific syscall entry).
    fn handle(&self, num: usize, args: &[usize; 6]) -> SyscallResult;
}
