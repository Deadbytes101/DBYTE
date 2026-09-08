//! Bounded AMD64 instruction encoder used by DBYTE64 targets.
//!
//! Every public operation constructs its complete byte sequence before the
//! capacity check, so a rejected instruction cannot partially mutate output.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Register {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    Byte,
    Word,
    Dword,
    Qword,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    Equal,
    NotEqual,
    Below,
    BelowOrEqual,
    Above,
    AboveOrEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShiftKind {
    Left,
    LogicalRight,
    ArithmeticRight,
}

impl Condition {
    fn code(self) -> u8 {
        match self {
            Self::Equal => 0x4,
            Self::NotEqual => 0x5,
            Self::Below => 0x2,
            Self::BelowOrEqual => 0x6,
            Self::Above => 0x7,
            Self::AboveOrEqual => 0x3,
            Self::Less => 0xc,
            Self::LessOrEqual => 0xe,
            Self::Greater => 0xf,
            Self::GreaterOrEqual => 0xd,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    Capacity { needed: usize, remaining: usize },
    InvalidOperand(&'static str),
    RelativeRange,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Capacity { needed, remaining } => {
                write!(
                    f,
                    "encoder capacity needs {needed} bytes but only {remaining} remain"
                )
            }
            Self::InvalidOperand(message) => f.write_str(message),
            Self::RelativeRange => f.write_str("relative displacement does not fit i32"),
        }
    }
}

impl std::error::Error for EncodeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoder {
    bytes: Vec<u8>,
    limit: usize,
}

impl Encoder {
    pub fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(4096)),
            limit,
        }
    }

    pub fn position(&self) -> usize {
        self.bytes.len()
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn commit(&mut self, instruction: &[u8]) -> Result<(), EncodeError> {
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if instruction.len() > remaining {
            return Err(EncodeError::Capacity {
                needed: instruction.len(),
                remaining,
            });
        }
        self.bytes.extend_from_slice(instruction);
        Ok(())
    }

    pub fn mov_imm64(&mut self, dst: Register, value: u64) -> Result<(), EncodeError> {
        let mut instruction = Vec::with_capacity(10);
        instruction.push(rex(true, false, false, high(dst)));
        instruction.push(0xb8 + low(dst));
        instruction.extend_from_slice(&value.to_le_bytes());
        self.commit(&instruction)
    }

    /// Writes a zero-extended 32-bit immediate to a 64-bit general register.
    /// The 32-bit destination form is both architecturally exact and smaller
    /// than MOVABS when the canonical value fits in `u32`.
    pub fn mov_imm32(&mut self, dst: Register, value: u32) -> Result<(), EncodeError> {
        let mut instruction = Vec::with_capacity(6);
        if high(dst) {
            instruction.push(rex(false, false, false, true));
        }
        instruction.push(0xb8 + low(dst));
        instruction.extend_from_slice(&value.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn mov_reg(
        &mut self,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        let mut instruction = prefixes(width, high(src), false, high(dst));
        force_rex_for_low_byte_registers(&mut instruction, width, &[dst, src]);
        instruction.push(match width {
            Width::Byte => 0x88,
            Width::Word | Width::Dword | Width::Qword => 0x89,
        });
        instruction.push(modrm(0b11, low(src), low(dst)));
        self.commit(&instruction)
    }

    pub fn mov_reg_from_base_disp32(
        &mut self,
        dst: Register,
        base: Register,
        displacement: i32,
        width: Width,
    ) -> Result<(), EncodeError> {
        if width == Width::Byte {
            return Err(EncodeError::InvalidOperand(
                "byte loads require an explicit extension form",
            ));
        }
        let mut instruction = prefixes(width, high(dst), false, high(base));
        instruction.push(0x8b);
        instruction.push(modrm(0b10, low(dst), low(base)));
        if low(base) == 4 {
            instruction.push(0x24);
        }
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn mov_base_disp32_from_reg(
        &mut self,
        base: Register,
        displacement: i32,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        let mut instruction = prefixes(width, high(src), false, high(base));
        force_rex_for_low_byte_registers(&mut instruction, width, &[src]);
        instruction.push(if width == Width::Byte { 0x88 } else { 0x89 });
        instruction.push(modrm(0b10, low(src), low(base)));
        if low(base) == 4 {
            instruction.push(0x24);
        }
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn movzx_reg_from_base_disp32(
        &mut self,
        dst: Register,
        base: Register,
        displacement: i32,
        source_width: Width,
    ) -> Result<(), EncodeError> {
        let opcode = match source_width {
            Width::Byte => 0xb6,
            Width::Word => 0xb7,
            _ => {
                return Err(EncodeError::InvalidOperand(
                    "movzx memory source must be byte or word",
                ))
            }
        };
        let mut instruction = prefixes(Width::Qword, high(dst), false, high(base));
        instruction.extend_from_slice(&[0x0f, opcode, modrm(0b10, low(dst), low(base))]);
        if low(base) == 4 {
            instruction.push(0x24);
        }
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn lea_rip_relative_placeholder(&mut self, dst: Register) -> Result<usize, EncodeError> {
        let mut instruction = prefixes(Width::Qword, high(dst), false, false);
        instruction.extend_from_slice(&[0x8d, modrm(0, low(dst), 5)]);
        let immediate = self
            .bytes
            .len()
            .checked_add(instruction.len())
            .ok_or(EncodeError::RelativeRange)?;
        instruction.extend_from_slice(&0i32.to_le_bytes());
        self.commit(&instruction)?;
        Ok(immediate)
    }

    pub fn lea_base_disp32(
        &mut self,
        dst: Register,
        base: Register,
        displacement: i32,
    ) -> Result<(), EncodeError> {
        let mut instruction = prefixes(Width::Qword, high(dst), false, high(base));
        instruction.push(0x8d);
        instruction.push(modrm(0b10, low(dst), low(base)));
        if low(base) == 4 {
            instruction.push(0x24);
        }
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn sub_imm32(&mut self, dst: Register, value: u32) -> Result<(), EncodeError> {
        let mut instruction = prefixes(Width::Qword, false, false, high(dst));
        instruction.push(0x81);
        instruction.push(modrm(0b11, 5, low(dst)));
        instruction.extend_from_slice(&value.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn add_imm32(&mut self, dst: Register, value: u32) -> Result<(), EncodeError> {
        let mut instruction = prefixes(Width::Qword, false, false, high(dst));
        instruction.push(0x81);
        instruction.push(modrm(0b11, 0, low(dst)));
        instruction.extend_from_slice(&value.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn add_reg_imm(
        &mut self,
        dst: Register,
        value: u32,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.alu_reg_imm(dst, value, width, 0)
    }

    pub fn or_reg_imm(
        &mut self,
        dst: Register,
        value: u32,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.alu_reg_imm(dst, value, width, 1)
    }

    pub fn and_reg_imm(
        &mut self,
        dst: Register,
        value: u32,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.alu_reg_imm(dst, value, width, 4)
    }

    pub fn sub_reg_imm(
        &mut self,
        dst: Register,
        value: u32,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.alu_reg_imm(dst, value, width, 5)
    }

    pub fn xor_reg_imm(
        &mut self,
        dst: Register,
        value: u32,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.alu_reg_imm(dst, value, width, 6)
    }

    pub fn cmp_reg_imm(
        &mut self,
        dst: Register,
        value: u32,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.alu_reg_imm(dst, value, width, 7)
    }

    pub fn add_reg(
        &mut self,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.binary_reg(0x00, 0x01, dst, src, width)
    }

    pub fn cmp_reg(
        &mut self,
        left: Register,
        right: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.binary_reg(0x38, 0x39, left, right, width)
    }

    pub fn test_reg(
        &mut self,
        left: Register,
        right: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        let mut instruction = prefixes(width, high(right), false, high(left));
        force_rex_for_low_byte_registers(&mut instruction, width, &[left, right]);
        instruction.push(if width == Width::Byte { 0x84 } else { 0x85 });
        instruction.push(modrm(0b11, low(right), low(left)));
        self.commit(&instruction)
    }

    pub fn set_condition(
        &mut self,
        condition: Condition,
        destination: Register,
    ) -> Result<(), EncodeError> {
        if high(destination) || low(destination) > 3 {
            return Err(EncodeError::InvalidOperand(
                "setcc currently requires AL/CL/DL/BL",
            ));
        }
        self.commit(&[
            0x0f,
            0x90 + condition.code(),
            modrm(0b11, 0, low(destination)),
        ])
    }

    pub fn movzx_byte_to_qword(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncodeError> {
        if high(source) || low(source) > 3 {
            return Err(EncodeError::InvalidOperand(
                "byte source currently requires AL/CL/DL/BL",
            ));
        }
        let mut instruction = prefixes(Width::Qword, high(destination), false, high(source));
        instruction.extend_from_slice(&[0x0f, 0xb6, modrm(0b11, low(destination), low(source))]);
        self.commit(&instruction)
    }

    pub fn movzx_word_to_qword(
        &mut self,
        destination: Register,
        source: Register,
    ) -> Result<(), EncodeError> {
        let mut instruction = prefixes(Width::Qword, high(destination), false, high(source));
        instruction.extend_from_slice(&[0x0f, 0xb7, modrm(0b11, low(destination), low(source))]);
        self.commit(&instruction)
    }

    pub fn sub_reg(
        &mut self,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.binary_reg(0x28, 0x29, dst, src, width)
    }

    pub fn xor_reg(
        &mut self,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.binary_reg(0x30, 0x31, dst, src, width)
    }

    pub fn not_reg(&mut self, register: Register, width: Width) -> Result<(), EncodeError> {
        let mut instruction = prefixes(width, false, false, high(register));
        force_rex_for_low_byte_registers(&mut instruction, width, &[register]);
        instruction.push(if width == Width::Byte { 0xf6 } else { 0xf7 });
        instruction.push(modrm(0b11, 2, low(register)));
        self.commit(&instruction)
    }

    pub fn neg_reg(&mut self, register: Register, width: Width) -> Result<(), EncodeError> {
        let mut instruction = prefixes(width, false, false, high(register));
        force_rex_for_low_byte_registers(&mut instruction, width, &[register]);
        instruction.push(if width == Width::Byte { 0xf6 } else { 0xf7 });
        instruction.push(modrm(0b11, 3, low(register)));
        self.commit(&instruction)
    }

    pub fn shift_reg_by_cl(
        &mut self,
        register: Register,
        kind: ShiftKind,
        width: Width,
    ) -> Result<(), EncodeError> {
        let operation = match kind {
            ShiftKind::Left => 4,
            ShiftKind::LogicalRight => 5,
            ShiftKind::ArithmeticRight => 7,
        };
        let mut instruction = prefixes(width, false, false, high(register));
        force_rex_for_low_byte_registers(&mut instruction, width, &[register]);
        instruction.push(if width == Width::Byte { 0xd2 } else { 0xd3 });
        instruction.push(modrm(0b11, operation, low(register)));
        self.commit(&instruction)
    }

    pub fn and_reg(
        &mut self,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.binary_reg(0x20, 0x21, dst, src, width)
    }

    pub fn or_reg(
        &mut self,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        self.binary_reg(0x08, 0x09, dst, src, width)
    }

    fn binary_reg(
        &mut self,
        byte_opcode: u8,
        wide_opcode: u8,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        let mut instruction = prefixes(width, high(src), false, high(dst));
        force_rex_for_low_byte_registers(&mut instruction, width, &[dst, src]);
        instruction.push(if width == Width::Byte {
            byte_opcode
        } else {
            wide_opcode
        });
        instruction.push(modrm(0b11, low(src), low(dst)));
        self.commit(&instruction)
    }

    fn alu_reg_imm(
        &mut self,
        dst: Register,
        value: u32,
        width: Width,
        group: u8,
    ) -> Result<(), EncodeError> {
        if group > 7 {
            return Err(EncodeError::InvalidOperand("invalid ALU immediate group"));
        }
        let mut instruction = prefixes(width, false, false, high(dst));
        force_rex_for_low_byte_registers(&mut instruction, width, &[dst]);
        instruction.push(if width == Width::Byte { 0x80 } else { 0x81 });
        instruction.push(modrm(0b11, group, low(dst)));
        match width {
            Width::Byte => instruction.push(value as u8),
            Width::Word => instruction.extend_from_slice(&(value as u16).to_le_bytes()),
            Width::Dword | Width::Qword => instruction.extend_from_slice(&value.to_le_bytes()),
        }
        self.commit(&instruction)
    }

    pub fn imul_reg(
        &mut self,
        dst: Register,
        src: Register,
        width: Width,
    ) -> Result<(), EncodeError> {
        if width == Width::Byte {
            return Err(EncodeError::InvalidOperand(
                "two-operand imul does not accept byte width",
            ));
        }
        let mut instruction = prefixes(width, high(dst), false, high(src));
        instruction.extend_from_slice(&[0x0f, 0xaf, modrm(0b11, low(dst), low(src))]);
        self.commit(&instruction)
    }

    pub fn imul_reg_imm32(
        &mut self,
        destination: Register,
        source: Register,
        value: u32,
        width: Width,
    ) -> Result<(), EncodeError> {
        if width == Width::Byte {
            return Err(EncodeError::InvalidOperand(
                "three-operand imul does not accept byte width",
            ));
        }
        let mut instruction = prefixes(width, high(destination), false, high(source));
        instruction.push(0x69);
        instruction.push(modrm(0b11, low(destination), low(source)));
        match width {
            Width::Word => instruction.extend_from_slice(&(value as u16).to_le_bytes()),
            Width::Dword | Width::Qword => instruction.extend_from_slice(&value.to_le_bytes()),
            Width::Byte => unreachable!(),
        }
        self.commit(&instruction)
    }

    pub fn cqo(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x48, 0x99])
    }

    pub fn cdq(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x99])
    }

    pub fn div_reg(
        &mut self,
        divisor: Register,
        signed: bool,
        width: Width,
    ) -> Result<(), EncodeError> {
        if width == Width::Byte {
            return Err(EncodeError::InvalidOperand(
                "DBYTE64 division requires a 16/32/64-bit divisor",
            ));
        }
        let mut instruction = prefixes(width, false, false, high(divisor));
        instruction.push(0xf7);
        instruction.push(modrm(0b11, if signed { 7 } else { 6 }, low(divisor)));
        self.commit(&instruction)
    }

    pub fn push(&mut self, register: Register) -> Result<(), EncodeError> {
        if high(register) {
            self.commit(&[0x41, 0x50 + low(register)])
        } else {
            self.commit(&[0x50 + low(register)])
        }
    }

    pub fn pop(&mut self, register: Register) -> Result<(), EncodeError> {
        if high(register) {
            self.commit(&[0x41, 0x58 + low(register)])
        } else {
            self.commit(&[0x58 + low(register)])
        }
    }

    pub fn call_relative(&mut self, displacement: i64) -> Result<(), EncodeError> {
        let displacement = i32::try_from(displacement).map_err(|_| EncodeError::RelativeRange)?;
        let mut instruction = vec![0xe8];
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn call_relative_placeholder(&mut self) -> Result<usize, EncodeError> {
        let immediate = self
            .bytes
            .len()
            .checked_add(1)
            .ok_or(EncodeError::RelativeRange)?;
        self.commit(&[0xe8, 0, 0, 0, 0])?;
        Ok(immediate)
    }

    /// Emits `call qword ptr [rip + disp32]` and returns the displacement offset.
    pub fn call_rip_relative_placeholder(&mut self) -> Result<usize, EncodeError> {
        let immediate = self
            .bytes
            .len()
            .checked_add(2)
            .ok_or(EncodeError::RelativeRange)?;
        self.commit(&[0xff, 0x15, 0, 0, 0, 0])?;
        Ok(immediate)
    }

    pub fn jump_relative(&mut self, displacement: i64) -> Result<(), EncodeError> {
        let displacement = i32::try_from(displacement).map_err(|_| EncodeError::RelativeRange)?;
        let mut instruction = vec![0xe9];
        instruction.extend_from_slice(&displacement.to_le_bytes());
        self.commit(&instruction)
    }

    pub fn jump_relative_placeholder(&mut self) -> Result<usize, EncodeError> {
        let immediate = self
            .bytes
            .len()
            .checked_add(1)
            .ok_or(EncodeError::RelativeRange)?;
        self.commit(&[0xe9, 0, 0, 0, 0])?;
        Ok(immediate)
    }

    pub fn jump_if_placeholder(&mut self, condition: Condition) -> Result<usize, EncodeError> {
        let immediate = self
            .bytes
            .len()
            .checked_add(2)
            .ok_or(EncodeError::RelativeRange)?;
        self.commit(&[0x0f, 0x80 + condition.code(), 0, 0, 0, 0])?;
        Ok(immediate)
    }

    pub fn patch_relative_i32(
        &mut self,
        immediate_offset: usize,
        destination: usize,
    ) -> Result<(), EncodeError> {
        let end = immediate_offset
            .checked_add(4)
            .ok_or(EncodeError::RelativeRange)?;
        if end > self.bytes.len() {
            return Err(EncodeError::InvalidOperand(
                "relative patch is outside encoded bytes",
            ));
        }
        let displacement = i64::try_from(destination)
            .ok()
            .and_then(|destination| {
                i64::try_from(end)
                    .ok()
                    .and_then(|end| destination.checked_sub(end))
            })
            .and_then(|value| i32::try_from(value).ok())
            .ok_or(EncodeError::RelativeRange)?;
        self.bytes[immediate_offset..end].copy_from_slice(&displacement.to_le_bytes());
        Ok(())
    }

    pub fn call_indirect(&mut self, register: Register) -> Result<(), EncodeError> {
        let mut instruction = prefixes(Width::Qword, false, false, high(register));
        instruction.push(0xff);
        instruction.push(modrm(0b11, 2, low(register)));
        self.commit(&instruction)
    }

    pub fn syscall(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x05])
    }

    pub fn ret(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xc3])
    }

    pub fn ud2(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x0b])
    }

    pub fn cli(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xfa])
    }

    pub fn sti(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xfb])
    }

    pub fn int3(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xcc])
    }

    pub fn software_interrupt(&mut self, vector: u8) -> Result<(), EncodeError> {
        self.commit(&[0xcd, vector])
    }

    pub fn pushfq(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x9c])
    }

    pub fn hlt(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xf4])
    }

    pub fn mov_rax_cr0(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x20, 0xc0])
    }

    pub fn mov_cr0_rax(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x22, 0xc0])
    }

    pub fn mov_rax_cr2(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x20, 0xd0])
    }

    pub fn mov_rax_cr3(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x20, 0xd8])
    }

    pub fn mov_cr3_rax(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x22, 0xd8])
    }

    pub fn invlpg_rax(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x01, 0x38])
    }

    pub fn lgdt_rax(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x01, 0x10])
    }

    pub fn lidt_rax(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x01, 0x18])
    }

    pub fn ltr_ax(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x00, 0xd8])
    }

    pub fn mov_rax_cr4(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x20, 0xe0])
    }

    pub fn mov_cr4_rax(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x22, 0xe0])
    }

    pub fn cpuid(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0xa2])
    }

    pub fn rdmsr(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x32])
    }

    pub fn wrmsr(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x0f, 0x30])
    }

    pub fn iretq(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x48, 0xcf])
    }

    pub fn in8(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xec])
    }

    pub fn in16(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x66, 0xed])
    }

    pub fn in32(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xed])
    }

    pub fn out8(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xee])
    }

    pub fn out16(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0x66, 0xef])
    }

    pub fn out32(&mut self) -> Result<(), EncodeError> {
        self.commit(&[0xef])
    }
}

