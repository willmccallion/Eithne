//! System call number definitions (Linux x86_64 compatible).

pub const SYS_READ: usize = 0;
pub const SYS_WRITE: usize = 1;
pub const SYS_OPEN: usize = 2;
pub const SYS_CLOSE: usize = 3;
pub const SYS_STAT: usize = 4;
pub const SYS_EXIT: usize = 60;
pub const SYS_MKDIR: usize = 83;
pub const SYS_UNLINK: usize = 87;
pub const SYS_READDIR: usize = 217;
