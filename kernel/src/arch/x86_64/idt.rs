//! Interrupt Descriptor Table for x86_64.
//!
//! Maps interrupt vectors (0-255) to handler functions.
//! Vectors 0-31 are CPU exceptions, 32-47 are PIC IRQs.

use core::arch::global_asm;
use core::cell::UnsafeCell;

use super::gdt;

const IDT_ENTRIES: usize = 256;

// ---------------------------------------------------------------------------
// Exception / IRQ vector numbers
// ---------------------------------------------------------------------------

/// CPU exception vectors (0-31) and PIC IRQ base.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Vector {
    DivideError = 0,
    Debug = 1,
    Nmi = 2,
    Breakpoint = 3,
    Overflow = 4,
    BoundRange = 5,
    InvalidOpcode = 6,
    DeviceNotAvail = 7,
    DoubleFault = 8,
    // 9 = legacy coprocessor segment overrun
    InvalidTss = 10,
    SegmentNotPresent = 11,
    StackSegment = 12,
    GeneralProtection = 13,
    PageFault = 14,
    // 15 = reserved
    X87Fp = 16,
    AlignmentCheck = 17,
    MachineCheck = 18,
    SimdFp = 19,
    // 20 = #VE, 21 = #CP, 28 = #HV, 29 = #VC, 30 = #SX
}

/// First vector used by PIC IRQs (IRQ0 = timer).
const IRQ_BASE: u8 = 32;
/// Last PIC IRQ vector (IRQ15).
const IRQ_END: u8 = 47;

// ---------------------------------------------------------------------------
// IDT entry
// ---------------------------------------------------------------------------

/// A single IDT gate descriptor (16 bytes in 64-bit mode).
#[derive(Clone, Copy)]
#[repr(C, packed)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    _reserved: u32,
}

impl IdtEntry {
    const fn empty() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            ist: 0,
            type_attr: 0,
            offset_mid: 0,
            offset_high: 0,
            _reserved: 0,
        }
    }

    /// Interrupt gate (type 0xE) — clears IF on entry.
    fn interrupt_gate(handler: u64, selector: u16, ist_index: u8, dpl: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            ist: ist_index & 0x7,
            type_attr: 0x80 | ((dpl & 0x3) << 5) | 0x0E,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }

    /// Trap gate (type 0xF) — does NOT clear IF. Used for #BP, #OF.
    fn trap_gate(handler: u64, selector: u16, ist_index: u8, dpl: u8) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            ist: ist_index & 0x7,
            type_attr: 0x80 | ((dpl & 0x3) << 5) | 0x0F,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _reserved: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Interrupt frame
// ---------------------------------------------------------------------------

