//! Virtual Filesystem Switch (stub).
//!
//! The VFS provides a unified interface over multiple mounted filesystems.
//! It handles path resolution, mount point lookup, and dispatches operations
//! to the correct underlying filesystem.

// TODO: Mount table: Vec<MountPoint> where MountPoint = { path, fs: Box<dyn FileSystem> }
// TODO: Path resolution: split path, walk mount table, delegate to correct fs
// TODO: File descriptor table (per-process)
// TODO: Global open file table
