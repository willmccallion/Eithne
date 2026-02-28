//! POSIX-lite signal mechanism.
//!
//! Supports a subset of POSIX signals: SIGINT, SIGKILL, SIGPIPE, SIGTERM,
//! SIGCHLD. Provides per-task pending/blocked masks, custom handlers via
//! sigaction, and signal delivery on return to user space.

extern crate alloc;

use crate::sched::traits::TaskId;

// ---------------------------------------------------------------------------
// Signal numbers (Linux-compatible)
// ---------------------------------------------------------------------------

pub const SIGINT: u8 = 2;
pub const SIGKILL: u8 = 9;
pub const SIGPIPE: u8 = 13;
pub const SIGTERM: u8 = 15;
pub const SIGCHLD: u8 = 17;

/// Maximum number of signals supported (signals 1..31).
pub const NSIG: usize = 32;

// ---------------------------------------------------------------------------
// Signal action
// ---------------------------------------------------------------------------

/// User-space sentinel values for handler field in SigActionUser.
pub const SIG_DFL: u64 = 0;
pub const SIG_IGN: u64 = 1;

/// What to do when a signal is delivered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SigAction {
    /// Default action for this signal.
    Default,
    /// Ignore the signal.
    Ignore,
    /// Run a user-space handler at the given address.
    Handler(u64),
}

/// Default action category for each signal.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DefaultAction {
    Terminate,
    Ignore,
}

/// Return the default action for a signal number.
pub fn default_action(sig: u8) -> DefaultAction {
    match sig {
        SIGCHLD => DefaultAction::Ignore,
        _ => DefaultAction::Terminate,
    }
}

// ---------------------------------------------------------------------------
// Per-task signal state
// ---------------------------------------------------------------------------

/// Per-task signal bookkeeping, stored in the Task struct.
///
/// Bitmask convention: bit N represents signal number N (1-indexed).
/// Bit 0 is unused (there is no signal 0).
#[derive(Clone)]
pub struct SignalState {
    /// Bitmask of signals that have been posted but not yet delivered.
    pub pending: u32,
    /// Bitmask of signals that are blocked (sigprocmask).
    pub blocked: u32,
    /// Disposition for each signal number (indexed 0..NSIG, index 0 unused).
    pub actions: [SigAction; NSIG],
    /// Signal mask to restore after the current handler returns (sigreturn).
    pub saved_mask: u32,
}

impl SignalState {
    /// Create a new signal state with all defaults.
    pub fn new() -> Self {
        Self {
            pending: 0,
            blocked: 0,
            actions: [SigAction::Default; NSIG],
            saved_mask: 0,
        }
    }

    /// Convert a signal number to its bitmask. Returns 0 for out-of-range.
    #[inline]
    pub fn sig_bit(sig: u8) -> u32 {
        if sig == 0 || sig as usize >= NSIG {
            0
        } else {
            1u32 << sig
        }
    }

    /// Returns the bitmask of signals that are pending and not blocked.
    #[inline]
    pub fn deliverable(&self) -> u32 {
        self.pending & !self.blocked
    }

    /// Returns the lowest-numbered deliverable signal, or None.
    pub fn next_deliverable(&self) -> Option<u8> {
        let d = self.deliverable();
        if d == 0 {
            None
        } else {
            Some(d.trailing_zeros() as u8)
        }
    }

    /// Returns true if there are any deliverable pending signals.
    #[inline]
    pub fn has_deliverable(&self) -> bool {
        self.deliverable() != 0
    }
}

// ---------------------------------------------------------------------------
// User-space sigaction struct (matches user/shell layout)
// ---------------------------------------------------------------------------

/// Layout passed between user space and kernel for sigaction.
#[repr(C)]
pub struct SigActionUser {
    /// 0 = SIG_DFL, 1 = SIG_IGN, else handler function pointer.
    pub handler: u64,
    /// Reserved flags (SA_RESTART, etc.) — unused for now.
    pub flags: u64,
    /// Additional signals to block during handler execution.
    pub mask: u32,
    pub _pad: u32,
}

// ---------------------------------------------------------------------------
// SigFrame — pushed on user stack for signal delivery
// ---------------------------------------------------------------------------

