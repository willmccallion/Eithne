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
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode};

/// Loaded initramfs: physical pointer and size, surviving exit_boot_services.
struct InitRamfs {
    ptr: NonNull<u8>,
    len: usize,
}

/// GOP framebuffer info captured before exit_boot_services.
///
/// Contains only plain scalar values — no UEFI pointers — so it survives
/// after boot services are torn down.
pub struct FrameBufferInfo {
    pub base: u64,
    pub size: usize,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub is_bgr: bool,
}

#[entry]
fn efi_main() -> Status {
    arch::early_init();

    println!("[eithne] kernel booting...");
    println!("[eithne] architecture: {}", arch::ARCH_NAME);

    // Load init ELF from ESP before exiting boot services.
    let initramfs = load_init_from_esp();

    // Capture GOP framebuffer info before exiting boot services.
    let fb_info = capture_gop_info();

    // Exit UEFI boot services and get the final memory map.
    let mmap = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    let boot_info = mm::boot_info::BootMemoryInfo::from_uefi_mmap(&mmap);

    kernel_main(boot_info, initramfs, fb_info);

    Status::SUCCESS
}

/// Capture GOP framebuffer information before exiting boot services.
///
/// The physical framebuffer address remains valid after exit_boot_services
/// even though the protocol handle itself becomes invalid.
fn capture_gop_info() -> Option<FrameBufferInfo> {
    let handle = boot::get_handle_for_protocol::<GraphicsOutput>().ok()?;
    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(handle).ok()?;

    let mode_info = gop.current_mode_info();
    let (width, height) = mode_info.resolution();
    let stride = mode_info.stride();

    let is_bgr = match mode_info.pixel_format() {
        PixelFormat::Bgr => true,
        PixelFormat::Rgb => false,
        other => {
            println!("[gop] unsupported pixel format: {:?}", other);
            return None;
        }
    };

    let mut fb = gop.frame_buffer();
    let base = fb.as_mut_ptr() as u64;
    let size = fb.size();

    println!(
        "[gop] {}x{} stride={} {} base={:#x} size={:#x}",
        width,
        height,
        stride,
        if is_bgr { "BGR" } else { "RGB" },
        base,
        size
    );

    Some(FrameBufferInfo {
        base,
        size,
        width,
        height,
        stride,
        is_bgr,
    })
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

    println!(
        "[initramfs] loaded {} bytes from \\EFI\\eithne\\init",
        bytes_read
    );

    Some(InitRamfs {
        ptr: buf_ptr,
        len: bytes_read,
    })
}

fn kernel_main(
    boot_info: mm::boot_info::BootMemoryInfo,
    initramfs: Option<InitRamfs>,
    fb_info: Option<FrameBufferInfo>,
) {
    println!("[eithne] entering kernel_main");

    // 1. Memory management (frame allocator + heap).
    mm::init(&boot_info);

    // 2. Higher-half direct map: all physical memory at KERNEL_VIRT_BASE.
    //    Must happen before init_interrupts so GDT/IDT pointers are stable.
    //    Extend to cover the GOP framebuffer which may be at a high physical address.
    let max_phys = if let Some(ref fb) = fb_info {
        boot_info
            .max_phys_addr
            .as_u64()
            .max(fb.base + fb.size as u64)
    } else {
        boot_info.max_phys_addr.as_u64()
    };
    arch::init_higher_half(max_phys);
    mm::set_higher_half_offset(crate::mm::layout::KERNEL_VIRT_BASE);

    // 3. GDT, IDT, PIC, interrupts.
    arch::init_interrupts();

    // 4. Page table smoke test.
    arch::test_paging();

    // 5. Syscall interface.
    arch::init_syscalls();

    // 5.5. Virtual filesystem (ramfs root).
    fs::init();

    // 6. Serial input buffer + COM1 RX interrupts.
    drivers::serial::init();
    arch::enable_serial_rx();

    // 7. Framebuffer console (if GOP was available).
    if let Some(fb) = fb_info {
        drivers::fb::init(fb);
        drivers::fb::console::init();
        println!("[fb] framebuffer console initialized");
    }

    // 8. Store init binary in VFS so `exec("/bin/shell")` works.
    #[cfg(target_arch = "x86_64")]
    if let Some(ref ramfs) = initramfs {
        let phys_addr = ramfs.ptr.as_ptr() as u64;
        let virt_ptr = mm::addr::phys_to_virt(mm::addr::PhysAddr::new(phys_addr));
        let elf_data = unsafe { core::slice::from_raw_parts(virt_ptr.as_ptr::<u8>(), ramfs.len) };
        store_init_in_vfs(elf_data);
    }

    // 9. Scheduler.
    sched::init();

    sched::spawn("idle", idle_task);

    // 10. User-mode init process from ELF.
    #[cfg(target_arch = "x86_64")]
    if let Some(ref ramfs) = initramfs {
        let phys_addr = ramfs.ptr.as_ptr() as u64;
        let virt_ptr = mm::addr::phys_to_virt(mm::addr::PhysAddr::new(phys_addr));
        let elf_data = unsafe { core::slice::from_raw_parts(virt_ptr.as_ptr::<u8>(), ramfs.len) };
        sched::spawn_user_from_elf(elf_data);
    }

    println!("[eithne] initialization complete - entering scheduler loop.");

    // Yield into the scheduler.  From here the timer IRQ drives preemption.
    sched::yield_cpu();

    // Fallback: if no tasks are runnable, halt.
    arch::halt();
}

/// Store the init ELF binary in the VFS at `/bin/shell` so child processes
/// can be spawned via `exec("/bin/shell")`.
fn store_init_in_vfs(elf_data: &[u8]) {
    use fs::traits::OpenFlags;

    if let Err(e) = fs::vfs::mkdir("/bin") {
        println!("[init] warning: mkdir /bin failed: {:?}", e);
    }

    let handle = match fs::vfs::open(
        "/bin/shell",
        OpenFlags {
            read: false,
            write: true,
            create: true,
            truncate: true,
            append: false,
        },
    ) {
        Ok(h) => h,
        Err(e) => {
            println!("[init] failed to create /bin/shell: {:?}", e);
            return;
        }
    };

    match fs::vfs::write(handle, elf_data) {
        Ok(n) => println!("[init] wrote {} bytes to /bin/shell", n),
        Err(e) => println!("[init] write to /bin/shell failed: {:?}", e),
    }

    let _ = fs::vfs::close(handle);
}

fn idle_task() -> ! {
    loop {
        arch::halt_once();
    }
}
