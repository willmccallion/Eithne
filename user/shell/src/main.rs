#![no_std]
#![no_main]

mod syscall;

use core::mem;
use syscall::*;

const LINE_MAX: usize = 256;
const STDOUT: usize = 1;
const STDIN: usize = 0;

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
#[unsafe(naked)]
extern "C" fn _start() -> ! {
    core::arch::naked_asm!("call main", "ud2")
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    print("eithne shell v0.2\n");

    let mut line = [0u8; LINE_MAX];
    loop {
        print("> ");
        let len = read_line(&mut line);
        if len == 0 {
            continue;
        }
        let input = &line[..len];
        dispatch_command(input);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    print("panic!\n");
    sys_exit(1);
}

// ---------------------------------------------------------------------------
// I/O helpers
// ---------------------------------------------------------------------------

fn print(s: &str) {
    sys_write(STDOUT, s.as_bytes());
}

fn print_bytes(b: &[u8]) {
    sys_write(STDOUT, b);
}

fn println(s: &str) {
    print(s);
    print("\n");
}

fn print_num(mut n: u64) {
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

fn print_isize(v: isize) {
    if v < 0 {
        print("-");
        print_num(-(v as i64) as u64);
    } else {
        print_num(v as u64);
    }
}

fn print_error(msg: &str, errno: isize) {
    print(msg);
    print(": error ");
    print_isize(errno);
    print("\n");
}

// ---------------------------------------------------------------------------
// Line reading
// ---------------------------------------------------------------------------

fn read_line(buf: &mut [u8; LINE_MAX]) -> usize {
    let mut len: usize = 0;
    let mut ch = [0u8; 1];

    loop {
        let n = sys_read(STDIN, &mut ch);
        if n <= 0 {
            continue;
        }

        let c = ch[0];
        match c {
            // Enter (CR or LF)
            b'\r' | b'\n' => {
                print("\n");
                return len;
            }
            // Backspace (BS or DEL)
            0x08 | 0x7F => {
                if len > 0 {
                    len -= 1;
                    // Erase character on terminal: BS, space, BS
                    print_bytes(&[0x08, b' ', 0x08]);
                }
            }
            // Normal printable character
            _ => {
                if len < LINE_MAX {
                    buf[len] = c;
                    len += 1;
                    // Echo
                    print_bytes(&[c]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

fn dispatch_command(input: &[u8]) {
    // Split into command and arguments at first space.
    let (cmd, args) = split_first(input);

    match cmd {
        b"help" => cmd_help(),
        b"echo" => cmd_echo(args),
        b"ls" => cmd_ls(args),
        b"cat" => cmd_cat(args),
        b"write" => cmd_write(args),
        b"mkdir" => cmd_mkdir(args),
        b"rm" => cmd_rm(args),
        b"stat" => cmd_stat(args),
        b"clear" => cmd_clear(),
        _ => {
            print("unknown command: ");
            print_bytes(cmd);
            print("\ntype 'help' for commands\n");
        }
    }
}

/// Split `input` into `(first_word, rest)`. `rest` may be empty.
fn split_first(input: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    // Find first space.
    while i < input.len() && input[i] != b' ' {
        i += 1;
    }
    let cmd = &input[..i];
    // Skip spaces.
    let mut j = i;
    while j < input.len() && input[j] == b' ' {
        j += 1;
    }
    let args = &input[j..];
    (cmd, args)
}

/// Convert a byte slice to a &str (unchecked, input is ASCII from our line reader).
fn bytes_to_str(b: &[u8]) -> &str {
    unsafe { core::str::from_utf8_unchecked(b) }
}

// ---------------------------------------------------------------------------
// Built-in commands
// ---------------------------------------------------------------------------

fn cmd_help() {
    print("commands:\n");
    print("  help              show this message\n");
    print("  echo [text]       print text\n");
    print("  ls [path]         list directory (default: /)\n");
    print("  cat <path>        print file contents\n");
    print("  write <path> <t>  write text to file\n");
    print("  mkdir <path>      create directory\n");
    print("  rm <path>         remove file\n");
    print("  stat <path>       show file info\n");
    print("  clear             clear screen\n");
}

fn cmd_echo(args: &[u8]) {
    print_bytes(args);
    print("\n");
}

fn cmd_ls(args: &[u8]) {
    let path = if args.is_empty() {
        "/"
    } else {
        bytes_to_str(args)
    };

    // Buffer for readdir results.
    const ENTRY_SIZE: usize = mem::size_of::<DirEntry>();
    const MAX_ENTRIES: usize = 64;
    let mut buf = [0u8; ENTRY_SIZE * MAX_ENTRIES];

    let ret = sys_readdir(path, &mut buf);
    if ret < 0 {
        print_error("ls", ret);
        return;
    }

    let bytes_returned = ret as usize;
    let count = bytes_returned / ENTRY_SIZE;

    if count == 0 {
        print("(empty)\n");
        return;
    }

    for i in 0..count {
        let offset = i * ENTRY_SIZE;
        let entry = unsafe { &*(buf.as_ptr().add(offset) as *const DirEntry) };
        print_bytes(entry.name());
        print(entry.type_indicator());
        print("\n");
    }
}

fn cmd_cat(args: &[u8]) {
    if args.is_empty() {
        println("usage: cat <path>");
        return;
    }
    let path = bytes_to_str(args);

    let fd = sys_open(path, O_RDONLY);
    if fd < 0 {
        print_error("cat: open", fd);
        return;
    }
    let fd = fd as usize;

    let mut buf = [0u8; 512];
    loop {
        let n = sys_read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        print_bytes(&buf[..n as usize]);
    }
    print("\n");

    sys_close(fd);
}

fn cmd_write(args: &[u8]) {
    if args.is_empty() {
        println("usage: write <path> <text>");
        return;
    }

    let (path_bytes, text) = split_first(args);
    if path_bytes.is_empty() || text.is_empty() {
        println("usage: write <path> <text>");
        return;
    }
    let path = bytes_to_str(path_bytes);

    let fd = sys_open(path, O_WRONLY | O_CREAT | O_TRUNC);
    if fd < 0 {
        print_error("write: open", fd);
        return;
    }
    let fd = fd as usize;

    let n = sys_write(fd, text);
    if n < 0 {
        print_error("write: write", n);
    }

    sys_close(fd);
}

fn cmd_mkdir(args: &[u8]) {
    if args.is_empty() {
        println("usage: mkdir <path>");
        return;
    }
    let path = bytes_to_str(args);
    let ret = sys_mkdir(path);
    if ret < 0 {
        print_error("mkdir", ret);
    }
}

fn cmd_rm(args: &[u8]) {
    if args.is_empty() {
        println("usage: rm <path>");
        return;
    }
    let path = bytes_to_str(args);
    let ret = sys_unlink(path);
    if ret < 0 {
        print_error("rm", ret);
    }
}

fn cmd_stat(args: &[u8]) {
    if args.is_empty() {
        println("usage: stat <path>");
        return;
    }
    let path = bytes_to_str(args);

    let mut st = StatBuf::zeroed();
    let ret = sys_stat(path, &mut st);
    if ret < 0 {
        print_error("stat", ret);
        return;
    }

    print("  type:  ");
    println(st.type_str());
    print("  size:  ");
    print_num(st.st_size);
    print("\n");
    print("  inode: ");
    print_num(st.st_ino);
    print("\n");
}

fn cmd_clear() {
    // ANSI escape: clear screen + cursor home
    print("\x1b[2J\x1b[H");
}
