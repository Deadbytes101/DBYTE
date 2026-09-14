//! Deterministic Multiboot2 wrapper for a linked DBYTE64 bare-metal payload.
//!
//! The wrapper is final-image machinery.  The embedded transition code is not
//! represented as an i386 object and never leaks i386 relocation semantics into
//! DBYTE64 ET_REL files.

const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;
const PROGRAM_HEADER_COUNT: usize = 4;
pub(crate) const IMAGE_BASE: u32 = 0x0010_0000;
const MULTIBOOT_HEADER_OFFSET: usize = 0x0200;
const BOOTSTRAP_OFFSET: usize = 0x1000;
const GDT_OFFSET: usize = 0x2000;
const GDTR_OFFSET: usize = 0x2018;
const PML4_OFFSET: usize = 0x3000;
const PDPT_OFFSET: usize = 0x4000;
const PAGE_DIRECTORY_OFFSET: usize = 0x5000;
const PAGE_DIRECTORY_COUNT: usize = 4;
const PAGE_DIRECTORY_STRIDE: usize = 0x1000;
const SAVED_MAGIC_OFFSET: usize = 0x9000;
const SAVED_INFO_OFFSET: usize = 0x9004;
const WRAPPER_RW_END: usize = 0x30000;
const STACK_TOP: u32 = IMAGE_BASE + WRAPPER_RW_END as u32;
const PAYLOAD_OFFSET: usize = 0x100000;
const PAYLOAD_ADDRESS: u32 = IMAGE_BASE + PAYLOAD_OFFSET as u32;
pub const DBYTE_TEXT_ADDRESS: u64 = 0x0020_0035;
const MAX_IMAGE_SIZE: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multiboot2Image {
    pub elf: Vec<u8>,
    pub entry: u64,
    pub payload_entry: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Multiboot2Error(pub String);

impl std::fmt::Display for Multiboot2Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Multiboot2Error {}

pub fn wrap_payload(payload: &[u8], entry_offset: u64) -> Result<Multiboot2Image, Multiboot2Error> {
    wrap_sections(payload, &[], &[], 0, entry_offset)
}

pub fn wrap_sections(
    text: &[u8],
    rodata: &[u8],
    data: &[u8],
    bss_size: u64,
    entry_offset: u64,
) -> Result<Multiboot2Image, Multiboot2Error> {
    if text.is_empty() {
        return Err(error("E_MULTIBOOT2_PAYLOAD", "payload must not be empty"));
    }
    let entry_offset = usize::try_from(entry_offset)
        .map_err(|_| error("E_MULTIBOOT2_ENTRY", "payload entry exceeds host range"))?;
    if entry_offset >= text.len() {
        return Err(error(
            "E_MULTIBOOT2_ENTRY",
            "payload entry lies outside payload text",
        ));
    }
    let provisional_entry = build_long_mode_entry(PAYLOAD_ADDRESS)?;
    if PAYLOAD_ADDRESS as u64 + provisional_entry.len() as u64 != DBYTE_TEXT_ADDRESS {
        return Err(error(
            "E_MULTIBOOT2_ENTRY_LAYOUT",
            "long-mode entry layout drifted from linker contract",
        ));
    }
    let dbyte_entry = PAYLOAD_ADDRESS
        .checked_add(
            u32::try_from(provisional_entry.len())
                .map_err(|_| error("E_MULTIBOOT2_ENTRY", "long-mode entry exceeds u32"))?,
        )
        .and_then(|address| address.checked_add(entry_offset as u32))
        .ok_or_else(|| error("E_MULTIBOOT2_ENTRY", "payload entry overflows u32"))?;
    let long_entry = build_long_mode_entry(dbyte_entry)?;
    let rx_size = long_entry
        .len()
        .checked_add(text.len())
        .and_then(|size| size.checked_add(rodata.len()))
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "payload size overflows"))?;
    let data_relative_to_text = align_up(
        0x1000usize
            .checked_add(text.len())
            .and_then(|size| size.checked_add(rodata.len()))
            .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "payload RX size overflows"))?,
        0x1000,
    )?
    .checked_sub(0x1000)
    .ok_or_else(|| error("E_MULTIBOOT2_LAYOUT", "data offset underflows"))?;
    let data_payload_offset = long_entry
        .len()
        .checked_add(data_relative_to_text)
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "payload data offset overflows"))?;
    if data_payload_offset < rx_size {
        return Err(error(
            "E_MULTIBOOT2_LAYOUT",
            "payload data overlaps RX content",
        ));
    }
    let payload_size = data_payload_offset
        .checked_add(data.len())
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "payload data size overflows"))?;
    let file_size = PAYLOAD_OFFSET
        .checked_add(payload_size)
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "image size overflows"))?;
    if file_size > MAX_IMAGE_SIZE {
        return Err(error("E_MULTIBOOT2_SIZE", "image exceeds 32 MiB bound"));
    }
    let load_end = IMAGE_BASE
        .checked_add(u32::try_from(file_size).map_err(|_| {
            error(
                "E_MULTIBOOT2_SIZE",
                "image load range exceeds 32-bit address space",
            )
        })?)
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "image load range overflows"))?;
    let bss_end = load_end
        .checked_add(
            u32::try_from(bss_size).map_err(|_| error("E_MULTIBOOT2_SIZE", "BSS exceeds u32"))?,
        )
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "BSS load range overflows"))?;

    let mut output = vec![0u8; file_size];
    write_elf_header(
        &mut output,
        file_size,
        rx_size,
        data_payload_offset,
        data.len(),
        bss_size,
    )?;
    write_multiboot_header(&mut output, load_end, bss_end)?;
    write_bootstrap(&mut output, PAYLOAD_ADDRESS)?;
    write_transition_data(&mut output)?;
    output[PAYLOAD_OFFSET..PAYLOAD_OFFSET + long_entry.len()].copy_from_slice(&long_entry);
    let text_start = PAYLOAD_OFFSET + long_entry.len();
    output[text_start..text_start + text.len()].copy_from_slice(text);
    let rodata_start = text_start + text.len();
    output[rodata_start..rodata_start + rodata.len()].copy_from_slice(rodata);
    let data_start = PAYLOAD_OFFSET + data_payload_offset;
    output[data_start..data_start + data.len()].copy_from_slice(data);

    Ok(Multiboot2Image {
        elf: output,
        entry: u64::from(IMAGE_BASE + BOOTSTRAP_OFFSET as u32),
        payload_entry: u64::from(PAYLOAD_ADDRESS),
    })
}

