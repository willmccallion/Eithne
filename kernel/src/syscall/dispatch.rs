extern crate alloc;
use alloc::string::String;

use core::sync::atomic::Ordering;

use super::table::*;
use super::traits::{SyscallError, SyscallResult};
use crate::fs::traits::{DirEntry, FileType, OpenFlags};

/// Kernel virtual address boundary — pointers at or above this are kernel space.
const KERNEL_SPACE_START: usize = 0xFFFF_8000_0000_0000;

/// Maximum path length we'll copy from user space.
const PATH_MAX: usize = 4096;

/// Dispatch a system call by number.
///
/// Called from the arch-specific syscall entry point after saving registers.
pub fn dispatch(num: usize, args: &[usize; 6]) -> SyscallResult {
    match num {
        SYS_READ => sys_read(args),
        SYS_WRITE => sys_write(args),
        SYS_OPEN => sys_open(args),
        SYS_CLOSE => sys_close(args),
        SYS_STAT => sys_stat(args),
        SYS_FORK | SYS_EXECVE => unreachable!("handled in arch syscall layer"),
        SYS_EXIT => sys_exit(args),
        SYS_WAIT4 => sys_wait4(args),
        SYS_MKDIR => sys_mkdir(args),
        SYS_UNLINK => sys_unlink(args),
        SYS_READDIR => sys_readdir(args),
        _ => Err(SyscallError::InvalidSyscall),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate that a user pointer is in user address space.
fn validate_user_ptr(ptr: usize) -> Result<(), SyscallError> {
    if ptr == 0 || ptr >= KERNEL_SPACE_START {
        return Err(SyscallError::InvalidArgument);
    }
    Ok(())
}

/// Copy a NUL-terminated path string from user space.
pub fn copy_path_from_user(ptr: usize) -> Result<String, SyscallError> {
    validate_user_ptr(ptr)?;

    let mut path = String::new();
    let base = ptr as *const u8;
    for i in 0..PATH_MAX {
        let addr = ptr + i;
        if addr >= KERNEL_SPACE_START {
            return Err(SyscallError::InvalidArgument);
        }
        let byte = unsafe { *base.add(i) };
        if byte == 0 {
            break;
        }
        path.push(byte as char);
    }

    if path.is_empty() {
        return Err(SyscallError::InvalidArgument);
    }

    Ok(path)
}

/// Convert Linux O_* flags integer to our OpenFlags.
///
/// Linux flags: O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_CREAT=0x40,
/// O_TRUNC=0x200, O_APPEND=0x400.
fn linux_flags_to_open_flags(flags: usize) -> OpenFlags {
    let access = flags & 0x3;
    OpenFlags {
        read: access == 0 || access == 2,  // O_RDONLY or O_RDWR
        write: access == 1 || access == 2, // O_WRONLY or O_RDWR
        create: flags & 0x40 != 0,         // O_CREAT
        truncate: flags & 0x200 != 0,      // O_TRUNC
        append: flags & 0x400 != 0,        // O_APPEND
    }
}

// ---------------------------------------------------------------------------
// sys_read(fd, buf_ptr, len) -> bytes_read
// ---------------------------------------------------------------------------

fn sys_read(args: &[usize; 6]) -> SyscallResult {
    let fd = args[0];
    let ptr = args[1] as *mut u8;
    let len = args[2];

    if len == 0 {
        return Ok(0);
    }

    validate_user_ptr(ptr as usize)?;

    // Stdio fd 0 (stdin) — serial console.
    if fd == 0 {
        let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len) };

        loop {
            let n = crate::drivers::serial::read(buf, len);
            if n > 0 {
                return Ok(n);
            }
            // Block only if the buffer is still empty.  The condition closure
            // runs with interrupts disabled (inside the scheduler's IrqSpinlock),
            // so no IRQ can push data between this check and the actual block.
            crate::drivers::serial::READER_BLOCKED.store(true, Ordering::Release);
            crate::sched::block_current_if(|| {
                if crate::drivers::serial::is_empty() {
                    true // buffer still empty → block
                } else {
                    // Data arrived between read() and here; cancel the block.
                    crate::drivers::serial::READER_BLOCKED.store(false, Ordering::Release);
                    false
                }
            });
        }
    }

    // Stdio fd 1/2 — reading from stdout/stderr not supported.
    if fd == 1 || fd == 2 {
        return Err(SyscallError::InvalidArgument);
    }

    // VFS file descriptor.
    let handle = crate::sched::with_current_fd_table(|fdt| fdt.get(fd))?;
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
    let n = crate::fs::vfs::read(handle, buf)?;
    Ok(n)
}

