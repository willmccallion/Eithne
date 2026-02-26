//! AArch64 MMU (Memory Management Unit) support.
//!
//! ARM uses a multi-level page table (up to 4 levels for 48-bit VA).
//! Translation tables are configured via TTBR0_EL1 (user) and TTBR1_EL1 (kernel).

// TODO: Page table entry structure
// TODO: Translation table setup
// TODO: Map / unmap pages
// TODO: TLB invalidation
