//! Kernel pipe — a unidirectional in-memory byte stream.
//!
//! One end writes, the other reads. Blocking when the buffer is empty (read)
//! or full (write). Reference-counted so that fork gives both parent and child
//! independent handles that share the underlying buffer.

extern crate alloc;

use alloc::collections::BTreeMap;

use crate::sched;
use crate::sync::{IrqSpinlock, Once};
use crate::syscall::traits::SyscallError;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Unique identifier for a pipe instance in the global table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PipeId(pub u64);

/// Which end of the pipe a handle refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipeEnd {
    Read,
    Write,
}

/// A handle to one end of a pipe, stored in per-task fd tables.
#[derive(Debug, Clone, Copy)]
pub struct PipeHandle {
    pub pipe_id: PipeId,
    pub end: PipeEnd,
}

// ---------------------------------------------------------------------------
// Internal ring buffer
// ---------------------------------------------------------------------------

const PIPE_BUF_SIZE: usize = 4096;

struct PipeInner {
    buf: [u8; PIPE_BUF_SIZE],
    /// Next position to read from.
    read_pos: usize,
    /// Next position to write to.
    write_pos: usize,
    /// Number of bytes currently in the buffer.
    count: usize,
    /// Number of open read-end handles.
    readers: usize,
    /// Number of open write-end handles.
    writers: usize,
    /// Task blocked waiting for data (pipe_read found buffer empty).
    blocked_reader: Option<u64>,
    /// Task blocked waiting for space (pipe_write found buffer full).
    blocked_writer: Option<u64>,
}

impl PipeInner {
    fn new() -> Self {
        Self {
            buf: [0u8; PIPE_BUF_SIZE],
            read_pos: 0,
            write_pos: 0,
            count: 0,
            readers: 1,
            writers: 1,
            blocked_reader: None,
            blocked_writer: None,
        }
    }

    /// Copy up to `dst.len()` bytes out of the ring buffer.
    fn read_into(&mut self, dst: &mut [u8]) -> usize {
        let n = dst.len().min(self.count);
        for i in 0..n {
            dst[i] = self.buf[self.read_pos];
            self.read_pos = (self.read_pos + 1) % PIPE_BUF_SIZE;
        }
        self.count -= n;
        n
    }

    /// Copy up to `src.len()` bytes into the ring buffer.
    fn write_from(&mut self, src: &[u8]) -> usize {
        let space = PIPE_BUF_SIZE - self.count;
        let n = src.len().min(space);
        for i in 0..n {
            self.buf[self.write_pos] = src[i];
            self.write_pos = (self.write_pos + 1) % PIPE_BUF_SIZE;
        }
        self.count += n;
        n
    }
}

// ---------------------------------------------------------------------------
// Global pipe table
// ---------------------------------------------------------------------------

struct PipeTable {
    pipes: BTreeMap<u64, PipeInner>,
    next_id: u64,
}

static PIPES: Once<IrqSpinlock<PipeTable>> = Once::new();

