//! Deterministic in-process linker core for canonical DBYTE64 ELF objects.

use crate::object64::{
    parse_relocatable, Binding, Contents, Definition, ObjectImage, RelocationKind, Section,
};
use crate::target::{validate_link_targets, TargetSpec};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionAddresses {
    pub text: u64,
    pub rodata: u64,
    pub data: u64,
    pub bss: u64,
}

impl SectionAddresses {
    fn get(self, section: Section) -> u64 {
        match section {
            Section::Text => self.text,
            Section::Rodata => self.rodata,
            Section::Data => self.data,
            Section::Bss => self.bss,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedSections {
    pub target: TargetSpec,
    pub entry_abi: Option<crate::object64::EntryAbi>,
    pub text: Vec<u8>,
    pub rodata: Vec<u8>,
    pub data: Vec<u8>,
    pub bss_size: u64,
    pub entry: u64,
    pub symbols: BTreeMap<String, u64>,
    pub relocation_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError(pub String);

impl std::fmt::Display for LinkError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LinkError {}

#[derive(Debug, Clone)]
struct InputObject {
    name: String,
    image: ObjectImage,
}

pub fn link_objects(
    inputs: &[LinkInput],
    requested: TargetSpec,
    addresses: SectionAddresses,
    entry_symbol: &str,
) -> Result<LinkedSections, LinkError> {
    if inputs.is_empty() {
        return Err(error("E_LINK64_INPUT", "link requires at least one object"));
    }
    let mut ordered: Vec<_> = inputs.iter().collect();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in ordered.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(error("E_LINK64_INPUT", "input names must be unique"));
        }
    }
    let objects = parse_inputs_and_archives(&ordered, requested, entry_symbol)?;
    validate_link_targets(objects.iter().map(|object| object.image.target), requested)
        .map_err(|error| LinkError(error.0))?;
    validate_interrupt_metadata(&objects)?;
    let entries: Vec<_> = objects
        .iter()
        .filter_map(|object| object.image.entry_abi)
        .collect();
    if entries.len() > 1 {
        return Err(error(
            "E_LINK64_ENTRY_METADATA",
            "multiple objects define .dbyte.entry",
        ));
    }
    let entry_abi = entries.first().copied();

    let mut merged = BTreeMap::<Section, Vec<u8>>::new();
    let mut bss_size = 0u64;
    let mut placements = BTreeMap::<(usize, Section), u64>::new();
    for (object_index, object) in objects.iter().enumerate() {
        for section in Section::ALL {
            let Some(data) = object.image.sections.get(&section) else {
                placements.insert(
                    (object_index, section),
                    if section == Section::Bss {
                        bss_size
                    } else {
                        merged.get(&section).map_or(0, |bytes| bytes.len() as u64)
                    },
                );
                continue;
            };
            if section == Section::Bss {
                bss_size = align_up(bss_size, data.alignment)?;
                placements.insert((object_index, section), bss_size);
                let Contents::Uninitialized(size) = data.contents else {
                    return Err(error("E_LINK64_SECTION", "BSS must be uninitialized"));
                };
                bss_size = bss_size
                    .checked_add(size)
                    .ok_or_else(|| error("E_LINK64_OVERFLOW", "BSS merge overflows"))?;
            } else {
                let output = merged.entry(section).or_default();
                let aligned = align_up(output.len() as u64, data.alignment)?;
                let aligned = usize::try_from(aligned)
                    .map_err(|_| error("E_LINK64_SIZE", "section exceeds host addressability"))?;
                output.resize(aligned, 0);
                placements.insert((object_index, section), aligned as u64);
                let Contents::Bytes(bytes) = &data.contents else {
                    return Err(error("E_LINK64_SECTION", "file section is uninitialized"));
                };
                output.extend_from_slice(bytes);
            }
        }
    }

    let mut global_definitions = BTreeMap::<String, (usize, Section, u64)>::new();
    for (object_index, object) in objects.iter().enumerate() {
        for symbol in &object.image.symbols {
            if symbol.binding != Binding::Global {
                continue;
            }
            if let Definition::Defined { section, value } = symbol.definition {
                if global_definitions
                    .insert(symbol.name.clone(), (object_index, section, value))
                    .is_some()
                {
                    return Err(error(
                        "E_LINK64_DUPLICATE_SYMBOL",
                        &format!("duplicate strong symbol `{}`", symbol.name),
                    ));
                }
            }
        }
    }
    let synthetic_symbols = kernel_synthetic_symbols(requested, addresses, bss_size)?;
    for object in &objects {
        for symbol in &object.image.symbols {
            if synthetic_symbols.contains_key(&symbol.name)
                && symbol.definition != Definition::Undefined
            {
                return Err(error(
                    "E_LINK64_RESERVED_SYMBOL",
                    &format!("input defines reserved linker symbol `{}`", symbol.name),
                ));
            }
        }
    }
    let mut public_symbols = synthetic_symbols.clone();
    for (name, (object_index, section, value)) in &global_definitions {
        public_symbols.insert(
            name.clone(),
            symbol_address(*object_index, *section, *value, &placements, addresses)?,
        );
    }
    let entry = public_symbols.get(entry_symbol).copied().ok_or_else(|| {
        error(
            "E_LINK64_ENTRY",
            &format!("missing entry symbol `{entry_symbol}`"),
        )
    })?;

    let mut relocation_count = 0usize;
    for (object_index, object) in objects.iter().enumerate() {
        for relocation in &object.image.relocations {
            let symbol = resolve_symbol(
                object_index,
                &relocation.symbol,
                &objects,
                &synthetic_symbols,
                &global_definitions,
                &placements,
                addresses,
            )?;
            let placement = placements[&(object_index, relocation.section)];
            let patch_offset = placement
                .checked_add(relocation.offset)
                .ok_or_else(|| error("E_LINK64_OVERFLOW", "relocation offset overflows"))?;
            let place = addresses
                .get(relocation.section)
                .checked_add(patch_offset)
                .ok_or_else(|| error("E_LINK64_OVERFLOW", "relocation place overflows"))?;
            let output = merged
                .get_mut(&relocation.section)
                .ok_or_else(|| error("E_LINK64_RELOCATION", "relocation section is absent"))?;
            apply_relocation(
                output,
                patch_offset,
                relocation.kind,
                symbol,
                relocation.addend,
                place,
            )?;
            relocation_count += 1;
        }
    }
    let undefined: BTreeSet<_> = objects
        .iter()
        .flat_map(|object| &object.image.symbols)
        .filter(|symbol| {
            symbol.binding == Binding::Global
                && symbol.definition == Definition::Undefined
                && !synthetic_symbols.contains_key(&symbol.name)
                && !global_definitions.contains_key(&symbol.name)
        })
        .map(|symbol| symbol.name.clone())
        .collect();
    if let Some(name) = undefined.first() {
        return Err(error(
            "E_LINK64_UNDEFINED_SYMBOL",
            &format!("undefined symbol `{name}`"),
        ));
    }

    Ok(LinkedSections {
        target: requested,
        entry_abi,
        text: merged.remove(&Section::Text).unwrap_or_default(),
        rodata: merged.remove(&Section::Rodata).unwrap_or_default(),
        data: merged.remove(&Section::Data).unwrap_or_default(),
        bss_size,
        entry,
        symbols: public_symbols,
        relocation_count,
    })
}

fn parse_inputs_and_archives(
    inputs: &[&LinkInput],
    requested: TargetSpec,
    entry_symbol: &str,
) -> Result<Vec<InputObject>, LinkError> {
    let mut objects = Vec::new();
    let mut archives = Vec::<(String, Vec<InputObject>)>::new();
    for input in inputs {
        if input.bytes.starts_with(b"!<arch>\n") {
            let members = crate::archive::parse_archive(&input.bytes)
                .map_err(|failure| LinkError(failure.to_string()))?;
            let mut parsed = Vec::with_capacity(members.len());
            for member in members {
                let image = parse_input_object(&member.bytes, requested).map_err(|failure| {
                    error(
                        "E_LINK64_OBJECT",
                        &format!("{}({}): {}", input.name, member.name, failure),
                    )
                })?;
                parsed.push(InputObject {
                    name: format!("{}({})", input.name, member.name),
                    image,
                });
            }
            parsed.sort_by(|left, right| left.name.cmp(&right.name));
            archives.push((input.name.clone(), parsed));
        } else {
            let image = parse_input_object(&input.bytes, requested)?;
            objects.push(InputObject {
                name: input.name.clone(),
                image,
            });
        }
    }
    archives.sort_by(|left, right| left.0.cmp(&right.0));
    let mut selected = BTreeSet::<(usize, usize)>::new();
    loop {
        let mut defined = BTreeSet::new();
        let mut unresolved = BTreeSet::from([entry_symbol.to_owned()]);
        for object in &objects {
            collect_symbol_sets(&object.image, &mut defined, &mut unresolved);
        }
        unresolved.retain(|symbol| !defined.contains(symbol));
        let mut changed = false;
        for (archive_index, (_, members)) in archives.iter().enumerate() {
            for (member_index, member) in members.iter().enumerate() {
                if selected.contains(&(archive_index, member_index)) {
                    continue;
                }
                let satisfies = member.image.symbols.iter().any(|symbol| {
                    symbol.binding == Binding::Global
                        && matches!(symbol.definition, Definition::Defined { .. })
                        && unresolved.contains(&symbol.name)
                });
                if satisfies {
                    selected.insert((archive_index, member_index));
                    objects.push(member.clone());
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
    Ok(objects)
}

fn parse_input_object(bytes: &[u8], requested: TargetSpec) -> Result<ObjectImage, LinkError> {
    if requested == crate::target::X86_64_WINDOWS {
        return match crate::coff64::parse_object(bytes) {
            Ok(image) => Ok(image),
            Err(_canonical)
                if !bytes
                    .windows(b".dbyte.".len())
                    .any(|window| window == b".dbyte.") =>
            {
                crate::external_coff64::parse_external_amd64_object(bytes)
                    .map_err(|failure| error("E_LINK64_EXTERNAL_OBJECT", &failure.to_string()))
            }
            Err(canonical) => Err(error("E_LINK64_COFF_OBJECT", &canonical.to_string())),
        };
    }
    match parse_relocatable(bytes) {
        Ok(image) => Ok(image),
        Err(_canonical)
            if requested == crate::target::X86_64_LINUX
                && !bytes
                    .windows(b".dbyte.".len())
                    .any(|window| window == b".dbyte.") =>
        {
            crate::external_elf64::parse_external_x86_64_object(bytes, requested)
                .map_err(|failure| error("E_LINK64_EXTERNAL_OBJECT", &failure.to_string()))
        }
        Err(canonical) => Err(error("E_LINK64_OBJECT", &canonical.to_string())),
    }
}

fn collect_symbol_sets(
    image: &ObjectImage,
    defined: &mut BTreeSet<String>,
    unresolved: &mut BTreeSet<String>,
) {
    for symbol in &image.symbols {
        if symbol.binding != Binding::Global {
            continue;
        }
        match symbol.definition {
            Definition::Undefined => {
                unresolved.insert(symbol.name.clone());
            }
            Definition::Defined { .. } => {
                defined.insert(symbol.name.clone());
            }
        }
    }
}

fn validate_interrupt_metadata(inputs: &[InputObject]) -> Result<(), LinkError> {
    let mut definitions = BTreeSet::new();
    let mut uses = Vec::new();
    for input in inputs {
        for symbol in &input.image.symbols {
            let Some(record) = symbol.name.strip_prefix("__dbyte_interrupt:") else {
                continue;
            };
            if symbol.binding != Binding::Local
                || symbol.kind != crate::object64::SymbolKind::Function
                || symbol.definition == Definition::Undefined
            {
                return Err(error(
                    "E_LINK64_INTERRUPT_METADATA",
                    &format!("invalid interrupt metadata symbol '{}'", symbol.name),
                ));
            }
            let parts: Vec<&str> = record.split(':').collect();
            match parts.as_slice() {
                ["def", class, handler] if valid_interrupt_class(class) && !handler.is_empty() => {
                    if !definitions.insert((class.to_string(), handler.to_string())) {
                        return Err(error(
                            "E_LINK64_INTERRUPT_DUPLICATE",
                            &format!("duplicate interrupt definition metadata for {handler}"),
                        ));
                    }
                }
                ["use", vector, class, handler]
                    if valid_interrupt_class(class) && !handler.is_empty() =>
                {
                    let vector: u8 = vector.parse().map_err(|_| {
                        error(
                            "E_LINK64_INTERRUPT_METADATA",
                            &format!("invalid interrupt vector metadata '{}'", symbol.name),
                        )
                    })?;
                    let requires_error = matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17);
                    let class_error = class.starts_with("error_code");
                    let class_transition = class.ends_with(",cpl3_transition");
                    if class_error != requires_error || (vector == 128 && !class_transition) {
                        return Err(error(
                            "E_LINK64_INTERRUPT_CLASS",
                            &format!("interrupt vector {vector} is incompatible with {class} handler '{handler}'"),
                        ));
                    }
                    uses.push((class.to_string(), handler.to_string(), vector));
                }
                _ => {
                    return Err(error(
                        "E_LINK64_INTERRUPT_METADATA",
                        &format!("malformed interrupt metadata symbol '{}'", symbol.name),
                    ));
                }
            }
        }
    }
    for (class, handler, vector) in uses {
        if !definitions.contains(&(class.clone(), handler.clone())) {
            return Err(error(
                "E_LINK64_INTERRUPT_UNDEFINED",
                &format!(
                    "interrupt vector {vector} references {class} handler '{handler}' without matching metadata"
                ),
            ));
        }
    }
    Ok(())
}

fn valid_interrupt_class(class: &str) -> bool {
    matches!(
        class,
        "no_error_code"
            | "error_code"
            | "no_error_code,cpl3_transition"
            | "error_code,cpl3_transition"
    )
}

fn kernel_synthetic_symbols(
    requested: TargetSpec,
    addresses: SectionAddresses,
    bss_size: u64,
) -> Result<BTreeMap<String, u64>, LinkError> {
    let mut symbols = BTreeMap::new();
    if requested.executable_format != crate::target::ExecutableFormat::Multiboot2Elf64 {
        return Ok(symbols);
    }
    let image_end = addresses
        .bss
        .checked_add(bss_size)
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "kernel image end overflows"))?;
    symbols.insert(
        "__dbyte_image_start".to_owned(),
        u64::from(crate::multiboot2::IMAGE_BASE),
    );
    symbols.insert("__dbyte_image_end".to_owned(), image_end);
    Ok(symbols)
}

fn resolve_symbol(
    object_index: usize,
    name: &str,
    objects: &[InputObject],
    synthetic: &BTreeMap<String, u64>,
    globals: &BTreeMap<String, (usize, Section, u64)>,
    placements: &BTreeMap<(usize, Section), u64>,
    addresses: SectionAddresses,
) -> Result<u64, LinkError> {
    if let Some(symbol) = objects[object_index]
        .image
        .symbols
        .iter()
        .find(|symbol| symbol.name == name && symbol.binding == Binding::Local)
    {
        if let Definition::Defined { section, value } = symbol.definition {
            return symbol_address(object_index, section, value, placements, addresses);
        }
    }
    if let Some(address) = synthetic.get(name).copied() {
        return Ok(address);
    }
    let (owner, section, value) = globals.get(name).copied().ok_or_else(|| {
        error(
            "E_LINK64_UNDEFINED_SYMBOL",
            &format!(
                "undefined symbol `{name}` referenced by `{}`",
                objects[object_index].name
            ),
        )
    })?;
    symbol_address(owner, section, value, placements, addresses)
}

fn symbol_address(
    object: usize,
    section: Section,
    value: u64,
    placements: &BTreeMap<(usize, Section), u64>,
    addresses: SectionAddresses,
) -> Result<u64, LinkError> {
    addresses
        .get(section)
        .checked_add(placements[&(object, section)])
        .and_then(|base| base.checked_add(value))
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "symbol address overflows"))
}

fn apply_relocation(
    output: &mut [u8],
    offset: u64,
    kind: RelocationKind,
    symbol: u64,
    addend: i64,
    place: u64,
) -> Result<(), LinkError> {
    let value = i128::from(symbol) + i128::from(addend);
    match kind {
        RelocationKind::Absolute64 => {
            let value = u64::try_from(value)
                .map_err(|_| error("E_LINK64_RELOCATION_OVERFLOW", "R_X86_64_64 overflows"))?;
            patch(output, offset, &value.to_le_bytes())
        }
        RelocationKind::Pc32 | RelocationKind::Plt32 => {
            let value = value - i128::from(place);
            let value = i32::try_from(value)
                .map_err(|_| error("E_LINK64_RELOCATION_OVERFLOW", "PC32 relocation overflows"))?;
            patch(output, offset, &value.to_le_bytes())
        }
        RelocationKind::Absolute32 => {
            let value = u32::try_from(value)
                .map_err(|_| error("E_LINK64_RELOCATION_OVERFLOW", "R_X86_64_32 overflows"))?;
            patch(output, offset, &value.to_le_bytes())
        }
        RelocationKind::Absolute32Signed => {
            let value = i32::try_from(value)
                .map_err(|_| error("E_LINK64_RELOCATION_OVERFLOW", "R_X86_64_32S overflows"))?;
            patch(output, offset, &value.to_le_bytes())
        }
    }
}

fn patch(output: &mut [u8], offset: u64, bytes: &[u8]) -> Result<(), LinkError> {
    let start = usize::try_from(offset)
        .map_err(|_| error("E_LINK64_RELOCATION_RANGE", "patch offset exceeds host"))?;
    let end = start
        .checked_add(bytes.len())
        .ok_or_else(|| error("E_LINK64_RELOCATION_RANGE", "patch range overflows"))?;
    let target = output
        .get_mut(start..end)
        .ok_or_else(|| error("E_LINK64_RELOCATION_RANGE", "patch exceeds section"))?;
    target.copy_from_slice(bytes);
    Ok(())
}

fn align_up(value: u64, alignment: u64) -> Result<u64, LinkError> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(error("E_LINK64_ALIGNMENT", "invalid section alignment"));
    }
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "section alignment overflows"))
}