fn build_long_mode_entry(dbyte_entry: u32) -> Result<Vec<u8>, Multiboot2Error> {
    let mut bytes = Vec::new();
    // Reload flat data selectors.
    bytes.extend_from_slice(&[0x66, 0xb8, 0x10, 0x00]);
    bytes.extend_from_slice(&[0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xd0, 0x8e, 0xe0, 0x8e, 0xe8]);
    // Static stack; the call boundary is 16-byte aligned.
    bytes.extend_from_slice(&[0x48, 0xbc]);
    bytes.extend_from_slice(&u64::from(STACK_TOP).to_le_bytes());
    bytes.extend_from_slice(&[0x48, 0x83, 0xe4, 0xf0]);
    // Existing two-argument entry ABI uses DBYTE64 RAX/RDX slots.
    bytes.push(0xa1);
    bytes.extend_from_slice(&u64::from(IMAGE_BASE + SAVED_MAGIC_OFFSET as u32).to_le_bytes());
    bytes.extend_from_slice(&[0x8b, 0x14, 0x25]);
    bytes.extend_from_slice(&(IMAGE_BASE + SAVED_INFO_OFFSET as u32).to_le_bytes());
    let call_offset = bytes.len();
    bytes.push(0xe8);
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&[0xfa, 0xf4, 0xeb, 0xfd]);
    let next = PAYLOAD_ADDRESS as i64 + call_offset as i64 + 5;
    let displacement = i64::from(dbyte_entry) - next;
    let displacement = i32::try_from(displacement).map_err(|_| {
        error(
            "E_MULTIBOOT2_ENTRY",
            "DBYTE64 call displacement exceeds i32",
        )
    })?;
    bytes[call_offset + 1..call_offset + 5].copy_from_slice(&displacement.to_le_bytes());
    Ok(bytes)
}

