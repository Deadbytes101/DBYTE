//! Deterministic System V/GNU `ar` archives for DByte ELF32/ELF64 objects.

use crate::diagnostics::{Diagnostic, DiagnosticCategory};
use crate::object::{parse_relocatable, SymbolBinding, SymbolDefinition};
use std::collections::BTreeMap;

const MAGIC: &[u8; 8] = b"!<arch>\n";
const HEADER_SIZE: usize = 60;
const MAX_ARCHIVE_SIZE: usize = 64 * 1024 * 1024;
const MAX_MEMBERS: usize = 65_535;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveMember {
    pub name: String,
    pub bytes: Vec<u8>,
}

fn error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::InvalidObject, message)
}

pub fn write_archive(members: &[ArchiveMember]) -> Result<Vec<u8>, Diagnostic> {
    if members.is_empty() {
        return Err(error("static archive cannot be empty"));
    }
    if members.len() > MAX_MEMBERS {
        return Err(error("static archive member count exceeds bound"));
    }
    let mut canonical = members.to_vec();
    canonical.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in canonical.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(error(format!(
                "duplicate archive member '{}'",
                pair[0].name
            )));
        }
    }
    let mut symbols = Vec::<(String, usize)>::new();
    let mut archive_target = None::<String>;
    for (member_index, member) in canonical.iter().enumerate() {
        validate_name(&member.name)?;
        let (target, member_symbols) = defined_global_symbols(&member.bytes)?;
        if archive_target
            .as_ref()
            .is_some_and(|current| current != &target)
        {
            return Err(error("archive members have mixed object targets"));
        }
        archive_target.get_or_insert(target);
        for symbol in member_symbols {
            if !symbol.starts_with("__dbyte_") {
                symbols.push((symbol, member_index));
            }
        }
    }
    symbols.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    for pair in symbols.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(error(format!(
                "duplicate global archive symbol '{}'",
                pair[0].0
            )));
        }
    }

    let symbol_data_len = 4usize
        .checked_add(
            symbols
                .len()
                .checked_mul(4)
                .ok_or_else(|| error("archive symbol table overflow"))?,
        )
        .and_then(|size| {
            symbols
                .iter()
                .try_fold(size, |size, (name, _)| size.checked_add(name.len() + 1))
        })
        .ok_or_else(|| error("archive symbol table overflow"))?;
    let symbol_member_total = HEADER_SIZE
        .checked_add(symbol_data_len)
        .and_then(|size| size.checked_add(symbol_data_len & 1))
        .ok_or_else(|| error("archive size overflow"))?;
    let mut member_offsets = Vec::with_capacity(canonical.len());
    let mut offset = MAGIC
        .len()
        .checked_add(symbol_member_total)
        .ok_or_else(|| error("archive size overflow"))?;
    for member in &canonical {
        member_offsets
            .push(u32::try_from(offset).map_err(|_| error("archive offset exceeds u32"))?);
        offset = offset
            .checked_add(HEADER_SIZE)
            .and_then(|value| value.checked_add(member.bytes.len()))
            .and_then(|value| value.checked_add(member.bytes.len() & 1))
            .ok_or_else(|| error("archive size overflow"))?;
    }
    if offset > MAX_ARCHIVE_SIZE {
        return Err(error("archive exceeds 64 MiB bound"));
    }

    let mut symbol_data = Vec::with_capacity(symbol_data_len);
    symbol_data.extend_from_slice(&(symbols.len() as u32).to_be_bytes());
    for (_, member) in &symbols {
        symbol_data.extend_from_slice(&member_offsets[*member].to_be_bytes());
    }
    for (name, _) in &symbols {
        symbol_data.extend_from_slice(name.as_bytes());
        symbol_data.push(0);
    }

    let mut output = Vec::with_capacity(offset);
    output.extend_from_slice(MAGIC);
    append_member(&mut output, "/", &symbol_data)?;
    for member in &canonical {
        append_member(&mut output, &format!("{}/", member.name), &member.bytes)?;
    }
    Ok(output)
}

fn defined_global_symbols(bytes: &[u8]) -> Result<(String, Vec<String>), Diagnostic> {
    if bytes.get(0..4) != Some(b"\x7fELF") {
        let object =
            crate::coff64::parse_object(bytes).map_err(|failure| error(failure.to_string()))?;
        let target = format!("coff-amd64:{}", object.target.triple);
        let symbols = object
            .symbols
            .into_iter()
            .filter(|symbol| {
                symbol.binding == crate::object64::Binding::Global
                    && matches!(
                        symbol.definition,
                        crate::object64::Definition::Defined { .. }
                    )
            })
            .map(|symbol| symbol.name)
            .collect();
        return Ok((target, symbols));
    }
    match bytes.get(4).copied() {
        Some(1) => {
            let object = parse_relocatable(bytes)?;
            let symbols = object
                .symbols
                .into_iter()
                .filter(|symbol| {
                    symbol.binding == SymbolBinding::Global
                        && matches!(symbol.definition, SymbolDefinition::Defined { .. })
                })
                .map(|symbol| symbol.name)
                .collect();
            Ok(("elf32-i386".into(), symbols))
        }
        Some(2) => {
            let object = crate::object64::parse_relocatable(bytes)
                .map_err(|failure| error(failure.to_string()))?;
            let target = format!("elf64-x86-64:{}", object.target.triple);
            let symbols = object
                .symbols
                .into_iter()
                .filter(|symbol| {
                    symbol.binding == crate::object64::Binding::Global
                        && matches!(
                            symbol.definition,
                            crate::object64::Definition::Defined { .. }
                        )
                })
                .map(|symbol| symbol.name)
                .collect();
            Ok((target, symbols))
        }
        _ => Err(error("archive member has unsupported ELF class")),
    }
}

