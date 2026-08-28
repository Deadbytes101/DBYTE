//! Deterministic AMD64 COFF objects for the Windows DBYTE64 target.

use crate::object64::{
    Binding, Contents, Definition, ObjectArtifact, ObjectImage, Relocation, RelocationKind,
    Section, SectionData, Symbol, SymbolKind,
};
use crate::target::{decode_target_metadata, encode_target_metadata, X86_64_WINDOWS};
use std::collections::{BTreeMap, BTreeSet};

const FILE_HEADER_SIZE: usize = 20;
const SECTION_HEADER_SIZE: usize = 40;
const SYMBOL_SIZE: usize = 18;
const RELOCATION_SIZE: usize = 10;
const SECTION_COUNT: usize = 5;
const MAX_OBJECT_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoffError(pub String);

impl std::fmt::Display for CoffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CoffError {}

pub fn write_object(image: &ObjectImage) -> Result<ObjectArtifact, CoffError> {
    if image.target != X86_64_WINDOWS {
        return Err(error(
            "E_COFF_TARGET",
            "COFF writer requires x86_64-dbyte-windows",
        ));
    }
    let mut symbols = image.symbols.clone();
    symbols.sort_by(|left, right| {
        (left.binding as u8, &left.name).cmp(&(right.binding as u8, &right.name))
    });
    let mut seen = BTreeSet::new();
    for symbol in &symbols {
        if symbol.name.is_empty() || !symbol.name.is_ascii() || !seen.insert(symbol.name.clone()) {
            return Err(error(
                "E_COFF_SYMBOL",
                "symbol names must be unique nonempty ASCII",
            ));
        }
        if let Definition::Defined { value, .. } = symbol.definition {
            u32::try_from(value).map_err(|_| error("E_COFF_RANGE", "symbol value exceeds u32"))?;
        }
    }
    let symbol_indices = symbols
        .iter()
        .enumerate()
        .map(|(index, symbol)| (symbol.name.clone(), index as u32))
        .collect::<BTreeMap<_, _>>();

    let mut section_bytes = BTreeMap::<usize, Vec<u8>>::new();
    let mut section_sizes = BTreeMap::<usize, u32>::new();
    for section in Section::ALL {
        let index = section_index(section);
        if let Some(data) = image.sections.get(&section) {
            validate_alignment(data.alignment)?;
            match &data.contents {
                Contents::Bytes(bytes) => {
                    section_sizes.insert(index, checked_u32(bytes.len(), "section size")?);
                    section_bytes.insert(index, bytes.clone());
                }
                Contents::Uninitialized(size) => {
                    section_sizes.insert(
                        index,
                        u32::try_from(*size)
                            .map_err(|_| error("E_COFF_RANGE", "BSS size exceeds u32"))?,
                    );
                }
            }
        }
    }
    section_bytes.insert(5, encode_target_metadata(image.target).to_vec());
    section_sizes.insert(5, 32);

    let mut relocations = BTreeMap::<usize, Vec<&Relocation>>::new();
    for relocation in &image.relocations {
        let index = section_index(relocation.section);
        let target = section_bytes.get_mut(&index).ok_or_else(|| {
            error(
                "E_COFF_RELOCATION",
                "relocation target has no initialized bytes",
            )
        })?;
        let width = relocation_width(relocation.kind);
        let start = usize::try_from(relocation.offset)
            .map_err(|_| error("E_COFF_RELOCATION", "relocation offset exceeds host bounds"))?;
        let end = start
            .checked_add(width)
            .ok_or_else(|| error("E_COFF_RELOCATION", "relocation range overflows"))?;
        let slot = target
            .get_mut(start..end)
            .ok_or_else(|| error("E_COFF_RELOCATION", "relocation exceeds target section"))?;
        if !symbol_indices.contains_key(&relocation.symbol) {
            return Err(error(
                "E_COFF_RELOCATION",
                "relocation references unknown symbol",
            ));
        }
        match width {
            8 => slot.copy_from_slice(&relocation.addend.to_le_bytes()),
            4 => slot.copy_from_slice(
                &i32::try_from(relocation.addend)
                    .map_err(|_| error("E_COFF_RELOCATION", "32-bit addend overflows"))?
                    .to_le_bytes(),
            ),
            _ => unreachable!(),
        }
        relocations.entry(index).or_default().push(relocation);
    }
    for values in relocations.values_mut() {
        values.sort_by_key(|relocation| (relocation.offset, &relocation.symbol));
        if values.len() > u16::MAX as usize {
            return Err(error(
                "E_COFF_RELOCATION",
                "section relocation count exceeds u16",
            ));
        }
    }

    let mut strings = Vec::from(4u32.to_le_bytes());
    let mut string_offsets = BTreeMap::<String, u32>::new();
    intern_string(".dbyte.target", &mut strings, &mut string_offsets)?;
    for symbol in &symbols {
        if symbol.name.len() > 8 {
            intern_string(&symbol.name, &mut strings, &mut string_offsets)?;
        }
    }
    let string_table_size = checked_u32(strings.len(), "COFF string table")?;
    strings[..4].copy_from_slice(&string_table_size.to_le_bytes());

    let headers_end = FILE_HEADER_SIZE + SECTION_COUNT * SECTION_HEADER_SIZE;
    let mut cursor = headers_end;
    let mut raw_offsets = BTreeMap::new();
    let mut relocation_offsets = BTreeMap::new();
    for index in 1..=SECTION_COUNT {
        if let Some(bytes) = section_bytes.get(&index) {
            raw_offsets.insert(index, checked_u32(cursor, "section offset")?);
            cursor = cursor
                .checked_add(bytes.len())
                .ok_or_else(|| error("E_COFF_RANGE", "object size overflows"))?;
        }
        if let Some(values) = relocations.get(&index) {
            relocation_offsets.insert(index, checked_u32(cursor, "relocation offset")?);
            cursor = cursor
                .checked_add(values.len() * RELOCATION_SIZE)
                .ok_or_else(|| error("E_COFF_RANGE", "object size overflows"))?;
        }
    }
    let symbol_offset = checked_u32(cursor, "symbol table offset")?;
    cursor = cursor
        .checked_add(symbols.len() * SYMBOL_SIZE)
        .and_then(|value| value.checked_add(strings.len()))
        .ok_or_else(|| error("E_COFF_RANGE", "object size overflows"))?;
    if cursor > MAX_OBJECT_SIZE {
        return Err(error("E_COFF_RANGE", "COFF object exceeds 64 MiB"));
    }

    let mut output = vec![0u8; headers_end];
    put_u16(&mut output, 0, 0x8664)?;
    put_u16(&mut output, 2, SECTION_COUNT as u16)?;
    put_u32(&mut output, 8, symbol_offset)?;
    put_u32(&mut output, 12, symbols.len() as u32)?;
    for index in 1..=SECTION_COUNT {
        let header = FILE_HEADER_SIZE + (index - 1) * SECTION_HEADER_SIZE;
        write_section_name(
            &mut output[header..header + 8],
            section_name(index),
            &string_offsets,
        )?;
        put_u32(
            &mut output,
            header + 16,
            *section_sizes.get(&index).unwrap_or(&0),
        )?;
        put_u32(
            &mut output,
            header + 20,
            *raw_offsets.get(&index).unwrap_or(&0),
        )?;
        put_u32(
            &mut output,
            header + 24,
            *relocation_offsets.get(&index).unwrap_or(&0),
        )?;
        put_u16(
            &mut output,
            header + 32,
            relocations
                .get(&index)
                .map_or(0, |values| values.len() as u16),
        )?;
        put_u32(&mut output, header + 36, section_characteristics(index))?;
    }
    for index in 1..=SECTION_COUNT {
        if let Some(bytes) = section_bytes.get(&index) {
            output.extend_from_slice(bytes);
        }
        if let Some(values) = relocations.get(&index) {
            for relocation in values {
                output.extend_from_slice(
                    &u32::try_from(relocation.offset)
                        .map_err(|_| error("E_COFF_RELOCATION", "offset exceeds u32"))?
                        .to_le_bytes(),
                );
                output.extend_from_slice(&symbol_indices[&relocation.symbol].to_le_bytes());
                output.extend_from_slice(&coff_relocation_kind(relocation.kind)?.to_le_bytes());
            }
        }
    }
    for symbol in &symbols {
        write_symbol_name(&mut output, &symbol.name, &string_offsets)?;
        let (section_number, value) = match symbol.definition {
            Definition::Undefined => (0i16, 0u32),
            Definition::Defined { section, value } => (
                section_index(section) as i16,
                u32::try_from(value)
                    .map_err(|_| error("E_COFF_SYMBOL", "symbol value exceeds u32"))?,
            ),
        };
        output.extend_from_slice(&value.to_le_bytes());
        output.extend_from_slice(&section_number.to_le_bytes());
        output.extend_from_slice(
            &(if symbol.kind == SymbolKind::Function {
                0x20u16
            } else {
                0
            })
            .to_le_bytes(),
        );
        output.push(if symbol.binding == Binding::Global {
            2
        } else {
            3
        });
        output.push(0);
    }
    output.extend_from_slice(&strings);
    Ok(ObjectArtifact {
        bytes: output,
        symbol_count: symbols.len(),
        relocation_count: image.relocations.len(),
    })
}

