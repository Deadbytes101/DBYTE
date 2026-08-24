//! Typed native IR to bounded i686 machine-code lowering.

use crate::diagnostics::{Diagnostic, DiagnosticCategory};
use crate::ir::{CompareOp, IrFunction, IrInst, IrProgram, LabelId, LocalId, StringId};
use crate::types::{Intrinsic, NativeType};
use crate::x86::{
    Condition, Encoder, EncoderError, FixupTrace, Label, Register, DEFAULT_MAX_CODE_SIZE,
};
use dbyte_ast::Span;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub const NATIVE_ENTRY_FUNCTION: &str = "kernel_main";
pub const MAX_MACHINE_CODE_SIZE: usize = DEFAULT_MAX_CODE_SIZE;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbsoluteFixupTarget {
    StackTop,
    String(StringId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbsoluteFixup {
    /// Offset of the four-byte little-endian immediate inside `machine_code`.
    pub immediate_offset: u32,
    pub target: AbsoluteFixupTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionSymbol {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub parameter_count: u32,
    pub slot_count: u32,
    pub return_type: NativeType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoweredProgram {
    /// Complete bootstrap plus all DByte functions. Absolute immediates remain
    /// zero until the ELF layout is known.
    pub machine_code: Vec<u8>,
    pub entry_offset: u32,
    pub functions: Vec<FunctionSymbol>,
    pub absolute_fixups: Vec<AbsoluteFixup>,
    pub relative_fixups: Vec<FixupTrace>,
    pub dump: String,
}

impl LoweredProgram {
    pub fn function(&self, name: &str) -> Option<&FunctionSymbol> {
        self.functions.iter().find(|symbol| symbol.name == name)
    }
}

/// Lower a semantically validated native program using the DBYTE32 ABI.
///
/// Arguments are evaluated and pushed left-to-right.  Consequently, parameter
/// `i` is copied by the callee from `[ebp + 8 + 4 * (argc - 1 - i)]` into its
/// deterministic EBP-negative slot.  The caller removes arguments and EAX
/// carries the return value.
pub fn lower_program(program: &IrProgram) -> Result<LoweredProgram, Diagnostic> {
    let mut encoder = Encoder::new(MAX_MACHINE_CODE_SIZE);
    let mut function_labels = BTreeMap::new();

    for function in &program.functions {
        if function_labels.contains_key(&function.name) {
            return Err(Diagnostic::new(
                DiagnosticCategory::DuplicateFunction,
                format!(
                    "duplicate native function '{}' reached lowering",
                    function.name
                ),
                function.span,
            ));
        }
        let label = encoder
            .new_label()
            .map_err(|error| encode_diagnostic(error, function.span))?;
        function_labels.insert(function.name.clone(), label);
    }

    let entry_function = program.function(NATIVE_ENTRY_FUNCTION).ok_or_else(|| {
        Diagnostic::without_span(
            DiagnosticCategory::InvalidEntryPoint,
            "native entry point 'kernel_main' is missing",
        )
    })?;
    if !entry_function.params.is_empty() || entry_function.return_type != NativeType::I32 {
        return Err(Diagnostic::new(
            DiagnosticCategory::InvalidEntryPoint,
            "native entry point must have signature fn kernel_main() -> int",
            entry_function.span,
        ));
    }
    let entry_label = function_labels[NATIVE_ENTRY_FUNCTION];

    let entry_offset = checked_u32(encoder.position(), entry_function.span)?;
    let mut absolute_fixups = Vec::new();

    // Compiler-owned Multiboot entry stub. The ELF writer patches stack_top.
    emit(&mut encoder, entry_function.span, |encoder| encoder.cli())?;
    let stack_immediate = encoder
        .mov_reg_imm32_placeholder(Register::Esp)
        .map_err(|error| encode_diagnostic(error, entry_function.span))?;
    absolute_fixups.push(AbsoluteFixup {
        immediate_offset: checked_u32(stack_immediate, entry_function.span)?,
        target: AbsoluteFixupTarget::StackTop,
    });
    emit(&mut encoder, entry_function.span, |encoder| {
        encoder.xor_reg_reg(Register::Ebp, Register::Ebp)
    })?;
    emit(&mut encoder, entry_function.span, |encoder| {
        encoder.call(entry_label)
    })?;
    let halt_label = encoder
        .new_label()
        .map_err(|error| encode_diagnostic(error, entry_function.span))?;
    encoder
        .bind_label(halt_label)
        .map_err(|error| encode_diagnostic(error, entry_function.span))?;
    emit(&mut encoder, entry_function.span, |encoder| encoder.hlt())?;
    emit(&mut encoder, entry_function.span, |encoder| {
        encoder.jump(halt_label)
    })?;

    let mut functions = Vec::with_capacity(program.functions.len());
    for function in &program.functions {
        lower_function(
            program,
            function,
            function_labels[&function.name],
            &function_labels,
            &mut encoder,
            &mut absolute_fixups,
            &mut functions,
        )?;
    }

    let encoded = encoder
        .finish()
        .map_err(|error| encode_diagnostic(error, entry_function.span))?;
    let mut dump = String::new();
    let _ = writeln!(dump, "target i686-dbyte-none");
    let _ = writeln!(dump, "abi DBYTE32");
    let _ = writeln!(dump, "entry +0x{entry_offset:08x}");
    for function in &functions {
        let _ = writeln!(
            dump,
            "function {} offset=0x{:08x} size={} params={} slots={} return={}",
            function.name,
            function.offset,
            function.size,
            function.parameter_count,
            function.slot_count,
            function.return_type
        );
    }
    for fixup in &absolute_fixups {
        let target = match fixup.target {
            AbsoluteFixupTarget::StackTop => "stack_top".to_owned(),
            AbsoluteFixupTarget::String(id) => format!("@str{}", id.0),
        };
        let _ = writeln!(
            dump,
            "absolute-fixup immediate=0x{:08x} target={target}",
            fixup.immediate_offset
        );
    }
    dump.push_str(&encoded.dump());

    Ok(LoweredProgram {
        machine_code: encoded.bytes,
        entry_offset,
        functions,
        absolute_fixups,
        relative_fixups: encoded.fixups,
        dump,
    })
}

#[allow(clippy::too_many_arguments)]
fn lower_function(
    program: &IrProgram,
    function: &IrFunction,
    function_label: Label,
    function_labels: &BTreeMap<String, Label>,
    encoder: &mut Encoder,
    absolute_fixups: &mut Vec<AbsoluteFixup>,
    symbols: &mut Vec<FunctionSymbol>,
) -> Result<(), Diagnostic> {
    validate_parameter_slots(function)?;
    let frame_size = function
        .slots
        .len()
        .checked_mul(4)
        .ok_or_else(|| backend_diagnostic("native stack frame size overflow", function.span))?;
    let frame_size_u32 = checked_u32(frame_size, function.span)?;
    if frame_size_u32 > i32::MAX as u32 {
        return Err(backend_diagnostic(
            "native stack frame exceeds signed i686 displacement range",
            function.span,
        ));
    }

    let mut ir_labels = BTreeMap::new();
    for instruction in &function.instructions {
        if let IrInst::Label(label_id) = instruction {
            if ir_labels.contains_key(label_id) {
                return Err(backend_diagnostic(
                    format!("duplicate IR label {label_id} in '{}'", function.name),
                    function.span,
                ));
            }
            let label = encoder
                .new_label()
                .map_err(|error| encode_diagnostic(error, function.span))?;
            ir_labels.insert(*label_id, label);
        }
    }

    encoder
        .bind_label(function_label)
        .map_err(|error| encode_diagnostic(error, function.span))?;
    let start = encoder.position();

    emit(encoder, function.span, |encoder| {
        encoder.push_reg(Register::Ebp)
    })?;
    emit(encoder, function.span, |encoder| {
        encoder.mov_reg_reg(Register::Ebp, Register::Esp)
    })?;
    if frame_size_u32 != 0 {
        emit(encoder, function.span, |encoder| {
            encoder.sub_esp_imm32(frame_size_u32)
        })?;
    }

    // Initialize every deterministic slot, including parameter slots, before
    // copying incoming arguments into their assigned locations.
    for slot_index in 0..function.slots.len() {
        let displacement = slot_displacement(LocalId(slot_index as u32), function)?;
        emit(encoder, function.span, |encoder| {
            encoder.mov_ebp_disp32_imm32(displacement, 0)
        })?;
    }
    for (parameter_index, parameter) in function.params.iter().enumerate() {
        let reverse_index = function.params.len() - 1 - parameter_index;
        let incoming_offset = 8usize
            .checked_add(reverse_index.checked_mul(4).ok_or_else(|| {
                backend_diagnostic("native parameter offset overflow", function.span)
            })?)
            .ok_or_else(|| backend_diagnostic("native parameter offset overflow", function.span))?;
        let incoming_offset = i32::try_from(incoming_offset).map_err(|_| {
            backend_diagnostic(
                "native parameter offset exceeds signed i686 displacement range",
                function.span,
            )
        })?;
        let destination = slot_displacement(parameter.slot, function)?;
        emit(encoder, function.span, |encoder| {
            encoder.mov_reg_from_ebp_disp32(Register::Eax, incoming_offset)
        })?;
        emit(encoder, function.span, |encoder| {
            encoder.mov_ebp_disp32_from_reg(destination, Register::Eax)
        })?;
    }

    let mut evaluation_depth = 0usize;
    let mut saw_return = false;
    for instruction in &function.instructions {
        lower_instruction(
            program,
            function,
            instruction,
            function_labels,
            &ir_labels,
            encoder,
            absolute_fixups,
            &mut evaluation_depth,
            &mut saw_return,
        )?;
    }
    if !saw_return {
        return Err(Diagnostic::new(
            DiagnosticCategory::InvalidReturn,
            format!(
                "native function '{}' has no return instruction",
                function.name
            ),
            function.span,
        ));
    }
    if evaluation_depth != 0 {
        return Err(backend_diagnostic(
            format!(
                "native function '{}' ends with {} values on its evaluation stack",
                function.name, evaluation_depth
            ),
            function.span,
        ));
    }

    let end = encoder.position();
    symbols.push(FunctionSymbol {
        name: function.name.clone(),
        offset: checked_u32(start, function.span)?,
        size: checked_u32(end - start, function.span)?,
        parameter_count: checked_u32(function.params.len(), function.span)?,
        slot_count: checked_u32(function.slots.len(), function.span)?,
        return_type: function.return_type,
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_instruction(
    program: &IrProgram,
    function: &IrFunction,
    instruction: &IrInst,
    function_labels: &BTreeMap<String, Label>,
    ir_labels: &BTreeMap<LabelId, Label>,
    encoder: &mut Encoder,
    absolute_fixups: &mut Vec<AbsoluteFixup>,
    evaluation_depth: &mut usize,
    saw_return: &mut bool,
) -> Result<(), Diagnostic> {
    let span = function.span;
    match instruction {
        IrInst::ConstI32(value) => {
            emit(encoder, span, |encoder| {
                encoder.mov_reg_imm32(Register::Eax, *value as u32)
            })?;
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::ConstBool(value) => {
            emit(encoder, span, |encoder| {
                encoder.mov_reg_imm32(Register::Eax, u32::from(*value))
            })?;
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::ConstStr(string_id) => {
            if program.strings.get(string_id.index()).is_none() {
                return Err(backend_diagnostic(
                    format!("unknown native string {string_id}"),
                    span,
                ));
            }
            let immediate = encoder
                .mov_reg_imm32_placeholder(Register::Eax)
                .map_err(|error| encode_diagnostic(error, span))?;
            absolute_fixups.push(AbsoluteFixup {
                immediate_offset: checked_u32(immediate, span)?,
                target: AbsoluteFixupTarget::String(*string_id),
            });
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::LocalLoad(local) => {
            let displacement = slot_displacement(*local, function)?;
            emit(encoder, span, |encoder| {
                encoder.mov_reg_from_ebp_disp32(Register::Eax, displacement)
            })?;
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::LocalStore(local) => {
            consume(evaluation_depth, 1, function, "local.store")?;
            let displacement = slot_displacement(*local, function)?;
            pop(encoder, span, Register::Eax)?;
            emit(encoder, span, |encoder| {
                encoder.mov_ebp_disp32_from_reg(displacement, Register::Eax)
            })?;
        }
        IrInst::AddI32 | IrInst::SubI32 | IrInst::MulI32 => {
            consume(evaluation_depth, 2, function, "binary arithmetic")?;
            pop(encoder, span, Register::Ecx)?;
            pop(encoder, span, Register::Eax)?;
            match instruction {
                IrInst::AddI32 => emit(encoder, span, |encoder| {
                    encoder.add_reg_reg(Register::Eax, Register::Ecx)
                })?,
                IrInst::SubI32 => emit(encoder, span, |encoder| {
                    encoder.sub_reg_reg(Register::Eax, Register::Ecx)
                })?,
                IrInst::MulI32 => emit(encoder, span, |encoder| {
                    encoder.imul_reg_reg(Register::Eax, Register::Ecx)
                })?,
                _ => unreachable!(),
            }
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::DivI32 | IrInst::RemI32 => {
            consume(evaluation_depth, 2, function, "division")?;
            pop(encoder, span, Register::Ecx)?;
            pop(encoder, span, Register::Eax)?;
            emit(encoder, span, |encoder| encoder.cdq())?;
            emit(encoder, span, |encoder| encoder.idiv_reg(Register::Ecx))?;
            if matches!(instruction, IrInst::RemI32) {
                emit(encoder, span, |encoder| {
                    encoder.mov_reg_reg(Register::Eax, Register::Edx)
                })?;
            }
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::NegI32 => {
            consume(evaluation_depth, 1, function, "neg.i32")?;
            pop(encoder, span, Register::Eax)?;
            emit(encoder, span, |encoder| encoder.neg_reg(Register::Eax))?;
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::BoolNot => {
            consume(evaluation_depth, 1, function, "not.bool")?;
            pop(encoder, span, Register::Eax)?;
            emit(encoder, span, |encoder| {
                encoder.test_reg_reg(Register::Eax, Register::Eax)
            })?;
            emit(encoder, span, |encoder| {
                encoder.setcc_reg8(Condition::Equal, Register::Eax)
            })?;
            emit(encoder, span, |encoder| {
                encoder.movzx_reg_reg8(Register::Eax, Register::Eax)
            })?;
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::Compare(operator) => {
            consume(evaluation_depth, 2, function, "compare")?;
            pop(encoder, span, Register::Ecx)?;
            pop(encoder, span, Register::Eax)?;
            emit(encoder, span, |encoder| {
                encoder.cmp_reg_reg(Register::Eax, Register::Ecx)
            })?;
            let condition = compare_condition(*operator);
            emit(encoder, span, |encoder| {
                encoder.setcc_reg8(condition, Register::Eax)
            })?;
            emit(encoder, span, |encoder| {
                encoder.movzx_reg_reg8(Register::Eax, Register::Eax)
            })?;
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::StrIndex8 => {
            consume(evaluation_depth, 2, function, "str.index8")?;
            pop(encoder, span, Register::Ecx)?;
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.movzx_reg_from_base_index8(Register::Eax, Register::Edx, Register::Ecx)
            })?;
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::Label(label) => {
            let target = ir_label(ir_labels, *label, function)?;
            encoder
                .bind_label(target)
                .map_err(|error| encode_diagnostic(error, span))?;
        }
        IrInst::Jump(label) => {
            let target = ir_label(ir_labels, *label, function)?;
            emit(encoder, span, |encoder| encoder.jump(target))?;
        }
        IrInst::JumpIfFalse(label) => {
            consume(evaluation_depth, 1, function, "jump_if_false")?;
            pop(encoder, span, Register::Eax)?;
            emit(encoder, span, |encoder| {
                encoder.test_reg_reg(Register::Eax, Register::Eax)
            })?;
            let target = ir_label(ir_labels, *label, function)?;
            emit(encoder, span, |encoder| {
                encoder.jump_if(Condition::Equal, target)
            })?;
        }
        IrInst::Call {
            function: callee_name,
            argument_count,
            result,
        } => {
            consume(evaluation_depth, *argument_count, function, "call")?;
            let callee = program.function(callee_name).ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticCategory::UnresolvedFunction,
                    format!("unresolved native function '{callee_name}'"),
                    span,
                )
            })?;
            if callee.params.len() != *argument_count {
                return Err(Diagnostic::new(
                    DiagnosticCategory::WrongArity,
                    format!(
                        "native call to '{}' expects {} arguments but IR supplied {}",
                        callee_name,
                        callee.params.len(),
                        argument_count
                    ),
                    span,
                ));
            }
            if callee.return_type != *result {
                return Err(Diagnostic::new(
                    DiagnosticCategory::InvalidType,
                    format!(
                        "native call to '{}' declares result {} but function returns {}",
                        callee_name, result, callee.return_type
                    ),
                    span,
                ));
            }
            let target = function_labels.get(callee_name).copied().ok_or_else(|| {
                Diagnostic::new(
                    DiagnosticCategory::UnresolvedFunction,
                    format!("unresolved native function '{callee_name}'"),
                    span,
                )
            })?;
            emit(encoder, span, |encoder| encoder.call(target))?;
            if *argument_count != 0 {
                let cleanup = argument_count.checked_mul(4).ok_or_else(|| {
                    backend_diagnostic("native caller cleanup overflow", function.span)
                })?;
                let cleanup = checked_u32(cleanup, span)?;
                emit(encoder, span, |encoder| encoder.add_esp_imm32(cleanup))?;
            }
            push_eax(encoder, span)?;
            *evaluation_depth += 1;
        }
        IrInst::Intrinsic {
            intrinsic,
            argument_count,
            result,
        } => {
            if intrinsic.argument_count() != *argument_count || intrinsic.result_type() != *result {
                return Err(Diagnostic::new(
                    DiagnosticCategory::InvalidIntrinsic,
                    format!(
                        "invalid IR signature for {}: expected {} arguments and result {:?}",
                        intrinsic,
                        intrinsic.argument_count(),
                        intrinsic.result_type()
                    ),
                    span,
                ));
            }
            consume(evaluation_depth, *argument_count, function, "intrinsic")?;
            lower_intrinsic(*intrinsic, encoder, span)?;
            if result.is_some() {
                push_eax(encoder, span)?;
                *evaluation_depth += 1;
            }
        }
        IrInst::Pop => {
            consume(evaluation_depth, 1, function, "pop")?;
            pop(encoder, span, Register::Eax)?;
        }
        IrInst::Return => {
            consume(evaluation_depth, 1, function, "return")?;
            pop(encoder, span, Register::Eax)?;
            emit_epilogue(encoder, span)?;
            *saw_return = true;
        }
    }
    Ok(())
}

fn lower_intrinsic(
    intrinsic: Intrinsic,
    encoder: &mut Encoder,
    span: Span,
) -> Result<(), Diagnostic> {
    match intrinsic {
        Intrinsic::VolatileLoad8 => {
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.movzx_reg_from_base8(Register::Eax, Register::Edx)
            })?;
        }
        Intrinsic::VolatileStore8 => {
            pop(encoder, span, Register::Eax)?;
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.mov_base8_from_reg(Register::Edx, Register::Eax)
            })?;
        }
        Intrinsic::VolatileLoad16 => {
            // stack: [addr]  →  EAX = zero_extend(*addr as u16)
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.movzx_reg_from_base16(Register::Eax, Register::Edx)
            })?;
        }
        Intrinsic::VolatileStore16 => {
            // stack: [addr, value]  (value pushed last)
            pop(encoder, span, Register::Eax)?; // value
            pop(encoder, span, Register::Edx)?; // address
            emit(encoder, span, |encoder| {
                encoder.mov_base16_from_reg(Register::Edx, Register::Eax)
            })?;
        }
        Intrinsic::VolatileLoad32 => {
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.mov_reg_from_base32(Register::Eax, Register::Edx)
            })?;
        }
        Intrinsic::VolatileStore32 => {
            pop(encoder, span, Register::Eax)?;
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.mov_base32_from_reg(Register::Edx, Register::Eax)
            })?;
        }
        Intrinsic::IoOut8 => {
            pop(encoder, span, Register::Eax)?;
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| encoder.out_dx_al())?;
        }
        Intrinsic::IoIn8 => {
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.xor_reg_reg(Register::Eax, Register::Eax)
            })?;
            emit(encoder, span, |encoder| encoder.in_al_dx())?;
            emit(encoder, span, |encoder| {
                encoder.movzx_reg_reg8(Register::Eax, Register::Eax)
            })?;
        }
        Intrinsic::CpuCli => emit(encoder, span, |encoder| encoder.cli())?,
        Intrinsic::CpuSti => emit(encoder, span, |encoder| encoder.sti())?,
        Intrinsic::CpuHlt => emit(encoder, span, |encoder| encoder.hlt())?,
        Intrinsic::CpuPause => emit(encoder, span, |encoder| encoder.pause())?,
        Intrinsic::Lidt => {
            // stack: [desc_ptr]
            // LIDT [EDX] — desc_ptr is a 6-byte { u16 limit, u32 base } in memory.
            pop(encoder, span, Register::Edx)?;
            emit(encoder, span, |encoder| {
                encoder.lidt(crate::x86::MemoryOperand::new(Register::Edx, 0))
            })?;
        }
        Intrinsic::ReadEflags => {
            // PUSHFD saves EFLAGS onto the stack; we then pop it into EAX and
            // push EAX onto the IR evaluation stack as the return value.
            emit(encoder, span, |encoder| encoder.pushfd())?;
            emit(encoder, span, |encoder| encoder.pop_reg(Register::Eax))?;
        }
        Intrinsic::WriteEflags => {
            // stack: [flags]
            // Pop the value into EAX, push it, then POPFD restores EFLAGS.
            pop(encoder, span, Register::Eax)?;
            emit(encoder, span, |encoder| encoder.push_reg(Register::Eax))?;
            emit(encoder, span, |encoder| encoder.popfd())?;
        }
        Intrinsic::MemCopy => {
            // stack: [dst, src, count]  (count pushed last)
            // REP MOVSB clobbers ESI, EDI, ECX — save callee-saved ESI and EDI.
            emit(encoder, span, |encoder| encoder.push_reg(Register::Esi))?;
            emit(encoder, span, |encoder| encoder.push_reg(Register::Edi))?;
            pop(encoder, span, Register::Ecx)?; // count
            pop(encoder, span, Register::Esi)?; // src
            pop(encoder, span, Register::Edi)?; // dst
            emit(encoder, span, |encoder| encoder.rep_movsb())?;
            emit(encoder, span, |encoder| encoder.pop_reg(Register::Edi))?;
            emit(encoder, span, |encoder| encoder.pop_reg(Register::Esi))?;
        }
        Intrinsic::MemSet => {
            // stack: [dst, value, count]  (count pushed last)
            // REP STOSB clobbers EDI and ECX — save callee-saved EDI.
            emit(encoder, span, |encoder| encoder.push_reg(Register::Edi))?;
            pop(encoder, span, Register::Ecx)?; // count
            pop(encoder, span, Register::Eax)?; // value (AL = fill byte)
            pop(encoder, span, Register::Edi)?; // dst
            emit(encoder, span, |encoder| encoder.rep_stosb())?;
            emit(encoder, span, |encoder| encoder.pop_reg(Register::Edi))?;
        }
        Intrinsic::MemZero => {
            // stack: [dst, count]  (count pushed last)
            // Like MemSet with AL = 0; save callee-saved EDI.
            emit(encoder, span, |encoder| encoder.push_reg(Register::Edi))?;
            pop(encoder, span, Register::Ecx)?; // count
            pop(encoder, span, Register::Edi)?; // dst
            emit(encoder, span, |encoder| {
                encoder.xor_reg_reg(Register::Eax, Register::Eax)
            })?;
            emit(encoder, span, |encoder| encoder.rep_stosb())?;
            emit(encoder, span, |encoder| encoder.pop_reg(Register::Edi))?;
        }
    }
    Ok(())
}

