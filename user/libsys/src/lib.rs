//! eithne user-space system library.
//!
//! Provides syscall wrappers, I/O helpers, path utilities, and a heap
//! allocator for all user-space binaries.

#![no_std]

/// Console I/O helpers (print, color codes, etc.).
pub mod io;
/// Path resolution and string utilities.
pub mod path;
/// Raw system call wrappers.
pub mod syscall;

mod heap;

// Provide _start entry point and panic handler for all user binaries.
// Each binary must define:
//   #[unsafe(no_mangle)]
//   extern "C" fn main(argc: usize, argv: *const *const u8) -> !

/// Entry point for user programs. Calls `main`.
#[unsafe(no_mangle)]
#[unsafe(naked)]
extern "C" fn _start() -> ! {
    core::arch::naked_asm!("call main", "ud2")
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo<'_>) -> ! {
    io::print("panic!\n");
    syscall::sys_exit(1);
}
