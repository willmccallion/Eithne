/* Minimal user-mode init process for eithne.
 *
 * Syscall ABI (Linux-compatible):
 *   RAX = syscall number
 *   RDI, RSI, RDX = args 1-3
 *   SYSCALL instruction
 *
 * Syscalls used:
 *   1 = sys_write(fd, buf, len)
 *  60 = sys_exit(code)
 */

.section .text
.globl _start
_start:
    /* sys_write(1, msg, 26) */
    mov     $1, %eax            /* SYS_WRITE */
    mov     $1, %edi            /* fd = stdout */
    lea     msg(%rip), %rsi     /* buf */
    mov     $26, %edx           /* len */
    syscall

    /* sys_exit(0) */
    mov     $60, %eax           /* SYS_EXIT */
    xor     %edi, %edi          /* code = 0 */
    syscall

    /* Should never reach here */
    ud2

.section .rodata
msg:
    .ascii "Hello from ELF user mode!\n"
