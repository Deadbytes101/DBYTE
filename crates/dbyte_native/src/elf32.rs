//! Deterministic, bounded ELF32/i386 executable writer.
//!
//! The emitted file is directly Multiboot-v1 loadable.  No linker, assembler,
//! object library, or host compiler participates in this packaging step.

use crate::diagnostics::{Diagnostic, DiagnosticCategory};
use crate::ir::{IrProgram, StringId};
use crate::lower::{AbsoluteFixupTarget, LoweredProgram, NATIVE_ENTRY_FUNCTION};
use crate::object::{ObjectSection, SymbolBinding, SymbolKind};
use std::collections::BTreeSet;
use std::fmt::Write;

pub const IMAGE_BASE: u32 = 0x0010_0000;
pub const LOAD_FILE_OFFSET: u32 = 0x0000_1000;
pub const MULTIBOOT_MAGIC: u32 = 0x1bad_b002;
pub const MULTIBOOT_FLAGS: u32 = 0x2;
pub const MULTIBOOT_CHECKSUM: u32 =
    0u32.wrapping_sub(MULTIBOOT_MAGIC.wrapping_add(MULTIBOOT_FLAGS));
pub const BOOTSTRAP_STACK_SIZE: u32 = 16 * 1024;
pub const MAX_ELF_SIZE: usize = 16 * 1024 * 1024;
pub const MAX_LOAD_MEMORY_SIZE: u32 = 8 * 1024 * 1024;
pub const USER_IMAGE_BASE: u32 = 0x0040_0000;
pub const USER_IMAGE_END: u32 = 0x0080_0000;
pub const MAX_USER_ELF_SIZE: usize = 4096;

const ELF_HEADER_SIZE: u32 = 52;
const PROGRAM_HEADER_SIZE: u32 = 32;
const SECTION_HEADER_SIZE: u32 = 40;
const SYMBOL_SIZE: u32 = 16;
const MULTIBOOT_HEADER_SIZE: u32 = 12;

const SECTION_MULTIBOOT: u16 = 1;
const SECTION_TEXT: u16 = 2;
const SECTION_RODATA: u16 = 3;
const SECTION_DATA: u16 = 4;
const SECTION_BSS: u16 = 5;
const SECTION_SYMTAB: u16 = 6;
const SECTION_STRTAB: u16 = 7;
const SECTION_SHSTRTAB: u16 = 8;
const SECTION_COUNT: u16 = 9;

const SHT_NULL: u32 = 0;
const SHT_PROGBITS: u32 = 1;
const SHT_SYMTAB: u32 = 2;
const SHT_STRTAB: u32 = 3;
const SHT_NOBITS: u32 = 8;

const SHF_WRITE: u32 = 0x1;
const SHF_ALLOC: u32 = 0x2;
const SHF_EXECINSTR: u32 = 0x4;

const PT_LOAD: u32 = 1;
const PF_X: u32 = 0x1;
const PF_W: u32 = 0x2;
const PF_R: u32 = 0x4;

