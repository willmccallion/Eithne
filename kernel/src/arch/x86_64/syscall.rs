//! x86_64 SYSCALL/SYSRET fast system call interface.
//!
//! SYSCALL saves RIP → RCX, RFLAGS → R11, then jumps to LSTAR.
//! SYSRET restores RIP ← RCX, RFLAGS ← R11, and returns to ring 3.
//!
//! Calling convention (Linux-compatible):
//!   RAX = syscall number
//!   RDI, RSI, RDX, R10, R8, R9 = arguments 1-6
//!   RAX = return value (negative errno on error)

extern crate alloc;

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
    // Save user RSP to a scratch variable without clobbering any GPR.
    // (Using a register like R15 as scratch would corrupt its value —
    //  user code legitimately keeps data in callee-saved registers
    //  across syscalls.)
    "mov qword ptr [rip + {USER_RSP}], rsp",
    // Switch to kernel stack.
    "mov rsp, qword ptr [rip + {KSTACK_TOP}]",
    // Build a syscall frame on the kernel stack:
    //   [ss, user_rsp, rflags, cs, rip] — mirrors iretq frame layout
    //   then error_code + vector slots, then all GPRs
    "push 0x23",     // user SS (0x20 | 3)
    "push qword ptr [rip + {USER_RSP}]", // user RSP
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
    "push r15",      // original R15 (preserved)
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
    USER_RSP   = sym SYSCALL_USER_RSP,
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

/// Scratch storage for the user RSP during syscall entry.
///
/// The entry stub saves the user RSP here (without clobbering any GPR)
/// before switching to the kernel stack.  In a multi-CPU kernel this
/// would be per-CPU (GS-relative).
#[unsafe(no_mangle)]
static mut SYSCALL_USER_RSP: u64 = 0;

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

// ---------------------------------------------------------------------------
// Linux errno constants
// ---------------------------------------------------------------------------

const EPERM: u64 = 1;
const ENOENT: u64 = 2;
const EIO: u64 = 5;
const ECHILD: u64 = 10;
const EAGAIN: u64 = 11;
const ENOMEM: u64 = 12;
const EEXIST: u64 = 17;
const ENOTDIR: u64 = 20;
const EISDIR: u64 = 21;
const EINVAL: u64 = 22;
const ENOSYS: u64 = 38;
const ENOEXEC: u64 = 8;

fn syscall_error_to_errno(e: crate::syscall::traits::SyscallError) -> u64 {
    use crate::syscall::traits::SyscallError;
    match e {
        SyscallError::InvalidSyscall => ENOSYS,
        SyscallError::InvalidArgument => EINVAL,
        SyscallError::PermissionDenied => EPERM,
        SyscallError::NotFound => ENOENT,
        SyscallError::IoError => EIO,
        SyscallError::NoMemory => ENOMEM,
        SyscallError::WouldBlock => EAGAIN,
        SyscallError::AlreadyExists => EEXIST,
        SyscallError::IsADirectory => EISDIR,
        SyscallError::NotADirectory => ENOTDIR,
        SyscallError::NoChildren => ECHILD,
    }
}

#[unsafe(no_mangle)]
extern "C" fn syscall_dispatch(frame: &mut SyscallFrame) {
    use crate::syscall::table::{SYS_EXECVE, SYS_FORK};

    let num = frame.rax as usize;

    // Fork and exec need direct access to the frame — handle before generic dispatch.
    match num {
        SYS_FORK => {
            frame.rax = do_fork(frame);
            return;
        }
        SYS_EXECVE => {
            do_exec(frame);
            return;
        }
        _ => {}
    }

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
        Err(e) => syscall_error_to_errno(e).wrapping_neg(),
    };
}

// ---------------------------------------------------------------------------
// fork()
// ---------------------------------------------------------------------------