fn write_elf_header(
    output: &mut [u8],
    file_size: usize,
    payload_rx_size: usize,
    payload_data_offset: usize,
    payload_data_size: usize,
    payload_bss_size: u64,
) -> Result<(), Multiboot2Error> {
    output[..4].copy_from_slice(b"\x7fELF");
    output[4] = 2;
    output[5] = 1;
    output[6] = 1;
    put_u16(output, 16, 2)?;
    put_u16(output, 18, 62)?;
    put_u32(output, 20, 1)?;
    put_u64(output, 24, u64::from(IMAGE_BASE + BOOTSTRAP_OFFSET as u32))?;
    put_u64(output, 32, ELF_HEADER_SIZE as u64)?;
    put_u16(output, 52, ELF_HEADER_SIZE as u16)?;
    put_u16(output, 54, PROGRAM_HEADER_SIZE as u16)?;
    put_u16(output, 56, PROGRAM_HEADER_COUNT as u16)?;
    write_program_header(output, 0, 0, IMAGE_BASE, 0x3000, 0x3000, 5)?;
    write_program_header(
        output,
        1,
        0x3000,
        IMAGE_BASE + 0x3000,
        WRAPPER_RW_END - 0x3000,
        WRAPPER_RW_END - 0x3000,
        6,
    )?;
    write_program_header(
        output,
        2,
        PAYLOAD_OFFSET,
        PAYLOAD_ADDRESS,
        payload_rx_size,
        payload_rx_size,
        5,
    )?;
    let data_file_offset = PAYLOAD_OFFSET
        .checked_add(payload_data_offset)
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "data file offset overflows"))?;
    let data_address = PAYLOAD_ADDRESS
        .checked_add(
            u32::try_from(payload_data_offset)
                .map_err(|_| error("E_MULTIBOOT2_SIZE", "data address exceeds u32"))?,
        )
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "data address overflows"))?;
    let data_memory_size = (payload_data_size as u64)
        .checked_add(payload_bss_size)
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "data memory size overflows"))?;
    write_program_header(
        output,
        3,
        data_file_offset,
        data_address,
        payload_data_size,
        usize::try_from(data_memory_size)
            .map_err(|_| error("E_MULTIBOOT2_SIZE", "data memory size exceeds host"))?,
        6,
    )?;
    if file_size < data_file_offset + payload_data_size {
        return Err(error("E_MULTIBOOT2_SIZE", "ELF segments exceed image"));
    }
    Ok(())
}

fn write_program_header(
    output: &mut [u8],
    index: usize,
    file_offset: usize,
    address: u32,
    file_size: usize,
    memory_size: usize,
    flags: u32,
) -> Result<(), Multiboot2Error> {
    if index >= PROGRAM_HEADER_COUNT {
        return Err(error(
            "E_MULTIBOOT2_HEADER",
            "program-header index exceeds bound",
        ));
    }
    let offset = ELF_HEADER_SIZE + index * PROGRAM_HEADER_SIZE;
    put_u32(output, offset, 1)?;
    put_u32(output, offset + 4, flags)?;
    put_u64(output, offset + 8, file_offset as u64)?;
    put_u64(output, offset + 16, u64::from(address))?;
    put_u64(output, offset + 24, u64::from(address))?;
    put_u64(output, offset + 32, file_size as u64)?;
    put_u64(output, offset + 40, memory_size as u64)?;
    put_u64(output, offset + 48, 1)
}

fn write_multiboot_header(
    output: &mut [u8],
    load_end: u32,
    bss_end: u32,
) -> Result<(), Multiboot2Error> {
    const MAGIC: u32 = 0xe852_50d6;
    const ARCH_I386: u32 = 0;
    const LENGTH: u32 = 64;
    let offset = MULTIBOOT_HEADER_OFFSET;
    put_u32(output, offset, MAGIC)?;
    put_u32(output, offset + 4, ARCH_I386)?;
    put_u32(output, offset + 8, LENGTH)?;
    put_u32(
        output,
        offset + 12,
        0u32.wrapping_sub(MAGIC)
            .wrapping_sub(ARCH_I386)
            .wrapping_sub(LENGTH),
    )?;
    // Address tag: load the deterministic flat image at 1 MiB.
    put_u16(output, offset + 16, 2)?;
    put_u16(output, offset + 18, 0)?;
    put_u32(output, offset + 20, 24)?;
    put_u32(output, offset + 24, IMAGE_BASE + offset as u32)?;
    put_u32(output, offset + 28, IMAGE_BASE)?;
    put_u32(output, offset + 32, load_end)?;
    put_u32(output, offset + 36, bss_end)?;
    // Entry-address tag followed by four bytes of required tag alignment.
    put_u16(output, offset + 40, 3)?;
    put_u16(output, offset + 42, 0)?;
    put_u32(output, offset + 44, 12)?;
    put_u32(output, offset + 48, IMAGE_BASE + BOOTSTRAP_OFFSET as u32)?;
    put_u16(output, offset + 56, 0)?;
    put_u16(output, offset + 58, 0)?;
    put_u32(output, offset + 60, 8)
}

