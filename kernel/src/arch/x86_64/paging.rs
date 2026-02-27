//! x86_64 4-level page table implementation.
//!
//! Supports the standard 48-bit virtual address space:
//!   PML4 (512 entries) → PDPT (512) → PD (512) → PT (512 × 4 KiB pages)
//!
//! All page tables are allocated from the global frame allocator and are
//! accessed via the identity mapping that UEFI established. When we later
//! move to a higher-half kernel the `phys_to_virt` shim in `mm::addr` will
//! be the only place that needs updating.

use core::arch::asm;

use crate::mm::addr::{PhysAddr, VirtAddr, phys_to_virt, virt_to_phys};
use crate::mm::traits::{
    AddressSpace, FrameAllocator, MapError, PageFlags, PageMapper, UnmapError,
};
use crate::mm::{frame, page::PAGE_SIZE};

// ---------------------------------------------------------------------------
// Page table entry flags
// ---------------------------------------------------------------------------

/// x86_64 page table entry flag bits.
#[allow(dead_code)]
mod flags {
    pub const PRESENT: u64 = 1 << 0;
    pub const WRITABLE: u64 = 1 << 1;
    pub const USER: u64 = 1 << 2;
    pub const WRITE_THROUGH: u64 = 1 << 3;
    pub const CACHE_DISABLE: u64 = 1 << 4;
    pub const ACCESSED: u64 = 1 << 5;
    pub const DIRTY: u64 = 1 << 6;
    pub const HUGE: u64 = 1 << 7;
    pub const GLOBAL: u64 = 1 << 8;
    /// No-Execute (requires EFER.NXE = 1, which UEFI enables).
    pub const NO_EXEC: u64 = 1 << 63;
    /// Mask that strips flags and extracts the 4 KiB-aligned physical address.
    pub const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;
}

// ---------------------------------------------------------------------------
// PageTableEntry
// ---------------------------------------------------------------------------

/// A single 8-byte entry in a page table at any level.
#[derive(Clone, Copy)]
#[repr(transparent)]
struct PageTableEntry(u64);

impl PageTableEntry {
    const fn empty() -> Self {
        Self(0)
    }

    fn is_present(self) -> bool {
        self.0 & flags::PRESENT != 0
    }

    fn phys_addr(self) -> PhysAddr {
        PhysAddr::new(self.0 & flags::ADDR_MASK)
    }

    /// Set this entry to point to the given physical frame with the given flags.
    fn set(&mut self, phys: PhysAddr, entry_flags: u64) {
        self.0 = (phys.as_u64() & flags::ADDR_MASK) | entry_flags;
    }

    fn clear(&mut self) {
        self.0 = 0;
    }

    /// Convert `PageFlags` to x86_64 hardware flag bits for an intermediate
    /// table entry (always present+writable; user bit follows the leaf).
    fn intermediate_flags(user: bool) -> u64 {
        let mut f = flags::PRESENT | flags::WRITABLE;
        if user {
            f |= flags::USER;
        }
        f
    }

    /// Convert `PageFlags` to x86_64 hardware flag bits for a leaf (4 KiB page).
    fn leaf_flags(pf: PageFlags) -> u64 {
        let mut f = flags::PRESENT;
        if pf.writable {
            f |= flags::WRITABLE;
        }
        if pf.user_accessible {
            f |= flags::USER;
        }
        if !pf.executable {
            f |= flags::NO_EXEC;
        }
        if pf.cache_disable {
            f |= flags::CACHE_DISABLE;
        }
        f
    }
}

// ---------------------------------------------------------------------------
// PageTable
// ---------------------------------------------------------------------------

/// A 512-entry page table (PML4, PDPT, PD, or PT).
/// Must be exactly one 4 KiB page and page-aligned.
#[repr(C, align(4096))]
struct PageTable {
    entries: [PageTableEntry; 512],
}

impl PageTable {
    /// Zero-initialize all entries.
    fn zero(&mut self) {
        for e in self.entries.iter_mut() {
            *e = PageTableEntry::empty();
        }
    }
}

// ---------------------------------------------------------------------------
// Index extraction from a virtual address
// ---------------------------------------------------------------------------

/// Split a virtual address into its four 9-bit page table indices and the
/// 12-bit page offset.
#[inline]
fn va_indices(va: VirtAddr) -> (usize, usize, usize, usize, usize) {
    let v = va.as_u64();
    let pml4 = ((v >> 39) & 0x1FF) as usize;
    let pdpt = ((v >> 30) & 0x1FF) as usize;
    let pd = ((v >> 21) & 0x1FF) as usize;
    let pt = ((v >> 12) & 0x1FF) as usize;
    let off = (v & 0xFFF) as usize;
    (pml4, pdpt, pd, pt, off)
}