fn validate_parameter_slots(function: &IrFunction) -> Result<(), Diagnostic> {
    let mut seen = BTreeSet::new();
    for parameter in &function.params {
        let slot = function.slots.get(parameter.slot.index()).ok_or_else(|| {
            backend_diagnostic(
                format!(
                    "parameter '{}' references missing slot {}",
                    parameter.name, parameter.slot
                ),
                function.span,
            )
        })?;
        if !slot.is_param || slot.ty != parameter.ty {
            return Err(backend_diagnostic(
                format!(
                    "parameter '{}' has inconsistent native slot {}",
                    parameter.name, parameter.slot
                ),
                function.span,
            ));
        }
        if !seen.insert(parameter.slot) {
            return Err(backend_diagnostic(
                format!("multiple parameters share native slot {}", parameter.slot),
                function.span,
            ));
        }
    }
    Ok(())
}

fn slot_displacement(local: LocalId, function: &IrFunction) -> Result<i32, Diagnostic> {
    if function.slots.get(local.index()).is_none() {
        return Err(backend_diagnostic(
            format!(
                "native local {} is outside function '{}' slot table",
                local, function.name
            ),
            function.span,
        ));
    }
    let magnitude = local
        .index()
        .checked_add(1)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| backend_diagnostic("native local displacement overflow", function.span))?;
    let magnitude = i32::try_from(magnitude).map_err(|_| {
        backend_diagnostic(
            "native local displacement exceeds signed i686 range",
            function.span,
        )
    })?;
    Ok(-magnitude)
}

