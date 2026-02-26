use super::traits::{SyscallError, SyscallResult};

/// Dispatch a system call by number.
///
/// Called from the arch-specific syscall entry point after saving registers.
pub fn dispatch(num: usize, _args: &[usize; 6]) -> SyscallResult {
    match num {
        // TODO: define syscall numbers and route to implementations
        // 0 => sys_read(args),
        // 1 => sys_write(args),
        // 2 => sys_open(args),
        // 3 => sys_close(args),
        // 60 => sys_exit(args),
        _ => Err(SyscallError::InvalidSyscall),
    }
}
