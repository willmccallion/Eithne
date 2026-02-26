/// IPC channel error.
#[derive(Debug)]
pub enum IpcError {
    ChannelClosed,
    BufferFull,
    WouldBlock,
    InvalidMessage,
}

/// Inter-process communication channel.
///
/// Generic trait for different IPC mechanisms (pipes, message queues,
/// shared memory, etc.).
pub trait IpcChannel {
    /// Send data through the channel.
    fn send(&self, data: &[u8]) -> Result<usize, IpcError>;

    /// Receive data from the channel.
    fn recv(&self, buf: &mut [u8]) -> Result<usize, IpcError>;

    /// Close this end of the channel.
    fn close(&self) -> Result<(), IpcError>;
}