fn error(code: &str, message: &str) -> LinkError {
    LinkError(format!("{code}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object64::{
        write_relocatable, Binding, Contents, Definition, ObjectImage, Relocation, SectionData,
        Symbol, SymbolKind,
    };
    use crate::target::{X86_64_LINUX, X86_64_NONE};

    fn object(
        target: TargetSpec,
        name: &str,
        text: Vec<u8>,
        symbols: Vec<Symbol>,
        relocations: Vec<Relocation>,
    ) -> LinkInput {
        let image = ObjectImage {
            target,
            entry_abi: None,
            sections: BTreeMap::from([(
                Section::Text,
                SectionData {
                    contents: Contents::Bytes(text),
                    alignment: 16,
                },
            )]),
            symbols,
            relocations,
        };
        LinkInput::new(name, write_relocatable(&image).unwrap().bytes)
    }

    #[test]
    fn links_cross_object_plt32_and_rejects_mixed_targets() {
        let caller = object(
            X86_64_NONE,
            "a.o",
            vec![0xe8, 0, 0, 0, 0, 0xc3],
            vec![
                Symbol {
                    name: "kernel_main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 6,
                },
                Symbol {
                    name: "callee".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            vec![Relocation {
                section: Section::Text,
                offset: 1,
                symbol: "callee".into(),
                kind: RelocationKind::Plt32,
                addend: -4,
            }],
        );
        let callee = object(
            X86_64_NONE,
            "b.o",
            vec![0xc3],
            vec![Symbol {
                name: "callee".into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Defined {
                    section: Section::Text,
                    value: 0,
                },
                size: 1,
            }],
            vec![],
        );
        let addresses = SectionAddresses {
            text: 0x200035,
            rodata: 0x210000,
            data: 0x220000,
            bss: 0x230000,
        };
        let linked = link_objects(
            &[callee.clone(), caller],
            X86_64_NONE,
            addresses,
            "kernel_main",
        )
        .unwrap();
        assert_eq!(&linked.text[1..5], &11i32.to_le_bytes());
        let mixed = LinkInput {
            name: "linux.o".into(),
            bytes: {
                let parsed = parse_relocatable(&callee.bytes).unwrap();
                let mut image = parsed;
                image.target = X86_64_LINUX;
                write_relocatable(&image).unwrap().bytes
            },
        };
        assert!(
            link_objects(&[callee, mixed], X86_64_NONE, addresses, "callee")
                .unwrap_err()
                .0
                .contains("E_TARGET_MISMATCH")
        );
    }

    #[test]
    fn interrupt_metadata_requires_matching_definition() {
        let input = object(
            X86_64_NONE,
            "irq.o",
            vec![0xc3],
            vec![
                Symbol {
                    name: "kernel_main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 1,
                },
                Symbol {
                    name: "__dbyte_interrupt:use:003:no_error_code:breakpoint".into(),
                    binding: Binding::Local,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 0,
                },
            ],
            vec![],
        );
        let addresses = SectionAddresses {
            text: 0x200000,
            rodata: 0x210000,
            data: 0x220000,
            bss: 0x230000,
        };
        let error = link_objects(&[input], X86_64_NONE, addresses, "kernel_main")
            .expect_err("interrupt use without a matching definition must fail");
        assert!(
            error.0.contains("E_LINK64_INTERRUPT_UNDEFINED"),
            "{}",
            error.0
        );
    }

    #[test]
    fn interrupt_metadata_accepts_matching_definition() {
        let input = object(
            X86_64_NONE,
            "irq-ok.o",
            vec![0xc3],
            vec![
                Symbol {
                    name: "kernel_main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 1,
                },
                Symbol {
                    name: "__dbyte_interrupt:def:no_error_code:breakpoint".into(),
                    binding: Binding::Local,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 0,
                },
                Symbol {
                    name: "__dbyte_interrupt:use:003:no_error_code:breakpoint".into(),
                    binding: Binding::Local,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 0,
                },
            ],
            vec![],
        );
        let addresses = SectionAddresses {
            text: 0x200000,
            rodata: 0x210000,
            data: 0x220000,
            bss: 0x230000,
        };
        link_objects(&[input], X86_64_NONE, addresses, "kernel_main").unwrap();
    }

    #[test]
    fn interrupt_metadata_rejects_vector_class_mismatch() {
        let input = object(
            X86_64_NONE,
            "irq-class.o",
            vec![0xc3],
            vec![
                Symbol {
                    name: "kernel_main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 1,
                },
                Symbol {
                    name: "__dbyte_interrupt:def:no_error_code:page_fault".into(),
                    binding: Binding::Local,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 0,
                },
                Symbol {
                    name: "__dbyte_interrupt:use:014:no_error_code:page_fault".into(),
                    binding: Binding::Local,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 0,
                },
            ],
            vec![],
        );
        let addresses = SectionAddresses {
            text: 0x200000,
            rodata: 0x210000,
            data: 0x220000,
            bss: 0x230000,
        };
        let error = link_objects(&[input], X86_64_NONE, addresses, "kernel_main")
            .expect_err("page fault must use an error-code interrupt class");
        assert!(error.0.contains("E_LINK64_INTERRUPT_CLASS"), "{}", error.0);
    }

    #[test]
    fn none_kernel_image_synthetic_symbols_resolve() {
        let input = object(
            X86_64_NONE,
            "image-symbols.o",
            vec![0; 16],
            vec![
                Symbol {
                    name: "kernel_main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 16,
                },
                Symbol {
                    name: "__dbyte_image_start".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Object,
                    definition: Definition::Undefined,
                    size: 0,
                },
                Symbol {
                    name: "__dbyte_image_end".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Object,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            vec![
                Relocation {
                    section: Section::Text,
                    offset: 0,
                    symbol: "__dbyte_image_start".into(),
                    kind: RelocationKind::Absolute64,
                    addend: 0,
                },
                Relocation {
                    section: Section::Text,
                    offset: 8,
                    symbol: "__dbyte_image_end".into(),
                    kind: RelocationKind::Absolute64,
                    addend: 0,
                },
            ],
        );
        let addresses = SectionAddresses {
            text: 0x200035,
            rodata: 0x210000,
            data: 0x220000,
            bss: 0x230000,
        };
        let linked = link_objects(&[input], X86_64_NONE, addresses, "kernel_main").unwrap();
        let image_start = u64::from(crate::multiboot2::IMAGE_BASE);
        assert_eq!(&linked.text[0..8], &image_start.to_le_bytes());
        assert_eq!(&linked.text[8..16], &addresses.bss.to_le_bytes());
        assert_eq!(linked.symbols["__dbyte_image_start"], image_start);
        assert_eq!(linked.symbols["__dbyte_image_end"], addresses.bss);
    }

    #[test]
    fn archive_extraction_is_symbol_driven_and_fixed_point() {
        let caller = object(
            X86_64_LINUX,
            "main.o",
            vec![0xe8, 0, 0, 0, 0, 0xc3],
            vec![
                Symbol {
                    name: "main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 6,
                },
                Symbol {
                    name: "needed".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            vec![Relocation {
                section: Section::Text,
                offset: 1,
                symbol: "needed".into(),
                kind: RelocationKind::Plt32,
                addend: -4,
            }],
        );
        let needed = object(
            X86_64_LINUX,
            "needed.o",
            vec![0xc3],
            vec![Symbol {
                name: "needed".into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Defined {
                    section: Section::Text,
                    value: 0,
                },
                size: 1,
            }],
            vec![],
        );
        let unused = object(
            X86_64_LINUX,
            "unused.o",
            vec![0xcc],
            vec![Symbol {
                name: "unused".into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Defined {
                    section: Section::Text,
                    value: 0,
                },
                size: 1,
            }],
            vec![],
        );
        let archive = crate::archive::write_archive(&[
            crate::archive::ArchiveMember {
                name: "needed.o".into(),
                bytes: needed.bytes,
            },
            crate::archive::ArchiveMember {
                name: "unused.o".into(),
                bytes: unused.bytes,
            },
        ])
        .unwrap();
        let addresses = SectionAddresses {
            text: 0x401000,
            rodata: 0x402000,
            data: 0x403000,
            bss: 0x404000,
        };
        let linked = link_objects(
            &[caller, LinkInput::new("libproof.a", archive)],
            X86_64_LINUX,
            addresses,
            "main",
        )
        .unwrap();
        assert!(linked.symbols.contains_key("needed"));
        assert!(!linked.symbols.contains_key("unused"));
        assert_eq!(linked.relocation_count, 1);
    }
}
