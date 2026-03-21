use crate::sync::{IrqSpinlock, Once};

use super::font::{FONT_HEIGHT, FONT_WIDTH};
use super::{FB, clear, fill_rect, put_pixel, scroll_up};

/// Modern ANSI colour palette (slightly desaturated, easy on the eyes).
const COLORS: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00), // 0  Black
    (0xCC, 0x33, 0x33), // 1  Red
    (0x33, 0xCC, 0x66), // 2  Green
    (0xCC, 0xCC, 0x33), // 3  Yellow
    (0x33, 0x66, 0xCC), // 4  Blue
    (0xCC, 0x33, 0xCC), // 5  Magenta
    (0x33, 0xCC, 0xCC), // 6  Cyan
    (0xCC, 0xCC, 0xCC), // 7  White
    (0x55, 0x55, 0x55), // 8  Bright Black
    (0xFF, 0x66, 0x66), // 9  Bright Red
    (0x66, 0xFF, 0x99), // 10 Bright Green
    (0xFF, 0xFF, 0x66), // 11 Bright Yellow
    (0x66, 0x99, 0xFF), // 12 Bright Blue
    (0xFF, 0x66, 0xFF), // 13 Bright Magenta
    (0x66, 0xFF, 0xFF), // 14 Bright Cyan
    (0xFF, 0xFF, 0xFF), // 15 Bright White
];

const DEFAULT_FG: (u8, u8, u8) = (0xCC, 0xCC, 0xCC);
const DEFAULT_BG: (u8, u8, u8) = (0x00, 0x00, 0x00);

/// Escape character (starts ANSI escape sequences).
const ESC: u8 = 0x1B;
/// Backspace character.
const BS: u8 = 0x08;

#[derive(Clone, Copy)]
enum EscState {
    Normal,
    Esc,
    Csi,
}

struct FbConsole {
    col: usize,
    row: usize,
    cols: usize,
    rows: usize,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    bold: bool,
    esc_state: EscState,
    esc_buf: [u8; 16],
    esc_len: usize,
}

static FB_CONSOLE: Once<IrqSpinlock<FbConsole>> = Once::new();

/// Initialize the framebuffer console.
///
/// Must be called after `super::init()` has set up the framebuffer.
#[allow(clippy::expect_used)] // Init code: framebuffer must exist at this point.
pub fn init() {
    let fb = FB.get().expect("[fb] framebuffer not initialized");
    let fb_guard = fb.lock();
    let cols = fb_guard.width / FONT_WIDTH;
    let rows = fb_guard.height / FONT_HEIGHT;
    clear(&fb_guard, DEFAULT_BG.0, DEFAULT_BG.1, DEFAULT_BG.2);
    drop(fb_guard);

    let _ = FB_CONSOLE.call_once(|| {
        IrqSpinlock::new(FbConsole {
            col: 0,
            row: 0,
            cols,
            rows,
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            bold: false,
            esc_state: EscState::Normal,
            esc_buf: [0; 16],
            esc_len: 0,
        })
    });
}

/// Write a string to the framebuffer console.
///
/// This is the public entry point called from `console::Writer` and
/// `sys_write`. It acquires the necessary locks internally.
pub fn fb_write_str(s: &str) {
    let Some(console_lock) = FB_CONSOLE.get() else {
        return;
    };
    let Some(fb_lock) = FB.get() else {
        return;
    };

    let mut con = console_lock.lock();
    let fb = fb_lock.lock();

    for &b in s.as_bytes() {
        put_char(&mut con, &fb, b);
    }
}

/// Render a single character, handling control characters, ANSI escapes,
/// and scrolling.
fn put_char(con: &mut FbConsole, fb: &super::FrameBuffer, ch: u8) {
    match con.esc_state {
        EscState::Normal => match ch {
            ESC => {
                con.esc_state = EscState::Esc;
            }
            b'\n' => {
                con.col = 0;
                con.row += 1;
            }
            b'\r' => {
                con.col = 0;
            }
            BS => {
                if con.col > 0 {
                    con.col -= 1;
                }
            }
            _ => {
                let px = con.col * FONT_WIDTH;
                let py = con.row * FONT_HEIGHT;
                draw_glyph(fb, px, py, ch, con.fg, con.bg);
                con.col += 1;
                if con.col >= con.cols {
                    con.col = 0;
                    con.row += 1;
                }
            }
        },
        EscState::Esc => {
            if ch == b'[' {
                con.esc_state = EscState::Csi;
                con.esc_len = 0;
            } else {
                con.esc_state = EscState::Normal;
            }
        }
        EscState::Csi => {
            if ch.is_ascii_digit() || ch == b';' {
                if con.esc_len < con.esc_buf.len() {
                    con.esc_buf[con.esc_len] = ch;
                    con.esc_len += 1;
                }
            } else {
                dispatch_csi(con, fb, ch);
                con.esc_state = EscState::Normal;
            }
        }
    }

    // Scroll if we've gone past the last row.
    if con.row >= con.rows {
        scroll_up(fb, FONT_HEIGHT, con.bg.0, con.bg.1, con.bg.2);
        con.row = con.rows - 1;
    }
}

