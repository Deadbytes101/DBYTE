//! Target-sized, semantics-free machine IR shared by DByte native backends.
//!
//! Concrete HIR remains the semantic authority. This layer owns only explicit
//! machine-width values, control flow, memory effects, ABI calls, and the data
//! needed by deterministic instruction selection and register allocation.

use dbyte_ast::{CallConvention, Span};
use dbyte_hir::FunctionInstanceId;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum NativeOptimizationLevel {
    O0,
    #[default]
    O1,
}

impl NativeOptimizationLevel {
    pub fn parse(value: &str) -> Result<Self, NativeMirError> {
        match value {
            "0" => Ok(Self::O0),
            "1" => Ok(Self::O1),
            _ => Err(NativeMirError::new(
                "E_NATIVE_OPT_LEVEL",
                format!("unsupported native optimization level '{value}'; expected 0 or 1"),
            )),
        }
    }
}

impl fmt::Display for NativeOptimizationLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::O0 => "O0",
            Self::O1 => "O1",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeMirError {
    pub code: &'static str,
    pub message: String,
}

impl NativeMirError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for NativeMirError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "error[{}]: {}", self.code, self.message)
    }
}

impl std::error::Error for NativeMirError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualRegister(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StackObjectId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAggregateLayout {
    pub bytes: u32,
    pub alignment: u32,
}

impl NativeAggregateLayout {
    pub const fn is_valid(self) -> bool {
        self.bytes != 0 && self.alignment != 0 && self.alignment.is_power_of_two()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MachineType {
    Bool,
    I8,
    I16,
    I32,
    I64,
    Pointer,
}

impl MachineType {
    pub const fn bytes(self, pointer_bytes: u8) -> u8 {
        match self {
            Self::Bool | Self::I8 => 1,
            Self::I16 => 2,
            Self::I32 => 4,
            Self::I64 => 8,
            Self::Pointer => pointer_bytes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeBinaryOp {
    Add,
    Sub,
    Mul,
    SignedDiv,
    UnsignedDiv,
    SignedRem,
    UnsignedRem,
    BitAnd,
    BitOr,
    BitXor,
    ShiftLeft,
    SignedShiftRight,
    UnsignedShiftRight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeUnaryOp {
    Negate,
    BitNot,
    BoolNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCompareOp {
    Equal,
    NotEqual,
    SignedLess,
    SignedLessEqual,
    SignedGreater,
    SignedGreaterEqual,
    UnsignedLess,
    UnsignedLessEqual,
    UnsignedGreater,
    UnsignedGreaterEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeValue {
    Register(VirtualRegister),
    Immediate(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCallee {
    Direct {
        symbol: String,
    },
    Indirect {
        address: VirtualRegister,
    },
    Closure {
        code: VirtualRegister,
        environment: VirtualRegister,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeMirInst {
    Const {
        destination: VirtualRegister,
        value: u64,
        ty: MachineType,
    },
    Copy {
        destination: VirtualRegister,
        source: VirtualRegister,
        ty: MachineType,
    },
    Binary {
        destination: VirtualRegister,
        op: NativeBinaryOp,
        left: NativeValue,
        right: NativeValue,
        ty: MachineType,
    },
    Unary {
        destination: VirtualRegister,
        op: NativeUnaryOp,
        value: NativeValue,
        ty: MachineType,
    },
    Cast {
        destination: VirtualRegister,
        value: VirtualRegister,
        from: MachineType,
        to: MachineType,
        signed: bool,
    },
    Compare {
        destination: VirtualRegister,
        op: NativeCompareOp,
        left: NativeValue,
        right: NativeValue,
        ty: MachineType,
    },
    StackAddress {
        destination: VirtualRegister,
        object: StackObjectId,
    },
    GlobalAddress {
        destination: VirtualRegister,
        symbol: String,
    },
    InterruptDefinition {
        class: dbyte_ast::InterruptClass,
    },
    InterruptAddress {
        destination: VirtualRegister,
        vector: u8,
        symbol: String,
        class: dbyte_ast::InterruptClass,
    },
    TaskAddress {
        destination: VirtualRegister,
        symbol: String,
    },
    AddressOffset {
        destination: VirtualRegister,
        base: VirtualRegister,
        index: Option<VirtualRegister>,
        scale: u32,
        displacement: i32,
    },
    Load {
        destination: VirtualRegister,
        address: VirtualRegister,
        ty: MachineType,
        volatile: bool,
    },
    Store {
        address: VirtualRegister,
        value: VirtualRegister,
        ty: MachineType,
        volatile: bool,
    },
    AggregateCopy {
        destination: VirtualRegister,
        source: VirtualRegister,
        bytes: u32,
        alignment: u32,
    },
    AggregateZero {
        destination: VirtualRegister,
        bytes: u32,
        alignment: u32,
    },
    Call {
        destination: Option<VirtualRegister>,
        callee: NativeCallee,
        arguments: Vec<VirtualRegister>,
        /// Aggregate arguments are address-valued virtual registers. A
        /// present layout makes selection create the ABI by-value stack copy.
        argument_aggregates: Vec<Option<NativeAggregateLayout>>,
        convention: CallConvention,
        result: Option<MachineType>,
        /// Aggregate results use destination as caller-owned storage and pass
        /// it through the canonical hidden sret parameter.
        aggregate_result: Option<NativeAggregateLayout>,
    },
    Intrinsic {
        destination: Option<VirtualRegister>,
        intrinsic: dbyte_hir::HirIntrinsic,
        arguments: Vec<VirtualRegister>,
        result: Option<MachineType>,
        volatile: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpannedNativeInst {
    pub instruction: NativeMirInst,
    pub span: Span,
}

impl From<NativeMirInst> for SpannedNativeInst {
    fn from(instruction: NativeMirInst) -> Self {
        Self {
            instruction,
            span: Span::zero(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeTerminator {
    Jump(BlockId),
    Branch {
        condition: NativeValue,
        then_block: BlockId,
        else_block: BlockId,
    },
    Return(Option<VirtualRegister>),
    ReturnAggregate {
        source: VirtualRegister,
        layout: NativeAggregateLayout,
    },
    Trap,
    Unreachable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeMirBlock {
    pub id: BlockId,
    pub instructions: Vec<SpannedNativeInst>,
    pub terminator: NativeTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeStackObject {
    pub id: StackObjectId,
    pub bytes: u32,
    pub alignment: u32,
    pub address_taken: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeMirFunction {
    pub instance: FunctionInstanceId,
    pub symbol: String,
    pub convention: CallConvention,
    pub parameters: Vec<VirtualRegister>,
    pub parameter_aggregates: Vec<Option<NativeAggregateLayout>>,
    pub result: Option<MachineType>,
    pub aggregate_result: Option<NativeAggregateLayout>,
    /// Address-valued virtual register bound to the hidden ABI sret pointer.
    pub aggregate_result_register: Option<VirtualRegister>,
    pub register_types: Vec<MachineType>,
    pub stack_objects: Vec<NativeStackObject>,
    pub entry: BlockId,
    pub blocks: Vec<NativeMirBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeMirProject {
    pub pointer_bytes: u8,
    pub globals: Vec<NativeMirGlobal>,
    pub functions: Vec<NativeMirFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeMirGlobal {
    pub symbol: String,
    pub bytes: Vec<u8>,
    pub size: u32,
    pub alignment: u32,
    pub mutable: bool,
}

impl NativeMirProject {
    pub fn validate(&self) -> Result<(), NativeMirError> {
        if !matches!(self.pointer_bytes, 4 | 8) {
            return Err(NativeMirError::new(
                "E_NATIVE_MIR_LAYOUT",
                "native MIR pointer width must be four or eight bytes",
            ));
        }
        let mut symbols = BTreeSet::new();
        for global in &self.globals {
            if global.symbol.is_empty()
                || global.size == 0
                || global.alignment == 0
                || !global.alignment.is_power_of_two()
                || global.bytes.len() > global.size as usize
                || !symbols.insert(global.symbol.as_str())
            {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_GLOBAL",
                    "native MIR contains a malformed or duplicate global",
                ));
            }
        }
        for function in &self.functions {
            if !symbols.insert(function.symbol.as_str()) {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_SYMBOL",
                    format!("duplicate native MIR symbol '{}'", function.symbol),
                ));
            }
            function.validate()?;
        }
        Ok(())
    }

    pub fn dump(&self) -> String {
        let mut output = format!(
            "native-mir pointer-bytes={} globals={}\n",
            self.pointer_bytes,
            self.globals.len()
        );
        for global in &self.globals {
            output.push_str(&format!(
                "global {} size={} align={} initialized={} mutable={}\n",
                global.symbol,
                global.size,
                global.alignment,
                global.bytes.len(),
                global.mutable
            ));
        }
        for function in &self.functions {
            output.push_str(&function.dump());
        }
        output
    }

    pub fn optimize(&mut self, level: NativeOptimizationLevel) -> Result<(), NativeMirError> {
        self.validate()?;
        if level == NativeOptimizationLevel::O0 {
            return Ok(());
        }
        for function in &mut self.functions {
            remove_unreachable_blocks(function);
            reuse_immediate_successor_bounds_check(function);
            remove_unreachable_blocks(function);
            coalesce_adjacent_scalar_copies(function);
            let live_out = block_liveness(function).1;
            for block in &mut function.blocks {
                fold_block_constants(block, &function.register_types)?;
                eliminate_dead_pure_definitions(
                    block,
                    live_out.get(&block.id).cloned().unwrap_or_default(),
                );
            }
        }
        self.validate()
    }
}

fn coalesce_adjacent_scalar_copies(function: &mut NativeMirFunction) {
    let mut uses = BTreeMap::<VirtualRegister, u32>::new();
    for block in &function.blocks {
        for instruction in &block.instructions {
            for register in instruction_uses(&instruction.instruction) {
                *uses.entry(register).or_default() += 1;
            }
        }
        for register in terminator_uses(&block.terminator) {
            *uses.entry(register).or_default() += 1;
        }
    }
    for block in &mut function.blocks {
        let mut index = 0usize;
        while index + 1 < block.instructions.len() {
            let (destination, source, ty) = match block.instructions[index + 1].instruction {
                NativeMirInst::Copy {
                    destination,
                    source,
                    ty,
                } => (destination, source, ty),
                _ => {
                    index += 1;
                    continue;
                }
            };
            if destination == source
                || uses.get(&source).copied().unwrap_or(0) != 1
                || function.register_types.get(destination.0 as usize) != Some(&ty)
                || function.register_types.get(source.0 as usize) != Some(&ty)
                || !rewrite_scalar_definition(
                    &mut block.instructions[index].instruction,
                    source,
                    destination,
                )
            {
                index += 1;
                continue;
            }
            block.instructions.remove(index + 1);
        }
    }
}

fn rewrite_scalar_definition(
    instruction: &mut NativeMirInst,
    expected: VirtualRegister,
    replacement: VirtualRegister,
) -> bool {
    let destination = match instruction {
        NativeMirInst::Const { destination, .. }
        | NativeMirInst::Copy { destination, .. }
        | NativeMirInst::Binary { destination, .. }
        | NativeMirInst::Unary { destination, .. }
        | NativeMirInst::Cast { destination, .. }
        | NativeMirInst::Compare { destination, .. }
        | NativeMirInst::StackAddress { destination, .. }
        | NativeMirInst::GlobalAddress { destination, .. }
        | NativeMirInst::AddressOffset { destination, .. }
        | NativeMirInst::Load { destination, .. } => destination,
        NativeMirInst::Call {
            destination: Some(destination),
            aggregate_result: None,
            ..
        }
        | NativeMirInst::Intrinsic {
            destination: Some(destination),
            ..
        } => destination,
        _ => return false,
    };
    if *destination != expected {
        return false;
    }
    *destination = replacement;
    true
}

pub fn legalize_project(
    project: &mut NativeMirProject,
    target: crate::target::TargetSpec,
) -> Result<(), NativeMirError> {
    project.validate()?;
    if target.layout.pointer_size as u8 != project.pointer_bytes {
        return Err(NativeMirError::new(
            "E_NATIVE_MIR_LAYOUT",
            "native MIR pointer width does not match target legalization",
        ));
    }
    for function in &mut project.functions {
        legalize_function(function, target)?;
    }
    project.validate()
}

fn legalize_function(
    function: &mut NativeMirFunction,
    target: crate::target::TargetSpec,
) -> Result<(), NativeMirError> {
    if target.machine != crate::target::MachineTarget::X86_64 {
        return Ok(());
    }
    for block in &mut function.blocks {
        let mut legalized = Vec::with_capacity(block.instructions.len());
        for spanned in block.instructions.drain(..) {
            let NativeMirInst::Binary {
                destination,
                op,
                left,
                right,
                ty: narrow @ (MachineType::I8 | MachineType::I16),
            } = spanned.instruction
            else {
                legalized.push(spanned);
                continue;
            };
            if !matches!(
                op,
                NativeBinaryOp::Mul
                    | NativeBinaryOp::SignedDiv
                    | NativeBinaryOp::UnsignedDiv
                    | NativeBinaryOp::SignedRem
                    | NativeBinaryOp::UnsignedRem
            ) {
                legalized.push(SpannedNativeInst {
                    instruction: NativeMirInst::Binary {
                        destination,
                        op,
                        left,
                        right,
                        ty: narrow,
                    },
                    span: spanned.span,
                });
                continue;
            }
            let signed = matches!(op, NativeBinaryOp::SignedDiv | NativeBinaryOp::SignedRem);
            let left = legalize_narrow_value(
                left,
                narrow,
                signed,
                &mut function.register_types,
                spanned.span,
                &mut legalized,
            );
            let right = legalize_narrow_value(
                right,
                narrow,
                signed,
                &mut function.register_types,
                spanned.span,
                &mut legalized,
            );
            let wide_destination = VirtualRegister(function.register_types.len() as u32);
            function.register_types.push(MachineType::I32);
            legalized.push(SpannedNativeInst {
                instruction: NativeMirInst::Binary {
                    destination: wide_destination,
                    op,
                    left,
                    right,
                    ty: MachineType::I32,
                },
                span: spanned.span,
            });
            legalized.push(SpannedNativeInst {
                instruction: NativeMirInst::Cast {
                    destination,
                    value: wide_destination,
                    from: MachineType::I32,
                    to: narrow,
                    signed: false,
                },
                span: spanned.span,
            });
        }
        block.instructions = legalized;
    }
    Ok(())
}

fn legalize_narrow_value(
    value: NativeValue,
    from: MachineType,
    signed: bool,
    register_types: &mut Vec<MachineType>,
    span: Span,
    output: &mut Vec<SpannedNativeInst>,
) -> NativeValue {
    match value {
        NativeValue::Immediate(value) => NativeValue::Immediate(if signed {
            signed_bits(value, from) as u32 as u64
        } else {
            normalize_bits(value, from)
        }),
        NativeValue::Register(value) => {
            let destination = VirtualRegister(register_types.len() as u32);
            register_types.push(MachineType::I32);
            output.push(SpannedNativeInst {
                instruction: NativeMirInst::Cast {
                    destination,
                    value,
                    from,
                    to: MachineType::I32,
                    signed,
                },
                span,
            });
            NativeValue::Register(destination)
        }
    }
}

impl NativeMirFunction {
    pub fn validate(&self) -> Result<(), NativeMirError> {
        if self.parameters.len() != self.parameter_aggregates.len()
            || self
                .parameter_aggregates
                .iter()
                .flatten()
                .any(|layout| !layout.is_valid())
            || self
                .aggregate_result
                .is_some_and(|layout| !layout.is_valid())
            || (self.result.is_some() && self.aggregate_result.is_some())
            || (self.aggregate_result.is_some() != self.aggregate_result_register.is_some())
            || self.aggregate_result_register.is_some_and(|register| {
                self.register_type(register).ok() != Some(MachineType::Pointer)
            })
        {
            return Err(NativeMirError::new(
                "E_NATIVE_MIR_ABI",
                format!(
                    "function '{}' has malformed aggregate ABI metadata",
                    self.symbol
                ),
            ));
        }
        let block_ids: BTreeSet<_> = self.blocks.iter().map(|block| block.id).collect();
        if block_ids.len() != self.blocks.len() || !block_ids.contains(&self.entry) {
            return Err(NativeMirError::new(
                "E_NATIVE_MIR_CFG",
                format!(
                    "function '{}' has duplicate blocks or a missing entry",
                    self.symbol
                ),
            ));
        }
        for (index, object) in self.stack_objects.iter().enumerate() {
            if object.id.0 as usize != index
                || object.bytes == 0
                || object.alignment == 0
                || !object.alignment.is_power_of_two()
            {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_STACK_OBJECT",
                    format!("function '{}' has a malformed stack object", self.symbol),
                ));
            }
        }
        for parameter in &self.parameters {
            self.register_type(*parameter)?;
        }
        for block in &self.blocks {
            for instruction in &block.instructions {
                self.validate_instruction(&instruction.instruction)?;
            }
            for target in terminator_targets(&block.terminator) {
                if !block_ids.contains(&target) {
                    return Err(NativeMirError::new(
                        "E_NATIVE_MIR_CFG",
                        format!(
                            "function '{}' branches to unknown block {}",
                            self.symbol, target.0
                        ),
                    ));
                }
            }
            for register in terminator_uses(&block.terminator) {
                self.register_type(register)?;
            }
            match &block.terminator {
                NativeTerminator::ReturnAggregate { source, layout }
                    if self.aggregate_result != Some(*layout)
                        || self.register_type(*source)? != MachineType::Pointer =>
                {
                    return Err(NativeMirError::new(
                        "E_NATIVE_MIR_ABI",
                        format!("function '{}' has an invalid aggregate return", self.symbol),
                    ));
                }
                NativeTerminator::ReturnAggregate { .. } => {}
                NativeTerminator::Return(Some(_)) if self.result.is_none() => {
                    return Err(NativeMirError::new(
                        "E_NATIVE_MIR_ABI",
                        format!(
                            "function '{}' returns a scalar without scalar result ABI",
                            self.symbol
                        ),
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn validate_instruction(&self, instruction: &NativeMirInst) -> Result<(), NativeMirError> {
        for register in instruction_definitions(instruction)
            .into_iter()
            .chain(instruction_uses(instruction))
        {
            self.register_type(register)?;
        }
        if let NativeMirInst::StackAddress { object, .. } = instruction {
            if self
                .stack_objects
                .get(object.0 as usize)
                .map(|item| item.id)
                != Some(*object)
            {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_STACK_OBJECT",
                    format!(
                        "function '{}' references unknown stack object {}",
                        self.symbol, object.0
                    ),
                ));
            }
        }
        if let NativeMirInst::AddressOffset { scale, .. } = instruction {
            if !matches!(*scale, 1 | 2 | 4 | 8) {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_ADDRESS",
                    "native MIR address scale must be 1, 2, 4, or 8",
                ));
            }
        }
        if let NativeMirInst::Call {
            destination,
            arguments,
            argument_aggregates,
            result,
            aggregate_result,
            ..
        } = instruction
        {
            if arguments.len() != argument_aggregates.len()
                || argument_aggregates
                    .iter()
                    .flatten()
                    .any(|layout| !layout.is_valid())
                || aggregate_result.is_some_and(|layout| !layout.is_valid())
                || (result.is_some() && aggregate_result.is_some())
                || (aggregate_result.is_some() && destination.is_none())
            {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_ABI",
                    format!("function '{}' has a malformed call ABI", self.symbol),
                ));
            }
            for (argument, aggregate) in arguments.iter().zip(argument_aggregates) {
                if aggregate.is_some() && self.register_type(*argument)? != MachineType::Pointer {
                    return Err(NativeMirError::new(
                        "E_NATIVE_MIR_ABI",
                        "aggregate call argument is not represented by an address",
                    ));
                }
            }
            if aggregate_result.is_some()
                && destination.is_some_and(|value| {
                    self.register_type(value).ok() != Some(MachineType::Pointer)
                })
            {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_ABI",
                    "aggregate call result storage is not represented by an address",
                ));
            }
        }
        for (register, expected) in instruction_typed_registers(instruction) {
            if self.register_type(register)? != expected {
                return Err(NativeMirError::new(
                    "E_NATIVE_MIR_TYPE",
                    format!(
                        "function '{}' uses v{} with inconsistent machine type",
                        self.symbol, register.0
                    ),
                ));
            }
        }
        Ok(())
    }

    fn register_type(&self, register: VirtualRegister) -> Result<MachineType, NativeMirError> {
        self.register_types
            .get(register.0 as usize)
            .copied()
            .ok_or_else(|| {
                NativeMirError::new(
                    "E_NATIVE_MIR_REGISTER",
                    format!(
                        "function '{}' references unknown v{}",
                        self.symbol, register.0
                    ),
                )
            })
    }

    pub fn dump(&self) -> String {
        let mut output = format!(
            "function {} instance={} vregs={} stack-objects={} entry=b{}\n",
            self.symbol,
            self.instance.0,
            self.register_types.len(),
            self.stack_objects.len(),
            self.entry.0
        );
        for block in &self.blocks {
            output.push_str(&format!("  b{}:\n", block.id.0));
            for instruction in &block.instructions {
                output.push_str(&format!(
                    "    {:?} @ {}\n",
                    instruction.instruction, instruction.span
                ));
            }
            output.push_str(&format!("    {:?}\n", block.terminator));
        }
        output
    }

    pub fn register_use_count(&self, register: VirtualRegister) -> usize {
        self.blocks
            .iter()
            .map(|block| {
                let instruction_uses = block
                    .instructions
                    .iter()
                    .flat_map(|instruction| instruction_uses(&instruction.instruction))
                    .filter(|used| *used == register)
                    .count();
                let terminator_uses = terminator_uses(&block.terminator)
                    .into_iter()
                    .filter(|used| *used == register)
                    .count();
                instruction_uses + terminator_uses
            })
            .sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalRegister(pub u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterLocation {
    Register(PhysicalRegister),
    RegisterPair {
        low: PhysicalRegister,
        high: PhysicalRegister,
    },
    Spill {
        slot: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterAllocation {
    pub locations: BTreeMap<VirtualRegister, RegisterLocation>,
    pub spill_slots: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterPolicy {
    pub allocatable: Vec<PhysicalRegister>,
    /// Ordered register pairs available for one scalar wider than a machine
    /// register. Targets without multiword scalar allocation leave this empty.
    pub allocatable_pairs: Vec<(PhysicalRegister, PhysicalRegister)>,
    /// Registers destroyed by an ordinary call. Values whose interval crosses
    /// a call are never assigned to these registers.
    pub call_clobbered: BTreeSet<PhysicalRegister>,
    /// Largest scalar that one physical register can hold on this machine.
    pub maximum_register_bytes: u8,
    pub pointer_bytes: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LiveInterval {
    register: VirtualRegister,
    start: u32,
    end: u32,
}

fn remove_unreachable_blocks(function: &mut NativeMirFunction) {
    let by_id: BTreeMap<_, _> = function
        .blocks
        .iter()
        .map(|block| (block.id, block))
        .collect();
    let mut reachable = BTreeSet::new();
    let mut pending = vec![function.entry];
    while let Some(block) = pending.pop() {
        if !reachable.insert(block) {
            continue;
        }
        if let Some(body) = by_id.get(&block) {
            let mut targets = terminator_targets(&body.terminator);
            targets.sort();
            targets.reverse();
            pending.extend(targets);
        }
    }
    function
        .blocks
        .retain(|block| reachable.contains(&block.id));
}

fn stack_bounds_check(
    block: &NativeMirBlock,
    require_entire_block: bool,
) -> Option<(
    VirtualRegister,
    VirtualRegister,
    StackObjectId,
    i32,
    BlockId,
    BlockId,
)> {
    let start = block.instructions.len().checked_sub(4)?;
    if require_entire_block && start != 0 {
        return None;
    }
    let instructions = &block.instructions[start..];
    let (
        NativeMirInst::StackAddress {
            destination: base,
            object,
        },
        NativeMirInst::AddressOffset {
            destination: address,
            base: offset_base,
            index: None,
            scale: 1,
            displacement,
        },
        NativeMirInst::Load {
            destination: bound,
            address: load_address,
            ty: MachineType::Pointer,
            volatile: false,
        },
        NativeMirInst::Compare {
            destination: compare,
            op: NativeCompareOp::UnsignedLess,
            left: NativeValue::Register(index),
            right: NativeValue::Register(compare_bound),
            ty: MachineType::Pointer,
        },
        NativeTerminator::Branch {
            condition: NativeValue::Register(branch_condition),
            then_block,
            else_block,
        },
    ) = (
        &instructions[0].instruction,
        &instructions[1].instruction,
        &instructions[2].instruction,
        &instructions[3].instruction,
        &block.terminator,
    )
    else {
        return None;
    };
    if base != offset_base
        || address != load_address
        || bound != compare_bound
        || compare != branch_condition
    {
        return None;
    }
    Some((
        *index,
        *base,
        *object,
        *displacement,
        *then_block,
        *else_block,
    ))
}

fn reuse_immediate_successor_bounds_check(function: &mut NativeMirFunction) {
    let by_id = function
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.id, index))
        .collect::<BTreeMap<_, _>>();
    let mut predecessor_counts = BTreeMap::<BlockId, usize>::new();
    for block in &function.blocks {
        for target in terminator_targets(&block.terminator) {
            *predecessor_counts.entry(target).or_default() += 1;
        }
    }

    let mut rewrites = Vec::<(usize, usize, BlockId, BlockId, VirtualRegister)>::new();
    for (predecessor_index, predecessor) in function.blocks.iter().enumerate() {
        let Some((index, base, object, displacement, successor, _)) =
            stack_bounds_check(predecessor, false)
        else {
            continue;
        };
        if predecessor_counts.get(&successor).copied() != Some(1) {
            continue;
        }
        let Some(&successor_index) = by_id.get(&successor) else {
            continue;
        };
        let Some((next_index, next_base, next_object, next_displacement, checked, failed)) =
            stack_bounds_check(&function.blocks[successor_index], true)
        else {
            continue;
        };
        if index != next_index || object != next_object || displacement != next_displacement {
            continue;
        }
        if predecessor_counts.get(&checked).copied() != Some(1)
            || function.register_use_count(next_base) != 2
        {
            continue;
        }
        let Some(&checked_index) = by_id.get(&checked) else {
            continue;
        };
        let Some(first_checked) = function.blocks[checked_index].instructions.first() else {
            continue;
        };
        if !matches!(
            first_checked.instruction,
            NativeMirInst::Load {
                address,
                ty: MachineType::Pointer,
                volatile: false,
                ..
            } if address == next_base
        ) {
            continue;
        }
        let Some(&failed_index) = by_id.get(&failed) else {
            continue;
        };
        let failed_block = &function.blocks[failed_index];
        if !failed_block.instructions.is_empty()
            || !matches!(failed_block.terminator, NativeTerminator::Trap)
        {
            continue;
        }
        rewrites.push((predecessor_index, checked_index, successor, checked, base));
    }

    for (predecessor_index, checked_index, successor, checked, base) in rewrites {
        let NativeMirInst::Load { address, .. } =
            &mut function.blocks[checked_index].instructions[0].instruction
        else {
            unreachable!("validated bounds body lost its leading descriptor load")
        };
        *address = base;
        let NativeTerminator::Branch { then_block, .. } =
            &mut function.blocks[predecessor_index].terminator
        else {
            unreachable!("validated bounds predecessor lost its branch")
        };
        debug_assert_eq!(*then_block, successor);
        *then_block = checked;
    }
}

fn fold_block_constants(
    block: &mut NativeMirBlock,
    types: &[MachineType],
) -> Result<(), NativeMirError> {
    let mut constants = BTreeMap::<VirtualRegister, u64>::new();
    for spanned in &mut block.instructions {
        let instruction = &mut spanned.instruction;
        replace_known_values(instruction, &constants);
        let definition = instruction_definitions(instruction);
        let folded = match instruction {
            NativeMirInst::Const {
                destination,
                value,
                ty,
            } => Some((*destination, normalize_bits(*value, *ty))),
            NativeMirInst::Copy {
                destination,
                source,
                ty,
            } => constants
                .get(source)
                .copied()
                .map(|value| (*destination, normalize_bits(value, *ty))),
            NativeMirInst::Binary {
                destination,
                op,
                left: NativeValue::Immediate(left),
                right: NativeValue::Immediate(right),
                ty,
            } => fold_binary(*op, *left, *right, *ty).map(|value| (*destination, value)),
            NativeMirInst::Unary {
                destination,
                op,
                value: NativeValue::Immediate(value),
                ty,
            } => Some((*destination, fold_unary(*op, *value, *ty))),
            NativeMirInst::Cast {
                destination,
                value,
                from,
                to,
                signed,
            } => constants
                .get(value)
                .copied()
                .map(|value| (*destination, fold_cast(value, *from, *to, *signed))),
            NativeMirInst::Compare {
                destination,
                op,
                left: NativeValue::Immediate(left),
                right: NativeValue::Immediate(right),
                ty,
            } => Some((
                *destination,
                u64::from(fold_compare(*op, *left, *right, *ty)),
            )),
            _ => None,
        };
        if let Some(register) = definition {
            constants.remove(&register);
        }
        if let Some((register, value)) = folded {
            let ty = *types.get(register.0 as usize).ok_or_else(|| {
                NativeMirError::new(
                    "E_NATIVE_MIR_REGISTER",
                    format!("constant folding references unknown v{}", register.0),
                )
            })?;
            *instruction = NativeMirInst::Const {
                destination: register,
                value: normalize_bits(value, ty),
                ty,
            };
            constants.insert(register, normalize_bits(value, ty));
        } else if let Some(replacement) = fold_identity(instruction) {
            *instruction = replacement;
        }
    }
    if let NativeTerminator::Branch {
        condition: NativeValue::Register(register),
        then_block,
        else_block,
    } = block.terminator
    {
        if let Some(value) = constants.get(&register) {
            block.terminator =
                NativeTerminator::Jump(if *value != 0 { then_block } else { else_block });
        }
    }
    Ok(())
}

fn fold_unary(op: NativeUnaryOp, value: u64, ty: MachineType) -> u64 {
    normalize_bits(
        match op {
            NativeUnaryOp::Negate => 0u64.wrapping_sub(value),
            NativeUnaryOp::BitNot => !value,
            NativeUnaryOp::BoolNot => u64::from(value == 0),
        },
        ty,
    )
}

fn fold_cast(value: u64, from: MachineType, to: MachineType, signed: bool) -> u64 {
    let value = if signed {
        signed_bits(value, from) as u64
    } else {
        normalize_bits(value, from)
    };
    normalize_bits(value, to)
}

fn fold_identity(instruction: &NativeMirInst) -> Option<NativeMirInst> {
    let NativeMirInst::Binary {
        destination,
        op,
        left,
        right,
        ty,
    } = instruction
    else {
        return None;
    };
    let replacement = match (op, left, right) {
        (
            NativeBinaryOp::Add
            | NativeBinaryOp::Sub
            | NativeBinaryOp::BitOr
            | NativeBinaryOp::BitXor
            | NativeBinaryOp::ShiftLeft
            | NativeBinaryOp::SignedShiftRight
            | NativeBinaryOp::UnsignedShiftRight,
            value,
            NativeValue::Immediate(0),
        ) => Some(*value),
        (
            NativeBinaryOp::Add | NativeBinaryOp::BitOr | NativeBinaryOp::BitXor,
            NativeValue::Immediate(0),
            value,
        ) => Some(*value),
        (
            NativeBinaryOp::Mul | NativeBinaryOp::SignedDiv | NativeBinaryOp::UnsignedDiv,
            value,
            NativeValue::Immediate(1),
        ) => Some(*value),
        (NativeBinaryOp::Mul, NativeValue::Immediate(1), value) => Some(*value),
        (NativeBinaryOp::SignedRem | NativeBinaryOp::UnsignedRem, _, NativeValue::Immediate(1)) => {
            return Some(NativeMirInst::Const {
                destination: *destination,
                value: 0,
                ty: *ty,
            });
        }
        _ => None,
    }?;
    Some(match replacement {
        NativeValue::Register(source) => NativeMirInst::Copy {
            destination: *destination,
            source,
            ty: *ty,
        },
        NativeValue::Immediate(value) => NativeMirInst::Const {
            destination: *destination,
            value: normalize_bits(value, *ty),
            ty: *ty,
        },
    })
}

fn replace_known_values(
    instruction: &mut NativeMirInst,
    constants: &BTreeMap<VirtualRegister, u64>,
) {
    let replace = |value: &mut NativeValue| {
        if let NativeValue::Register(register) = value {
            if let Some(constant) = constants.get(register) {
                *value = NativeValue::Immediate(*constant);
            }
        }
    };
    match instruction {
        NativeMirInst::Binary { left, right, .. } | NativeMirInst::Compare { left, right, .. } => {
            replace(left);
            replace(right);
        }
        _ => {}
    }
}

fn normalize_bits(value: u64, ty: MachineType) -> u64 {
    match ty {
        MachineType::Bool => u64::from(value != 0),
        MachineType::I8 => value & 0xff,
        MachineType::I16 => value & 0xffff,
        MachineType::I32 => value & 0xffff_ffff,
        MachineType::I64 | MachineType::Pointer => value,
    }
}

fn signed_bits(value: u64, ty: MachineType) -> i64 {
    match ty {
        MachineType::I8 => (value as u8 as i8) as i64,
        MachineType::I16 => (value as u16 as i16) as i64,
        MachineType::I32 => (value as u32 as i32) as i64,
        MachineType::I64 => value as i64,
        MachineType::Bool | MachineType::Pointer => value as i64,
    }
}

fn fold_binary(op: NativeBinaryOp, left: u64, right: u64, ty: MachineType) -> Option<u64> {
    let width: u32 = match ty {
        MachineType::Bool | MachineType::I8 => 8,
        MachineType::I16 => 16,
        MachineType::I32 => 32,
        MachineType::I64 | MachineType::Pointer => 64,
    };
    let shift = (right & u64::from(width - 1)) as u32;
    let value = match op {
        NativeBinaryOp::Add => left.wrapping_add(right),
        NativeBinaryOp::Sub => left.wrapping_sub(right),
        NativeBinaryOp::Mul => left.wrapping_mul(right),
        NativeBinaryOp::BitAnd => left & right,
        NativeBinaryOp::BitOr => left | right,
        NativeBinaryOp::BitXor => left ^ right,
        NativeBinaryOp::ShiftLeft => left.wrapping_shl(shift),
        NativeBinaryOp::UnsignedShiftRight => left.wrapping_shr(shift),
        NativeBinaryOp::SignedShiftRight => (signed_bits(left, ty) >> shift) as u64,
        NativeBinaryOp::UnsignedDiv if right != 0 => left / right,
        NativeBinaryOp::UnsignedRem if right != 0 => left % right,
        NativeBinaryOp::SignedDiv | NativeBinaryOp::SignedRem => {
            let left = signed_bits(left, ty);
            let right = signed_bits(right, ty);
            if right == 0 || (left == signed_minimum(ty) && right == -1) {
                return None;
            }
            if op == NativeBinaryOp::SignedDiv {
                (left / right) as u64
            } else {
                (left % right) as u64
            }
        }
        NativeBinaryOp::UnsignedDiv | NativeBinaryOp::UnsignedRem => return None,
    };
    Some(normalize_bits(value, ty))
}

fn signed_minimum(ty: MachineType) -> i64 {
    match ty {
        MachineType::I8 => i8::MIN as i64,
        MachineType::I16 => i16::MIN as i64,
        MachineType::I32 => i32::MIN as i64,
        MachineType::I64 => i64::MIN,
        MachineType::Bool | MachineType::Pointer => i64::MIN,
    }
}

fn fold_compare(op: NativeCompareOp, left: u64, right: u64, ty: MachineType) -> bool {
    match op {
        NativeCompareOp::Equal => normalize_bits(left, ty) == normalize_bits(right, ty),
        NativeCompareOp::NotEqual => normalize_bits(left, ty) != normalize_bits(right, ty),
        NativeCompareOp::SignedLess => signed_bits(left, ty) < signed_bits(right, ty),
        NativeCompareOp::SignedLessEqual => signed_bits(left, ty) <= signed_bits(right, ty),
        NativeCompareOp::SignedGreater => signed_bits(left, ty) > signed_bits(right, ty),
        NativeCompareOp::SignedGreaterEqual => signed_bits(left, ty) >= signed_bits(right, ty),
        NativeCompareOp::UnsignedLess => normalize_bits(left, ty) < normalize_bits(right, ty),
        NativeCompareOp::UnsignedLessEqual => normalize_bits(left, ty) <= normalize_bits(right, ty),
        NativeCompareOp::UnsignedGreater => normalize_bits(left, ty) > normalize_bits(right, ty),
        NativeCompareOp::UnsignedGreaterEqual => {
            normalize_bits(left, ty) >= normalize_bits(right, ty)
        }
    }
}

fn eliminate_dead_pure_definitions(
    block: &mut NativeMirBlock,
    mut live: BTreeSet<VirtualRegister>,
) {
    live.extend(terminator_uses(&block.terminator));
    let mut retained = Vec::with_capacity(block.instructions.len());
    for instruction in block.instructions.iter().rev() {
        let definition = instruction_definitions(&instruction.instruction);
        let removable = is_pure(&instruction.instruction)
            && definition.is_some_and(|register| !live.contains(&register));
        if removable {
            continue;
        }
        if let Some(register) = definition {
            live.remove(&register);
        }
        live.extend(instruction_uses(&instruction.instruction));
        retained.push(instruction.clone());
    }
    retained.reverse();
    block.instructions = retained;
}

fn is_pure(instruction: &NativeMirInst) -> bool {
    matches!(
        instruction,
        NativeMirInst::Const { .. }
            | NativeMirInst::Copy { .. }
            | NativeMirInst::Binary {
                op: NativeBinaryOp::Add
                    | NativeBinaryOp::Sub
                    | NativeBinaryOp::Mul
                    | NativeBinaryOp::BitAnd
                    | NativeBinaryOp::BitOr
                    | NativeBinaryOp::BitXor
                    | NativeBinaryOp::ShiftLeft
                    | NativeBinaryOp::SignedShiftRight
                    | NativeBinaryOp::UnsignedShiftRight,
                ..
            }
            | NativeMirInst::Unary { .. }
            | NativeMirInst::Cast { .. }
            | NativeMirInst::Compare { .. }
            | NativeMirInst::StackAddress { .. }
            | NativeMirInst::GlobalAddress { .. }
            | NativeMirInst::AddressOffset { .. }
    )
}

pub fn allocate_registers(
    function: &NativeMirFunction,
    policy: &RegisterPolicy,
    level: NativeOptimizationLevel,
) -> Result<RegisterAllocation, NativeMirError> {
    function.validate()?;
    let mut intervals = live_intervals(function);
    let entry_values = function
        .parameters
        .iter()
        .copied()
        .chain(function.aggregate_result_register);
    for register in entry_values {
        if let Some(interval) = intervals
            .iter_mut()
            .find(|interval| interval.register == register)
        {
            interval.start = 0;
        }
    }
    intervals.sort_by_key(|interval| (interval.start, interval.end, interval.register.0));
    if level == NativeOptimizationLevel::O0 || policy.allocatable.is_empty() {
        return Ok(RegisterAllocation {
            locations: intervals
                .iter()
                .enumerate()
                .map(|(slot, interval)| {
                    (
                        interval.register,
                        RegisterLocation::Spill { slot: slot as u32 },
                    )
                })
                .collect(),
            spill_slots: intervals.len() as u32,
        });
    }

    let call_positions = call_positions(function);
    let mut parameters = function.parameters.iter().copied().collect::<BTreeSet<_>>();
    if let Some(register) = function.aggregate_result_register {
        parameters.insert(register);
    }
    let mut active = Vec::<(LiveInterval, PhysicalRegister)>::new();
    let mut active_pairs = Vec::<(LiveInterval, (PhysicalRegister, PhysicalRegister))>::new();
    let mut active_spills = Vec::<(LiveInterval, u32)>::new();
    let mut free_spills = BTreeSet::<u32>::new();
    let mut locations = BTreeMap::new();
    let mut spill_slots = 0u32;
    for interval in intervals {
        active.retain(|(candidate, _)| candidate.end >= interval.start);
        active_pairs.retain(|(candidate, _)| candidate.end >= interval.start);
        active_spills.retain(|(candidate, slot)| {
            // MIR instructions read all inputs before committing their result,
            // so a destination may reuse a spill whose final use is the same
            // instruction. Physical registers stay conservative because fixed
            // selector operands can overlap before the commit point.
            if candidate.end <= interval.start {
                free_spills.insert(*slot);
                false
            } else {
                true
            }
        });
        let mut used: BTreeSet<_> = active.iter().map(|(_, register)| *register).collect();
        for (_, (low, high)) in &active_pairs {
            used.insert(*low);
            used.insert(*high);
        }
        // Function parameters are live on entry and have no MIR definition at
        // position zero. Starting their call scan at start + 1 would miss a
        // first-instruction call and allow a still-live parameter into a
        // caller-clobbered register. Defined virtual registers still skip the
        // defining position itself.
        let call_scan_start = if parameters.contains(&interval.register) {
            interval.start
        } else {
            interval.start.saturating_add(1)
        };
        let crosses_call = call_scan_start < interval.end
            && call_positions
                .range(call_scan_start..interval.end)
                .next()
                .is_some();
        let ty = function.register_types[interval.register.0 as usize];
        let register_eligible = ty.bytes(policy.pointer_bytes) <= policy.maximum_register_bytes;
        let wide_pair = (ty.bytes(policy.pointer_bytes) > policy.maximum_register_bytes)
            .then(|| {
                policy
                    .allocatable_pairs
                    .iter()
                    .copied()
                    .find(|(low, high)| {
                        !used.contains(low)
                            && !used.contains(high)
                            && (!crosses_call
                                || (!policy.call_clobbered.contains(low)
                                    && !policy.call_clobbered.contains(high)))
                    })
            })
            .flatten();
        if let Some((low, high)) = wide_pair {
            locations.insert(
                interval.register,
                RegisterLocation::RegisterPair { low, high },
            );
            active_pairs.push((interval, (low, high)));
            active_pairs.sort_by_key(|(candidate, pair)| (candidate.end, pair.0 .0, pair.1 .0));
        } else if let Some(register) = policy.allocatable.iter().copied().find(|register| {
            register_eligible
                && !used.contains(register)
                && (!crosses_call || !policy.call_clobbered.contains(register))
        }) {
            locations.insert(interval.register, RegisterLocation::Register(register));
            active.push((interval, register));
            active.sort_by_key(|(candidate, register)| (candidate.end, register.0));
        } else {
            let slot = free_spills.pop_first().unwrap_or_else(|| {
                let slot = spill_slots;
                spill_slots += 1;
                slot
            });
            locations.insert(interval.register, RegisterLocation::Spill { slot });
            active_spills.push((interval, slot));
            active_spills.sort_by_key(|(candidate, slot)| (candidate.end, *slot));
        }
    }
    Ok(RegisterAllocation {
        locations,
        spill_slots,
    })
}

fn call_positions(function: &NativeMirFunction) -> BTreeSet<u32> {
    let mut positions = BTreeSet::new();
    let mut position = 0u32;
    for block in &function.blocks {
        for instruction in &block.instructions {
            if matches!(
                instruction.instruction,
                NativeMirInst::Call { .. } | NativeMirInst::Intrinsic { .. }
            ) {
                positions.insert(position);
            }
            position += 1;
        }
        position += 1;
    }
    positions
}

fn live_intervals(function: &NativeMirFunction) -> Vec<LiveInterval> {
    let (live_in, live_out) = block_liveness(function);
    let mut ranges = BTreeMap::<VirtualRegister, (u32, u32)>::new();
    for parameter in &function.parameters {
        ranges.insert(*parameter, (0, 0));
    }
    if let Some(register) = function.aggregate_result_register {
        ranges.insert(register, (0, 0));
    }
    let mut position = 0u32;
    for block in &function.blocks {
        let block_start = position;
        for register in &live_in[&block.id] {
            ranges
                .entry(*register)
                .and_modify(|range| range.0 = range.0.min(block_start))
                .or_insert((block_start, block_start));
        }
        for instruction in &block.instructions {
            for register in instruction_definitions(&instruction.instruction)
                .into_iter()
                .chain(instruction_uses(&instruction.instruction))
            {
                ranges
                    .entry(register)
                    .and_modify(|range| range.1 = position)
                    .or_insert((position, position));
            }
            position += 1;
        }
        for register in terminator_uses(&block.terminator) {
            ranges
                .entry(register)
                .and_modify(|range| range.1 = position)
                .or_insert((position, position));
        }
        let block_end = position;
        for register in &live_out[&block.id] {
            ranges
                .entry(*register)
                .and_modify(|range| range.1 = range.1.max(block_end))
                .or_insert((block_start, block_end));
        }
        position += 1;
    }
    let mut intervals = ranges
        .into_iter()
        .map(|(register, (start, end))| LiveInterval {
            register,
            start,
            end,
        })
        .collect::<Vec<_>>();
    intervals.sort_by_key(|interval| (interval.start, interval.end, interval.register.0));
    intervals
}

fn block_liveness(
    function: &NativeMirFunction,
) -> (
    BTreeMap<BlockId, BTreeSet<VirtualRegister>>,
    BTreeMap<BlockId, BTreeSet<VirtualRegister>>,
) {
    let mut block_uses = BTreeMap::<BlockId, BTreeSet<VirtualRegister>>::new();
    let mut block_defs = BTreeMap::<BlockId, BTreeSet<VirtualRegister>>::new();
    for block in &function.blocks {
        let mut uses = BTreeSet::new();
        let mut defs = BTreeSet::new();
        for instruction in &block.instructions {
            for register in instruction_uses(&instruction.instruction) {
                if !defs.contains(&register) {
                    uses.insert(register);
                }
            }
            if let Some(register) = instruction_definitions(&instruction.instruction) {
                defs.insert(register);
            }
        }
        for register in terminator_uses(&block.terminator) {
            if !defs.contains(&register) {
                uses.insert(register);
            }
        }
        block_uses.insert(block.id, uses);
        block_defs.insert(block.id, defs);
    }

    let mut live_in = BTreeMap::<BlockId, BTreeSet<VirtualRegister>>::new();
    let mut live_out = BTreeMap::<BlockId, BTreeSet<VirtualRegister>>::new();
    loop {
        let mut changed = false;
        for block in function.blocks.iter().rev() {
            let outgoing: BTreeSet<_> = terminator_targets(&block.terminator)
                .into_iter()
                .flat_map(|target| live_in.get(&target).into_iter().flatten().copied())
                .collect();
            let mut incoming = block_uses[&block.id].clone();
            incoming.extend(
                outgoing
                    .iter()
                    .filter(|register| !block_defs[&block.id].contains(register))
                    .copied(),
            );
            changed |= live_out.get(&block.id) != Some(&outgoing);
            changed |= live_in.get(&block.id) != Some(&incoming);
            live_out.insert(block.id, outgoing);
            live_in.insert(block.id, incoming);
        }
        if !changed {
            break;
        }
    }
    (live_in, live_out)
}

fn instruction_definitions(instruction: &NativeMirInst) -> Option<VirtualRegister> {
    match instruction {
        NativeMirInst::Const { destination, .. }
        | NativeMirInst::Copy { destination, .. }
        | NativeMirInst::Binary { destination, .. }
        | NativeMirInst::Unary { destination, .. }
        | NativeMirInst::Cast { destination, .. }
        | NativeMirInst::Compare { destination, .. }
        | NativeMirInst::StackAddress { destination, .. }
        | NativeMirInst::GlobalAddress { destination, .. }
        | NativeMirInst::InterruptAddress { destination, .. }
        | NativeMirInst::TaskAddress { destination, .. }
        | NativeMirInst::AddressOffset { destination, .. }
        | NativeMirInst::Load { destination, .. } => Some(*destination),
        NativeMirInst::Call {
            destination,
            aggregate_result,
            ..
        } => aggregate_result.is_none().then_some(*destination).flatten(),
        NativeMirInst::Intrinsic { destination, .. } => *destination,
        NativeMirInst::Store { .. }
        | NativeMirInst::AggregateCopy { .. }
        | NativeMirInst::AggregateZero { .. }
        | NativeMirInst::InterruptDefinition { .. } => None,
    }
}

fn instruction_uses(instruction: &NativeMirInst) -> Vec<VirtualRegister> {
    let mut uses = Vec::new();
    let mut push_value = |value: NativeValue| {
        if let NativeValue::Register(register) = value {
            uses.push(register);
        }
    };
    match instruction {
        NativeMirInst::Copy { source, .. } => uses.push(*source),
        NativeMirInst::Binary { left, right, .. } | NativeMirInst::Compare { left, right, .. } => {
            push_value(*left);
            push_value(*right);
        }
        NativeMirInst::Unary { value, .. } => push_value(*value),
        NativeMirInst::Cast { value, .. } => uses.push(*value),
        NativeMirInst::AddressOffset { base, index, .. } => {
            uses.push(*base);
            uses.extend(*index);
        }
        NativeMirInst::Load { address, .. } => uses.push(*address),
        NativeMirInst::Store { address, value, .. } => uses.extend([*address, *value]),
        NativeMirInst::AggregateCopy {
            destination,
            source,
            ..
        } => uses.extend([*destination, *source]),
        NativeMirInst::AggregateZero { destination, .. } => uses.push(*destination),
        NativeMirInst::Call {
            destination,
            callee,
            arguments,
            aggregate_result,
            ..
        } => {
            if aggregate_result.is_some() {
                uses.extend(destination.iter().copied());
            }
            match callee {
                NativeCallee::Direct { .. } => {}
                NativeCallee::Indirect { address } => uses.push(*address),
                NativeCallee::Closure { code, environment } => uses.extend([*code, *environment]),
            }
            uses.extend(arguments.iter().copied());
        }
        NativeMirInst::Intrinsic { arguments, .. } => uses.extend(arguments.iter().copied()),
        NativeMirInst::Const { .. }
        | NativeMirInst::StackAddress { .. }
        | NativeMirInst::GlobalAddress { .. }
        | NativeMirInst::InterruptDefinition { .. }
        | NativeMirInst::InterruptAddress { .. }
        | NativeMirInst::TaskAddress { .. } => {}
    }
    uses
}

fn instruction_typed_registers(instruction: &NativeMirInst) -> Vec<(VirtualRegister, MachineType)> {
    match instruction {
        NativeMirInst::Const {
            destination, ty, ..
        }
        | NativeMirInst::Copy {
            destination, ty, ..
        }
        | NativeMirInst::Binary {
            destination, ty, ..
        }
        | NativeMirInst::Unary {
            destination, ty, ..
        } => vec![(*destination, *ty)],
        NativeMirInst::Cast {
            destination, to, ..
        } => vec![(*destination, *to)],
        NativeMirInst::Compare { destination, .. } => vec![(*destination, MachineType::Bool)],
        NativeMirInst::InterruptAddress { destination, .. }
        | NativeMirInst::TaskAddress { destination, .. } => {
            vec![(*destination, MachineType::Pointer)]
        }
        NativeMirInst::Load {
            destination, ty, ..
        } => vec![(*destination, *ty)],
        NativeMirInst::Call {
            destination: Some(destination),
            result: Some(ty),
            ..
        }
        | NativeMirInst::Intrinsic {
            destination: Some(destination),
            result: Some(ty),
            ..
        } => vec![(*destination, *ty)],
        _ => Vec::new(),
    }
}

fn terminator_targets(terminator: &NativeTerminator) -> Vec<BlockId> {
    match terminator {
        NativeTerminator::Jump(target) => vec![*target],
        NativeTerminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        NativeTerminator::Return(_)
        | NativeTerminator::ReturnAggregate { .. }
        | NativeTerminator::Trap
        | NativeTerminator::Unreachable => Vec::new(),
    }
}

fn terminator_uses(terminator: &NativeTerminator) -> Vec<VirtualRegister> {
    match terminator {
        NativeTerminator::Branch {
            condition: NativeValue::Register(register),
            ..
        }
        | NativeTerminator::Return(Some(register))
        | NativeTerminator::ReturnAggregate {
            source: register, ..
        } => vec![*register],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn function() -> NativeMirFunction {
        NativeMirFunction {
            instance: FunctionInstanceId(0),
            symbol: "main".into(),
            convention: CallConvention::DByte,
            parameters: vec![VirtualRegister(0)],
            parameter_aggregates: vec![None],
            result: Some(MachineType::I32),
            aggregate_result: None,
            aggregate_result_register: None,
            register_types: vec![MachineType::I32, MachineType::I32, MachineType::I32],
            stack_objects: vec![],
            entry: BlockId(0),
            blocks: vec![NativeMirBlock {
                id: BlockId(0),
                instructions: vec![
                    NativeMirInst::Const {
                        destination: VirtualRegister(1),
                        value: 1,
                        ty: MachineType::I32,
                    }
                    .into(),
                    NativeMirInst::Binary {
                        destination: VirtualRegister(2),
                        op: NativeBinaryOp::Add,
                        left: NativeValue::Register(VirtualRegister(0)),
                        right: NativeValue::Register(VirtualRegister(1)),
                        ty: MachineType::I32,
                    }
                    .into(),
                ],
                terminator: NativeTerminator::Return(Some(VirtualRegister(2))),
            }],
        }
    }

    fn bounds_function() -> NativeMirFunction {
        NativeMirFunction {
            instance: FunctionInstanceId(1),
            symbol: "bounds".into(),
            convention: CallConvention::DByte,
            parameters: vec![VirtualRegister(0)],
            parameter_aggregates: vec![None],
            result: None,
            aggregate_result: None,
            aggregate_result_register: None,
            register_types: vec![
                MachineType::Pointer,
                MachineType::Pointer,
                MachineType::Pointer,
                MachineType::Pointer,
                MachineType::Bool,
                MachineType::Pointer,
                MachineType::Pointer,
                MachineType::Pointer,
                MachineType::Bool,
            ],
            stack_objects: vec![NativeStackObject {
                id: StackObjectId(0),
                bytes: 16,
                alignment: 8,
                address_taken: true,
            }],
            entry: BlockId(0),
            blocks: vec![
                bounds_block(BlockId(0), 1, 2, 3, 4, BlockId(1), BlockId(3)),
                bounds_block(BlockId(1), 5, 6, 7, 8, BlockId(2), BlockId(4)),
                NativeMirBlock {
                    id: BlockId(2),
                    instructions: vec![],
                    terminator: NativeTerminator::Return(None),
                },
                NativeMirBlock {
                    id: BlockId(3),
                    instructions: vec![],
                    terminator: NativeTerminator::Return(None),
                },
                NativeMirBlock {
                    id: BlockId(4),
                    instructions: vec![],
                    terminator: NativeTerminator::Trap,
                },
            ],
        }
    }

    fn bounds_block(
        id: BlockId,
        base: u32,
        address: u32,
        bound: u32,
        compare: u32,
        checked: BlockId,
        failed: BlockId,
    ) -> NativeMirBlock {
        NativeMirBlock {
            id,
            instructions: vec![
                NativeMirInst::StackAddress {
                    destination: VirtualRegister(base),
                    object: StackObjectId(0),
                }
                .into(),
                NativeMirInst::AddressOffset {
                    destination: VirtualRegister(address),
                    base: VirtualRegister(base),
                    index: None,
                    scale: 1,
                    displacement: 8,
                }
                .into(),
                NativeMirInst::Load {
                    destination: VirtualRegister(bound),
                    address: VirtualRegister(address),
                    ty: MachineType::Pointer,
                    volatile: false,
                }
                .into(),
                NativeMirInst::Compare {
                    destination: VirtualRegister(compare),
                    op: NativeCompareOp::UnsignedLess,
                    left: NativeValue::Register(VirtualRegister(0)),
                    right: NativeValue::Register(VirtualRegister(bound)),
                    ty: MachineType::Pointer,
                }
                .into(),
            ],
            terminator: NativeTerminator::Branch {
                condition: NativeValue::Register(VirtualRegister(compare)),
                then_block: checked,
                else_block: failed,
            },
        }
    }

    #[test]
    fn o1_bounds_reuse_forwards_descriptor_base_into_checked_body() {
        let mut function = bounds_function();
        function.register_types.push(MachineType::Pointer);
        function.blocks[2].instructions.push(
            NativeMirInst::Load {
                destination: VirtualRegister(9),
                address: VirtualRegister(5),
                ty: MachineType::Pointer,
                volatile: false,
            }
            .into(),
        );
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: vec![],
            functions: vec![function],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        let function = &project.functions[0];
        assert!(function.blocks.iter().all(|block| block.id != BlockId(1)));
        let predecessor = function
            .blocks
            .iter()
            .find(|block| block.id == BlockId(0))
            .unwrap();
        assert!(matches!(
            predecessor.terminator,
            NativeTerminator::Branch {
                then_block: BlockId(2),
                else_block: BlockId(3),
                ..
            }
        ));
        let checked = function
            .blocks
            .iter()
            .find(|block| block.id == BlockId(2))
            .unwrap();
        assert!(matches!(
            checked.instructions[0].instruction,
            NativeMirInst::Load {
                address: VirtualRegister(1),
                ..
            }
        ));
    }

    #[test]
    fn o1_bounds_reuse_requires_single_proven_predecessor_and_trap_failure() {
        let mut multiple = bounds_function();
        multiple.register_types.push(MachineType::Bool);
        multiple.parameters.push(VirtualRegister(9));
        multiple.parameter_aggregates.push(None);
        multiple.entry = BlockId(5);
        multiple.blocks.push(NativeMirBlock {
            id: BlockId(5),
            instructions: vec![],
            terminator: NativeTerminator::Branch {
                condition: NativeValue::Register(VirtualRegister(9)),
                then_block: BlockId(0),
                else_block: BlockId(6),
            },
        });
        multiple.blocks.push(NativeMirBlock {
            id: BlockId(6),
            instructions: vec![],
            terminator: NativeTerminator::Jump(BlockId(1)),
        });
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: vec![],
            functions: vec![multiple],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        let repeated = project.functions[0]
            .blocks
            .iter()
            .find(|block| block.id == BlockId(1))
            .unwrap();
        assert_eq!(repeated.instructions.len(), 4);
        assert!(matches!(
            repeated.terminator,
            NativeTerminator::Branch { .. }
        ));

        let mut non_trap = bounds_function();
        non_trap.blocks[4].terminator = NativeTerminator::Return(None);
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: vec![],
            functions: vec![non_trap],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        let repeated = project.functions[0]
            .blocks
            .iter()
            .find(|block| block.id == BlockId(1))
            .unwrap();
        assert_eq!(repeated.instructions.len(), 4);
        assert!(matches!(
            repeated.terminator,
            NativeTerminator::Branch { .. }
        ));
    }

    #[test]
    fn o1_bounds_reuse_rejects_ambiguous_checked_body() {
        let mut multiple_checked = bounds_function();
        multiple_checked.register_types.push(MachineType::Pointer);
        multiple_checked.blocks[2].instructions.push(
            NativeMirInst::Load {
                destination: VirtualRegister(9),
                address: VirtualRegister(5),
                ty: MachineType::Pointer,
                volatile: false,
            }
            .into(),
        );
        multiple_checked.register_types.push(MachineType::Bool);
        multiple_checked.parameters.push(VirtualRegister(10));
        multiple_checked.parameter_aggregates.push(None);
        multiple_checked.entry = BlockId(5);
        multiple_checked.blocks.push(NativeMirBlock {
            id: BlockId(5),
            instructions: vec![],
            terminator: NativeTerminator::Branch {
                condition: NativeValue::Register(VirtualRegister(10)),
                then_block: BlockId(0),
                else_block: BlockId(6),
            },
        });
        multiple_checked.blocks.push(NativeMirBlock {
            id: BlockId(6),
            instructions: vec![],
            terminator: NativeTerminator::Jump(BlockId(2)),
        });
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: vec![],
            functions: vec![multiple_checked],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        let repeated = project.functions[0]
            .blocks
            .iter()
            .find(|block| block.id == BlockId(1))
            .unwrap();
        assert_eq!(repeated.instructions.len(), 4);

        let mut extra_use = bounds_function();
        extra_use.register_types.push(MachineType::Pointer);
        extra_use.register_types.push(MachineType::Pointer);
        for destination in [VirtualRegister(9), VirtualRegister(10)] {
            extra_use.blocks[2].instructions.push(
                NativeMirInst::Load {
                    destination,
                    address: VirtualRegister(5),
                    ty: MachineType::Pointer,
                    volatile: false,
                }
                .into(),
            );
        }
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: vec![],
            functions: vec![extra_use],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        let repeated = project.functions[0]
            .blocks
            .iter()
            .find(|block| block.id == BlockId(1))
            .unwrap();
        assert_eq!(repeated.instructions.len(), 4);
    }

    #[test]
    fn optimization_level_is_narrow_and_o1_is_default() {
        assert_eq!(
            NativeOptimizationLevel::default(),
            NativeOptimizationLevel::O1
        );
        assert_eq!(
            NativeOptimizationLevel::parse("0").unwrap(),
            NativeOptimizationLevel::O0
        );
        assert_eq!(
            NativeOptimizationLevel::parse("1").unwrap(),
            NativeOptimizationLevel::O1
        );
        assert_eq!(
            NativeOptimizationLevel::parse("2").unwrap_err().code,
            "E_NATIVE_OPT_LEVEL"
        );
    }

    #[test]
    fn project_validation_and_dump_are_deterministic() {
        let project = NativeMirProject {
            pointer_bytes: 8,
            globals: Vec::new(),
            functions: vec![function()],
        };
        project.validate().unwrap();
        assert_eq!(project.dump(), project.dump());
        assert!(project.dump().contains("function main"));
    }

    #[test]
    fn malformed_cfg_register_and_stack_object_fail_closed() {
        let mut candidate = function();
        candidate.blocks[0].terminator = NativeTerminator::Jump(BlockId(9));
        assert_eq!(candidate.validate().unwrap_err().code, "E_NATIVE_MIR_CFG");

        let mut candidate = function();
        candidate.blocks[0].terminator = NativeTerminator::Return(Some(VirtualRegister(9)));
        assert_eq!(
            candidate.validate().unwrap_err().code,
            "E_NATIVE_MIR_REGISTER"
        );

        let mut candidate = function();
        candidate.stack_objects.push(NativeStackObject {
            id: StackObjectId(1),
            bytes: 0,
            alignment: 3,
            address_taken: true,
        });
        assert_eq!(
            candidate.validate().unwrap_err().code,
            "E_NATIVE_MIR_STACK_OBJECT"
        );
    }

    #[test]
    fn o0_spills_all_and_o1_linear_scan_is_deterministic() {
        let function = function();
        let policy = RegisterPolicy {
            allocatable: vec![PhysicalRegister(0), PhysicalRegister(1)],
            allocatable_pairs: Vec::new(),
            call_clobbered: BTreeSet::new(),
            maximum_register_bytes: 8,
            pointer_bytes: 8,
        };
        let o0 = allocate_registers(&function, &policy, NativeOptimizationLevel::O0).unwrap();
        assert_eq!(o0.spill_slots, 3);
        assert!(o0
            .locations
            .values()
            .all(|location| matches!(location, RegisterLocation::Spill { .. })));

        let first = allocate_registers(&function, &policy, NativeOptimizationLevel::O1).unwrap();
        let second = allocate_registers(&function, &policy, NativeOptimizationLevel::O1).unwrap();
        assert_eq!(first, second);
        assert!(first
            .locations
            .values()
            .any(|location| matches!(location, RegisterLocation::Register(_))));
    }

    #[test]
    fn live_intervals_are_sorted_by_program_position_not_virtual_register_id() {
        let mut candidate = function();
        candidate.blocks[0].instructions = vec![
            NativeMirInst::Const {
                destination: VirtualRegister(2),
                value: 7,
                ty: MachineType::I32,
            }
            .into(),
            NativeMirInst::Const {
                destination: VirtualRegister(1),
                value: 9,
                ty: MachineType::I32,
            }
            .into(),
        ];
        candidate.blocks[0].terminator = NativeTerminator::Return(Some(VirtualRegister(1)));
        let intervals = live_intervals(&candidate);
        assert!(intervals.windows(2).all(|pair| {
            (pair[0].start, pair[0].end, pair[0].register.0)
                <= (pair[1].start, pair[1].end, pair[1].register.0)
        }));
        assert!(
            intervals
                .iter()
                .position(|interval| interval.register == VirtualRegister(2))
                .unwrap()
                < intervals
                    .iter()
                    .position(|interval| interval.register == VirtualRegister(1))
                    .unwrap()
        );
    }

    #[test]
    fn o1_folds_pure_constants_and_preserves_divide_faults() {
        let mut candidate = function();
        candidate
            .register_types
            .extend([MachineType::I32, MachineType::I32]);
        candidate.blocks[0].instructions = vec![
            NativeMirInst::Const {
                destination: VirtualRegister(1),
                value: 40,
                ty: MachineType::I32,
            }
            .into(),
            NativeMirInst::Binary {
                destination: VirtualRegister(2),
                op: NativeBinaryOp::Add,
                left: NativeValue::Register(VirtualRegister(1)),
                right: NativeValue::Immediate(2),
                ty: MachineType::I32,
            }
            .into(),
            NativeMirInst::Binary {
                destination: VirtualRegister(3),
                op: NativeBinaryOp::SignedDiv,
                left: NativeValue::Immediate(i32::MIN as u32 as u64),
                right: NativeValue::Immediate(u32::MAX as u64),
                ty: MachineType::I32,
            }
            .into(),
            NativeMirInst::Binary {
                destination: VirtualRegister(4),
                op: NativeBinaryOp::SignedDiv,
                left: NativeValue::Immediate(9),
                right: NativeValue::Immediate(0),
                ty: MachineType::I32,
            }
            .into(),
        ];
        candidate.blocks[0].terminator = NativeTerminator::Return(Some(VirtualRegister(4)));
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: Vec::new(),
            functions: vec![candidate],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        let instructions = &project.functions[0].blocks[0].instructions;
        assert!(instructions.iter().any(|instruction| matches!(
            &instruction.instruction,
            NativeMirInst::Binary {
                destination: VirtualRegister(4),
                op: NativeBinaryOp::SignedDiv,
                right: NativeValue::Immediate(0),
                ..
            }
        )));
        assert!(instructions.iter().any(|instruction| matches!(
            &instruction.instruction,
            NativeMirInst::Binary {
                destination: VirtualRegister(3),
                op: NativeBinaryOp::SignedDiv,
                ..
            }
        )));
    }

    #[test]
    fn o1_removes_unreachable_blocks_and_constant_branches() {
        let mut candidate = function();
        candidate.blocks = vec![
            NativeMirBlock {
                id: BlockId(0),
                instructions: vec![NativeMirInst::Const {
                    destination: VirtualRegister(1),
                    value: 1,
                    ty: MachineType::I32,
                }
                .into()],
                terminator: NativeTerminator::Branch {
                    condition: NativeValue::Register(VirtualRegister(1)),
                    then_block: BlockId(1),
                    else_block: BlockId(2),
                },
            },
            NativeMirBlock {
                id: BlockId(1),
                instructions: vec![],
                terminator: NativeTerminator::Return(Some(VirtualRegister(0))),
            },
            NativeMirBlock {
                id: BlockId(2),
                instructions: vec![],
                terminator: NativeTerminator::Return(Some(VirtualRegister(0))),
            },
            NativeMirBlock {
                id: BlockId(9),
                instructions: vec![],
                terminator: NativeTerminator::Trap,
            },
        ];
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: Vec::new(),
            functions: vec![candidate],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        assert_eq!(project.functions[0].blocks.len(), 3);
        assert_eq!(
            project.functions[0].blocks[0].terminator,
            NativeTerminator::Jump(BlockId(1))
        );
    }

    #[test]
    fn o1_preserves_definitions_live_across_cfg_edges() {
        let mut candidate = function();
        candidate.blocks = vec![
            NativeMirBlock {
                id: BlockId(0),
                instructions: vec![NativeMirInst::Binary {
                    destination: VirtualRegister(1),
                    op: NativeBinaryOp::Add,
                    left: NativeValue::Register(VirtualRegister(0)),
                    right: NativeValue::Immediate(1),
                    ty: MachineType::I32,
                }
                .into()],
                terminator: NativeTerminator::Jump(BlockId(1)),
            },
            NativeMirBlock {
                id: BlockId(1),
                instructions: vec![NativeMirInst::Copy {
                    destination: VirtualRegister(2),
                    source: VirtualRegister(1),
                    ty: MachineType::I32,
                }
                .into()],
                terminator: NativeTerminator::Return(Some(VirtualRegister(2))),
            },
        ];
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: Vec::new(),
            functions: vec![candidate],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        assert!(matches!(
            project.functions[0].blocks[0].instructions[0].instruction,
            NativeMirInst::Binary {
                destination: VirtualRegister(1),
                ..
            }
        ));
    }

    #[test]
    fn o1_folds_constant_casts_and_safe_identity_operations() {
        let mut candidate = function();
        candidate
            .register_types
            .extend([MachineType::I32, MachineType::I64, MachineType::I64]);
        candidate.blocks[0].instructions = vec![
            NativeMirInst::Const {
                destination: VirtualRegister(3),
                value: u32::MAX as u64,
                ty: MachineType::I32,
            }
            .into(),
            NativeMirInst::Cast {
                destination: VirtualRegister(4),
                value: VirtualRegister(3),
                from: MachineType::I32,
                to: MachineType::I64,
                signed: true,
            }
            .into(),
            NativeMirInst::Binary {
                destination: VirtualRegister(5),
                op: NativeBinaryOp::Mul,
                left: NativeValue::Register(VirtualRegister(4)),
                right: NativeValue::Immediate(1),
                ty: MachineType::I64,
            }
            .into(),
        ];
        candidate.blocks[0].terminator = NativeTerminator::Return(Some(VirtualRegister(5)));
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: Vec::new(),
            functions: vec![candidate],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        assert!(project.functions[0].blocks[0]
            .instructions
            .iter()
            .any(|instruction| matches!(
                instruction.instruction,
                NativeMirInst::Const {
                    destination: VirtualRegister(5),
                    value: u64::MAX,
                    ..
                }
            )));
    }

    #[test]
    fn liveness_keeps_values_across_cfg_edges() {
        let mut candidate = function();
        candidate.blocks = vec![
            NativeMirBlock {
                id: BlockId(0),
                instructions: vec![],
                terminator: NativeTerminator::Branch {
                    condition: NativeValue::Register(VirtualRegister(0)),
                    then_block: BlockId(1),
                    else_block: BlockId(2),
                },
            },
            NativeMirBlock {
                id: BlockId(1),
                instructions: candidate.blocks[0].instructions.clone(),
                terminator: NativeTerminator::Return(Some(VirtualRegister(2))),
            },
            NativeMirBlock {
                id: BlockId(2),
                instructions: vec![],
                terminator: NativeTerminator::Return(Some(VirtualRegister(0))),
            },
        ];
        let allocation = allocate_registers(
            &candidate,
            &RegisterPolicy {
                allocatable: vec![PhysicalRegister(0)],
                allocatable_pairs: Vec::new(),
                call_clobbered: BTreeSet::new(),
                maximum_register_bytes: 8,
                pointer_bytes: 8,
            },
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert!(allocation.spill_slots >= 1);
        assert_ne!(
            allocation.locations[&VirtualRegister(0)],
            allocation.locations[&VirtualRegister(1)]
        );
    }

    #[test]
    fn live_values_crossing_calls_avoid_call_clobbered_registers() {
        let mut candidate = function();
        candidate.blocks[0].instructions = vec![
            NativeMirInst::Const {
                destination: VirtualRegister(0),
                value: 7,
                ty: MachineType::I32,
            }
            .into(),
            NativeMirInst::Call {
                destination: Some(VirtualRegister(1)),
                callee: NativeCallee::Direct {
                    symbol: "callee".into(),
                },
                arguments: vec![],
                argument_aggregates: vec![],
                convention: CallConvention::DByte,
                result: Some(MachineType::I32),
                aggregate_result: None,
            }
            .into(),
            NativeMirInst::Binary {
                destination: VirtualRegister(2),
                op: NativeBinaryOp::Add,
                left: NativeValue::Register(VirtualRegister(0)),
                right: NativeValue::Register(VirtualRegister(1)),
                ty: MachineType::I32,
            }
            .into(),
        ];
        candidate.blocks[0].terminator = NativeTerminator::Return(Some(VirtualRegister(2)));
        let allocation = allocate_registers(
            &candidate,
            &RegisterPolicy {
                allocatable: vec![PhysicalRegister(0), PhysicalRegister(1)],
                allocatable_pairs: Vec::new(),
                call_clobbered: BTreeSet::from([PhysicalRegister(0)]),
                maximum_register_bytes: 8,
                pointer_bytes: 8,
            },
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert_ne!(
            allocation.locations[&VirtualRegister(0)],
            RegisterLocation::Register(PhysicalRegister(0))
        );
    }

    #[test]
    fn entry_parameter_live_across_first_call_avoids_call_clobbered_registers() {
        let mut candidate = function();
        candidate.blocks[0].instructions = vec![
            NativeMirInst::Call {
                destination: Some(VirtualRegister(1)),
                callee: NativeCallee::Direct {
                    symbol: "callee".into(),
                },
                arguments: vec![],
                argument_aggregates: vec![],
                convention: CallConvention::DByte,
                result: Some(MachineType::I32),
                aggregate_result: None,
            }
            .into(),
            NativeMirInst::Binary {
                destination: VirtualRegister(2),
                op: NativeBinaryOp::Add,
                left: NativeValue::Register(VirtualRegister(0)),
                right: NativeValue::Register(VirtualRegister(1)),
                ty: MachineType::I32,
            }
            .into(),
        ];
        candidate.blocks[0].terminator = NativeTerminator::Return(Some(VirtualRegister(2)));
        let allocation = allocate_registers(
            &candidate,
            &RegisterPolicy {
                allocatable: vec![PhysicalRegister(0), PhysicalRegister(1)],
                allocatable_pairs: Vec::new(),
                call_clobbered: BTreeSet::from([PhysicalRegister(0)]),
                maximum_register_bytes: 8,
                pointer_bytes: 8,
            },
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert_ne!(
            allocation.locations[&VirtualRegister(0)],
            RegisterLocation::Register(PhysicalRegister(0))
        );
    }

    #[test]
    fn aggregate_sret_destination_is_a_live_input_and_survives_o1() {
        let layout = NativeAggregateLayout {
            bytes: 16,
            alignment: 8,
        };
        let mut candidate = function();
        candidate.parameters.clear();
        candidate.parameter_aggregates.clear();
        candidate.result = None;
        candidate.register_types = vec![MachineType::Pointer, MachineType::Pointer];
        candidate.stack_objects = vec![
            NativeStackObject {
                id: StackObjectId(0),
                bytes: 16,
                alignment: 8,
                address_taken: true,
            },
            NativeStackObject {
                id: StackObjectId(1),
                bytes: 16,
                alignment: 8,
                address_taken: true,
            },
        ];
        candidate.blocks[0].instructions = vec![
            NativeMirInst::StackAddress {
                destination: VirtualRegister(0),
                object: StackObjectId(0),
            }
            .into(),
            NativeMirInst::Call {
                destination: Some(VirtualRegister(0)),
                callee: NativeCallee::Direct {
                    symbol: "aggregate_callee".into(),
                },
                arguments: vec![],
                argument_aggregates: vec![],
                convention: CallConvention::DByte,
                result: None,
                aggregate_result: Some(layout),
            }
            .into(),
            NativeMirInst::StackAddress {
                destination: VirtualRegister(1),
                object: StackObjectId(1),
            }
            .into(),
            NativeMirInst::AggregateCopy {
                destination: VirtualRegister(1),
                source: VirtualRegister(0),
                bytes: 16,
                alignment: 8,
            }
            .into(),
        ];
        candidate.blocks[0].terminator = NativeTerminator::Return(None);

        let call = &candidate.blocks[0].instructions[1].instruction;
        assert_eq!(instruction_definitions(call), None);
        assert!(instruction_uses(call).contains(&VirtualRegister(0)));

        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: Vec::new(),
            functions: vec![candidate],
        };
        project.optimize(NativeOptimizationLevel::O1).unwrap();
        assert!(project.functions[0].blocks[0]
            .instructions
            .iter()
            .any(|instruction| matches!(
                instruction.instruction,
                NativeMirInst::StackAddress {
                    destination: VirtualRegister(0),
                    ..
                }
            )));
    }
    #[test]
    fn x86_64_legalization_widens_narrow_hard_arithmetic_explicitly() {
        let mut candidate = function();
        candidate.register_types[1] = MachineType::I16;
        candidate.register_types[2] = MachineType::I16;
        candidate.blocks[0].instructions = vec![NativeMirInst::Binary {
            destination: VirtualRegister(2),
            op: NativeBinaryOp::SignedDiv,
            left: NativeValue::Register(VirtualRegister(1)),
            right: NativeValue::Immediate(2),
            ty: MachineType::I16,
        }
        .into()];
        candidate.blocks[0].terminator = NativeTerminator::Return(Some(VirtualRegister(2)));
        let mut project = NativeMirProject {
            pointer_bytes: 8,
            globals: Vec::new(),
            functions: vec![candidate],
        };
        legalize_project(&mut project, crate::target::X86_64_LINUX).unwrap();
        let instructions = &project.functions[0].blocks[0].instructions;
        assert!(instructions.iter().any(|instruction| matches!(
            instruction.instruction,
            NativeMirInst::Binary {
                op: NativeBinaryOp::SignedDiv,
                ty: MachineType::I32,
                ..
            }
        )));
        assert!(instructions.iter().any(|instruction| matches!(
            instruction.instruction,
            NativeMirInst::Cast {
                to: MachineType::I16,
                ..
            }
        )));
    }
}
