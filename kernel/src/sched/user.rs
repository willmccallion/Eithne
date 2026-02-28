//! User-mode process creation from ELF binaries.
//!
//! Parses an ELF64 binary, maps its segments into a new user address space,
//! and creates a ring-3 task.

use crate::elf::loader;
use crate::elf::parser::Elf64Info;
use crate::mm::addr::phys_to_virt;
use crate::mm::frame;
use crate::mm::page::PAGE_SIZE;

use super::context::InterruptFrame;
use super::process::{Task, TaskState};
use super::traits::TaskId;

// ---------------------------------------------------------------------------
// Segment selectors (must match gdt.rs)
// ---------------------------------------------------------------------------

const USER_CS: u64 = 0x18 | 3; // DPL=3
const USER_DS: u64 = 0x20 | 3; // DPL=3
const USER_RFLAGS: u64 = 0x202; // IF=1, reserved=1

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Create a user-mode task from an ELF binary.
///
/// Parses the ELF, loads segments into a new address space, and builds a
/// task ready to run in ring 3.
///
/// Returns `None` on parse/load failure or OOM.
pub fn create_user_task_from_elf(id: TaskId, name: &'static str, elf_data: &[u8]) -> Option<Task> {
    // 1. Parse ELF.
    let elf = match Elf64Info::parse(elf_data) {
        Ok(e) => e,
        Err(e) => {
            crate::println!("[user] ELF parse error: {:?}", e);
            return None;
        }
    };

    // 2. Load segments into a new address space.
    let loaded = match loader::load_elf(&elf) {
        Ok(l) => l,
        Err(e) => {
            crate::println!("[user] ELF load error: {:?}", e);
            return None;
        }
    };

    // 3. Allocate a kernel stack for this task (used during syscall/interrupt).
    let kernel_stack_pages = super::context::TaskStack::STACK_PAGES;
    let kstack_phys = frame::alloc_pages(crate::mm::page::pages_to_order(kernel_stack_pages))?;
    let kstack_base = phys_to_virt(kstack_phys);
    let kstack_top = kstack_base.offset((kernel_stack_pages * PAGE_SIZE) as u64);

    // 4. Write the initial InterruptFrame at the top of the kernel stack.
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
            rip: loaded.entry_point,
            cs: USER_CS,
            rflags: USER_RFLAGS,
            rsp: loaded.stack_top,
            ss: USER_DS,
        });
    }

    crate::println!(
        "[user] ELF loaded: entry={:#x} stack_top={:#x}",
        loaded.entry_point,
        loaded.stack_top
    );

    // 5. Build the Task.
    Some(Task {
        id,
        state: TaskState::Ready,
        saved_rsp: frame_ptr as u64,
        stack: super::context::TaskStack::from_raw(kstack_base, kernel_stack_pages),
        name,
        aspace: Some(loaded.aspace),
        kstack_top: kstack_top.as_u64(),
        fd_table: Some(super::fd::FdTable::new_with_stdio()),
        parent: None,
        exit_code: None,
    })
}
