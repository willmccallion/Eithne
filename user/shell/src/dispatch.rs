//! Command dispatch and prompt rendering.

use core::cmp::Ordering;

use crate::builtins::{cmd_cd, cmd_clear, cmd_echo, cmd_exit, cmd_help, cmd_kill, cmd_pwd};
use libsys::io::{
    print, print_bytes, print_error, CLR_BLUE, CLR_CYAN, CLR_GREEN, CLR_RED, CLR_RESET, LINE_MAX,
};
use libsys::path::{contains_byte, split_first};
use libsys::syscall::{
    sys_exit, sys_fork, sys_sigaction, sys_waitpid, SigActionUser, NEG_EINTR, SIG_DFL, SIGINT,
};

use crate::exec::EXEC_FAILURE_EXIT_CODE;
use crate::pipeline::{cmd_pipeline, run_pipeline_stage};
use crate::redirect::{apply_redirects, has_redirects, parse_redirects};

/// Print the shell prompt.
pub fn print_prompt(cwd: &[u8]) {
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

/// Dispatch a command line (may contain pipes or redirects).
pub fn dispatch_command(input: &[u8], cwd: &mut [u8; 256], cwd_len: &mut usize) {
    // Check for pipe operator — if present, run as a pipeline.
    if contains_byte(input, b'|') {
        cmd_pipeline(input, &cwd[..*cwd_len]);
        return;
    }

    // Check for redirections — if present and not `cd`/`exit`, fork a child
    // so we can apply redirections without affecting the shell's own fds.
    if has_redirects(input) {
        let mut clean_buf = [0u8; LINE_MAX];
        let (clean_len, redir) = parse_redirects(input, &mut clean_buf);
        let clean = &clean_buf[..clean_len];
        let (cmd, _) = split_first(clean);

        // cd and exit must run in the parent — redirect doesn't make sense for them.
        if cmd == b"cd" || cmd == b"exit" {
            dispatch_simple(clean, cwd, cwd_len);
            return;
        }

        let pid = sys_fork();
        match pid.cmp(&0) {
            Ordering::Equal => {
                // Child: apply redirections.
                let dfl_act = SigActionUser {
                    handler: SIG_DFL,
                    flags: 0,
                    mask: 0,
                    pad: 0,
                };
                let _ = sys_sigaction(SIGINT, &dfl_act as *const SigActionUser as usize, 0);

                if !apply_redirects(&redir, &cwd[..*cwd_len]) {
                    sys_exit(1);
                }
                // Run the command in the child.
                run_pipeline_stage(clean, &cwd[..*cwd_len]);
                sys_exit(0);
            }
            Ordering::Greater => {
                // Parent: wait for child.
                let mut status: i32 = 0;
                loop {
                    let ret = sys_waitpid(pid as usize, &mut status);
                    if ret == NEG_EINTR {
                        continue; // EINTR
                    }
                    if ret < 0 {
                        print_error("waitpid", ret);
                    }
                    break;
                }
            }
            Ordering::Less => {
                print_error("fork", pid);
            }
        }
        return;
    }

    dispatch_simple(input, cwd, cwd_len);
}

/// Dispatch a simple command (no redirections, no pipes).
fn dispatch_simple(input: &[u8], cwd: &mut [u8; 256], cwd_len: &mut usize) {
    let (cmd, args) = split_first(input);

    match cmd {
        b"help" => cmd_help(),
        b"echo" => cmd_echo(args),
        b"cd" => cmd_cd(args, cwd, cwd_len),
        b"pwd" => cmd_pwd(&cwd[..*cwd_len]),
        b"clear" => cmd_clear(),
        b"exit" => cmd_exit(args),
        b"kill" => cmd_kill(args),
        _ => {
            // Try as external command: fork + exec /bin/<cmd>
            try_external(cmd, args, &cwd[..*cwd_len]);
        }
    }
}

/// Fork and exec an external command from `/bin/`.
fn try_external(cmd: &[u8], args: &[u8], cwd: &[u8]) {
    use crate::exec::build_argv;
    use libsys::path::{bytes_to_str, resolve_path};
    use libsys::syscall::{sys_exec, sys_waitpid};

    // Build path: if cmd contains '/', use as-is; otherwise try /bin/<cmd>.
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

    let pid = sys_fork();
    match pid.cmp(&0) {
        Ordering::Equal => {
            // Child: reset SIGINT to default.
            let dfl_act = SigActionUser {
                handler: SIG_DFL,
                flags: 0,
                mask: 0,
                pad: 0,
            };
            let _ = sys_sigaction(SIGINT, &dfl_act as *const SigActionUser as usize, 0);

            let argv_ptr = build_argv(path.as_bytes(), args);
            let ret = sys_exec(path, argv_ptr);
            // exec only returns on failure.
            print(CLR_RED);
            print("error");
            print(CLR_RESET);
            print(": unknown command: ");
            print_bytes(cmd);
            print(" (exec: ");
            libsys::io::print_isize(ret);
            print(")\n");
            sys_exit(EXEC_FAILURE_EXIT_CODE);
        }
        Ordering::Greater => {
            let mut status: i32 = 0;
            loop {
                let ret = sys_waitpid(pid as usize, &mut status);
                if ret == NEG_EINTR {
                    continue;
                }
                if ret < 0 {
                    print_error("waitpid", ret);
                }
                break;
            }
        }
        Ordering::Less => {
            print_error("fork", pid);
        }
    }
}
