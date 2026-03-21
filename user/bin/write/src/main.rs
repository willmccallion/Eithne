//! eithne `write` utility -- write text to a file.

#![no_std]
#![no_main]

use libsys::io::{print, print_bytes, print_error, println};
use libsys::path::bytes_to_str;
use libsys::syscall::{O_CREAT, O_TRUNC, O_WRONLY, sys_close, sys_exit, sys_open, sys_write};

/// Extract the argument at `index` from the argv array.
const unsafe fn get_arg(argv: *const *const u8, index: usize) -> Option<&'static [u8]> {
    let ptr = unsafe { *argv.add(index) };
    if ptr.is_null() {
        return None;
    }
    let mut len = 0;
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        Some(core::slice::from_raw_parts(ptr, len))
    }
}

/// Entry point.
#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
    if argc < 3 {
        println("usage: write <path> <text> ...");
        sys_exit(1);
    }

    // argv[1] is the path.
    let Some(path) = (unsafe { get_arg(argv, 1) }) else {
        println("write: missing path");
        sys_exit(1);
    };
    let path_str = bytes_to_str(path);

    let fd = sys_open(path_str, O_WRONLY | O_CREAT | O_TRUNC);
    if fd < 0 {
        print("write: ");
        print_bytes(path);
        print(": ");
        print_error("open", fd);
        sys_exit(1);
    }
    let fd = fd as usize;

    // Write argv[2..] joined with spaces.
    let mut i = 2;
    while i < argc {
        if let Some(arg) = unsafe { get_arg(argv, i) } {
            if i > 2 {
                // Insert a space between arguments.
                let _ = sys_write(fd, b" ");
            }
            let n = sys_write(fd, arg);
            if n < 0 {
                print_error("write", n);
                let _ = sys_close(fd);
                sys_exit(1);
            }
        }
        i += 1;
    }

    let _ = sys_close(fd);
    sys_exit(0);
}