/// Frame pushed onto the user stack when delivering a signal.
///
/// The signal handler sees this as its stack frame. When the handler
/// returns, the restorer trampoline calls sigreturn, which reads this
/// frame to restore the interrupted context.
#[repr(C)]
pub struct SigFrame {
    /// Address of the sigreturn trampoline (pushed as return address).
    pub restorer: u64,
    /// Signal number (first argument to handler, in RDI).
    pub sig_num: u64,
    // Saved register state
    pub saved_rip: u64,
    pub saved_rsp: u64,
    pub saved_rflags: u64,
    pub saved_rax: u64,
    pub saved_rdx: u64,
    pub saved_rsi: u64,
    pub saved_rdi: u64,
    pub saved_r8: u64,
    pub saved_r9: u64,
    pub saved_r10: u64,
    /// Signal mask to restore on sigreturn.
    pub saved_mask: u32,
    pub _pad: u32,
}

/// Sigreturn trampoline: `mov eax, 15; syscall; ud2`
/// This is 9 bytes written onto the user stack.
///
/// Machine code:
///   b8 0f 00 00 00    mov eax, 15   (SYS_SIGRETURN)
///   0f 05              syscall
///   0f 0b              ud2           (trap if sigreturn fails)
pub const SIGRETURN_TRAMPOLINE: [u8; 9] = [
    0xb8, 0x0f, 0x00, 0x00, 0x00, // mov eax, 15
    0x0f, 0x05, // syscall
    0x0f, 0x0b, // ud2
];

// ---------------------------------------------------------------------------
// Sending signals
// ---------------------------------------------------------------------------

/// Post a signal to a task by ID.
///
/// Sets the pending bit in the target's signal state. If the task is
/// `Blocked` and the signal is not blocked, wakes it (so blocking syscalls
/// return EINTR).
///
/// Returns `true` if the signal was posted, `false` if the target was not
/// found or has no signal state (kernel thread).
///
/// Must be called with the scheduler lock held (via `with_scheduler`).
pub fn send_signal_locked(
    guard: &mut crate::sched::round_robin::RoundRobinScheduler,
    tid: TaskId,
    sig: u8,
) -> bool {
    use crate::sched::process::TaskState;

    let bit = SignalState::sig_bit(sig);
    if bit == 0 {
        return false;
    }

    let task = match guard.find_task_mut(tid) {
        Some(t) => t,
        None => return false,
    };

    let sig_state = match task.signals.as_mut() {
        Some(s) => s,
        None => return false, // kernel thread
    };

    // Always set pending, even if blocked (delivered when unblocked).
    sig_state.pending |= bit;

    // If the signal is not blocked (or is SIGKILL), wake the task.
    let should_wake =
        task.state == TaskState::Blocked && (sig == SIGKILL || (sig_state.blocked & bit) == 0);

    if should_wake {
        task.state = TaskState::Ready;
    }

    true
}

// ---------------------------------------------------------------------------
// Checking for pending signals (used by interruptible blocking)
// ---------------------------------------------------------------------------

