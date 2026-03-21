//! eithne UEFI kernel.
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

/// Maximum number of ELF binaries loaded from ESP at boot.
const MAX_ESP_BINARIES: usize = 16;

/// A single ELF binary loaded from the ESP into LOADER_DATA memory.
struct EspBinary {
    /// Physical pointer to the loaded data.
    ptr: NonNull<u8>,
    /// Size in bytes.
    len: usize,
    /// Target VFS path (e.g. "/bin/shell").
    vfs_path: &'static str,
}

/// Collection of ELF binaries loaded from ESP before `exit_boot_services`.
struct EspBinaries {
    /// Loaded binaries.
    entries: [Option<EspBinary>; MAX_ESP_BINARIES],
    /// Number of entries loaded.
    count: usize,
}

/// GOP framebuffer info captured before exit_boot_services.
///
/// Contains only plain scalar values — no UEFI pointers — so it survives
/// after boot services are torn down.
#[derive(Debug)]
pub struct FrameBufferInfo {
    /// Framebuffer base address.
    pub base: u64,
    /// Size in bytes.
    pub size: usize,
    /// Width in pixels.
    pub width: usize,
    /// Height in pixels.
    pub height: usize,
    /// Stride in pixels.
    pub stride: usize,
    /// Whether pixel format is BGR.
    pub is_bgr: bool,
}

#[allow(clippy::expect_used)] // Boot-time entry point: panicking is appropriate for fatal init failures.
#[entry]
fn efi_main() -> Status {
    arch::early_init();

    println!("[eithne] kernel booting...");
    println!("[eithne] architecture: {}", arch::ARCH_NAME);

    // Load ELF binaries from ESP before exiting boot services.
    let esp_bins = load_esp_binaries();

    // Capture GOP framebuffer info before exiting boot services.
    let fb_info = capture_gop_info();

    // Exit UEFI boot services and get the final memory map.
    let mmap = unsafe { boot::exit_boot_services(MemoryType::LOADER_DATA) };

    let boot_info = mm::boot_info::BootMemoryInfo::from_uefi_mmap(&mmap);

    kernel_main(&boot_info, &esp_bins, fb_info.as_ref());

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

/// Load user-space ELF binaries from the ESP into LOADER_DATA pool allocations.
///
/// Loads the init shell and all `/bin/` utilities so they survive
/// `exit_boot_services` and can be placed into the VFS later.
#[allow(clippy::expect_used)] // Boot-time: panicking is appropriate if ESP is unreadable.
fn load_esp_binaries() -> EspBinaries {
    #[repr(align(8))]
    struct AlignedBuf([u8; 256]);

    const NONE: Option<EspBinary> = None;

    let binaries: &[(&uefi::CStr16, &str)] = &[
        (cstr16!(r"\EFI\eithne\init"), "/bin/shell"),
        (cstr16!(r"\EFI\eithne\bin\cat"), "/bin/cat"),
        (cstr16!(r"\EFI\eithne\bin\ls"), "/bin/ls"),
        (cstr16!(r"\EFI\eithne\bin\mkdir"), "/bin/mkdir"),
        (cstr16!(r"\EFI\eithne\bin\rm"), "/bin/rm"),
        (cstr16!(r"\EFI\eithne\bin\stat"), "/bin/stat"),
        (cstr16!(r"\EFI\eithne\bin\write"), "/bin/write"),
    ];

    let mut sfs = boot::get_image_file_system(boot::image_handle())
        .expect("[esp] failed to open ESP filesystem");
    let mut root = sfs.open_volume().expect("[esp] failed to open root");

    let mut bins = EspBinaries {
        entries: [NONE; MAX_ESP_BINARIES],
        count: 0,
    };

    for &(esp_path, vfs_path) in binaries {
        let Ok(handle) = root.open(esp_path, FileMode::Read, FileAttribute::empty()) else {
            println!("[esp] skipping {} (not found)", vfs_path);
            continue;
        };

        let Some(mut file) = handle.into_regular_file() else { continue };

        let mut info_buf = AlignedBuf([0u8; 256]);
        let Ok(info) = file.get_info::<FileInfo>(&mut info_buf.0) else { continue };
        let file_size = info.file_size() as usize;
        if file_size == 0 {
            continue;
        }

        let buf_ptr = boot::allocate_pool(MemoryType::LOADER_DATA, file_size)
            .expect("[esp] failed to allocate pool");
        let buf_slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr.as_ptr(), file_size) };
        let bytes_read = file.read(buf_slice).expect("[esp] failed to read file");

        println!("[esp] loaded {} ({} bytes)", vfs_path, bytes_read);

        if bins.count < MAX_ESP_BINARIES {
            bins.entries[bins.count] = Some(EspBinary {
                ptr: buf_ptr,
                len: bytes_read,
                vfs_path,
            });
            bins.count += 1;
        }
    }

    bins
}