pub fn init() {
    PIPES.call_once(|| {
        IrqSpinlock::new(PipeTable {
            pipes: BTreeMap::new(),
            next_id: 1,
        })
    });
    crate::println!("[pipe] pipe subsystem initialized");
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a new pipe, returning (read_handle, write_handle).
pub fn pipe_create() -> Option<(PipeHandle, PipeHandle)> {
    let table = PIPES.get()?;
    let mut guard = table.lock();
    let id = guard.next_id;
    guard.next_id += 1;
    guard.pipes.insert(id, PipeInner::new());
    Some((
        PipeHandle {
            pipe_id: PipeId(id),
            end: PipeEnd::Read,
        },
        PipeHandle {
            pipe_id: PipeId(id),
            end: PipeEnd::Write,
        },
    ))
}

/// Read from a pipe. Blocks if the buffer is empty and writers still exist.
/// Returns 0 (EOF) when all writers have closed.
pub fn pipe_read(id: PipeId, buf: &mut [u8]) -> Result<usize, SyscallError> {
    loop {
        // Phase 1: Try to read under the PIPES lock.
        let wake_tid: Option<u64>;
        {
            let table = PIPES.get().ok_or(SyscallError::IoError)?;
            let mut guard = table.lock();
            let pipe = guard
                .pipes
                .get_mut(&id.0)
                .ok_or(SyscallError::InvalidArgument)?;

            if pipe.count > 0 {
                let n = pipe.read_into(buf);
                // If a writer was blocked waiting for space, extract its TID.
                wake_tid = pipe.blocked_writer.take();
                drop(guard);
                // Wake the writer outside the PIPES lock.
                if let Some(tid) = wake_tid {
                    sched::wake_pipe_task(tid);
                }
                return Ok(n);
            }

            // Buffer empty — check if any writers remain.
            if pipe.writers == 0 {
                return Ok(0); // EOF
            }

            // Must block. Record our TID so the writer can wake us.
            let my_tid = sched::current_task_id().ok_or(SyscallError::IoError)?;
            pipe.blocked_reader = Some(my_tid.0);
        }
        // PIPES lock is dropped.

        // Phase 2: Block, re-checking under scheduler lock → PIPES lock.
        // The condition closure runs with the scheduler's IrqSpinlock held
        // (interrupts disabled). Acquiring the PIPES lock inside is safe
        // because the lock ordering is scheduler → PIPES.
        sched::block_current_if(|| {
            let table = match PIPES.get() {
                Some(t) => t,
                None => return false,
            };
            let guard = table.lock();
            match guard.pipes.get(&id.0) {
                Some(p) => p.count == 0 && p.writers > 0,
                None => false, // pipe gone, don't block
            }
        });
        // Woken — loop back and re-try the read.
    }
}

/// Write to a pipe. Blocks if the buffer is full and readers still exist.
/// Returns `BrokenPipe` when all readers have closed.
pub fn pipe_write(id: PipeId, buf: &[u8]) -> Result<usize, SyscallError> {
    let mut written = 0;

    while written < buf.len() {
        // Phase 1: Try to write under the PIPES lock.
        let wake_tid: Option<u64>;
        {
            let table = PIPES.get().ok_or(SyscallError::IoError)?;
            let mut guard = table.lock();
            let pipe = guard
                .pipes
                .get_mut(&id.0)
                .ok_or(SyscallError::InvalidArgument)?;

            // No readers — broken pipe.
            if pipe.readers == 0 {
                return Err(SyscallError::BrokenPipe);
            }

            if pipe.count < PIPE_BUF_SIZE {
                let n = pipe.write_from(&buf[written..]);
                written += n;
                // Wake blocked reader.
                wake_tid = pipe.blocked_reader.take();
                drop(guard);
                if let Some(tid) = wake_tid {
                    sched::wake_pipe_task(tid);
                }
                continue;
            }

            // Buffer full — must block.
            let my_tid = sched::current_task_id().ok_or(SyscallError::IoError)?;
            pipe.blocked_writer = Some(my_tid.0);
        }
        // PIPES lock dropped.

        // Phase 2: Block until space is available.
        sched::block_current_if(|| {
            let table = match PIPES.get() {
                Some(t) => t,
                None => return false,
            };
            let guard = table.lock();
            match guard.pipes.get(&id.0) {
                Some(p) => p.count >= PIPE_BUF_SIZE && p.readers > 0,
                None => false,
            }
        });
    }

    Ok(written)
}

/// Close one end of a pipe. Decrements the refcount for the appropriate end.
/// When all writers close, blocked readers are woken (they see EOF).
/// When all readers close, blocked writers are woken (they see BrokenPipe).
/// When both refcounts reach zero, the pipe is removed from the table.
pub fn pipe_close(handle: PipeHandle) {
    let table = match PIPES.get() {
        Some(t) => t,
        None => return,
    };

    let wake_tid: Option<u64>;
    let should_remove: bool;
    {
        let mut guard = table.lock();
        let pipe = match guard.pipes.get_mut(&handle.pipe_id.0) {
            Some(p) => p,
            None => return,
        };

        match handle.end {
            PipeEnd::Read => {
                pipe.readers = pipe.readers.saturating_sub(1);
                // If last reader closed, wake the blocked writer.
                wake_tid = if pipe.readers == 0 {
                    pipe.blocked_writer.take()
                } else {
                    None
                };
            }
            PipeEnd::Write => {
                pipe.writers = pipe.writers.saturating_sub(1);
                // If last writer closed, wake the blocked reader (it will see EOF).
                wake_tid = if pipe.writers == 0 {
                    pipe.blocked_reader.take()
                } else {
                    None
                };
            }
        }

        should_remove = pipe.readers == 0 && pipe.writers == 0;
        if should_remove {
            guard.pipes.remove(&handle.pipe_id.0);
        }
    }
    // PIPES lock dropped — now safe to call into the scheduler.

    if let Some(tid) = wake_tid {
        sched::wake_pipe_task(tid);
    }
}

/// Increment the refcount for a pipe end. Called during fork's fd table clone.
///
/// SAFETY: This acquires the PIPES lock but NOT the scheduler lock, so it is
/// safe to call while the scheduler lock is already held (lock ordering:
/// scheduler → PIPES).
pub fn pipe_clone(handle: &PipeHandle) {
    let table = match PIPES.get() {
        Some(t) => t,
        None => return,
    };
    let mut guard = table.lock();
    let pipe = match guard.pipes.get_mut(&handle.pipe_id.0) {
        Some(p) => p,
        None => return,
    };

    match handle.end {
        PipeEnd::Read => pipe.readers += 1,
        PipeEnd::Write => pipe.writers += 1,
    }
}