const STB_GLOBAL: u8 = 1;
const STB_LOCAL: u8 = 0;
const STT_OBJECT: u8 = 1;
const STT_FUNC: u8 = 2;
const STT_SECTION: u8 = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElfLayout {
    pub image_base: u32,
    pub load_file_offset: u32,
    pub load_file_size: u32,
    pub load_memory_size: u32,
    pub entry: u32,
    pub multiboot_offset: u32,
    pub multiboot_address: u32,
    pub text_offset: u32,
    pub text_address: u32,
    pub text_size: u32,
    pub rodata_offset: u32,
    pub rodata_address: u32,
    pub rodata_size: u32,
    pub data_offset: u32,
    pub data_address: u32,
    pub data_size: u32,
    pub bss_offset: u32,
    pub bss_address: u32,
    pub bss_size: u32,
    pub stack_bottom: u32,
    pub stack_top: u32,
    pub symtab_offset: u32,
    pub strtab_offset: u32,
    pub shstrtab_offset: u32,
    pub section_header_offset: u32,
    pub file_size: u32,
    pub string_addresses: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElfArtifact {
    /// Complete ELF32 executable.
    pub elf: Vec<u8>,
    /// Exact file-backed bytes of the sole PT_LOAD segment.
    pub bin: Vec<u8>,
    /// Bootstrap and DByte functions after absolute address patching.
    pub machine_code: Vec<u8>,
    pub layout: ElfLayout,
    pub dump: String,
}

/// A fully resolved, section-relative image consumed by the final ELF writer.
/// Relocations must already have been applied by the DByte linker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedExecutableImage {
    pub text: Vec<u8>,
    pub rodata: Vec<u8>,
    pub data: Vec<u8>,
    pub bss_size: u32,
    pub text_alignment: u32,
    pub rodata_alignment: u32,
    pub data_alignment: u32,
    pub bss_alignment: u32,
    pub entry_offset: u32,
    pub stack_bottom_offset: u32,
    pub stack_top_offset: u32,
    pub symbols: Vec<ExecutableSymbol>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutableSymbol {
    pub name: String,
    pub binding: SymbolBinding,
    pub kind: SymbolKind,
    pub section: ObjectSection,
    pub offset: u32,
    pub size: u32,
}

/// Writes the compact, two-segment ELF consumed by the Ring3 loader. Unlike
/// the kernel writer this deliberately has no Multiboot header, section table,
/// symbol table, or binary-image form.
pub fn write_user_executable(image: &LinkedExecutableImage) -> Result<ElfArtifact, Diagnostic> {
    validate_linked_image_contract(image, false)?;
    let text_size = usize_u32(image.text.len(), "user text size")?;
    let rodata_start = align_up(text_size, image.rodata_alignment.max(1))?;
    let rx_size = checked_add(
        rodata_start,
        usize_u32(image.rodata.len(), "user rodata size")?,
        "user RX size",
    )?;
    let data_address = align_up(checked_add(USER_IMAGE_BASE, rx_size, "user RX end")?, 4096)?;
    let data_size = usize_u32(image.data.len(), "user data size")?;
    let bss_relative = align_up(data_size, image.bss_alignment.max(1))?;
    let rw_memory_size = checked_add(bss_relative, image.bss_size, "user RW memory size")?;
    let image_end = checked_add(data_address, rw_memory_size, "user image end")?;
    if image_end > USER_IMAGE_END {
        return Err(elf_diagnostic("user image exceeds 0x00400000..0x007fffff"));
    }
    if image.entry_offset >= text_size {
        return Err(elf_diagnostic("user entry is outside executable text"));
    }

    const USER_TEXT_OFFSET: u32 = ELF_HEADER_SIZE + 2 * PROGRAM_HEADER_SIZE;
    let data_offset = checked_add(USER_TEXT_OFFSET, rx_size, "user data file offset")?;
    let file_size = checked_add(data_offset, data_size, "user ELF size")?;
    if file_size as usize > MAX_USER_ELF_SIZE {
        return Err(elf_diagnostic(format!(
            "user ELF size {file_size} exceeds bound {MAX_USER_ELF_SIZE}"
        )));
    }
    let entry = checked_add(USER_IMAGE_BASE, image.entry_offset, "user entry")?;
    let mut elf = bounded_zeros(file_size as usize, "user ELF")?;
    elf[0..4].copy_from_slice(b"\x7fELF");
    elf[4] = 1;
    elf[5] = 1;
    elf[6] = 1;
    write_u16(&mut elf, 16, 2)?;
    write_u16(&mut elf, 18, 3)?;
    write_u32(&mut elf, 20, 1)?;
    write_u32(&mut elf, 24, entry)?;
    write_u32(&mut elf, 28, ELF_HEADER_SIZE)?;
    write_u32(&mut elf, 32, 0)?;
    write_u32(&mut elf, 36, 0)?;
    write_u16(&mut elf, 40, ELF_HEADER_SIZE as u16)?;
    write_u16(&mut elf, 42, PROGRAM_HEADER_SIZE as u16)?;
    write_u16(&mut elf, 44, 2)?;
    write_u16(&mut elf, 46, 0)?;
    write_u16(&mut elf, 48, 0)?;
    write_u16(&mut elf, 50, 0)?;

    write_user_program_header(
        &mut elf,
        ELF_HEADER_SIZE as usize,
        USER_TEXT_OFFSET,
        USER_IMAGE_BASE,
        rx_size,
        rx_size,
        PF_R | PF_X,
    )?;
    write_user_program_header(
        &mut elf,
        (ELF_HEADER_SIZE + PROGRAM_HEADER_SIZE) as usize,
        data_offset,
        data_address,
        data_size,
        rw_memory_size,
        PF_R | PF_W,
    )?;
    copy_bytes(&mut elf, USER_TEXT_OFFSET as usize, &image.text)?;
    copy_bytes(
        &mut elf,
        checked_add(USER_TEXT_OFFSET, rodata_start, "user rodata offset")? as usize,
        &image.rodata,
    )?;
    copy_bytes(&mut elf, data_offset as usize, &image.data)?;

    let layout = ElfLayout {
        image_base: USER_IMAGE_BASE,
        load_file_offset: USER_TEXT_OFFSET,
        load_file_size: checked_add(rx_size, data_size, "user file image size")?,
        load_memory_size: checked_add(
            data_address - USER_IMAGE_BASE,
            rw_memory_size,
            "user memory image size",
        )?,
        entry,
        multiboot_offset: 0,
        multiboot_address: 0,
        text_offset: USER_TEXT_OFFSET,
        text_address: USER_IMAGE_BASE,
        text_size,
        rodata_offset: USER_TEXT_OFFSET + rodata_start,
        rodata_address: USER_IMAGE_BASE + rodata_start,
        rodata_size: usize_u32(image.rodata.len(), "user rodata size")?,
        data_offset,
        data_address,
        data_size,
        bss_offset: data_offset + bss_relative,
        bss_address: data_address + bss_relative,
        bss_size: image.bss_size,
        stack_bottom: 0,
        stack_top: 0,
        symtab_offset: 0,
        strtab_offset: 0,
        shstrtab_offset: 0,
        section_header_offset: 0,
        file_size,
        string_addresses: Vec::new(),
    };
    Ok(ElfArtifact {
        elf,
        bin: Vec::new(),
        machine_code: image.text.clone(),
        layout,
        dump: format!(
            "user-elf32 entry=0x{entry:08x} rx=0x{USER_IMAGE_BASE:08x}+0x{rx_size:08x} rw=0x{data_address:08x}+0x{rw_memory_size:08x}\n"
        ),
    })
}

fn write_user_program_header(
    elf: &mut [u8],
    offset: usize,
    file_offset: u32,
    address: u32,
    file_size: u32,
    memory_size: u32,
    flags: u32,
) -> Result<(), Diagnostic> {
    write_u32(elf, offset, PT_LOAD)?;
    write_u32(elf, offset + 4, file_offset)?;
    write_u32(elf, offset + 8, address)?;
    write_u32(elf, offset + 12, address)?;
    write_u32(elf, offset + 16, file_size)?;
    write_u32(elf, offset + 20, memory_size)?;
    write_u32(elf, offset + 24, flags)?;
    write_u32(elf, offset + 28, 1)
}

#[derive(Clone, Debug)]
struct SectionHeader {
    name: u32,
    section_type: u32,
    flags: u32,
    address: u32,
    offset: u32,
    size: u32,
    link: u32,
    info: u32,
    address_align: u32,
    entry_size: u32,
}

#[derive(Clone, Debug)]
struct SymbolSpec {
    name: String,
    value: u32,
    size: u32,
    info: u8,
    section_index: u16,
}

#[derive(Clone, Debug)]
struct FileRange {
    name: &'static str,
    start: u32,
    end: u32,
}

pub fn write_executable(
    program: &IrProgram,
    lowered: &LoweredProgram,
) -> Result<ElfArtifact, Diagnostic> {
    validate_lowered(program, lowered)?;

    let multiboot_relative = 0u32;
    let text_relative = align_up(MULTIBOOT_HEADER_SIZE, 16)?;
    let text_size = usize_u32(lowered.machine_code.len(), "machine code size")?;
    let text_end_relative = checked_add(text_relative, text_size, "text end")?;
    let rodata_relative = align_up(text_end_relative, 16)?;

    let (rodata, string_offsets) = build_rodata(&program.strings)?;
    let rodata_size = usize_u32(rodata.len(), "rodata size")?;
    let rodata_end_relative = checked_add(rodata_relative, rodata_size, "rodata end")?;
    let data_relative = align_up(rodata_end_relative, 16)?;
    let data_size = 0u32;
    let data_end_relative = checked_add(data_relative, data_size, "data end")?;
    let bss_relative = align_up(data_end_relative, 16)?;
    let bss_size = BOOTSTRAP_STACK_SIZE;
    let load_memory_size = checked_add(bss_relative, bss_size, "load memory size")?;
    if load_memory_size > MAX_LOAD_MEMORY_SIZE {
        return Err(elf_diagnostic(format!(
            "ELF load image {} exceeds bound {}",
            load_memory_size, MAX_LOAD_MEMORY_SIZE
        )));
    }
    let load_file_size = data_end_relative;

    let multiboot_address = checked_add(IMAGE_BASE, multiboot_relative, "Multiboot address")?;
    let text_address = checked_add(IMAGE_BASE, text_relative, "text address")?;
    let rodata_address = checked_add(IMAGE_BASE, rodata_relative, "rodata address")?;
    let data_address = checked_add(IMAGE_BASE, data_relative, "data address")?;
    let bss_address = checked_add(IMAGE_BASE, bss_relative, "bss address")?;
    let stack_bottom = bss_address;
    let stack_top = checked_add(stack_bottom, BOOTSTRAP_STACK_SIZE, "stack top")?;
    let entry = checked_add(text_address, lowered.entry_offset, "entry address")?;

    let text_offset = checked_add(LOAD_FILE_OFFSET, text_relative, "text file offset")?;
    let rodata_offset = checked_add(LOAD_FILE_OFFSET, rodata_relative, "rodata file offset")?;
    let data_offset = checked_add(LOAD_FILE_OFFSET, data_relative, "data file offset")?;
    let bss_offset = checked_add(LOAD_FILE_OFFSET, bss_relative, "bss file offset")?;

    let mut string_addresses = Vec::with_capacity(string_offsets.len());
    for offset in &string_offsets {
        string_addresses.push(checked_add(rodata_address, *offset, "string address")?);
    }

    let mut machine_code = lowered.machine_code.clone();
    patch_absolute_fixups(&mut machine_code, lowered, stack_top, &string_addresses)?;

    let mut bin = bounded_zeros(load_file_size as usize, "PT_LOAD file image")?;
    write_u32(&mut bin, multiboot_relative as usize, MULTIBOOT_MAGIC)?;
    write_u32(&mut bin, multiboot_relative as usize + 4, MULTIBOOT_FLAGS)?;
    write_u32(
        &mut bin,
        multiboot_relative as usize + 8,
        MULTIBOOT_CHECKSUM,
    )?;
    copy_bytes(&mut bin, text_relative as usize, &machine_code)?;
    copy_bytes(&mut bin, rodata_relative as usize, &rodata)?;

    let bootstrap_size = lowered
        .functions
        .iter()
        .map(|symbol| symbol.offset)
        .min()
        .unwrap_or(text_size)
        .saturating_sub(lowered.entry_offset);
    let mut symbol_specs = Vec::with_capacity(lowered.functions.len() + 3);
    symbol_specs.push(SymbolSpec {
        name: "_start".to_owned(),
        value: entry,
        size: bootstrap_size,
        info: symbol_info(STB_GLOBAL, STT_FUNC),
        section_index: SECTION_TEXT,
    });
    for function in &lowered.functions {
        symbol_specs.push(SymbolSpec {
            name: function.name.clone(),
            value: checked_add(text_address, function.offset, "function symbol address")?,
            size: function.size,
            info: symbol_info(STB_GLOBAL, STT_FUNC),
            section_index: SECTION_TEXT,
        });
    }
    symbol_specs.push(SymbolSpec {
        name: "__dbyte_stack_bottom".to_owned(),
        value: stack_bottom,
        size: BOOTSTRAP_STACK_SIZE,
        info: symbol_info(STB_GLOBAL, STT_OBJECT),
        section_index: SECTION_BSS,
    });
    symbol_specs.push(SymbolSpec {
        name: "__dbyte_stack_top".to_owned(),
        value: stack_top,
        size: 0,
        info: symbol_info(STB_GLOBAL, STT_OBJECT),
        section_index: SECTION_BSS,
    });
    let (symtab, strtab) = build_symbol_tables(&symbol_specs)?;
    let (shstrtab, section_name_offsets) = build_section_name_table()?;

    let load_file_end = checked_add(LOAD_FILE_OFFSET, load_file_size, "load file end")?;
    let symtab_offset = align_up(load_file_end, 4)?;
    let symtab_size = usize_u32(symtab.len(), "symbol table size")?;
    let strtab_offset = checked_add(symtab_offset, symtab_size, "string table offset")?;
    let strtab_size = usize_u32(strtab.len(), "string table size")?;
    let shstrtab_offset = checked_add(strtab_offset, strtab_size, "section-name table offset")?;
    let shstrtab_size = usize_u32(shstrtab.len(), "section-name table size")?;
    let section_header_offset = align_up(
        checked_add(shstrtab_offset, shstrtab_size, "section-header offset")?,
        4,
    )?;
    let section_headers_size = checked_mul(
        u32::from(SECTION_COUNT),
        SECTION_HEADER_SIZE,
        "section-header table size",
    )?;
    let file_size = checked_add(section_header_offset, section_headers_size, "ELF file size")?;
    if file_size as usize > MAX_ELF_SIZE {
        return Err(elf_diagnostic(format!(
            "ELF file size {} exceeds bound {}",
            file_size, MAX_ELF_SIZE
        )));
    }

    let layout = ElfLayout {
        image_base: IMAGE_BASE,
        load_file_offset: LOAD_FILE_OFFSET,
        load_file_size,
        load_memory_size,
        entry,
        multiboot_offset: LOAD_FILE_OFFSET,
        multiboot_address,
        text_offset,
        text_address,
        text_size,
        rodata_offset,
        rodata_address,
        rodata_size,
        data_offset,
        data_address,
        data_size,
        bss_offset,
        bss_address,
        bss_size,
        stack_bottom,
        stack_top,
        symtab_offset,
        strtab_offset,
        shstrtab_offset,
        section_header_offset,
        file_size,
        string_addresses,
    };

    let section_headers = vec![
        SectionHeader {
            name: 0,
            section_type: SHT_NULL,
            flags: 0,
            address: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            address_align: 0,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_MULTIBOOT as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC,
            address: multiboot_address,
            offset: LOAD_FILE_OFFSET,
            size: MULTIBOOT_HEADER_SIZE,
            link: 0,
            info: 0,
            address_align: 4,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_TEXT as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC | SHF_EXECINSTR,
            address: text_address,
            offset: text_offset,
            size: text_size,
            link: 0,
            info: 0,
            address_align: 16,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_RODATA as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC,
            address: rodata_address,
            offset: rodata_offset,
            size: rodata_size,
            link: 0,
            info: 0,
            address_align: 16,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_DATA as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC | SHF_WRITE,
            address: data_address,
            offset: data_offset,
            size: data_size,
            link: 0,
            info: 0,
            address_align: 16,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_BSS as usize],
            section_type: SHT_NOBITS,
            flags: SHF_ALLOC | SHF_WRITE,
            address: bss_address,
            offset: bss_offset,
            size: bss_size,
            link: 0,
            info: 0,
            address_align: 16,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_SYMTAB as usize],
            section_type: SHT_SYMTAB,
            flags: 0,
            address: 0,
            offset: symtab_offset,
            size: symtab_size,
            link: u32::from(SECTION_STRTAB),
            info: 1,
            address_align: 4,
            entry_size: SYMBOL_SIZE,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_STRTAB as usize],
            section_type: SHT_STRTAB,
            flags: 0,
            address: 0,
            offset: strtab_offset,
            size: strtab_size,
            link: 0,
            info: 0,
            address_align: 1,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_SHSTRTAB as usize],
            section_type: SHT_STRTAB,
            flags: 0,
            address: 0,
            offset: shstrtab_offset,
            size: shstrtab_size,
            link: 0,
            info: 0,
            address_align: 1,
            entry_size: 0,
        },
    ];

    validate_layout(&layout, &section_headers)?;

    let mut elf = bounded_zeros(file_size as usize, "ELF file")?;
    write_elf_header(&mut elf, &layout)?;
    write_program_header(&mut elf, &layout)?;
    copy_bytes(&mut elf, LOAD_FILE_OFFSET as usize, &bin)?;
    copy_bytes(&mut elf, symtab_offset as usize, &symtab)?;
    copy_bytes(&mut elf, strtab_offset as usize, &strtab)?;
    copy_bytes(&mut elf, shstrtab_offset as usize, &shstrtab)?;
    write_section_headers(&mut elf, section_header_offset, &section_headers)?;

    validate_final_bytes(&elf, &layout)?;
    let dump = layout_dump(&layout, &section_headers, lowered);
    Ok(ElfArtifact {
        elf,
        bin,
        machine_code,
        layout,
        dump,
    })
}

