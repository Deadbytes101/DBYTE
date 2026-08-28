use dbyte_hir::{Profile, TargetDataLayout};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MachineTarget {
    X86I686,
    X86_64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperatingSystem {
    None,
    Linux,
    Windows,
    DByteUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DByteAbi {
    DByte32,
    DByte64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CAbi {
    I686Cdecl,
    SysV64,
    Win64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    MultibootElf32,
    UserElf32,
    Multiboot2LongMode,
    LinuxStart,
    WindowsEntry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectFormat {
    Elf32,
    Elf64,
    CoffAmd64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExecutableFormat {
    MultibootElf32,
    UserElf32,
    Multiboot2Elf64,
    StaticElf64,
    Pe32Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetSpec {
    pub triple: &'static str,
    pub machine: MachineTarget,
    pub os: OperatingSystem,
    pub profile: Profile,
    pub layout: TargetDataLayout,
    pub dbyte_abi: DByteAbi,
    pub c_abi: Option<CAbi>,
    pub object_format: ObjectFormat,
    pub executable_format: ExecutableFormat,
    pub entry: EntryKind,
}

/// Compatibility name retained only for the candidate DBYTE64 object seam.
/// New target-dispatch code uses `TargetSpec` directly.
pub type TargetDescriptor = TargetSpec;

pub const I686_NONE: TargetSpec = TargetSpec {
    triple: "i686-dbyte-none",
    machine: MachineTarget::X86I686,
    os: OperatingSystem::None,
    profile: Profile::I686None,
    layout: TargetDataLayout::I686,
    dbyte_abi: DByteAbi::DByte32,
    c_abi: Some(CAbi::I686Cdecl),
    object_format: ObjectFormat::Elf32,
    executable_format: ExecutableFormat::MultibootElf32,
    entry: EntryKind::MultibootElf32,
};

pub const I686_USER: TargetSpec = TargetSpec {
    triple: "i686-dbyte-user",
    machine: MachineTarget::X86I686,
    os: OperatingSystem::DByteUser,
    profile: Profile::I686User,
    layout: TargetDataLayout::I686,
    dbyte_abi: DByteAbi::DByte32,
    c_abi: Some(CAbi::I686Cdecl),
    object_format: ObjectFormat::Elf32,
    executable_format: ExecutableFormat::UserElf32,
    entry: EntryKind::UserElf32,
};

pub const X86_64_NONE: TargetSpec = TargetSpec {
    triple: "x86_64-dbyte-none",
    machine: MachineTarget::X86_64,
    os: OperatingSystem::None,
    profile: Profile::X86_64None,
    layout: TargetDataLayout::X86_64,
    dbyte_abi: DByteAbi::DByte64,
    c_abi: None,
    object_format: ObjectFormat::Elf64,
    executable_format: ExecutableFormat::Multiboot2Elf64,
    entry: EntryKind::Multiboot2LongMode,
};

pub const X86_64_LINUX: TargetSpec = TargetSpec {
    triple: "x86_64-dbyte-linux",
    machine: MachineTarget::X86_64,
    os: OperatingSystem::Linux,
    profile: Profile::X86_64Linux,
    layout: TargetDataLayout::X86_64,
    dbyte_abi: DByteAbi::DByte64,
    c_abi: Some(CAbi::SysV64),
    object_format: ObjectFormat::Elf64,
    executable_format: ExecutableFormat::StaticElf64,
    entry: EntryKind::LinuxStart,
};

pub const X86_64_WINDOWS: TargetSpec = TargetSpec {
    triple: "x86_64-dbyte-windows",
    machine: MachineTarget::X86_64,
    os: OperatingSystem::Windows,
    profile: Profile::X86_64Windows,
    layout: TargetDataLayout::X86_64,
    dbyte_abi: DByteAbi::DByte64,
    c_abi: Some(CAbi::Win64),
    object_format: ObjectFormat::CoffAmd64,
    executable_format: ExecutableFormat::Pe32Plus,
    entry: EntryKind::WindowsEntry,
};

pub const TARGETS: [TargetDescriptor; 5] = [
    I686_NONE,
    I686_USER,
    X86_64_NONE,
    X86_64_LINUX,
    X86_64_WINDOWS,
];

pub fn parse_target(triple: &str) -> Option<TargetDescriptor> {
    TARGETS
        .iter()
        .copied()
        .find(|target| target.triple == triple)
}

pub const TARGET_METADATA_SECTION: &str = ".dbyte.target";
const TARGET_METADATA_MAGIC: &[u8; 8] = b"DBYTGT64";
const TARGET_METADATA_VERSION: u16 = 1;
const TARGET_METADATA_SIZE: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetMetadataError(pub String);

impl std::fmt::Display for TargetMetadataError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TargetMetadataError {}

pub fn encode_target_metadata(target: TargetDescriptor) -> [u8; TARGET_METADATA_SIZE] {
    let mut bytes = [0u8; TARGET_METADATA_SIZE];
    bytes[..8].copy_from_slice(TARGET_METADATA_MAGIC);
    bytes[8..10].copy_from_slice(&TARGET_METADATA_VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(TARGET_METADATA_SIZE as u16).to_le_bytes());
    bytes[12] = target.machine as u8;
    bytes[13] = target.os as u8;
    bytes[14] = target.dbyte_abi as u8;
    bytes[15] = target.c_abi.map_or(0xff, |abi| abi as u8);
    bytes[16] = target.object_format as u8;
    bytes[17] = target.executable_format as u8;
    bytes[18] = target.layout.pointer_size as u8;
    bytes[19] = target.layout.pointer_align as u8;
    bytes[20] = target.layout.usize_size as u8;
    bytes[21] = target.layout.usize_align as u8;
    bytes[22] = match target.profile {
        Profile::Hosted => 0,
        Profile::Vm => 1,
        Profile::I686None => 2,
        Profile::I686User => 3,
        Profile::X86_64None => 4,
        Profile::X86_64Linux => 5,
        Profile::X86_64Windows => 6,
    };
    bytes[23] = target.entry as u8;
    let triple = target.triple.as_bytes();
    let hash = triple.iter().fold(0x811c_9dc5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    bytes[24..28].copy_from_slice(&hash.to_le_bytes());
    bytes
}

pub fn decode_target_metadata(bytes: &[u8]) -> Result<TargetDescriptor, TargetMetadataError> {
    if bytes.len() != TARGET_METADATA_SIZE {
        return Err(TargetMetadataError(format!(
            "E_TARGET_METADATA_SIZE: expected {TARGET_METADATA_SIZE} bytes, found {}",
            bytes.len()
        )));
    }
    if &bytes[..8] != TARGET_METADATA_MAGIC {
        return Err(TargetMetadataError(
            "E_TARGET_METADATA_MAGIC: invalid .dbyte.target magic".into(),
        ));
    }
    let version = u16::from_le_bytes([bytes[8], bytes[9]]);
    if version != TARGET_METADATA_VERSION {
        return Err(TargetMetadataError(format!(
            "E_TARGET_METADATA_VERSION: unsupported version {version}"
        )));
    }
    if u16::from_le_bytes([bytes[10], bytes[11]]) as usize != TARGET_METADATA_SIZE {
        return Err(TargetMetadataError(
            "E_TARGET_METADATA_RECORD: invalid record size".into(),
        ));
    }
    if bytes[28..].iter().any(|byte| *byte != 0) {
        return Err(TargetMetadataError(
            "E_TARGET_METADATA_RESERVED: reserved bytes must be zero".into(),
        ));
    }
    TARGETS
        .iter()
        .copied()
        .find(|target| encode_target_metadata(*target) == bytes)
        .ok_or_else(|| {
            TargetMetadataError(
                "E_TARGET_METADATA_CONFLICT: unknown or conflicting target tuple".into(),
            )
        })
}

pub fn validate_link_targets(
    objects: impl IntoIterator<Item = TargetDescriptor>,
    requested: TargetDescriptor,
) -> Result<(), TargetMetadataError> {
    for (index, object) in objects.into_iter().enumerate() {
        if object != requested {
            return Err(TargetMetadataError(format!(
                "E_TARGET_MISMATCH: input {index} is `{}` but link target is `{}`",
                object.triple, requested.triple
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_matrix_has_explicit_layout_and_abi() {
        assert_eq!(parse_target("i686-dbyte-none"), Some(I686_NONE));
        assert_eq!(X86_64_NONE.layout.pointer_size, 8);
        assert_eq!(X86_64_NONE.c_abi, None);
        assert_eq!(X86_64_LINUX.c_abi, Some(CAbi::SysV64));
        assert_eq!(X86_64_WINDOWS.c_abi, Some(CAbi::Win64));
        assert_eq!(X86_64_WINDOWS.object_format, ObjectFormat::CoffAmd64);
        assert_eq!(X86_64_NONE.entry, EntryKind::Multiboot2LongMode);
        assert_eq!(parse_target("x86_64-unknown-linux-gnu"), None);
    }

    #[test]
    fn x86_64_target_metadata_round_trips_and_is_strict() {
        for target in [X86_64_NONE, X86_64_LINUX, X86_64_WINDOWS] {
            let bytes = encode_target_metadata(target);
            assert_eq!(decode_target_metadata(&bytes).unwrap(), target);
        }
        let mut malformed = encode_target_metadata(X86_64_LINUX);
        malformed[28] = 1;
        assert!(decode_target_metadata(&malformed)
            .unwrap_err()
            .0
            .contains("E_TARGET_METADATA_RESERVED"));
    }

    #[test]
    fn linker_rejects_mixed_target_tuples() {
        let error =
            validate_link_targets([X86_64_LINUX, X86_64_WINDOWS], X86_64_LINUX).unwrap_err();
        assert!(error.0.contains("E_TARGET_MISMATCH"));
    }
}
