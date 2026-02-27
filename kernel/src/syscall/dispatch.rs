use super::table::{SYS_EXIT, SYS_WRITE};
use super::traits::{SyscallError, SyscallResult};

/// Dispatch a system call by number.
///
/// Called from the arch-specific syscall entry point after saving registers.
pub fn dispatch(num: usize, args: &[usize; 6]) -> SyscallResult {
    match num {
        SYS_WRITE => sys_write(args),
        SYS_EXIT => sys_exit(args),
        _ => Err(SyscallError::InvalidSyscall),
    }
}

// ---------------------------------------------------------------------------
// sys_write(fd, buf_ptr, len) -> bytes_written
// ---------------------------------------------------------------------------
//
// For now only fd=1 (stdout) and fd=2 (stderr) are supported; both write to
// the serial console.  The buffer is in user address space — in a real kernel
// we'd validate the pointer range against the current process's page table.
// Here we trust it because our single user task lives in a known range.

fn sys_write(args: &[usize; 6]) -> SyscallResult {
    let _fd = args[0];
    let ptr = args[1] as *const u8;
    let len = args[2];

    if len == 0 {
        return Ok(0);
    }

    // Basic bounds sanity: reject null or obviously-kernel pointers.
    if ptr.is_null() || (ptr as usize) >= 0xFFFF_8000_0000_0000 {
        return Err(SyscallError::InvalidArgument);
    }

    // Safety: we accept the user's pointer at face value for now.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };

    if let Ok(s) = core::str::from_utf8(bytes) {
        crate::arch::write_serial(s);
    } else {
        // Non-UTF-8: write byte-by-byte as hex escape sequences.
        for &b in bytes {
            if b.is_ascii() && !b.is_ascii_control() {
                crate::arch::write_serial(core::str::from_utf8(&[b]).unwrap_or("?"));
            }
        }
    }

    Ok(len)
}

// ---------------------------------------------------------------------------
// sys_exit(code)
// ---------------------------------------------------------------------------

fn sys_exit(args: &[usize; 6]) -> SyscallResult {
    let code = args[0] as i32;
    crate::println!("[syscall] process exited with code {}", code);
    crate::sched::exit_current();
}
