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

    // 2. Higher-half direct map: all physical memory at KERNEL_VIRT_BASE.
    //    Must happen before init_interrupts so GDT/IDT pointers are stable.
    arch::init_higher_half(boot_info.max_phys_addr.as_u64());
    mm::set_higher_half_offset(crate::mm::layout::KERNEL_VIRT_BASE);

    // 3. GDT, IDT, PIC, interrupts.
    arch::init_interrupts();

    // 4. Page table smoke test.
    arch::test_paging();

    // 5. Syscall interface.
    arch::init_syscalls();

    // 6. Scheduler.
    sched::init();

    sched::spawn("idle", idle_task);

    // 7. User-mode init process.
    #[cfg(target_arch = "x86_64")]
    sched::spawn_user();

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
