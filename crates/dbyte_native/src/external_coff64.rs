//! Fail-closed importer for the documented Clang AMD64 COFF fixture subset.

use crate::object64::{
    Binding, Contents, Definition, ObjectImage, Relocation, RelocationKind, Section, SectionData,
    Symbol, SymbolKind,
};
use crate::target::X86_64_WINDOWS;
use std::collections::BTreeMap;

const HEADER_SIZE: usize = 20;
const SECTION_SIZE: usize = 40;
const SYMBOL_SIZE: usize = 18;
const RELOCATION_SIZE: usize = 10;
const MAX_SIZE: usize = 64 * 1024 * 1024;
const MAX_SECTIONS: usize = 512;
const MAX_SYMBOLS: usize = 65_535;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalCoff64Error(pub String);

impl std::fmt::Display for ExternalCoff64Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ExternalCoff64Error {}

#[derive(Clone)]
struct RawSection {
    mapped: Option<Section>,
    merged_offset: u64,
    raw_offset: usize,
    raw_size: usize,
    relocation_offset: usize,
    relocation_count: usize,
}

pub fn parse_external_amd64_object(bytes: &[u8]) -> Result<ObjectImage, ExternalCoff64Error> {
    if bytes.len() < HEADER_SIZE || bytes.len() > MAX_SIZE || u16_at(bytes, 0)? != 0x8664 {
        return Err(error("external object is not bounded AMD64 COFF"));
    }
    let section_count = u16_at(bytes, 2)? as usize;
    let symbol_offset = u32_at(bytes, 8)? as usize;
    let symbol_count = u32_at(bytes, 12)? as usize;
    if section_count == 0 || section_count > MAX_SECTIONS || symbol_count > MAX_SYMBOLS {
        return Err(error("external COFF section or symbol count exceeds bound"));
    }
    // The compiler-owned output is deterministic even though MSVC-flavoured
    // Clang records a wall-clock COFF timestamp in its fixture object.
    if u16_at(bytes, 16)? != 0 {
        return Err(error("external COFF optional headers are unsupported"));
    }
    let section_end = HEADER_SIZE
        .checked_add(
            section_count
                .checked_mul(SECTION_SIZE)
                .ok_or_else(|| error("section table overflows"))?,
        )
        .ok_or_else(|| error("section table overflows"))?;
    let symbols_end = symbol_offset
        .checked_add(
            symbol_count
                .checked_mul(SYMBOL_SIZE)
                .ok_or_else(|| error("symbol table overflows"))?,
        )
        .ok_or_else(|| error("symbol table overflows"))?;
    if section_end > bytes.len()
        || symbols_end
            .checked_add(4)
            .is_none_or(|end| end > bytes.len())
    {
        return Err(error("external COFF tables are truncated"));
    }
    let string_size = u32_at(bytes, symbols_end)? as usize;
    if string_size < 4
        || symbols_end
            .checked_add(string_size)
            .is_none_or(|end| end > bytes.len())
    {
        return Err(error("external COFF string table is malformed"));
    }
    let strings = &bytes[symbols_end..symbols_end + string_size];

    let mut image = ObjectImage {
        target: X86_64_WINDOWS,
        entry_abi: None,
        sections: BTreeMap::new(),
        symbols: Vec::new(),
        relocations: Vec::new(),
    };
    let mut sections = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let offset = HEADER_SIZE + index * SECTION_SIZE;
        let name = name_at(&bytes[offset..offset + 8], strings)?;
        if name.starts_with(".dbyte.") {
            return Err(error("DByte metadata must pass the canonical COFF parser"));
        }
        let raw_size = u32_at(bytes, offset + 16)? as usize;
        let raw_offset = u32_at(bytes, offset + 20)? as usize;
        let relocation_offset = u32_at(bytes, offset + 24)? as usize;
        let relocation_count = u16_at(bytes, offset + 32)? as usize;
        let characteristics = u32_at(bytes, offset + 36)?;
        let mapped = match name.as_str() {
            ".text" => Some(Section::Text),
            ".rdata" => Some(Section::Rodata),
            ".data" => Some(Section::Data),
            ".bss" => Some(Section::Bss),
            _ => None,
        };
        if mapped.is_none()
            && characteristics & 0xe000_0000 != 0
            && characteristics & 0x0200_0800 == 0
        {
            return Err(error(format!("unsupported external COFF section `{name}`")));
        }
        if raw_size != 0
            && raw_offset
                .checked_add(raw_size)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(error("external COFF section data is truncated"));
        }
        if relocation_count != 0
            && relocation_offset
                .checked_add(relocation_count * RELOCATION_SIZE)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(error("external COFF relocations are truncated"));
        }
        let merged_offset = if let Some(section) = mapped {
            let alignment = section_alignment(characteristics)?;
            merge_mapped_section(
                &mut image,
                section,
                alignment,
                &bytes[raw_offset..raw_offset + raw_size],
                raw_size as u64,
            )?
        } else {
            0
        };
        sections.push(RawSection {
            mapped,
            merged_offset,
            raw_offset,
            raw_size,
            relocation_offset,
            relocation_count,
        });
    }

    let mut symbol_map = vec![None; symbol_count];
    let mut index = 0usize;
    while index < symbol_count {
        let offset = symbol_offset + index * SYMBOL_SIZE;
        let name = name_at(&bytes[offset..offset + 8], strings)?;
        let value = u32_at(bytes, offset + 8)? as u64;
        let section_number =
            i16::from_le_bytes(bytes[offset + 12..offset + 14].try_into().unwrap());
        let storage = bytes[offset + 16];
        let auxiliary = bytes[offset + 17] as usize;
        if index
            .checked_add(auxiliary)
            .is_none_or(|last| last >= symbol_count)
        {
            return Err(error("external COFF auxiliary symbols exceed table"));
        }
        if storage == 2 || storage == 3 {
            if section_number > 0 {
                if let Some(raw) = sections.get(section_number as usize - 1) {
                    let Some(section) = raw.mapped else {
                        index += auxiliary + 1;
                        continue;
                    };
                    let value = raw.merged_offset.checked_add(value).ok_or_else(|| {
                        error("external COFF symbol value overflows merged section")
                    })?;
                    let canonical_name = if name.starts_with('.') && storage == 3 {
                        format!("__coff_section_{index}")
                    } else {
                        name.clone()
                    };
                    symbol_map[index] = Some(canonical_name.clone());
                    image.symbols.push(Symbol {
                        name: canonical_name,
                        binding: if storage == 2 {
                            Binding::Global
                        } else {
                            Binding::Local
                        },
                        kind: if u16_at(bytes, offset + 14)? == 0x20 {
                            SymbolKind::Function
                        } else {
                            SymbolKind::Object
                        },
                        definition: Definition::Defined { section, value },
                        size: 0,
                    });
                }
            } else if section_number == 0 && storage == 2 {
                symbol_map[index] = Some(name.clone());
                image.symbols.push(Symbol {
                    name,
                    binding: Binding::Global,
                    kind: SymbolKind::Function,
                    definition: Definition::Undefined,
                    size: 0,
                });
            }
        }
        index += auxiliary + 1;
    }

    for raw in &sections {
        let Some(section) = raw.mapped else { continue };
        let section_bytes = if section == Section::Bss {
            &[][..]
        } else {
            &bytes[raw.raw_offset..raw.raw_offset + raw.raw_size]
        };
        for relocation_index in 0..raw.relocation_count {
            let offset = raw.relocation_offset + relocation_index * RELOCATION_SIZE;
            let patch = u32_at(bytes, offset)? as usize;
            let symbol_index = u32_at(bytes, offset + 4)? as usize;
            let relocation_type = u16_at(bytes, offset + 8)?;
            let (kind, width, bias) = match relocation_type {
                1 => (RelocationKind::Absolute64, 8, 0),
                2 => (RelocationKind::Absolute32, 4, 0),
                4..=9 => (RelocationKind::Pc32, 4, -4 - i64::from(relocation_type - 4)),
                other => return Err(error(format!("unsupported AMD64 COFF relocation {other}"))),
            };
            let bytes_at_patch = section_bytes
                .get(patch..patch + width)
                .ok_or_else(|| error("external COFF relocation exceeds section"))?;
            let stored = if width == 8 {
                i64::from_le_bytes(bytes_at_patch.try_into().unwrap())
            } else {
                i64::from(i32::from_le_bytes(bytes_at_patch.try_into().unwrap()))
            };
            image.relocations.push(Relocation {
                section,
                offset: raw.merged_offset.checked_add(patch as u64).ok_or_else(|| {
                    error("external COFF relocation offset overflows merged section")
                })?,
                symbol: symbol_map
                    .get(symbol_index)
                    .and_then(Clone::clone)
                    .ok_or_else(|| {
                        error("external COFF relocation references unsupported symbol")
                    })?,
                kind,
                addend: stored
                    .checked_add(bias)
                    .ok_or_else(|| error("external COFF relocation addend overflows"))?,
            });
        }
    }
    Ok(image)
}

