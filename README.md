# Eithne

An x86_64 operating system kernel written in Rust, booting via UEFI firmware. Implements the full low-level foundation: memory management, interrupt handling, virtual memory, and a preemptive round-robin scheduler with assembly context switching.

```
[eithne] kernel booting...          [mm]  buddy allocator ready — 20535 free pages (80 MiB)
[gdt]   loaded — CS=0x8 TSS=0x28   [idt] loaded at 0x605d2b0
[pic]   IRQ0 (timer) unmasked       [sched] spawned task TaskId(2) 'heartbeat'
[eithne] initialization complete — entering scheduler loop.
```

---

## Boot & Initialization Sequence

```mermaid
flowchart TD
    FW["UEFI Firmware"]
    EP["efi_main()<br/>EFI Application Entry"]
    EARLYINIT["arch::early_init()<br/>COM1 serial at 115200 baud"]
    EBS["exit_boot_services()<br/>take hardware ownership"]
    KM["kernel_main(boot_info, initramfs, fb)"]

    MM["mm::init()<br/>parse UEFI memory map<br/>build buddy frame allocator"]
    ARCH["arch::init_interrupts()<br/>GDT · TSS · IDT (256 vectors)<br/>8259 PIC remap → vectors 32-47"]
    PAGING["paging::init_higher_half()<br/>4-level page tables<br/>PML4 → PDPT → PD → PT"]
    SCHED["sched::init()<br/>round-robin scheduler<br/>spawn idle + heartbeat tasks"]
    LOOP["scheduler_loop()<br/>IRQ0 timer preemption<br/>10-tick timeslice"]

    FW --> EP --> EARLYINIT --> EBS --> KM
    KM --> MM --> ARCH --> PAGING --> SCHED --> LOOP
```

---

## Memory Subsystem

```mermaid
flowchart LR
    subgraph BuddyAllocator["Buddy Frame Allocator (O(1))"]
        FL["Free Lists<br/>orders 0–10<br/>4 KiB → 2 MiB"]
        BM["XOR Bitmap<br/>buddy = addr ⊕ (1 << order)<br/>O(1) coalesce check"]
    end

    subgraph SlabHeap["Slab Heap (#[global_allocator])"]
        SC["9 size classes<br/>8 · 16 · 32 · 64 · 128<br/>256 · 512 · 1024 · 2048 B"]
        DIRECT["Large allocs → buddy frames directly"]
    end

    subgraph VirtualMemory["Virtual Memory"]
        PT["4-Level Page Tables<br/>PML4[47:39] → PDPT[38:30]<br/>PD[29:21] → PT[20:12]"]
        INVLPG["INVLPG + huge page walk<br/>map / unmap / translate"]
    end

    UEFI["UEFI Memory Map"] --> BuddyAllocator
    BuddyAllocator --> SlabHeap
    SlabHeap --> VirtualMemory
```

**Buddy allocator:** Allocation pops from the appropriate order's free list, splitting down if needed. Deallocation coalesces with the buddy (found via XOR) recursively up to `MAX_ORDER`. All O(1) amortized.

**Slab heap:** Each size class maintains an intrusive free list stored directly in the free slots. Allocations under 2,048 bytes never touch the buddy allocator — O(1) with no fragmentation.

---

## Interrupt & Exception Handling

256-entry IDT with hand-written assembly stubs for every vector:

| Vectors | Purpose |
|---|---|
| 0–31 | CPU exceptions — divide error, page fault, double fault, GPF, … |
| 32–47 | 8259 PIC IRQs (remapped from legacy 8–15) |
| 48–255 | Reserved |

Every exception stub pushes a full `InterruptFrame` (15 GPRs + vector + error code + CPU-pushed RIP/CS/RFLAGS/RSP/SS) and prints a register dump to COM1. Page faults additionally decode CR2.

**IST stacks:** Double-fault and NMI use dedicated IST stacks from the TSS — a GPF inside the exception handler won't triple-fault.

---

## Scheduler

Cooperative + preemptive round-robin. The IRQ0 timer fires at ~18.2 Hz; after 10 ticks the current task is preempted.

```mermaid
stateDiagram-v2
    [*] --> Ready : spawn
    Ready --> Running : scheduler selects
    Running --> Ready : yield or timer preempt
    Running --> Blocked : wait for event
    Blocked --> Ready : event fires
    Running --> Dead : task returns
    Dead --> [*] : waitpid
```

**Context switch** (`switch_context` in assembly): saves callee-saved registers (RBX, RBP, R12–R15) and RSP onto the current task's kernel stack; loads the next task's RSP and pops its saved state. `iretq` restores RIP, CS, RFLAGS, RSP, SS — returning into the new task wherever it last yielded or was preempted.

Each task gets a **64 KiB kernel stack** allocated from the buddy allocator.

---

## Synchronization Primitives

| Type | Mechanism |
|---|---|
| `Spinlock<T>` | `AtomicBool` test-and-set, busy-wait |
| `IrqSpinlock<T>` | Spinlock + save/restore `RFLAGS.IF` — safe across interrupt boundaries |
| `Once<T>` | `AtomicU8` state machine (uninit → initializing → initialized) — one-time global init |

---

The `ArchPlatform` trait abstracts early init, interrupt control, serial output, and halt — x86_64 is fully implemented; aarch64 and riscv64 have typed stubs for future porting.

---

## Building & Running

**Requirements:** Rust nightly (pinned via `rust-toolchain.toml`) · QEMU · OVMF (`edk2-ovmf`)

```bash
cargo build              # compile UEFI kernel binary
./run.sh                 # assemble ESP image and boot in QEMU
```

Press `Ctrl-B X` to exit QEMU.
