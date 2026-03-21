//! Raw system call wrappers for eithne.
//!
//! Uses the Linux x86_64 syscall ABI:
//!   RAX = syscall number
//!   RDI, RSI, RDX, R10, R8, R9 = arguments
//!   RAX = return value (negative errno on error)

use core::arch::asm;

// Syscall numbers (must match kernel/src/syscall/table.rs).
const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_OPEN: usize = 2;
const SYS_CLOSE: usize = 3;
const SYS_STAT: usize = 4;
const SYS_FORK: usize = 57;
const SYS_EXECVE: usize = 59;
const SYS_EXIT: usize = 60;
const SYS_WAIT4: usize = 61;
const SYS_MKDIR: usize = 83;
const SYS_UNLINK: usize = 87;
const SYS_SIGACTION: usize = 13;
const SYS_PIPE: usize = 22;
const SYS_DUP2: usize = 33;
const SYS_KILL: usize = 62;
const SYS_CHDIR: usize = 80;
const SYS_BRK: usize = 12;
const SYS_READDIR: usize = 217;

// ---------------------------------------------------------------------------
// Linux O_* flags
// ---------------------------------------------------------------------------

/// Open file for reading only.
pub const O_RDONLY: usize = 0;
/// Open file for writing only.
pub const O_WRONLY: usize = 1;
/// Create the file if it does not exist.
pub const O_CREAT: usize = 0x40;
/// Truncate the file to zero length on open.
pub const O_TRUNC: usize = 0x200;
/// Append writes to the end of the file.
pub const O_APPEND: usize = 0x400;

/// Linux directory entry type for directories.
pub const DT_DIR: u8 = 4;

// ---------------------------------------------------------------------------
// Inline syscall helpers
// ---------------------------------------------------------------------------

#[allow(clippy::inline_always)]
#[inline(always)]
unsafe fn syscall0(num: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") num as isize => ret,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

#[allow(clippy::inline_always)]
#[inline(always)]
unsafe fn syscall1(num: usize, a1: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") num as isize => ret,
            in("rdi") a1,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

#[allow(clippy::inline_always)]
#[inline(always)]
unsafe fn syscall2(num: usize, a1: usize, a2: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") num as isize => ret,
            in("rdi") a1,
            in("rsi") a2,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

#[allow(clippy::inline_always)]
#[inline(always)]
unsafe fn syscall3(num: usize, a1: usize, a2: usize, a3: usize) -> isize {
    let ret: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") num as isize => ret,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            out("rcx") _,
            out("r11") _,
            options(nostack),
        );
    }
    ret
}

// ---------------------------------------------------------------------------
// Public wrappers
// ---------------------------------------------------------------------------

/// Read from a file descriptor into `buf`. Returns bytes read or negative errno.
pub fn sys_read(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len()) }
}

/// Write `buf` to a file descriptor. Returns bytes written or negative errno.
#[must_use]
pub fn sys_write(fd: usize, buf: &[u8]) -> isize {
    unsafe { syscall3(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len()) }
}

/// Open a file by path. Returns file descriptor or negative errno.
#[must_use]
pub fn sys_open(path: &str, flags: usize) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall2(SYS_OPEN, ptr as usize, flags)
    })
}

/// Close a file descriptor.
#[must_use]
pub fn sys_close(fd: usize) -> isize {
    unsafe { syscall1(SYS_CLOSE, fd) }
}

/// Get file status. Returns 0 on success or negative errno.
pub fn sys_stat(path: &str, statbuf: &mut StatBuf) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall2(SYS_STAT, ptr as usize, statbuf as *mut StatBuf as usize)
    })
}

/// Create a directory. Returns 0 on success or negative errno.
#[must_use]
pub fn sys_mkdir(path: &str) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall2(SYS_MKDIR, ptr as usize, 0o755)
    })
}

/// Remove a file. Returns 0 on success or negative errno.
#[must_use]
pub fn sys_unlink(path: &str) -> isize {
    with_cstr(path, |ptr| unsafe { syscall1(SYS_UNLINK, ptr as usize) })
}

/// Read directory entries. Returns bytes of entries or negative errno.
pub fn sys_readdir(path: &str, buf: &mut [u8]) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall3(
            SYS_READDIR,
            ptr as usize,
            buf.as_mut_ptr() as usize,
            buf.len(),
        )
    })
}

