extern crate alloc;
use alloc::vec::Vec;

/// File open mode flags.
#[derive(Debug, Clone, Copy)]
pub struct OpenFlags {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub truncate: bool,
    pub append: bool,
}

impl OpenFlags {
    pub const READ: Self = Self {
        read: true,
        write: false,
        create: false,
        truncate: false,
        append: false,
    };

    pub const WRITE: Self = Self {
        read: false,
        write: true,
        create: true,
        truncate: true,
        append: false,
    };

    pub const READ_WRITE: Self = Self {
        read: true,
        write: true,
        create: false,
        truncate: false,
        append: false,
    };
}

/// File type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    CharDevice,
    BlockDevice,
    Pipe,
    Socket,
}

/// File metadata.
#[derive(Debug, Clone)]
pub struct FileStat {
    pub file_type: FileType,
    pub size: u64,
    pub inode: u64,
}

/// Opaque handle to an open file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileHandle(pub u64);

/// Filesystem error.
#[derive(Debug)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    IoError,
    NoSpace,
    InvalidPath,
}

/// A mounted filesystem.
///
/// Each filesystem implementation (ramfs, ext4, fat32, etc.) implements this trait.
pub trait FileSystem {
    /// Return the filesystem type name.
    fn name(&self) -> &str;

    /// Open a file by path within this filesystem.
    fn open(&self, path: &str, flags: OpenFlags) -> Result<FileHandle, FsError>;

    /// Read from an open file.
    fn read(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError>;

    /// Write to an open file.
    fn write(&self, handle: FileHandle, buf: &[u8]) -> Result<usize, FsError>;

    /// Close an open file handle.
    fn close(&self, handle: FileHandle) -> Result<(), FsError>;

    /// Get file metadata.
    fn stat(&self, path: &str) -> Result<FileStat, FsError>;

    /// List directory entries.
    fn readdir(&self, path: &str) -> Result<Vec<DirEntry>, FsError>;

    /// Create a directory.
    fn mkdir(&self, path: &str) -> Result<(), FsError>;

    /// Remove a file.
    fn unlink(&self, path: &str) -> Result<(), FsError>;
}

/// An individual file within a filesystem.
pub trait File {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, FsError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, FsError>;
    fn seek(&mut self, offset: i64, whence: SeekWhence) -> Result<u64, FsError>;
}

#[derive(Debug, Clone, Copy)]
pub enum SeekWhence {
    Start,
    Current,
    End,
}

/// A directory entry.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: [u8; 256],
    pub name_len: usize,
    pub file_type: FileType,
    pub inode: u64,
}
