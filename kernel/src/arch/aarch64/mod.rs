pub mod boot;
pub mod exceptions;
pub mod mmu;
pub mod serial;

use super::traits::ArchPlatform;

pub struct Platform;

impl ArchPlatform for Platform {
    fn early_init() {
        // TODO: PL011 UART init, exception vector setup
    }

    fn init_interrupts() {
        // TODO: GIC (Generic Interrupt Controller) setup
    }

    fn halt() -> ! {
        loop {
            unsafe {
                core::arch::asm!("wfe");
            }
        }
    }

    fn enable_interrupts() {
        // TODO: clear DAIF interrupt mask bits
    }

    fn disable_interrupts() {
        // TODO: set DAIF interrupt mask bits
    }

    fn write_serial(s: &str) {
        serial::write_str(s);
    }

    fn arch_name() -> &'static str {
        "aarch64"
    }

    fn test_paging() {
        // TODO: AArch64 TTBR0/TTBR1 page table smoke test
    }

    fn halt_once() {
        unsafe { core::arch::asm!("wfe") }
    }
}
