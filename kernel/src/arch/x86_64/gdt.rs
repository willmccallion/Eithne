//! Global Descriptor Table for x86_64.
//!
//! In long mode, the GDT is mostly vestigial — segment bases/limits are
//! ignored. We still need valid code/data segment descriptors and a TSS
//! for interrupt stack switching.

use core::cell::UnsafeCell;

use crate::mm;

/// Segment selectors (byte offsets into the GDT).
pub const KERNEL_CS: u16 = 0x08;
pub const KERNEL_DS: u16 = 0x10;
pub const _USER_CS: u16 = 0x18 | 3; // RPL=3
pub const _USER_DS: u16 = 0x20 | 3; // RPL=3
const TSS_SEL: u16 = 5 * 8; // 0x28 (TSS occupies entries 5 and 6)

/// null(1) + kernel_cs(1) + kernel_ds(1) + user_cs(1) + user_ds(1) + tss(2) = 7
const GDT_ENTRY_COUNT: usize = 7;

/// IST stack size: 2 pages (8 KiB) each.
const IST_STACK_PAGES: usize = 2;

// ---------------------------------------------------------------------------
// GDT entry encoding
// ---------------------------------------------------------------------------

/// A single 8-byte GDT segment descriptor.
#[derive(Clone, Copy)]
#[repr(transparent)]
struct GdtEntry(u64);

impl GdtEntry {
    const fn null() -> Self {
        Self(0)
    }

    /// Build a segment descriptor from component fields.
    const fn new(base: u32, limit: u32, access: u8, flags: u8) -> Self {
        let mut d: u64 = 0;
        d |= (limit & 0xFFFF) as u64;
        d |= ((base & 0xFFFF) as u64) << 16;
        d |= (((base >> 16) & 0xFF) as u64) << 32;
        d |= (access as u64) << 40;
        d |= (((limit >> 16) & 0x0F) as u64) << 48;
        d |= ((flags & 0x0F) as u64) << 52;
        d |= (((base >> 24) & 0xFF) as u64) << 56;
        Self(d)
    }

    /// Kernel code: P=1 DPL=0 S=1 E=1 RW=1 (0x9A), G=1 L=1 (0xA)
    const fn kernel_code() -> Self {
        Self::new(0, 0xFFFFF, 0x9A, 0xA)
    }

    /// Kernel data: P=1 DPL=0 S=1 E=0 RW=1 (0x92), G=1 Sz=1 (0xC)
    const fn kernel_data() -> Self {
        Self::new(0, 0xFFFFF, 0x92, 0xC)
    }

    /// User code: P=1 DPL=3 S=1 E=1 RW=1 (0xFA), G=1 L=1 (0xA)
    const fn user_code() -> Self {
        Self::new(0, 0xFFFFF, 0xFA, 0xA)
    }

    /// User data: P=1 DPL=3 S=1 E=0 RW=1 (0xF2), G=1 Sz=1 (0xC)
    const fn user_data() -> Self {
        Self::new(0, 0xFFFFF, 0xF2, 0xC)
    }
}

// ---------------------------------------------------------------------------
// TSS
// ---------------------------------------------------------------------------

/// Task State Segment (64-bit mode, 104 bytes).
#[repr(C, packed)]
struct Tss {
    _reserved0: u32,
    rsp: [u64; 3],
    _reserved1: u64,
    ist: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    iomap_base: u16,
}

impl Tss {
    const fn new() -> Self {
        Self {
            _reserved0: 0,
            rsp: [0; 3],
            _reserved1: 0,
            ist: [0; 7],
            _reserved2: 0,
            _reserved3: 0,
            iomap_base: core::mem::size_of::<Tss>() as u16,
        }
    }
}

/// Build the two u64 halves of a 64-bit TSS descriptor.
fn tss_descriptor(tss: &Tss) -> (u64, u64) {
    let base = tss as *const Tss as u64;
    let limit = (core::mem::size_of::<Tss>() - 1) as u64;

    let mut low: u64 = 0;
    low |= limit & 0xFFFF; // limit 0:15
    low |= (base & 0xFFFF) << 16; // base 0:15
    low |= ((base >> 16) & 0xFF) << 32; // base 16:23
    low |= 0x89u64 << 40; // access: P=1, type=0x9 (available 64-bit TSS)
    low |= ((limit >> 16) & 0xF) << 48; // limit 16:19
    low |= ((base >> 24) & 0xFF) << 56; // base 24:31

    let high = (base >> 32) & 0xFFFF_FFFF;

    (low, high)
}

