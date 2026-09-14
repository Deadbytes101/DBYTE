//! Constrained textual frontend for the existing bounded DBYTE64 encoder.

use crate::object64::{
    write_relocatable, Binding, Contents, Definition, ObjectArtifact, ObjectImage, Relocation,
    RelocationKind, Section, SectionData, Symbol, SymbolKind,
};
use crate::target::{TargetSpec, X86_64_NONE};
use crate::x86_64::{Condition, Encoder, Register, Width};
use std::collections::{BTreeMap, BTreeSet};

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_LINES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbtasmError(pub String);

impl std::fmt::Display for DbtasmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DbtasmError {}

#[derive(Debug, Clone)]
struct Fixup {
    line: usize,
    immediate: usize,
    symbol: String,
}

pub fn assemble(source: &str, target: TargetSpec) -> Result<ObjectArtifact, DbtasmError> {
    if target != X86_64_NONE {
        return Err(error(0, "DBTASM64 currently requires x86_64-dbyte-none"));
    }
    if source.len() > MAX_SOURCE_BYTES {
        return Err(error(0, "source exceeds the 1 MiB bound"));
    }
    let lines: Vec<_> = source.lines().collect();
    if lines.len() > MAX_LINES {
        return Err(error(0, "source exceeds the line-count bound"));
    }
    let mut globals = BTreeSet::new();
    let mut externs = BTreeSet::new();
    for (index, raw) in lines.iter().enumerate() {
        let line = strip_comment(raw).trim();
        if let Some(name) = line.strip_prefix("global ") {
            insert_declaration(&mut globals, name.trim(), index + 1, "global")?;
        } else if let Some(name) = line.strip_prefix("extern ") {
            insert_declaration(&mut externs, name.trim(), index + 1, "extern")?;
        }
    }
    if let Some(name) = globals.intersection(&externs).next() {
        return Err(error(
            0,
            &format!("symbol `{name}` is both global and extern"),
        ));
    }

    let mut encoder = Encoder::new(MAX_TEXT_BYTES);
    let mut labels = BTreeMap::<String, usize>::new();
    let mut fixups = Vec::new();
    let mut relocations = Vec::new();
    for (index, raw) in lines.iter().enumerate() {
        let line_number = index + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() || line.starts_with("global ") || line.starts_with("extern ") {
            continue;
        }
        if let Some(name) = line.strip_suffix(':') {
            let name = validate_identifier(name.trim(), line_number)?;
            if labels.insert(name.to_owned(), encoder.position()).is_some() {
                return Err(error(line_number, &format!("duplicate label `{name}`")));
            }
            continue;
        }
        if let Some(symbol) = line.strip_prefix("call ") {
            let symbol = validate_identifier(symbol.trim(), line_number)?.to_owned();
            let immediate = encoder
                .call_relative_placeholder()
                .map_err(|failure| encode_error(line_number, failure))?;
            if externs.contains(&symbol) {
                relocations.push(Relocation {
                    section: Section::Text,
                    offset: immediate as u64,
                    symbol,
                    kind: RelocationKind::Pc32,
                    addend: -4,
                });
            } else {
                fixups.push(Fixup {
                    line: line_number,
                    immediate,
                    symbol,
                });
            }
            continue;
        }
        if let Some(symbol) = line.strip_prefix("jmp ") {
            let immediate = encoder
                .jump_relative_placeholder()
                .map_err(|failure| encode_error(line_number, failure))?;
            fixups.push(Fixup {
                line: line_number,
                immediate,
                symbol: validate_identifier(symbol.trim(), line_number)?.to_owned(),
            });
            continue;
        }
        let conditional = [("je ", Condition::Equal), ("jne ", Condition::NotEqual)];
        if let Some((symbol, condition)) = conditional.iter().find_map(|(prefix, condition)| {
            line.strip_prefix(prefix).map(|rest| (rest, *condition))
        }) {
            let immediate = encoder
                .jump_if_placeholder(condition)
                .map_err(|failure| encode_error(line_number, failure))?;
            fixups.push(Fixup {
                line: line_number,
                immediate,
                symbol: validate_identifier(symbol.trim(), line_number)?.to_owned(),
            });
            continue;
        }
        if let Some(value) = line.strip_prefix("cmp eax, ") {
            encoder
                .cmp_reg_imm(
                    Register::Rax,
                    parse_u32(value.trim(), line_number)?,
                    Width::Dword,
                )
                .map_err(|failure| encode_error(line_number, failure))?;
            continue;
        }
        if let Some(text) = line.strip_prefix("debug ") {
            let bytes = parse_string(text.trim(), line_number)?;
            encoder
                .mov_imm32(Register::Rdx, 0xe9)
                .map_err(|failure| encode_error(line_number, failure))?;
            for byte in bytes {
                encoder
                    .mov_imm32(Register::Rax, u32::from(byte))
                    .and_then(|_| encoder.out8())
                    .map_err(|failure| encode_error(line_number, failure))?;
            }
            continue;
        }
        let operation = match line {
            "cli" => encoder.cli(),
            "hlt" => encoder.hlt(),
            "ret" => encoder.ret(),
            _ => {
                return Err(error(
                    line_number,
                    &format!("unsupported statement `{line}`"),
                ))
            }
        };
        operation.map_err(|failure| encode_error(line_number, failure))?;
    }

    for global in &globals {
        if !labels.contains_key(global) {
            return Err(error(0, &format!("global `{global}` has no label")));
        }
    }
    for fixup in fixups {
        let destination = labels.get(&fixup.symbol).copied().ok_or_else(|| {
            error(
                fixup.line,
                &format!("unresolved local symbol `{}`", fixup.symbol),
            )
        })?;
        encoder
            .patch_relative_i32(fixup.immediate, destination)
            .map_err(|failure| encode_error(fixup.line, failure))?;
    }
    let mut symbols = labels
        .into_iter()
        .map(|(name, value)| Symbol {
            binding: if globals.contains(&name) {
                Binding::Global
            } else {
                Binding::Local
            },
            name,
            kind: SymbolKind::Function,
            definition: Definition::Defined {
                section: Section::Text,
                value: value as u64,
            },
            size: 0,
        })
        .collect::<Vec<_>>();
    symbols.extend(externs.into_iter().map(|name| Symbol {
        name,
        binding: Binding::Global,
        kind: SymbolKind::Function,
        definition: Definition::Undefined,
        size: 0,
    }));
    let image = ObjectImage {
        target,
        entry_abi: None,
        sections: BTreeMap::from([(
            Section::Text,
            SectionData {
                contents: Contents::Bytes(encoder.into_bytes()),
                alignment: 16,
            },
        )]),
        symbols,
        relocations,
    };
    write_relocatable(&image).map_err(|failure| DbtasmError(failure.to_string()))
}

