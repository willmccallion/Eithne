use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::println!("[oxeidia] KERNEL PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}
