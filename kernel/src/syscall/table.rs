//! System call number definitions (Linux x86_64 compatible).

pub const SYS_READ: usize = 0;
pub const SYS_WRITE: usize = 1;
pub const SYS_OPEN: usize = 2;
pub const SYS_CLOSE: usize = 3;
pub const SYS_STAT: usize = 4;
pub const SYS_BRK: usize = 12;
pub const SYS_FORK: usize = 57;
pub const SYS_EXECVE: usize = 59;
pub const SYS_EXIT: usize = 60;
pub const SYS_WAIT4: usize = 61;
pub const SYS_GETCWD: usize = 79;
pub const SYS_CHDIR: usize = 80;
pub const SYS_MKDIR: usize = 83;
pub const SYS_UNLINK: usize = 87;
pub const SYS_PIPE: usize = 22;
pub const SYS_DUP2: usize = 33;
pub const SYS_SIGACTION: usize = 13;
pub const SYS_SIGPROCMASK: usize = 14;
pub const SYS_SIGRETURN: usize = 15;
pub const SYS_KILL: usize = 62;
pub const SYS_GETPID: usize = 39;
pub const SYS_GETPPID: usize = 110;
pub const SYS_READDIR: usize = 217;
