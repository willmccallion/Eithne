//! AArch64 exception vector table.
//!
//! ARM requires a vector table aligned to 2048 bytes with entries for:
//! - Synchronous exceptions (syscalls, aborts)
//! - IRQ
//! - FIQ
//! - SError
//!
//! Each set of 4 entries is duplicated for:
//! - Current EL with SP0
//! - Current EL with SPx
//! - Lower EL using AArch64
//! - Lower EL using AArch32

// TODO: Define vector table
// TODO: Exception handlers
