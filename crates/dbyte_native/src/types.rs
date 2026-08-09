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
    VolatileLoad32,
    VolatileStore32,
    IoOut8,
    IoIn8,
    CpuHlt,
}

impl Intrinsic {
    /// Resolve either a bare member name (`io_out8`) or its fully-qualified
    /// spelling (`native.io_out8`).
    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix("native.").unwrap_or(name);
        match name {
            "volatile_load8" => Some(Self::VolatileLoad8),
            "volatile_store8" => Some(Self::VolatileStore8),
            "volatile_load32" => Some(Self::VolatileLoad32),
            "volatile_store32" => Some(Self::VolatileStore32),
            "io_out8" => Some(Self::IoOut8),
            "io_in8" => Some(Self::IoIn8),
            "cpu_hlt" => Some(Self::CpuHlt),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::VolatileLoad8 => "volatile_load8",
            Self::VolatileStore8 => "volatile_store8",
            Self::VolatileLoad32 => "volatile_load32",
            Self::VolatileStore32 => "volatile_store32",
            Self::IoOut8 => "io_out8",
            Self::IoIn8 => "io_in8",
            Self::CpuHlt => "cpu_hlt",
        }
    }

    pub const fn qualified_name(self) -> &'static str {
        match self {
            Self::VolatileLoad8 => "native.volatile_load8",
            Self::VolatileStore8 => "native.volatile_store8",
            Self::VolatileLoad32 => "native.volatile_load32",
            Self::VolatileStore32 => "native.volatile_store32",
            Self::IoOut8 => "native.io_out8",
            Self::IoIn8 => "native.io_in8",
            Self::CpuHlt => "native.cpu_hlt",
        }
    }

    pub const fn parameter_types(self) -> &'static [NativeType] {
        const ADDRESS: &[NativeType] = &[NativeType::I32];
        const ADDRESS_VALUE: &[NativeType] = &[NativeType::I32, NativeType::I32];
        const NONE: &[NativeType] = &[];

        match self {
            Self::VolatileLoad8 | Self::VolatileLoad32 | Self::IoIn8 => ADDRESS,
            Self::VolatileStore8 | Self::VolatileStore32 | Self::IoOut8 => ADDRESS_VALUE,
            Self::CpuHlt => NONE,
        }
    }

    pub const fn argument_count(self) -> usize {
        self.parameter_types().len()
    }

    pub const fn result_type(self) -> Option<NativeType> {
        match self {
            Self::VolatileLoad8 | Self::VolatileLoad32 | Self::IoIn8 => Some(NativeType::I32),
            Self::VolatileStore8 | Self::VolatileStore32 | Self::IoOut8 | Self::CpuHlt => None,
        }
    }
}

impl fmt::Display for Intrinsic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.qualified_name())
    }
}