/// Returns true if the current task has any deliverable (pending & ~blocked)
/// signals. Must be called with the scheduler lock held.
pub fn has_pending_signal_locked(
    guard: &mut crate::sched::round_robin::RoundRobinScheduler,
) -> bool {
    guard
        .current_task_mut()
        .and_then(|t| t.signals.as_ref())
        .map(|s| s.has_deliverable())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Signal delivery (syscall and interrupt return paths)
// ---------------------------------------------------------------------------

/// Information extracted from the scheduler lock about a deliverable signal.
struct DeliverInfo {
    sig: u8,
    handler: u64,
    old_mask: u32,
}

/// Result from checking a signal's disposition under the scheduler lock.
enum SignalDisposition {
    /// No deliverable signals.
    None,
    /// Signal was ignored or had ignore-by-default (SIGCHLD). Check again.
    Ignored,
    /// Signal has default-terminate action. Fds have been drained (returned).
    Terminate(alloc::vec::Vec<crate::sched::fd::FdKind>),
    /// Signal has a custom handler — deliver via SigFrame.
    Handler(DeliverInfo),
}

/// Check the current task for pending signals and deliver one if possible.
///
/// This is called from two places:
///   1. End of `syscall_dispatch` (syscall return path)
///   2. `check_resched` (interrupt return path)
///
/// If a signal has a custom handler: pushes a SigFrame on the user stack,
/// rewrites the frame to jump to the handler, and returns.
///
/// If a signal has a default-terminate action: kills the current task.
///
/// If a signal is ignored or is SIGCHLD with default action: clears it.
///
/// This function acquires and releases the scheduler lock internally.
pub fn deliver_pending_signal(frame: &mut crate::arch::x86_64::syscall::SyscallFrame) {
    // Only deliver signals when returning to user mode.
    if frame.cs & 3 == 0 {
        return;
    }

    loop {
        // Under scheduler lock: find the next deliverable signal and determine disposition.
        let disposition = crate::sched::with_scheduler(|guard| {
            // First pass: read signal info from current task.
            let (sig, action) = {
                let task = match guard.current_task_mut() {
                    Some(t) => t,
                    None => return SignalDisposition::None,
                };
                let sig_state = match task.signals.as_mut() {
                    Some(s) => s,
                    None => return SignalDisposition::None,
                };
                let sig = match sig_state.next_deliverable() {
                    Some(s) => s,
                    None => return SignalDisposition::None,
                };
                let bit = SignalState::sig_bit(sig);
                sig_state.pending &= !bit;
                let action = sig_state.actions[sig as usize];

                // For Handler: update mask now while we have the borrow.
                if let SigAction::Handler(_) = action {
                    let old_mask = sig_state.blocked;
                    sig_state.saved_mask = old_mask;
                    sig_state.blocked |= bit;
                }

                (sig, action)
            };
            // task borrow dropped here.

            match action {
                SigAction::Ignore => SignalDisposition::Ignored,
                SigAction::Default => {
                    match default_action(sig) {
                        DefaultAction::Ignore => SignalDisposition::Ignored,
                        DefaultAction::Terminate => {
                            let status = sig as i32 & 0x7F;

                            // Drain fds (re-borrow current task).
                            let fds = guard
                                .current_task_mut()
                                .and_then(|t| t.fd_table.as_mut())
                                .map(|fdt| fdt.drain_all())
                                .unwrap_or_default();

                            guard.kill_current_with_status(status);

                            if let Some(my_id) = guard.current_id() {
                                guard.reparent_children(my_id, crate::sched::traits::TaskId(2));
                            }
                            if let Some(parent_id) = guard.current_parent() {
                                guard.wake_task(parent_id);
                            }

                            SignalDisposition::Terminate(fds)
                        }
                    }
                }
                SigAction::Handler(addr) => {
                    // Mask was already updated above.
                    let old_mask = guard
                        .current_task_mut()
                        .and_then(|t| t.signals.as_ref())
                        .map(|s| s.saved_mask)
                        .unwrap_or(0);
                    SignalDisposition::Handler(DeliverInfo {
                        sig,
                        handler: addr,
                        old_mask,
                    })
                }
            }
        });

        match disposition {
            Some(SignalDisposition::None) | None => return,
            Some(SignalDisposition::Ignored) => continue,
            Some(SignalDisposition::Terminate(fds)) => {
                // Close fds outside the scheduler lock.
                for kind in fds {
                    match kind {
                        crate::sched::fd::FdKind::Pipe(h) => {
                            crate::ipc::pipe::pipe_close(h);
                        }
                        crate::sched::fd::FdKind::Vfs(h) => {
                            let _ = crate::fs::vfs::close(h);
                        }
                    }
                }
                // Yield — this task is Dead, won't be scheduled again.
                crate::sched::yield_cpu();
                loop {
                    crate::arch::halt_once();
                }
            }
            Some(SignalDisposition::Handler(info)) => {
                push_sigframe(frame, &info);
                return;
            }
        }
    }
}

/// Push a `SigFrame` + sigreturn trampoline onto the user stack and
/// rewrite the frame to jump to the signal handler.
fn push_sigframe(frame: &mut crate::arch::x86_64::syscall::SyscallFrame, info: &DeliverInfo) {
    let user_rsp = frame.rsp;

    // Layout on user stack (growing downward):
    //   [trampoline bytes]    (SIGRETURN_TRAMPOLINE, 9 bytes)
    //   [SigFrame]
    //
    // The trampoline address is stored as `restorer` in SigFrame.
    // When the handler does `ret`, it pops `restorer` and jumps to the trampoline.

    // Align down for the trampoline.
    let trampoline_size = SIGRETURN_TRAMPOLINE.len();
    let trampoline_addr = (user_rsp as usize - trampoline_size) & !0xF; // 16-byte align
    let sigframe_addr = trampoline_addr - core::mem::size_of::<SigFrame>();
    // Ensure 16-byte alignment for the sigframe (and thus RSP for the handler).
    let sigframe_addr = sigframe_addr & !0xF;

    // Write trampoline bytes.
    let tramp_ptr = trampoline_addr as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(SIGRETURN_TRAMPOLINE.as_ptr(), tramp_ptr, trampoline_size);
    }

    // Write SigFrame.
    let sigframe_ptr = sigframe_addr as *mut SigFrame;
    unsafe {
        sigframe_ptr.write(SigFrame {
            restorer: trampoline_addr as u64,
            sig_num: info.sig as u64,
            saved_rip: frame.rip,
            saved_rsp: frame.rsp,
            saved_rflags: frame.rflags,
            saved_rax: frame.rax,
            saved_rdx: frame.rdx,
            saved_rsi: frame.rsi,
            saved_rdi: frame.rdi,
            saved_r8: frame.r8,
            saved_r9: frame.r9,
            saved_r10: frame.r10,
            saved_mask: info.old_mask,
            _pad: 0,
        });
    }

    // Rewrite the frame so that when we return to user space:
    //   RIP = handler address
    //   RDI = signal number (first argument, System V ABI)
    //   RSP = sigframe_addr (handler's "return address" is restorer)
    frame.rip = info.handler;
    frame.rdi = info.sig as u64;
    frame.rsp = sigframe_addr as u64;
    // Clear direction flag, keep interrupts enabled.
    frame.rflags = 0x202;
}

