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

const COMMANDS: &[&[u8]] = &[
    b"cat", b"cd", b"clear", b"echo", b"exit", b"help", b"kill", b"ls", b"mkdir", b"pwd", b"rm",
    b"run", b"stat", b"write",
];

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
    // Ignore SIGINT so Ctrl+C kills foreground children but not the shell.
    let ign_act = SigActionUser {
        handler: SIG_IGN,
        flags: 0,
        mask: 0,
        _pad: 0,
    };
    sys_sigaction(SIGINT, &ign_act as *const SigActionUser as usize, 0);
    // Also ignore SIGPIPE for the shell itself.
    sys_sigaction(SIGPIPE, &ign_act as *const SigActionUser as usize, 0);

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
        let len = read_line(&mut line, &cwd[..cwd_len]);
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

/// Negative errno returned by sys_read when interrupted by a signal.
const NEG_EINTR: isize = -4;

fn read_line(buf: &mut [u8; LINE_MAX], cwd: &[u8]) -> usize {
    let mut len: usize = 0;
    let mut ch = [0u8; 1];

    loop {
        let n = sys_read(STDIN, &mut ch);
        if n == NEG_EINTR {
            // Ctrl+C: discard current input, print ^C, re-show prompt.
            print("^C\n");
            print_prompt(cwd);
            len = 0;
            continue;
        }
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
                    print_bytes(&[0x08, b' ', 0x08]);
                }
            }
            // Tab — autocomplete command name
            b'\t' => {
                try_complete(buf, &mut len, cwd);
            }
            // Normal printable character
            _ => {
                if len < LINE_MAX {
                    buf[len] = c;
                    len += 1;
                    print_bytes(&[c]);
                }
            }
        }
    }
}

