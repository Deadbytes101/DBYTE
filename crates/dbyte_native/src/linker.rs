//! Deterministic in-process static linker for DByte ELF32/i386 objects.

use crate::archive::parse_archive;
use crate::diagnostics::{Diagnostic, DiagnosticCategory};
use crate::elf32::{
    write_linked_executable, write_user_executable, ElfLayout, ExecutableSymbol,
    LinkedExecutableImage, BOOTSTRAP_STACK_SIZE, MAX_ELF_SIZE,
};
use crate::external_elf::parse_external_i386_object;
use crate::object::{
    parse_relocatable, write_relocatable, ObjectImage, ObjectRelocation, ObjectSection,
    RelocationKind, SectionContents, SymbolBinding, SymbolDefinition, SymbolKind,
    ENTRY_METADATA_PREFIX, PROFILE_METADATA_PREFIX, TASK_METADATA_PREFIX,
};
use crate::x86::{Encoder, Register};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub const LINK_ENTRY_SYMBOL: &str = "kernel_main";
pub const LINK_USER_ENTRY_SYMBOL: &str = "user_main";
pub const LINK_START_SYMBOL: &str = "_start";
pub const LINK_STACK_BOTTOM_SYMBOL: &str = "__dbyte_stack_bottom";
pub const LINK_STACK_TOP_SYMBOL: &str = "__dbyte_stack_top";
pub const LINK_IMAGE_START_SYMBOL: &str = "__dbyte_image_start";
pub const LINK_IMAGE_END_SYMBOL: &str = "__dbyte_image_end";
pub const LINK_TASK_TRAMPOLINE_SYMBOL: &str = "__dbyte_task_trampoline";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryAbi {
    Legacy,
    MultibootV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkInput {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl LinkInput {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            bytes,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkArtifact {
    pub elf: Vec<u8>,
    pub bin: Vec<u8>,
    pub machine_code: Vec<u8>,
    pub layout: ElfLayout,
    pub dump: String,
}

#[derive(Clone, Debug)]
struct ParsedInput {
    name: String,
    image: ObjectImage,
}

#[derive(Clone, Copy, Debug)]
struct Bootstrap {
    stack_immediate: Option<u32>,
    entry_immediate: u32,
    size: u32,
}

#[derive(Clone, Copy, Debug)]
struct ExecutableSymbolLayout {
    bootstrap: Bootstrap,
    stack_bottom_offset: u32,
    stack_top_offset: u32,
    image_end_offset: u32,
    task_trampoline: Option<u32>,
    profile: LinkProfile,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LinkProfile {
    Kernel,
    User,
}

#[derive(Clone, Copy, Debug)]
enum DefinitionLocation {
    Input { input: usize, symbol: usize },
    Synthetic { section: ObjectSection, offset: u32 },
}

#[derive(Clone, Copy, Debug)]
struct GlobalDefinition {
    kind: SymbolKind,
    location: DefinitionLocation,
}

#[derive(Clone, Debug)]
struct MergedImage {
    executable: LinkedExecutableImage,
    placements: BTreeMap<(usize, ObjectSection), u32>,
    bootstrap: Bootstrap,
    task_trampoline: Option<u32>,
}

/// Parses and links serialized DByte ELF objects. Input order does not affect
/// the result; the canonical leaf name controls deterministic ordering.
pub fn link_object_bytes(inputs: &[LinkInput]) -> Result<LinkArtifact, Diagnostic> {
    if inputs.is_empty() {
        return Err(link_error("cannot link an empty object set"));
    }
    let mut parsed = parse_inputs_and_archives(inputs, LINK_ENTRY_SYMBOL)?;
    parsed.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in parsed.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(link_error(format!(
                "duplicate linker input name '{}'",
                pair[0].name
            )));
        }
    }
    link_parsed(&parsed, LinkProfile::Kernel)
}

pub fn link_user_object_bytes(inputs: &[LinkInput]) -> Result<LinkArtifact, Diagnostic> {
    if inputs.is_empty() {
        return Err(link_error("cannot link an empty object set"));
    }
    let mut parsed = parse_inputs_and_archives(inputs, LINK_USER_ENTRY_SYMBOL)?;
    parsed.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in parsed.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(link_error(format!(
                "duplicate linker input name '{}'",
                pair[0].name
            )));
        }
    }
    link_parsed(&parsed, LinkProfile::User)
}