// ---------------------------------------------------------------------------
// sigreturn — restore context from SigFrame
// ---------------------------------------------------------------------------

/// Restore the interrupted context from a SigFrame on the user stack.
///
/// Called from the `sys_sigreturn` handler. Reads the SigFrame from the
/// current user RSP, restores the saved registers into the syscall frame,
/// and restores the signal mask.
pub fn do_sigreturn(frame: &mut crate::arch::x86_64::syscall::SyscallFrame) {
    // The user RSP should point at the SigFrame (the handler did `ret`
    // which popped the restorer, then the trampoline called syscall, so
    // RSP is pointing past the trampoline call. We need to find the SigFrame.)
    //
    // Actually: the handler's stack looks like:
    //   RSP → [SigFrame.restorer]  (this is the "return address")
    //          [SigFrame fields...]
    //
    // When the handler does `ret`, it pops restorer into RIP and RSP moves
    // up by 8 bytes. The trampoline then calls `syscall`. On SYSCALL entry
    // RCX=user_rip, R11=rflags, and the user RSP in the frame is past the
    // restorer.
    //
    // So on sigreturn entry, frame.rsp is at sigframe_addr + 8 (past restorer).
    // We need to go back 8 bytes to find the SigFrame start.
    let sigframe_addr = frame.rsp.wrapping_sub(8);
    let sigframe = unsafe { &*(sigframe_addr as *const SigFrame) };

    // Restore saved registers.
    frame.rip = sigframe.saved_rip;
    frame.rsp = sigframe.saved_rsp;
    frame.rflags = sigframe.saved_rflags;
    frame.rax = sigframe.saved_rax;
    frame.rdx = sigframe.saved_rdx;
    frame.rsi = sigframe.saved_rsi;
    frame.rdi = sigframe.saved_rdi;
    frame.r8 = sigframe.saved_r8;
    frame.r9 = sigframe.saved_r9;
    frame.r10 = sigframe.saved_r10;

    // Also restore RCX and R11 for the sysret path.
    frame.rcx = sigframe.saved_rip; // RCX = user RIP for SYSRET
    frame.r11 = sigframe.saved_rflags; // R11 = user RFLAGS for SYSRET

    // Restore signal mask under scheduler lock.
    let saved_mask = sigframe.saved_mask;
    let _ = crate::sched::with_current_task(|task| {
        if let Some(ref mut sig_state) = task.signals {
            sig_state.blocked = saved_mask;
        }
        Ok(())
    });

    // Check for newly deliverable signals after mask restore.
    deliver_pending_signal(frame);
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

/// Initialize the signal subsystem. Currently a no-op since all state
/// is per-task, but reserves the slot for future global signal infrastructure.
pub fn init() {
    crate::println!("[signal] signal subsystem initialized");
}
