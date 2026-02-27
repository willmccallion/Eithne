#![no_std]
#![no_main]
#![allow(dead_code, unused_imports)]

extern crate alloc;

mod arch;
mod console;
mod drivers;
mod elf;
mod fs;
mod ipc;
mod mm;
mod panic;
mod sched;
mod sync;
mod syscall;

use core::ptr::NonNull;

use uefi::boot::{self, MemoryType};
use uefi::prelude::*;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode};

/// Loaded initramfs: physical pointer and size, surviving exit_boot_services.
struct InitRamfs {
    ptr: NonNull<u8>,
    len: usize,
}

#[entry]
fn efi_main() -> Status {
    arch::early_init();

    println!("[eithne] kernel booting...");
    println!("[eithne] architecture: {}", arch::ARCH_NAME);

    // Load init ELF from ESP before exiting boot services.
    let initramfs = load_init_from_esp();

    // Exit UEFI boot services and get the final memory map.
    let mmap = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    let boot_info = mm::boot_info::BootMemoryInfo::from_uefi_mmap(&mmap);

    kernel_main(boot_info, initramfs);

    Status::SUCCESS
}

/// Load `\EFI\eithne\init` from the ESP into a LOADER_DATA pool allocation.
fn load_init_from_esp() -> Option<InitRamfs> {
    let mut sfs = boot::get_image_file_system(boot::image_handle())
        .expect("[initramfs] failed to open ESP filesystem");

    let mut root = sfs.open_volume().expect("[initramfs] failed to open root");

    let path = cstr16!(r"\EFI\eithne\init");
    let handle = root
        .open(path, FileMode::Read, FileAttribute::empty())
        .expect("[initramfs] failed to open \\EFI\\eithne\\init");

    let mut file = handle
        .into_regular_file()
        .expect("[initramfs] init is not a regular file");

    // Query file size.
    #[repr(align(8))]
    struct AlignedBuf([u8; 256]);
    let mut info_buf = AlignedBuf([0u8; 256]);
    let info = file
        .get_info::<FileInfo>(&mut info_buf.0)
        .expect("[initramfs] failed to get file info");
    let file_size = info.file_size() as usize;

    if file_size == 0 {
        println!("[initramfs] warning: init file is empty");
        return None;
    }

    // Allocate pool buffer (survives exit_boot_services as LOADER_DATA).
    let buf_ptr = boot::allocate_pool(MemoryType::LOADER_DATA, file_size)
        .expect("[initramfs] failed to allocate pool");

    let buf_slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr.as_ptr(), file_size) };
    let bytes_read = file
        .read(buf_slice)
        .expect("[initramfs] failed to read init file");

    println!("[initramfs] loaded {} bytes from \\EFI\\eithne\\init", bytes_read);

    Some(InitRamfs {
        ptr: buf_ptr,
        len: bytes_read,
    })
}

fn kernel_main(boot_info: mm::boot_info::BootMemoryInfo, initramfs: Option<InitRamfs>) {
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

    // 6. Serial input buffer + COM1 RX interrupts.
    drivers::serial::init();
    arch::enable_serial_rx();

    // 8. Scheduler.
    sched::init();

    sched::spawn("idle", idle_task);

    // 9. User-mode init process from ELF.
    #[cfg(target_arch = "x86_64")]
    if let Some(ref ramfs) = initramfs {
        // Convert the physical pool pointer to a virtual address through the
        // higher-half direct map.
        let phys_addr = ramfs.ptr.as_ptr() as u64;
        let virt_ptr = mm::addr::phys_to_virt(mm::addr::PhysAddr::new(phys_addr));
        let elf_data = unsafe { core::slice::from_raw_parts(virt_ptr.as_ptr::<u8>(), ramfs.len) };
        sched::spawn_user_from_elf(elf_data);
    }

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