fn parse_inputs_and_archives(
    inputs: &[LinkInput],
    entry_symbol: &str,
) -> Result<Vec<ParsedInput>, Diagnostic> {
    let mut parsed = Vec::new();
    let mut archives = Vec::<(String, Vec<ParsedInput>)>::new();
    for input in inputs {
        let name = canonical_input_name(&input.name)?;
        if input.bytes.starts_with(b"!<arch>\n") {
            let members = parse_archive(&input.bytes).map_err(|error| {
                if error.path.is_some() {
                    error
                } else {
                    error.with_path(name.clone())
                }
            })?;
            let mut parsed_members = Vec::with_capacity(members.len());
            for member in members {
                let member_name = format!("{}__{}", name.replace('.', "_"), member.name);
                let image = parse_input_object(&member.bytes)
                    .map_err(|error| error.with_path(format!("{name}({})", member.name)))?;
                parsed_members.push(ParsedInput {
                    name: member_name,
                    image,
                });
            }
            parsed_members.sort_by(|left, right| left.name.cmp(&right.name));
            archives.push((name, parsed_members));
        } else {
            let image = parse_input_object(&input.bytes).map_err(|error| {
                if error.path.is_some() {
                    error
                } else {
                    error.with_path(name.clone())
                }
            })?;
            parsed.push(ParsedInput { name, image });
        }
    }
    archives.sort_by(|left, right| left.0.cmp(&right.0));
    let mut selected = BTreeSet::<(usize, usize)>::new();
    loop {
        let mut defined = BTreeSet::new();
        let mut unresolved = BTreeSet::from([entry_symbol.to_owned()]);
        for input in &parsed {
            collect_symbol_sets(&input.image, &mut defined, &mut unresolved);
        }
        unresolved.retain(|symbol| !defined.contains(symbol));
        let mut changed = false;
        for (archive_index, (_, members)) in archives.iter().enumerate() {
            for (member_index, member) in members.iter().enumerate() {
                if selected.contains(&(archive_index, member_index)) {
                    continue;
                }
                let satisfies = member.image.symbols.iter().any(|symbol| {
                    symbol.binding == SymbolBinding::Global
                        && matches!(symbol.definition, SymbolDefinition::Defined { .. })
                        && unresolved.contains(&symbol.name)
                });
                if satisfies {
                    selected.insert((archive_index, member_index));
                    parsed.push(member.clone());
                    changed = true;
                    break;
                }
            }
            if changed {
                break;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(parsed)
}

fn parse_input_object(bytes: &[u8]) -> Result<ObjectImage, Diagnostic> {
    match parse_relocatable(bytes) {
        Ok(image) => Ok(image),
        Err(canonical) if canonical.category == DiagnosticCategory::InvalidObject => {
            match parse_external_i386_object(bytes) {
                Ok(image) => Ok(image),
                Err(_)
                    if bytes
                        .windows(b".dbyte.".len())
                        .any(|window| window == b".dbyte.") =>
                {
                    Err(canonical)
                }
                Err(external) => Err(external),
            }
        }
        Err(canonical) => Err(canonical),
    }
}

fn collect_symbol_sets(
    image: &ObjectImage,
    defined: &mut BTreeSet<String>,
    unresolved: &mut BTreeSet<String>,
) {
    for symbol in &image.symbols {
        if symbol.binding != SymbolBinding::Global {
            continue;
        }
        match symbol.definition {
            SymbolDefinition::Undefined => {
                unresolved.insert(symbol.name.clone());
            }
            SymbolDefinition::Defined { .. } => {
                defined.insert(symbol.name.clone());
            }
        }
    }
}

/// Serializes every image through the ET_REL seam before linking it. This
/// deliberately exercises the same bounded parser used by the standalone
/// `dbytec link` path.
pub fn link_images(inputs: &[(String, ObjectImage)]) -> Result<LinkArtifact, Diagnostic> {
    let mut serialized = Vec::with_capacity(inputs.len());
    for (name, image) in inputs {
        let artifact = write_relocatable(image)?;
        serialized.push(LinkInput::new(name.clone(), artifact.bytes));
    }
    link_object_bytes(&serialized)
}

fn link_parsed(inputs: &[ParsedInput], profile: LinkProfile) -> Result<LinkArtifact, Diagnostic> {
    validate_profiles(inputs, profile)?;
    let (entry_abi, needs_task_trampoline) = if profile == LinkProfile::Kernel {
        validate_interrupt_metadata(inputs)?;
        (
            validate_entry_metadata(inputs)?,
            validate_task_metadata(inputs)?,
        )
    } else {
        (EntryAbi::Legacy, false)
    };
    let entry_symbol = if profile == LinkProfile::Kernel {
        LINK_ENTRY_SYMBOL
    } else {
        LINK_USER_ENTRY_SYMBOL
    };
    let mut merged = merge_sections(inputs, entry_abi, needs_task_trampoline, profile)?;
    let definitions = collect_definitions(inputs, &merged, entry_symbol, profile)?;
    validate_undefined_symbols(inputs, &definitions)?;

    let writer = if profile == LinkProfile::Kernel {
        write_linked_executable
    } else {
        write_user_executable
    };
    let preliminary = writer(&merged.executable).map_err(map_elf_layout_error)?;
    let mut relocation_dump = String::new();
    apply_bootstrap_relocations(
        &mut merged,
        &preliminary.layout,
        &definitions,
        inputs,
        &mut relocation_dump,
        entry_symbol,
    )?;
    apply_input_relocations(
        &mut merged,
        &preliminary.layout,
        &definitions,
        inputs,
        &mut relocation_dump,
    )?;

    let final_elf = writer(&merged.executable).map_err(map_elf_layout_error)?;
    let mut dump = link_dump(inputs, &merged, &definitions);
    dump.push_str(&relocation_dump);
    dump.push_str(&final_elf.dump);
    Ok(LinkArtifact {
        elf: final_elf.elf,
        bin: final_elf.bin,
        machine_code: final_elf.machine_code,
        layout: final_elf.layout,
        dump,
    })
}

fn validate_profiles(inputs: &[ParsedInput], profile: LinkProfile) -> Result<(), Diagnostic> {
    for input in inputs {
        let records: Vec<&str> = input
            .image
            .symbols
            .iter()
            .filter_map(|symbol| symbol.name.strip_prefix(PROFILE_METADATA_PREFIX))
            .collect();
        match profile {
            LinkProfile::Kernel if !records.is_empty() => {
                return Err(invalid_object(format!(
                    "kernel link rejects user-profile object '{}'",
                    input.name
                )))
            }
            LinkProfile::User if records.as_slice() != ["user"] => {
                return Err(invalid_object(format!(
                    "user link requires canonical user profile metadata in '{}'",
                    input.name
                )))
            }
            _ => {}
        }
    }
    Ok(())
}

fn merge_sections(
    inputs: &[ParsedInput],
    entry_abi: EntryAbi,
    needs_task_trampoline: bool,
    profile: LinkProfile,
) -> Result<MergedImage, Diagnostic> {
    let (bootstrap_bytes, bootstrap) = if profile == LinkProfile::Kernel {
        build_bootstrap(entry_abi)?
    } else {
        build_user_bootstrap()?
    };
    let mut text = bootstrap_bytes;
    let task_trampoline = if needs_task_trampoline {
        let offset = usize_u32(text.len(), "task trampoline offset")?;
        let mut encoder = Encoder::new(3);
        encoder.call_reg(Register::Eax).map_err(encoder_error)?;
        encoder.ret().map_err(encoder_error)?;
        extend_bounded(
            &mut text,
            &encoder.finish().map_err(encoder_error)?.bytes,
            "task trampoline",
        )?;
        Some(offset)
    } else {
        None
    };
    let mut rodata = Vec::new();
    let mut data = Vec::new();
    let mut bss_size = 0u32;
    let mut placements = BTreeMap::new();
    let mut alignments = BTreeMap::from([
        (ObjectSection::Text, 16u32),
        (ObjectSection::Rodata, 1u32),
        (ObjectSection::Data, 1u32),
        (ObjectSection::Bss, 1u32),
    ]);

    for section in [
        ObjectSection::Text,
        ObjectSection::Rodata,
        ObjectSection::Data,
    ] {
        let destination = match section {
            ObjectSection::Text => &mut text,
            ObjectSection::Rodata => &mut rodata,
            ObjectSection::Data => &mut data,
            ObjectSection::Bss => unreachable!(),
        };
        for (input_index, input) in inputs.iter().enumerate() {
            let source = input.image.sections.get(&section).ok_or_else(|| {
                invalid_object(format!("{} is missing {}", input.name, section.name()))
            })?;
            let SectionContents::Bytes(bytes) = &source.contents else {
                return Err(invalid_object(format!(
                    "{} {} is not file-backed",
                    input.name,
                    section.name()
                )));
            };
            align_buffer(destination, source.alignment, section.name())?;
            let offset = usize_u32(destination.len(), "merged section offset")?;
            placements.insert((input_index, section), offset);
            extend_bounded(destination, bytes, section.name())?;
            alignments.insert(section, alignments[&section].max(source.alignment));
        }
    }

    for (input_index, input) in inputs.iter().enumerate() {
        let source = input
            .image
            .sections
            .get(&ObjectSection::Bss)
            .ok_or_else(|| invalid_object(format!("{} is missing .bss", input.name)))?;
        let SectionContents::Uninitialized(size) = &source.contents else {
            return Err(invalid_object(format!(
                "{} .bss is file-backed",
                input.name
            )));
        };
        bss_size = align_u32(bss_size, source.alignment, "merged .bss placement")?;
        placements.insert((input_index, ObjectSection::Bss), bss_size);
        bss_size = checked_add(bss_size, *size, "merged .bss size")?;
        alignments.insert(
            ObjectSection::Bss,
            alignments[&ObjectSection::Bss].max(source.alignment),
        );
    }
    let (stack_bottom_offset, stack_top_offset) = if profile == LinkProfile::Kernel {
        let bottom = align_u32(bss_size, 16, "bootstrap stack placement")?;
        let top = checked_add(bottom, BOOTSTRAP_STACK_SIZE, "bootstrap stack top")?;
        bss_size = top;
        alignments.insert(ObjectSection::Bss, alignments[&ObjectSection::Bss].max(16));
        (bottom, top)
    } else {
        (0, 0)
    };

    let symbols = build_executable_symbols(
        inputs,
        &placements,
        ExecutableSymbolLayout {
            bootstrap,
            stack_bottom_offset,
            stack_top_offset,
            image_end_offset: bss_size,
            task_trampoline,
            profile,
        },
    )?;
    Ok(MergedImage {
        executable: LinkedExecutableImage {
            text,
            rodata,
            data,
            bss_size,
            text_alignment: alignments[&ObjectSection::Text],
            rodata_alignment: alignments[&ObjectSection::Rodata],
            data_alignment: alignments[&ObjectSection::Data],
            bss_alignment: alignments[&ObjectSection::Bss],
            entry_offset: 0,
            stack_bottom_offset,
            stack_top_offset,
            symbols,
        },
        placements,
        bootstrap,
        task_trampoline,
    })
}

fn build_bootstrap(entry_abi: EntryAbi) -> Result<(Vec<u8>, Bootstrap), Diagnostic> {
    let mut encoder = Encoder::new(64);
    encoder.cli().map_err(encoder_error)?;
    let stack_immediate = usize_u32(
        encoder
            .mov_reg_imm32_placeholder(Register::Esp)
            .map_err(encoder_error)?,
        "bootstrap stack relocation offset",
    )?;
    encoder
        .xor_reg_reg(Register::Ebp, Register::Ebp)
        .map_err(encoder_error)?;
    if entry_abi == EntryAbi::MultibootV1 {
        encoder.push_reg(Register::Ebx).map_err(encoder_error)?;
        encoder.push_reg(Register::Eax).map_err(encoder_error)?;
    }
    let entry_immediate = usize_u32(
        encoder.call_rel32_placeholder().map_err(encoder_error)?,
        "bootstrap entry relocation offset",
    )?;
    if entry_abi == EntryAbi::MultibootV1 {
        encoder.add_esp_imm32(8).map_err(encoder_error)?;
    }
    let halt = encoder.new_label().map_err(encoder_error)?;
    encoder.bind_label(halt).map_err(encoder_error)?;
    encoder.hlt().map_err(encoder_error)?;
    encoder.jump(halt).map_err(encoder_error)?;
    let encoded = encoder.finish().map_err(encoder_error)?;
    let size = usize_u32(encoded.bytes.len(), "bootstrap size")?;
    Ok((
        encoded.bytes,
        Bootstrap {
            stack_immediate: Some(stack_immediate),
            entry_immediate,
            size,
        },
    ))
}

fn build_user_bootstrap() -> Result<(Vec<u8>, Bootstrap), Diagnostic> {
    let mut encoder = Encoder::new(32);
    let entry_immediate = usize_u32(
        encoder.call_rel32_placeholder().map_err(encoder_error)?,
        "user entry relocation offset",
    )?;
    encoder
        .mov_reg_reg(Register::Ebx, Register::Eax)
        .map_err(encoder_error)?;
    encoder
        .xor_reg_reg(Register::Eax, Register::Eax)
        .map_err(encoder_error)?;
    encoder.software_interrupt(0x80).map_err(encoder_error)?;
    let stopped = encoder.new_label().map_err(encoder_error)?;
    encoder.bind_label(stopped).map_err(encoder_error)?;
    encoder.jump(stopped).map_err(encoder_error)?;
    let encoded = encoder.finish().map_err(encoder_error)?;
    let size = usize_u32(encoded.bytes.len(), "user bootstrap size")?;
    Ok((
        encoded.bytes,
        Bootstrap {
            stack_immediate: None,
            entry_immediate,
            size,
        },
    ))
}

fn build_executable_symbols(
    inputs: &[ParsedInput],
    placements: &BTreeMap<(usize, ObjectSection), u32>,
    layout: ExecutableSymbolLayout,
) -> Result<Vec<ExecutableSymbol>, Diagnostic> {
    let mut symbols = vec![ExecutableSymbol {
        name: LINK_START_SYMBOL.to_owned(),
        binding: SymbolBinding::Global,
        kind: SymbolKind::Function,
        section: ObjectSection::Text,
        offset: 0,
        size: layout.bootstrap.size,
    }];
    if layout.profile == LinkProfile::Kernel {
        symbols.extend([
            ExecutableSymbol {
                name: LINK_STACK_BOTTOM_SYMBOL.to_owned(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Object,
                section: ObjectSection::Bss,
                offset: layout.stack_bottom_offset,
                size: BOOTSTRAP_STACK_SIZE,
            },
            ExecutableSymbol {
                name: LINK_STACK_TOP_SYMBOL.to_owned(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Object,
                section: ObjectSection::Bss,
                offset: layout.stack_top_offset,
                size: 0,
            },
            ExecutableSymbol {
                name: LINK_IMAGE_START_SYMBOL.to_owned(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Object,
                section: ObjectSection::Text,
                offset: 0,
                size: 0,
            },
            ExecutableSymbol {
                name: LINK_IMAGE_END_SYMBOL.to_owned(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Object,
                section: ObjectSection::Bss,
                offset: layout.image_end_offset,
                size: 0,
            },
        ]);
    }
    if let Some(offset) = layout.task_trampoline {
        symbols.push(ExecutableSymbol {
            name: LINK_TASK_TRAMPOLINE_SYMBOL.to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            section: ObjectSection::Text,
            offset,
            size: 3,
        });
    }
    for (input_index, input) in inputs.iter().enumerate() {
        for symbol in &input.image.symbols {
            let SymbolDefinition::Defined { section, value } = symbol.definition else {
                continue;
            };
            // Per-object STT_SECTION entries are an ET_REL implementation
            // detail. The final executable already describes each merged
            // section in its section table, so emitting one entry per input
            // would give those symbols misleading nonzero section offsets.
            if symbol.kind == SymbolKind::Section
                || symbol.name.starts_with("__dbyte_interrupt:")
                || symbol.name.starts_with(ENTRY_METADATA_PREFIX)
                || symbol.name.starts_with(TASK_METADATA_PREFIX)
                || symbol.name.starts_with(PROFILE_METADATA_PREFIX)
            {
                continue;
            }
            let base = placements
                .get(&(input_index, section))
                .copied()
                .ok_or_else(|| link_error("missing merged symbol placement"))?;
            let offset = checked_add(base, value, "merged symbol offset")?;
            let name = if symbol.binding == SymbolBinding::Local {
                let leaf = if symbol.name.is_empty() {
                    section.name()
                } else {
                    &symbol.name
                };
                format!("{}::{leaf}", input.name)
            } else {
                symbol.name.clone()
            };
            symbols.push(ExecutableSymbol {
                name,
                binding: symbol.binding,
                kind: symbol.kind,
                section,
                offset,
                size: symbol.size,
            });
        }
    }
    Ok(symbols)
}

fn validate_interrupt_metadata(inputs: &[ParsedInput]) -> Result<(), Diagnostic> {
    let mut definitions = BTreeSet::new();
    let mut uses = Vec::new();
    for input in inputs {
        for symbol in &input.image.symbols {
            let Some(record) = symbol.name.strip_prefix("__dbyte_interrupt:") else {
                continue;
            };
            let parts: Vec<&str> = record.split(':').collect();
            match parts.as_slice() {
                ["def", class, handler]
                    if matches!(
                        *class,
                        "no_error_code"
                            | "error_code"
                            | "no_error_code,cpl3_transition"
                            | "error_code,cpl3_transition"
                    ) && !handler.is_empty() =>
                {
                    if !definitions.insert((class.to_string(), handler.to_string())) {
                        return Err(Diagnostic::without_span(
                            DiagnosticCategory::DuplicateSymbol,
                            format!("duplicate interrupt definition metadata for {handler}"),
                        ));
                    }
                }
                ["use", vector, class, handler]
                    if matches!(
                        *class,
                        "no_error_code"
                            | "error_code"
                            | "no_error_code,cpl3_transition"
                            | "error_code,cpl3_transition"
                    ) && !handler.is_empty() =>
                {
                    let vector: u8 = vector.parse().map_err(|_| {
                        Diagnostic::without_span(
                            DiagnosticCategory::InvalidObject,
                            format!("invalid interrupt vector metadata '{}'", symbol.name),
                        )
                    })?;
                    let base_required = if matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17) {
                        "error_code"
                    } else {
                        "no_error_code"
                    };
                    let class_error = class.starts_with("error_code");
                    let class_transition = class.ends_with(",cpl3_transition");
                    let transition_valid = vector != 128 || class_transition;
                    if class_error != (base_required == "error_code") || !transition_valid {
                        return Err(Diagnostic::without_span(
                            DiagnosticCategory::InterruptClassMismatch,
                            format!(
                                "vector {vector} requires {base_required}{} handler, metadata names {class}",
                                if vector == 128 { ",cpl3_transition" } else { "" }
                            ),
                        ));
                    }
                    uses.push((class.to_string(), handler.to_string(), vector));
                }
                _ => {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::InvalidObject,
                        format!("malformed interrupt metadata symbol '{}'", symbol.name),
                    ))
                }
            }
        }
    }
    for (class, handler, vector) in uses {
        if !definitions.contains(&(class.clone(), handler.clone())) {
            return Err(Diagnostic::without_span(
                DiagnosticCategory::UndefinedSymbol,
                format!(
                    "interrupt vector {vector} references {class} handler '{handler}' without matching metadata"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_task_metadata(inputs: &[ParsedInput]) -> Result<bool, Diagnostic> {
    let mut definitions = BTreeSet::new();
    let mut uses = BTreeSet::new();
    let mut trampoline = false;
    for input in inputs {
        for symbol in &input.image.symbols {
            let Some(record) = symbol.name.strip_prefix(TASK_METADATA_PREFIX) else {
                continue;
            };
            if symbol.binding != SymbolBinding::Local
                || symbol.kind != SymbolKind::Function
                || symbol.definition == SymbolDefinition::Undefined
            {
                return Err(Diagnostic::without_span(
                    DiagnosticCategory::InvalidObject,
                    format!("invalid task metadata symbol contract '{}'", symbol.name),
                ));
            }
            if record == "trampoline" {
                if trampoline {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::DuplicateSymbol,
                        "duplicate task trampoline metadata",
                    ));
                }
                trampoline = true;
                let target = input
                    .image
                    .symbols
                    .iter()
                    .position(|candidate| {
                        candidate.name == LINK_TASK_TRAMPOLINE_SYMBOL
                            && candidate.binding == SymbolBinding::Global
                            && candidate.kind == SymbolKind::Function
                            && candidate.definition == SymbolDefinition::Undefined
                    })
                    .ok_or_else(|| {
                        Diagnostic::without_span(
                            DiagnosticCategory::InvalidObject,
                            "task trampoline metadata lacks its undefined function symbol",
                        )
                    })?;
                if !input.image.relocations.iter().any(|relocation| {
                    relocation.symbol.0 as usize == target
                        && relocation.kind == RelocationKind::Abs32
                }) {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::InvalidObject,
                        "task trampoline metadata lacks R_386_32",
                    ));
                }
            } else if let Some(name) = record.strip_prefix("def:") {
                if name.is_empty() || name.contains(':') || !definitions.insert(name.to_owned()) {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::DuplicateSymbol,
                        format!("duplicate or malformed task definition metadata for '{name}'"),
                    ));
                }
                let valid = input.image.symbols.iter().any(|candidate| {
                    candidate.name == name
                        && candidate.binding == SymbolBinding::Global
                        && candidate.kind == SymbolKind::Function
                });
                if !valid {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::InvalidObject,
                        format!("task definition metadata for '{name}' lacks a global function"),
                    ));
                }
            } else if let Some(name) = record.strip_prefix("use:") {
                if name.is_empty() || name.contains(':') || !uses.insert(name.to_owned()) {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::DuplicateSymbol,
                        format!("duplicate or malformed task reference metadata for '{name}'"),
                    ));
                }
                let target = input
                    .image
                    .symbols
                    .iter()
                    .position(|candidate| {
                        candidate.name == name
                            && candidate.binding == SymbolBinding::Global
                            && candidate.kind == SymbolKind::Function
                    })
                    .ok_or_else(|| {
                        Diagnostic::without_span(
                            DiagnosticCategory::InvalidObject,
                            format!("task reference metadata for '{name}' lacks a function symbol"),
                        )
                    })?;
                if !input.image.relocations.iter().any(|relocation| {
                    relocation.symbol.0 as usize == target
                        && relocation.kind == RelocationKind::Abs32
                }) {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::InvalidObject,
                        format!("task reference metadata for '{name}' lacks R_386_32"),
                    ));
                }
            } else {
                return Err(Diagnostic::without_span(
                    DiagnosticCategory::InvalidObject,
                    format!("malformed task metadata symbol '{}'", symbol.name),
                ));
            }
        }
    }
    for name in &uses {
        if !definitions.contains(name) {
            return Err(Diagnostic::without_span(
                DiagnosticCategory::UndefinedSymbol,
                format!("task entry '{name}' has no checked task definition metadata"),
            ));
        }
    }
    Ok(trampoline || !uses.is_empty())
}

