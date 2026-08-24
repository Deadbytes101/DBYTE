//! Small, bounded i686 instruction encoder used by the freestanding backend.
//!
//! This is deliberately not a general-purpose assembler.  It only exposes the
//! instruction forms required by the native DByte lowering pass.  Relative
//! targets are symbolic until [`Encoder::finish`], which resolves every fixup
//! in emission order and rejects duplicate or unresolved labels.

use std::collections::BTreeMap;
use std::fmt;

/// Default upper bound for one generated machine-code buffer.
pub const DEFAULT_MAX_CODE_SIZE: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Register {
    Eax = 0,
    Ecx = 1,
    Edx = 2,
    Ebx = 3,
    Esp = 4,
    Ebp = 5,
    Esi = 6,
    Edi = 7,
}

impl Register {
    const fn code(self) -> u8 {
        self as u8
    }

    const fn has_legacy_byte_register(self) -> bool {
        matches!(self, Self::Eax | Self::Ecx | Self::Edx | Self::Ebx)
    }
}

impl TryFrom<u8> for Register {
    type Error = EncoderError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Eax),
            1 => Ok(Self::Ecx),
            2 => Ok(Self::Edx),
            3 => Ok(Self::Ebx),
            4 => Ok(Self::Esp),
            5 => Ok(Self::Ebp),
            6 => Ok(Self::Esi),
            7 => Ok(Self::Edi),
            other => Err(EncoderError::InvalidRegister(other)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label(u32);

impl Label {
    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Condition {
    Overflow = 0x0,
    NotOverflow = 0x1,
    Below = 0x2,
    AboveOrEqual = 0x3,
    Equal = 0x4,
    NotEqual = 0x5,
    BelowOrEqual = 0x6,
    Above = 0x7,
    Sign = 0x8,
    NotSign = 0x9,
    Parity = 0xa,
    NotParity = 0xb,
    Less = 0xc,
    GreaterOrEqual = 0xd,
    LessOrEqual = 0xe,
    Greater = 0xf,
}

impl Condition {
    const fn code(self) -> u8 {
        self as u8
    }
}

/// Width of a scalar register or memory operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandWidth {
    Byte,
    Word,
    Dword,
}

/// Extension applied while loading a narrow integer into a 32-bit register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Extension {
    Zero,
    Sign,
}

/// Scale encoded in a 32-bit SIB memory operand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Scale {
    One = 0,
    Two = 1,
    Four = 2,
    Eight = 3,
}

impl Scale {
    const fn code(self) -> u8 {
        self as u8
    }
}

/// A bounded i686 base/index/scale/displacement memory address.
///
/// `ESP` is valid as the base but not as the index because the corresponding
/// SIB index encoding means "no index" on i686.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryOperand {
    pub base: Register,
    pub index: Option<(Register, Scale)>,
    pub displacement: i32,
}

impl MemoryOperand {
    pub const fn new(base: Register, displacement: i32) -> Self {
        Self {
            base,
            index: None,
            displacement,
        }
    }

    pub const fn indexed(base: Register, index: Register, scale: Scale, displacement: i32) -> Self {
        Self {
            base,
            index: Some((index, scale)),
            displacement,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShiftKind {
    LeftLogical,
    RightLogical,
    RightArithmetic,
}

impl ShiftKind {
    const fn opcode_extension(self) -> u8 {
        match self {
            Self::LeftLogical => 4,
            Self::RightLogical => 5,
            Self::RightArithmetic => 7,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FixupKind {
    CallRel32,
    JumpRel32,
    ConditionalRel32(Condition),
}

impl FixupKind {
    fn name(self) -> &'static str {
        match self {
            Self::CallRel32 => "call-rel32",
            Self::JumpRel32 => "jump-rel32",
            Self::ConditionalRel32(_) => "jcc-rel32",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixupTrace {
    pub kind: FixupKind,
    pub instruction_offset: u32,
    pub immediate_offset: u32,
    pub target: Label,
    pub target_offset: u32,
    pub displacement: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedCode {
    pub bytes: Vec<u8>,
    pub fixups: Vec<FixupTrace>,
}

impl EncodedCode {
    pub fn dump(&self) -> String {
        let mut output = String::new();
        for (row, chunk) in self.bytes.chunks(16).enumerate() {
            use fmt::Write;
            let _ = write!(output, "{:08x}:", row * 16);
            for byte in chunk {
                let _ = write!(output, " {byte:02x}");
            }
            output.push('\n');
        }
        for fixup in &self.fixups {
            use fmt::Write;
            let _ = writeln!(
                output,
                "fixup {} instruction=0x{:08x} immediate=0x{:08x} target=L{} target_offset=0x{:08x} displacement={}",
                fixup.kind.name(),
                fixup.instruction_offset,
                fixup.immediate_offset,
                fixup.target.index(),
                fixup.target_offset,
                fixup.displacement
            );
        }
        output
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncoderError {
    InvalidRegister(u8),
    InvalidByteRegister(Register),
    InvalidIndexRegister(Register),
    InvalidLoadExtension {
        width: OperandWidth,
        extension: Option<Extension>,
    },
    InvalidExtensionWidth(OperandWidth),
    BufferOverflow {
        limit: usize,
        attempted_len: usize,
    },
    LabelSpaceExhausted,
    UnknownLabel(Label),
    DuplicateLabel(Label),
    UnboundLabel(Label),
    OffsetOutOfRange(usize),
    Rel32OutOfRange {
        target: Label,
        origin: usize,
        destination: usize,
    },
}

impl fmt::Display for EncoderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRegister(value) => write!(formatter, "invalid i686 register {value}"),
            Self::InvalidByteRegister(register) => {
                write!(formatter, "register {register:?} has no encodable low byte")
            }
            Self::InvalidIndexRegister(register) => {
                write!(formatter, "register {register:?} cannot be a SIB index")
            }
            Self::InvalidLoadExtension { width, extension } => write!(
                formatter,
                "invalid {extension:?} extension for {width:?} memory load"
            ),
            Self::InvalidExtensionWidth(width) => {
                write!(formatter, "cannot extend a {width:?} register operand")
            }
            Self::BufferOverflow {
                limit,
                attempted_len,
            } => write!(
                formatter,
                "machine-code buffer limit {limit} exceeded by length {attempted_len}"
            ),
            Self::LabelSpaceExhausted => formatter.write_str("i686 label space exhausted"),
            Self::UnknownLabel(label) => write!(formatter, "unknown label L{}", label.index()),
            Self::DuplicateLabel(label) => {
                write!(formatter, "duplicate binding for label L{}", label.index())
            }
            Self::UnboundLabel(label) => write!(formatter, "unbound label L{}", label.index()),
            Self::OffsetOutOfRange(offset) => {
                write!(formatter, "machine-code offset {offset} exceeds u32")
            }
            Self::Rel32OutOfRange {
                target,
                origin,
                destination,
            } => write!(
                formatter,
                "rel32 target L{} from {origin} to {destination} is out of range",
                target.index()
            ),
        }
    }
}

impl std::error::Error for EncoderError {}

#[derive(Clone, Debug)]
struct PendingFixup {
    kind: FixupKind,
    instruction_offset: usize,
    immediate_offset: usize,
    target: Label,
}

#[derive(Clone, Debug)]
pub struct Encoder {
    bytes: Vec<u8>,
    max_size: usize,
    next_label: u32,
    labels: BTreeMap<Label, usize>,
    fixups: Vec<PendingFixup>,
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CODE_SIZE)
    }
}

impl Encoder {
    pub fn new(max_size: usize) -> Self {
        Self {
            bytes: Vec::new(),
            max_size,
            next_label: 0,
            labels: BTreeMap::new(),
            fixups: Vec::new(),
        }
    }

    pub fn position(&self) -> usize {
        self.bytes.len()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    pub fn new_label(&mut self) -> Result<Label, EncoderError> {
        let label = Label(self.next_label);
        self.next_label = self
            .next_label
            .checked_add(1)
            .ok_or(EncoderError::LabelSpaceExhausted)?;
        Ok(label)
    }

    pub fn bind_label(&mut self, label: Label) -> Result<(), EncoderError> {
        self.require_known_label(label)?;
        if self.labels.contains_key(&label) {
            return Err(EncoderError::DuplicateLabel(label));
        }
        self.labels.insert(label, self.bytes.len());
        Ok(())
    }

    pub fn emit_raw(&mut self, bytes: &[u8]) -> Result<(), EncoderError> {
        self.ensure_capacity(bytes.len())?;
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    pub fn nop(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x90])
    }

    pub fn cli(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xfa])
    }

    pub fn sti(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xfb])
    }

    pub fn int3(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xcc])
    }

