//! ELF64 binary parser.
//!
//! Hand-rolled, no external crates. Validates headers and provides access
//! to PT_LOAD program headers for the ELF loader.

use core::mem;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1; // Little-endian
const ET_EXEC: u16 = 2; // Executable
const ET_DYN: u16 = 3; // Position-independent executable
const EM_X86_64: u16 = 62;

pub const PT_LOAD: u32 = 1;
pub const PT_DYNAMIC: u32 = 2;
pub const PF_X: u32 = 1;
pub const PF_W: u32 = 2;
pub const PF_R: u32 = 4;

// Dynamic section tag constants
const DT_NULL: i64 = 0;
const DT_RELA: i64 = 7;
const DT_RELASZ: i64 = 8;

// Relocation type constants
pub const R_X86_64_RELATIVE: u32 = 8;

// ---------------------------------------------------------------------------
// ELF64 header (64 bytes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Elf64Header {
    pub e_ident: [u8; 16],
    pub e_type: u16,
    pub e_machine: u16,
    pub e_version: u32,
    pub e_entry: u64,
    pub e_phoff: u64,
    pub e_shoff: u64,
    pub e_flags: u32,
    pub e_ehsize: u16,
    pub e_phentsize: u16,
    pub e_phnum: u16,
    pub e_shentsize: u16,
    pub e_shnum: u16,
    pub e_shstrndx: u16,
}

const _: () = assert!(mem::size_of::<Elf64Header>() == 64);

// ---------------------------------------------------------------------------
// ELF64 program header (56 bytes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Elf64Phdr {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

const _: () = assert!(mem::size_of::<Elf64Phdr>() == 56);

// ---------------------------------------------------------------------------
// ELF64 relocation entry with addend (24 bytes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct Elf64Rela {
    pub r_offset: u64,
    pub r_info: u64,
    pub r_addend: i64,
}

const _: () = assert!(mem::size_of::<Elf64Rela>() == 24);

impl Elf64Rela {
    /// Extract the relocation type from r_info.
    pub fn r_type(&self) -> u32 {
        (self.r_info & 0xFFFF_FFFF) as u32
    }
}

// ---------------------------------------------------------------------------
// ELF64 dynamic section entry (16 bytes)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
#[repr(C)]
struct Elf64Dyn {
    d_tag: i64,
    d_val: u64,
}

const _: () = assert!(mem::size_of::<Elf64Dyn>() == 16);

// ---------------------------------------------------------------------------
// Parse errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ElfParseError {
    TooSmall,
    BadMagic,
    Not64Bit,
    NotLittleEndian,
    NotExecutable,
    NotX86_64,
    BadPhdrSize,
    Truncated,
}

// ---------------------------------------------------------------------------
// Parsed ELF info
// ---------------------------------------------------------------------------

/// Parsed and validated ELF64 binary.
pub struct Elf64Info<'a> {
    data: &'a [u8],
    header: &'a Elf64Header,
}

impl<'a> Elf64Info<'a> {
    /// Parse and validate an ELF64 executable from raw bytes.
    pub fn parse(data: &'a [u8]) -> Result<Self, ElfParseError> {
        if data.len() < mem::size_of::<Elf64Header>() {
            return Err(ElfParseError::TooSmall);
        }

        // SAFETY: We checked the slice is large enough, and Elf64Header is
        // repr(C) with no padding requirements beyond what u8 provides.
        let header = unsafe { &*(data.as_ptr() as *const Elf64Header) };

        // Validate ELF identification
        if header.e_ident[0..4] != ELF_MAGIC {
            return Err(ElfParseError::BadMagic);
        }
        if header.e_ident[4] != ELFCLASS64 {
            return Err(ElfParseError::Not64Bit);
        }
        if header.e_ident[5] != ELFDATA2LSB {
            return Err(ElfParseError::NotLittleEndian);
        }
        if header.e_type != ET_EXEC && header.e_type != ET_DYN {
            return Err(ElfParseError::NotExecutable);
        }
        if header.e_machine != EM_X86_64 {
            return Err(ElfParseError::NotX86_64);
        }
        if header.e_phentsize as usize != mem::size_of::<Elf64Phdr>() {
            return Err(ElfParseError::BadPhdrSize);
        }

        // Verify program header table fits within the data
        let phdr_end =
            header.e_phoff as usize + (header.e_phnum as usize) * mem::size_of::<Elf64Phdr>();
        if phdr_end > data.len() {
            return Err(ElfParseError::Truncated);
        }

        Ok(Self { data, header })
    }

    /// Entry point virtual address from the ELF header.
    pub fn entry_point(&self) -> u64 {
        self.header.e_entry
    }

