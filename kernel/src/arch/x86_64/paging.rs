//! x86_64 4-level page table manipulation.
//!
//! Implements the arch-specific side of virtual memory management.
//! The generic mm::PageMapper trait delegates to this for x86_64.

// TODO: Page table entry structure (PML4, PDPT, PD, PT)
// TODO: Map / unmap pages
// TODO: Translate virtual → physical
// TODO: TLB invalidation (invlpg)
