/* Minimal interactive shell for eithne.
 *
 * Displays a prompt, reads user input one byte at a time, echoes it,
 * and on Enter prints the line back. Handles backspace.
 *
 * Syscalls (Linux x86_64 ABI):
 *   0 = sys_read(fd, buf, len)
 *   1 = sys_write(fd, buf, len)
 *  60 = sys_exit(code)
 */

.equ SYS_READ,  0
.equ SYS_WRITE, 1
.equ SYS_EXIT,  60

.equ FD_STDIN,  0
.equ FD_STDOUT, 1

.equ LINE_MAX,  256

.section .text
.globl _start
_start:
    /* Print welcome banner */
    mov     $SYS_WRITE, %eax
    mov     $FD_STDOUT, %edi
    lea     banner(%rip), %rsi
    mov     $banner_len, %edx
    syscall

prompt_loop:
    /* Print prompt "> " */
    mov     $SYS_WRITE, %eax
    mov     $FD_STDOUT, %edi
    lea     prompt(%rip), %rsi
    mov     $prompt_len, %edx
    syscall

    /* Reset line buffer index */
    xor     %r12d, %r12d           /* r12 = current line length */
    lea     linebuf(%rip), %r13    /* r13 = line buffer base */

read_loop:
    /* Read one byte from stdin */
    mov     $SYS_READ, %eax
    mov     $FD_STDIN, %edi
    lea     charbuf(%rip), %rsi
    mov     $1, %edx
    syscall

    /* Check what we got */
    lea     charbuf(%rip), %rsi
    movzbl  (%rsi), %ecx           /* ecx = the byte */

    /* Handle Enter (CR or LF) */
    cmp     $0x0D, %cl             /* carriage return */
    je      handle_enter
    cmp     $0x0A, %cl             /* line feed */
    je      handle_enter

    /* Handle Backspace (0x7F or 0x08) */
    cmp     $0x7F, %cl
    je      handle_backspace
    cmp     $0x08, %cl
    je      handle_backspace

    /* Normal character — store if room */
    cmp     $LINE_MAX, %r12d
    jge     read_loop              /* line full, drop */

    mov     %cl, (%r13, %r12, 1)  /* linebuf[r12] = ch */
    inc     %r12d

    /* Echo the character */
    mov     $SYS_WRITE, %eax
    mov     $FD_STDOUT, %edi
    lea     charbuf(%rip), %rsi
    mov     $1, %edx
    syscall

    jmp     read_loop

handle_backspace:
    /* Ignore if line is empty */
    test    %r12d, %r12d
    jz      read_loop

    dec     %r12d

    /* Erase on terminal: \b \b (backspace, space, backspace) */
    mov     $SYS_WRITE, %eax
    mov     $FD_STDOUT, %edi
    lea     erase_seq(%rip), %rsi
    mov     $erase_seq_len, %edx
    syscall

    jmp     read_loop

handle_enter:
    /* Echo newline */
    mov     $SYS_WRITE, %eax
    mov     $FD_STDOUT, %edi
    lea     newline(%rip), %rsi
    mov     $newline_len, %edx
    syscall

    /* If line is empty, just re-prompt */
    test    %r12d, %r12d
    jz      prompt_loop

    /* Echo the line back */
    mov     $SYS_WRITE, %eax
    mov     $FD_STDOUT, %edi
    mov     %r13, %rsi
    mov     %r12d, %edx
    syscall

    /* Print trailing newline */
    mov     $SYS_WRITE, %eax
    mov     $FD_STDOUT, %edi
    lea     newline(%rip), %rsi
    mov     $newline_len, %edx
    syscall

    jmp     prompt_loop

/* --- Read-only data --- */
.section .rodata

banner:
    .ascii "eithne shell v0.1\n"
    .equ banner_len, . - banner

prompt:
    .ascii "> "
    .equ prompt_len, . - prompt

newline:
    .ascii "\r\n"
    .equ newline_len, . - newline

erase_seq:
    .byte 0x08, 0x20, 0x08         /* BS, space, BS */
    .equ erase_seq_len, . - erase_seq

/* --- Mutable data --- */
.section .bss

charbuf:
    .skip 1

linebuf:
    .skip LINE_MAX
