//! RISC-V trap/interrupt handling.
//!
//! RISC-V uses the stvec CSR to point to the trap handler.
//! Traps include: exceptions (illegal instruction, page fault, ecall)
//! and interrupts (timer, software, external via PLIC).

// TODO: Trap vector setup (stvec)
// TODO: Trap handler dispatch
// TODO: Context save/restore