fn write_bootstrap(output: &mut [u8], long_entry: u32) -> Result<(), Multiboot2Error> {
    let mut code = Vec::new();
    code.push(0xfa); // cli
    code.push(0xa3); // mov [saved_magic], eax
    code.extend_from_slice(&(IMAGE_BASE + SAVED_MAGIC_OFFSET as u32).to_le_bytes());
    code.extend_from_slice(&[0x89, 0xd8]); // mov eax, ebx
    code.push(0xa3); // mov [saved_info], eax
    code.extend_from_slice(&(IMAGE_BASE + SAVED_INFO_OFFSET as u32).to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x01, 0x15]); // lgdt [abs32]
    code.extend_from_slice(&(IMAGE_BASE + GDTR_OFFSET as u32).to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x20, 0xe0, 0x83, 0xc8, 0x20, 0x0f, 0x22, 0xe0]);
    code.push(0xb8);
    code.extend_from_slice(&(IMAGE_BASE + PML4_OFFSET as u32).to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x22, 0xd8]);
    code.push(0xb9);
    code.extend_from_slice(&0xc000_0080u32.to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x32, 0x0d]);
    code.extend_from_slice(&0x100u32.to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x30, 0x0f, 0x20, 0xc0, 0x0d]);
    code.extend_from_slice(&0x8000_0000u32.to_le_bytes());
    code.extend_from_slice(&[0x0f, 0x22, 0xc0, 0xea]);
    code.extend_from_slice(&long_entry.to_le_bytes());
    code.extend_from_slice(&0x08u16.to_le_bytes());
    let end = BOOTSTRAP_OFFSET + code.len();
    if end > GDT_OFFSET {
        return Err(error(
            "E_MULTIBOOT2_BOOTSTRAP",
            "transition stub exceeds bound",
        ));
    }
    output[BOOTSTRAP_OFFSET..end].copy_from_slice(&code);
    Ok(())
}

fn write_transition_data(output: &mut [u8]) -> Result<(), Multiboot2Error> {
    let gdt = [0u64, 0x00af_9a00_0000_ffff, 0x00cf_9200_0000_ffff];
    for (index, descriptor) in gdt.into_iter().enumerate() {
        put_u64(output, GDT_OFFSET + index * 8, descriptor)?;
    }
    put_u16(output, GDTR_OFFSET, 23)?;
    put_u32(output, GDTR_OFFSET + 2, IMAGE_BASE + GDT_OFFSET as u32)?;
    put_u64(
        output,
        PML4_OFFSET,
        u64::from(IMAGE_BASE + PDPT_OFFSET as u32) | 3,
    )?;
    for directory in 0..PAGE_DIRECTORY_COUNT {
        let directory_offset = PAGE_DIRECTORY_OFFSET
            .checked_add(directory * PAGE_DIRECTORY_STRIDE)
            .ok_or_else(|| error("E_MULTIBOOT2_LAYOUT", "page-directory offset overflows"))?;
        let directory_address =
            IMAGE_BASE
                .checked_add(u32::try_from(directory_offset).map_err(|_| {
                    error("E_MULTIBOOT2_LAYOUT", "page-directory address exceeds u32")
                })?)
                .ok_or_else(|| error("E_MULTIBOOT2_LAYOUT", "page-directory address overflows"))?;
        put_u64(
            output,
            PDPT_OFFSET + directory * 8,
            u64::from(directory_address) | 3,
        )?;
        for index in 0..512usize {
            let page = directory
                .checked_mul(512)
                .and_then(|base| base.checked_add(index))
                .ok_or_else(|| error("E_MULTIBOOT2_LAYOUT", "identity-map index overflows"))?;
            put_u64(
                output,
                directory_offset + index * 8,
                (page as u64 * 0x20_0000) | 0x83,
            )?;
        }
    }
    Ok(())
}

fn put_u16(output: &mut [u8], offset: usize, value: u16) -> Result<(), Multiboot2Error> {
    put(output, offset, &value.to_le_bytes())
}

