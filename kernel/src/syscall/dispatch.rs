use core::sync::atomic::Ordering;

use super::table::{SYS_EXIT, SYS_READ, SYS_WRITE};
use super::traits::{SyscallError, SyscallResult};

/// Kernel virtual address boundary — pointers at or above this are kernel space.
const KERNEL_SPACE_START: usize = 0xFFFF_8000_0000_0000;

/// Standard file descriptor numbers.
const FD_STDIN: usize = 0;

/// Dispatch a system call by number.
///
/// Called from the arch-specific syscall entry point after saving registers.
pub fn dispatch(num: usize, args: &[usize; 6]) -> SyscallResult {
    match num {
        SYS_READ => sys_read(args),
        SYS_WRITE => sys_write(args),
        SYS_EXIT => sys_exit(args),
        _ => Err(SyscallError::InvalidSyscall),
    }
}

// ---------------------------------------------------------------------------
// sys_read(fd, buf_ptr, len) -> bytes_read
// ---------------------------------------------------------------------------

fn sys_read(args: &[usize; 6]) -> SyscallResult {
    let fd = args[0];
    let ptr = args[1] as *mut u8;
    let len = args[2];

    if fd != FD_STDIN {
        return Err(SyscallError::InvalidArgument);
    }

    if len == 0 {
        return Ok(0);
    }

    if ptr.is_null() || (ptr as usize) >= KERNEL_SPACE_START {
        return Err(SyscallError::InvalidArgument);
    }

    // Block until at least one byte is available.
    loop {
        let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
        let n = crate::drivers::serial::read(buf, len);
        if n > 0 {
            return Ok(n);
        }
        // No data available — mark ourselves as waiting and yield.
        crate::drivers::serial::READER_BLOCKED.store(true, Ordering::Release);
        crate::sched::block_current_and_yield();
        // We've been woken — loop back to try reading again.
    }
}

// ---------------------------------------------------------------------------
// sys_write(fd, buf_ptr, len) -> bytes_written
// ---------------------------------------------------------------------------
//
// For now only fd=1 (stdout) and fd=2 (stderr) are supported; both write to
// the serial console.  The buffer is in user address space — in a real kernel
// we'd validate the pointer range against the current process's page table.

fn sys_write(args: &[usize; 6]) -> SyscallResult {
    let _fd = args[0];
    let ptr = args[1] as *const u8;
    let len = args[2];

    if len == 0 {
        return Ok(0);
    }

    if ptr.is_null() || (ptr as usize) >= KERNEL_SPACE_START {
        return Err(SyscallError::InvalidArgument);
    }

    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };

    if let Ok(s) = core::str::from_utf8(bytes) {
        crate::arch::write_serial(s);
        crate::drivers::fb::console::fb_write_str(s);
    } else {
        // Non-UTF-8: write byte-by-byte as hex escape sequences.
        for &b in bytes {
            if b.is_ascii() && !b.is_ascii_control() {
                let buf = [b];
                let s = core::str::from_utf8(&buf).unwrap_or("?");
                crate::arch::write_serial(s);
                crate::drivers::fb::console::fb_write_str(s);
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
