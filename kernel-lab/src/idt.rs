#![allow(dead_code)]

//! Interrupt Descriptor Table (IDT) Foundation for x86
//!
//! Under freestanding constraints, this skeleton defines Gate Descriptors
//! (IDT entries) and the base pointer representation to be loaded via LIDT.

/// A standard packed 8-byte x86 Gate Descriptor representing an IDT entry.
///
/// Layout constraints (8 bytes, packed):
/// - Bytes 0..1: Offset low bits (0..15 of target handler address)
/// - Bytes 2..3: GDT segment selector (typically code selector 0x08)
/// - Byte 4: Reserved/Zero (always 0x00)
/// - Byte 5: Type attributes (Present flag, DPL privilege levels, Gate type details)
/// - Bytes 6..7: Offset high bits (16..31 of target handler address)
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct IdtEntry {
    /// Low 16 bits of the interrupt service routine (ISR) address.
    pub offset_low: u16,
    /// Code segment selector in the Global Descriptor Table (GDT).
    pub selector: u16,
    /// Reserved byte, always 0.
    pub zero: u8,
    /// Gate type and attributes (e.g. Present flag, Privilege level).
    pub type_attr: u8,
    /// High 16 bits of the interrupt service routine (ISR) address.
    pub offset_high: u16,
}

impl IdtEntry {
    /// Create a zero-initialized gate descriptor.
    pub const fn new() -> Self {
        Self {
            offset_low: 0,
            selector: 0,
            zero: 0,
            type_attr: 0,
            offset_high: 0,
        }
    }

    /// Create a dummy / non-present missing entry.
    pub const fn missing() -> Self {
        Self::new()
    }
}

/// The IDT Pointer structure loaded into the processor register via the `lidt` assembly instruction.
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct IdtPtr {
    /// Size of the IDT in bytes minus 1.
    pub limit: u16,
    /// Linear base address of the IDT.
    pub base: u32,
}

impl IdtPtr {
    /// Create a zero-initialized pointer descriptor.
    pub const fn new() -> Self {
        Self { limit: 0, base: 0 }
    }
}

/// The main IDT table structure containing gate descriptors.
/// For standard x86, we allocate 256 entry gates.
pub struct InterruptDescriptorTable {
    pub entries: [IdtEntry; 256],
}

impl InterruptDescriptorTable {
    /// Create a new zeroed out Interrupt Descriptor Table.
    pub const fn new() -> Self {
        Self {
            entries: [IdtEntry::new(); 256],
        }
    }

    /// Load the Interrupt Descriptor Table into the CPU's IDTR register using standard `lidt` assembly.
    pub unsafe fn load(&self) {
        let ptr = IdtPtr {
            limit: (core::mem::size_of::<Self>() - 1) as u16,
            base: self as *const _ as u32,
        };
        core::arch::asm!(
            "lidt [{}]",
            in(reg) &ptr,
            options(readonly, nostack, preserves_flags)
        );
    }
}

/// Global static instance representing the active CPU Interrupt Descriptor Table.
pub static mut IDT: InterruptDescriptorTable = InterruptDescriptorTable::new();