/// Writes a final Multiboot-loadable ET_EXEC from an already linked image.
/// This is the only serialization seam used by the in-process static linker.
pub fn write_linked_executable(image: &LinkedExecutableImage) -> Result<ElfArtifact, Diagnostic> {
    validate_linked_image(image)?;

    let text_size = usize_u32(image.text.len(), "linked text size")?;
    let rodata_size = usize_u32(image.rodata.len(), "linked rodata size")?;
    let data_size = usize_u32(image.data.len(), "linked data size")?;
    let text_relative = align_up(MULTIBOOT_HEADER_SIZE, image.text_alignment.max(16))?;
    let rodata_relative = align_up(
        checked_add(text_relative, text_size, "linked text end")?,
        image.rodata_alignment.max(16),
    )?;
    let data_relative = align_up(
        checked_add(rodata_relative, rodata_size, "linked rodata end")?,
        image.data_alignment.max(16),
    )?;
    let data_end_relative = checked_add(data_relative, data_size, "linked data end")?;
    let bss_relative = align_up(data_end_relative, image.bss_alignment.max(16))?;
    let load_file_size = data_end_relative;
    let load_memory_size = checked_add(bss_relative, image.bss_size, "linked load memory size")?;
    if load_memory_size > MAX_LOAD_MEMORY_SIZE {
        return Err(elf_diagnostic(format!(
            "ELF load image {load_memory_size} exceeds bound {MAX_LOAD_MEMORY_SIZE}"
        )));
    }

    let multiboot_address = IMAGE_BASE;
    let text_address = checked_add(IMAGE_BASE, text_relative, "linked text address")?;
    let rodata_address = checked_add(IMAGE_BASE, rodata_relative, "linked rodata address")?;
    let data_address = checked_add(IMAGE_BASE, data_relative, "linked data address")?;
    let bss_address = checked_add(IMAGE_BASE, bss_relative, "linked bss address")?;
    let entry = checked_add(text_address, image.entry_offset, "linked entry address")?;
    let stack_bottom = checked_add(
        bss_address,
        image.stack_bottom_offset,
        "linked stack bottom",
    )?;
    let stack_top = checked_add(bss_address, image.stack_top_offset, "linked stack top")?;

    let text_offset = checked_add(LOAD_FILE_OFFSET, text_relative, "linked text file offset")?;
    let rodata_offset = checked_add(
        LOAD_FILE_OFFSET,
        rodata_relative,
        "linked rodata file offset",
    )?;
    let data_offset = checked_add(LOAD_FILE_OFFSET, data_relative, "linked data file offset")?;
    let bss_offset = checked_add(LOAD_FILE_OFFSET, bss_relative, "linked bss file offset")?;

    let mut bin = bounded_zeros(load_file_size as usize, "linked PT_LOAD file image")?;
    write_u32(&mut bin, 0, MULTIBOOT_MAGIC)?;
    write_u32(&mut bin, 4, MULTIBOOT_FLAGS)?;
    write_u32(&mut bin, 8, MULTIBOOT_CHECKSUM)?;
    copy_bytes(&mut bin, text_relative as usize, &image.text)?;
    copy_bytes(&mut bin, rodata_relative as usize, &image.rodata)?;
    copy_bytes(&mut bin, data_relative as usize, &image.data)?;

    let mut symbol_order: Vec<&ExecutableSymbol> = image.symbols.iter().collect();
    symbol_order.sort_by(|left, right| {
        left.binding
            .cmp(&right.binding)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.section.cmp(&right.section))
            .then_with(|| left.offset.cmp(&right.offset))
            .then_with(|| left.size.cmp(&right.size))
    });
    let local_symbol_count = symbol_order
        .iter()
        .take_while(|symbol| symbol.binding == SymbolBinding::Local)
        .count();
    let mut symbol_specs = Vec::with_capacity(symbol_order.len());
    for symbol in &symbol_order {
        let (section_address, section_index) = match symbol.section {
            ObjectSection::Text => (text_address, SECTION_TEXT),
            ObjectSection::Rodata => (rodata_address, SECTION_RODATA),
            ObjectSection::Data => (data_address, SECTION_DATA),
            ObjectSection::Bss => (bss_address, SECTION_BSS),
        };
        let binding = match symbol.binding {
            SymbolBinding::Local => STB_LOCAL,
            SymbolBinding::Global => STB_GLOBAL,
        };
        let symbol_type = match symbol.kind {
            SymbolKind::Function => STT_FUNC,
            SymbolKind::Object => STT_OBJECT,
            SymbolKind::Section => STT_SECTION,
        };
        symbol_specs.push(SymbolSpec {
            name: symbol.name.clone(),
            value: checked_add(section_address, symbol.offset, "linked symbol address")?,
            size: symbol.size,
            info: symbol_info(binding, symbol_type),
            section_index,
        });
    }
    let (symtab, strtab) = build_symbol_tables(&symbol_specs)?;
    let (shstrtab, section_name_offsets) = build_section_name_table()?;

    let load_file_end = checked_add(LOAD_FILE_OFFSET, load_file_size, "linked load file end")?;
    let symtab_offset = align_up(load_file_end, 4)?;
    let symtab_size = usize_u32(symtab.len(), "linked symbol table size")?;
    let strtab_offset = checked_add(symtab_offset, symtab_size, "linked string table offset")?;
    let strtab_size = usize_u32(strtab.len(), "linked string table size")?;
    let shstrtab_offset = checked_add(
        strtab_offset,
        strtab_size,
        "linked section-name table offset",
    )?;
    let shstrtab_size = usize_u32(shstrtab.len(), "linked section-name table size")?;
    let section_header_offset = align_up(
        checked_add(
            shstrtab_offset,
            shstrtab_size,
            "linked section-header offset",
        )?,
        4,
    )?;
    let file_size = checked_add(
        section_header_offset,
        checked_mul(
            u32::from(SECTION_COUNT),
            SECTION_HEADER_SIZE,
            "linked section-header table size",
        )?,
        "linked ELF file size",
    )?;
    if file_size as usize > MAX_ELF_SIZE {
        return Err(elf_diagnostic(format!(
            "ELF file size {file_size} exceeds bound {MAX_ELF_SIZE}"
        )));
    }

    let layout = ElfLayout {
        image_base: IMAGE_BASE,
        load_file_offset: LOAD_FILE_OFFSET,
        load_file_size,
        load_memory_size,
        entry,
        multiboot_offset: LOAD_FILE_OFFSET,
        multiboot_address,
        text_offset,
        text_address,
        text_size,
        rodata_offset,
        rodata_address,
        rodata_size,
        data_offset,
        data_address,
        data_size,
        bss_offset,
        bss_address,
        bss_size: image.bss_size,
        stack_bottom,
        stack_top,
        symtab_offset,
        strtab_offset,
        shstrtab_offset,
        section_header_offset,
        file_size,
        string_addresses: Vec::new(),
    };

    let section_headers = vec![
        SectionHeader {
            name: 0,
            section_type: SHT_NULL,
            flags: 0,
            address: 0,
            offset: 0,
            size: 0,
            link: 0,
            info: 0,
            address_align: 0,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_MULTIBOOT as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC,
            address: multiboot_address,
            offset: LOAD_FILE_OFFSET,
            size: MULTIBOOT_HEADER_SIZE,
            link: 0,
            info: 0,
            address_align: 4,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_TEXT as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC | SHF_EXECINSTR,
            address: text_address,
            offset: text_offset,
            size: text_size,
            link: 0,
            info: 0,
            address_align: image.text_alignment,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_RODATA as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC,
            address: rodata_address,
            offset: rodata_offset,
            size: rodata_size,
            link: 0,
            info: 0,
            address_align: image.rodata_alignment,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_DATA as usize],
            section_type: SHT_PROGBITS,
            flags: SHF_ALLOC | SHF_WRITE,
            address: data_address,
            offset: data_offset,
            size: data_size,
            link: 0,
            info: 0,
            address_align: image.data_alignment,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_BSS as usize],
            section_type: SHT_NOBITS,
            flags: SHF_ALLOC | SHF_WRITE,
            address: bss_address,
            offset: bss_offset,
            size: image.bss_size,
            link: 0,
            info: 0,
            address_align: image.bss_alignment,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_SYMTAB as usize],
            section_type: SHT_SYMTAB,
            flags: 0,
            address: 0,
            offset: symtab_offset,
            size: symtab_size,
            link: u32::from(SECTION_STRTAB),
            info: usize_u32(local_symbol_count + 1, "first linked global symbol")?,
            address_align: 4,
            entry_size: SYMBOL_SIZE,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_STRTAB as usize],
            section_type: SHT_STRTAB,
            flags: 0,
            address: 0,
            offset: strtab_offset,
            size: strtab_size,
            link: 0,
            info: 0,
            address_align: 1,
            entry_size: 0,
        },
        SectionHeader {
            name: section_name_offsets[SECTION_SHSTRTAB as usize],
            section_type: SHT_STRTAB,
            flags: 0,
            address: 0,
            offset: shstrtab_offset,
            size: shstrtab_size,
            link: 0,
            info: 0,
            address_align: 1,
            entry_size: 0,
        },
    ];

    validate_layout(&layout, &section_headers)?;
    let mut elf = bounded_zeros(file_size as usize, "linked ELF file")?;
    write_elf_header(&mut elf, &layout)?;
    write_program_header(&mut elf, &layout)?;
    copy_bytes(&mut elf, LOAD_FILE_OFFSET as usize, &bin)?;
    copy_bytes(&mut elf, symtab_offset as usize, &symtab)?;
    copy_bytes(&mut elf, strtab_offset as usize, &strtab)?;
    copy_bytes(&mut elf, shstrtab_offset as usize, &shstrtab)?;
    write_section_headers(&mut elf, section_header_offset, &section_headers)?;
    validate_final_bytes(&elf, &layout)?;
    let dump = linked_layout_dump(&layout, &section_headers, &symbol_order);
    Ok(ElfArtifact {
        elf,
        bin,
        machine_code: image.text.clone(),
        layout,
        dump,
    })
}

