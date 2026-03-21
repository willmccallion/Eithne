use alloc;
use alloc::string::String;
use alloc::vec::Vec;

use core::sync::atomic::Ordering;

use super::table::{
    SYS_BRK, SYS_CHDIR, SYS_CLOSE, SYS_DUP2, SYS_EXECVE, SYS_EXIT, SYS_FORK, SYS_GETCWD,
    SYS_GETPID, SYS_GETPPID, SYS_KILL, SYS_MKDIR, SYS_OPEN, SYS_PIPE, SYS_READ, SYS_READDIR,
    SYS_SIGACTION, SYS_SIGPROCMASK, SYS_STAT, SYS_UNLINK, SYS_WAIT4, SYS_WRITE,
};
use super::traits::{SyscallError, SyscallResult};
use crate::fs::traits::{DirEntry, FileType, OpenFlags};
use crate::mm::traits::PageMapper;
use crate::sched::fd::FdKind;

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
        SYS_BRK => sys_brk(args),
        SYS_PIPE => sys_pipe(args),
        SYS_DUP2 => sys_dup2(args),
        SYS_FORK | SYS_EXECVE => unreachable!("handled in arch syscall layer"),
        SYS_EXIT => sys_exit(args),
        SYS_WAIT4 => sys_wait4(args),
        SYS_MKDIR => sys_mkdir(args),
        SYS_UNLINK => sys_unlink(args),
        SYS_GETPID => sys_getpid(),
        SYS_GETPPID => sys_getppid(),
        SYS_GETCWD => sys_getcwd(args),
        SYS_CHDIR => sys_chdir(args),
        SYS_READDIR => sys_readdir(args),
        SYS_KILL => sys_kill(args),
        SYS_SIGACTION => sys_sigaction(args),
        SYS_SIGPROCMASK => sys_sigprocmask(args),
        _ => Err(SyscallError::InvalidSyscall),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Validate that a user pointer is in user address space.
const fn validate_user_ptr(ptr: usize) -> Result<(), SyscallError> {
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

/// Copy a path from user space and resolve it to an absolute path.
///
/// If the path is relative (doesn't start with '/'), it is resolved against
/// the current task's working directory. Handles "." and ".." components.
fn copy_and_resolve_path(ptr: usize) -> Result<String, SyscallError> {
    let raw = copy_path_from_user(ptr)?;
    if raw.starts_with('/') {
        return Ok(raw);
    }
    // Resolve relative path against CWD.
    let cwd = crate::sched::with_current_task(|task| {
        task.cwd.clone().ok_or(SyscallError::InvalidArgument)
    })?;
    let mut parts: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
    for component in raw.split('/') {
        match component {
            "" | "." => {}
            ".." => { let _ = parts.pop(); }
            c => parts.push(c),
        }
    }
    let mut resolved = String::from("/");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            resolved.push('/');
        }
        resolved.push_str(part);
    }
    Ok(resolved)
}

/// Convert Linux O_* flags integer to our OpenFlags.
///
/// Linux flags: O_RDONLY=0, O_WRONLY=1, O_RDWR=2, O_CREAT=0x40,
/// O_TRUNC=0x200, O_APPEND=0x400.
const fn linux_flags_to_open_flags(flags: usize) -> OpenFlags {
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
// Default stdio helpers (serial + framebuffer)
// ---------------------------------------------------------------------------

/// Default stdin: blocking read from the serial console.
fn default_stdin_read(ptr: *mut u8, len: usize) -> SyscallResult {
    let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len) };

    loop {
        let n = crate::drivers::serial::read(buf, len);
        if n > 0 {
            return Ok(n);
        }
        let my_tid = crate::sched::current_task_id().unwrap_or(crate::sched::TaskId(0));
        crate::drivers::serial::SERIAL_READER_TID.store(my_tid.0, Ordering::Release);
        crate::sched::block_current_if_interruptible(|| {
            if crate::drivers::serial::is_empty() {
                true
            } else {
                crate::drivers::serial::SERIAL_READER_TID.store(0, Ordering::Release);
                false
            }
        })?;
    }
}

