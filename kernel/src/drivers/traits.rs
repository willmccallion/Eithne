/// Driver error type.
#[derive(Debug)]
pub enum IoError {
    Timeout,
    DeviceNotReady,
    InvalidOperation,
    BufferTooSmall,
    HardwareError,
}

/// Base trait for all device drivers.
pub trait Driver {
    /// Human-readable driver name.
    fn name(&self) -> &'static str;

    /// Initialize the device. Called once during boot.
    fn init(&mut self) -> Result<(), IoError>;

    /// Reset the device to a known state.
    fn reset(&mut self) -> Result<(), IoError>;
}

/// Block device (disk-like) interface.
///
/// Block devices transfer data in fixed-size blocks (typically 512 bytes or 4 KiB).
pub trait BlockDevice: Driver {
    /// Read a single block at the given LBA (logical block address).
    fn read_block(&self, lba: u64, buf: &mut [u8]) -> Result<(), IoError>;

    /// Write a single block at the given LBA.
    fn write_block(&self, lba: u64, buf: &[u8]) -> Result<(), IoError>;

    /// Return the block size in bytes.
    fn block_size(&self) -> usize;

    /// Return the total number of blocks.
    fn block_count(&self) -> u64;
}

/// Character device (stream-like) interface.
///
/// Character devices transfer data byte-by-byte (serial ports, terminals, etc.).
pub trait CharDevice: Driver {
    /// Read available bytes into the buffer. Returns number of bytes read.
    fn read(&self, buf: &mut [u8]) -> Result<usize, IoError>;

    /// Write bytes from the buffer. Returns number of bytes written.
    fn write(&self, buf: &[u8]) -> Result<usize, IoError>;
}

/// Network device interface.
///
/// Network devices send and receive packets (Ethernet frames).
pub trait NetworkDevice: Driver {
    /// Send a packet.
    fn send(&self, packet: &[u8]) -> Result<(), IoError>;

    /// Receive a packet into the buffer. Returns the packet length.
    fn recv(&self, buf: &mut [u8]) -> Result<usize, IoError>;

    /// Get the device's MAC address.
    fn mac_address(&self) -> [u8; 6];
}
