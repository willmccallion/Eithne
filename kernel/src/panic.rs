use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    crate::println!("[eithne] KERNEL PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
