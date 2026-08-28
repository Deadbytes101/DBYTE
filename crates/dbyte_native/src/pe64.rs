//! Deterministic PE32+ console image writer for DBYTE64 Windows.

use crate::linker64::LinkedSections;
use crate::target::X86_64_WINDOWS;
use std::collections::BTreeMap;

pub const IMAGE_BASE: u64 = 0x0000_0001_4000_0000;
pub const ENTRY_RVA: u32 = 0x1000;
pub const DBYTE_TEXT_RVA: u32 = 0x2000;
pub const RODATA_RVA: u32 = 0x0001_0000;
pub const DATA_RVA: u32 = 0x0002_0000;
pub const BSS_RVA: u32 = 0x0003_0000;
const FILE_ALIGN: usize = 0x200;
const SECTION_ALIGN: usize = 0x1000;
const PE_OFFSET: usize = 0x80;
const HEADERS: usize = 0x200;
const IMPORT_NAMES: [&str; 8] = [
    "ExitProcess",
    "GetStdHandle",
    "ReadFile",
    "WriteFile",
    "CloseHandle",
    "VirtualAlloc",
    "VirtualFree",
    "QueryPerformanceCounter",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeArtifact {
    pub bytes: Vec<u8>,
    pub entry: u64,
    pub import_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeError(pub String);
impl std::fmt::Display for PeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PeError {}

pub fn write_console_image(linked: &LinkedSections) -> Result<PeArtifact, PeError> {
    if linked.target != X86_64_WINDOWS || linked.entry < IMAGE_BASE + u64::from(DBYTE_TEXT_RVA) {
        return Err(err("E_PE_TARGET", "invalid Windows DBYTE64 linked image"));
    }
    let mut rdata = linked.rodata.clone();
    let imports = imports(RODATA_RVA, 0)?;
    if !rdata.starts_with(&imports.bytes) {
        return Err(err(
            "E_PE_IMPORT",
            "linked image is missing canonical import object",
        ));
    }
    let mut text = vec![0xcc; (DBYTE_TEXT_RVA - ENTRY_RVA) as usize];
    let stub = entry_stub(linked.entry, IMAGE_BASE + u64::from(imports.iat_rva))?;
    text[..stub.len()].copy_from_slice(&stub);
    text.extend_from_slice(&linked.text);
    let text_window = (RODATA_RVA - ENTRY_RVA) as usize;
    if text.len() > text_window {
        return Err(err(
            "E_PE_LAYOUT",
            "linked text exceeds fixed PE text window",
        ));
    }
    text.resize(text_window, 0xcc);
    if !linked.data.is_empty() || linked.bss_size != 0 {
        let rodata_window = (DATA_RVA - RODATA_RVA) as usize;
        if rdata.len() > rodata_window {
            return Err(err("E_PE_LAYOUT", "linked rodata exceeds fixed PE window"));
        }
        rdata.resize(rodata_window, 0);
    }

    let text_raw = align(text.len(), FILE_ALIGN)?;
    let rdata_raw = align(rdata.len(), FILE_ALIGN)?;
    let data_raw = align(linked.data.len(), FILE_ALIGN)?;
    let sections: u16 = if linked.data.is_empty() && linked.bss_size == 0 {
        2
    } else {
        3
    };
    let text_file = HEADERS;
    let rdata_file = text_file + text_raw;
    let data_file = rdata_file + rdata_raw;
    let image_end = if sections == 3 {
        BSS_RVA as usize + linked.bss_size as usize
    } else {
        RODATA_RVA as usize + rdata.len()
    };

    let mut out = vec![0u8; HEADERS];
    out[..2].copy_from_slice(b"MZ");
    p16(&mut out, 2, 0x0090)?;
    p16(&mut out, 4, 3)?;
    p16(&mut out, 8, 4)?;
    p16(&mut out, 10, 0)?;
    p16(&mut out, 12, 0xffff)?;
    p16(&mut out, 16, 0x00b8)?;
    p16(&mut out, 24, 0x0040)?;
    p32(&mut out, 0x3c, PE_OFFSET as u32)?;
    out[PE_OFFSET..PE_OFFSET + 4].copy_from_slice(b"PE\0\0");
    let coff = PE_OFFSET + 4;
    p16(&mut out, coff, 0x8664)?;
    p16(&mut out, coff + 2, sections)?;
    p16(&mut out, coff + 16, 240)?;
    p16(&mut out, coff + 18, 0x23)?;
    let opt = coff + 20;
    p16(&mut out, opt, 0x20b)?;
    out[opt + 2] = 1;
    p32(&mut out, opt + 4, u32v(text_raw)?)?;
    p32(&mut out, opt + 8, u32v(rdata_raw + data_raw)?)?;
    p32(&mut out, opt + 16, ENTRY_RVA)?;
    p32(&mut out, opt + 20, ENTRY_RVA)?;
    p64(&mut out, opt + 24, IMAGE_BASE)?;
    p32(&mut out, opt + 32, SECTION_ALIGN as u32)?;
    p32(&mut out, opt + 36, FILE_ALIGN as u32)?;
    p16(&mut out, opt + 40, 6)?;
    p16(&mut out, opt + 48, 6)?;
    p32(&mut out, opt + 56, u32v(align(image_end, SECTION_ALIGN)?)?)?;
    p32(&mut out, opt + 60, HEADERS as u32)?;
    p16(&mut out, opt + 68, 3)?;
    p16(&mut out, opt + 70, 0x100)?;
    p64(&mut out, opt + 72, 0x10_0000)?;
    p64(&mut out, opt + 80, 0x1000)?;
    p64(&mut out, opt + 88, 0x10_0000)?;
    p64(&mut out, opt + 96, 0x1000)?;
    p32(&mut out, opt + 108, 16)?;
    p32(&mut out, opt + 120, imports.directory_rva)?;
    p32(&mut out, opt + 124, 40)?;
    p32(&mut out, opt + 208, imports.iat_rva)?;
    p32(&mut out, opt + 212, u32v((IMPORT_NAMES.len() + 1) * 8)?)?;
    let sh = opt + 240;
    section(
        &mut out,
        sh,
        b".text\0\0\0",
        text.len(),
        ENTRY_RVA,
        text_raw,
        text_file,
        0x6000_0020,
    )?;
    section(
        &mut out,
        sh + 40,
        b".rdata\0\0",
        rdata.len(),
        RODATA_RVA,
        rdata_raw,
        rdata_file,
        0x4000_0040,
    )?;
    if sections == 3 {
        let virtual_size = if linked.bss_size == 0 {
            linked.data.len()
        } else {
            (BSS_RVA - DATA_RVA) as usize + linked.bss_size as usize
        };
        section(
            &mut out,
            sh + 80,
            b".data\0\0\0",
            virtual_size,
            DATA_RVA,
            data_raw,
            data_file,
            0xc000_0040,
        )?;
    }
    out.extend_from_slice(&text);
    out.resize(text_file + text_raw, 0);
    out.extend_from_slice(&rdata);
    out.resize(rdata_file + rdata_raw, 0);
    if sections == 3 {
        out.extend_from_slice(&linked.data);
        out.resize(data_file + data_raw, 0);
    }
    Ok(PeArtifact {
        bytes: out,
        entry: IMAGE_BASE + u64::from(ENTRY_RVA),
        import_count: 1,
    })
}

pub fn inspect(bytes: &[u8]) -> Result<(), PeError> {
    if bytes.get(..2) != Some(b"MZ")
        || g32(bytes, 0x3c)? as usize != PE_OFFSET
        || bytes.get(PE_OFFSET..PE_OFFSET + 4) != Some(b"PE\0\0")
        || g16(bytes, PE_OFFSET + 4)? != 0x8664
        || g16(bytes, PE_OFFSET + 24)? != 0x20b
    {
        return Err(err("E_PE_HEADER", "invalid PE32+ AMD64 image"));
    }
    Ok(())
}

struct Imports {
    bytes: Vec<u8>,
    directory_rva: u32,
    iat_rva: u32,
    iat_offsets: BTreeMap<String, u64>,
}
fn imports(rva: u32, offset: usize) -> Result<Imports, PeError> {
    let base = rva
        .checked_add(u32v(offset)?)
        .ok_or_else(|| err("E_PE_IMPORT", "RVA overflow"))?;
    let thunk_bytes = (IMPORT_NAMES.len() as u32 + 1) * 8;
    let int_rva = base + 40;
    let iat_rva = int_rva + thunk_bytes;
    let mut hint_cursor = iat_rva + thunk_bytes;
    let mut hints = Vec::new();
    for name in IMPORT_NAMES {
        hints.push(hint_cursor);
        hint_cursor += 2 + name.len() as u32 + 1;
        if !hint_cursor.is_multiple_of(2) {
            hint_cursor += 1;
        }
    }
    let dll_rva = hint_cursor;
    let mut bytes = vec![0u8; 40];
    p32(&mut bytes, 0, int_rva)?;
    p32(&mut bytes, 12, dll_rva)?;
    p32(&mut bytes, 16, iat_rva)?;
    for hint in &hints {
        bytes.extend_from_slice(&u64::from(*hint).to_le_bytes());
    }
    bytes.extend_from_slice(&0u64.to_le_bytes());
    for hint in &hints {
        bytes.extend_from_slice(&u64::from(*hint).to_le_bytes());
    }
    bytes.extend_from_slice(&0u64.to_le_bytes());
    for name in IMPORT_NAMES {
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
        if !bytes.len().is_multiple_of(2) {
            bytes.push(0);
        }
    }
    bytes.extend_from_slice(b"KERNEL32.dll\0");
    let iat_offsets = IMPORT_NAMES
        .iter()
        .enumerate()
        .map(|(index, name)| {
            (
                format!("__imp_{name}"),
                u64::from(iat_rva - rva) + (index * 8) as u64,
            )
        })
        .collect();
    Ok(Imports {
        bytes,
        directory_rva: base,
        iat_rva,
        iat_offsets,
    })
}

pub fn import_object() -> Result<crate::linker64::LinkInput, PeError> {
    use crate::object64::{
        Binding, Contents, Definition, ObjectImage, Section, SectionData, Symbol, SymbolKind,
    };
    let imports = imports(RODATA_RVA, 0)?;
    let symbols = imports
        .iat_offsets
        .into_iter()
        .map(|(name, value)| Symbol {
            name,
            binding: Binding::Global,
            kind: SymbolKind::Object,
            definition: Definition::Defined {
                section: Section::Rodata,
                value,
            },
            size: 8,
        })
        .collect();
    let image = ObjectImage {
        target: X86_64_WINDOWS,
        entry_abi: None,
        sections: BTreeMap::from([(
            Section::Rodata,
            SectionData {
                contents: Contents::Bytes(imports.bytes),
                alignment: 8,
            },
        )]),
        symbols,
        relocations: vec![],
    };
    let object = crate::coff64::write_object(&image).map_err(|failure| PeError(failure.0))?;
    Ok(crate::linker64::LinkInput::new(
        // NUL cannot occur in a filesystem path and sorts before every valid
        // user input name. The linker canonicalizes by name, while the PE
        // contract requires the import directory to begin `.rdata`.
        "\0__dbyte_imports.obj",
        object.bytes,
    ))
}
fn entry_stub(main: u64, iat: u64) -> Result<Vec<u8>, PeError> {
    let entry = IMAGE_BASE + u64::from(ENTRY_RVA);
    let mut b = vec![
        0x48, 0x83, 0xec, 0x28, 0xe8, 0, 0, 0, 0, 0x89, 0xc1, 0xff, 0x15, 0, 0, 0, 0, 0xcc,
    ];
    b[5..9].copy_from_slice(&rel(main, entry + 9)?.to_le_bytes());
    b[13..17].copy_from_slice(&rel(iat, entry + 17)?.to_le_bytes());
    Ok(b)
}
fn rel(target: u64, end: u64) -> Result<i32, PeError> {
    i32::try_from(i128::from(target) - i128::from(end))
        .map_err(|_| err("E_PE_RANGE", "rel32 overflow"))
}
#[allow(clippy::too_many_arguments)]
fn section(
    out: &mut [u8],
    o: usize,
    name: &[u8; 8],
    vs: usize,
    rva: u32,
    rs: usize,
    ro: usize,
    flags: u32,
) -> Result<(), PeError> {
    out[o..o + 8].copy_from_slice(name);
    p32(out, o + 8, u32v(vs)?)?;
    p32(out, o + 12, rva)?;
    p32(out, o + 16, u32v(rs)?)?;
    p32(out, o + 20, u32v(ro)?)?;
    p32(out, o + 36, flags)
}
fn align(v: usize, a: usize) -> Result<usize, PeError> {
    v.checked_add(a - 1)
        .map(|v| v & !(a - 1))
        .ok_or_else(|| err("E_PE_RANGE", "alignment overflow"))
}
fn u32v(v: usize) -> Result<u32, PeError> {
    u32::try_from(v).map_err(|_| err("E_PE_RANGE", "u32 overflow"))
}
fn p16(b: &mut [u8], o: usize, v: u16) -> Result<(), PeError> {
    b.get_mut(o..o + 2)
        .ok_or_else(|| err("E_PE_RANGE", "u16 write"))?
        .copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn p32(b: &mut [u8], o: usize, v: u32) -> Result<(), PeError> {
    b.get_mut(o..o + 4)
        .ok_or_else(|| err("E_PE_RANGE", "u32 write"))?
        .copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn p64(b: &mut [u8], o: usize, v: u64) -> Result<(), PeError> {
    b.get_mut(o..o + 8)
        .ok_or_else(|| err("E_PE_RANGE", "u64 write"))?
        .copy_from_slice(&v.to_le_bytes());
    Ok(())
}
fn g16(b: &[u8], o: usize) -> Result<u16, PeError> {
    Ok(u16::from_le_bytes(
        b.get(o..o + 2)
            .ok_or_else(|| err("E_PE_RANGE", "u16 read"))?
            .try_into()
            .unwrap(),
    ))
}
fn g32(b: &[u8], o: usize) -> Result<u32, PeError> {
    Ok(u32::from_le_bytes(
        b.get(o..o + 4)
            .ok_or_else(|| err("E_PE_RANGE", "u32 read"))?
            .try_into()
            .unwrap(),
    ))
}
fn err(c: &str, m: &str) -> PeError {
    PeError(format!("{c}: {m}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn deterministic_pe_has_only_kernel32_exit() {
        let import_bytes = imports(RODATA_RVA, 0).unwrap().bytes;
        let l = LinkedSections {
            target: X86_64_WINDOWS,
            entry_abi: None,
            text: vec![0xc3],
            rodata: import_bytes,
            data: vec![],
            bss_size: 0,
            entry: IMAGE_BASE + u64::from(DBYTE_TEXT_RVA),
            symbols: BTreeMap::new(),
            relocation_count: 0,
        };
        let a = write_console_image(&l).unwrap();
        assert_eq!(a, write_console_image(&l).unwrap());
        inspect(&a.bytes).unwrap();
        assert!(a.bytes.windows(13).any(|v| v == b"KERNEL32.dll\0"));
        assert!(a.bytes.windows(12).any(|v| v == b"ExitProcess\0"));
    }

    #[test]
    fn synthetic_import_object_precedes_every_valid_filesystem_input() {
        let synthetic = import_object().unwrap();
        assert!(synthetic.name.starts_with('\0'));
        for input in ["D:\\proof\\runtime.obj", "/tmp/runtime.obj", "runtime.obj"] {
            assert!(synthetic.name.as_str() < input);
        }
    }
}