/// Default stdout/stderr: write to serial + framebuffer console.
#[allow(clippy::unnecessary_wraps)]
fn default_console_write(ptr: *const u8, len: usize) -> SyscallResult {
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
    Ok(len)
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

    // Check the fd table — pipes and VFS fds override default stdio.
    let kind = crate::sched::with_current_fd_table(|fdt| fdt.get_kind(fd))?;

    match kind {
        Some(FdKind::Pipe(h)) => {
            let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
            crate::ipc::pipe::pipe_read(h.pipe_id, buf)
        }
        Some(FdKind::Vfs(h)) => {
            let buf = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
            let n = crate::fs::vfs::read(h, buf)?;
            Ok(n)
        }
        None => {
            // Default stdio: fd 0 = serial read, fd 1/2 = not readable.
            match fd {
                0 => default_stdin_read(ptr, len),
                // fd 1/2 are not readable; any other fd is unreachable
                // (get_kind returns Err for unknown fds).
                _ => Err(SyscallError::InvalidArgument),
            }
        }
    }
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

    let kind = crate::sched::with_current_fd_table(|fdt| fdt.get_kind(fd))?;

    match kind {
        Some(FdKind::Pipe(h)) => {
            let buf = unsafe { core::slice::from_raw_parts(ptr, len) };
            crate::ipc::pipe::pipe_write(h.pipe_id, buf)
        }
        Some(FdKind::Vfs(h)) => {
            let buf = unsafe { core::slice::from_raw_parts(ptr, len) };
            let n = crate::fs::vfs::write(h, buf)?;
            Ok(n)
        }
        None => {
            // Default stdio: fd 1/2 = console write, fd 0 = not writable.
            match fd {
                1 | 2 => default_console_write(ptr, len),
                // fd 0 is not writable; any other fd is unreachable
                // (get_kind returns Err for unknown fds).
                _ => Err(SyscallError::InvalidArgument),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// sys_open(path_ptr, flags, mode) -> fd
// ---------------------------------------------------------------------------

fn sys_open(args: &[usize; 6]) -> SyscallResult {
    let path = copy_and_resolve_path(args[0])?;
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

    // Check if this is a default stdio slot (None in the table).
    let kind = crate::sched::with_current_fd_table(|fdt| fdt.get_kind(fd));
    match kind {
        Ok(None) | Err(_) => return Ok(0), // closing default stdio or already closed
        Ok(Some(_)) => {}                  // has a real entry — close it below
    }

    // Extract the entry from the table (under scheduler lock).
    let kind = crate::sched::with_current_fd_table(|fdt| fdt.close(fd))?;

    // Close the underlying resource outside the scheduler lock.
    match kind {
        FdKind::Vfs(h) => {
            crate::fs::vfs::close(h)?;
        }
        FdKind::Pipe(h) => {
            crate::ipc::pipe::pipe_close(h);
        }
    }
    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_pipe(fds_ptr) -> 0
// ---------------------------------------------------------------------------

fn sys_pipe(args: &[usize; 6]) -> SyscallResult {
    let fds_ptr = args[0];
    validate_user_ptr(fds_ptr)?;

    let (read_handle, write_handle) =
        crate::ipc::pipe::pipe_create().ok_or(SyscallError::NoMemory)?;

    let read_fd = crate::sched::with_current_fd_table(|fdt| fdt.alloc_fd_pipe(read_handle))?;

    let write_fd = match crate::sched::with_current_fd_table(|fdt| fdt.alloc_fd_pipe(write_handle))
    {
        Ok(fd) => fd,
        Err(e) => {
            // Rollback: close the read fd and both pipe ends.
            let _ = crate::sched::with_current_fd_table(|fdt| fdt.close(read_fd));
            crate::ipc::pipe::pipe_close(read_handle);
            crate::ipc::pipe::pipe_close(write_handle);
            return Err(e);
        }
    };

    // Write [read_fd, write_fd] to user space.
    let dst = fds_ptr as *mut [i32; 2];
    unsafe {
        (*dst)[0] = read_fd as i32;
        (*dst)[1] = write_fd as i32;
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_dup2(old_fd, new_fd) -> new_fd
// ---------------------------------------------------------------------------

fn sys_dup2(args: &[usize; 6]) -> SyscallResult {
    let old_fd = args[0];
    let new_fd = args[1];

    if new_fd >= 256 {
        return Err(SyscallError::InvalidArgument);
    }

    if old_fd == new_fd {
        // POSIX: if old_fd == new_fd and old_fd is valid, just return it.
        // Validate that old_fd is open.
        let _ = crate::sched::with_current_fd_table(|fdt| fdt.get_kind(old_fd))?;
        return Ok(new_fd);
    }

    // Duplicate the underlying resource: VFS handles need a real dup
    // (new open file entry), pipes just need a refcount bump.
    // This must happen outside the scheduler lock for VFS (acquires VFS lock).
    let source_kind = crate::sched::with_current_fd_table(|fdt| {
        let kind = fdt.get_kind(old_fd)?;
        kind.ok_or(SyscallError::InvalidArgument)
    })?;

    let new_kind = match source_kind {
        FdKind::Vfs(h) => {
            let new_h = crate::fs::vfs::dup(h).map_err(|_| SyscallError::IoError)?;
            FdKind::Vfs(new_h)
        }
        FdKind::Pipe(h) => {
            crate::ipc::pipe::pipe_clone(&h);
            FdKind::Pipe(h)
        }
    };

    // Under the scheduler lock: place the new entry at new_fd.
    let evicted = crate::sched::with_current_fd_table(|fdt| {
        let old = fdt.set_fd(new_fd, new_kind);
        Ok(old)
    })?;

    // Close evicted resource outside the scheduler lock.
    if let Some(kind) = evicted {
        match kind {
            FdKind::Vfs(h) => {
                let _ = crate::fs::vfs::close(h);
            }
            FdKind::Pipe(h) => {
                crate::ipc::pipe::pipe_close(h);
            }
        }
    }

    Ok(new_fd)
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
    let path = copy_and_resolve_path(args[0])?;
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
    let path = copy_and_resolve_path(args[0])?;
    crate::fs::vfs::mkdir(&path)?;
    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_unlink(path_ptr) -> 0
// ---------------------------------------------------------------------------

fn sys_unlink(args: &[usize; 6]) -> SyscallResult {
    let path = copy_and_resolve_path(args[0])?;
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
    let path = copy_and_resolve_path(args[0])?;
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
// sys_getpid() -> pid
// ---------------------------------------------------------------------------

fn sys_getpid() -> SyscallResult {
    let tid = crate::sched::current_task_id().ok_or(SyscallError::IoError)?;
    Ok(tid.0 as usize)
}

// ---------------------------------------------------------------------------
// sys_getppid() -> parent pid
// ---------------------------------------------------------------------------

fn sys_getppid() -> SyscallResult {
    crate::sched::with_current_task(|task| Ok(task.parent.map_or(0, |p| p.0 as usize)))
}

// ---------------------------------------------------------------------------
// sys_getcwd(buf_ptr, buf_len) -> bytes_written (including NUL)
// ---------------------------------------------------------------------------

fn sys_getcwd(args: &[usize; 6]) -> SyscallResult {
    let buf_ptr = args[0];
    let buf_len = args[1];

    validate_user_ptr(buf_ptr)?;

    let cwd = crate::sched::with_current_task(|task| {
        task.cwd.clone().ok_or(SyscallError::InvalidArgument)
    })?;

    let cwd_bytes = cwd.as_bytes();
    let needed = cwd_bytes.len() + 1; // +1 for NUL

    if buf_len < needed {
        return Err(SyscallError::InvalidArgument);
    }

    let dst = buf_ptr as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(cwd_bytes.as_ptr(), dst, cwd_bytes.len());
        *dst.add(cwd_bytes.len()) = 0; // NUL terminator
    }

    Ok(needed)
}

// ---------------------------------------------------------------------------
// sys_chdir(path_ptr) -> 0
// ---------------------------------------------------------------------------

fn sys_chdir(args: &[usize; 6]) -> SyscallResult {
    let path = copy_and_resolve_path(args[0])?;

    // Verify the path is a directory.
    let stat = crate::fs::vfs::stat(&path)?;
    if stat.file_type != crate::fs::traits::FileType::Directory {
        return Err(SyscallError::NotADirectory);
    }

    crate::sched::with_current_task(|task| {
        task.cwd = Some(path);
        Ok(0)
    })
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

// ---------------------------------------------------------------------------
// sys_kill(pid, sig) -> 0
// ---------------------------------------------------------------------------

fn sys_kill(args: &[usize; 6]) -> SyscallResult {
    let pid = args[0] as u64;
    let sig = args[1] as u8;

    if sig == 0 || (sig as usize) >= crate::ipc::signal::NSIG {
        return Err(SyscallError::InvalidArgument);
    }

    let tid = crate::sched::traits::TaskId(pid);
    if crate::sched::send_signal_to(tid, sig) {
        Ok(0)
    } else {
        Err(SyscallError::NotFound)
    }
}

// ---------------------------------------------------------------------------
// sys_sigaction(sig, new_act_ptr, old_act_ptr) -> 0
// ---------------------------------------------------------------------------

fn sys_sigaction(args: &[usize; 6]) -> SyscallResult {
    use crate::ipc::signal::{NSIG, SIG_DFL, SIG_IGN, SIGKILL, SigAction, SigActionUser};

    let sig = args[0] as u8;
    let new_ptr = args[1];
    let old_ptr = args[2];

    // Validate signal number.
    if sig == 0 || (sig as usize) >= NSIG {
        return Err(SyscallError::InvalidArgument);
    }
    // Cannot change disposition of SIGKILL.
    if sig == SIGKILL {
        return Err(SyscallError::InvalidArgument);
    }

    // Read the new action from user space (if provided).
    let new_action = if new_ptr != 0 {
        validate_user_ptr(new_ptr)?;
        let user_act = unsafe { &*(new_ptr as *const SigActionUser) };
        Some(match user_act.handler {
            SIG_DFL => SigAction::Default,
            SIG_IGN => SigAction::Ignore,
            addr => SigAction::Handler(addr),
        })
    } else {
        None
    };

    // Under scheduler lock: swap action.
    let old_action = crate::sched::with_current_task(|task| {
        let sig_state = task.signals.as_mut().ok_or(SyscallError::InvalidArgument)?;

        let old = sig_state.actions[sig as usize];

        if let Some(new) = new_action {
            sig_state.actions[sig as usize] = new;
        }

        Ok(old)
    })?;

    // Write old action to user space (if requested).
    if old_ptr != 0 {
        validate_user_ptr(old_ptr)?;
        let (handler, mask) = match old_action {
            SigAction::Default => (SIG_DFL, 0u32),
            SigAction::Ignore => (SIG_IGN, 0u32),
            SigAction::Handler(addr) => (addr, 0u32),
        };
        let dst = old_ptr as *mut SigActionUser;
        unsafe {
            (*dst).handler = handler;
            (*dst).flags = 0;
            (*dst).mask = mask;
            (*dst).pad = 0;
        }
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_sigprocmask(how, set_ptr, oldset_ptr) -> 0
// ---------------------------------------------------------------------------

/// sigprocmask `how` values (Linux-compatible).
const SIG_BLOCK: usize = 0;
const SIG_UNBLOCK: usize = 1;
const SIG_SETMASK: usize = 2;

fn sys_sigprocmask(args: &[usize; 6]) -> SyscallResult {
    use crate::ipc::signal::{SIGKILL, SignalState};

    let how = args[0];
    let set_ptr = args[1];
    let oldset_ptr = args[2];

    if how > SIG_SETMASK {
        return Err(SyscallError::InvalidArgument);
    }

    // Read the new mask from user space (if provided).
    let new_mask_val = if set_ptr != 0 {
        validate_user_ptr(set_ptr)?;
        Some(unsafe { *(set_ptr as *const u32) })
    } else {
        None
    };

    // Under scheduler lock: read old mask, apply new mask.
    let old_mask = crate::sched::with_current_task(|task| {
        let sig_state = task.signals.as_mut().ok_or(SyscallError::InvalidArgument)?;

        let old = sig_state.blocked;

        if let Some(set) = new_mask_val {
            // SIGKILL can never be blocked.
            let kill_bit = SignalState::sig_bit(SIGKILL);

            match how {
                SIG_BLOCK => sig_state.blocked |= set & !kill_bit,
                SIG_UNBLOCK => sig_state.blocked &= !set,
                SIG_SETMASK => sig_state.blocked = set & !kill_bit,
                _ => {}
            }
        }

        Ok(old)
    })?;

    // Write old mask to user space (if requested).
    if oldset_ptr != 0 {
        validate_user_ptr(oldset_ptr)?;
        unsafe {
            *(oldset_ptr as *mut u32) = old_mask;
        }
    }

    Ok(0)
}

// ---------------------------------------------------------------------------
// sys_brk(new_brk) -> current_brk
// ---------------------------------------------------------------------------
//
// If new_brk == 0, returns the current program break.
// Otherwise, grows or shrinks the heap by mapping/unmapping pages.

fn sys_brk(args: &[usize; 6]) -> SyscallResult {
    use crate::mm::addr::VirtAddr;
    use crate::mm::page::PAGE_SIZE;
    use crate::mm::traits::PageFlags;

    let requested = args[0] as u64;

    // brk(0): query current break.
    if requested == 0 {
        return crate::sched::with_current_task(|t| Ok(t.brk as usize));
    }

    // Page-align the request upward.
    let page_mask = (PAGE_SIZE as u64) - 1;
    let new_brk = (requested + page_mask) & !page_mask;

    let cur_brk = crate::sched::with_current_task(|t| Ok(t.brk as usize))? as u64;

    if new_brk == cur_brk {
        return Ok(new_brk as usize);
    }

    if new_brk > cur_brk {
        // Growing: allocate and map new pages.
        let mut addr = cur_brk;
        while addr < new_brk {
            let frame = crate::mm::frame::alloc_frame().ok_or(SyscallError::NoMemory)?;

            // Zero the frame.
            unsafe {
                let dst = crate::mm::addr::phys_to_virt(frame).as_mut_ptr::<u8>();
                core::ptr::write_bytes(dst, 0, PAGE_SIZE);
            }

            // Map into the current task's address space.
            let result = crate::sched::with_current_task(|task| {
                let aspace = task.aspace.as_mut().ok_or(SyscallError::InvalidArgument)?;
                aspace
                    .map(VirtAddr::new(addr), frame, PageFlags::USER_RW)
                    .map_err(|_| SyscallError::NoMemory)?;
                Ok(())
            });

            if let Err(e) = result {
                crate::mm::frame::dealloc_frame(frame);
                return Err(e);
            }

            addr += PAGE_SIZE as u64;
        }

        // Update the task's brk.
        crate::sched::with_current_task(|t| {
            t.brk = new_brk;
            Ok(new_brk as usize)
        })
    } else {
        // Shrinking: unmap pages from new_brk to cur_brk.
        let mut addr = new_brk;
        while addr < cur_brk {
            let _ = crate::sched::with_current_task(|task| {
                let aspace = task.aspace.as_mut().ok_or(SyscallError::InvalidArgument)?;
                match aspace.unmap(VirtAddr::new(addr)) {
                    Ok(phys) => {
                        crate::mm::frame::dealloc_frame(phys);
                        Ok(())
                    }
                    Err(_) => Ok(()), // page wasn't mapped, skip
                }
            });
            addr += PAGE_SIZE as u64;
        }

        // Update the task's brk.
        crate::sched::with_current_task(|t| {
            t.brk = new_brk;
            Ok(new_brk as usize)
        })
    }
}
