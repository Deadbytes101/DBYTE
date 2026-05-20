#![allow(dead_code)]

//! Page Fault frame layout foundation.
//!
//! This module documents the vector 14 data shape used by the controlled
//! Page Fault handler smoke path.

/// x86 exception vector reserved for Page Fault.
pub const PAGE_FAULT_VECTOR: u8 = 14;

/// Page Fault error-code bit masks.
///
/// Intel-style names:
/// - P: page-present violation when set, non-present page when clear.
/// - W/R: write access when set, read access when clear.
/// - U/S: user-mode access when set, supervisor access when clear.
/// - RSVD: reserved page-table bit violation.
/// - I/D: instruction fetch violation.
pub struct PageFaultErrorCode;

impl PageFaultErrorCode {
    /// P bit: present/protection violation.
    pub const PRESENT: u32 = 1 << 0;
    /// W/R bit: write access when set, read access when clear.
    pub const WRITE: u32 = 1 << 1;
    /// U/S bit: user-mode access when set, supervisor access when clear.
    pub const USER: u32 = 1 << 2;
    /// RSVD bit: reserved page-table bit was set.
    pub const RESERVED_WRITE: u32 = 1 << 3;
    /// I/D bit: instruction fetch caused the fault.
    pub const INSTRUCTION_FETCH: u32 = 1 << 4;
}

/// Planned same-ring Page Fault frame documentation.
///
/// This records the same-ring frame shape used by the v8.12.1 Page Fault smoke
/// handler after vector 14 is explicitly enabled.
#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct PageFaultFrame {
    /// CPU-pushed Page Fault error code.
    pub error_code: u32,
    /// Saved instruction pointer for the faulting context.
    pub eip: u32,
    /// Saved code-segment selector.
    pub cs: u32,
    /// Saved EFLAGS value.
    pub eflags: u32,
    /// CR2 faulting linear address snapshot captured by the handler.
    pub cr2: u32,
}