fn validate_linked_image(image: &LinkedExecutableImage) -> Result<(), Diagnostic> {
    validate_linked_image_contract(image, true)
}

fn validate_linked_image_contract(
    image: &LinkedExecutableImage,
    require_bootstrap_stack: bool,
) -> Result<(), Diagnostic> {
    if image.text.is_empty() {
        return Err(elf_diagnostic(
            "linked executable has an empty .text section",
        ));
    }
    if image.entry_offset as usize >= image.text.len() {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidEntryPoint,
            "linked entry offset is outside .text",
        ));
    }
    for (name, alignment) in [
        (".text", image.text_alignment),
        (".rodata", image.rodata_alignment),
        (".data", image.data_alignment),
        (".bss", image.bss_alignment),
    ] {
        if alignment == 0 || !alignment.is_power_of_two() || alignment > 4096 {
            return Err(elf_diagnostic(format!(
                "invalid linked {name} alignment {alignment}"
            )));
        }
    }
    if require_bootstrap_stack
        && (image
            .stack_top_offset
            .checked_sub(image.stack_bottom_offset)
            != Some(BOOTSTRAP_STACK_SIZE)
            || image.stack_top_offset > image.bss_size
            || !image.stack_top_offset.is_multiple_of(16))
    {
        return Err(elf_diagnostic(
            "linked bootstrap stack offsets are inconsistent",
        ));
    }
    let section_sizes = [
        (
            ObjectSection::Text,
            usize_u32(image.text.len(), "linked text size")?,
        ),
        (
            ObjectSection::Rodata,
            usize_u32(image.rodata.len(), "linked rodata size")?,
        ),
        (
            ObjectSection::Data,
            usize_u32(image.data.len(), "linked data size")?,
        ),
        (ObjectSection::Bss, image.bss_size),
    ];
    let mut globals = BTreeSet::new();
    for symbol in &image.symbols {
        if symbol.name.as_bytes().contains(&0)
            || (symbol.name.is_empty() && symbol.kind != SymbolKind::Section)
        {
            return Err(elf_diagnostic("linked symbol has an invalid name"));
        }
        if symbol.kind == SymbolKind::Section
            && (symbol.binding != SymbolBinding::Local || symbol.offset != 0 || symbol.size != 0)
        {
            return Err(elf_diagnostic(
                "linked section symbols must be local at offset zero",
            ));
        }
        let section_size = section_sizes
            .iter()
            .find_map(|(section, size)| (*section == symbol.section).then_some(*size))
            .expect("all object sections have final sizes");
        if symbol
            .offset
            .checked_add(symbol.size)
            .is_none_or(|end| end > section_size)
        {
            return Err(elf_diagnostic(format!(
                "linked symbol '{}' exceeds {} bounds",
                symbol.name,
                symbol.section.name()
            )));
        }
        if symbol.kind == SymbolKind::Function && symbol.section != ObjectSection::Text {
            return Err(elf_diagnostic(format!(
                "linked function '{}' is outside .text",
                symbol.name
            )));
        }
        if symbol.binding == SymbolBinding::Global && !globals.insert(symbol.name.as_str()) {
            return Err(elf_diagnostic(format!(
                "duplicate linked global symbol '{}'",
                symbol.name
            )));
        }
    }
    Ok(())
}

