//! Systems IR to per-module i686 code and ELF-relocatable object image.

use crate::diagnostics::{Diagnostic, DiagnosticCategory, NativeResult};
use crate::ir::{CompareOp, LabelId, LocalId};
use crate::object::{
    ObjectImage, ObjectRelocation, ObjectSection, ObjectSectionData, ObjectSymbol, RelocationKind,
    SectionContents, SymbolBinding, SymbolDefinition, SymbolId, SymbolKind, ENTRY_METADATA_PREFIX,
    PROFILE_METADATA_PREFIX, TASK_METADATA_PREFIX,
};
use crate::system_types::SystemType;
use crate::systems_ir::{
    GlobalSection, SystemIntrinsic, SystemsFunction, SystemsInst, SystemsModuleIr, SystemsSlot,
    Visibility,
};
use crate::x86::{
    Condition, Encoder, EncoderError, Extension, Label, MemoryOperand, OperandWidth, Register,
    Scale, ShiftKind, DEFAULT_MAX_CODE_SIZE,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

const MAX_FRAME_SIZE: u32 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoweredSystemsObject {
    pub image: ObjectImage,
    pub machine_dump: String,
}

pub fn lower_systems_module(module: &SystemsModuleIr) -> NativeResult<LoweredSystemsObject> {
    validate_module(module)?;

    let mut rodata = Vec::new();
    let mut data = Vec::new();
    let mut bss_size = 0u32;
    let mut section_alignment = BTreeMap::from([
        (ObjectSection::Text, 16u32),
        (ObjectSection::Rodata, 1u32),
        (ObjectSection::Data, 1u32),
        (ObjectSection::Bss, 1u32),
    ]);
    let mut global_locations = BTreeMap::<String, (ObjectSection, u32)>::new();

    let mut globals: Vec<_> = module.globals.iter().collect();
    globals.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for global in globals {
        let section = match global.section {
            GlobalSection::ReadOnly => ObjectSection::Rodata,
            GlobalSection::Data => ObjectSection::Data,
            GlobalSection::Bss => ObjectSection::Bss,
        };
        section_alignment
            .entry(section)
            .and_modify(|alignment| *alignment = (*alignment).max(global.alignment));
        let offset = match section {
            ObjectSection::Rodata => append_aligned(
                &mut rodata,
                &global.bytes,
                global.size,
                global.alignment,
                "native rodata global",
            )?,
            ObjectSection::Data => append_aligned(
                &mut data,
                &global.bytes,
                global.size,
                global.alignment,
                "native data global",
            )?,
            ObjectSection::Bss => {
                bss_size = align_u32(bss_size, global.alignment, "native bss global")?;
                let offset = bss_size;
                bss_size = bss_size.checked_add(global.size).ok_or_else(|| {
                    backend_error(
                        DiagnosticCategory::LayoutOverflow,
                        "native bss size overflow",
                    )
                })?;
                offset
            }
            ObjectSection::Text => unreachable!(),
        };
        global_locations.insert(global.symbol.clone(), (section, offset));
    }

    let mut string_locations = Vec::with_capacity(module.strings.len());
    for bytes in &module.strings {
        if !bytes.is_ascii() {
            return Err(backend_error(
                DiagnosticCategory::InvalidType,
                "native systems rodata string is not ASCII",
            ));
        }
        let offset = u32::try_from(rodata.len()).map_err(|_| {
            backend_error(
                DiagnosticCategory::LayoutOverflow,
                "native rodata offset overflow",
            )
        })?;
        rodata.extend_from_slice(bytes);
        rodata.push(0);
        string_locations.push(offset);
    }

    let mut symbols = Vec::new();
    let mut symbol_ids = BTreeMap::<String, SymbolId>::new();
    for (name, section) in [
        (".text", ObjectSection::Text),
        (".rodata", ObjectSection::Rodata),
        (".data", ObjectSection::Data),
        (".bss", ObjectSection::Bss),
    ] {
        push_symbol(
            &mut symbols,
            Some(&mut symbol_ids),
            ObjectSymbol {
                name: name.into(),
                binding: SymbolBinding::Local,
                kind: SymbolKind::Section,
                definition: SymbolDefinition::Defined { section, value: 0 },
                size: 0,
            },
        )?;
    }
    if module.user_profile {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: format!("{PROFILE_METADATA_PREFIX}user"),
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            },
        )?;
    }

    let mut private_functions: Vec<_> = module
        .functions
        .iter()
        .filter(|function| {
            function.visibility == Visibility::Private && function.symbol != "kernel_main"
        })
        .collect();
    private_functions.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for function in private_functions {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: function.symbol.clone(),
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            },
        )?;
    }
    let mut interrupt_bodies: Vec<_> = module
        .functions
        .iter()
        .filter(|function| function.interrupt.is_some())
        .collect();
    interrupt_bodies.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for function in interrupt_bodies {
        let name = interrupt_body_symbol(function);
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name,
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            },
        )?;
        let marker = interrupt_definition_marker(function);
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: marker,
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            },
        )?;
    }
    let mut use_markers = BTreeSet::new();
    for function in &module.functions {
        for instruction in &function.instructions {
            if let SystemsInst::InterruptAddress {
                vector,
                symbol,
                class,
            } = instruction
            {
                use_markers.insert(format!(
                    "__dbyte_interrupt:use:{vector:03}:{class}:{symbol}"
                ));
            }
        }
    }
    for marker in use_markers {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: marker,
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            },
        )?;
    }
    let mut task_markers = BTreeSet::new();
    for function in &module.functions {
        for instruction in &function.instructions {
            match instruction {
                SystemsInst::TaskAddress { symbol } => {
                    task_markers.insert(format!("{TASK_METADATA_PREFIX}def:{symbol}"));
                    task_markers.insert(format!("{TASK_METADATA_PREFIX}use:{symbol}"));
                }
                SystemsInst::LinkerAddress { symbol } if symbol == "__dbyte_task_trampoline" => {
                    task_markers.insert(format!("{TASK_METADATA_PREFIX}trampoline"));
                }
                _ => {}
            }
        }
    }
    for marker in task_markers {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: marker,
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            },
        )?;
    }
    for (index, offset) in string_locations.iter().enumerate() {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: string_symbol(index),
                binding: SymbolBinding::Local,
                kind: SymbolKind::Object,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Rodata,
                    value: *offset,
                },
                size: u32::try_from(module.strings[index].len() + 1).map_err(|_| {
                    backend_error(
                        DiagnosticCategory::LayoutOverflow,
                        "native string size overflow",
                    )
                })?,
            },
        )?;
    }
    let mut private_globals: Vec<_> = module
        .globals
        .iter()
        .filter(|global| global.visibility == Visibility::Private)
        .collect();
    private_globals.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for global in private_globals {
        let (section, value) = global_locations[&global.symbol];
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            object_symbol(global, SymbolBinding::Local, section, value),
        )?;
    }

    let mut public_functions: Vec<_> = module
        .functions
        .iter()
        .filter(|function| {
            function.visibility == Visibility::Public || function.symbol == "kernel_main"
        })
        .collect();
    public_functions.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for function in public_functions {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: function.symbol.clone(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            },
        )?;
        if function.symbol == "kernel_main" {
            let abi = if function.parameter_types.is_empty() {
                "legacy"
            } else {
                "multiboot_v1"
            };
            insert_named_symbol(
                &mut symbols,
                &mut symbol_ids,
                ObjectSymbol {
                    name: format!("{ENTRY_METADATA_PREFIX}{abi}"),
                    binding: SymbolBinding::Local,
                    kind: SymbolKind::Function,
                    definition: SymbolDefinition::Defined {
                        section: ObjectSection::Text,
                        value: 0,
                    },
                    size: 0,
                },
            )?;
        }
    }
    let mut public_globals: Vec<_> = module
        .globals
        .iter()
        .filter(|global| global.visibility == Visibility::Public)
        .collect();
    public_globals.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for global in public_globals {
        let (section, value) = global_locations[&global.symbol];
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            object_symbol(global, SymbolBinding::Global, section, value),
        )?;
    }

    let defined: BTreeSet<String> = symbol_ids.keys().cloned().collect();
    let mut undefined_functions: Vec<_> = module
        .external_functions
        .iter()
        .filter(|external| !defined.contains(&external.symbol))
        .collect();
    undefined_functions.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for external in undefined_functions {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: external.symbol.clone(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Undefined,
                size: 0,
            },
        )?;
    }
    let mut undefined_globals: Vec<_> = module
        .external_globals
        .iter()
        .filter(|external| !defined.contains(&external.symbol))
        .collect();
    undefined_globals.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for external in undefined_globals {
        insert_named_symbol(
            &mut symbols,
            &mut symbol_ids,
            ObjectSymbol {
                name: external.symbol.clone(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Object,
                definition: SymbolDefinition::Undefined,
                size: 0,
            },
        )?;
    }
    let linker_symbols: BTreeSet<String> = module
        .functions
        .iter()
        .flat_map(|function| function.instructions.iter())
        .filter_map(|instruction| match instruction {
            SystemsInst::LinkerAddress { symbol } => Some(symbol.clone()),
            _ => None,
        })
        .collect();
    for symbol in linker_symbols {
        if !symbol_ids.contains_key(&symbol) {
            let kind = if symbol == "__dbyte_task_trampoline" {
                SymbolKind::Function
            } else {
                SymbolKind::Object
            };
            insert_named_symbol(
                &mut symbols,
                &mut symbol_ids,
                ObjectSymbol {
                    name: symbol,
                    binding: SymbolBinding::Global,
                    kind,
                    definition: SymbolDefinition::Undefined,
                    size: 0,
                },
            )?;
        }
    }

    let mut encoder = Encoder::new(DEFAULT_MAX_CODE_SIZE);
    let mut relocations = Vec::new();
    let mut machine_dump = String::new();
    let _ = writeln!(machine_dump, "module {}", module.module_name);
    let mut functions: Vec<_> = module.functions.iter().collect();
    functions.sort_by(|left, right| left.symbol.cmp(&right.symbol));
    for function in functions {
        let start = checked_u32(encoder.position(), "native function offset")?;
        let body_start = if let Some(class) = function.interrupt {
            lower_interrupt_stub(function, class, &mut encoder, &symbol_ids, &mut relocations)?;
            Some(checked_u32(encoder.position(), "interrupt body offset")?)
        } else {
            None
        };
        lower_function(
            function,
            &mut encoder,
            &symbol_ids,
            &mut relocations,
            &mut machine_dump,
        )?;
        let end = checked_u32(encoder.position(), "native function end")?;
        let symbol_id = symbol_ids.get(&function.symbol).copied().ok_or_else(|| {
            backend_error(
                DiagnosticCategory::UndefinedSymbol,
                format!(
                    "missing object symbol for native function '{}'",
                    function.symbol
                ),
            )
        })?;
        let symbol = symbols.get_mut(symbol_id.0 as usize).ok_or_else(|| {
            backend_error(
                DiagnosticCategory::InvalidObject,
                "function symbol id is out of range",
            )
        })?;
        symbol.definition = SymbolDefinition::Defined {
            section: ObjectSection::Text,
            value: start,
        };
        symbol.size = body_start.unwrap_or(end) - start;
        if let Some(body_start) = body_start {
            let body_name = interrupt_body_symbol(function);
            let body_id = symbol_ids.get(&body_name).copied().ok_or_else(|| {
                backend_error(
                    DiagnosticCategory::UndefinedSymbol,
                    format!("missing object symbol for interrupt body '{body_name}'"),
                )
            })?;
            let body_symbol = symbols.get_mut(body_id.0 as usize).ok_or_else(|| {
                backend_error(
                    DiagnosticCategory::InvalidObject,
                    "interrupt body symbol id is out of range",
                )
            })?;
            body_symbol.definition = SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: body_start,
            };
            body_symbol.size = end - body_start;
        }
    }
    let encoded = encoder
        .finish()
        .map_err(|error| encoder_error(error, None))?;
    machine_dump.push_str(&encoded.dump());
    relocations.sort_by(|left, right| {
        left.section
            .cmp(&right.section)
            .then(left.offset.cmp(&right.offset))
            .then(left.symbol.cmp(&right.symbol))
    });

    let sections = BTreeMap::from([
        (
            ObjectSection::Text,
            ObjectSectionData {
                contents: SectionContents::Bytes(encoded.bytes),
                alignment: section_alignment[&ObjectSection::Text],
            },
        ),
        (
            ObjectSection::Rodata,
            ObjectSectionData {
                contents: SectionContents::Bytes(rodata),
                alignment: section_alignment[&ObjectSection::Rodata],
            },
        ),
        (
            ObjectSection::Data,
            ObjectSectionData {
                contents: SectionContents::Bytes(data),
                alignment: section_alignment[&ObjectSection::Data],
            },
        ),
        (
            ObjectSection::Bss,
            ObjectSectionData {
                contents: SectionContents::Uninitialized(bss_size),
                alignment: section_alignment[&ObjectSection::Bss],
            },
        ),
    ]);
    Ok(LoweredSystemsObject {
        image: ObjectImage {
            sections,
            symbols,
            relocations,
        },
        machine_dump,
    })
}