fn insert_declaration(
    declarations: &mut BTreeSet<String>,
    name: &str,
    line: usize,
    kind: &str,
) -> Result<(), DbtasmError> {
    let name = validate_identifier(name, line)?;
    if !declarations.insert(name.to_owned()) {
        return Err(error(line, &format!("duplicate {kind} `{name}`")));
    }
    Ok(())
}

fn validate_identifier(name: &str, line: usize) -> Result<&str, DbtasmError> {
    let valid = !name.is_empty()
        && name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        });
    if !valid {
        return Err(error(line, &format!("invalid identifier `{name}`")));
    }
    Ok(name)
}

fn parse_u32(value: &str, line: usize) -> Result<u32, DbtasmError> {
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u32::from_str_radix(hex, 16)
    } else {
        value.parse()
    };
    parsed.map_err(|_| error(line, "expected a u32 immediate"))
}

fn parse_string(value: &str, line: usize) -> Result<Vec<u8>, DbtasmError> {
    if value.len() < 2 || !value.starts_with('"') || !value.ends_with('"') {
        return Err(error(line, "debug requires a quoted ASCII string"));
    }
    let mut output = Vec::new();
    let mut escaped = false;
    for byte in value.as_bytes()[1..value.len() - 1].iter().copied() {
        if escaped {
            output.push(match byte {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'\\' => b'\\',
                b'"' => b'"',
                _ => return Err(error(line, "unsupported debug string escape")),
            });
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte.is_ascii() && !byte.is_ascii_control() {
            output.push(byte);
        } else {
            return Err(error(
                line,
                "debug string must be printable ASCII plus escapes",
            ));
        }
    }
    if escaped {
        return Err(error(line, "debug string ends with an escape"));
    }
    Ok(output)
}

fn strip_comment(line: &str) -> &str {
    line.split_once(';').map_or(line, |(code, _)| code)
}

fn encode_error(line: usize, failure: crate::x86_64::EncodeError) -> DbtasmError {
    error(line, &failure.to_string())
}

fn error(line: usize, message: &str) -> DbtasmError {
    if line == 0 {
        DbtasmError(format!("E_DBTASM64: {message}"))
    } else {
        DbtasmError(format!("E_DBTASM64: line {line}: {message}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn textual_frontend_uses_canonical_encoder_and_object_relocations() {
        let source = "global entry\nextern body\nentry:\n call body\n cmp eax, 0x5a17\n jne fail\n debug \"OK\\n\"\nhalt:\n cli\n hlt\n jmp halt\nfail:\n ret\n";
        let first = assemble(source, X86_64_NONE).unwrap();
        let second = assemble(source, X86_64_NONE).unwrap();
        assert_eq!(first, second);
        let image = crate::object64::parse_relocatable(&first.bytes).unwrap();
        assert_eq!(image.target, X86_64_NONE);
        assert_eq!(image.relocations.len(), 1);
        assert_eq!(image.relocations[0].symbol, "body");
        assert_eq!(image.relocations[0].kind, RelocationKind::Pc32);
        assert!(image.symbols.iter().any(|symbol| symbol.name == "entry"));
    }

    #[test]
    fn textual_frontend_fails_closed_on_scope_target_and_syntax() {
        assert!(assemble("global missing\n", X86_64_NONE)
            .unwrap_err()
            .0
            .contains("has no label"));
        assert!(assemble("entry:\n jmp nowhere\n", X86_64_NONE)
            .unwrap_err()
            .0
            .contains("unresolved local"));
        assert!(assemble("entry:\n nop\n", X86_64_NONE)
            .unwrap_err()
            .0
            .contains("unsupported statement"));
        assert!(assemble("entry:\n ret\n", crate::target::X86_64_LINUX)
            .unwrap_err()
            .0
            .contains("requires x86_64-dbyte-none"));
    }
}
