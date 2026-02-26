//! Device filesystem (stub).
//!
//! Provides /dev entries for character and block devices.
//! Each device node maps to a registered driver.

// TODO: Device registry: name → CharDevice | BlockDevice
// TODO: impl FileSystem for DevFs
// TODO: Auto-create entries when drivers register