// ---------------------------------------------------------------------------
// sys_write(fd, buf_ptr, len) -> bytes_written
// ---------------------------------------------------------------------------

fn sys_write(args: &[usize; 6]) -> SyscallResult {
    let fd = args[0];
    let ptr = args[1] as *const u8;
    let len = args[2];

    if len == 0 {
        return Ok(0);
    }

    validate_user_ptr(ptr as usize)?;

    // Stdio fds 1/2 — serial + framebuffer console.
    if fd == 1 || fd == 2 {
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        if let Ok(s) = core::str::from_utf8(bytes) {
            crate::arch::write_serial(s);
            crate::drivers::fb::console::fb_write_str(s);
        } else {
            for &b in bytes {
                if b.is_ascii() && !b.is_ascii_control() {
                    let buf = [b];
                    let s = core::str::from_utf8(&buf).unwrap_or("?");
                    crate::arch::write_serial(s);
                    crate::drivers::fb::console::fb_write_str(s);
                }
            }
        }
        return Ok(len);
    }

    // Stdio fd 0 — writing to stdin not supported.
    if fd == 0 {
        return Err(SyscallError::InvalidArgument);
    }

    // VFS file descriptor.
    let handle = crate::sched::with_current_fd_table(|fdt| fdt.get(fd))?;
    let buf = unsafe { core::slice::from_raw_parts(ptr, len) };
    let n = crate::fs::vfs::write(handle, buf)?;
    Ok(n)
}

// ---------------------------------------------------------------------------
// sys_open(path_ptr, flags, mode) -> fd
// ---------------------------------------------------------------------------

fn sys_open(args: &[usize; 6]) -> SyscallResult {
    let path = copy_path_from_user(args[0])?;
    let flags = linux_flags_to_open_flags(args[1]);

    let handle = crate::fs::vfs::open(&path, flags)?;
    let fd = crate::sched::with_current_fd_table(|fdt| fdt.alloc_fd(handle))?;
    Ok(fd)
}

// ---------------------------------------------------------------------------
// sys_close(fd) -> 0
// ---------------------------------------------------------------------------

