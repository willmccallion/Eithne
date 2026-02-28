extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::fs::traits::FileHandle;
use crate::syscall::traits::SyscallError;

const MAX_FDS: usize = 256;

#[derive(Clone)]
struct FdEntry {
    vfs_handle: FileHandle,
    cloexec: bool,
}

pub struct FdTable {
    entries: Vec<Option<FdEntry>>,
}

impl FdTable {
    /// Create an empty fd table.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Create an fd table with fd 0/1/2 reserved for stdio.
    ///
    /// These slots are marked as `None` — stdio fds are handled specially
    /// by the syscall layer and never go through the VFS.
    pub fn new_with_stdio() -> Self {
        Self {
            entries: vec![None, None, None],
        }
    }

    /// Returns true if `fd` is a stdio descriptor (0, 1, or 2).
    pub fn is_stdio(fd: usize) -> bool {
        fd <= 2
    }

    /// Allocate the first available fd slot and store `handle`.
    pub fn alloc_fd(&mut self, handle: FileHandle) -> Result<usize, SyscallError> {
        // Look for the first None slot at index >= 3 (skip stdio).
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if i < 3 {
                continue;
            }
            if slot.is_none() {
                *slot = Some(FdEntry {
                    vfs_handle: handle,
                    cloexec: false,
                });
                return Ok(i);
            }
        }

        // No free slot — extend if under the limit.
        let new_fd = self.entries.len();
        if new_fd >= MAX_FDS {
            return Err(SyscallError::NoMemory);
        }
        self.entries.push(Some(FdEntry {
            vfs_handle: handle,
            cloexec: false,
        }));
        Ok(new_fd)
    }

    /// Get the VFS handle for an open fd.
    pub fn get(&self, fd: usize) -> Result<FileHandle, SyscallError> {
        let entry = self
            .entries
            .get(fd)
            .and_then(|e| e.as_ref())
            .ok_or(SyscallError::InvalidArgument)?;
        Ok(entry.vfs_handle)
    }

    /// Close an fd, returning the VFS handle so the caller can close it in the VFS.
    pub fn close(&mut self, fd: usize) -> Result<FileHandle, SyscallError> {
        let slot = self
            .entries
            .get_mut(fd)
            .ok_or(SyscallError::InvalidArgument)?;
        let entry = slot.take().ok_or(SyscallError::InvalidArgument)?;
        Ok(entry.vfs_handle)
    }

    /// Clone the entire fd table (used by fork).
    pub fn clone_table(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }

    /// Close all FDs marked close-on-exec, returning their VFS handles.
    pub fn close_cloexec(&mut self) -> Vec<FileHandle> {
        let mut handles = Vec::new();
        for slot in self.entries.iter_mut() {
            if let Some(entry) = slot.as_ref() {
                if entry.cloexec {
                    handles.push(entry.vfs_handle);
                    *slot = None;
                }
            }
        }
        handles
    }

    /// Set the close-on-exec flag for an fd.
    pub fn set_cloexec(&mut self, fd: usize, val: bool) {
        if let Some(Some(entry)) = self.entries.get_mut(fd) {
            entry.cloexec = val;
        }
    }
}