/// Terminate the current process with the given exit code.
pub fn sys_exit(code: usize) -> ! {
    unsafe {
        let _ = syscall1(SYS_EXIT, code);
    }
    #[allow(clippy::empty_loop)]
    loop {}
}

/// Fork the current process. Returns 0 in child, child PID in parent, or negative errno.
#[must_use]
pub fn sys_fork() -> isize {
    unsafe { syscall0(SYS_FORK) }
}

/// Execute a program, replacing the current process image.
///
/// `argv` must be a NULL-terminated array of pointers to NUL-terminated strings.
/// Pass `core::ptr::null()` for no arguments.
#[must_use]
pub fn sys_exec(path: &str, argv: *const *const u8) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall2(SYS_EXECVE, ptr as usize, argv as usize)
    })
}

/// Wait for a child process.
///
/// `pid == usize::MAX` (i.e. -1 as unsigned) means wait for any child.
pub fn sys_waitpid(pid: usize, status: &mut i32) -> isize {
    unsafe { syscall2(SYS_WAIT4, pid, status as *mut i32 as usize) }
}

/// Create a pipe. Writes `[read_fd, write_fd]` into `fds`.
pub fn sys_pipe(fds: &mut [i32; 2]) -> isize {
    unsafe { syscall1(SYS_PIPE, fds.as_mut_ptr() as usize) }
}

/// Duplicate `old_fd` to `new_fd`, closing `new_fd` first if open.
#[must_use]
pub fn sys_dup2(old_fd: usize, new_fd: usize) -> isize {
    unsafe { syscall2(SYS_DUP2, old_fd, new_fd) }
}

/// Send a signal to a process.
#[must_use]
pub fn sys_kill(pid: usize, sig: usize) -> isize {
    unsafe { syscall2(SYS_KILL, pid, sig) }
}

/// Register a signal handler.
#[must_use]
pub fn sys_sigaction(sig: usize, new_act: usize, old_act: usize) -> isize {
    unsafe { syscall3(SYS_SIGACTION, sig, new_act, old_act) }
}

/// Change the current working directory.
#[must_use]
pub fn sys_chdir(path: &str) -> isize {
    with_cstr(path, |ptr| unsafe { syscall1(SYS_CHDIR, ptr as usize) })
}

// ---------------------------------------------------------------------------
// brk / sbrk
// ---------------------------------------------------------------------------

/// Adjust the program break by `increment` bytes.
///
/// Returns a pointer to the start of the newly allocated region on success,
/// or a null pointer on failure.  If `increment` is 0, returns the current
/// break without changing it.
#[must_use]
pub fn sys_sbrk(increment: isize) -> *mut u8 {
    let current = unsafe { syscall1(SYS_BRK, 0) };
    if current < 0 {
        return core::ptr::null_mut();
    }
    let current = current as usize;
    if increment == 0 {
        return current as *mut u8;
    }
    let new_brk = if increment > 0 {
        current + increment as usize
    } else {
        current.wrapping_sub((-increment) as usize)
    };
    let result = unsafe { syscall1(SYS_BRK, new_brk) };
    if result < 0 {
        core::ptr::null_mut()
    } else {
        current as *mut u8 // return OLD break (start of newly allocated region)
    }
}

// ---------------------------------------------------------------------------
// Signal constants
// ---------------------------------------------------------------------------

/// Interrupt signal (Ctrl+C).
pub const SIGINT: usize = 2;
/// Broken pipe signal.
pub const SIGPIPE: usize = 13;
/// Termination signal.
pub const SIGTERM: usize = 15;

/// User-space sigaction struct (must match kernel's `SigActionUser` layout).
#[repr(C)]
#[derive(Debug)]
pub struct SigActionUser {
    /// Signal handler address (or `SIG_DFL`/`SIG_IGN`).
    pub handler: u64,
    /// Signal action flags.
    pub flags: u64,
    /// Signal mask during handler execution.
    pub mask: u32,
    /// Padding for alignment.
    pub pad: u32,
}

/// Default signal disposition sentinel.
pub const SIG_DFL: u64 = 0;
/// Ignore signal sentinel.
pub const SIG_IGN: u64 = 1;

// ---------------------------------------------------------------------------
// Unix file type constants (S_IF* from <sys/stat.h>)
// ---------------------------------------------------------------------------

