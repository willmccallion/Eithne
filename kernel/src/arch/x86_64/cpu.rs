//! x86_64 CPU feature detection and control.
//!
//! CPUID queries, MSR access, and CPU feature flags.

// TODO: CPUID wrapper
// TODO: MSR read/write
// TODO: Feature detection (SSE, AVX, XSAVE, etc.)
// TODO: CR register access

/// Execute a single `hlt` instruction.
///
/// Suspends the CPU until the next interrupt arrives.  Unlike `halt()` in the
/// arch facade, this returns after the interrupt is handled, allowing a loop
/// body to yield back to the scheduler.
#[inline]
pub fn halt_once() {
    unsafe {
        core::arch::asm!("hlt", options(nomem, nostack, preserves_flags));
    }
}
