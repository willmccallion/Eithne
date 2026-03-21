/// AArch64 UEFI boot handoff.
///
/// UEFI leaves us in EL1 (or EL2) with MMU enabled and identity-mapped memory.

pub const fn post_uefi_init() {
    // TODO:
    // - Determine current exception level
    // - Set up exception vectors
    // - Parse UEFI memory map
    // - Configure MMU with our own page tables
}
