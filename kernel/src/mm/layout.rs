//! Kernel virtual memory layout constants.
//!
//! Defines the virtual address space partitioning between kernel and user space.

/// Start of the kernel's virtual address space (the x86_64 upper half).
pub const KERNEL_VIRT_BASE: u64 = 0xFFFF_8000_0000_0000;

/// Size of the kernel heap (initial).
pub const KERNEL_HEAP_SIZE: usize = 1024 * 1024; // 1 MiB

/// Size of each kernel stack.
pub const KERNEL_STACK_SIZE: usize = 64 * 1024; // 64 KiB
