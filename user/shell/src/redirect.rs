use libsys::io::{print_error, LINE_MAX, STDIN, STDOUT};
use libsys::path::{bytes_to_str, resolve_path};
use libsys::syscall::{sys_close, sys_dup2, sys_open, O_APPEND, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};

/// Parsed redirection targets for a single command.
pub struct Redirects {
    /// File to redirect stdin from (<).
    pub stdin_file: [u8; 256],
    pub stdin_len: usize,
    /// File to redirect stdout to (> or >>).
    pub stdout_file: [u8; 256],
    pub stdout_len: usize,
    /// Whether stdout should be appended (>>).
    pub stdout_append: bool,
}

impl Redirects {
    pub const fn empty() -> Self {
        Self {
            stdin_file: [0u8; 256],
            stdin_len: 0,
            stdout_file: [0u8; 256],
            stdout_len: 0,
            stdout_append: false,
        }
    }

    pub const fn has_stdin(&self) -> bool {
        self.stdin_len > 0
    }

    pub const fn has_stdout(&self) -> bool {
        self.stdout_len > 0
    }
}

/// Parse and strip redirection tokens from a command line.
///
/// Scans `input` for `>`, `>>`, and `<` operators, extracts the filenames,
/// and writes the remaining (clean) command into `clean_buf`.
/// Returns `(clean_len, redirects)`.
pub fn parse_redirects(input: &[u8], clean_buf: &mut [u8; LINE_MAX]) -> (usize, Redirects) {
    let mut redir = Redirects::empty();
    let mut clean_len: usize = 0;
    let mut i: usize = 0;

    while i < input.len() {
        if input[i] == b'>' {
            // Check for >> (append).
            let append = i + 1 < input.len() && input[i + 1] == b'>';
            if append {
                i += 2;
            } else {
                i += 1;
            }
            // Skip spaces after >/>>
            while i < input.len() && input[i] == b' ' {
                i += 1;
            }
            // Read filename.
            let start = i;
            while i < input.len() && input[i] != b' ' && input[i] != b'>' && input[i] != b'<' {
                i += 1;
            }
            let name = &input[start..i];
            let n = name.len().min(255);
            redir.stdout_file[..n].copy_from_slice(&name[..n]);
            redir.stdout_len = n;
            redir.stdout_append = append;
        } else if input[i] == b'<' {
            i += 1;
            // Skip spaces after <
            while i < input.len() && input[i] == b' ' {
                i += 1;
            }
            // Read filename.
            let start = i;
            while i < input.len() && input[i] != b' ' && input[i] != b'>' && input[i] != b'<' {
                i += 1;
            }
            let name = &input[start..i];
            let n = name.len().min(255);
            redir.stdin_file[..n].copy_from_slice(&name[..n]);
            redir.stdin_len = n;
        } else {
            if clean_len < LINE_MAX {
                clean_buf[clean_len] = input[i];
                clean_len += 1;
            }
            i += 1;
        }
    }

    // Trim trailing spaces from clean buffer.
    while clean_len > 0 && clean_buf[clean_len - 1] == b' ' {
        clean_len -= 1;
    }

    (clean_len, redir)
}

/// Apply redirections in the current (child) process.
/// Opens redirect files and dup2s them onto stdin/stdout.
/// Returns true on success, false on error (prints error message).
pub fn apply_redirects(redir: &Redirects, cwd: &[u8]) -> bool {
    if redir.has_stdin() {
        let (resolved, rlen) = resolve_path(cwd, &redir.stdin_file[..redir.stdin_len]);
        let path = bytes_to_str(&resolved[..rlen]);
        let fd = sys_open(path, O_RDONLY);
        if fd < 0 {
            print_error("redirect: open <", fd);
            return false;
        }
        let _ = sys_dup2(fd as usize, STDIN);
        let _ = sys_close(fd as usize);
    }

    if redir.has_stdout() {
        let (resolved, rlen) = resolve_path(cwd, &redir.stdout_file[..redir.stdout_len]);
        let path = bytes_to_str(&resolved[..rlen]);
        let flags = if redir.stdout_append {
            O_WRONLY | O_CREAT | O_APPEND
        } else {
            O_WRONLY | O_CREAT | O_TRUNC
        };
        let fd = sys_open(path, flags);
        if fd < 0 {
            print_error("redirect: open >", fd);
            return false;
        }
        let _ = sys_dup2(fd as usize, STDOUT);
        let _ = sys_close(fd as usize);
    }

    true
}

/// Check if a byte slice contains any redirection operators.
pub const fn has_redirects(input: &[u8]) -> bool {
    let mut i = 0;
    while i < input.len() {
        if input[i] == b'>' || input[i] == b'<' {
            return true;
        }
        i += 1;
    }
    false
}
