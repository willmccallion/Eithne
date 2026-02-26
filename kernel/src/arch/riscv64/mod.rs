pub mod boot;
pub mod paging;
pub mod serial;
pub mod trap;

use super::traits::ArchPlatform;

pub struct Platform;

impl ArchPlatform for Platform {
    fn early_init() {
        // TODO: UART init, trap vector setup
    }

    fn init_interrupts() {
        // TODO: PLIC (Platform-Level Interrupt Controller) setup
    }

    fn halt() -> ! {
        loop {
            unsafe {
                core::arch::asm!("wfi");
            }
        }
    }

    fn enable_interrupts() {
        // TODO: set sstatus.SIE
    }

    fn disable_interrupts() {
        // TODO: clear sstatus.SIE
    }

    fn write_serial(s: &str) {
        serial::write_str(s);
    }

    fn arch_name() -> &'static str {
        "riscv64"
    }
}
