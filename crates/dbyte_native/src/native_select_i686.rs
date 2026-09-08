//! Deterministic i686 instruction selection from validated Native MIR.
//!
//! This selector is representation-only. Canonical HIR owns semantics and the
//! MIR validator owns CFG/type consistency before this module is entered.

use crate::native_mir::{
    allocate_registers, BlockId, MachineType, NativeBinaryOp, NativeCallee, NativeCompareOp,
    NativeMirError, NativeMirFunction, NativeMirInst, NativeMirProject, NativeOptimizationLevel,
    NativeTerminator, NativeUnaryOp, NativeValue, PhysicalRegister, RegisterAllocation,
    RegisterLocation, RegisterPolicy, VirtualRegister,
};
use crate::object::{
    ObjectArtifact, ObjectImage, ObjectRelocation, ObjectSection, ObjectSectionData, ObjectSymbol,
    RelocationKind, SectionContents, SymbolBinding, SymbolDefinition, SymbolId, SymbolKind,
};
use crate::x86::{
    Condition, Encoder, EncoderError, Extension, Label, MemoryOperand, OperandWidth, Register,
    Scale, ShiftKind,
};
use std::collections::{BTreeMap, BTreeSet};

const VALUE: Register = Register::Eax;
const OTHER: Register = Register::Edx;
const COUNT: Register = Register::Ecx;
const MAX_FUNCTION_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRelocation {
    offset: u32,
    symbol: String,
    kind: RelocationKind,
    addend: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedFunction {
    symbol: String,
    bytes: Vec<u8>,
    relocations: Vec<PendingRelocation>,
    statistics: crate::native_select_x86_64::NativeCodegenStatistics,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedNativeObject32 {
    pub object: ObjectArtifact,
    pub mir_dump: String,
    pub machine_dump: String,
}

pub fn dbyte32_register_policy() -> RegisterPolicy {
    RegisterPolicy {
        // EAX/EDX/ECX remain fixed arithmetic and selector operands. Values
        // crossing calls use the preserved EBX/ESI/EDI set.
        allocatable: vec![
            physical(Register::Ebx),
            physical(Register::Esi),
            physical(Register::Edi),
        ],
        allocatable_pairs: vec![(physical(Register::Esi), physical(Register::Edi))],
        call_clobbered: BTreeSet::new(),
        maximum_register_bytes: 4,
        pointer_bytes: 4,
    }
}

pub fn select_project_object(
    project: &NativeMirProject,
    level: NativeOptimizationLevel,
) -> Result<SelectedNativeObject32, NativeMirError> {
    select_project_object_with_metadata(project, level, &[])
}

pub fn select_project_object_with_metadata(
    project: &NativeMirProject,
    level: NativeOptimizationLevel,
    metadata_symbols: &[String],
) -> Result<SelectedNativeObject32, NativeMirError> {
    select_project_object_with_metadata_and_locals(
        project,
        level,
        metadata_symbols,
        &BTreeSet::new(),
    )
}

pub fn select_project_object_with_metadata_and_locals(
    project: &NativeMirProject,
    level: NativeOptimizationLevel,
    metadata_symbols: &[String],
    local_symbols: &BTreeSet<String>,
) -> Result<SelectedNativeObject32, NativeMirError> {
    if project.pointer_bytes != 4 {
        return Err(codegen_error(
            "DBYTE32 selection requires a four-byte target layout",
        ));
    }
    let mut optimized = project.clone();
    optimized.optimize(level)?;
    optimized.validate()?;

    let mut text = Vec::new();
    let mut rodata = Vec::new();
    let mut data = Vec::new();
    let mut bss_size = 0u32;
    let mut symbols = Vec::<ObjectSymbol>::new();
    let mut definitions = BTreeMap::<String, SymbolId>::new();
    let mut pending = Vec::<(u32, PendingRelocation)>::new();
    let mut machine_dump = String::new();

    for function in &optimized.functions {
        align_bytes(&mut text, 16)?;
        let start = checked_u32(text.len(), "function offset")?;
        let interrupt = function_interrupt_class(function)?;
        let stub = interrupt
            .map(|class| select_interrupt_stub(&function.symbol, class))
            .transpose()?;
        if let Some(stub) = &stub {
            for relocation in &stub.relocations {
                pending.push((start, relocation.clone()));
            }
            text.extend_from_slice(&stub.bytes);
        }
        let body_start = checked_u32(text.len(), "function body offset")?;
        let selected = select_function(function, level)?;
        let body_size = checked_u32(selected.bytes.len(), "function body size")?;
        let public_size = if let Some(stub) = &stub {
            checked_u32(stub.bytes.len(), "interrupt stub size")?
        } else {
            body_size
        };
        let symbol = SymbolId(symbols.len() as u32);
        definitions.insert(selected.symbol.clone(), symbol);
        symbols.push(ObjectSymbol {
            name: selected.symbol.clone(),
            binding: if local_symbols.contains(&selected.symbol) {
                SymbolBinding::Local
            } else {
                SymbolBinding::Global
            },
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: start,
            },
            size: public_size,
        });
        if selected.symbol == "kernel_main" {
            let abi = if function.parameters.is_empty() {
                "legacy"
            } else {
                "multiboot_v1"
            };
            symbols.push(metadata_symbol(format!("__dbyte_entry:{abi}")));
        }
        if let Some(class) = interrupt {
            let body_name = interrupt_body_symbol(&selected.symbol);
            let body_id = SymbolId(symbols.len() as u32);
            definitions.insert(body_name.clone(), body_id);
            symbols.push(ObjectSymbol {
                name: body_name,
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: body_start,
                },
                size: body_size,
            });
            symbols.push(metadata_symbol(format!(
                "__dbyte_interrupt:def:{class}:{}",
                selected.symbol
            )));
        }
        for relocation in selected.relocations {
            pending.push((body_start, relocation));
        }
        machine_dump.push_str(&format!(
            "function {} opt={} mir={} selected={} text={} frame={} vregs={} assigned={} spills={} reloads={} loads={} stores={} calls={} branches={}\n",
            selected.symbol,
            level,
            selected.statistics.mir_instructions,
            selected.statistics.selected_instructions,
            selected.statistics.text_bytes,
            selected.statistics.frame_bytes,
            selected.statistics.virtual_registers,
            selected.statistics.assigned_registers,
            selected.statistics.spills,
            selected.statistics.reloads,
            selected.statistics.loads,
            selected.statistics.stores,
            selected.statistics.calls,
            selected.statistics.branches,
        ));
        text.extend_from_slice(&selected.bytes);
    }

    for global in &optimized.globals {
        let (section, offset) = if global.bytes.is_empty() {
            bss_size = align_up(bss_size, global.alignment)?;
            let offset = bss_size;
            bss_size = bss_size
                .checked_add(global.size)
                .ok_or_else(|| codegen_error("BSS exceeds u32"))?;
            (ObjectSection::Bss, offset)
        } else if global.mutable {
            align_bytes(&mut data, global.alignment)?;
            let offset = checked_u32(data.len(), "data global offset")?;
            data.extend_from_slice(&global.bytes);
            data.resize(
                usize::try_from(offset + global.size)
                    .map_err(|_| codegen_error("data global exceeds usize"))?,
                0,
            );
            (ObjectSection::Data, offset)
        } else {
            align_bytes(&mut rodata, global.alignment)?;
            let offset = checked_u32(rodata.len(), "rodata global offset")?;
            rodata.extend_from_slice(&global.bytes);
            rodata.resize(
                usize::try_from(offset + global.size)
                    .map_err(|_| codegen_error("rodata global exceeds usize"))?,
                0,
            );
            (ObjectSection::Rodata, offset)
        };
        let id = SymbolId(symbols.len() as u32);
        definitions.insert(global.symbol.clone(), id);
        symbols.push(ObjectSymbol {
            name: global.symbol.clone(),
            binding: if global.symbol.starts_with("__dbyte_string_")
                || local_symbols.contains(&global.symbol)
            {
                SymbolBinding::Local
            } else {
                SymbolBinding::Global
            },
            kind: SymbolKind::Object,
            definition: SymbolDefinition::Defined {
                section,
                value: offset,
            },
            size: global.size,
        });
    }

    let mut referenced = pending
        .iter()
        .map(|(_, relocation)| relocation.symbol.clone())
        .collect::<BTreeSet<_>>();
    referenced.retain(|symbol| !definitions.contains_key(symbol));
    for name in referenced {
        let id = SymbolId(symbols.len() as u32);
        definitions.insert(name.clone(), id);
        let kind = match name.as_str() {
            "__dbyte_image_start"
            | "__dbyte_image_end"
            | "__dbyte_stack_bottom"
            | "__dbyte_stack_top" => SymbolKind::Object,
            _ => SymbolKind::Function,
        };
        symbols.push(ObjectSymbol {
            name,
            binding: SymbolBinding::Global,
            kind,
            definition: SymbolDefinition::Undefined,
            size: 0,
        });
    }

    let mut metadata = metadata_symbols.iter().cloned().collect::<BTreeSet<_>>();
    for function in &optimized.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                match &instruction.instruction {
                    NativeMirInst::InterruptAddress {
                        vector,
                        symbol,
                        class,
                        ..
                    } => {
                        metadata.insert(format!(
                            "__dbyte_interrupt:use:{vector:03}:{class}:{symbol}"
                        ));
                    }
                    NativeMirInst::TaskAddress { symbol, .. } => {
                        metadata.insert(format!("__dbyte_task:def:{symbol}"));
                        metadata.insert(format!("__dbyte_task:use:{symbol}"));
                    }
                    NativeMirInst::GlobalAddress { symbol, .. }
                        if symbol == "__dbyte_task_trampoline" =>
                    {
                        metadata.insert("__dbyte_task:trampoline".to_owned());
                    }
                    _ => {}
                }
            }
        }
    }
    for name in metadata {
        symbols.push(metadata_symbol(name));
    }

    let relocations = pending
        .into_iter()
        .map(|(base, relocation)| {
            Ok(ObjectRelocation {
                section: ObjectSection::Text,
                offset: base
                    .checked_add(relocation.offset)
                    .ok_or_else(|| codegen_error("text relocation offset exceeds u32"))?,
                symbol: definitions[&relocation.symbol],
                kind: relocation.kind,
                addend: relocation.addend,
            })
        })
        .collect::<Result<Vec<_>, NativeMirError>>()?;
    let image = ObjectImage {
        sections: BTreeMap::from([
            (ObjectSection::Text, ObjectSectionData::bytes(text, 16)),
            (ObjectSection::Rodata, ObjectSectionData::bytes(rodata, 4)),
            (ObjectSection::Data, ObjectSectionData::bytes(data, 4)),
            (
                ObjectSection::Bss,
                ObjectSectionData {
                    contents: SectionContents::Uninitialized(bss_size),
                    alignment: 4,
                },
            ),
        ]),
        symbols,
        relocations,
    };
    let object = crate::object::write_relocatable(&image)
        .map_err(|diagnostic| NativeMirError::new("E_NATIVE_SELECT_I686", diagnostic.message))?;
    Ok(SelectedNativeObject32 {
        object,
        mir_dump: optimized.dump(),
        machine_dump,
    })
}

#[derive(Debug, Clone)]
struct SelectedInterruptStub {
    bytes: Vec<u8>,
    relocations: Vec<PendingRelocation>,
}

fn interrupt_body_symbol(symbol: &str) -> String {
    format!("__dbyte_interrupt_{symbol}_body")
}

