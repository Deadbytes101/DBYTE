//! Freestanding native compilation for the `i686-dbyte-none` target.
//!
//! This crate reuses DByte's lexer, parser, and AST. It does not invoke the
//! hosted interpreter/VM or any external compiler, linker, or assembler.

#[cfg(test)]
mod block_device_contract;
#[cfg(test)]
mod blocking_contract;
pub mod cli;
pub mod diagnostics;
pub mod elf32;
#[cfg(test)]
mod filesystem_contract;
pub mod ir;
#[cfg(test)]
mod irq_io_contract;
pub mod linker;
pub mod lower;
#[cfg(test)]
mod memory_contract;
pub mod modules;
pub mod object;
#[cfg(test)]
mod scheduler_contract;
pub mod semantic;
pub mod system_types;
pub mod systems_compiler;
pub mod systems_ir;
pub mod systems_lower;
pub mod systems_semantic;
pub mod types;
pub mod x86;

use std::fmt::Write;

use dbyte_lexer::Lexer;
use dbyte_parser::Parser;

pub use diagnostics::{Diagnostic, DiagnosticCategory};
pub use elf32::ElfLayout;

pub const TARGET: &str = "i686-dbyte-none";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitKind {
    Elf,
    Bin,
    Obj,
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

fn attach_path(error: Diagnostic, path: &str) -> Diagnostic {
    if error.path.is_some() {
        error
    } else {
        error.with_path(path)
    }
}

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
        let error = compile_source("bad.dby", "@").unwrap_err();
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
}