fn validate_entry_metadata(inputs: &[ParsedInput]) -> Result<EntryAbi, Diagnostic> {
    let mut abi = None;
    for input in inputs {
        for symbol in &input.image.symbols {
            let Some(record) = symbol.name.strip_prefix(ENTRY_METADATA_PREFIX) else {
                continue;
            };
            let candidate = match record {
                "legacy" => EntryAbi::Legacy,
                "multiboot_v1" => EntryAbi::MultibootV1,
                _ => {
                    return Err(Diagnostic::without_span(
                        DiagnosticCategory::InvalidObject,
                        format!("unsupported native entry metadata ABI '{record}'"),
                    ))
                }
            };
            if abi.replace(candidate).is_some() {
                return Err(Diagnostic::without_span(
                    DiagnosticCategory::DuplicateSymbol,
                    "multiple native entry metadata records",
                ));
            }
        }
    }
    Ok(abi.unwrap_or(EntryAbi::Legacy))
}

fn collect_definitions(
    inputs: &[ParsedInput],
    merged: &MergedImage,
    entry_symbol: &str,
    profile: LinkProfile,
) -> Result<BTreeMap<String, GlobalDefinition>, Diagnostic> {
    let mut definitions = BTreeMap::from([(
        LINK_START_SYMBOL.to_owned(),
        GlobalDefinition {
            kind: SymbolKind::Function,
            location: DefinitionLocation::Synthetic {
                section: ObjectSection::Text,
                offset: 0,
            },
        },
    )]);
    if profile == LinkProfile::Kernel {
        definitions.extend([
            (
                LINK_STACK_BOTTOM_SYMBOL.to_owned(),
                GlobalDefinition {
                    kind: SymbolKind::Object,
                    location: DefinitionLocation::Synthetic {
                        section: ObjectSection::Bss,
                        offset: merged.executable.stack_bottom_offset,
                    },
                },
            ),
            (
                LINK_STACK_TOP_SYMBOL.to_owned(),
                GlobalDefinition {
                    kind: SymbolKind::Object,
                    location: DefinitionLocation::Synthetic {
                        section: ObjectSection::Bss,
                        offset: merged.executable.stack_top_offset,
                    },
                },
            ),
            (
                LINK_IMAGE_START_SYMBOL.to_owned(),
                GlobalDefinition {
                    kind: SymbolKind::Object,
                    location: DefinitionLocation::Synthetic {
                        section: ObjectSection::Text,
                        offset: 0,
                    },
                },
            ),
            (
                LINK_IMAGE_END_SYMBOL.to_owned(),
                GlobalDefinition {
                    kind: SymbolKind::Object,
                    location: DefinitionLocation::Synthetic {
                        section: ObjectSection::Bss,
                        offset: merged.executable.bss_size,
                    },
                },
            ),
        ]);
    }
    if let Some(offset) = merged.task_trampoline {
        definitions.insert(
            LINK_TASK_TRAMPOLINE_SYMBOL.to_owned(),
            GlobalDefinition {
                kind: SymbolKind::Function,
                location: DefinitionLocation::Synthetic {
                    section: ObjectSection::Text,
                    offset,
                },
            },
        );
    }
    for (input_index, input) in inputs.iter().enumerate() {
        for (symbol_index, symbol) in input.image.symbols.iter().enumerate() {
            if symbol.binding != SymbolBinding::Global
                || symbol.definition == SymbolDefinition::Undefined
            {
                continue;
            }
            let definition = GlobalDefinition {
                kind: symbol.kind,
                location: DefinitionLocation::Input {
                    input: input_index,
                    symbol: symbol_index,
                },
            };
            if definitions
                .insert(symbol.name.clone(), definition)
                .is_some()
            {
                return Err(Diagnostic::without_span(
                    DiagnosticCategory::DuplicateSymbol,
                    format!("duplicate global symbol '{}'", symbol.name),
                ));
            }
        }
    }
    let entry = definitions.get(entry_symbol).ok_or_else(|| {
        Diagnostic::without_span(
            DiagnosticCategory::UndefinedSymbol,
            format!("undefined entry symbol '{entry_symbol}'"),
        )
    })?;
    if entry.kind != SymbolKind::Function {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::UndefinedSymbol,
            format!("entry symbol '{entry_symbol}' is not a function"),
        ));
    }
    Ok(definitions)
}

