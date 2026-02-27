pub mod boot;
pub mod cpu;
pub mod gdt;
pub mod idt;
pub mod interrupts;
pub mod io;
pub mod paging;
pub mod serial;

use super::traits::ArchPlatform;

pub struct Platform;

impl ArchPlatform for Platform {
    fn early_init() {
        serial::init();
    }

    fn init_interrupts() {
        gdt::init();
        idt::init();
        interrupts::init_pic();
        crate::println!("[arch] enabling interrupts");
        unsafe { core::arch::asm!("sti", options(nomem, nostack)) };
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

    fn test_paging() {
        paging::test_paging();
    }

    fn halt_once() {
        cpu::halt_once();
    }
}
