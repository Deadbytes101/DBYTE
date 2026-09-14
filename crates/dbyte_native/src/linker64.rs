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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlatImageLayout {
    pub load_address: u64,
    pub entry: u64,
    pub text_offset: u64,
    pub rodata_offset: u64,
    pub data_offset: u64,
    pub bss_offset: u64,
    pub file_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatImage {
    pub bytes: Vec<u8>,
    pub layout: FlatImageLayout,
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

pub fn write_flat_image(
    linked: &LinkedSections,
    addresses: SectionAddresses,
    load_address: u64,
    maximum_size: u64,
) -> Result<FlatImage, LinkError> {
    if linked.target != crate::target::X86_64_NONE {
        return Err(error(
            "E_FLAT64_TARGET",
            "flat machine payloads require x86_64-dbyte-none",
        ));
    }
    if linked.text.is_empty() {
        return Err(error("E_FLAT64_TEXT", "flat payload text cannot be empty"));
    }
    if maximum_size == 0 {
        return Err(error(
            "E_FLAT64_SIZE",
            "flat payload maximum size must be nonzero",
        ));
    }
    if addresses.text != load_address {
        return Err(error(
            "E_FLAT64_LAYOUT",
            "flat payload text must begin at the load address",
        ));
    }
    let text_end = addresses
        .text
        .checked_add(linked.text.len() as u64)
        .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat text range overflows"))?;
    let rodata_end = addresses
        .rodata
        .checked_add(linked.rodata.len() as u64)
        .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat rodata range overflows"))?;
    let data_end = addresses
        .data
        .checked_add(linked.data.len() as u64)
        .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat data range overflows"))?;
    let bss_end = addresses
        .bss
        .checked_add(linked.bss_size)
        .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat BSS range overflows"))?;
    if addresses.rodata < text_end || addresses.data < rodata_end || addresses.bss < data_end {
        return Err(error(
            "E_FLAT64_OVERLAP",
            "flat payload sections overlap or are out of order",
        ));
    }
    if linked.entry < addresses.text || linked.entry >= text_end {
        return Err(error(
            "E_FLAT64_ENTRY",
            "flat payload entry must be inside text",
        ));
    }
    let file_size = bss_end
        .checked_sub(load_address)
        .ok_or_else(|| error("E_FLAT64_LAYOUT", "flat payload precedes load address"))?;
    if file_size > maximum_size {
        return Err(error(
            "E_FLAT64_SIZE",
            "flat payload exceeds the configured maximum size",
        ));
    }
    let file_len = usize::try_from(file_size)
        .map_err(|_| error("E_FLAT64_SIZE", "flat payload exceeds host addressability"))?;
    let mut bytes = vec![0; file_len];
    copy_flat_section(&mut bytes, load_address, addresses.text, &linked.text)?;
    copy_flat_section(&mut bytes, load_address, addresses.rodata, &linked.rodata)?;
    copy_flat_section(&mut bytes, load_address, addresses.data, &linked.data)?;
    Ok(FlatImage {
        bytes,
        layout: FlatImageLayout {
            load_address,
            entry: linked.entry,
            text_offset: addresses.text - load_address,
            rodata_offset: addresses.rodata - load_address,
            data_offset: addresses.data - load_address,
            bss_offset: addresses.bss - load_address,
            file_size,
        },
        relocation_count: linked.relocation_count,
    })
}

fn copy_flat_section(
    output: &mut [u8],
    load_address: u64,
    address: u64,
    section: &[u8],
) -> Result<(), LinkError> {
    let offset = address
        .checked_sub(load_address)
        .ok_or_else(|| error("E_FLAT64_LAYOUT", "flat section precedes load address"))?;
    let start = usize::try_from(offset)
        .map_err(|_| error("E_FLAT64_SIZE", "flat section offset exceeds host"))?;
    let end = start
        .checked_add(section.len())
        .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat section range overflows"))?;
    let target = output
        .get_mut(start..end)
        .ok_or_else(|| error("E_FLAT64_RANGE", "flat section exceeds output image"))?;
    target.copy_from_slice(section);
    Ok(())
}

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
    link_objects_with_image_start(inputs, requested, addresses, entry_symbol, None)
}

fn link_objects_with_image_start(
    inputs: &[LinkInput],
    requested: TargetSpec,
    addresses: SectionAddresses,
    entry_symbol: &str,
    image_start: Option<u64>,
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
    let reserved_synthetic_symbols =
        kernel_synthetic_symbols(requested, addresses, bss_size, image_start)?;
    for object in &objects {
        for symbol in &object.image.symbols {
            if reserved_synthetic_symbols.contains_key(&symbol.name)
                && symbol.definition != Definition::Undefined
            {
                return Err(error(
                    "E_LINK64_RESERVED_SYMBOL",
                    &format!("input defines reserved linker symbol `{}`", symbol.name),
                ));
            }
        }
    }
    let mut synthetic_symbols = reserved_synthetic_symbols;
    if requested == crate::target::X86_64_WINDOWS
        && !global_definitions.contains_key("_fltused")
        && objects
            .iter()
            .flat_map(|object| &object.image.symbols)
            .any(|symbol| {
                symbol.binding == Binding::Global
                    && symbol.name == "_fltused"
                    && symbol.definition == Definition::Undefined
            })
    {
        let data = merged.entry(Section::Data).or_default();
        let offset = align_up(data.len() as u64, 4)?;
        let offset = usize::try_from(offset)
            .map_err(|_| error("E_LINK64_SIZE", "synthetic Win64 data exceeds host"))?;
        data.resize(offset, 0);
        let address = addresses
            .data
            .checked_add(offset as u64)
            .ok_or_else(|| error("E_LINK64_OVERFLOW", "synthetic _fltused address overflows"))?;
        data.extend_from_slice(&1i32.to_le_bytes());
        synthetic_symbols.insert("_fltused".to_owned(), address);
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

pub fn link_flat_objects(
    inputs: &[LinkInput],
    entry_symbol: &str,
    load_address: u64,
    section_alignment: u64,
    maximum_size: u64,
) -> Result<FlatImage, LinkError> {
    if section_alignment == 0 || !section_alignment.is_power_of_two() || section_alignment > 4096 {
        return Err(error(
            "E_FLAT64_ALIGNMENT",
            "flat section alignment must be a power of two no greater than 4096",
        ));
    }
    let sizing = link_objects_with_image_start(
        inputs,
        crate::target::X86_64_NONE,
        SectionAddresses {
            text: 0,
            rodata: 0,
            data: 0,
            bss: 0,
        },
        entry_symbol,
        Some(0),
    )?;
    let rodata = align_up(
        load_address
            .checked_add(sizing.text.len() as u64)
            .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat text end overflows"))?,
        section_alignment,
    )?;
    let data = align_up(
        rodata
            .checked_add(sizing.rodata.len() as u64)
            .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat rodata end overflows"))?,
        section_alignment,
    )?;
    let bss = align_up(
        data.checked_add(sizing.data.len() as u64)
            .ok_or_else(|| error("E_FLAT64_OVERFLOW", "flat data end overflows"))?,
        section_alignment,
    )?;
    let addresses = SectionAddresses {
        text: load_address,
        rodata,
        data,
        bss,
    };
    let linked = link_objects_with_image_start(
        inputs,
        crate::target::X86_64_NONE,
        addresses,
        entry_symbol,
        Some(load_address),
    )?;
    write_flat_image(&linked, addresses, load_address, maximum_size)
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
            if matches!(
                requested,
                crate::target::X86_64_LINUX | crate::target::X86_64_NONE
            ) && !bytes
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
    image_start: Option<u64>,
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
        image_start.unwrap_or_else(|| u64::from(crate::multiboot2::IMAGE_BASE)),
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
    use crate::target::{X86_64_LINUX, X86_64_NONE, X86_64_WINDOWS};

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

    fn externalize(mut input: LinkInput) -> LinkInput {
        for index in 0..input.bytes.len().saturating_sub(b".dbyte.".len()) {
            if &input.bytes[index..index + b".dbyte.".len()] == b".dbyte." {
                input.bytes[index..index + b".dbyte.".len()].copy_from_slice(b".xbyte.");
            }
        }
        input.bytes.extend_from_slice(&[0; 64]);
        input.bytes[60..62].copy_from_slice(&14u16.to_le_bytes());
        input
    }

    #[test]
    fn explicit_none_target_accepts_machine_boundary_external_elf64() {
        let external = externalize(object(
            X86_64_LINUX,
            "boundary.o",
            vec![0xb8, 0x17, 0x5a, 0, 0, 0xc3],
            vec![Symbol {
                name: "kernel_main".into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Defined {
                    section: Section::Text,
                    value: 0,
                },
                size: 6,
            }],
            vec![],
        ));
        let addresses = SectionAddresses {
            text: 0x1_0000,
            rodata: 0x1_1000,
            data: 0x1_2000,
            bss: 0x1_3000,
        };
        let linked = link_objects(&[external], X86_64_NONE, addresses, "kernel_main")
            .expect("an explicit none target should bind a metadata-free ELF64 boundary object");
        assert_eq!(linked.entry, addresses.text);
        assert_eq!(linked.text, vec![0xb8, 0x17, 0x5a, 0, 0, 0xc3]);
    }

    #[test]
    fn external_elf64_remains_rejected_for_user_profile() {
        let external = externalize(object(
            X86_64_LINUX,
            "boundary.o",
            vec![0xc3],
            vec![Symbol {
                name: "user_main".into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Defined {
                    section: Section::Text,
                    value: 0,
                },
                size: 1,
            }],
            vec![],
        ));
        let failure = link_objects(
            &[external],
            crate::target::X86_64_USER,
            SectionAddresses {
                text: 0x40_0000,
                rodata: 0x41_0000,
                data: 0x42_0000,
                bss: 0x43_0000,
            },
            "user_main",
        )
        .expect_err("the user profile must reject untyped machine-boundary objects");
        assert!(failure.0.contains("E_LINK64_OBJECT"), "{}", failure.0);
    }

    #[test]
    fn flat_none_image_materializes_layout_gaps_and_bss_deterministically() {
        let linked = LinkedSections {
            target: X86_64_NONE,
            entry_abi: None,
            text: vec![0x90, 0xc3],
            rodata: vec![0xaa, 0xbb],
            data: vec![0x11],
            bss_size: 3,
            entry: 0x1_0001,
            symbols: BTreeMap::from([("kernel_main".into(), 0x1_0001)]),
            relocation_count: 7,
        };
        let addresses = SectionAddresses {
            text: 0x1_0000,
            rodata: 0x1_0004,
            data: 0x1_0008,
            bss: 0x1_000c,
        };
        let first = write_flat_image(&linked, addresses, 0x1_0000, 0x100).unwrap();
        let second = write_flat_image(&linked, addresses, 0x1_0000, 0x100).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.bytes.len(), 15);
        assert_eq!(&first.bytes[0..2], &[0x90, 0xc3]);
        assert_eq!(&first.bytes[4..6], &[0xaa, 0xbb]);
        assert_eq!(first.bytes[8], 0x11);
        assert_eq!(&first.bytes[12..15], &[0, 0, 0]);
        assert_eq!(first.layout.entry, 0x1_0001);
        assert_eq!(first.layout.bss_offset, 12);
        assert_eq!(first.relocation_count, 7);
    }

    #[test]
    fn flat_none_image_rejects_overlap_size_and_entry_violations() {
        let linked = LinkedSections {
            target: X86_64_NONE,
            entry_abi: None,
            text: vec![0x90, 0xc3],
            rodata: vec![0xaa],
            data: vec![],
            bss_size: 0,
            entry: 0x1_0010,
            symbols: BTreeMap::new(),
            relocation_count: 0,
        };
        let overlap = SectionAddresses {
            text: 0x1_0000,
            rodata: 0x1_0001,
            data: 0x1_0002,
            bss: 0x1_0002,
        };
        assert!(write_flat_image(&linked, overlap, 0x1_0000, 0x100)
            .unwrap_err()
            .0
            .contains("E_FLAT64_OVERLAP"));
        let valid = SectionAddresses {
            text: 0x1_0000,
            rodata: 0x1_0002,
            data: 0x1_0003,
            bss: 0x1_0003,
        };
        assert!(write_flat_image(&linked, valid, 0x1_0000, 0x100)
            .unwrap_err()
            .0
            .contains("E_FLAT64_ENTRY"));
        let mut entry_valid = linked;
        entry_valid.entry = 0x1_0000;
        assert!(write_flat_image(&entry_valid, valid, 0x1_0000, 2)
            .unwrap_err()
            .0
            .contains("E_FLAT64_SIZE"));
    }

    #[test]
    fn flat_link_seam_resolves_external_calls_and_uses_requested_image_base() {
        let external = externalize(object(
            X86_64_LINUX,
            "boundary.o",
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
                    name: "dbyte_body".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            vec![Relocation {
                section: Section::Text,
                offset: 1,
                symbol: "dbyte_body".into(),
                kind: RelocationKind::Plt32,
                addend: -4,
            }],
        ));
        let dbyte = object(
            X86_64_NONE,
            "body.o",
            vec![0x48, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0xc3],
            vec![
                Symbol {
                    name: "dbyte_body".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 11,
                },
                Symbol {
                    name: "__dbyte_image_start".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Object,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            vec![Relocation {
                section: Section::Text,
                offset: 2,
                symbol: "__dbyte_image_start".into(),
                kind: RelocationKind::Absolute64,
                addend: 0,
            }],
        );
        let image =
            link_flat_objects(&[dbyte, external], "kernel_main", 0x1_0000, 16, 252 * 512).unwrap();
        assert_eq!(image.layout.load_address, 0x1_0000);
        assert_eq!(image.layout.entry, 0x1_0010);
        assert_eq!(&image.bytes[2..10], &0x1_0000u64.to_le_bytes());
        assert_eq!(&image.bytes[17..21], &(-21i32).to_le_bytes());
        assert_eq!(image.relocation_count, 2);
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
    fn win64_synthesizes_fltused_for_freestanding_float_objects() {
        let image = ObjectImage {
            target: X86_64_WINDOWS,
            entry_abi: None,
            sections: BTreeMap::from([(
                Section::Text,
                SectionData {
                    contents: Contents::Bytes(vec![0xc3]),
                    alignment: 16,
                },
            )]),
            symbols: vec![
                Symbol {
                    name: "main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 1,
                },
                Symbol {
                    name: "_fltused".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Object,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            relocations: vec![],
        };
        let input = LinkInput::new(
            "float.obj",
            crate::coff64::write_object(&image).unwrap().bytes,
        );
        let addresses = SectionAddresses {
            text: 0x140001000,
            rodata: 0x140002000,
            data: 0x140003000,
            bss: 0x140004000,
        };
        let linked = link_objects(&[input], X86_64_WINDOWS, addresses, "main").unwrap();
        assert_eq!(linked.symbols["_fltused"], addresses.data);
        assert_eq!(linked.data, 1i32.to_le_bytes());
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