fn interrupt_body_symbol(function: &SystemsFunction) -> String {
    format!("__dbyte_interrupt_{}_body", function.symbol)
}

fn interrupt_definition_marker(function: &SystemsFunction) -> String {
    format!(
        "__dbyte_interrupt:def:{}:{}",
        function.interrupt.expect("interrupt definition marker"),
        function.symbol
    )
}

fn lower_interrupt_stub(
    function: &SystemsFunction,
    class: dbyte_ast::InterruptClass,
    encoder: &mut Encoder,
    symbols: &BTreeMap<String, SymbolId>,
    relocations: &mut Vec<ObjectRelocation>,
) -> NativeResult<()> {
    emit(encoder, function, Encoder::pushad)?;
    if !class.has_error_code() {
        emit(encoder, function, |encoder| {
            encoder.lea_reg_from_memory(Register::Eax, MemoryOperand::new(Register::Esp, 32))
        })?;
        push(encoder, function, Register::Eax)?;
    } else {
        emit(encoder, function, |encoder| {
            encoder.mov_reg_from_memory(
                Register::Eax,
                MemoryOperand::new(Register::Esp, 32),
                OperandWidth::Dword,
                None,
            )
        })?;
        emit(encoder, function, |encoder| {
            encoder.lea_reg_from_memory(Register::Edx, MemoryOperand::new(Register::Esp, 36))
        })?;
        push(encoder, function, Register::Edx)?;
        push(encoder, function, Register::Eax)?;
    }
    let immediate = encoder
        .call_rel32_placeholder()
        .map_err(|error| encoder_error(error, Some(function)))?;
    relocations.push(ObjectRelocation {
        section: ObjectSection::Text,
        offset: checked_u32(immediate, "interrupt body call relocation")?,
        symbol: lookup_symbol(symbols, &interrupt_body_symbol(function))?,
        kind: RelocationKind::Pc32,
        addend: -4,
    });
    let cleanup = if class.has_error_code() { 8 } else { 4 };
    emit(encoder, function, |encoder| encoder.add_esp_imm32(cleanup))?;
    emit(encoder, function, Encoder::popad)?;
    if class.has_error_code() {
        emit(encoder, function, |encoder| encoder.add_esp_imm32(4))?;
    }
    emit(encoder, function, Encoder::iretd)
}

