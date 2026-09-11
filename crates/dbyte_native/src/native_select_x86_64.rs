//! Deterministic Native MIR instruction selection for DBYTE64.
//!
//! This module contains no HIR or source-name logic. It accepts validated,
//! target-sized MIR plus a register allocation and emits only machine bytes,
//! local branch patches, and explicit external call relocations.

use crate::native_mir::{
    allocate_registers, BlockId, MachineType, NativeBinaryOp, NativeCallee, NativeCompareOp,
    NativeMirError, NativeMirFunction, NativeMirInst, NativeMirProject, NativeOptimizationLevel,
    NativeTerminator, NativeUnaryOp, NativeValue, PhysicalRegister, RegisterAllocation,
    RegisterLocation, RegisterPolicy, VirtualRegister,
};
use crate::x86_64::{Condition, EncodeError, Encoder, Register, ShiftKind, Width, XmmRegister};
use std::collections::{BTreeMap, BTreeSet};

const SCRATCH_VALUE: Register = Register::Rax;
const SCRATCH_OTHER: Register = Register::R11;
const AGGREGATE_TEMP: Register = Register::R10;
const SHIFT_COUNT: Register = Register::Rcx;
const MAX_FUNCTION_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
enum CFloatLocation {
    Gpr(usize),
    Xmm(usize),
    Stack(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeRelocation {
    pub offset: u32,
    pub symbol: String,
    pub kind: NativeRelocationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeRelocationKind {
    Pc32,
    Plt32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeCodegenStatistics {
    pub mir_instructions: u32,
    pub selected_instructions: u32,
    pub text_bytes: u32,
    pub frame_bytes: u32,
    pub virtual_registers: u32,
    pub assigned_registers: u32,
    pub spills: u32,
    pub reloads: u32,
    pub loads: u32,
    pub stores: u32,
    pub calls: u32,
    pub branches: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedNativeFunction {
    pub symbol: String,
    pub bytes: Vec<u8>,
    pub relocations: Vec<NativeRelocation>,
    pub statistics: NativeCodegenStatistics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedNativeObject {
    pub object: crate::object64::ObjectArtifact,
    pub mir_dump: String,
    pub machine_dump: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedInterruptStub {
    bytes: Vec<u8>,
    relocations: Vec<NativeRelocation>,
}

fn interrupt_body_symbol(symbol: &str) -> String {
    format!("__dbyte_interrupt_{symbol}_body")
}

fn metadata_symbol(name: String) -> crate::object64::Symbol {
    crate::object64::Symbol {
        name,
        binding: crate::object64::Binding::Local,
        kind: crate::object64::SymbolKind::Function,
        definition: crate::object64::Definition::Defined {
            section: crate::object64::Section::Text,
            value: 0,
        },
        size: 0,
    }
}

fn function_interrupt_class(
    function: &NativeMirFunction,
) -> Result<Option<dbyte_ast::InterruptClass>, NativeMirError> {
    let mut found = None;
    for block in &function.blocks {
        for instruction in &block.instructions {
            if let NativeMirInst::InterruptDefinition { class } = instruction.instruction {
                if found.replace(class).is_some() {
                    return Err(codegen_error(format!(
                        "function '{}' contains duplicate interrupt definition metadata",
                        function.symbol
                    )));
                }
            }
        }
    }
    Ok(found)
}

fn select_interrupt_stub(
    symbol: &str,
    class: dbyte_ast::InterruptClass,
) -> Result<SelectedInterruptStub, NativeMirError> {
    const SAVED: [Register; 15] = [
        Register::Rax,
        Register::Rbx,
        Register::Rcx,
        Register::Rdx,
        Register::Rbp,
        Register::Rsi,
        Register::Rdi,
        Register::R8,
        Register::R9,
        Register::R10,
        Register::R11,
        Register::R12,
        Register::R13,
        Register::R14,
        Register::R15,
    ];
    const SAVED_BYTES: i32 = 15 * 8;

    let mut encoder = Encoder::new(1024);
    for register in SAVED {
        encoder.push(register).map_err(encode_error)?;
    }

    // Interrupts may arrive at any instruction boundary, so do not assume the
    // interrupted RSP satisfies the ordinary DBYTE64 call-site alignment.
    // Preserve the post-save stack base in private scratch storage, then align
    // the temporary call stack without disturbing the architectural frame.
    encoder
        .mov_reg(Register::R11, Register::Rsp, Width::Qword)
        .map_err(encode_error)?;
    encoder
        .mov_imm64(Register::R10, !0x0fu64)
        .map_err(encode_error)?;
    encoder
        .and_reg(Register::Rsp, Register::R10, Width::Qword)
        .map_err(encode_error)?;
    encoder.sub_imm32(Register::Rsp, 16).map_err(encode_error)?;
    encoder
        .mov_base_disp32_from_reg(Register::Rsp, 0, Register::R11, Width::Qword)
        .map_err(encode_error)?;

    let frame_displacement = SAVED_BYTES + if class.has_error_code() { 8 } else { 0 };
    encoder
        .lea_base_disp32(Register::Rax, Register::R11, frame_displacement)
        .map_err(encode_error)?;
    if class.has_error_code() {
        encoder
            .mov_reg_from_base_disp32(Register::Rdx, Register::R11, SAVED_BYTES, Width::Qword)
            .map_err(encode_error)?;
    }

    let immediate = encoder.call_relative_placeholder().map_err(encode_error)?;
    encoder
        .mov_reg_from_base_disp32(Register::R11, Register::Rsp, 0, Width::Qword)
        .map_err(encode_error)?;
    encoder
        .mov_reg(Register::Rsp, Register::R11, Width::Qword)
        .map_err(encode_error)?;
    for register in SAVED.into_iter().rev() {
        encoder.pop(register).map_err(encode_error)?;
    }
    if class.has_error_code() {
        encoder.add_imm32(Register::Rsp, 8).map_err(encode_error)?;
    }
    encoder.iretq().map_err(encode_error)?;

    Ok(SelectedInterruptStub {
        bytes: encoder.into_bytes(),
        relocations: vec![NativeRelocation {
            offset: u32::try_from(immediate)
                .map_err(|_| codegen_error("interrupt body relocation exceeds u32"))?,
            symbol: interrupt_body_symbol(symbol),
            kind: NativeRelocationKind::Pc32,
        }],
    })
}

pub fn select_project_object(
    project: &NativeMirProject,
    target: crate::target::TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<SelectedNativeObject, NativeMirError> {
    if target.machine != crate::target::MachineTarget::X86_64 || project.pointer_bytes != 8 {
        return Err(codegen_error(
            "DBYTE64 selection requires an x86_64 target and layout",
        ));
    }
    let mut optimized = project.clone();
    crate::native_mir::legalize_project(&mut optimized, target)?;
    optimized.optimize(level)?;
    let mut text = Vec::new();
    let mut rodata = Vec::new();
    let mut data = Vec::new();
    let mut bss_size = 0u64;
    let mut symbols = Vec::new();
    let mut relocations = Vec::new();
    let mut defined = BTreeSet::new();
    let mut totals = NativeCodegenStatistics::default();
    let mut interrupt_metadata = BTreeSet::new();
    for function in &optimized.functions {
        while text.len() % 16 != 0 {
            text.push(0x90);
        }
        let base = text.len() as u64;
        let interrupt = function_interrupt_class(function)?;
        let stub = interrupt
            .map(|class| select_interrupt_stub(&function.symbol, class))
            .transpose()?;
        if let Some(stub) = &stub {
            for relocation in &stub.relocations {
                relocations.push(crate::object64::Relocation {
                    section: crate::object64::Section::Text,
                    offset: base + u64::from(relocation.offset),
                    symbol: relocation.symbol.clone(),
                    kind: match relocation.kind {
                        NativeRelocationKind::Pc32 => crate::object64::RelocationKind::Pc32,
                        NativeRelocationKind::Plt32 => crate::object64::RelocationKind::Plt32,
                    },
                    addend: -4,
                });
            }
            text.extend_from_slice(&stub.bytes);
        }
        let body_base = text.len() as u64;
        let selected = select_function_for_target(function, target, level)?;
        defined.insert(function.symbol.clone());
        symbols.push(crate::object64::Symbol {
            name: function.symbol.clone(),
            binding: crate::object64::Binding::Global,
            kind: crate::object64::SymbolKind::Function,
            definition: crate::object64::Definition::Defined {
                section: crate::object64::Section::Text,
                value: base,
            },
            size: stub
                .as_ref()
                .map_or(selected.bytes.len() as u64, |stub| stub.bytes.len() as u64),
        });
        if let Some(class) = interrupt {
            let body_name = interrupt_body_symbol(&function.symbol);
            defined.insert(body_name.clone());
            symbols.push(crate::object64::Symbol {
                name: body_name,
                binding: crate::object64::Binding::Local,
                kind: crate::object64::SymbolKind::Function,
                definition: crate::object64::Definition::Defined {
                    section: crate::object64::Section::Text,
                    value: body_base,
                },
                size: selected.bytes.len() as u64,
            });
            interrupt_metadata.insert(format!("__dbyte_interrupt:def:{class}:{}", function.symbol));
        }
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let NativeMirInst::InterruptAddress {
                    vector,
                    symbol,
                    class,
                    ..
                } = &instruction.instruction
                {
                    interrupt_metadata.insert(format!(
                        "__dbyte_interrupt:use:{vector:03}:{class}:{symbol}"
                    ));
                }
            }
        }
        for relocation in &selected.relocations {
            relocations.push(crate::object64::Relocation {
                section: crate::object64::Section::Text,
                offset: body_base + u64::from(relocation.offset),
                symbol: relocation.symbol.clone(),
                kind: match relocation.kind {
                    NativeRelocationKind::Pc32 => crate::object64::RelocationKind::Pc32,
                    NativeRelocationKind::Plt32 => crate::object64::RelocationKind::Plt32,
                },
                addend: -4,
            });
        }
        add_statistics(&mut totals, &selected.statistics)?;
        text.extend(selected.bytes);
    }
    symbols.extend(interrupt_metadata.into_iter().map(metadata_symbol));
    for global in &optimized.globals {
        let (section, value) = if global.bytes.is_empty() {
            bss_size = align_up_u64(bss_size, u64::from(global.alignment))?;
            let value = bss_size;
            bss_size = bss_size
                .checked_add(u64::from(global.size))
                .ok_or_else(|| codegen_error("native BSS exceeds u64"))?;
            (crate::object64::Section::Bss, value)
        } else {
            let output = if global.mutable {
                &mut data
            } else {
                &mut rodata
            };
            align_bytes(output, global.alignment)?;
            let value = output.len() as u64;
            output.extend_from_slice(&global.bytes);
            output.resize(
                output
                    .len()
                    .checked_add(global.size as usize - global.bytes.len())
                    .ok_or_else(|| codegen_error("native global bytes exceed usize"))?,
                0,
            );
            (
                if global.mutable {
                    crate::object64::Section::Data
                } else {
                    crate::object64::Section::Rodata
                },
                value,
            )
        };
        defined.insert(global.symbol.clone());
        symbols.push(crate::object64::Symbol {
            name: global.symbol.clone(),
            binding: crate::object64::Binding::Global,
            kind: crate::object64::SymbolKind::Object,
            definition: crate::object64::Definition::Defined { section, value },
            size: u64::from(global.size),
        });
    }
    let undefined: BTreeSet<_> = relocations
        .iter()
        .map(|relocation| relocation.symbol.clone())
        .filter(|symbol| !defined.contains(symbol))
        .collect();
    symbols.extend(undefined.into_iter().map(|name| crate::object64::Symbol {
        name,
        binding: crate::object64::Binding::Global,
        kind: crate::object64::SymbolKind::Function,
        definition: crate::object64::Definition::Undefined,
        size: 0,
    }));
    let entry_abi = optimized
        .functions
        .iter()
        .find(|function| function.symbol == "main")
        .map(|function| match function.parameters.len() {
            0 => Ok(crate::object64::EntryAbi::ZeroArguments),
            3 => Ok(crate::object64::EntryAbi::LinuxProcessArguments),
            _ => Err(codegen_error(
                "Linux main has unsupported DBYTE64 entry arity",
            )),
        })
        .transpose()?;
    let mut sections = BTreeMap::from([(
        crate::object64::Section::Text,
        crate::object64::SectionData {
            contents: crate::object64::Contents::Bytes(text),
            alignment: 16,
        },
    )]);
    if !rodata.is_empty() {
        sections.insert(
            crate::object64::Section::Rodata,
            crate::object64::SectionData {
                contents: crate::object64::Contents::Bytes(rodata),
                alignment: 8,
            },
        );
    }
    if !data.is_empty() {
        sections.insert(
            crate::object64::Section::Data,
            crate::object64::SectionData {
                contents: crate::object64::Contents::Bytes(data),
                alignment: 8,
            },
        );
    }
    if bss_size != 0 {
        sections.insert(
            crate::object64::Section::Bss,
            crate::object64::SectionData {
                contents: crate::object64::Contents::Uninitialized(bss_size),
                alignment: 8,
            },
        );
    }
    let image = crate::object64::ObjectImage {
        target,
        entry_abi,
        sections,
        symbols,
        relocations,
    };
    let artifact = match target.object_format {
        crate::target::ObjectFormat::Elf64 => crate::object64::write_relocatable(&image)
            .map_err(|error| codegen_error(error.to_string()))?,
        crate::target::ObjectFormat::CoffAmd64 => {
            crate::coff64::write_object(&image).map_err(|error| codegen_error(error.to_string()))?
        }
        crate::target::ObjectFormat::Elf32 => {
            return Err(codegen_error("x86_64 selection cannot write ELF32"));
        }
    };
    Ok(SelectedNativeObject {
        machine_dump: format_statistics(level, &totals),
        mir_dump: format!("optimization={level}\n{}", optimized.dump()),
        object: artifact,
    })
}

fn add_statistics(
    total: &mut NativeCodegenStatistics,
    value: &NativeCodegenStatistics,
) -> Result<(), NativeMirError> {
    macro_rules! add {
        ($field:ident) => {
            total.$field = total
                .$field
                .checked_add(value.$field)
                .ok_or_else(|| codegen_error("native codegen statistic exceeds u32"))?;
        };
    }
    add!(mir_instructions);
    add!(selected_instructions);
    add!(text_bytes);
    total.frame_bytes = total.frame_bytes.max(value.frame_bytes);
    add!(virtual_registers);
    add!(assigned_registers);
    add!(spills);
    add!(reloads);
    add!(loads);
    add!(stores);
    add!(calls);
    add!(branches);
    Ok(())
}

fn format_statistics(level: NativeOptimizationLevel, value: &NativeCodegenStatistics) -> String {
    format!(
        "native-codegen opt={level} mir={} selected={} text={} frame-max={} vregs={} assigned={} spills={} reloads={} loads={} stores={} calls={} branches={}\n",
        value.mir_instructions,
        value.selected_instructions,
        value.text_bytes,
        value.frame_bytes,
        value.virtual_registers,
        value.assigned_registers,
        value.spills,
        value.reloads,
        value.loads,
        value.stores,
        value.calls,
        value.branches
    )
}

pub fn select_function(
    function: &NativeMirFunction,
    level: NativeOptimizationLevel,
) -> Result<SelectedNativeFunction, NativeMirError> {
    select_function_for_target(function, crate::target::X86_64_LINUX, level)
}

pub fn select_function_for_target(
    function: &NativeMirFunction,
    target: crate::target::TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<SelectedNativeFunction, NativeMirError> {
    function.validate()?;
    let policy = dbyte64_register_policy();
    let allocation = allocate_registers(function, &policy, level)?;
    Selector::new(function, allocation, target)?.select()
}

pub fn dbyte64_register_policy() -> RegisterPolicy {
    let caller_saved = [Register::Rdi, Register::Rsi, Register::R8, Register::R9]
        .into_iter()
        .map(physical)
        .collect::<BTreeSet<_>>();
    RegisterPolicy {
        // RAX/RDX/RCX are fixed arithmetic operands, R10 is the aggregate
        // scratch, R11 is the general selector scratch, and RSP/RBP are never
        // allocatable. Call arguments are snapshotted before parallel ABI
        // moves, so the remaining caller-saved registers are safe hot-path
        // choices for intervals that do not cross a call.
        allocatable: vec![
            physical(Register::Rdi),
            physical(Register::Rsi),
            physical(Register::R8),
            physical(Register::R9),
            physical(Register::Rbx),
            physical(Register::R12),
            physical(Register::R13),
            physical(Register::R14),
            physical(Register::R15),
        ],
        allocatable_pairs: Vec::new(),
        call_clobbered: caller_saved,
        maximum_register_bytes: 8,
        pointer_bytes: 8,
    }
}

struct Selector<'a> {
    function: &'a NativeMirFunction,
    allocation: RegisterAllocation,
    encoder: Encoder,
    saved: Vec<Register>,
    frame_bytes: u32,
    sret_displacement: Option<i32>,
    stack_offsets: BTreeMap<crate::native_mir::StackObjectId, u32>,
    target: crate::target::TargetSpec,
    block_offsets: BTreeMap<BlockId, usize>,
    branch_patches: Vec<(usize, BlockId)>,
    return_patches: Vec<usize>,
    relocations: Vec<NativeRelocation>,
    statistics: NativeCodegenStatistics,
}

impl<'a> Selector<'a> {
    fn new(
        function: &'a NativeMirFunction,
        allocation: RegisterAllocation,
        target: crate::target::TargetSpec,
    ) -> Result<Self, NativeMirError> {
        let assigned: BTreeSet<_> = allocation
            .locations
            .values()
            .filter_map(|location| match location {
                RegisterLocation::Register(register) => Some(*register),
                RegisterLocation::RegisterPair { .. } => None,
                RegisterLocation::Spill { .. } => None,
            })
            .collect();
        let saved: Vec<_> = [
            Register::Rbx,
            Register::R12,
            Register::R13,
            Register::R14,
            Register::R15,
        ]
        .into_iter()
        .filter(|register| assigned.contains(&physical(*register)))
        .collect();
        let spill_bytes = allocation
            .spill_slots
            .checked_mul(8)
            .ok_or_else(|| codegen_error("native spill frame exceeds u32"))?;
        let fixed_bytes = spill_bytes
            .checked_add(if function.aggregate_result.is_some() {
                8
            } else {
                0
            })
            .ok_or_else(|| codegen_error("native fixed frame exceeds u32"))?;
        let (stack_bytes, stack_offsets) = layout_stack_objects(function, fixed_bytes)?;
        let raw_frame = fixed_bytes
            .checked_add(stack_bytes)
            .ok_or_else(|| codegen_error("native stack frame exceeds u32"))?;
        let aligned = align_up(raw_frame, 16)?;
        let frame_bytes = aligned
            .checked_add(if saved.len() % 2 == 0 { 0 } else { 8 })
            .ok_or_else(|| codegen_error("native aligned frame exceeds u32"))?;
        let sret_displacement = function
            .aggregate_result
            .map(|_| {
                let bytes = (saved.len() as u32)
                    .checked_mul(8)
                    .and_then(|value| value.checked_add(spill_bytes))
                    .and_then(|value| value.checked_add(8))
                    .ok_or_else(|| codegen_error("sret displacement exceeds u32"))?;
                i32::try_from(bytes)
                    .ok()
                    .and_then(i32::checked_neg)
                    .ok_or_else(|| codegen_error("sret displacement exceeds i32"))
            })
            .transpose()?;
        Ok(Self {
            function,
            allocation,
            encoder: Encoder::new(MAX_FUNCTION_BYTES),
            saved,
            frame_bytes,
            sret_displacement,
            stack_offsets,
            target,
            block_offsets: BTreeMap::new(),
            branch_patches: Vec::new(),
            return_patches: Vec::new(),
            relocations: Vec::new(),
            statistics: NativeCodegenStatistics::default(),
        })
    }

    fn select(mut self) -> Result<SelectedNativeFunction, NativeMirError> {
        self.emit_prologue()?;
        self.move_parameters()?;
        for (block_index, block) in self.function.blocks.iter().enumerate() {
            let next_block = self
                .function
                .blocks
                .get(block_index + 1)
                .map(|block| block.id);
            self.block_offsets.insert(block.id, self.encoder.position());
            let fused_compare = match (&block.instructions.last(), &block.terminator) {
                (
                    Some(instruction),
                    NativeTerminator::Branch {
                        condition: NativeValue::Register(condition),
                        then_block,
                        else_block,
                    },
                ) => match &instruction.instruction {
                    NativeMirInst::Compare {
                        destination,
                        op,
                        left,
                        right,
                        ty,
                    } if destination == condition
                        && self.function.register_use_count(*destination) == 1
                        && !matches!(ty, MachineType::F32 | MachineType::F64) =>
                    {
                        Some((*op, *left, *right, *ty, *then_block, *else_block))
                    }
                    _ => None,
                },
                _ => None,
            };
            let instruction_count = block.instructions.len() - usize::from(fused_compare.is_some());
            let mut instruction_index = 0usize;
            while instruction_index < instruction_count {
                let fused_binary_copy = if instruction_index + 1 < instruction_count {
                    match (
                        &block.instructions[instruction_index].instruction,
                        &block.instructions[instruction_index + 1].instruction,
                    ) {
                        (
                            NativeMirInst::Binary {
                                destination: temporary,
                                op,
                                left,
                                right,
                                ty,
                            },
                            NativeMirInst::Copy {
                                destination,
                                source,
                                ty: copy_ty,
                            },
                        ) if temporary == source
                            && ty == copy_ty
                            && self.function.register_use_count(*temporary) == 1 =>
                        {
                            Some((*destination, *op, *left, *right, *ty))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some((destination, op, left, right, ty)) = fused_binary_copy {
                    self.statistics.mir_instructions += 2;
                    self.select_binary(destination, op, left, right, ty)?;
                    instruction_index += 2;
                    continue;
                }
                let fused_load_cast = if instruction_index + 1 < instruction_count {
                    match (
                        &block.instructions[instruction_index].instruction,
                        &block.instructions[instruction_index + 1].instruction,
                    ) {
                        (
                            NativeMirInst::Load {
                                destination: loaded,
                                address,
                                ty: load_ty @ (MachineType::I8 | MachineType::I16),
                                volatile: false,
                            },
                            NativeMirInst::Cast {
                                destination,
                                value,
                                from,
                                to: MachineType::I64,
                                signed: false,
                            },
                        ) if loaded == value
                            && load_ty == from
                            && self.function.register_use_count(*loaded) == 1 =>
                        {
                            Some((*address, *load_ty, *destination))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some((address, load_ty, destination)) = fused_load_cast {
                    self.statistics.mir_instructions += 2;
                    self.select_unsigned_load_widening_cast(address, load_ty, destination)?;
                    instruction_index += 2;
                    continue;
                }
                self.statistics.mir_instructions += 1;
                self.select_instruction(&block.instructions[instruction_index].instruction)?;
                instruction_index += 1;
            }
            if let Some((op, left, right, ty, then_block, else_block)) = fused_compare {
                self.statistics.mir_instructions += 1;
                self.select_compare_branch(
                    (op, left, right, ty),
                    then_block,
                    else_block,
                    next_block,
                )?;
            } else {
                self.select_terminator(&block.terminator, next_block)?;
            }
        }
        let epilogue = self.encoder.position();
        self.emit_epilogue()?;
        for (patch, block) in &self.branch_patches {
            let target = *self
                .block_offsets
                .get(block)
                .ok_or_else(|| codegen_error("branch references an unselected block"))?;
            self.encoder
                .patch_relative_i32(*patch, target)
                .map_err(encode_error)?;
        }
        for patch in &self.return_patches {
            self.encoder
                .patch_relative_i32(*patch, epilogue)
                .map_err(encode_error)?;
        }
        self.statistics.text_bytes = u32::try_from(self.encoder.position())
            .map_err(|_| codegen_error("selected function exceeds u32 size"))?;
        self.statistics.frame_bytes = self.frame_bytes;
        self.statistics.virtual_registers = self.function.register_types.len() as u32;
        self.statistics.spills = self.allocation.spill_slots;
        self.statistics.assigned_registers = self
            .allocation
            .locations
            .values()
            .filter(|location| matches!(location, RegisterLocation::Register(_)))
            .count() as u32;
        Ok(SelectedNativeFunction {
            symbol: self.function.symbol.clone(),
            bytes: self.encoder.into_bytes(),
            relocations: self.relocations,
            statistics: self.statistics,
        })
    }

    fn emit_prologue(&mut self) -> Result<(), NativeMirError> {
        self.emit(|encoder| encoder.push(Register::Rbp))?;
        self.emit(|encoder| encoder.mov_reg(Register::Rbp, Register::Rsp, Width::Qword))?;
        for register in self.saved.clone() {
            self.emit(|encoder| encoder.push(register))?;
        }
        if self.frame_bytes != 0 {
            let frame_bytes = self.frame_bytes;
            self.emit(|encoder| encoder.sub_imm32(Register::Rsp, frame_bytes))?;
        }
        Ok(())
    }

    fn emit_epilogue(&mut self) -> Result<(), NativeMirError> {
        if self.frame_bytes != 0 {
            let frame_bytes = self.frame_bytes;
            self.emit(|encoder| encoder.add_imm32(Register::Rsp, frame_bytes))?;
        }
        for register in self.saved.clone().into_iter().rev() {
            self.emit(|encoder| encoder.pop(register))?;
        }
        self.emit(|encoder| encoder.pop(Register::Rbp))?;
        self.emit(Encoder::ret)
    }

    fn move_parameters(&mut self) -> Result<(), NativeMirError> {
        let parameters = abi_argument_registers(self.function.convention, self.target)?;
        let win64 = self.function.convention == dbyte_ast::CallConvention::C
            && self.target.c_abi == Some(crate::target::CAbi::Win64);
        let sysv64 = self.function.convention == dbyte_ast::CallConvention::C
            && self.target.c_abi == Some(crate::target::CAbi::SysV64);
        if self.function.parameter_aggregates.len() != self.function.parameters.len() {
            return Err(codegen_error(
                "native parameter ABI metadata length mismatch",
            ));
        }
        if self.function.convention == dbyte_ast::CallConvention::C
            && self
                .function
                .parameter_aggregates
                .iter()
                .any(Option::is_some)
        {
            return Err(unsupported("aggregate by-value C parameter"));
        }
        if self.function.convention == dbyte_ast::CallConvention::C
            && self.function.parameters.iter().any(|parameter| {
                matches!(
                    self.function.register_types[parameter.0 as usize],
                    MachineType::F32 | MachineType::F64
                )
            })
        {
            return self.move_c_float_parameters();
        }
        let mut scalar_index = usize::from(self.function.aggregate_result.is_some());
        let mut stack_offset = 0u32;
        if let Some(displacement) = self.sret_displacement {
            let source = *parameters
                .first()
                .ok_or_else(|| codegen_error("sret ABI has no argument register"))?;
            self.emit(|encoder| {
                encoder.mov_base_disp32_from_reg(Register::Rbp, displacement, source, Width::Qword)
            })?;
            self.statistics.stores += 1;
        }
        let register_snapshot_start = scalar_index;
        let scalar_parameter_count = self
            .function
            .parameter_aggregates
            .iter()
            .filter(|aggregate| aggregate.is_none())
            .count();
        let register_snapshot_count =
            scalar_parameter_count.min(parameters.len().saturating_sub(register_snapshot_start));
        for source in parameters
            [register_snapshot_start..register_snapshot_start + register_snapshot_count]
            .iter()
            .copied()
        {
            self.emit(|encoder| encoder.push(source))?;
        }
        let register_snapshot_bytes = u32::try_from(register_snapshot_count)
            .map_err(|_| codegen_error("parameter snapshot count exceeds u32"))?
            .checked_mul(8)
            .ok_or_else(|| codegen_error("parameter snapshot area overflows"))?;
        for (parameter, aggregate) in self
            .function
            .parameters
            .iter()
            .copied()
            .zip(self.function.parameter_aggregates.iter().copied())
        {
            if let Some(layout) = aggregate {
                let displacement = 16i32
                    .checked_add(
                        i32::try_from(stack_offset)
                            .map_err(|_| codegen_error("aggregate parameter offset exceeds i32"))?,
                    )
                    .ok_or_else(|| codegen_error("aggregate parameter offset overflows"))?;
                self.emit(|encoder| {
                    encoder.lea_base_disp32(SCRATCH_OTHER, Register::Rbp, displacement)
                })?;
                stack_offset = stack_offset
                    .checked_add(aggregate_stack_bytes(layout)?)
                    .ok_or_else(|| codegen_error("aggregate parameter area overflows"))?;
                self.store_register(parameter, SCRATCH_OTHER)?;
                continue;
            }
            if scalar_index < parameters.len() {
                let snapshot_index = scalar_index
                    .checked_sub(register_snapshot_start)
                    .ok_or_else(|| codegen_error("parameter snapshot index underflows"))?;
                if snapshot_index >= register_snapshot_count {
                    return Err(codegen_error(
                        "parameter snapshot index exceeds captured registers",
                    ));
                }
                let displacement =
                    i32::try_from((register_snapshot_count - snapshot_index - 1) * 8)
                        .map_err(|_| codegen_error("parameter snapshot offset exceeds i32"))?;
                self.emit(|encoder| {
                    encoder.mov_reg_from_base_disp32(
                        SCRATCH_VALUE,
                        Register::Rsp,
                        displacement,
                        Width::Qword,
                    )
                })?;
                self.statistics.loads += 1;
            } else {
                let extra = i32::try_from((scalar_index - parameters.len()) * 8)
                    .map_err(|_| codegen_error("stack parameter offset exceeds i32"))?;
                let displacement = if win64 {
                    48i32
                        .checked_add(extra)
                        .ok_or_else(|| codegen_error("Win64 parameter offset overflows"))?
                } else if sysv64 {
                    16i32
                        .checked_add(extra)
                        .ok_or_else(|| codegen_error("SysV64 parameter offset overflows"))?
                } else {
                    return Err(unsupported("stack scalar native parameters"));
                };
                self.emit(|encoder| {
                    encoder.mov_reg_from_base_disp32(
                        SCRATCH_VALUE,
                        Register::Rbp,
                        displacement,
                        Width::Qword,
                    )
                })?;
                self.statistics.loads += 1;
            }
            self.normalize(
                SCRATCH_VALUE,
                self.function.register_types[parameter.0 as usize],
            )?;
            self.store_register(parameter, SCRATCH_VALUE)?;
            scalar_index += 1;
        }
        if register_snapshot_bytes != 0 {
            self.emit(|encoder| encoder.add_imm32(Register::Rsp, register_snapshot_bytes))?;
        }
        if let (Some(register), Some(displacement)) = (
            self.function.aggregate_result_register,
            self.sret_displacement,
        ) {
            self.emit(|encoder| {
                encoder.mov_reg_from_base_disp32(
                    SCRATCH_VALUE,
                    Register::Rbp,
                    displacement,
                    Width::Qword,
                )
            })?;
            self.statistics.loads += 1;
            self.store_register(register, SCRATCH_VALUE)?;
        }
        Ok(())
    }

    fn move_c_float_parameters(&mut self) -> Result<(), NativeMirError> {
        let win64 = self.target.c_abi == Some(crate::target::CAbi::Win64);
        let gprs = if win64 {
            &[Register::Rcx, Register::Rdx, Register::R8, Register::R9][..]
        } else {
            &[
                Register::Rdi,
                Register::Rsi,
                Register::Rdx,
                Register::Rcx,
                Register::R8,
                Register::R9,
            ][..]
        };
        let mut integer_index = 0usize;
        let mut float_index = 0usize;
        let mut stack_index = 0usize;
        let mut placements = Vec::with_capacity(self.function.parameters.len());
        for (index, parameter) in self.function.parameters.iter().copied().enumerate() {
            let ty = self.function.register_types[parameter.0 as usize];
            let is_float = matches!(ty, MachineType::F32 | MachineType::F64);
            let location = if win64 {
                if index < 4 {
                    if is_float {
                        CFloatLocation::Xmm(index)
                    } else {
                        CFloatLocation::Gpr(index)
                    }
                } else {
                    let slot = stack_index;
                    stack_index += 1;
                    CFloatLocation::Stack(slot)
                }
            } else if is_float && float_index < 8 {
                let slot = float_index;
                float_index += 1;
                CFloatLocation::Xmm(slot)
            } else if !is_float && integer_index < 6 {
                let slot = integer_index;
                integer_index += 1;
                CFloatLocation::Gpr(slot)
            } else {
                let slot = stack_index;
                stack_index += 1;
                CFloatLocation::Stack(slot)
            };
            placements.push((parameter, ty, location));
        }
        let register_count = placements
            .iter()
            .filter(|(_, _, location)| !matches!(location, CFloatLocation::Stack(_)))
            .count();
        for (_, ty, location) in &placements {
            match *location {
                CFloatLocation::Gpr(slot) => self.emit(|encoder| encoder.push(gprs[slot]))?,
                CFloatLocation::Xmm(slot) => {
                    let source = xmm_register(slot)?;
                    let source_width = width(*ty);
                    self.emit(|encoder| {
                        encoder.mov_gpr_from_xmm(SCRATCH_VALUE, source, source_width)
                    })?;
                    self.emit(|encoder| encoder.push(SCRATCH_VALUE))?;
                }
                CFloatLocation::Stack(_) => {}
            }
        }
        let mut snapshot_index = 0usize;
        for (parameter, ty, location) in placements {
            match location {
                CFloatLocation::Gpr(_) | CFloatLocation::Xmm(_) => {
                    let displacement = i32::try_from((register_count - snapshot_index - 1) * 8)
                        .map_err(|_| {
                            codegen_error("C ABI parameter snapshot offset exceeds i32")
                        })?;
                    self.emit(|encoder| {
                        encoder.mov_reg_from_base_disp32(
                            SCRATCH_VALUE,
                            Register::Rsp,
                            displacement,
                            Width::Qword,
                        )
                    })?;
                    snapshot_index += 1;
                }
                CFloatLocation::Stack(slot) => {
                    let base = if win64 { 48 } else { 16 };
                    let displacement = base
                        + i32::try_from(slot * 8).map_err(|_| {
                            codegen_error("C ABI stack parameter offset exceeds i32")
                        })?;
                    self.emit(|encoder| {
                        encoder.mov_reg_from_base_disp32(
                            SCRATCH_VALUE,
                            Register::Rbp,
                            displacement,
                            Width::Qword,
                        )
                    })?;
                }
            }
            self.normalize(SCRATCH_VALUE, ty)?;
            self.store_register(parameter, SCRATCH_VALUE)?;
            self.statistics.loads += 1;
        }
        if register_count != 0 {
            let snapshot_bytes = u32::try_from(register_count)
                .map_err(|_| codegen_error("C ABI snapshot count exceeds u32"))?
                .checked_mul(8)
                .ok_or_else(|| codegen_error("C ABI snapshot bytes overflow"))?;
            self.emit(|encoder| encoder.add_imm32(Register::Rsp, snapshot_bytes))?;
        }
        Ok(())
    }

    fn select_unsigned_load_widening_cast(
        &mut self,
        address: VirtualRegister,
        load_ty: MachineType,
        destination: VirtualRegister,
    ) -> Result<(), NativeMirError> {
        debug_assert!(matches!(load_ty, MachineType::I8 | MachineType::I16));
        let target = match self.location(destination)? {
            RegisterLocation::Register(register) => machine_register(register),
            RegisterLocation::Spill { .. } => SCRATCH_VALUE,
            RegisterLocation::RegisterPair { .. } => {
                return Err(codegen_error("x86_64 scalar has register-pair allocation"));
            }
        };
        let base = match self.location(address)? {
            RegisterLocation::Register(register) => machine_register(register),
            RegisterLocation::Spill { .. } => {
                self.load_register(address, SCRATCH_OTHER)?;
                SCRATCH_OTHER
            }
            RegisterLocation::RegisterPair { .. } => {
                return Err(codegen_error("x86_64 address has register-pair allocation"));
            }
        };
        self.emit(|encoder| encoder.movzx_reg_from_base_disp32(target, base, 0, width(load_ty)))?;
        self.statistics.loads += 1;
        if target == SCRATCH_VALUE {
            self.store_register(destination, target)?;
        }
        Ok(())
    }

    fn select_instruction(&mut self, instruction: &NativeMirInst) -> Result<(), NativeMirError> {
        match instruction {
            NativeMirInst::Const {
                destination, value, ..
            } => {
                let target = match self.location(*destination)? {
                    RegisterLocation::Register(register) => machine_register(register),
                    RegisterLocation::Spill { .. } => SCRATCH_VALUE,
                    RegisterLocation::RegisterPair { .. } => {
                        return Err(codegen_error("x86_64 scalar has register-pair allocation"));
                    }
                };
                self.emit_immediate(target, *value)?;
                if target == SCRATCH_VALUE {
                    self.store_register(*destination, target)?;
                }
                Ok(())
            }
            NativeMirInst::Copy {
                destination,
                source,
                ..
            } => match (self.location(*source)?, self.location(*destination)?) {
                (RegisterLocation::Register(source), RegisterLocation::Register(destination)) => {
                    let source = machine_register(source);
                    let destination = machine_register(destination);
                    if source != destination {
                        self.emit(|encoder| encoder.mov_reg(destination, source, Width::Qword))?;
                    }
                    Ok(())
                }
                _ => {
                    self.load_register(*source, SCRATCH_VALUE)?;
                    self.store_register(*destination, SCRATCH_VALUE)
                }
            },
            NativeMirInst::Binary {
                destination,
                op,
                left,
                right,
                ty,
            } => self.select_binary(*destination, *op, *left, *right, *ty),
            NativeMirInst::Unary {
                destination,
                op,
                value,
                ty,
            } => {
                self.load_value(*value, SCRATCH_VALUE)?;
                match op {
                    NativeUnaryOp::Negate => {
                        if *ty == MachineType::F32 {
                            self.emit(|encoder| {
                                encoder.xor_reg_imm(SCRATCH_VALUE, 0x8000_0000, Width::Dword)
                            })?;
                        } else if *ty == MachineType::F64 {
                            self.emit(|encoder| {
                                encoder.mov_imm64(SCRATCH_OTHER, 0x8000_0000_0000_0000)
                            })?;
                            self.emit(|encoder| {
                                encoder.xor_reg(SCRATCH_VALUE, SCRATCH_OTHER, Width::Qword)
                            })?;
                        } else {
                            self.emit(|encoder| encoder.neg_reg(SCRATCH_VALUE, width(*ty)))?;
                        }
                    }
                    NativeUnaryOp::BitNot => {
                        self.emit(|encoder| encoder.not_reg(SCRATCH_VALUE, width(*ty)))?
                    }
                    NativeUnaryOp::BoolNot => {
                        self.emit(|encoder| {
                            encoder.test_reg(SCRATCH_VALUE, SCRATCH_VALUE, Width::Qword)
                        })?;
                        self.emit(|encoder| {
                            encoder.set_condition(Condition::Equal, SCRATCH_VALUE)
                        })?;
                        self.emit(|encoder| {
                            encoder.movzx_byte_to_qword(SCRATCH_VALUE, SCRATCH_VALUE)
                        })?;
                    }
                }
                self.normalize(SCRATCH_VALUE, *ty)?;
                self.store_register(*destination, SCRATCH_VALUE)
            }
            NativeMirInst::Cast {
                destination,
                value,
                from,
                to,
                signed,
            } => {
                self.load_register(*value, SCRATCH_VALUE)?;
                self.extend_or_truncate(SCRATCH_VALUE, *from, *to, *signed)?;
                self.store_register(*destination, SCRATCH_VALUE)
            }
            NativeMirInst::Compare {
                destination,
                op,
                left,
                right,
                ty,
            } => {
                if matches!(ty, MachineType::F32 | MachineType::F64) {
                    return self.select_float_compare(*destination, *op, *left, *right, *ty);
                }
                self.load_value(*left, SCRATCH_VALUE)?;
                if let NativeValue::Immediate(value) = right {
                    if let Some(immediate) = immediate_u32(*value, *ty) {
                        self.emit(|encoder| {
                            encoder.cmp_reg_imm(SCRATCH_VALUE, immediate, width(*ty))
                        })?;
                    } else {
                        self.load_value(*right, SCRATCH_OTHER)?;
                        self.emit(|encoder| {
                            encoder.cmp_reg(SCRATCH_VALUE, SCRATCH_OTHER, width(*ty))
                        })?;
                    }
                } else {
                    self.load_value(*right, SCRATCH_OTHER)?;
                    self.emit(|encoder| encoder.cmp_reg(SCRATCH_VALUE, SCRATCH_OTHER, width(*ty)))?;
                }
                self.emit(|encoder| encoder.set_condition(compare_condition(*op), SCRATCH_VALUE))?;
                self.emit(|encoder| encoder.movzx_byte_to_qword(SCRATCH_VALUE, SCRATCH_VALUE))?;
                self.store_register(*destination, SCRATCH_VALUE)
            }
            NativeMirInst::Call {
                destination,
                callee,
                arguments,
                argument_aggregates,
                convention,
                aggregate_result,
                ..
            } => self.select_call(
                *destination,
                callee,
                arguments,
                argument_aggregates,
                *convention,
                *aggregate_result,
            ),
            NativeMirInst::StackAddress {
                destination,
                object,
            } => {
                let displacement = self.stack_object_displacement(*object)?;
                let target = match self.location(*destination)? {
                    RegisterLocation::Register(register) => machine_register(register),
                    RegisterLocation::Spill { .. } => SCRATCH_VALUE,
                    RegisterLocation::RegisterPair { .. } => {
                        return Err(codegen_error("x86_64 scalar has register-pair allocation"));
                    }
                };
                self.emit(|encoder| encoder.lea_base_disp32(target, Register::Rbp, displacement))?;
                if target == SCRATCH_VALUE {
                    self.store_register(*destination, target)?;
                }
                Ok(())
            }
            NativeMirInst::GlobalAddress {
                destination,
                symbol,
            }
            | NativeMirInst::InterruptAddress {
                destination,
                symbol,
                ..
            }
            | NativeMirInst::TaskAddress {
                destination,
                symbol,
            } => {
                let target = match self.location(*destination)? {
                    RegisterLocation::Register(register) => machine_register(register),
                    RegisterLocation::Spill { .. } => SCRATCH_VALUE,
                    RegisterLocation::RegisterPair { .. } => {
                        return Err(codegen_error("x86_64 scalar has register-pair allocation"));
                    }
                };
                let offset = self
                    .encoder
                    .lea_rip_relative_placeholder(target)
                    .map_err(encode_error)?;
                self.statistics.selected_instructions += 1;
                self.relocations.push(NativeRelocation {
                    offset: u32::try_from(offset)
                        .map_err(|_| codegen_error("global relocation exceeds u32"))?,
                    symbol: symbol.clone(),
                    kind: NativeRelocationKind::Pc32,
                });
                if target == SCRATCH_VALUE {
                    self.store_register(*destination, target)?;
                }
                Ok(())
            }
            NativeMirInst::AddressOffset {
                destination,
                base,
                index,
                scale,
                displacement,
            } => {
                self.load_register(*base, SCRATCH_VALUE)?;
                if let Some(index) = index {
                    self.load_register(*index, SCRATCH_OTHER)?;
                    if *scale != 1 {
                        self.emit_immediate(SHIFT_COUNT, u64::from(*scale))?;
                        self.emit(|encoder| {
                            encoder.imul_reg(SCRATCH_OTHER, SHIFT_COUNT, Width::Qword)
                        })?;
                    }
                    self.emit(|encoder| {
                        encoder.add_reg(SCRATCH_VALUE, SCRATCH_OTHER, Width::Qword)
                    })?;
                }
                if *displacement > 0 {
                    self.emit(|encoder| encoder.add_imm32(SCRATCH_VALUE, *displacement as u32))?;
                } else if *displacement < 0 {
                    self.emit(|encoder| {
                        encoder.sub_imm32(SCRATCH_VALUE, displacement.unsigned_abs())
                    })?;
                }
                self.store_register(*destination, SCRATCH_VALUE)
            }
            NativeMirInst::Load {
                destination,
                address,
                ty,
                ..
            } => {
                let target = match self.location(*destination)? {
                    RegisterLocation::Register(register) => machine_register(register),
                    RegisterLocation::Spill { .. } => SCRATCH_VALUE,
                    RegisterLocation::RegisterPair { .. } => {
                        return Err(codegen_error("x86_64 scalar has register-pair allocation"));
                    }
                };
                let base = match self.location(*address)? {
                    RegisterLocation::Register(register) => machine_register(register),
                    RegisterLocation::Spill { .. } => {
                        self.load_register(*address, SCRATCH_OTHER)?;
                        SCRATCH_OTHER
                    }
                    RegisterLocation::RegisterPair { .. } => {
                        return Err(codegen_error("x86_64 address has register-pair allocation"));
                    }
                };
                match width(*ty) {
                    Width::Byte | Width::Word => self.emit(|encoder| {
                        encoder.movzx_reg_from_base_disp32(target, base, 0, width(*ty))
                    })?,
                    operation_width => self.emit(|encoder| {
                        encoder.mov_reg_from_base_disp32(target, base, 0, operation_width)
                    })?,
                }
                self.statistics.loads += 1;
                if target == SCRATCH_VALUE {
                    self.store_register(*destination, target)?;
                }
                Ok(())
            }
            NativeMirInst::Store {
                address, value, ty, ..
            } => {
                self.load_register(*address, SCRATCH_OTHER)?;
                self.load_register(*value, SCRATCH_VALUE)?;
                self.emit(|encoder| {
                    encoder.mov_base_disp32_from_reg(SCRATCH_OTHER, 0, SCRATCH_VALUE, width(*ty))
                })?;
                self.statistics.stores += 1;
                Ok(())
            }
            NativeMirInst::AggregateCopy {
                destination,
                source,
                bytes,
                ..
            } => self.select_aggregate_copy(*destination, *source, *bytes),
            NativeMirInst::AggregateZero {
                destination, bytes, ..
            } => self.select_aggregate_zero(*destination, *bytes),
            NativeMirInst::Intrinsic {
                destination,
                intrinsic,
                arguments,
                ..
            } => self.select_intrinsic(*destination, *intrinsic, arguments),
            NativeMirInst::InterruptDefinition { .. } => Ok(()),
        }
    }

    fn select_binary(
        &mut self,
        destination: VirtualRegister,
        op: NativeBinaryOp,
        left: NativeValue,
        right: NativeValue,
        ty: MachineType,
    ) -> Result<(), NativeMirError> {
        if matches!(ty, MachineType::F32 | MachineType::F64) {
            return self.select_float_binary(destination, op, left, right, ty);
        }
        let in_place = if matches!(ty, MachineType::I64 | MachineType::Pointer)
            && !matches!(
                op,
                NativeBinaryOp::SignedDiv
                    | NativeBinaryOp::UnsignedDiv
                    | NativeBinaryOp::SignedRem
                    | NativeBinaryOp::UnsignedRem
            ) {
            match (self.location(destination)?, left) {
                (
                    RegisterLocation::Register(destination_location),
                    NativeValue::Register(left_register),
                ) if self.location(left_register)?
                    == RegisterLocation::Register(destination_location) =>
                {
                    Some(machine_register(destination_location))
                }
                _ => None,
            }
        } else {
            None
        };
        let accumulator = in_place.unwrap_or(SCRATCH_VALUE);
        if in_place.is_none() {
            self.load_value(left, accumulator)?;
        }
        let operation_width = width(ty);
        if let NativeValue::Immediate(value) = right {
            if let Some(immediate) = immediate_u32(value, ty) {
                let selected = match op {
                    NativeBinaryOp::Add => Some(self.emit(|encoder| {
                        encoder.add_reg_imm(accumulator, immediate, operation_width)
                    })),
                    NativeBinaryOp::Sub => Some(self.emit(|encoder| {
                        encoder.sub_reg_imm(accumulator, immediate, operation_width)
                    })),
                    NativeBinaryOp::BitAnd => Some(self.emit(|encoder| {
                        encoder.and_reg_imm(accumulator, immediate, operation_width)
                    })),
                    NativeBinaryOp::BitOr => Some(self.emit(|encoder| {
                        encoder.or_reg_imm(accumulator, immediate, operation_width)
                    })),
                    NativeBinaryOp::BitXor => Some(self.emit(|encoder| {
                        encoder.xor_reg_imm(accumulator, immediate, operation_width)
                    })),
                    NativeBinaryOp::Mul if operation_width != Width::Byte => {
                        Some(self.emit(|encoder| {
                            encoder.imul_reg_imm32(
                                accumulator,
                                accumulator,
                                immediate,
                                operation_width,
                            )
                        }))
                    }
                    _ => None,
                };
                if let Some(selected) = selected {
                    selected?;
                    self.normalize(accumulator, ty)?;
                    return self.store_register(destination, accumulator);
                }
            }
        }
        self.load_value(right, SCRATCH_OTHER)?;
        match op {
            NativeBinaryOp::Add => {
                self.emit(|encoder| encoder.add_reg(accumulator, SCRATCH_OTHER, operation_width))?
            }
            NativeBinaryOp::Sub => {
                self.emit(|encoder| encoder.sub_reg(accumulator, SCRATCH_OTHER, operation_width))?
            }
            NativeBinaryOp::Mul => {
                self.emit(|encoder| encoder.imul_reg(accumulator, SCRATCH_OTHER, operation_width))?
            }
            NativeBinaryOp::BitAnd => {
                self.emit(|encoder| encoder.and_reg(accumulator, SCRATCH_OTHER, operation_width))?
            }
            NativeBinaryOp::BitOr => {
                self.emit(|encoder| encoder.or_reg(accumulator, SCRATCH_OTHER, operation_width))?
            }
            NativeBinaryOp::BitXor => {
                self.emit(|encoder| encoder.xor_reg(accumulator, SCRATCH_OTHER, operation_width))?
            }
            NativeBinaryOp::ShiftLeft
            | NativeBinaryOp::SignedShiftRight
            | NativeBinaryOp::UnsignedShiftRight => {
                self.emit(|encoder| encoder.mov_reg(SHIFT_COUNT, SCRATCH_OTHER, Width::Qword))?;
                let kind = match op {
                    NativeBinaryOp::ShiftLeft => ShiftKind::Left,
                    NativeBinaryOp::SignedShiftRight => ShiftKind::ArithmeticRight,
                    NativeBinaryOp::UnsignedShiftRight => ShiftKind::LogicalRight,
                    _ => unreachable!(),
                };
                self.emit(|encoder| encoder.shift_reg_by_cl(accumulator, kind, operation_width))?;
            }
            NativeBinaryOp::SignedDiv
            | NativeBinaryOp::UnsignedDiv
            | NativeBinaryOp::SignedRem
            | NativeBinaryOp::UnsignedRem => {
                let signed = matches!(op, NativeBinaryOp::SignedDiv | NativeBinaryOp::SignedRem);
                if signed {
                    self.emit(|encoder| match operation_width {
                        Width::Dword => encoder.cdq(),
                        Width::Qword => encoder.cqo(),
                        Width::Word | Width::Byte => Err(EncodeError::InvalidOperand(
                            "narrow native MIR division must be legalized before selection",
                        )),
                    })?;
                } else {
                    self.emit(|encoder| {
                        encoder.xor_reg(Register::Rdx, Register::Rdx, Width::Dword)
                    })?;
                }
                self.emit(|encoder| encoder.div_reg(SCRATCH_OTHER, signed, operation_width))?;
                if matches!(op, NativeBinaryOp::SignedRem | NativeBinaryOp::UnsignedRem) {
                    self.emit(|encoder| {
                        encoder.mov_reg(SCRATCH_VALUE, Register::Rdx, Width::Qword)
                    })?;
                }
            }
        }
        self.normalize(accumulator, ty)?;
        self.store_register(destination, accumulator)
    }

    fn select_float_binary(
        &mut self,
        destination: VirtualRegister,
        op: NativeBinaryOp,
        left: NativeValue,
        right: NativeValue,
        ty: MachineType,
    ) -> Result<(), NativeMirError> {
        let operation_width = width(ty);
        self.load_value(left, SCRATCH_VALUE)?;
        self.load_value(right, SCRATCH_OTHER)?;
        self.emit(|encoder| {
            encoder.mov_xmm_from_gpr(XmmRegister::Xmm0, SCRATCH_VALUE, operation_width)
        })?;
        self.emit(|encoder| {
            encoder.mov_xmm_from_gpr(XmmRegister::Xmm1, SCRATCH_OTHER, operation_width)
        })?;
        let opcode = match op {
            NativeBinaryOp::Add => 0x58,
            NativeBinaryOp::Sub => 0x5c,
            NativeBinaryOp::Mul => 0x59,
            NativeBinaryOp::SignedDiv | NativeBinaryOp::UnsignedDiv => 0x5e,
            _ => return Err(codegen_error("unsupported scalar SSE arithmetic operation")),
        };
        self.emit(|encoder| {
            encoder.scalar_float_binary(
                XmmRegister::Xmm0,
                XmmRegister::Xmm1,
                operation_width,
                opcode,
            )
        })?;
        self.emit(|encoder| {
            encoder.mov_gpr_from_xmm(SCRATCH_VALUE, XmmRegister::Xmm0, operation_width)
        })?;
        self.normalize(SCRATCH_VALUE, ty)?;
        self.store_register(destination, SCRATCH_VALUE)
    }

    fn select_float_compare(
        &mut self,
        destination: VirtualRegister,
        op: NativeCompareOp,
        left: NativeValue,
        right: NativeValue,
        ty: MachineType,
    ) -> Result<(), NativeMirError> {
        let operation_width = width(ty);
        self.load_value(left, SCRATCH_VALUE)?;
        self.load_value(right, SCRATCH_OTHER)?;
        self.emit(|encoder| {
            encoder.mov_xmm_from_gpr(XmmRegister::Xmm0, SCRATCH_VALUE, operation_width)
        })?;
        self.emit(|encoder| {
            encoder.mov_xmm_from_gpr(XmmRegister::Xmm1, SCRATCH_OTHER, operation_width)
        })?;
        self.emit(|encoder| encoder.ucomis(XmmRegister::Xmm0, XmmRegister::Xmm1, operation_width))?;
        let (condition, ordered) = match op {
            NativeCompareOp::Equal => (Condition::Equal, Some(Condition::NotParity)),
            NativeCompareOp::NotEqual => (Condition::NotEqual, Some(Condition::Parity)),
            NativeCompareOp::UnsignedLess | NativeCompareOp::SignedLess => {
                (Condition::Below, Some(Condition::NotParity))
            }
            NativeCompareOp::UnsignedLessEqual | NativeCompareOp::SignedLessEqual => {
                (Condition::BelowOrEqual, Some(Condition::NotParity))
            }
            NativeCompareOp::UnsignedGreater | NativeCompareOp::SignedGreater => {
                (Condition::Above, None)
            }
            NativeCompareOp::UnsignedGreaterEqual | NativeCompareOp::SignedGreaterEqual => {
                (Condition::AboveOrEqual, None)
            }
        };
        self.emit(|encoder| encoder.set_condition(condition, SCRATCH_VALUE))?;
        if let Some(ordered) = ordered {
            self.emit(|encoder| encoder.set_condition(ordered, SHIFT_COUNT))?;
            self.emit(|encoder| {
                if op == NativeCompareOp::NotEqual {
                    encoder.or_reg(SCRATCH_VALUE, SHIFT_COUNT, Width::Byte)
                } else {
                    encoder.and_reg(SCRATCH_VALUE, SHIFT_COUNT, Width::Byte)
                }
            })?;
        }
        self.emit(|encoder| encoder.movzx_byte_to_qword(SCRATCH_VALUE, SCRATCH_VALUE))?;
        self.store_register(destination, SCRATCH_VALUE)
    }

    fn select_compare_branch(
        &mut self,
        comparison: (NativeCompareOp, NativeValue, NativeValue, MachineType),
        then_block: BlockId,
        else_block: BlockId,
        next_block: Option<BlockId>,
    ) -> Result<(), NativeMirError> {
        let (op, left, right, ty) = comparison;
        let left_register = self.comparison_register(left, SCRATCH_VALUE)?;
        if let NativeValue::Immediate(value) = right {
            if let Some(immediate) = immediate_u32(value, ty) {
                self.emit(|encoder| encoder.cmp_reg_imm(left_register, immediate, width(ty)))?;
            } else {
                let right_register = self.comparison_register(right, SCRATCH_OTHER)?;
                self.emit(|encoder| encoder.cmp_reg(left_register, right_register, width(ty)))?;
            }
        } else {
            let right_register = self.comparison_register(right, SCRATCH_OTHER)?;
            self.emit(|encoder| encoder.cmp_reg(left_register, right_register, width(ty)))?;
        }
        let condition = compare_condition(op);
        if next_block == Some(else_block) {
            let patch = self
                .encoder
                .jump_if_placeholder(condition)
                .map_err(encode_error)?;
            self.statistics.selected_instructions += 1;
            self.statistics.branches += 1;
            self.branch_patches.push((patch, then_block));
        } else if next_block == Some(then_block) {
            let patch = self
                .encoder
                .jump_if_placeholder(invert_condition(condition))
                .map_err(encode_error)?;
            self.statistics.selected_instructions += 1;
            self.statistics.branches += 1;
            self.branch_patches.push((patch, else_block));
        } else {
            let then_patch = self
                .encoder
                .jump_if_placeholder(condition)
                .map_err(encode_error)?;
            self.statistics.selected_instructions += 1;
            let else_patch = self
                .encoder
                .jump_relative_placeholder()
                .map_err(encode_error)?;
            self.statistics.selected_instructions += 1;
            self.statistics.branches += 2;
            self.branch_patches
                .extend([(then_patch, then_block), (else_patch, else_block)]);
        }
        Ok(())
    }

    fn select_call(
        &mut self,
        destination: Option<VirtualRegister>,
        callee: &NativeCallee,
        arguments: &[VirtualRegister],
        argument_aggregates: &[Option<crate::native_mir::NativeAggregateLayout>],
        convention: dbyte_ast::CallConvention,
        aggregate_result: Option<crate::native_mir::NativeAggregateLayout>,
    ) -> Result<(), NativeMirError> {
        if argument_aggregates.len() != arguments.len() {
            return Err(codegen_error("native call ABI metadata length mismatch"));
        }
        if convention == dbyte_ast::CallConvention::C
            && (aggregate_result.is_some() || argument_aggregates.iter().any(Option::is_some))
        {
            return Err(unsupported("aggregate by-value C call"));
        }
        let parameters = abi_argument_registers(convention, self.target)?;
        let win64 = convention == dbyte_ast::CallConvention::C
            && self.target.c_abi == Some(crate::target::CAbi::Win64);
        let sysv64 = convention == dbyte_ast::CallConvention::C
            && self.target.c_abi == Some(crate::target::CAbi::SysV64);
        let closure_environment = match callee {
            NativeCallee::Closure { environment, .. } => Some(*environment),
            _ => None,
        };
        let scalar_count = arguments.len()
            - argument_aggregates
                .iter()
                .filter(|item| item.is_some())
                .count()
            + usize::from(aggregate_result.is_some())
            + usize::from(closure_environment.is_some());
        if scalar_count > parameters.len() && !win64 && !sysv64 {
            return Err(unsupported("stack native call arguments"));
        }
        let aggregate_area = self.prepare_aggregate_arguments(arguments, argument_aggregates)?;
        let scalar_arguments = arguments
            .iter()
            .copied()
            .zip(argument_aggregates.iter())
            .filter_map(|(value, aggregate)| aggregate.is_none().then_some(value))
            .collect::<Vec<_>>();
        let mut hidden_and_scalars = Vec::with_capacity(scalar_arguments.len() + 1);
        if let Some(environment) = closure_environment {
            hidden_and_scalars.push(environment);
        }
        if aggregate_result.is_some() {
            hidden_and_scalars.push(
                destination.ok_or_else(|| codegen_error("aggregate call has no result storage"))?,
            );
        }
        hidden_and_scalars.extend(scalar_arguments);
        let indirect_target = match callee {
            NativeCallee::Indirect { address } => Some(*address),
            NativeCallee::Closure { code, .. } => Some(*code),
            NativeCallee::Direct { .. } => None,
        };
        if indirect_target.is_some() && sysv64 && hidden_and_scalars.len() > parameters.len() {
            return Err(unsupported("indirect SysV64 stack call arguments"));
        }
        if let Some(target) = indirect_target {
            self.load_register(target, SCRATCH_OTHER)?;
            self.emit(|encoder| encoder.push(SCRATCH_OTHER))?;
        }
        let has_float = hidden_and_scalars.iter().any(|argument| {
            matches!(
                self.function.register_types[argument.0 as usize],
                MachineType::F32 | MachineType::F64
            )
        });
        let scalar_call_area = if (win64 || sysv64) && has_float {
            self.prepare_c_float_arguments(&hidden_and_scalars, win64)?
        } else if win64 {
            self.prepare_win64_arguments(&hidden_and_scalars, parameters)?
        } else if sysv64 {
            self.prepare_sysv64_arguments(&hidden_and_scalars, parameters)?
        } else {
            self.prepare_register_arguments(
                &hidden_and_scalars,
                &parameters[..hidden_and_scalars.len()],
            )?;
            0
        };
        if indirect_target.is_some() {
            self.emit(|encoder| encoder.pop(SCRATCH_OTHER))?;
        }
        let call_area = aggregate_area
            .checked_add(scalar_call_area)
            .ok_or_else(|| codegen_error("native call area overflows"))?;
        match callee {
            NativeCallee::Direct { symbol } => {
                let offset = self
                    .encoder
                    .call_relative_placeholder()
                    .map_err(encode_error)?;
                self.statistics.selected_instructions += 1;
                self.relocations.push(NativeRelocation {
                    offset: u32::try_from(offset)
                        .map_err(|_| codegen_error("call relocation exceeds u32"))?,
                    symbol: symbol.clone(),
                    kind: NativeRelocationKind::Plt32,
                });
            }
            NativeCallee::Indirect { .. } | NativeCallee::Closure { .. } => {
                self.emit(|encoder| encoder.call_indirect(SCRATCH_OTHER))?;
            }
        }
        self.statistics.calls += 1;
        if call_area != 0 {
            self.emit(|encoder| encoder.add_imm32(Register::Rsp, call_area))?;
        }
        if let Some(destination) = destination.filter(|_| aggregate_result.is_none()) {
            if convention == dbyte_ast::CallConvention::C
                && matches!(
                    self.function.register_types[destination.0 as usize],
                    MachineType::F32 | MachineType::F64
                )
            {
                self.emit(|encoder| {
                    encoder.mov_gpr_from_xmm(
                        Register::Rax,
                        XmmRegister::Xmm0,
                        width(self.function.register_types[destination.0 as usize]),
                    )
                })?;
            }
            self.store_register(destination, Register::Rax)?;
        }
        Ok(())
    }

    fn select_intrinsic(
        &mut self,
        destination: Option<VirtualRegister>,
        intrinsic: dbyte_hir::HirIntrinsic,
        arguments: &[VirtualRegister],
    ) -> Result<(), NativeMirError> {
        use dbyte_hir::HirIntrinsic;
        let windows_symbol = match intrinsic {
            HirIntrinsic::WindowsGetStdHandle => Some("__imp_GetStdHandle"),
            HirIntrinsic::WindowsReadFile => Some("__imp_ReadFile"),
            HirIntrinsic::WindowsWriteFile => Some("__imp_WriteFile"),
            HirIntrinsic::WindowsCloseHandle => Some("__imp_CloseHandle"),
            HirIntrinsic::WindowsVirtualAlloc => Some("__imp_VirtualAlloc"),
            HirIntrinsic::WindowsVirtualFree => Some("__imp_VirtualFree"),
            HirIntrinsic::WindowsQueryPerformanceCounter => Some("__imp_QueryPerformanceCounter"),
            _ => None,
        };
        if let Some(symbol) = windows_symbol {
            return self.select_windows_import(destination, symbol, arguments);
        }
        let linux = match intrinsic {
            HirIntrinsic::LinuxRead => {
                Some((0, &[Register::Rdi, Register::Rsi, Register::Rdx][..]))
            }
            HirIntrinsic::LinuxWrite => {
                Some((1, &[Register::Rdi, Register::Rsi, Register::Rdx][..]))
            }
            HirIntrinsic::LinuxClose => Some((3, &[Register::Rdi][..])),
            HirIntrinsic::LinuxMmap => Some((
                9,
                &[
                    Register::Rdi,
                    Register::Rsi,
                    Register::Rdx,
                    Register::R10,
                    Register::R8,
                    Register::R9,
                ][..],
            )),
            HirIntrinsic::LinuxMunmap => Some((11, &[Register::Rdi, Register::Rsi][..])),
            HirIntrinsic::LinuxExit => Some((60, &[Register::Rdi][..])),
            HirIntrinsic::LinuxClockGettime => Some((228, &[Register::Rdi, Register::Rsi][..])),
            HirIntrinsic::LinuxOpenAt => Some((
                257,
                &[Register::Rdi, Register::Rsi, Register::Rdx, Register::R10][..],
            )),
            _ => None,
        };
        if let Some((number, registers)) = linux {
            if arguments.len() != registers.len() {
                return Err(codegen_error("resolved Linux intrinsic has invalid arity"));
            }
            self.prepare_register_arguments(arguments, registers)?;
            self.emit_immediate(Register::Rax, number)?;
            self.emit(Encoder::syscall)?;
            if let Some(destination) = destination {
                self.store_register(destination, Register::Rax)?;
            }
            return Ok(());
        }
        let user_arity = match intrinsic {
            HirIntrinsic::UserSyscall0 => Some(1),
            HirIntrinsic::UserSyscall1 => Some(2),
            HirIntrinsic::UserSyscall2 => Some(3),
            HirIntrinsic::UserSyscall3 => Some(4),
            _ => None,
        };
        if let Some(expected) = user_arity {
            if self.target != crate::target::X86_64_USER {
                return Err(codegen_error(
                    "user syscall reached non-user DBYTE64 target",
                ));
            }
            if arguments.len() != expected {
                return Err(codegen_error("resolved user syscall has invalid arity"));
            }
            let abi = [Register::Rax, Register::Rdx, Register::Rcx, Register::R8];
            self.prepare_register_arguments(arguments, &abi[..expected])?;
            self.emit(|encoder| encoder.software_interrupt(0x80))?;
            if let Some(destination) = destination {
                self.store_register(destination, Register::Rax)?;
            }
            return Ok(());
        }
        match intrinsic {
            HirIntrinsic::IoOut8 if arguments.len() == 2 => {
                self.load_register(arguments[0], Register::Rdx)?;
                self.load_register(arguments[1], Register::Rax)?;
                self.emit(Encoder::out8)?;
            }
            HirIntrinsic::IoOut16 if arguments.len() == 2 => {
                self.load_register(arguments[0], Register::Rdx)?;
                self.load_register(arguments[1], Register::Rax)?;
                self.emit(Encoder::out16)?;
            }
            HirIntrinsic::IoOut32 if arguments.len() == 2 => {
                self.load_register(arguments[0], Register::Rdx)?;
                self.load_register(arguments[1], Register::Rax)?;
                self.emit(Encoder::out32)?;
            }
            HirIntrinsic::IoIn8 if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rdx)?;
                self.emit(Encoder::in8)?;
                self.emit(|encoder| encoder.movzx_byte_to_qword(Register::Rax, Register::Rax))?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::IoIn16 if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rdx)?;
                self.emit(Encoder::in16)?;
                self.emit(|encoder| encoder.movzx_word_to_qword(Register::Rax, Register::Rax))?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::IoIn32 if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rdx)?;
                self.emit(Encoder::in32)?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::VolatileLoad8
            | HirIntrinsic::VolatileLoad16
            | HirIntrinsic::VolatileLoad32
            | HirIntrinsic::VolatileLoad64
                if arguments.len() == 1 =>
            {
                self.load_register(arguments[0], Register::R11)?;
                let width = match intrinsic {
                    HirIntrinsic::VolatileLoad8 => Width::Byte,
                    HirIntrinsic::VolatileLoad16 => Width::Word,
                    HirIntrinsic::VolatileLoad32 => Width::Dword,
                    HirIntrinsic::VolatileLoad64 => Width::Qword,
                    _ => unreachable!(),
                };
                if matches!(width, Width::Dword | Width::Qword) {
                    self.emit(|encoder| {
                        encoder.mov_reg_from_base_disp32(Register::Rax, Register::R11, 0, width)
                    })?;
                } else {
                    self.emit(|encoder| {
                        encoder.movzx_reg_from_base_disp32(Register::Rax, Register::R11, 0, width)
                    })?;
                }
                self.statistics.loads += 1;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::VolatileStore8
            | HirIntrinsic::VolatileStore16
            | HirIntrinsic::VolatileStore32
            | HirIntrinsic::VolatileStore64
                if arguments.len() == 2 =>
            {
                self.load_register(arguments[0], Register::R11)?;
                self.load_register(arguments[1], Register::Rax)?;
                let width = match intrinsic {
                    HirIntrinsic::VolatileStore8 => Width::Byte,
                    HirIntrinsic::VolatileStore16 => Width::Word,
                    HirIntrinsic::VolatileStore32 => Width::Dword,
                    HirIntrinsic::VolatileStore64 => Width::Qword,
                    _ => unreachable!(),
                };
                self.emit(|encoder| {
                    encoder.mov_base_disp32_from_reg(Register::R11, 0, Register::Rax, width)
                })?;
                self.statistics.stores += 1;
            }
            HirIntrinsic::CpuCpuidEax
            | HirIntrinsic::CpuCpuidEbx
            | HirIntrinsic::CpuCpuidEcx
            | HirIntrinsic::CpuCpuidEdx
                if arguments.len() == 2 =>
            {
                self.emit(|encoder| encoder.push(Register::Rbx))?;
                self.load_register(arguments[0], Register::Rax)?;
                self.load_register(arguments[1], Register::Rcx)?;
                self.emit(Encoder::cpuid)?;
                let result_register = match intrinsic {
                    HirIntrinsic::CpuCpuidEax => Register::Rax,
                    HirIntrinsic::CpuCpuidEbx => {
                        self.emit(|encoder| {
                            encoder.mov_reg(Register::R11, Register::Rbx, Width::Qword)
                        })?;
                        Register::R11
                    }
                    HirIntrinsic::CpuCpuidEcx => Register::Rcx,
                    HirIntrinsic::CpuCpuidEdx => Register::Rdx,
                    _ => unreachable!(),
                };
                self.emit(|encoder| encoder.pop(Register::Rbx))?;
                if let Some(destination) = destination {
                    self.store_register(destination, result_register)?;
                }
            }
            HirIntrinsic::CpuRdmsr if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rcx)?;
                self.emit(Encoder::rdmsr)?;
                self.emit(|encoder| encoder.mov_reg(Register::R11, Register::Rdx, Width::Qword))?;
                self.emit_immediate(Register::Rcx, 32)?;
                self.emit(|encoder| {
                    encoder.shift_reg_by_cl(Register::R11, ShiftKind::Left, Width::Qword)
                })?;
                self.emit(|encoder| encoder.or_reg(Register::Rax, Register::R11, Width::Qword))?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::CpuWrmsr if arguments.len() == 2 => {
                self.load_register(arguments[1], Register::Rax)?;
                self.emit(|encoder| encoder.mov_reg(Register::Rdx, Register::Rax, Width::Qword))?;
                self.emit_immediate(Register::Rcx, 32)?;
                self.emit(|encoder| {
                    encoder.shift_reg_by_cl(Register::Rdx, ShiftKind::LogicalRight, Width::Qword)
                })?;
                self.load_register(arguments[0], Register::Rcx)?;
                self.emit(Encoder::wrmsr)?;
            }
            HirIntrinsic::CpuReadEflags if arguments.is_empty() => {
                self.emit(Encoder::pushfq)?;
                self.emit(|encoder| encoder.pop(Register::Rax))?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::CpuReadCr0 if arguments.is_empty() => {
                self.emit(Encoder::mov_rax_cr0)?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::CpuWriteCr0 if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(Encoder::mov_cr0_rax)?;
            }
            HirIntrinsic::CpuReadCr2 if arguments.is_empty() => {
                self.emit(Encoder::mov_rax_cr2)?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::CpuReadCr3 if arguments.is_empty() => {
                self.emit(Encoder::mov_rax_cr3)?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::CpuWriteCr3 if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(Encoder::mov_cr3_rax)?;
            }
            HirIntrinsic::CpuReadCr4 if arguments.is_empty() => {
                self.emit(Encoder::mov_rax_cr4)?;
                if let Some(destination) = destination {
                    self.store_register(destination, Register::Rax)?;
                }
            }
            HirIntrinsic::CpuWriteCr4 if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(Encoder::mov_cr4_rax)?;
            }
            HirIntrinsic::CpuInvlpg if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(Encoder::invlpg_rax)?;
            }
            HirIntrinsic::CpuLgdt if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(Encoder::lgdt_rax)?;
            }
            HirIntrinsic::CpuLidt if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(Encoder::lidt_rax)?;
            }
            HirIntrinsic::CpuLtr if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(Encoder::ltr_ax)?;
            }
            HirIntrinsic::InterruptResume if arguments.len() == 1 => {
                self.load_register(arguments[0], Register::Rax)?;
                self.emit(|encoder| encoder.mov_reg(Register::Rsp, Register::Rax, Width::Qword))?;
                for register in [
                    Register::R15,
                    Register::R14,
                    Register::R13,
                    Register::R12,
                    Register::R11,
                    Register::R10,
                    Register::R9,
                    Register::R8,
                    Register::Rdi,
                    Register::Rsi,
                    Register::Rbp,
                    Register::Rdx,
                    Register::Rcx,
                    Register::Rbx,
                    Register::Rax,
                ] {
                    self.emit(|encoder| encoder.pop(register))?;
                }
                self.emit(Encoder::iretq)?;
            }
            HirIntrinsic::CpuCli if arguments.is_empty() => self.emit(Encoder::cli)?,
            HirIntrinsic::CpuSti if arguments.is_empty() => self.emit(Encoder::sti)?,
            HirIntrinsic::CpuInt3 if arguments.is_empty() => self.emit(Encoder::int3)?,
            HirIntrinsic::CpuHlt if arguments.is_empty() => self.emit(Encoder::hlt)?,
            _ => return Err(unsupported("resolved target intrinsic")),
        }
        Ok(())
    }

    fn prepare_aggregate_arguments(
        &mut self,
        arguments: &[VirtualRegister],
        layouts: &[Option<crate::native_mir::NativeAggregateLayout>],
    ) -> Result<u32, NativeMirError> {
        let mut offsets = Vec::with_capacity(arguments.len());
        let mut used = 0u32;
        for layout in layouts {
            if let Some(layout) = layout {
                offsets.push(Some(used));
                used = used
                    .checked_add(aggregate_stack_bytes(*layout)?)
                    .ok_or_else(|| codegen_error("aggregate call area overflows"))?;
            } else {
                offsets.push(None);
            }
        }
        if used == 0 {
            return Ok(0);
        }
        let area = align_up(used, 16)?;
        self.emit(|encoder| encoder.sub_imm32(Register::Rsp, area))?;
        for ((argument, layout), offset) in arguments
            .iter()
            .copied()
            .zip(layouts.iter().copied())
            .zip(offsets)
        {
            let (Some(layout), Some(offset)) = (layout, offset) else {
                continue;
            };
            self.load_register(argument, SCRATCH_VALUE)?;
            let displacement = i32::try_from(offset)
                .map_err(|_| codegen_error("aggregate call offset exceeds i32"))?;
            self.emit(|encoder| {
                encoder.lea_base_disp32(SCRATCH_OTHER, Register::Rsp, displacement)
            })?;
            let mut copied = 0u32;
            while copied < layout.bytes {
                let (chunk, width) = aggregate_chunk(layout.bytes - copied);
                let displacement = i32::try_from(copied)
                    .map_err(|_| codegen_error("aggregate argument copy offset exceeds i32"))?;
                self.emit(|encoder| {
                    encoder.mov_reg_from_base_disp32(
                        AGGREGATE_TEMP,
                        SCRATCH_VALUE,
                        displacement,
                        width,
                    )
                })?;
                self.emit(|encoder| {
                    encoder.mov_base_disp32_from_reg(
                        SCRATCH_OTHER,
                        displacement,
                        AGGREGATE_TEMP,
                        width,
                    )
                })?;
                self.statistics.loads += 1;
                self.statistics.stores += 1;
                copied += chunk;
            }
        }
        Ok(area)
    }

    fn snapshot_arguments(&mut self, arguments: &[VirtualRegister]) -> Result<u32, NativeMirError> {
        for argument in arguments.iter().copied() {
            self.load_register(argument, SCRATCH_OTHER)?;
            self.emit(|encoder| encoder.push(SCRATCH_OTHER))?;
        }
        u32::try_from(arguments.len())
            .map_err(|_| codegen_error("native argument count exceeds u32"))?
            .checked_mul(8)
            .ok_or_else(|| codegen_error("native argument snapshot overflows"))
    }

    fn register_arguments_can_move_directly(
        &self,
        arguments: &[VirtualRegister],
        targets: &[Register],
    ) -> Result<bool, NativeMirError> {
        let targets = targets
            .iter()
            .copied()
            .map(physical)
            .collect::<BTreeSet<_>>();
        for argument in arguments.iter().copied() {
            if let RegisterLocation::Register(source) = self.location(argument)? {
                if targets.contains(&source) {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn prepare_register_arguments(
        &mut self,
        arguments: &[VirtualRegister],
        targets: &[Register],
    ) -> Result<(), NativeMirError> {
        if arguments.len() != targets.len() {
            return Err(codegen_error("register argument arity mismatch"));
        }
        if self.register_arguments_can_move_directly(arguments, targets)? {
            for (argument, target) in arguments.iter().copied().zip(targets.iter().copied()) {
                self.load_register(argument, target)?;
            }
            return Ok(());
        }
        let snapshot_bytes = self.snapshot_arguments(arguments)?;
        for (index, target) in targets.iter().copied().enumerate() {
            let displacement = i32::try_from((arguments.len() - index - 1) * 8)
                .map_err(|_| codegen_error("argument snapshot offset exceeds i32"))?;
            self.emit(|encoder| {
                encoder.mov_reg_from_base_disp32(target, Register::Rsp, displacement, Width::Qword)
            })?;
            self.statistics.loads += 1;
        }
        if snapshot_bytes != 0 {
            self.emit(|encoder| encoder.add_imm32(Register::Rsp, snapshot_bytes))?;
        }
        Ok(())
    }

    fn prepare_sysv64_arguments(
        &mut self,
        arguments: &[VirtualRegister],
        targets: &[Register],
    ) -> Result<u32, NativeMirError> {
        if arguments.len() <= targets.len() {
            self.prepare_register_arguments(arguments, &targets[..arguments.len()])?;
            return Ok(0);
        }
        let extra_count = arguments.len() - targets.len();
        let extra_bytes = u32::try_from(extra_count)
            .map_err(|_| codegen_error("SysV64 argument count exceeds u32"))?
            .checked_mul(8)
            .ok_or_else(|| codegen_error("SysV64 stack arguments overflow"))?;
        let call_area = align_up(extra_bytes, 16)?;
        self.emit(|encoder| encoder.sub_imm32(Register::Rsp, call_area))?;
        for (index, argument) in arguments[targets.len()..].iter().copied().enumerate() {
            self.load_register(argument, SCRATCH_OTHER)?;
            let displacement = i32::try_from(index * 8)
                .map_err(|_| codegen_error("SysV64 argument offset exceeds i32"))?;
            self.emit(|encoder| {
                encoder.mov_base_disp32_from_reg(
                    Register::Rsp,
                    displacement,
                    SCRATCH_OTHER,
                    Width::Qword,
                )
            })?;
            self.statistics.stores += 1;
        }
        self.prepare_register_arguments(&arguments[..targets.len()], targets)?;
        Ok(call_area)
    }
    fn prepare_win64_arguments(
        &mut self,
        arguments: &[VirtualRegister],
        targets: &[Register],
    ) -> Result<u32, NativeMirError> {
        let snapshot_bytes = self.snapshot_arguments(arguments)?;
        let extra_count = arguments.len().saturating_sub(targets.len());
        let extra_bytes = u32::try_from(extra_count)
            .map_err(|_| codegen_error("Win64 argument count exceeds u32"))?
            .checked_mul(8)
            .ok_or_else(|| codegen_error("Win64 stack arguments overflow"))?;
        let call_area = 32u32
            .checked_add(extra_bytes)
            .ok_or_else(|| codegen_error("Win64 call area overflows"))?;
        self.emit(|encoder| encoder.sub_imm32(Register::Rsp, call_area))?;
        for (index, _) in arguments.iter().enumerate() {
            let staged = call_area
                .checked_add(
                    u32::try_from((arguments.len() - index - 1) * 8)
                        .map_err(|_| codegen_error("Win64 snapshot offset exceeds u32"))?,
                )
                .ok_or_else(|| codegen_error("Win64 snapshot offset overflows"))?;
            let staged = i32::try_from(staged)
                .map_err(|_| codegen_error("Win64 snapshot offset exceeds i32"))?;
            self.emit(|encoder| {
                encoder.mov_reg_from_base_disp32(SCRATCH_OTHER, Register::Rsp, staged, Width::Qword)
            })?;
            self.statistics.loads += 1;
            if let Some(target) = targets.get(index).copied() {
                self.emit(|encoder| encoder.mov_reg(target, SCRATCH_OTHER, Width::Qword))?;
            } else {
                let extra = i32::try_from((index - targets.len()) * 8)
                    .map_err(|_| codegen_error("Win64 argument offset exceeds i32"))?;
                let destination = 32i32
                    .checked_add(extra)
                    .ok_or_else(|| codegen_error("Win64 argument offset overflows"))?;
                self.emit(|encoder| {
                    encoder.mov_base_disp32_from_reg(
                        Register::Rsp,
                        destination,
                        SCRATCH_OTHER,
                        Width::Qword,
                    )
                })?;
                self.statistics.stores += 1;
            }
        }
        call_area
            .checked_add(snapshot_bytes)
            .ok_or_else(|| codegen_error("Win64 call cleanup overflows"))
    }

    fn prepare_c_float_arguments(
        &mut self,
        arguments: &[VirtualRegister],
        win64: bool,
    ) -> Result<u32, NativeMirError> {
        let snapshot_bytes = self.snapshot_arguments(arguments)?;
        let mut integer_index = 0usize;
        let mut float_index = 0usize;
        let mut stack_index = 0usize;
        let mut placements = Vec::with_capacity(arguments.len());
        for (index, argument) in arguments.iter().copied().enumerate() {
            let ty = self.function.register_types[argument.0 as usize];
            let is_float = matches!(ty, MachineType::F32 | MachineType::F64);
            let placement = if win64 {
                if index < 4 {
                    if is_float {
                        CFloatLocation::Xmm(index)
                    } else {
                        CFloatLocation::Gpr(index)
                    }
                } else {
                    let slot = stack_index;
                    stack_index += 1;
                    CFloatLocation::Stack(slot)
                }
            } else if is_float && float_index < 8 {
                let slot = float_index;
                float_index += 1;
                CFloatLocation::Xmm(slot)
            } else if !is_float && integer_index < 6 {
                let slot = integer_index;
                integer_index += 1;
                CFloatLocation::Gpr(slot)
            } else {
                let slot = stack_index;
                stack_index += 1;
                CFloatLocation::Stack(slot)
            };
            placements.push((placement, ty));
        }
        let base = if win64 { 32 } else { 0 };
        let required = base
            + u32::try_from(stack_index)
                .map_err(|_| codegen_error("C ABI stack argument count exceeds u32"))?
                * 8;
        let padding = (16 - ((snapshot_bytes + required) % 16)) % 16;
        let call_area = required
            .checked_add(padding)
            .ok_or_else(|| codegen_error("C ABI call area overflows"))?;
        if call_area != 0 {
            self.emit(|encoder| encoder.sub_imm32(Register::Rsp, call_area))?;
        }
        let gprs = if win64 {
            &[Register::Rcx, Register::Rdx, Register::R8, Register::R9][..]
        } else {
            &[
                Register::Rdi,
                Register::Rsi,
                Register::Rdx,
                Register::Rcx,
                Register::R8,
                Register::R9,
            ][..]
        };
        for (index, (placement, ty)) in placements.into_iter().enumerate() {
            let staged = call_area
                .checked_add(
                    u32::try_from((arguments.len() - index - 1) * 8)
                        .map_err(|_| codegen_error("C ABI snapshot offset exceeds u32"))?,
                )
                .ok_or_else(|| codegen_error("C ABI snapshot offset overflows"))?;
            let staged = i32::try_from(staged)
                .map_err(|_| codegen_error("C ABI snapshot offset exceeds i32"))?;
            self.emit(|encoder| {
                encoder.mov_reg_from_base_disp32(SCRATCH_OTHER, Register::Rsp, staged, Width::Qword)
            })?;
            match placement {
                CFloatLocation::Gpr(slot) => {
                    self.emit(|encoder| encoder.mov_reg(gprs[slot], SCRATCH_OTHER, Width::Qword))?
                }
                CFloatLocation::Xmm(slot) => {
                    let destination = xmm_register(slot)?;
                    let destination_width = width(ty);
                    self.emit(|encoder| {
                        encoder.mov_xmm_from_gpr(destination, SCRATCH_OTHER, destination_width)
                    })?;
                }
                CFloatLocation::Stack(slot) => {
                    let displacement = base
                        + u32::try_from(slot)
                            .map_err(|_| codegen_error("C ABI stack slot exceeds u32"))?
                            * 8;
                    let displacement = i32::try_from(displacement)
                        .map_err(|_| codegen_error("C ABI stack offset exceeds i32"))?;
                    self.emit(|encoder| {
                        encoder.mov_base_disp32_from_reg(
                            Register::Rsp,
                            displacement,
                            SCRATCH_OTHER,
                            Width::Qword,
                        )
                    })?;
                    self.statistics.stores += 1;
                }
            }
            self.statistics.loads += 1;
        }
        call_area
            .checked_add(snapshot_bytes)
            .ok_or_else(|| codegen_error("C ABI call cleanup overflows"))
    }

    fn select_windows_import(
        &mut self,
        destination: Option<VirtualRegister>,
        symbol: &str,
        arguments: &[VirtualRegister],
    ) -> Result<(), NativeMirError> {
        if self.target.c_abi != Some(crate::target::CAbi::Win64) {
            return Err(unsupported("Windows import outside the Win64 target"));
        }
        const PARAMETERS: [Register; 4] =
            [Register::Rcx, Register::Rdx, Register::R8, Register::R9];
        let call_area = self.prepare_win64_arguments(arguments, &PARAMETERS)?;
        let offset = self
            .encoder
            .call_rip_relative_placeholder()
            .map_err(encode_error)?;
        self.statistics.selected_instructions += 1;
        self.relocations.push(NativeRelocation {
            offset: u32::try_from(offset)
                .map_err(|_| codegen_error("Win64 import relocation exceeds u32"))?,
            symbol: symbol.to_owned(),
            kind: NativeRelocationKind::Pc32,
        });
        self.statistics.calls += 1;
        self.emit(|encoder| encoder.add_imm32(Register::Rsp, call_area))?;
        if let Some(destination) = destination {
            self.store_register(destination, Register::Rax)?;
        }
        Ok(())
    }

    fn select_aggregate_zero(
        &mut self,
        destination: VirtualRegister,
        bytes: u32,
    ) -> Result<(), NativeMirError> {
        if bytes > 4096 {
            return Err(unsupported("aggregate zero larger than 4096 bytes"));
        }
        self.load_register(destination, SCRATCH_OTHER)?;
        self.emit(|encoder| encoder.xor_reg(SCRATCH_VALUE, SCRATCH_VALUE, Width::Qword))?;
        let mut offset = 0u32;
        while offset < bytes {
            let remaining = bytes - offset;
            let (chunk, width) = aggregate_chunk(remaining);
            let displacement = i32::try_from(offset)
                .map_err(|_| codegen_error("aggregate zero offset exceeds i32"))?;
            self.emit(|encoder| {
                encoder.mov_base_disp32_from_reg(SCRATCH_OTHER, displacement, SCRATCH_VALUE, width)
            })?;
            self.statistics.stores += 1;
            offset += chunk;
        }
        Ok(())
    }

    fn select_aggregate_copy(
        &mut self,
        destination: VirtualRegister,
        source: VirtualRegister,
        bytes: u32,
    ) -> Result<(), NativeMirError> {
        if bytes > 4096 {
            return Err(unsupported("aggregate copy larger than 4096 bytes"));
        }
        self.load_register(destination, SCRATCH_OTHER)?;
        self.load_register(source, SCRATCH_VALUE)?;
        let mut offset = 0u32;
        while offset < bytes {
            let remaining = bytes - offset;
            let (chunk, width) = aggregate_chunk(remaining);
            let displacement = i32::try_from(offset)
                .map_err(|_| codegen_error("aggregate copy offset exceeds i32"))?;
            self.emit(|encoder| {
                encoder.mov_reg_from_base_disp32(AGGREGATE_TEMP, SCRATCH_VALUE, displacement, width)
            })?;
            self.emit(|encoder| {
                encoder.mov_base_disp32_from_reg(SCRATCH_OTHER, displacement, AGGREGATE_TEMP, width)
            })?;
            self.statistics.loads += 1;
            self.statistics.stores += 1;
            offset += chunk;
        }
        Ok(())
    }

    fn select_terminator(
        &mut self,
        terminator: &NativeTerminator,
        next_block: Option<BlockId>,
    ) -> Result<(), NativeMirError> {
        match terminator {
            NativeTerminator::Jump(target) => {
                if next_block != Some(*target) {
                    let patch = self
                        .encoder
                        .jump_relative_placeholder()
                        .map_err(encode_error)?;
                    self.statistics.selected_instructions += 1;
                    self.statistics.branches += 1;
                    self.branch_patches.push((patch, *target));
                }
            }
            NativeTerminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                self.load_value(*condition, SCRATCH_VALUE)?;
                self.emit(|encoder| encoder.test_reg(SCRATCH_VALUE, SCRATCH_VALUE, Width::Qword))?;
                if next_block == Some(*else_block) {
                    let patch = self
                        .encoder
                        .jump_if_placeholder(Condition::NotEqual)
                        .map_err(encode_error)?;
                    self.statistics.selected_instructions += 1;
                    self.statistics.branches += 1;
                    self.branch_patches.push((patch, *then_block));
                } else if next_block == Some(*then_block) {
                    let patch = self
                        .encoder
                        .jump_if_placeholder(Condition::Equal)
                        .map_err(encode_error)?;
                    self.statistics.selected_instructions += 1;
                    self.statistics.branches += 1;
                    self.branch_patches.push((patch, *else_block));
                } else {
                    let then_patch = self
                        .encoder
                        .jump_if_placeholder(Condition::NotEqual)
                        .map_err(encode_error)?;
                    self.statistics.selected_instructions += 1;
                    let else_patch = self
                        .encoder
                        .jump_relative_placeholder()
                        .map_err(encode_error)?;
                    self.statistics.selected_instructions += 1;
                    self.statistics.branches += 2;
                    self.branch_patches
                        .extend([(then_patch, *then_block), (else_patch, *else_block)]);
                }
            }
            NativeTerminator::Return(value) => {
                if let Some(value) = value {
                    self.load_register(*value, Register::Rax)?;
                    let ty = self.function.register_types[value.0 as usize];
                    if self.function.convention == dbyte_ast::CallConvention::C
                        && matches!(ty, MachineType::F32 | MachineType::F64)
                    {
                        self.emit(|encoder| {
                            encoder.mov_xmm_from_gpr(XmmRegister::Xmm0, Register::Rax, width(ty))
                        })?;
                    }
                }
                let patch = self
                    .encoder
                    .jump_relative_placeholder()
                    .map_err(encode_error)?;
                self.statistics.selected_instructions += 1;
                self.statistics.branches += 1;
                self.return_patches.push(patch);
            }
            NativeTerminator::ReturnAggregate { source, layout } => {
                if self.function.aggregate_result_register == Some(*source) {
                    self.load_register(*source, Register::Rax)?;
                } else {
                    let displacement = self.sret_displacement.ok_or_else(|| {
                        codegen_error("aggregate return has no saved sret pointer")
                    })?;
                    self.emit(|encoder| {
                        encoder.mov_reg_from_base_disp32(
                            SCRATCH_OTHER,
                            Register::Rbp,
                            displacement,
                            Width::Qword,
                        )
                    })?;
                    self.load_register(*source, SCRATCH_VALUE)?;
                    let mut copied = 0u32;
                    while copied < layout.bytes {
                        let (chunk, width) = aggregate_chunk(layout.bytes - copied);
                        let displacement = i32::try_from(copied)
                            .map_err(|_| codegen_error("aggregate return offset exceeds i32"))?;
                        self.emit(|encoder| {
                            encoder.mov_reg_from_base_disp32(
                                AGGREGATE_TEMP,
                                SCRATCH_VALUE,
                                displacement,
                                width,
                            )
                        })?;
                        self.emit(|encoder| {
                            encoder.mov_base_disp32_from_reg(
                                SCRATCH_OTHER,
                                displacement,
                                AGGREGATE_TEMP,
                                width,
                            )
                        })?;
                        self.statistics.loads += 1;
                        self.statistics.stores += 1;
                        copied += chunk;
                    }
                    self.emit(|encoder| {
                        encoder.mov_reg(Register::Rax, SCRATCH_OTHER, Width::Qword)
                    })?;
                }
                let patch = self
                    .encoder
                    .jump_relative_placeholder()
                    .map_err(encode_error)?;
                self.statistics.selected_instructions += 1;
                self.statistics.branches += 1;
                self.return_patches.push(patch);
            }
            NativeTerminator::Trap | NativeTerminator::Unreachable => {
                self.emit(Encoder::ud2)?;
            }
        }
        Ok(())
    }

    fn load_value(&mut self, value: NativeValue, target: Register) -> Result<(), NativeMirError> {
        match value {
            NativeValue::Register(register) => self.load_register(register, target),
            NativeValue::Immediate(value) => self.emit_immediate(target, value),
        }
    }

    fn emit_immediate(&mut self, target: Register, value: u64) -> Result<(), NativeMirError> {
        if let Ok(value) = u32::try_from(value) {
            self.emit(|encoder| encoder.mov_imm32(target, value))
        } else {
            self.emit(|encoder| encoder.mov_imm64(target, value))
        }
    }

    fn comparison_register(
        &mut self,
        value: NativeValue,
        fallback: Register,
    ) -> Result<Register, NativeMirError> {
        if let NativeValue::Register(register) = value {
            if let RegisterLocation::Register(source) = self.location(register)? {
                return Ok(machine_register(source));
            }
        }
        self.load_value(value, fallback)?;
        Ok(fallback)
    }

    fn load_register(
        &mut self,
        register: VirtualRegister,
        target: Register,
    ) -> Result<(), NativeMirError> {
        match self.location(register)? {
            RegisterLocation::Register(source) => {
                let source = machine_register(source);
                if source != target {
                    self.emit(|encoder| encoder.mov_reg(target, source, Width::Qword))?;
                }
            }
            RegisterLocation::Spill { slot } => {
                let displacement = self.spill_displacement(slot)?;
                self.emit(|encoder| {
                    encoder.mov_reg_from_base_disp32(
                        target,
                        Register::Rbp,
                        displacement,
                        Width::Qword,
                    )
                })?;
                self.statistics.loads += 1;
                self.statistics.reloads += 1;
            }
            RegisterLocation::RegisterPair { .. } => {
                return Err(codegen_error("x86_64 scalar has register-pair allocation"));
            }
        }
        Ok(())
    }

    fn store_register(
        &mut self,
        register: VirtualRegister,
        source: Register,
    ) -> Result<(), NativeMirError> {
        match self.location(register)? {
            RegisterLocation::Register(destination) => {
                let destination = machine_register(destination);
                if destination != source {
                    self.emit(|encoder| encoder.mov_reg(destination, source, Width::Qword))?;
                }
            }
            RegisterLocation::Spill { slot } => {
                let displacement = self.spill_displacement(slot)?;
                self.emit(|encoder| {
                    encoder.mov_base_disp32_from_reg(
                        Register::Rbp,
                        displacement,
                        source,
                        Width::Qword,
                    )
                })?;
                self.statistics.stores += 1;
            }
            RegisterLocation::RegisterPair { .. } => {
                return Err(codegen_error("x86_64 scalar has register-pair allocation"));
            }
        }
        Ok(())
    }

    fn location(&self, register: VirtualRegister) -> Result<RegisterLocation, NativeMirError> {
        self.allocation
            .locations
            .get(&register)
            .copied()
            .ok_or_else(|| codegen_error("register allocation omitted a virtual register"))
    }

    fn spill_displacement(&self, slot: u32) -> Result<i32, NativeMirError> {
        let bytes = (self.saved.len() as u32)
            .checked_mul(8)
            .and_then(|saved| saved.checked_add((slot + 1).checked_mul(8)?))
            .ok_or_else(|| codegen_error("spill displacement exceeds u32"))?;
        i32::try_from(bytes)
            .ok()
            .and_then(i32::checked_neg)
            .ok_or_else(|| codegen_error("spill displacement exceeds i32"))
    }

    fn stack_object_displacement(
        &self,
        object: crate::native_mir::StackObjectId,
    ) -> Result<i32, NativeMirError> {
        let offset = self
            .stack_offsets
            .get(&object)
            .copied()
            .ok_or_else(|| codegen_error("selector references an unknown stack object"))?;
        let bytes = (self.saved.len() as u32)
            .checked_mul(8)
            .and_then(|saved| saved.checked_add(offset))
            .ok_or_else(|| codegen_error("stack object displacement exceeds u32"))?;
        i32::try_from(bytes)
            .ok()
            .and_then(i32::checked_neg)
            .ok_or_else(|| codegen_error("stack object displacement exceeds i32"))
    }

    fn normalize(&mut self, register: Register, ty: MachineType) -> Result<(), NativeMirError> {
        match ty {
            MachineType::Bool | MachineType::I8 => {
                if register != Register::Rax {
                    return Err(codegen_error(
                        "byte normalization requires selector scratch RAX",
                    ));
                }
                self.emit(|encoder| encoder.movzx_byte_to_qword(register, register))?;
            }
            MachineType::I16 => {
                self.shift_pair(register, 48, ShiftKind::Left, ShiftKind::LogicalRight)?;
            }
            MachineType::I32 | MachineType::F32 => {
                self.emit(|encoder| encoder.mov_reg(register, register, Width::Dword))?;
            }
            MachineType::I64 | MachineType::F64 | MachineType::Pointer => {}
        }
        Ok(())
    }

    fn extend_or_truncate(
        &mut self,
        register: Register,
        from: MachineType,
        to: MachineType,
        signed: bool,
    ) -> Result<(), NativeMirError> {
        self.normalize(register, from)?;
        let from_bits = u64::from(from.bytes(8)) * 8;
        let to_bits = u64::from(to.bytes(8)) * 8;
        if signed && from_bits < 64 && to_bits > from_bits {
            let shift = 64 - from_bits;
            self.shift_pair(register, shift, ShiftKind::Left, ShiftKind::ArithmeticRight)?;
        }
        self.normalize(register, to)
    }

    fn shift_pair(
        &mut self,
        register: Register,
        count: u64,
        first: ShiftKind,
        second: ShiftKind,
    ) -> Result<(), NativeMirError> {
        self.emit_immediate(SHIFT_COUNT, count)?;
        self.emit(|encoder| encoder.shift_reg_by_cl(register, first, Width::Qword))?;
        self.emit_immediate(SHIFT_COUNT, count)?;
        self.emit(|encoder| encoder.shift_reg_by_cl(register, second, Width::Qword))
    }

    fn emit(
        &mut self,
        instruction: impl FnOnce(&mut Encoder) -> Result<(), EncodeError>,
    ) -> Result<(), NativeMirError> {
        instruction(&mut self.encoder).map_err(encode_error)?;
        self.statistics.selected_instructions += 1;
        Ok(())
    }
}

fn layout_stack_objects(
    function: &NativeMirFunction,
    spill_bytes: u32,
) -> Result<(u32, BTreeMap<crate::native_mir::StackObjectId, u32>), NativeMirError> {
    let mut cursor = spill_bytes;
    let mut offsets = BTreeMap::new();
    for object in &function.stack_objects {
        cursor = align_up(cursor, object.alignment)?;
        cursor = cursor
            .checked_add(object.bytes)
            .ok_or_else(|| codegen_error("stack object layout exceeds u32"))?;
        offsets.insert(object.id, cursor);
    }
    Ok((cursor - spill_bytes, offsets))
}

fn aggregate_chunk(remaining: u32) -> (u32, Width) {
    if remaining >= 8 {
        (8, Width::Qword)
    } else if remaining >= 4 {
        (4, Width::Dword)
    } else if remaining >= 2 {
        (2, Width::Word)
    } else {
        (1, Width::Byte)
    }
}

fn aggregate_stack_bytes(
    layout: crate::native_mir::NativeAggregateLayout,
) -> Result<u32, NativeMirError> {
    if !layout.is_valid() || layout.alignment > 16 {
        return Err(codegen_error(
            "aggregate ABI layout is invalid or over-aligned",
        ));
    }
    align_up(layout.bytes, layout.alignment.max(8))
}

fn abi_argument_registers(
    convention: dbyte_ast::CallConvention,
    target: crate::target::TargetSpec,
) -> Result<&'static [Register], NativeMirError> {
    const DBYTE64: [Register; 6] = [
        Register::Rax,
        Register::Rdx,
        Register::Rcx,
        Register::R8,
        Register::R9,
        Register::R10,
    ];
    const SYSV64: [Register; 6] = [
        Register::Rdi,
        Register::Rsi,
        Register::Rdx,
        Register::Rcx,
        Register::R8,
        Register::R9,
    ];
    const WIN64: [Register; 4] = [Register::Rcx, Register::Rdx, Register::R8, Register::R9];
    match (convention, target.c_abi) {
        (dbyte_ast::CallConvention::DByte, _) => Ok(&DBYTE64),
        (dbyte_ast::CallConvention::C, Some(crate::target::CAbi::SysV64)) => Ok(&SYSV64),
        (dbyte_ast::CallConvention::C, Some(crate::target::CAbi::Win64)) => Ok(&WIN64),
        (dbyte_ast::CallConvention::C, _) => Err(unsupported("C ABI on this target")),
    }
}

fn align_up(value: u32, alignment: u32) -> Result<u32, NativeMirError> {
    let mask = alignment
        .checked_sub(1)
        .ok_or_else(|| codegen_error("zero stack alignment"))?;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or_else(|| codegen_error("stack alignment exceeds u32"))
}

fn align_up_u64(value: u64, alignment: u64) -> Result<u64, NativeMirError> {
    let mask = alignment
        .checked_sub(1)
        .ok_or_else(|| codegen_error("zero global alignment"))?;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or_else(|| codegen_error("global alignment exceeds u64"))
}

fn align_bytes(bytes: &mut Vec<u8>, alignment: u32) -> Result<(), NativeMirError> {
    let aligned = align_up_u64(bytes.len() as u64, u64::from(alignment))?;
    let aligned =
        usize::try_from(aligned).map_err(|_| codegen_error("global alignment exceeds usize"))?;
    bytes.resize(aligned, 0);
    Ok(())
}

fn width(ty: MachineType) -> Width {
    match ty {
        MachineType::Bool | MachineType::I8 => Width::Byte,
        MachineType::I16 => Width::Word,
        MachineType::I32 | MachineType::F32 => Width::Dword,
        MachineType::I64 | MachineType::F64 | MachineType::Pointer => Width::Qword,
    }
}

fn immediate_u32(value: u64, ty: MachineType) -> Option<u32> {
    match ty {
        MachineType::Bool | MachineType::I8 => Some(value as u8 as u32),
        MachineType::I16 => Some(value as u16 as u32),
        MachineType::I32 | MachineType::F32 => Some(value as u32),
        MachineType::I64 | MachineType::F64 | MachineType::Pointer => {
            let immediate = value as u32;
            ((immediate as i32 as i64) as u64 == value).then_some(immediate)
        }
    }
}

fn compare_condition(operation: NativeCompareOp) -> Condition {
    match operation {
        NativeCompareOp::Equal => Condition::Equal,
        NativeCompareOp::NotEqual => Condition::NotEqual,
        NativeCompareOp::SignedLess => Condition::Less,
        NativeCompareOp::SignedLessEqual => Condition::LessOrEqual,
        NativeCompareOp::SignedGreater => Condition::Greater,
        NativeCompareOp::SignedGreaterEqual => Condition::GreaterOrEqual,
        NativeCompareOp::UnsignedLess => Condition::Below,
        NativeCompareOp::UnsignedLessEqual => Condition::BelowOrEqual,
        NativeCompareOp::UnsignedGreater => Condition::Above,
        NativeCompareOp::UnsignedGreaterEqual => Condition::AboveOrEqual,
    }
}

fn invert_condition(condition: Condition) -> Condition {
    match condition {
        Condition::Equal => Condition::NotEqual,
        Condition::NotEqual => Condition::Equal,
        Condition::Below => Condition::AboveOrEqual,
        Condition::BelowOrEqual => Condition::Above,
        Condition::Above => Condition::BelowOrEqual,
        Condition::AboveOrEqual => Condition::Below,
        Condition::Less => Condition::GreaterOrEqual,
        Condition::LessOrEqual => Condition::Greater,
        Condition::Greater => Condition::LessOrEqual,
        Condition::GreaterOrEqual => Condition::Less,
        Condition::Parity => Condition::NotParity,
        Condition::NotParity => Condition::Parity,
    }
}

fn xmm_register(index: usize) -> Result<XmmRegister, NativeMirError> {
    match index {
        0 => Ok(XmmRegister::Xmm0),
        1 => Ok(XmmRegister::Xmm1),
        2 => Ok(XmmRegister::Xmm2),
        3 => Ok(XmmRegister::Xmm3),
        4 => Ok(XmmRegister::Xmm4),
        5 => Ok(XmmRegister::Xmm5),
        6 => Ok(XmmRegister::Xmm6),
        7 => Ok(XmmRegister::Xmm7),
        _ => Err(codegen_error("x86_64 C ABI XMM slot exceeds XMM7")),
    }
}

fn physical(register: Register) -> PhysicalRegister {
    PhysicalRegister(register as u8)
}

fn machine_register(register: PhysicalRegister) -> Register {
    match register.0 {
        0 => Register::Rax,
        1 => Register::Rcx,
        2 => Register::Rdx,
        3 => Register::Rbx,
        4 => Register::Rsp,
        5 => Register::Rbp,
        6 => Register::Rsi,
        7 => Register::Rdi,
        8 => Register::R8,
        9 => Register::R9,
        10 => Register::R10,
        11 => Register::R11,
        12 => Register::R12,
        13 => Register::R13,
        14 => Register::R14,
        15 => Register::R15,
        _ => unreachable!("validated register policy contains an unknown x86_64 register"),
    }
}

fn encode_error(error: EncodeError) -> NativeMirError {
    NativeMirError::new("E_NATIVE_SELECT_X86_64", error.to_string())
}

fn codegen_error(message: impl Into<String>) -> NativeMirError {
    NativeMirError::new("E_NATIVE_SELECT_X86_64", message)
}

fn unsupported(operation: &str) -> NativeMirError {
    NativeMirError::new(
        "E_NATIVE_MIR_UNSUPPORTED",
        format!("DBYTE64 selector does not yet support {operation}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_hir::{Profile, TargetDataLayout};
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn mir(source: &str) -> crate::native_mir::NativeMirProject {
        mir_for(source, Profile::X86_64Linux)
    }

    fn mir_for(source: &str, profile: Profile) -> crate::native_mir::NativeMirProject {
        let syntax = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let executable =
            dbyte_hir::analyze_executable(&syntax, profile, TargetDataLayout::X86_64).unwrap();
        let concrete = dbyte_hir::monomorphize(&executable).unwrap();
        crate::native_mir_lower::lower_program_to_native_mir(&concrete).unwrap()
    }

    #[test]
    fn interrupt_stub_preserves_all_general_purpose_registers() {
        let stub = select_interrupt_stub("probe", dbyte_ast::InterruptClass::NoErrorCode).unwrap();
        let saved_prefix = [
            0x50, 0x53, 0x51, 0x52, 0x55, 0x56, 0x57, 0x41, 0x50, 0x41, 0x51, 0x41, 0x52, 0x41,
            0x53, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
        ];
        let restored_suffix = [
            0x41, 0x5f, 0x41, 0x5e, 0x41, 0x5d, 0x41, 0x5c, 0x41, 0x5b, 0x41, 0x5a, 0x41, 0x59,
            0x41, 0x58, 0x5f, 0x5e, 0x5d, 0x5a, 0x59, 0x5b, 0x58, 0x48, 0xcf,
        ];
        assert!(stub.bytes.starts_with(&saved_prefix));
        assert!(stub.bytes.ends_with(&restored_suffix));
        assert_eq!(stub.relocations.len(), 1);
        assert_eq!(stub.relocations[0].symbol, "__dbyte_interrupt_probe_body");
    }

    #[test]
    fn interrupt_resume_restores_gprs_and_iretq_at_o0_and_o1() {
        let source = "pub fn kernel_main() -> int:\n    native.interrupt_resume(0x100000 as usize)\n    return 0\n";
        let project = mir_for(source, Profile::X86_64None);
        let oracle = [
            0x48, 0x89, 0xc4, 0x41, 0x5f, 0x41, 0x5e, 0x41, 0x5d, 0x41, 0x5c, 0x41, 0x5b, 0x41,
            0x5a, 0x41, 0x59, 0x41, 0x58, 0x5f, 0x5e, 0x5d, 0x5a, 0x59, 0x5b, 0x58, 0x48, 0xcf,
        ];
        for level in [NativeOptimizationLevel::O0, NativeOptimizationLevel::O1] {
            let selected =
                select_project_object(&project, crate::target::X86_64_NONE, level).unwrap();
            assert!(selected
                .object
                .bytes
                .windows(oracle.len())
                .any(|bytes| bytes == oracle));
        }
    }

    #[test]
    fn volatile_machine_widths_select_at_o0_and_o1() {
        for (bits, ty) in [(8, "u8"), (16, "u16"), (32, "u32"), (64, "u64")] {
            let source = format!(
                "pub fn kernel_main() -> int:\n    let p: ptr[{ty}] = (0x200000 as usize) as ptr[{ty}]\n    native.volatile_store{bits}(p, 42 as {ty})\n    return native.volatile_load{bits}(p) as int\n"
            );
            let project = mir_for(&source, Profile::X86_64None);
            for level in [NativeOptimizationLevel::O0, NativeOptimizationLevel::O1] {
                let selected = select_project_object(&project, crate::target::X86_64_NONE, level)
                    .unwrap_or_else(|error| panic!("{bits}-bit {level}: {error}"));
                let (load, store): (&[u8], &[u8]) = match bits {
                    8 => (
                        &[0x49, 0x0f, 0xb6, 0x83, 0, 0, 0, 0],
                        &[0x41, 0x88, 0x83, 0, 0, 0, 0],
                    ),
                    16 => (
                        &[0x49, 0x0f, 0xb7, 0x83, 0, 0, 0, 0],
                        &[0x66, 0x41, 0x89, 0x83, 0, 0, 0, 0],
                    ),
                    32 => (
                        &[0x41, 0x8b, 0x83, 0, 0, 0, 0],
                        &[0x41, 0x89, 0x83, 0, 0, 0, 0],
                    ),
                    64 => (
                        &[0x49, 0x8b, 0x83, 0, 0, 0, 0],
                        &[0x49, 0x89, 0x83, 0, 0, 0, 0],
                    ),
                    _ => unreachable!(),
                };
                assert!(selected
                    .object
                    .bytes
                    .windows(load.len())
                    .any(|bytes| bytes == load));
                assert!(selected
                    .object
                    .bytes
                    .windows(store.len())
                    .any(|bytes| bytes == store));
            }
        }
    }

    #[test]
    fn selector_is_deterministic_and_o1_uses_registers() {
        let project = mir(
            "pub fn main() -> i64:\n    let value: i64 = 40 as i64\n    return (value + (2 as i64)) * (1 as i64)\n",
        );
        let function = &project.functions[0];
        let o0 = select_function(function, NativeOptimizationLevel::O0).unwrap();
        let o1 = select_function(function, NativeOptimizationLevel::O1).unwrap();
        assert_eq!(
            o1,
            select_function(function, NativeOptimizationLevel::O1).unwrap()
        );
        assert!(o0.statistics.spills > 0);
        assert!(o1.statistics.assigned_registers > 0);
        assert!(o1.statistics.frame_bytes <= o0.statistics.frame_bytes);
        assert!(o1.bytes.ends_with(&[0x5d, 0xc3]));
    }

    #[test]
    fn selector_moves_allocated_register_copy_directly() {
        let project =
            mir("pub fn main(value: u64) -> u64:\n    let copy: u64 = value\n    return copy\n");
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        let (destination, source, ty) = main
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .find_map(|instruction| match instruction.instruction {
                NativeMirInst::Copy {
                    destination,
                    source,
                    ty,
                } => Some((destination, source, ty)),
                _ => None,
            })
            .unwrap();
        let allocation = RegisterAllocation {
            locations: BTreeMap::from([
                (source, RegisterLocation::Register(physical(Register::Rbx))),
                (
                    destination,
                    RegisterLocation::Register(physical(Register::R12)),
                ),
            ]),
            spill_slots: 0,
        };
        let mut selector = Selector::new(main, allocation, crate::target::X86_64_LINUX).unwrap();
        selector
            .select_instruction(&NativeMirInst::Copy {
                destination,
                source,
                ty,
            })
            .unwrap();
        let mut expected = Encoder::new(8);
        expected
            .mov_reg(Register::R12, Register::Rbx, Width::Qword)
            .unwrap();
        assert_eq!(selector.encoder.bytes(), expected.bytes());
    }

    #[test]
    fn selector_fuses_single_use_binary_into_following_copy() {
        let project = mir(
            "pub fn main(value: u64) -> u64:\n    let result: u64 = value\n    result = result + (1 as u64)\n    return result\n",
        );
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        let (temporary, destination) = main
            .blocks
            .iter()
            .flat_map(|block| block.instructions.windows(2))
            .find_map(|pair| match (&pair[0].instruction, &pair[1].instruction) {
                (
                    NativeMirInst::Binary {
                        destination: temporary,
                        ..
                    },
                    NativeMirInst::Copy {
                        destination,
                        source,
                        ..
                    },
                ) if temporary == source && main.register_use_count(*temporary) == 1 => {
                    Some((*temporary, *destination))
                }
                _ => None,
            })
            .unwrap();
        let allocation = allocate_registers(
            main,
            &dbyte64_register_policy(),
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let RegisterLocation::Register(temporary_location) = allocation.locations[&temporary]
        else {
            panic!("binary temporary unexpectedly spilled");
        };
        let RegisterLocation::Register(destination_location) = allocation.locations[&destination]
        else {
            panic!("copy destination unexpectedly spilled");
        };
        assert_ne!(temporary_location, destination_location);
        let mut redundant_copy = Encoder::new(8);
        redundant_copy
            .mov_reg(
                machine_register(destination_location),
                machine_register(temporary_location),
                Width::Qword,
            )
            .unwrap();
        let selected = Selector::new(main, allocation, crate::target::X86_64_LINUX)
            .unwrap()
            .select()
            .unwrap();
        assert!(!selected
            .bytes
            .windows(redundant_copy.bytes().len())
            .any(|bytes| bytes == redundant_copy.bytes()));
    }

    #[test]
    fn selector_updates_wide_binary_in_place_when_destination_is_left_register() {
        let project = mir(
            "pub fn main(value: u64) -> u64:\n    let result: u64 = value\n    result = result + (1 as u64)\n    return result\n",
        );
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        let destination = main.parameters[0];
        let allocation = RegisterAllocation {
            locations: BTreeMap::from([(
                destination,
                RegisterLocation::Register(physical(Register::R8)),
            )]),
            spill_slots: 0,
        };
        let mut selector = Selector::new(main, allocation, crate::target::X86_64_LINUX).unwrap();
        selector
            .select_binary(
                destination,
                NativeBinaryOp::Add,
                NativeValue::Register(destination),
                NativeValue::Immediate(1),
                MachineType::I64,
            )
            .unwrap();
        let mut expected = Encoder::new(8);
        expected.add_reg_imm(Register::R8, 1, Width::Qword).unwrap();
        assert_eq!(selector.encoder.bytes(), expected.bytes());
    }

    #[test]
    fn selector_compares_allocated_register_directly_for_branch() {
        let project = mir(
            "pub fn main(value: u64) -> u64:\n    if value < (10 as u64):\n        return 42 as u64\n    return 1 as u64\n",
        );
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        let value = main.parameters[0];
        let allocation = RegisterAllocation {
            locations: BTreeMap::from([(
                value,
                RegisterLocation::Register(physical(Register::R8)),
            )]),
            spill_slots: 0,
        };
        let mut selector = Selector::new(main, allocation, crate::target::X86_64_LINUX).unwrap();
        selector
            .select_compare_branch(
                (
                    NativeCompareOp::UnsignedLess,
                    NativeValue::Register(value),
                    NativeValue::Immediate(10),
                    MachineType::I64,
                ),
                BlockId(1),
                BlockId(2),
                Some(BlockId(2)),
            )
            .unwrap();
        let mut expected = Encoder::new(16);
        expected
            .cmp_reg_imm(Register::R8, 10, Width::Qword)
            .unwrap();
        assert!(selector.encoder.bytes().starts_with(expected.bytes()));
    }

    #[test]
    fn selector_fuses_single_use_unsigned_byte_load_widening_cast() {
        let project = mir(
            "pub fn main() -> i64:\n    let view = \"abcd\"\n    let index: usize = 0 as usize\n    let total: u64 = 0 as u64\n    while index < len(view):\n        total = total + (view[index] as u64)\n        index = index + (1 as usize)\n    return total as i64\n",
        );
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        assert!(main.blocks.iter().any(|block| {
            block.instructions.windows(2).any(|pair| {
                matches!(
                    (&pair[0].instruction, &pair[1].instruction),
                    (
                        NativeMirInst::Load {
                            destination: loaded,
                            ty: MachineType::I8,
                            volatile: false,
                            ..
                        },
                        NativeMirInst::Cast {
                            value,
                            from: MachineType::I8,
                            to: MachineType::I64,
                            signed: false,
                            ..
                        }
                    ) if loaded == value && main.register_use_count(*loaded) == 1
                )
            })
        }));
        let selected = select_function(main, NativeOptimizationLevel::O1).unwrap();
        assert!(!selected
            .bytes
            .windows(4)
            .any(|bytes| bytes == [0x48, 0x0f, 0xb6, 0xc0]));
    }

    #[test]
    fn selector_load_widening_fusion_rejects_signed_and_multi_use_values() {
        let source = "pub fn main() -> i64:\n    let view = \"abcd\"\n    let index: usize = 0 as usize\n    let total: u64 = 0 as u64\n    while index < len(view):\n        total = total + (view[index] as u64)\n        index = index + (1 as usize)\n    return total as i64\n";
        let mut signed_project = mir(source);
        let main_index = signed_project
            .functions
            .iter()
            .position(|function| function.symbol == "main")
            .unwrap();
        let (block_index, instruction_index, loaded) = {
            let main = &signed_project.functions[main_index];
            main.blocks
                .iter()
                .enumerate()
                .find_map(|(block_index, block)| {
                    block.instructions.windows(2).enumerate().find_map(
                        |(instruction_index, pair)| match (
                            &pair[0].instruction,
                            &pair[1].instruction,
                        ) {
                            (
                                NativeMirInst::Load {
                                    destination: loaded,
                                    ty: MachineType::I8,
                                    volatile: false,
                                    ..
                                },
                                NativeMirInst::Cast {
                                    value,
                                    from: MachineType::I8,
                                    to: MachineType::I64,
                                    signed: false,
                                    ..
                                },
                            ) if loaded == value => Some((block_index, instruction_index, *loaded)),
                            _ => None,
                        },
                    )
                })
                .unwrap()
        };
        let NativeMirInst::Cast { signed, .. } = &mut signed_project.functions[main_index].blocks
            [block_index]
            .instructions[instruction_index + 1]
            .instruction
        else {
            unreachable!()
        };
        *signed = true;
        let signed = select_function(
            &signed_project.functions[main_index],
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert!(signed
            .bytes
            .windows(4)
            .any(|bytes| bytes == [0x48, 0x0f, 0xb6, 0xc0]));

        let mut multi_use_project = mir(source);
        let main = &mut multi_use_project.functions[main_index];
        let extra = VirtualRegister(main.register_types.len() as u32);
        main.register_types.push(MachineType::I8);
        main.blocks[block_index].instructions.insert(
            instruction_index + 2,
            NativeMirInst::Copy {
                destination: extra,
                source: loaded,
                ty: MachineType::I8,
            }
            .into(),
        );
        assert_eq!(main.register_use_count(loaded), 2);
        let multi_use = select_function(main, NativeOptimizationLevel::O1).unwrap();
        assert!(multi_use
            .bytes
            .windows(4)
            .any(|bytes| bytes == [0x48, 0x0f, 0xb6, 0xc0]));
    }

    #[test]
    fn selector_uses_layout_fallthrough_for_adjacent_successors() {
        let project = mir(
            "pub fn choose(value: u64) -> i64:\n    if value < (10 as u64):\n        return 42 as i64\n    return 1 as i64\n",
        );
        let choose = project
            .functions
            .iter()
            .find(|function| function.symbol == "choose")
            .unwrap();
        let naive_branches: u32 = choose
            .blocks
            .iter()
            .map(|block| match block.terminator {
                NativeTerminator::Jump(_) | NativeTerminator::Return(_) => 1,
                NativeTerminator::Branch { .. } => 2,
                NativeTerminator::ReturnAggregate { .. } => 1,
                NativeTerminator::Trap | NativeTerminator::Unreachable => 0,
            })
            .sum();
        assert!(choose
            .blocks
            .windows(2)
            .any(|pair| match pair[0].terminator {
                NativeTerminator::Jump(target) => target == pair[1].id,
                NativeTerminator::Branch {
                    then_block,
                    else_block,
                    ..
                } => {
                    then_block == pair[1].id || else_block == pair[1].id
                }
                _ => false,
            }));
        let selected = select_function(choose, NativeOptimizationLevel::O1).unwrap();
        assert!(selected.statistics.branches < naive_branches);
    }

    #[test]
    fn aggregate_parameter_addressing_preserves_first_dbyte64_scalar_register() {
        let project = mir(
            "struct Pair:\n    left: u64\n    right: u64\n\npub fn capture(pair: Pair, pointer: usize, size: usize, alignment: usize) -> usize:\n    return pointer\n",
        );
        let capture = project
            .functions
            .iter()
            .find(|function| function.symbol == "capture")
            .unwrap();
        assert!(capture.parameter_aggregates[0].is_some());
        let mut expected = Encoder::new(16);
        expected
            .lea_base_disp32(SCRATCH_OTHER, Register::Rbp, 16)
            .unwrap();
        for level in [NativeOptimizationLevel::O0, NativeOptimizationLevel::O1] {
            let selected = select_function(capture, level).unwrap();
            assert!(
                selected
                    .bytes
                    .windows(expected.bytes().len())
                    .any(|window| window == expected.bytes()),
                "{level}: aggregate parameter address clobbers an incoming DBYTE64 scalar register"
            );
        }
    }

    #[test]
    fn callee_snapshots_register_parameters_before_parallel_location_moves() {
        let project =
            mir("pub fn fourth(a: usize, b: usize, c: usize, d: usize) -> usize:\n    return d\n");
        let fourth = project
            .functions
            .iter()
            .find(|function| function.symbol == "fourth")
            .unwrap();
        let incoming = abi_argument_registers(
            dbyte_ast::CallConvention::DByte,
            crate::target::X86_64_LINUX,
        )
        .unwrap();
        let mut expected = Encoder::new(32);
        for register in incoming[..4].iter().copied() {
            expected.push(register).unwrap();
        }
        let selected = select_function(fourth, NativeOptimizationLevel::O1).unwrap();
        assert!(
            selected
                .bytes
                .windows(expected.bytes().len())
                .any(|window| window == expected.bytes()),
            "callee parameter setup did not snapshot incoming DBYTE64 registers before parallel moves"
        );
    }

    #[test]
    fn direct_calls_produce_checked_relocations_and_call_safe_allocations() {
        let project = mir(
            "pub fn add(a: i64, b: i64) -> i64:\n    return a + b\n\npub fn main() -> i64:\n    let retained: i64 = 40 as i64\n    return add(retained, 2 as i64)\n",
        );
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        let selected = select_function(main, NativeOptimizationLevel::O1).unwrap();
        assert_eq!(selected.relocations.len(), 1);
        assert_eq!(selected.relocations[0].symbol, "add");
        assert_eq!(selected.statistics.calls, 1);
    }

    #[test]
    fn dbyte64_direct_call_skips_snapshot_when_sources_cannot_alias_targets() {
        let project = mir(
            "pub fn add3(a: u64, b: u64, c: u64) -> u64:\n    return a + b + c\n\npub fn main() -> i64:\n    let a: u64 = 40 as u64\n    let b: u64 = 1 as u64\n    let c: u64 = 1 as u64\n    return add3(a, b, c) as i64\n",
        );
        let main = project
            .functions
            .iter()
            .find(|function| function.symbol == "main")
            .unwrap();
        let selected = select_function(main, NativeOptimizationLevel::O1).unwrap();
        assert_eq!(selected.relocations.len(), 1);
        assert_eq!(selected.relocations[0].symbol, "add3");
        let mut push_r11 = Encoder::new(4);
        push_r11.push(SCRATCH_OTHER).unwrap();
        assert!(!selected
            .bytes
            .windows(push_r11.bytes().len())
            .any(|bytes| bytes == push_r11.bytes()));
    }

    #[test]
    fn register_argument_direct_move_rejects_target_source_alias() {
        let project =
            mir("pub fn fourth(a: usize, b: usize, c: usize, d: usize) -> usize:\n    return d\n");
        let fourth = project
            .functions
            .iter()
            .find(|function| function.symbol == "fourth")
            .unwrap();
        let allocation = RegisterAllocation {
            locations: BTreeMap::from([
                (
                    fourth.parameters[0],
                    RegisterLocation::Register(physical(Register::R8)),
                ),
                (
                    fourth.parameters[1],
                    RegisterLocation::Register(physical(Register::Rbx)),
                ),
                (
                    fourth.parameters[2],
                    RegisterLocation::Register(physical(Register::R12)),
                ),
                (
                    fourth.parameters[3],
                    RegisterLocation::Register(physical(Register::R13)),
                ),
            ]),
            spill_slots: 0,
        };
        let mut selector = Selector::new(fourth, allocation, crate::target::X86_64_LINUX).unwrap();
        let targets = &abi_argument_registers(
            dbyte_ast::CallConvention::DByte,
            crate::target::X86_64_LINUX,
        )
        .unwrap()[..4];
        assert!(!selector
            .register_arguments_can_move_directly(&fourth.parameters, targets)
            .unwrap());
        selector
            .prepare_register_arguments(&fourth.parameters, targets)
            .unwrap();
        let mut push_r11 = Encoder::new(4);
        push_r11.push(SCRATCH_OTHER).unwrap();
        assert!(selector
            .encoder
            .bytes()
            .windows(push_r11.bytes().len())
            .any(|bytes| bytes == push_r11.bytes()));
    }

    #[test]
    fn sysv64_c_calls_and_definitions_use_target_register_contract() {
        let project = mir(
            "extern \"C\" fn c_add(left: u64, right: u64) -> u64\n\npub extern \"C\" fn callback(left: u64, right: u64) -> u64:\n    return left + right\n\npub fn main() -> i64:\n    return c_add(40 as u64, 2 as u64) as i64\n",
        );
        let callback = project
            .functions
            .iter()
            .find(|function| function.symbol == "callback")
            .unwrap();
        let selected = select_function_for_target(
            callback,
            crate::target::X86_64_LINUX,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert!(!selected.bytes.is_empty());
        let object = select_project_object(
            &project,
            crate::target::X86_64_LINUX,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let parsed = crate::object64::parse_relocatable(&object.object.bytes).unwrap();
        assert!(parsed
            .relocations
            .iter()
            .any(|relocation| relocation.symbol == "c_add"));
    }

    #[test]
    fn sysv64_seventh_scalar_argument_uses_stack_for_call_and_definition() {
        let project = mir(
            "extern \"C\" fn c_sum7(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64) -> u64\n\npub extern \"C\" fn callback7(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64) -> u64:\n    return g\n\npub fn main() -> i64:\n    return c_sum7(1 as u64, 2 as u64, 3 as u64, 4 as u64, 5 as u64, 6 as u64, 42 as u64) as i64\n",
        );
        let callback = project
            .functions
            .iter()
            .find(|function| function.symbol == "callback7")
            .unwrap();
        let selected = select_function_for_target(
            callback,
            crate::target::X86_64_LINUX,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert!(!selected.bytes.is_empty());
        let object = select_project_object(
            &project,
            crate::target::X86_64_LINUX,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let parsed = crate::object64::parse_relocatable(&object.object.bytes).unwrap();
        assert!(parsed
            .relocations
            .iter()
            .any(|relocation| relocation.symbol == "c_sum7"));
    }
    #[test]
    fn win64_import_calls_emit_coff_shadow_space_and_iat_relocations() {
        let project = mir_for(
            "fn windows_get_std_handle_raw(kind: i32) -> usize:\n    return windows.get_std_handle(kind)\n\nfn windows_write_file_raw(handle: usize, text: str, length: u32) -> i32:\n    return windows.write_file(handle, text, length, 0 as usize, 0 as usize)\n\npub fn main() -> int:\n    let output = windows_get_std_handle_raw(-11 as i32)\n    windows_write_file_raw(output, \"WIN MIR OK\\r\\n\", 12 as u32)\n    return 42\n",
            Profile::X86_64Windows,
        );
        let selected = select_project_object(
            &project,
            crate::target::X86_64_WINDOWS,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let parsed = crate::coff64::parse_object(&selected.object.bytes).unwrap();
        assert!(parsed
            .relocations
            .iter()
            .any(|relocation| relocation.symbol == "__imp_WriteFile"));
        assert!(parsed
            .relocations
            .iter()
            .any(|relocation| relocation.symbol == "__imp_GetStdHandle"));
        assert!(selected.machine_dump.contains("calls="));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "executes a verifier-owned PE on the Windows host"]
    fn selected_win64_pe_executes_import_calls_on_the_host() {
        let project = mir_for(
            "fn windows_get_std_handle_raw(kind: i32) -> usize:\n    return windows.get_std_handle(kind)\n\nfn windows_write_file_raw(handle: usize, text: str, length: u32) -> i32:\n    return windows.write_file(handle, text, length, 0 as usize, 0 as usize)\n\npub fn main() -> int:\n    let output = windows_get_std_handle_raw(-11 as i32)\n    windows_write_file_raw(output, \"WIN MIR OK\\r\\n\", 12 as u32)\n    return 42\n",
            Profile::X86_64Windows,
        );
        for level in [NativeOptimizationLevel::O0, NativeOptimizationLevel::O1] {
            let selected =
                select_project_object(&project, crate::target::X86_64_WINDOWS, level).unwrap();
            let pe = crate::modern_native::link_windows_object_bytes(&[
                crate::linker64::LinkInput::new("mir.obj", selected.object.bytes),
            ])
            .unwrap();
            let path = std::env::temp_dir().join(format!(
                "dbyte-win64-mir-{}-{level}.exe",
                std::process::id()
            ));
            std::fs::write(&path, pe.bytes).unwrap();
            let output = std::process::Command::new(&path).output().unwrap();
            assert_eq!(output.status.code(), Some(42), "{level}");
            assert_eq!(output.stdout, b"WIN MIR OK\r\n", "{level}");
            std::fs::remove_file(path).unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires WSL2 GRUB tools and QEMU x86_64"]
    fn selected_none_image_boots_long_mode_and_emits_exact_serial() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_v12_2_none/bootstrap.dby");
        let artifact = crate::modern_native::compile_mir_source(
            &source,
            crate::target::X86_64_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!("dbyte-none-mir-{}", std::process::id()));
        let grub = root.join("iso/boot/grub");
        std::fs::create_dir_all(&grub).unwrap();
        std::fs::write(root.join("iso/boot/kernel.elf"), artifact.elf.bytes).unwrap();
        std::fs::write(
            grub.join("grub.cfg"),
            "set timeout=0\nset default=0\nmenuentry 'DByte MIR' {\n multiboot2 /boot/kernel.elf\n boot\n}\n",
        )
        .unwrap();
        let translate = |path: &std::path::Path| {
            let output = std::process::Command::new("wsl.exe")
                .args([
                    "--distribution",
                    "Debian",
                    "--exec",
                    "wslpath",
                    "-a",
                    "-u",
                    &path.to_string_lossy(),
                ])
                .output()
                .unwrap();
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        let iso = root.join("runtime.iso");
        let grub_output = std::process::Command::new("wsl.exe")
            .args([
                "--distribution",
                "Debian",
                "--exec",
                "grub-mkrescue",
                "-o",
                &translate(&iso),
                &translate(&root.join("iso")),
            ])
            .output()
            .unwrap();
        assert!(
            grub_output.status.success(),
            "{}",
            String::from_utf8_lossy(&grub_output.stderr)
        );
        let serial = root.join("serial.log");
        let trace = root.join("trace.log");
        let mut child = std::process::Command::new(r"C:\Program Files\qemu\qemu-system-x86_64.exe")
            .args(["-cdrom", &iso.to_string_lossy()])
            .args(["-m", "32M", "-display", "none", "-monitor", "none"])
            .args(["-serial", &format!("file:{}", serial.display())])
            .args([
                "-d",
                "int,cpu_reset,guest_errors",
                "-D",
                &trace.to_string_lossy(),
            ])
            .args(["-no-reboot", "-no-shutdown"])
            .spawn()
            .unwrap();
        let expected =
            b"DBYTE X86_64 NONE\r\nLONG MODE ONLINE\r\nDBYTE64 ABI ONLINE\r\nNONE CHECK OK\r\n";
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut actual = Vec::new();
        while std::time::Instant::now() < deadline {
            if let Ok(bytes) = std::fs::read(&serial) {
                actual = bytes;
                if actual == expected {
                    break;
                }
                assert!(expected.starts_with(&actual), "serial diverged: {actual:?}");
            }
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(actual, expected);
        let trace = std::fs::read_to_string(trace).unwrap();
        assert!(!trace.contains("Triple fault"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn selector_rejects_unlegalized_narrow_division() {
        let project = mir("pub fn main() -> i16:\n    return (9 as i16) / (2 as i16)\n");
        let error =
            select_function(&project.functions[0], NativeOptimizationLevel::O0).unwrap_err();
        assert_eq!(error.code, "E_NATIVE_SELECT_X86_64");
        assert!(error.message.contains("legalized"));
    }

    #[test]
    fn unused_parameters_still_receive_deterministic_entry_locations() {
        let project = mir(
            "pub fn ignore(value: i64) -> i64:\n    return 42 as i64\n\npub fn main() -> i64:\n    return ignore(0 as i64)\n",
        );
        let ignore = project
            .functions
            .iter()
            .find(|function| function.symbol == "ignore")
            .unwrap();
        let selected = select_function(ignore, NativeOptimizationLevel::O1).unwrap();
        assert!(selected.statistics.assigned_registers >= 1);
    }

    #[test]
    fn selected_project_is_real_elf64_et_rel_and_links_without_relocations() {
        let project = mir(
            "pub fn add(a: i64, b: i64) -> i64:\n    return a + b\n\npub fn main() -> i64:\n    return add(40 as i64, 2 as i64)\n",
        );
        let selected = select_project_object(
            &project,
            crate::target::X86_64_LINUX,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let parsed = crate::object64::parse_relocatable(&selected.object.bytes).unwrap();
        assert!(parsed.symbols.iter().any(|symbol| symbol.name == "main"));
        assert!(parsed
            .relocations
            .iter()
            .any(|relocation| relocation.symbol == "add"));
        let elf = crate::modern_native::link_object_bytes(
            &[crate::linker64::LinkInput::new(
                "mir.o",
                selected.object.bytes,
            )],
            crate::target::X86_64_LINUX,
        )
        .unwrap();
        let inspection = crate::elf64::inspect(&elf.bytes).unwrap();
        assert!(!inspection.has_dynamic);
        assert!(selected.machine_dump.contains("assigned="));
    }

    #[test]
    fn tracked_native_core_corpus_compiles_through_the_mir_production_seam() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/native");
        for name in [
            "integer_loop.dby",
            "branch_loop.dby",
            "direct_calls.dby",
            "indirect_calls.dby",
            "scalar_struct.dby",
            "wide_arithmetic.dby",
        ] {
            let artifact = crate::modern_native::compile_mir_source(
                &root.join(name),
                crate::target::X86_64_LINUX,
                NativeOptimizationLevel::O1,
            )
            .unwrap_or_else(|failure| panic!("{name}: {failure}"));
            assert!(!artifact.elf.bytes.is_empty(), "{name}");
            assert!(artifact.machine_dump.contains("native-codegen opt=O1"));
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires WSL2 Debian execution"]
    fn tracked_native_core_corpus_executes_o0_and_o1_with_exact_results() {
        let source_root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/native");
        let output_root =
            std::env::temp_dir().join(format!("dbyte-native-corpus-{}", std::process::id()));
        std::fs::create_dir_all(&output_root).unwrap();
        for name in [
            "integer_loop",
            "branch_loop",
            "direct_calls",
            "indirect_calls",
            "scalar_struct",
            "wide_arithmetic",
        ] {
            for level in [NativeOptimizationLevel::O0, NativeOptimizationLevel::O1] {
                let artifact = crate::modern_native::compile_mir_source(
                    &source_root.join(format!("{name}.dby")),
                    crate::target::X86_64_LINUX,
                    level,
                )
                .unwrap();
                let path = output_root.join(format!("{name}-{level}.elf"));
                std::fs::write(&path, artifact.elf.bytes).unwrap();
                let translated = std::process::Command::new("wsl.exe")
                    .args([
                        "--distribution",
                        "Debian",
                        "--exec",
                        "wslpath",
                        "-a",
                        "-u",
                        &path.to_string_lossy(),
                    ])
                    .output()
                    .unwrap();
                assert!(translated.status.success(), "{name} {level}");
                let linux_path = String::from_utf8(translated.stdout).unwrap();
                let output = std::process::Command::new("wsl.exe")
                    .args([
                        "--distribution",
                        "Debian",
                        "--exec",
                        "bash",
                        "-lc",
                        &format!("chmod +x '{0}'; '{0}'", linux_path.trim()),
                    ])
                    .output()
                    .unwrap();
                assert_eq!(
                    output.status.code(),
                    Some(42),
                    "{name} {level}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(output.stdout.is_empty(), "{name} {level}");
            }
        }
        std::fs::remove_dir_all(output_root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires WSL2 Debian execution"]
    fn selected_static_elf_executes_real_machine_code_in_wsl() {
        let project = mir(
            "struct Pair:\n    left: u32\n    right: u32\n\nstatic mut RESULT: u32\n\nfn linux_write_raw(fd: i32, text: str, length: usize) -> i64:\n    return linux.write(fd, text, length)\n\nfn step(value: i64) -> i64:\n    return value + (1 as i64)\n\nfn read(value: borrow u32) -> u32:\n    return *value\n\npub fn main() -> i64:\n    let value: i64 = 40 as i64\n    let callback: fn(i64) -> i64 = step\n    while value < (42 as i64):\n        value = callback(value)\n    let narrowed: u32 = value as u32\n    let pair = Pair(left = 0 as u32, right = read(borrow narrowed))\n    RESULT = pair.right\n    linux_write_raw(1 as i32, \"MIR O0 O1 OK\\n\", 13 as usize)\n    return RESULT as i64\n",
        );
        let root = std::env::temp_dir().join(format!("dbyte-native-mir-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for level in [NativeOptimizationLevel::O0, NativeOptimizationLevel::O1] {
            let selected =
                select_project_object(&project, crate::target::X86_64_LINUX, level).unwrap();
            let elf = crate::modern_native::link_object_bytes(
                &[crate::linker64::LinkInput::new(
                    "mir.o",
                    selected.object.bytes,
                )],
                crate::target::X86_64_LINUX,
            )
            .unwrap();
            let path = root.join(format!("exit42-{level}.elf"));
            std::fs::write(&path, &elf.bytes).unwrap();
            let translated = std::process::Command::new("wsl.exe")
                .args([
                    "--distribution",
                    "Debian",
                    "--exec",
                    "wslpath",
                    "-a",
                    "-u",
                    &path.to_string_lossy(),
                ])
                .output()
                .unwrap();
            assert!(translated.status.success());
            let linux_path = String::from_utf8(translated.stdout).unwrap();
            let linux_path = linux_path.trim();
            let output = std::process::Command::new("wsl.exe")
                .args([
                    "--distribution",
                    "Debian",
                    "--exec",
                    "bash",
                    "-lc",
                    &format!("chmod +x '{linux_path}'; '{linux_path}'"),
                ])
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(42),
                "{level} execution diverged: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, b"MIR O0 O1 OK\n", "{level} output diverged");
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
