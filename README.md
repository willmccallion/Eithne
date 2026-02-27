# Eithne

A minimal operating system kernel written in Rust, targeting x86_64 via UEFI.

## What works

- **UEFI boot** — boots as a standard EFI application, exits boot services, takes ownership of hardware
- **Serial console** — COM1 at 115200 baud for debug output
- **Memory management** — UEFI memory map parsing, O(1) buddy frame allocator with XOR bitmap coalescing, per-size-class slab heap (`#[global_allocator]`)
- **GDT + TSS** — kernel/user code and data segments, TSS with IST stacks for double-fault and NMI isolation
- **IDT** — 256-entry interrupt descriptor table with assembly stubs for all CPU exceptions (with diagnostics: register dump, CR2 for page faults, error code decoding) and PIC IRQs
- **8259 PIC** — remapped to vectors 32-47, PIT timer interrupt running at default ~18.2 Hz
- **Synchronization** — spinlock and once-cell primitives
- **Virtual memory** — x86_64 4-level page tables (PML4→PDPT→PD→PT), `map`/`unmap`/`translate`, `invlpg`, huge page walk, `KernelAddressSpace` implementing `PageMapper` + `AddressSpace`
- **Kernel threads** — cooperative + preemptive round-robin scheduler; kernel stacks allocated from the buddy allocator; assembly `switch_context` saves/restores callee-saved registers; timer IRQ drives preemption (10-tick timeslice); `sched::spawn` / `sched::yield_cpu`

## Architecture

The project is structured for multi-architecture support (x86_64, aarch64, riscv64) behind an `ArchPlatform` trait, though only x86_64 is implemented beyond stubs.

```
kernel/src/
  main.rs          UEFI entry point and kernel_main
  arch/            Platform abstraction + per-arch implementations
    x86_64/        GDT, IDT, PIC, serial, port I/O
  mm/              Frame allocator, slab heap, address types
  sync/            Spinlock, Once<T>
  sched/           Round-robin scheduler, kernel threads, context switch
  drivers/         (stub) Hardware drivers
  fs/              (stub) Filesystem
  syscall/         (stub) System call interface
  ipc/             (stub) Inter-process communication
```

## Building

Requires nightly Rust (pinned via `rust-toolchain.toml`):

```
cargo build
```

## Running

Requires QEMU and OVMF firmware (`edk2-ovmf` package on Arch):

```
./run.sh
```

Press `Ctrl-B X` to exit QEMU.

## Boot output

```
[eithne] kernel booting...
[eithne] architecture: x86_64
[eithne] entering kernel_main
[mm] 9 usable regions, 20536 pages (80 MiB), max_phys=PhysAddr(0x7e6e000)
[mm] buddy allocator ready — 20535 free pages (80 MiB)
[mm] heap test: [1, 2, 3, 4, 5]
[mm] memory subsystem initialized
[gdt] initializing
[gdt] loaded — CS=0x8 DS=0x10 TSS=0x28 IST1=0x7e6e000 IST2=0x671a000
[idt] initializing (256 entries)
[idt] loaded at 0x605d2b0
[pic] remapping to vectors 32-47
[pic] IRQ0 (timer) unmasked
[arch] enabling interrupts
[paging] page table smoke test passed
[sched] round-robin scheduler initialized
[sched] spawned task TaskId(1) 'idle'
[sched] spawned task TaskId(2) 'heartbeat'
[eithne] initialization complete — entering scheduler loop.
[timer] 100 ticks
[heartbeat] still alive (500)
[timer] 200 ticks
...
```
