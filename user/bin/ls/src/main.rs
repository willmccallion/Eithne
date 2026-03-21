//! eithne `ls` utility -- list directory contents.

#![no_std]
#![no_main]

use core::mem;

use libsys::io::{CLR_BLUE, CLR_RESET, print, print_bytes, print_error};
use libsys::path::bytes_to_str;
use libsys::syscall::{DT_DIR, DirEntry, sys_exit, sys_readdir};

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

/// List the contents of a directory.
#[allow(clippy::large_stack_arrays, clippy::cast_ptr_alignment)]
fn list_dir(path: &[u8]) {
    const ENTRY_SIZE: usize = mem::size_of::<DirEntry>();
    const MAX_ENTRIES: usize = 64;

    let path_str = bytes_to_str(path);
    let mut buf = [0u8; ENTRY_SIZE * MAX_ENTRIES];

    let ret = sys_readdir(path_str, &mut buf);
    if ret < 0 {
        print_error("ls", ret);
        return;
    }

    let bytes_returned = ret as usize;
    let count = bytes_returned / ENTRY_SIZE;

    for i in 0..count {
        let offset = i * ENTRY_SIZE;
        let entry = unsafe { &*(buf.as_ptr().add(offset) as *const DirEntry) };
        if entry.d_type == DT_DIR {
            print(CLR_BLUE);
            print_bytes(entry.name());
            print("/");
            print(CLR_RESET);
        } else {
            print_bytes(entry.name());
        }
        print("\n");
    }
}

/// Entry point.
#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
    if argc <= 1 {
        list_dir(b".");
    } else if let Some(arg) = unsafe { get_arg(argv, 1) } {
        list_dir(arg);
    }
    sys_exit(0);
}
