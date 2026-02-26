//! Kernel pipe (stub).
//!
//! A unidirectional byte stream between two processes/threads.
//! One end writes, the other reads. Blocking when the buffer is full/empty.

// TODO: Ring buffer for pipe data
// TODO: Read end / write end handles
// TODO: Blocking with scheduler integration
// TODO: impl IpcChannel for PipeEnd
