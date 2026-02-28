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
const SYS_EXIT: usize = 60;
const SYS_MKDIR: usize = 83;
const SYS_UNLINK: usize = 87;
const SYS_READDIR: usize = 217;

// Linux O_* flags.
pub const O_RDONLY: usize = 0;
pub const O_WRONLY: usize = 1;
pub const O_CREAT: usize = 0x40;
pub const O_TRUNC: usize = 0x200;

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

pub fn sys_read(fd: usize, buf: &mut [u8]) -> isize {
    unsafe { syscall3(SYS_READ, fd, buf.as_mut_ptr() as usize, buf.len()) }
}

pub fn sys_write(fd: usize, buf: &[u8]) -> isize {
    unsafe { syscall3(SYS_WRITE, fd, buf.as_ptr() as usize, buf.len()) }
}

pub fn sys_open(path: &str, flags: usize) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall2(SYS_OPEN, ptr as usize, flags)
    })
}

pub fn sys_close(fd: usize) -> isize {
    unsafe { syscall1(SYS_CLOSE, fd) }
}

pub fn sys_stat(path: &str, statbuf: &mut StatBuf) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall2(SYS_STAT, ptr as usize, statbuf as *mut StatBuf as usize)
    })
}

pub fn sys_mkdir(path: &str) -> isize {
    with_cstr(path, |ptr| unsafe {
        syscall2(SYS_MKDIR, ptr as usize, 0o755)
    })
}

pub fn sys_unlink(path: &str) -> isize {
    with_cstr(path, |ptr| unsafe { syscall1(SYS_UNLINK, ptr as usize) })
}

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

pub fn sys_exit(code: usize) -> ! {
    unsafe {
        syscall1(SYS_EXIT, code);
    }
    loop {}
}

// ---------------------------------------------------------------------------
// Types matching kernel structs
// ---------------------------------------------------------------------------

/// Matches the kernel's `UserStat` layout in dispatch.rs.
#[repr(C)]
pub struct StatBuf {
    pub st_ino: u64,
    pub st_mode: u32,
    pub _pad0: u32,
    pub st_size: u64,
    pub _pad1: [u8; 128],
}

#[allow(dead_code)]
impl StatBuf {
    pub const fn zeroed() -> Self {
        Self {
            st_ino: 0,
            st_mode: 0,
            _pad0: 0,
            st_size: 0,
            _pad1: [0u8; 128],
        }
    }

    pub fn is_dir(&self) -> bool {
        (self.st_mode & 0o170000) == 0o040000
    }

    pub fn is_file(&self) -> bool {
        (self.st_mode & 0o170000) == 0o100000
    }

    pub fn type_str(&self) -> &'static str {
        let fmt = self.st_mode & 0o170000;
        match fmt {
            0o100000 => "file",
            0o040000 => "dir",
            0o120000 => "link",
            0o020000 => "cdev",
            0o060000 => "bdev",
            0o010000 => "fifo",
            0o140000 => "sock",
            _ => "???",
        }
    }
}

/// Matches the kernel's `UserDirEntry` layout in dispatch.rs.
#[repr(C)]
pub struct DirEntry {
    pub d_ino: u64,
    pub d_type: u8,
    pub d_name_len: u8,
    pub _pad: [u8; 6],
    pub d_name: [u8; 256],
}

#[allow(dead_code)]
impl DirEntry {
    pub fn name(&self) -> &[u8] {
        &self.d_name[..self.d_name_len as usize]
    }

    pub fn type_indicator(&self) -> &'static str {
        match self.d_type {
            4 => "/", // DT_DIR
            _ => "",
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Copy a Rust `&str` to a NUL-terminated buffer on the stack, then call `f`.
fn with_cstr(s: &str, f: impl FnOnce(*const u8) -> isize) -> isize {
    let bytes = s.as_bytes();
    if bytes.len() >= 255 {
        return -22; // EINVAL
    }
    let mut buf = [0u8; 256];
    buf[..bytes.len()].copy_from_slice(bytes);
    buf[bytes.len()] = 0;
    f(buf.as_ptr())
}
