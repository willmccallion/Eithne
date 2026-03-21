//! Virtual Filesystem Switch.
//!
//! Provides a unified interface over multiple mounted filesystems.
//! Handles path resolution, mount point lookup, and dispatches operations
//! to the correct underlying filesystem.
//!
//! Lock ordering: VFS lock → filesystem lock (e.g. RamFs lock).

extern crate alloc;
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::sync::{IrqSpinlock, Once};

use super::ramfs::RamFs;
use super::traits::{FileSystem, FileHandle, FsError, OpenFlags, File, FileStat, DirEntry};

struct MountPoint {
    path: String,
    fs: Box<dyn FileSystem + Send + Sync>,
}

struct VfsOpenFile {
    mount_idx: usize,
    fs_handle: FileHandle,
}

struct VfsInner {
    mounts: Vec<MountPoint>,
    open_files: BTreeMap<u64, VfsOpenFile>,
    next_handle: u64,
}

impl VfsInner {
    /// Find the filesystem that handles a given path.
    ///
    /// Returns `(mount_index, &dyn FileSystem, fs_relative_path)`.
    /// Uses longest-prefix matching so `/dev` beats `/` for paths under `/dev/`.
    fn resolve_mount<'a>(
        &'a self,
        path: &'a str,
    ) -> Result<(usize, &'a (dyn FileSystem + Send + Sync), &'a str), FsError> {
        if !path.starts_with('/') {
            return Err(FsError::InvalidPath);
        }
        let mut best: Option<(usize, &MountPoint)> = None;
        for (idx, mount) in self.mounts.iter().enumerate() {
            let matches = if mount.path == "/" {
                true
            } else {
                path.starts_with(mount.path.as_str())
                    && (path.len() == mount.path.len()
                        || path.as_bytes().get(mount.path.len()) == Some(&b'/'))
            };
            if matches {
                match best {
                    None => best = Some((idx, mount)),
                    Some((_, prev)) if mount.path.len() > prev.path.len() => {
                        best = Some((idx, mount));
                    }
                    _ => {}
                }
            }
        }
        let Some((idx, mount)) = best else {
            return Err(FsError::NotFound);
        };
        let relative = if mount.path == "/" {
            path
        } else {
            let rest = &path[mount.path.len()..];
            if rest.is_empty() { "/" } else { rest }
        };
        Ok((idx, &*mount.fs, relative))
    }
}

struct Vfs {
    inner: IrqSpinlock<VfsInner>,
}

impl Vfs {
    const fn new() -> Self {
        Self {
            inner: IrqSpinlock::new(VfsInner {
                mounts: Vec::new(),
                open_files: BTreeMap::new(),
                next_handle: 1,
            }),
        }
    }

    fn mount(&self, path: &str, fs: Box<dyn FileSystem + Send + Sync>) {
        let mut inner = self.inner.lock();
        crate::println!("[vfs] mounting '{}' at {}", fs.name(), path);
        inner.mounts.push(MountPoint {
            path: String::from(path),
            fs,
        });
    }

    fn open(&self, path: &str, flags: OpenFlags) -> Result<FileHandle, FsError> {
        let mut inner = self.inner.lock();
        let (mount_idx, fs, rel_path) = inner.resolve_mount(path)?;
        let fs_handle = fs.open(rel_path, flags)?;

        let vfs_handle = inner.next_handle;
        inner.next_handle += 1;
        let _ = inner.open_files.insert(
            vfs_handle,
            VfsOpenFile {
                mount_idx,
                fs_handle,
            },
        );
        Ok(FileHandle(vfs_handle))
    }

    fn read(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let entry = inner.open_files.get(&handle.0).ok_or(FsError::NotFound)?;
        inner.mounts[entry.mount_idx].fs.read(entry.fs_handle, buf)
    }

    fn write(&self, handle: FileHandle, buf: &[u8]) -> Result<usize, FsError> {
        let inner = self.inner.lock();
        let entry = inner.open_files.get(&handle.0).ok_or(FsError::NotFound)?;
        inner.mounts[entry.mount_idx].fs.write(entry.fs_handle, buf)
    }

    fn close(&self, handle: FileHandle) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        let entry = inner
            .open_files
            .remove(&handle.0)
            .ok_or(FsError::NotFound)?;
        inner.mounts[entry.mount_idx].fs.close(entry.fs_handle)
    }

    fn dup(&self, handle: FileHandle) -> Result<FileHandle, FsError> {
        let mut inner = self.inner.lock();
        let entry = inner.open_files.get(&handle.0).ok_or(FsError::NotFound)?;
        let mount_idx = entry.mount_idx;
        let new_fs_handle = inner.mounts[mount_idx].fs.dup(entry.fs_handle)?;

        let new_vfs_handle = inner.next_handle;
        inner.next_handle += 1;
        let _ = inner.open_files.insert(
            new_vfs_handle,
            VfsOpenFile {
                mount_idx,
                fs_handle: new_fs_handle,
            },
        );
        Ok(FileHandle(new_vfs_handle))
    }

    fn stat(&self, path: &str) -> Result<FileStat, FsError> {
        let inner = self.inner.lock();
        let (_idx, fs, rel_path) = inner.resolve_mount(path)?;
        fs.stat(rel_path)
    }

    fn readdir(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let inner = self.inner.lock();
        let (_idx, fs, rel_path) = inner.resolve_mount(path)?;
        fs.readdir(rel_path)
    }

    fn mkdir(&self, path: &str) -> Result<(), FsError> {
        let inner = self.inner.lock();
        let (_idx, fs, rel_path) = inner.resolve_mount(path)?;
        fs.mkdir(rel_path)
    }

    fn unlink(&self, path: &str) -> Result<(), FsError> {
        let inner = self.inner.lock();
        let (_idx, fs, rel_path) = inner.resolve_mount(path)?;
        fs.unlink(rel_path)
    }
}

// ---------------------------------------------------------------------------
// Global singleton + public API
// ---------------------------------------------------------------------------

static VFS: Once<Vfs> = Once::new();

/// Initialize the VFS and mount the root ramfs.
pub fn init() {
    let _ = VFS.call_once(|| {
        let vfs = Vfs::new();
        vfs.mount("/", Box::new(RamFs::new()));
        vfs
    });
}

pub fn open(path: &str, flags: OpenFlags) -> Result<FileHandle, FsError> {
    VFS.get().ok_or(FsError::IoError)?.open(path, flags)
}

pub fn read(handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
    VFS.get().ok_or(FsError::IoError)?.read(handle, buf)
}

pub fn write(handle: FileHandle, buf: &[u8]) -> Result<usize, FsError> {
    VFS.get().ok_or(FsError::IoError)?.write(handle, buf)
}

pub fn close(handle: FileHandle) -> Result<(), FsError> {
    VFS.get().ok_or(FsError::IoError)?.close(handle)
}

pub fn dup(handle: FileHandle) -> Result<FileHandle, FsError> {
    VFS.get().ok_or(FsError::IoError)?.dup(handle)
}

pub fn stat(path: &str) -> Result<FileStat, FsError> {
    VFS.get().ok_or(FsError::IoError)?.stat(path)
}

pub fn readdir(path: &str) -> Result<Vec<DirEntry>, FsError> {
    VFS.get().ok_or(FsError::IoError)?.readdir(path)
}

pub fn mkdir(path: &str) -> Result<(), FsError> {
    VFS.get().ok_or(FsError::IoError)?.mkdir(path)
}

pub fn unlink(path: &str) -> Result<(), FsError> {
    VFS.get().ok_or(FsError::IoError)?.unlink(path)
}
