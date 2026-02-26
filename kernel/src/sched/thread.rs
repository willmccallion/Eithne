//! Kernel threads.
//!
//! Kernel threads run in ring 0 and share the kernel address space.
//! They are used for background work (e.g., flushing dirty pages,
//! handling deferred interrupts).

// TODO: spawn_kernel_thread(name, entry_fn) -> TaskId
// TODO: Allocate a kernel stack
// TODO: Set up initial context (instruction pointer = entry_fn)
// TODO: Add to scheduler
