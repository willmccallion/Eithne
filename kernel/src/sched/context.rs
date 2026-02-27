//! x86_64 kernel-thread context switching.
//!
//! Every task's `saved_rsp` points at an `InterruptFrame` on its kernel stack.
//! All tasks resume via a single path: load RSP, pop GPRs, `iretq`.
//!
//! - **Preempted tasks**: `isr_common` built the frame; `check_resched` swaps
//!   RSP before the `iretq` at the end of `isr_common`.
//! - **New tasks**: `TaskStack::new_for_fn` builds a synthetic frame pointing
//!   at the entry function with interrupts enabled.
//! - **Cooperative yield**: `yield_now` builds a synthetic frame (resume label
//!   = the instruction after `yield_now` returns), saves RSP, loads the next
//!   task's RSP, and falls into `sched_iretq_exit`.

use core::arch::global_asm;

use crate::mm::page::PAGE_SIZE;
use crate::mm::{VirtAddr, addr::virt_to_phys, frame};

// ---------------------------------------------------------------------------
// InterruptFrame — must match the push order in isr_common (idt.rs)
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct InterruptFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    // CPU-pushed:
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

/// RFLAGS for a new kernel thread: IF=1 + reserved bit 1.
pub const RFLAGS_KERNEL_IF: u64 = 0x202;

// ---------------------------------------------------------------------------
// TaskStack
// ---------------------------------------------------------------------------

pub struct TaskStack {
    base: VirtAddr,
    pages: usize,
    pub initial_rsp: u64,
}

impl TaskStack {
    pub const STACK_PAGES: usize = 16;

    pub fn new_for_fn(entry_fn: fn() -> !) -> Option<Self> {
        // KERNEL_CS is only defined for x86_64; on other archs the scheduler
        // stubs don't use this path.
        #[cfg(target_arch = "x86_64")]
        let kernel_cs: u16 = 0x08; // matches gdt::KERNEL_CS
        use crate::mm::page::pages_to_order;
        let pages = Self::STACK_PAGES;
        let phys = frame::alloc_pages(pages_to_order(pages))?;
        let base = VirtAddr::new(phys.as_u64());
        let top = base.offset((pages * PAGE_SIZE) as u64);
        let frame_ptr =
            (top.as_u64() - core::mem::size_of::<InterruptFrame>() as u64) as *mut InterruptFrame;
        unsafe {
            frame_ptr.write(InterruptFrame {
                r15: 0,
                r14: 0,
                r13: 0,
                r12: 0,
                r11: 0,
                r10: 0,
                r9: 0,
                r8: 0,
                rbp: 0,
                rdi: 0,
                rsi: 0,
                rdx: 0,
                rcx: 0,
                rbx: 0,
                rax: 0,
                vector: 0xFF,
                error_code: 0,
                rip: entry_fn as usize as u64,
                cs: kernel_cs as u64,
                rflags: RFLAGS_KERNEL_IF,
                rsp: top.as_u64(),
                ss: 0,
            });
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
        frame::dealloc_pages(virt_to_phys(self.base), pages_to_order(self.pages));
    }
}

// ---------------------------------------------------------------------------
// sched_iretq_exit — single resume path: pop InterruptFrame + iretq
// ---------------------------------------------------------------------------

global_asm!(
    ".align 16",
    ".globl sched_iretq_exit",
    "sched_iretq_exit:",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rbp",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",
    "pop rbx",
    "pop rax",
    "add rsp, 16", // skip vector + error_code
    "iretq",
);

// ---------------------------------------------------------------------------
// yield_now(old_rsp_ptr: *mut u64, new_rsp: u64)  [MS ABI: rcx, rdx]
// ---------------------------------------------------------------------------
//
// Builds a synthetic InterruptFrame on the current kernel stack so this task
// can be resumed via sched_iretq_exit.  The saved RIP points at .Lresume,
// which executes `ret` back to yield_cpu's caller.
//
// Uses R10/R11 as scratch (volatile in MS ABI, not used for arg passing here).

global_asm!(
    ".align 16",
    ".globl yield_now",
    "yield_now:",
    // Stash args in volatile regs not used for passing (r10, r11)
    "mov r10, rcx", // r10 = old_rsp_ptr
    "mov r11, rdx", // r11 = new_rsp
    // Build iretq frame (high address → low address on stack):
    "xor eax, eax",
    "push rax",           // ss = 0
    "lea rax, [rsp + 8]", // rsp value before this frame (8 = size of ss slot)
    "push rax",           // rsp
    "pushfq",             // rflags (captures current IF state)
    "push 0x8",           // cs = KERNEL_CS
    "lea rax, [rip + .Lyield_resume]",
    "push rax",  // rip = resume label
    "push 0",    // error_code
    "push 0xFF", // vector
    // Push all GPRs (rax clobbered above — saved value is 0xFF from last push; fine)
    "push rax",
    "push rbx",
    "push rcx",
    "push rdx",
    "push rsi",
    "push rdi",
    "push rbp",
    "push r8",
    "push r9",
    "push r10",
    "push r11",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    // RSP now points at the InterruptFrame. Save into *old_rsp_ptr.
    "mov [r10], rsp",
    // Switch to new task and resume via iretq.
    "mov rsp, r11",
    "jmp sched_iretq_exit",
    // Resume label: this task is scheduled back in. iretq lands here.
    ".Lyield_resume:",
    "ret", // return to yield_cpu's caller
);

unsafe extern "C" {
    pub fn yield_now(old_rsp_ptr: *mut u64, new_rsp: u64);
}