fn metadata_symbol(name: String) -> ObjectSymbol {
    ObjectSymbol {
        name,
        binding: SymbolBinding::Local,
        kind: SymbolKind::Function,
        definition: SymbolDefinition::Defined {
            section: ObjectSection::Text,
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
    let mut encoder = Encoder::new(256);
    encoder.pushad().map_err(encode_error)?;
    if !class.has_error_code() {
        encoder
            .lea_reg_from_memory(Register::Eax, MemoryOperand::new(Register::Esp, 32))
            .map_err(encode_error)?;
        encoder.push_reg(Register::Eax).map_err(encode_error)?;
    } else {
        encoder
            .mov_reg_from_memory(
                Register::Eax,
                MemoryOperand::new(Register::Esp, 32),
                OperandWidth::Dword,
                None,
            )
            .map_err(encode_error)?;
        encoder
            .lea_reg_from_memory(Register::Edx, MemoryOperand::new(Register::Esp, 36))
            .map_err(encode_error)?;
        encoder.push_reg(Register::Edx).map_err(encode_error)?;
        encoder.push_reg(Register::Eax).map_err(encode_error)?;
    }
    let immediate = encoder.call_rel32_placeholder().map_err(encode_error)?;
    encoder
        .add_esp_imm32(if class.has_error_code() { 8 } else { 4 })
        .map_err(encode_error)?;
    encoder.popad().map_err(encode_error)?;
    if class.has_error_code() {
        encoder.add_esp_imm32(4).map_err(encode_error)?;
    }
    encoder.iretd().map_err(encode_error)?;
    let encoded = encoder.finish().map_err(encode_error)?;
    Ok(SelectedInterruptStub {
        bytes: encoded.bytes,
        relocations: vec![PendingRelocation {
            offset: checked_u32(immediate, "interrupt body call relocation")?,
            symbol: interrupt_body_symbol(symbol),
            kind: RelocationKind::Pc32,
            addend: -4,
        }],
    })
}

fn select_function(
    function: &NativeMirFunction,
    level: NativeOptimizationLevel,
) -> Result<SelectedFunction, NativeMirError> {
    function.validate()?;
    let allocation = allocate_registers(function, &dbyte32_register_policy(), level)?;
    Selector::new(function, allocation)?.select()
}

struct Selector<'a> {
    function: &'a NativeMirFunction,
    allocation: RegisterAllocation,
    encoder: Encoder,
    labels: BTreeMap<BlockId, Label>,
    epilogue: Label,
    saved: Vec<Register>,
    frame_bytes: u32,
    spill_offsets: BTreeMap<u32, u32>,
    sret_displacement: Option<i32>,
    stack_offsets: BTreeMap<crate::native_mir::StackObjectId, u32>,
    relocations: Vec<PendingRelocation>,
    statistics: crate::native_select_x86_64::NativeCodegenStatistics,
}

impl<'a> Selector<'a> {
    fn new(
        function: &'a NativeMirFunction,
        allocation: RegisterAllocation,
    ) -> Result<Self, NativeMirError> {
        let assigned = allocation
            .locations
            .values()
            .filter_map(|location| match location {
                RegisterLocation::Register(register) => Some(machine_register(*register)),
                RegisterLocation::RegisterPair { low, high } => {
                    // Flattened below so both halves participate in the
                    // callee-saved prologue contract.
                    debug_assert_ne!(low, high);
                    None
                }
                RegisterLocation::Spill { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let pair_assigned = allocation
            .locations
            .values()
            .flat_map(|location| match location {
                RegisterLocation::RegisterPair { low, high } => {
                    vec![machine_register(*low), machine_register(*high)]
                }
                _ => Vec::new(),
            })
            .collect::<BTreeSet<_>>();
        let saved = [Register::Ebx, Register::Esi, Register::Edi]
            .into_iter()
            .filter(|register| assigned.contains(register) || pair_assigned.contains(register))
            .collect::<Vec<_>>();
        let (spill_bytes, spill_offsets) = layout_spill_slots(function, &allocation)?;
        let fixed_bytes = spill_bytes
            .checked_add(if function.aggregate_result.is_some() {
                4
            } else {
                0
            })
            .ok_or_else(|| codegen_error("native fixed frame exceeds u32"))?;
        let (object_bytes, stack_offsets) = layout_stack_objects(function, fixed_bytes)?;
        let frame_bytes = align_up(
            fixed_bytes
                .checked_add(object_bytes)
                .ok_or_else(|| codegen_error("native frame exceeds u32"))?,
            4,
        )?;
        let sret_displacement = function
            .aggregate_result
            .map(|_| {
                let end = (saved.len() as u32)
                    .checked_mul(4)
                    .and_then(|bytes| bytes.checked_add(spill_bytes))
                    .and_then(|bytes| bytes.checked_add(4))
                    .ok_or_else(|| codegen_error("sret displacement exceeds u32"))?;
                i32::try_from(end)
                    .ok()
                    .and_then(i32::checked_neg)
                    .ok_or_else(|| codegen_error("sret displacement exceeds i32"))
            })
            .transpose()?;
        let mut encoder = Encoder::new(MAX_FUNCTION_BYTES);
        let mut labels = BTreeMap::new();
        for block in &function.blocks {
            labels.insert(block.id, encoder.new_label().map_err(encode_error)?);
        }
        let epilogue = encoder.new_label().map_err(encode_error)?;
        Ok(Self {
            function,
            allocation,
            encoder,
            labels,
            epilogue,
            saved,
            frame_bytes,
            spill_offsets,
            sret_displacement,
            stack_offsets,
            relocations: Vec::new(),
            statistics: Default::default(),
        })
    }

    fn select(mut self) -> Result<SelectedFunction, NativeMirError> {
        self.emit(|encoder| encoder.push_reg(Register::Ebp))?;
        self.emit(|encoder| encoder.mov_reg_reg(Register::Ebp, Register::Esp))?;
        for register in self.saved.clone() {
            self.emit(|encoder| encoder.push_reg(register))?;
        }
        if self.frame_bytes != 0 {
            let bytes = self.frame_bytes;
            self.emit(|encoder| encoder.sub_esp_imm32(bytes))?;
        }
        self.move_parameters()?;
        for block in &self.function.blocks {
            self.encoder
                .bind_label(self.labels[&block.id])
                .map_err(encode_error)?;
            for instruction in &block.instructions {
                self.statistics.mir_instructions += 1;
                self.select_instruction(&instruction.instruction)?;
            }
            self.select_terminator(&block.terminator)?;
        }
        self.encoder
            .bind_label(self.epilogue)
            .map_err(encode_error)?;
        if self.frame_bytes != 0 {
            let bytes = self.frame_bytes;
            self.emit(|encoder| encoder.add_esp_imm32(bytes))?;
        }
        for register in self.saved.clone().into_iter().rev() {
            self.emit(|encoder| encoder.pop_reg(register))?;
        }
        self.emit(|encoder| encoder.pop_reg(Register::Ebp))?;
        self.emit(Encoder::ret)?;
        let encoded = self.encoder.finish().map_err(encode_error)?;
        self.statistics.text_bytes = checked_u32(encoded.bytes.len(), "selected function size")?;
        self.statistics.frame_bytes = self.frame_bytes;
        self.statistics.virtual_registers = self.function.register_types.len() as u32;
        self.statistics.spills = self.allocation.spill_slots;
        self.statistics.assigned_registers = self
            .allocation
            .locations
            .values()
            .map(|location| match location {
                RegisterLocation::Register(_) => 1,
                RegisterLocation::RegisterPair { .. } => 2,
                RegisterLocation::Spill { .. } => 0,
            })
            .sum();
        Ok(SelectedFunction {
            symbol: self.function.symbol.clone(),
            bytes: encoded.bytes,
            relocations: self.relocations,
            statistics: self.statistics,
        })
    }

    fn move_parameters(&mut self) -> Result<(), NativeMirError> {
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
            return Err(unsupported("aggregate by-value C parameter on i686"));
        }
        if let Some(displacement) = self.sret_displacement {
            if self.function.convention == dbyte_ast::CallConvention::C {
                return Err(unsupported("aggregate return C ABI on i686"));
            }
            let source_bytes = self
                .function
                .parameters
                .iter()
                .copied()
                .zip(self.function.parameter_aggregates.iter().copied())
                .try_fold(0u32, |total, (parameter, aggregate)| {
                    let bytes = if let Some(layout) = aggregate {
                        align_up(layout.bytes, 4)?
                    } else {
                        parameter_stack_bytes(self.function.register_types[parameter.0 as usize])
                    };
                    total
                        .checked_add(bytes)
                        .ok_or_else(|| codegen_error("sret incoming offset exceeds u32"))
                })?;
            let incoming = i32::try_from(
                8u32.checked_add(source_bytes)
                    .ok_or_else(|| codegen_error("sret incoming offset exceeds u32"))?,
            )
            .map_err(|_| codegen_error("sret incoming offset exceeds i32"))?;
            self.emit(|encoder| encoder.mov_reg_from_ebp_disp32(Register::Eax, incoming))?;
            self.emit(|encoder| encoder.mov_ebp_disp32_from_reg(displacement, Register::Eax))?;
        }
        let multiboot_entry = self.function.symbol == "kernel_main"
            && self.function.parameters.len() == 2
            && self.function.parameter_aggregates == [None, None]
            && self.function.register_types[self.function.parameters[0].0 as usize]
                == MachineType::I32
            && self.function.register_types[self.function.parameters[1].0 as usize]
                == MachineType::Pointer;
        for (index, parameter) in self.function.parameters.iter().copied().enumerate() {
            let ty = self.function.register_types[parameter.0 as usize];
            let mut payload_indices: Box<dyn Iterator<Item = usize>> =
                if self.function.convention == dbyte_ast::CallConvention::C || multiboot_entry {
                    Box::new(0..index)
                } else {
                    Box::new(index + 1..self.function.parameters.len())
                };
            let payload_bytes = payload_indices.try_fold(0u32, |total, payload_index| {
                let payload_parameter = self.function.parameters[payload_index];
                let bytes = if let Some(layout) = self.function.parameter_aggregates[payload_index]
                {
                    align_up(layout.bytes, 4)?
                } else {
                    parameter_stack_bytes(
                        self.function.register_types[payload_parameter.0 as usize],
                    )
                };
                total
                    .checked_add(bytes)
                    .ok_or_else(|| codegen_error("parameter payload exceeds u32"))
            })?;
            let displacement = i32::try_from(
                8u32.checked_add(payload_bytes)
                    .ok_or_else(|| codegen_error("parameter displacement exceeds u32"))?,
            )
            .map_err(|_| codegen_error("parameter displacement exceeds i32"))?;
            if self.function.parameter_aggregates[index].is_some() {
                self.emit(|encoder| {
                    encoder.lea_reg_from_memory(
                        Register::Eax,
                        MemoryOperand::new(Register::Ebp, displacement),
                    )
                })?;
                self.store_register(parameter, Register::Eax)?;
            } else if ty == MachineType::I64 {
                let high = displacement
                    .checked_add(4)
                    .ok_or_else(|| codegen_error("wide parameter displacement overflows"))?;
                self.emit(|encoder| encoder.mov_reg_from_ebp_disp32(Register::Eax, displacement))?;
                self.emit(|encoder| encoder.mov_reg_from_ebp_disp32(Register::Edx, high))?;
                self.store_wide_pair(parameter, Register::Eax, Register::Edx)?;
            } else {
                self.emit(|encoder| encoder.mov_reg_from_ebp_disp32(VALUE, displacement))?;
                self.normalize(VALUE, ty)?;
                self.store_register(parameter, VALUE)?;
            }
        }
        if let (Some(register), Some(displacement)) = (
            self.function.aggregate_result_register,
            self.sret_displacement,
        ) {
            self.emit(|encoder| encoder.mov_reg_from_ebp_disp32(Register::Eax, displacement))?;
            self.statistics.loads += 1;
            self.store_register(register, Register::Eax)?;
        }
        Ok(())
    }

    fn select_instruction(&mut self, instruction: &NativeMirInst) -> Result<(), NativeMirError> {
        match instruction {
            NativeMirInst::Const {
                destination,
                value,
                ty,
            } => {
                if *ty == MachineType::I64 {
                    self.store_wide_immediate(*destination, *value)
                } else {
                    self.emit(|encoder| encoder.mov_reg_imm32(VALUE, *value as u32))?;
                    self.store_register(*destination, VALUE)
                }
            }
            NativeMirInst::Copy {
                destination,
                source,
                ty,
            } => {
                if *ty == MachineType::I64 {
                    self.copy_wide(*destination, *source)
                } else {
                    self.load_register(*source, VALUE)?;
                    self.store_register(*destination, VALUE)
                }
            }
            NativeMirInst::Binary {
                destination,
                op,
                left,
                right,
                ty,
            } => {
                if *ty == MachineType::I64 {
                    return self.select_wide_binary(*destination, *op, *left, *right);
                }
                self.load_value(*left, VALUE)?;
                self.load_value(*right, OTHER)?;
                match op {
                    NativeBinaryOp::Add => {
                        self.emit(|encoder| encoder.add_reg_reg(VALUE, OTHER))?
                    }
                    NativeBinaryOp::Sub => {
                        self.emit(|encoder| encoder.sub_reg_reg(VALUE, OTHER))?
                    }
                    NativeBinaryOp::Mul => {
                        self.emit(|encoder| encoder.imul_reg_reg(VALUE, OTHER))?
                    }
                    NativeBinaryOp::BitAnd => {
                        self.emit(|encoder| encoder.and_reg_reg(VALUE, OTHER))?
                    }
                    NativeBinaryOp::BitOr => {
                        self.emit(|encoder| encoder.or_reg_reg(VALUE, OTHER))?
                    }
                    NativeBinaryOp::BitXor => {
                        self.emit(|encoder| encoder.xor_reg_reg(VALUE, OTHER))?
                    }
                    NativeBinaryOp::ShiftLeft
                    | NativeBinaryOp::SignedShiftRight
                    | NativeBinaryOp::UnsignedShiftRight => {
                        self.emit(|encoder| encoder.mov_reg_reg(COUNT, OTHER))?;
                        let kind = match op {
                            NativeBinaryOp::ShiftLeft => ShiftKind::LeftLogical,
                            NativeBinaryOp::SignedShiftRight => ShiftKind::RightArithmetic,
                            NativeBinaryOp::UnsignedShiftRight => ShiftKind::RightLogical,
                            _ => unreachable!(),
                        };
                        self.emit(|encoder| encoder.shift_reg_by_cl(VALUE, kind))?;
                    }
                    NativeBinaryOp::SignedDiv
                    | NativeBinaryOp::UnsignedDiv
                    | NativeBinaryOp::SignedRem
                    | NativeBinaryOp::UnsignedRem => {
                        let signed =
                            matches!(op, NativeBinaryOp::SignedDiv | NativeBinaryOp::SignedRem);
                        // DIV/IDIV use EDX:EAX as the dividend, so EDX cannot
                        // simultaneously hold the divisor. ECX is a selector
                        // scratch register and is not in the DBYTE32 allocation set.
                        self.emit(|encoder| encoder.mov_reg_reg(COUNT, OTHER))?;
                        if signed {
                            self.emit(Encoder::cdq)?;
                            self.emit(|encoder| encoder.idiv_reg(COUNT))?;
                        } else {
                            self.emit(|encoder| encoder.xor_reg_reg(Register::Edx, Register::Edx))?;
                            self.emit(|encoder| encoder.div_reg(COUNT))?;
                        }
                        if matches!(op, NativeBinaryOp::SignedRem | NativeBinaryOp::UnsignedRem) {
                            self.emit(|encoder| encoder.mov_reg_reg(VALUE, Register::Edx))?;
                        }
                    }
                }
                self.normalize(VALUE, *ty)?;
                self.store_register(*destination, VALUE)
            }
            NativeMirInst::Unary {
                destination,
                op,
                value,
                ty,
            } => {
                self.load_value(*value, VALUE)?;
                match op {
                    NativeUnaryOp::Negate => self.emit(|encoder| encoder.neg_reg(VALUE))?,
                    NativeUnaryOp::BitNot => self.emit(|encoder| encoder.not_reg(VALUE))?,
                    NativeUnaryOp::BoolNot => {
                        self.emit(|encoder| encoder.test_reg_reg(VALUE, VALUE))?;
                        self.emit(|encoder| encoder.setcc_reg8(Condition::Equal, VALUE))?;
                        self.emit(|encoder| encoder.movzx_reg_reg8(VALUE, VALUE))?;
                    }
                }
                self.normalize(VALUE, *ty)?;
                self.store_register(*destination, VALUE)
            }
            NativeMirInst::Cast {
                destination,
                value,
                from,
                to,
                signed,
            } => match (*from == MachineType::I64, *to == MachineType::I64) {
                (true, true) => self.copy_wide(*destination, *value),
                (true, false) => {
                    self.load_wide(*value, Register::Eax, Register::Edx)?;
                    self.extend_or_truncate(Register::Eax, *to, *signed)?;
                    self.store_register(*destination, Register::Eax)
                }
                (false, true) => {
                    self.load_register(*value, Register::Eax)?;
                    self.extend_or_truncate(Register::Eax, *from, *signed)?;
                    if *signed {
                        self.emit(Encoder::cdq)?;
                    } else {
                        self.emit(|encoder| encoder.xor_reg_reg(Register::Edx, Register::Edx))?;
                    }
                    self.store_wide_pair(*destination, Register::Eax, Register::Edx)
                }
                (false, false) => {
                    self.load_register(*value, VALUE)?;
                    self.extend_or_truncate(VALUE, *to, *signed)?;
                    self.store_register(*destination, VALUE)
                }
            },
            NativeMirInst::Compare {
                destination,
                op,
                left,
                right,
                ty,
            } => {
                if *ty == MachineType::I64 {
                    self.select_wide_compare(*destination, *op, *left, *right)
                } else {
                    self.load_value(*left, VALUE)?;
                    self.load_value(*right, OTHER)?;
                    self.emit(|encoder| encoder.cmp_reg_reg(VALUE, OTHER))?;
                    self.emit(|encoder| encoder.setcc_reg8(compare_condition(*op), VALUE))?;
                    self.emit(|encoder| encoder.movzx_reg_reg8(VALUE, VALUE))?;
                    self.store_register(*destination, VALUE)
                }
            }
            NativeMirInst::StackAddress {
                destination,
                object,
            } => {
                let displacement = self.stack_object_displacement(*object)?;
                self.emit(|encoder| {
                    encoder
                        .lea_reg_from_memory(VALUE, MemoryOperand::new(Register::Ebp, displacement))
                })?;
                self.store_register(*destination, VALUE)
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
                let offset = self
                    .encoder
                    .mov_reg_imm32_placeholder(VALUE)
                    .map_err(encode_error)?;
                self.statistics.selected_instructions += 1;
                self.relocations.push(PendingRelocation {
                    offset: checked_u32(offset, "absolute relocation")?,
                    symbol: symbol.clone(),
                    kind: RelocationKind::Abs32,
                    addend: 0,
                });
                self.store_register(*destination, VALUE)
            }
            NativeMirInst::InterruptDefinition { .. } => Ok(()),
            NativeMirInst::AddressOffset {
                destination,
                base,
                index,
                scale,
                displacement,
            } => {
                self.load_register(*base, VALUE)?;
                let index = if let Some(index) = index {
                    self.load_register(*index, OTHER)?;
                    Some((OTHER, scale_value(*scale)?))
                } else {
                    None
                };
                self.emit(|encoder| {
                    encoder.lea_reg_from_memory(
                        VALUE,
                        MemoryOperand {
                            base: VALUE,
                            index,
                            displacement: *displacement,
                        },
                    )
                })?;
                self.store_register(*destination, VALUE)
            }
            NativeMirInst::Load {
                destination,
                address,
                ty,
                volatile: _,
            } => {
                if *ty == MachineType::I64 {
                    self.load_register(*address, COUNT)?;
                    self.emit(|encoder| {
                        encoder.mov_reg_from_memory(
                            Register::Eax,
                            MemoryOperand::new(COUNT, 0),
                            OperandWidth::Dword,
                            None,
                        )
                    })?;
                    self.emit(|encoder| {
                        encoder.mov_reg_from_memory(
                            Register::Edx,
                            MemoryOperand::new(COUNT, 4),
                            OperandWidth::Dword,
                            None,
                        )
                    })?;
                    self.statistics.loads += 2;
                    return self.store_wide_pair(*destination, Register::Eax, Register::Edx);
                }
                self.load_register(*address, OTHER)?;
                self.emit(|encoder| {
                    encoder.mov_reg_from_memory(
                        VALUE,
                        MemoryOperand::new(OTHER, 0),
                        operand_width(*ty),
                        load_extension(*ty),
                    )
                })?;
                self.statistics.loads += 1;
                self.store_register(*destination, VALUE)
            }
            NativeMirInst::Store {
                address,
                value,
                ty,
                volatile: _,
            } => {
                if *ty == MachineType::I64 {
                    self.load_register(*address, COUNT)?;
                    self.load_wide(*value, Register::Eax, Register::Edx)?;
                    self.emit(|encoder| {
                        encoder.mov_memory_from_reg(
                            MemoryOperand::new(COUNT, 0),
                            Register::Eax,
                            OperandWidth::Dword,
                        )
                    })?;
                    self.emit(|encoder| {
                        encoder.mov_memory_from_reg(
                            MemoryOperand::new(COUNT, 4),
                            Register::Edx,
                            OperandWidth::Dword,
                        )
                    })?;
                    self.statistics.stores += 2;
                    return Ok(());
                }
                self.load_register(*address, OTHER)?;
                self.load_register(*value, VALUE)?;
                self.emit(|encoder| {
                    encoder.mov_memory_from_reg(
                        MemoryOperand::new(OTHER, 0),
                        VALUE,
                        operand_width(*ty),
                    )
                })?;
                self.statistics.stores += 1;
                Ok(())
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
        }
    }

    fn select_wide_binary(
        &mut self,
        destination: VirtualRegister,
        op: NativeBinaryOp,
        left: NativeValue,
        right: NativeValue,
    ) -> Result<(), NativeMirError> {
        if matches!(
            op,
            NativeBinaryOp::Add
                | NativeBinaryOp::Sub
                | NativeBinaryOp::BitAnd
                | NativeBinaryOp::BitOr
                | NativeBinaryOp::BitXor
        ) {
            self.load_wide_half_value(left, false, Register::Eax)?;
            self.load_wide_half_value(right, false, Register::Ecx)?;
            match op {
                NativeBinaryOp::Add => {
                    self.emit(|encoder| encoder.add_reg_reg(Register::Eax, Register::Ecx))?
                }
                NativeBinaryOp::Sub => {
                    self.emit(|encoder| encoder.sub_reg_reg(Register::Eax, Register::Ecx))?
                }
                NativeBinaryOp::BitAnd => {
                    self.emit(|encoder| encoder.and_reg_reg(Register::Eax, Register::Ecx))?
                }
                NativeBinaryOp::BitOr => {
                    self.emit(|encoder| encoder.or_reg_reg(Register::Eax, Register::Ecx))?
                }
                NativeBinaryOp::BitXor => {
                    self.emit(|encoder| encoder.xor_reg_reg(Register::Eax, Register::Ecx))?
                }
                _ => unreachable!(),
            }
            self.load_wide_half_value(left, true, Register::Edx)?;
            self.load_wide_half_value(right, true, Register::Ecx)?;
            match op {
                NativeBinaryOp::Add => {
                    self.emit(|encoder| encoder.adc_reg_reg(Register::Edx, Register::Ecx))?
                }
                NativeBinaryOp::Sub => {
                    self.emit(|encoder| encoder.sbb_reg_reg(Register::Edx, Register::Ecx))?
                }
                NativeBinaryOp::BitAnd => {
                    self.emit(|encoder| encoder.and_reg_reg(Register::Edx, Register::Ecx))?
                }
                NativeBinaryOp::BitOr => {
                    self.emit(|encoder| encoder.or_reg_reg(Register::Edx, Register::Ecx))?
                }
                NativeBinaryOp::BitXor => {
                    self.emit(|encoder| encoder.xor_reg_reg(Register::Edx, Register::Ecx))?
                }
                _ => unreachable!(),
            }
            return self.store_wide_pair(destination, Register::Eax, Register::Edx);
        }
        // Wide helpers borrow all preserved DBYTE32 scratch registers. Save them
        // because scalar virtual registers may be live in EBX/ESI/EDI.
        for register in [Register::Ebx, Register::Esi, Register::Edi] {
            self.emit(|encoder| encoder.push_reg(register))?;
        }
        self.load_wide_value(right, Register::Ecx, Register::Ebx)?;
        self.load_wide_value(left, Register::Eax, Register::Edx)?;
        match op {
            NativeBinaryOp::Add
            | NativeBinaryOp::Sub
            | NativeBinaryOp::BitAnd
            | NativeBinaryOp::BitOr
            | NativeBinaryOp::BitXor => unreachable!(),
            NativeBinaryOp::Mul => self.emit_wide_multiply()?,
            NativeBinaryOp::SignedDiv
            | NativeBinaryOp::UnsignedDiv
            | NativeBinaryOp::SignedRem
            | NativeBinaryOp::UnsignedRem => {
                self.emit_wide_division(op)?;
            }
            NativeBinaryOp::ShiftLeft
            | NativeBinaryOp::SignedShiftRight
            | NativeBinaryOp::UnsignedShiftRight => self.emit_wide_shift(op)?,
        }
        for register in [Register::Edi, Register::Esi, Register::Ebx] {
            self.emit(|encoder| encoder.pop_reg(register))?;
        }
        self.store_wide_pair(destination, Register::Eax, Register::Edx)
    }

    fn emit_wide_multiply(&mut self) -> Result<(), NativeMirError> {
        self.emit(|encoder| encoder.mov_reg_reg(Register::Esi, Register::Eax))?;
        self.emit(|encoder| encoder.mov_reg_reg(Register::Edi, Register::Edx))?;
        self.emit(|encoder| encoder.mul_reg(Register::Ecx))?;
        self.emit(|encoder| encoder.imul_reg_reg(Register::Edi, Register::Ecx))?;
        self.emit(|encoder| encoder.imul_reg_reg(Register::Esi, Register::Ebx))?;
        self.emit(|encoder| encoder.add_reg_reg(Register::Edx, Register::Edi))?;
        self.emit(|encoder| encoder.add_reg_reg(Register::Edx, Register::Esi))
    }

    fn emit_wide_shift(&mut self, op: NativeBinaryOp) -> Result<(), NativeMirError> {
        self.emit(|encoder| encoder.and_reg_imm32(Register::Ecx, 63))?;
        let large = self.encoder.new_label().map_err(encode_error)?;
        let done = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.cmp_reg_imm32(Register::Ecx, 32))?;
        self.emit(|encoder| encoder.jump_if(Condition::AboveOrEqual, large))?;
        match op {
            NativeBinaryOp::ShiftLeft => {
                self.emit(|encoder| encoder.shld_reg_reg_cl(Register::Edx, Register::Eax))?;
                self.emit(|encoder| {
                    encoder.shift_reg_by_cl(Register::Eax, ShiftKind::LeftLogical)
                })?;
            }
            NativeBinaryOp::SignedShiftRight | NativeBinaryOp::UnsignedShiftRight => {
                self.emit(|encoder| encoder.shrd_reg_reg_cl(Register::Eax, Register::Edx))?;
                let kind = if op == NativeBinaryOp::SignedShiftRight {
                    ShiftKind::RightArithmetic
                } else {
                    ShiftKind::RightLogical
                };
                self.emit(|encoder| encoder.shift_reg_by_cl(Register::Edx, kind))?;
            }
            _ => unreachable!(),
        }
        self.emit(|encoder| encoder.jump(done))?;
        self.encoder.bind_label(large).map_err(encode_error)?;
        self.emit(|encoder| encoder.sub_reg_imm32(Register::Ecx, 32))?;
        match op {
            NativeBinaryOp::ShiftLeft => {
                self.emit(|encoder| encoder.mov_reg_reg(Register::Edx, Register::Eax))?;
                self.emit(|encoder| encoder.xor_reg_reg(Register::Eax, Register::Eax))?;
                self.emit(|encoder| {
                    encoder.shift_reg_by_cl(Register::Edx, ShiftKind::LeftLogical)
                })?;
            }
            NativeBinaryOp::SignedShiftRight => {
                self.emit(|encoder| encoder.mov_reg_reg(Register::Eax, Register::Edx))?;
                self.emit(|encoder| {
                    encoder.shift_reg_imm8(Register::Edx, ShiftKind::RightArithmetic, 31)
                })?;
                self.emit(|encoder| {
                    encoder.shift_reg_by_cl(Register::Eax, ShiftKind::RightArithmetic)
                })?;
            }
            NativeBinaryOp::UnsignedShiftRight => {
                self.emit(|encoder| encoder.mov_reg_reg(Register::Eax, Register::Edx))?;
                self.emit(|encoder| encoder.xor_reg_reg(Register::Edx, Register::Edx))?;
                self.emit(|encoder| {
                    encoder.shift_reg_by_cl(Register::Eax, ShiftKind::RightLogical)
                })?;
            }
            _ => unreachable!(),
        }
        self.encoder.bind_label(done).map_err(encode_error)?;
        Ok(())
    }

    fn emit_wide_division(&mut self, op: NativeBinaryOp) -> Result<(), NativeMirError> {
        let divisor_valid = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.mov_reg_reg(Register::Esi, Register::Ecx))?;
        self.emit(|encoder| encoder.or_reg_reg(Register::Esi, Register::Ebx))?;
        self.emit(|encoder| encoder.test_reg_reg(Register::Esi, Register::Esi))?;
        self.emit(|encoder| encoder.jump_if(Condition::NotEqual, divisor_valid))?;
        // Preserve the DBYTE32 architectural contract: divide by zero faults as #DE.
        self.emit(|encoder| encoder.xor_reg_reg(Register::Edx, Register::Edx))?;
        self.emit(|encoder| encoder.xor_reg_reg(Register::Eax, Register::Eax))?;
        self.emit(|encoder| encoder.div_reg(Register::Esi))?;
        self.encoder
            .bind_label(divisor_valid)
            .map_err(encode_error)?;

        let signed = matches!(op, NativeBinaryOp::SignedDiv | NativeBinaryOp::SignedRem);
        if signed {
            self.emit_signed_wide_division_prelude()?;
        }
        self.emit_unsigned_wide_division()?;
        if signed {
            self.emit_signed_wide_division_finish()?;
        }
        if matches!(op, NativeBinaryOp::SignedRem | NativeBinaryOp::UnsignedRem) {
            self.emit(|encoder| encoder.mov_reg_reg(Register::Eax, Register::Esi))?;
            self.emit(|encoder| encoder.mov_reg_reg(Register::Edx, Register::Edi))?;
        }
        Ok(())
    }

    fn emit_signed_wide_division_prelude(&mut self) -> Result<(), NativeMirError> {
        let not_overflow = self.encoder.new_label().map_err(encode_error)?;
        let check_divisor = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.cmp_reg_imm32(Register::Edx, i32::MIN))?;
        self.emit(|encoder| encoder.jump_if(Condition::Equal, check_divisor))?;
        self.emit(|encoder| encoder.jump(not_overflow))?;
        self.encoder
            .bind_label(check_divisor)
            .map_err(encode_error)?;
        self.emit(|encoder| encoder.test_reg_reg(Register::Eax, Register::Eax))?;
        self.emit(|encoder| encoder.jump_if(Condition::NotEqual, not_overflow))?;
        self.emit(|encoder| encoder.cmp_reg_imm32(Register::Ebx, -1))?;
        self.emit(|encoder| encoder.jump_if(Condition::NotEqual, not_overflow))?;
        self.emit(|encoder| encoder.cmp_reg_imm32(Register::Ecx, -1))?;
        self.emit(|encoder| encoder.jump_if(Condition::NotEqual, not_overflow))?;
        // i64::MIN / -1 must raise architectural signed-overflow #DE.
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Eax, 0x8000_0000))?;
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Edx, u32::MAX))?;
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Ecx, u32::MAX))?;
        self.emit(|encoder| encoder.idiv_reg(Register::Ecx))?;
        self.encoder
            .bind_label(not_overflow)
            .map_err(encode_error)?;

        // Save remainder sign, then quotient sign, for the balanced finish path.
        self.emit(|encoder| encoder.push_reg(Register::Edx))?;
        self.emit(|encoder| encoder.mov_reg_reg(Register::Esi, Register::Edx))?;
        self.emit(|encoder| encoder.xor_reg_reg(Register::Esi, Register::Ebx))?;
        self.emit(|encoder| encoder.push_reg(Register::Esi))?;
        self.emit_abs_wide(Register::Eax, Register::Edx)?;
        self.emit_abs_wide(Register::Ecx, Register::Ebx)
    }

    fn emit_abs_wide(&mut self, low: Register, high: Register) -> Result<(), NativeMirError> {
        let done = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.test_reg_reg(high, high))?;
        self.emit(|encoder| encoder.jump_if(Condition::GreaterOrEqual, done))?;
        self.emit(|encoder| encoder.not_reg(low))?;
        self.emit(|encoder| encoder.not_reg(high))?;
        self.emit(|encoder| encoder.add_reg_imm32(low, 1))?;
        self.emit(|encoder| encoder.xor_reg_reg(Register::Esi, Register::Esi))?;
        self.emit(|encoder| encoder.adc_reg_reg(high, Register::Esi))?;
        self.encoder.bind_label(done).map_err(encode_error)
    }

    fn emit_unsigned_wide_division(&mut self) -> Result<(), NativeMirError> {
        let loop_head = self.encoder.new_label().map_err(encode_error)?;
        let subtract = self.encoder.new_label().map_err(encode_error)?;
        let skip_subtract = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.push_reg(Register::Ebp))?;
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Ebp, 64))?;
        self.emit(|encoder| encoder.xor_reg_reg(Register::Esi, Register::Esi))?;
        self.emit(|encoder| encoder.xor_reg_reg(Register::Edi, Register::Edi))?;
        self.encoder.bind_label(loop_head).map_err(encode_error)?;
        self.emit(|encoder| encoder.shift_reg_imm8(Register::Eax, ShiftKind::LeftLogical, 1))?;
        self.emit(|encoder| encoder.rcl_reg_one(Register::Edx))?;
        self.emit(|encoder| encoder.rcl_reg_one(Register::Esi))?;
        self.emit(|encoder| encoder.rcl_reg_one(Register::Edi))?;
        self.emit(|encoder| encoder.cmp_reg_reg(Register::Edi, Register::Ebx))?;
        self.emit(|encoder| encoder.jump_if(Condition::Above, subtract))?;
        self.emit(|encoder| encoder.jump_if(Condition::Below, skip_subtract))?;
        self.emit(|encoder| encoder.cmp_reg_reg(Register::Esi, Register::Ecx))?;
        self.emit(|encoder| encoder.jump_if(Condition::Below, skip_subtract))?;
        self.encoder.bind_label(subtract).map_err(encode_error)?;
        self.emit(|encoder| encoder.sub_reg_reg(Register::Esi, Register::Ecx))?;
        self.emit(|encoder| encoder.sbb_reg_reg(Register::Edi, Register::Ebx))?;
        self.emit(|encoder| encoder.add_reg_imm32(Register::Eax, 1))?;
        self.encoder
            .bind_label(skip_subtract)
            .map_err(encode_error)?;
        self.emit(|encoder| encoder.dec_reg(Register::Ebp))?;
        self.emit(|encoder| encoder.jump_if(Condition::NotEqual, loop_head))?;
        self.emit(|encoder| encoder.pop_reg(Register::Ebp))
    }

    fn emit_signed_wide_division_finish(&mut self) -> Result<(), NativeMirError> {
        // Quotient sign is on top, followed by dividend/remainder sign.
        self.emit(|encoder| encoder.pop_reg(Register::Ecx))?;
        let quotient_done = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.test_reg_reg(Register::Ecx, Register::Ecx))?;
        self.emit(|encoder| encoder.jump_if(Condition::GreaterOrEqual, quotient_done))?;
        self.emit_negate_wide(Register::Eax, Register::Edx)?;
        self.encoder
            .bind_label(quotient_done)
            .map_err(encode_error)?;

        self.emit(|encoder| encoder.pop_reg(Register::Ecx))?;
        let remainder_done = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.test_reg_reg(Register::Ecx, Register::Ecx))?;
        self.emit(|encoder| encoder.jump_if(Condition::GreaterOrEqual, remainder_done))?;
        self.emit_negate_wide(Register::Esi, Register::Edi)?;
        self.encoder
            .bind_label(remainder_done)
            .map_err(encode_error)
    }

    fn emit_negate_wide(&mut self, low: Register, high: Register) -> Result<(), NativeMirError> {
        self.emit(|encoder| encoder.not_reg(low))?;
        self.emit(|encoder| encoder.not_reg(high))?;
        self.emit(|encoder| encoder.add_reg_imm32(low, 1))?;
        self.emit(|encoder| encoder.xor_reg_reg(Register::Ecx, Register::Ecx))?;
        self.emit(|encoder| encoder.adc_reg_reg(high, Register::Ecx))
    }

    fn select_wide_compare(
        &mut self,
        destination: VirtualRegister,
        op: NativeCompareOp,
        left: NativeValue,
        right: NativeValue,
    ) -> Result<(), NativeMirError> {
        // Compare one word at a time so DBYTE32's allocatable EBX/ESI/EDI set
        // is never borrowed by the wide helper. This keeps live scalar values
        // intact without per-operation save/restore traffic.
        self.load_wide_half_value(left, true, Register::Edx)?;
        self.load_wide_half_value(right, true, Register::Ecx)?;
        let is_true = self.encoder.new_label().map_err(encode_error)?;
        let is_false = self.encoder.new_label().map_err(encode_error)?;
        let done = self.encoder.new_label().map_err(encode_error)?;
        self.emit(|encoder| encoder.cmp_reg_reg(Register::Edx, Register::Ecx))?;
        match op {
            NativeCompareOp::Equal | NativeCompareOp::NotEqual => {
                let mismatch = if op == NativeCompareOp::Equal {
                    is_false
                } else {
                    is_true
                };
                let matched = if op == NativeCompareOp::Equal {
                    is_true
                } else {
                    is_false
                };
                self.emit(|encoder| encoder.jump_if(Condition::NotEqual, mismatch))?;
                self.load_wide_half_value(left, false, Register::Eax)?;
                self.load_wide_half_value(right, false, Register::Ecx)?;
                self.emit(|encoder| encoder.cmp_reg_reg(Register::Eax, Register::Ecx))?;
                self.emit(|encoder| encoder.jump_if(Condition::Equal, matched))?;
                self.emit(|encoder| encoder.jump(mismatch))?;
            }
            _ => {
                let (high_true, high_false, low_true) = match op {
                    NativeCompareOp::SignedLess => {
                        (Condition::Less, Condition::Greater, Condition::Below)
                    }
                    NativeCompareOp::SignedLessEqual => {
                        (Condition::Less, Condition::Greater, Condition::BelowOrEqual)
                    }
                    NativeCompareOp::SignedGreater => {
                        (Condition::Greater, Condition::Less, Condition::Above)
                    }
                    NativeCompareOp::SignedGreaterEqual => {
                        (Condition::Greater, Condition::Less, Condition::AboveOrEqual)
                    }
                    NativeCompareOp::UnsignedLess => {
                        (Condition::Below, Condition::Above, Condition::Below)
                    }
                    NativeCompareOp::UnsignedLessEqual => {
                        (Condition::Below, Condition::Above, Condition::BelowOrEqual)
                    }
                    NativeCompareOp::UnsignedGreater => {
                        (Condition::Above, Condition::Below, Condition::Above)
                    }
                    NativeCompareOp::UnsignedGreaterEqual => {
                        (Condition::Above, Condition::Below, Condition::AboveOrEqual)
                    }
                    NativeCompareOp::Equal | NativeCompareOp::NotEqual => unreachable!(),
                };
                self.emit(|encoder| encoder.jump_if(high_true, is_true))?;
                self.emit(|encoder| encoder.jump_if(high_false, is_false))?;
                self.load_wide_half_value(left, false, Register::Eax)?;
                self.load_wide_half_value(right, false, Register::Ecx)?;
                self.emit(|encoder| encoder.cmp_reg_reg(Register::Eax, Register::Ecx))?;
                self.emit(|encoder| encoder.jump_if(low_true, is_true))?;
                self.emit(|encoder| encoder.jump(is_false))?;
            }
        }
        self.encoder.bind_label(is_false).map_err(encode_error)?;
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Eax, 0))?;
        self.emit(|encoder| encoder.jump(done))?;
        self.encoder.bind_label(is_true).map_err(encode_error)?;
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Eax, 1))?;
        self.encoder.bind_label(done).map_err(encode_error)?;
        self.store_register(destination, Register::Eax)
    }

    fn constant_u16(&self, register: VirtualRegister) -> Result<u16, NativeMirError> {
        let mut value = None;
        for block in &self.function.blocks {
            for instruction in &block.instructions {
                if let NativeMirInst::Const {
                    destination,
                    value: candidate,
                    ..
                } = &instruction.instruction
                {
                    if *destination == register {
                        let candidate = u16::try_from(*candidate)
                            .map_err(|_| codegen_error("i686 intrinsic constant exceeds u16"))?;
                        if value.replace(candidate).is_some() {
                            return Err(codegen_error(
                                "virtual register has multiple constant definitions",
                            ));
                        }
                    }
                }
            }
        }
        value.ok_or_else(|| unsupported("i686 intrinsic requires a compile-time u16 argument"))
    }

    fn select_intrinsic(
        &mut self,
        destination: Option<VirtualRegister>,
        intrinsic: dbyte_hir::HirIntrinsic,
        arguments: &[VirtualRegister],
    ) -> Result<(), NativeMirError> {
        use dbyte_hir::HirIntrinsic;
        let argument = |index: usize| {
            arguments
                .get(index)
                .copied()
                .ok_or_else(|| codegen_error("resolved intrinsic argument is missing"))
        };
        match intrinsic {
            HirIntrinsic::IoOut8 | HirIntrinsic::IoOut16 | HirIntrinsic::IoOut32 => {
                self.load_register(argument(0)?, Register::Edx)?;
                self.load_register(argument(1)?, Register::Eax)?;
                match intrinsic {
                    HirIntrinsic::IoOut8 => self.emit(Encoder::out_dx_al)?,
                    HirIntrinsic::IoOut16 => self.emit(Encoder::out_dx_ax)?,
                    HirIntrinsic::IoOut32 => self.emit(Encoder::out_dx_eax)?,
                    _ => unreachable!(),
                }
            }
            HirIntrinsic::IoIn8 | HirIntrinsic::IoIn16 | HirIntrinsic::IoIn32 => {
                self.load_register(argument(0)?, Register::Edx)?;
                match intrinsic {
                    HirIntrinsic::IoIn8 => {
                        self.emit(Encoder::in_al_dx)?;
                        self.emit(|encoder| {
                            encoder.movzx_reg_reg(Register::Eax, Register::Eax, OperandWidth::Byte)
                        })?;
                    }
                    HirIntrinsic::IoIn16 => {
                        self.emit(Encoder::in_ax_dx)?;
                        self.emit(|encoder| {
                            encoder.movzx_reg_reg(Register::Eax, Register::Eax, OperandWidth::Word)
                        })?;
                    }
                    HirIntrinsic::IoIn32 => self.emit(Encoder::in_eax_dx)?,
                    _ => unreachable!(),
                }
            }
            HirIntrinsic::VolatileLoad8
            | HirIntrinsic::VolatileLoad16
            | HirIntrinsic::VolatileLoad32 => {
                self.load_register(argument(0)?, Register::Edx)?;
                let width = match intrinsic {
                    HirIntrinsic::VolatileLoad8 => OperandWidth::Byte,
                    HirIntrinsic::VolatileLoad16 => OperandWidth::Word,
                    _ => OperandWidth::Dword,
                };
                let extension = (width != OperandWidth::Dword).then_some(Extension::Zero);
                self.emit(|encoder| {
                    encoder.mov_reg_from_memory(
                        Register::Eax,
                        MemoryOperand::new(Register::Edx, 0),
                        width,
                        extension,
                    )
                })?;
                self.statistics.loads += 1;
            }
            HirIntrinsic::VolatileStore8
            | HirIntrinsic::VolatileStore16
            | HirIntrinsic::VolatileStore32 => {
                self.load_register(argument(0)?, Register::Edx)?;
                self.load_register(argument(1)?, Register::Eax)?;
                let width = match intrinsic {
                    HirIntrinsic::VolatileStore8 => OperandWidth::Byte,
                    HirIntrinsic::VolatileStore16 => OperandWidth::Word,
                    _ => OperandWidth::Dword,
                };
                self.emit(|encoder| {
                    encoder.mov_memory_from_reg(
                        MemoryOperand::new(Register::Edx, 0),
                        Register::Eax,
                        width,
                    )
                })?;
                self.statistics.stores += 1;
            }
            HirIntrinsic::CpuHlt => self.emit(Encoder::hlt)?,
            HirIntrinsic::CpuCli => self.emit(Encoder::cli)?,
            HirIntrinsic::CpuSti => self.emit(Encoder::sti)?,
            HirIntrinsic::CpuInt3 => self.emit(Encoder::int3)?,
            HirIntrinsic::CpuReadEflags => {
                self.emit(Encoder::pushfd)?;
                self.emit(|encoder| encoder.pop_reg(Register::Eax))?;
            }
            HirIntrinsic::CpuReadCr0 => self.emit(Encoder::mov_eax_cr0)?,
            HirIntrinsic::CpuWriteCr0 => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.emit(Encoder::mov_cr0_eax)?;
            }
            HirIntrinsic::CpuReadCr2 => self.emit(Encoder::mov_eax_cr2)?,
            HirIntrinsic::CpuReadCr3 => self.emit(Encoder::mov_eax_cr3)?,
            HirIntrinsic::CpuWriteCr3 => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.emit(Encoder::mov_cr3_eax)?;
            }
            HirIntrinsic::CpuInvlpg => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.emit(|encoder| encoder.invlpg(MemoryOperand::new(Register::Eax, 0)))?;
            }
            HirIntrinsic::CpuLgdt => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.emit(|encoder| encoder.lgdt(MemoryOperand::new(Register::Eax, 0)))?;
            }
            HirIntrinsic::CpuLidt => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.emit(|encoder| encoder.lidt(MemoryOperand::new(Register::Eax, 0)))?;
            }
            HirIntrinsic::CpuLtr => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.emit(Encoder::ltr_ax)?;
            }
            HirIntrinsic::TaskBlockTrap => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.load_register(argument(1)?, Register::Edx)?;
                self.emit(|encoder| encoder.software_interrupt(0x81))?;
            }
            HirIntrinsic::UserSyscall0
            | HirIntrinsic::UserSyscall1
            | HirIntrinsic::UserSyscall2
            | HirIntrinsic::UserSyscall3 => {
                let expected = match intrinsic {
                    HirIntrinsic::UserSyscall0 => 1,
                    HirIntrinsic::UserSyscall1 => 2,
                    HirIntrinsic::UserSyscall2 => 3,
                    HirIntrinsic::UserSyscall3 => 4,
                    _ => unreachable!(),
                };
                if arguments.len() != expected {
                    return Err(codegen_error("resolved user syscall arity mismatch"));
                }
                for source in arguments {
                    self.load_register(*source, Register::Eax)?;
                    self.emit(|encoder| encoder.push_reg(Register::Eax))?;
                }
                let abi = [Register::Eax, Register::Ebx, Register::Ecx, Register::Edx];
                for register in abi[..expected].iter().rev().copied() {
                    self.emit(|encoder| encoder.pop_reg(register))?;
                }
                self.emit(|encoder| encoder.software_interrupt(0x80))?;
            }
            HirIntrinsic::InterruptResume => {
                self.load_register(argument(0)?, Register::Eax)?;
                self.emit(|encoder| encoder.mov_reg_reg(Register::Esp, Register::Eax))?;
                self.emit(Encoder::popad)?;
                self.emit(Encoder::iretd)?;
            }
            HirIntrinsic::CpuReloadSegments => {
                let code_selector = self.constant_u16(argument(0)?)?;
                let data_selector = self.constant_u16(argument(1)?)?;
                self.emit(|encoder| encoder.mov_ax_imm16(data_selector))?;
                self.emit(Encoder::load_data_segments_from_ax)?;
                let immediate = self
                    .encoder
                    .far_jump_imm32_placeholder(code_selector)
                    .map_err(encode_error)?;
                self.statistics.selected_instructions += 1;
                let destination = checked_u32(self.encoder.position(), "far-jump destination")?;
                self.relocations.push(PendingRelocation {
                    offset: checked_u32(immediate, "far-jump relocation offset")?,
                    symbol: self.function.symbol.clone(),
                    kind: RelocationKind::Abs32,
                    addend: i32::try_from(destination)
                        .map_err(|_| codegen_error("far-jump destination exceeds i32"))?,
                });
            }
            _ => return Err(unsupported("resolved i686 intrinsic capability")),
        }
        if let Some(destination) = destination {
            self.store_register(destination, Register::Eax)?;
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
            return Err(unsupported("aggregate by-value C call on i686"));
        }
        let order: Box<dyn Iterator<Item = usize>> = if convention == dbyte_ast::CallConvention::C {
            Box::new((0..arguments.len()).rev())
        } else {
            Box::new(0..arguments.len())
        };
        let mut bytes = 0u32;
        if aggregate_result.is_some() {
            let destination =
                destination.ok_or_else(|| codegen_error("aggregate call has no result storage"))?;
            self.load_register(destination, Register::Eax)?;
            self.emit(|encoder| encoder.push_reg(Register::Eax))?;
            bytes = 4;
        }
        if let NativeCallee::Closure { environment, .. } = callee {
            if convention != dbyte_ast::CallConvention::DByte {
                return Err(unsupported("i686 C closure call ABI"));
            }
            self.load_register(*environment, VALUE)?;
            self.emit(|encoder| encoder.push_reg(VALUE))?;
            bytes = bytes
                .checked_add(4)
                .ok_or_else(|| codegen_error("call area exceeds u32"))?;
        }
        for index in order {
            let argument = arguments[index];
            let ty = self.function.register_types[argument.0 as usize];
            if let Some(layout) = argument_aggregates[index] {
                let area = align_up(layout.bytes, 4)?;
                self.load_register(argument, Register::Eax)?;
                self.emit(|encoder| encoder.sub_esp_imm32(area))?;
                let mut copied = 0u32;
                while copied < layout.bytes {
                    let remaining = layout.bytes - copied;
                    let (chunk, width) = if remaining >= 4 {
                        (4, OperandWidth::Dword)
                    } else if remaining >= 2 {
                        (2, OperandWidth::Word)
                    } else {
                        (1, OperandWidth::Byte)
                    };
                    let displacement = i32::try_from(copied)
                        .map_err(|_| codegen_error("aggregate argument offset exceeds i32"))?;
                    let extension = (width != OperandWidth::Dword).then_some(Extension::Zero);
                    self.emit(|encoder| {
                        encoder.mov_reg_from_memory(
                            Register::Ecx,
                            MemoryOperand::new(Register::Eax, displacement),
                            width,
                            extension,
                        )
                    })?;
                    self.emit(|encoder| {
                        encoder.mov_memory_from_reg(
                            MemoryOperand::new(Register::Esp, displacement),
                            Register::Ecx,
                            width,
                        )
                    })?;
                    self.statistics.loads += 1;
                    self.statistics.stores += 1;
                    copied += chunk;
                }
                if area > layout.bytes {
                    self.emit(|encoder| encoder.xor_reg_reg(Register::Ecx, Register::Ecx))?;
                    for padding in layout.bytes..area {
                        let displacement = i32::try_from(padding)
                            .map_err(|_| codegen_error("aggregate padding offset exceeds i32"))?;
                        self.emit(|encoder| {
                            encoder.mov_memory_from_reg(
                                MemoryOperand::new(Register::Esp, displacement),
                                Register::Ecx,
                                OperandWidth::Byte,
                            )
                        })?;
                        self.statistics.stores += 1;
                    }
                }
                bytes = bytes
                    .checked_add(area)
                    .ok_or_else(|| codegen_error("call area exceeds u32"))?;
            } else if ty == MachineType::I64 {
                self.load_wide(argument, Register::Eax, Register::Edx)?;
                // Stack memory remains little-endian low/high. Because PUSH grows
                // downward, emit the high word first and the low word last.
                self.emit(|encoder| encoder.push_reg(Register::Edx))?;
                self.emit(|encoder| encoder.push_reg(Register::Eax))?;
                bytes = bytes
                    .checked_add(8)
                    .ok_or_else(|| codegen_error("call area exceeds u32"))?;
            } else {
                self.load_register(argument, VALUE)?;
                self.emit(|encoder| encoder.push_reg(VALUE))?;
                bytes = bytes
                    .checked_add(4)
                    .ok_or_else(|| codegen_error("call area exceeds u32"))?;
            }
        }
        match callee {
            NativeCallee::Direct { symbol } => {
                let offset = self
                    .encoder
                    .call_rel32_placeholder()
                    .map_err(encode_error)?;
                self.statistics.selected_instructions += 1;
                self.relocations.push(PendingRelocation {
                    offset: checked_u32(offset, "call relocation")?,
                    symbol: symbol.clone(),
                    kind: RelocationKind::Pc32,
                    addend: -4,
                });
            }
            NativeCallee::Indirect { address } => {
                self.load_register(*address, OTHER)?;
                self.emit(|encoder| encoder.call_reg(OTHER))?;
            }
            NativeCallee::Closure { code, .. } => {
                self.load_register(*code, OTHER)?;
                self.emit(|encoder| encoder.call_reg(OTHER))?;
            }
        }
        self.statistics.calls += 1;
        if bytes != 0 {
            self.emit(|encoder| encoder.add_esp_imm32(bytes))?;
        }
        if let Some(destination) = destination.filter(|_| aggregate_result.is_none()) {
            if self.function.register_types[destination.0 as usize] == MachineType::I64 {
                self.store_wide_pair(destination, Register::Eax, Register::Edx)?;
            } else {
                self.store_register(destination, VALUE)?;
            }
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
        self.emit(|encoder| encoder.push_reg(Register::Esi))?;
        self.emit(|encoder| encoder.push_reg(Register::Edi))?;
        self.load_register(destination, Register::Eax)?;
        self.load_register(source, Register::Esi)?;
        self.emit(|encoder| encoder.mov_reg_reg(Register::Edi, Register::Eax))?;
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Ecx, bytes))?;
        self.emit(Encoder::rep_movsb)?;
        self.emit(|encoder| encoder.pop_reg(Register::Edi))?;
        self.emit(|encoder| encoder.pop_reg(Register::Esi))
    }

    fn select_aggregate_zero(
        &mut self,
        destination: VirtualRegister,
        bytes: u32,
    ) -> Result<(), NativeMirError> {
        if bytes > 4096 {
            return Err(unsupported("aggregate zero larger than 4096 bytes"));
        }
        self.emit(|encoder| encoder.push_reg(Register::Edi))?;
        self.load_register(destination, Register::Edi)?;
        self.emit(|encoder| encoder.xor_reg_reg(Register::Eax, Register::Eax))?;
        self.emit(|encoder| encoder.mov_reg_imm32(Register::Ecx, bytes))?;
        self.emit(Encoder::rep_stosb)?;
        self.emit(|encoder| encoder.pop_reg(Register::Edi))
    }

    fn select_terminator(&mut self, terminator: &NativeTerminator) -> Result<(), NativeMirError> {
        match terminator {
            NativeTerminator::Jump(target) => {
                let target = self.labels[target];
                self.emit(|encoder| encoder.jump(target))?;
                self.statistics.branches += 1;
            }
            NativeTerminator::Branch {
                condition,
                then_block,
                else_block,
            } => {
                self.load_value(*condition, VALUE)?;
                self.emit(|encoder| encoder.test_reg_reg(VALUE, VALUE))?;
                let then_block = self.labels[then_block];
                let else_block = self.labels[else_block];
                self.emit(|encoder| encoder.jump_if(Condition::NotEqual, then_block))?;
                self.emit(|encoder| encoder.jump(else_block))?;
                self.statistics.branches += 2;
            }
            NativeTerminator::Return(value) => {
                if let Some(value) = value {
                    let ty = self.function.register_types[value.0 as usize];
                    if ty == MachineType::I64 {
                        self.load_wide(*value, Register::Eax, Register::Edx)?;
                    } else {
                        self.load_register(*value, Register::Eax)?;
                    }
                }
                let epilogue = self.epilogue;
                self.emit(|encoder| encoder.jump(epilogue))?;
                self.statistics.branches += 1;
            }
            NativeTerminator::ReturnAggregate { source, layout } => {
                if self.function.aggregate_result_register == Some(*source) {
                    self.load_register(*source, Register::Eax)?;
                } else {
                    let displacement = self.sret_displacement.ok_or_else(|| {
                        codegen_error("aggregate return has no saved sret pointer")
                    })?;
                    // ESI/EDI are callee-saved in DBYTE32. Preserve them locally even
                    // when the register allocator did not otherwise assign them.
                    self.emit(|encoder| encoder.push_reg(Register::Esi))?;
                    self.emit(|encoder| encoder.push_reg(Register::Edi))?;
                    self.load_register(*source, Register::Esi)?;
                    self.emit(|encoder| {
                        encoder.mov_reg_from_ebp_disp32(Register::Edi, displacement)
                    })?;
                    self.emit(|encoder| encoder.mov_reg_imm32(Register::Ecx, layout.bytes))?;
                    self.emit(Encoder::rep_movsb)?;
                    self.emit(|encoder| {
                        encoder.mov_reg_from_ebp_disp32(Register::Eax, displacement)
                    })?;
                    self.emit(|encoder| encoder.pop_reg(Register::Edi))?;
                    self.emit(|encoder| encoder.pop_reg(Register::Esi))?;
                }
                let epilogue = self.epilogue;
                self.emit(|encoder| encoder.jump(epilogue))?;
                self.statistics.branches += 1;
            }
            NativeTerminator::Trap | NativeTerminator::Unreachable => self.emit(Encoder::ud2)?,
        }
        Ok(())
    }

    fn load_value(&mut self, value: NativeValue, target: Register) -> Result<(), NativeMirError> {
        match value {
            NativeValue::Register(register) => self.load_register(register, target),
            NativeValue::Immediate(value) => {
                self.emit(|encoder| encoder.mov_reg_imm32(target, value as u32))
            }
        }
    }

    fn load_wide_value(
        &mut self,
        value: NativeValue,
        low: Register,
        high: Register,
    ) -> Result<(), NativeMirError> {
        match value {
            NativeValue::Register(register) => self.load_wide(register, low, high),
            NativeValue::Immediate(value) => {
                self.emit(|encoder| encoder.mov_reg_imm32(low, value as u32))?;
                self.emit(|encoder| encoder.mov_reg_imm32(high, (value >> 32) as u32))
            }
        }
    }

    fn load_wide_half_value(
        &mut self,
        value: NativeValue,
        high: bool,
        target: Register,
    ) -> Result<(), NativeMirError> {
        match value {
            NativeValue::Register(register) => match self.location(register)? {
                RegisterLocation::RegisterPair {
                    low,
                    high: pair_high,
                } => {
                    let source = machine_register(if high { pair_high } else { low });
                    if source != target {
                        self.emit(|encoder| encoder.mov_reg_reg(target, source))?;
                    }
                    Ok(())
                }
                RegisterLocation::Spill { slot } => {
                    let mut displacement = self.spill_displacement(slot)?;
                    if high {
                        displacement = displacement
                            .checked_add(4)
                            .ok_or_else(|| codegen_error("wide half displacement overflows"))?;
                    }
                    self.emit(|encoder| {
                        encoder.mov_reg_from_memory(
                            target,
                            MemoryOperand::new(Register::Ebp, displacement),
                            OperandWidth::Dword,
                            None,
                        )
                    })?;
                    self.statistics.loads += 1;
                    self.statistics.reloads += 1;
                    Ok(())
                }
                RegisterLocation::Register(_) => Err(codegen_error(
                    "wide i686 value has scalar register location",
                )),
            },
            NativeValue::Immediate(value) => self.emit(|encoder| {
                encoder.mov_reg_imm32(
                    target,
                    if high {
                        (value >> 32) as u32
                    } else {
                        value as u32
                    },
                )
            }),
        }
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
                    self.emit(|encoder| encoder.mov_reg_reg(target, source))?;
                }
            }
            RegisterLocation::RegisterPair { .. } => {
                return Err(codegen_error(
                    "scalar i686 value has register-pair location",
                ));
            }
            RegisterLocation::Spill { slot } => {
                let displacement = self.spill_displacement(slot)?;
                self.emit(|encoder| {
                    encoder.mov_reg_from_memory(
                        target,
                        MemoryOperand::new(Register::Ebp, displacement),
                        OperandWidth::Dword,
                        None,
                    )
                })?;
                self.statistics.loads += 1;
                self.statistics.reloads += 1;
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
                    self.emit(|encoder| encoder.mov_reg_reg(destination, source))?;
                }
            }
            RegisterLocation::RegisterPair { .. } => {
                return Err(codegen_error(
                    "scalar i686 value has register-pair location",
                ));
            }
            RegisterLocation::Spill { slot } => {
                let displacement = self.spill_displacement(slot)?;
                self.emit(|encoder| {
                    encoder.mov_memory_from_reg(
                        MemoryOperand::new(Register::Ebp, displacement),
                        source,
                        OperandWidth::Dword,
                    )
                })?;
                self.statistics.stores += 1;
            }
        }
        Ok(())
    }

    fn store_wide_immediate(
        &mut self,
        register: VirtualRegister,
        value: u64,
    ) -> Result<(), NativeMirError> {
        match self.location(register)? {
            RegisterLocation::RegisterPair { low, high } => {
                let low = machine_register(low);
                let high = machine_register(high);
                self.emit(|encoder| encoder.mov_reg_imm32(low, value as u32))?;
                self.emit(|encoder| encoder.mov_reg_imm32(high, (value >> 32) as u32))
            }
            RegisterLocation::Spill { slot } => {
                let displacement = self.spill_displacement(slot)?;
                self.emit(|encoder| {
                    encoder.mov_memory_imm32(
                        MemoryOperand::new(Register::Ebp, displacement),
                        value as u32,
                    )
                })?;
                self.emit(|encoder| {
                    encoder.mov_memory_imm32(
                        MemoryOperand::new(Register::Ebp, displacement + 4),
                        (value >> 32) as u32,
                    )
                })?;
                self.statistics.stores += 2;
                Ok(())
            }
            RegisterLocation::Register(_) => Err(codegen_error(
                "wide i686 value has scalar register location",
            )),
        }
    }

    fn load_wide(
        &mut self,
        register: VirtualRegister,
        low: Register,
        high: Register,
    ) -> Result<(), NativeMirError> {
        match self.location(register)? {
            RegisterLocation::RegisterPair {
                low: pair_low,
                high: pair_high,
            } => {
                let pair_low = machine_register(pair_low);
                let pair_high = machine_register(pair_high);
                if pair_low != low {
                    self.emit(|encoder| encoder.mov_reg_reg(low, pair_low))?;
                }
                if pair_high != high {
                    self.emit(|encoder| encoder.mov_reg_reg(high, pair_high))?;
                }
                Ok(())
            }
            RegisterLocation::Spill { slot } => {
                let displacement = self.spill_displacement(slot)?;
                self.emit(|encoder| {
                    encoder.mov_reg_from_memory(
                        low,
                        MemoryOperand::new(Register::Ebp, displacement),
                        OperandWidth::Dword,
                        None,
                    )
                })?;
                self.emit(|encoder| {
                    encoder.mov_reg_from_memory(
                        high,
                        MemoryOperand::new(Register::Ebp, displacement + 4),
                        OperandWidth::Dword,
                        None,
                    )
                })?;
                self.statistics.loads += 2;
                Ok(())
            }
            RegisterLocation::Register(_) => Err(codegen_error(
                "wide i686 value has scalar register location",
            )),
        }
    }

    fn copy_wide(
        &mut self,
        destination: VirtualRegister,
        source: VirtualRegister,
    ) -> Result<(), NativeMirError> {
        self.load_wide(source, Register::Eax, Register::Edx)?;
        self.store_wide_pair(destination, Register::Eax, Register::Edx)
    }

    fn store_wide_pair(
        &mut self,
        destination: VirtualRegister,
        low: Register,
        high: Register,
    ) -> Result<(), NativeMirError> {
        match self.location(destination)? {
            RegisterLocation::RegisterPair {
                low: pair_low,
                high: pair_high,
            } => {
                let pair_low = machine_register(pair_low);
                let pair_high = machine_register(pair_high);
                if pair_low != low {
                    self.emit(|encoder| encoder.mov_reg_reg(pair_low, low))?;
                }
                if pair_high != high {
                    self.emit(|encoder| encoder.mov_reg_reg(pair_high, high))?;
                }
                Ok(())
            }
            RegisterLocation::Spill { slot } => {
                let displacement = self.spill_displacement(slot)?;
                self.emit(|encoder| {
                    encoder.mov_memory_from_reg(
                        MemoryOperand::new(Register::Ebp, displacement),
                        low,
                        OperandWidth::Dword,
                    )
                })?;
                self.emit(|encoder| {
                    encoder.mov_memory_from_reg(
                        MemoryOperand::new(Register::Ebp, displacement + 4),
                        high,
                        OperandWidth::Dword,
                    )
                })?;
                self.statistics.stores += 2;
                Ok(())
            }
            RegisterLocation::Register(_) => Err(codegen_error(
                "wide i686 value has scalar register location",
            )),
        }
    }

    fn location(&self, register: VirtualRegister) -> Result<RegisterLocation, NativeMirError> {
        self.allocation
            .locations
            .get(&register)
            .copied()
            .ok_or_else(|| codegen_error(format!("v{} has no register allocation", register.0)))
    }

    fn spill_displacement(&self, slot: u32) -> Result<i32, NativeMirError> {
        let end = (self.saved.len() as u32)
            .checked_mul(4)
            .and_then(|saved| saved.checked_add(*self.spill_offsets.get(&slot)?))
            .ok_or_else(|| codegen_error("spill displacement exceeds u32"))?;
        i32::try_from(end)
            .map(|value| -value)
            .map_err(|_| codegen_error("spill displacement exceeds i32"))
    }

    fn stack_object_displacement(
        &self,
        object: crate::native_mir::StackObjectId,
    ) -> Result<i32, NativeMirError> {
        let end = self
            .stack_offsets
            .get(&object)
            .copied()
            .ok_or_else(|| codegen_error("unknown stack object"))?;
        let end = (self.saved.len() as u32)
            .checked_mul(4)
            .and_then(|saved| saved.checked_add(end))
            .ok_or_else(|| codegen_error("stack object displacement exceeds u32"))?;
        i32::try_from(end)
            .map(|value| -value)
            .map_err(|_| codegen_error("stack object displacement exceeds i32"))
    }

    fn normalize(&mut self, register: Register, ty: MachineType) -> Result<(), NativeMirError> {
        self.extend_or_truncate(register, ty, false)
    }

    fn extend_or_truncate(
        &mut self,
        register: Register,
        ty: MachineType,
        signed: bool,
    ) -> Result<(), NativeMirError> {
        match ty {
            MachineType::Bool | MachineType::I8 => self.emit(|encoder| {
                if signed {
                    encoder.movsx_reg_reg(register, register, OperandWidth::Byte)
                } else {
                    encoder.movzx_reg_reg(register, register, OperandWidth::Byte)
                }
            }),
            MachineType::I16 => self.emit(|encoder| {
                if signed {
                    encoder.movsx_reg_reg(register, register, OperandWidth::Word)
                } else {
                    encoder.movzx_reg_reg(register, register, OperandWidth::Word)
                }
            }),
            MachineType::I32 | MachineType::Pointer => Ok(()),
            MachineType::I64 => Err(unsupported("wide normalization in one i686 register")),
        }
    }

    fn emit(
        &mut self,
        instruction: impl FnOnce(&mut Encoder) -> Result<(), EncoderError>,
    ) -> Result<(), NativeMirError> {
        instruction(&mut self.encoder).map_err(encode_error)?;
        self.statistics.selected_instructions += 1;
        Ok(())
    }
}

