//! x86_64 kernel-thread context switching.
//!
//! Only callee-saved registers need to be preserved across a cooperative
//! switch: RBX, RBP, R12-R15, RSP.  The instruction pointer is implicit —
//! after `switch_context` returns on the new stack, execution continues at
//! the next instruction in the new task's call chain.
//!
//! The first time a newly-created task is switched to, its stack has a
//! synthetic `SwitchFrame` placed by `TaskStack::new_for_fn` with RSP
//! pointing just above it and the "return address" slot set to the task
//! entry function.  When `switch_context` does its `ret`, it pops that
//! address and jumps there.

use core::arch::global_asm;

// ---------------------------------------------------------------------------
// SwitchFrame — the register state saved on the kernel stack
// ---------------------------------------------------------------------------

/// Callee-saved registers pushed by `switch_context` before switching stacks.
/// The layout must match the `switch_context` assembly exactly.
#[repr(C)]
pub struct SwitchFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    /// The "return address" — where execution resumes when this task is
    /// next switched to and `switch_context` does its `ret`.
    pub rip: u64,
}

// ---------------------------------------------------------------------------
// switch_context — assembly routine
// ---------------------------------------------------------------------------

// switch_context(old_rsp: *mut u64, new_rsp: u64)
//
// x86-64 System V ABI: first arg in RDI, second in RSI.
// (The UEFI target uses the MS ABI, but this is a purely internal function
//  declared `extern "C"` with System V semantics since we control both
//  call sites — on x86-64 UEFI the MS ABI applies.  We therefore use
//  RCX/RDX for the two arguments, matching the MS ABI.)
//
// Calling convention on the UEFI/MS-ABI target:
//   RCX = old_rsp (pointer to where to save the old stack pointer)
//   RDX = new_rsp (the stack pointer to switch to)
//
// What this does:
//   1. Push callee-saved regs + the return address already on the stack.
//   2. Save RSP into *old_rsp.
//   3. Load RSP from new_rsp.
//   4. Pop callee-saved regs.
//   5. ret → jumps to the saved RIP of the new task.
global_asm!(
    ".align 16",
    ".globl switch_context",
    "switch_context:",
    // Push callee-saved registers + implicit return address (already on stack
    // from the `call` instruction) in the order matching SwitchFrame.
    // Note: `call` pushed RIP before we got here, so RSP is already below it.
    // We push the remaining callee-saved registers on top.
    "push rbx",
    "push rbp",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    // *old_rsp = RSP
    "mov [rcx], rsp",
    // RSP = new_rsp
    "mov rsp, rdx",
    // Restore callee-saved registers of the new task
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbp",
    "pop rbx",
    // Return to new task's saved RIP
    "ret",
);

unsafe extern "C" {
    /// Low-level context switch.
    ///
    /// Saves the current task's callee-saved registers + RSP to `*old_rsp`,
    /// then loads the new task's stack pointer and restores its registers.
    ///
    /// # Safety
    /// - `old_rsp` must point to writable memory that outlives the switch.
    /// - `new_rsp` must be a valid kernel stack with a `SwitchFrame` at its top.
    pub fn switch_context(old_rsp: *mut u64, new_rsp: u64);
}

// ---------------------------------------------------------------------------
// TaskStack — an allocated kernel stack
// ---------------------------------------------------------------------------

use crate::mm::page::PAGE_SIZE;
use crate::mm::{VirtAddr, frame};

/// A kernel stack allocated from the frame allocator.
pub struct TaskStack {
    /// Physical / virtual base (bottom) of the stack allocation.
    base: VirtAddr,
    /// Number of pages allocated.
    pages: usize,
    /// Initial RSP to pass to `switch_context`.
    pub initial_rsp: u64,
}

impl TaskStack {
    /// Number of pages per kernel stack (64 KiB).
    pub const STACK_PAGES: usize = 16;

    /// Allocate a kernel stack and set it up so that switching to it will
    /// call `entry_fn` with no arguments.
    pub fn new_for_fn(entry_fn: fn() -> !) -> Option<Self> {
        use crate::mm::page::pages_to_order;

        let pages = Self::STACK_PAGES;
        let order = pages_to_order(pages);
        let phys = frame::alloc_pages(order)?;

        // Identity-mapped: virt == phys.
        let base = VirtAddr::new(phys.as_u64());
        // Stack top (grows down).
        let stack_top = base.offset((pages * PAGE_SIZE) as u64);

        // Build a synthetic SwitchFrame at the top of the stack.
        // We lay it out manually: push rip last (highest address when
        // viewed as pushed by `call`), then rbx, rbp, r12-r15.
        //
        // Memory layout at stack_top (growing downward):
        //   [stack_top - 8 ] = entry_fn   ← this is what `ret` will pop as RIP
        //   [stack_top - 16] = rbx = 0
        //   [stack_top - 24] = rbp = 0
        //   [stack_top - 32] = r12 = 0
        //   [stack_top - 40] = r13 = 0
        //   [stack_top - 48] = r14 = 0
        //   [stack_top - 56] = r15 = 0   ← initial RSP points here

        let frame_size = core::mem::size_of::<SwitchFrame>();
        let frame_ptr = (stack_top.as_u64() - frame_size as u64) as *mut SwitchFrame;
        unsafe {
            (*frame_ptr) = SwitchFrame {
                rip: entry_fn as usize as u64,
                rbx: 0,
                rbp: 0,
                r12: 0,
                r13: 0,
                r14: 0,
                r15: 0,
            };
        }

        Some(Self {
            base,
            pages,
            initial_rsp: frame_ptr as u64,
        })
    }
}

impl Drop for TaskStack {
    fn drop(&mut self) {
        use crate::mm::page::pages_to_order;
        let order = pages_to_order(self.pages);
        let phys = crate::mm::addr::virt_to_phys(self.base);
        frame::dealloc_pages(phys, order);
    }
}