fn merge_mapped_section(
    image: &mut ObjectImage,
    section: Section,
    alignment: u64,
    raw: &[u8],
    logical_size: u64,
) -> Result<u64, ExternalCoff64Error> {
    match image.sections.get_mut(&section) {
        None => {
            if logical_size == 0 {
                return Ok(0);
            }
            image.sections.insert(
                section,
                SectionData {
                    contents: if section == Section::Bss {
                        Contents::Uninitialized(logical_size)
                    } else {
                        Contents::Bytes(raw.to_vec())
                    },
                    alignment,
                },
            );
            Ok(0)
        }
        Some(existing) => {
            existing.alignment = existing.alignment.max(alignment);
            match &mut existing.contents {
                Contents::Bytes(output) if section != Section::Bss => {
                    let base = align_up(output.len() as u64, alignment)?;
                    let base_usize = usize::try_from(base)
                        .map_err(|_| error("external COFF merged section exceeds host bounds"))?;
                    output.resize(base_usize, 0);
                    output.extend_from_slice(raw);
                    Ok(base)
                }
                Contents::Uninitialized(size) if section == Section::Bss => {
                    let base = align_up(*size, alignment)?;
                    *size = base
                        .checked_add(logical_size)
                        .ok_or_else(|| error("external COFF merged BSS size overflows"))?;
                    Ok(base)
                }
                _ => Err(error("external COFF mapped section changes storage kind")),
            }
        }
    }
}

