pub mod console;
pub mod font;

use crate::mm::addr::{PhysAddr, phys_to_virt};
use crate::sync::{IrqSpinlock, Once};

/// GOP framebuffer info captured at boot time.
///
/// Re-exported from `main.rs` — contains only plain scalars, no UEFI pointers.
pub use crate::FrameBufferInfo;

/// The raw framebuffer: virtual pointer + geometry.
struct FrameBuffer {
    ptr: *mut u8,
    width: usize,
    height: usize,
    stride: usize, // pixels per scan line
    is_bgr: bool,
}

// SAFETY: The framebuffer pointer points to device memory mapped into the
// kernel's address space. Only one CPU accesses it at a time via IrqSpinlock.
unsafe impl Send for FrameBuffer {}

static FB: Once<IrqSpinlock<FrameBuffer>> = Once::new();

/// Initialize the framebuffer from boot-captured GOP info.
///
/// Converts the physical base address to a virtual address via the
/// higher-half direct map.
pub fn init(info: &FrameBufferInfo) {
    let virt = phys_to_virt(PhysAddr::new(info.base));
    let _ = FB.call_once(|| {
        IrqSpinlock::new(FrameBuffer {
            ptr: virt.as_mut_ptr::<u8>(),
            width: info.width,
            height: info.height,
            stride: info.stride,
            is_bgr: info.is_bgr,
        })
    });
}

/// Write a single 32-bit pixel at `(x, y)`.
#[inline]
fn put_pixel(fb: &FrameBuffer, x: usize, y: usize, r: u8, g: u8, b: u8) {
    if x >= fb.width || y >= fb.height {
        return;
    }
    let offset = (y * fb.stride + x) * 4;
    let pixel: [u8; 4] = if fb.is_bgr {
        [b, g, r, 0]
    } else {
        [r, g, b, 0]
    };
    unsafe {
        let dst = fb.ptr.add(offset);
        #[allow(clippy::volatile_composites)]
        core::ptr::write_volatile(dst as *mut [u8; 4], pixel);
    }
}

/// Fill a rectangle with a solid color.
#[allow(clippy::too_many_arguments)]
fn fill_rect(fb: &FrameBuffer, x: usize, y: usize, w: usize, h: usize, r: u8, g: u8, b: u8) {
    let pixel: u32 = if fb.is_bgr {
        (r as u32) << 16 | (g as u32) << 8 | b as u32
    } else {
        (b as u32) << 16 | (g as u32) << 8 | r as u32
    };
    for row in y..core::cmp::min(y + h, fb.height) {
        let row_start = (row * fb.stride + x) * 4;
        for col in 0..core::cmp::min(w, fb.width.saturating_sub(x)) {
            let offset = row_start + col * 4;
            unsafe {
                #[allow(clippy::cast_ptr_alignment)]
                core::ptr::write_volatile(fb.ptr.add(offset) as *mut u32, pixel);
            }
        }
    }
}

/// Clear the entire screen to a solid color.
fn clear(fb: &FrameBuffer, r: u8, g: u8, b: u8) {
    fill_rect(fb, 0, 0, fb.width, fb.height, r, g, b);
}

/// Scroll the screen up by `rows_px` pixel rows.
///
/// The bottom band is cleared to the given background color.
fn scroll_up(fb: &FrameBuffer, rows_px: usize, bg_r: u8, bg_g: u8, bg_b: u8) {
    if rows_px == 0 || rows_px >= fb.height {
        clear(fb, bg_r, bg_g, bg_b);
        return;
    }

    let bytes_per_row = fb.stride * 4;
    let src_offset = rows_px * bytes_per_row;
    let copy_rows = fb.height - rows_px;
    let copy_bytes = copy_rows * bytes_per_row;

    unsafe {
        // memmove: source and destination overlap.
        core::ptr::copy(fb.ptr.add(src_offset), fb.ptr, copy_bytes);
    }

    // Clear the bottom band.
    fill_rect(fb, 0, copy_rows, fb.width, rows_px, bg_r, bg_g, bg_b);
}

/// Get framebuffer width (pixels). Returns 0 if not initialized.
pub fn width() -> usize {
    FB.get().map_or(0, |fb| fb.lock().width)
}

/// Get framebuffer height (pixels). Returns 0 if not initialized.
pub fn height() -> usize {
    FB.get().map_or(0, |fb| fb.lock().height)
}
