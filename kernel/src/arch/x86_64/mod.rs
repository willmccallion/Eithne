pub mod boot;
pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod paging;
pub mod serial;

use super::traits::ArchPlatform;

pub struct Platform;

impl ArchPlatform for Platform {
    fn early_init() {
        serial::init();
    }

    fn init_interrupts() {
        // TODO: set up IDT, PIC/APIC
    }

    fn halt() -> ! {
        loop {
            unsafe {
                core::arch::asm!("hlt");
            }
        }
    }

    fn enable_interrupts() {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
    }

    fn disable_interrupts() {
        unsafe {
            core::arch::asm!("cli", options(nomem, nostack));
        }
    }

    fn write_serial(s: &str) {
        serial::write_str(s);
    }

    fn arch_name() -> &'static str {
        "x86_64"
    }
}