pub fn parse_archive(bytes: &[u8]) -> Result<Vec<ArchiveMember>, Diagnostic> {
    if bytes.len() > MAX_ARCHIVE_SIZE || !bytes.starts_with(MAGIC) {
        return Err(error("invalid or oversized static archive"));
    }
    let mut offset = MAGIC.len();
    let mut members = Vec::new();
    let mut symbol_offsets = BTreeMap::<u32, Vec<String>>::new();
    while offset < bytes.len() {
        let header_end = offset
            .checked_add(HEADER_SIZE)
            .ok_or_else(|| error("archive header range overflow"))?;
        let header = bytes
            .get(offset..header_end)
            .ok_or_else(|| error("truncated archive member header"))?;
        if &header[58..60] != b"`\n" {
            return Err(error("invalid archive member trailer"));
        }
        let raw_name = ascii_field(&header[0..16], "member name")?;
        let size = parse_decimal(&header[48..58], "member size")?;
        let data_start = header_end;
        let data_end = data_start
            .checked_add(size)
            .ok_or_else(|| error("archive member range overflow"))?;
        let data = bytes
            .get(data_start..data_end)
            .ok_or_else(|| error("truncated archive member"))?;
        if raw_name == "/" {
            parse_symbol_table(data, &mut symbol_offsets)?;
        } else {
            let name = raw_name
                .strip_suffix('/')
                .ok_or_else(|| error("archive member name is not canonical"))?;
            validate_name(name)?;
            members.push(ArchiveMember {
                name: name.to_owned(),
                bytes: data.to_vec(),
            });
        }
        offset = data_end
            .checked_add(size & 1)
            .ok_or_else(|| error("archive padding range overflow"))?;
        if size & 1 != 0 && bytes.get(data_end) != Some(&b'\n') {
            return Err(error("archive member padding is not canonical"));
        }
        if members.len() > MAX_MEMBERS {
            return Err(error("archive member count exceeds bound"));
        }
    }
    if members.is_empty() {
        return Err(error("static archive contains no object members"));
    }
    let actual_offsets: BTreeMap<_, _> = scan_member_offsets(bytes)?.into_iter().collect();
    for indexed_offset in symbol_offsets.keys() {
        if !actual_offsets.contains_key(indexed_offset) {
            return Err(error("archive symbol table references a non-member offset"));
        }
    }
    members.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in members.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(error(format!(
                "duplicate archive member '{}'",
                pair[0].name
            )));
        }
    }
    Ok(members)
}

fn append_member(output: &mut Vec<u8>, name: &str, data: &[u8]) -> Result<(), Diagnostic> {
    let mut header = [b' '; HEADER_SIZE];
    write_field(&mut header[0..16], name)?;
    write_field(&mut header[16..28], "0")?;
    write_field(&mut header[28..34], "0")?;
    write_field(&mut header[34..40], "0")?;
    write_field(&mut header[40..48], "100644")?;
    write_field(&mut header[48..58], &data.len().to_string())?;
    header[58..60].copy_from_slice(b"`\n");
    output.extend_from_slice(&header);
    output.extend_from_slice(data);
    if data.len() & 1 != 0 {
        output.push(b'\n');
    }
    Ok(())
}

fn write_field(field: &mut [u8], value: &str) -> Result<(), Diagnostic> {
    if !value.is_ascii() || value.len() > field.len() {
        return Err(error("archive header field is not representable"));
    }
    field[..value.len()].copy_from_slice(value.as_bytes());
    Ok(())
}

fn ascii_field(field: &[u8], context: &str) -> Result<String, Diagnostic> {
    if !field.is_ascii() {
        return Err(error(format!("archive {context} is not ASCII")));
    }
    Ok(String::from_utf8_lossy(field).trim().to_owned())
}

fn parse_decimal(field: &[u8], context: &str) -> Result<usize, Diagnostic> {
    ascii_field(field, context)?
        .parse::<usize>()
        .map_err(|_| error(format!("archive {context} is not canonical decimal")))
}

fn validate_name(name: &str) -> Result<(), Diagnostic> {
    if name.is_empty()
        || name.len() > 15
        || !name.is_ascii()
        || name
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'\\' | 0))
    {
        return Err(error(format!("invalid archive member name '{name}'")));
    }
    Ok(())
}