// ---------------------------------------------------------------------------
// Helper: allocate and zero a new page table frame
// ---------------------------------------------------------------------------

/// Allocate one physical frame, zero it, and return a mutable reference to
/// it interpreted as a `PageTable`. Returns `None` on OOM.
///
/// # Safety
/// The caller must ensure the returned reference is not aliased.
unsafe fn alloc_table<'a>() -> Option<&'a mut PageTable> {
    let phys = frame::alloc_frame()?;
    let virt = phys_to_virt(phys);
    let table = unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) };
    table.zero();
    Some(table)
}

/// Obtain a mutable reference to the `PageTable` pointed to by an entry.
///
/// # Safety
/// The entry must be present and point to a valid page table frame.
unsafe fn table_from_entry<'a>(entry: PageTableEntry) -> &'a mut PageTable {
    let virt = phys_to_virt(entry.phys_addr());
    unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) }
}

// ---------------------------------------------------------------------------
// KernelAddressSpace
// ---------------------------------------------------------------------------

/// Owns an x86_64 4-level page table hierarchy.
///
/// `cr3` holds the physical address of the PML4 table.
pub struct KernelAddressSpace {
    cr3: PhysAddr,
}

impl KernelAddressSpace {
    /// Create a new, empty address space (a fresh PML4 table full of zeroes).
    pub fn new() -> Option<Self> {
        let table = unsafe { alloc_table()? };
        let cr3 = virt_to_phys(VirtAddr::new(table as *mut PageTable as u64));
        Some(Self { cr3 })
    }

    /// Build an address space from the currently active CR3 value.
    ///
    /// Used to wrap UEFI's existing page tables so we can extend them before
    /// switching to our own mapping.
    ///
    /// # Safety
    /// The current CR3 must point to a valid 4-level page table hierarchy.
    pub unsafe fn from_current_cr3() -> Self {
        let cr3: u64;
        unsafe {
            asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        }
        // Bits 0-11 of CR3 are flags (PCID, PWT, PCD); the address is 12-aligned.
        Self {
            cr3: PhysAddr::new(cr3 & flags::ADDR_MASK),
        }
    }

    /// Physical address of the PML4 table.
    pub fn cr3(&self) -> PhysAddr {
        self.cr3
    }

    fn pml4(&mut self) -> &mut PageTable {
        let virt = phys_to_virt(self.cr3);
        unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) }
    }

    fn pml4_ref(&self) -> &PageTable {
        let virt = phys_to_virt(self.cr3);
        unsafe { &*(virt.as_ptr::<PageTable>()) }
    }
}

impl PageMapper for KernelAddressSpace {
    fn map(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        page_flags: PageFlags,
    ) -> Result<(), MapError> {
        let (i4, i3, i2, i1, _off) = va_indices(virt);

        if !virt.is_aligned(PAGE_SIZE as u64) || !phys.is_aligned(PAGE_SIZE as u64) {
            return Err(MapError::InvalidAddress);
        }

        let pml4 = self.pml4();

        // PML4 → PDPT
        let pdpt: &mut PageTable = if pml4.entries[i4].is_present() {
            unsafe { table_from_entry(pml4.entries[i4]) }
        } else {
            let t = unsafe { alloc_table().ok_or(MapError::FrameAllocationFailed)? };
            pml4.entries[i4].set(
                virt_to_phys(VirtAddr::new(t as *mut PageTable as u64)),
                PageTableEntry::intermediate_flags(page_flags.user_accessible),
            );
            t
        };

        // PDPT → PD
        let pd: &mut PageTable = if pdpt.entries[i3].is_present() {
            unsafe { table_from_entry(pdpt.entries[i3]) }
        } else {
            let t = unsafe { alloc_table().ok_or(MapError::FrameAllocationFailed)? };
            pdpt.entries[i3].set(
                virt_to_phys(VirtAddr::new(t as *mut PageTable as u64)),
                PageTableEntry::intermediate_flags(page_flags.user_accessible),
            );
            t
        };

        // PD → PT
        let pt: &mut PageTable = if pd.entries[i2].is_present() {
            unsafe { table_from_entry(pd.entries[i2]) }
        } else {
            let t = unsafe { alloc_table().ok_or(MapError::FrameAllocationFailed)? };
            pd.entries[i2].set(
                virt_to_phys(VirtAddr::new(t as *mut PageTable as u64)),
                PageTableEntry::intermediate_flags(page_flags.user_accessible),
            );
            t
        };

        // PT → leaf
        if pt.entries[i1].is_present() {
            return Err(MapError::AlreadyMapped);
        }
        pt.entries[i1].set(phys, PageTableEntry::leaf_flags(page_flags));

        Ok(())
    }

