/// Virtual memory page management (stub).
///
/// This module provides the arch-agnostic interface for page table operations.
/// Actual page table manipulation is delegated to arch-specific code.

// TODO:
// - Page fault handler (dispatched from arch interrupt/trap handler)
// - Demand paging support
// - Copy-on-write page tracking

/// Standard page size (4 KiB).
pub const PAGE_SIZE: usize = 4096;

/// Large page size (2 MiB on x86_64, varies on other archs).
pub const LARGE_PAGE_SIZE: usize = 2 * 1024 * 1024;
