//! Deterministic ELF32/i386 relocatable object images.
//!
//! DByte objects intentionally support a small, auditable ELF subset.  The
//! parser accepts only the sections, symbols, and relocation forms emitted by
//! this module; it never trusts offsets or counts from an input file.

use crate::diagnostics::{Diagnostic, DiagnosticCategory};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

pub const MAX_OBJECT_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_OBJECT_SECTION_SIZE: usize = 8 * 1024 * 1024;
pub const MAX_OBJECT_ALIGNMENT: u32 = 4096;
pub const MAX_OBJECT_SYMBOLS: usize = 65_535;
pub const MAX_OBJECT_RELOCATIONS: usize = 262_144;

const ELF_HEADER_SIZE: u32 = 52;
const SECTION_HEADER_SIZE: u32 = 40;
const SYMBOL_SIZE: u32 = 16;
const REL_SIZE: u32 = 8;

const ET_REL: u16 = 1;
const EM_386: u16 = 3;
const SHN_UNDEF: u16 = 0;
const SHT_NULL: u32 = 0;
const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHT_NOBITS: u32 = 8;
const SHT_REL: u32 = 9;
const SHF_WRITE: u32 = 0x1;
const SHF_ALLOC: u32 = 0x2;
const SHF_EXECINSTR: u32 = 0x4;
const STB_LOCAL: u8 = 0;
const STB_GLOBAL: u8 = 1;
const STT_OBJECT: u8 = 1;
const STT_FUNC: u8 = 2;
const STT_SECTION: u8 = 3;
const R_386_32: u8 = 1;
const R_386_PC32: u8 = 2;
const INTERRUPT_METADATA_SECTION: &str = ".dbyte.interrupts";
const INTERRUPT_METADATA_PREFIX: &str = "__dbyte_interrupt:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectSection {
    Text,
    Rodata,
    Data,
    Bss,
}

