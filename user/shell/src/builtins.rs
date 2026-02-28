use core::mem;

use crate::io::*;
use crate::path::*;
use crate::syscall::*;

pub fn cmd_help() {
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
    print("  ");
    print(CLR_BOLD);
    print("exit");
    print(CLR_RESET);
    print(" [code]       exit shell\n");
    print("  ");
    print(CLR_BOLD);
    print("kill");
    print(CLR_RESET);
    print(" <pid> [sig]   send signal to process\n");
    print("  ");
    print(CLR_BOLD);
    print("run");
    print(CLR_RESET);
    print(" <path>        run program (fork+exec)\n");
}

pub fn cmd_echo(args: &[u8]) {
    print_bytes(args);
    print("\n");
}

pub fn cmd_ls(args: &[u8], cwd: &[u8]) {
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

    for i in 0..count {
        let offset = i * ENTRY_SIZE;
        let entry = unsafe { &*(buf.as_ptr().add(offset) as *const DirEntry) };
        if entry.d_type == DT_DIR {
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

pub fn cmd_cat(args: &[u8], cwd: &[u8]) {
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

pub fn cmd_write(args: &[u8], cwd: &[u8]) {
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

pub fn cmd_mkdir(args: &[u8], cwd: &[u8]) {
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

pub fn cmd_rm(args: &[u8], cwd: &[u8]) {
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

pub fn cmd_stat(args: &[u8], cwd: &[u8]) {
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

pub fn cmd_cd(args: &[u8], cwd: &mut [u8; 256], cwd_len: &mut usize) {
    if args.is_empty() {
        cwd[0] = b'/';
        *cwd_len = 1;
        sys_chdir("/");
        return;
    }

    let (resolved, rlen) = resolve_path(&cwd[..*cwd_len], args);
    let path = bytes_to_str(&resolved[..rlen]);

    let ret = sys_chdir(path);
    if ret < 0 {
        print_error("cd", ret);
        return;
    }

    cwd[..rlen].copy_from_slice(&resolved[..rlen]);
    *cwd_len = rlen;
}

pub fn cmd_pwd(cwd: &[u8]) {
    print_bytes(cwd);
    print("\n");
}

pub fn cmd_clear() {
    // ANSI escape: clear screen + cursor home
    print("\x1b[2J\x1b[H");
}

pub fn cmd_exit(args: &[u8]) {
    let code = if args.is_empty() {
        0
    } else {
        parse_usize(args)
    };
    sys_exit(code);
}

pub fn cmd_kill(args: &[u8]) {
    if args.is_empty() {
        println("usage: kill <pid> [signal]");
        return;
    }

    let (pid_str, rest) = split_first(args);
    let pid = parse_usize(pid_str);

    // Default signal is SIGTERM (15).
    let sig = if rest.is_empty() {
        SIGTERM
    } else {
        let (sig_str, _) = split_first(rest);
        parse_usize(sig_str)
    };

    let ret = sys_kill(pid, sig);
    if ret < 0 {
        print(CLR_RED);
        print("error");
        print(CLR_RESET);
        print(": kill failed\n");
    }
}

/// Read stdin until EOF and write everything to stdout (for `cat` with no args).
pub fn cat_stdin() {
    let mut buf = [0u8; 512];
    loop {
        let n = sys_read(STDIN, &mut buf);
        if n <= 0 {
            break;
        }
        sys_write(STDOUT, &buf[..n as usize]);
    }
}