fn validate_lowered(program: &IrProgram, lowered: &LoweredProgram) -> Result<(), Diagnostic> {
    if lowered.machine_code.is_empty() {
        return Err(elf_diagnostic("cannot write an empty native text section"));
    }
    if lowered.entry_offset as usize >= lowered.machine_code.len() {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidEntryPoint,
            "native ELF entry offset is outside machine code",
        ));
    }
    if lowered.function(NATIVE_ENTRY_FUNCTION).is_none() {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidEntryPoint,
            "lowered program has no kernel_main symbol",
        ));
    }
    let mut names = BTreeSet::new();
    for symbol in &lowered.functions {
        if !names.insert(symbol.name.as_str()) {
            return Err(elf_diagnostic(format!(
                "duplicate lowered function symbol '{}'",
                symbol.name
            )));
        }
        if program.function(&symbol.name).is_none() {
            return Err(elf_diagnostic(format!(
                "lowered function '{}' is absent from native IR",
                symbol.name
            )));
        }
        let end = symbol
            .offset
            .checked_add(symbol.size)
            .ok_or_else(|| elf_diagnostic("function symbol range overflow"))?;
        if end as usize > lowered.machine_code.len() {
            return Err(elf_diagnostic(format!(
                "lowered function '{}' exceeds machine-code bounds",
                symbol.name
            )));
        }
    }
    for bytes in &program.strings {
        if !bytes.is_ascii() {
            return Err(elf_diagnostic("native rodata contains a non-ASCII string"));
        }
    }
    Ok(())
}

fn build_rodata(strings: &[Vec<u8>]) -> Result<(Vec<u8>, Vec<u32>), Diagnostic> {
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(strings.len());
    for string in strings {
        offsets.push(usize_u32(bytes.len(), "string rodata offset")?);
        let needed = bytes
            .len()
            .checked_add(string.len())
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| elf_diagnostic("rodata size overflow"))?;
        if needed > MAX_ELF_SIZE {
            return Err(elf_diagnostic("rodata exceeds ELF size bound"));
        }
        bytes.extend_from_slice(string);
        bytes.push(0);
    }
    Ok((bytes, offsets))
}