fn low(register: Register) -> u8 {
    (register as u8) & 7
}

fn high(register: Register) -> bool {
    register as u8 >= 8
}

fn rex(w: bool, r: bool, x: bool, b: bool) -> u8 {
    0x40 | ((w as u8) << 3) | ((r as u8) << 2) | ((x as u8) << 1) | b as u8
}

fn prefixes(width: Width, r: bool, x: bool, b: bool) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2);
    if width == Width::Word {
        bytes.push(0x66);
    }
    let needs_rex = width == Width::Qword || r || x || b;
    if needs_rex {
        bytes.push(rex(width == Width::Qword, r, x, b));
    }
    bytes
}

fn force_rex_for_low_byte_registers(prefixes: &mut Vec<u8>, width: Width, registers: &[Register]) {
    if width == Width::Byte
        && prefixes.is_empty()
        && registers
            .iter()
            .copied()
            .any(|register| !high(register) && low(register) >= 4)
    {
        prefixes.push(rex(false, false, false, false));
    }
}

fn modrm(mode: u8, reg: u8, rm: u8) -> u8 {
    (mode << 6) | ((reg & 7) << 3) | (rm & 7)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_rex_registers_and_64_bit_arithmetic_exactly() {
        let mut encoder = Encoder::new(64);
        encoder
            .mov_imm64(Register::R10, 0x1122_3344_5566_7788)
            .unwrap();
        encoder
            .mov_reg(Register::Rax, Register::R10, Width::Qword)
            .unwrap();
        encoder
            .add_reg(Register::Rax, Register::R9, Width::Qword)
            .unwrap();
        encoder
            .imul_reg(Register::Rax, Register::Rcx, Width::Qword)
            .unwrap();
        encoder.cqo().unwrap();
        encoder.div_reg(Register::Rcx, true, Width::Qword).unwrap();
        assert_eq!(
            encoder.bytes(),
            &[
                0x49, 0xba, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x4c, 0x89, 0xd0, 0x4c,
                0x01, 0xc8, 0x48, 0x0f, 0xaf, 0xc1, 0x48, 0x99, 0x48, 0xf7, 0xf9,
            ]
        );
    }

    #[test]
    fn compact_zero_extended_immediates_are_exact_and_transactional() {
        let mut encoder = Encoder::new(11);
        encoder.mov_imm32(Register::Rax, 0x7856_3412).unwrap();
        encoder.mov_imm32(Register::R9, 0x1122_3344).unwrap();
        assert_eq!(
            encoder.bytes(),
            &[0xb8, 0x12, 0x34, 0x56, 0x78, 0x41, 0xb9, 0x44, 0x33, 0x22, 0x11]
        );
        let before = encoder.bytes().to_vec();
        assert!(matches!(
            encoder.mov_imm32(Register::Rdx, 1),
            Err(EncodeError::Capacity { .. })
        ));
        assert_eq!(encoder.bytes(), before);
    }

    #[test]
    fn port_io_byte_word_and_dword_forms_are_exact_and_transactional() {
        let mut encoder = Encoder::new(8);
        encoder.in8().unwrap();
        encoder.in16().unwrap();
        encoder.in32().unwrap();
        encoder.out8().unwrap();
        encoder.out16().unwrap();
        encoder.out32().unwrap();
        assert_eq!(
            encoder.bytes(),
            &[0xec, 0x66, 0xed, 0xed, 0xee, 0x66, 0xef, 0xef]
        );
        let before = encoder.bytes().to_vec();
        assert!(matches!(encoder.out32(), Err(EncodeError::Capacity { .. })));
        assert_eq!(encoder.bytes(), before);
    }

    #[test]
    fn privileged_control_forms_match_long_mode_oracle_and_are_transactional() {
        let mut encoder = Encoder::new(40);
        encoder.cli().unwrap();
        encoder.sti().unwrap();
        encoder.int3().unwrap();
        encoder.pushfq().unwrap();
        encoder.pop(Register::Rax).unwrap();
        encoder.mov_rax_cr0().unwrap();
        encoder.mov_cr0_rax().unwrap();
        encoder.mov_rax_cr2().unwrap();
        encoder.mov_rax_cr3().unwrap();
        encoder.mov_cr3_rax().unwrap();
        encoder.invlpg_rax().unwrap();
        encoder.lgdt_rax().unwrap();
        encoder.lidt_rax().unwrap();
        encoder.ltr_ax().unwrap();
        assert_eq!(
            encoder.bytes(),
            &[
                0xfa, 0xfb, 0xcc, 0x9c, 0x58, 0x0f, 0x20, 0xc0, 0x0f, 0x22, 0xc0, 0x0f, 0x20, 0xd0,
                0x0f, 0x20, 0xd8, 0x0f, 0x22, 0xd8, 0x0f, 0x01, 0x38, 0x0f, 0x01, 0x10, 0x0f, 0x01,
                0x18, 0x0f, 0x00, 0xd8,
            ]
        );
        let before = encoder.bytes().to_vec();
        let mut full = Encoder::new(before.len());
        full.commit(&before).unwrap();
        assert!(matches!(full.ltr_ax(), Err(EncodeError::Capacity { .. })));
        assert_eq!(full.bytes(), before);
    }

    #[test]
    fn g2_feature_and_msr_forms_match_long_mode_oracle() {
        let mut encoder = Encoder::new(32);
        encoder.mov_rax_cr4().unwrap();
        encoder.mov_cr4_rax().unwrap();
        encoder.cpuid().unwrap();
        encoder.rdmsr().unwrap();
        encoder.wrmsr().unwrap();
        encoder.iretq().unwrap();
        assert_eq!(
            encoder.bytes(),
            &[0x0f, 0x20, 0xe0, 0x0f, 0x22, 0xe0, 0x0f, 0xa2, 0x0f, 0x32, 0x0f, 0x30, 0x48, 0xcf,]
        );
    }

    #[test]
    fn extended_push_pop_call_and_syscall_are_exact() {
        let mut encoder = Encoder::new(32);
        encoder.push(Register::R12).unwrap();
        encoder.pop(Register::R15).unwrap();
        encoder.call_indirect(Register::R11).unwrap();
        encoder.syscall().unwrap();
        encoder.ret().unwrap();
        assert_eq!(
            encoder.bytes(),
            &[0x41, 0x54, 0x41, 0x5f, 0x49, 0xff, 0xd3, 0x0f, 0x05, 0xc3]
        );
    }

    #[test]
    fn frame_memory_rip_and_control_flow_are_exact() {
        let mut encoder = Encoder::new(128);
        encoder
            .mov_reg_from_base_disp32(Register::R12, Register::Rsp, 0x20, Width::Qword)
            .unwrap();
        encoder
            .mov_base_disp32_from_reg(Register::Rbp, -8, Register::R9, Width::Dword)
            .unwrap();
        let rip = encoder.lea_rip_relative_placeholder(Register::R10).unwrap();
        let call = encoder.call_relative_placeholder().unwrap();
        let branch = encoder.jump_if_placeholder(Condition::NotEqual).unwrap();
        let destination = encoder.position();
        encoder.patch_relative_i32(rip, destination).unwrap();
        encoder.patch_relative_i32(call, destination).unwrap();
        encoder.patch_relative_i32(branch, destination).unwrap();
        assert_eq!(
            encoder.bytes(),
            &[
                0x4c, 0x8b, 0xa4, 0x24, 0x20, 0x00, 0x00, 0x00, 0x44, 0x89, 0x8d, 0xf8, 0xff, 0xff,
                0xff, 0x4c, 0x8d, 0x15, 0x0b, 0x00, 0x00, 0x00, 0xe8, 0x06, 0x00, 0x00, 0x00, 0x0f,
                0x85, 0x00, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn rip_relative_import_call_is_exact_and_transactional() {
        let mut encoder = Encoder::new(6);
        let patch = encoder.call_rip_relative_placeholder().unwrap();
        assert_eq!(patch, 2);
        assert_eq!(encoder.bytes(), &[0xff, 0x15, 0, 0, 0, 0]);

        let before = encoder.bytes().to_vec();
        assert!(matches!(
            encoder.call_rip_relative_placeholder(),
            Err(EncodeError::Capacity { .. })
        ));
        assert_eq!(encoder.bytes(), before);
    }

    #[test]
    fn base_relative_address_is_exact() {
        let mut encoder = Encoder::new(16);
        encoder
            .lea_base_disp32(Register::R10, Register::Rsp, -32)
            .unwrap();
        assert_eq!(
            encoder.bytes(),
            &[0x4c, 0x8d, 0x94, 0x24, 0xe0, 0xff, 0xff, 0xff]
        );
    }

    #[test]
    fn byte_not_encodes_legacy_extended_and_rex_low_registers_exactly() {
        let mut encoder = Encoder::new(16);
        encoder.not_reg(Register::Rax, Width::Byte).unwrap();
        encoder.not_reg(Register::R9, Width::Byte).unwrap();
        encoder.not_reg(Register::Rsp, Width::Byte).unwrap();
        assert_eq!(
            encoder.bytes(),
            &[0xf6, 0xd0, 0x41, 0xf6, 0xd1, 0x40, 0xf6, 0xd4]
        );
    }

    #[test]
    fn byte_register_ops_force_rex_for_sil_and_dil() {
        let mut encoder = Encoder::new(64);
        encoder
            .mov_reg(Register::Rdi, Register::Rsi, Width::Byte)
            .unwrap();
        encoder
            .mov_base_disp32_from_reg(Register::Rax, 0, Register::Rsi, Width::Byte)
            .unwrap();
        encoder
            .test_reg(Register::Rdi, Register::Rsi, Width::Byte)
            .unwrap();
        encoder
            .cmp_reg(Register::Rdi, Register::Rsi, Width::Byte)
            .unwrap();
        encoder.cmp_reg_imm(Register::Rdi, 0, Width::Byte).unwrap();
        encoder.neg_reg(Register::Rsi, Width::Byte).unwrap();
        encoder
            .shift_reg_by_cl(Register::Rdi, ShiftKind::Left, Width::Byte)
            .unwrap();
        assert_eq!(
            encoder.bytes(),
            &[
                0x40, 0x88, 0xf7, 0x40, 0x88, 0xb0, 0, 0, 0, 0, 0x40, 0x84, 0xf7, 0x40, 0x38, 0xf7,
                0x40, 0x80, 0xff, 0, 0x40, 0xf6, 0xde, 0x40, 0xd2, 0xe7,
            ]
        );
    }

    #[test]
    fn variable_shifts_are_exact_and_transactional() {
        let mut encoder = Encoder::new(32);
        encoder
            .shift_reg_by_cl(Register::Rax, ShiftKind::Left, Width::Qword)
            .unwrap();
        encoder
            .shift_reg_by_cl(Register::R9, ShiftKind::LogicalRight, Width::Dword)
            .unwrap();
        encoder
            .shift_reg_by_cl(Register::Rdx, ShiftKind::ArithmeticRight, Width::Word)
            .unwrap();
        encoder
            .shift_reg_by_cl(Register::Rax, ShiftKind::Left, Width::Byte)
            .unwrap();
        encoder
            .shift_reg_by_cl(Register::R9, ShiftKind::LogicalRight, Width::Byte)
            .unwrap();
        encoder
            .shift_reg_by_cl(Register::Rdx, ShiftKind::ArithmeticRight, Width::Byte)
            .unwrap();
        assert_eq!(
            encoder.bytes(),
            &[
                0x48, 0xd3, 0xe0, 0x41, 0xd3, 0xe9, 0x66, 0xd3, 0xfa, 0xd2, 0xe0, 0x41, 0xd2, 0xe9,
                0xd2, 0xfa,
            ]
        );

        let mut full = Encoder::new(2);
        let before = full.bytes().to_vec();
        assert!(matches!(
            full.shift_reg_by_cl(Register::Rax, ShiftKind::Left, Width::Qword),
            Err(EncodeError::Capacity { .. })
        ));
        assert_eq!(full.bytes(), before);
    }

    #[test]
    fn alu_immediates_are_width_correct_and_transactional() {
        let mut encoder = Encoder::new(64);
        encoder.add_reg_imm(Register::Rax, 1, Width::Qword).unwrap();
        encoder.sub_reg_imm(Register::Rax, 2, Width::Dword).unwrap();
        encoder.and_reg_imm(Register::Rax, 3, Width::Word).unwrap();
        encoder.or_reg_imm(Register::Rax, 4, Width::Byte).unwrap();
        encoder.xor_reg_imm(Register::R9, 5, Width::Qword).unwrap();
        encoder.cmp_reg_imm(Register::R10, 6, Width::Dword).unwrap();
        encoder
            .imul_reg_imm32(Register::Rax, Register::Rax, 7, Width::Qword)
            .unwrap();
        assert_eq!(
            encoder.bytes(),
            &[
                0x48, 0x81, 0xc0, 1, 0, 0, 0, 0x81, 0xe8, 2, 0, 0, 0, 0x66, 0x81, 0xe0, 3, 0, 0x80,
                0xc8, 4, 0x49, 0x81, 0xf1, 5, 0, 0, 0, 0x41, 0x81, 0xfa, 6, 0, 0, 0, 0x48, 0x69,
                0xc0, 7, 0, 0, 0,
            ]
        );

        let mut full = Encoder::new(6);
        let before = full.bytes().to_vec();
        assert!(matches!(
            full.add_reg_imm(Register::Rax, 1, Width::Qword),
            Err(EncodeError::Capacity { .. })
        ));
        assert_eq!(full.bytes(), before);
        assert!(matches!(
            full.imul_reg_imm32(Register::Rax, Register::Rax, 1, Width::Byte),
            Err(EncodeError::InvalidOperand(_))
        ));
        assert_eq!(full.bytes(), before);
    }

    #[test]
    fn invalid_relative_patch_is_transactional() {
        let mut encoder = Encoder::new(16);
        let patch = encoder.call_relative_placeholder().unwrap();
        let before = encoder.bytes().to_vec();
        assert!(matches!(
            encoder.patch_relative_i32(patch + 8, 0),
            Err(EncodeError::InvalidOperand(_))
        ));
        assert_eq!(encoder.bytes(), before);
    }

    #[test]
    fn encodes_software_interrupt_exactly() {
        let mut encoder = Encoder::new(8);
        encoder.software_interrupt(0x80).unwrap();
        assert_eq!(encoder.bytes(), &[0xcd, 0x80]);
    }

    #[test]
    fn every_failure_is_transactional() {
        let mut encoder = Encoder::new(2);
        encoder.syscall().unwrap();
        let before = encoder.bytes().to_vec();
        assert!(matches!(
            encoder.mov_imm64(Register::Rax, 1),
            Err(EncodeError::Capacity { .. })
        ));
        assert_eq!(encoder.bytes(), before);
        assert!(matches!(
            encoder.div_reg(Register::Rax, true, Width::Byte),
            Err(EncodeError::InvalidOperand(_))
        ));
        assert_eq!(encoder.bytes(), before);
    }
}
