use crate::sync::{IrqSpinlock, Once};

use super::font::{FONT_HEIGHT, FONT_WIDTH};
use super::{FB, clear, fill_rect, put_pixel, scroll_up};

/// Foreground color: light gray (0xAA, 0xAA, 0xAA).
const FG_R: u8 = 0xAA;
const FG_G: u8 = 0xAA;
const FG_B: u8 = 0xAA;

/// Background color: black.
const BG_R: u8 = 0;
const BG_G: u8 = 0;
const BG_B: u8 = 0;

struct FbConsole {
    col: usize,
    row: usize,
    cols: usize,
    rows: usize,
}

static FB_CONSOLE: Once<IrqSpinlock<FbConsole>> = Once::new();

/// Initialize the framebuffer console.
///
/// Must be called after `super::init()` has set up the framebuffer.
pub fn init() {
    let fb = FB.get().expect("[fb] framebuffer not initialized");
    let fb_guard = fb.lock();
    let cols = fb_guard.width / FONT_WIDTH;
    let rows = fb_guard.height / FONT_HEIGHT;
    clear(&fb_guard, BG_R, BG_G, BG_B);
    drop(fb_guard);

    FB_CONSOLE.call_once(|| {
        IrqSpinlock::new(FbConsole {
            col: 0,
            row: 0,
            cols,
            rows,
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

/// Render a single character, handling control characters and scrolling.
fn put_char(con: &mut FbConsole, fb: &super::FrameBuffer, ch: u8) {
    match ch {
        b'\n' => {
            con.col = 0;
            con.row += 1;
        }
        b'\r' => {
            con.col = 0;
        }
        0x08 => {
            // Backspace: move cursor left by one (if possible).
            if con.col > 0 {
                con.col -= 1;
            }
        }
        _ => {
            let px = con.col * FONT_WIDTH;
            let py = con.row * FONT_HEIGHT;
            draw_glyph(fb, px, py, ch);
            con.col += 1;
            if con.col >= con.cols {
                con.col = 0;
                con.row += 1;
            }
        }
    }

    // Scroll if we've gone past the last row.
    if con.row >= con.rows {
        scroll_up(fb, FONT_HEIGHT, BG_R, BG_G, BG_B);
        con.row = con.rows - 1;
    }
}

/// Draw a single glyph at pixel position `(px, py)`.
fn draw_glyph(fb: &super::FrameBuffer, px: usize, py: usize, ch: u8) {
    let glyph = super::font::glyph(ch);
    for (row, &bits) in glyph.iter().enumerate() {
        for col in 0..FONT_WIDTH {
            let on = (bits >> (7 - col)) & 1 != 0;
            let (r, g, b) = if on {
                (FG_R, FG_G, FG_B)
            } else {
                (BG_R, BG_G, BG_B)
            };
            put_pixel(fb, px + col, py + row, r, g, b);
        }
    }
}