    pub fn software_interrupt(&mut self, vector: u8) -> Result<(), EncoderError> {
        self.emit_raw(&[0xcd, vector])
    }

    pub fn ltr_ax(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0x00, 0xd8])
    }

    pub fn pushfd(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x9c])
    }

    /// Emit `POPFD` (`9D`) — pop EFLAGS from the stack.
    pub fn popfd(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x9d])
    }

    pub fn pushad(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x60])
    }

    pub fn popad(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x61])
    }

    pub fn iretd(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xcf])
    }

    pub fn hlt(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf4])
    }

    /// Emit `PAUSE` (`F3 90`) — spin-loop hint for busy-wait loops.
    ///
    /// On SMT processors this improves throughput by signalling a spin-wait;
    /// on older processors it degrades to a harmless `NOP`.
    pub fn pause(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf3, 0x90])
    }

    pub fn ret(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xc3])
    }

    pub fn lgdt(&mut self, source: MemoryOperand) -> Result<(), EncoderError> {
        let mut instruction = vec![0x0f, 0x01];
        encode_memory_operand(&mut instruction, 2, source)?;
        self.emit_raw(&instruction)
    }

    pub fn lidt(&mut self, source: MemoryOperand) -> Result<(), EncoderError> {
        let mut instruction = vec![0x0f, 0x01];
        encode_memory_operand(&mut instruction, 3, source)?;
        self.emit_raw(&instruction)
    }

    pub fn mov_eax_cr0(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0x20, 0xc0])
    }

    pub fn mov_cr0_eax(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0x22, 0xc0])
    }

    pub fn mov_eax_cr2(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0x20, 0xd0])
    }

    pub fn mov_eax_cr3(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0x20, 0xd8])
    }

    pub fn mov_cr3_eax(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0x22, 0xd8])
    }

    pub fn invlpg(&mut self, address: MemoryOperand) -> Result<(), EncoderError> {
        let mut instruction = vec![0x0f, 0x01];
        encode_memory_operand(&mut instruction, 7, address)?;
        self.emit_raw(&instruction)
    }

    pub fn mov_ax_imm16(&mut self, value: u16) -> Result<(), EncoderError> {
        let mut instruction = vec![0x66, 0xb8];
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    /// Loads AX into DS, ES, FS, GS, and SS in that order.
    pub fn load_data_segments_from_ax(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x8e, 0xd8, 0x8e, 0xc0, 0x8e, 0xe0, 0x8e, 0xe8, 0x8e, 0xd0])
    }

    /// Emits `jmp ptr16:32` with a placeholder offset and returns its byte offset.
    pub fn far_jump_imm32_placeholder(&mut self, selector: u16) -> Result<usize, EncoderError> {
        self.ensure_capacity(7)?;
        self.bytes.push(0xea);
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(&0u32.to_le_bytes());
        self.bytes.extend_from_slice(&selector.to_le_bytes());
        Ok(offset)
    }

    pub fn push_reg(&mut self, register: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0x50 + register.code()])
    }

    pub fn pop_reg(&mut self, register: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0x58 + register.code()])
    }

    pub fn push_imm32(&mut self, value: i32) -> Result<(), EncoderError> {
        let mut instruction = Vec::with_capacity(5);
        instruction.push(0x68);
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn mov_reg_imm32(&mut self, destination: Register, value: u32) -> Result<(), EncoderError> {
        let mut instruction = Vec::with_capacity(5);
        instruction.push(0xb8 + destination.code());
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    /// Emits `mov reg, imm32(0)` and returns the byte offset of the immediate.
    pub fn mov_reg_imm32_placeholder(
        &mut self,
        destination: Register,
    ) -> Result<usize, EncoderError> {
        let instruction_offset = self.position();
        self.emit_raw(&[0xb8 + destination.code(), 0, 0, 0, 0])?;
        Ok(instruction_offset + 1)
    }

    pub fn mov_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0x89, modrm(0b11, source.code(), destination.code())])
    }

    pub fn mov_reg_from_ebp_disp32(
        &mut self,
        destination: Register,
        displacement: i32,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x8b, modrm(0b10, destination.code(), Register::Ebp.code())];
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn mov_ebp_disp32_from_reg(
        &mut self,
        displacement: i32,
        source: Register,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x89, modrm(0b10, source.code(), Register::Ebp.code())];
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn mov_ebp_disp32_imm32(
        &mut self,
        displacement: i32,
        value: u32,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0xc7, modrm(0b10, 0, Register::Ebp.code())];
        instruction.extend_from_slice(&displacement.to_le_bytes());
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn mov_reg_from_base32(
        &mut self,
        destination: Register,
        base: Register,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x8b];
        encode_base_operand(&mut instruction, destination.code(), base);
        self.emit_raw(&instruction)
    }

    pub fn mov_base32_from_reg(
        &mut self,
        base: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x89];
        encode_base_operand(&mut instruction, source.code(), base);
        self.emit_raw(&instruction)
    }

    pub fn movzx_reg_from_base8(
        &mut self,
        destination: Register,
        base: Register,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x0f, 0xb6];
        encode_base_operand(&mut instruction, destination.code(), base);
        self.emit_raw(&instruction)
    }

    pub fn mov_base8_from_reg(
        &mut self,
        base: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        require_byte_register(source)?;
        let mut instruction = vec![0x88];
        encode_base_operand(&mut instruction, source.code(), base);
        self.emit_raw(&instruction)
    }

    /// Zero-extend a 16-bit word from `[base]` into a 32-bit register.
    ///
    /// Emits `MOVZX r32, WORD PTR [base]` (opcode `0F B7 /r`).
    pub fn movzx_reg_from_base16(
        &mut self,
        destination: Register,
        base: Register,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x0f, 0xb7];
        encode_base_operand(&mut instruction, destination.code(), base);
        self.emit_raw(&instruction)
    }

    /// Store the low 16 bits of `source` to `[base]`.
    ///
    /// Emits `MOV WORD PTR [base], r16` (operand-size prefix `66` + opcode `89 /r`).
    /// Any 32-bit register is accepted; only its low 16 bits are written.
    pub fn mov_base16_from_reg(
        &mut self,
        base: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x66, 0x89];
        encode_base_operand(&mut instruction, source.code(), base);
        self.emit_raw(&instruction)
    }

    /// Emit `REP MOVSB` — copy `ECX` bytes from `[ESI]` to `[EDI]`.
    ///
    /// The caller is responsible for loading ESI, EDI, ECX and for preserving
    /// any callee-saved registers that are clobbered.
    pub fn rep_movsb(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf3, 0xa4])
    }

    /// Emit `REP STOSB` — fill `ECX` bytes at `[EDI]` with `AL`.
    ///
    /// The caller is responsible for loading AL, EDI, ECX and for preserving
    /// any callee-saved registers that are clobbered.
    pub fn rep_stosb(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf3, 0xaa])
    }

    /// Deterministic compiler-generated trap for an unreachable canonical HIR
    /// edge. This is not exposed as a source intrinsic.
    pub fn ud2(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0x0b])
    }

    pub fn movzx_reg_from_base_index8(
        &mut self,
        destination: Register,
        base: Register,
        index: Register,
    ) -> Result<(), EncoderError> {
        if index == Register::Esp {
            return Err(EncoderError::InvalidIndexRegister(index));
        }
        let mut instruction = vec![0x0f, 0xb6];
        let mode = if base == Register::Ebp { 0b01 } else { 0b00 };
        instruction.push(modrm(mode, destination.code(), 0b100));
        instruction.push(sib(0, index.code(), base.code()));
        if mode == 0b01 {
            instruction.push(0);
        }
        self.emit_raw(&instruction)
    }

    /// Loads a scalar from memory into a 32-bit register.
    ///
    /// Byte and word loads require either zero or sign extension. A dword
    /// load is already register-sized and therefore requires `None`.
    pub fn mov_reg_from_memory(
        &mut self,
        destination: Register,
        source: MemoryOperand,
        width: OperandWidth,
        extension: Option<Extension>,
    ) -> Result<(), EncoderError> {
        let mut instruction = match (width, extension) {
            (OperandWidth::Byte, Some(Extension::Zero)) => vec![0x0f, 0xb6],
            (OperandWidth::Byte, Some(Extension::Sign)) => vec![0x0f, 0xbe],
            (OperandWidth::Word, Some(Extension::Zero)) => vec![0x0f, 0xb7],
            (OperandWidth::Word, Some(Extension::Sign)) => vec![0x0f, 0xbf],
            (OperandWidth::Dword, None) => vec![0x8b],
            _ => return Err(EncoderError::InvalidLoadExtension { width, extension }),
        };
        encode_memory_operand(&mut instruction, destination.code(), source)?;
        self.emit_raw(&instruction)
    }

    pub fn movzx_reg_from_memory(
        &mut self,
        destination: Register,
        source: MemoryOperand,
        width: OperandWidth,
    ) -> Result<(), EncoderError> {
        if width == OperandWidth::Dword {
            return Err(EncoderError::InvalidExtensionWidth(width));
        }
        self.mov_reg_from_memory(destination, source, width, Some(Extension::Zero))
    }

    pub fn movsx_reg_from_memory(
        &mut self,
        destination: Register,
        source: MemoryOperand,
        width: OperandWidth,
    ) -> Result<(), EncoderError> {
        if width == OperandWidth::Dword {
            return Err(EncoderError::InvalidExtensionWidth(width));
        }
        self.mov_reg_from_memory(destination, source, width, Some(Extension::Sign))
    }

    /// Stores the low byte, low word, or full dword of a register to memory.
    pub fn mov_memory_from_reg(
        &mut self,
        destination: MemoryOperand,
        source: Register,
        width: OperandWidth,
    ) -> Result<(), EncoderError> {
        let mut instruction = match width {
            OperandWidth::Byte => {
                require_byte_register(source)?;
                vec![0x88]
            }
            OperandWidth::Word => vec![0x66, 0x89],
            OperandWidth::Dword => vec![0x89],
        };
        encode_memory_operand(&mut instruction, source.code(), destination)?;
        self.emit_raw(&instruction)
    }

    pub fn lea_reg_from_memory(
        &mut self,
        destination: Register,
        source: MemoryOperand,
    ) -> Result<(), EncoderError> {
        let mut instruction = vec![0x8d];
        encode_memory_operand(&mut instruction, destination.code(), source)?;
        self.emit_raw(&instruction)
    }

    pub fn add_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0x01, modrm(0b11, source.code(), destination.code())])
    }

    pub fn sub_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0x29, modrm(0b11, source.code(), destination.code())])
    }

    pub fn and_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0x21, modrm(0b11, source.code(), destination.code())])
    }

    pub fn or_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0x09, modrm(0b11, source.code(), destination.code())])
    }

    pub fn xor_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0x31, modrm(0b11, source.code(), destination.code())])
    }

    pub fn imul_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0x0f, 0xaf, modrm(0b11, destination.code(), source.code())])
    }

    pub fn cdq(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x99])
    }

    pub fn div_reg(&mut self, divisor: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf7, modrm(0b11, 6, divisor.code())])
    }

    pub fn idiv_reg(&mut self, divisor: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf7, modrm(0b11, 7, divisor.code())])
    }

    pub fn neg_reg(&mut self, register: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf7, modrm(0b11, 3, register.code())])
    }

    pub fn not_reg(&mut self, register: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0xf7, modrm(0b11, 2, register.code())])
    }

    pub fn shift_reg_by_cl(
        &mut self,
        register: Register,
        kind: ShiftKind,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[0xd3, modrm(0b11, kind.opcode_extension(), register.code())])
    }

    pub fn shift_reg_imm8(
        &mut self,
        register: Register,
        kind: ShiftKind,
        amount: u8,
    ) -> Result<(), EncoderError> {
        self.emit_raw(&[
            0xc1,
            modrm(0b11, kind.opcode_extension(), register.code()),
            amount,
        ])
    }

    pub fn cmp_reg_reg(&mut self, left: Register, right: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0x39, modrm(0b11, right.code(), left.code())])
    }

    pub fn cmp_reg_imm32(&mut self, register: Register, value: i32) -> Result<(), EncoderError> {
        let mut instruction = vec![0x81, modrm(0b11, 7, register.code())];
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn test_reg_reg(&mut self, left: Register, right: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0x85, modrm(0b11, right.code(), left.code())])
    }

    pub fn setcc_reg8(
        &mut self,
        condition: Condition,
        destination: Register,
    ) -> Result<(), EncoderError> {
        require_byte_register(destination)?;
        self.emit_raw(&[
            0x0f,
            0x90 + condition.code(),
            modrm(0b11, 0, destination.code()),
        ])
    }

    pub fn movzx_reg_reg8(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncoderError> {
        require_byte_register(source)?;
        self.emit_raw(&[0x0f, 0xb6, modrm(0b11, destination.code(), source.code())])
    }

    pub fn movzx_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
        width: OperandWidth,
    ) -> Result<(), EncoderError> {
        self.mov_extend_reg_reg(destination, source, width, Extension::Zero)
    }

    pub fn movsx_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
        width: OperandWidth,
    ) -> Result<(), EncoderError> {
        self.mov_extend_reg_reg(destination, source, width, Extension::Sign)
    }

    pub fn add_reg_imm32(&mut self, register: Register, value: i32) -> Result<(), EncoderError> {
        let mut instruction = vec![0x81, modrm(0b11, 0, register.code())];
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn sub_reg_imm32(&mut self, register: Register, value: i32) -> Result<(), EncoderError> {
        let mut instruction = vec![0x81, modrm(0b11, 5, register.code())];
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn and_reg_imm32(&mut self, register: Register, value: u32) -> Result<(), EncoderError> {
        let mut instruction = vec![0x81, modrm(0b11, 4, register.code())];
        instruction.extend_from_slice(&value.to_le_bytes());
        self.emit_raw(&instruction)
    }

    pub fn add_esp_imm32(&mut self, value: u32) -> Result<(), EncoderError> {
        self.add_reg_imm32(Register::Esp, value as i32)
    }

    pub fn sub_esp_imm32(&mut self, value: u32) -> Result<(), EncoderError> {
        self.sub_reg_imm32(Register::Esp, value as i32)
    }

    pub fn call(&mut self, target: Label) -> Result<(), EncoderError> {
        self.emit_rel32(&[0xe8], target, FixupKind::CallRel32)
    }

    /// Emits an unresolved `call rel32` and returns its immediate offset.
    ///
    /// ELF `R_386_PC32` computes `S + A - P`. Because x86 measures a call
    /// displacement from the end of its four-byte immediate, the in-place REL
    /// addend is `-4` (`fc ff ff ff`).
    pub fn call_rel32_placeholder(&mut self) -> Result<usize, EncoderError> {
        let immediate_offset = self
            .position()
            .checked_add(1)
            .ok_or(EncoderError::OffsetOutOfRange(self.position()))?;
        self.emit_raw(&[0xe8, 0xfc, 0xff, 0xff, 0xff])?;
        Ok(immediate_offset)
    }

    pub fn call_reg(&mut self, register: Register) -> Result<(), EncoderError> {
        self.emit_raw(&[0xff, modrm(0b11, 2, register.code())])
    }

    pub fn jump(&mut self, target: Label) -> Result<(), EncoderError> {
        self.emit_rel32(&[0xe9], target, FixupKind::JumpRel32)
    }

    pub fn jump_if(&mut self, condition: Condition, target: Label) -> Result<(), EncoderError> {
        self.emit_rel32(
            &[0x0f, 0x80 + condition.code()],
            target,
            FixupKind::ConditionalRel32(condition),
        )
    }

    pub fn in_al_dx(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xec])
    }

    pub fn in_ax_dx(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x66, 0xed])
    }

    pub fn out_dx_al(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0xee])
    }

    pub fn out_dx_ax(&mut self) -> Result<(), EncoderError> {
        self.emit_raw(&[0x66, 0xef])
    }

    pub fn finish(self) -> Result<EncodedCode, EncoderError> {
        let mut bytes = self.bytes;
        let mut trace = Vec::with_capacity(self.fixups.len());
        for fixup in self.fixups {
            let destination = *self
                .labels
                .get(&fixup.target)
                .ok_or(EncoderError::UnboundLabel(fixup.target))?;
            let origin = fixup
                .immediate_offset
                .checked_add(4)
                .ok_or(EncoderError::OffsetOutOfRange(fixup.immediate_offset))?;
            let displacement_i64 = destination as i64 - origin as i64;
            let displacement =
                i32::try_from(displacement_i64).map_err(|_| EncoderError::Rel32OutOfRange {
                    target: fixup.target,
                    origin,
                    destination,
                })?;
            let immediate_end = fixup
                .immediate_offset
                .checked_add(4)
                .ok_or(EncoderError::OffsetOutOfRange(fixup.immediate_offset))?;
            let destination_bytes = bytes
                .get_mut(fixup.immediate_offset..immediate_end)
                .ok_or(EncoderError::OffsetOutOfRange(fixup.immediate_offset))?;
            destination_bytes.copy_from_slice(&displacement.to_le_bytes());
            trace.push(FixupTrace {
                kind: fixup.kind,
                instruction_offset: u32::try_from(fixup.instruction_offset)
                    .map_err(|_| EncoderError::OffsetOutOfRange(fixup.instruction_offset))?,
                immediate_offset: u32::try_from(fixup.immediate_offset)
                    .map_err(|_| EncoderError::OffsetOutOfRange(fixup.immediate_offset))?,
                target: fixup.target,
                target_offset: u32::try_from(destination)
                    .map_err(|_| EncoderError::OffsetOutOfRange(destination))?,
                displacement,
            });
        }
        Ok(EncodedCode {
            bytes,
            fixups: trace,
        })
    }

    fn emit_rel32(
        &mut self,
        opcode: &[u8],
        target: Label,
        kind: FixupKind,
    ) -> Result<(), EncoderError> {
        self.require_known_label(target)?;
        let instruction_offset = self.position();
        let instruction_len = opcode
            .len()
            .checked_add(4)
            .ok_or(EncoderError::BufferOverflow {
                limit: self.max_size,
                attempted_len: usize::MAX,
            })?;
        self.ensure_capacity(instruction_len)?;
        self.bytes.extend_from_slice(opcode);
        let immediate_offset = self.position();
        self.bytes.extend_from_slice(&[0, 0, 0, 0]);
        self.fixups.push(PendingFixup {
            kind,
            instruction_offset,
            immediate_offset,
            target,
        });
        Ok(())
    }

    fn mov_extend_reg_reg(
        &mut self,
        destination: Register,
        source: Register,
        width: OperandWidth,
        extension: Extension,
    ) -> Result<(), EncoderError> {
        let opcode = match (width, extension) {
            (OperandWidth::Byte, Extension::Zero) => 0xb6,
            (OperandWidth::Byte, Extension::Sign) => 0xbe,
            (OperandWidth::Word, Extension::Zero) => 0xb7,
            (OperandWidth::Word, Extension::Sign) => 0xbf,
            (OperandWidth::Dword, _) => {
                return Err(EncoderError::InvalidExtensionWidth(width));
            }
        };
        if width == OperandWidth::Byte {
            require_byte_register(source)?;
        }
        self.emit_raw(&[0x0f, opcode, modrm(0b11, destination.code(), source.code())])
    }

    fn require_known_label(&self, label: Label) -> Result<(), EncoderError> {
        if label.index() >= self.next_label {
            Err(EncoderError::UnknownLabel(label))
        } else {
            Ok(())
        }
    }

    fn ensure_capacity(&self, additional: usize) -> Result<(), EncoderError> {
        let attempted_len =
            self.bytes
                .len()
                .checked_add(additional)
                .ok_or(EncoderError::BufferOverflow {
                    limit: self.max_size,
                    attempted_len: usize::MAX,
                })?;
        if attempted_len > self.max_size {
            return Err(EncoderError::BufferOverflow {
                limit: self.max_size,
                attempted_len,
            });
        }
        Ok(())
    }
}

