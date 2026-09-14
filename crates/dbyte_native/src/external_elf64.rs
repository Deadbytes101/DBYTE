//! Fail-closed importer for the documented x86-64 machine-boundary object subset.
//! Linux imports may carry validated informational GNU x86 USED property notes.

use crate::object64::{
    Binding, Contents, Definition, ObjectImage, Relocation, RelocationKind, Section, SectionData,
    Symbol, SymbolKind,
};
use crate::target::TargetSpec;
use std::collections::BTreeMap;

const ELF_HEADER_SIZE: usize = 64;
const SECTION_HEADER_SIZE: usize = 64;
const SYMBOL_SIZE: usize = 24;
const RELA_SIZE: usize = 24;
const MAX_SIZE: usize = 64 * 1024 * 1024;
const MAX_SECTIONS: usize = 512;
const MAX_SYMBOLS: usize = 65_535;
const MAX_RELOCATIONS: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalElf64Error(pub String);

impl std::fmt::Display for ExternalElf64Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ExternalElf64Error {}

#[derive(Clone)]
struct RawSection {
    name: String,
    kind: u32,
    flags: u64,
    offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entsize: u64,
}

pub fn parse_external_x86_64_object(
    bytes: &[u8],
    target: TargetSpec,
) -> Result<ObjectImage, ExternalElf64Error> {
    if bytes.len() < ELF_HEADER_SIZE || bytes.len() > MAX_SIZE {
        return Err(error(
            "external ELF64 object size is outside the supported bound",
        ));
    }
    if bytes.get(0..7) != Some(b"\x7fELF\x02\x01\x01")
        || u16_at(bytes, 16)? != 1
        || u16_at(bytes, 18)? != 62
        || u32_at(bytes, 20)? != 1
        || u16_at(bytes, 52)? as usize != ELF_HEADER_SIZE
        || u16_at(bytes, 58)? as usize != SECTION_HEADER_SIZE
    {
        return Err(error(
            "external object is not canonical ELF64/x86-64 ET_REL",
        ));
    }
    if u64_at(bytes, 32)? != 0 || u16_at(bytes, 54)? != 0 || u16_at(bytes, 56)? != 0 {
        return Err(error(
            "external relocatable object must not contain program headers",
        ));
    }
    let shoff = usize::try_from(u64_at(bytes, 40)?)
        .map_err(|_| error("external section table offset exceeds host bounds"))?;
    let shnum = u16_at(bytes, 60)? as usize;
    let shstrndx = u16_at(bytes, 62)? as usize;
    if shnum == 0 || shnum > MAX_SECTIONS || shstrndx >= shnum {
        return Err(error(
            "external ELF64 section count or shstr index is invalid",
        ));
    }
    let table_size = shnum
        .checked_mul(SECTION_HEADER_SIZE)
        .ok_or_else(|| error("external section table overflows"))?;
    if shoff
        .checked_add(table_size)
        .filter(|end| *end <= bytes.len())
        .is_none()
    {
        return Err(error("external ELF64 section table is truncated"));
    }
    let raw_headers = (0..shnum)
        .map(|index| read_section(bytes, shoff + index * SECTION_HEADER_SIZE))
        .collect::<Result<Vec<_>, _>>()?;
    let shstr_header = &raw_headers[shstrndx];
    if shstr_header.1 != 3 {
        return Err(error("external ELF64 shstr index is not a string table"));
    }
    let shstr = raw_bytes(bytes, shstr_header.4, shstr_header.5)?;
    let sections = raw_headers
        .into_iter()
        .map(|raw| {
            let name = string_at(shstr, raw.0)?;
            if name.starts_with(".dbyte.") {
                return Err(error(
                    "DByte metadata object must pass the canonical parser",
                ));
            }
            Ok(RawSection {
                name,
                kind: raw.1,
                flags: raw.2,
                offset: raw.4,
                size: raw.5,
                link: raw.6,
                info: raw.7,
                align: raw.8,
                entsize: raw.9,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut image = ObjectImage {
        target,
        entry_abi: None,
        sections: BTreeMap::new(),
        symbols: Vec::new(),
        relocations: Vec::new(),
    };
    let mut section_map = BTreeMap::<usize, (Section, u64)>::new();
    for (index, section) in sections.iter().enumerate().skip(1) {
        let mapped = match section.name.as_str() {
            ".text" => Some(Section::Text),
            ".rodata" => Some(Section::Rodata),
            name if name.starts_with(".rodata.") => Some(Section::Rodata),
            ".data" => Some(Section::Data),
            ".bss" => Some(Section::Bss),
            _ => None,
        };
        if let Some(mapped) = mapped {
            validate_alignment(section.align)?;
            let base = merge_mapped_section(&mut image, mapped, section, bytes)?;
            section_map.insert(index, (mapped, base));
        } else if section.name == ".note.gnu.property" && target == crate::target::X86_64_LINUX {
            validate_ignorable_gnu_property_note(section, bytes)?;
        } else if section.flags & 0x2 != 0 && section.name != ".eh_frame" {
            return Err(error(format!(
                "unsupported allocatable external section '{}'",
                section.name
            )));
        }
    }

    let symtab_index = sections
        .iter()
        .position(|section| section.kind == 2)
        .ok_or_else(|| error("external ELF64 object has no symbol table"))?;
    let symtab = &sections[symtab_index];
    if symtab.entsize != SYMBOL_SIZE as u64 || symtab.size % SYMBOL_SIZE as u64 != 0 {
        return Err(error("external ELF64 symbol table has invalid entry size"));
    }
    let strtab = sections
        .get(symtab.link as usize)
        .ok_or_else(|| error("external ELF64 symbol string-table index is invalid"))?;
    if strtab.kind != 3 {
        return Err(error("external ELF64 symbol link is not a string table"));
    }
    let strings = section_bytes(bytes, strtab)?;
    let symbols = section_bytes(bytes, symtab)?;
    let count = symbols.len() / SYMBOL_SIZE;
    if count > MAX_SYMBOLS {
        return Err(error("external ELF64 symbol count exceeds bound"));
    }
    let mut symbol_map = vec![None; count];
    for index in 1..count {
        let entry = &symbols[index * SYMBOL_SIZE..(index + 1) * SYMBOL_SIZE];
        let mut name = string_at(strings, u32_at(entry, 0)?)?;
        let info = entry[4];
        let binding = match info >> 4 {
            0 => Binding::Local,
            1 => Binding::Global,
            _ => {
                return Err(error(
                    "weak or processor-specific ELF64 symbols are unsupported",
                ))
            }
        };
        let symbol_type = info & 0xf;
        let kind = match symbol_type {
            0 | 2 => SymbolKind::Function,
            1 => SymbolKind::Object,
            3 if binding == Binding::Local => SymbolKind::Object,
            4 if binding == Binding::Local => continue,
            _ => return Err(error("unsupported external ELF64 symbol type")),
        };
        let section_index = u16_at(entry, 6)? as usize;
        if symbol_type == 3 && binding == Binding::Local {
            name = format!("\0external-section:{section_index}");
        }
        let value = u64_at(entry, 8)?;
        let size = u64_at(entry, 16)?;
        let definition = if section_index == 0 {
            Definition::Undefined
        } else if let Some((section, base)) = section_map.get(&section_index).copied() {
            let value = base
                .checked_add(value)
                .ok_or_else(|| error("external ELF64 symbol value overflows merged section"))?;
            Definition::Defined { section, value }
        } else if binding == Binding::Local {
            continue;
        } else {
            return Err(error(format!(
                "global external symbol '{name}' is defined in an unsupported section"
            )));
        };
        if name.is_empty() && symbol_type != 3 {
            return Err(error("external ELF64 non-section symbol has no name"));
        }
        symbol_map[index] = Some(name.clone());
        image.symbols.push(Symbol {
            name,
            binding,
            kind,
            definition,
            size,
        });
    }

    for section in sections.iter().filter(|section| section.kind == 4) {
        let Some((target_section, target_base)) =
            section_map.get(&(section.info as usize)).copied()
        else {
            if sections
                .get(section.info as usize)
                .is_some_and(|target| target.name == ".eh_frame")
            {
                continue;
            }
            return Err(error(
                "external ELF64 relocation targets an unsupported section",
            ));
        };
        if section.link as usize != symtab_index
            || section.entsize != RELA_SIZE as u64
            || section.size % RELA_SIZE as u64 != 0
        {
            return Err(error("external ELF64 RELA section is malformed"));
        }
        for rela in section_bytes(bytes, section)?.as_chunks::<RELA_SIZE>().0 {
            if image.relocations.len() >= MAX_RELOCATIONS {
                return Err(error("external ELF64 relocation count exceeds bound"));
            }
            let offset = target_base.checked_add(u64_at(rela, 0)?).ok_or_else(|| {
                error("external ELF64 relocation offset overflows merged section")
            })?;
            let info = u64_at(rela, 8)?;
            let symbol_index = usize::try_from(info >> 32)
                .map_err(|_| error("external ELF64 relocation symbol index overflows"))?;
            let symbol = symbol_map
                .get(symbol_index)
                .and_then(|symbol| symbol.clone())
                .ok_or_else(|| error("external ELF64 relocation references unsupported symbol"))?;
            let kind = match info as u32 {
                1 => RelocationKind::Absolute64,
                2 => RelocationKind::Pc32,
                4 => RelocationKind::Plt32,
                10 => RelocationKind::Absolute32,
                11 => RelocationKind::Absolute32Signed,
                other => return Err(error(format!("unsupported x86-64 relocation type {other}"))),
            };
            image.relocations.push(Relocation {
                section: target_section,
                offset,
                symbol,
                kind,
                addend: i64_at(rela, 16)?,
            });
        }
    }
    Ok(image)
}

const NT_GNU_PROPERTY_TYPE_0: u32 = 5;
const GNU_PROPERTY_X86_FEATURE_2_USED: u32 = 0xc001_0001;
const GNU_PROPERTY_X86_ISA_1_USED: u32 = 0xc001_0002;

fn validate_ignorable_gnu_property_note(
    section: &RawSection,
    bytes: &[u8],
) -> Result<(), ExternalElf64Error> {
    if section.kind != 7 || section.flags != 0x2 {
        return Err(error(
            "external .note.gnu.property must be an allocatable SHT_NOTE section",
        ));
    }
    validate_alignment(section.align)?;
    let note = section_bytes(bytes, section)?;
    if note.len() < 16 {
        return Err(error("external GNU property note is truncated"));
    }

    let name_size = u32_at(note, 0)? as usize;
    let desc_size = u32_at(note, 4)? as usize;
    let note_type = u32_at(note, 8)?;
    if name_size != 4 || note_type != NT_GNU_PROPERTY_TYPE_0 {
        return Err(error("unsupported external GNU property note header"));
    }
    let name_end = 12usize
        .checked_add(name_size)
        .ok_or_else(|| error("external GNU property name range overflows"))?;
    if note.get(12..name_end) != Some(b"GNU\0") {
        return Err(error("external GNU property note owner is not GNU"));
    }
    let desc_start = align_usize(name_end, 4)?;
    let desc_end = desc_start
        .checked_add(desc_size)
        .ok_or_else(|| error("external GNU property descriptor range overflows"))?;
    let note_end = align_usize(desc_end, 4)?;
    if note_end != note.len() || desc_end > note.len() {
        return Err(error("external GNU property note size is malformed"));
    }
    if note[desc_end..note_end].iter().any(|byte| *byte != 0) {
        return Err(error("external GNU property note padding is nonzero"));
    }

    let descriptor = &note[desc_start..desc_end];
    let mut cursor = 0usize;
    let mut seen_isa_used = false;
    let mut seen_feature_used = false;
    if descriptor.is_empty() {
        return Err(error("external GNU property descriptor is empty"));
    }
    while cursor < descriptor.len() {
        if descriptor.len() - cursor < 8 {
            return Err(error("external GNU property entry is truncated"));
        }
        let property_type = u32_at(descriptor, cursor)?;
        let data_size = u32_at(descriptor, cursor + 4)? as usize;
        if data_size != 4 {
            return Err(error("external GNU property data size is unsupported"));
        }
        let data_start = cursor + 8;
        let data_end = data_start
            .checked_add(data_size)
            .ok_or_else(|| error("external GNU property data range overflows"))?;
        let next = align_usize(data_end, 8)?;
        if next > descriptor.len() {
            return Err(error("external GNU property entry exceeds descriptor"));
        }
        if descriptor[data_end..next].iter().any(|byte| *byte != 0) {
            return Err(error("external GNU property padding is nonzero"));
        }
        let _value = u32_at(descriptor, data_start)?;
        match property_type {
            GNU_PROPERTY_X86_ISA_1_USED if !seen_isa_used => seen_isa_used = true,
            GNU_PROPERTY_X86_FEATURE_2_USED if !seen_feature_used => seen_feature_used = true,
            GNU_PROPERTY_X86_ISA_1_USED | GNU_PROPERTY_X86_FEATURE_2_USED => {
                return Err(error("duplicate external GNU property type"));
            }
            other => {
                return Err(error(format!(
                    "unsupported GNU property type 0x{other:08x}"
                )));
            }
        }
        cursor = next;
    }
    Ok(())
}

fn align_usize(value: usize, alignment: usize) -> Result<usize, ExternalElf64Error> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| error("external GNU property alignment overflows"))
}

fn merge_mapped_section(
    image: &mut ObjectImage,
    mapped: Section,
    section: &RawSection,
    bytes: &[u8],
) -> Result<u64, ExternalElf64Error> {
    let alignment = section.align.max(1);
    match image.sections.get_mut(&mapped) {
        None => {
            let contents = match section.kind {
                1 => Contents::Bytes(section_bytes(bytes, section)?.to_vec()),
                8 => Contents::Uninitialized(section.size),
                other => {
                    return Err(error(format!(
                        "external section '{}' has unsupported type {other}",
                        section.name
                    )))
                }
            };
            image.sections.insert(
                mapped,
                SectionData {
                    contents,
                    alignment,
                },
            );
            Ok(0)
        }
        Some(existing) => {
            existing.alignment = existing.alignment.max(alignment);
            match (&mut existing.contents, section.kind) {
                (Contents::Bytes(output), 1) => {
                    let base = align_up(output.len() as u64, alignment)?;
                    let base_usize = usize::try_from(base)
                        .map_err(|_| error("external merged section exceeds host bounds"))?;
                    output.resize(base_usize, 0);
                    output.extend_from_slice(section_bytes(bytes, section)?);
                    Ok(base)
                }
                (Contents::Uninitialized(size), 8) => {
                    let base = align_up(*size, alignment)?;
                    *size = base
                        .checked_add(section.size)
                        .ok_or_else(|| error("external merged BSS size overflows"))?;
                    Ok(base)
                }
                _ => Err(error(format!(
                    "external section '{}' changes canonical section storage kind",
                    section.name
                ))),
            }
        }
    }
}

fn align_up(value: u64, alignment: u64) -> Result<u64, ExternalElf64Error> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| error("external section merge alignment overflows"))
}
type RawHeader = (u32, u32, u64, u64, u64, u64, u32, u32, u64, u64);