fn align_up(value: usize, alignment: usize) -> Result<usize, Multiboot2Error> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(error(
            "E_MULTIBOOT2_ALIGN",
            "alignment must be a power of two",
        ));
    }
    value
        .checked_add(alignment - 1)
        .map(|aligned| aligned & !(alignment - 1))
        .ok_or_else(|| error("E_MULTIBOOT2_SIZE", "alignment overflows"))
}

fn put_u32(output: &mut [u8], offset: usize, value: u32) -> Result<(), Multiboot2Error> {
    put(output, offset, &value.to_le_bytes())
}
fn put_u64(output: &mut [u8], offset: usize, value: u64) -> Result<(), Multiboot2Error> {
    put(output, offset, &value.to_le_bytes())
}
fn put(output: &mut [u8], offset: usize, bytes: &[u8]) -> Result<(), Multiboot2Error> {
    let end = offset
        .checked_add(bytes.len())
        .ok_or_else(|| error("E_MULTIBOOT2_OVERFLOW", "write range overflows"))?;
    let target = output
        .get_mut(offset..end)
        .ok_or_else(|| error("E_MULTIBOOT2_RANGE", "write exceeds image"))?;
    target.copy_from_slice(bytes);
    Ok(())
}

fn error(code: &str, message: &str) -> Multiboot2Error {
    Multiboot2Error(format!("{code}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_has_valid_header_and_deterministic_page_tables() {
        let first = wrap_payload(&[0xc3], 0).unwrap();
        let second = wrap_payload(&[0xc3], 0).unwrap();
        assert_eq!(first, second);
        assert_eq!(&first.elf[..4], b"\x7fELF");
        assert_eq!(
            u16::from_le_bytes(first.elf[18..20].try_into().unwrap()),
            62
        );
        let header = &first.elf[MULTIBOOT_HEADER_OFFSET..MULTIBOOT_HEADER_OFFSET + 64];
        let checksum = header[..16]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .fold(0u32, u32::wrapping_add);
        assert_eq!(checksum, 0);
        assert_eq!(
            u64::from_le_bytes(
                first.elf[PAGE_DIRECTORY_OFFSET..PAGE_DIRECTORY_OFFSET + 8]
                    .try_into()
                    .unwrap()
            ),
            0x83
        );
        for directory in 0..PAGE_DIRECTORY_COUNT {
            let entry = PDPT_OFFSET + directory * 8;
            assert_eq!(
                u64::from_le_bytes(first.elf[entry..entry + 8].try_into().unwrap()),
                u64::from(
                    IMAGE_BASE + (PAGE_DIRECTORY_OFFSET + directory * PAGE_DIRECTORY_STRIDE) as u32
                ) | 3
            );
        }
        let last =
            PAGE_DIRECTORY_OFFSET + (PAGE_DIRECTORY_COUNT - 1) * PAGE_DIRECTORY_STRIDE + 511 * 8;
        assert_eq!(
            u64::from_le_bytes(first.elf[last..last + 8].try_into().unwrap()),
            0xffe0_0083
        );
        assert_eq!(first.entry, 0x0010_1000);
        assert_eq!(first.payload_entry, 0x0020_0000);
    }

    #[test]
    fn invalid_payload_is_transactional() {
        assert!(wrap_payload(&[], 0)
            .unwrap_err()
            .0
            .contains("E_MULTIBOOT2_PAYLOAD"));
        assert!(wrap_payload(&[0xc3], 1)
            .unwrap_err()
            .0
            .contains("E_MULTIBOOT2_ENTRY"));
    }

    #[test]
    fn payload_data_and_bss_use_a_distinct_writable_segment() {
        let image = wrap_sections(&[0xc3], b"read-only", &[1, 2, 3, 4], 32, 0).unwrap();
        assert_eq!(u16::from_le_bytes(image.elf[56..58].try_into().unwrap()), 4);
        let header = ELF_HEADER_SIZE + 3 * PROGRAM_HEADER_SIZE;
        assert_eq!(
            u32::from_le_bytes(image.elf[header + 4..header + 8].try_into().unwrap()),
            6
        );
        assert_eq!(
            u64::from_le_bytes(image.elf[header + 32..header + 40].try_into().unwrap()),
            4
        );
        assert_eq!(
            u64::from_le_bytes(image.elf[header + 40..header + 48].try_into().unwrap()),
            36
        );
    }
}