/// Full register state on the stack during an interrupt.
/// Layout matches the push order in the assembly stubs.
#[repr(C)]
pub struct InterruptFrame {
    // Pushed by our stub (15 GPRs):
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
    // Pushed by stub: vector number and error code
    pub vector: u64,
    pub error_code: u64,
    // Pushed by CPU:
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

// ---------------------------------------------------------------------------
// Assembly stubs
// ---------------------------------------------------------------------------

/// Generate a stub for vectors where the CPU does NOT push an error code.
macro_rules! isr_no_err {
    ($vec:literal) => {
        concat!(
            ".align 16\n",
            "isr_stub_",
            stringify!($vec),
            ":\n",
            "push 0\n",
            "push ",
            stringify!($vec),
            "\n",
            "jmp isr_common\n",
        )
    };
}

/// Generate a stub for vectors where the CPU pushes an error code.
macro_rules! isr_err {
    ($vec:literal) => {
        concat!(
            ".align 16\n",
            "isr_stub_",
            stringify!($vec),
            ":\n",
            "push ",
            stringify!($vec),
            "\n",
            "jmp isr_common\n",
        )
    };
}

// Common handler: save registers, call Rust dispatcher, check for reschedule,
// restore registers (or switch to new task), iretq.
// Uses MS x64 ABI: first arg in RCX, 32-byte shadow space required.
global_asm!(
    ".align 16",
    "isr_common:",
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
    // RCX = &InterruptFrame (first MS ABI arg)
    "mov rcx, rsp",
    // Call the Rust interrupt dispatcher
    "sub rsp, 32",       // shadow space
    "call {dispatch}",
    "add rsp, 32",
    // --- Preemption check ---
    // Call check_resched(interrupted_rsp=rsp) → returns new_rsp or 0
    "mov rcx, rsp",      // arg: current RSP = top of InterruptFrame
    "sub rsp, 32",
    "call {check_resched}",
    "add rsp, 32",
    // RAX = new_rsp (0 = no switch)
    "test rax, rax",
    "jz .Lno_switch",
    // Switch: load new task's RSP and resume it via sched_iretq_exit
    "mov rsp, rax",
    "jmp sched_iretq_exit",
    ".Lno_switch:",
    // No switch: restore current task's registers and iretq
    "pop r15", "pop r14", "pop r13", "pop r12",
    "pop r11", "pop r10", "pop r9",  "pop r8",
    "pop rbp", "pop rdi", "pop rsi", "pop rdx",
    "pop rcx", "pop rbx", "pop rax",
    "add rsp, 16",       // skip vector + error_code
    "iretq",
    dispatch       = sym interrupt_dispatch,
    check_resched  = sym crate::sched::check_resched,
);

// Exception stubs (vectors 0-31)
global_asm!(
    isr_no_err!(0),  // #DE
    isr_no_err!(1),  // #DB
    isr_no_err!(2),  // #NMI
    isr_no_err!(3),  // #BP
    isr_no_err!(4),  // #OF
    isr_no_err!(5),  // #BR
    isr_no_err!(6),  // #UD
    isr_no_err!(7),  // #NM
    isr_err!(8),     // #DF
    isr_no_err!(9),  // coprocessor segment overrun
    isr_err!(10),    // #TS
    isr_err!(11),    // #NP
    isr_err!(12),    // #SS
    isr_err!(13),    // #GP
    isr_err!(14),    // #PF
    isr_no_err!(15), // reserved
    isr_no_err!(16), // #MF
    isr_err!(17),    // #AC
    isr_no_err!(18), // #MC
    isr_no_err!(19), // #XM
    isr_no_err!(20), // #VE
    isr_err!(21),    // #CP
    isr_no_err!(22),
    isr_no_err!(23),
    isr_no_err!(24),
    isr_no_err!(25),
    isr_no_err!(26),
    isr_no_err!(27),
    isr_no_err!(28), // #HV
    isr_err!(29),    // #VC
    isr_err!(30),    // #SX
    isr_no_err!(31), // reserved
);

// PIC IRQ stubs (vectors 32-47)
global_asm!(
    isr_no_err!(32),
    isr_no_err!(33),
    isr_no_err!(34),
    isr_no_err!(35),
    isr_no_err!(36),
    isr_no_err!(37),
    isr_no_err!(38),
    isr_no_err!(39),
    isr_no_err!(40),
    isr_no_err!(41),
    isr_no_err!(42),
    isr_no_err!(43),
    isr_no_err!(44),
    isr_no_err!(45),
    isr_no_err!(46),
    isr_no_err!(47),
);

// Catch-all for vectors 48-255
global_asm!(
    ".align 16",
    "isr_stub_unhandled:",
    "push 0",
    "push 0xFF",
    "jmp isr_common",
);

// Import stub addresses into Rust.
unsafe extern "C" {
    fn isr_stub_0();
    fn isr_stub_1();
    fn isr_stub_2();
    fn isr_stub_3();
    fn isr_stub_4();
    fn isr_stub_5();
    fn isr_stub_6();
    fn isr_stub_7();
    fn isr_stub_8();
    fn isr_stub_9();
    fn isr_stub_10();
    fn isr_stub_11();
    fn isr_stub_12();
    fn isr_stub_13();
    fn isr_stub_14();
    fn isr_stub_15();
    fn isr_stub_16();
    fn isr_stub_17();
    fn isr_stub_18();
    fn isr_stub_19();
    fn isr_stub_20();
    fn isr_stub_21();
    fn isr_stub_22();
    fn isr_stub_23();
    fn isr_stub_24();
    fn isr_stub_25();
    fn isr_stub_26();
    fn isr_stub_27();
    fn isr_stub_28();
    fn isr_stub_29();
    fn isr_stub_30();
    fn isr_stub_31();
    fn isr_stub_32();
    fn isr_stub_33();
    fn isr_stub_34();
    fn isr_stub_35();
    fn isr_stub_36();
    fn isr_stub_37();
    fn isr_stub_38();
    fn isr_stub_39();
    fn isr_stub_40();
    fn isr_stub_41();
    fn isr_stub_42();
    fn isr_stub_43();
    fn isr_stub_44();
    fn isr_stub_45();
    fn isr_stub_46();
    fn isr_stub_47();
    fn isr_stub_unhandled();
}

/// Convert a function pointer to a u64 address.
fn stub_addr(f: unsafe extern "C" fn()) -> u64 {
    f as *const () as u64
}

/// Build the table of stub addresses, indexed by vector number.
fn isr_stub_table() -> [u64; IDT_ENTRIES] {
    let mut table = [stub_addr(isr_stub_unhandled); IDT_ENTRIES];
    table[0] = stub_addr(isr_stub_0);
    table[1] = stub_addr(isr_stub_1);
    table[2] = stub_addr(isr_stub_2);
    table[3] = stub_addr(isr_stub_3);
    table[4] = stub_addr(isr_stub_4);
    table[5] = stub_addr(isr_stub_5);
    table[6] = stub_addr(isr_stub_6);
    table[7] = stub_addr(isr_stub_7);
    table[8] = stub_addr(isr_stub_8);
    table[9] = stub_addr(isr_stub_9);
    table[10] = stub_addr(isr_stub_10);
    table[11] = stub_addr(isr_stub_11);
    table[12] = stub_addr(isr_stub_12);
    table[13] = stub_addr(isr_stub_13);
    table[14] = stub_addr(isr_stub_14);
    table[15] = stub_addr(isr_stub_15);
    table[16] = stub_addr(isr_stub_16);
    table[17] = stub_addr(isr_stub_17);
    table[18] = stub_addr(isr_stub_18);
    table[19] = stub_addr(isr_stub_19);
    table[20] = stub_addr(isr_stub_20);
    table[21] = stub_addr(isr_stub_21);
    table[22] = stub_addr(isr_stub_22);
    table[23] = stub_addr(isr_stub_23);
    table[24] = stub_addr(isr_stub_24);
    table[25] = stub_addr(isr_stub_25);
    table[26] = stub_addr(isr_stub_26);
    table[27] = stub_addr(isr_stub_27);
    table[28] = stub_addr(isr_stub_28);
    table[29] = stub_addr(isr_stub_29);
    table[30] = stub_addr(isr_stub_30);
    table[31] = stub_addr(isr_stub_31);
    table[32] = stub_addr(isr_stub_32);
    table[33] = stub_addr(isr_stub_33);
    table[34] = stub_addr(isr_stub_34);
    table[35] = stub_addr(isr_stub_35);
    table[36] = stub_addr(isr_stub_36);
    table[37] = stub_addr(isr_stub_37);
    table[38] = stub_addr(isr_stub_38);
    table[39] = stub_addr(isr_stub_39);
    table[40] = stub_addr(isr_stub_40);
    table[41] = stub_addr(isr_stub_41);
    table[42] = stub_addr(isr_stub_42);
    table[43] = stub_addr(isr_stub_43);
    table[44] = stub_addr(isr_stub_44);
    table[45] = stub_addr(isr_stub_45);
    table[46] = stub_addr(isr_stub_46);
    table[47] = stub_addr(isr_stub_47);
    table
}

// ---------------------------------------------------------------------------
// IDT storage
// ---------------------------------------------------------------------------

#[repr(C, align(16))]
struct IdtStorage {
    entries: UnsafeCell<[IdtEntry; IDT_ENTRIES]>,
}
unsafe impl Sync for IdtStorage {}

static IDT_STORAGE: IdtStorage = IdtStorage {
    entries: UnsafeCell::new([IdtEntry::empty(); IDT_ENTRIES]),
};

#[repr(C, packed)]
struct IdtPointer {
    limit: u16,
    base: u64,
}

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

pub fn init() {
    crate::println!("[idt] initializing ({} entries)", IDT_ENTRIES);

    let entries = unsafe { &mut *IDT_STORAGE.entries.get() };
    let stubs = isr_stub_table();

    for i in 0..IDT_ENTRIES {
        let v = i as u8;

        let ist = match v {
            v if v == Vector::DoubleFault as u8 => 1, // IST1
            v if v == Vector::Nmi as u8 => 2,         // IST2
            _ => 0,
        };

        let dpl: u8 = match v {
            v if v == Vector::Breakpoint as u8 => 3, // allow int3 from ring 3
            _ => 0,
        };

        entries[i] = match v {
            // Trap gates for #BP and #OF (don't clear IF)
            v if v == Vector::Breakpoint as u8 || v == Vector::Overflow as u8 => {
                IdtEntry::trap_gate(stubs[i], gdt::KERNEL_CS, ist, dpl)
            }
            _ => IdtEntry::interrupt_gate(stubs[i], gdt::KERNEL_CS, ist, dpl),
        };
    }

    let idtr = IdtPointer {
        limit: (IDT_ENTRIES * core::mem::size_of::<IdtEntry>() - 1) as u16,
        base: entries.as_ptr() as u64,
    };

    unsafe {
        core::arch::asm!("lidt [{}]", in(reg) &idtr, options(nostack));
    }

    crate::println!("[idt] loaded at {:#x}", entries.as_ptr() as u64);
}

// ---------------------------------------------------------------------------
// Rust interrupt dispatcher
// ---------------------------------------------------------------------------

#[unsafe(no_mangle)]
extern "C" fn interrupt_dispatch(frame: &InterruptFrame) {
    let v = frame.vector as u8;

    use Vector::*;
    match v {
        v if v == DivideError as u8 => exception_halt("Divide Error (#DE)", frame),
        v if v == Debug as u8 => exception_halt("Debug (#DB)", frame),
        v if v == Nmi as u8 => exception_halt("NMI", frame),
        v if v == Breakpoint as u8 => {
            crate::println!("[int] breakpoint at RIP={:#x}", frame.rip);
        }
        v if v == Overflow as u8 => {
            crate::println!("[int] overflow at RIP={:#x}", frame.rip);
        }
        v if v == BoundRange as u8 => exception_halt("Bound Range (#BR)", frame),
        v if v == InvalidOpcode as u8 => exception_halt("Invalid Opcode (#UD)", frame),
        v if v == DeviceNotAvail as u8 => exception_halt("Device Not Available (#NM)", frame),
        v if v == DoubleFault as u8 => {
            panic_dump("Double Fault (#DF)", frame);
        }
        v if v == InvalidTss as u8 => exception_halt("Invalid TSS (#TS)", frame),
        v if v == SegmentNotPresent as u8 => exception_halt("Segment Not Present (#NP)", frame),
        v if v == StackSegment as u8 => exception_halt("Stack-Segment Fault (#SS)", frame),
        v if v == GeneralProtection as u8 => {
            exception_halt("General Protection Fault (#GP)", frame)
        }
        v if v == PageFault as u8 => {
            panic_dump("Page Fault (#PF)", frame);
        }
        v if v == X87Fp as u8 => exception_halt("x87 FP Exception (#MF)", frame),
        v if v == AlignmentCheck as u8 => exception_halt("Alignment Check (#AC)", frame),
        v if v == MachineCheck as u8 => {
            panic_dump("Machine Check (#MC)", frame);
        }
        v if v == SimdFp as u8 => exception_halt("SIMD FP Exception (#XM)", frame),

        // PIC IRQs
        IRQ_BASE..=IRQ_END => {
            super::interrupts::irq_handler(v - IRQ_BASE, frame);
        }

        _ => {
            crate::println!("[int] unhandled vector {}", frame.vector);
        }
    }
}

fn exception_halt(name: &str, frame: &InterruptFrame) -> ! {
    panic_dump(name, frame);
}

/// Print a full register dump and halt.  This is the kernel's equivalent of a
/// Linux kernel panic / oops — it gives you enough state to diagnose the fault
/// instead of silently triple-faulting.
fn panic_dump(name: &str, frame: &InterruptFrame) -> ! {
    // Read CR2 and CR3 while we still can.
    let cr2: u64;
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr2", out(reg) cr2, options(nomem, nostack));
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
    }

