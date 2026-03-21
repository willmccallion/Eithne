/// Resolve `arg` relative to `cwd`, returning an absolute normalized path.
pub fn resolve_path(cwd: &[u8], arg: &[u8]) -> ([u8; 256], usize) {
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

pub const fn contains_byte(hay: &[u8], needle: u8) -> bool {
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
pub fn split_first(input: &[u8]) -> (&[u8], &[u8]) {
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
pub const fn bytes_to_str(b: &[u8]) -> &str {
    unsafe { core::str::from_utf8_unchecked(b) }
}

/// Trim leading and trailing whitespace from a byte slice.
pub fn trim(b: &[u8]) -> &[u8] {
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

/// Parse a decimal number from a byte slice.
pub fn parse_usize(b: &[u8]) -> usize {
    let mut n: usize = 0;
    for &c in b {
        if c.is_ascii_digit() {
            n = n * 10 + (c - b'0') as usize;
        } else {
            break;
        }
    }
    n
}