fn patch_absolute_fixups(
    machine_code: &mut [u8],
    lowered: &LoweredProgram,
    stack_top: u32,
    string_addresses: &[u32],
) -> Result<(), Diagnostic> {
    let mut occupied = BTreeSet::new();
    for fixup in &lowered.absolute_fixups {
        let offset = fixup.immediate_offset as usize;
        let end = offset
            .checked_add(4)
            .ok_or_else(|| elf_diagnostic("absolute fixup offset overflow"))?;
        if end > machine_code.len() {
            return Err(elf_diagnostic(format!(
                "absolute fixup at 0x{:08x} exceeds machine-code bounds",
                fixup.immediate_offset
            )));
        }
        for byte_offset in offset..end {
            if !occupied.insert(byte_offset) {
                return Err(elf_diagnostic(format!(
                    "overlapping absolute fixup at 0x{:08x}",
                    fixup.immediate_offset
                )));
            }
        }
        let value = match fixup.target {
            AbsoluteFixupTarget::StackTop => stack_top,
            AbsoluteFixupTarget::String(StringId(id)) => string_addresses
                .get(id as usize)
                .copied()
                .ok_or_else(|| elf_diagnostic(format!("unknown string fixup @str{id}")))?,
        };
        machine_code[offset..end].copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

fn build_symbol_tables(symbols: &[SymbolSpec]) -> Result<(Vec<u8>, Vec<u8>), Diagnostic> {
    let mut strings = vec![0u8];
    let entry_count = symbols
        .len()
        .checked_add(1)
        .ok_or_else(|| elf_diagnostic("symbol count overflow"))?;
    let table_len = entry_count
        .checked_mul(SYMBOL_SIZE as usize)
        .ok_or_else(|| elf_diagnostic("symbol table size overflow"))?;
    let mut table = bounded_zeros(table_len, "symbol table")?;
    for (index, symbol) in symbols.iter().enumerate() {
        let name_offset = usize_u32(strings.len(), "symbol name offset")?;
        let required = strings
            .len()
            .checked_add(symbol.name.len())
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| elf_diagnostic("symbol string table overflow"))?;
        if required > MAX_ELF_SIZE {
            return Err(elf_diagnostic("symbol string table exceeds bound"));
        }
        strings.extend_from_slice(symbol.name.as_bytes());
        strings.push(0);

        let offset = (index + 1) * SYMBOL_SIZE as usize;
        write_u32(&mut table, offset, name_offset)?;
        write_u32(&mut table, offset + 4, symbol.value)?;
        write_u32(&mut table, offset + 8, symbol.size)?;
        write_u8(&mut table, offset + 12, symbol.info)?;
        write_u8(&mut table, offset + 13, 0)?;
        write_u16(&mut table, offset + 14, symbol.section_index)?;
    }
    Ok((table, strings))
}

fn build_section_name_table() -> Result<(Vec<u8>, Vec<u32>), Diagnostic> {
    let names = [
        "",
        ".multiboot",
        ".text",
        ".rodata",
        ".data",
        ".bss",
        ".symtab",
        ".strtab",
        ".shstrtab",
    ];
    let mut bytes = Vec::new();
    let mut offsets = Vec::with_capacity(names.len());
    for name in names {
        offsets.push(usize_u32(bytes.len(), "section name offset")?);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(0);
    }
    Ok((bytes, offsets))
}

fn write_elf_header(elf: &mut [u8], layout: &ElfLayout) -> Result<(), Diagnostic> {
    copy_bytes(elf, 0, b"\x7fELF")?;
    write_u8(elf, 4, 1)?; // ELFCLASS32
    write_u8(elf, 5, 1)?; // ELFDATA2LSB
    write_u8(elf, 6, 1)?; // EV_CURRENT
    write_u8(elf, 7, 0)?; // ELFOSABI_NONE
    write_u16(elf, 16, 2)?; // ET_EXEC
    write_u16(elf, 18, 3)?; // EM_386
    write_u32(elf, 20, 1)?; // EV_CURRENT
    write_u32(elf, 24, layout.entry)?;
    write_u32(elf, 28, ELF_HEADER_SIZE)?;
    write_u32(elf, 32, layout.section_header_offset)?;
    write_u32(elf, 36, 0)?;
    write_u16(elf, 40, ELF_HEADER_SIZE as u16)?;
    write_u16(elf, 42, PROGRAM_HEADER_SIZE as u16)?;
    write_u16(elf, 44, 1)?;
    write_u16(elf, 46, SECTION_HEADER_SIZE as u16)?;
    write_u16(elf, 48, SECTION_COUNT)?;
    write_u16(elf, 50, SECTION_SHSTRTAB)?;
    Ok(())
}

fn write_program_header(elf: &mut [u8], layout: &ElfLayout) -> Result<(), Diagnostic> {
    let offset = ELF_HEADER_SIZE as usize;
    write_u32(elf, offset, PT_LOAD)?;
    write_u32(elf, offset + 4, layout.load_file_offset)?;
    write_u32(elf, offset + 8, layout.image_base)?;
    write_u32(elf, offset + 12, layout.image_base)?;
    write_u32(elf, offset + 16, layout.load_file_size)?;
    write_u32(elf, offset + 20, layout.load_memory_size)?;
    write_u32(elf, offset + 24, PF_R | PF_W | PF_X)?;
    write_u32(elf, offset + 28, 0x1000)?;
    Ok(())
}

fn write_section_headers(
    elf: &mut [u8],
    table_offset: u32,
    sections: &[SectionHeader],
) -> Result<(), Diagnostic> {
    if sections.len() != SECTION_COUNT as usize {
        return Err(elf_diagnostic("internal section count mismatch"));
    }
    for (index, section) in sections.iter().enumerate() {
        let offset = (table_offset as usize)
            .checked_add(index * SECTION_HEADER_SIZE as usize)
            .ok_or_else(|| elf_diagnostic("section-header offset overflow"))?;
        write_u32(elf, offset, section.name)?;
        write_u32(elf, offset + 4, section.section_type)?;
        write_u32(elf, offset + 8, section.flags)?;
        write_u32(elf, offset + 12, section.address)?;
        write_u32(elf, offset + 16, section.offset)?;
        write_u32(elf, offset + 20, section.size)?;
        write_u32(elf, offset + 24, section.link)?;
        write_u32(elf, offset + 28, section.info)?;
        write_u32(elf, offset + 32, section.address_align)?;
        write_u32(elf, offset + 36, section.entry_size)?;
    }
    Ok(())
}

fn validate_layout(layout: &ElfLayout, sections: &[SectionHeader]) -> Result<(), Diagnostic> {
    if layout.multiboot_offset != LOAD_FILE_OFFSET
        || layout.multiboot_offset >= 8192
        || !layout.multiboot_offset.is_multiple_of(4)
    {
        return Err(elf_diagnostic(
            "Multiboot header is not aligned within the first 8 KiB",
        ));
    }
    let text_end = checked_add(layout.text_address, layout.text_size, "text address end")?;
    if layout.entry < layout.text_address || layout.entry >= text_end {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidEntryPoint,
            "ELF entry is outside .text",
        ));
    }
    if layout.load_file_size > layout.load_memory_size {
        return Err(elf_diagnostic("PT_LOAD filesz exceeds memsz"));
    }
    if layout.stack_top.checked_sub(layout.stack_bottom) != Some(BOOTSTRAP_STACK_SIZE) {
        return Err(elf_diagnostic("bootstrap stack layout is inconsistent"));
    }
    if !layout.stack_top.is_multiple_of(16) {
        return Err(elf_diagnostic("bootstrap stack top is not 16-byte aligned"));
    }

    let mut file_ranges = Vec::new();
    for (name, index) in [
        (".multiboot", SECTION_MULTIBOOT),
        (".text", SECTION_TEXT),
        (".rodata", SECTION_RODATA),
        (".data", SECTION_DATA),
        (".symtab", SECTION_SYMTAB),
        (".strtab", SECTION_STRTAB),
        (".shstrtab", SECTION_SHSTRTAB),
    ] {
        let section = &sections[index as usize];
        if section.size == 0 {
            continue;
        }
        let end = checked_add(section.offset, section.size, "section file range")?;
        if end > layout.file_size {
            return Err(elf_diagnostic(format!("section {name} exceeds ELF bounds")));
        }
        file_ranges.push(FileRange {
            name,
            start: section.offset,
            end,
        });
    }
    let section_table_end = checked_add(
        layout.section_header_offset,
        checked_mul(
            u32::from(SECTION_COUNT),
            SECTION_HEADER_SIZE,
            "section table range",
        )?,
        "section table end",
    )?;
    file_ranges.push(FileRange {
        name: "section headers",
        start: layout.section_header_offset,
        end: section_table_end,
    });
    file_ranges.sort_by_key(|range| range.start);
    for pair in file_ranges.windows(2) {
        if pair[0].end > pair[1].start {
            return Err(elf_diagnostic(format!(
                "ELF file ranges {} and {} overlap",
                pair[0].name, pair[1].name
            )));
        }
    }

    let segment_file_end = checked_add(
        layout.load_file_offset,
        layout.load_file_size,
        "segment file end",
    )?;
    for index in [
        SECTION_MULTIBOOT,
        SECTION_TEXT,
        SECTION_RODATA,
        SECTION_DATA,
    ] {
        let section = &sections[index as usize];
        let end = checked_add(section.offset, section.size, "allocated section end")?;
        if section.offset < layout.load_file_offset || end > segment_file_end {
            return Err(elf_diagnostic(
                "file-backed allocated section is outside PT_LOAD",
            ));
        }
    }
    let segment_memory_end = checked_add(
        layout.image_base,
        layout.load_memory_size,
        "segment memory end",
    )?;
    let bss_end = checked_add(layout.bss_address, layout.bss_size, "bss end")?;
    if layout.bss_address < layout.image_base || bss_end > segment_memory_end {
        return Err(elf_diagnostic(".bss is outside PT_LOAD memory range"));
    }
    Ok(())
}

