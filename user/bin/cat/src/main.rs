//! eithne `cat` utility -- concatenate and print files.

#![no_std]
#![no_main]

use libsys::io::{print, print_bytes, print_error, STDIN, STDOUT};
use libsys::path::bytes_to_str;
use libsys::syscall::{sys_close, sys_exit, sys_open, sys_read, sys_write, O_RDONLY};

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

/// Read stdin until EOF and write everything to stdout.
fn cat_stdin() {
    let mut buf = [0u8; 512];
    loop {
        let n = sys_read(STDIN, &mut buf);
        if n <= 0 {
            break;
        }
        let _ = sys_write(STDOUT, &buf[..n as usize]);
    }
}

/// Open a file by path, print its contents, and close it.
fn cat_file(path: &[u8]) {
    let path_str = bytes_to_str(path);

    let fd = sys_open(path_str, O_RDONLY);
    if fd < 0 {
        print("cat: ");
        print_bytes(path);
        print(": ");
        print_error("open", fd);
        return;
    }
    let fd = fd as usize;

    let mut buf = [0u8; 512];
    loop {
        let n = sys_read(fd, &mut buf);
        if n <= 0 {
            break;
        }
        print_bytes(&buf[..n as usize]);
    }

    let _ = sys_close(fd);
}

/// Entry point.
#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
    if argc <= 1 {
        // No arguments: copy stdin to stdout.
        cat_stdin();
    } else {
        // For each argument, print the file.
        let mut i = 1;
        while i < argc {
            if let Some(arg) = unsafe { get_arg(argv, i) } {
                cat_file(arg);
            }
            i += 1;
        }
    }
    sys_exit(0);
}