fn validate_undefined_symbols(
    inputs: &[ParsedInput],
    definitions: &BTreeMap<String, GlobalDefinition>,
) -> Result<(), Diagnostic> {
    for input in inputs {
        for symbol in &input.image.symbols {
            if symbol.definition != SymbolDefinition::Undefined {
                continue;
            }
            let definition = definitions.get(&symbol.name).ok_or_else(|| {
                Diagnostic::without_span(
                    DiagnosticCategory::UndefinedSymbol,
                    format!(
                        "undefined symbol '{}' referenced by {}",
                        symbol.name, input.name
                    ),
                )
            })?;
            if definition.kind != symbol.kind {
                return Err(Diagnostic::without_span(
                    DiagnosticCategory::UndefinedSymbol,
                    format!(
                        "symbol '{}' kind mismatch: {:?} declaration, {:?} definition",
                        symbol.name, symbol.kind, definition.kind
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn apply_bootstrap_relocations(
    merged: &mut MergedImage,
    layout: &ElfLayout,
    definitions: &BTreeMap<String, GlobalDefinition>,
    inputs: &[ParsedInput],
    dump: &mut String,
    entry_symbol: &str,
) -> Result<(), Diagnostic> {
    if let Some(stack_immediate) = merged.bootstrap.stack_immediate {
        let stack = definitions
            .get(LINK_STACK_TOP_SYMBOL)
            .expect("kernel linker always defines stack top");
        let stack_address = definition_address(*stack, inputs, merged, layout)?;
        apply_relocation_value(
            &mut merged.executable.text,
            stack_immediate,
            RelocationKind::Abs32,
            stack_address,
            0,
            checked_add(
                layout.text_address,
                stack_immediate,
                "bootstrap stack relocation place",
            )?,
            LINK_STACK_TOP_SYMBOL,
        )?;
        let _ = writeln!(
            dump,
            "apply bootstrap .text+0x{stack_immediate:08x} Abs32 {} S=0x{stack_address:08x} A=0",
            LINK_STACK_TOP_SYMBOL
        );
    }

    let entry = *definitions.get(entry_symbol).ok_or_else(|| {
        Diagnostic::without_span(
            DiagnosticCategory::UndefinedSymbol,
            format!("undefined entry symbol '{entry_symbol}'"),
        )
    })?;
    let entry_address = definition_address(entry, inputs, merged, layout)?;
    let place = checked_add(
        layout.text_address,
        merged.bootstrap.entry_immediate,
        "bootstrap entry relocation place",
    )?;
    apply_relocation_value(
        &mut merged.executable.text,
        merged.bootstrap.entry_immediate,
        RelocationKind::Pc32,
        entry_address,
        -4,
        place,
        entry_symbol,
    )?;
    let _ = writeln!(
        dump,
        "apply bootstrap .text+0x{:08x} Pc32 {} S=0x{:08x} A=-4 P=0x{:08x}",
        merged.bootstrap.entry_immediate, entry_symbol, entry_address, place
    );
    Ok(())
}

fn apply_input_relocations(
    merged: &mut MergedImage,
    layout: &ElfLayout,
    definitions: &BTreeMap<String, GlobalDefinition>,
    inputs: &[ParsedInput],
    dump: &mut String,
) -> Result<(), Diagnostic> {
    for (input_index, input) in inputs.iter().enumerate() {
        let mut relocations: Vec<&ObjectRelocation> = input.image.relocations.iter().collect();
        relocations.sort_by_key(|relocation| {
            (
                relocation.section,
                relocation.offset,
                relocation.kind,
                relocation.symbol,
            )
        });
        for relocation in relocations {
            let symbol_index = relocation.symbol.0 as usize;
            let symbol = input.image.symbols.get(symbol_index).ok_or_else(|| {
                Diagnostic::without_span(
                    DiagnosticCategory::InvalidRelocation,
                    format!(
                        "{} relocation references missing symbol {}",
                        input.name, symbol_index
                    ),
                )
            })?;
            let definition = match symbol.definition {
                SymbolDefinition::Defined { .. } => GlobalDefinition {
                    kind: symbol.kind,
                    location: DefinitionLocation::Input {
                        input: input_index,
                        symbol: symbol_index,
                    },
                },
                SymbolDefinition::Undefined => *definitions.get(&symbol.name).ok_or_else(|| {
                    Diagnostic::without_span(
                        DiagnosticCategory::UndefinedSymbol,
                        format!("undefined symbol '{}'", symbol.name),
                    )
                })?,
            };
            if definition.kind != symbol.kind {
                return Err(Diagnostic::without_span(
                    DiagnosticCategory::UndefinedSymbol,
                    format!("symbol '{}' has conflicting kinds", symbol.name),
                ));
            }
            let symbol_address = definition_address(definition, inputs, merged, layout)?;
            let placement = merged
                .placements
                .get(&(input_index, relocation.section))
                .copied()
                .ok_or_else(|| link_error("missing relocation section placement"))?;
            let merged_offset =
                checked_add(placement, relocation.offset, "merged relocation offset")?;
            let place = checked_add(
                section_address(layout, relocation.section),
                merged_offset,
                "relocation place address",
            )?;
            let target = section_bytes_mut(&mut merged.executable, relocation.section)?;
            apply_relocation_value(
                target,
                merged_offset,
                relocation.kind,
                symbol_address,
                relocation.addend,
                place,
                &symbol.name,
            )?;
            let _ = writeln!(
                dump,
                "apply {} {}+0x{:08x} {:?} {} S=0x{:08x} A={} P=0x{:08x}",
                input.name,
                relocation.section.name(),
                merged_offset,
                relocation.kind,
                symbol.name,
                symbol_address,
                relocation.addend,
                place
            );
        }
    }
    Ok(())
}

fn definition_address(
    definition: GlobalDefinition,
    inputs: &[ParsedInput],
    merged: &MergedImage,
    layout: &ElfLayout,
) -> Result<u32, Diagnostic> {
    match definition.location {
        DefinitionLocation::Synthetic { section, offset } => checked_add(
            section_address(layout, section),
            offset,
            "synthetic symbol address",
        ),
        DefinitionLocation::Input { input, symbol } => {
            let object_symbol = inputs
                .get(input)
                .and_then(|value| value.image.symbols.get(symbol))
                .ok_or_else(|| link_error("global definition points outside input symbols"))?;
            let SymbolDefinition::Defined { section, value } = object_symbol.definition else {
                return Err(link_error("global definition unexpectedly has no section"));
            };
            let placement = merged
                .placements
                .get(&(input, section))
                .copied()
                .ok_or_else(|| link_error("missing definition section placement"))?;
            checked_add(
                section_address(layout, section),
                checked_add(placement, value, "definition section offset")?,
                "definition address",
            )
        }
    }
}

fn apply_relocation_value(
    section: &mut [u8],
    offset: u32,
    kind: RelocationKind,
    symbol: u32,
    addend: i32,
    place: u32,
    symbol_name: &str,
) -> Result<(), Diagnostic> {
    let value = match kind {
        RelocationKind::Abs32 => {
            let value = i64::from(symbol) + i64::from(addend);
            u32::try_from(value).map_err(|_| {
                relocation_range_error(format!(
                    "R_386_32 for '{symbol_name}' computes {value}, outside u32"
                ))
            })?
        }
        RelocationKind::Pc32 => {
            let value = i64::from(symbol) + i64::from(addend) - i64::from(place);
            let displacement = i32::try_from(value).map_err(|_| {
                relocation_range_error(format!(
                    "R_386_PC32 for '{symbol_name}' computes {value}, outside i32"
                ))
            })?;
            u32::from_le_bytes(displacement.to_le_bytes())
        }
    };
    let end = offset
        .checked_add(4)
        .ok_or_else(|| invalid_relocation("relocation write range overflows"))?;
    section
        .get_mut(offset as usize..end as usize)
        .ok_or_else(|| invalid_relocation("relocation write exceeds merged section"))?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn section_address(layout: &ElfLayout, section: ObjectSection) -> u32 {
    match section {
        ObjectSection::Text => layout.text_address,
        ObjectSection::Rodata => layout.rodata_address,
        ObjectSection::Data => layout.data_address,
        ObjectSection::Bss => layout.bss_address,
    }
}

fn section_bytes_mut(
    image: &mut LinkedExecutableImage,
    section: ObjectSection,
) -> Result<&mut [u8], Diagnostic> {
    match section {
        ObjectSection::Text => Ok(&mut image.text),
        ObjectSection::Rodata => Ok(&mut image.rodata),
        ObjectSection::Data => Ok(&mut image.data),
        ObjectSection::Bss => Err(invalid_relocation("cannot relocate .bss bytes")),
    }
}

fn link_dump(
    inputs: &[ParsedInput],
    merged: &MergedImage,
    definitions: &BTreeMap<String, GlobalDefinition>,
) -> String {
    let mut dump = String::new();
    let _ = writeln!(dump, "DBYTE STATIC LINK ELF32 EM_386");
    for (index, input) in inputs.iter().enumerate() {
        let _ = writeln!(dump, "input {index} {}", input.name);
        for section in ObjectSection::ALL {
            if let Some(offset) = merged.placements.get(&(index, section)) {
                let _ = writeln!(
                    dump,
                    "placement {} {}+0x{:08x}",
                    input.name,
                    section.name(),
                    offset
                );
            }
        }
    }
    let _ = writeln!(
        dump,
        "bootstrap .text+0x00000000 size={} entry={} stack={}",
        merged.bootstrap.size, LINK_ENTRY_SYMBOL, LINK_STACK_TOP_SYMBOL
    );
    let _ = writeln!(
        dump,
        "stack .bss+0x{:08x}..0x{:08x}",
        merged.executable.stack_bottom_offset, merged.executable.stack_top_offset
    );
    for (name, definition) in definitions {
        let _ = writeln!(
            dump,
            "global {name} {:?} {:?}",
            definition.kind, definition.location
        );
    }
    dump
}

fn canonical_input_name(name: &str) -> Result<String, Diagnostic> {
    if name.as_bytes().contains(&0) {
        return Err(link_error("linker input name contains NUL"));
    }
    let normalized = name.replace('\\', "/");
    let leaf = normalized
        .rsplit('/')
        .find(|component| !component.is_empty())
        .ok_or_else(|| link_error("linker input name is empty"))?;
    if leaf == "." || leaf == ".." {
        return Err(link_error("linker input name has no stable leaf component"));
    }
    Ok(leaf.to_owned())
}

fn align_buffer(bytes: &mut Vec<u8>, alignment: u32, context: &str) -> Result<(), Diagnostic> {
    let current = usize_u32(bytes.len(), context)?;
    let aligned = align_u32(current, alignment, context)?;
    if aligned as usize > MAX_ELF_SIZE {
        return Err(layout_error(format!("{context} exceeds ELF size bound")));
    }
    bytes.resize(aligned as usize, 0);
    Ok(())
}

fn extend_bounded(bytes: &mut Vec<u8>, source: &[u8], context: &str) -> Result<(), Diagnostic> {
    let length = bytes
        .len()
        .checked_add(source.len())
        .ok_or_else(|| layout_error(format!("{context} size overflow")))?;
    if length > MAX_ELF_SIZE {
        return Err(layout_error(format!("{context} exceeds ELF size bound")));
    }
    bytes.extend_from_slice(source);
    Ok(())
}

fn align_u32(value: u32, alignment: u32, context: &str) -> Result<u32, Diagnostic> {
    if alignment == 0 || !alignment.is_power_of_two() || alignment > 4096 {
        return Err(layout_error(format!(
            "invalid {context} alignment {alignment}"
        )));
    }
    value
        .checked_add(alignment - 1)
        .map(|sum| sum & !(alignment - 1))
        .ok_or_else(|| layout_error(format!("{context} alignment overflow")))
}

fn checked_add(left: u32, right: u32, context: &str) -> Result<u32, Diagnostic> {
    left.checked_add(right)
        .ok_or_else(|| layout_error(format!("{context} overflow")))
}

fn usize_u32(value: usize, context: &str) -> Result<u32, Diagnostic> {
    u32::try_from(value).map_err(|_| layout_error(format!("{context} exceeds u32")))
}

fn map_elf_layout_error(error: Diagnostic) -> Diagnostic {
    if error.category == DiagnosticCategory::Elf
        && (error.message.contains("overflow") || error.message.contains("exceeds bound"))
    {
        layout_error(error.message)
    } else {
        error
    }
}

fn encoder_error(error: impl std::fmt::Display) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::Encoder, error.to_string())
}

fn link_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::Link, message)
}

fn invalid_object(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidObject, message)
}

fn invalid_relocation(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidRelocation, message)
}

fn relocation_range_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::RelocationOutOfRange, message)
}

