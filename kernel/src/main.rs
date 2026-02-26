#![no_std]
#![no_main]
#![allow(dead_code, unused_imports)]

mod arch;
mod console;
mod drivers;
mod fs;
mod ipc;
mod mm;
mod panic;
mod sched;
mod sync;
mod syscall;

use uefi::prelude::*;

#[entry]
fn efi_main() -> Status {
    arch::early_init();

    println!("[oxeidia] kernel booting...");
    println!("[oxeidia] architecture: {}", arch::ARCH_NAME);

    kernel_main();

    Status::SUCCESS
}

fn kernel_main() {
    println!("[oxeidia] entering kernel_main");

    // Future init order:
    // 1. mm::init()       — set up frame allocator, page tables, heap
    // 2. sched::init()    — initialize scheduler
    // 3. drivers::init()  — probe and init hardware
    // 4. fs::init()       — mount root filesystem
    // 5. syscall::init()  — register syscall handler

    println!("[oxeidia] nothing to do yet — halting.");
    arch::halt();
}
