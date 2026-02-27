//! User-mode process creation.
//!
//! Creates an x86_64 ring-3 task with its own page table, a user stack, and
//! a small inline machine-code program that exercises the syscall interface.

use crate::arch::x86_64::paging::KernelAddressSpace;
use crate::mm::addr::{PhysAddr, VirtAddr, phys_to_virt};
use crate::mm::frame;
use crate::mm::page::PAGE_SIZE;
use crate::mm::traits::{AddressSpace, MapError, PageFlags, PageMapper};

use super::context::InterruptFrame;
use super::process::{Task, TaskState};
use super::traits::TaskId;

// ---------------------------------------------------------------------------
// Virtual address layout for the user process
// ---------------------------------------------------------------------------

/// Offset from the dynamic PML4 base for user code.
const USER_CODE_OFFSET: u64 = 0x0040_0000; // 4 MiB into the slot

/// Offset from the dynamic PML4 base for user stack top.
const USER_STACK_OFFSET: u64 = 0x0080_0000; // 8 MiB into the slot
const USER_STACK_PAGES: usize = 4; // 16 KiB

// ---------------------------------------------------------------------------
// Segment selectors (must match gdt.rs)
// ---------------------------------------------------------------------------

const USER_CS: u64 = 0x18 | 3; // DPL=3
const USER_DS: u64 = 0x20 | 3; // DPL=3
const USER_RFLAGS: u64 = 0x202; // IF=1, reserved=1

// ---------------------------------------------------------------------------
// Inline user program
// ---------------------------------------------------------------------------
//
// The program:
//   1. Writes "Hello from user mode!\n" via sys_write(1, msg, len)
//   2. Calls sys_exit(0)
//
// x86_64 Linux syscall ABI:
//   RAX = syscall number
//   RDI, RSI, RDX, R10, R8, R9 = args
//   syscall instruction
//
// Assembled by hand; no external toolchain dependency.

static USER_PROGRAM: &[u8] = &[
    // --- write syscall ---
    // mov eax, 1              ; SYS_WRITE
    0xB8, 0x01, 0x00, 0x00, 0x00, // mov edi, 1              ; fd = stdout
    0xBF, 0x01, 0x00, 0x00, 0x00,
    // lea rsi, [rip + msg]    ; buf ptr  (msg is 18 bytes after this instruction's end)
    // RIP after this instruction = USER_CODE_BASE + 15
    // msg offset from that RIP = 16 (to the mov edx below: 5 bytes, syscall: 2 bytes, exit: 7 bytes, msg: right after)
    // We'll compute the RIP-relative offset at build time:
    // After lea (7 bytes), mov edx (5), syscall (2), mov eax (5), xor edi (2), syscall (2) = 21 bytes to msg
    // RIP after lea = base + 5 + 5 + 7 = base + 17
    // msg is at base + 17 + 5 + 2 + 5 + 2 + 2 = base + 33
    // offset = 33 - 17 = 16
    0x48, 0x8D, 0x35, 0x10, 0x00, 0x00, 0x00, // mov edx, 22             ; len = 22
    0xBA, 0x16, 0x00, 0x00, 0x00, // syscall
    0x0F, 0x05, // --- exit syscall ---
    // mov eax, 60             ; SYS_EXIT
    0xB8, 0x3C, 0x00, 0x00, 0x00, // xor edi, edi            ; code = 0
    0x31, 0xFF, // syscall
    0x0F, 0x05, // --- message data ---
    b'H', b'e', b'l', b'l', b'o', b' ', b'f', b'r', b'o', b'm', b' ', b'u', b's', b'e', b'r', b' ',
    b'm', b'o', b'd', b'e', b'!', b'\n',
];

// ---------------------------------------------------------------------------
// Page table creation
// ---------------------------------------------------------------------------

/// Build a user address space.
///
/// Copies all kernel page table entries, then finds an empty PML4 slot in
/// the lower canonical half and maps user code + stack there.
///
/// Returns `(address_space, code_base, stack_top)`.
fn build_user_aspace(
    code_phys: PhysAddr,
    stack_bottom_phys: PhysAddr,
) -> Result<(KernelAddressSpace, u64, u64), MapError> {
    let mut aspace = KernelAddressSpace::new_with_kernel_mappings()?;

    let user_base = aspace
        .find_free_user_base()
        .ok_or(MapError::FrameAllocationFailed)?;
    let code_base = user_base + USER_CODE_OFFSET;
    let stack_top = user_base + USER_STACK_OFFSET;

    // Map user code page (RX, user-accessible, one page).
    aspace.map(VirtAddr::new(code_base), code_phys, PageFlags::USER_RX)?;

    // Map user stack pages (RW, user-accessible).
    for i in 0..USER_STACK_PAGES {
        let page_phys = stack_bottom_phys.offset((i * PAGE_SIZE) as u64);
        let page_virt = VirtAddr::new(stack_top - ((USER_STACK_PAGES - i) * PAGE_SIZE) as u64);
        aspace.map(page_virt, page_phys, PageFlags::USER_RW)?;
    }

    Ok((aspace, code_base, stack_top))
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Allocate a user-mode task that runs the built-in user program.
///
/// The task starts in ring 3 at `USER_CODE_BASE` with a private page table
/// that shares the kernel's upper-half direct map.
///
/// Returns `None` on OOM.
pub fn create_user_task(id: TaskId) -> Option<Task> {
    // 1. Allocate a physical frame for user code and copy the program in.
    let code_phys = frame::alloc_frame()?;
    let code_virt = phys_to_virt(code_phys);
    unsafe {
        let dst = core::slice::from_raw_parts_mut(code_virt.as_mut_ptr::<u8>(), PAGE_SIZE);
        dst[..USER_PROGRAM.len()].copy_from_slice(USER_PROGRAM);
        // Zero the rest of the page.
        dst[USER_PROGRAM.len()..].fill(0);
    }

    // 2. Allocate user stack frames.
    let stack_bottom_phys = frame::alloc_pages(crate::mm::page::pages_to_order(USER_STACK_PAGES))?;

    // 3. Build the user address space.
    let (aspace, user_code_base, user_stack_top) = build_user_aspace(code_phys, stack_bottom_phys)
        .expect("[user] failed to build user address space");

    // 4. Allocate a kernel stack for this task (used during syscall/interrupt).
    let kernel_stack_pages = super::context::TaskStack::STACK_PAGES;
    let kstack_phys = frame::alloc_pages(crate::mm::page::pages_to_order(kernel_stack_pages))?;
    let kstack_base = phys_to_virt(kstack_phys);
    let kstack_top = kstack_base.offset((kernel_stack_pages * PAGE_SIZE) as u64);

    // 5. Write the initial InterruptFrame at the top of the kernel stack.
    //    iretq will land in ring 3 at the user code base with the user RSP.
    let frame_ptr = (kstack_top.as_u64() - core::mem::size_of::<InterruptFrame>() as u64)
        as *mut InterruptFrame;
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
            vector: 0xFD, // user task pseudo-vector
            error_code: 0,
            rip: user_code_base,
            cs: USER_CS,
            rflags: USER_RFLAGS,
            rsp: user_stack_top,
            ss: USER_DS,
        });
    }

    // 6. Build the Task.
    Some(Task {
        id,
        state: TaskState::Ready,
        saved_rsp: frame_ptr as u64,
        stack: super::context::TaskStack::from_raw(kstack_base, kernel_stack_pages),
        name: "user_init",
        aspace: Some(aspace),
        kstack_top: kstack_top.as_u64(),
    })
}