pub fn parse_object(bytes: &[u8]) -> Result<ObjectImage, CoffError> {
    if bytes.len() < FILE_HEADER_SIZE + SECTION_COUNT * SECTION_HEADER_SIZE
        || bytes.len() > MAX_OBJECT_SIZE
        || u16_at(bytes, 0)? != 0x8664
        || u16_at(bytes, 2)? as usize != SECTION_COUNT
        || u16_at(bytes, 16)? != 0
    {
        return Err(error(
            "E_COFF_HEADER",
            "invalid canonical AMD64 COFF header",
        ));
    }
    let symbol_offset = u32_at(bytes, 8)? as usize;
    let symbol_count = u32_at(bytes, 12)? as usize;
    let symbols_end = symbol_offset
        .checked_add(symbol_count * SYMBOL_SIZE)
        .ok_or_else(|| error("E_COFF_RANGE", "symbol table overflows"))?;
    let string_size = u32_at(bytes, symbols_end)? as usize;
    if string_size < 4 {
        return Err(error("E_COFF_STRING", "invalid COFF string table size"));
    }
    let strings = bytes
        .get(symbols_end..symbols_end + string_size)
        .ok_or_else(|| error("E_COFF_STRING", "truncated COFF string table"))?;
    let mut sections = BTreeMap::new();
    let mut section_headers = Vec::new();
    let mut target_metadata = None;
    for index in 1..=SECTION_COUNT {
        let header = FILE_HEADER_SIZE + (index - 1) * SECTION_HEADER_SIZE;
        let name = read_section_name(&bytes[header..header + 8], strings)?;
        if name != section_name(index) {
            return Err(error(
                "E_COFF_SECTION",
                "noncanonical COFF section order/name",
            ));
        }
        let size = u32_at(bytes, header + 16)? as usize;
        let raw_offset = u32_at(bytes, header + 20)? as usize;
        let reloc_offset = u32_at(bytes, header + 24)? as usize;
        let reloc_count = u16_at(bytes, header + 32)? as usize;
        let raw = if raw_offset == 0 {
            &[][..]
        } else {
            bytes
                .get(raw_offset..raw_offset + size)
                .ok_or_else(|| error("E_COFF_SECTION", "truncated COFF section"))?
        };
        if index == 5 {
            target_metadata = Some(raw.to_vec());
        } else {
            let section = index_section(index).unwrap();
            if size != 0 {
                sections.insert(
                    section,
                    SectionData {
                        contents: if index == 4 && raw_offset == 0 {
                            Contents::Uninitialized(size as u64)
                        } else {
                            Contents::Bytes(raw.to_vec())
                        },
                        alignment: 16,
                    },
                );
            }
        }
        section_headers.push((reloc_offset, reloc_count));
    }
    let target = decode_target_metadata(
        target_metadata
            .as_deref()
            .ok_or_else(|| error("E_COFF_TARGET", "missing .dbyte.target"))?,
    )
    .map_err(|failure| error("E_COFF_TARGET", &failure.to_string()))?;
    if target != X86_64_WINDOWS {
        return Err(error(
            "E_COFF_TARGET",
            "COFF target metadata is not Windows AMD64",
        ));
    }
    let mut indexed_symbols = Vec::with_capacity(symbol_count);
    let mut symbols = Vec::with_capacity(symbol_count);
    for index in 0..symbol_count {
        let entry = symbol_offset + index * SYMBOL_SIZE;
        let name = read_symbol_name(&bytes[entry..entry + 8], strings)?;
        let value = u32_at(bytes, entry + 8)? as u64;
        let section_number = i16::from_le_bytes(bytes[entry + 12..entry + 14].try_into().unwrap());
        let kind = if u16_at(bytes, entry + 14)? == 0x20 {
            SymbolKind::Function
        } else {
            SymbolKind::Object
        };
        let binding = match bytes[entry + 16] {
            2 => Binding::Global,
            3 => Binding::Local,
            _ => return Err(error("E_COFF_SYMBOL", "unsupported COFF storage class")),
        };
        if bytes[entry + 17] != 0 {
            return Err(error(
                "E_COFF_SYMBOL",
                "COFF auxiliary symbols are unsupported",
            ));
        }
        let definition = if section_number == 0 {
            Definition::Undefined
        } else {
            Definition::Defined {
                section: index_section(section_number as usize)
                    .ok_or_else(|| error("E_COFF_SYMBOL", "symbol section is unsupported"))?,
                value,
            }
        };
        indexed_symbols.push(name.clone());
        symbols.push(Symbol {
            name,
            binding,
            kind,
            definition,
            size: 0,
        });
    }
    let mut relocations = Vec::new();
    for (index, (offset, count)) in section_headers.into_iter().enumerate().take(4) {
        if count == 0 {
            continue;
        }
        let section = index_section(index + 1).unwrap();
        let section_data = sections
            .get(&section)
            .ok_or_else(|| error("E_COFF_RELOCATION", "relocation target section is absent"))?;
        let Contents::Bytes(raw) = &section_data.contents else {
            return Err(error("E_COFF_RELOCATION", "relocation targets BSS"));
        };
        for relocation_index in 0..count {
            let entry = offset + relocation_index * RELOCATION_SIZE;
            let patch = u32_at(bytes, entry)? as u64;
            let symbol_index = u32_at(bytes, entry + 4)? as usize;
            let kind = parse_relocation_kind(u16_at(bytes, entry + 8)?)?;
            let width = relocation_width(kind);
            let start = patch as usize;
            let addend_bytes = raw
                .get(start..start + width)
                .ok_or_else(|| error("E_COFF_RELOCATION", "relocation exceeds section"))?;
            let addend = if width == 8 {
                i64::from_le_bytes(addend_bytes.try_into().unwrap())
            } else {
                i64::from(i32::from_le_bytes(addend_bytes.try_into().unwrap()))
            };
            relocations.push(Relocation {
                section,
                offset: patch,
                symbol: indexed_symbols
                    .get(symbol_index)
                    .cloned()
                    .ok_or_else(|| error("E_COFF_RELOCATION", "symbol index is invalid"))?,
                kind,
                addend,
            });
        }
    }
    Ok(ObjectImage {
        target,
        entry_abi: None,
        sections,
        symbols,
        relocations,
    })
}