fn layout_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::LayoutOverflow, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{write_archive, ArchiveMember};
    use crate::object::{ObjectSectionData, ObjectSymbol, SymbolId};

    fn u16_at(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn section_range(bytes: &[u8], wanted: &str) -> (usize, usize, usize) {
        let shoff = u32_at(bytes, 32) as usize;
        let shentsize = u16_at(bytes, 46) as usize;
        let shnum = u16_at(bytes, 48) as usize;
        let shstrndx = u16_at(bytes, 50) as usize;
        let shstr_header = shoff + shstrndx * shentsize;
        let shstr_offset = u32_at(bytes, shstr_header + 16) as usize;
        let shstr_size = u32_at(bytes, shstr_header + 20) as usize;
        let shstr = &bytes[shstr_offset..shstr_offset + shstr_size];
        for index in 0..shnum {
            let header = shoff + index * shentsize;
            let name_offset = u32_at(bytes, header) as usize;
            let tail = &shstr[name_offset..];
            let length = tail.iter().position(|byte| *byte == 0).unwrap();
            if std::str::from_utf8(&tail[..length]).unwrap() == wanted {
                return (
                    header,
                    u32_at(bytes, header + 16) as usize,
                    u32_at(bytes, header + 20) as usize,
                );
            }
        }
        panic!("missing section {wanted}")
    }

    fn function_object(name: &str, text: Vec<u8>) -> ObjectImage {
        let mut image = ObjectImage::empty();
        image.sections.insert(
            ObjectSection::Text,
            ObjectSectionData::bytes(text.clone(), 16),
        );
        image.symbols.push(ObjectSymbol {
            name: name.to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: text.len() as u32,
        });
        image
    }

    fn multiboot_entry_object() -> ObjectImage {
        let mut image = function_object(LINK_ENTRY_SYMBOL, vec![0xc3]);
        image.symbols.push(ObjectSymbol {
            name: "__dbyte_entry:multiboot_v1".to_owned(),
            binding: SymbolBinding::Local,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: 0,
        });
        image
    }

    fn caller_and_callee() -> Vec<(String, ObjectImage)> {
        let mut caller =
            function_object(LINK_ENTRY_SYMBOL, vec![0xe8, 0xfc, 0xff, 0xff, 0xff, 0xc3]);
        caller.symbols.push(ObjectSymbol {
            name: "helper".to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Undefined,
            size: 0,
        });
        caller.relocations.push(ObjectRelocation {
            section: ObjectSection::Text,
            offset: 1,
            symbol: SymbolId(1),
            kind: RelocationKind::Pc32,
            addend: -4,
        });
        let callee = function_object("helper", vec![0xb8, 42, 0, 0, 0, 0xc3]);
        vec![
            ("z-caller.o".to_owned(), caller),
            ("a-callee.o".to_owned(), callee),
        ]
    }

    fn calling_function(name: &str, target: &str) -> ObjectImage {
        let mut caller = function_object(name, vec![0xe8, 0xfc, 0xff, 0xff, 0xff, 0xc3]);
        caller.symbols.push(ObjectSymbol {
            name: target.to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Undefined,
            size: 0,
        });
        caller.relocations.push(ObjectRelocation {
            section: ObjectSection::Text,
            offset: 1,
            symbol: SymbolId(1),
            kind: RelocationKind::Pc32,
            addend: -4,
        });
        caller
    }

    #[test]
    fn links_one_two_and_three_object_programs() {
        let one = link_images(&[(
            "kernel.o".to_owned(),
            function_object(LINK_ENTRY_SYMBOL, vec![0xc3]),
        )])
        .unwrap();
        assert!(one.dump.contains("input 0 kernel.o"));

        let two = link_images(&caller_and_callee()).unwrap();
        assert!(two.dump.contains("input 1 z-caller.o"));

        let three = link_images(&[
            (
                "kernel.o".to_owned(),
                calling_function(LINK_ENTRY_SYMBOL, "middle"),
            ),
            ("middle.o".to_owned(), calling_function("middle", "leaf")),
            ("leaf.o".to_owned(), function_object("leaf", vec![0xc3])),
        ])
        .unwrap();
        assert!(three.dump.contains("Pc32 middle"));
        assert!(three.dump.contains("Pc32 leaf"));
    }

    #[test]
    fn archives_extract_only_members_needed_by_unresolved_symbols() {
        let caller = calling_function(LINK_ENTRY_SYMBOL, "needed");
        let caller = write_relocatable(&caller).unwrap().bytes;
        let needed = write_relocatable(&function_object("needed", vec![0xc3]))
            .unwrap()
            .bytes;
        let unused = write_relocatable(&function_object(LINK_ENTRY_SYMBOL, vec![0xc3]))
            .unwrap()
            .bytes;
        let archive = write_archive(&[
            ArchiveMember {
                name: "need.o".into(),
                bytes: needed,
            },
            ArchiveMember {
                name: "unused.o".into(),
                bytes: unused,
            },
        ])
        .unwrap();
        let linked = link_object_bytes(&[
            LinkInput::new("main.o", caller),
            LinkInput::new("libproof.a", archive),
        ])
        .unwrap();
        assert!(linked.dump.contains("needed"));
        assert!(!linked.dump.contains("unused.o"));
    }

    #[test]
    fn links_cross_object_pc32_and_synthesizes_bootstrap() {
        let artifact = link_images(&caller_and_callee()).unwrap();
        assert_eq!(&artifact.elf[..7], b"\x7fELF\x01\x01\x01");
        assert_eq!(u16::from_le_bytes([artifact.elf[16], artifact.elf[17]]), 2);
        assert_eq!(artifact.layout.entry, artifact.layout.text_address);
        assert_eq!(artifact.machine_code[0], 0xfa);
        assert_eq!(artifact.machine_code[1], 0xbc);
        assert_eq!(
            artifact.layout.stack_top - artifact.layout.stack_bottom,
            BOOTSTRAP_STACK_SIZE
        );
        assert!(artifact.dump.contains("Pc32 helper"));
        assert!(artifact.dump.contains("bootstrap .text"));

        // Canonical order puts a-callee at text+32 and z-caller at text+48.
        // The caller's relocation field is therefore P=text+49 and computes
        // S + (-4) - P = -21, landing the call at text+32.
        assert_eq!(
            i32::from_le_bytes(artifact.machine_code[49..53].try_into().unwrap()),
            -21
        );
    }

    #[test]
    fn multiboot_entry_metadata_selects_two_argument_bootstrap() {
        let artifact = link_images(&[("kernel.o".to_owned(), multiboot_entry_object())]).unwrap();
        assert_eq!(artifact.machine_code[0], 0xfa);
        assert_eq!(artifact.machine_code[1], 0xbc);
        assert_eq!(&artifact.machine_code[6..10], &[0x31, 0xed, 0x53, 0x50]);
        assert_eq!(artifact.machine_code[10], 0xe8);
        assert_eq!(&artifact.machine_code[15..21], &[0x81, 0xc4, 0x08, 0, 0, 0]);
        assert!(!artifact.dump.contains("__dbyte_entry:"));

        let mut conflict = multiboot_entry_object();
        conflict.symbols.push(ObjectSymbol {
            name: "__dbyte_entry:legacy".to_owned(),
            binding: SymbolBinding::Local,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: 0,
        });
        let error = link_images(&[("kernel.o".to_owned(), conflict)]).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::Object);
        assert!(error.message.contains("conflicting native entry metadata"));
    }

    #[test]
    fn checked_task_metadata_synthesizes_exact_trampoline_and_requires_abs32() {
        let mut image = function_object(LINK_ENTRY_SYMBOL, vec![0; 9]);
        image.symbols.push(ObjectSymbol {
            name: "worker".to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 8,
            },
            size: 1,
        });
        image.symbols.push(ObjectSymbol {
            name: LINK_TASK_TRAMPOLINE_SYMBOL.to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Undefined,
            size: 0,
        });
        for name in [
            "__dbyte_task:def:worker",
            "__dbyte_task:use:worker",
            "__dbyte_task:trampoline",
        ] {
            image.symbols.push(ObjectSymbol {
                name: name.to_owned(),
                binding: SymbolBinding::Local,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 0,
            });
        }
        image.relocations.push(ObjectRelocation {
            section: ObjectSection::Text,
            offset: 0,
            symbol: SymbolId(1),
            kind: RelocationKind::Abs32,
            addend: 0,
        });
        image.relocations.push(ObjectRelocation {
            section: ObjectSection::Text,
            offset: 4,
            symbol: SymbolId(2),
            kind: RelocationKind::Abs32,
            addend: 0,
        });

        let artifact = link_images(&[("kernel.o".to_owned(), image.clone())]).unwrap();
        let (_, bootstrap) = build_bootstrap(EntryAbi::Legacy).unwrap();
        assert_eq!(
            &artifact.machine_code[bootstrap.size as usize..bootstrap.size as usize + 3],
            &[0xff, 0xd0, 0xc3]
        );
        assert!(artifact.dump.contains(LINK_TASK_TRAMPOLINE_SYMBOL));
        assert!(!artifact.dump.contains(".dbyte.tasks"));

        image.relocations.pop();
        let error = link_images(&[("kernel.o".to_owned(), image)]).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InvalidObject);
        assert!(error.message.contains("trampoline metadata lacks R_386_32"));
    }

    #[test]
    fn abs32_resolves_rodata_address_into_data() {
        let mut kernel = function_object(LINK_ENTRY_SYMBOL, vec![0xc3]);
        kernel
            .sections
            .insert(ObjectSection::Data, ObjectSectionData::bytes(vec![0; 4], 4));
        kernel.symbols.push(ObjectSymbol {
            name: "message".to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Object,
            definition: SymbolDefinition::Undefined,
            size: 0,
        });
        kernel.relocations.push(ObjectRelocation {
            section: ObjectSection::Data,
            offset: 0,
            symbol: SymbolId(1),
            kind: RelocationKind::Abs32,
            addend: 0,
        });

        let mut provider = ObjectImage::empty();
        provider.sections.insert(
            ObjectSection::Rodata,
            ObjectSectionData::bytes(b"X".to_vec(), 4),
        );
        provider.symbols.push(ObjectSymbol {
            name: "message".to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Object,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Rodata,
                value: 0,
            },
            size: 1,
        });
        let artifact = link_images(&[
            ("z-kernel.o".to_owned(), kernel),
            ("a-provider.o".to_owned(), provider),
        ])
        .unwrap();
        assert_eq!(
            u32_at(&artifact.elf, artifact.layout.data_offset as usize),
            artifact.layout.rodata_address
        );
        assert!(artifact.dump.contains("Abs32 message"));
    }

    #[test]
    fn bss_and_text_placements_honor_alignment_padding() {
        let mut first = ObjectImage::empty();
        first
            .sections
            .insert(ObjectSection::Text, ObjectSectionData::bytes(vec![0x90], 1));
        first
            .sections
            .insert(ObjectSection::Bss, ObjectSectionData::bss(1, 1));
        let mut kernel = function_object(LINK_ENTRY_SYMBOL, vec![0xc3]);
        kernel.sections.insert(
            ObjectSection::Text,
            ObjectSectionData::bytes(vec![0xc3], 32),
        );
        kernel
            .sections
            .insert(ObjectSection::Bss, ObjectSectionData::bss(4, 16));
        let artifact = link_images(&[
            ("a-first.o".to_owned(), first),
            ("z-kernel.o".to_owned(), kernel),
        ])
        .unwrap();
        assert!(artifact
            .dump
            .contains("placement z-kernel.o .text+0x00000020"));
        assert!(artifact
            .dump
            .contains("placement z-kernel.o .bss+0x00000010"));
        assert_eq!(
            artifact.layout.stack_bottom - artifact.layout.bss_address,
            32
        );
    }

    #[test]
    fn input_order_does_not_change_link_output() {
        let first_inputs = caller_and_callee();
        let mut second_inputs = first_inputs.clone();
        second_inputs.reverse();
        assert_eq!(
            link_images(&first_inputs).unwrap(),
            link_images(&second_inputs).unwrap()
        );
        let dump = &link_images(&first_inputs).unwrap().dump;
        assert!(
            dump.find("input 0 a-callee.o").unwrap() < dump.find("input 1 z-caller.o").unwrap()
        );
        assert!(
            dump.find("global __dbyte_stack_bottom").unwrap()
                < dump.find("global kernel_main").unwrap()
        );
    }

    #[test]
    fn duplicate_and_undefined_globals_have_stable_categories() {
        let kernel = function_object(LINK_ENTRY_SYMBOL, vec![0xc3]);
        let duplicate = function_object(LINK_ENTRY_SYMBOL, vec![0xc3]);
        let error = link_images(&[
            ("a.o".to_owned(), kernel.clone()),
            ("b.o".to_owned(), duplicate),
        ])
        .unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::DuplicateSymbol);

        let mut unresolved = kernel;
        unresolved.symbols.push(ObjectSymbol {
            name: "missing".to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Undefined,
            size: 0,
        });
        let error = link_images(&[("kernel.o".to_owned(), unresolved)]).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::UndefinedSymbol);
    }

    #[test]
    fn malformed_serialized_object_is_rejected_without_panicking() {
        let error = link_object_bytes(&[LinkInput::new("bad.o", vec![0; 52])]).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InvalidObject);
        assert_eq!(error.path.as_deref(), Some("bad.o"));
    }

    #[test]
    fn serialized_wrong_machine_and_bad_relocations_have_stable_categories() {
        let caller = write_relocatable(&caller_and_callee()[0].1).unwrap();

        let mut wrong_machine = caller.bytes.clone();
        wrong_machine[18..20].copy_from_slice(&62u16.to_le_bytes());
        let error = link_object_bytes(&[LinkInput::new("wrong.o", wrong_machine)]).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::UnsupportedMachine);

        let (_, rel_offset, _) = section_range(&caller.bytes, ".rel.text");
        let mut bad_type = caller.bytes.clone();
        let info = u32_at(&bad_type, rel_offset + 4);
        bad_type[rel_offset + 4..rel_offset + 8]
            .copy_from_slice(&((info & !0xff) | 99).to_le_bytes());
        assert_eq!(
            link_object_bytes(&[LinkInput::new("bad-type.o", bad_type)])
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidRelocation
        );

        let mut bad_offset = caller.bytes.clone();
        bad_offset[rel_offset..rel_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            link_object_bytes(&[LinkInput::new("bad-offset.o", bad_offset)])
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidRelocation
        );

        let mut bad_symbol = caller.bytes;
        bad_symbol[rel_offset + 4..rel_offset + 8]
            .copy_from_slice(&((0x00ff_ffffu32 << 8) | 2).to_le_bytes());
        assert_eq!(
            link_object_bytes(&[LinkInput::new("bad-symbol.o", bad_symbol)])
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidRelocation
        );
    }

    #[test]
    fn relocation_range_and_layout_overflow_have_stable_categories() {
        let mut bytes = [0u8; 4];
        let error =
            apply_relocation_value(&mut bytes, 0, RelocationKind::Pc32, u32::MAX, 0, 0, "far")
                .unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::RelocationOutOfRange);

        let error = align_u32(u32::MAX, 4096, "test layout").unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::LayoutOverflow);
    }

    #[test]
    fn user_link_is_profile_separated_and_emits_compact_two_segment_elf() {
        let mut user = function_object(LINK_USER_ENTRY_SYMBOL, vec![0xc3]);
        user.symbols.push(ObjectSymbol {
            name: "__dbyte_profile:user".to_owned(),
            binding: SymbolBinding::Local,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: 0,
        });
        let object = write_relocatable(&user).unwrap();
        let linked = link_user_object_bytes(&[LinkInput::new("user.o", object.bytes)]).unwrap();
        assert!(linked.elf.len() <= 4096);
        assert_eq!(u16_at(&linked.elf, 16), 2);
        assert_eq!(u16_at(&linked.elf, 18), 3);
        assert_eq!(u16_at(&linked.elf, 44), 2);
        assert_eq!(u16_at(&linked.elf, 48), 0);
        assert_eq!(u32_at(&linked.elf, 52 + 24), 5);
        assert_eq!(u32_at(&linked.elf, 52 + 32 + 24), 6);
        assert!(linked.elf.windows(2).any(|bytes| bytes == [0xcd, 0x80]));

        let user_object = write_relocatable(&user).unwrap();
        assert_eq!(
            link_object_bytes(&[LinkInput::new("user.o", user_object.bytes)])
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidObject
        );
        let kernel = write_relocatable(&function_object(LINK_ENTRY_SYMBOL, vec![0xc3])).unwrap();
        assert_eq!(
            link_user_object_bytes(&[LinkInput::new("kernel.o", kernel.bytes)])
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidObject
        );
    }
}