fn read_section(bytes: &[u8], offset: usize) -> Result<RawHeader, ExternalElf64Error> {
    let section = bytes
        .get(offset..offset + SECTION_HEADER_SIZE)
        .ok_or_else(|| error("truncated external ELF64 section header"))?;
    Ok((
        u32_at(section, 0)?,
        u32_at(section, 4)?,
        u64_at(section, 8)?,
        u64_at(section, 16)?,
        u64_at(section, 24)?,
        u64_at(section, 32)?,
        u32_at(section, 40)?,
        u32_at(section, 44)?,
        u64_at(section, 48)?,
        u64_at(section, 56)?,
    ))
}

fn section_bytes<'a>(
    bytes: &'a [u8],
    section: &RawSection,
) -> Result<&'a [u8], ExternalElf64Error> {
    if section.kind == 8 {
        return Ok(&[]);
    }
    raw_bytes(bytes, section.offset, section.size)
}

fn raw_bytes(bytes: &[u8], offset: u64, size: u64) -> Result<&[u8], ExternalElf64Error> {
    let start = usize::try_from(offset).map_err(|_| error("external ELF64 offset overflows"))?;
    let length = usize::try_from(size).map_err(|_| error("external ELF64 size overflows"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| error("external ELF64 range overflows"))?;
    bytes
        .get(start..end)
        .ok_or_else(|| error("external ELF64 section is truncated"))
}

fn string_at(strings: &[u8], offset: u32) -> Result<String, ExternalElf64Error> {
    let tail = strings
        .get(offset as usize..)
        .ok_or_else(|| error("external ELF64 string offset is invalid"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| error("external ELF64 string is unterminated"))?;
    std::str::from_utf8(&tail[..length])
        .map(str::to_owned)
        .map_err(|_| error("external ELF64 name is not UTF-8"))
}

fn validate_alignment(align: u64) -> Result<(), ExternalElf64Error> {
    if align == 0 || !align.is_power_of_two() || align > 4096 {
        return Err(error("external ELF64 section alignment is unsupported"));
    }
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, ExternalElf64Error> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| error("truncated external ELF64 u16"))?;
    Ok(u16::from_le_bytes(value.try_into().unwrap()))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, ExternalElf64Error> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| error("truncated external ELF64 u32"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, ExternalElf64Error> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| error("truncated external ELF64 u64"))?;
    Ok(u64::from_le_bytes(value.try_into().unwrap()))
}

fn i64_at(bytes: &[u8], offset: usize) -> Result<i64, ExternalElf64Error> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| error("truncated external ELF64 i64"))?;
    Ok(i64::from_le_bytes(value.try_into().unwrap()))
}

