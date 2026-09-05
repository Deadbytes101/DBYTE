//! Freestanding native compilation for the `i686-dbyte-none` target.
//!
//! This crate reuses DByte's lexer, parser, and AST. It does not invoke the
//! hosted interpreter/VM or any external compiler, linker, or assembler.

pub mod abi64;
pub mod archive;
#[cfg(test)]
mod block_device_contract;
#[cfg(test)]
mod blocking_contract;
pub mod cli;
pub mod coff64;
pub mod dbup;
pub mod diagnostics;
pub mod elf32;
pub mod elf64;
pub mod external_coff64;
pub mod external_elf;
pub mod external_elf64;
#[cfg(test)]
mod filesystem_contract;
#[cfg(test)]
mod filesystem_runtime_contract;
pub mod hir_native;
pub mod inspection;
pub mod ir;
#[cfg(test)]
mod irq_io_contract;
pub mod legacy_hir;
pub mod linker;
pub mod linker64;
pub mod lower;
#[cfg(test)]
mod memory_contract;
pub mod modern_native;
pub mod modules;
pub mod multiboot2;
pub mod native_mir;
pub mod native_mir_lower;
pub mod native_select_i686;
pub mod native_select_x86_64;
pub mod object;
pub mod object64;
pub mod pe64;
#[cfg(test)]
mod scheduler_contract;
#[cfg(test)]
pub mod semantic;
pub mod system_types;
pub mod systems_compiler;
pub mod systems_ir;
pub mod systems_lower;
pub mod target;

pub use inspection::inspect_elf;
#[cfg(test)]
pub mod systems_semantic;
pub mod types;
#[cfg(test)]
mod user_process_contract;
pub mod x86;
pub mod x86_64;

#[cfg(test)]
use std::fmt::Write;

#[cfg(test)]
use dbyte_lexer::Lexer;
#[cfg(test)]
use dbyte_parser::Parser;

pub use diagnostics::{Diagnostic, DiagnosticCategory};
pub use elf32::ElfLayout;