fn align_up(value: u64, alignment: u64) -> Result<u64, ExternalCoff64Error> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| error("external COFF section merge alignment overflows"))
}
fn section_alignment(characteristics: u32) -> Result<u64, ExternalCoff64Error> {
    let encoded = (characteristics >> 20) & 0xf;
    if !(1..=13).contains(&encoded) {
        return Err(error("external COFF section alignment is unsupported"));
    }
    Ok(1u64 << (encoded - 1))
}

fn name_at(raw: &[u8], strings: &[u8]) -> Result<String, ExternalCoff64Error> {
    let bytes = if raw.get(..4) == Some(&[0, 0, 0, 0]) {
        let offset = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        strings
            .get(offset..)
            .ok_or_else(|| error("external COFF string offset is invalid"))?
    } else if raw.first() == Some(&b'/') {
        let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
        let offset = std::str::from_utf8(&raw[1..end])
            .map_err(|_| error("external COFF long name is invalid"))?
            .parse::<usize>()
            .map_err(|_| error("external COFF long name offset is invalid"))?;
        strings
            .get(offset..)
            .ok_or_else(|| error("external COFF long name offset is invalid"))?
    } else {
        raw
    };
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| error("external COFF name is not UTF-8"))
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, ExternalCoff64Error> {
    bytes
        .get(offset..offset + 2)
        .map(|value| u16::from_le_bytes(value.try_into().unwrap()))
        .ok_or_else(|| error("truncated external COFF u16"))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, ExternalCoff64Error> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_le_bytes(value.try_into().unwrap()))
        .ok_or_else(|| error("truncated external COFF u32"))
}

fn error(message: impl Into<String>) -> ExternalCoff64Error {
    ExternalCoff64Error(format!("E_EXTERNAL_COFF64: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(timestamp: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER_SIZE + SECTION_SIZE];
        bytes[0..2].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&1u16.to_le_bytes());
        bytes[4..8].copy_from_slice(&timestamp.to_le_bytes());
        bytes[8..12].copy_from_slice(&61u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&1u32.to_le_bytes());
        bytes[HEADER_SIZE..HEADER_SIZE + 8].copy_from_slice(b".text\0\0\0");
        bytes[HEADER_SIZE + 16..HEADER_SIZE + 20].copy_from_slice(&1u32.to_le_bytes());
        bytes[HEADER_SIZE + 20..HEADER_SIZE + 24].copy_from_slice(&60u32.to_le_bytes());
        bytes[HEADER_SIZE + 36..HEADER_SIZE + 40].copy_from_slice(&0x6050_0020u32.to_le_bytes());
        bytes.push(0xc3);
        let mut symbol = [0u8; SYMBOL_SIZE];
        symbol[..7].copy_from_slice(b"c_entry");
        symbol[12..14].copy_from_slice(&1i16.to_le_bytes());
        symbol[14..16].copy_from_slice(&0x20u16.to_le_bytes());
        symbol[16] = 2;
        bytes.extend_from_slice(&symbol);
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes
    }

    #[test]
    fn external_timestamp_does_not_affect_imported_semantics() {
        let first = parse_external_amd64_object(&fixture(1)).unwrap();
        let second = parse_external_amd64_object(&fixture(u32::MAX)).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            first.sections[&Section::Text].contents,
            Contents::Bytes(vec![0xc3])
        );
        assert_eq!(first.symbols[0].name, "c_entry");
    }

    #[test]
    fn canonical_metadata_cannot_bypass_the_canonical_parser() {
        let canonical = crate::coff64::write_object(&ObjectImage {
            target: X86_64_WINDOWS,
            entry_abi: None,
            sections: BTreeMap::new(),
            symbols: Vec::new(),
            relocations: Vec::new(),
        })
        .unwrap();
        assert!(parse_external_amd64_object(&canonical.bytes)
            .unwrap_err()
            .0
            .contains("canonical COFF parser"));
    }
}