fn validate_final_bytes(elf: &[u8], layout: &ElfLayout) -> Result<(), Diagnostic> {
    if elf.len() != layout.file_size as usize || &elf[..4] != b"\x7fELF" {
        return Err(elf_diagnostic("final ELF header or size is invalid"));
    }
    let header = layout.multiboot_offset as usize;
    let magic = read_u32(elf, header)?;
    let flags = read_u32(elf, header + 4)?;
    let checksum = read_u32(elf, header + 8)?;
    if magic != MULTIBOOT_MAGIC
        || flags != MULTIBOOT_FLAGS
        || magic.wrapping_add(flags).wrapping_add(checksum) != 0
    {
        return Err(elf_diagnostic("final Multiboot header is invalid"));
    }
    // There is exactly one program header and it is PT_LOAD. Therefore a
    // dynamic interpreter or PT_DYNAMIC cannot be present.
    if read_u16(elf, 44)? != 1 || read_u32(elf, ELF_HEADER_SIZE as usize)? != PT_LOAD {
        return Err(elf_diagnostic(
            "final ELF program-header contract is invalid",
        ));
    }
    Ok(())
}

fn layout_dump(layout: &ElfLayout, sections: &[SectionHeader], lowered: &LoweredProgram) -> String {
    let mut dump = String::new();
    let _ = writeln!(dump, "ELF32 little-endian ET_EXEC EM_386");
    let _ = writeln!(dump, "entry 0x{:08x}", layout.entry);
    let _ = writeln!(
        dump,
        "PT_LOAD file=0x{:08x}+0x{:x} memory=0x{:08x}+0x{:x} flags=rwx",
        layout.load_file_offset, layout.load_file_size, layout.image_base, layout.load_memory_size
    );
    let names = [
        "",
        ".multiboot",
        ".text",
        ".rodata",
        ".data",
        ".bss",
        ".symtab",
        ".strtab",
        ".shstrtab",
    ];
    for (index, section) in sections.iter().enumerate().skip(1) {
        let _ = writeln!(
            dump,
            "section {} offset=0x{:08x} address=0x{:08x} size=0x{:x} type={}",
            names[index], section.offset, section.address, section.size, section.section_type
        );
    }
    let _ = writeln!(
        dump,
        "stack bottom=0x{:08x} top=0x{:08x} size={}",
        layout.stack_bottom, layout.stack_top, BOOTSTRAP_STACK_SIZE
    );
    for (index, address) in layout.string_addresses.iter().enumerate() {
        let _ = writeln!(dump, "string @str{index} address=0x{address:08x}");
    }
    for fixup in &lowered.absolute_fixups {
        let _ = writeln!(
            dump,
            "patched absolute immediate=0x{:08x} target={:?}",
            fixup.immediate_offset, fixup.target
        );
    }
    dump
}

fn linked_layout_dump(
    layout: &ElfLayout,
    sections: &[SectionHeader],
    symbols: &[&ExecutableSymbol],
) -> String {
    let mut dump = String::new();
    let _ = writeln!(dump, "ELF32 little-endian ET_EXEC EM_386 linked");
    let _ = writeln!(dump, "entry 0x{:08x}", layout.entry);
    let _ = writeln!(
        dump,
        "PT_LOAD file=0x{:08x}+0x{:x} memory=0x{:08x}+0x{:x} flags=rwx",
        layout.load_file_offset, layout.load_file_size, layout.image_base, layout.load_memory_size
    );
    let names = [
        "",
        ".multiboot",
        ".text",
        ".rodata",
        ".data",
        ".bss",
        ".symtab",
        ".strtab",
        ".shstrtab",
    ];
    for (index, section) in sections.iter().enumerate().skip(1) {
        let _ = writeln!(
            dump,
            "section {} offset=0x{:08x} address=0x{:08x} size=0x{:x} type={}",
            names[index], section.offset, section.address, section.size, section.section_type
        );
    }
    let _ = writeln!(
        dump,
        "stack bottom=0x{:08x} top=0x{:08x} size={}",
        layout.stack_bottom, layout.stack_top, BOOTSTRAP_STACK_SIZE
    );
    for symbol in symbols {
        let _ = writeln!(
            dump,
            "symbol {:?} {:?} {:?} {}+0x{:08x} size={}",
            symbol.binding,
            symbol.kind,
            symbol.name,
            symbol.section.name(),
            symbol.offset,
            symbol.size
        );
    }
    dump
}

fn symbol_info(binding: u8, symbol_type: u8) -> u8 {
    (binding << 4) | (symbol_type & 0x0f)
}

fn align_up(value: u32, alignment: u32) -> Result<u32, Diagnostic> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(elf_diagnostic("invalid ELF alignment"));
    }
    let mask = alignment - 1;
    value
        .checked_add(mask)
        .map(|sum| sum & !mask)
        .ok_or_else(|| elf_diagnostic("ELF alignment overflow"))
}

fn checked_add(left: u32, right: u32, context: &str) -> Result<u32, Diagnostic> {
    left.checked_add(right)
        .ok_or_else(|| elf_diagnostic(format!("{context} overflow")))
}

fn checked_mul(left: u32, right: u32, context: &str) -> Result<u32, Diagnostic> {
    left.checked_mul(right)
        .ok_or_else(|| elf_diagnostic(format!("{context} overflow")))
}

fn usize_u32(value: usize, context: &str) -> Result<u32, Diagnostic> {
    u32::try_from(value).map_err(|_| elf_diagnostic(format!("{context} exceeds u32")))
}

fn bounded_zeros(length: usize, context: &str) -> Result<Vec<u8>, Diagnostic> {
    if length > MAX_ELF_SIZE {
        return Err(elf_diagnostic(format!(
            "{context} length {length} exceeds bound {MAX_ELF_SIZE}"
        )));
    }
    Ok(vec![0; length])
}

fn copy_bytes(destination: &mut [u8], offset: usize, source: &[u8]) -> Result<(), Diagnostic> {
    let end = offset
        .checked_add(source.len())
        .ok_or_else(|| elf_diagnostic("ELF byte range overflow"))?;
    let target = destination
        .get_mut(offset..end)
        .ok_or_else(|| elf_diagnostic("ELF byte write is out of bounds"))?;
    target.copy_from_slice(source);
    Ok(())
}

