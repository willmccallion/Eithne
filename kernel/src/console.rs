use core::fmt;

use crate::arch;

/// Arch-agnostic writer that delegates to the current platform's serial output.
pub struct Writer;

impl fmt::Write for Writer {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        arch::write_serial(s);
        crate::drivers::fb::console::fb_write_str(s);
        Ok(())
    }
}

/// Prints formatted text to the console.
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _ = write!($crate::console::Writer, $($arg)*);
    }};
}

/// Prints formatted text to the console with a trailing newline.
#[macro_export]
macro_rules! println {
    () => { $crate::print!("\n") };
    ($($arg:tt)*) => {{
        $crate::print!("{}\n", format_args!($($arg)*));
    }};
}
