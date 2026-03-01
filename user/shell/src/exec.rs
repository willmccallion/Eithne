use crate::io::*;
use crate::path::*;
use crate::syscall::*;

// ---------------------------------------------------------------------------
// Wait status macros (standard Unix encoding)
// ---------------------------------------------------------------------------

/// Standard Unix: low 7 bits are the signal number (0 for normal exit).
const WAIT_STATUS_SIGNAL_MASK: i32 = 0x7F;
/// Standard Unix: bits 15..8 hold the exit code.
const WAIT_STATUS_CODE_SHIFT: i32 = 8;
const WAIT_STATUS_CODE_MASK: i32 = 0xFF;

/// True if the child exited normally (not killed by a signal).
pub fn wifexited(status: i32) -> bool {
    (status & WAIT_STATUS_SIGNAL_MASK) == 0
}

/// Extract the exit code from a normal-exit wait status.
pub fn wexitstatus(status: i32) -> i32 {
    (status >> WAIT_STATUS_CODE_SHIFT) & WAIT_STATUS_CODE_MASK
}

/// Returns true if the child was killed by a signal.
pub fn wifsignaled(status: i32) -> bool {
    (status & WAIT_STATUS_SIGNAL_MASK) != 0
}

/// Extract the signal number from a signal-killed wait status.
pub fn wtermsig(status: i32) -> i32 {
    status & WAIT_STATUS_SIGNAL_MASK
}

/// Exit code used when exec fails (standard Unix convention).
pub const EXEC_FAILURE_EXIT_CODE: usize = 127;

// ---------------------------------------------------------------------------
// Argv builder for exec
// ---------------------------------------------------------------------------

/// Maximum number of arguments for exec (including argv[0]).
const MAX_EXEC_ARGS: usize = 64;

/// Storage for NUL-terminated argument strings on the stack.
/// Each arg gets up to 256 bytes (including NUL terminator).
static mut ARGV_BUFS: [[u8; 256]; MAX_EXEC_ARGS] = [[0u8; 256]; MAX_EXEC_ARGS];
/// Pointer array passed to sys_exec (NULL-terminated).
static mut ARGV_PTRS: [*const u8; MAX_EXEC_ARGS + 1] = [core::ptr::null(); MAX_EXEC_ARGS + 1];

/// Build a NULL-terminated argv array from the program path and a space-
/// separated argument string. Returns a pointer to the argv array.
///
/// argv[0] is the program path.
pub fn build_argv(path: &[u8], args: &[u8]) -> *const *const u8 {
    use core::ptr::addr_of_mut;

    unsafe {
        let bufs = addr_of_mut!(ARGV_BUFS);
        let ptrs = addr_of_mut!(ARGV_PTRS);
        let mut argc = 0usize;

        // argv[0] = program path
        let n = path.len().min(255);
        let buf = &mut (*bufs)[0];
        buf[..n].copy_from_slice(&path[..n]);
        buf[n] = 0;
        (*ptrs)[0] = buf.as_ptr();
        argc += 1;

        // Split remaining args by spaces.
        let mut i = 0;
        while i < args.len() && argc < MAX_EXEC_ARGS {
            // Skip spaces.
            while i < args.len() && args[i] == b' ' {
                i += 1;
            }
            if i >= args.len() {
                break;
            }
            // Find end of this arg.
            let start = i;
            while i < args.len() && args[i] != b' ' {
                i += 1;
            }
            let arg = &args[start..i];
            let n = arg.len().min(255);
            let buf = &mut (*bufs)[argc];
            buf[..n].copy_from_slice(&arg[..n]);
            buf[n] = 0;
            (*ptrs)[argc] = buf.as_ptr();
            argc += 1;
        }

        // NULL-terminate the array.
        (*ptrs)[argc] = core::ptr::null();

        (*ptrs).as_ptr()
    }
}

// ---------------------------------------------------------------------------
// Run external program (fork + exec + wait)
// ---------------------------------------------------------------------------

pub fn cmd_run(args: &[u8], cwd: &[u8]) {
    if args.is_empty() {
        println("usage: run <path> [args...]");
        return;
    }

    // Split into path and remaining arguments.
    let (path_arg, rest) = split_first(args);
    let (resolved, rlen) = resolve_path(cwd, path_arg);
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

        // Build argv: [path, arg1, arg2, ..., NULL]
        let argv_ptr = build_argv(path.as_bytes(), rest);

        // Replace with the new program.
        let ret = sys_exec(path, argv_ptr);
        // exec only returns on failure.
        print_error("exec", ret);
        sys_exit(EXEC_FAILURE_EXIT_CODE);
    } else if pid > 0 {
        // Parent: wait for child, retrying on EINTR.
        let mut status: i32 = 0;
        loop {
            let ret = sys_waitpid(pid as usize, &mut status);
            if ret == -4_isize {
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