fn lower_function(
    function: &SystemsFunction,
    encoder: &mut Encoder,
    symbols: &BTreeMap<String, SymbolId>,
    relocations: &mut Vec<ObjectRelocation>,
    dump: &mut String,
) -> NativeResult<()> {
    let frame = FrameLayout::build(&function.slots)?;
    let start = encoder.position();
    emit(encoder, function, |encoder| encoder.push_reg(Register::Ebp))?;
    emit(encoder, function, |encoder| {
        encoder.mov_reg_reg(Register::Ebp, Register::Esp)
    })?;
    let uses_c_abi = function.call_convention == dbyte_ast::CallConvention::C
        || function.instructions.iter().any(|instruction| {
            matches!(
                instruction,
                SystemsInst::Call {
                    call_convention: dbyte_ast::CallConvention::C,
                    ..
                } | SystemsInst::CallPacked {
                    call_convention: dbyte_ast::CallConvention::C,
                    ..
                }
            )
        });
    if uses_c_abi {
        emit(encoder, function, |encoder| {
            encoder.and_reg_imm32(Register::Esp, 0xfffffff0)
        })?;
    }
    if frame.size != 0 {
        emit(encoder, function, |encoder| {
            encoder.sub_esp_imm32(frame.size)
        })?;
        for offset in (4..=frame.size).step_by(4) {
            let displacement = -i32::try_from(offset)
                .map_err(|_| function_error(function, "native frame displacement exceeds i32"))?;
            emit(encoder, function, |encoder| {
                encoder.mov_ebp_disp32_imm32(displacement, 0)
            })?;
        }
    }
    if function.call_convention == dbyte_ast::CallConvention::C {
        emit(encoder, function, |encoder| encoder.push_reg(Register::Ebx))?;
        emit(encoder, function, |encoder| encoder.push_reg(Register::Esi))?;
        emit(encoder, function, |encoder| encoder.push_reg(Register::Edi))?;
    }
    let multiboot_entry = function.symbol == "kernel_main"
        && function.parameter_types == [SystemType::U32, SystemType::Usize];
    for (index, parameter) in function.params.iter().enumerate() {
        let incoming_payload =
            if multiboot_entry || function.call_convention == dbyte_ast::CallConvention::C {
                function.params[..index]
                    .iter()
                    .map(|parameter| function.slots[parameter.slot.index()].storage_size.max(4))
                    .try_fold(0usize, |total, size| total.checked_add(size as usize))
            } else {
                function.params[index + 1..]
                    .iter()
                    .map(|parameter| {
                        align_u32(
                            function.slots[parameter.slot.index()].storage_size,
                            4,
                            "native aggregate parameter",
                        )
                    })
                    .collect::<NativeResult<Vec<_>>>()?
                    .into_iter()
                    .try_fold(0usize, |total, size| total.checked_add(size as usize))
            }
            .ok_or_else(|| function_error(function, "native incoming parameter offset overflow"))?;
        let incoming = 8usize
            .checked_add(incoming_payload)
            .ok_or_else(|| function_error(function, "native incoming parameter offset overflow"))?;
        let incoming = i32::try_from(incoming).map_err(|_| {
            function_error(function, "native incoming parameter offset exceeds i32")
        })?;
        let destination = frame.offset(parameter.slot, function)?;
        if parameter.ty.is_scalar() {
            emit(encoder, function, |encoder| {
                encoder.mov_reg_from_ebp_disp32(Register::Eax, incoming)
            })?;
            store_memory(
                encoder,
                function,
                MemoryOperand::new(Register::Ebp, destination),
                Register::Eax,
                &parameter.ty,
            )?;
        } else {
            let size = function.slots[parameter.slot.index()].storage_size;
            emit(encoder, function, |encoder| {
                encoder
                    .lea_reg_from_memory(Register::Esi, MemoryOperand::new(Register::Ebp, incoming))
            })?;
            emit(encoder, function, |encoder| {
                encoder.lea_reg_from_memory(
                    Register::Edi,
                    MemoryOperand::new(Register::Ebp, destination),
                )
            })?;
            emit(encoder, function, |encoder| {
                encoder.mov_reg_imm32(Register::Ecx, size)
            })?;
            emit(encoder, function, Encoder::rep_movsb)?;
        }
    }

    let mut labels = BTreeMap::<LabelId, Label>::new();
    for instruction in &function.instructions {
        if let SystemsInst::Label(id) = instruction {
            if labels.contains_key(id) {
                return Err(function_error(
                    function,
                    format!("duplicate systems IR label {id}"),
                ));
            }
            labels.insert(
                *id,
                encoder
                    .new_label()
                    .map_err(|error| encoder_error(error, Some(function)))?,
            );
        }
    }

    let mut depth = 0usize;
    let mut returns = 0usize;
    for instruction in &function.instructions {
        lower_instruction(
            instruction,
            function,
            &frame,
            &labels,
            symbols,
            relocations,
            encoder,
            &mut depth,
            &mut returns,
        )?;
    }
    let terminal_resume = function.instructions.iter().any(|instruction| {
        matches!(
            instruction,
            SystemsInst::Intrinsic {
                intrinsic: SystemIntrinsic::InterruptResume,
                ..
            }
        )
    });
    if returns == 0 && !terminal_resume {
        return Err(function_error(
            function,
            "native systems function has no return instruction",
        ));
    }
    if depth != 0 {
        return Err(function_error(
            function,
            format!("native systems function ends with evaluation depth {depth}"),
        ));
    }
    let _ = writeln!(
        dump,
        "function {} offset=0x{start:08x} size={} frame={} params={} slots={}",
        function.symbol,
        encoder.position() - start,
        frame.size,
        function.params.len(),
        function.slots.len()
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn lower_instruction(
    instruction: &SystemsInst,
    function: &SystemsFunction,
    frame: &FrameLayout,
    labels: &BTreeMap<LabelId, Label>,
    symbols: &BTreeMap<String, SymbolId>,
    relocations: &mut Vec<ObjectRelocation>,
    encoder: &mut Encoder,
    depth: &mut usize,
    returns: &mut usize,
) -> NativeResult<()> {
    match instruction {
        SystemsInst::Const { bits, .. } => {
            emit(encoder, function, |encoder| {
                encoder.mov_reg_imm32(Register::Eax, *bits)
            })?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::ConstBool(value) => {
            emit(encoder, function, |encoder| {
                encoder.mov_reg_imm32(Register::Eax, u32::from(*value))
            })?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::StringAddress(id) => {
            let symbol = string_symbol(id.index());
            emit_absolute_address(encoder, function, symbols, relocations, &symbol)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::LocalAddress(local) => {
            let offset = frame.offset(*local, function)?;
            emit(encoder, function, |encoder| {
                encoder
                    .lea_reg_from_memory(Register::Eax, MemoryOperand::new(Register::Ebp, offset))
            })?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::LocalLoad { local, ty } => {
            let offset = frame.offset(*local, function)?;
            load_memory(
                encoder,
                function,
                Register::Eax,
                MemoryOperand::new(Register::Ebp, offset),
                ty,
            )?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::LocalStore { local, ty } => {
            consume(depth, 1, function, "local.store")?;
            pop(encoder, function, Register::Eax)?;
            let offset = frame.offset(*local, function)?;
            store_memory(
                encoder,
                function,
                MemoryOperand::new(Register::Ebp, offset),
                Register::Eax,
                ty,
            )?;
        }
        SystemsInst::GlobalAddress { symbol, .. } => {
            emit_absolute_address(encoder, function, symbols, relocations, symbol)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::FunctionAddress { symbol } => {
            emit_absolute_address(encoder, function, symbols, relocations, symbol)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::Load(ty) => {
            consume(depth, 1, function, "load")?;
            pop(encoder, function, Register::Edx)?;
            load_memory(
                encoder,
                function,
                Register::Eax,
                MemoryOperand::new(Register::Edx, 0),
                ty,
            )?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::Store(ty) => {
            consume(depth, 2, function, "store")?;
            pop(encoder, function, Register::Eax)?;
            pop(encoder, function, Register::Edx)?;
            store_memory(
                encoder,
                function,
                MemoryOperand::new(Register::Edx, 0),
                Register::Eax,
                ty,
            )?;
        }
        SystemsInst::AggregateCopy { size } => {
            consume(depth, 2, function, "aggregate.copy")?;
            pop(encoder, function, Register::Esi)?;
            pop(encoder, function, Register::Edi)?;
            emit(encoder, function, |encoder| {
                encoder.mov_reg_imm32(Register::Ecx, *size)
            })?;
            emit(encoder, function, Encoder::rep_movsb)?;
        }
        SystemsInst::AggregateZero { size } => {
            consume(depth, 1, function, "aggregate.zero")?;
            pop(encoder, function, Register::Edi)?;
            emit(encoder, function, |encoder| {
                encoder.xor_reg_reg(Register::Eax, Register::Eax)
            })?;
            emit(encoder, function, |encoder| {
                encoder.mov_reg_imm32(Register::Ecx, *size)
            })?;
            emit(encoder, function, Encoder::rep_stosb)?;
        }
        SystemsInst::Trap => emit(encoder, function, Encoder::ud2)?,
        SystemsInst::Add(ty)
        | SystemsInst::Sub(ty)
        | SystemsInst::Mul(ty)
        | SystemsInst::BitAnd(ty)
        | SystemsInst::BitOr(ty)
        | SystemsInst::BitXor(ty) => {
            consume(depth, 2, function, "binary operation")?;
            pop(encoder, function, Register::Ecx)?;
            pop(encoder, function, Register::Eax)?;
            match instruction {
                SystemsInst::Add(_) => emit(encoder, function, |encoder| {
                    encoder.add_reg_reg(Register::Eax, Register::Ecx)
                })?,
                SystemsInst::Sub(_) => emit(encoder, function, |encoder| {
                    encoder.sub_reg_reg(Register::Eax, Register::Ecx)
                })?,
                SystemsInst::Mul(_) => emit(encoder, function, |encoder| {
                    encoder.imul_reg_reg(Register::Eax, Register::Ecx)
                })?,
                SystemsInst::BitAnd(_) => emit(encoder, function, |encoder| {
                    encoder.and_reg_reg(Register::Eax, Register::Ecx)
                })?,
                SystemsInst::BitOr(_) => emit(encoder, function, |encoder| {
                    encoder.or_reg_reg(Register::Eax, Register::Ecx)
                })?,
                SystemsInst::BitXor(_) => emit(encoder, function, |encoder| {
                    encoder.xor_reg_reg(Register::Eax, Register::Ecx)
                })?,
                _ => unreachable!(),
            }
            normalize(encoder, function, Register::Eax, ty)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::Div(ty) | SystemsInst::Rem(ty) => {
            consume(depth, 2, function, "division")?;
            pop(encoder, function, Register::Ecx)?;
            pop(encoder, function, Register::Eax)?;
            if ty.is_signed_integer() {
                emit(encoder, function, |encoder| encoder.cdq())?;
                emit(encoder, function, |encoder| encoder.idiv_reg(Register::Ecx))?;
            } else {
                emit(encoder, function, |encoder| {
                    encoder.xor_reg_reg(Register::Edx, Register::Edx)
                })?;
                emit(encoder, function, |encoder| encoder.div_reg(Register::Ecx))?;
            }
            if matches!(instruction, SystemsInst::Rem(_)) {
                emit(encoder, function, |encoder| {
                    encoder.mov_reg_reg(Register::Eax, Register::Edx)
                })?;
            }
            normalize(encoder, function, Register::Eax, ty)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::Neg(ty) | SystemsInst::BitNot(ty) => {
            consume(depth, 1, function, "unary integer operation")?;
            pop(encoder, function, Register::Eax)?;
            match instruction {
                SystemsInst::Neg(_) => {
                    emit(encoder, function, |encoder| encoder.neg_reg(Register::Eax))?
                }
                SystemsInst::BitNot(_) => {
                    emit(encoder, function, |encoder| encoder.not_reg(Register::Eax))?
                }
                _ => unreachable!(),
            }
            normalize(encoder, function, Register::Eax, ty)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::ShiftLeft(ty) | SystemsInst::ShiftRight(ty) => {
            consume(depth, 2, function, "shift")?;
            pop(encoder, function, Register::Ecx)?;
            pop(encoder, function, Register::Eax)?;
            let kind = match instruction {
                SystemsInst::ShiftLeft(_) => ShiftKind::LeftLogical,
                SystemsInst::ShiftRight(_) if ty.is_signed_integer() => ShiftKind::RightArithmetic,
                SystemsInst::ShiftRight(_) => ShiftKind::RightLogical,
                _ => unreachable!(),
            };
            emit(encoder, function, |encoder| {
                encoder.shift_reg_by_cl(Register::Eax, kind)
            })?;
            normalize(encoder, function, Register::Eax, ty)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::Cast { to, .. } => {
            consume(depth, 1, function, "cast")?;
            pop(encoder, function, Register::Eax)?;
            normalize(encoder, function, Register::Eax, to)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::BoolNot => {
            consume(depth, 1, function, "not.bool")?;
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, |encoder| {
                encoder.test_reg_reg(Register::Eax, Register::Eax)
            })?;
            emit(encoder, function, |encoder| {
                encoder.setcc_reg8(Condition::Equal, Register::Eax)
            })?;
            emit(encoder, function, |encoder| {
                encoder.movzx_reg_reg8(Register::Eax, Register::Eax)
            })?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::Compare { op, operand_type } => {
            consume(depth, 2, function, "compare")?;
            pop(encoder, function, Register::Ecx)?;
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, |encoder| {
                encoder.cmp_reg_reg(Register::Eax, Register::Ecx)
            })?;
            let condition = comparison_condition(*op, operand_type);
            emit(encoder, function, |encoder| {
                encoder.setcc_reg8(condition, Register::Eax)
            })?;
            emit(encoder, function, |encoder| {
                encoder.movzx_reg_reg8(Register::Eax, Register::Eax)
            })?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::IndexAddress { element_size } => {
            consume(depth, 2, function, "address.index")?;
            pop(encoder, function, Register::Ecx)?;
            pop(encoder, function, Register::Eax)?;
            if let Some(scale) = scale_for(*element_size) {
                emit(encoder, function, |encoder| {
                    encoder.lea_reg_from_memory(
                        Register::Eax,
                        MemoryOperand::indexed(Register::Eax, Register::Ecx, scale, 0),
                    )
                })?;
            } else {
                emit(encoder, function, |encoder| {
                    encoder.mov_reg_imm32(Register::Edx, *element_size)
                })?;
                emit(encoder, function, |encoder| {
                    encoder.imul_reg_reg(Register::Ecx, Register::Edx)
                })?;
                emit(encoder, function, |encoder| {
                    encoder.add_reg_reg(Register::Eax, Register::Ecx)
                })?;
            }
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::FieldAddress { offset } => {
            consume(depth, 1, function, "address.field")?;
            pop(encoder, function, Register::Eax)?;
            if *offset != 0 {
                let offset = i32::try_from(*offset).map_err(|_| {
                    function_error(function, "native field offset exceeds signed i32")
                })?;
                emit(encoder, function, |encoder| {
                    encoder.add_reg_imm32(Register::Eax, offset)
                })?;
            }
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::Label(id) => {
            let label = ir_label(labels, *id, function)?;
            encoder
                .bind_label(label)
                .map_err(|error| encoder_error(error, Some(function)))?;
        }
        SystemsInst::Jump(id) => {
            let label = ir_label(labels, *id, function)?;
            emit(encoder, function, |encoder| encoder.jump(label))?;
        }
        SystemsInst::JumpIfFalse(id) => {
            consume(depth, 1, function, "jump_if_false")?;
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, |encoder| {
                encoder.test_reg_reg(Register::Eax, Register::Eax)
            })?;
            let label = ir_label(labels, *id, function)?;
            emit(encoder, function, |encoder| {
                encoder.jump_if(Condition::Equal, label)
            })?;
        }
        SystemsInst::Call {
            symbol,
            argument_count,
            call_convention,
            ..
        } => {
            let depth_before = *depth;
            consume(depth, *argument_count, function, "call")?;
            let padding = if *call_convention == dbyte_ast::CallConvention::C {
                let saved = if function.call_convention == dbyte_ast::CallConvention::C {
                    12usize
                } else {
                    0
                };
                (16 - ((frame.size as usize + saved + depth_before * 4) & 15)) & 15
            } else {
                0
            };
            if padding != 0 {
                emit(encoder, function, |encoder| {
                    encoder.sub_esp_imm32(padding as u32)
                })?;
                for index in 0..*argument_count {
                    let destination = i32::try_from(index * 4).map_err(|_| {
                        function_error(function, "C ABI argument destination exceeds i32")
                    })?;
                    let source = i32::try_from(padding + index * 4).map_err(|_| {
                        function_error(function, "C ABI argument source exceeds i32")
                    })?;
                    emit(encoder, function, |encoder| {
                        encoder.mov_reg_from_memory(
                            Register::Eax,
                            MemoryOperand::new(Register::Esp, source),
                            OperandWidth::Dword,
                            None,
                        )
                    })?;
                    emit(encoder, function, |encoder| {
                        encoder.mov_memory_from_reg(
                            MemoryOperand::new(Register::Esp, destination),
                            Register::Eax,
                            OperandWidth::Dword,
                        )
                    })?;
                }
            }
            let symbol_id = lookup_symbol(symbols, symbol)?;
            let immediate = encoder
                .call_rel32_placeholder()
                .map_err(|error| encoder_error(error, Some(function)))?;
            relocations.push(ObjectRelocation {
                section: ObjectSection::Text,
                offset: checked_u32(immediate, "call relocation offset")?,
                symbol: symbol_id,
                kind: RelocationKind::Pc32,
                addend: -4,
            });
            if *argument_count != 0 || padding != 0 {
                let cleanup = argument_count
                    .checked_mul(4)
                    .and_then(|cleanup| cleanup.checked_add(padding))
                    .ok_or_else(|| function_error(function, "native caller cleanup overflow"))?;
                emit(encoder, function, |encoder| {
                    encoder.add_esp_imm32(
                        u32::try_from(cleanup)
                            .map_err(|_| EncoderError::OffsetOutOfRange(cleanup))?,
                    )
                })?;
            }
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::CallPacked {
            symbol,
            argument_bytes,
            call_convention,
            ..
        } => {
            if *call_convention == dbyte_ast::CallConvention::C {
                return Err(function_error(
                    function,
                    "packed aggregate calls are unavailable for the C ABI",
                ));
            }
            consume(depth, 1, function, "call.packed")?;
            pop(encoder, function, Register::Esi)?;
            if *argument_bytes != 0 {
                emit(encoder, function, |encoder| {
                    encoder.sub_esp_imm32(*argument_bytes)
                })?;
                emit(encoder, function, |encoder| {
                    encoder.mov_reg_reg(Register::Edi, Register::Esp)
                })?;
                emit(encoder, function, |encoder| {
                    encoder.mov_reg_imm32(Register::Ecx, *argument_bytes)
                })?;
                emit(encoder, function, Encoder::rep_movsb)?;
            }
            let symbol_id = lookup_symbol(symbols, symbol)?;
            let immediate = encoder
                .call_rel32_placeholder()
                .map_err(|error| encoder_error(error, Some(function)))?;
            relocations.push(ObjectRelocation {
                section: ObjectSection::Text,
                offset: checked_u32(immediate, "packed call relocation offset")?,
                symbol: symbol_id,
                kind: RelocationKind::Pc32,
                addend: -4,
            });
            if *argument_bytes != 0 {
                emit(encoder, function, |encoder| {
                    encoder.add_esp_imm32(*argument_bytes)
                })?;
            }
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::CallIndirect {
            argument_count,
            call_convention,
            ..
        } => {
            let depth_before = *depth;
            consume(depth, argument_count + 1, function, "call.indirect")?;
            let padding = if *call_convention == dbyte_ast::CallConvention::C {
                let saved = if function.call_convention == dbyte_ast::CallConvention::C {
                    12usize
                } else {
                    0
                };
                (16 - ((frame.size as usize + saved + depth_before * 4) & 15)) & 15
            } else {
                0
            };
            if padding != 0 {
                emit(encoder, function, |encoder| {
                    encoder.sub_esp_imm32(padding as u32)
                })?;
                for index in 0..*argument_count {
                    let destination = i32::try_from(index * 4).map_err(|_| {
                        function_error(function, "indirect C argument destination exceeds i32")
                    })?;
                    let source = i32::try_from(padding + index * 4).map_err(|_| {
                        function_error(function, "indirect C argument source exceeds i32")
                    })?;
                    emit(encoder, function, |encoder| {
                        encoder.mov_reg_from_memory(
                            Register::Eax,
                            MemoryOperand::new(Register::Esp, source),
                            OperandWidth::Dword,
                            None,
                        )
                    })?;
                    emit(encoder, function, |encoder| {
                        encoder.mov_memory_from_reg(
                            MemoryOperand::new(Register::Esp, destination),
                            Register::Eax,
                            OperandWidth::Dword,
                        )
                    })?;
                }
            }
            let displacement = i32::try_from(
                padding
                    + argument_count
                        .checked_mul(4)
                        .ok_or_else(|| function_error(function, "indirect call offset overflow"))?,
            )
            .map_err(|_| function_error(function, "indirect call offset exceeds i32"))?;
            emit(encoder, function, |encoder| {
                encoder.mov_reg_from_memory(
                    Register::Eax,
                    MemoryOperand::new(Register::Esp, displacement),
                    OperandWidth::Dword,
                    None,
                )
            })?;
            emit(encoder, function, |encoder| encoder.call_reg(Register::Eax))?;
            let cleanup = u32::try_from(
                (argument_count + 1)
                    .checked_mul(4)
                    .and_then(|bytes| bytes.checked_add(padding))
                    .ok_or_else(|| function_error(function, "indirect cleanup overflow"))?,
            )
            .map_err(|_| function_error(function, "indirect cleanup exceeds u32"))?;
            emit(encoder, function, |encoder| encoder.add_esp_imm32(cleanup))?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::InterruptAddress { symbol, .. } => {
            emit_absolute_address(encoder, function, symbols, relocations, symbol)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::TaskAddress { symbol } => {
            emit_absolute_address(encoder, function, symbols, relocations, symbol)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::LinkerAddress { symbol } => {
            emit_absolute_address(encoder, function, symbols, relocations, symbol)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemsInst::ReloadSegments {
            code_selector,
            data_selector,
        } => {
            emit(encoder, function, |encoder| {
                encoder.mov_ax_imm16(*data_selector)
            })?;
            emit(encoder, function, Encoder::load_data_segments_from_ax)?;
            let immediate = encoder
                .far_jump_imm32_placeholder(*code_selector)
                .map_err(|error| encoder_error(error, Some(function)))?;
            let destination = checked_u32(encoder.position(), "far-jump destination")?;
            relocations.push(ObjectRelocation {
                section: ObjectSection::Text,
                offset: checked_u32(immediate, "far-jump relocation offset")?,
                symbol: lookup_symbol(symbols, ".text")?,
                kind: RelocationKind::Abs32,
                addend: i32::try_from(destination).map_err(|_| {
                    function_error(function, "far-jump destination exceeds signed REL addend")
                })?,
            });
        }
        SystemsInst::Intrinsic {
            intrinsic,
            argument_count,
            result,
        } => lower_intrinsic(
            *intrinsic,
            *argument_count,
            result.as_ref(),
            function,
            encoder,
            depth,
        )?,
        SystemsInst::Pop => {
            consume(depth, 1, function, "pop")?;
            pop(encoder, function, Register::Eax)?;
        }
        SystemsInst::Return => {
            consume(depth, 1, function, "return")?;
            pop(encoder, function, Register::Eax)?;
            if function.call_convention == dbyte_ast::CallConvention::C {
                emit(encoder, function, |encoder| encoder.pop_reg(Register::Edi))?;
                emit(encoder, function, |encoder| encoder.pop_reg(Register::Esi))?;
                emit(encoder, function, |encoder| encoder.pop_reg(Register::Ebx))?;
            }
            emit(encoder, function, |encoder| {
                encoder.mov_reg_reg(Register::Esp, Register::Ebp)
            })?;
            emit(encoder, function, |encoder| encoder.pop_reg(Register::Ebp))?;
            emit(encoder, function, |encoder| encoder.ret())?;
            *returns += 1;
        }
    }
    Ok(())
}

fn lower_intrinsic(
    intrinsic: SystemIntrinsic,
    argument_count: usize,
    result: Option<&SystemType>,
    function: &SystemsFunction,
    encoder: &mut Encoder,
    depth: &mut usize,
) -> NativeResult<()> {
    consume(depth, argument_count, function, "intrinsic")?;
    match intrinsic {
        SystemIntrinsic::VolatileLoad8
        | SystemIntrinsic::VolatileLoad16
        | SystemIntrinsic::VolatileLoad32 => {
            require_intrinsic_arity(intrinsic, argument_count, 1, function)?;
            pop(encoder, function, Register::Edx)?;
            let ty = result.ok_or_else(|| {
                function_error(
                    function,
                    format!("intrinsic {intrinsic} has no result type"),
                )
            })?;
            load_memory(
                encoder,
                function,
                Register::Eax,
                MemoryOperand::new(Register::Edx, 0),
                ty,
            )?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemIntrinsic::VolatileStore8
        | SystemIntrinsic::VolatileStore16
        | SystemIntrinsic::VolatileStore32 => {
            require_intrinsic_arity(intrinsic, argument_count, 2, function)?;
            pop(encoder, function, Register::Eax)?;
            pop(encoder, function, Register::Edx)?;
            let ty = match intrinsic {
                SystemIntrinsic::VolatileStore8 => SystemType::U8,
                SystemIntrinsic::VolatileStore16 => SystemType::U16,
                SystemIntrinsic::VolatileStore32 => SystemType::U32,
                _ => unreachable!(),
            };
            store_memory(
                encoder,
                function,
                MemoryOperand::new(Register::Edx, 0),
                Register::Eax,
                &ty,
            )?;
        }
        SystemIntrinsic::IoOut8 | SystemIntrinsic::IoOut16 => {
            require_intrinsic_arity(intrinsic, argument_count, 2, function)?;
            pop(encoder, function, Register::Eax)?;
            pop(encoder, function, Register::Edx)?;
            match intrinsic {
                SystemIntrinsic::IoOut8 => emit(encoder, function, |encoder| encoder.out_dx_al())?,
                SystemIntrinsic::IoOut16 => emit(encoder, function, |encoder| encoder.out_dx_ax())?,
                _ => unreachable!(),
            }
        }
        SystemIntrinsic::IoIn8 | SystemIntrinsic::IoIn16 => {
            require_intrinsic_arity(intrinsic, argument_count, 1, function)?;
            pop(encoder, function, Register::Edx)?;
            match intrinsic {
                SystemIntrinsic::IoIn8 => {
                    emit(encoder, function, |encoder| encoder.in_al_dx())?;
                    emit(encoder, function, |encoder| {
                        encoder.movzx_reg_reg(Register::Eax, Register::Eax, OperandWidth::Byte)
                    })?;
                }
                SystemIntrinsic::IoIn16 => {
                    emit(encoder, function, |encoder| encoder.in_ax_dx())?;
                    emit(encoder, function, |encoder| {
                        encoder.movzx_reg_reg(Register::Eax, Register::Eax, OperandWidth::Word)
                    })?;
                }
                _ => unreachable!(),
            }
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemIntrinsic::CpuHlt => {
            require_intrinsic_arity(intrinsic, argument_count, 0, function)?;
            emit(encoder, function, |encoder| encoder.hlt())?;
        }
        SystemIntrinsic::CpuCli => {
            require_intrinsic_arity(intrinsic, argument_count, 0, function)?;
            emit(encoder, function, Encoder::cli)?;
        }
        SystemIntrinsic::CpuSti => {
            require_intrinsic_arity(intrinsic, argument_count, 0, function)?;
            emit(encoder, function, Encoder::sti)?;
        }
        SystemIntrinsic::CpuInt3 => {
            require_intrinsic_arity(intrinsic, argument_count, 0, function)?;
            emit(encoder, function, Encoder::int3)?;
        }
        SystemIntrinsic::CpuReadEflags => {
            require_intrinsic_arity(intrinsic, argument_count, 0, function)?;
            emit(encoder, function, Encoder::pushfd)?;
            pop(encoder, function, Register::Eax)?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemIntrinsic::CpuReadCr0 | SystemIntrinsic::CpuReadCr2 | SystemIntrinsic::CpuReadCr3 => {
            require_intrinsic_arity(intrinsic, argument_count, 0, function)?;
            match intrinsic {
                SystemIntrinsic::CpuReadCr0 => emit(encoder, function, Encoder::mov_eax_cr0)?,
                SystemIntrinsic::CpuReadCr2 => emit(encoder, function, Encoder::mov_eax_cr2)?,
                SystemIntrinsic::CpuReadCr3 => emit(encoder, function, Encoder::mov_eax_cr3)?,
                _ => unreachable!(),
            }
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemIntrinsic::CpuWriteCr0 | SystemIntrinsic::CpuWriteCr3 => {
            require_intrinsic_arity(intrinsic, argument_count, 1, function)?;
            pop(encoder, function, Register::Eax)?;
            match intrinsic {
                SystemIntrinsic::CpuWriteCr0 => emit(encoder, function, Encoder::mov_cr0_eax)?,
                SystemIntrinsic::CpuWriteCr3 => emit(encoder, function, Encoder::mov_cr3_eax)?,
                _ => unreachable!(),
            }
        }
        SystemIntrinsic::CpuInvlpg => {
            require_intrinsic_arity(intrinsic, argument_count, 1, function)?;
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, |encoder| {
                encoder.invlpg(MemoryOperand::new(Register::Eax, 0))
            })?;
        }
        SystemIntrinsic::CpuLgdt | SystemIntrinsic::CpuLidt => {
            require_intrinsic_arity(intrinsic, argument_count, 1, function)?;
            pop(encoder, function, Register::Eax)?;
            match intrinsic {
                SystemIntrinsic::CpuLgdt => emit(encoder, function, |encoder| {
                    encoder.lgdt(MemoryOperand::new(Register::Eax, 0))
                })?,
                SystemIntrinsic::CpuLidt => emit(encoder, function, |encoder| {
                    encoder.lidt(MemoryOperand::new(Register::Eax, 0))
                })?,
                _ => unreachable!(),
            }
        }
        SystemIntrinsic::CpuLtr => {
            require_intrinsic_arity(intrinsic, argument_count, 1, function)?;
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, Encoder::ltr_ax)?;
        }
        SystemIntrinsic::TaskBlockTrap => {
            require_intrinsic_arity(intrinsic, argument_count, 2, function)?;
            pop(encoder, function, Register::Edx)?;
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, |encoder| {
                encoder.software_interrupt(0x81)
            })?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
        SystemIntrinsic::InterruptResume => {
            require_intrinsic_arity(intrinsic, argument_count, 1, function)?;
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, |encoder| {
                encoder.mov_reg_reg(Register::Esp, Register::Eax)
            })?;
            emit(encoder, function, Encoder::popad)?;
            emit(encoder, function, Encoder::iretd)?;
        }
        SystemIntrinsic::UserSyscall0
        | SystemIntrinsic::UserSyscall1
        | SystemIntrinsic::UserSyscall2
        | SystemIntrinsic::UserSyscall3 => {
            let expected = match intrinsic {
                SystemIntrinsic::UserSyscall0 => 1,
                SystemIntrinsic::UserSyscall1 => 2,
                SystemIntrinsic::UserSyscall2 => 3,
                SystemIntrinsic::UserSyscall3 => 4,
                _ => unreachable!(),
            };
            require_intrinsic_arity(intrinsic, argument_count, expected, function)?;
            if expected >= 4 {
                pop(encoder, function, Register::Edx)?;
            }
            if expected >= 3 {
                pop(encoder, function, Register::Ecx)?;
            }
            if expected >= 2 {
                pop(encoder, function, Register::Ebx)?;
            }
            pop(encoder, function, Register::Eax)?;
            emit(encoder, function, |encoder| {
                encoder.software_interrupt(0x80)
            })?;
            push(encoder, function, Register::Eax)?;
            *depth += 1;
        }
    }
    Ok(())
}

fn require_intrinsic_arity(
    intrinsic: SystemIntrinsic,
    actual: usize,
    expected: usize,
    function: &SystemsFunction,
) -> NativeResult<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(function_error(
            function,
            format!("intrinsic {intrinsic} expects {expected} arguments, IR supplied {actual}"),
        ))
    }
}

fn emit_absolute_address(
    encoder: &mut Encoder,
    function: &SystemsFunction,
    symbols: &BTreeMap<String, SymbolId>,
    relocations: &mut Vec<ObjectRelocation>,
    symbol: &str,
) -> NativeResult<()> {
    let immediate = encoder
        .mov_reg_imm32_placeholder(Register::Eax)
        .map_err(|error| encoder_error(error, Some(function)))?;
    relocations.push(ObjectRelocation {
        section: ObjectSection::Text,
        offset: checked_u32(immediate, "absolute relocation offset")?,
        symbol: lookup_symbol(symbols, symbol)?,
        kind: RelocationKind::Abs32,
        addend: 0,
    });
    Ok(())
}

fn load_memory(
    encoder: &mut Encoder,
    function: &SystemsFunction,
    destination: Register,
    source: MemoryOperand,
    ty: &SystemType,
) -> NativeResult<()> {
    let (width, extension) = memory_load_form(ty).ok_or_else(|| {
        function_error(
            function,
            format!("cannot load aggregate native type '{ty}'"),
        )
    })?;
    emit(encoder, function, |encoder| {
        encoder.mov_reg_from_memory(destination, source, width, extension)
    })
}

fn store_memory(
    encoder: &mut Encoder,
    function: &SystemsFunction,
    destination: MemoryOperand,
    source: Register,
    ty: &SystemType,
) -> NativeResult<()> {
    let width = operand_width(ty).ok_or_else(|| {
        function_error(
            function,
            format!("cannot store aggregate native type '{ty}'"),
        )
    })?;
    emit(encoder, function, |encoder| {
        encoder.mov_memory_from_reg(destination, source, width)
    })
}

fn normalize(
    encoder: &mut Encoder,
    function: &SystemsFunction,
    register: Register,
    ty: &SystemType,
) -> NativeResult<()> {
    match ty {
        SystemType::Bool => {
            emit(encoder, function, |encoder| {
                encoder.mov_reg_imm32(Register::Edx, 1)
            })?;
            emit(encoder, function, |encoder| {
                encoder.and_reg_reg(register, Register::Edx)
            })
        }
        SystemType::I8 => emit(encoder, function, |encoder| {
            encoder.movsx_reg_reg(register, register, OperandWidth::Byte)
        }),
        SystemType::U8 => emit(encoder, function, |encoder| {
            encoder.movzx_reg_reg(register, register, OperandWidth::Byte)
        }),
        SystemType::I16 => emit(encoder, function, |encoder| {
            encoder.movsx_reg_reg(register, register, OperandWidth::Word)
        }),
        SystemType::U16 => emit(encoder, function, |encoder| {
            encoder.movzx_reg_reg(register, register, OperandWidth::Word)
        }),
        SystemType::I32
        | SystemType::U32
        | SystemType::Usize
        | SystemType::Str
        | SystemType::Pointer(_) => Ok(()),
        SystemType::Array(_, _) | SystemType::Struct(_) => Err(function_error(
            function,
            format!("cannot normalize aggregate native type '{ty}'"),
        )),
    }
}

fn operand_width(ty: &SystemType) -> Option<OperandWidth> {
    match ty {
        SystemType::Bool | SystemType::I8 | SystemType::U8 => Some(OperandWidth::Byte),
        SystemType::I16 | SystemType::U16 => Some(OperandWidth::Word),
        SystemType::I32
        | SystemType::U32
        | SystemType::Usize
        | SystemType::Str
        | SystemType::Pointer(_) => Some(OperandWidth::Dword),
        SystemType::Array(_, _) | SystemType::Struct(_) => None,
    }
}

fn memory_load_form(ty: &SystemType) -> Option<(OperandWidth, Option<Extension>)> {
    match ty {
        SystemType::Bool | SystemType::U8 => Some((OperandWidth::Byte, Some(Extension::Zero))),
        SystemType::I8 => Some((OperandWidth::Byte, Some(Extension::Sign))),
        SystemType::U16 => Some((OperandWidth::Word, Some(Extension::Zero))),
        SystemType::I16 => Some((OperandWidth::Word, Some(Extension::Sign))),
        SystemType::I32
        | SystemType::U32
        | SystemType::Usize
        | SystemType::Str
        | SystemType::Pointer(_) => Some((OperandWidth::Dword, None)),
        SystemType::Array(_, _) | SystemType::Struct(_) => None,
    }
}

fn comparison_condition(operator: CompareOp, ty: &SystemType) -> Condition {
    let signed = ty.is_signed_integer();
    match operator {
        CompareOp::Eq => Condition::Equal,
        CompareOp::NotEq => Condition::NotEqual,
        CompareOp::Lt if signed => Condition::Less,
        CompareOp::Lt => Condition::Below,
        CompareOp::LtEq if signed => Condition::LessOrEqual,
        CompareOp::LtEq => Condition::BelowOrEqual,
        CompareOp::Gt if signed => Condition::Greater,
        CompareOp::Gt => Condition::Above,
        CompareOp::GtEq if signed => Condition::GreaterOrEqual,
        CompareOp::GtEq => Condition::AboveOrEqual,
    }
}

fn scale_for(size: u32) -> Option<Scale> {
    match size {
        1 => Some(Scale::One),
        2 => Some(Scale::Two),
        4 => Some(Scale::Four),
        8 => Some(Scale::Eight),
        _ => None,
    }
}

#[derive(Debug)]
struct FrameLayout {
    size: u32,
    offsets: Vec<i32>,
}

impl FrameLayout {
    fn build(slots: &[SystemsSlot]) -> NativeResult<Self> {
        let mut cursor = 0u32;
        let mut offsets = Vec::with_capacity(slots.len());
        for (index, slot) in slots.iter().enumerate() {
            if slot.id.index() != index {
                return Err(backend_error(
                    DiagnosticCategory::Encoder,
                    format!("native slot ids are not dense at index {index}"),
                ));
            }
            let alignment = slot.alignment.max(if slot.ty.is_scalar() { 4 } else { 1 });
            cursor = align_u32(cursor, alignment, "native stack slot")?;
            cursor = cursor.checked_add(slot.storage_size).ok_or_else(|| {
                backend_error(
                    DiagnosticCategory::LayoutOverflow,
                    "native stack frame overflow",
                )
            })?;
            let offset = -i32::try_from(cursor).map_err(|_| {
                backend_error(
                    DiagnosticCategory::LayoutOverflow,
                    "native stack slot offset exceeds i32",
                )
            })?;
            offsets.push(offset);
        }
        let size = align_u32(cursor, 4, "native stack frame")?;
        if size > MAX_FRAME_SIZE {
            return Err(backend_error(
                DiagnosticCategory::LayoutOverflow,
                format!("native stack frame size {size} exceeds bound {MAX_FRAME_SIZE}"),
            ));
        }
        Ok(Self { size, offsets })
    }

    fn offset(&self, local: LocalId, function: &SystemsFunction) -> NativeResult<i32> {
        self.offsets
            .get(local.index())
            .copied()
            .ok_or_else(|| function_error(function, format!("unknown native local slot {local}")))
    }
}

fn validate_module(module: &SystemsModuleIr) -> NativeResult<()> {
    let mut names = BTreeSet::new();
    for function in &module.functions {
        if !names.insert(function.symbol.as_str()) {
            return Err(backend_error(
                DiagnosticCategory::DuplicateSymbol,
                format!("duplicate object symbol '{}'", function.symbol),
            ));
        }
        if !function.return_type.is_scalar() {
            return Err(function_error(
                function,
                "DBYTE32 object functions must return a scalar",
            ));
        }
        if function.call_convention == dbyte_ast::CallConvention::C
            && function.parameter_types.iter().any(|ty| !ty.is_scalar())
        {
            return Err(function_error(
                function,
                "i686 C object function parameters must be scalar",
            ));
        }
        validate_function_storage(function)?;
    }
    for global in &module.globals {
        if !names.insert(global.symbol.as_str()) {
            return Err(backend_error(
                DiagnosticCategory::DuplicateSymbol,
                format!("duplicate object symbol '{}'", global.symbol),
            ));
        }
        if global.section == GlobalSection::Bss && !global.bytes.is_empty() {
            return Err(backend_error(
                DiagnosticCategory::InvalidObject,
                format!("bss global '{}' unexpectedly contains bytes", global.symbol),
            ));
        }
        if global.section != GlobalSection::Bss && global.bytes.len() != global.size as usize {
            return Err(backend_error(
                DiagnosticCategory::InvalidObject,
                format!("initialized global '{}' byte size mismatch", global.symbol),
            ));
        }
    }
    Ok(())
}

fn validate_function_storage(function: &SystemsFunction) -> NativeResult<()> {
    if function.params.len() != function.parameter_types.len() {
        return Err(function_error(
            function,
            format!(
                "DBYTE32 parameter metadata count mismatch: signature has {}, slots have {}",
                function.parameter_types.len(),
                function.params.len()
            ),
        ));
    }

    let mut parameter_slots = BTreeSet::new();
    for (index, (parameter, expected)) in function
        .params
        .iter()
        .zip(&function.parameter_types)
        .enumerate()
    {
        if parameter.ty != *expected {
            return Err(function_error(
                function,
                format!(
                    "DBYTE32 parameter {} type mismatch: signature has {expected}, metadata has {}",
                    index + 1,
                    parameter.ty
                ),
            ));
        }
        if !parameter_slots.insert(parameter.slot) {
            return Err(function_error(
                function,
                format!("DBYTE32 parameter slot {} is reused", parameter.slot),
            ));
        }
        let slot = function.slots.get(parameter.slot.index()).ok_or_else(|| {
            function_error(
                function,
                format!(
                    "DBYTE32 parameter '{}' references unknown slot {}",
                    parameter.name, parameter.slot
                ),
            )
        })?;
        if !slot.is_parameter || slot.ty != parameter.ty {
            return Err(function_error(
                function,
                format!(
                    "DBYTE32 parameter '{}' does not match its frame slot",
                    parameter.name
                ),
            ));
        }
    }

    for (index, slot) in function.slots.iter().enumerate() {
        if slot.id.index() != index {
            return Err(function_error(
                function,
                format!("native slot ids are not dense at index {index}"),
            ));
        }
        if slot.is_parameter != parameter_slots.contains(&slot.id) {
            return Err(function_error(
                function,
                format!(
                    "native slot {} has inconsistent parameter ownership",
                    slot.id
                ),
            ));
        }
        if let Some(expected_size) = slot.ty.scalar_size() {
            if slot.storage_size != expected_size {
                return Err(function_error(
                    function,
                    format!(
                        "native scalar slot {} for {} requires exact size {expected_size}, found {}",
                        slot.id, slot.ty, slot.storage_size
                    ),
                ));
            }
        }
    }

    for instruction in &function.instructions {
        let (local, ty) = match instruction {
            SystemsInst::LocalLoad { local, ty } | SystemsInst::LocalStore { local, ty } => {
                (*local, ty)
            }
            _ => continue,
        };
        let slot = function.slots.get(local.index()).ok_or_else(|| {
            function_error(function, format!("unknown native local slot {local}"))
        })?;
        if slot.ty != *ty {
            return Err(function_error(
                function,
                format!(
                    "typed local access to {local} uses {ty}, but its frame slot is {}",
                    slot.ty
                ),
            ));
        }
    }
    Ok(())
}

fn object_symbol(
    global: &crate::systems_ir::SystemsGlobal,
    binding: SymbolBinding,
    section: ObjectSection,
    value: u32,
) -> ObjectSymbol {
    ObjectSymbol {
        name: global.symbol.clone(),
        binding,
        kind: SymbolKind::Object,
        definition: SymbolDefinition::Defined { section, value },
        size: global.size,
    }
}

fn push_symbol(
    symbols: &mut Vec<ObjectSymbol>,
    named: Option<&mut BTreeMap<String, SymbolId>>,
    symbol: ObjectSymbol,
) -> NativeResult<SymbolId> {
    let id = SymbolId(u32::try_from(symbols.len()).map_err(|_| {
        backend_error(
            DiagnosticCategory::LayoutOverflow,
            "object symbol count exceeds u32",
        )
    })?);
    if let Some(named) = named {
        if named.insert(symbol.name.clone(), id).is_some() {
            return Err(backend_error(
                DiagnosticCategory::DuplicateSymbol,
                format!("duplicate object symbol '{}'", symbol.name),
            ));
        }
    }
    symbols.push(symbol);
    Ok(id)
}

fn insert_named_symbol(
    symbols: &mut Vec<ObjectSymbol>,
    named: &mut BTreeMap<String, SymbolId>,
    symbol: ObjectSymbol,
) -> NativeResult<SymbolId> {
    push_symbol(symbols, Some(named), symbol)
}

fn lookup_symbol(symbols: &BTreeMap<String, SymbolId>, name: &str) -> NativeResult<SymbolId> {
    symbols.get(name).copied().ok_or_else(|| {
        backend_error(
            DiagnosticCategory::UndefinedSymbol,
            format!("object relocation references unknown symbol '{name}'"),
        )
    })
}

fn append_aligned(
    destination: &mut Vec<u8>,
    bytes: &[u8],
    declared_size: u32,
    alignment: u32,
    context: &str,
) -> NativeResult<u32> {
    if bytes.len() != declared_size as usize {
        return Err(backend_error(
            DiagnosticCategory::InvalidObject,
            format!("{context} byte count does not match declared size"),
        ));
    }
    let offset = align_u32(
        u32::try_from(destination.len()).map_err(|_| {
            backend_error(
                DiagnosticCategory::LayoutOverflow,
                format!("{context} offset overflow"),
            )
        })?,
        alignment,
        context,
    )?;
    destination.resize(offset as usize, 0);
    destination.extend_from_slice(bytes);
    Ok(offset)
}

fn align_u32(value: u32, alignment: u32, context: &str) -> NativeResult<u32> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(backend_error(
            DiagnosticCategory::LayoutOverflow,
            format!("{context} has invalid alignment {alignment}"),
        ));
    }
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| {
            backend_error(
                DiagnosticCategory::LayoutOverflow,
                format!("{context} alignment overflow"),
            )
        })
}

fn consume(
    depth: &mut usize,
    count: usize,
    function: &SystemsFunction,
    operation: &str,
) -> NativeResult<()> {
    if *depth < count {
        return Err(function_error(
            function,
            format!(
                "systems IR stack underflow in {operation}: need {count}, have {}",
                *depth
            ),
        ));
    }
    *depth -= count;
    Ok(())
}

fn push(encoder: &mut Encoder, function: &SystemsFunction, register: Register) -> NativeResult<()> {
    emit(encoder, function, |encoder| encoder.push_reg(register))
}

fn pop(encoder: &mut Encoder, function: &SystemsFunction, register: Register) -> NativeResult<()> {
    emit(encoder, function, |encoder| encoder.pop_reg(register))
}

fn emit(
    encoder: &mut Encoder,
    function: &SystemsFunction,
    operation: impl FnOnce(&mut Encoder) -> Result<(), EncoderError>,
) -> NativeResult<()> {
    operation(encoder).map_err(|error| encoder_error(error, Some(function)))
}

fn ir_label(
    labels: &BTreeMap<LabelId, Label>,
    id: LabelId,
    function: &SystemsFunction,
) -> NativeResult<Label> {
    labels
        .get(&id)
        .copied()
        .ok_or_else(|| function_error(function, format!("unknown systems IR label {id}")))
}

fn string_symbol(index: usize) -> String {
    format!(".Lstr{index}")
}

fn checked_u32(value: usize, context: &str) -> NativeResult<u32> {
    u32::try_from(value).map_err(|_| {
        backend_error(
            DiagnosticCategory::LayoutOverflow,
            format!("{context} exceeds u32"),
        )
    })
}

fn function_error(function: &SystemsFunction, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(DiagnosticCategory::Encoder, message, function.span)
}

fn encoder_error(error: EncoderError, function: Option<&SystemsFunction>) -> Diagnostic {
    match function {
        Some(function) => Diagnostic::new(
            DiagnosticCategory::Encoder,
            error.to_string(),
            function.span,
        ),
        None => Diagnostic::without_span(DiagnosticCategory::Encoder, error.to_string()),
    }
}

fn backend_error(category: DiagnosticCategory, message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(category, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems_ir::{ExternalFunction, ExternalGlobal, SystemsParam};
    use dbyte_ast::{CallConvention, Span};

    fn function(instructions: Vec<SystemsInst>) -> SystemsFunction {
        SystemsFunction {
            source_name: "kernel_main".into(),
            symbol: "kernel_main".into(),
            visibility: Visibility::Private,
            interrupt: None,
            call_convention: CallConvention::DByte,
            parameter_types: vec![],
            params: vec![],
            return_type: SystemType::I32,
            slots: vec![],
            instructions,
            span: Span::new(1, 1),
        }
    }

    fn module(functions: Vec<SystemsFunction>) -> SystemsModuleIr {
        SystemsModuleIr {
            user_profile: false,
            module_name: "kernel".into(),
            functions,
            globals: vec![],
            strings: vec![],
            external_functions: vec![],
            external_globals: vec![],
        }
    }

    #[test]
    fn emits_abs32_and_pc32_relocations_separately() {
        let mut ir = module(vec![function(vec![
            SystemsInst::GlobalAddress {
                symbol: "external_data".into(),
                pointee: SystemType::U32,
            },
            SystemsInst::Pop,
            SystemsInst::Call {
                symbol: "external_fn".into(),
                argument_count: 0,
                result: SystemType::I32,
                call_convention: CallConvention::DByte,
            },
            SystemsInst::Return,
        ])]);
        ir.external_functions.push(ExternalFunction {
            symbol: "external_fn".into(),
            parameter_types: vec![],
            return_type: SystemType::I32,
            call_convention: CallConvention::DByte,
        });
        ir.external_globals.push(ExternalGlobal {
            symbol: "external_data".into(),
            ty: SystemType::U32,
            mutable: true,
        });
        let lowered = lower_systems_module(&ir).unwrap();
        assert_eq!(lowered.image.relocations.len(), 2);
        assert!(lowered
            .image
            .relocations
            .iter()
            .any(|relocation| relocation.kind == RelocationKind::Abs32));
        let call = lowered
            .image
            .relocations
            .iter()
            .find(|relocation| relocation.kind == RelocationKind::Pc32)
            .unwrap();
        assert_eq!(call.addend, -4);
    }

    #[test]
    fn width_operations_and_aggregate_addressing_reach_machine_code() {
        let ir = module(vec![function(vec![
            SystemsInst::Const {
                bits: 0x1000,
                ty: SystemType::Usize,
            },
            SystemsInst::Const {
                bits: 2,
                ty: SystemType::Usize,
            },
            SystemsInst::IndexAddress { element_size: 2 },
            SystemsInst::Load(SystemType::U16),
            SystemsInst::Const {
                bits: 1,
                ty: SystemType::U16,
            },
            SystemsInst::ShiftRight(SystemType::U16),
            SystemsInst::Cast {
                from: SystemType::U16,
                to: SystemType::I32,
            },
            SystemsInst::Return,
        ])]);
        let lowered = lower_systems_module(&ir).unwrap();
        let text = match &lowered.image.sections[&ObjectSection::Text].contents {
            SectionContents::Bytes(bytes) => bytes,
            SectionContents::Uninitialized(_) => panic!("text must be file-backed"),
        };
        assert!(text.windows(2).any(|bytes| bytes == [0x0f, 0xb7]));
        assert!(text.windows(2).any(|bytes| bytes == [0xd3, 0xe8]));
    }

    #[test]
    fn ltr_and_user_syscall_register_abi_reach_exact_machine_bytes() {
        let ir = module(vec![function(vec![
            SystemsInst::Const {
                bits: 0x28,
                ty: SystemType::U16,
            },
            SystemsInst::Intrinsic {
                intrinsic: SystemIntrinsic::CpuLtr,
                argument_count: 1,
                result: None,
            },
            SystemsInst::Const {
                bits: 4,
                ty: SystemType::U32,
            },
            SystemsInst::Const {
                bits: 3,
                ty: SystemType::Usize,
            },
            SystemsInst::Const {
                bits: 0x401000,
                ty: SystemType::Usize,
            },
            SystemsInst::Const {
                bits: 512,
                ty: SystemType::Usize,
            },
            SystemsInst::Intrinsic {
                intrinsic: SystemIntrinsic::UserSyscall3,
                argument_count: 4,
                result: Some(SystemType::I32),
            },
            SystemsInst::Return,
        ])]);
        let lowered = lower_systems_module(&ir).unwrap();
        let SectionContents::Bytes(text) = &lowered.image.sections[&ObjectSection::Text].contents
        else {
            panic!("text must be file-backed")
        };
        assert!(text
            .windows(4)
            .any(|bytes| bytes == [0x58, 0x0f, 0x00, 0xd8]));
        assert!(text
            .windows(6)
            .any(|bytes| bytes == [0x5a, 0x59, 0x5b, 0x58, 0xcd, 0x80]));
    }

    #[test]
    fn dbyte32_parameter_offsets_and_frame_are_cross_object_safe() {
        let helper = SystemsFunction {
            source_name: "helper".into(),
            symbol: "helper".into(),
            visibility: Visibility::Public,
            interrupt: None,
            call_convention: CallConvention::DByte,
            parameter_types: vec![SystemType::U8, SystemType::U32],
            params: vec![
                SystemsParam {
                    name: "left".into(),
                    ty: SystemType::U8,
                    slot: LocalId(0),
                },
                SystemsParam {
                    name: "right".into(),
                    ty: SystemType::U32,
                    slot: LocalId(1),
                },
            ],
            return_type: SystemType::I32,
            slots: vec![
                SystemsSlot {
                    id: LocalId(0),
                    name: "left".into(),
                    ty: SystemType::U8,
                    storage_size: 1,
                    alignment: 1,
                    is_parameter: true,
                },
                SystemsSlot {
                    id: LocalId(1),
                    name: "right".into(),
                    ty: SystemType::U32,
                    storage_size: 4,
                    alignment: 4,
                    is_parameter: true,
                },
            ],
            instructions: vec![
                SystemsInst::Const {
                    bits: 0,
                    ty: SystemType::I32,
                },
                SystemsInst::Return,
            ],
            span: Span::new(1, 1),
        };
        let lowered = lower_systems_module(&module(vec![
            function(vec![
                SystemsInst::Const {
                    bits: 0,
                    ty: SystemType::I32,
                },
                SystemsInst::Return,
            ]),
            helper,
        ]))
        .unwrap();
        assert!(lowered.machine_dump.contains("params=2 slots=2"));
    }

    #[test]
    fn narrow_parameter_spills_use_exact_width_below_saved_ebp() {
        let narrow8 = narrow_parameter_function("narrow8", SystemType::U8, 1, 1);
        let narrow16 = narrow_parameter_function("narrow16", SystemType::U16, 2, 2);
        let lowered = lower_systems_module(&module(vec![narrow8, narrow16])).unwrap();

        let bytes8 = defined_function_bytes(&lowered, "narrow8");
        assert_parameter_spill(bytes8, &[0x88, 0x45, 0xff], "mov BYTE PTR [ebp-1], al");

        let bytes16 = defined_function_bytes(&lowered, "narrow16");
        assert_parameter_spill(
            bytes16,
            &[0x66, 0x89, 0x45, 0xfe],
            "mov WORD PTR [ebp-2], ax",
        );
    }

    #[test]
    fn narrow_slots_are_fully_contained_in_the_allocated_frame() {
        let slots = vec![
            SystemsSlot {
                id: LocalId(0),
                name: "byte".into(),
                ty: SystemType::U8,
                storage_size: 1,
                alignment: 1,
                is_parameter: false,
            },
            SystemsSlot {
                id: LocalId(1),
                name: "word".into(),
                ty: SystemType::U16,
                storage_size: 2,
                alignment: 2,
                is_parameter: false,
            },
        ];
        let frame = FrameLayout::build(&slots).unwrap();
        assert_eq!(frame.size, 8);
        assert_eq!(frame.offsets, [-1, -6]);
        for (slot, offset) in slots.iter().zip(&frame.offsets) {
            assert!(*offset >= -(frame.size as i32));
            assert!(*offset + slot.storage_size as i32 - 1 < 0);
        }
    }

    #[test]
    fn cross_object_call_with_narrow_parameters_links_with_safe_spills() {
        let callee = SystemsFunction {
            source_name: "narrow_mix".into(),
            symbol: "narrow_mix".into(),
            visibility: Visibility::Public,
            interrupt: None,
            call_convention: CallConvention::DByte,
            parameter_types: vec![SystemType::U16, SystemType::U8],
            params: vec![
                SystemsParam {
                    name: "word".into(),
                    ty: SystemType::U16,
                    slot: LocalId(0),
                },
                SystemsParam {
                    name: "byte".into(),
                    ty: SystemType::U8,
                    slot: LocalId(1),
                },
            ],
            return_type: SystemType::I32,
            slots: vec![
                SystemsSlot {
                    id: LocalId(0),
                    name: "word".into(),
                    ty: SystemType::U16,
                    storage_size: 2,
                    alignment: 2,
                    is_parameter: true,
                },
                SystemsSlot {
                    id: LocalId(1),
                    name: "byte".into(),
                    ty: SystemType::U8,
                    storage_size: 1,
                    alignment: 1,
                    is_parameter: true,
                },
            ],
            instructions: vec![
                SystemsInst::LocalLoad {
                    local: LocalId(0),
                    ty: SystemType::U16,
                },
                SystemsInst::Cast {
                    from: SystemType::U16,
                    to: SystemType::I32,
                },
                SystemsInst::Return,
            ],
            span: Span::new(1, 1),
        };
        let callee = lower_systems_module(&SystemsModuleIr {
            user_profile: false,
            module_name: "callee".into(),
            functions: vec![callee],
            globals: vec![],
            strings: vec![],
            external_functions: vec![],
            external_globals: vec![],
        })
        .unwrap();
        let callee_bytes = defined_function_bytes(&callee, "narrow_mix");
        assert!(callee_bytes
            .windows(4)
            .any(|bytes| bytes == [0x66, 0x89, 0x45, 0xfe]));
        assert!(callee_bytes
            .windows(3)
            .any(|bytes| bytes == [0x88, 0x45, 0xfb]));

        let mut caller = module(vec![function(vec![
            SystemsInst::Const {
                bits: 0x1234,
                ty: SystemType::U16,
            },
            SystemsInst::Const {
                bits: 0x56,
                ty: SystemType::U8,
            },
            SystemsInst::Call {
                symbol: "narrow_mix".into(),
                argument_count: 2,
                result: SystemType::I32,
                call_convention: CallConvention::DByte,
            },
            SystemsInst::Return,
        ])]);
        caller.external_functions.push(ExternalFunction {
            symbol: "narrow_mix".into(),
            parameter_types: vec![SystemType::U16, SystemType::U8],
            return_type: SystemType::I32,
            call_convention: CallConvention::DByte,
        });
        let caller = lower_systems_module(&caller).unwrap();
        let linked = crate::linker::link_images(&[
            ("callee.o".into(), callee.image),
            ("caller.o".into(), caller.image),
        ])
        .unwrap();
        assert!(linked.elf.starts_with(b"\x7fELF"));
    }

    fn narrow_parameter_function(
        symbol: &str,
        ty: SystemType,
        storage_size: u32,
        alignment: u32,
    ) -> SystemsFunction {
        SystemsFunction {
            source_name: symbol.into(),
            symbol: symbol.into(),
            visibility: Visibility::Public,
            interrupt: None,
            call_convention: CallConvention::DByte,
            parameter_types: vec![ty.clone()],
            params: vec![SystemsParam {
                name: "value".into(),
                ty: ty.clone(),
                slot: LocalId(0),
            }],
            return_type: SystemType::I32,
            slots: vec![SystemsSlot {
                id: LocalId(0),
                name: "value".into(),
                ty: ty.clone(),
                storage_size,
                alignment,
                is_parameter: true,
            }],
            instructions: vec![
                SystemsInst::LocalLoad {
                    local: LocalId(0),
                    ty: ty.clone(),
                },
                SystemsInst::Cast {
                    from: ty,
                    to: SystemType::I32,
                },
                SystemsInst::Return,
            ],
            span: Span::new(1, 1),
        }
    }

    fn defined_function_bytes<'a>(lowered: &'a LoweredSystemsObject, symbol: &str) -> &'a [u8] {
        let function = lowered
            .image
            .symbols
            .iter()
            .find(|candidate| candidate.name == symbol)
            .unwrap();
        let SymbolDefinition::Defined {
            section: ObjectSection::Text,
            value,
        } = function.definition
        else {
            panic!("function must be defined in text")
        };
        let text = match &lowered.image.sections[&ObjectSection::Text].contents {
            SectionContents::Bytes(bytes) => bytes,
            SectionContents::Uninitialized(_) => panic!("text must be file-backed"),
        };
        &text[value as usize..(value + function.size) as usize]
    }

    fn assert_parameter_spill(function: &[u8], expected: &[u8], description: &str) {
        const LOAD_INCOMING_PARAMETER: [u8; 6] = [0x8b, 0x85, 0x08, 0x00, 0x00, 0x00];
        let load = function
            .windows(LOAD_INCOMING_PARAMETER.len())
            .position(|bytes| bytes == LOAD_INCOMING_PARAMETER)
            .expect("parameter function must load its DBYTE32 stack argument");
        let spill_start = load + LOAD_INCOMING_PARAMETER.len();
        let spill_end = spill_start + expected.len();
        assert_eq!(
            function.get(spill_start..spill_end),
            Some(expected),
            "parameter spill must be `{description}`: {function:02x?}"
        );
    }

    #[test]
    fn malformed_stack_ir_is_rejected_without_finishing_bytes() {
        let error =
            lower_systems_module(&module(vec![function(vec![SystemsInst::Return])])).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::Encoder);
        assert!(error.message.contains("underflow"));
    }
}
