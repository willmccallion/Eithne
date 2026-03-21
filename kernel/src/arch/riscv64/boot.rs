/// RISC-V 64-bit UEFI boot handoff.
///
/// UEFI leaves us in S-mode with virtual memory enabled.

pub const fn post_uefi_init() {
    // TODO:
    // - Set up trap vector (stvec)
    // - Parse UEFI memory map
    // - Configure Sv48 page tables
}
