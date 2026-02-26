pub mod addr;
pub mod boot_info;
pub mod frame;
pub mod heap;
pub mod layout;
pub mod page;
pub mod traits;

pub use addr::{PhysAddr, VirtAddr};
pub use traits::{FrameAllocator, PageMapper};

/// Initialize the entire memory management subsystem.
///
/// Called once during boot after `exit_boot_services()` with the parsed
/// UEFI memory information. After this returns, the `alloc` crate
/// (Vec, Box, String, etc.) is available.
pub fn init(boot_info: &boot_info::BootMemoryInfo) {
    crate::println!(
        "[mm] {} usable regions, {} pages ({} MiB), max_phys={}",
        boot_info.region_count,
        boot_info.total_pages,
        boot_info.total_pages * page::PAGE_SIZE as u64 / (1024 * 1024),
        boot_info.max_phys_addr,
    );

    // Initialize buddy frame allocator
    frame::init(boot_info);

    crate::println!(
        "[mm] buddy allocator ready — {} free pages ({} MiB)",
        frame::free_page_count(),
        frame::free_page_count() * page::PAGE_SIZE / (1024 * 1024),
    );

    // Heap allocator is already wired via #[global_allocator] in heap.rs.
    // It lazily calls frame::alloc_frame() on first use.
    // Verify it works:
    extern crate alloc;
    let v = alloc::vec![1u64, 2, 3, 4, 5];
    crate::println!("[mm] heap test: {:?}", v);
    drop(v);

    crate::println!("[mm] memory subsystem initialized");
}
