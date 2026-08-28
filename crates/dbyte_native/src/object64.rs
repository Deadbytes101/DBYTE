//! Canonical ELF64/x86-64 relocatable objects for DBYTE64 targets.

use crate::target::{decode_target_metadata, encode_target_metadata, TargetDescriptor};
use std::collections::{BTreeMap, BTreeSet};

const ELF_HEADER_SIZE: u64 = 64;
const SECTION_HEADER_SIZE: u64 = 64;
const SYMBOL_SIZE: u64 = 24;
const RELA_SIZE: u64 = 24;
const SECTION_COUNT: u16 = 13;
const MAX_OBJECT_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Text,
    Rodata,
    Data,
    Bss,
}

impl Section {
    pub const ALL: [Self; 4] = [Self::Text, Self::Rodata, Self::Data, Self::Bss];

    fn index(self) -> u16 {
        match self {
            Self::Text => 1,
            Self::Rodata => 2,
            Self::Data => 3,
            Self::Bss => 4,
        }
    }

    fn relocation_index(self) -> Option<u16> {
        match self {
            Self::Text => Some(5),
            Self::Rodata => Some(6),
            Self::Data => Some(7),
            Self::Bss => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contents {
    Bytes(Vec<u8>),
    Uninitialized(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionData {
    pub contents: Contents,
    pub alignment: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Binding {
    Local,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Object,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    Undefined,
    Defined { section: Section, value: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub binding: Binding,
    pub kind: SymbolKind,
    pub definition: Definition,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RelocationKind {
    Absolute64 = 1,
    Pc32 = 2,
    Plt32 = 4,
    Absolute32 = 10,
    Absolute32Signed = 11,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relocation {
    pub section: Section,
    pub offset: u64,
    pub symbol: String,
    pub kind: RelocationKind,
    pub addend: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectImage {
    pub target: TargetDescriptor,
    pub entry_abi: Option<EntryAbi>,
    pub sections: BTreeMap<Section, SectionData>,
    pub symbols: Vec<Symbol>,
    pub relocations: Vec<Relocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryAbi {
    ZeroArguments,
    LinuxProcessArguments,
}

const ENTRY_MAGIC: &[u8; 8] = b"DBYENT64";
const ENTRY_SIZE: usize = 16;

fn encode_entry(entry: EntryAbi) -> [u8; ENTRY_SIZE] {
    let mut bytes = [0u8; ENTRY_SIZE];
    bytes[..8].copy_from_slice(ENTRY_MAGIC);
    bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&(ENTRY_SIZE as u16).to_le_bytes());
    bytes[12] = match entry {
        EntryAbi::ZeroArguments => 0,
        EntryAbi::LinuxProcessArguments => 3,
    };
    bytes
}

fn decode_entry(bytes: &[u8]) -> Result<Option<EntryAbi>, ObjectError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    if bytes.len() != ENTRY_SIZE || &bytes[..8] != ENTRY_MAGIC {
        return Err(error("E_OBJECT64_ENTRY", "malformed .dbyte.entry record"));
    }
    if u16::from_le_bytes([bytes[8], bytes[9]]) != 1
        || usize::from(u16::from_le_bytes([bytes[10], bytes[11]])) != ENTRY_SIZE
        || bytes[13..].iter().any(|byte| *byte != 0)
    {
        return Err(error(
            "E_OBJECT64_ENTRY",
            "noncanonical .dbyte.entry record",
        ));
    }
    match bytes[12] {
        0 => Ok(Some(EntryAbi::ZeroArguments)),
        3 => Ok(Some(EntryAbi::LinuxProcessArguments)),
        _ => Err(error("E_OBJECT64_ENTRY", "unsupported entry ABI")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectArtifact {
    pub bytes: Vec<u8>,
    pub symbol_count: usize,
    pub relocation_count: usize,
}

pub fn parse_relocatable(bytes: &[u8]) -> Result<ObjectImage, ObjectError> {
    let target = read_target(bytes)?;
    let shoff = get_u64(bytes, 40)?;
    let section_header = |index: u16| -> Result<u64, ObjectError> {
        shoff
            .checked_add(u64::from(index) * SECTION_HEADER_SIZE)
            .ok_or_else(|| error("E_OBJECT64_OVERFLOW", "section header overflows"))
    };
    let mut sections = BTreeMap::new();
    for section in Section::ALL {
        let header = section_header(section.index())?;
        let kind = get_u32(bytes, header + 4)?;
        let offset = get_u64(bytes, header + 24)?;
        let size = get_u64(bytes, header + 32)?;
        let alignment = get_u64(bytes, header + 48)?;
        let contents = if section == Section::Bss {
            if kind != 8 {
                return Err(error("E_OBJECT64_SECTION_KIND", "BSS must be SHT_NOBITS"));
            }
            Contents::Uninitialized(size)
        } else {
            if kind != 1 {
                return Err(error(
                    "E_OBJECT64_SECTION_KIND",
                    "loadable section must be SHT_PROGBITS",
                ));
            }
            Contents::Bytes(range(bytes, offset, size)?.to_vec())
        };
        if size != 0 {
            sections.insert(
                section,
                SectionData {
                    contents,
                    alignment,
                },
            );
        }
    }

    let entry_header = section_header(9)?;
    let entry_abi = decode_entry(range(
        bytes,
        get_u64(bytes, entry_header + 24)?,
        get_u64(bytes, entry_header + 32)?,
    )?)?;
    let strtab_header = section_header(11)?;
    let strtab = range(
        bytes,
        get_u64(bytes, strtab_header + 24)?,
        get_u64(bytes, strtab_header + 32)?,
    )?;
    if strtab.first() != Some(&0) {
        return Err(error("E_OBJECT64_STRTAB", "string table lacks empty entry"));
    }
    let symtab_header = section_header(10)?;
    if get_u32(bytes, symtab_header + 4)? != 2 || get_u64(bytes, symtab_header + 56)? != SYMBOL_SIZE
    {
        return Err(error("E_OBJECT64_SYMTAB", "invalid canonical symbol table"));
    }
    let symtab = range(
        bytes,
        get_u64(bytes, symtab_header + 24)?,
        get_u64(bytes, symtab_header + 32)?,
    )?;
    if symtab.len() % SYMBOL_SIZE as usize != 0 || symtab.len() < 5 * SYMBOL_SIZE as usize {
        return Err(error("E_OBJECT64_SYMTAB", "malformed symbol table size"));
    }
    let symbol_count = symtab.len() / SYMBOL_SIZE as usize;
    let mut indexed_names = vec![String::new(); symbol_count];
    let mut symbols = Vec::with_capacity(symbol_count.saturating_sub(5));
    for (index, indexed_name) in indexed_names
        .iter_mut()
        .enumerate()
        .take(symbol_count)
        .skip(5)
    {
        let offset = index * SYMBOL_SIZE as usize;
        let name_offset = u32::from_le_bytes(symtab[offset..offset + 4].try_into().unwrap());
        let name = read_name(strtab, name_offset)?;
        *indexed_name = name.clone();
        let info = symtab[offset + 4];
        let binding = match info >> 4 {
            0 => Binding::Local,
            1 => Binding::Global,
            _ => {
                return Err(error(
                    "E_OBJECT64_SYMBOL_BINDING",
                    "unsupported symbol binding",
                ))
            }
        };
        let kind = match info & 0x0f {
            1 => SymbolKind::Object,
            2 => SymbolKind::Function,
            _ => return Err(error("E_OBJECT64_SYMBOL_KIND", "unsupported symbol kind")),
        };
        let section_index = u16::from_le_bytes(symtab[offset + 6..offset + 8].try_into().unwrap());
        let value = u64::from_le_bytes(symtab[offset + 8..offset + 16].try_into().unwrap());
        let size = u64::from_le_bytes(symtab[offset + 16..offset + 24].try_into().unwrap());
        let definition = match section_index {
            0 => Definition::Undefined,
            1 => Definition::Defined {
                section: Section::Text,
                value,
            },
            2 => Definition::Defined {
                section: Section::Rodata,
                value,
            },
            3 => Definition::Defined {
                section: Section::Data,
                value,
            },
            4 => Definition::Defined {
                section: Section::Bss,
                value,
            },
            _ => {
                return Err(error(
                    "E_OBJECT64_SYMBOL_SECTION",
                    "symbol references invalid section",
                ))
            }
        };
        symbols.push(Symbol {
            name,
            binding,
            kind,
            definition,
            size,
        });
    }

    let mut relocations = Vec::new();
    for (section, relocation_index) in [
        (Section::Text, 5u16),
        (Section::Rodata, 6u16),
        (Section::Data, 7u16),
    ] {
        let header = section_header(relocation_index)?;
        if get_u32(bytes, header + 4)? != 4 || get_u64(bytes, header + 56)? != RELA_SIZE {
            return Err(error(
                "E_OBJECT64_RELOCATION",
                "invalid canonical RELA section",
            ));
        }
        let entries = range(
            bytes,
            get_u64(bytes, header + 24)?,
            get_u64(bytes, header + 32)?,
        )?;
        if entries.len() % RELA_SIZE as usize != 0 {
            return Err(error(
                "E_OBJECT64_RELOCATION",
                "malformed RELA section size",
            ));
        }
        for entry in entries.chunks_exact(RELA_SIZE as usize) {
            let offset = u64::from_le_bytes(entry[0..8].try_into().unwrap());
            let info = u64::from_le_bytes(entry[8..16].try_into().unwrap());
            let symbol_index = usize::try_from(info >> 32)
                .map_err(|_| error("E_OBJECT64_RELOCATION_SYMBOL", "symbol index exceeds host"))?;
            let symbol = indexed_names
                .get(symbol_index)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    error(
                        "E_OBJECT64_RELOCATION_SYMBOL",
                        "relocation references invalid symbol",
                    )
                })?
                .clone();
            let kind = match info as u32 {
                1 => RelocationKind::Absolute64,
                2 => RelocationKind::Pc32,
                4 => RelocationKind::Plt32,
                10 => RelocationKind::Absolute32,
                11 => RelocationKind::Absolute32Signed,
                _ => {
                    return Err(error(
                        "E_OBJECT64_RELOCATION_KIND",
                        "unsupported x86_64 relocation",
                    ))
                }
            };
            let addend = i64::from_le_bytes(entry[16..24].try_into().unwrap());
            relocations.push(Relocation {
                section,
                offset,
                symbol,
                kind,
                addend,
            });
        }
    }
    let image = ObjectImage {
        target,
        entry_abi,
        sections,
        symbols,
        relocations,
    };
    validate_sections(&image)?;
    Ok(image)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectError(pub String);

impl std::fmt::Display for ObjectError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ObjectError {}

pub fn write_relocatable(image: &ObjectImage) -> Result<ObjectArtifact, ObjectError> {
    if image.target.machine != crate::target::MachineTarget::X86_64 {
        return Err(error(
            "E_OBJECT64_TARGET",
            "ELF64 object requires x86_64 target",
        ));
    }
    validate_sections(image)?;
    let mut seen = BTreeSet::new();
    for symbol in &image.symbols {
        if symbol.name.is_empty() || !seen.insert(symbol.name.clone()) {
            return Err(error(
                "E_OBJECT64_SYMBOL",
                "symbol names must be unique and nonempty",
            ));
        }
        if let Definition::Defined { section, value } = symbol.definition {
            if value > section_size(image, section)? {
                return Err(error(
                    "E_OBJECT64_SYMBOL_RANGE",
                    "defined symbol exceeds its section",
                ));
            }
        }
    }
    let mut locals: Vec<_> = image
        .symbols
        .iter()
        .filter(|symbol| symbol.binding == Binding::Local)
        .collect();
    let mut globals: Vec<_> = image
        .symbols
        .iter()
        .filter(|symbol| symbol.binding == Binding::Global)
        .collect();
    locals.sort_by(|left, right| left.name.cmp(&right.name));
    globals.sort_by(|left, right| left.name.cmp(&right.name));
    let ordered: Vec<_> = locals.into_iter().chain(globals).collect();

    let mut strtab = vec![0u8];
    let mut name_offsets = BTreeMap::new();
    for symbol in &ordered {
        let offset = u32::try_from(strtab.len())
            .map_err(|_| error("E_OBJECT64_SIZE", "string table exceeds u32"))?;
        name_offsets.insert(symbol.name.clone(), offset);
        strtab.extend_from_slice(symbol.name.as_bytes());
        strtab.push(0);
    }
    let mut symbol_indices = BTreeMap::new();
    for (index, symbol) in ordered.iter().enumerate() {
        symbol_indices.insert(symbol.name.clone(), (index + 5) as u32);
    }
    let mut relocations = image.relocations.clone();
    relocations.sort_by(|left, right| {
        (
            left.section,
            left.offset,
            &left.symbol,
            left.kind as u32,
            left.addend,
        )
            .cmp(&(
                right.section,
                right.offset,
                &right.symbol,
                right.kind as u32,
                right.addend,
            ))
    });
    for relocation in &relocations {
        if relocation.section == Section::Bss {
            return Err(error(
                "E_OBJECT64_RELOCATION",
                "BSS relocations are unsupported",
            ));
        }
        let width = match relocation.kind {
            RelocationKind::Absolute64 => 8,
            RelocationKind::Pc32
            | RelocationKind::Plt32
            | RelocationKind::Absolute32
            | RelocationKind::Absolute32Signed => 4,
        };
        if relocation
            .offset
            .checked_add(width)
            .is_none_or(|end| end > section_size(image, relocation.section).unwrap_or(0))
        {
            return Err(error(
                "E_OBJECT64_RELOCATION_RANGE",
                "relocation exceeds section bounds",
            ));
        }
        if !symbol_indices.contains_key(&relocation.symbol) {
            return Err(error(
                "E_OBJECT64_RELOCATION_SYMBOL",
                "relocation references unknown symbol",
            ));
        }
    }

    let mut section_bytes = BTreeMap::<u16, Vec<u8>>::new();
    for section in Section::ALL {
        if let Some(data) = image.sections.get(&section) {
            if let Contents::Bytes(bytes) = &data.contents {
                section_bytes.insert(section.index(), bytes.clone());
            }
        }
    }
    for section in [Section::Text, Section::Rodata, Section::Data] {
        let mut bytes = Vec::new();
        for relocation in relocations
            .iter()
            .filter(|relocation| relocation.section == section)
        {
            bytes.extend_from_slice(&relocation.offset.to_le_bytes());
            let info = (u64::from(symbol_indices[&relocation.symbol]) << 32)
                | u64::from(relocation.kind as u32);
            bytes.extend_from_slice(&info.to_le_bytes());
            bytes.extend_from_slice(&relocation.addend.to_le_bytes());
        }
        section_bytes.insert(section.relocation_index().unwrap(), bytes);
    }
    section_bytes.insert(8, encode_target_metadata(image.target).to_vec());
    section_bytes.insert(
        9,
        image
            .entry_abi
            .map_or_else(Vec::new, |entry| encode_entry(entry).to_vec()),
    );

    let mut symtab = vec![0u8; (5 + ordered.len()) * SYMBOL_SIZE as usize];
    for section in Section::ALL {
        let offset = section.index() as usize * SYMBOL_SIZE as usize;
        symtab[offset + 4] = 3;
        symtab[offset + 6..offset + 8].copy_from_slice(&section.index().to_le_bytes());
    }
    for (index, symbol) in ordered.iter().enumerate() {
        let offset = (index + 5) * SYMBOL_SIZE as usize;
        symtab[offset..offset + 4].copy_from_slice(&name_offsets[&symbol.name].to_le_bytes());
        symtab[offset + 4] = ((symbol.binding == Binding::Global) as u8) << 4
            | match symbol.kind {
                SymbolKind::Object => 1,
                SymbolKind::Function => 2,
            };
        match symbol.definition {
            Definition::Undefined => {}
            Definition::Defined { section, value } => {
                symtab[offset + 6..offset + 8].copy_from_slice(&section.index().to_le_bytes());
                symtab[offset + 8..offset + 16].copy_from_slice(&value.to_le_bytes());
            }
        }
        symtab[offset + 16..offset + 24].copy_from_slice(&symbol.size.to_le_bytes());
    }
    section_bytes.insert(10, symtab);
    section_bytes.insert(11, strtab);
    let shstrtab = b"\0.text\0.rodata\0.data\0.bss\0.rela.text\0.rela.rodata\0.rela.data\0.dbyte.target\0.dbyte.entry\0.symtab\0.strtab\0.shstrtab\0".to_vec();
    section_bytes.insert(12, shstrtab);

    let alignments = [1u64, 16, 1, 8, 8, 8, 8, 8, 1, 1, 8, 1, 1];
    let mut offsets = [0u64; SECTION_COUNT as usize];
    let mut cursor = ELF_HEADER_SIZE;
    for index in 1..SECTION_COUNT as usize {
        if index == 4 {
            continue;
        }
        cursor = align_up(cursor, alignments[index])?;
        offsets[index] = cursor;
        cursor = cursor
            .checked_add(
                section_bytes
                    .get(&(index as u16))
                    .map_or(0, |bytes| bytes.len() as u64),
            )
            .ok_or_else(|| error("E_OBJECT64_OVERFLOW", "section layout overflows"))?;
    }
    let shoff = align_up(cursor, 8)?;
    let file_size = shoff
        .checked_add(u64::from(SECTION_COUNT) * SECTION_HEADER_SIZE)
        .ok_or_else(|| error("E_OBJECT64_OVERFLOW", "section headers overflow"))?;
    let file_len = usize::try_from(file_size)
        .map_err(|_| error("E_OBJECT64_SIZE", "object exceeds host addressability"))?;
    if file_len > MAX_OBJECT_SIZE {
        return Err(error("E_OBJECT64_SIZE", "object exceeds 64 MiB bound"));
    }
    let mut output = vec![0u8; file_len];
    output[..4].copy_from_slice(b"\x7fELF");
    output[4] = 2;
    output[5] = 1;
    output[6] = 1;
    put_u16(&mut output, 16, 1)?;
    put_u16(&mut output, 18, 62)?;
    put_u32(&mut output, 20, 1)?;
    put_u64(&mut output, 40, shoff)?;
    put_u16(&mut output, 52, ELF_HEADER_SIZE as u16)?;
    put_u16(&mut output, 58, SECTION_HEADER_SIZE as u16)?;
    put_u16(&mut output, 60, SECTION_COUNT)?;
    put_u16(&mut output, 62, 12)?;
    for (index, bytes) in &section_bytes {
        copy_at(&mut output, offsets[*index as usize], bytes)?;
    }

    let names = [0, 1, 7, 15, 21, 26, 37, 50, 61, 75, 88, 96, 104];
    for index in 1..SECTION_COUNT as usize {
        let header = shoff + index as u64 * SECTION_HEADER_SIZE;
        let (kind, flags, link, info, entry_size) = match index {
            1 => (1, 0x6, 0, 0, 0),
            2 => (1, 0x2, 0, 0, 0),
            3 => (1, 0x3, 0, 0, 0),
            4 => (8, 0x3, 0, 0, 0),
            5..=7 => (4, 0, 10, index as u32 - 4, RELA_SIZE),
            8 | 9 => (1, 0, 0, 0, 0),
            10 => (
                2,
                0,
                11,
                (5 + ordered
                    .iter()
                    .take_while(|symbol| symbol.binding == Binding::Local)
                    .count()) as u32,
                SYMBOL_SIZE,
            ),
            11 | 12 => (3, 0, 0, 0, 0),
            _ => unreachable!(),
        };
        put_u32(&mut output, header, names[index])?;
        put_u32(&mut output, header + 4, kind)?;
        put_u64(&mut output, header + 8, flags)?;
        put_u64(&mut output, header + 24, offsets[index])?;
        let size = if index == 4 {
            section_size(image, Section::Bss)?
        } else {
            section_bytes
                .get(&(index as u16))
                .map_or(0, |bytes| bytes.len() as u64)
        };
        put_u64(&mut output, header + 32, size)?;
        put_u32(&mut output, header + 40, link)?;
        put_u32(&mut output, header + 44, info)?;
        put_u64(&mut output, header + 48, alignments[index])?;
        put_u64(&mut output, header + 56, entry_size)?;
    }
    Ok(ObjectArtifact {
        bytes: output,
        symbol_count: ordered.len(),
        relocation_count: relocations.len(),
    })
}

pub fn read_target(bytes: &[u8]) -> Result<TargetDescriptor, ObjectError> {
    if bytes.len() < ELF_HEADER_SIZE as usize
        || &bytes[..4] != b"\x7fELF"
        || bytes[4] != 2
        || bytes[5] != 1
    {
        return Err(error("E_OBJECT64_HEADER", "invalid ELF64 identification"));
    }
    if get_u16(bytes, 16)? != 1 || get_u16(bytes, 18)? != 62 {
        return Err(error("E_OBJECT64_KIND", "expected ELF64 ET_REL EM_X86_64"));
    }
    let shoff = get_u64(bytes, 40)?;
    let shentsize = get_u16(bytes, 58)?;
    let shnum = get_u16(bytes, 60)?;
    if shentsize != SECTION_HEADER_SIZE as u16 || shnum != SECTION_COUNT {
        return Err(error(
            "E_OBJECT64_SECTIONS",
            "unexpected canonical section table",
        ));
    }
    let header = shoff
        .checked_add(8 * SECTION_HEADER_SIZE)
        .ok_or_else(|| error("E_OBJECT64_OVERFLOW", "target section header overflows"))?;
    let offset = get_u64(bytes, header + 24)?;
    let size = get_u64(bytes, header + 32)?;
    let target = range(bytes, offset, size)?;
    decode_target_metadata(target).map_err(|error| ObjectError(error.0))
}

fn validate_sections(image: &ObjectImage) -> Result<(), ObjectError> {
    for section in Section::ALL {
        let Some(data) = image.sections.get(&section) else {
            continue;
        };
        if data.alignment == 0 || !data.alignment.is_power_of_two() || data.alignment > 4096 {
            return Err(error(
                "E_OBJECT64_ALIGNMENT",
                "section alignment must be a power of two <= 4096",
            ));
        }
        match (&data.contents, section) {
            (Contents::Uninitialized(_), Section::Bss) | (Contents::Bytes(_), _) => {}
            _ => {
                return Err(error(
                    "E_OBJECT64_SECTION_KIND",
                    "only BSS may be uninitialized",
                ))
            }
        }
    }
    Ok(())
}

fn section_size(image: &ObjectImage, section: Section) -> Result<u64, ObjectError> {
    Ok(
        match image.sections.get(&section).map(|data| &data.contents) {
            Some(Contents::Bytes(bytes)) => bytes.len() as u64,
            Some(Contents::Uninitialized(size)) => *size,
            None => 0,
        },
    )
}

fn align_up(value: u64, alignment: u64) -> Result<u64, ObjectError> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| error("E_OBJECT64_OVERFLOW", "alignment overflows"))
}

fn range(bytes: &[u8], offset: u64, size: u64) -> Result<&[u8], ObjectError> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| error("E_OBJECT64_OVERFLOW", "range overflows"))?;
    let start =
        usize::try_from(offset).map_err(|_| error("E_OBJECT64_RANGE", "offset exceeds host"))?;
    let end = usize::try_from(end).map_err(|_| error("E_OBJECT64_RANGE", "end exceeds host"))?;
    bytes
        .get(start..end)
        .ok_or_else(|| error("E_OBJECT64_RANGE", "range exceeds object"))
}

fn copy_at(output: &mut [u8], offset: u64, bytes: &[u8]) -> Result<(), ObjectError> {
    let target = range_mut(output, offset, bytes.len() as u64)?;
    target.copy_from_slice(bytes);
    Ok(())
}

fn range_mut(bytes: &mut [u8], offset: u64, size: u64) -> Result<&mut [u8], ObjectError> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| error("E_OBJECT64_OVERFLOW", "range overflows"))?;
    let start =
        usize::try_from(offset).map_err(|_| error("E_OBJECT64_RANGE", "offset exceeds host"))?;
    let end = usize::try_from(end).map_err(|_| error("E_OBJECT64_RANGE", "end exceeds host"))?;
    bytes
        .get_mut(start..end)
        .ok_or_else(|| error("E_OBJECT64_RANGE", "range exceeds object"))
}

fn put_u16(bytes: &mut [u8], offset: u64, value: u16) -> Result<(), ObjectError> {
    copy_at(bytes, offset, &value.to_le_bytes())
}
fn put_u32(bytes: &mut [u8], offset: u64, value: u32) -> Result<(), ObjectError> {
    copy_at(bytes, offset, &value.to_le_bytes())
}
fn put_u64(bytes: &mut [u8], offset: u64, value: u64) -> Result<(), ObjectError> {
    copy_at(bytes, offset, &value.to_le_bytes())
}
fn get_u16(bytes: &[u8], offset: u64) -> Result<u16, ObjectError> {
    Ok(u16::from_le_bytes(
        range(bytes, offset, 2)?.try_into().unwrap(),
    ))
}
fn get_u32(bytes: &[u8], offset: u64) -> Result<u32, ObjectError> {
    Ok(u32::from_le_bytes(
        range(bytes, offset, 4)?.try_into().unwrap(),
    ))
}
fn get_u64(bytes: &[u8], offset: u64) -> Result<u64, ObjectError> {
    Ok(u64::from_le_bytes(
        range(bytes, offset, 8)?.try_into().unwrap(),
    ))
}
fn read_name(bytes: &[u8], offset: u32) -> Result<String, ObjectError> {
    let start = usize::try_from(offset)
        .map_err(|_| error("E_OBJECT64_STRTAB", "name offset exceeds host"))?;
    let tail = bytes
        .get(start..)
        .ok_or_else(|| error("E_OBJECT64_STRTAB", "name offset exceeds string table"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| error("E_OBJECT64_STRTAB", "unterminated symbol name"))?;
    let name = std::str::from_utf8(&tail[..length])
        .map_err(|_| error("E_OBJECT64_STRTAB", "symbol name is not UTF-8"))?;
    if name.is_empty() {
        return Err(error("E_OBJECT64_SYMBOL", "symbol name must not be empty"));
    }
    Ok(name.to_owned())
}
fn error(code: &str, message: &str) -> ObjectError {
    ObjectError(format!("{code}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::X86_64_LINUX;

    #[test]
    fn writes_elf64_rel_rela_symbols_and_target_metadata_deterministically() {
        let image = ObjectImage {
            target: X86_64_LINUX,
            entry_abi: Some(EntryAbi::LinuxProcessArguments),
            sections: BTreeMap::from([
                (
                    Section::Text,
                    SectionData {
                        contents: Contents::Bytes(vec![0xe8, 0, 0, 0, 0, 0xc3]),
                        alignment: 16,
                    },
                ),
                (
                    Section::Rodata,
                    SectionData {
                        contents: Contents::Bytes(b"DBYTE64".to_vec()),
                        alignment: 1,
                    },
                ),
                (
                    Section::Bss,
                    SectionData {
                        contents: Contents::Uninitialized(16),
                        alignment: 8,
                    },
                ),
            ]),
            symbols: vec![
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
                    name: "other".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            relocations: vec![Relocation {
                section: Section::Text,
                offset: 1,
                symbol: "other".into(),
                kind: RelocationKind::Plt32,
                addend: -4,
            }],
        };
        let first = write_relocatable(&image).unwrap();
        let second = write_relocatable(&image).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.bytes[4], 2);
        assert_eq!(read_target(&first.bytes).unwrap(), X86_64_LINUX);
        assert_eq!(parse_relocatable(&first.bytes).unwrap(), image);
        assert_eq!(first.relocation_count, 1);
        let mut malformed = first.bytes.clone();
        let shoff = get_u64(&malformed, 40).unwrap();
        let entry_header = shoff + 9 * SECTION_HEADER_SIZE;
        let entry_offset = get_u64(&malformed, entry_header + 24).unwrap() as usize;
        malformed[entry_offset + 12] = 2;
        assert!(parse_relocatable(&malformed)
            .unwrap_err()
            .0
            .contains("E_OBJECT64_ENTRY"));
    }

    #[test]
    fn rejects_mixed_or_malformed_target_metadata() {
        let image = ObjectImage {
            target: crate::target::I686_NONE,
            entry_abi: None,
            sections: BTreeMap::new(),
            symbols: vec![],
            relocations: vec![],
        };
        assert!(write_relocatable(&image)
            .unwrap_err()
            .0
            .contains("E_OBJECT64_TARGET"));
    }
}