pub const TARGET: &str = "i686-dbyte-none";
pub const USER_TARGET: &str = "i686-dbyte-user";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitKind {
    Elf,
    Bin,
    Obj,
    StaticLib,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeArtifact {
    pub elf: Vec<u8>,
    pub bin: Vec<u8>,
    pub machine_code: Vec<u8>,
    pub layout: ElfLayout,
    pub ir_dump: String,
    pub machine_dump: String,
}

/// Compile DByte source completely in memory through the freestanding native
/// pipeline. No output file is touched by this function.
#[cfg(test)]
pub fn compile_source(path: &str, source: &str) -> Result<NativeArtifact, Diagnostic> {
    let tokens = Lexer::new(source).tokenize().map_err(|error| {
        Diagnostic::new(DiagnosticCategory::Lex, error.msg, error.span).with_path(path)
    })?;
    let program = Parser::new(tokens).parse_program().map_err(|error| {
        Diagnostic::new(DiagnosticCategory::Parse, error.msg, error.span).with_path(path)
    })?;
    let ir = semantic::analyze(&program).map_err(|error| attach_path(error, path))?;
    let ir_dump = ir.dump();
    let lowered = lower::lower_program(&ir).map_err(|error| attach_path(error, path))?;
    let elf_artifact =
        elf32::write_executable(&ir, &lowered).map_err(|error| attach_path(error, path))?;

    let mut machine_dump = lowered.dump;
    if !machine_dump.ends_with('\n') {
        machine_dump.push('\n');
    }
    machine_dump.push_str(&elf_artifact.dump);
    if !machine_dump.ends_with('\n') {
        machine_dump.push('\n');
    }
    machine_dump.push_str("patched-machine-code\n");
    append_hex_dump(&mut machine_dump, &elf_artifact.machine_code);

    Ok(NativeArtifact {
        elf: elf_artifact.elf,
        bin: elf_artifact.bin,
        machine_code: elf_artifact.machine_code,
        layout: elf_artifact.layout,
        ir_dump,
        machine_dump,
    })
}

#[cfg(test)]
fn attach_path(error: Diagnostic, path: &str) -> Diagnostic {
    if error.path.is_some() {
        error
    } else {
        error.with_path(path)
    }
}

#[cfg(test)]
fn append_hex_dump(output: &mut String, bytes: &[u8]) {
    for (row, chunk) in bytes.chunks(16).enumerate() {
        let _ = write!(output, "{:08x}:", row * 16);
        for byte in chunk {
            let _ = write!(output, " {byte:02x}");
        }
        output.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_KERNEL: &str =
        "fn kernel_main() -> int:\n    let value: int = 40 + 2\n    return value\n";

    #[test]
    fn compiles_source_through_real_elf32_pipeline() {
        let artifact = compile_source("minimal.dby", MINIMAL_KERNEL).unwrap();
        assert!(artifact.elf.starts_with(b"\x7fELF"));
        assert_eq!(artifact.elf[4], 1);
        assert_eq!(u16::from_le_bytes([artifact.elf[18], artifact.elf[19]]), 3);
        assert!(!artifact.machine_code.is_empty());
        assert!(artifact.layout.entry >= artifact.layout.text_address);
        assert!(artifact.ir_dump.contains("add.i32"));
        assert!(artifact.machine_dump.contains("patched-machine-code"));
    }

    #[test]
    fn complete_pipeline_is_deterministic() {
        let first = compile_source("same.dby", MINIMAL_KERNEL).unwrap();
        let second = compile_source("same.dby", MINIMAL_KERNEL).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn frontend_failure_keeps_category_path_and_span() {
        let error = compile_source("bad.dby", "`").unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::Lex);
        assert_eq!(error.path.as_deref(), Some("bad.dby"));
        assert!(error.span.is_some());
    }

    #[test]
    fn native_kernel_example_reaches_intrinsic_and_control_flow_ir() {
        let source = include_str!("../../../examples/native_kernel/kernel.dby");
        let artifact = compile_source("examples/native_kernel/kernel.dby", source).unwrap();
        assert!(artifact.ir_dump.contains("fn kernel_main("));
        assert!(artifact.ir_dump.contains("mul.i32"));
        assert!(artifact.ir_dump.contains("call add 2 -> i32"));
        assert!(artifact.ir_dump.contains("jump_if_false"));
        assert!(artifact.ir_dump.contains("intrinsic native.io_out8"));
        assert!(artifact.machine_dump.contains("fixup call-rel32"));
        assert!(artifact.machine_dump.contains("patched absolute immediate"));
    }

    #[test]
    fn new_memory_intrinsics_reach_ir_and_produce_elf() {
        // Minimal kernel that exercises all five new memory intrinsics through
        // the complete compile_source() pipeline: lex → parse → semantic → lower → ELF.
        let source = r#"
fn kernel_main() -> int:
    let addr: int = 0x1000
    let v16: int = native.volatile_load16(addr)
    native.volatile_store16(addr, v16)
    let buf: int = 0x2000
    let src: int = 0x3000
    native.mem_copy(buf, src, 64)
    native.mem_set(buf, 0xff, 32)
    native.mem_zero(buf, 32)
    return 0
"#;
        let artifact = compile_source("mem_intrinsics_test.dby", source).unwrap();
        assert!(artifact.elf.starts_with(b"\x7fELF"));
        assert!(
            artifact
                .ir_dump
                .contains("intrinsic native.volatile_load16"),
            "expected volatile_load16 in IR dump"
        );
        assert!(
            artifact
                .ir_dump
                .contains("intrinsic native.volatile_store16"),
            "expected volatile_store16 in IR dump"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.mem_copy"),
            "expected mem_copy in IR dump"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.mem_set"),
            "expected mem_set in IR dump"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.mem_zero"),
            "expected mem_zero in IR dump"
        );
        // Machine dump should be non-empty and contain the REP string opcodes
        assert!(
            artifact.machine_dump.contains("patched-machine-code"),
            "machine dump should include hex section"
        );
    }

    #[test]
    fn vga_byte_write_probe_produces_valid_elf() {
        // This is the exact VGA write pattern that will display "GB" in GBTOS.
        // Passes through the complete lex→parse→semantic→lower→ELF pipeline.
        let source = r#"
fn kernel_main() -> int:
    let vga: int = 0xb8000

    native.volatile_store8(vga, 71)
    native.volatile_store8(vga + 1, 15)

    native.volatile_store8(vga + 2, 66)
    native.volatile_store8(vga + 3, 15)

    return 0
"#;
        let artifact = compile_source("vga_probe.dby", source).unwrap();
        assert!(artifact.elf.starts_with(b"\x7fELF"), "ELF magic missing");
        assert!(
            artifact
                .ir_dump
                .contains("intrinsic native.volatile_store8"),
            "expected volatile_store8 in IR dump"
        );
        // MOV BYTE [r], r8 uses opcode 0x88
        assert!(
            artifact.machine_code.contains(&0x88),
            "expected byte-store opcode 0x88 in machine code"
        );
    }

    #[test]
    fn cpu_control_and_port_aliases_reach_ir() {
        // Verify that cpu_cli/sti/pause and port_in8/port_out8 aliases all
        // resolve through the full pipeline without errors.
        let source = r#"
fn kernel_main() -> int:
    native.cpu_cli()
    native.port_out8(0x20, 0x20)
    let status: int = native.port_in8(0x64)
    native.cpu_sti()
    native.cpu_pause()
    return status
"#;
        let artifact = compile_source("cpu_port_test.dby", source).unwrap();
        assert!(artifact.elf.starts_with(b"\x7fELF"));
        // Both port aliases should resolve to the io_out8 / io_in8 qualified names.
        assert!(
            artifact.ir_dump.contains("intrinsic native.io_out8"),
            "port_out8 should resolve to native.io_out8 in IR"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.io_in8"),
            "port_in8 should resolve to native.io_in8 in IR"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.cpu_cli"),
            "expected cpu_cli in IR"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.cpu_sti"),
            "expected cpu_sti in IR"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.cpu_pause"),
            "expected cpu_pause in IR"
        );
        // CLI = FA, STI = FB must appear in machine code
        assert!(
            artifact.machine_code.contains(&0xfa),
            "expected CLI byte FA"
        );
        assert!(
            artifact.machine_code.contains(&0xfb),
            "expected STI byte FB"
        );
    }

    #[test]
    fn idt_and_eflags_primitives_reach_ir_and_produce_elf() {
        // Simulates the skeleton of a real interrupt_init() routine:
        // disable interrupts, load the IDT, re-enable interrupts.
        let source = r#"
fn interrupt_init(idt_ptr: int) -> int:
    native.cpu_cli()
    let flags: int = native.read_eflags()
    native.lidt(idt_ptr)
    native.write_eflags(flags)
    native.cpu_sti()
    return 0

fn kernel_main() -> int:
    let idt: int = 0x5000
    return interrupt_init(idt)
"#;
        let artifact = compile_source("idt_test.dby", source).unwrap();
        assert!(artifact.elf.starts_with(b"\x7fELF"), "ELF magic missing");
        assert!(
            artifact.ir_dump.contains("intrinsic native.lidt"),
            "expected native.lidt in IR"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.read_eflags"),
            "expected native.read_eflags in IR"
        );
        assert!(
            artifact.ir_dump.contains("intrinsic native.write_eflags"),
            "expected native.write_eflags in IR"
        );
        // PUSHFD = 9C, POPFD = 9D, LIDT prefix = 0F
        assert!(
            artifact.machine_code.contains(&0x9c),
            "expected PUSHFD (9C)"
        );
        assert!(artifact.machine_code.contains(&0x9d), "expected POPFD (9D)");
        assert!(
            artifact.machine_code.contains(&0x0f),
            "expected LIDT escape (0F)"
        );
    }
}