fn write_u8(destination: &mut [u8], offset: usize, value: u8) -> Result<(), Diagnostic> {
    let byte = destination
        .get_mut(offset)
        .ok_or_else(|| elf_diagnostic("ELF u8 write is out of bounds"))?;
    *byte = value;
    Ok(())
}

fn write_u16(destination: &mut [u8], offset: usize, value: u16) -> Result<(), Diagnostic> {
    copy_bytes(destination, offset, &value.to_le_bytes())
}

fn write_u32(destination: &mut [u8], offset: usize, value: u32) -> Result<(), Diagnostic> {
    copy_bytes(destination, offset, &value.to_le_bytes())
}

fn read_u16(source: &[u8], offset: usize) -> Result<u16, Diagnostic> {
    let end = offset
        .checked_add(2)
        .ok_or_else(|| elf_diagnostic("ELF u16 read overflow"))?;
    let bytes: [u8; 2] = source
        .get(offset..end)
        .ok_or_else(|| elf_diagnostic("ELF u16 read is out of bounds"))?
        .try_into()
        .map_err(|_| elf_diagnostic("ELF u16 read has invalid width"))?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(source: &[u8], offset: usize) -> Result<u32, Diagnostic> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| elf_diagnostic("ELF u32 read overflow"))?;
    let bytes: [u8; 4] = source
        .get(offset..end)
        .ok_or_else(|| elf_diagnostic("ELF u32 read is out of bounds"))?
        .try_into()
        .map_err(|_| elf_diagnostic("ELF u32 read has invalid width"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn elf_diagnostic(message: impl Into<String>) -> Diagnostic {
    Diagnostic::without_span(DiagnosticCategory::Elf, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{IrFunction, IrInst};
    use crate::lower::{lower_program, AbsoluteFixup};
    use crate::types::NativeType;
    use dbyte_ast::Span;

    fn program_with_string() -> IrProgram {
        IrProgram {
            functions: vec![IrFunction {
                name: NATIVE_ENTRY_FUNCTION.to_owned(),
                params: Vec::new(),
                return_type: NativeType::I32,
                slots: Vec::new(),
                instructions: vec![
                    IrInst::ConstStr(StringId(0)),
                    IrInst::Pop,
                    IrInst::ConstI32(42),
                    IrInst::Return,
                ],
                span: Span::zero(),
            }],
            strings: vec![b"DBYTE".to_vec()],
        }
    }

    fn artifact() -> ElfArtifact {
        let program = program_with_string();
        let lowered = lower_program(&program).unwrap();
        write_executable(&program, &lowered).unwrap()
    }

    #[test]
    fn writes_exact_elf32_i386_header_contract() {
        let artifact = artifact();
        assert_eq!(&artifact.elf[..7], b"\x7fELF\x01\x01\x01");
        assert_eq!(read_u16(&artifact.elf, 16).unwrap(), 2);
        assert_eq!(read_u16(&artifact.elf, 18).unwrap(), 3);
        assert_eq!(read_u32(&artifact.elf, 24).unwrap(), artifact.layout.entry);
        assert_eq!(read_u16(&artifact.elf, 44).unwrap(), 1);
        assert_eq!(
            read_u32(&artifact.elf, ELF_HEADER_SIZE as usize).unwrap(),
            PT_LOAD
        );
    }

    #[test]
    fn multiboot_header_is_exact_and_within_first_eight_kib() {
        let artifact = artifact();
        let offset = artifact.layout.multiboot_offset as usize;
        assert_eq!(offset, 0x1000);
        assert!(offset < 8192 && offset.is_multiple_of(4));
        assert_eq!(read_u32(&artifact.elf, offset).unwrap(), MULTIBOOT_MAGIC);
        assert_eq!(read_u32(&artifact.elf, offset + 4).unwrap(), 0x2);
        assert_eq!(
            read_u32(&artifact.elf, offset + 8).unwrap(),
            MULTIBOOT_CHECKSUM
        );
    }

    #[test]
    fn entry_and_sections_are_bounded_and_non_overlapping() {
        let artifact = artifact();
        let layout = &artifact.layout;
        assert!(layout.entry >= layout.text_address);
        assert!(layout.entry < layout.text_address + layout.text_size);
        assert!(layout.text_offset >= layout.multiboot_offset + MULTIBOOT_HEADER_SIZE);
        assert!(layout.rodata_offset >= layout.text_offset + layout.text_size);
        assert!(layout.symtab_offset >= layout.load_file_offset + layout.load_file_size);
        assert_eq!(artifact.elf.len(), layout.file_size as usize);
        assert_eq!(layout.bss_size, 16 * 1024);
        assert!(layout.load_memory_size > layout.load_file_size);
    }

    #[test]
    fn patches_stack_and_string_addresses_after_layout() {
        let program = program_with_string();
        let lowered = lower_program(&program).unwrap();
        let artifact = write_executable(&program, &lowered).unwrap();
        for fixup in &lowered.absolute_fixups {
            let offset = fixup.immediate_offset as usize;
            let patched = read_u32(&artifact.machine_code, offset).unwrap();
            match fixup.target {
                AbsoluteFixupTarget::StackTop => assert_eq!(patched, artifact.layout.stack_top),
                AbsoluteFixupTarget::String(StringId(0)) => {
                    assert_eq!(patched, artifact.layout.string_addresses[0])
                }
                AbsoluteFixupTarget::String(other) => panic!("unexpected string {other:?}"),
            }
        }
        let text_in_bin = (artifact.layout.text_address - artifact.layout.image_base) as usize;
        assert_eq!(
            &artifact.bin[text_in_bin..text_in_bin + artifact.machine_code.len()],
            artifact.machine_code
        );
    }

    #[test]
    fn section_name_and_symbol_tables_are_present() {
        let artifact = artifact();
        assert!(artifact
            .elf
            .windows(b".text\0".len())
            .any(|window| window == b".text\0"));
        assert!(artifact
            .elf
            .windows(b".symtab\0".len())
            .any(|window| window == b".symtab\0"));
        assert!(artifact
            .elf
            .windows(b"kernel_main\0".len())
            .any(|window| window == b"kernel_main\0"));
    }

    #[test]
    fn repeated_writes_are_byte_for_byte_deterministic() {
        let program = program_with_string();
        let lowered = lower_program(&program).unwrap();
        let first = write_executable(&program, &lowered).unwrap();
        let second = write_executable(&program, &lowered).unwrap();
        assert_eq!(first.elf, second.elf);
        assert_eq!(first.bin, second.bin);
        assert_eq!(first.machine_code, second.machine_code);
        assert_eq!(first.dump, second.dump);
    }

    #[test]
    fn invalid_absolute_fixup_is_rejected() {
        let program = program_with_string();
        let mut lowered = lower_program(&program).unwrap();
        lowered.absolute_fixups.push(AbsoluteFixup {
            immediate_offset: u32::MAX,
            target: AbsoluteFixupTarget::StackTop,
        });
        let error = write_executable(&program, &lowered).unwrap_err();
        assert!(error.message.contains("absolute fixup"));
    }

    #[test]
    fn no_dynamic_or_interpreter_program_headers_can_exist() {
        let artifact = artifact();
        let program_header_count = read_u16(&artifact.elf, 44).unwrap();
        assert_eq!(program_header_count, 1);
        assert_eq!(read_u32(&artifact.elf, 52).unwrap(), PT_LOAD);
        assert!(!artifact.dump.contains("PT_INTERP"));
        assert!(!artifact.dump.contains("PT_DYNAMIC"));
    }
}
