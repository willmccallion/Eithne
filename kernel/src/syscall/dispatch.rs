use super::traits::{SyscallError, SyscallResult};

/// Dispatch a system call by number.
///
/// Called from the arch-specific syscall entry point after saving registers.
pub fn dispatch(_num: usize, _args: &[usize; 6]) -> SyscallResult {
    // TODO: define syscall numbers and route to implementations
    // match num {
    //     0 => sys_read(args),
    //     1 => sys_write(args),
    //     2 => sys_open(args),
    //     3 => sys_close(args),
    //     60 => sys_exit(args),
    //     _ => Err(SyscallError::InvalidSyscall),
    // }
    Err(SyscallError::InvalidSyscall)
}
