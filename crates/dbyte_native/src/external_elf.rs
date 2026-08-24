//! Fail-closed importer for the documented i686 C ABI fixture object subset.

use crate::diagnostics::{Diagnostic, DiagnosticCategory};
use crate::object::{
    ObjectImage, ObjectRelocation, ObjectSection, ObjectSectionData, ObjectSymbol, RelocationKind,
    SectionContents, SymbolBinding, SymbolDefinition, SymbolId, SymbolKind, MAX_OBJECT_ALIGNMENT,
    MAX_OBJECT_RELOCATIONS, MAX_OBJECT_SECTION_SIZE, MAX_OBJECT_SIZE, MAX_OBJECT_SYMBOLS,
};
use std::collections::BTreeMap;

const ELF_HEADER_SIZE: usize = 52;
const SECTION_HEADER_SIZE: usize = 40;
const SYMBOL_SIZE: usize = 16;
const REL_SIZE: usize = 8;

fn error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidObject, message)
}

#[derive(Clone)]
struct Section {
    name: String,
    kind: u32,
    flags: u32,
    offset: u32,
    size: u32,
    link: u32,
    info: u32,
    align: u32,
    entsize: u32,
}

pub fn parse_external_i386_object(bytes: &[u8]) -> Result<ObjectImage, Diagnostic> {
    if bytes.len() < ELF_HEADER_SIZE || bytes.len() > MAX_OBJECT_SIZE {
        return Err(error(
            "external ELF object size is outside the supported bound",
        ));
    }
    if &bytes[0..7] != b"\x7fELF\x01\x01\x01"
        || u16_at(bytes, 16)? != 1
        || u16_at(bytes, 18)? != 3
        || u32_at(bytes, 20)? != 1
        || u16_at(bytes, 40)? as usize != ELF_HEADER_SIZE
        || u16_at(bytes, 46)? as usize != SECTION_HEADER_SIZE
    {
        return Err(error("external object is not canonical ELF32/i386 ET_REL"));
    }
    if u32_at(bytes, 28)? != 0 || u16_at(bytes, 42)? != 0 || u16_at(bytes, 44)? != 0 {
        return Err(error(
            "external relocatable object must not contain program headers",
        ));
    }
    let shoff = u32_at(bytes, 32)? as usize;
    let shnum = u16_at(bytes, 48)? as usize;
    let shstrndx = u16_at(bytes, 50)? as usize;
    if shnum == 0 || shnum > 256 || shstrndx >= shnum {
        return Err(error(
            "external ELF section count or shstr index is invalid",
        ));
    }
    let table_end = shoff
        .checked_add(
            shnum
                .checked_mul(SECTION_HEADER_SIZE)
                .ok_or_else(|| error("section table overflow"))?,
        )
        .ok_or_else(|| error("section table overflow"))?;
    if table_end > bytes.len() {
        return Err(error("external ELF section table is truncated"));
    }
    let raw_sections: Vec<_> = (0..shnum)
        .map(|index| read_section(bytes, shoff + index * SECTION_HEADER_SIZE))
        .collect::<Result<_, _>>()?;
    let shstr_raw = &raw_sections[shstrndx];
    if shstr_raw.1 != 3 {
        return Err(error("external shstr index is not a string table"));
    }
    let shstr_start = shstr_raw.4 as usize;
    let shstr_end = shstr_start
        .checked_add(shstr_raw.5 as usize)
        .ok_or_else(|| error("external shstr range overflow"))?;
    let shstr = bytes
        .get(shstr_start..shstr_end)
        .ok_or_else(|| error("external shstr table is truncated"))?;
    let mut sections = Vec::with_capacity(shnum);
    for raw in raw_sections {
        let name = string_at(shstr, raw.0)?;
        if name.starts_with(".dbyte.") {
            return Err(error(
                "DByte metadata object must pass the canonical DByte parser",
            ));
        }
        sections.push(Section {
            name,
            kind: raw.1,
            flags: raw.2,
            offset: raw.4,
            size: raw.5,
            link: raw.6,
            info: raw.7,
            align: raw.8,
            entsize: raw.9,
        });
    }

    let mut image = ObjectImage::empty();
    let mut section_map = BTreeMap::<usize, ObjectSection>::new();
    for (index, section) in sections.iter().enumerate().skip(1) {
        let mapped = match section.name.as_str() {
            ".text" => Some(ObjectSection::Text),
            ".rodata" => Some(ObjectSection::Rodata),
            ".data" => Some(ObjectSection::Data),
            ".bss" => Some(ObjectSection::Bss),
            _ => None,
        };
        if let Some(mapped) = mapped {
            validate_alignment(section.align)?;
            if section.size as usize > MAX_OBJECT_SECTION_SIZE {
                return Err(error("external section exceeds supported bound"));
            }
            let contents = if section.kind == 8 {
                SectionContents::Uninitialized(section.size)
            } else if section.kind == 1 {
                SectionContents::Bytes(section_bytes(bytes, section)?.to_vec())
            } else {
                return Err(error(format!(
                    "external section '{}' has unsupported type {}",
                    section.name, section.kind
                )));
            };
            image.sections.insert(
                mapped,
                ObjectSectionData {
                    contents,
                    alignment: section.align.max(1),
                },
            );
            section_map.insert(index, mapped);
        } else if section.flags & 0x2 != 0 {
            return Err(error(format!(
                "unsupported allocatable external section '{}'",
                section.name
            )));
        }
    }

    let symtab_index = sections
        .iter()
        .position(|section| section.kind == 2)
        .ok_or_else(|| error("external object has no symbol table"))?;
    let symtab = &sections[symtab_index];
    if symtab.entsize as usize != SYMBOL_SIZE || !(symtab.size as usize).is_multiple_of(SYMBOL_SIZE)
    {
        return Err(error("external symbol table has invalid entry size"));
    }
    let strtab = sections
        .get(symtab.link as usize)
        .ok_or_else(|| error("external symbol table string-table index is invalid"))?;
    if strtab.kind != 3 {
        return Err(error("external symbol table link is not a string table"));
    }
    let strings = section_bytes(bytes, strtab)?;
    let symbols = section_bytes(bytes, symtab)?;
    let symbol_count = symbols.len() / SYMBOL_SIZE;
    if symbol_count > MAX_OBJECT_SYMBOLS {
        return Err(error("external symbol count exceeds supported bound"));
    }
    let mut symbol_map = vec![None; symbol_count];
    for index in 1..symbol_count {
        let entry = &symbols[index * SYMBOL_SIZE..(index + 1) * SYMBOL_SIZE];
        let name = string_at(strings, u32_at(entry, 0)?)?;
        let value = u32_at(entry, 4)?;
        let size = u32_at(entry, 8)?;
        let info = entry[12];
        let binding = info >> 4;
        let kind = info & 0xf;
        let section_index = u16_at(entry, 14)? as usize;
        if binding == 0 && kind == 4 {
            continue;
        }
        let binding = match binding {
            0 => SymbolBinding::Local,
            1 => SymbolBinding::Global,
            _ => {
                return Err(error(
                    "weak or processor-specific external symbols are unsupported",
                ))
            }
        };
        let definition = if section_index == 0 {
            SymbolDefinition::Undefined
        } else if let Some(section) = section_map.get(&section_index).copied() {
            SymbolDefinition::Defined { section, value }
        } else if binding == SymbolBinding::Local {
            continue;
        } else {
            return Err(error(format!(
                "global external symbol '{name}' is defined in an unsupported section"
            )));
        };
        let kind = match kind {
            0 => SymbolKind::Function,
            1 => SymbolKind::Object,
            2 => SymbolKind::Function,
            3 => SymbolKind::Section,
            _ => return Err(error("unsupported external ELF symbol type")),
        };
        if name.is_empty() && kind != SymbolKind::Section {
            return Err(error("external non-section symbol has no name"));
        }
        let new_id = SymbolId(image.symbols.len() as u32);
        symbol_map[index] = Some(new_id);
        image.symbols.push(ObjectSymbol {
            name,
            binding,
            kind,
            definition,
            size,
        });
    }

    for section in sections.iter().filter(|section| section.kind == 9) {
        if section.entsize as usize != REL_SIZE || !(section.size as usize).is_multiple_of(REL_SIZE)
        {
            return Err(error("external REL section has invalid entry size"));
        }
        let target = section_map
            .get(&(section.info as usize))
            .copied()
            .ok_or_else(|| error("external relocation targets an unsupported section"))?;
        if section.link as usize != symtab_index {
            return Err(error(
                "external relocation does not link to the canonical symbol table",
            ));
        }
        let rels = section_bytes(bytes, section)?;
        for rel in rels.chunks_exact(REL_SIZE) {
            if image.relocations.len() >= MAX_OBJECT_RELOCATIONS {
                return Err(error("external relocation count exceeds supported bound"));
            }
            let offset = u32_at(rel, 0)?;
            let info = u32_at(rel, 4)?;
            let old_symbol = (info >> 8) as usize;
            let symbol = symbol_map
                .get(old_symbol)
                .copied()
                .flatten()
                .ok_or_else(|| error("external relocation references an unsupported symbol"))?;
            let kind = match info & 0xff {
                1 => RelocationKind::Abs32,
                2 => RelocationKind::Pc32,
                other => {
                    return Err(error(format!(
                        "unsupported external i386 relocation type {other}"
                    )))
                }
            };
            let target_data = image
                .sections
                .get(&target)
                .ok_or_else(|| error("external relocation target section is missing"))?;
            let SectionContents::Bytes(target_bytes) = &target_data.contents else {
                return Err(error("external relocation cannot target NOBITS"));
            };
            let end = offset
                .checked_add(4)
                .ok_or_else(|| error("external relocation range overflow"))?;
            let addend_bytes = target_bytes
                .get(offset as usize..end as usize)
                .ok_or_else(|| error("external relocation exceeds target section"))?;
            let addend = i32::from_le_bytes(addend_bytes.try_into().unwrap());
            image.relocations.push(ObjectRelocation {
                section: target,
                offset,
                symbol,
                kind,
                addend,
            });
        }
    }
    Ok(image)
}

