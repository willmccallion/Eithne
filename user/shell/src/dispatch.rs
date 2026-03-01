use crate::builtins::*;
use crate::exec::*;
use crate::io::*;
use crate::path::*;
use crate::pipeline::*;
use crate::redirect::*;
use crate::syscall::*;

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
        if pid == 0 {
            // Child: apply redirections.
            let dfl_act = SigActionUser {
                handler: SIG_DFL,
                flags: 0,
                mask: 0,
                _pad: 0,
            };
            sys_sigaction(SIGINT, &dfl_act as *const SigActionUser as usize, 0);

            if !apply_redirects(&redir, &cwd[..*cwd_len]) {
                sys_exit(1);
            }
            // Run the command in the child.
            run_pipeline_stage(clean, &cwd[..*cwd_len]);
            sys_exit(0);
        } else if pid > 0 {
            // Parent: wait for child.
            let mut status: i32 = 0;
            loop {
                let ret = sys_waitpid(pid as usize, &mut status);
                if ret == -4_isize {
                    continue; // EINTR
                }
                if ret < 0 {
                    print_error("waitpid", ret);
                }
                break;
            }
        } else {
            print_error("fork", pid);
        }
        return;
    }

    dispatch_simple(input, cwd, cwd_len);
}

/// Dispatch a simple command (no redirections, no pipes).
fn dispatch_simple(input: &[u8], cwd: &mut [u8; 256], cwd_len: &mut usize) {
    let (cmd, args) = split_first(input);

    match cmd {
        b"alloc" => cmd_alloc(args),
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
