#![no_std]
#![no_main]

mod syscall;

use core::mem;
use syscall::*;

const LINE_MAX: usize = 256;
const STDOUT: usize = 1;
const STDIN: usize = 0;

// ANSI escape helpers
const CLR_RESET: &str = "\x1b[0m";
const CLR_BOLD: &str = "\x1b[1m";
const CLR_RED: &str = "\x1b[1;31m";
const CLR_GREEN: &str = "\x1b[1;32m";
const CLR_BLUE: &str = "\x1b[1;34m";
const CLR_CYAN: &str = "\x1b[1;36m";

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
    print(CLR_CYAN);
    print("eithne");
    print(CLR_RESET);
    print(" shell v0.2\n\n");

    let mut cwd = [0u8; 256];
    cwd[0] = b'/';
    let mut cwd_len: usize = 1;

    let mut line = [0u8; LINE_MAX];
    loop {
        print_prompt(&cwd[..cwd_len]);
        let len = read_line(&mut line);
        if len == 0 {
            continue;
        }
        let input = &line[..len];
        dispatch_command(input, &mut cwd, &mut cwd_len);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    print("panic!\n");
    sys_exit(1);
}

// ---------------------------------------------------------------------------
// Prompt
// ---------------------------------------------------------------------------

fn print_prompt(cwd: &[u8]) {
    print(CLR_CYAN);
    print("eithne");
    print(CLR_RESET);
    print(":");
    print(CLR_BLUE);
    print_bytes(cwd);
    print(CLR_GREEN);
    print("$ ");
    print(CLR_RESET);
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
    print(CLR_RED);
    print("error");
    print(CLR_RESET);
    print(": ");
    print(msg);
    print(": errno ");
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
// Path resolution
// ---------------------------------------------------------------------------

/// Resolve `arg` relative to `cwd`, returning an absolute normalized path.
fn resolve_path(cwd: &[u8], arg: &[u8]) -> ([u8; 256], usize) {
    let mut tmp = [0u8; 512];
    let tmp_len = if !arg.is_empty() && arg[0] == b'/' {
        // Absolute path — use as-is.
        let n = arg.len().min(511);
        tmp[..n].copy_from_slice(&arg[..n]);
        n
    } else {
        // Relative — join cwd + "/" + arg.
        let clen = cwd.len().min(511);
        tmp[..clen].copy_from_slice(&cwd[..clen]);
        let mut len = clen;
        if len > 0 && tmp[len - 1] != b'/' {
            tmp[len] = b'/';
            len += 1;
        }
        let alen = arg.len().min(511 - len);
        tmp[len..len + alen].copy_from_slice(&arg[..alen]);
        len + alen
    };

    // Normalize: split by '/', resolve "." and "..", collapse "//".
    let mut result = [0u8; 256];
    result[0] = b'/';
    let mut rlen: usize = 1;

    // Stack of component-start positions in result so we can pop for "..".
    let mut starts = [0usize; 64];
    let mut depth: usize = 0;

    let mut i: usize = 0;
    while i < tmp_len {
        // Skip slashes.
        while i < tmp_len && tmp[i] == b'/' {
            i += 1;
        }
        if i >= tmp_len {
            break;
        }
        // Find end of component.
        let start = i;
        while i < tmp_len && tmp[i] != b'/' {
            i += 1;
        }
        let comp = &tmp[start..i];

        if comp == b"." {
            // Current dir — skip.
        } else if comp == b".." {
            // Parent dir — pop if possible.
            if depth > 0 {
                depth -= 1;
                rlen = starts[depth];
            }
        } else {
            // Normal component — push.
            if depth < 64 {
                starts[depth] = rlen;
                depth += 1;
            }
            if rlen > 1 {
                result[rlen] = b'/';
                rlen += 1;
            }
            let n = comp.len().min(255 - rlen);
            result[rlen..rlen + n].copy_from_slice(&comp[..n]);
            rlen += n;
        }
    }

    if rlen == 0 {
        result[0] = b'/';
        rlen = 1;
    }

    (result, rlen)
}

// ---------------------------------------------------------------------------
// Command dispatch
// ---------------------------------------------------------------------------

fn dispatch_command(input: &[u8], cwd: &mut [u8; 256], cwd_len: &mut usize) {
    let (cmd, args) = split_first(input);

    match cmd {
        b"help" => cmd_help(),
        b"echo" => cmd_echo(args),
        b"ls" => cmd_ls(args, &cwd[..*cwd_len]),
        b"cat" => cmd_cat(args, &cwd[..*cwd_len]),
        b"write" => cmd_write(args, &cwd[..*cwd_len]),
        b"mkdir" => cmd_mkdir(args, &cwd[..*cwd_len]),
        b"rm" => cmd_rm(args, &cwd[..*cwd_len]),
        b"stat" => cmd_stat(args, &cwd[..*cwd_len]),
        b"cd" => cmd_cd(args, cwd, cwd_len),
        b"pwd" => cmd_pwd(&cwd[..*cwd_len]),
        b"clear" => cmd_clear(),
        _ => {
            print(CLR_RED);
            print("error");
            print(CLR_RESET);
            print(": unknown command: ");
            print_bytes(cmd);
            print("\ntype 'help' for commands\n");
        }
    }
}

/// Split `input` into `(first_word, rest)`. `rest` may be empty.
fn split_first(input: &[u8]) -> (&[u8], &[u8]) {
    let mut i = 0;
    while i < input.len() && input[i] != b' ' {
        i += 1;
    }
    let cmd = &input[..i];
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
    print("  ");
    print(CLR_BOLD);
    print("help");
    print(CLR_RESET);
    print("              show this message\n");
    print("  ");
    print(CLR_BOLD);
    print("echo");
    print(CLR_RESET);
    print(" [text]       print text\n");
    print("  ");
    print(CLR_BOLD);
    print("ls");
    print(CLR_RESET);
    print(" [path]         list directory\n");
    print("  ");
    print(CLR_BOLD);
    print("cat");
    print(CLR_RESET);
    print(" <path>        print file contents\n");
    print("  ");
    print(CLR_BOLD);
    print("write");
    print(CLR_RESET);
    print(" <path> <t>  write text to file\n");
    print("  ");
    print(CLR_BOLD);
    print("mkdir");
    print(CLR_RESET);
    print(" <path>      create directory\n");
    print("  ");
    print(CLR_BOLD);
    print("rm");
    print(CLR_RESET);
    print(" <path>         remove file\n");
    print("  ");
    print(CLR_BOLD);
    print("stat");
    print(CLR_RESET);
    print(" <path>       show file info\n");
    print("  ");
    print(CLR_BOLD);
    print("cd");
    print(CLR_RESET);
    print(" [path]         change directory\n");
    print("  ");
    print(CLR_BOLD);
    print("pwd");
    print(CLR_RESET);
    print("               print working directory\n");
    print("  ");
    print(CLR_BOLD);
    print("clear");
    print(CLR_RESET);
    print("             clear screen\n");
}

fn cmd_echo(args: &[u8]) {
    print_bytes(args);
    print("\n");
}

fn cmd_ls(args: &[u8], cwd: &[u8]) {
    let arg = if args.is_empty() { b"." as &[u8] } else { args };
    let (resolved, rlen) = resolve_path(cwd, arg);
    let path = bytes_to_str(&resolved[..rlen]);

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
        if entry.d_type == 4 {
            // Directory — bold blue
            print(CLR_BLUE);
            print_bytes(entry.name());
            print("/");
            print(CLR_RESET);
        } else {
            print_bytes(entry.name());
        }
        print("\n");
    }
}

