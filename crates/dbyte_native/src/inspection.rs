//! Bounded read-only inspection for DByte ELF32 artifacts.

use crate::diagnostics::{Diagnostic, DiagnosticCategory};
use crate::object::{parse_relocatable, ObjectSection, SectionContents, SymbolDefinition};
use std::fmt::Write;

fn invalid(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidObject, message)
}

fn word(bytes: &[u8], offset: usize) -> Result<u16, Diagnostic> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or_else(|| invalid("truncated ELF word"))?
            .try_into()
            .unwrap(),
    ))
}

fn dword(bytes: &[u8], offset: usize) -> Result<u32, Diagnostic> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or_else(|| invalid("truncated ELF dword"))?
            .try_into()
            .unwrap(),
    ))
}

pub fn inspect_elf(
    bytes: &[u8],
    show_symbols: bool,
    show_relocations: bool,
    show_disassembly: bool,
) -> Result<String, Diagnostic> {
    if bytes.len() < 52
        || &bytes[..4] != b"\x7fELF"
        || bytes[4] != 1
        || bytes[5] != 1
        || word(bytes, 18)? != 3
    {
        return Err(invalid("inspect requires ELF32 little-endian EM_386"));
    }
    let elf_type = word(bytes, 16)?;
    let mut output = format!("ELF32 EM_386 type={elf_type} bytes={}\n", bytes.len());
    if elf_type == 1 {
        let object = parse_relocatable(bytes)?;
        if show_symbols {
            for (index, symbol) in object.symbols.iter().enumerate() {
                let definition = match symbol.definition {
                    SymbolDefinition::Undefined => "UND".to_owned(),
                    SymbolDefinition::Defined { section, value } => {
                        format!("{section:?}+0x{value:08x}")
                    }
                };
                let _ = writeln!(
                    output,
                    "SYMBOL {index:04} {:?} {:?} {} size={} {definition}",
                    symbol.binding, symbol.kind, symbol.name, symbol.size
                );
            }
        }
        if show_relocations {
            for relocation in &object.relocations {
                let name = object
                    .symbols
                    .get(relocation.symbol.0 as usize)
                    .map_or("<invalid>", |symbol| symbol.name.as_str());
                let _ = writeln!(
                    output,
                    "RELOCATION {:?}+0x{:08x} {:?} addend={} {name}",
                    relocation.section, relocation.offset, relocation.kind, relocation.addend
                );
            }
        }
        if show_disassembly {
            let text = object
                .sections
                .get(&ObjectSection::Text)
                .ok_or_else(|| invalid("object has no .text"))?;
            let SectionContents::Bytes(text) = &text.contents else {
                return Err(invalid("object .text is not initialized"));
            };
            disassemble(&mut output, 0, text);
        }
        return Ok(output);
    }
    if elf_type != 2 {
        return Err(invalid(format!("unsupported ELF type {elf_type}")));
    }
    let sections = sections(bytes)?;
    for section in &sections {
        if show_symbols && section.kind == 2 {
            inspect_symbols(bytes, &sections, section, &mut output)?;
        }
        if show_relocations && section.kind == 9 {
            inspect_relocations(bytes, section, &mut output)?;
        }
    }
    if show_disassembly {
        let text = sections
            .iter()
            .find(|section| section.name == ".text")
            .ok_or_else(|| invalid("executable has no .text"))?;
        disassemble(&mut output, text.address, data(bytes, text)?);
    }
    Ok(output)
}

#[derive(Clone)]
struct ElfSection {
    name: String,
    kind: u32,
    address: u32,
    offset: u32,
    size: u32,
    link: u32,
    entry_size: u32,
}

fn sections(bytes: &[u8]) -> Result<Vec<ElfSection>, Diagnostic> {
    let table = dword(bytes, 32)? as usize;
    let entry = word(bytes, 46)? as usize;
    let count = word(bytes, 48)? as usize;
    let names_index = word(bytes, 50)? as usize;
    if entry != 40 || count == 0 || count > 4096 || names_index >= count {
        return Err(invalid("invalid ELF section table"));
    }
    let table_size = count
        .checked_mul(entry)
        .ok_or_else(|| invalid("section table overflow"))?;
    if table
        .checked_add(table_size)
        .is_none_or(|end| end > bytes.len())
    {
        return Err(invalid("section table exceeds file"));
    }
    let names_header = table + names_index * entry;
    let names = bytes
        .get(
            dword(bytes, names_header + 16)? as usize
                ..(dword(bytes, names_header + 16)? as usize)
                    .checked_add(dword(bytes, names_header + 20)? as usize)
                    .ok_or_else(|| invalid("section names overflow"))?,
        )
        .ok_or_else(|| invalid("section names exceed file"))?;
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let header = table + index * entry;
        let name = string(names, dword(bytes, header)? as usize)?.to_owned();
        result.push(ElfSection {
            name,
            kind: dword(bytes, header + 4)?,
            address: dword(bytes, header + 12)?,
            offset: dword(bytes, header + 16)?,
            size: dword(bytes, header + 20)?,
            link: dword(bytes, header + 24)?,
            entry_size: dword(bytes, header + 36)?,
        });
    }
    Ok(result)
}

