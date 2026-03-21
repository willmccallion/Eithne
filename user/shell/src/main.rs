//! eithne user-space shell.

#![no_std]
#![no_main]

mod builtins;
mod dispatch;
mod exec;
mod heap;
mod io;
mod line;
mod path;
mod pipeline;
mod redirect;
mod syscall;

use dispatch::{dispatch_command, print_prompt};
use io::{print, CLR_CYAN, CLR_RESET, LINE_MAX};
use line::read_line;
use syscall::{SigActionUser, SIG_IGN, sys_sigaction, SIGINT, SIGPIPE, sys_exit};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
#[unsafe(naked)]
extern "C" fn _start() -> ! {
    core::arch::naked_asm!("call main", "ud2")
}

#[unsafe(no_mangle)]
extern "C" fn main(_argc: usize, _argv: *const *const u8) -> ! {
    // Ignore SIGINT so Ctrl+C kills foreground children but not the shell.
    let ign_act = SigActionUser {
        handler: SIG_IGN,
        flags: 0,
        mask: 0,
        _pad: 0,
    };
    let _ = sys_sigaction(SIGINT, &ign_act as *const SigActionUser as usize, 0);
    // Also ignore SIGPIPE for the shell itself.
    let _ = sys_sigaction(SIGPIPE, &ign_act as *const SigActionUser as usize, 0);

    print(CLR_CYAN);
    print("eithne");
    print(CLR_RESET);
    print(" shell v0.2\n\n");

    let mut cwd = [0u8; 256];
    cwd[0] = b'/';
    let mut cwd_len: usize = 1;

    let mut line = [0u8; LINE_MAX];
    loop {
        print_prompt(&cwd[..cwd_len]);
        let len = read_line(&mut line, &cwd[..cwd_len]);
        if len == 0 {
            continue;
        }
        let input = &line[..len];
        dispatch_command(input, &mut cwd, &mut cwd_len);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    print("panic!\n");
    sys_exit(1);
}