fn cmd_cat(args: &[u8], cwd: &[u8]) {
    if args.is_empty() {
        println("usage: cat <path>");
        return;
    }
    let (resolved, rlen) = resolve_path(cwd, args);
    let path = bytes_to_str(&resolved[..rlen]);

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

fn cmd_write(args: &[u8], cwd: &[u8]) {
    if args.is_empty() {
        println("usage: write <path> <text>");
        return;
    }

    let (path_bytes, text) = split_first(args);
    if path_bytes.is_empty() || text.is_empty() {
        println("usage: write <path> <text>");
        return;
    }
    let (resolved, rlen) = resolve_path(cwd, path_bytes);
    let path = bytes_to_str(&resolved[..rlen]);

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

fn cmd_mkdir(args: &[u8], cwd: &[u8]) {
    if args.is_empty() {
        println("usage: mkdir <path>");
        return;
    }
    let (resolved, rlen) = resolve_path(cwd, args);
    let path = bytes_to_str(&resolved[..rlen]);
    let ret = sys_mkdir(path);
    if ret < 0 {
        print_error("mkdir", ret);
    }
}

fn cmd_rm(args: &[u8], cwd: &[u8]) {
    if args.is_empty() {
        println("usage: rm <path>");
        return;
    }
    let (resolved, rlen) = resolve_path(cwd, args);
    let path = bytes_to_str(&resolved[..rlen]);
    let ret = sys_unlink(path);
    if ret < 0 {
        print_error("rm", ret);
    }
}

fn cmd_stat(args: &[u8], cwd: &[u8]) {
    if args.is_empty() {
        println("usage: stat <path>");
        return;
    }
    let (resolved, rlen) = resolve_path(cwd, args);
    let path = bytes_to_str(&resolved[..rlen]);

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

fn cmd_cd(args: &[u8], cwd: &mut [u8; 256], cwd_len: &mut usize) {
    if args.is_empty() {
        cwd[0] = b'/';
        *cwd_len = 1;
        return;
    }

    let (resolved, rlen) = resolve_path(&cwd[..*cwd_len], args);
    let path = bytes_to_str(&resolved[..rlen]);

    let mut st = StatBuf::zeroed();
    let ret = sys_stat(path, &mut st);
    if ret < 0 {
        print_error("cd", ret);
        return;
    }
    if !st.is_dir() {
        print(CLR_RED);
        print("error");
        print(CLR_RESET);
        print(": cd: not a directory\n");
        return;
    }

    cwd[..rlen].copy_from_slice(&resolved[..rlen]);
    *cwd_len = rlen;
}

fn cmd_pwd(cwd: &[u8]) {
    print_bytes(cwd);
    print("\n");
}

fn cmd_clear() {
    // ANSI escape: clear screen + cursor home
    print("\x1b[2J\x1b[H");
}