fn ir_label(
    labels: &BTreeMap<LabelId, Label>,
    id: LabelId,
    function: &IrFunction,
) -> Result<Label, Diagnostic> {
    labels.get(&id).copied().ok_or_else(|| {
        backend_diagnostic(
            format!("unresolved IR label {id} in '{}'", function.name),
            function.span,
        )
    })
}

fn compare_condition(operator: CompareOp) -> Condition {
    match operator {
        CompareOp::Eq => Condition::Equal,
        CompareOp::NotEq => Condition::NotEqual,
        CompareOp::Lt => Condition::Less,
        CompareOp::LtEq => Condition::LessOrEqual,
        CompareOp::Gt => Condition::Greater,
        CompareOp::GtEq => Condition::GreaterOrEqual,
    }
}

fn emit_epilogue(encoder: &mut Encoder, span: Span) -> Result<(), Diagnostic> {
    emit(encoder, span, |encoder| {
        encoder.mov_reg_reg(Register::Esp, Register::Ebp)
    })?;
    emit(encoder, span, |encoder| encoder.pop_reg(Register::Ebp))?;
    emit(encoder, span, |encoder| encoder.ret())
}

fn push_eax(encoder: &mut Encoder, span: Span) -> Result<(), Diagnostic> {
    emit(encoder, span, |encoder| encoder.push_reg(Register::Eax))
}

