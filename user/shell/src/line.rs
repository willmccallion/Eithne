use crate::dispatch::print_prompt;
use crate::io::*;
use crate::syscall::*;

/// Negative errno returned by sys_read when interrupted by a signal.
const NEG_EINTR: isize = -4;

pub fn read_line(buf: &mut [u8; LINE_MAX], cwd: &[u8]) -> usize {
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
            0x08 | 0x7F if len > 0 => {
                len -= 1;
                print_bytes(&[0x08, b' ', 0x08]);
            }
            // Tab — autocomplete command name
            b'\t' => {
                try_complete(buf, &mut len, cwd);
            }
            // Escape: consume the full ANSI sequence so it doesn't leak
            // into the line buffer (e.g. arrow keys send \x1b[A).
            0x1b => {
                let mut seq = [0u8; 1];
                let n2 = sys_read(STDIN, &mut seq);
                if n2 > 0 && seq[0] == b'[' {
                    // CSI sequence — read until a letter (0x40..0x7E).
                    loop {
                        let n3 = sys_read(STDIN, &mut seq);
                        if n3 <= 0 || (seq[0] >= 0x40 && seq[0] <= 0x7E) {
                            break;
                        }
                    }
                }
            }
            // Normal printable character (0x20..=0x7E)
            0x20..=0x7E if len < LINE_MAX => {
                buf[len] = c;
                len += 1;
                print_bytes(&[c]);
            }
            // Ignore all other non-printable bytes.
            _ => {}
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
        if cmd.len() >= prefix.len() && cmd[..prefix.len()] == *prefix && count < matches.len() {
            matches[count] = idx as u8;
            count += 1;
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
            print_bytes(b" ");
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