/// User-mode segment selectors (must match gdt.rs).
const USER_CS: u64 = 0x18 | 3;
const USER_DS: u64 = 0x20 | 3;
/// RFLAGS with interrupts enabled + reserved bit 1.
const USER_RFLAGS: u64 = 0x202;
/// Pseudo-vector number for user tasks created via fork (not a real IRQ).
const USER_TASK_VECTOR: u64 = 0xFD;

fn do_fork(parent_frame: &SyscallFrame) -> u64 {
    use crate::mm::addr::phys_to_virt;
    use crate::mm::frame;
    use crate::mm::page::PAGE_SIZE;
    use crate::sched::context::{InterruptFrame, TaskStack};
    use crate::sched::process::{Task, TaskState};

    // Gather info from the current (parent) task under the scheduler lock.
    let result = crate::sched::with_scheduler(|guard| {
        let cur = match guard.current_task_mut() {
            Some(t) => t,
            None => return Err(ENOMEM),
        };

        let parent_id = cur.id;

        // Clone address space.
        let aspace = match cur.aspace.as_ref() {
            Some(a) => match a.clone_user_pages() {
                Ok(child) => child,
                Err(e) => {
                    crate::println!("[fork] clone_user_pages failed: {:?}", e);
                    return Err(ENOMEM);
                }
            },
            None => return Err(EINVAL),
        };

        // Clone FD table.
        let fdt = match cur.fd_table.as_ref() {
            Some(f) => f.clone_table(),
            None => crate::sched::fd::FdTable::new_with_stdio(),
        };

        Ok((parent_id, aspace, fdt))
    });

    let (parent_id, child_aspace, child_fdt) = match result {
        Some(Ok(v)) => v,
        Some(Err(e)) => return e.wrapping_neg(),
        None => return ENOMEM.wrapping_neg(),
    };

    // Allocate child kernel stack.
    let kernel_stack_pages = TaskStack::STACK_PAGES;
    let kstack_phys = match frame::alloc_pages(crate::mm::page::pages_to_order(kernel_stack_pages))
    {
        Some(p) => p,
        None => return ENOMEM.wrapping_neg(),
    };
    let kstack_base = phys_to_virt(kstack_phys);
    let kstack_top = kstack_base.offset((kernel_stack_pages * PAGE_SIZE) as u64);

    // Copy the parent's SyscallFrame to the top of the child's kernel stack
    // as an InterruptFrame (same layout). Set child's RAX = 0 (fork returns 0 in child).
    let child_frame_ptr = (kstack_top.as_u64() - core::mem::size_of::<InterruptFrame>() as u64)
        as *mut InterruptFrame;
    unsafe {
        child_frame_ptr.write(InterruptFrame {
            r15: parent_frame.r15,
            r14: parent_frame.r14,
            r13: parent_frame.r13,
            r12: parent_frame.r12,
            r11: parent_frame.r11,
            r10: parent_frame.r10,
            r9: parent_frame.r9,
            r8: parent_frame.r8,
            rbp: parent_frame.rbp,
            rdi: parent_frame.rdi,
            rsi: parent_frame.rsi,
            rdx: parent_frame.rdx,
            rcx: parent_frame.rcx,
            rbx: parent_frame.rbx,
            rax: 0, // child gets 0 from fork
            vector: USER_TASK_VECTOR,
            error_code: 0,
            rip: parent_frame.rip,
            cs: USER_CS,
            rflags: USER_RFLAGS,
            rsp: parent_frame.rsp,
            ss: USER_DS,
        });
    }

    // Build child Task and add to scheduler.
    let child_id = match crate::sched::alloc_id() {
        Some(id) => id,
        None => return ENOMEM.wrapping_neg(),
    };

    let child_task = Task {
        id: child_id,
        state: TaskState::Ready,
        saved_rsp: child_frame_ptr as u64,
        stack: TaskStack::from_raw(kstack_base, kernel_stack_pages),
        name: "forked",
        aspace: Some(child_aspace),
        kstack_top: kstack_top.as_u64(),
        fd_table: Some(child_fdt),
        parent: Some(parent_id),
        exit_code: None,
    };

    crate::sched::add_task(child_task);

    // Parent gets the child's PID.
    child_id.0
}

