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
    let mmap = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    let boot_info = mm::boot_info::BootMemoryInfo::from_uefi_mmap(&mmap);

    kernel_main(boot_info);

    Status::SUCCESS
}

fn kernel_main(boot_info: mm::boot_info::BootMemoryInfo) {
    println!("[eithne] entering kernel_main");

    // 1. Memory management (frame allocator + heap).
    mm::init(&boot_info);

    // 2. GDT, IDT, PIC, interrupts.
    arch::init_interrupts();

    // 3. Page table smoke test.
    arch::test_paging();

    // 4. Scheduler.
    sched::init();

    sched::spawn("idle", idle_task);

    println!("[eithne] initialization complete — entering scheduler loop.");

    // Yield into the scheduler.  From here the timer IRQ drives preemption.
    sched::yield_cpu();

    // Fallback: if no tasks are runnable, halt.
    arch::halt();
}

fn idle_task() -> ! {
    loop {
        arch::halt_once();
    }
}
