use crate::{IntegerType, TargetDataLayout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntegerValue {
    bits: u64,
    ty: IntegerType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerError {
    LiteralOutOfRange,
    TypeMismatch,
    DivisionByZero,
    SignedDivisionOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    ShiftRight,
}

impl IntegerValue {
    pub fn from_literal(
        value: i128,
        ty: IntegerType,
        layout: TargetDataLayout,
    ) -> Result<Self, IntegerError> {
        let width = bit_width(ty, layout);
        let signed = is_signed(ty);
        let fits = if signed {
            let minimum = -(1i128 << (width - 1));
            let maximum = (1i128 << (width - 1)) - 1;
            (minimum..=maximum).contains(&value)
        } else {
            value >= 0 && (value as u128) <= unsigned_max(width) as u128
        };
        if !fits {
            return Err(IntegerError::LiteralOutOfRange);
        }
        Ok(Self {
            bits: normalize(value as u64, width),
            ty,
        })
    }

    pub fn from_bits(bits: u64, ty: IntegerType, layout: TargetDataLayout) -> Self {
        Self {
            bits: normalize(bits, bit_width(ty, layout)),
            ty,
        }
    }

    pub fn bits(self) -> u64 {
        self.bits
    }

    pub fn ty(self) -> IntegerType {
        self.ty
    }

    pub fn signed(self, layout: TargetDataLayout) -> i64 {
        sign_extend(self.bits, bit_width(self.ty, layout))
    }

    pub fn binary(
        self,
        op: IntegerBinaryOp,
        rhs: Self,
        layout: TargetDataLayout,
    ) -> Result<Self, IntegerError> {
        if self.ty != rhs.ty {
            return Err(IntegerError::TypeMismatch);
        }
        let width = bit_width(self.ty, layout);
        let shift_mask = if width == 64 { 63 } else { 31 };
        let bits = match op {
            IntegerBinaryOp::Add => self.bits.wrapping_add(rhs.bits),
            IntegerBinaryOp::Sub => self.bits.wrapping_sub(rhs.bits),
            IntegerBinaryOp::Mul => self.bits.wrapping_mul(rhs.bits),
            IntegerBinaryOp::BitAnd => self.bits & rhs.bits,
            IntegerBinaryOp::BitOr => self.bits | rhs.bits,
            IntegerBinaryOp::BitXor => self.bits ^ rhs.bits,
            IntegerBinaryOp::ShiftLeft => self.bits.wrapping_shl((rhs.bits & shift_mask) as u32),
            IntegerBinaryOp::ShiftRight if is_signed(self.ty) => {
                (self.signed(layout) >> ((rhs.bits & shift_mask) as u32)) as u64
            }
            IntegerBinaryOp::ShiftRight => self.bits >> ((rhs.bits & shift_mask) as u32),
            IntegerBinaryOp::Div | IntegerBinaryOp::Rem if rhs.bits == 0 => {
                return Err(IntegerError::DivisionByZero)
            }
            IntegerBinaryOp::Div | IntegerBinaryOp::Rem if is_signed(self.ty) => {
                let left = self.signed(layout);
                let right = rhs.signed(layout);
                let minimum = if width == 64 {
                    i64::MIN
                } else {
                    -(1i64 << (width - 1))
                };
                if left == minimum && right == -1 {
                    return Err(IntegerError::SignedDivisionOverflow);
                }
                if op == IntegerBinaryOp::Div {
                    (left / right) as u64
                } else {
                    (left % right) as u64
                }
            }
            IntegerBinaryOp::Div => self.bits / rhs.bits,
            IntegerBinaryOp::Rem => self.bits % rhs.bits,
        };
        Ok(Self::from_bits(bits, self.ty, layout))
    }
}

pub const fn bit_width(ty: IntegerType, layout: TargetDataLayout) -> u32 {
    match ty {
        IntegerType::I8 | IntegerType::U8 => 8,
        IntegerType::I16 | IntegerType::U16 => 16,
        IntegerType::I32 | IntegerType::U32 | IntegerType::LegacyInt => 32,
        IntegerType::I64 | IntegerType::U64 => 64,
        IntegerType::Usize => layout.usize_size * 8,
    }
}

pub const fn is_signed(ty: IntegerType) -> bool {
    matches!(
        ty,
        IntegerType::I8
            | IntegerType::I16
            | IntegerType::I32
            | IntegerType::I64
            | IntegerType::LegacyInt
    )
}

const fn unsigned_max(width: u32) -> u64 {
    if width == 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

const fn normalize(value: u64, width: u32) -> u64 {
    value & unsigned_max(width)
}

const fn sign_extend(value: u64, width: u32) -> i64 {
    if width == 64 {
        value as i64
    } else {
        let shift = 64 - width;
        ((value << shift) as i64) >> shift
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(value: i128, ty: IntegerType, layout: TargetDataLayout) -> IntegerValue {
        IntegerValue::from_literal(value, ty, layout).unwrap()
    }

    #[test]
    fn i64_division_remainder_and_faults_are_exact() {
        let layout = TargetDataLayout::X86_64;
        let left = value(-17, IntegerType::I64, layout);
        let right = value(5, IntegerType::I64, layout);
        assert_eq!(
            left.binary(IntegerBinaryOp::Div, right, layout)
                .unwrap()
                .signed(layout),
            -3
        );
        assert_eq!(
            left.binary(IntegerBinaryOp::Rem, right, layout)
                .unwrap()
                .signed(layout),
            -2
        );
        assert_eq!(
            value(i64::MIN as i128, IntegerType::I64, layout).binary(
                IntegerBinaryOp::Div,
                value(-1, IntegerType::I64, layout),
                layout
            ),
            Err(IntegerError::SignedDivisionOverflow)
        );
        assert_eq!(
            left.binary(
                IntegerBinaryOp::Div,
                value(0, IntegerType::I64, layout),
                layout
            ),
            Err(IntegerError::DivisionByZero)
        );
    }

    #[test]
    fn u64_preserves_full_range_and_wraps() {
        let layout = TargetDataLayout::X86_64;
        let maximum =
            IntegerValue::from_literal(u64::MAX as i128, IntegerType::U64, layout).unwrap();
        let one = value(1, IntegerType::U64, layout);
        assert_eq!(
            maximum
                .binary(IntegerBinaryOp::Add, one, layout)
                .unwrap()
                .bits(),
            0
        );
        assert_eq!(
            one.binary(
                IntegerBinaryOp::ShiftLeft,
                value(64, IntegerType::U64, layout),
                layout
            )
            .unwrap()
            .bits(),
            1
        );
    }

    #[test]
    fn usize_width_follows_target_layout_not_host() {
        let over32 = u64::from(u32::MAX) + 1;
        assert_eq!(
            IntegerValue::from_literal(over32 as i128, IntegerType::Usize, TargetDataLayout::I686),
            Err(IntegerError::LiteralOutOfRange)
        );
        assert_eq!(
            IntegerValue::from_literal(
                over32 as i128,
                IntegerType::Usize,
                TargetDataLayout::X86_64
            )
            .unwrap()
            .bits(),
            over32
        );
    }
}