fn data<'a>(bytes: &'a [u8], section: &ElfSection) -> Result<&'a [u8], Diagnostic> {
    let start = section.offset as usize;
    bytes
        .get(
            start
                ..start
                    .checked_add(section.size as usize)
                    .ok_or_else(|| invalid("section overflow"))?,
        )
        .ok_or_else(|| invalid(format!("section '{}' exceeds file", section.name)))
}

fn string(bytes: &[u8], offset: usize) -> Result<&str, Diagnostic> {
    let tail = bytes
        .get(offset..)
        .ok_or_else(|| invalid("string offset is invalid"))?;
    let length = tail
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| invalid("unterminated ELF string"))?;
    std::str::from_utf8(&tail[..length]).map_err(|_| invalid("ELF string is not UTF-8"))
}

fn inspect_symbols(
    bytes: &[u8],
    sections: &[ElfSection],
    section: &ElfSection,
    output: &mut String,
) -> Result<(), Diagnostic> {
    if section.entry_size != 16 || !section.size.is_multiple_of(16) {
        return Err(invalid("invalid ELF32 symbol table"));
    }
    let strings = sections
        .get(section.link as usize)
        .ok_or_else(|| invalid("invalid symbol string table"))?;
    let strings = data(bytes, strings)?;
    for (index, symbol) in data(bytes, section)?.as_chunks::<16>().0.iter().enumerate() {
        let name = string(
            strings,
            u32::from_le_bytes(symbol[..4].try_into().unwrap()) as usize,
        )?;
        let value = u32::from_le_bytes(symbol[4..8].try_into().unwrap());
        let size = u32::from_le_bytes(symbol[8..12].try_into().unwrap());
        let section_index = u16::from_le_bytes(symbol[14..16].try_into().unwrap());
        let _ = writeln!(
            output,
            "SYMBOL {index:04} value=0x{value:08x} size={size} section={section_index} {name}"
        );
    }
    Ok(())
}

fn inspect_relocations(
    bytes: &[u8],
    section: &ElfSection,
    output: &mut String,
) -> Result<(), Diagnostic> {
    if section.entry_size != 8 || !section.size.is_multiple_of(8) {
        return Err(invalid("invalid ELF32 REL table"));
    }
    for relocation in data(bytes, section)?.as_chunks::<8>().0 {
        let offset = u32::from_le_bytes(relocation[..4].try_into().unwrap());
        let info = u32::from_le_bytes(relocation[4..8].try_into().unwrap());
        let _ = writeln!(
            output,
            "RELOCATION {} offset=0x{offset:08x} symbol={} type={}",
            section.name,
            info >> 8,
            info & 0xff
        );
    }
    Ok(())
}

fn disassemble(output: &mut String, base: u32, bytes: &[u8]) {
    let mut offset = 0usize;
    while offset < bytes.len() {
        let (size, mnemonic) = match bytes[offset] {
            0x55 => (1, "push ebp".to_owned()),
            0x5d => (1, "pop ebp".to_owned()),
            0xc3 => (1, "ret".to_owned()),
            0x90 => (1, "nop".to_owned()),
            0xfa => (1, "cli".to_owned()),
            0xfb => (1, "sti".to_owned()),
            0xf4 => (1, "hlt".to_owned()),
            0xcf => (1, "iretd".to_owned()),
            0xe8 if offset + 5 <= bytes.len() => {
                let displacement =
                    i32::from_le_bytes(bytes[offset + 1..offset + 5].try_into().unwrap());
                (5, format!("call rel32 {displacement:+}"))
            }
            0xcd if offset + 2 <= bytes.len() => (2, format!("int 0x{:02x}", bytes[offset + 1])),
            0x89 if bytes.get(offset + 1) == Some(&0xe5) => (2, "mov ebp, esp".to_owned()),
            byte => (1, format!("db 0x{byte:02x}")),
        };
        let size = size.min(bytes.len() - offset);
        let encoded = bytes[offset..offset + size]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(
            output,
            "DISASM {:08x} {:<14} {mnemonic}",
            base.wrapping_add(offset as u32),
            encoded
        );
        offset += size;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{
        write_relocatable, ObjectImage, ObjectSectionData, ObjectSymbol, SymbolBinding, SymbolKind,
    };

    #[test]
    fn relocatable_inspection_is_deterministic_and_exposes_machine_code() {
        let mut image = ObjectImage::empty();
        image.sections.insert(
            ObjectSection::Text,
            ObjectSectionData::bytes(vec![0x55, 0x89, 0xe5, 0x5d, 0xc3], 16),
        );
        image.symbols.push(ObjectSymbol {
            name: "entry".into(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: 5,
        });
        let object = write_relocatable(&image).unwrap();
        let first = inspect_elf(&object.bytes, true, true, true).unwrap();
        let second = inspect_elf(&object.bytes, true, true, true).unwrap();
        assert_eq!(first, second);
        assert!(first.contains("SYMBOL"));
        assert!(first.contains("mov ebp, esp"));
        assert!(first.contains("ret"));
    }

    #[test]
    fn executable_inspection_reads_final_symbols_without_mutation() {
        let artifact =
            crate::compile_source("inspect.dby", "fn kernel_main() -> int:\n    return 0\n")
                .unwrap();
        let dump = inspect_elf(&artifact.elf, true, true, true).unwrap();
        assert!(dump.contains("type=2"));
        assert!(dump.contains("kernel_main"));
        assert!(dump.contains("DISASM"));
    }
}