type RawSection = (u32, u32, u32, u32, u32, u32, u32, u32, u32, u32);

fn read_section(bytes: &[u8], offset: usize) -> Result<RawSection, Diagnostic> {
    let section = bytes
        .get(offset..offset + SECTION_HEADER_SIZE)
        .ok_or_else(|| error("truncated external section header"))?;
    Ok((
        u32_at(section, 0)?,
        u32_at(section, 4)?,
        u32_at(section, 8)?,
        u32_at(section, 12)?,
        u32_at(section, 16)?,
        u32_at(section, 20)?,
        u32_at(section, 24)?,
        u32_at(section, 28)?,
        u32_at(section, 32)?,
        u32_at(section, 36)?,
    ))
}

fn section_bytes<'a>(bytes: &'a [u8], section: &Section) -> Result<&'a [u8], Diagnostic> {
    if section.kind == 8 {
        return Ok(&[]);
    }
    let start = section.offset as usize;
    let end = start
        .checked_add(section.size as usize)
        .ok_or_else(|| error("external section range overflow"))?;
    bytes
        .get(start..end)
        .ok_or_else(|| error("external section is truncated"))
}

fn string_at(strings: &[u8], offset: u32) -> Result<String, Diagnostic> {
    let tail = strings
        .get(offset as usize..)
        .ok_or_else(|| error("external string-table offset is invalid"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| error("external string table entry is unterminated"))?;
    std::str::from_utf8(&tail[..length])
        .map(str::to_owned)
        .map_err(|_| error("external ELF name is not UTF-8"))
}

fn validate_alignment(align: u32) -> Result<(), Diagnostic> {
    if align == 0 || !align.is_power_of_two() || align > MAX_OBJECT_ALIGNMENT {
        return Err(error("external section alignment is unsupported"));
    }
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, Diagnostic> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| error("truncated external ELF u16"))?;
    Ok(u16::from_le_bytes(value.try_into().unwrap()))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Diagnostic> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| error("truncated external ELF u32"))?;
    Ok(u32::from_le_bytes(value.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{write_relocatable, ObjectSection, ObjectSymbol};

    #[test]
    fn dbyte_metadata_cannot_bypass_canonical_parser_through_external_path() {
        let mut image = ObjectImage::empty();
        image.symbols.push(ObjectSymbol {
            name: "c_fixture".into(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: 0,
        });
        let bytes = write_relocatable(&image).unwrap().bytes;
        let error = parse_external_i386_object(&bytes).unwrap_err();
        assert!(error.message.contains("canonical DByte parser"));
    }
}
