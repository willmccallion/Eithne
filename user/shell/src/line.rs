//! Line editing and tab completion.

use crate::dispatch::print_prompt;
use libsys::io::{print, print_bytes, COMMANDS, LINE_MAX, STDIN};
use libsys::syscall::{sys_read, NEG_EINTR};

// ---- Terminal control characters ----

/// Backspace (BS).
const BS: u8 = 0x08;
/// Delete (DEL).
const DEL: u8 = 0x7F;
/// Escape (starts ANSI escape sequences).
const ESC: u8 = 0x1B;
/// First printable ASCII character (space).
const PRINTABLE_START: u8 = 0x20;
/// Last printable ASCII character (tilde).
const PRINTABLE_END: u8 = 0x7E;
/// Start of CSI final byte range.
const CSI_FINAL_START: u8 = 0x40;
/// End of CSI final byte range.
const CSI_FINAL_END: u8 = 0x7E;

/// Read a line of input from stdin with basic editing support.
pub fn read_line(buf: &mut [u8; LINE_MAX], cwd: &[u8]) -> usize {
    let mut len: usize = 0;
    let mut ch = [0u8; 1];

    loop {
        let n = sys_read(STDIN, &mut ch);
        if n == NEG_EINTR {
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
            b'\r' | b'\n' => {
                print("\n");
                return len;
            }
            BS | DEL if len > 0 => {
                len -= 1;
                print_bytes(&[BS, b' ', BS]);
            }
            b'\t' => {
                try_complete(buf, &mut len, cwd);
            }
            ESC => {
                let mut seq = [0u8; 1];
                let n2 = sys_read(STDIN, &mut seq);
                if n2 > 0 && seq[0] == b'[' {
                    loop {
                        let n3 = sys_read(STDIN, &mut seq);
                        if n3 <= 0 || (seq[0] >= CSI_FINAL_START && seq[0] <= CSI_FINAL_END) {
                            break;
                        }
                    }
                }
            }
            PRINTABLE_START..=PRINTABLE_END if len < LINE_MAX => {
                buf[len] = c;
                len += 1;
                print_bytes(&[c]);
            }
            _ => {}
        }
    }
}

/// Tab-complete command names.
fn try_complete(buf: &mut [u8; LINE_MAX], len: &mut usize, cwd: &[u8]) {
    let prefix = &buf[..*len];

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
