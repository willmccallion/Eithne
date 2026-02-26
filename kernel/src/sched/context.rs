//! Context switching.
//!
//! Saves the current task's CPU state and restores the next task's state.
//! The actual register save/restore is arch-specific (inline asm).

// TODO: Define ArchContext per architecture:
//   x86_64: rsp, rbp, rbx, r12-r15, rflags, rip
//   aarch64: x19-x30, sp, elr_el1, spsr_el1
//   riscv64: s0-s11, sp, ra, sstatus, sepc
// TODO: switch_context(old: &mut ArchContext, new: &ArchContext)