fn pop(encoder: &mut Encoder, span: Span, register: Register) -> Result<(), Diagnostic> {
    emit(encoder, span, |encoder| encoder.pop_reg(register))
}

fn emit(
    encoder: &mut Encoder,
    span: Span,
    operation: impl FnOnce(&mut Encoder) -> Result<(), EncoderError>,
) -> Result<(), Diagnostic> {
    operation(encoder).map_err(|error| encode_diagnostic(error, span))
}

fn consume(
    depth: &mut usize,
    count: usize,
    function: &IrFunction,
    operation: &str,
) -> Result<(), Diagnostic> {
    if *depth < count {
        return Err(backend_diagnostic(
            format!(
                "IR evaluation stack underflow in '{}' during {operation}",
                function.name
            ),
            function.span,
        ));
    }
    *depth -= count;
    Ok(())
}

fn checked_u32(value: usize, span: Span) -> Result<u32, Diagnostic> {
    u32::try_from(value)
        .map_err(|_| backend_diagnostic("native machine-code offset exceeds u32", span))
}

fn encode_diagnostic(error: EncoderError, span: Span) -> Diagnostic {
    backend_diagnostic(format!("i686 encoder: {error}"), span)
}

fn backend_diagnostic(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::new(DiagnosticCategory::Encoder, message, span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrParam, IrSlot};

    fn function(
        name: &str,
        params: Vec<IrParam>,
        slots: Vec<IrSlot>,
        instructions: Vec<IrInst>,
    ) -> IrFunction {
        IrFunction {
            name: name.to_owned(),
            params,
            return_type: NativeType::I32,
            slots,
            instructions,
            span: Span::zero(),
        }
    }

    fn kernel(instructions: Vec<IrInst>) -> IrFunction {
        function(NATIVE_ENTRY_FUNCTION, Vec::new(), Vec::new(), instructions)
    }

    #[test]
    fn lowers_arithmetic_and_return() {
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(40),
                IrInst::ConstI32(2),
                IrInst::AddI32,
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let lowered = lower_program(&program).unwrap();
        assert!(lowered
            .machine_code
            .windows(2)
            .any(|bytes| bytes == [0x01, 0xc8]));
        assert!(lowered.machine_code.ends_with(&[0x89, 0xec, 0x5d, 0xc3]));
    }

    #[test]
    fn lowers_local_slot_zero_store_and_load() {
        let slot = IrSlot {
            name: "value".to_owned(),
            ty: NativeType::I32,
            is_param: false,
        };
        let program = IrProgram {
            functions: vec![function(
                NATIVE_ENTRY_FUNCTION,
                Vec::new(),
                vec![slot],
                vec![
                    IrInst::ConstI32(42),
                    IrInst::LocalStore(LocalId(0)),
                    IrInst::LocalLoad(LocalId(0)),
                    IrInst::Return,
                ],
            )],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(code
            .windows(10)
            .any(|bytes| bytes == [0xc7, 0x85, 0xfc, 0xff, 0xff, 0xff, 0, 0, 0, 0]));
        assert!(code
            .windows(6)
            .any(|bytes| bytes == [0x8b, 0x85, 0xfc, 0xff, 0xff, 0xff]));
    }

    #[test]
    fn lowers_branch_and_while_shape_with_rel32_fixups() {
        let loop_label = LabelId(0);
        let exit_label = LabelId(1);
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::Label(loop_label),
                IrInst::ConstBool(false),
                IrInst::JumpIfFalse(exit_label),
                IrInst::Jump(loop_label),
                IrInst::Label(exit_label),
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let lowered = lower_program(&program).unwrap();
        assert!(lowered
            .relative_fixups
            .iter()
            .any(|fixup| matches!(fixup.kind, crate::x86::FixupKind::ConditionalRel32(_))));
        assert!(lowered
            .relative_fixups
            .iter()
            .any(|fixup| fixup.displacement < 0));
    }

    #[test]
    fn function_call_uses_reverse_incoming_offsets_and_caller_cleanup() {
        let slots = vec![
            IrSlot {
                name: "a".to_owned(),
                ty: NativeType::I32,
                is_param: true,
            },
            IrSlot {
                name: "b".to_owned(),
                ty: NativeType::I32,
                is_param: true,
            },
        ];
        let add = function(
            "add",
            vec![
                IrParam {
                    name: "a".to_owned(),
                    ty: NativeType::I32,
                    slot: LocalId(0),
                },
                IrParam {
                    name: "b".to_owned(),
                    ty: NativeType::I32,
                    slot: LocalId(1),
                },
            ],
            slots,
            vec![
                IrInst::LocalLoad(LocalId(0)),
                IrInst::LocalLoad(LocalId(1)),
                IrInst::AddI32,
                IrInst::Return,
            ],
        );
        let entry = kernel(vec![
            IrInst::ConstI32(40),
            IrInst::ConstI32(2),
            IrInst::Call {
                function: "add".to_owned(),
                argument_count: 2,
                result: NativeType::I32,
            },
            IrInst::Return,
        ]);
        let code = lower_program(&IrProgram {
            functions: vec![entry, add],
            strings: Vec::new(),
        })
        .unwrap()
        .machine_code;
        assert!(code
            .windows(6)
            .any(|bytes| bytes == [0x8b, 0x85, 0x0c, 0, 0, 0]));
        assert!(code
            .windows(6)
            .any(|bytes| bytes == [0x8b, 0x85, 0x08, 0, 0, 0]));
        assert!(code
            .windows(6)
            .any(|bytes| bytes == [0x81, 0xc4, 0x08, 0, 0, 0]));
    }

    #[test]
    fn string_fixup_and_index_lowering_are_explicit() {
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstStr(StringId(0)),
                IrInst::ConstI32(1),
                IrInst::StrIndex8,
                IrInst::Return,
            ])],
            strings: vec![b"AB".to_vec()],
        };
        let lowered = lower_program(&program).unwrap();
        assert!(lowered
            .absolute_fixups
            .iter()
            .any(|fixup| { matches!(fixup.target, AbsoluteFixupTarget::String(StringId(0))) }));
        assert!(lowered
            .machine_code
            .windows(4)
            .any(|bytes| bytes == [0x0f, 0xb6, 0x04, 0x0a]));
    }

    #[test]
    fn lowers_port_intrinsics_without_host_bridge() {
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x3f8),
                IrInst::ConstI32(65),
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::IoOut8,
                    argument_count: 2,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(code.contains(&0xee));
    }

    #[test]
    fn unresolved_function_is_a_deterministic_diagnostic() {
        let program = IrProgram {
            functions: vec![kernel(vec![IrInst::Call {
                function: "missing".to_owned(),
                argument_count: 0,
                result: NativeType::I32,
            }])],
            strings: Vec::new(),
        };
        let error = lower_program(&program).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::UnresolvedFunction);
        assert_eq!(error.message, "unresolved native function 'missing'");
    }

    #[test]
    fn malformed_stack_ir_is_rejected_before_finish() {
        let program = IrProgram {
            functions: vec![kernel(vec![IrInst::AddI32, IrInst::Return])],
            strings: Vec::new(),
        };
        let error = lower_program(&program).unwrap_err();
        assert!(error.message.contains("IR evaluation stack underflow"));
    }

    // ── new memory intrinsic tests ─────────────────────────────────────────

    #[test]
    fn lowers_volatile_load16_emits_movzx_word() {
        // MOVZX r32, WORD PTR [r] encodes as 0F B7 /r
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x1000), // address
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::VolatileLoad16,
                    argument_count: 1,
                    result: Some(NativeType::I32),
                },
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        // 0F B7 is the two-byte opcode prefix for MOVZX r32, r/m16
        assert!(
            code.windows(2).any(|bytes| bytes == [0x0f, 0xb7]),
            "expected MOVZX word opcode 0F B7 in machine code"
        );
    }

    #[test]
    fn lowers_volatile_store16_emits_operand_size_prefix() {
        // MOV WORD PTR [r], r16 encodes as 66 89 /r
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x1000), // address
                IrInst::ConstI32(0xabcd), // value
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::VolatileStore16,
                    argument_count: 2,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        // Operand-size prefix 66 followed by MOV r/m16, r16 opcode 89
        assert!(
            code.windows(2).any(|bytes| bytes == [0x66, 0x89]),
            "expected operand-size prefix 66 89 for 16-bit store"
        );
    }

    #[test]
    fn lowers_mem_copy_emits_rep_movsb_with_esi_edi_save() {
        // REP MOVSB encodes as F3 A4; ESI = 0x56, EDI = 0x57
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x2000), // dst
                IrInst::ConstI32(0x1000), // src
                IrInst::ConstI32(16),     // count
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::MemCopy,
                    argument_count: 3,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(
            code.windows(2).any(|bytes| bytes == [0xf3, 0xa4]),
            "expected REP MOVSB opcode F3 A4"
        );
        // PUSH ESI = 56, PUSH EDI = 57; POP EDI = 5F, POP ESI = 5E
        assert!(code.contains(&0x56), "expected PUSH ESI (0x56)");
        assert!(code.contains(&0x57), "expected PUSH EDI (0x57)");
        assert!(code.contains(&0x5f), "expected POP EDI (0x5F)");
        assert!(code.contains(&0x5e), "expected POP ESI (0x5E)");
    }

    #[test]
    fn lowers_mem_set_emits_rep_stosb_with_edi_save() {
        // REP STOSB encodes as F3 AA; EDI = 0x57
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x3000), // dst
                IrInst::ConstI32(0xff),   // value
                IrInst::ConstI32(32),     // count
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::MemSet,
                    argument_count: 3,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(
            code.windows(2).any(|bytes| bytes == [0xf3, 0xaa]),
            "expected REP STOSB opcode F3 AA"
        );
        assert!(code.contains(&0x57), "expected PUSH EDI (0x57)");
        assert!(code.contains(&0x5f), "expected POP EDI (0x5F)");
    }

    #[test]
    fn lowers_mem_zero_emits_xor_then_rep_stosb() {
        // XOR EAX, EAX = 31 C0; REP STOSB = F3 AA
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x4000), // dst
                IrInst::ConstI32(64),     // count
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::MemZero,
                    argument_count: 2,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(
            code.windows(2).any(|bytes| bytes == [0xf3, 0xaa]),
            "expected REP STOSB opcode F3 AA"
        );
        // XOR EAX, EAX = 31 C0
        assert!(
            code.windows(2).any(|bytes| bytes == [0x31, 0xc0]),
            "expected XOR EAX, EAX (31 C0) for zero fill"
        );
        assert!(code.contains(&0x57), "expected PUSH EDI (0x57)");
        assert!(code.contains(&0x5f), "expected POP EDI (0x5F)");
    }

    #[test]
    fn lowers_cpu_cli_emits_fa() {
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::CpuCli,
                    argument_count: 0,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(code.contains(&0xfa), "expected CLI opcode FA");
    }

    #[test]
    fn lowers_cpu_sti_emits_fb() {
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::CpuSti,
                    argument_count: 0,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(code.contains(&0xfb), "expected STI opcode FB");
    }

    #[test]
    fn lowers_cpu_pause_emits_f3_90() {
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::CpuPause,
                    argument_count: 0,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(
            code.windows(2).any(|bytes| bytes == [0xf3, 0x90]),
            "expected PAUSE opcode F3 90"
        );
    }

    #[test]
    fn lowers_lidt_emits_0f_01_1a() {
        // LIDT [EDX] encodes as 0F 01 /3 with rm=010 (EDX) → 0F 01 1A
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x1000), // desc_ptr
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::Lidt,
                    argument_count: 1,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        // 0F 01 is the escape prefix for LIDT/LGDT
        assert!(
            code.windows(2).any(|bytes| bytes == [0x0f, 0x01]),
            "expected LIDT escape prefix 0F 01"
        );
        // ModRM 0x1A = mod=00, reg=011 (LIDT), rm=010 (EDX)
        assert!(code.contains(&0x1a), "expected ModRM 1A for LIDT [EDX]");
    }

    #[test]
    fn lowers_read_eflags_emits_pushfd_pop_eax() {
        // PUSHFD = 9C, POP EAX = 58
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::ReadEflags,
                    argument_count: 0,
                    result: Some(NativeType::I32),
                },
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(code.contains(&0x9c), "expected PUSHFD opcode 9C");
        // POP EAX = 58; it must follow the PUSHFD
        assert!(
            code.windows(2).any(|bytes| bytes == [0x9c, 0x58]),
            "expected PUSHFD (9C) immediately followed by POP EAX (58)"
        );
    }

    #[test]
    fn lowers_write_eflags_emits_push_eax_popfd() {
        // PUSH EAX = 50, POPFD = 9D
        let program = IrProgram {
            functions: vec![kernel(vec![
                IrInst::ConstI32(0x202), // IF=1 (interrupts enabled)
                IrInst::Intrinsic {
                    intrinsic: Intrinsic::WriteEflags,
                    argument_count: 1,
                    result: None,
                },
                IrInst::ConstI32(0),
                IrInst::Return,
            ])],
            strings: Vec::new(),
        };
        let code = lower_program(&program).unwrap().machine_code;
        assert!(code.contains(&0x9d), "expected POPFD opcode 9D");
        assert!(
            code.windows(2).any(|bytes| bytes == [0x50, 0x9d]),
            "expected PUSH EAX (50) immediately followed by POPFD (9D)"
        );
    }
}
