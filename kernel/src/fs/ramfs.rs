//! In-memory filesystem.
//!
//! A simple tmpfs-like filesystem that stores all data in RAM.
//! Useful for early boot before disk drivers are ready.

extern crate alloc;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::sync::IrqSpinlock;

use super::traits::*;

struct Inode {
    file_type: FileType,
    /// File content (Regular files only).
    data: Vec<u8>,
    /// Directory children: name → child inode number (Directory only).
    children: BTreeMap<String, u64>,
}

struct OpenFile {
    ino: u64,
    cursor: usize,
    flags: OpenFlags,
}

struct RamFsInner {
    /// Inode table. Index = inode number. [0] = root directory.
    inodes: Vec<Inode>,
    /// Open file handles → open file state.
    open_files: BTreeMap<u64, OpenFile>,
    /// Next handle counter.
    next_handle: u64,
}

impl RamFsInner {
    /// Resolve an absolute path to an inode number.
    fn resolve_path(&self, path: &str) -> Result<u64, FsError> {
        if !path.starts_with('/') {
            return Err(FsError::InvalidPath);
        }
        let mut ino: u64 = 0; // root
        for component in path.split('/').filter(|c| !c.is_empty()) {
            let inode = &self.inodes[ino as usize];
            if inode.file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            ino = *inode.children.get(component).ok_or(FsError::NotFound)?;
        }
        Ok(ino)
    }

    /// Resolve a path to its parent directory inode and the final component name.
    fn resolve_parent<'a>(&self, path: &'a str) -> Result<(u64, &'a str), FsError> {
        if !path.starts_with('/') {
            return Err(FsError::InvalidPath);
        }
        let path = path.trim_end_matches('/');
        if path.is_empty() {
            return Err(FsError::InvalidPath); // can't get parent of root
        }
        match path.rfind('/') {
            Some(pos) => {
                let parent_path = if pos == 0 { "/" } else { &path[..pos] };
                let name = &path[pos + 1..];
                if name.is_empty() {
                    return Err(FsError::InvalidPath);
                }
                let parent_ino = self.resolve_path(parent_path)?;
                Ok((parent_ino, name))
            }
            None => Err(FsError::InvalidPath),
        }
    }

    /// Allocate a new inode and return its inode number.
    fn alloc_inode(&mut self, file_type: FileType) -> u64 {
        let ino = self.inodes.len() as u64;
        self.inodes.push(Inode {
            file_type,
            data: Vec::new(),
            children: BTreeMap::new(),
        });
        ino
    }
}

pub struct RamFs {
    inner: IrqSpinlock<RamFsInner>,
}

impl RamFs {
    pub fn new() -> Self {
        let root = Inode {
            file_type: FileType::Directory,
            data: Vec::new(),
            children: BTreeMap::new(),
        };
        RamFs {
            inner: IrqSpinlock::new(RamFsInner {
                inodes: vec![root],
                open_files: BTreeMap::new(),
                next_handle: 1,
            }),
        }
    }
}

impl FileSystem for RamFs {
    fn name(&self) -> &str {
        "ramfs"
    }

    fn open(&self, path: &str, flags: OpenFlags) -> Result<FileHandle, FsError> {
        let mut inner = self.inner.lock();

        let ino = match inner.resolve_path(path) {
            Ok(ino) => {
                if flags.truncate {
                    inner.inodes[ino as usize].data.clear();
                }
                ino
            }
            Err(FsError::NotFound) if flags.create => {
                let (parent_ino, name) = inner.resolve_parent(path)?;
                if inner.inodes[parent_ino as usize].file_type != FileType::Directory {
                    return Err(FsError::NotADirectory);
                }
                let new_ino = inner.alloc_inode(FileType::Regular);
                inner.inodes[parent_ino as usize]
                    .children
                    .insert(String::from(name), new_ino);
                new_ino
            }
            Err(e) => return Err(e),
        };

        let handle_id = inner.next_handle;
        inner.next_handle += 1;
        let cursor = if flags.append {
            inner.inodes[ino as usize].data.len()
        } else {
            0
        };
        inner
            .open_files
            .insert(handle_id, OpenFile { ino, cursor, flags });
        Ok(FileHandle(handle_id))
    }