fn section_index(section: Section) -> usize {
    match section {
        Section::Text => 1,
        Section::Rodata => 2,
        Section::Data => 3,
        Section::Bss => 4,
    }
}

fn index_section(index: usize) -> Option<Section> {
    match index {
        1 => Some(Section::Text),
        2 => Some(Section::Rodata),
        3 => Some(Section::Data),
        4 => Some(Section::Bss),
        _ => None,
    }
}

fn section_name(index: usize) -> &'static str {
    ["", ".text", ".rdata", ".data", ".bss", ".dbyte.target"][index]
}

fn section_characteristics(index: usize) -> u32 {
    match index {
        1 => 0x6050_0020,
        2 => 0x4050_0040,
        3 => 0xc050_0040,
        4 => 0xc050_0080,
        5 => 0x0010_0200,
        _ => unreachable!(),
    }
}

fn relocation_width(kind: RelocationKind) -> usize {
    if kind == RelocationKind::Absolute64 {
        8
    } else {
        4
    }
}

fn coff_relocation_kind(kind: RelocationKind) -> Result<u16, CoffError> {
    match kind {
        RelocationKind::Absolute64 => Ok(1),
        RelocationKind::Absolute32 | RelocationKind::Absolute32Signed => Ok(2),
        RelocationKind::Pc32 | RelocationKind::Plt32 => Ok(4),
    }
}

