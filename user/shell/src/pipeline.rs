//! Pipeline execution (commands connected by `|`).

use core::cmp::Ordering;

use crate::builtins::cmd_echo;
use crate::exec::{build_argv, EXEC_FAILURE_EXIT_CODE};
use libsys::io::{print_error, LINE_MAX, STDIN, STDOUT};
use libsys::path::{bytes_to_str, resolve_path, split_first, trim};
use libsys::syscall::{
    sys_close, sys_dup2, sys_exec, sys_exit, sys_fork, sys_pipe, sys_read, sys_sigaction,
    sys_waitpid, sys_write, SigActionUser, NEG_EINTR, SIG_DFL, SIGINT,
};

/// Maximum number of pipeline stages.
const MAX_PIPELINE_STAGES: usize = 8;

/// Execute a pipeline of commands connected by pipes.
pub fn cmd_pipeline(input: &[u8], cwd: &[u8]) {
    let mut stages = [[0u8; LINE_MAX]; MAX_PIPELINE_STAGES];
    let mut stage_lens = [0usize; MAX_PIPELINE_STAGES];
    let n = split_pipeline(input, &mut stages, &mut stage_lens);

    if n < 2 {
        return;
    }

    let mut pipes = [[0i32; 2]; MAX_PIPELINE_STAGES - 1];
    for i in 0..n - 1 {
        let ret = sys_pipe(&mut pipes[i]);
        if ret < 0 {
            print_error("pipe", ret);
            for pipe in &pipes[..i] {
                let _ = sys_close(pipe[0] as usize);
                let _ = sys_close(pipe[1] as usize);
            }
            return;
        }
    }

    let mut pids = [0isize; MAX_PIPELINE_STAGES];
    let mut fork_ok = true;

    for i in 0..n {
        let pid = sys_fork();
        match pid.cmp(&0) {
            Ordering::Equal => {
                let dfl_act = SigActionUser {
                    handler: SIG_DFL,
                    flags: 0,
                    mask: 0,
                    pad: 0,
                };
                let _ = sys_sigaction(SIGINT, &dfl_act as *const SigActionUser as usize, 0);

                if i > 0 {
                    let _ = sys_dup2(pipes[i - 1][0] as usize, STDIN);
                }
                if i < n - 1 {
                    let _ = sys_dup2(pipes[i][1] as usize, STDOUT);
                }

                for pipe in &pipes[..n - 1] {
                    let _ = sys_close(pipe[0] as usize);
                    let _ = sys_close(pipe[1] as usize);
                }

                let stage = &stages[i][..stage_lens[i]];
                run_pipeline_stage(stage, cwd);
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

    for pipe in &pipes[..n - 1] {
        let _ = sys_close(pipe[0] as usize);
        let _ = sys_close(pipe[1] as usize);
    }

    if fork_ok {
        for &pid in &pids[..n] {
            if pid > 0 {
                let mut status: i32 = 0;
                loop {
                    let ret = sys_waitpid(pid as usize, &mut status);
                    if ret == NEG_EINTR {
                        continue;
                    }
                    break;
                }
            }
        }
    }
}

/// Split input by `|`, trimming whitespace around each stage.
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

/// Run a single pipeline stage in a child process.
pub fn run_pipeline_stage(stage: &[u8], cwd: &[u8]) {
    use crate::redirect::{apply_redirects, has_redirects, parse_redirects};

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
                let mut buf = [0u8; 512];
                loop {
                    let n = sys_read(STDIN, &mut buf);
                    if n <= 0 {
                        break;
                    }
                    let _ = sys_write(STDOUT, &buf[..n as usize]);
                }
            } else {
                // Try external /bin/cat
                exec_external(cmd, args, cwd);
            }
        }
        _ => {
            // Try as external program.
            exec_external(cmd, args, cwd);
        }
    }
}

/// Attempt to exec an external program.
fn exec_external(cmd: &[u8], args: &[u8], cwd: &[u8]) {
    use libsys::path::contains_byte;

    let (resolved, rlen) = if contains_byte(cmd, b'/') {
        resolve_path(cwd, cmd)
    } else {
        let mut path_buf = [0u8; 270];
        let prefix = b"/bin/";
        let plen = prefix.len();
        path_buf[..plen].copy_from_slice(prefix);
        let clen = cmd.len().min(265);
        path_buf[plen..plen + clen].copy_from_slice(&cmd[..clen]);
        let mut result = [0u8; 256];
        let total = (plen + clen).min(256);
        result[..total].copy_from_slice(&path_buf[..total]);
        (result, total)
    };
    let path = bytes_to_str(&resolved[..rlen]);
    let argv_ptr = build_argv(path.as_bytes(), args);
    let ret = sys_exec(path, argv_ptr);
    print_error("exec", ret);
    sys_exit(EXEC_FAILURE_EXIT_CODE);
}
