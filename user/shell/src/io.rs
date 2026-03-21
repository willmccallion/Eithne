use crate::syscall::sys_write;

pub const LINE_MAX: usize = 256;
pub const STDOUT: usize = 1;
pub const STDIN: usize = 0;

// ANSI escape helpers
pub const CLR_RESET: &str = "\x1b[0m";
pub const CLR_BOLD: &str = "\x1b[1m";
pub const CLR_RED: &str = "\x1b[1;31m";
pub const CLR_GREEN: &str = "\x1b[1;32m";
pub const CLR_BLUE: &str = "\x1b[1;34m";
pub const CLR_CYAN: &str = "\x1b[1;36m";

pub const COMMANDS: &[&[u8]] = &[
    b"alloc", b"cat", b"cd", b"clear", b"echo", b"exit", b"help", b"kill", b"ls", b"mkdir",
    b"pwd", b"rm", b"run", b"stat", b"write",
];

pub fn print(s: &str) {
    let _ = sys_write(STDOUT, s.as_bytes());
}

pub fn print_bytes(b: &[u8]) {
    let _ = sys_write(STDOUT, b);
}

pub fn println(s: &str) {
    print(s);
    print("\n");
}

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

pub fn print_isize(v: isize) {
    if v < 0 {
        print("-");
        print_num(-(v as i64) as u64);
    } else {
        print_num(v as u64);
    }
}

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