fn parse_relocation_kind(kind: u16) -> Result<RelocationKind, CoffError> {
    match kind {
        1 => Ok(RelocationKind::Absolute64),
        2 => Ok(RelocationKind::Absolute32),
        4 => Ok(RelocationKind::Pc32),
        _ => Err(error(
            "E_COFF_RELOCATION",
            "unsupported AMD64 relocation type",
        )),
    }
}

fn intern_string(
    value: &str,
    strings: &mut Vec<u8>,
    offsets: &mut BTreeMap<String, u32>,
) -> Result<(), CoffError> {
    if offsets.contains_key(value) {
        return Ok(());
    }
    let offset = checked_u32(strings.len(), "string offset")?;
    offsets.insert(value.into(), offset);
    strings.extend_from_slice(value.as_bytes());
    strings.push(0);
    Ok(())
}

fn write_section_name(
    output: &mut [u8],
    name: &str,
    offsets: &BTreeMap<String, u32>,
) -> Result<(), CoffError> {
    if name.len() <= 8 {
        output[..name.len()].copy_from_slice(name.as_bytes());
    } else {
        let encoded = format!("/{}", offsets[name]);
        if encoded.len() > 8 {
            return Err(error(
                "E_COFF_STRING",
                "long section-name offset exceeds header",
            ));
        }
        output[..encoded.len()].copy_from_slice(encoded.as_bytes());
    }
    Ok(())
}