    crate::println!();
    crate::println!("========================================");
    crate::println!("  KERNEL PANIC: {}", name);
    crate::println!("========================================");
    crate::println!(
        "  vector={:#x}  error_code={:#x}",
        frame.vector,
        frame.error_code
    );
    crate::println!();
    crate::println!("  RIP={:#018x}  RSP={:#018x}", frame.rip, frame.rsp);
    crate::println!(
        "  CS={:#06x}  SS={:#06x}  RFLAGS={:#018x}",
        frame.cs,
        frame.ss,
        frame.rflags
    );
    crate::println!("  CR2={:#018x}  CR3={:#018x}", cr2, cr3);
    crate::println!();
    crate::println!("  RAX={:#018x}  RBX={:#018x}", frame.rax, frame.rbx);
    crate::println!("  RCX={:#018x}  RDX={:#018x}", frame.rcx, frame.rdx);
    crate::println!("  RSI={:#018x}  RDI={:#018x}", frame.rsi, frame.rdi);
    crate::println!("  RBP={:#018x}  R8 ={:#018x}", frame.rbp, frame.r8);
    crate::println!("  R9 ={:#018x}  R10={:#018x}", frame.r9, frame.r10);
    crate::println!("  R11={:#018x}  R12={:#018x}", frame.r11, frame.r12);
    crate::println!("  R13={:#018x}  R14={:#018x}", frame.r13, frame.r14);
    crate::println!("  R15={:#018x}", frame.r15);

    // Decode page-fault error code bits.
    if frame.vector == Vector::PageFault as u64 {
        let e = frame.error_code;
        crate::println!();
        crate::println!(
            "  #PF flags: P={} W={} U={} RSVD={} I/D={}",
            e & 1,
            (e >> 1) & 1,
            (e >> 2) & 1,
            (e >> 3) & 1,
            (e >> 4) & 1,
        );
    }

    crate::println!();
    crate::println!("  --- system halted ---");
    crate::println!();

    loop {
        unsafe { core::arch::asm!("cli; hlt", options(nomem, nostack)) };
    }
}