// ---------------------------------------------------------------------------
// Static storage
// ---------------------------------------------------------------------------

#[repr(C, align(16))]
struct GdtStorage {
    entries: UnsafeCell<[u64; GDT_ENTRY_COUNT]>,
}
unsafe impl Sync for GdtStorage {}

static GDT_STORAGE: GdtStorage = GdtStorage {
    entries: UnsafeCell::new([0; GDT_ENTRY_COUNT]),
};

#[repr(C, align(16))]
struct TssStorage {
    tss: UnsafeCell<Tss>,
}
unsafe impl Sync for TssStorage {}

static TSS_STORAGE: TssStorage = TssStorage {
    tss: UnsafeCell::new(Tss::new()),
};

// ---------------------------------------------------------------------------
// GDTR pseudo-descriptor
// ---------------------------------------------------------------------------

#[repr(C, packed)]
struct GdtPointer {
    limit: u16,
    base: u64,
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the GDT with TSS and IST stacks.
/// Must be called after mm::init() (needs frame allocator for IST stacks).
pub fn init() {
    crate::println!("[gdt] initializing");

    // Allocate IST stacks (order 1 = 2 pages = 8 KiB each).
    let order = mm::page::pages_to_order(IST_STACK_PAGES);
    let ist1_bottom = mm::frame::alloc_pages(order).expect("failed to allocate IST1 stack");
    let ist2_bottom = mm::frame::alloc_pages(order).expect("failed to allocate IST2 stack");

    // IST entries point to the TOP of the stack (grows downward).
    let ist1_top = ist1_bottom.as_u64() + (IST_STACK_PAGES * mm::page::PAGE_SIZE) as u64;
    let ist2_top = ist2_bottom.as_u64() + (IST_STACK_PAGES * mm::page::PAGE_SIZE) as u64;

    // Populate TSS.
    let tss = unsafe { &mut *TSS_STORAGE.tss.get() };
    tss.ist[0] = ist1_top; // IST1 — double fault
    tss.ist[1] = ist2_top; // IST2 — NMI

    // Build GDT entries.
    let entries = unsafe { &mut *GDT_STORAGE.entries.get() };
    entries[0] = GdtEntry::null().0;
    entries[1] = GdtEntry::kernel_code().0;
    entries[2] = GdtEntry::kernel_data().0;
    entries[3] = GdtEntry::user_code().0;
    entries[4] = GdtEntry::user_data().0;
    let (tss_low, tss_high) = tss_descriptor(tss);
    entries[5] = tss_low;
    entries[6] = tss_high;

    // Load GDT.
    let gdtr = GdtPointer {
        limit: (GDT_ENTRY_COUNT * 8 - 1) as u16,
        base: entries.as_ptr() as u64,
    };
    unsafe {
        core::arch::asm!("lgdt [{}]", in(reg) &gdtr, options(nostack));
    }

    // Reload segment registers.
    reload_segments();

    // Load TSS.
    unsafe {
        core::arch::asm!("ltr {0:x}", in(reg) TSS_SEL, options(nostack, nomem));
    }

    crate::println!(
        "[gdt] loaded — CS={:#x} DS={:#x} TSS={:#x} IST1={:#x} IST2={:#x}",
        KERNEL_CS,
        KERNEL_DS,
        TSS_SEL,
        ist1_top,
        ist2_top
    );
}

/// Reload CS via far return and set DS/SS/ES to kernel data selector.
fn reload_segments() {
    unsafe {
        core::arch::asm!(
            "push {kcs}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {kds:x}",
            "mov es, {kds:x}",
            "mov ss, {kds:x}",
            "xor {zero:e}, {zero:e}",
            "mov fs, {zero:x}",
            "mov gs, {zero:x}",
            kcs = in(reg) KERNEL_CS as u64,
            kds = in(reg) KERNEL_DS as u64,
            tmp = out(reg) _,
            zero = out(reg) _,
            options(preserves_flags),
        );
    }
}
