//! eithne `stat` utility -- display file status.

#![no_std]
#![no_main]

use libsys::io::{print, print_bytes, print_error, print_num, println};
use libsys::path::bytes_to_str;
use libsys::syscall::{StatBuf, sys_exit, sys_stat};

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

/// Print stat information for a single path.
fn stat_path(path: &[u8]) {
    let path_str = bytes_to_str(path);

    let mut st = StatBuf::zeroed();
    let ret = sys_stat(path_str, &mut st);
    if ret < 0 {
        print("stat: ");
        print_bytes(path);
        print(": ");
        print_error("failed", ret);
        return;
    }

    print("  type:  ");
    println(st.type_str());
    print("  size:  ");
    print_num(st.st_size);
    print("\n");
    print("  inode: ");
    print_num(st.st_ino);
    print("\n");
}

/// Entry point.
#[unsafe(no_mangle)]
extern "C" fn main(argc: usize, argv: *const *const u8) -> ! {
    if argc <= 1 {
        println("usage: stat <path> ...");
        sys_exit(1);
    }

    let mut i = 1;
    while i < argc {
        if let Some(arg) = unsafe { get_arg(argv, i) } {
            stat_path(arg);
        }
        i += 1;
    }
    sys_exit(0);
}