fn read_section_name(raw: &[u8], strings: &[u8]) -> Result<String, CoffError> {
    if raw.first() == Some(&b'/') {
        let text =
            std::str::from_utf8(raw).map_err(|_| error("E_COFF_STRING", "bad section name"))?;
        let offset = text.trim_end_matches('\0')[1..]
            .parse::<usize>()
            .map_err(|_| error("E_COFF_STRING", "bad long section-name offset"))?;
        return string_at(strings, offset);
    }
    let length = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    std::str::from_utf8(&raw[..length])
        .map(str::to_owned)
        .map_err(|_| error("E_COFF_STRING", "section name is not UTF-8"))
}

fn write_symbol_name(
    output: &mut Vec<u8>,
    name: &str,
    offsets: &BTreeMap<String, u32>,
) -> Result<(), CoffError> {
    if name.len() <= 8 {
        let mut raw = [0u8; 8];
        raw[..name.len()].copy_from_slice(name.as_bytes());
        output.extend_from_slice(&raw);
    } else {
        output.extend_from_slice(&0u32.to_le_bytes());
        output.extend_from_slice(&offsets[name].to_le_bytes());
    }
    Ok(())
}

fn read_symbol_name(raw: &[u8], strings: &[u8]) -> Result<String, CoffError> {
    if raw[..4] == [0, 0, 0, 0] {
        return string_at(
            strings,
            u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize,
        );
    }
    let length = raw.iter().position(|byte| *byte == 0).unwrap_or(8);
    std::str::from_utf8(&raw[..length])
        .map(str::to_owned)
        .map_err(|_| error("E_COFF_STRING", "symbol name is not UTF-8"))
}

fn string_at(strings: &[u8], offset: usize) -> Result<String, CoffError> {
    let tail = strings
        .get(offset..)
        .ok_or_else(|| error("E_COFF_STRING", "string offset is invalid"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| error("E_COFF_STRING", "string is unterminated"))?;
    std::str::from_utf8(&tail[..length])
        .map(str::to_owned)
        .map_err(|_| error("E_COFF_STRING", "string is not UTF-8"))
}

fn validate_alignment(alignment: u64) -> Result<(), CoffError> {
    if alignment == 0 || !alignment.is_power_of_two() || alignment > 4096 {
        return Err(error(
            "E_COFF_ALIGNMENT",
            "section alignment is unsupported",
        ));
    }
    Ok(())
}

fn checked_u32(value: usize, context: &str) -> Result<u32, CoffError> {
    u32::try_from(value).map_err(|_| error("E_COFF_RANGE", &format!("{context} exceeds u32")))
}

fn put_u16(bytes: &mut [u8], offset: usize, value: u16) -> Result<(), CoffError> {
    bytes
        .get_mut(offset..offset + 2)
        .ok_or_else(|| error("E_COFF_RANGE", "u16 write exceeds header"))?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), CoffError> {
    bytes
        .get_mut(offset..offset + 4)
        .ok_or_else(|| error("E_COFF_RANGE", "u32 write exceeds header"))?
        .copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, CoffError> {
    let raw = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| error("E_COFF_RANGE", "truncated u16"))?;
    Ok(u16::from_le_bytes(raw.try_into().unwrap()))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, CoffError> {
    let raw = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| error("E_COFF_RANGE", "truncated u32"))?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}

fn error(code: &str, message: &str) -> CoffError {
    CoffError(format!("{code}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_coff_round_trip_preserves_target_symbols_and_relocations() {
        let image = ObjectImage {
            target: X86_64_WINDOWS,
            entry_abi: None,
            sections: BTreeMap::from([(
                Section::Text,
                SectionData {
                    contents: Contents::Bytes(vec![0xe8, 0, 0, 0, 0, 0xc3]),
                    alignment: 16,
                },
            )]),
            symbols: vec![
                Symbol {
                    name: "windows_main".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Defined {
                        section: Section::Text,
                        value: 0,
                    },
                    size: 6,
                },
                Symbol {
                    name: "external_callback".into(),
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Undefined,
                    size: 0,
                },
            ],
            relocations: vec![Relocation {
                section: Section::Text,
                offset: 1,
                symbol: "external_callback".into(),
                kind: RelocationKind::Pc32,
                addend: -4,
            }],
        };
        let first = write_object(&image).unwrap();
        let second = write_object(&image).unwrap();
        assert_eq!(first.bytes, second.bytes);
        let parsed = parse_object(&first.bytes).unwrap();
        assert_eq!(parsed.target, X86_64_WINDOWS);
        assert_eq!(parsed.symbols.len(), 2);
        assert_eq!(parsed.relocations, image.relocations);
    }
}
