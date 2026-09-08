//! Deterministic bounded ELF64/x86-64 static executable support.

const ELF_HEADER_SIZE: u64 = 64;
const PROGRAM_HEADER_SIZE: u64 = 56;
const SECTION_HEADER_SIZE: u64 = 64;
const PROGRAM_HEADER_COUNT: u16 = 2;
const SECTION_COUNT: u16 = 6;
const IMAGE_BASE: u64 = 0x0040_0000;
const TEXT_OFFSET: u64 = 0x1000;
const MAX_IMAGE_SIZE: usize = 64 * 1024 * 1024;
pub const USER_IMAGE_BASE: u64 = 0x0040_0000;
pub const USER_IMAGE_END: u64 = 0x0060_0000;
pub const USER_TEXT_OFFSET: u64 = ELF_HEADER_SIZE + 2 * PROGRAM_HEADER_SIZE;
pub const USER_PAGE_SIZE: u64 = 0x1000;
pub const USER_MAX_FILE_SIZE: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutableInput {
    pub text: Vec<u8>,
    pub rodata: Vec<u8>,
    pub data: Vec<u8>,
    pub bss_size: u64,
    pub entry_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf64Layout {
    pub entry: u64,
    pub text_address: u64,
    pub rodata_address: u64,
    pub data_address: u64,
    pub bss_address: u64,
    pub file_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf64Executable {
    pub bytes: Vec<u8>,
    pub layout: Elf64Layout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf64Info {
    pub object_type: u16,
    pub machine: u16,
    pub entry: u64,
    pub program_headers: u16,
    pub section_headers: u16,
    pub has_interp: bool,
    pub has_dynamic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Elf64Error(pub String);

impl std::fmt::Display for Elf64Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Elf64Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserElf64Layout {
    pub text_offset: u64,
    pub rodata_offset: u64,
    pub data_offset: u64,
    pub text_address: u64,
    pub rodata_address: u64,
    pub data_address: u64,
    pub bss_address: u64,
    pub rx_file_size: u64,
    pub rw_memory_size: u64,
    pub file_size: u64,
}

pub fn user_executable_layout(
    text_size: u64,
    rodata_size: u64,
    data_size: u64,
    bss_size: u64,
) -> Result<UserElf64Layout, Elf64Error> {
    if text_size == 0 {
        return Err(error(
            "E_ELF64_TEXT",
            "user executable text cannot be empty",
        ));
    }
    let text_offset = USER_TEXT_OFFSET;
    let text_end = add(text_offset, text_size, "user text end")?;
    let rodata_offset = align_up(text_end, 8)?;
    let rx_file_end = add(rodata_offset, rodata_size, "user RX file end")?;
    let data_offset = align_up(rx_file_end, 16)?;
    let file_size = add(data_offset, data_size, "user file size")?;
    if file_size > USER_MAX_FILE_SIZE as u64 {
        return Err(error(
            "E_ELF64_USER_SIZE",
            "compact user ELF64 exceeds the DBFS1 4096-byte file limit",
        ));
    }
    let text_address = add(USER_IMAGE_BASE, text_offset, "user text address")?;
    let rodata_address = add(USER_IMAGE_BASE, rodata_offset, "user rodata address")?;
    let rx_memory_end = add(USER_IMAGE_BASE, rx_file_end, "user RX memory end")?;
    let data_page = align_up(rx_memory_end, USER_PAGE_SIZE)?;
    let data_address = add(
        data_page,
        data_offset & (USER_PAGE_SIZE - 1),
        "user data address",
    )?;
    let data_end = add(data_address, data_size, "user data end")?;
    let bss_address = align_up(data_end, 16)?;
    let memory_end = add(bss_address, bss_size, "user image end")?;
    if memory_end > USER_IMAGE_END {
        return Err(error(
            "E_ELF64_USER_SIZE",
            "compact user ELF64 exceeds the reserved user image range",
        ));
    }
    Ok(UserElf64Layout {
        text_offset,
        rodata_offset,
        data_offset,
        text_address,
        rodata_address,
        data_address,
        bss_address,
        rx_file_size: rx_file_end - text_offset,
        rw_memory_size: memory_end - data_address,
        file_size,
    })
}

pub fn write_user_executable(input: &ExecutableInput) -> Result<Elf64Executable, Elf64Error> {
    if input.entry_offset >= input.text.len() as u64 {
        return Err(error("E_ELF64_ENTRY", "entry offset is outside .text"));
    }
    let layout = user_executable_layout(
        input.text.len() as u64,
        input.rodata.len() as u64,
        input.data.len() as u64,
        input.bss_size,
    )?;
    let file_len = usize::try_from(layout.file_size)
        .map_err(|_| error("E_ELF64_SIZE", "user ELF64 exceeds host addressability"))?;
    let mut bytes = vec![0u8; file_len];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    put_u16(&mut bytes, 16, 2)?;
    put_u16(&mut bytes, 18, 62)?;
    put_u32(&mut bytes, 20, 1)?;
    let entry = add(layout.text_address, input.entry_offset, "user entry")?;
    put_u64(&mut bytes, 24, entry)?;
    put_u64(&mut bytes, 32, ELF_HEADER_SIZE)?;
    put_u64(&mut bytes, 40, 0)?;
    put_u16(&mut bytes, 52, ELF_HEADER_SIZE as u16)?;
    put_u16(&mut bytes, 54, PROGRAM_HEADER_SIZE as u16)?;
    put_u16(&mut bytes, 56, PROGRAM_HEADER_COUNT)?;
    put_u16(&mut bytes, 58, 0)?;
    put_u16(&mut bytes, 60, 0)?;
    put_u16(&mut bytes, 62, 0)?;

    write_program_header(
        &mut bytes,
        ELF_HEADER_SIZE,
        1,
        5,
        layout.text_offset,
        layout.text_address,
        layout.rx_file_size,
        layout.rx_file_size,
        USER_PAGE_SIZE,
    )?;
    write_program_header(
        &mut bytes,
        ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE,
        1,
        6,
        layout.data_offset,
        layout.data_address,
        input.data.len() as u64,
        layout.rw_memory_size,
        USER_PAGE_SIZE,
    )?;
    copy_at(&mut bytes, layout.text_offset, &input.text)?;
    copy_at(&mut bytes, layout.rodata_offset, &input.rodata)?;
    copy_at(&mut bytes, layout.data_offset, &input.data)?;

    Ok(Elf64Executable {
        bytes,
        layout: Elf64Layout {
            entry,
            text_address: layout.text_address,
            rodata_address: layout.rodata_address,
            data_address: layout.data_address,
            bss_address: layout.bss_address,
            file_size: layout.file_size,
        },
    })
}

pub fn write_static_executable(input: &ExecutableInput) -> Result<Elf64Executable, Elf64Error> {
    if input.text.is_empty() {
        return Err(error("E_ELF64_TEXT", "executable text cannot be empty"));
    }
    if input.entry_offset >= input.text.len() as u64 {
        return Err(error("E_ELF64_ENTRY", "entry offset is outside .text"));
    }
    let rodata_offset = add(TEXT_OFFSET, input.text.len() as u64, "rodata offset")?;
    let rx_file_end = add(rodata_offset, input.rodata.len() as u64, "RX segment size")?;
    let data_offset = align_up(rx_file_end, 0x1000)?;
    let data_file_end = add(data_offset, input.data.len() as u64, "data end")?;
    let bss_address = add(IMAGE_BASE, data_file_end, "BSS address")?;
    let _rw_memory_end = add(data_file_end, input.bss_size, "RW memory size")?;

    let shstrtab = b"\0.text\0.rodata\0.data\0.bss\0.shstrtab\0";
    let shstrtab_offset = data_file_end;
    let section_header_offset = align_up(
        add(shstrtab_offset, shstrtab.len() as u64, "section table")?,
        8,
    )?;
    let file_size = add(
        section_header_offset,
        u64::from(SECTION_COUNT) * SECTION_HEADER_SIZE,
        "file size",
    )?;
    let file_len = usize::try_from(file_size)
        .map_err(|_| error("E_ELF64_SIZE", "ELF64 file exceeds host addressability"))?;
    if file_len > MAX_IMAGE_SIZE {
        return Err(error(
            "E_ELF64_SIZE",
            "ELF64 file exceeds bounded image size",
        ));
    }
    let mut bytes = vec![0u8; file_len];

    // ELF identification and header.
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    put_u16(&mut bytes, 16, 2)?; // ET_EXEC
    put_u16(&mut bytes, 18, 62)?; // EM_X86_64
    put_u32(&mut bytes, 20, 1)?;
    let entry = add(IMAGE_BASE + TEXT_OFFSET, input.entry_offset, "entry")?;
    put_u64(&mut bytes, 24, entry)?;
    put_u64(&mut bytes, 32, ELF_HEADER_SIZE)?;
    put_u64(&mut bytes, 40, section_header_offset)?;
    put_u16(&mut bytes, 52, ELF_HEADER_SIZE as u16)?;
    put_u16(&mut bytes, 54, PROGRAM_HEADER_SIZE as u16)?;
    put_u16(&mut bytes, 56, PROGRAM_HEADER_COUNT)?;
    put_u16(&mut bytes, 58, SECTION_HEADER_SIZE as u16)?;
    put_u16(&mut bytes, 60, SECTION_COUNT)?;
    put_u16(&mut bytes, 62, 5)?;

    // RX PT_LOAD includes headers and file-backed text/rodata.
    write_program_header(
        &mut bytes,
        ELF_HEADER_SIZE,
        1,
        5,
        0,
        IMAGE_BASE,
        rx_file_end,
        rx_file_end,
        0x1000,
    )?;
    // RW PT_LOAD contains data and BSS.
    write_program_header(
        &mut bytes,
        ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE,
        1,
        6,
        data_offset,
        IMAGE_BASE + data_offset,
        input.data.len() as u64,
        input.data.len() as u64 + input.bss_size,
        0x1000,
    )?;

    copy_at(&mut bytes, TEXT_OFFSET, &input.text)?;
    copy_at(&mut bytes, rodata_offset, &input.rodata)?;
    copy_at(&mut bytes, data_offset, &input.data)?;
    copy_at(&mut bytes, shstrtab_offset, shstrtab)?;

    let text_address = IMAGE_BASE + TEXT_OFFSET;
    let rodata_address = IMAGE_BASE + rodata_offset;
    let data_address = IMAGE_BASE + data_offset;
    write_section_header(
        &mut bytes,
        section_header_offset + SECTION_HEADER_SIZE,
        1,
        1,
        0x6,
        text_address,
        TEXT_OFFSET,
        input.text.len() as u64,
        0,
        0,
        16,
        0,
    )?;
    write_section_header(
        &mut bytes,
        section_header_offset + SECTION_HEADER_SIZE * 2,
        7,
        1,
        0x2,
        rodata_address,
        rodata_offset,
        input.rodata.len() as u64,
        0,
        0,
        1,
        0,
    )?;
    write_section_header(
        &mut bytes,
        section_header_offset + SECTION_HEADER_SIZE * 3,
        15,
        1,
        0x3,
        data_address,
        data_offset,
        input.data.len() as u64,
        0,
        0,
        8,
        0,
    )?;
    write_section_header(
        &mut bytes,
        section_header_offset + SECTION_HEADER_SIZE * 4,
        21,
        8,
        0x3,
        bss_address,
        data_file_end,
        input.bss_size,
        0,
        0,
        8,
        0,
    )?;
    write_section_header(
        &mut bytes,
        section_header_offset + SECTION_HEADER_SIZE * 5,
        26,
        3,
        0,
        0,
        shstrtab_offset,
        shstrtab.len() as u64,
        0,
        0,
        1,
        0,
    )?;

    Ok(Elf64Executable {
        bytes,
        layout: Elf64Layout {
            entry,
            text_address,
            rodata_address,
            data_address,
            bss_address,
            file_size,
        },
    })
}

pub fn inspect(bytes: &[u8]) -> Result<Elf64Info, Elf64Error> {
    if bytes.len() < ELF_HEADER_SIZE as usize || &bytes[..4] != b"\x7fELF" {
        return Err(error("E_ELF64_HEADER", "truncated or invalid ELF magic"));
    }
    if bytes[4] != 2 || bytes[5] != 1 || bytes[6] != 1 {
        return Err(error(
            "E_ELF64_IDENT",
            "expected ELFCLASS64 little-endian version 1",
        ));
    }
    let object_type = get_u16(bytes, 16)?;
    let machine = get_u16(bytes, 18)?;
    if machine != 62 {
        return Err(error("E_ELF64_MACHINE", "expected EM_X86_64"));
    }
    let entry = get_u64(bytes, 24)?;
    let phoff = get_u64(bytes, 32)?;
    let shoff = get_u64(bytes, 40)?;
    let phentsize = get_u16(bytes, 54)?;
    let phnum = get_u16(bytes, 56)?;
    let shentsize = get_u16(bytes, 58)?;
    let shnum = get_u16(bytes, 60)?;
    if phnum != 0 && phentsize != PROGRAM_HEADER_SIZE as u16 {
        return Err(error(
            "E_ELF64_PROGRAM_HEADERS",
            "invalid program-header size",
        ));
    }
    if shnum != 0 && shentsize != SECTION_HEADER_SIZE as u16 {
        return Err(error(
            "E_ELF64_SECTION_HEADERS",
            "invalid section-header size",
        ));
    }
    checked_table(
        bytes,
        phoff,
        u64::from(phentsize),
        u64::from(phnum),
        "program headers",
    )?;
    checked_table(
        bytes,
        shoff,
        u64::from(shentsize),
        u64::from(shnum),
        "section headers",
    )?;
    let mut has_interp = false;
    let mut has_dynamic = false;
    for index in 0..phnum {
        let offset = phoff + u64::from(index) * u64::from(phentsize);
        let kind = get_u32(bytes, offset)?;
        has_interp |= kind == 3;
        has_dynamic |= kind == 2;
        let file_offset = get_u64(bytes, offset + 8)?;
        let file_size = get_u64(bytes, offset + 32)?;
        checked_range(bytes, file_offset, file_size, "segment")?;
    }
    Ok(Elf64Info {
        object_type,
        machine,
        entry,
        program_headers: phnum,
        section_headers: shnum,
        has_interp,
        has_dynamic,
    })
}

#[allow(clippy::too_many_arguments)]
fn write_program_header(
    bytes: &mut [u8],
    offset: u64,
    kind: u32,
    flags: u32,
    file_offset: u64,
    virtual_address: u64,
    file_size: u64,
    memory_size: u64,
    align: u64,
) -> Result<(), Elf64Error> {
    if file_size > memory_size {
        return Err(error("E_ELF64_SEGMENT", "segment filesz exceeds memsz"));
    }
    put_u32(bytes, offset, kind)?;
    put_u32(bytes, offset + 4, flags)?;
    put_u64(bytes, offset + 8, file_offset)?;
    put_u64(bytes, offset + 16, virtual_address)?;
    put_u64(bytes, offset + 24, virtual_address)?;
    put_u64(bytes, offset + 32, file_size)?;
    put_u64(bytes, offset + 40, memory_size)?;
    put_u64(bytes, offset + 48, align)
}

#[allow(clippy::too_many_arguments)]
fn write_section_header(
    bytes: &mut [u8],
    offset: u64,
    name: u32,
    kind: u32,
    flags: u64,
    address: u64,
    file_offset: u64,
    size: u64,
    link: u32,
    info: u32,
    align: u64,
    entry_size: u64,
) -> Result<(), Elf64Error> {
    put_u32(bytes, offset, name)?;
    put_u32(bytes, offset + 4, kind)?;
    put_u64(bytes, offset + 8, flags)?;
    put_u64(bytes, offset + 16, address)?;
    put_u64(bytes, offset + 24, file_offset)?;
    put_u64(bytes, offset + 32, size)?;
    put_u32(bytes, offset + 40, link)?;
    put_u32(bytes, offset + 44, info)?;
    put_u64(bytes, offset + 48, align)?;
    put_u64(bytes, offset + 56, entry_size)
}

fn add(left: u64, right: u64, what: &str) -> Result<u64, Elf64Error> {
    left.checked_add(right)
        .ok_or_else(|| error("E_ELF64_OVERFLOW", &format!("{what} overflows u64")))
}

fn align_up(value: u64, align: u64) -> Result<u64, Elf64Error> {
    let mask = align
        .checked_sub(1)
        .ok_or_else(|| error("E_ELF64_ALIGNMENT", "zero alignment"))?;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or_else(|| error("E_ELF64_OVERFLOW", "alignment overflows u64"))
}

fn checked_table(
    bytes: &[u8],
    offset: u64,
    entry_size: u64,
    count: u64,
    what: &str,
) -> Result<(), Elf64Error> {
    let size = entry_size
        .checked_mul(count)
        .ok_or_else(|| error("E_ELF64_OVERFLOW", &format!("{what} size overflows")))?;
    checked_range(bytes, offset, size, what)
}

fn checked_range(bytes: &[u8], offset: u64, size: u64, what: &str) -> Result<(), Elf64Error> {
    let end = offset
        .checked_add(size)
        .ok_or_else(|| error("E_ELF64_OVERFLOW", &format!("{what} range overflows")))?;
    if end > bytes.len() as u64 {
        Err(error(
            "E_ELF64_RANGE",
            &format!("{what} exceeds file bounds"),
        ))
    } else {
        Ok(())
    }
}

fn copy_at(bytes: &mut [u8], offset: u64, source: &[u8]) -> Result<(), Elf64Error> {
    checked_range(bytes, offset, source.len() as u64, "write")?;
    let start = offset as usize;
    bytes[start..start + source.len()].copy_from_slice(source);
    Ok(())
}

fn put_u16(bytes: &mut [u8], offset: u64, value: u16) -> Result<(), Elf64Error> {
    copy_at(bytes, offset, &value.to_le_bytes())
}

fn put_u32(bytes: &mut [u8], offset: u64, value: u32) -> Result<(), Elf64Error> {
    copy_at(bytes, offset, &value.to_le_bytes())
}

fn put_u64(bytes: &mut [u8], offset: u64, value: u64) -> Result<(), Elf64Error> {
    copy_at(bytes, offset, &value.to_le_bytes())
}

fn get_u16(bytes: &[u8], offset: u64) -> Result<u16, Elf64Error> {
    checked_range(bytes, offset, 2, "u16")?;
    let offset = offset as usize;
    Ok(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]))
}

fn get_u32(bytes: &[u8], offset: u64) -> Result<u32, Elf64Error> {
    checked_range(bytes, offset, 4, "u32")?;
    let offset = offset as usize;
    Ok(u32::from_le_bytes(
        bytes[offset..offset + 4].try_into().unwrap(),
    ))
}

fn get_u64(bytes: &[u8], offset: u64) -> Result<u64, Elf64Error> {
    checked_range(bytes, offset, 8, "u64")?;
    let offset = offset as usize;
    Ok(u64::from_le_bytes(
        bytes[offset..offset + 8].try_into().unwrap(),
    ))
}

fn error(code: &str, message: &str) -> Elf64Error {
    Elf64Error(format!("{code}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_canonical_static_elf64_without_dynamic_runtime() {
        let executable = write_static_executable(&ExecutableInput {
            text: vec![0xb8, 60, 0, 0, 0, 0x31, 0xff, 0x0f, 0x05],
            rodata: b"DBYTE64".to_vec(),
            data: vec![1, 2, 3, 4],
            bss_size: 4096,
            entry_offset: 0,
        })
        .unwrap();
        let info = inspect(&executable.bytes).unwrap();
        assert_eq!(info.object_type, 2);
        assert_eq!(info.machine, 62);
        assert_eq!(info.entry, executable.layout.entry);
        assert_eq!(info.program_headers, 2);
        assert_eq!(info.section_headers, 6);
        assert!(!info.has_interp);
        assert!(!info.has_dynamic);
    }

    #[test]
    fn writes_compact_user_elf64_for_dbfs1() {
        let executable = write_user_executable(&ExecutableInput {
            text: vec![0x90, 0xc3],
            rodata: b"USER".to_vec(),
            data: vec![1, 2, 3],
            bss_size: 64,
            entry_offset: 0,
        })
        .unwrap();
        let info = inspect(&executable.bytes).unwrap();
        assert_eq!(info.entry, 0x0040_00b0);
        assert_eq!(info.program_headers, 2);
        assert_eq!(info.section_headers, 0);
        assert!(executable.bytes.len() <= 4096);
        for index in 0..2u64 {
            let header = ELF_HEADER_SIZE + index * PROGRAM_HEADER_SIZE;
            assert_eq!(get_u32(&executable.bytes, header).unwrap(), 1);
            let flags = get_u32(&executable.bytes, header + 4).unwrap();
            let file_offset = get_u64(&executable.bytes, header + 8).unwrap();
            let virtual_address = get_u64(&executable.bytes, header + 16).unwrap();
            let alignment = get_u64(&executable.bytes, header + 48).unwrap();
            assert_eq!(alignment, 4096);
            assert_eq!(file_offset % alignment, virtual_address % alignment);
            assert_eq!(flags, if index == 0 { 5 } else { 6 });
        }
    }

    #[test]
    fn rejects_bad_entry_and_malformed_ranges_transactionally() {
        let error = write_static_executable(&ExecutableInput {
            text: vec![0xc3],
            rodata: vec![],
            data: vec![],
            bss_size: 0,
            entry_offset: 1,
        })
        .unwrap_err();
        assert!(error.0.contains("E_ELF64_ENTRY"));

        let mut executable = write_static_executable(&ExecutableInput {
            text: vec![0xc3],
            rodata: vec![],
            data: vec![],
            bss_size: 0,
            entry_offset: 0,
        })
        .unwrap()
        .bytes;
        executable[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(inspect(&executable)
            .unwrap_err()
            .0
            .contains("E_ELF64_OVERFLOW"));
    }
}
