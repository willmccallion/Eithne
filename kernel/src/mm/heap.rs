//! Kernel heap allocator (stub).
//!
//! Provides `#[global_allocator]` so that `alloc` types (Box, Vec, etc.)
//! work in kernel space. Backed by a slab or buddy allocator operating
//! on pages obtained from the frame allocator.

// TODO:
// - Implement a simple bump allocator as a first pass
// TODO: Replace with slab allocator for small allocations
// TODO: Register as #[global_allocator]
// TODO: Wire up to frame allocator for growing the heap