fn parse_symbol_table(
    data: &[u8],
    offsets: &mut BTreeMap<u32, Vec<String>>,
) -> Result<(), Diagnostic> {
    let count_bytes = data
        .get(0..4)
        .ok_or_else(|| error("truncated archive symbol count"))?;
    let count = u32::from_be_bytes(count_bytes.try_into().unwrap()) as usize;
    let offsets_end = 4usize
        .checked_add(
            count
                .checked_mul(4)
                .ok_or_else(|| error("archive symbol count overflow"))?,
        )
        .ok_or_else(|| error("archive symbol table overflow"))?;
    let raw_offsets = data
        .get(4..offsets_end)
        .ok_or_else(|| error("truncated archive symbol offsets"))?;
    let names = data
        .get(offsets_end..)
        .ok_or_else(|| error("truncated archive symbol names"))?;
    let mut cursor = 0;
    for index in 0..count {
        let start = index * 4;
        let member_offset = u32::from_be_bytes(raw_offsets[start..start + 4].try_into().unwrap());
        let end = names[cursor..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|length| cursor + length)
            .ok_or_else(|| error("unterminated archive symbol name"))?;
        let name = std::str::from_utf8(&names[cursor..end])
            .map_err(|_| error("archive symbol name is not UTF-8"))?;
        if name.is_empty() {
            return Err(error("archive symbol name cannot be empty"));
        }
        offsets
            .entry(member_offset)
            .or_default()
            .push(name.to_owned());
        cursor = end + 1;
    }
    if cursor != names.len() {
        return Err(error("archive symbol table has trailing bytes"));
    }
    Ok(())
}

fn scan_member_offsets(bytes: &[u8]) -> Result<Vec<(u32, String)>, Diagnostic> {
    let mut offset = MAGIC.len();
    let mut result = Vec::new();
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + HEADER_SIZE)
            .ok_or_else(|| error("truncated archive during index validation"))?;
        let name = ascii_field(&header[0..16], "member name")?;
        let size = parse_decimal(&header[48..58], "member size")?;
        if name != "/" {
            result.push((offset as u32, name));
        }
        offset = offset
            .checked_add(HEADER_SIZE + size + (size & 1))
            .ok_or_else(|| error("archive scan overflow"))?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{write_relocatable, ObjectImage, ObjectSection, ObjectSymbol, SymbolKind};

    fn object(symbol: &str) -> Vec<u8> {
        let mut image = ObjectImage::empty();
        image.symbols.push(ObjectSymbol {
            name: symbol.into(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: 0,
        });
        write_relocatable(&image).unwrap().bytes
    }

    #[test]
    fn archive_is_deterministic_canonical_and_round_trips() {
        let inputs = vec![
            ArchiveMember {
                name: "b.o".into(),
                bytes: object("beta"),
            },
            ArchiveMember {
                name: "a.o".into(),
                bytes: object("alpha"),
            },
        ];
        let first = write_archive(&inputs).unwrap();
        let second = write_archive(&inputs).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with(MAGIC));
        let parsed = parse_archive(&first).unwrap();
        assert_eq!(
            parsed
                .iter()
                .map(|member| member.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.o", "b.o"]
        );
    }

    #[test]
    fn malformed_archive_fails_closed() {
        let mut archive = write_archive(&[ArchiveMember {
            name: "a.o".into(),
            bytes: object("alpha"),
        }])
        .unwrap();
        archive[8 + 58] = b'!';
        assert!(parse_archive(&archive).is_err());
    }

    #[test]
    fn elf64_members_have_canonical_symbol_index_and_mixed_targets_fail() {
        use crate::object64::{
            write_relocatable as write64, Binding as Binding64, Definition as Definition64,
            ObjectImage as ObjectImage64, Symbol as Symbol64, SymbolKind as SymbolKind64,
        };
        use crate::target::{X86_64_LINUX, X86_64_NONE};
        use std::collections::BTreeMap;

        let object64 = |target, symbol: &str| {
            write64(&ObjectImage64 {
                target,
                entry_abi: None,
                sections: BTreeMap::new(),
                symbols: vec![Symbol64 {
                    name: symbol.into(),
                    binding: Binding64::Global,
                    kind: SymbolKind64::Function,
                    definition: Definition64::Defined {
                        section: crate::object64::Section::Text,
                        value: 0,
                    },
                    size: 0,
                }],
                relocations: vec![],
            })
            .unwrap()
            .bytes
        };
        let archive = write_archive(&[
            ArchiveMember {
                name: "b.o".into(),
                bytes: object64(X86_64_LINUX, "beta64"),
            },
            ArchiveMember {
                name: "a.o".into(),
                bytes: object64(X86_64_LINUX, "alpha64"),
            },
        ])
        .unwrap();
        assert_eq!(parse_archive(&archive).unwrap().len(), 2);

        let mixed = write_archive(&[
            ArchiveMember {
                name: "linux.o".into(),
                bytes: object64(X86_64_LINUX, "linux_symbol"),
            },
            ArchiveMember {
                name: "none.o".into(),
                bytes: object64(X86_64_NONE, "none_symbol"),
            },
        ]);
        assert!(mixed.unwrap_err().message.contains("mixed object targets"));
    }
}