const fn modrm(mode: u8, reg: u8, rm: u8) -> u8 {
    ((mode & 0b11) << 6) | ((reg & 0b111) << 3) | (rm & 0b111)
}

const fn sib(scale: u8, index: u8, base: u8) -> u8 {
    ((scale & 0b11) << 6) | ((index & 0b111) << 3) | (base & 0b111)
}

fn require_byte_register(register: Register) -> Result<(), EncoderError> {
    if register.has_legacy_byte_register() {
        Ok(())
    } else {
        Err(EncoderError::InvalidByteRegister(register))
    }
}

fn encode_base_operand(output: &mut Vec<u8>, reg_field: u8, base: Register) {
    match base {
        Register::Esp => {
            output.push(modrm(0b00, reg_field, 0b100));
            output.push(sib(0, 0b100, Register::Esp.code()));
        }
        Register::Ebp => {
            output.push(modrm(0b01, reg_field, Register::Ebp.code()));
            output.push(0);
        }
        other => output.push(modrm(0b00, reg_field, other.code())),
    }
}

fn encode_memory_operand(
    output: &mut Vec<u8>,
    reg_field: u8,
    operand: MemoryOperand,
) -> Result<(), EncoderError> {
    if let Some((Register::Esp, _)) = operand.index {
        return Err(EncoderError::InvalidIndexRegister(Register::Esp));
    }

    let displacement_fits_i8 = i8::try_from(operand.displacement).is_ok();
    let mode = if operand.displacement == 0 && operand.base != Register::Ebp {
        0b00
    } else if displacement_fits_i8 {
        0b01
    } else {
        0b10
    };
    let uses_sib = operand.index.is_some() || operand.base == Register::Esp;
    let rm = if uses_sib { 0b100 } else { operand.base.code() };
    output.push(modrm(mode, reg_field, rm));

    if uses_sib {
        let (index, scale) = operand
            .index
            .map(|(index, scale)| (index.code(), scale.code()))
            .unwrap_or((0b100, Scale::One.code()));
        output.push(sib(scale, index, operand.base.code()));
    }

    match mode {
        0b00 => {}
        0b01 => output.push(operand.displacement as i8 as u8),
        0b10 => output.extend_from_slice(&operand.displacement.to_le_bytes()),
        _ => unreachable!("memory operand mode is selected internally"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finish(encoder: Encoder) -> Vec<u8> {
        encoder.finish().expect("encoding should finish").bytes
    }

    #[test]
    fn register_try_from_rejects_values_above_seven() {
        assert_eq!(Register::try_from(7), Ok(Register::Edi));
        assert_eq!(Register::try_from(8), Err(EncoderError::InvalidRegister(8)));
    }

    #[test]
    fn mov_push_and_pop_exact_bytes() {
        let mut encoder = Encoder::new(32);
        encoder.mov_reg_imm32(Register::Eax, 0x1234_5678).unwrap();
        encoder.push_reg(Register::Ecx).unwrap();
        encoder.pop_reg(Register::Edx).unwrap();
        assert_eq!(
            finish(encoder),
            vec![0xb8, 0x78, 0x56, 0x34, 0x12, 0x51, 0x5a]
        );
    }

    #[test]
    fn ebp_load_store_and_zero_exact_bytes() {
        let mut encoder = Encoder::new(64);
        encoder.mov_reg_from_ebp_disp32(Register::Eax, -4).unwrap();
        encoder.mov_ebp_disp32_from_reg(-8, Register::Ecx).unwrap();
        encoder.mov_ebp_disp32_imm32(-12, 0).unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0x8b, 0x85, 0xfc, 0xff, 0xff, 0xff, 0x89, 0x8d, 0xf8, 0xff, 0xff, 0xff, 0xc7, 0x85,
                0xf4, 0xff, 0xff, 0xff, 0, 0, 0, 0,
            ]
        );
    }

    #[test]
    fn arithmetic_forms_have_exact_bytes() {
        let mut encoder = Encoder::new(64);
        encoder.add_reg_reg(Register::Eax, Register::Ecx).unwrap();
        encoder.sub_reg_reg(Register::Eax, Register::Ecx).unwrap();
        encoder.imul_reg_reg(Register::Eax, Register::Ecx).unwrap();
        encoder.cmp_reg_reg(Register::Eax, Register::Ecx).unwrap();
        assert_eq!(
            finish(encoder),
            vec![0x01, 0xc8, 0x29, 0xc8, 0x0f, 0xaf, 0xc1, 0x39, 0xc8]
        );
    }

    #[test]
    fn c_abi_stack_alignment_mask_is_exact_and_transactional() {
        let mut encoder = Encoder::new(6);
        encoder.and_reg_imm32(Register::Esp, 0xfffffff0).unwrap();
        assert_eq!(finish(encoder), vec![0x81, 0xe4, 0xf0, 0xff, 0xff, 0xff]);

        let mut encoder = Encoder::new(5);
        let before = encoder.bytes().to_vec();
        assert!(encoder.and_reg_imm32(Register::Esp, 0xfffffff0).is_err());
        assert_eq!(encoder.bytes(), before);
    }

    #[test]
    fn division_forms_have_exact_bytes_and_are_transactional() {
        let mut encoder = Encoder::new(5);
        encoder.cdq().unwrap();
        encoder.div_reg(Register::Ecx).unwrap();
        encoder.idiv_reg(Register::Ecx).unwrap();
        assert_eq!(finish(encoder), vec![0x99, 0xf7, 0xf1, 0xf7, 0xf9]);

        let mut encoder = Encoder::new(1);
        encoder.cdq().unwrap();
        let before = encoder.bytes().to_vec();
        assert!(encoder.div_reg(Register::Ecx).is_err());
        assert_eq!(encoder.bytes(), before);
    }

    #[test]
    fn call_rel32_positive_displacement_is_resolved() {
        let mut encoder = Encoder::new(32);
        let target = encoder.new_label().unwrap();
        encoder.call(target).unwrap();
        encoder.hlt().unwrap();
        encoder.bind_label(target).unwrap();
        let encoded = encoder.finish().unwrap();
        assert_eq!(encoded.bytes, vec![0xe8, 1, 0, 0, 0, 0xf4]);
        assert_eq!(encoded.fixups[0].displacement, 1);
    }

    #[test]
    fn jump_rel32_negative_displacement_is_resolved() {
        let mut encoder = Encoder::new(32);
        let target = encoder.new_label().unwrap();
        encoder.bind_label(target).unwrap();
        encoder.hlt().unwrap();
        encoder.jump(target).unwrap();
        let encoded = encoder.finish().unwrap();
        assert_eq!(encoded.bytes, vec![0xf4, 0xe9, 0xfa, 0xff, 0xff, 0xff]);
        assert_eq!(encoded.fixups[0].displacement, -6);
    }

    #[test]
    fn conditional_rel32_fixup_has_exact_bytes() {
        let mut encoder = Encoder::new(32);
        let target = encoder.new_label().unwrap();
        encoder.jump_if(Condition::Equal, target).unwrap();
        encoder.hlt().unwrap();
        encoder.bind_label(target).unwrap();
        assert_eq!(finish(encoder), vec![0x0f, 0x84, 1, 0, 0, 0, 0xf4]);
    }

    #[test]
    fn duplicate_and_unbound_labels_are_errors() {
        let mut duplicate = Encoder::new(16);
        let label = duplicate.new_label().unwrap();
        duplicate.bind_label(label).unwrap();
        assert_eq!(
            duplicate.bind_label(label),
            Err(EncoderError::DuplicateLabel(label))
        );

        let mut unbound = Encoder::new(16);
        let label = unbound.new_label().unwrap();
        unbound.call(label).unwrap();
        assert_eq!(unbound.finish(), Err(EncoderError::UnboundLabel(label)));
    }

    #[test]
    fn buffer_overflow_is_atomic() {
        let mut encoder = Encoder::new(4);
        assert_eq!(
            encoder.mov_reg_imm32(Register::Eax, 1),
            Err(EncoderError::BufferOverflow {
                limit: 4,
                attempted_len: 5,
            })
        );
        assert!(encoder.bytes().is_empty());
    }

    #[test]
    fn byte_memory_and_port_io_forms_have_exact_bytes() {
        let mut encoder = Encoder::new(32);
        encoder
            .movzx_reg_from_base_index8(Register::Eax, Register::Ebx, Register::Ecx)
            .unwrap();
        encoder
            .mov_base8_from_reg(Register::Edx, Register::Eax)
            .unwrap();
        encoder.in_al_dx().unwrap();
        encoder.in_ax_dx().unwrap();
        encoder.out_dx_al().unwrap();
        encoder.out_dx_ax().unwrap();
        encoder.cli().unwrap();
        encoder.hlt().unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0x0f, 0xb6, 0x04, 0x0b, 0x88, 0x02, 0xec, 0x66, 0xed, 0xee, 0x66, 0xef, 0xfa, 0xf4,
            ]
        );
    }

    #[test]
    fn setcc_and_movzx_exact_bytes() {
        let mut encoder = Encoder::new(16);
        encoder.setcc_reg8(Condition::Less, Register::Eax).unwrap();
        encoder
            .movzx_reg_reg8(Register::Eax, Register::Eax)
            .unwrap();
        assert_eq!(finish(encoder), vec![0x0f, 0x9c, 0xc0, 0x0f, 0xb6, 0xc0]);
    }

    #[test]
    fn width_specific_memory_loads_have_exact_bytes() {
        let mut encoder = Encoder::new(64);
        encoder
            .mov_reg_from_memory(
                Register::Eax,
                MemoryOperand::new(Register::Ebx, 0),
                OperandWidth::Dword,
                None,
            )
            .unwrap();
        encoder
            .movzx_reg_from_memory(
                Register::Edx,
                MemoryOperand::new(Register::Ebp, -128),
                OperandWidth::Byte,
            )
            .unwrap();
        encoder
            .movsx_reg_from_memory(
                Register::Edi,
                MemoryOperand::indexed(Register::Esp, Register::Esi, Scale::Four, 127),
                OperandWidth::Word,
            )
            .unwrap();
        encoder
            .movzx_reg_from_memory(
                Register::Ecx,
                MemoryOperand::new(Register::Ebx, 128),
                OperandWidth::Word,
            )
            .unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0x8b, 0x03, 0x0f, 0xb6, 0x55, 0x80, 0x0f, 0xbf, 0x7c, 0xb4, 0x7f, 0x0f, 0xb7, 0x8b,
                0x80, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn width_specific_memory_stores_have_exact_bytes() {
        let mut encoder = Encoder::new(64);
        encoder
            .mov_memory_from_reg(
                MemoryOperand::indexed(Register::Edi, Register::Ecx, Scale::Two, -4),
                Register::Eax,
                OperandWidth::Byte,
            )
            .unwrap();
        encoder
            .mov_memory_from_reg(
                MemoryOperand::new(Register::Ebp, 0),
                Register::Esi,
                OperandWidth::Word,
            )
            .unwrap();
        encoder
            .mov_memory_from_reg(
                MemoryOperand::new(Register::Esp, 0x1234_5678),
                Register::Edx,
                OperandWidth::Dword,
            )
            .unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0x88, 0x44, 0x4f, 0xfc, 0x66, 0x89, 0x75, 0x00, 0x89, 0x94, 0x24, 0x78, 0x56, 0x34,
                0x12,
            ]
        );
    }

    #[test]
    fn register_extensions_lea_not_and_shifts_have_exact_bytes() {
        let mut encoder = Encoder::new(64);
        encoder
            .movsx_reg_reg(Register::Eax, Register::Ecx, OperandWidth::Byte)
            .unwrap();
        encoder
            .movzx_reg_reg(Register::Edx, Register::Esi, OperandWidth::Word)
            .unwrap();
        encoder
            .movsx_reg_reg(Register::Edi, Register::Ebx, OperandWidth::Word)
            .unwrap();
        encoder
            .lea_reg_from_memory(
                Register::Eax,
                MemoryOperand::indexed(Register::Ebx, Register::Ecx, Scale::Four, 8),
            )
            .unwrap();
        encoder.not_reg(Register::Edx).unwrap();
        encoder
            .shift_reg_by_cl(Register::Eax, ShiftKind::LeftLogical)
            .unwrap();
        encoder
            .shift_reg_imm8(Register::Ebx, ShiftKind::RightLogical, 7)
            .unwrap();
        encoder
            .shift_reg_by_cl(Register::Esi, ShiftKind::RightArithmetic)
            .unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0x0f, 0xbe, 0xc1, 0x0f, 0xb7, 0xd6, 0x0f, 0xbf, 0xfb, 0x8d, 0x44, 0x8b, 0x08, 0xf7,
                0xd2, 0xd3, 0xe0, 0xc1, 0xeb, 0x07, 0xd3, 0xfe,
            ]
        );
    }

    #[test]
    fn memory_displacement_boundaries_have_exact_bytes() {
        let mut encoder = Encoder::new(64);
        for displacement in [127, 128, -128, -129] {
            encoder
                .lea_reg_from_memory(
                    Register::Eax,
                    MemoryOperand::new(Register::Ebx, displacement),
                )
                .unwrap();
        }
        encoder
            .lea_reg_from_memory(Register::Eax, MemoryOperand::new(Register::Ebp, 0))
            .unwrap();
        encoder
            .lea_reg_from_memory(Register::Eax, MemoryOperand::new(Register::Esp, 0))
            .unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0x8d, 0x43, 0x7f, 0x8d, 0x83, 0x80, 0x00, 0x00, 0x00, 0x8d, 0x43, 0x80, 0x8d, 0x83,
                0x7f, 0xff, 0xff, 0xff, 0x8d, 0x45, 0x00, 0x8d, 0x04, 0x24,
            ]
        );
    }

    #[test]
    fn external_call_placeholder_contains_rel_addend() {
        let mut encoder = Encoder::new(16);
        encoder.nop().unwrap();
        assert_eq!(encoder.call_rel32_placeholder(), Ok(2));
        assert_eq!(finish(encoder), vec![0x90, 0xe8, 0xfc, 0xff, 0xff, 0xff]);
    }

    #[test]
    fn task_trampoline_and_resume_sequence_have_exact_bytes_and_are_atomic() {
        let mut encoder = Encoder::new(8);
        encoder.call_reg(Register::Eax).unwrap();
        encoder.ret().unwrap();
        encoder.mov_reg_reg(Register::Esp, Register::Eax).unwrap();
        encoder.popad().unwrap();
        encoder.iretd().unwrap();
        assert_eq!(
            finish(encoder),
            vec![0xff, 0xd0, 0xc3, 0x89, 0xc4, 0x61, 0xcf]
        );

        let mut bounded = Encoder::new(1);
        assert!(matches!(
            bounded.call_reg(Register::Eax),
            Err(EncoderError::BufferOverflow {
                limit: 1,
                attempted_len: 2
            })
        ));
        assert!(bounded.bytes().is_empty());
    }

    #[test]
    fn invalid_memory_and_width_forms_are_atomic() {
        let invalid_index = MemoryOperand::indexed(Register::Eax, Register::Esp, Scale::Eight, 0);
        let mut encoder = Encoder::new(16);
        assert_eq!(
            encoder.lea_reg_from_memory(Register::Eax, invalid_index),
            Err(EncoderError::InvalidIndexRegister(Register::Esp))
        );
        assert_eq!(
            encoder.mov_memory_from_reg(
                MemoryOperand::new(Register::Eax, 0),
                Register::Esi,
                OperandWidth::Byte,
            ),
            Err(EncoderError::InvalidByteRegister(Register::Esi))
        );
        assert_eq!(
            encoder.mov_reg_from_memory(
                Register::Eax,
                MemoryOperand::new(Register::Ebx, 0),
                OperandWidth::Dword,
                Some(Extension::Sign),
            ),
            Err(EncoderError::InvalidLoadExtension {
                width: OperandWidth::Dword,
                extension: Some(Extension::Sign),
            })
        );
        assert_eq!(
            encoder.movzx_reg_reg(Register::Eax, Register::Ecx, OperandWidth::Dword),
            Err(EncoderError::InvalidExtensionWidth(OperandWidth::Dword))
        );
        assert!(encoder.bytes().is_empty());

        let mut bounded = Encoder::new(3);
        assert_eq!(
            bounded.lea_reg_from_memory(
                Register::Eax,
                MemoryOperand::indexed(Register::Ebx, Register::Ecx, Scale::Four, 8),
            ),
            Err(EncoderError::BufferOverflow {
                limit: 3,
                attempted_len: 4,
            })
        );
        assert!(bounded.bytes().is_empty());
    }

    #[test]
    fn fixup_dump_is_stable_and_in_emission_order() {
        let mut encoder = Encoder::new(64);
        let first = encoder.new_label().unwrap();
        let second = encoder.new_label().unwrap();
        encoder.call(first).unwrap();
        encoder.jump(second).unwrap();
        encoder.bind_label(first).unwrap();
        encoder.hlt().unwrap();
        encoder.bind_label(second).unwrap();
        let dump = encoder.finish().unwrap().dump();
        let call = dump.find("fixup call-rel32").unwrap();
        let jump = dump.find("fixup jump-rel32").unwrap();
        assert!(call < jump);
        assert!(dump.contains("target=L0"));
        assert!(dump.contains("target=L1"));
    }

    #[test]
    fn privileged_interrupt_forms_have_exact_bytes() {
        let mut encoder = Encoder::new(64);
        encoder.cli().unwrap();
        encoder.sti().unwrap();
        encoder.hlt().unwrap();
        encoder.int3().unwrap();
        encoder.software_interrupt(0x81).unwrap();
        encoder.pushfd().unwrap();
        encoder.pushad().unwrap();
        encoder.popad().unwrap();
        encoder.lgdt(MemoryOperand::new(Register::Eax, 0)).unwrap();
        encoder.lidt(MemoryOperand::new(Register::Edx, 4)).unwrap();
        encoder.iretd().unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0xfa, 0xfb, 0xf4, 0xcc, 0xcd, 0x81, 0x9c, 0x60, 0x61, 0x0f, 0x01, 0x10, 0x0f, 0x01,
                0x5a, 0x04, 0xcf,
            ]
        );

        let mut bounded = Encoder::new(1);
        assert!(matches!(
            bounded.software_interrupt(0x81),
            Err(EncoderError::BufferOverflow {
                limit: 1,
                attempted_len: 2
            })
        ));
        assert!(bounded.bytes().is_empty());
    }

    #[test]
    fn ltr_has_exact_bytes_and_is_transactional() {
        let mut encoder = Encoder::new(3);
        encoder.ltr_ax().unwrap();
        assert_eq!(finish(encoder), vec![0x0f, 0x00, 0xd8]);

        let mut bounded = Encoder::new(2);
        assert!(matches!(
            bounded.ltr_ax(),
            Err(EncoderError::BufferOverflow {
                limit: 2,
                attempted_len: 3
            })
        ));
        assert!(bounded.bytes().is_empty());
    }

    #[test]
    fn paging_control_register_forms_have_exact_bytes_and_are_atomic() {
        let mut encoder = Encoder::new(64);
        encoder.mov_eax_cr0().unwrap();
        encoder.mov_cr0_eax().unwrap();
        encoder.mov_eax_cr2().unwrap();
        encoder.mov_eax_cr3().unwrap();
        encoder.mov_cr3_eax().unwrap();
        encoder
            .invlpg(MemoryOperand::new(Register::Eax, 0))
            .unwrap();
        encoder
            .invlpg(MemoryOperand::new(Register::Ebp, -4))
            .unwrap();
        assert_eq!(
            finish(encoder),
            vec![
                0x0f, 0x20, 0xc0, 0x0f, 0x22, 0xc0, 0x0f, 0x20, 0xd0, 0x0f, 0x20, 0xd8, 0x0f, 0x22,
                0xd8, 0x0f, 0x01, 0x38, 0x0f, 0x01, 0x7d, 0xfc,
            ]
        );

        let mut bounded = Encoder::new(2);
        assert!(matches!(
            bounded.mov_eax_cr0(),
            Err(EncoderError::BufferOverflow {
                limit: 2,
                attempted_len: 3
            })
        ));
        assert!(bounded.bytes().is_empty());
    }
}