/// Tab-complete command names. Only acts when the cursor is still in the
/// first word (no space typed yet).
fn try_complete(buf: &mut [u8; LINE_MAX], len: &mut usize, cwd: &[u8]) {
    let prefix = &buf[..*len];

    // Only complete the command word (before any space).
    let mut has_space = false;
    let mut i = 0;
    while i < prefix.len() {
        if prefix[i] == b' ' {
            has_space = true;
            break;
        }
        i += 1;
    }
    if has_space {
        return;
    }

    // Gather matches.
    let mut matches = [0u8; 16];
    let mut count: usize = 0;
    for (idx, cmd) in COMMANDS.iter().enumerate() {
        if cmd.len() >= prefix.len() && cmd[..prefix.len()] == *prefix {
            if count < matches.len() {
                matches[count] = idx as u8;
                count += 1;
            }
        }
    }

    if count == 0 {
        return;
    }

    if count == 1 {
        // Single match — complete and append a space.
        let cmd = COMMANDS[matches[0] as usize];
        for &b in &cmd[*len..] {
            if *len < LINE_MAX {
                buf[*len] = b;
                *len += 1;
                print_bytes(&[b]);
            }
        }
        if *len < LINE_MAX {
            buf[*len] = b' ';
            *len += 1;
            print_bytes(&[b' ']);
        }
    } else {
        // Multiple matches — print them, then reprint prompt + input.
        print("\n");
        for i in 0..count {
            print_bytes(COMMANDS[matches[i] as usize]);
            print("  ");
        }
        print("\n");
        print_prompt(cwd);
        print_bytes(&buf[..*len]);
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
    // Check for pipe operator — if present, run as a pipeline.
    if contains_byte(input, b'|') {
        cmd_pipeline(input, &cwd[..*cwd_len]);
        return;
    }

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
        b"exit" => cmd_exit(args),
        b"kill" => cmd_kill(args),
        b"run" => cmd_run(args, &cwd[..*cwd_len]),
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

fn contains_byte(hay: &[u8], needle: u8) -> bool {
    let mut i = 0;
    while i < hay.len() {
        if hay[i] == needle {
            return true;
        }
        i += 1;
    }
    false
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

    for i in 0..count {
        let offset = i * ENTRY_SIZE;
        let entry = unsafe { &*(buf.as_ptr().add(offset) as *const DirEntry) };
        if entry.d_type == DT_DIR {
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

fn cmd_exit(args: &[u8]) {
    let code = if args.is_empty() {
        0
    } else {
        parse_usize(args)
    };
    sys_exit(code);
}

// ---------------------------------------------------------------------------
// Wait status macros (standard Unix encoding)
// ---------------------------------------------------------------------------

/// Standard Unix: low 7 bits are the signal number (0 for normal exit).
const WAIT_STATUS_SIGNAL_MASK: i32 = 0x7F;
/// Standard Unix: bits 15..8 hold the exit code.
const WAIT_STATUS_CODE_SHIFT: i32 = 8;
const WAIT_STATUS_CODE_MASK: i32 = 0xFF;

/// True if the child exited normally (not killed by a signal).
fn wifexited(status: i32) -> bool {
    (status & WAIT_STATUS_SIGNAL_MASK) == 0
}

/// Extract the exit code from a normal-exit wait status.
fn wexitstatus(status: i32) -> i32 {
    (status >> WAIT_STATUS_CODE_SHIFT) & WAIT_STATUS_CODE_MASK
}

/// Returns true if the child was killed by a signal.
fn wifsignaled(status: i32) -> bool {
    (status & WAIT_STATUS_SIGNAL_MASK) != 0
}

/// Extract the signal number from a signal-killed wait status.
fn wtermsig(status: i32) -> i32 {
    status & WAIT_STATUS_SIGNAL_MASK
}

/// Exit code used when exec fails (standard Unix convention).
const EXEC_FAILURE_EXIT_CODE: usize = 127;

// ---------------------------------------------------------------------------
// Pipeline execution (cmd1 | cmd2 | ...)
// ---------------------------------------------------------------------------

/// Maximum number of pipeline stages.
const MAX_PIPELINE_STAGES: usize = 8;

/// Execute a pipeline of commands connected by pipes.
fn cmd_pipeline(input: &[u8], cwd: &[u8]) {
    // Split input by '|' into stages.
    let mut stages = [[0u8; LINE_MAX]; MAX_PIPELINE_STAGES];
    let mut stage_lens = [0usize; MAX_PIPELINE_STAGES];
    let n = split_pipeline(input, &mut stages, &mut stage_lens);

    if n < 2 {
        // Shouldn't happen (caller checked for '|'), but handle gracefully.
        return;
    }

    // Create n-1 pipes.
    let mut pipes = [[0i32; 2]; MAX_PIPELINE_STAGES - 1];
    for i in 0..n - 1 {
        let ret = sys_pipe(&mut pipes[i]);
        if ret < 0 {
            print_error("pipe", ret);
            // Clean up any already-created pipes.
            for j in 0..i {
                sys_close(pipes[j][0] as usize);
                sys_close(pipes[j][1] as usize);
            }
            return;
        }
    }

    // Fork n children.
    let mut pids = [0isize; MAX_PIPELINE_STAGES];
    let mut fork_ok = true;

    for i in 0..n {
        let pid = sys_fork();
        if pid == 0 {
            // ---- Child process for stage i ----

            // Reset SIGINT to default (shell had SIG_IGN).
            let dfl_act = SigActionUser {
                handler: SIG_DFL,
                flags: 0,
                mask: 0,
                _pad: 0,
            };
            sys_sigaction(SIGINT, &dfl_act as *const SigActionUser as usize, 0);

            // Wire up stdin from previous pipe's read end (except first stage).
            if i > 0 {
                sys_dup2(pipes[i - 1][0] as usize, STDIN);
            }

            // Wire up stdout to current pipe's write end (except last stage).
            if i < n - 1 {
                sys_dup2(pipes[i][1] as usize, STDOUT);
            }

            // Close all pipe fds in the child (they've been dup2'd as needed).
            for j in 0..n - 1 {
                sys_close(pipes[j][0] as usize);
                sys_close(pipes[j][1] as usize);
            }

            // Dispatch this stage's command.
            let stage = &stages[i][..stage_lens[i]];
            run_pipeline_stage(stage, cwd);
            // Should not return.
            sys_exit(0);
        } else if pid > 0 {
            pids[i] = pid;
        } else {
            print_error("fork", pid);
            fork_ok = false;
            break;
        }
    }

    // Parent: close all pipe fds.
    for j in 0..n - 1 {
        sys_close(pipes[j][0] as usize);
        sys_close(pipes[j][1] as usize);
    }

    // Wait for all children, retrying on EINTR.
    if fork_ok {
        for i in 0..n {
            if pids[i] > 0 {
                let mut status: i32 = 0;
                loop {
                    let ret = sys_waitpid(pids[i] as usize, &mut status);
                    if ret == -(4 as isize) {
                        continue; // EINTR
                    }
                    break;
                }
            }
        }
    }
}

/// Split `input` by `|`, trimming whitespace around each stage.
fn split_pipeline(
    input: &[u8],
    stages: &mut [[u8; LINE_MAX]; MAX_PIPELINE_STAGES],
    lens: &mut [usize; MAX_PIPELINE_STAGES],
) -> usize {
    let mut count = 0;
    let mut start = 0;

    let mut i = 0;
    while i <= input.len() {
        if i == input.len() || input[i] == b'|' {
            if count < MAX_PIPELINE_STAGES {
                let segment = trim(&input[start..i]);
                let n = segment.len().min(LINE_MAX);
                stages[count][..n].copy_from_slice(&segment[..n]);
                lens[count] = n;
                if n > 0 {
                    count += 1;
                }
            }
            start = i + 1;
        }
        i += 1;
    }

    count
}

/// Trim leading and trailing whitespace from a byte slice.
fn trim(b: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < b.len() && b[start] == b' ' {
        start += 1;
    }
    let mut end = b.len();
    while end > start && b[end - 1] == b' ' {
        end -= 1;
    }
    &b[start..end]
}

/// Run a single pipeline stage in a child process. The child's stdin/stdout
/// have already been redirected via dup2.
///
/// For built-in commands (echo, cat, ls, etc.) we run them directly — they
/// use sys_write(STDOUT, ...) / sys_read(STDIN, ...) which now go through
/// the pipe. For `run <path>`, we exec the external program.
fn run_pipeline_stage(stage: &[u8], cwd: &[u8]) {
    let (cmd, args) = split_first(stage);

    match cmd {
        b"echo" => {
            cmd_echo(args);
        }
        b"cat" => {
            if args.is_empty() {
                // cat with no args: copy stdin to stdout
                cat_stdin();
            } else {
                cmd_cat(args, cwd);
            }
        }
        b"ls" => {
            cmd_ls(args, cwd);
        }
        b"pwd" => {
            cmd_pwd(cwd);
        }
        b"stat" => {
            cmd_stat(args, cwd);
        }
        b"help" => {
            cmd_help();
        }
        b"run" => {
            // External program — exec replaces this child.
            if args.is_empty() {
                print_error("run", -22);
                sys_exit(EXEC_FAILURE_EXIT_CODE);
            }
            let (resolved, rlen) = resolve_path(cwd, args);
            let path = bytes_to_str(&resolved[..rlen]);
            let ret = sys_exec(path);
            print_error("exec", ret);
            sys_exit(EXEC_FAILURE_EXIT_CODE);
        }
        _ => {
            // Try as an external program path.
            let (resolved, rlen) = resolve_path(cwd, cmd);
            let path = bytes_to_str(&resolved[..rlen]);
            let ret = sys_exec(path);
            // If exec failed, print error and exit.
            print_error("exec", ret);
            sys_exit(EXEC_FAILURE_EXIT_CODE);
        }
    }
}

/// Read stdin until EOF and write everything to stdout (for `cat` with no args).
fn cat_stdin() {
    let mut buf = [0u8; 512];
    loop {
        let n = sys_read(STDIN, &mut buf);
        if n <= 0 {
            break;
        }
        sys_write(STDOUT, &buf[..n as usize]);
    }
}

// ---------------------------------------------------------------------------
// kill <pid> [signal]
// ---------------------------------------------------------------------------

fn cmd_kill(args: &[u8]) {
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

// ---------------------------------------------------------------------------
// Run external program (fork + exec + wait)
// ---------------------------------------------------------------------------

fn cmd_run(args: &[u8], cwd: &[u8]) {
    if args.is_empty() {
        println("usage: run <path>");
        return;
    }

    let (resolved, rlen) = resolve_path(cwd, args);
    let path = bytes_to_str(&resolved[..rlen]);

    let pid = sys_fork();
    if pid == 0 {
        // Child: reset SIGINT to default (shell had SIG_IGN).
        let dfl_act = SigActionUser {
            handler: SIG_DFL,
            flags: 0,
            mask: 0,
            _pad: 0,
        };
        sys_sigaction(SIGINT, &dfl_act as *const SigActionUser as usize, 0);

        // Replace with the new program.
        let ret = sys_exec(path);
        // exec only returns on failure.
        print_error("exec", ret);
        sys_exit(EXEC_FAILURE_EXIT_CODE);
    } else if pid > 0 {
        // Parent: wait for child, retrying on EINTR.
        let mut status: i32 = 0;
        loop {
            let ret = sys_waitpid(pid as usize, &mut status);
            if ret == -(4 as isize) {
                // EINTR — interrupted by signal, retry.
                continue;
            }
            if ret < 0 {
                print_error("waitpid", ret);
            } else if wifsignaled(status) {
                let sig = wtermsig(status);
                if sig == SIGINT as i32 {
                    print("\n"); // newline after ^C
                } else {
                    print("killed by signal ");
                    print_isize(sig as isize);
                    print("\n");
                }
            } else if wifexited(status) {
                let code = wexitstatus(status);
                if code != 0 {
                    print("child exited with code ");
                    print_isize(code as isize);
                    print("\n");
                }
            }
            break;
        }
    } else {
        print_error("fork", pid);
    }
}

/// Parse a decimal number from a byte slice.
fn parse_usize(b: &[u8]) -> usize {
    let mut n: usize = 0;
    for &c in b {
        if c >= b'0' && c <= b'9' {
            n = n * 10 + (c - b'0') as usize;
        } else {
            break;
        }
    }
    n
}
