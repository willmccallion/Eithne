#![no_std]
#![no_main]
#![allow(dead_code, unused_imports)]

extern crate alloc;

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

use uefi::boot::{self, MemoryType};
use uefi::prelude::*;

#[entry]
fn efi_main() -> Status {
    arch::early_init();

    println!("[eithne] kernel booting...");
    println!("[eithne] architecture: {}", arch::ARCH_NAME);

    // Exit UEFI boot services and get the final memory map.
    // After this call, UEFI boot services are gone — only runtime services
    // and our own hardware access (serial) remain.
    let mmap = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    // Parse memory map into our fixed-size representation (no heap needed)
    let boot_info = mm::boot_info::BootMemoryInfo::from_uefi_mmap(&mmap);

    kernel_main(boot_info);

    Status::SUCCESS
}

fn kernel_main(boot_info: mm::boot_info::BootMemoryInfo) {
    println!("[eithne] entering kernel_main");

    // 1. Initialize memory management (frame allocator + heap)
    mm::init(&boot_info);

    // After mm::init(), alloc crate works (Vec, Box, String, etc.)

    // 2. Initialize interrupts (GDT, IDT, PIC)
    arch::init_interrupts();

    // 3. sched::init()    — initialize scheduler
    // 3. drivers::init()  — probe and init hardware
    // 4. fs::init()       — mount root filesystem
    // 5. syscall::init()  — register syscall handler

    println!("[eithne] initialization complete — halting.");
    arch::halt();
}