impl ObjectSection {
    pub const ALL: [Self; 4] = [Self::Text, Self::Rodata, Self::Data, Self::Bss];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => ".text",
            Self::Rodata => ".rodata",
            Self::Data => ".data",
            Self::Bss => ".bss",
        }
    }

    const fn elf_index(self) -> u16 {
        match self {
            Self::Text => 1,
            Self::Rodata => 2,
            Self::Data => 3,
            Self::Bss => 4,
        }
    }

    const fn relocation_name(self) -> Option<&'static str> {
        match self {
            Self::Text => Some(".rel.text"),
            Self::Rodata => Some(".rel.rodata"),
            Self::Data => Some(".rel.data"),
            Self::Bss => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SectionContents {
    Bytes(Vec<u8>),
    Uninitialized(u32),
}

impl SectionContents {
    pub fn size(&self) -> Result<u32, Diagnostic> {
        match self {
            Self::Bytes(bytes) => u32::try_from(bytes.len())
                .map_err(|_| object_error("section byte length exceeds u32")),
            Self::Uninitialized(size) => Ok(*size),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectSectionData {
    pub contents: SectionContents,
    pub alignment: u32,
}

impl ObjectSectionData {
    pub fn bytes(bytes: Vec<u8>, alignment: u32) -> Self {
        Self {
            contents: SectionContents::Bytes(bytes),
            alignment,
        }
    }

    pub fn bss(size: u32, alignment: u32) -> Self {
        Self {
            contents: SectionContents::Uninitialized(size),
            alignment,
        }
    }

    pub fn size(&self) -> Result<u32, Diagnostic> {
        self.contents.size()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SymbolId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SymbolBinding {
    Local,
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SymbolKind {
    Function,
    Object,
    Section,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SymbolDefinition {
    Undefined,
    Defined { section: ObjectSection, value: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectSymbol {
    pub name: String,
    pub binding: SymbolBinding,
    pub kind: SymbolKind,
    pub definition: SymbolDefinition,
    pub size: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelocationKind {
    Abs32,
    Pc32,
}

impl RelocationKind {
    pub const fn elf_type(self) -> u8 {
        match self {
            Self::Abs32 => R_386_32,
            Self::Pc32 => R_386_PC32,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectRelocation {
    pub section: ObjectSection,
    pub offset: u32,
    pub symbol: SymbolId,
    pub kind: RelocationKind,
    /// ELF32 `SHT_REL` stores this addend in the relocated section bytes.
    pub addend: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectImage {
    pub sections: BTreeMap<ObjectSection, ObjectSectionData>,
    pub symbols: Vec<ObjectSymbol>,
    pub relocations: Vec<ObjectRelocation>,
}

impl ObjectImage {
    pub fn empty() -> Self {
        Self {
            sections: BTreeMap::from([
                (
                    ObjectSection::Text,
                    ObjectSectionData::bytes(Vec::new(), 16),
                ),
                (
                    ObjectSection::Rodata,
                    ObjectSectionData::bytes(Vec::new(), 4),
                ),
                (ObjectSection::Data, ObjectSectionData::bytes(Vec::new(), 4)),
                (ObjectSection::Bss, ObjectSectionData::bss(0, 4)),
            ]),
            symbols: Vec::new(),
            relocations: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectArtifact {
    pub bytes: Vec<u8>,
    pub dump: String,
}

#[derive(Clone, Debug)]
struct SectionHeader {
    name: u32,
    section_type: u32,
    flags: u32,
    address: u32,
    offset: u32,
    size: u32,
    link: u32,
    info: u32,
    alignment: u32,
    entry_size: u32,
}

#[derive(Clone, Debug)]
struct ParsedSection {
    header: SectionHeader,
    name: String,
}

pub fn write_relocatable(image: &ObjectImage) -> Result<ObjectArtifact, Diagnostic> {
    validate_image_shape(image)?;

    let mut section_bytes = BTreeMap::new();
    for section in [
        ObjectSection::Text,
        ObjectSection::Rodata,
        ObjectSection::Data,
    ] {
        let data = required_section(image, section)?;
        let SectionContents::Bytes(bytes) = &data.contents else {
            return Err(object_error(format!(
                "{} must be file-backed",
                section.name()
            )));
        };
        section_bytes.insert(section, bytes.clone());
    }

    let symbol_order = canonical_symbol_order(image)?;
    let mut elf_symbol_indices = vec![0u32; image.symbols.len()];
    for (position, source_index) in symbol_order.iter().copied().enumerate() {
        elf_symbol_indices[source_index] = checked_usize_u32(position + 1, "symbol index")?;
    }

    let mut relocations = image.relocations.clone();
    relocations.sort_by_key(|relocation| {
        (
            relocation.section,
            relocation.offset,
            relocation.kind,
            relocation.symbol,
            relocation.addend,
        )
    });
    validate_and_apply_addends(&mut section_bytes, image, &relocations)?;

    let relocation_sections: Vec<ObjectSection> = [
        ObjectSection::Text,
        ObjectSection::Rodata,
        ObjectSection::Data,
    ]
    .into_iter()
    .filter(|section| {
        relocations
            .iter()
            .any(|relocation| relocation.section == *section)
    })
    .collect();

    let interrupt_metadata = build_interrupt_metadata(image)?;
    let symtab_index = 6u16
        .checked_add(
            u16::try_from(relocation_sections.len())
                .map_err(|_| object_error("too many relocation sections"))?,
        )
        .ok_or_else(|| object_error("section index overflow"))?;
    let strtab_index = symtab_index + 1;
    let shstrtab_index = strtab_index + 1;
    let section_count = shstrtab_index + 1;

    let mut section_names = vec![
        "",
        ".text",
        ".rodata",
        ".data",
        ".bss",
        INTERRUPT_METADATA_SECTION,
    ];
    for section in &relocation_sections {
        section_names.push(
            section
                .relocation_name()
                .ok_or_else(|| object_error(".bss cannot have a relocation section"))?,
        );
    }
    section_names.extend([".symtab", ".strtab", ".shstrtab"]);
    let (shstrtab, section_name_offsets) = build_string_table(&section_names)?;

    let (symtab, strtab, local_symbol_count) = build_symbol_table(image, &symbol_order)?;
    let mut relocation_tables = BTreeMap::new();
    for section in &relocation_sections {
        let entries: Vec<&ObjectRelocation> = relocations
            .iter()
            .filter(|relocation| relocation.section == *section)
            .collect();
        let mut table = bounded_zeros(
            entries
                .len()
                .checked_mul(REL_SIZE as usize)
                .ok_or_else(|| object_error("relocation table size overflow"))?,
            "relocation table",
        )?;
        for (index, relocation) in entries.iter().enumerate() {
            let offset = index * REL_SIZE as usize;
            let symbol_index = *elf_symbol_indices
                .get(relocation.symbol.0 as usize)
                .ok_or_else(|| invalid_relocation("relocation references an unknown symbol"))?;
            if symbol_index > 0x00ff_ffff {
                return Err(invalid_relocation(
                    "ELF relocation symbol index is too large",
                ));
            }
            write_u32(&mut table, offset, relocation.offset)?;
            write_u32(
                &mut table,
                offset + 4,
                (symbol_index << 8) | u32::from(relocation.kind.elf_type()),
            )?;
        }
        relocation_tables.insert(*section, table);
    }

    let mut headers = vec![SectionHeader {
        name: 0,
        section_type: SHT_NULL,
        flags: 0,
        address: 0,
        offset: 0,
        size: 0,
        link: 0,
        info: 0,
        alignment: 0,
        entry_size: 0,
    }];
    let mut cursor = ELF_HEADER_SIZE;
    for section in ObjectSection::ALL {
        let data = required_section(image, section)?;
        cursor = align_up(cursor, data.alignment)?;
        let size = data.size()?;
        let (section_type, flags) = section_header_contract(section);
        headers.push(SectionHeader {
            name: section_name_offsets[section.elf_index() as usize],
            section_type,
            flags,
            address: 0,
            offset: cursor,
            size,
            link: 0,
            info: 0,
            alignment: data.alignment,
            entry_size: 0,
        });
        if section != ObjectSection::Bss {
            cursor = checked_add(cursor, size, "section file range")?;
        }
    }

    let interrupt_metadata_offset = cursor;
    let interrupt_metadata_size =
        checked_usize_u32(interrupt_metadata.len(), "interrupt metadata size")?;
    headers.push(SectionHeader {
        name: section_name_offsets[5],
        section_type: SHT_PROGBITS,
        flags: 0,
        address: 0,
        offset: interrupt_metadata_offset,
        size: interrupt_metadata_size,
        link: 0,
        info: 0,
        alignment: 1,
        entry_size: 0,
    });
    cursor = checked_add(
        cursor,
        interrupt_metadata_size,
        "interrupt metadata section end",
    )?;

    for (relocation_position, section) in relocation_sections.iter().copied().enumerate() {
        cursor = align_up(cursor, 4)?;
        let table = &relocation_tables[&section];
        let size = checked_usize_u32(table.len(), "relocation section size")?;
        headers.push(SectionHeader {
            name: section_name_offsets[6 + relocation_position],
            section_type: SHT_REL,
            flags: 0,
            address: 0,
            offset: cursor,
            size,
            link: u32::from(symtab_index),
            info: u32::from(section.elf_index()),
            alignment: 4,
            entry_size: REL_SIZE,
        });
        cursor = checked_add(cursor, size, "relocation section end")?;
    }

    cursor = align_up(cursor, 4)?;
    let symtab_offset = cursor;
    cursor = checked_add(
        cursor,
        checked_usize_u32(symtab.len(), "symbol table size")?,
        "symbol table end",
    )?;
    let strtab_offset = cursor;
    cursor = checked_add(
        cursor,
        checked_usize_u32(strtab.len(), "string table size")?,
        "string table end",
    )?;
    let shstrtab_offset = cursor;
    cursor = checked_add(
        cursor,
        checked_usize_u32(shstrtab.len(), "section string table size")?,
        "section string table end",
    )?;
    let section_header_offset = align_up(cursor, 4)?;
    let file_size = checked_add(
        section_header_offset,
        checked_mul(
            u32::from(section_count),
            SECTION_HEADER_SIZE,
            "section table size",
        )?,
        "object file size",
    )?;
    if file_size as usize > MAX_OBJECT_SIZE {
        return Err(object_error(format!(
            "object size {file_size} exceeds bound {MAX_OBJECT_SIZE}"
        )));
    }

    headers.push(SectionHeader {
        name: section_name_offsets[symtab_index as usize],
        section_type: SHT_SYMTAB,
        flags: 0,
        address: 0,
        offset: symtab_offset,
        size: checked_usize_u32(symtab.len(), "symbol table size")?,
        link: u32::from(strtab_index),
        info: checked_usize_u32(local_symbol_count + 1, "first global symbol index")?,
        alignment: 4,
        entry_size: SYMBOL_SIZE,
    });
    headers.push(SectionHeader {
        name: section_name_offsets[strtab_index as usize],
        section_type: SHT_STRTAB,
        flags: 0,
        address: 0,
        offset: strtab_offset,
        size: checked_usize_u32(strtab.len(), "string table size")?,
        link: 0,
        info: 0,
        alignment: 1,
        entry_size: 0,
    });
    headers.push(SectionHeader {
        name: section_name_offsets[shstrtab_index as usize],
        section_type: SHT_STRTAB,
        flags: 0,
        address: 0,
        offset: shstrtab_offset,
        size: checked_usize_u32(shstrtab.len(), "section string table size")?,
        link: 0,
        info: 0,
        alignment: 1,
        entry_size: 0,
    });
    if headers.len() != section_count as usize {
        return Err(object_error("internal section-count mismatch"));
    }

    let mut bytes = bounded_zeros(file_size as usize, "object file")?;
    write_object_header(
        &mut bytes,
        section_header_offset,
        section_count,
        shstrtab_index,
    )?;
    for section in [
        ObjectSection::Text,
        ObjectSection::Rodata,
        ObjectSection::Data,
    ] {
        copy_bytes(
            &mut bytes,
            headers[section.elf_index() as usize].offset as usize,
            &section_bytes[&section],
        )?;
    }
    for (index, section) in relocation_sections.iter().enumerate() {
        copy_bytes(
            &mut bytes,
            headers[6 + index].offset as usize,
            &relocation_tables[section],
        )?;
    }
    copy_bytes(
        &mut bytes,
        interrupt_metadata_offset as usize,
        &interrupt_metadata,
    )?;
    copy_bytes(&mut bytes, symtab_offset as usize, &symtab)?;
    copy_bytes(&mut bytes, strtab_offset as usize, &strtab)?;
    copy_bytes(&mut bytes, shstrtab_offset as usize, &shstrtab)?;
    write_section_headers(&mut bytes, section_header_offset, &headers)?;

    let dump = object_dump(&headers, &section_names, image, &symbol_order, &relocations);
    Ok(ObjectArtifact { bytes, dump })
}

pub fn parse_relocatable(bytes: &[u8]) -> Result<ObjectImage, Diagnostic> {
    if bytes.len() > MAX_OBJECT_SIZE {
        return Err(invalid_object(format!(
            "object length {} exceeds bound {MAX_OBJECT_SIZE}",
            bytes.len()
        )));
    }
    validate_object_header(bytes)?;
    let section_header_offset = read_u32(bytes, 32)?;
    let section_header_size = read_u16(bytes, 46)?;
    let section_count = read_u16(bytes, 48)?;
    let shstrtab_index = read_u16(bytes, 50)?;
    if section_header_size != SECTION_HEADER_SIZE as u16 || !(9..=12).contains(&section_count) {
        return Err(invalid_object("unsupported ELF section-header shape"));
    }
    if shstrtab_index == 0 || shstrtab_index >= section_count {
        return Err(invalid_object("invalid section-name table index"));
    }
    let section_table_size = checked_mul_parse(
        u32::from(section_count),
        SECTION_HEADER_SIZE,
        "section-header table size",
    )?;
    checked_range(
        bytes,
        section_header_offset,
        section_table_size,
        "section-header table",
    )?;

    let mut raw_headers = Vec::with_capacity(section_count as usize);
    for index in 0..section_count {
        let offset = checked_add_parse(
            section_header_offset,
            checked_mul_parse(
                u32::from(index),
                SECTION_HEADER_SIZE,
                "section-header offset",
            )?,
            "section-header offset",
        )?;
        raw_headers.push(read_section_header(bytes, offset as usize)?);
    }
    if raw_headers[0].section_type != SHT_NULL {
        return Err(invalid_object("section zero is not SHT_NULL"));
    }
    let shstr_header = &raw_headers[shstrtab_index as usize];
    if shstr_header.section_type != SHT_STRTAB {
        return Err(invalid_object("section-name table is not SHT_STRTAB"));
    }
    let shstrtab = section_file_bytes(bytes, shstr_header, ".shstrtab")?;
    validate_string_table(shstrtab, ".shstrtab")?;

    let mut parsed = Vec::with_capacity(raw_headers.len());
    let mut by_name = BTreeMap::new();
    for (index, header) in raw_headers.into_iter().enumerate() {
        let name = read_string(shstrtab, header.name, "section name")?;
        if index != 0 {
            if name.is_empty() {
                return Err(invalid_object("non-null section has an empty name"));
            }
            if by_name.insert(name.clone(), index as u16).is_some() {
                return Err(invalid_object(format!("duplicate section '{name}'")));
            }
        }
        parsed.push(ParsedSection { header, name });
    }
    validate_allowed_sections(&parsed)?;
    if parsed[shstrtab_index as usize].name != ".shstrtab" {
        return Err(invalid_object(
            "ELF e_shstrndx does not reference .shstrtab",
        ));
    }
    validate_string_section_header(&parsed[shstrtab_index as usize].header, ".shstrtab")?;
    validate_file_ranges(bytes, &parsed, section_header_offset, section_table_size)?;

    let mut section_indices = BTreeMap::new();
    let mut sections = BTreeMap::new();
    for section in ObjectSection::ALL {
        let index = *by_name.get(section.name()).ok_or_else(|| {
            invalid_object(format!("missing required section {}", section.name()))
        })?;
        let parsed_section = &parsed[index as usize];
        validate_core_section(section, &parsed_section.header)?;
        let contents = if section == ObjectSection::Bss {
            SectionContents::Uninitialized(parsed_section.header.size)
        } else {
            SectionContents::Bytes(
                section_file_bytes(bytes, &parsed_section.header, section.name())?.to_vec(),
            )
        };
        sections.insert(
            section,
            ObjectSectionData {
                contents,
                alignment: parsed_section.header.alignment,
            },
        );
        section_indices.insert(index, section);
    }

    let symtab_index = *by_name
        .get(".symtab")
        .ok_or_else(|| invalid_object("missing .symtab"))?;
    let strtab_index = *by_name
        .get(".strtab")
        .ok_or_else(|| invalid_object("missing .strtab"))?;
    let symtab_header = &parsed[symtab_index as usize].header;
    let strtab_header = &parsed[strtab_index as usize].header;
    validate_table_headers(symtab_header, strtab_header, strtab_index)?;
    let strtab = section_file_bytes(bytes, strtab_header, ".strtab")?;
    validate_string_table(strtab, ".strtab")?;
    let symtab = section_file_bytes(bytes, symtab_header, ".symtab")?;
    let symbols = parse_symbols(
        symtab,
        strtab,
        symtab_header.info,
        &section_indices,
        &sections,
    )?;
    let metadata_index = *by_name
        .get(INTERRUPT_METADATA_SECTION)
        .ok_or_else(|| invalid_object("missing .dbyte.interrupts"))?;
    let metadata_header = &parsed[metadata_index as usize].header;
    if metadata_header.section_type != SHT_PROGBITS
        || metadata_header.flags != 0
        || metadata_header.link != 0
        || metadata_header.info != 0
        || metadata_header.alignment != 1
        || metadata_header.entry_size != 0
    {
        return Err(invalid_object("invalid .dbyte.interrupts section contract"));
    }
    validate_interrupt_metadata(
        section_file_bytes(bytes, metadata_header, INTERRUPT_METADATA_SECTION)?,
        &symbols,
    )?;

    let mut relocations = Vec::new();
    for target in [
        ObjectSection::Text,
        ObjectSection::Rodata,
        ObjectSection::Data,
    ] {
        let name = target
            .relocation_name()
            .ok_or_else(|| invalid_relocation(".bss cannot have relocations"))?;
        let Some(index) = by_name.get(name).copied() else {
            continue;
        };
        let header = &parsed[index as usize].header;
        if header.section_type != SHT_REL
            || header.link != u32::from(symtab_index)
            || header.info
                != u32::from(
                    *by_name
                        .get(target.name())
                        .ok_or_else(|| invalid_object("missing relocation target section"))?,
                )
            || header.alignment != 4
            || header.entry_size != REL_SIZE
            || !header.size.is_multiple_of(REL_SIZE)
        {
            return Err(invalid_relocation(format!(
                "invalid {name} section contract"
            )));
        }
        let table = section_file_bytes(bytes, header, name)?;
        let target_bytes = match &sections[&target].contents {
            SectionContents::Bytes(value) => value,
            SectionContents::Uninitialized(_) => unreachable!(),
        };
        for entry in table.chunks_exact(REL_SIZE as usize) {
            if relocations.len() >= MAX_OBJECT_RELOCATIONS {
                return Err(invalid_relocation("object relocation count exceeds bound"));
            }
            let offset = read_u32(entry, 0)?;
            let info = read_u32(entry, 4)?;
            let elf_symbol_index = info >> 8;
            let symbol_index = elf_symbol_index
                .checked_sub(1)
                .ok_or_else(|| invalid_relocation("relocation references ELF null symbol"))?;
            if symbol_index as usize >= symbols.len() {
                return Err(invalid_relocation(
                    "relocation symbol index is out of bounds",
                ));
            }
            let kind = match (info & 0xff) as u8 {
                R_386_32 => RelocationKind::Abs32,
                R_386_PC32 => RelocationKind::Pc32,
                value => {
                    return Err(invalid_relocation(format!(
                        "unsupported i386 relocation type {value}"
                    )))
                }
            };
            let end = offset
                .checked_add(4)
                .ok_or_else(|| invalid_relocation("relocation target range overflows"))?;
            let addend_bytes = target_bytes
                .get(offset as usize..end as usize)
                .ok_or_else(|| invalid_relocation("relocation target exceeds section bounds"))?;
            let addend = i32::from_le_bytes(
                addend_bytes
                    .try_into()
                    .map_err(|_| invalid_relocation("invalid relocation addend width"))?,
            );
            relocations.push(ObjectRelocation {
                section: target,
                offset,
                symbol: SymbolId(symbol_index),
                kind,
                addend,
            });
        }
    }
    relocations.sort_by_key(|relocation| {
        (
            relocation.section,
            relocation.offset,
            relocation.kind,
            relocation.symbol,
            relocation.addend,
        )
    });
    validate_non_overlapping_relocations(&relocations)?;

    let image = ObjectImage {
        sections,
        symbols,
        relocations,
    };
    validate_image_shape(&image).map_err(|error| invalid_object(error.message))?;
    Ok(image)
}

fn validate_image_shape(image: &ObjectImage) -> Result<(), Diagnostic> {
    if image.sections.len() != ObjectSection::ALL.len() {
        return Err(object_error(
            "object image must contain exactly four core sections",
        ));
    }
    for section in ObjectSection::ALL {
        let data = required_section(image, section)?;
        validate_alignment(data.alignment)?;
        let size = data.size()?;
        if size as usize > MAX_OBJECT_SECTION_SIZE {
            return Err(object_error(format!(
                "{} size {size} exceeds bound {MAX_OBJECT_SECTION_SIZE}",
                section.name()
            )));
        }
        match (section, &data.contents) {
            (ObjectSection::Bss, SectionContents::Uninitialized(_)) => {}
            (ObjectSection::Bss, SectionContents::Bytes(_)) => {
                return Err(object_error(".bss must be uninitialized"));
            }
            (_, SectionContents::Bytes(_)) => {}
            (_, SectionContents::Uninitialized(_)) => {
                return Err(object_error(format!(
                    "{} must be file-backed",
                    section.name()
                )));
            }
        }
    }
    if image.symbols.len() > MAX_OBJECT_SYMBOLS {
        return Err(object_error(format!(
            "symbol count {} exceeds bound {MAX_OBJECT_SYMBOLS}",
            image.symbols.len()
        )));
    }
    for symbol in &image.symbols {
        validate_symbol(image, symbol)?;
    }
    if image.relocations.len() > MAX_OBJECT_RELOCATIONS {
        return Err(invalid_relocation(format!(
            "relocation count {} exceeds bound {MAX_OBJECT_RELOCATIONS}",
            image.relocations.len()
        )));
    }
    for relocation in &image.relocations {
        if relocation.section == ObjectSection::Bss {
            return Err(invalid_relocation("relocations cannot target .bss"));
        }
        if relocation.symbol.0 as usize >= image.symbols.len() {
            return Err(invalid_relocation(format!(
                "relocation references unknown symbol {}",
                relocation.symbol.0
            )));
        }
        let size = required_section(image, relocation.section)?.size()?;
        if relocation
            .offset
            .checked_add(4)
            .is_none_or(|end| end > size)
        {
            return Err(invalid_relocation(format!(
                "relocation at 0x{:08x} exceeds {}",
                relocation.offset,
                relocation.section.name()
            )));
        }
    }
    validate_non_overlapping_relocations(&image.relocations)
}

fn validate_symbol(image: &ObjectImage, symbol: &ObjectSymbol) -> Result<(), Diagnostic> {
    if symbol.name.as_bytes().contains(&0) {
        return Err(object_error("symbol names cannot contain NUL bytes"));
    }
    if symbol.name.is_empty() && symbol.kind != SymbolKind::Section {
        return Err(object_error("function and object symbols require names"));
    }
    match symbol.definition {
        SymbolDefinition::Undefined => {
            if symbol.binding != SymbolBinding::Global {
                return Err(object_error("undefined symbols must have global binding"));
            }
            if symbol.kind == SymbolKind::Section {
                return Err(object_error("section symbols cannot be undefined"));
            }
            if symbol.size != 0 {
                return Err(object_error("undefined symbols must have size zero"));
            }
        }
        SymbolDefinition::Defined { section, value } => {
            let section_size = required_section(image, section)?.size()?;
            let end = value
                .checked_add(symbol.size)
                .ok_or_else(|| object_error(format!("symbol '{}' range overflows", symbol.name)))?;
            if value > section_size || end > section_size {
                return Err(object_error(format!(
                    "symbol '{}' exceeds {} bounds",
                    symbol.name,
                    section.name()
                )));
            }
            if symbol.kind == SymbolKind::Function && section != ObjectSection::Text {
                return Err(object_error(format!(
                    "function symbol '{}' is not defined in .text",
                    symbol.name
                )));
            }
            if symbol.kind == SymbolKind::Section
                && (symbol.binding != SymbolBinding::Local || value != 0 || symbol.size != 0)
            {
                return Err(object_error(
                    "section symbols must be local with zero value and size",
                ));
            }
        }
    }
    Ok(())
}

fn required_section(
    image: &ObjectImage,
    section: ObjectSection,
) -> Result<&ObjectSectionData, Diagnostic> {
    image
        .sections
        .get(&section)
        .ok_or_else(|| object_error(format!("missing required section {}", section.name())))
}

fn validate_alignment(alignment: u32) -> Result<(), Diagnostic> {
    if alignment == 0 || !alignment.is_power_of_two() || alignment > MAX_OBJECT_ALIGNMENT {
        return Err(object_error(format!(
            "invalid section alignment {alignment}; expected power of two through {MAX_OBJECT_ALIGNMENT}"
        )));
    }
    Ok(())
}

fn canonical_symbol_order(image: &ObjectImage) -> Result<Vec<usize>, Diagnostic> {
    let mut order: Vec<usize> = (0..image.symbols.len()).collect();
    order.sort_by(|left, right| {
        let left_symbol = &image.symbols[*left];
        let right_symbol = &image.symbols[*right];
        left_symbol
            .binding
            .cmp(&right_symbol.binding)
            .then_with(|| left_symbol.name.cmp(&right_symbol.name))
            .then_with(|| left_symbol.kind.cmp(&right_symbol.kind))
            .then_with(|| left_symbol.definition.cmp(&right_symbol.definition))
            .then_with(|| left_symbol.size.cmp(&right_symbol.size))
            .then_with(|| left.cmp(right))
    });
    Ok(order)
}

fn validate_and_apply_addends(
    section_bytes: &mut BTreeMap<ObjectSection, Vec<u8>>,
    image: &ObjectImage,
    relocations: &[ObjectRelocation],
) -> Result<(), Diagnostic> {
    validate_non_overlapping_relocations(relocations)?;
    for relocation in relocations {
        if relocation.symbol.0 as usize >= image.symbols.len() {
            return Err(invalid_relocation(
                "relocation references an unknown symbol",
            ));
        }
        let bytes = section_bytes
            .get_mut(&relocation.section)
            .ok_or_else(|| invalid_relocation("relocation target is not file-backed"))?;
        let target = checked_slice_mut(bytes, relocation.offset, 4, "relocation target")?;
        target.copy_from_slice(&relocation.addend.to_le_bytes());
    }
    Ok(())
}

fn validate_non_overlapping_relocations(
    relocations: &[ObjectRelocation],
) -> Result<(), Diagnostic> {
    let mut ranges: BTreeMap<ObjectSection, Vec<(u32, u32)>> = BTreeMap::new();
    for relocation in relocations {
        let end = relocation
            .offset
            .checked_add(4)
            .ok_or_else(|| invalid_relocation("relocation target range overflows"))?;
        ranges
            .entry(relocation.section)
            .or_default()
            .push((relocation.offset, end));
    }
    for (section, section_ranges) in &mut ranges {
        section_ranges.sort_unstable();
        for pair in section_ranges.windows(2) {
            if pair[0].1 > pair[1].0 {
                return Err(invalid_relocation(format!(
                    "overlapping relocations in {} at 0x{:08x}",
                    section.name(),
                    pair[1].0
                )));
            }
        }
    }
    Ok(())
}

fn build_symbol_table(
    image: &ObjectImage,
    order: &[usize],
) -> Result<(Vec<u8>, Vec<u8>, usize), Diagnostic> {
    let entry_count = order
        .len()
        .checked_add(1)
        .ok_or_else(|| object_error("symbol count overflow"))?;
    let mut table = bounded_zeros(
        entry_count
            .checked_mul(SYMBOL_SIZE as usize)
            .ok_or_else(|| object_error("symbol table size overflow"))?,
        "symbol table",
    )?;
    let mut strings = vec![0u8];
    let mut local_count = 0usize;
    for (position, source_index) in order.iter().copied().enumerate() {
        let symbol = &image.symbols[source_index];
        if symbol.binding == SymbolBinding::Local {
            local_count += 1;
        }
        let name_offset = checked_usize_u32(strings.len(), "symbol name offset")?;
        extend_string_table(&mut strings, &symbol.name)?;
        let offset = (position + 1) * SYMBOL_SIZE as usize;
        write_u32(&mut table, offset, name_offset)?;
        let (value, section_index) = match symbol.definition {
            SymbolDefinition::Undefined => (0, SHN_UNDEF),
            SymbolDefinition::Defined { section, value } => (value, section.elf_index()),
        };
        write_u32(&mut table, offset + 4, value)?;
        write_u32(&mut table, offset + 8, symbol.size)?;
        write_u8(
            &mut table,
            offset + 12,
            symbol_info(symbol.binding, symbol.kind),
        )?;
        write_u8(&mut table, offset + 13, 0)?;
        write_u16(&mut table, offset + 14, section_index)?;
    }
    Ok((table, strings, local_count))
}

fn build_string_table(values: &[&str]) -> Result<(Vec<u8>, Vec<u32>), Diagnostic> {
    let mut table = Vec::new();
    let mut offsets = Vec::with_capacity(values.len());
    for value in values {
        offsets.push(checked_usize_u32(table.len(), "string-table offset")?);
        extend_string_table(&mut table, value)?;
    }
    Ok((table, offsets))
}

fn extend_string_table(table: &mut Vec<u8>, value: &str) -> Result<(), Diagnostic> {
    if value.as_bytes().contains(&0) {
        return Err(object_error("ELF strings cannot contain NUL bytes"));
    }
    let required = table
        .len()
        .checked_add(value.len())
        .and_then(|length| length.checked_add(1))
        .ok_or_else(|| object_error("string table size overflow"))?;
    if required > MAX_OBJECT_SIZE {
        return Err(object_error("string table exceeds object size bound"));
    }
    table.extend_from_slice(value.as_bytes());
    table.push(0);
    Ok(())
}

fn symbol_info(binding: SymbolBinding, kind: SymbolKind) -> u8 {
    let binding = match binding {
        SymbolBinding::Local => STB_LOCAL,
        SymbolBinding::Global => STB_GLOBAL,
    };
    let kind = match kind {
        SymbolKind::Function => STT_FUNC,
        SymbolKind::Object => STT_OBJECT,
        SymbolKind::Section => STT_SECTION,
    };
    (binding << 4) | kind
}

fn section_header_contract(section: ObjectSection) -> (u32, u32) {
    match section {
        ObjectSection::Text => (SHT_PROGBITS, SHF_ALLOC | SHF_EXECINSTR),
        ObjectSection::Rodata => (SHT_PROGBITS, SHF_ALLOC),
        ObjectSection::Data => (SHT_PROGBITS, SHF_ALLOC | SHF_WRITE),
        ObjectSection::Bss => (SHT_NOBITS, SHF_ALLOC | SHF_WRITE),
    }
}

fn write_object_header(
    bytes: &mut [u8],
    section_header_offset: u32,
    section_count: u16,
    shstrtab_index: u16,
) -> Result<(), Diagnostic> {
    copy_bytes(bytes, 0, b"\x7fELF")?;
    write_u8(bytes, 4, 1)?;
    write_u8(bytes, 5, 1)?;
    write_u8(bytes, 6, 1)?;
    write_u8(bytes, 7, 0)?;
    write_u16(bytes, 16, ET_REL)?;
    write_u16(bytes, 18, EM_386)?;
    write_u32(bytes, 20, 1)?;
    write_u32(bytes, 24, 0)?;
    write_u32(bytes, 28, 0)?;
    write_u32(bytes, 32, section_header_offset)?;
    write_u32(bytes, 36, 0)?;
    write_u16(bytes, 40, ELF_HEADER_SIZE as u16)?;
    write_u16(bytes, 42, 0)?;
    write_u16(bytes, 44, 0)?;
    write_u16(bytes, 46, SECTION_HEADER_SIZE as u16)?;
    write_u16(bytes, 48, section_count)?;
    write_u16(bytes, 50, shstrtab_index)
}

fn write_section_headers(
    bytes: &mut [u8],
    table_offset: u32,
    headers: &[SectionHeader],
) -> Result<(), Diagnostic> {
    for (index, header) in headers.iter().enumerate() {
        let offset = (table_offset as usize)
            .checked_add(
                index
                    .checked_mul(SECTION_HEADER_SIZE as usize)
                    .ok_or_else(|| object_error("section-header offset overflow"))?,
            )
            .ok_or_else(|| object_error("section-header offset overflow"))?;
        write_u32(bytes, offset, header.name)?;
        write_u32(bytes, offset + 4, header.section_type)?;
        write_u32(bytes, offset + 8, header.flags)?;
        write_u32(bytes, offset + 12, header.address)?;
        write_u32(bytes, offset + 16, header.offset)?;
        write_u32(bytes, offset + 20, header.size)?;
        write_u32(bytes, offset + 24, header.link)?;
        write_u32(bytes, offset + 28, header.info)?;
        write_u32(bytes, offset + 32, header.alignment)?;
        write_u32(bytes, offset + 36, header.entry_size)?;
    }
    Ok(())
}

fn validate_object_header(bytes: &[u8]) -> Result<(), Diagnostic> {
    if bytes.len() < ELF_HEADER_SIZE as usize {
        return Err(invalid_object("truncated ELF header"));
    }
    if bytes.get(..4) != Some(b"\x7fELF") {
        return Err(invalid_object("bad ELF magic"));
    }
    if bytes[4] != 1 || bytes[5] != 1 || bytes[6] != 1 {
        return Err(invalid_object(
            "object is not ELFCLASS32 little-endian EV_CURRENT",
        ));
    }
    if read_u16(bytes, 16)? != ET_REL {
        return Err(invalid_object("ELF object is not ET_REL"));
    }
    let machine = read_u16(bytes, 18)?;
    if machine != EM_386 {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::UnsupportedMachine,
            format!("unsupported ELF machine {machine}; expected EM_386"),
        ));
    }
    if read_u32(bytes, 20)? != 1
        || read_u32(bytes, 24)? != 0
        || read_u32(bytes, 28)? != 0
        || read_u16(bytes, 40)? != ELF_HEADER_SIZE as u16
        || read_u16(bytes, 42)? != 0
        || read_u16(bytes, 44)? != 0
    {
        return Err(invalid_object(
            "unsupported ELF relocatable header contract",
        ));
    }
    Ok(())
}

fn read_section_header(bytes: &[u8], offset: usize) -> Result<SectionHeader, Diagnostic> {
    Ok(SectionHeader {
        name: read_u32(bytes, offset)?,
        section_type: read_u32(bytes, offset + 4)?,
        flags: read_u32(bytes, offset + 8)?,
        address: read_u32(bytes, offset + 12)?,
        offset: read_u32(bytes, offset + 16)?,
        size: read_u32(bytes, offset + 20)?,
        link: read_u32(bytes, offset + 24)?,
        info: read_u32(bytes, offset + 28)?,
        alignment: read_u32(bytes, offset + 32)?,
        entry_size: read_u32(bytes, offset + 36)?,
    })
}

fn validate_allowed_sections(sections: &[ParsedSection]) -> Result<(), Diagnostic> {
    let allowed = BTreeSet::from([
        "",
        ".text",
        ".rodata",
        ".data",
        ".bss",
        ".dbyte.interrupts",
        ".rel.text",
        ".rel.rodata",
        ".rel.data",
        ".symtab",
        ".strtab",
        ".shstrtab",
    ]);
    for section in sections {
        if !allowed.contains(section.name.as_str()) {
            return Err(invalid_object(format!(
                "unsupported object section '{}'",
                section.name
            )));
        }
        if section.header.address != 0 {
            return Err(invalid_object(format!(
                "ET_REL section '{}' has a nonzero address",
                section.name
            )));
        }
    }
    Ok(())
}

fn validate_core_section(section: ObjectSection, header: &SectionHeader) -> Result<(), Diagnostic> {
    let (expected_type, expected_flags) = section_header_contract(section);
    if header.section_type != expected_type
        || header.flags != expected_flags
        || header.link != 0
        || header.info != 0
        || header.entry_size != 0
    {
        return Err(invalid_object(format!(
            "invalid {} section contract",
            section.name()
        )));
    }
    validate_alignment(header.alignment).map_err(|error| invalid_object(error.message))?;
    if header.size as usize > MAX_OBJECT_SECTION_SIZE {
        return Err(invalid_object(format!(
            "{} exceeds section size bound",
            section.name()
        )));
    }
    if section != ObjectSection::Bss && !header.offset.is_multiple_of(header.alignment) {
        return Err(invalid_object(format!(
            "{} file offset is misaligned",
            section.name()
        )));
    }
    Ok(())
}

fn validate_table_headers(
    symtab: &SectionHeader,
    strtab: &SectionHeader,
    strtab_index: u16,
) -> Result<(), Diagnostic> {
    if symtab.section_type != SHT_SYMTAB
        || symtab.link != u32::from(strtab_index)
        || symtab.alignment != 4
        || symtab.entry_size != SYMBOL_SIZE
        || !symtab.size.is_multiple_of(SYMBOL_SIZE)
        || symtab.address != 0
        || symtab.flags != 0
    {
        return Err(invalid_object("invalid .symtab section contract"));
    }
    if strtab.section_type != SHT_STRTAB
        || strtab.alignment != 1
        || strtab.entry_size != 0
        || strtab.address != 0
        || strtab.flags != 0
        || strtab.link != 0
        || strtab.info != 0
    {
        return Err(invalid_object("invalid .strtab section contract"));
    }
    Ok(())
}

fn validate_string_section_header(header: &SectionHeader, name: &str) -> Result<(), Diagnostic> {
    if header.section_type != SHT_STRTAB
        || header.flags != 0
        || header.address != 0
        || header.link != 0
        || header.info != 0
        || header.alignment != 1
        || header.entry_size != 0
    {
        return Err(invalid_object(format!("invalid {name} section contract")));
    }
    Ok(())
}

fn parse_symbols(
    table: &[u8],
    strings: &[u8],
    first_global: u32,
    section_indices: &BTreeMap<u16, ObjectSection>,
    sections: &BTreeMap<ObjectSection, ObjectSectionData>,
) -> Result<Vec<ObjectSymbol>, Diagnostic> {
    if table.len() < SYMBOL_SIZE as usize || !table.len().is_multiple_of(SYMBOL_SIZE as usize) {
        return Err(invalid_object("symbol table has invalid length"));
    }
    let count = table.len() / SYMBOL_SIZE as usize;
    if count - 1 > MAX_OBJECT_SYMBOLS {
        return Err(invalid_object("symbol count exceeds bound"));
    }
    if first_global == 0 || first_global as usize > count {
        return Err(invalid_object("symbol table sh_info is out of bounds"));
    }
    if table[..SYMBOL_SIZE as usize].iter().any(|byte| *byte != 0) {
        return Err(invalid_object("ELF null symbol is not zeroed"));
    }
    let mut symbols = Vec::with_capacity(count - 1);
    let validation_image = ObjectImage {
        sections: sections.clone(),
        symbols: Vec::new(),
        relocations: Vec::new(),
    };
    for elf_index in 1..count {
        let offset = elf_index * SYMBOL_SIZE as usize;
        let name = read_string(strings, read_u32(table, offset)?, "symbol name")?;
        let value = read_u32(table, offset + 4)?;
        let size = read_u32(table, offset + 8)?;
        let info = *table
            .get(offset + 12)
            .ok_or_else(|| invalid_object("truncated symbol info"))?;
        let other = *table
            .get(offset + 13)
            .ok_or_else(|| invalid_object("truncated symbol visibility"))?;
        let section_index = read_u16(table, offset + 14)?;
        if other != 0 {
            return Err(invalid_object("unsupported non-default symbol visibility"));
        }
        let binding = match info >> 4 {
            STB_LOCAL => SymbolBinding::Local,
            STB_GLOBAL => SymbolBinding::Global,
            value => {
                return Err(invalid_object(format!(
                    "unsupported symbol binding {value}"
                )))
            }
        };
        let kind = match info & 0x0f {
            STT_OBJECT => SymbolKind::Object,
            STT_FUNC => SymbolKind::Function,
            STT_SECTION => SymbolKind::Section,
            value => return Err(invalid_object(format!("unsupported symbol type {value}"))),
        };
        if (elf_index < first_global as usize) != (binding == SymbolBinding::Local) {
            return Err(invalid_object(
                "symbol bindings disagree with symtab sh_info",
            ));
        }
        let definition = if section_index == SHN_UNDEF {
            if value != 0 {
                return Err(invalid_object("undefined symbol has a nonzero value"));
            }
            SymbolDefinition::Undefined
        } else {
            let section = section_indices
                .get(&section_index)
                .copied()
                .ok_or_else(|| invalid_object("symbol references unsupported section index"))?;
            SymbolDefinition::Defined { section, value }
        };
        let symbol = ObjectSymbol {
            name,
            binding,
            kind,
            definition,
            size,
        };
        validate_symbol(&validation_image, &symbol)
            .map_err(|error| invalid_object(error.message))?;
        symbols.push(symbol);
    }
    Ok(symbols)
}

fn validate_file_ranges(
    bytes: &[u8],
    sections: &[ParsedSection],
    section_table_offset: u32,
    section_table_size: u32,
) -> Result<(), Diagnostic> {
    let mut ranges = vec![(0u32, ELF_HEADER_SIZE, "ELF header".to_owned())];
    ranges.push((
        section_table_offset,
        checked_add_parse(
            section_table_offset,
            section_table_size,
            "section-header table end",
        )?,
        "section-header table".to_owned(),
    ));
    for section in sections.iter().skip(1) {
        if section.header.section_type == SHT_NOBITS || section.header.size == 0 {
            continue;
        }
        let (_, end) = checked_range(
            bytes,
            section.header.offset,
            section.header.size,
            &section.name,
        )?;
        ranges.push((section.header.offset, end, section.name.clone()));
    }
    ranges.sort_by_key(|range| (range.0, range.1));
    for pair in ranges.windows(2) {
        if pair[0].1 > pair[1].0 {
            return Err(invalid_object(format!(
                "file ranges '{}' and '{}' overlap",
                pair[0].2, pair[1].2
            )));
        }
    }
    Ok(())
}

fn section_file_bytes<'a>(
    bytes: &'a [u8],
    header: &SectionHeader,
    context: &str,
) -> Result<&'a [u8], Diagnostic> {
    if header.section_type == SHT_NOBITS {
        return Err(invalid_object(format!("{context} has no file bytes")));
    }
    let (start, end) = checked_range(bytes, header.offset, header.size, context)?;
    Ok(&bytes[start as usize..end as usize])
}

fn validate_string_table(table: &[u8], name: &str) -> Result<(), Diagnostic> {
    if table.first() != Some(&0) || table.last() != Some(&0) {
        return Err(invalid_object(format!("{name} is not NUL bounded")));
    }
    Ok(())
}

fn read_string(table: &[u8], offset: u32, context: &str) -> Result<String, Diagnostic> {
    let start = usize::try_from(offset)
        .map_err(|_| invalid_object(format!("{context} offset exceeds usize")))?;
    let tail = table
        .get(start..)
        .ok_or_else(|| invalid_object(format!("{context} offset is out of bounds")))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| invalid_object(format!("{context} is not NUL terminated")))?;
    std::str::from_utf8(&tail[..length])
        .map(str::to_owned)
        .map_err(|_| invalid_object(format!("{context} is not UTF-8")))
}

fn object_dump(
    headers: &[SectionHeader],
    names: &[&str],
    image: &ObjectImage,
    symbol_order: &[usize],
    relocations: &[ObjectRelocation],
) -> String {
    let mut dump = String::new();
    let _ = writeln!(dump, "ELF32 little-endian ET_REL EM_386");
    for (index, header) in headers.iter().enumerate().skip(1) {
        let _ = writeln!(
            dump,
            "section {} offset=0x{:08x} size=0x{:x} align={} type={}",
            names[index], header.offset, header.size, header.alignment, header.section_type
        );
    }
    for source_index in symbol_order {
        let symbol = &image.symbols[*source_index];
        let _ = writeln!(
            dump,
            "symbol id={} name={:?} binding={:?} kind={:?} definition={:?} size={}",
            source_index, symbol.name, symbol.binding, symbol.kind, symbol.definition, symbol.size
        );
    }
    for relocation in relocations {
        let _ = writeln!(
            dump,
            "relocation {}+0x{:08x} {:?} symbol={} addend={}",
            relocation.section.name(),
            relocation.offset,
            relocation.kind,
            relocation.symbol.0,
            relocation.addend
        );
    }
    dump
}

fn build_interrupt_metadata(image: &ObjectImage) -> Result<Vec<u8>, Diagnostic> {
    let mut records: Vec<&str> = image
        .symbols
        .iter()
        .filter_map(|symbol| {
            symbol
                .name
                .strip_prefix(INTERRUPT_METADATA_PREFIX)
                .map(|_| symbol.name.as_str())
        })
        .collect();
    records.sort_unstable();
    records.dedup();
    let mut bytes = b"DBYTEINT\0\x01\0".to_vec();
    for record in records {
        if !record.is_ascii() || record.contains('\n') {
            return Err(object_error(
                "interrupt metadata symbol is not canonical ASCII",
            ));
        }
        bytes.extend_from_slice(record.as_bytes());
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn validate_interrupt_metadata(bytes: &[u8], symbols: &[ObjectSymbol]) -> Result<(), Diagnostic> {
    let expected = build_interrupt_metadata(&ObjectImage {
        sections: BTreeMap::new(),
        symbols: symbols.to_vec(),
        relocations: Vec::new(),
    })?;
    if bytes != expected {
        return Err(invalid_object(
            "non-canonical .dbyte.interrupts metadata does not match symbols",
        ));
    }
    Ok(())
}

fn align_up(value: u32, alignment: u32) -> Result<u32, Diagnostic> {
    validate_alignment(alignment)?;
    value
        .checked_add(alignment - 1)
        .map(|sum| sum & !(alignment - 1))
        .ok_or_else(|| object_error("object alignment overflow"))
}

fn checked_add(left: u32, right: u32, context: &str) -> Result<u32, Diagnostic> {
    left.checked_add(right)
        .ok_or_else(|| object_error(format!("{context} overflow")))
}

fn checked_mul(left: u32, right: u32, context: &str) -> Result<u32, Diagnostic> {
    left.checked_mul(right)
        .ok_or_else(|| object_error(format!("{context} overflow")))
}

fn checked_add_parse(left: u32, right: u32, context: &str) -> Result<u32, Diagnostic> {
    left.checked_add(right)
        .ok_or_else(|| invalid_object(format!("{context} overflow")))
}

fn checked_mul_parse(left: u32, right: u32, context: &str) -> Result<u32, Diagnostic> {
    left.checked_mul(right)
        .ok_or_else(|| invalid_object(format!("{context} overflow")))
}

fn checked_usize_u32(value: usize, context: &str) -> Result<u32, Diagnostic> {
    u32::try_from(value).map_err(|_| object_error(format!("{context} exceeds u32")))
}

fn checked_range(
    bytes: &[u8],
    offset: u32,
    size: u32,
    context: &str,
) -> Result<(u32, u32), Diagnostic> {
    let end = checked_add_parse(offset, size, context)?;
    if end as usize > bytes.len() {
        return Err(invalid_object(format!("{context} exceeds object bounds")));
    }
    Ok((offset, end))
}

fn checked_slice_mut<'a>(
    bytes: &'a mut [u8],
    offset: u32,
    size: u32,
    context: &str,
) -> Result<&'a mut [u8], Diagnostic> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| invalid_relocation(format!("{context} range overflow")))?;
    bytes
        .get_mut(offset as usize..end as usize)
        .ok_or_else(|| invalid_relocation(format!("{context} exceeds section bounds")))
}

fn bounded_zeros(length: usize, context: &str) -> Result<Vec<u8>, Diagnostic> {
    if length > MAX_OBJECT_SIZE {
        return Err(object_error(format!("{context} exceeds object size bound")));
    }
    Ok(vec![0; length])
}

fn copy_bytes(destination: &mut [u8], offset: usize, source: &[u8]) -> Result<(), Diagnostic> {
    let end = offset
        .checked_add(source.len())
        .ok_or_else(|| object_error("object byte range overflow"))?;
    destination
        .get_mut(offset..end)
        .ok_or_else(|| object_error("object byte write is out of bounds"))?
        .copy_from_slice(source);
    Ok(())
}

fn write_u8(destination: &mut [u8], offset: usize, value: u8) -> Result<(), Diagnostic> {
    *destination
        .get_mut(offset)
        .ok_or_else(|| object_error("object u8 write is out of bounds"))? = value;
    Ok(())
}

fn write_u16(destination: &mut [u8], offset: usize, value: u16) -> Result<(), Diagnostic> {
    copy_bytes(destination, offset, &value.to_le_bytes())
}

fn write_u32(destination: &mut [u8], offset: usize, value: u32) -> Result<(), Diagnostic> {
    copy_bytes(destination, offset, &value.to_le_bytes())
}

fn read_u16(source: &[u8], offset: usize) -> Result<u16, Diagnostic> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| invalid_object("object u16 read overflow"))?;
    let bytes: [u8; 2] = source
        .get(offset..end)
        .ok_or_else(|| invalid_object("object u16 read is out of bounds"))?
        .try_into()
        .map_err(|_| invalid_object("object u16 read has invalid width"))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(source: &[u8], offset: usize) -> Result<u32, Diagnostic> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| invalid_object("object u32 read overflow"))?;
    let bytes: [u8; 4] = source
        .get(offset..end)
        .ok_or_else(|| invalid_object("object u32 read is out of bounds"))?
        .try_into()
        .map_err(|_| invalid_object("object u32 read has invalid width"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn object_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::Object, message)
}

fn invalid_object(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidObject, message)
}

fn invalid_relocation(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidRelocation, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section_header_offset(bytes: &[u8], wanted: &str) -> usize {
        let table = read_u32(bytes, 32).unwrap() as usize;
        let entry_size = read_u16(bytes, 46).unwrap() as usize;
        let count = read_u16(bytes, 48).unwrap() as usize;
        let names_index = read_u16(bytes, 50).unwrap() as usize;
        let names_header = table + names_index * entry_size;
        let names_offset = read_u32(bytes, names_header + 16).unwrap() as usize;
        let names_size = read_u32(bytes, names_header + 20).unwrap() as usize;
        let names = &bytes[names_offset..names_offset + names_size];
        for index in 0..count {
            let header = table + index * entry_size;
            let name_offset = read_u32(bytes, header).unwrap() as usize;
            let tail = &names[name_offset..];
            let length = tail.iter().position(|byte| *byte == 0).unwrap();
            if std::str::from_utf8(&tail[..length]).unwrap() == wanted {
                return header;
            }
        }
        panic!("missing section {wanted}")
    }

    fn section_data_offset(bytes: &[u8], wanted: &str) -> usize {
        read_u32(bytes, section_header_offset(bytes, wanted) + 16).unwrap() as usize
    }

    fn sample_object() -> ObjectImage {
        let mut image = ObjectImage::empty();
        image.sections.insert(
            ObjectSection::Text,
            ObjectSectionData::bytes(vec![0xe8, 0xfc, 0xff, 0xff, 0xff, 0xc3], 16),
        );
        image.symbols = vec![
            ObjectSymbol {
                name: "caller".to_owned(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Defined {
                    section: ObjectSection::Text,
                    value: 0,
                },
                size: 6,
            },
            ObjectSymbol {
                name: "callee".to_owned(),
                binding: SymbolBinding::Global,
                kind: SymbolKind::Function,
                definition: SymbolDefinition::Undefined,
                size: 0,
            },
        ];
        image.relocations.push(ObjectRelocation {
            section: ObjectSection::Text,
            offset: 1,
            symbol: SymbolId(1),
            kind: RelocationKind::Pc32,
            addend: -4,
        });
        image
    }

    #[test]
    fn writes_and_parses_real_i386_et_rel() {
        let artifact = write_relocatable(&sample_object()).unwrap();
        assert_eq!(&artifact.bytes[..7], b"\x7fELF\x01\x01\x01");
        assert_eq!(read_u16(&artifact.bytes, 16).unwrap(), ET_REL);
        assert_eq!(read_u16(&artifact.bytes, 18).unwrap(), EM_386);
        assert!(artifact.dump.contains(".rel.text"));

        let parsed = parse_relocatable(&artifact.bytes).unwrap();
        assert_eq!(parsed.relocations.len(), 1);
        assert_eq!(parsed.relocations[0].kind, RelocationKind::Pc32);
        assert_eq!(parsed.relocations[0].addend, -4);
        assert_eq!(parsed.symbols[0].name, "callee");
        assert_eq!(parsed.symbols[1].name, "caller");
    }

    #[test]
    fn writer_is_deterministic_and_stores_rel_addend_in_text() {
        let first = write_relocatable(&sample_object()).unwrap();
        let second = write_relocatable(&sample_object()).unwrap();
        assert_eq!(first, second);
        let parsed = parse_relocatable(&first.bytes).unwrap();
        let SectionContents::Bytes(text) = &parsed.sections[&ObjectSection::Text].contents else {
            panic!("text must be bytes")
        };
        assert_eq!(&text[1..5], &(-4i32).to_le_bytes());
    }

    #[test]
    fn malformed_headers_and_ranges_are_rejected_without_panicking() {
        let artifact = write_relocatable(&sample_object()).unwrap();
        let mut bad_magic = artifact.bytes.clone();
        bad_magic[0] = 0;
        assert_eq!(
            parse_relocatable(&bad_magic).unwrap_err().category,
            DiagnosticCategory::InvalidObject
        );

        let mut elf64 = artifact.bytes.clone();
        elf64[4] = 2;
        assert_eq!(
            parse_relocatable(&elf64).unwrap_err().category,
            DiagnosticCategory::InvalidObject
        );

        let mut machine = artifact.bytes.clone();
        machine[18..20].copy_from_slice(&62u16.to_le_bytes());
        assert_eq!(
            parse_relocatable(&machine).unwrap_err().category,
            DiagnosticCategory::UnsupportedMachine
        );

        let mut truncated_table = artifact.bytes.clone();
        truncated_table[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_relocatable(&truncated_table).unwrap_err().category,
            DiagnosticCategory::InvalidObject
        );
    }

    #[test]
    fn malformed_interrupt_metadata_is_rejected_deterministically() {
        let artifact = write_relocatable(&sample_object()).unwrap();
        let mut bytes = artifact.bytes;
        let at = bytes
            .windows(b"DBYTEINT".len())
            .position(|window| window == b"DBYTEINT")
            .expect("metadata magic is present");
        bytes[at] = b'X';
        let error = parse_relocatable(&bytes).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InvalidObject);
        assert!(error.message.contains(".dbyte.interrupts"));
    }

    #[test]
    fn adversarial_section_symbol_and_relocation_matrix_is_bounded() {
        let artifact = write_relocatable(&sample_object()).unwrap();

        let mut bad_name = artifact.bytes.clone();
        let text_header = section_header_offset(&bad_name, ".text");
        bad_name[text_header..text_header + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_relocatable(&bad_name).unwrap_err().category,
            DiagnosticCategory::InvalidObject
        );

        let mut bad_symtab_link = artifact.bytes.clone();
        let symtab_header = section_header_offset(&bad_symtab_link, ".symtab");
        bad_symtab_link[symtab_header + 24..symtab_header + 28]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_relocatable(&bad_symtab_link).unwrap_err().category,
            DiagnosticCategory::InvalidObject
        );

        let relocation_offset = section_data_offset(&artifact.bytes, ".rel.text");
        let mut bad_relocation_offset = artifact.bytes.clone();
        bad_relocation_offset[relocation_offset..relocation_offset + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            parse_relocatable(&bad_relocation_offset)
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidRelocation
        );

        let mut bad_symbol_index = artifact.bytes.clone();
        bad_symbol_index[relocation_offset + 4..relocation_offset + 8]
            .copy_from_slice(&((0x00ff_ffffu32 << 8) | u32::from(R_386_PC32)).to_le_bytes());
        assert_eq!(
            parse_relocatable(&bad_symbol_index).unwrap_err().category,
            DiagnosticCategory::InvalidRelocation
        );

        let mut bad_relocation_type = artifact.bytes.clone();
        let info = read_u32(&bad_relocation_type, relocation_offset + 4).unwrap();
        bad_relocation_type[relocation_offset + 4..relocation_offset + 8]
            .copy_from_slice(&((info & !0xff) | 127).to_le_bytes());
        assert_eq!(
            parse_relocatable(&bad_relocation_type)
                .unwrap_err()
                .category,
            DiagnosticCategory::InvalidRelocation
        );

        let mut overflowing_range = artifact.bytes.clone();
        overflowing_range[text_header + 16..text_header + 20]
            .copy_from_slice(&(u32::MAX - 1).to_le_bytes());
        overflowing_range[text_header + 20..text_header + 24].copy_from_slice(&4u32.to_le_bytes());
        assert_eq!(
            parse_relocatable(&overflowing_range).unwrap_err().category,
            DiagnosticCategory::InvalidObject
        );

        let mut overlapping_ranges = artifact.bytes;
        let rodata_header = section_header_offset(&overlapping_ranges, ".rodata");
        let text_offset = read_u32(&overlapping_ranges, text_header + 16).unwrap();
        overlapping_ranges[rodata_header + 16..rodata_header + 20]
            .copy_from_slice(&text_offset.to_le_bytes());
        overlapping_ranges[rodata_header + 20..rodata_header + 24]
            .copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            parse_relocatable(&overlapping_ranges).unwrap_err().category,
            DiagnosticCategory::InvalidObject
        );
    }

    #[test]
    fn bad_relocation_symbol_and_offset_are_rejected() {
        let mut image = sample_object();
        image.relocations[0].symbol = SymbolId(99);
        assert_eq!(
            write_relocatable(&image).unwrap_err().category,
            DiagnosticCategory::InvalidRelocation
        );

        let mut image = sample_object();
        image.relocations[0].offset = 4;
        assert_eq!(
            write_relocatable(&image).unwrap_err().category,
            DiagnosticCategory::InvalidRelocation
        );
    }
}
