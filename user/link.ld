/* Linker script for eithne user-mode PIE program.
 *
 * Links at virtual address 0 (PIE base). The kernel ELF loader picks the
 * actual load address and applies R_X86_64_RELATIVE relocations to fix up
 * the GOT and other absolute references.
 */

OUTPUT_FORMAT("elf64-x86-64")
OUTPUT_ARCH(i386:x86-64)
ENTRY(_start)

SECTIONS
{
    . = 0;

    .text ALIGN(4K) : {
        *(.text .text.*)
    }

    .rodata ALIGN(4K) : {
        *(.rodata .rodata.*)
    }

    .dynsym ALIGN(8) : { *(.dynsym) }
    .dynstr ALIGN(8) : { *(.dynstr) }
    .hash ALIGN(8) : { *(.hash) }
    .gnu.hash ALIGN(8) : { *(.gnu.hash) }

    .rela.dyn ALIGN(8) : { *(.rela.dyn) }

    .dynamic ALIGN(8) : { *(.dynamic) }

    .got ALIGN(8) : { *(.got .got.plt) }

    .data ALIGN(4K) : {
        *(.data .data.*)
    }

    .bss ALIGN(4K) : {
        *(.bss .bss.*)
        *(COMMON)
    }

    /DISCARD/ : {
        *(.comment)
        *(.note*)
        *(.eh_frame*)
    }
}
