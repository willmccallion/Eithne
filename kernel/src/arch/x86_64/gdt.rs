//! Global Descriptor Table for x86_64.
//!
//! In long mode, the GDT is mostly vestigial — segment bases/limits are
//! ignored. We still need valid code/data segment descriptors and a TSS
//! for interrupt stack switching.

// TODO: Define GDT entries (kernel code, kernel data, user code, user data, TSS)
// TODO: Load GDT with lgdt instruction
// TODO: Reload segment registers