/// File type mask.
pub const S_IFMT: u32 = 0o170000;
/// FIFO (named pipe).
pub const S_IFIFO: u32 = 0o010000;
/// Character device.
pub const S_IFCHR: u32 = 0o020000;
/// Directory.
pub const S_IFDIR: u32 = 0o040000;
/// Block device.
pub const S_IFBLK: u32 = 0o060000;
/// Regular file.
pub const S_IFREG: u32 = 0o100000;
/// Symbolic link.
pub const S_IFLNK: u32 = 0o120000;
/// Socket.
pub const S_IFSOCK: u32 = 0o140000;

// ---------------------------------------------------------------------------
// Types matching kernel structs
// ---------------------------------------------------------------------------

/// File status buffer (matches the kernel's `UserStat` layout).
#[repr(C)]
#[derive(Debug)]
pub struct StatBuf {
    /// Inode number.
    pub st_ino: u64,
    /// File mode (type + permissions).
    pub st_mode: u32,
    pad0: u32,
    /// File size in bytes.
    pub st_size: u64,
    pad1: [u8; 128],
}

#[allow(dead_code)]
impl StatBuf {
    /// Create a zeroed `StatBuf`.
    #[must_use]
    pub const fn zeroed() -> Self {
        Self {
            st_ino: 0,
            st_mode: 0,
            pad0: 0,
            st_size: 0,
            pad1: [0u8; 128],
        }
    }

    /// Returns true if this is a directory.
    #[must_use]
    pub const fn is_dir(&self) -> bool {
        (self.st_mode & S_IFMT) == S_IFDIR
    }

    /// Returns true if this is a regular file.
    #[must_use]
    pub const fn is_file(&self) -> bool {
        (self.st_mode & S_IFMT) == S_IFREG
    }

    /// Returns a human-readable file type string.
    #[must_use]
    pub const fn type_str(&self) -> &'static str {
        let fmt = self.st_mode & S_IFMT;
        match fmt {
            S_IFREG => "file",
            S_IFDIR => "dir",
            S_IFLNK => "link",
            S_IFCHR => "cdev",
            S_IFBLK => "bdev",
            S_IFIFO => "fifo",
            S_IFSOCK => "sock",
            _ => "???",
        }
    }
}

/// Directory entry (matches the kernel's `UserDirEntry` layout).
#[repr(C)]
pub struct DirEntry {
    /// Inode number.
    pub d_ino: u64,
    /// Entry type (e.g. `DT_DIR`).
    pub d_type: u8,
    /// Length of the name in bytes.
    pub d_name_len: u8,
    pad: [u8; 6],
    /// File name bytes (not NUL-terminated).
    pub d_name: [u8; 256],
}

// Manual Debug impl because [u8; 256] doesn't implement Debug.
impl core::fmt::Debug for DirEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DirEntry")
            .field("d_ino", &self.d_ino)
            .field("d_type", &self.d_type)
            .field("d_name", &&self.d_name[..self.d_name_len as usize])
            .finish_non_exhaustive()
    }
}

#[allow(dead_code)]
impl DirEntry {
    /// Returns the entry name as a byte slice.
    #[must_use]
    pub fn name(&self) -> &[u8] {
        &self.d_name[..self.d_name_len as usize]
    }

    /// Returns "/" for directories, "" for other types.
    #[must_use]
    pub const fn type_indicator(&self) -> &'static str {
        match self.d_type {
            DT_DIR => "/",
            _ => "",
        }
    }
}

// ---------------------------------------------------------------------------
// Errno constants
// ---------------------------------------------------------------------------

/// Negative `EINTR` — syscall interrupted by signal.
pub const NEG_EINTR: isize = -4;

/// Negative `EINVAL` — invalid argument (e.g. path too long).
const NEG_EINVAL: isize = -22;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy a Rust `&str` to a NUL-terminated buffer on the stack, then call `f`.
fn with_cstr(s: &str, f: impl FnOnce(*const u8) -> isize) -> isize {
    let bytes = s.as_bytes();
    if bytes.len() >= 255 {
        return NEG_EINVAL;
    }
    let mut buf = [0u8; 256];
    buf[..bytes.len()].copy_from_slice(bytes);
    buf[bytes.len()] = 0;
    f(buf.as_ptr())
}
