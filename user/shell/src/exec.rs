//! Argv builder and exec helpers for external program execution.

/// Exit code used when exec fails (standard Unix convention).
pub const EXEC_FAILURE_EXIT_CODE: usize = 127;

// ---------------------------------------------------------------------------
// Argv builder for exec
// ---------------------------------------------------------------------------

/// Maximum number of arguments for exec (including argv[0]).
const MAX_EXEC_ARGS: usize = 64;

/// Storage for NUL-terminated argument strings on the stack.
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
