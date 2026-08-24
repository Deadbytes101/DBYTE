use std::fmt;

/// Value types admitted by the freestanding `i686-dbyte-none` profile.
///
/// Hosted DByte's `int` is an i64. The native profile deliberately maps the
/// same source spelling to a signed 32-bit machine value and validates every
/// literal before lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NativeType {
    I32,
    Bool,
    Str,
}

impl fmt::Display for NativeType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::I32 => f.write_str("i32"),
            Self::Bool => f.write_str("bool"),
            Self::Str => f.write_str("str"),
        }
    }
}

/// Privileged operations available only through the explicit `native.*`
/// namespace in the freestanding semantic pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Intrinsic {
    VolatileLoad8,
    VolatileStore8,
    VolatileLoad16,
    VolatileStore16,
    VolatileLoad32,
    VolatileStore32,
    /// x86 port I/O — also callable as `native.port_out8`.
    IoOut8,
    /// x86 port I/O — also callable as `native.port_in8`.
    IoIn8,
    /// Disable interrupts (`CLI`).
    CpuCli,
    /// Enable interrupts (`STI`).
    CpuSti,
    /// Halt the CPU (`HLT`).
    CpuHlt,
    /// Spin-loop hint (`PAUSE` = `F3 90`). Use in busy-wait loops to reduce
    /// power consumption and improve SMT behaviour.
    CpuPause,
    /// Copy `count` bytes from `src` to `dst` (non-overlapping, like `memcpy`).
    MemCopy,
    /// Fill `count` bytes at `dst` with the low byte of `value` (like `memset`).
    MemSet,
    /// Zero `count` bytes starting at `dst`.
    MemZero,
    /// Load IDT from a 6-byte descriptor at `desc_ptr` (`LIDT [r]`).
    /// The descriptor must be already written to memory: `{ u16 limit, u32 base }`.
    Lidt,
    /// Read the EFLAGS register into an `int` (`PUSHFD` + `POP EAX`).
    ReadEflags,
    /// Write `flags` into the EFLAGS register (`PUSH EAX` + `POPFD`).
    WriteEflags,
}

impl Intrinsic {
    /// Resolve either a bare member name (`io_out8`) or its fully-qualified
    /// spelling (`native.io_out8`).
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix("native.").unwrap_or(name);
        match name {
            "volatile_load8" => Some(Self::VolatileLoad8),
            "volatile_store8" => Some(Self::VolatileStore8),
            "volatile_load16" => Some(Self::VolatileLoad16),
            "volatile_store16" => Some(Self::VolatileStore16),
            "volatile_load32" => Some(Self::VolatileLoad32),
            "volatile_store32" => Some(Self::VolatileStore32),
            // I/O port intrinsics — accept both spellings.
            "io_out8" | "port_out8" => Some(Self::IoOut8),
            "io_in8" | "port_in8" => Some(Self::IoIn8),
            "cpu_cli" => Some(Self::CpuCli),
            "cpu_sti" => Some(Self::CpuSti),
            "cpu_hlt" => Some(Self::CpuHlt),
            "cpu_pause" => Some(Self::CpuPause),
            "mem_copy" => Some(Self::MemCopy),
            "mem_set" => Some(Self::MemSet),
            "mem_zero" => Some(Self::MemZero),
            "lidt" => Some(Self::Lidt),
            "read_eflags" => Some(Self::ReadEflags),
            "write_eflags" => Some(Self::WriteEflags),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::VolatileLoad8 => "volatile_load8",
            Self::VolatileStore8 => "volatile_store8",
            Self::VolatileLoad16 => "volatile_load16",
            Self::VolatileStore16 => "volatile_store16",
            Self::VolatileLoad32 => "volatile_load32",
            Self::VolatileStore32 => "volatile_store32",
            Self::IoOut8 => "io_out8",
            Self::IoIn8 => "io_in8",
            Self::CpuCli => "cpu_cli",
            Self::CpuSti => "cpu_sti",
            Self::CpuHlt => "cpu_hlt",
            Self::CpuPause => "cpu_pause",
            Self::MemCopy => "mem_copy",
            Self::MemSet => "mem_set",
            Self::MemZero => "mem_zero",
            Self::Lidt => "lidt",
            Self::ReadEflags => "read_eflags",
            Self::WriteEflags => "write_eflags",
        }
    }

    pub const fn qualified_name(self) -> &'static str {
        match self {
            Self::VolatileLoad8 => "native.volatile_load8",
            Self::VolatileStore8 => "native.volatile_store8",
            Self::VolatileLoad16 => "native.volatile_load16",
            Self::VolatileStore16 => "native.volatile_store16",
            Self::VolatileLoad32 => "native.volatile_load32",
            Self::VolatileStore32 => "native.volatile_store32",
            Self::IoOut8 => "native.io_out8",
            Self::IoIn8 => "native.io_in8",
            Self::CpuCli => "native.cpu_cli",
            Self::CpuSti => "native.cpu_sti",
            Self::CpuHlt => "native.cpu_hlt",
            Self::CpuPause => "native.cpu_pause",
            Self::MemCopy => "native.mem_copy",
            Self::MemSet => "native.mem_set",
            Self::MemZero => "native.mem_zero",
            Self::Lidt => "native.lidt",
            Self::ReadEflags => "native.read_eflags",
            Self::WriteEflags => "native.write_eflags",
        }
    }

    pub const fn parameter_types(self) -> &'static [NativeType] {
        const ADDRESS: &[NativeType] = &[NativeType::I32];
        const ADDRESS_VALUE: &[NativeType] = &[NativeType::I32, NativeType::I32];
        const THREE_I32: &[NativeType] = &[NativeType::I32, NativeType::I32, NativeType::I32];
        const NONE: &[NativeType] = &[];

        match self {
            Self::VolatileLoad8
            | Self::VolatileLoad16
            | Self::VolatileLoad32
            | Self::IoIn8
            | Self::Lidt
            | Self::WriteEflags => ADDRESS,
            Self::VolatileStore8
            | Self::VolatileStore16
            | Self::VolatileStore32
            | Self::IoOut8
            | Self::MemZero => ADDRESS_VALUE,
            Self::MemCopy | Self::MemSet => THREE_I32,
            Self::CpuCli | Self::CpuSti | Self::CpuHlt | Self::CpuPause | Self::ReadEflags => NONE,
        }
    }

    pub const fn argument_count(self) -> usize {
        self.parameter_types().len()
    }

    pub const fn result_type(self) -> Option<NativeType> {
        match self {
            Self::VolatileLoad8
            | Self::VolatileLoad16
            | Self::VolatileLoad32
            | Self::IoIn8
            | Self::ReadEflags => Some(NativeType::I32),
            Self::VolatileStore8
            | Self::VolatileStore16
            | Self::VolatileStore32
            | Self::IoOut8
            | Self::CpuCli
            | Self::CpuSti
            | Self::CpuHlt
            | Self::CpuPause
            | Self::MemCopy
            | Self::MemSet
            | Self::MemZero
            | Self::Lidt
            | Self::WriteEflags => None,
        }
    }
}

impl fmt::Display for Intrinsic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.qualified_name())
    }
}
