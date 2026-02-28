//! PS/2 keyboard driver.
//!
//! Translates scan code set 1 (make/break) into ASCII bytes and pushes them
//! into the shared input buffer so `sys_read` picks them up identically to
//! serial input.

use core::sync::atomic::{AtomicBool, Ordering};

/// Shift key state — set on make, cleared on break.
static SHIFT_HELD: AtomicBool = AtomicBool::new(false);

/// Alt key state — when held, keystrokes are suppressed so the window
/// manager (e.g. Hyprland) can handle Alt+key combos.
static ALT_HELD: AtomicBool = AtomicBool::new(false);

/// Scan code set 1 → ASCII (unshifted).
///
/// Index = make code (0x00–0x3A). 0 = no mapping.
#[rustfmt::skip]
static SCANCODE_MAP: [u8; 59] = [
    0,   0x1B, b'1', b'2', b'3', b'4', b'5', b'6',  // 0x00-0x07
    b'7', b'8', b'9', b'0', b'-', b'=', 0x08, b'\t', // 0x08-0x0F
    b'q', b'w', b'e', b'r', b't', b'y', b'u', b'i',  // 0x10-0x17
    b'o', b'p', b'[', b']', b'\n', 0,   b'a', b's',   // 0x18-0x1F
    b'd', b'f', b'g', b'h', b'j', b'k', b'l', b';',  // 0x20-0x27
    b'\'', b'`', 0,   b'\\', b'z', b'x', b'c', b'v', // 0x28-0x2F
    b'b', b'n', b'm', b',', b'.', b'/', 0,    b'*',   // 0x30-0x37
    0,    b' ', 0,                                      // 0x38-0x3A
];

/// Scan code set 1 → ASCII (shifted).
#[rustfmt::skip]
static SCANCODE_MAP_SHIFT: [u8; 59] = [
    0,   0x1B, b'!', b'@', b'#', b'$', b'%', b'^',   // 0x00-0x07
    b'&', b'*', b'(', b')', b'_', b'+', 0x08, b'\t',  // 0x08-0x0F
    b'Q', b'W', b'E', b'R', b'T', b'Y', b'U', b'I',  // 0x10-0x17
    b'O', b'P', b'{', b'}', b'\n', 0,   b'A', b'S',   // 0x18-0x1F
    b'D', b'F', b'G', b'H', b'J', b'K', b'L', b':',  // 0x20-0x27
    b'"', b'~', 0,   b'|', b'Z', b'X', b'C', b'V',   // 0x28-0x2F
    b'B', b'N', b'M', b'<', b'>', b'?', 0,    b'*',   // 0x30-0x37
    0,    b' ', 0,                                      // 0x38-0x3A
];

/// Left/right shift make and break codes.
const LSHIFT_MAKE: u8 = 0x2A;
const RSHIFT_MAKE: u8 = 0x36;
const LSHIFT_BREAK: u8 = 0xAA;
const RSHIFT_BREAK: u8 = 0xB6;

/// Left Alt make and break codes.
const LALT_MAKE: u8 = 0x38;
const LALT_BREAK: u8 = 0xB8;

/// Process a raw PS/2 scan code (set 1).
///
/// On a valid make code, translates to ASCII and pushes to the input buffer.
/// Called from the IRQ1 handler.
pub fn handle_scancode(scancode: u8) {
    // Track modifier state.
    match scancode {
        LSHIFT_MAKE | RSHIFT_MAKE => {
            SHIFT_HELD.store(true, Ordering::Relaxed);
            return;
        }
        LSHIFT_BREAK | RSHIFT_BREAK => {
            SHIFT_HELD.store(false, Ordering::Relaxed);
            return;
        }
        LALT_MAKE => {
            ALT_HELD.store(true, Ordering::Relaxed);
            return;
        }
        LALT_BREAK => {
            ALT_HELD.store(false, Ordering::Relaxed);
            return;
        }
        _ => {}
    }

    // Ignore break codes (bit 7 set).
    if scancode & 0x80 != 0 {
        return;
    }

    let idx = scancode as usize;
    if idx >= SCANCODE_MAP.len() {
        return;
    }

    // Suppress character output when Alt is held so the window manager
    // (e.g. Hyprland) can handle Alt+key combos like Alt+C to close.
    if ALT_HELD.load(Ordering::Relaxed) {
        return;
    }

    let map = if SHIFT_HELD.load(Ordering::Relaxed) {
        &SCANCODE_MAP_SHIFT
    } else {
        &SCANCODE_MAP
    };

    let ascii = map[idx];
    if ascii != 0 {
        crate::drivers::serial::push_byte(ascii);

        // Wake a blocked reader if one is waiting.
        if crate::drivers::serial::READER_BLOCKED.swap(false, Ordering::AcqRel) {
            crate::sched::wake_reader();
        }
    }
}