// ---- CSI dispatch -----------------------------------------------------------

fn dispatch_csi(con: &mut FbConsole, fb: &super::FrameBuffer, cmd: u8) {
    // Copy buffer so we can mutate `con` freely below.
    let mut buf = [0u8; 16];
    let len = con.esc_len;
    buf[..len].copy_from_slice(&con.esc_buf[..len]);
    let buf = &buf[..len];

    match cmd {
        b'm' => {
            // SGR — Select Graphic Rendition
            if buf.is_empty() {
                sgr_reset(con);
                return;
            }
            let mut iter = ParamIter::new(buf);
            while let Some(p) = iter.next() {
                apply_sgr(con, p);
            }
        }
        b'J'
            // Erase in Display
            if parse_single_param(buf) == 2 =>
        {
            clear(fb, con.bg.0, con.bg.1, con.bg.2);
            con.col = 0;
            con.row = 0;
        }
        b'H' => {
            // Cursor Position — params are 1-based
            let (r, c) = parse_two_params(buf);
            con.row = if r == 0 { 0 } else { r - 1 };
            con.col = if c == 0 { 0 } else { c - 1 };
            if con.row >= con.rows {
                con.row = con.rows - 1;
            }
            if con.col >= con.cols {
                con.col = con.cols - 1;
            }
        }
        _ => {} // unsupported CSI, ignore
    }
}

const fn sgr_reset(con: &mut FbConsole) {
    con.fg = DEFAULT_FG;
    con.bg = DEFAULT_BG;
    con.bold = false;
}

const fn apply_sgr(con: &mut FbConsole, param: usize) {
    match param {
        0 => sgr_reset(con),
        1 => con.bold = true,
        30..=37 => {
            let idx = param - 30 + if con.bold { 8 } else { 0 };
            con.fg = COLORS[idx];
        }
        40..=47 => {
            con.bg = COLORS[param - 40];
        }
        90..=97 => {
            con.fg = COLORS[param - 90 + 8];
        }
        100..=107 => {
            con.bg = COLORS[param - 100 + 8];
        }
        _ => {}
    }
}

// ---- CSI parameter parsing --------------------------------------------------

fn parse_single_param(buf: &[u8]) -> usize {
    let mut val = 0usize;
    for &b in buf {
        if b.is_ascii_digit() {
            val = val * 10 + (b - b'0') as usize;
        }
    }
    val
}

fn parse_two_params(buf: &[u8]) -> (usize, usize) {
    let mut iter = ParamIter::new(buf);
    let first = iter.next().unwrap_or(0);
    let second = iter.next().unwrap_or(0);
    (first, second)
}

struct ParamIter<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ParamIter<'a> {
    const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn next(&mut self) -> Option<usize> {
        if self.pos > self.buf.len() {
            return None;
        }
        let start = self.pos;
        let mut end = start;
        while end < self.buf.len() && self.buf[end] != b';' {
            end += 1;
        }
        self.pos = end + 1;
        let mut val = 0usize;
        for i in start..end {
            if self.buf[i].is_ascii_digit() {
                val = val * 10 + (self.buf[i] - b'0') as usize;
            }
        }
        Some(val)
    }
}

// ---- Glyph rendering --------------------------------------------------------

/// Draw a single glyph at pixel position `(px, py)`.
fn draw_glyph(
    fb: &super::FrameBuffer,
    px: usize,
    py: usize,
    ch: u8,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
) {
    let glyph = super::font::glyph(ch);
    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0..FONT_WIDTH {
            let on = (bits >> (7 - col)) & 1 != 0;
            let (r, g, b) = if on { fg } else { bg };
            put_pixel(fb, px + col, py + row, r, g, b);
        }
    }
}
