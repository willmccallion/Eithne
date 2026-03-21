use core::cmp::Ordering;

use crate::builtins::{cmd_echo, cat_stdin, cmd_cat, cmd_ls, cmd_pwd, cmd_stat, cmd_help};
use crate::exec::{EXEC_FAILURE_EXIT_CODE, build_argv};
use crate::io::{LINE_MAX, print_error, STDIN, STDOUT};
use crate::path::{trim, split_first, resolve_path, bytes_to_str};
use crate::redirect::{parse_redirects, has_redirects, apply_redirects};
use crate::syscall::{sys_pipe, sys_close, sys_fork, SigActionUser, SIG_DFL, sys_sigaction, SIGINT, sys_dup2, sys_exit, sys_waitpid, sys_exec};

/// Maximum number of pipeline stages.
const MAX_PIPELINE_STAGES: usize = 8;

/// Execute a pipeline of commands connected by pipes.
pub fn cmd_pipeline(input: &[u8], cwd: &[u8]) {
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
            for pipe in &pipes[..i] {
                let _ = sys_close(pipe[0] as usize);
                let _ = sys_close(pipe[1] as usize);
            }
            return;
        }
    }

    // Fork n children.
    let mut pids = [0isize; MAX_PIPELINE_STAGES];
    let mut fork_ok = true;

    for i in 0..n {
        let pid = sys_fork();
        match pid.cmp(&0) {
            Ordering::Equal => {
                // ---- Child process for stage i ----

                // Reset SIGINT to default (shell had SIG_IGN).
                let dfl_act = SigActionUser {
                    handler: SIG_DFL,
                    flags: 0,
                    mask: 0,
                    _pad: 0,
                };
                let _ = sys_sigaction(SIGINT, &dfl_act as *const SigActionUser as usize, 0);

                // Wire up stdin from previous pipe's read end (except first stage).
                if i > 0 {
                    let _ = sys_dup2(pipes[i - 1][0] as usize, STDIN);
                }

                // Wire up stdout to current pipe's write end (except last stage).
                if i < n - 1 {
                    let _ = sys_dup2(pipes[i][1] as usize, STDOUT);
                }

                // Close all pipe fds in the child (they've been dup2'd as needed).
                for pipe in &pipes[..n - 1] {
                    let _ = sys_close(pipe[0] as usize);
                    let _ = sys_close(pipe[1] as usize);
                }

                // Dispatch this stage's command.
                let stage = &stages[i][..stage_lens[i]];
                run_pipeline_stage(stage, cwd);
                // Should not return.
                sys_exit(0);
            }
            Ordering::Greater => {
                pids[i] = pid;
            }
            Ordering::Less => {
                print_error("fork", pid);
                fork_ok = false;
                break;
            }
        }
    }

    // Parent: close all pipe fds.
    for pipe in &pipes[..n - 1] {
        let _ = sys_close(pipe[0] as usize);
        let _ = sys_close(pipe[1] as usize);
    }

    // Wait for all children, retrying on EINTR.
    if fork_ok {
        for &pid in &pids[..n] {
            if pid > 0 {
                let mut status: i32 = 0;
                loop {
                    let ret = sys_waitpid(pid as usize, &mut status);
                    if ret == -4_isize {
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

/// Run a single pipeline stage in a child process. The child's stdin/stdout
/// have already been redirected via dup2.
///
/// For built-in commands (echo, cat, ls, etc.) we run them directly — they
/// use sys_write(STDOUT, ...) / sys_read(STDIN, ...) which now go through
/// the pipe. For `run <path>`, we exec the external program.
pub fn run_pipeline_stage(stage: &[u8], cwd: &[u8]) {
    // Parse and apply any per-stage redirections (e.g., `cat < file`).
    let mut clean_buf = [0u8; LINE_MAX];
    let (clean_len, redir) = parse_redirects(stage, &mut clean_buf);
    let clean = &clean_buf[..clean_len];

    if has_redirects(stage) && !apply_redirects(&redir, cwd) {
        sys_exit(1);
    }

    let (cmd, args) = split_first(clean);

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
            let (path_arg, rest) = split_first(args);
            let (resolved, rlen) = resolve_path(cwd, path_arg);
            let path = bytes_to_str(&resolved[..rlen]);
            let argv_ptr = build_argv(path.as_bytes(), rest);
            let ret = sys_exec(path, argv_ptr);
            print_error("exec", ret);
            sys_exit(EXEC_FAILURE_EXIT_CODE);
        }
        _ => {
            // Try as an external program path.
            let (resolved, rlen) = resolve_path(cwd, cmd);
            let path = bytes_to_str(&resolved[..rlen]);
            let argv_ptr = build_argv(path.as_bytes(), args);
            let ret = sys_exec(path, argv_ptr);
            // If exec failed, print error and exit.
            print_error("exec", ret);
            sys_exit(EXEC_FAILURE_EXIT_CODE);
        }
    }
}
