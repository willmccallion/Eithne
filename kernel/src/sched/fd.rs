extern crate alloc;
use alloc::vec;
use alloc::vec::Vec;

use crate::fs::traits::FileHandle;
use crate::ipc::pipe::{PipeEnd, PipeHandle};
use crate::syscall::traits::SyscallError;

const MAX_FDS: usize = 256;

/// The kind of resource backing a file descriptor.
#[derive(Clone)]
pub enum FdKind {
    Vfs(FileHandle),
    Pipe(PipeHandle),
}

#[derive(Clone)]
struct FdEntry {
    kind: FdKind,
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

    /// Allocate the first available fd slot and store a VFS handle.
    pub fn alloc_fd(&mut self, handle: FileHandle) -> Result<usize, SyscallError> {
        self.alloc_fd_kind(FdKind::Vfs(handle))
    }

    /// Allocate the first available fd slot and store a pipe handle.
    pub fn alloc_fd_pipe(&mut self, handle: PipeHandle) -> Result<usize, SyscallError> {
        self.alloc_fd_kind(FdKind::Pipe(handle))
    }

    /// Internal: allocate first free fd >= 3.
    fn alloc_fd_kind(&mut self, kind: FdKind) -> Result<usize, SyscallError> {
        // Look for the first None slot at index >= 3 (skip stdio).
        for (i, slot) in self.entries.iter_mut().enumerate() {
            if i < 3 {
                continue;
            }
            if slot.is_none() {
                *slot = Some(FdEntry {
                    kind,
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
            kind,
            cloexec: false,
        }));
        Ok(new_fd)
    }

    /// Get the VFS handle for an open fd (only for VFS fds).
    pub fn get(&self, fd: usize) -> Result<FileHandle, SyscallError> {
        let entry = self
            .entries
            .get(fd)
            .and_then(|e| e.as_ref())
            .ok_or(SyscallError::InvalidArgument)?;
        match &entry.kind {
            FdKind::Vfs(h) => Ok(*h),
            FdKind::Pipe(_) => Err(SyscallError::InvalidArgument),
        }
    }

    /// Get the kind of resource at `fd`.
    ///
    /// Returns:
    /// - `Ok(Some(kind))` if the slot is occupied
    /// - `Ok(None)` if the slot is empty and fd <= 2 (default stdio)
    /// - `Err(InvalidArgument)` if the slot is empty and fd > 2 (not open)
    pub fn get_kind(&self, fd: usize) -> Result<Option<FdKind>, SyscallError> {
        if fd >= self.entries.len() {
            if fd <= 2 {
                return Ok(None);
            }
            return Err(SyscallError::InvalidArgument);
        }
        match &self.entries[fd] {
            Some(entry) => Ok(Some(entry.kind.clone())),
            None => {
                if fd <= 2 {
                    Ok(None) // stdio default
                } else {
                    Err(SyscallError::InvalidArgument) // closed fd
                }
            }
        }
    }

    /// Close an fd, returning its kind so the caller can close the
    /// underlying resource (VFS handle or pipe) outside the scheduler lock.
    pub fn close(&mut self, fd: usize) -> Result<FdKind, SyscallError> {
        let slot = self
            .entries
            .get_mut(fd)
            .ok_or(SyscallError::InvalidArgument)?;
        let entry = slot.take().ok_or(SyscallError::InvalidArgument)?;
        Ok(entry.kind)
    }

    /// Clone the entire fd table (used by fork).
    ///
    /// For pipe entries, increments the refcount in the global pipe table.
    /// For VFS entries, creates independent file handles via `vfs::dup`.
    /// This acquires the PIPES / VFS locks but NOT the scheduler lock, so
    /// it is safe to call while the scheduler lock is already held.
    pub fn clone_table(&self) -> Self {
        let mut cloned = Vec::with_capacity(self.entries.len());
        for slot in self.entries.iter() {
            cloned.push(match slot {
                Some(entry) => match &entry.kind {
                    FdKind::Pipe(handle) => {
                        crate::ipc::pipe::pipe_clone(handle);
                        Some(FdEntry {
                            kind: FdKind::Pipe(handle.clone()),
                            cloexec: entry.cloexec,
                        })
                    }
                    FdKind::Vfs(handle) => {
                        match crate::fs::vfs::dup(*handle) {
                            Ok(new_handle) => Some(FdEntry {
                                kind: FdKind::Vfs(new_handle),
                                cloexec: entry.cloexec,
                            }),
                            Err(_) => {
                                // Best effort: if dup fails, skip this fd.
                                None
                            }
                        }
                    }
                },
                None => None,
            });
        }
        Self { entries: cloned }
    }

    /// Close all FDs marked close-on-exec, returning their kinds.
    pub fn close_cloexec(&mut self) -> Vec<FdKind> {
        let mut kinds = Vec::new();
        for slot in self.entries.iter_mut() {
            if let Some(entry) = slot.as_ref() {
                if entry.cloexec {
                    kinds.push(entry.kind.clone());
                    *slot = None;
                }
            }
        }
        kinds
    }

    /// Set the close-on-exec flag for an fd.
    pub fn set_cloexec(&mut self, fd: usize, val: bool) {
        if let Some(Some(entry)) = self.entries.get_mut(fd) {
            entry.cloexec = val;
        }
    }

    /// Place an entry at a specific fd slot (used by dup2).
    ///
    /// Extends the entries vector if needed. Returns the evicted entry's
    /// kind (if any) so the caller can close it outside the scheduler lock.
    pub fn set_fd(&mut self, fd: usize, kind: FdKind) -> Option<FdKind> {
        // Extend if necessary.
        while self.entries.len() <= fd {
            self.entries.push(None);
        }
        let old = self.entries[fd].take();
        self.entries[fd] = Some(FdEntry {
            kind,
            cloexec: false,
        });
        old.map(|e| e.kind)
    }

    /// Drain all open fds, returning their kinds.
    ///
    /// Used during process exit to extract all resources for cleanup
    /// outside the scheduler lock.
    pub fn drain_all(&mut self) -> Vec<FdKind> {
        let mut kinds = Vec::new();
        for slot in self.entries.iter_mut() {
            if let Some(entry) = slot.take() {
                kinds.push(entry.kind);
            }
        }
        kinds
    }
}