    fn unmap(&mut self, virt: VirtAddr) -> Result<PhysAddr, UnmapError> {
        let (i4, i3, i2, i1, _off) = va_indices(virt);

        let pml4 = self.pml4();

        let pdpt_entry = &mut pml4.entries[i4];
        if !pdpt_entry.is_present() {
            return Err(UnmapError::NotMapped);
        }
        let pdpt = unsafe { table_from_entry(*pdpt_entry) };

        let pd_entry = &mut pdpt.entries[i3];
        if !pd_entry.is_present() {
            return Err(UnmapError::NotMapped);
        }
        let pd = unsafe { table_from_entry(*pd_entry) };

        let pt_entry = &mut pd.entries[i2];
        if !pt_entry.is_present() {
            return Err(UnmapError::NotMapped);
        }
        let pt = unsafe { table_from_entry(*pt_entry) };

        let leaf = &mut pt.entries[i1];
        if !leaf.is_present() {
            return Err(UnmapError::NotMapped);
        }

        let phys = leaf.phys_addr();
        leaf.clear();
        invlpg(virt);

        Ok(phys)
    }

    fn translate(&self, virt: VirtAddr) -> Option<PhysAddr> {
        let (i4, i3, i2, i1, off) = va_indices(virt);

        let pml4 = self.pml4_ref();

        let e4 = pml4.entries[i4];
        if !e4.is_present() {
            return None;
        }
        let pdpt = unsafe { &*(phys_to_virt(e4.phys_addr()).as_ptr::<PageTable>()) };

        let e3 = pdpt.entries[i3];
        if !e3.is_present() {
            return None;
        }
        // 1 GiB huge page
        if e3.0 & flags::HUGE != 0 {
            return Some(PhysAddr::new(
                (e3.0 & 0x000F_FFFF_C000_0000) + (virt.as_u64() & 0x3FFF_FFFF),
            ));
        }
        let pd = unsafe { &*(phys_to_virt(e3.phys_addr()).as_ptr::<PageTable>()) };

        let e2 = pd.entries[i2];
        if !e2.is_present() {
            return None;
        }
        // 2 MiB huge page
        if e2.0 & flags::HUGE != 0 {
            return Some(PhysAddr::new(
                (e2.0 & 0x000F_FFFF_FFE0_0000) + (virt.as_u64() & 0x001F_FFFF),
            ));
        }
        let pt = unsafe { &*(phys_to_virt(e2.phys_addr()).as_ptr::<PageTable>()) };

        let e1 = pt.entries[i1];
        if !e1.is_present() {
            return None;
        }
        Some(PhysAddr::new(e1.phys_addr().as_u64() + off as u64))
    }
}

impl AddressSpace for KernelAddressSpace {
    fn activate(&self) {
        unsafe {
            asm!("mov cr3, {}", in(reg) self.cr3.as_u64(), options(nomem, nostack));
        }
    }

    fn map_region(
        &mut self,
        virt_start: VirtAddr,
        phys_start: PhysAddr,
        size: usize,
        page_flags: PageFlags,
    ) -> Result<(), MapError> {
        let mut offset = 0u64;
        while offset < size as u64 {
            self.map(
                virt_start.offset(offset),
                phys_start.offset(offset),
                page_flags,
            )?;
            offset += PAGE_SIZE as u64;
        }
        Ok(())
    }

