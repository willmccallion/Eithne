//! Console I/O helpers for user-space programs.

use crate::syscall::sys_write;

/// Maximum line length for command input buffers.
pub const LINE_MAX: usize = 256;
/// File descriptor for standard output.
pub const STDOUT: usize = 1;
/// File descriptor for standard input.
pub const STDIN: usize = 0;

// ANSI escape helpers

/// Reset all terminal attributes.
pub const CLR_RESET: &str = "\x1b[0m";
/// Bold text.
pub const CLR_BOLD: &str = "\x1b[1m";
/// Bold red text.
pub const CLR_RED: &str = "\x1b[1;31m";
/// Bold green text.
pub const CLR_GREEN: &str = "\x1b[1;32m";
/// Bold blue text.
pub const CLR_BLUE: &str = "\x1b[1;34m";
/// Bold cyan text.
pub const CLR_CYAN: &str = "\x1b[1;36m";

/// Known command names for tab completion (must be sorted).
pub const COMMANDS: &[&[u8]] = &[
    b"cat", b"cd", b"clear", b"echo", b"exit", b"help", b"kill", b"ls", b"mkdir", b"pwd", b"rm",
    b"stat", b"write",
];

/// Print a string to stdout.
pub fn print(s: &str) {
    let _ = sys_write(STDOUT, s.as_bytes());
}

/// Print raw bytes to stdout.
pub fn print_bytes(b: &[u8]) {
    let _ = sys_write(STDOUT, b);
}

/// Print a string followed by a newline to stdout.
pub fn println(s: &str) {
    print(s);
    print("\n");
}

/// Print an unsigned 64-bit integer in decimal.
pub fn print_num(mut n: u64) {
    if n == 0 {
        print("0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = 20;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    print_bytes(&buf[i..]);
}

/// Print a signed integer in decimal.
pub fn print_isize(v: isize) {
    if v < 0 {
        print("-");
        print_num(-(v as i64) as u64);
    } else {
        print_num(v as u64);
    }
}

/// Print a 64-bit value in hexadecimal with `0x` prefix.
pub fn print_hex(mut n: u64) {
    if n == 0 {
        print("0x0");
        return;
    }
    let mut buf = [0u8; 18]; // "0x" + 16 hex digits
    buf[0] = b'0';
    buf[1] = b'x';
    let mut i = 18;
    while n > 0 {
        i -= 1;
        let digit = (n & 0xF) as u8;
        buf[i] = if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        };
        n >>= 4;
    }
    print_bytes(&buf[..2]); // "0x"
    print_bytes(&buf[i..]);
}

/// Print a colored error message with errno value.
pub fn print_error(msg: &str, errno: isize) {
    print(CLR_RED);
    print("error");
    print(CLR_RESET);
    print(": ");
    print(msg);
    print(": errno ");
    print_isize(errno);
    print("\n");
}
