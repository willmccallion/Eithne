//! x86_64 SYSCALL/SYSRET fast system call interface.
//!
//! SYSCALL saves RIP → RCX, RFLAGS → R11, then jumps to LSTAR.
//! SYSRET restores RIP ← RCX, RFLAGS ← R11, and returns to ring 3.
//!
//! Calling convention (Linux-compatible):
//!   RAX = syscall number
//!   RDI, RSI, RDX, R10, R8, R9 = arguments 1-6
//!   RAX = return value (negative errno on error)

use core::arch::global_asm;

// MSR addresses
const MSR_STAR: u32 = 0xC000_0081;
const MSR_LSTAR: u32 = 0xC000_0082;
const MSR_SFMASK: u32 = 0xC000_0084;
const MSR_EFER: u32 = 0xC000_0080;

// EFER.SCE bit — enables SYSCALL/SYSRET
const EFER_SCE: u64 = 1 << 0;

// RFLAGS bits to mask on syscall entry (IF=1<<9, DF=1<<10, TF=1<<8)
// We clear IF so we enter the handler with interrupts disabled, then
// re-enable them in the Rust dispatcher after saving state.
const SFMASK: u64 = (1 << 9) | (1 << 10) | (1 << 8);

/// Read a model-specific register.
#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack),
        );
    }
    ((hi as u64) << 32) | lo as u64
}

/// Write a model-specific register.
#[inline]
unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") lo,
            in("edx") hi,
            options(nomem, nostack),
        );
    }
}

/// Initialize the SYSCALL/SYSRET machinery.
///
/// Must be called after GDT is loaded.
pub fn init() {
    unsafe {
        // Enable SCE in EFER.
        let efer = rdmsr(MSR_EFER);
        wrmsr(MSR_EFER, efer | EFER_SCE);

        // STAR layout (bits):
        //   [63:48] SYSRET CS/SS base  → user CS = this + 16, user SS = this + 8
        //   [47:32] SYSCALL CS/SS base → kernel CS = this, kernel SS = this + 8
        //
        // We use:
        //   kernel CS = 0x08, kernel SS = 0x10
        //   user CS   = 0x18|3, user SS = 0x20|3
        //
        // STAR[47:32] = 0x0008  → SYSCALL uses CS=0x08, SS=0x10
        // STAR[63:48] = 0x0010  → SYSRET uses  CS=0x18|3, SS=0x20|3
        //   (hardware adds 8 to get SS and ORs 3 for SYSRET)
        let star: u64 = (0x0010u64 << 48) | (0x0008u64 << 32);
        wrmsr(MSR_STAR, star);

        // LSTAR = address of our syscall entry stub.
        wrmsr(MSR_LSTAR, syscall_entry as *const () as usize as u64);

        // SFMASK: bits set here are cleared in RFLAGS on syscall entry.
        wrmsr(MSR_SFMASK, SFMASK);
    }

    crate::println!(
        "[syscall] SYSCALL/SYSRET enabled, LSTAR={:#x}",
        syscall_entry as *const () as usize
    );
}

// ---------------------------------------------------------------------------
// Syscall entry stub
// ---------------------------------------------------------------------------
//
// On SYSCALL entry the CPU has:
//   RCX = saved user RIP
//   R11 = saved user RFLAGS
//   CS/SS  = kernel selectors (from STAR)
//   RSP    = still the USER stack — we must switch to kernel stack immediately
//   RFLAGS = cleared per SFMASK (IF=0, so we're non-preemptible)
//
// We need a per-CPU kernel stack pointer.  For now (single-CPU) we use a
// static scratch location.  The real implementation will store this in the
// TSS RSP0 or a per-CPU GS-base variable.
//
// Register usage:
//   RAX  = syscall number (in), return value (out)
//   RDI, RSI, RDX, R10, R8, R9 = arguments
//   RCX  = saved RIP (must be preserved for SYSRET)
//   R11  = saved RFLAGS (must be preserved for SYSRET)

