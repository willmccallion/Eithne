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

    fn test_paging() {
        // TODO: RISC-V Sv48 page table smoke test
    }

    fn halt_once() {
        unsafe { core::arch::asm!("wfi") }
    }

    fn init_higher_half(_max_phys: u64) {
        // TODO: RISC-V Sv48 direct-map setup
    }

    fn init_syscalls() {
        // TODO: RISC-V ecall handler setup
    }

    fn set_syscall_stack(_rsp_top: u64) {
        // TODO: per-CPU kernel stack for trap entry
    }

    fn enable_serial_rx() {
        // TODO: UART RX interrupt enable
    }
}