// ---------------------------------------------------------------------------
// exec(path)
// ---------------------------------------------------------------------------

fn do_exec(frame: &mut SyscallFrame) {
    use crate::elf::loader;
    use crate::elf::parser::Elf64Info;
    use crate::fs::traits::OpenFlags;

    let path_ptr = frame.rdi as usize;

    // Copy path from user space.
    let path = match crate::syscall::dispatch::copy_path_from_user(path_ptr) {
        Ok(p) => p,
        Err(_) => {
            frame.rax = EINVAL.wrapping_neg();
            return;
        }
    };

    // Read the entire file from VFS.
    let file_data = match read_file_from_vfs(path.as_str()) {
        Ok(data) => data,
        Err(_) => {
            frame.rax = ENOENT.wrapping_neg();
            return;
        }
    };

    // Parse ELF.
    let elf = match Elf64Info::parse(file_data.as_slice()) {
        Ok(e) => e,
        Err(_) => {
            frame.rax = ENOEXEC.wrapping_neg();
            return;
        }
    };

    // Load into new address space.
    let loaded = match loader::load_elf(&elf) {
        Ok(l) => l,
        Err(_) => {
            frame.rax = ENOMEM.wrapping_neg();
            return;
        }
    };

    // Close close-on-exec FDs and swap address space.
    {
        let result = crate::sched::with_current_task(|task| {
            // Close cloexec FDs.
            if let Some(ref mut fdt) = task.fd_table {
                let handles = fdt.close_cloexec();
                for h in handles {
                    let _ = crate::fs::vfs::close(h);
                }
            }

            // Replace address space (old one is dropped, freeing pages).
            task.aspace = Some(loaded.aspace);

            Ok(())
        });
        if result.is_err() {
            frame.rax = ENOMEM.wrapping_neg();
            return;
        }
    }

    // Activate the new address space.
    let new_cr3 = crate::sched::with_current_task(|task| {
        Ok(task.aspace.as_ref().map(|a| a.cr3().as_u64()).unwrap_or(0))
    })
    .unwrap_or(0);
    if new_cr3 != 0 {
        unsafe {
            core::arch::asm!("mov cr3, {}", in(reg) new_cr3, options(nomem, nostack));
        }
    }

    // Update the frame to jump to the new entry point.
    frame.rip = loaded.entry_point;
    frame.rsp = loaded.stack_top;
    frame.rflags = USER_RFLAGS;
    frame.cs = USER_CS;
    frame.ss = USER_DS;
    frame.rax = 0;
    // Clear all GPRs for a clean environment.
    frame.rbx = 0;
    frame.rcx = loaded.entry_point; // RCX = RIP for sysret path
    frame.rdx = 0;
    frame.rsi = 0;
    frame.rdi = 0;
    frame.rbp = 0;
    frame.r8 = 0;
    frame.r9 = 0;
    frame.r10 = 0;
    frame.r11 = USER_RFLAGS; // R11 = RFLAGS for sysret path
    frame.r12 = 0;
    frame.r13 = 0;
    frame.r14 = 0;
    frame.r15 = 0;
}

/// Read an entire file into a Vec<u8> via the VFS.
fn read_file_from_vfs(path: &str) -> Result<alloc::vec::Vec<u8>, crate::fs::traits::FsError> {
    use crate::fs::traits::OpenFlags;

    let handle = crate::fs::vfs::open(
        path,
        OpenFlags {
            read: true,
            write: false,
            create: false,
            truncate: false,
            append: false,
        },
    )?;

    // Get file size via stat.
    let stat = crate::fs::vfs::stat(path)?;
    let size = stat.size as usize;

    let mut data = alloc::vec![0u8; size];
    let mut offset = 0;
    while offset < size {
        let n = crate::fs::vfs::read(handle, &mut data[offset..])?;
        if n == 0 {
            break;
        }
        offset += n;
    }
    crate::fs::vfs::close(handle)?;

    Ok(data)
}
