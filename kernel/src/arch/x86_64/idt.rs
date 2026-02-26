//! Interrupt Descriptor Table for x86_64.
//!
//! Maps interrupt vectors (0-255) to handler functions.
//! Vectors 0-31 are CPU exceptions, 32+ are for hardware IRQs and software interrupts.

// TODO: Define IDT entry structure
// TODO: Set up exception handlers (divide error, page fault, double fault, etc.)
// TODO: Load IDT with lidt instruction
