//! In-memory filesystem (stub).
//!
//! A simple tmpfs-like filesystem that stores all data in RAM.
//! This is the first filesystem to implement — useful for early boot
//! before disk drivers are ready.

// TODO: Inode table (in-memory)
// TODO: Directory tree (inode → children mapping)
// TODO: File data storage (Vec<u8> per file, or page-backed)
// TODO: impl FileSystem for RamFs