    /// Iterate over PT_LOAD program headers.
    pub fn load_segments(&self) -> impl Iterator<Item = &Elf64Phdr> {
        let count = self.header.e_phnum as usize;
        let base = self.header.e_phoff as usize;

        (0..count).filter_map(move |i| {
            let offset = base + i * mem::size_of::<Elf64Phdr>();
            // SAFETY: We validated that the phdr table fits within data in parse().
            let phdr = unsafe { &*(self.data.as_ptr().add(offset) as *const Elf64Phdr) };
            if phdr.p_type == PT_LOAD {
                Some(phdr)
            } else {
                None
            }
        })
    }

    /// Get the file data slice for a program header segment.
    pub fn segment_data(&self, phdr: &Elf64Phdr) -> Option<&[u8]> {
        let start = phdr.p_offset as usize;
        let end = start + phdr.p_filesz as usize;
        if end <= self.data.len() {
            Some(&self.data[start..end])
        } else {
            None
        }
    }

    /// True if this is a position-independent executable (ET_DYN).
    pub fn is_pie(&self) -> bool {
        self.header.e_type == ET_DYN
    }

    /// Iterate over all program headers.
    fn phdrs(&self) -> impl Iterator<Item = &Elf64Phdr> {
        let count = self.header.e_phnum as usize;
        let base = self.header.e_phoff as usize;

        (0..count).map(move |i| {
            let offset = base + i * mem::size_of::<Elf64Phdr>();
            // SAFETY: We validated that the phdr table fits within data in parse().
            unsafe { &*(self.data.as_ptr().add(offset) as *const Elf64Phdr) }
        })
    }

    /// Find the PT_DYNAMIC program header, if present.
    fn dynamic_segment(&self) -> Option<&Elf64Phdr> {
        self.phdrs().find(|phdr| phdr.p_type == PT_DYNAMIC)
    }

    /// Parse the dynamic segment to find RELA relocation entries.
    ///
    /// Returns a slice of `Elf64Rela` entries from the ELF data, or `None`
    /// if there is no dynamic segment or no RELA entries.
    pub fn rela_entries(&self) -> Option<&[Elf64Rela]> {
        let dyn_phdr = self.dynamic_segment()?;
        let dyn_offset = dyn_phdr.p_offset as usize;
        let dyn_size = dyn_phdr.p_filesz as usize;

        if dyn_offset + dyn_size > self.data.len() {
            return None;
        }

        // Parse dynamic entries to find DT_RELA and DT_RELASZ.
        let mut rela_offset: Option<u64> = None;
        let mut rela_size: Option<u64> = None;

        let entry_size = mem::size_of::<Elf64Dyn>();
        let num_entries = dyn_size / entry_size;

        for i in 0..num_entries {
            let off = dyn_offset + i * entry_size;
            // SAFETY: We checked bounds above, Elf64Dyn is repr(C).
            let dyn_entry = unsafe { &*(self.data.as_ptr().add(off) as *const Elf64Dyn) };

            match dyn_entry.d_tag {
                DT_NULL => break,
                DT_RELA => rela_offset = Some(dyn_entry.d_val),
                DT_RELASZ => rela_size = Some(dyn_entry.d_val),
                _ => {}
            }
        }

        let rela_off = rela_offset? as usize;
        let rela_sz = rela_size? as usize;

        if rela_sz == 0 {
            return None;
        }

        // DT_RELA gives a virtual address; for PIE binaries linked at 0 this
        // equals the file offset. For ET_EXEC it may differ — find the
        // containing PT_LOAD segment and convert to a file offset.
        let file_off = self.vaddr_to_file_offset(rela_off as u64)? as usize;

        if file_off + rela_sz > self.data.len() {
            return None;
        }

        let count = rela_sz / mem::size_of::<Elf64Rela>();
        // SAFETY: We validated bounds, Elf64Rela is repr(C).
        let slice = unsafe {
            core::slice::from_raw_parts(self.data.as_ptr().add(file_off) as *const Elf64Rela, count)
        };

        Some(slice)
    }

    /// Convert a virtual address (as found in the ELF) to a file offset
    /// by searching PT_LOAD segments.
    fn vaddr_to_file_offset(&self, vaddr: u64) -> Option<u64> {
        for phdr in self.load_segments() {
            if vaddr >= phdr.p_vaddr && vaddr < phdr.p_vaddr + phdr.p_filesz {
                return Some(phdr.p_offset + (vaddr - phdr.p_vaddr));
            }
        }
        None
    }

    /// The raw ELF data.
    pub fn data(&self) -> &[u8] {
        self.data
    }
}