fn error(message: impl Into<String>) -> ExternalElf64Error {
    ExternalElf64Error(format!("E_EXTERNAL_ELF64: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gcc16_used_property_note() -> Vec<u8> {
        vec![
            0x04, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, b'G', b'N',
            b'U', 0x00, 0x02, 0x00, 0x01, 0xc0, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0xc0, 0x04, 0x00, 0x00, 0x00, 0x09, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]
    }

    fn property_note_section(size: usize) -> RawSection {
        RawSection {
            name: ".note.gnu.property".to_owned(),
            kind: 7,
            flags: 0x2,
            offset: 0,
            size: size as u64,
            link: 0,
            info: 0,
            align: 8,
            entsize: 0,
        }
    }

    #[test]
    fn gcc16_used_gnu_properties_are_accepted_as_informational() {
        let bytes = gcc16_used_property_note();
        validate_ignorable_gnu_property_note(&property_note_section(bytes.len()), &bytes)
            .expect("GCC 16 used-only GNU properties must be accepted");
    }

    #[test]
    fn required_or_unknown_gnu_properties_remain_fail_closed() {
        let mut bytes = gcc16_used_property_note();
        bytes[16..20].copy_from_slice(&0xc000_8000u32.to_le_bytes());
        let failure =
            validate_ignorable_gnu_property_note(&property_note_section(bytes.len()), &bytes)
                .expect_err("NEEDED GNU property must remain unsupported");
        assert!(failure.0.contains("unsupported GNU property type"));
    }

    #[test]
    fn malformed_gnu_property_note_is_rejected() {
        let mut bytes = gcc16_used_property_note();
        bytes[4..8].copy_from_slice(&31u32.to_le_bytes());
        assert!(
            validate_ignorable_gnu_property_note(&property_note_section(bytes.len()), &bytes,)
                .is_err()
        );
    }

    #[test]
    fn canonical_target_metadata_cannot_enter_through_external_importer() {
        let image = ObjectImage {
            target: crate::target::X86_64_LINUX,
            entry_abi: None,
            sections: BTreeMap::new(),
            symbols: vec![],
            relocations: vec![],
        };
        let bytes = crate::object64::write_relocatable(&image).unwrap().bytes;
        let failure =
            parse_external_x86_64_object(&bytes, crate::target::X86_64_LINUX).unwrap_err();
        assert!(failure.0.contains("canonical parser"));
    }
}