    fn unmap_region(&mut self, virt_start: VirtAddr, size: usize) -> Result<(), UnmapError> {
        let mut offset = 0u64;
        while offset < size as u64 {
            self.unmap(virt_start.offset(offset))?;
            offset += PAGE_SIZE as u64;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TLB flush
// ---------------------------------------------------------------------------

/// Invalidate the TLB entry for a single virtual address.
#[inline]
pub fn invlpg(virt: VirtAddr) {
    unsafe {
        asm!("invlpg [{}]", in(reg) virt.as_u64(), options(nostack, preserves_flags));
    }
}

/// Flush the entire TLB by reloading CR3.
#[inline]
pub fn flush_tlb() {
    unsafe {
        let cr3: u64;
        asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        asm!("mov cr3, {}", in(reg) cr3, options(nomem, nostack));
    }
}

// ---------------------------------------------------------------------------
// Higher-half setup
// ---------------------------------------------------------------------------

/// Map all of physical memory at `KERNEL_VIRT_BASE` using 1 GiB huge pages,
/// keeping the existing identity map intact, then load the new CR3.
///
/// After this returns, every physical address `p` is accessible at both
/// `p` (identity) and `KERNEL_VIRT_BASE + p` (direct map).
/// `mm::addr::phys_to_virt` is updated separately to use the new offset.
///
/// Uses 1 GiB pages (PDPT huge entries) to map `max_phys` bytes with
/// minimal page table overhead: one PDPT per 512 GiB, one entry per GiB.
pub fn init_higher_half(max_phys: u64) {
    use crate::mm::layout::KERNEL_VIRT_BASE;

    // Round max_phys up to the next 1 GiB boundary.
    const GIB: u64 = 1 << 30;
    let gib_count = max_phys.div_ceil(GIB) as usize;

    // Allocate a fresh PML4 (identity-mapped, so phys == virt right now).
    let pml4_phys = frame::alloc_frame().expect("[paging] OOM for higher-half PML4");
    let pml4 = unsafe { &mut *(phys_to_virt(pml4_phys).as_mut_ptr::<PageTable>()) };
    pml4.zero();

    // --- Copy existing UEFI identity-map entries ---
    // Read current CR3 and clone its PML4 entries into the new PML4.
    // This preserves UEFI runtime services mappings and lets us keep
    // executing after the CR3 switch.
    let old_cr3: u64;
    unsafe { asm!("mov {}, cr3", out(reg) old_cr3, options(nomem, nostack)) };
    let old_pml4 = unsafe {
        &*(phys_to_virt(PhysAddr::new(old_cr3 & flags::ADDR_MASK)).as_ptr::<PageTable>())
    };
    for i in 0..512 {
        pml4.entries[i] = old_pml4.entries[i];
    }

    // --- Build the direct-map region at KERNEL_VIRT_BASE ---
    // KERNEL_VIRT_BASE = 0xFFFF_8000_0000_0000
    // PML4 index for that address:
    let base_pml4_idx = ((KERNEL_VIRT_BASE >> 39) & 0x1FF) as usize;

    // Each PML4 entry covers 512 GiB.  We need ceil(gib_count / 512) PDPTs.
    let pdpt_count = gib_count.div_ceil(512);

    for pdpt_i in 0..pdpt_count {
        let pdpt_phys = frame::alloc_frame().expect("[paging] OOM for higher-half PDPT");
        let pdpt = unsafe { &mut *(phys_to_virt(pdpt_phys).as_mut_ptr::<PageTable>()) };
        pdpt.zero();

        // Fill this PDPT with 1 GiB huge page entries.
        let gib_base = pdpt_i as u64 * 512 * GIB;
        for entry_i in 0..512usize {
            let phys_gib = gib_base + entry_i as u64 * GIB;
            if phys_gib >= max_phys.div_ceil(GIB) * GIB && phys_gib >= max_phys {
                break;
            }
            // 1 GiB page entry: PRESENT | WRITABLE | HUGE | GLOBAL | NO_EXEC
            pdpt.entries[entry_i].0 = (phys_gib & 0x000F_FFFF_C000_0000)
                | flags::PRESENT
                | flags::WRITABLE
                | flags::HUGE
                | flags::GLOBAL
                | flags::NO_EXEC;
        }

        // Wire this PDPT into the new PML4.
        let pml4_idx = base_pml4_idx + pdpt_i;
        assert!(pml4_idx < 512, "[paging] higher-half exceeds PML4 bounds");
        pml4.entries[pml4_idx].set(pdpt_phys, flags::PRESENT | flags::WRITABLE);
    }

    // Load the new PML4.
    unsafe {
        asm!("mov cr3, {}", in(reg) pml4_phys.as_u64(), options(nomem, nostack));
    }

    crate::println!(
        "[paging] higher-half direct map: {} GiB at {:#x}",
        gib_count,
        KERNEL_VIRT_BASE,
    );
}

// ---------------------------------------------------------------------------
// Smoke test called from mm::init
// ---------------------------------------------------------------------------

/// Basic sanity check: allocate a fresh address space, map one page, translate
/// it, unmap it, and verify translate returns None.
pub fn test_paging() {
    let mut aspace = KernelAddressSpace::new().expect("[paging] OOM creating test address space");

    // Use a scratch virtual address that won't conflict with identity map
    let test_virt = VirtAddr::new(0x0000_1234_5678_0000);
    let test_phys = frame::alloc_frame().expect("[paging] OOM allocating test frame");

    aspace
        .map(test_virt, test_phys, PageFlags::KERNEL_RW)
        .expect("[paging] map failed");

    let translated = aspace
        .translate(test_virt)
        .expect("[paging] translate returned None");
    assert_eq!(
        translated.as_u64(),
        test_phys.as_u64(),
        "[paging] translate returned wrong address"
    );

    aspace.unmap(test_virt).expect("[paging] unmap failed");

    assert!(
        aspace.translate(test_virt).is_none(),
        "[paging] translate should return None after unmap"
    );

    frame::dealloc_frame(test_phys);

    crate::println!("[paging] page table smoke test passed");
}