fn layout_spill_slots(
    function: &NativeMirFunction,
    allocation: &RegisterAllocation,
) -> Result<(u32, BTreeMap<u32, u32>), NativeMirError> {
    let mut widths = BTreeMap::<u32, u32>::new();
    for (register, location) in &allocation.locations {
        let RegisterLocation::Spill { slot } = location else {
            continue;
        };
        let ty = function.register_types[register.0 as usize];
        let width = u32::from(ty.bytes(4)).max(4);
        widths
            .entry(*slot)
            .and_modify(|existing| *existing = (*existing).max(width))
            .or_insert(width);
    }
    let mut cursor = 0u32;
    let mut offsets = BTreeMap::new();
    for slot in 0..allocation.spill_slots {
        let width = widths
            .get(&slot)
            .copied()
            .ok_or_else(|| codegen_error("allocated spill slot has no virtual register"))?;
        cursor = cursor
            .checked_add(width)
            .ok_or_else(|| codegen_error("spill frame exceeds u32"))?;
        offsets.insert(slot, cursor);
    }
    Ok((cursor, offsets))
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

fn parameter_stack_bytes(ty: MachineType) -> u32 {
    if ty == MachineType::I64 {
        8
    } else {
        4
    }
}

fn operand_width(ty: MachineType) -> OperandWidth {
    match ty {
        MachineType::Bool | MachineType::I8 => OperandWidth::Byte,
        MachineType::I16 => OperandWidth::Word,
        MachineType::I32 | MachineType::Pointer | MachineType::I64 => OperandWidth::Dword,
    }
}

fn load_extension(ty: MachineType) -> Option<Extension> {
    match ty {
        MachineType::Bool | MachineType::I8 | MachineType::I16 => Some(Extension::Zero),
        MachineType::I32 | MachineType::Pointer | MachineType::I64 => None,
    }
}

fn scale_value(scale: u32) -> Result<Scale, NativeMirError> {
    match scale {
        1 => Ok(Scale::One),
        2 => Ok(Scale::Two),
        4 => Ok(Scale::Four),
        8 => Ok(Scale::Eight),
        _ => Err(codegen_error("i686 address scale must be 1, 2, 4, or 8")),
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

fn physical(register: Register) -> PhysicalRegister {
    PhysicalRegister(register as u8)
}

fn machine_register(register: PhysicalRegister) -> Register {
    Register::try_from(register.0).expect("validated i686 register policy")
}

fn align_up(value: u32, alignment: u32) -> Result<u32, NativeMirError> {
    let mask = alignment
        .checked_sub(1)
        .ok_or_else(|| codegen_error("zero alignment"))?;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or_else(|| codegen_error("alignment exceeds u32"))
}

fn align_bytes(bytes: &mut Vec<u8>, alignment: u32) -> Result<(), NativeMirError> {
    let aligned = align_up(checked_u32(bytes.len(), "section size")?, alignment)?;
    bytes.resize(
        usize::try_from(aligned).map_err(|_| codegen_error("section size exceeds usize"))?,
        0,
    );
    Ok(())
}

fn checked_u32(value: usize, what: &str) -> Result<u32, NativeMirError> {
    u32::try_from(value).map_err(|_| codegen_error(format!("{what} exceeds u32")))
}

fn encode_error(error: EncoderError) -> NativeMirError {
    NativeMirError::new("E_NATIVE_SELECT_I686", error.to_string())
}
fn codegen_error(message: impl Into<String>) -> NativeMirError {
    NativeMirError::new("E_NATIVE_SELECT_I686", message)
}
fn unsupported(operation: &str) -> NativeMirError {
    NativeMirError::new(
        "E_NATIVE_MIR_UNSUPPORTED",
        format!("DBYTE32 selector does not yet support {operation}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_hir::{Profile, TargetDataLayout};
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn mir(source: &str) -> NativeMirProject {
        let syntax = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let executable =
            dbyte_hir::analyze_executable(&syntax, Profile::I686None, TargetDataLayout::I686)
                .unwrap();
        let concrete = dbyte_hir::monomorphize(&executable).unwrap();
        crate::native_mir_lower::lower_program_to_native_mir(&concrete).unwrap()
    }

    #[test]
    fn i686_selector_emits_real_et_rel_and_uses_width_safe_allocation() {
        let project =
            mir("pub fn kernel_main() -> int:\n    let value: int = 40\n    return value + 2\n");
        let selected = select_project_object(&project, NativeOptimizationLevel::O1).unwrap();
        let parsed = crate::object::parse_relocatable(&selected.object.bytes).unwrap();
        assert!(parsed
            .symbols
            .iter()
            .any(|symbol| symbol.name == "kernel_main"));
        assert!(selected.machine_dump.contains("assigned="));
        assert_eq!(
            selected.object.bytes,
            select_project_object(&project, NativeOptimizationLevel::O1)
                .unwrap()
                .object
                .bytes
        );
    }

    #[test]
    fn dbyte32_interrupt_resume_emits_exact_context_restore() {
        let project = mir(
            "fn switch(resume: usize) -> int:\n    native.interrupt_resume(resume)\n\npub fn kernel_main() -> int:\n    return switch(0 as usize)\n",
        );
        let function = project
            .functions
            .iter()
            .find(|function| function.symbol == "switch" || function.symbol.ends_with("_switch"))
            .unwrap();
        let selected = select_function(function, NativeOptimizationLevel::O0).unwrap();
        assert!(selected
            .bytes
            .windows(4)
            .any(|bytes| bytes == [0x89, 0xc4, 0x61, 0xcf]));
    }

    #[test]
    fn dbyte32_multiboot_entry_uses_bootstrap_argument_order() {
        let project = mir(
            "pub fn kernel_main(multiboot_magic: u32, multiboot_info: usize) -> int:\n    return multiboot_magic as int\n",
        );
        let function = project
            .functions
            .iter()
            .find(|function| function.symbol == "kernel_main")
            .unwrap();
        let selected = select_function(function, NativeOptimizationLevel::O0).unwrap();
        let magic = selected
            .bytes
            .windows(6)
            .position(|bytes| bytes == [0x8b, 0x85, 0x08, 0x00, 0x00, 0x00])
            .expect("multiboot magic must load from [ebp+8]");
        let info = selected
            .bytes
            .windows(6)
            .position(|bytes| bytes == [0x8b, 0x85, 0x0c, 0x00, 0x00, 0x00])
            .expect("multiboot info must load from [ebp+12]");
        assert!(
            magic < info,
            "bootstrap arguments were materialized in reverse order"
        );
    }

    #[test]
    fn dbyte32_wide_parameters_and_call_results_use_two_word_abi() {
        let project = mir(
            "fn identity(value: i64) -> i64:\n    return value\n\npub fn caller() -> i64:\n    return identity(42 as i64)\n",
        );
        let identity = project
            .functions
            .iter()
            .find(|function| function.symbol == "identity")
            .unwrap();
        let caller = project
            .functions
            .iter()
            .find(|function| function.symbol == "caller")
            .unwrap();
        select_function(identity, NativeOptimizationLevel::O0).unwrap();
        select_function(caller, NativeOptimizationLevel::O0).unwrap();
    }

    #[test]
    fn dbyte32_aggregate_result_uses_hidden_sret_and_selects_end_to_end() {
        let project = mir(
            "struct Pair:\n    left: u32\n    right: u32\n\nfn make(left: u32, right: u32) -> Pair:\n    return Pair(left = left, right = right)\n\npub fn kernel_main() -> int:\n    let pair = make(40 as u32, 2 as u32)\n    return (pair.left + pair.right) as int\n",
        );
        let make_symbol = project
            .functions
            .iter()
            .find(|function| function.symbol.ends_with("_make") || function.symbol == "make")
            .unwrap()
            .symbol
            .clone();
        let selected = select_project_object(&project, NativeOptimizationLevel::O1).unwrap();
        assert!(selected
            .machine_dump
            .lines()
            .any(|line| line.starts_with(&format!("function {make_symbol} "))));
        assert!(!selected.object.bytes.is_empty());
    }

    #[test]
    fn dbyte32_aggregate_by_value_parameter_uses_caller_owned_stack_copy() {
        let project = mir(
            "struct Pair:\n    left: u32\n    right: u32\n\nfn sum(pair: Pair) -> u32:\n    return pair.left + pair.right\n\npub fn kernel_main() -> int:\n    let pair = Pair(left = 40 as u32, right = 2 as u32)\n    return sum(pair) as int\n",
        );
        let sum = project
            .functions
            .iter()
            .find(|function| function.symbol.ends_with("_sum") || function.symbol == "sum")
            .unwrap();
        assert!(sum.parameter_aggregates.iter().any(Option::is_some));
        select_project_object(&project, NativeOptimizationLevel::O1).unwrap();
    }

    #[test]
    fn dbyte32_aggregate_zero_restores_edi_after_rep_stosb() {
        let project = mir(
            "struct Pair:\n    left: u32\n    right: u32\n\npub fn kernel_main() -> int:\n    let pair = Pair(left = 40 as u32, right = 2 as u32)\n    return (pair.left + pair.right) as int\n",
        );
        let function = project
            .functions
            .iter()
            .find(|function| function.symbol == "kernel_main")
            .unwrap();
        let selected = select_function(function, NativeOptimizationLevel::O0).unwrap();
        let stos = selected
            .bytes
            .windows(2)
            .position(|bytes| bytes == [0xf3, 0xaa])
            .expect("aggregate construction must emit rep stosb");
        assert_eq!(
            selected.bytes.get(stos + 2),
            Some(&0x5f),
            "rep stosb must restore EDI before any live virtual value can observe it"
        );
    }

    #[test]
    fn dbyte32_division_moves_divisor_out_of_edx_before_dividend_extension() {
        let project = mir(
            "pub fn unsigned_div(left: u32, right: u32) -> u32:\n    return left / right\n\npub fn signed_div(left: int, right: int) -> int:\n    return left / right\n",
        );
        let unsigned = project
            .functions
            .iter()
            .find(|function| function.symbol == "unsigned_div")
            .unwrap();
        let signed = project
            .functions
            .iter()
            .find(|function| function.symbol == "signed_div")
            .unwrap();
        let unsigned = select_function(unsigned, NativeOptimizationLevel::O0).unwrap();
        let signed = select_function(signed, NativeOptimizationLevel::O0).unwrap();
        assert!(unsigned.bytes.windows(2).any(|bytes| bytes == [0xf7, 0xf1]));
        assert!(signed.bytes.windows(2).any(|bytes| bytes == [0xf7, 0xf9]));
        assert!(!unsigned.bytes.windows(2).any(|bytes| bytes == [0xf7, 0xf2]));
        assert!(!signed.bytes.windows(2).any(|bytes| bytes == [0xf7, 0xfa]));
    }

    #[test]
    fn dbyte32_aggregate_copy_restores_edi_and_esi_after_rep_movsb() {
        let project = mir(
            "struct Pair:\n    left: u32\n    right: u32\n\npub fn kernel_main() -> int:\n    let source = Pair(left = 40 as u32, right = 2 as u32)\n    let pair = source\n    return (pair.left + pair.right) as int\n",
        );
        let function = project
            .functions
            .iter()
            .find(|function| function.symbol == "kernel_main")
            .unwrap();
        let selected = select_function(function, NativeOptimizationLevel::O0).unwrap();
        let movs = selected
            .bytes
            .windows(2)
            .position(|bytes| bytes == [0xf3, 0xa4])
            .expect("aggregate assignment must emit rep movsb");
        assert_eq!(
            selected.bytes.get(movs + 2..movs + 4),
            Some(&[0x5f, 0x5e][..]),
            "rep movsb must restore EDI and ESI before live virtual values resume"
        );
    }

    #[test]
    fn dbyte32_wide_indirect_load_and_store_select_end_to_end() {
        let project = mir(
            "fn read(value: borrow u64) -> u64:\n    return *value\n\nfn write(value: borrow mut u64, replacement: u64) -> unit:\n    *value = replacement\n    return unit\n\npub fn kernel_main() -> int:\n    let value: u64 = 40 as u64\n    write(borrow mut value, 42 as u64)\n    if read(borrow value) == (42 as u64):\n        return 0\n    return 1\n",
        );
        let selected = select_project_object(&project, NativeOptimizationLevel::O1).unwrap();
        assert!(!selected.object.bytes.is_empty());
        assert_eq!(
            selected.object.bytes,
            select_project_object(&project, NativeOptimizationLevel::O1)
                .unwrap()
                .object
                .bytes
        );
    }

    #[test]
    fn i64_virtual_registers_use_only_wide_pairs_or_wide_spill_storage() {
        let project = mir("pub fn kernel_main() -> i64:\n    return 42 as i64\n");
        let function = &project.functions[0];
        let allocation = allocate_registers(
            function,
            &dbyte32_register_policy(),
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert!(function
            .register_types
            .iter()
            .enumerate()
            .filter(|(_, ty)| **ty == MachineType::I64)
            .all(|(index, _)| {
                matches!(
                    allocation.locations[&VirtualRegister(index as u32)],
                    RegisterLocation::RegisterPair { .. } | RegisterLocation::Spill { .. }
                )
            }));
        assert!(allocation.locations.values().any(|location| matches!(
            location,
            RegisterLocation::RegisterPair { low, high }
                if *low == physical(Register::Esi) && *high == physical(Register::Edi)
        )));
    }
}