fn sys_close(args: &[usize; 6]) -> SyscallResult {
    let fd = args[0];

    // Closing stdio fds is a no-op.
    if crate::sched::FdTable::is_stdio(fd) {
        return Ok(0);
    }

    let handle = crate::sched::with_current_fd_table(|fdt| fdt.close(fd))?;
    crate::fs::vfs::close(handle)?;
    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_stat(path_ptr, statbuf_ptr) -> 0
// ---------------------------------------------------------------------------

/// Simplified Linux-compatible stat struct written to user space.
///
/// We only fill in the fields our VFS provides; the rest are zeroed.
#[repr(C)]
struct UserStat {
    st_ino: u64,
    st_mode: u32,
    _pad0: u32,
    st_size: u64,
    _pad1: [u8; 128], // remaining fields zeroed
}

fn sys_stat(args: &[usize; 6]) -> SyscallResult {
    let path = copy_path_from_user(args[0])?;
    let statbuf = args[1];

    validate_user_ptr(statbuf)?;

    let stat = crate::fs::vfs::stat(&path)?;

    // Convert FileType to a Linux st_mode value (type bits only).
    let mode: u32 = match stat.file_type {
        FileType::Regular => 0o100644,     // S_IFREG | 0644
        FileType::Directory => 0o040755,   // S_IFDIR | 0755
        FileType::Symlink => 0o120777,     // S_IFLNK | 0777
        FileType::CharDevice => 0o020666,  // S_IFCHR | 0666
        FileType::BlockDevice => 0o060660, // S_IFBLK | 0660
        FileType::Pipe => 0o010644,        // S_IFIFO | 0644
        FileType::Socket => 0o140755,      // S_IFSOCK | 0755
    };

    let user_stat = UserStat {
        st_ino: stat.inode,
        st_mode: mode,
        _pad0: 0,
        st_size: stat.size,
        _pad1: [0u8; 128],
    };

    let dst = statbuf as *mut UserStat;
    unsafe {
        dst.write(user_stat);
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_mkdir(path_ptr, mode) -> 0
// ---------------------------------------------------------------------------

fn sys_mkdir(args: &[usize; 6]) -> SyscallResult {
    let path = copy_path_from_user(args[0])?;
    crate::fs::vfs::mkdir(&path)?;
    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_unlink(path_ptr) -> 0
// ---------------------------------------------------------------------------

fn sys_unlink(args: &[usize; 6]) -> SyscallResult {
    let path = copy_path_from_user(args[0])?;
    crate::fs::vfs::unlink(&path)?;
    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_readdir(path_ptr, buf_ptr, buf_len) -> bytes_written
// ---------------------------------------------------------------------------
//
// Custom syscall (uses getdents64 number 217).
// Writes a sequence of UserDirEntry structs to the user buffer.

#[repr(C)]
struct UserDirEntry {
    d_ino: u64,
    d_type: u8,
    d_name_len: u8,
    _pad: [u8; 6],
    d_name: [u8; 256],
}

fn sys_readdir(args: &[usize; 6]) -> SyscallResult {
    let path = copy_path_from_user(args[0])?;
    let buf_ptr = args[1];
    let buf_len = args[2];

    validate_user_ptr(buf_ptr)?;

    let entries = crate::fs::vfs::readdir(&path)?;
    let entry_size = core::mem::size_of::<UserDirEntry>();
    let max_entries = buf_len / entry_size;

    let mut written = 0usize;
    for (i, entry) in entries.iter().enumerate() {
        if i >= max_entries {
            break;
        }

        let d_type: u8 = match entry.file_type {
            FileType::Regular => 8,     // DT_REG
            FileType::Directory => 4,   // DT_DIR
            FileType::Symlink => 10,    // DT_LNK
            FileType::CharDevice => 2,  // DT_CHR
            FileType::BlockDevice => 6, // DT_BLK
            FileType::Pipe => 1,        // DT_FIFO
            FileType::Socket => 12,     // DT_SOCK
        };

        let user_entry = UserDirEntry {
            d_ino: entry.inode,
            d_type,
            d_name_len: entry.name_len as u8,
            _pad: [0u8; 6],
            d_name: entry.name,
        };

        let dst = (buf_ptr + i * entry_size) as *mut UserDirEntry;
        unsafe {
            dst.write(user_entry);
        }
        written += entry_size;
    }

    Ok(written)
}

// ---------------------------------------------------------------------------
// sys_exit(code)
// ---------------------------------------------------------------------------

fn sys_exit(args: &[usize; 6]) -> SyscallResult {
    let code = args[0] as i32;
    crate::sched::exit_current_with_code(code);
}

// ---------------------------------------------------------------------------
// sys_wait4(pid, status_ptr, options, rusage) -> child_pid
// ---------------------------------------------------------------------------

/// `pid` semantics:
///   - `usize::MAX` (i.e. -1 as unsigned): wait for any child
///   - otherwise: wait for the specific child PID
const WAIT_ANY: u64 = u64::MAX;

fn sys_wait4(args: &[usize; 6]) -> SyscallResult {
    let raw_pid = args[0];
    let status_ptr = args[1];

    // Convert to the u64 the scheduler expects.
    // -1 (0xFFFF...) means "any child".
    let child_pid = if raw_pid as isize == -1 {
        WAIT_ANY
    } else {
        raw_pid as u64
    };

    let (pid, status) = crate::sched::wait_for_child(child_pid)?;

    // Write wait status to user space if pointer is non-null.
    if status_ptr != 0 {
        validate_user_ptr(status_ptr)?;
        let dst = status_ptr as *mut i32;
        unsafe {
            dst.write(status);
        }
    }

    Ok(pid as usize)
}