fn kernel_main(
    boot_info: &mm::boot_info::BootMemoryInfo,
    esp_bins: &EspBinaries,
    fb_info: Option<&FrameBufferInfo>,
) {
    println!("[eithne] entering kernel_main");

    // 1. Memory management (frame allocator + heap).
    mm::init(boot_info);

    // 2. PCI bus enumeration (I/O ports only — no MMIO yet).
    //    Must happen before init_higher_half so we know the max BAR address
    //    and can map enough physical address space for PCI MMIO regions.
    drivers::pci::init();

    // 3. Higher-half direct map: all physical memory at KERNEL_VIRT_BASE.
    //    Must happen before init_interrupts so GDT/IDT pointers are stable.
    //    Cover RAM, GOP framebuffer, and PCI MMIO BARs (which OVMF may
    //    place above 4 GiB for 64-bit BARs).
    let mut max_phys = boot_info.max_phys_addr.as_u64();
    if let Some(fb) = fb_info {
        max_phys = max_phys.max(fb.base + fb.size as u64);
    }
    max_phys = max_phys.max(drivers::pci::max_bar_address());
    arch::init_higher_half(max_phys);
    mm::set_higher_half_offset(crate::mm::layout::KERNEL_VIRT_BASE);

    // 3.5. Snapshot boot page table before any user processes.
    arch::save_boot_mappings();

    // 4. GDT, IDT, PIC, interrupts.
    arch::init_interrupts();

    // 5. Page table smoke test.
    arch::test_paging();

    // 6. Syscall interface.
    arch::init_syscalls();

    // 6.5. Virtual filesystem (ramfs root).
    fs::init();

    // 7. Serial input buffer + COM1 RX interrupts.
    drivers::serial::init();
    arch::enable_serial_rx();

    // 7.5. Block device drivers (needs PCI + higher-half for MMIO).
    drivers::block::init();

    // 8. Framebuffer console (if GOP was available).
    if let Some(fb) = fb_info {
        drivers::fb::init(fb);
        drivers::fb::console::init();
        println!("[fb] framebuffer console initialized");
    }

    // 9. Store ESP binaries in VFS so exec() can find them.
    #[cfg(target_arch = "x86_64")]
    store_esp_binaries_in_vfs(esp_bins);

    // 10. Scheduler.
    sched::init();

    // 11. Pipe subsystem.
    ipc::pipe::init();

    // 12. Signal subsystem.
    ipc::signal::init();

    let _ = sched::spawn("idle", idle_task);

    // 13. User-mode init process from ELF (first ESP binary = shell).
    #[cfg(target_arch = "x86_64")]
    if let Some(ref bin) = esp_bins.entries[0] {
        let phys_addr = bin.ptr.as_ptr() as u64;
        let virt_ptr = mm::addr::phys_to_virt(mm::addr::PhysAddr::new(phys_addr));
        let elf_data = unsafe { core::slice::from_raw_parts(virt_ptr.as_ptr::<u8>(), bin.len) };
        let _ = sched::spawn_user_from_elf(elf_data);
    }

    println!("[eithne] initialization complete - entering scheduler loop.");

    // Yield into the scheduler.  From here the timer IRQ drives preemption.
    sched::yield_cpu();

    // Fallback: if no tasks are runnable, halt.
    arch::halt();
}

/// Store all ESP-loaded binaries into the VFS.
fn store_esp_binaries_in_vfs(bins: &EspBinaries) {
    use fs::traits::OpenFlags;

    // Create standard Unix directory hierarchy.
    for dir in ["/bin", "/dev", "/etc", "/home", "/tmp", "/var"] {
        if let Err(e) = fs::vfs::mkdir(dir) {
            println!("[init] warning: mkdir {} failed: {:?}", dir, e);
        }
    }

    for entry in &bins.entries[..bins.count] {
        let Some(bin) = entry else { continue };

        let phys_addr = bin.ptr.as_ptr() as u64;
        let virt_ptr = mm::addr::phys_to_virt(mm::addr::PhysAddr::new(phys_addr));
        let elf_data = unsafe { core::slice::from_raw_parts(virt_ptr.as_ptr::<u8>(), bin.len) };

        let handle = match fs::vfs::open(
            bin.vfs_path,
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
                println!("[init] failed to create {}: {:?}", bin.vfs_path, e);
                continue;
            }
        };

        match fs::vfs::write(handle, elf_data) {
            Ok(n) => println!("[init] wrote {} bytes to {}", n, bin.vfs_path),
            Err(e) => println!("[init] write to {} failed: {:?}", bin.vfs_path, e),
        }

        let _ = fs::vfs::close(handle);
    }
}

fn idle_task() -> ! {
    loop {
        arch::halt_once();
    }
}
