pub mod traits;

#[cfg(target_arch = "x86_64")]
mod x86_64;
#[cfg(target_arch = "x86_64")]
use x86_64 as platform;

#[cfg(target_arch = "aarch64")]
mod aarch64;
#[cfg(target_arch = "aarch64")]
use aarch64 as platform;

#[cfg(target_arch = "riscv64")]
mod riscv64;
#[cfg(target_arch = "riscv64")]
use riscv64 as platform;

use traits::ArchPlatform;

#[cfg(target_arch = "x86_64")]
pub const ARCH_NAME: &str = "x86_64";
#[cfg(target_arch = "aarch64")]
pub const ARCH_NAME: &str = "aarch64";
#[cfg(target_arch = "riscv64")]
pub const ARCH_NAME: &str = "riscv64";

pub fn early_init() {
    platform::Platform::early_init();
}

pub fn halt() -> ! {
    platform::Platform::halt()
}

pub fn write_serial(s: &str) {
    platform::Platform::write_serial(s);
}

pub fn enable_interrupts() {
    platform::Platform::enable_interrupts();
}

pub fn disable_interrupts() {
    platform::Platform::disable_interrupts();
}