global_asm!(
    ".align 16",
    ".globl syscall_entry",
    "syscall_entry:",
    // Switch to kernel stack.
    // We keep the user RSP in R15 temporarily (it's callee-saved but we
    // save/restore all GPRs in the frame anyway).
    "mov r15, rsp",
    "mov rsp, qword ptr [rip + {KSTACK_TOP}]",
    // Build a syscall frame on the kernel stack:
    //   [ss, user_rsp, rflags, cs, rip] — mirrors iretq frame layout
    //   then error_code + vector slots, then all GPRs
    "push 0x23",     // user SS (0x20 | 3)
    "push r15",      // user RSP
    "push r11",      // user RFLAGS (saved by CPU)
    "push 0x1b",     // user CS (0x18 | 3)
    "push rcx",      // user RIP (saved by CPU)
    "push 0",        // error_code slot
    "push 0xfe",     // vector = 0xFE (syscall pseudo-vector)
    "push rax",      // rax (syscall number)
    "push rbx",
    "push rcx",      // user RIP copy
    "push rdx",
    "push rsi",
    "push rdi",
    "push rbp",
    "push r8",
    "push r9",
    "push r10",
    "push r11",      // user RFLAGS copy
    "push r12",
    "push r13",
    "push r14",
    "push r15",      // user RSP copy
    // Re-enable interrupts now that we're on the kernel stack.
    "sti",
    // Call the Rust dispatcher: syscall_dispatch(frame: &mut SyscallFrame)
    // MS ABI: first arg in RCX.
    "mov rcx, rsp",
    "sub rsp, 32",
    "call {dispatch}",
    "add rsp, 32",
    // Disable interrupts before touching user state.
    "cli",
    // Restore GPRs from frame (RAX = return value written by dispatcher).
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop r11",    // restore R11 = user RFLAGS for SYSRET
    "pop r10",
    "pop r9",
    "pop r8",
    "pop rbp",
    "pop rdi",
    "pop rsi",
    "pop rdx",
    "pop rcx",    // restore RCX = user RIP for SYSRET
    "pop rbx",
    "pop rax",    // return value
    "add rsp, 16", // skip vector + error_code
    "pop rcx",    // user RIP  → RCX for SYSRET
    "add rsp, 8", // skip CS slot
    "pop r11",    // user RFLAGS → R11 for SYSRET
    "pop rsp",    // restore user RSP
    // SYSRET to 64-bit user code (REX.W prefix).
    "sysretq",
    dispatch   = sym syscall_dispatch,
    KSTACK_TOP = sym SYSCALL_KSTACK_TOP,
);

unsafe extern "C" {
    pub fn syscall_entry();
}

/// Scratch storage for the kernel RSP used on syscall entry.
///
/// Set by `set_syscall_stack` after each task switch.  In a multi-CPU kernel
/// this would be per-CPU (GS-relative).
#[unsafe(no_mangle)]
pub static mut SYSCALL_KSTACK_TOP: u64 = 0;

/// Update the kernel stack pointer used by the syscall entry stub and the
/// TSS RSP0 used by hardware interrupts from ring 3.
///
/// Called from the scheduler whenever we switch to a new task, so that
/// both syscalls and interrupts from user mode land on the correct kernel
/// stack.
pub fn set_syscall_stack(rsp_top: u64) {
    unsafe {
        SYSCALL_KSTACK_TOP = rsp_top;
    }
    super::gdt::set_rsp0(rsp_top);
}

// ---------------------------------------------------------------------------
// Syscall frame — mirrors isr_common's InterruptFrame
// ---------------------------------------------------------------------------

/// Register state saved on the kernel stack by `syscall_entry`.
/// Layout must match the push sequence above exactly.
#[repr(C)]
pub struct SyscallFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64, // user RFLAGS
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64, // user RIP
    pub rbx: u64,
    pub rax: u64, // syscall number / return value
    pub vector: u64,
    pub error_code: u64,
    pub rip: u64, // user RIP (for iretq compat)
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64, // user RSP
    pub ss: u64,
}

// ---------------------------------------------------------------------------
// Rust syscall dispatcher
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
extern "C" fn syscall_dispatch(frame: &mut SyscallFrame) {
    let num = frame.rax as usize;
    let args = [
        frame.rdi as usize,
        frame.rsi as usize,
        frame.rdx as usize,
        frame.r10 as usize,
        frame.r8 as usize,
        frame.r9 as usize,
    ];

    let result = crate::syscall::dispatch::dispatch(num, &args);

    frame.rax = match result {
        Ok(val) => val as u64,
        Err(e) => {
            use crate::syscall::traits::SyscallError;
            let errno: u64 = match e {
                SyscallError::InvalidSyscall => 38,  // ENOSYS
                SyscallError::InvalidArgument => 22, // EINVAL
                SyscallError::PermissionDenied => 1, // EPERM
                SyscallError::NotFound => 2,         // ENOENT
                SyscallError::IoError => 5,          // EIO
                SyscallError::NoMemory => 12,        // ENOMEM
                SyscallError::WouldBlock => 11,      // EAGAIN
                SyscallError::AlreadyExists => 17,   // EEXIST
                SyscallError::IsADirectory => 21,    // EISDIR
                SyscallError::NotADirectory => 20,   // ENOTDIR
            };
            errno.wrapping_neg() // return negative errno
        }
    };
}
