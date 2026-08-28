//! Target-independent call-frame planning for the DBYTE64, SysV64, and Win64
//! conventions. The encoder consumes this concrete plan and never re-infers
//! language types or ownership.

use crate::target::CAbi;
use crate::x86_64::Register;

pub const DBYTE64_SCALAR_REGISTERS: [Register; 6] = [
    Register::Rax,
    Register::Rdx,
    Register::Rcx,
    Register::R8,
    Register::R9,
    Register::R10,
];
pub const DBYTE64_CALLEE_SAVED: [Register; 6] = [
    Register::Rbx,
    Register::Rbp,
    Register::R12,
    Register::R13,
    Register::R14,
    Register::R15,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiValue {
    Scalar { bytes: u8 },
    Aggregate { size: u32, align: u32 },
}

impl AbiValue {
    pub fn scalar(bytes: u8) -> Result<Self, AbiError> {
        if matches!(bytes, 1 | 2 | 4 | 8) {
            Ok(Self::Scalar { bytes })
        } else {
            Err(AbiError::UnsupportedScalar(bytes))
        }
    }

    pub fn aggregate(size: u32, align: u32) -> Result<Self, AbiError> {
        if size == 0 || align == 0 || !align.is_power_of_two() || align > 4096 {
            Err(AbiError::InvalidAggregate { size, align })
        } else {
            Ok(Self::Aggregate { size, align })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiLocation {
    Register(Register),
    Stack { offset: u32, size: u32, align: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallPlan {
    /// Logical order: closure environment, sret, then source parameters.
    pub arguments: Vec<AbiLocation>,
    pub stack_argument_bytes: u32,
    pub shadow_space: u32,
    pub pre_call_padding: u32,
    pub total_stack_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiError {
    UnsupportedScalar(u8),
    InvalidAggregate { size: u32, align: u32 },
    AggregateInCAbi,
    StackOverflow,
}

pub fn plan_dbyte64_call(
    arguments: &[AbiValue],
    incoming_rsp_mod16: u8,
) -> Result<CallPlan, AbiError> {
    plan_call(
        arguments,
        &DBYTE64_SCALAR_REGISTERS,
        0,
        true,
        incoming_rsp_mod16,
    )
}

pub fn plan_c_call(
    abi: CAbi,
    arguments: &[AbiValue],
    incoming_rsp_mod16: u8,
) -> Result<CallPlan, AbiError> {
    let (registers, shadow) = match abi {
        CAbi::SysV64 => (
            &[
                Register::Rdi,
                Register::Rsi,
                Register::Rdx,
                Register::Rcx,
                Register::R8,
                Register::R9,
            ][..],
            0,
        ),
        CAbi::Win64 => (
            &[Register::Rcx, Register::Rdx, Register::R8, Register::R9][..],
            32,
        ),
        CAbi::I686Cdecl => return Err(AbiError::UnsupportedScalar(0)),
    };
    plan_call(arguments, registers, shadow, false, incoming_rsp_mod16)
}

fn plan_call(
    arguments: &[AbiValue],
    scalar_registers: &[Register],
    shadow_space: u32,
    aggregate_stack: bool,
    incoming_rsp_mod16: u8,
) -> Result<CallPlan, AbiError> {
    if incoming_rsp_mod16 >= 16 {
        return Err(AbiError::StackOverflow);
    }
    let mut scalar_index = 0usize;
    let mut locations = vec![
        AbiLocation::Stack {
            offset: 0,
            size: 0,
            align: 1
        };
        arguments.len()
    ];
    let mut stack_offset = shadow_space;
    for (index, argument) in arguments.iter().enumerate() {
        match *argument {
            AbiValue::Scalar { .. } if scalar_index < scalar_registers.len() => {
                locations[index] = AbiLocation::Register(scalar_registers[scalar_index]);
                scalar_index += 1;
            }
            AbiValue::Scalar { .. } => {
                stack_offset = align_up(stack_offset, 8)?;
                locations[index] = AbiLocation::Stack {
                    offset: stack_offset,
                    size: 8,
                    align: 8,
                };
                stack_offset = stack_offset.checked_add(8).ok_or(AbiError::StackOverflow)?;
            }
            AbiValue::Aggregate { size, align } if aggregate_stack => {
                let physical_align = align.max(8);
                stack_offset = align_up(stack_offset, physical_align)?;
                let physical_size = align_up(size, 8)?;
                locations[index] = AbiLocation::Stack {
                    offset: stack_offset,
                    size: physical_size,
                    align: physical_align,
                };
                stack_offset = stack_offset
                    .checked_add(physical_size)
                    .ok_or(AbiError::StackOverflow)?;
            }
            AbiValue::Aggregate { .. } => return Err(AbiError::AggregateInCAbi),
        }
    }

    let stack_argument_bytes = stack_offset
        .checked_sub(shadow_space)
        .ok_or(AbiError::StackOverflow)?;
    let without_padding = stack_offset;
    let current = (u32::from(incoming_rsp_mod16) + 16 - (without_padding % 16)) % 16;
    let pre_call_padding = current;
    let total_stack_bytes = without_padding
        .checked_add(pre_call_padding)
        .ok_or(AbiError::StackOverflow)?;
    Ok(CallPlan {
        arguments: locations,
        stack_argument_bytes,
        shadow_space,
        pre_call_padding,
        total_stack_bytes,
    })
}

fn align_up(value: u32, align: u32) -> Result<u32, AbiError> {
    let mask = align.checked_sub(1).ok_or(AbiError::StackOverflow)?;
    value
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(AbiError::StackOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbyte64_scalar_order_and_aggregate_stack_contract_are_exact() {
        let scalar = AbiValue::scalar(8).unwrap();
        let aggregate = AbiValue::aggregate(24, 8).unwrap();
        let plan = plan_dbyte64_call(
            &[
                scalar, scalar, aggregate, scalar, scalar, scalar, scalar, scalar,
            ],
            0,
        )
        .unwrap();
        assert_eq!(plan.arguments[0], AbiLocation::Register(Register::Rax));
        assert_eq!(plan.arguments[1], AbiLocation::Register(Register::Rdx));
        assert_eq!(
            plan.arguments[2],
            AbiLocation::Stack {
                offset: 0,
                size: 24,
                align: 8
            }
        );
        assert_eq!(plan.arguments[3], AbiLocation::Register(Register::Rcx));
        assert_eq!(plan.arguments[6], AbiLocation::Register(Register::R10));
        assert_eq!(
            plan.arguments[7],
            AbiLocation::Stack {
                offset: 24,
                size: 8,
                align: 8
            }
        );
        assert_eq!(plan.total_stack_bytes % 16, 0);
    }

    #[test]
    fn win64_reserves_shadow_space_and_sysv_does_not() {
        let args = vec![AbiValue::scalar(4).unwrap(); 6];
        let windows = plan_c_call(CAbi::Win64, &args, 0).unwrap();
        assert_eq!(windows.shadow_space, 32);
        assert_eq!(windows.arguments[0], AbiLocation::Register(Register::Rcx));
        assert_eq!(
            windows.arguments[4],
            AbiLocation::Stack {
                offset: 32,
                size: 8,
                align: 8
            }
        );
        assert_eq!(windows.total_stack_bytes % 16, 0);

        let sysv = plan_c_call(CAbi::SysV64, &args, 0).unwrap();
        assert_eq!(sysv.shadow_space, 0);
        assert_eq!(sysv.arguments[0], AbiLocation::Register(Register::Rdi));
        assert_eq!(sysv.arguments[5], AbiLocation::Register(Register::R9));
        assert_eq!(sysv.total_stack_bytes, 0);
    }

    #[test]
    fn c_abi_rejects_aggregate_by_value() {
        let error =
            plan_c_call(CAbi::SysV64, &[AbiValue::aggregate(16, 8).unwrap()], 0).unwrap_err();
        assert_eq!(error, AbiError::AggregateInCAbi);
    }
}
