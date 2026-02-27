/// Architecture abstraction trait.
///
/// Each supported architecture implements this trait to provide
/// platform-specific initialization and control primitives.
pub trait ArchPlatform {
    /// Perform early platform initialization (before memory management).
    fn early_init();

    /// Initialize interrupt/exception handling for this platform.
    fn init_interrupts();

    /// Halt the CPU permanently. Used on panic and clean shutdown.
    fn halt() -> !;

    /// Enable hardware interrupts.
    fn enable_interrupts();

    /// Disable hardware interrupts.
    fn disable_interrupts();

    /// Write a string to the platform's early serial/debug console.
    fn write_serial(s: &str);

    /// Return the architecture name (e.g. "x86_64").
    fn arch_name() -> &'static str;

    /// Run the architecture's page-table smoke test.
    fn test_paging();

    /// Set up the higher-half direct mapping of all physical memory.
    /// `max_phys` is the highest physical address to map (exclusive, rounded up).
    fn init_higher_half(max_phys: u64);

    /// Execute a single halt/wait-for-interrupt instruction and return after
    /// the next interrupt.  Used by the idle task.
    fn halt_once();
}
