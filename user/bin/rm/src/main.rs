//! eithne `rm` utility -- remove files.

#![no_std]
#![no_main]

use libsys::io::{print, print_bytes, print_error, println};
use libsys::path::bytes_to_str;
use libsys::syscall::{sys_exit, sys_unlink};

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
    if argc <= 1 {
        println("usage: rm <path> ...");
        sys_exit(1);
    }

    let mut i = 1;
    while i < argc {
        if let Some(arg) = unsafe { get_arg(argv, i) } {
            let path_str = bytes_to_str(arg);
            let ret = sys_unlink(path_str);
            if ret < 0 {
                print("rm: ");
                print_bytes(arg);
                print(": ");
                print_error("failed", ret);
            }
        }
        i += 1;
    }
    sys_exit(0);
}
