//! Shell builtin commands.
//!
//! Only commands that must run in the shell process (cd, exit) or are
//! trivial enough to not warrant a separate binary live here. Everything
//! else is a standalone program in `/bin/`.

use libsys::io::{print, print_bytes, print_error, println, CLR_BOLD, CLR_RED, CLR_RESET};
use libsys::path::{bytes_to_str, parse_usize, resolve_path, split_first};
use libsys::syscall::{sys_chdir, sys_exit, sys_kill, SIGTERM};

/// Print the list of available commands.
pub fn cmd_help() {
    print("builtins:\n");
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
    print(" <pid> [sig]  send signal to process\n");
    print("\nexternal (in /bin/):\n");
    print("  cat, ls, mkdir, rm, stat, write\n");
}

/// Echo text to stdout.
pub fn cmd_echo(args: &[u8]) {
    print_bytes(args);
    print("\n");
}

/// Change working directory.
pub fn cmd_cd(args: &[u8], cwd: &mut [u8; 256], cwd_len: &mut usize) {
    if args.is_empty() {
        cwd[0] = b'/';
        *cwd_len = 1;
        let _ = sys_chdir("/");
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

/// Print working directory.
pub fn cmd_pwd(cwd: &[u8]) {
    print_bytes(cwd);
    print("\n");
}

/// Clear screen.
pub fn cmd_clear() {
    print("\x1b[2J\x1b[H");
}

/// Exit shell.
pub fn cmd_exit(args: &[u8]) {
    let code = if args.is_empty() {
        0
    } else {
        parse_usize(args)
    };
    sys_exit(code);
}

/// Send signal to a process.
pub fn cmd_kill(args: &[u8]) {
    if args.is_empty() {
        println("usage: kill <pid> [signal]");
        return;
    }

    let (pid_str, rest) = split_first(args);
    let pid = parse_usize(pid_str);

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