    fn read(&self, handle: FileHandle, buf: &mut [u8]) -> Result<usize, FsError> {
        let mut inner = self.inner.lock();
        let file = inner.open_files.get(&handle.0).ok_or(FsError::NotFound)?;
        if !file.flags.read {
            return Err(FsError::PermissionDenied);
        }
        let ino = file.ino as usize;
        let cursor = file.cursor;

        let inode = &inner.inodes[ino];
        if inode.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        let available = inode.data.len().saturating_sub(cursor);
        let to_read = buf.len().min(available);
        buf[..to_read].copy_from_slice(&inode.data[cursor..cursor + to_read]);

        // Update cursor (immutable borrow of inodes is done).
        inner.open_files.get_mut(&handle.0).unwrap().cursor += to_read;
        Ok(to_read)
    }

    fn write(&self, handle: FileHandle, buf: &[u8]) -> Result<usize, FsError> {
        let mut inner = self.inner.lock();
        let file = inner.open_files.get(&handle.0).ok_or(FsError::NotFound)?;
        if !file.flags.write {
            return Err(FsError::PermissionDenied);
        }
        let ino = file.ino as usize;
        let cursor = file.cursor;

        let inode = &mut inner.inodes[ino];
        if inode.file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }
        let new_end = cursor + buf.len();
        if new_end > inode.data.len() {
            inode.data.resize(new_end, 0);
        }
        inode.data[cursor..new_end].copy_from_slice(buf);

        inner.open_files.get_mut(&handle.0).unwrap().cursor = new_end;
        Ok(buf.len())
    }

    fn close(&self, handle: FileHandle) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        inner
            .open_files
            .remove(&handle.0)
            .ok_or(FsError::NotFound)?;
        Ok(())
    }

    fn stat(&self, path: &str) -> Result<FileStat, FsError> {
        let inner = self.inner.lock();
        let ino = inner.resolve_path(path)?;
        let inode = &inner.inodes[ino as usize];
        Ok(FileStat {
            file_type: inode.file_type,
            size: inode.data.len() as u64,
            inode: ino,
        })
    }

    fn readdir(&self, path: &str) -> Result<Vec<DirEntry>, FsError> {
        let inner = self.inner.lock();
        let ino = inner.resolve_path(path)?;
        let inode = &inner.inodes[ino as usize];
        if inode.file_type != FileType::Directory {
            return Err(FsError::NotADirectory);
        }
        let mut entries = Vec::with_capacity(inode.children.len());
        for (name, &child_ino) in &inode.children {
            let child = &inner.inodes[child_ino as usize];
            let mut name_buf = [0u8; 256];
            let name_bytes = name.as_bytes();
            let copy_len = name_bytes.len().min(256);
            name_buf[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
            entries.push(DirEntry {
                name: name_buf,
                name_len: copy_len,
                file_type: child.file_type,
                inode: child_ino,
            });
        }
        Ok(entries)
    }

    fn mkdir(&self, path: &str) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        let (parent_ino, name) = inner.resolve_parent(path)?;

        {
            let parent = &inner.inodes[parent_ino as usize];
            if parent.file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            if parent.children.contains_key(name) {
                return Err(FsError::AlreadyExists);
            }
        }

        let new_ino = inner.alloc_inode(FileType::Directory);
        inner.inodes[parent_ino as usize]
            .children
            .insert(String::from(name), new_ino);
        Ok(())
    }

    fn unlink(&self, path: &str) -> Result<(), FsError> {
        let mut inner = self.inner.lock();
        let (parent_ino, name) = inner.resolve_parent(path)?;

        let child_ino = {
            let parent = &inner.inodes[parent_ino as usize];
            if parent.file_type != FileType::Directory {
                return Err(FsError::NotADirectory);
            }
            *parent.children.get(name).ok_or(FsError::NotFound)?
        };

        if inner.inodes[child_ino as usize].file_type == FileType::Directory {
            return Err(FsError::IsADirectory);
        }

        inner.inodes[parent_ino as usize].children.remove(name);
        Ok(())
    }
}
