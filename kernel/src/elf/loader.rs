//! ELF segment loader.
//!
//! Maps PT_LOAD segments from a parsed ELF64 binary into a new user address
//! space. Handles relocation when the ELF's linked addresses conflict with
//! the kernel identity map.

use crate::arch::x86_64::paging::KernelAddressSpace;
use crate::mm::addr::{PhysAddr, VirtAddr, phys_to_virt};
use crate::mm::frame;
use crate::mm::page::PAGE_SIZE;
use crate::mm::traits::{PageFlags, PageMapper};

use super::parser::{Elf64Info, Elf64Phdr, PF_W, PF_X, R_X86_64_RELATIVE};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ElfLoadError {
    Parse(super::parser::ElfParseError),
    NoFreeUserSlot,
    FrameAlloc,
    Map(crate::mm::traits::MapError),
    BadSegmentData,
    Relocation,
}

impl From<super::parser::ElfParseError> for ElfLoadError {
    fn from(e: super::parser::ElfParseError) -> Self {
        Self::Parse(e)
    }
}

impl From<crate::mm::traits::MapError> for ElfLoadError {
    fn from(e: crate::mm::traits::MapError) -> Self {
        Self::Map(e)
    }
}

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

pub struct LoadedElf {
    pub aspace: KernelAddressSpace,
    pub entry_point: u64,
    pub stack_top: u64,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of pages for the user stack (64 KiB).
const USER_STACK_PAGES: usize = 16;

/// Offset from user_base for the stack top (high within the PML4 slot,
/// well above where ELF segments are typically linked).
const USER_STACK_OFFSET: u64 = 0x7FFF_0000;

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

/// Load an ELF binary into a new user address space.
///
/// Creates a fresh address space with kernel mappings copied, finds a free
/// PML4 slot, and maps all PT_LOAD segments with an offset to avoid conflicts
/// with the kernel identity map.
pub fn load_elf(elf: &Elf64Info) -> Result<LoadedElf, ElfLoadError> {
    let mut aspace = KernelAddressSpace::new_with_kernel_mappings().map_err(ElfLoadError::Map)?;

    let user_base = aspace
        .find_free_user_base()
        .ok_or(ElfLoadError::NoFreeUserSlot)?;

    // The relocation offset: added to every p_vaddr in the ELF.
    let reloc_offset = user_base;

    // Map each PT_LOAD segment.
    for phdr in elf.load_segments() {
        map_segment(&mut aspace, elf, phdr, reloc_offset)?;
    }

    // Apply relocations for PIE binaries.
    apply_relocations(&aspace, elf, reloc_offset)?;

    // Map user stack.
    let stack_top = user_base + USER_STACK_OFFSET;
    let stack_bottom = stack_top - (USER_STACK_PAGES * PAGE_SIZE) as u64;
    for i in 0..USER_STACK_PAGES {
        let page_virt = stack_bottom + (i * PAGE_SIZE) as u64;
        let page_phys = frame::alloc_frame().ok_or(ElfLoadError::FrameAlloc)?;
        // Zero the stack page.
        unsafe {
            let dst = phys_to_virt(page_phys).as_mut_ptr::<u8>();
            core::ptr::write_bytes(dst, 0, PAGE_SIZE);
        }
        aspace.map(VirtAddr::new(page_virt), page_phys, PageFlags::USER_RW)?;
    }

    let entry_point = elf.entry_point() + reloc_offset;

    Ok(LoadedElf {
        aspace,
        entry_point,
        stack_top,
    })
}

/// Map a single PT_LOAD segment into the address space.
fn map_segment(
    aspace: &mut KernelAddressSpace,
    elf: &Elf64Info,
    phdr: &Elf64Phdr,
    reloc_offset: u64,
) -> Result<(), ElfLoadError> {
    let vaddr = phdr.p_vaddr + reloc_offset;
    let memsz = phdr.p_memsz as usize;
    let filesz = phdr.p_filesz as usize;

    if memsz == 0 {
        return Ok(());
    }

    // Get file data for this segment.
    let file_data = if filesz > 0 {
        elf.segment_data(phdr).ok_or(ElfLoadError::BadSegmentData)?
    } else {
        &[]
    };

    // Page-align the region.
    let page_align = PAGE_SIZE as u64;
    let aligned_start = vaddr & !(page_align - 1);
    let offset_in_first_page = (vaddr - aligned_start) as usize;
    let aligned_end = (vaddr + memsz as u64 + page_align - 1) & !(page_align - 1);
    let total_pages = ((aligned_end - aligned_start) as usize) / PAGE_SIZE;

    // Convert p_flags to PageFlags.
    let flags = phdr_flags(phdr);

    // Track how many bytes of file data we've copied.
    let mut file_copied = 0usize;

    for page_idx in 0..total_pages {
        let page_virt = aligned_start + (page_idx * PAGE_SIZE) as u64;

        // Check if this page was already mapped by a previous segment.
        let (page_ptr, already_mapped) =
            if let Some(existing_phys) = aspace.translate(VirtAddr::new(page_virt)) {
                // Page already mapped — reuse the existing frame.
                let phys_page = PhysAddr::new(existing_phys.as_u64() & !(PAGE_SIZE as u64 - 1));
                (phys_to_virt(phys_page).as_mut_ptr::<u8>(), true)
            } else {
                // Allocate a new frame and zero it.
                let page_phys = frame::alloc_frame().ok_or(ElfLoadError::FrameAlloc)?;
                let ptr = phys_to_virt(page_phys).as_mut_ptr::<u8>();
                unsafe {
                    core::ptr::write_bytes(ptr, 0, PAGE_SIZE);
                }
                aspace.map(VirtAddr::new(page_virt), page_phys, flags)?;
                (ptr, false)
            };
        let _ = already_mapped;

        // Copy file data into this page.
        if file_copied < filesz {
            let dst_offset = if page_idx == 0 {
                offset_in_first_page
            } else {
                0
            };

            // Bytes available in this page for data
            let space_in_page = PAGE_SIZE - dst_offset;
            let data_remaining = filesz - file_copied;
            let copy_len = space_in_page.min(data_remaining);

            if copy_len > 0 {
                unsafe {
                    let dst = page_ptr.add(dst_offset);
                    let src = file_data[file_copied..file_copied + copy_len].as_ptr();
                    core::ptr::copy_nonoverlapping(src, dst, copy_len);
                }
                file_copied += copy_len;
            }
        }
    }

    Ok(())
}

/// Apply ELF relocations to the loaded binary.
///
/// For PIE (ET_DYN) binaries, patches R_X86_64_RELATIVE entries so that
/// absolute addresses in the GOT point to the correct relocated addresses.
/// For ET_EXEC binaries with no relocations, this is a no-op.
fn apply_relocations(
    aspace: &KernelAddressSpace,
    elf: &Elf64Info,
    reloc_offset: u64,
) -> Result<(), ElfLoadError> {
    let rela_entries = match elf.rela_entries() {
        Some(entries) => entries,
        None => return Ok(()), // No relocations — backwards compat with ET_EXEC
    };

    for rela in rela_entries {
        if rela.r_type() != R_X86_64_RELATIVE {
            continue;
        }

        let target_vaddr = rela.r_offset + reloc_offset;
        let value = (reloc_offset as i64 + rela.r_addend) as u64;

        write_to_user(aspace, target_vaddr, value)?;
    }

    Ok(())
}

/// Write a u64 value to a user-space virtual address via the kernel mapping.
///
/// Translates the user vaddr to a physical address, then accesses it through
/// the kernel's identity map to perform the write.
fn write_to_user(aspace: &KernelAddressSpace, vaddr: u64, value: u64) -> Result<(), ElfLoadError> {
    let phys = aspace
        .translate(VirtAddr::new(vaddr))
        .ok_or(ElfLoadError::Relocation)?;

    let kern_ptr = phys_to_virt(phys).as_mut_ptr::<u64>();

    // SAFETY: The page was mapped and zeroed by map_segment, and phys_to_virt
    // gives us a kernel-accessible pointer. We use write_unaligned because the
    // GOT entry may not be 8-byte aligned within the page.
    unsafe {
        kern_ptr.write_unaligned(value);
    }

    Ok(())
}

/// Convert ELF p_flags to kernel PageFlags.
fn phdr_flags(phdr: &Elf64Phdr) -> PageFlags {
    PageFlags {
        readable: true,
        writable: phdr.p_flags & PF_W != 0,
        executable: phdr.p_flags & PF_X != 0,
        user_accessible: true,
        cache_disable: false,
    }
}
