//! Canonical-HIR compatibility lowering for the frozen v11.0 single-image ABI.
//!
//! This adapter is deliberately representation-only: canonical analysis and
//! monomorphization have already resolved every name and type before this code
//! runs.  It never reads the source AST and it declines programs outside the
//! original scalar/string single-image surface.

use crate::diagnostics::{Diagnostic, DiagnosticCategory, NativeResult};
use crate::ir::{
    CompareOp, IrFunction, IrInst, IrParam, IrProgram, IrSlot, LabelId, LocalId, StringId,
};
use crate::types::{Intrinsic, NativeType};
use dbyte_ast::{BinOp, CallConvention, UnaryOp};
use dbyte_hir::{
    CanonicalHirProject, CanonicalType, ConcreteHirFunction, ConcreteHirProgram, HirBlock, HirExpr,
    HirExprKind, HirIntrinsic, HirPlace, HirPlaceBase, HirProjection, HirStatement, IntegerType,
    TypeId,
};
use std::collections::BTreeMap;

pub fn lower_legacy_single_image(
    project: &CanonicalHirProject,
    concrete: &ConcreteHirProgram,
) -> NativeResult<Option<IrProgram>> {
    if project.module_order.len() != 1
        || !concrete.globals.is_empty()
        || !concrete.top_level.statements.is_empty()
        || concrete.functions.iter().any(|function| {
            function.declaration_only
                || function.interrupt.is_some()
                || function.convention != CallConvention::DByte
                || !function.key.arguments.is_empty()
        })
    {
        return Ok(None);
    }

    let Some(entry) = concrete.functions.iter().find(|function| {
        project
            .link_names
            .get(&function.key.symbol)
            .map(String::as_str)
            == Some("kernel_main")
    }) else {
        return Ok(None);
    };
    if !entry.parameters.is_empty()
        || native_type(concrete, entry.result).ok() != Some(NativeType::I32)
    {
        return Ok(None);
    }

    let mut functions = concrete.functions.iter().collect::<Vec<_>>();
    functions
        .sort_by_key(|function| (function.span.line, function.span.col, function.key.symbol.0));

    let mut strings = Vec::new();
    let mut string_ids = BTreeMap::new();
    let mut lowered = Vec::with_capacity(functions.len());
    for function in functions {
        let Some(adapter) =
            FunctionAdapter::new(project, concrete, function, &mut strings, &mut string_ids)?
        else {
            return Ok(None);
        };
        let Some(function) = adapter.lower()? else {
            return Ok(None);
        };
        lowered.push(function);
    }
    Ok(Some(IrProgram {
        functions: lowered,
        strings,
    }))
}

struct FunctionAdapter<'a, 's> {
    project: &'a CanonicalHirProject,
    program: &'a ConcreteHirProgram,
    function: &'a ConcreteHirFunction,
    strings: &'s mut Vec<Vec<u8>>,
    string_ids: &'s mut BTreeMap<Vec<u8>, StringId>,
    instructions: Vec<IrInst>,
    next_label: u32,
}

impl<'a, 's> FunctionAdapter<'a, 's> {
    fn new(
        project: &'a CanonicalHirProject,
        program: &'a ConcreteHirProgram,
        function: &'a ConcreteHirFunction,
        strings: &'s mut Vec<Vec<u8>>,
        string_ids: &'s mut BTreeMap<Vec<u8>, StringId>,
    ) -> NativeResult<Option<Self>> {
        if function.locals.iter().enumerate().any(|(index, local)| {
            local.id.0 as usize != index || native_type(program, local.ty).is_err()
        }) || native_type(program, function.result).is_err()
        {
            return Ok(None);
        }
        Ok(Some(Self {
            project,
            program,
            function,
            strings,
            string_ids,
            instructions: Vec::new(),
            next_label: 0,
        }))
    }

    fn lower(mut self) -> NativeResult<Option<IrFunction>> {
        if !self.lower_block(&self.function.body)? {
            return Ok(None);
        }
        let mut params = Vec::with_capacity(self.function.parameters.len());
        for parameter in &self.function.parameters {
            params.push(IrParam {
                name: parameter.name.clone(),
                ty: native_type(self.program, parameter.ty)?,
                slot: LocalId(parameter.id.0),
            });
        }
        let mut slots = Vec::with_capacity(self.function.locals.len());
        for local in &self.function.locals {
            slots.push(IrSlot {
                name: local.name.clone(),
                ty: native_type(self.program, local.ty)?,
                is_param: local.parameter,
            });
        }
        let name = self
            .project
            .link_names
            .get(&self.function.key.symbol)
            .cloned()
            .ok_or_else(|| self.error("canonical function has no source link name"))?;
        Ok(Some(IrFunction {
            name,
            params,
            return_type: native_type(self.program, self.function.result)?,
            slots,
            instructions: self.instructions,
            span: self.function.span,
        }))
    }

    fn lower_block(&mut self, block: &HirBlock) -> NativeResult<bool> {
        for statement in &block.statements {
            if !self.lower_statement(statement)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn lower_statement(&mut self, statement: &HirStatement) -> NativeResult<bool> {
        match statement {
            HirStatement::Let { local, value, .. } => {
                if !self.lower_expr(value)? {
                    return Ok(false);
                }
                self.instructions.push(IrInst::LocalStore(LocalId(local.0)));
            }
            HirStatement::Assign { place, value, .. } => {
                let Some(local) = plain_local(place) else {
                    return Ok(false);
                };
                if !self.lower_expr(value)? {
                    return Ok(false);
                }
                self.instructions.push(IrInst::LocalStore(local));
            }
            HirStatement::Expression(value) => {
                if !self.lower_expr(value)? {
                    return Ok(false);
                }
                if native_type(self.program, value.ty).is_ok() {
                    self.instructions.push(IrInst::Pop);
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if !cleanup.is_empty() {
                    return Ok(false);
                }
                let Some(value) = value else {
                    return Ok(false);
                };
                if !self.lower_expr(value)? {
                    return Ok(false);
                }
                self.instructions.push(IrInst::Return);
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                if !self.lower_expr(condition)? {
                    return Ok(false);
                }
                if let Some(else_block) = else_block {
                    let otherwise = self.label();
                    let done = self.label();
                    self.instructions.push(IrInst::JumpIfFalse(otherwise));
                    if !self.lower_block(then_block)? {
                        return Ok(false);
                    }
                    self.instructions.push(IrInst::Jump(done));
                    self.instructions.push(IrInst::Label(otherwise));
                    if !self.lower_block(else_block)? {
                        return Ok(false);
                    }
                    self.instructions.push(IrInst::Label(done));
                } else {
                    let done = self.label();
                    self.instructions.push(IrInst::JumpIfFalse(done));
                    if !self.lower_block(then_block)? {
                        return Ok(false);
                    }
                    self.instructions.push(IrInst::Label(done));
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                let head = self.label();
                let done = self.label();
                self.instructions.push(IrInst::Label(head));
                if !self.lower_expr(condition)? {
                    return Ok(false);
                }
                self.instructions.push(IrInst::JumpIfFalse(done));
                if !self.lower_block(body)? {
                    return Ok(false);
                }
                self.instructions.push(IrInst::Jump(head));
                self.instructions.push(IrInst::Label(done));
            }
            HirStatement::Match { .. }
            | HirStatement::For { .. }
            | HirStatement::Defer { .. }
            | HirStatement::Break { .. }
            | HirStatement::Continue { .. } => return Ok(false),
        }
        Ok(true)
    }

    fn lower_expr(&mut self, expression: &HirExpr) -> NativeResult<bool> {
        match &expression.kind {
            HirExprKind::Unit => {}
            HirExprKind::LayoutOf { .. } => return Ok(false),
            HirExprKind::Integer(value) => {
                let Ok(value) = i32::try_from(*value) else {
                    return Ok(false);
                };
                self.instructions.push(IrInst::ConstI32(value));
            }
            HirExprKind::Bool(value) => self.instructions.push(IrInst::ConstBool(*value)),
            HirExprKind::String(bytes) => {
                if !bytes.is_ascii() {
                    return Ok(false);
                }
                let id = if let Some(id) = self.string_ids.get(bytes) {
                    *id
                } else {
                    let id = StringId(self.strings.len() as u32);
                    self.strings.push(bytes.clone());
                    self.string_ids.insert(bytes.clone(), id);
                    id
                };
                self.instructions.push(IrInst::ConstStr(id));
            }
            HirExprKind::Local(local) => {
                self.instructions.push(IrInst::LocalLoad(LocalId(local.0)));
            }
            HirExprKind::Place(place) => {
                if !self.lower_place_value(place)? {
                    return Ok(false);
                }
            }
            HirExprKind::Binary { left, op, right } => {
                if !self.lower_expr(left)? || !self.lower_expr(right)? {
                    return Ok(false);
                }
                let instruction = match op {
                    BinOp::Add => IrInst::AddI32,
                    BinOp::Sub => IrInst::SubI32,
                    BinOp::Mul => IrInst::MulI32,
                    BinOp::Div => IrInst::DivI32,
                    BinOp::Rem => IrInst::RemI32,
                    BinOp::EqEq => IrInst::Compare(CompareOp::Eq),
                    BinOp::NotEq => IrInst::Compare(CompareOp::NotEq),
                    BinOp::Lt => IrInst::Compare(CompareOp::Lt),
                    BinOp::LtEq => IrInst::Compare(CompareOp::LtEq),
                    BinOp::Gt => IrInst::Compare(CompareOp::Gt),
                    BinOp::GtEq => IrInst::Compare(CompareOp::GtEq),
                    BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                        return Ok(false)
                    }
                };
                self.instructions.push(instruction);
            }
            HirExprKind::Unary { op, value } => {
                if !self.lower_expr(value)? {
                    return Ok(false);
                }
                match op {
                    UnaryOp::Neg => self.instructions.push(IrInst::NegI32),
                    UnaryOp::Not => self.instructions.push(IrInst::BoolNot),
                    UnaryOp::BitNot | UnaryOp::AddressOf | UnaryOp::Deref => return Ok(false),
                }
            }
            HirExprKind::Cast { value, .. } => {
                if !self.lower_expr(value)? {
                    return Ok(false);
                }
            }
            HirExprKind::Drop { .. } => return Ok(false),
            HirExprKind::DirectCall {
                callee,
                convention,
                arguments,
            } => {
                if *convention != CallConvention::DByte {
                    return Ok(false);
                }
                for argument in arguments {
                    if !self.lower_expr(argument)? {
                        return Ok(false);
                    }
                }
                let Some(function) = self
                    .program
                    .functions
                    .iter()
                    .find(|function| function.key == *callee)
                else {
                    return Err(self.error("canonical direct-call target is missing"));
                };
                let Some(name) = self.project.link_names.get(&callee.symbol) else {
                    return Err(self.error("canonical direct-call target has no source link name"));
                };
                self.instructions.push(IrInst::Call {
                    function: name.clone(),
                    argument_count: arguments.len(),
                    result: native_type(self.program, function.result)?,
                });
            }
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } => {
                let Some(intrinsic) = legacy_intrinsic(*intrinsic) else {
                    return Ok(false);
                };
                for argument in arguments {
                    if !self.lower_expr(argument)? {
                        return Ok(false);
                    }
                }
                self.instructions.push(IrInst::Intrinsic {
                    intrinsic,
                    argument_count: arguments.len(),
                    result: intrinsic.result_type(),
                });
            }
            HirExprKind::Constant { value, .. } => {
                if !self.lower_expr(value)? {
                    return Ok(false);
                }
            }
            HirExprKind::Bitcast { .. }
            | HirExprKind::Float(..)
            | HirExprKind::Bytes(..)
            | HirExprKind::Null
            | HirExprKind::Global(..)
            | HirExprKind::Function(..)
            | HirExprKind::List(..)
            | HirExprKind::IndirectCall { .. }
            | HirExprKind::InterruptAddress { .. }
            | HirExprKind::TaskAddress { .. }
            | HirExprKind::Struct { .. }
            | HirExprKind::Enum { .. }
            | HirExprKind::Borrow { .. }
            | HirExprKind::Try { .. }
            | HirExprKind::Length { .. }
            | HirExprKind::Closure { .. }
            | HirExprKind::HostCall { .. }
            | HirExprKind::FormatString(..) => return Ok(false),
        }
        Ok(true)
    }

    fn lower_place_value(&mut self, place: &HirPlace) -> NativeResult<bool> {
        if let Some(local) = plain_local(place) {
            self.instructions.push(IrInst::LocalLoad(local));
            return Ok(true);
        }
        let [HirProjection::Index {
            index,
            element_size: 1,
            ..
        }] = place.projections.as_slice()
        else {
            return Ok(false);
        };
        let HirPlaceBase::Pointer(base) = &place.base else {
            return Ok(false);
        };
        if !matches!(
            self.program.semantic.type_of(base.ty),
            CanonicalType::LegacyStr
        ) || !self.lower_expr(base)?
            || !self.lower_expr(index)?
        {
            return Ok(false);
        }
        self.instructions.push(IrInst::StrIndex8);
        Ok(true)
    }

    fn label(&mut self) -> LabelId {
        let label = LabelId(self.next_label);
        self.next_label += 1;
        label
    }

    fn error(&self, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("E_NATIVE_HIR_COMPAT: {}", message.into()),
            self.function.span,
        )
    }
}

fn plain_local(place: &HirPlace) -> Option<LocalId> {
    match (&place.base, place.projections.as_slice()) {
        (HirPlaceBase::Local(local), []) => Some(LocalId(local.0)),
        _ => None,
    }
}

fn native_type(program: &ConcreteHirProgram, ty: TypeId) -> NativeResult<NativeType> {
    match program.semantic.type_of(ty) {
        CanonicalType::Integer(
            IntegerType::I8
            | IntegerType::U8
            | IntegerType::I16
            | IntegerType::U16
            | IntegerType::I32
            | IntegerType::U32
            | IntegerType::Usize
            | IntegerType::LegacyInt,
        ) => Ok(NativeType::I32),
        CanonicalType::Bool => Ok(NativeType::Bool),
        CanonicalType::LegacyStr => Ok(NativeType::Str),
        _ => Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidType,
            "E_NATIVE_HIR_COMPAT: type is outside the frozen single-image representation",
        )),
    }
}

fn legacy_intrinsic(intrinsic: HirIntrinsic) -> Option<Intrinsic> {
    Some(match intrinsic {
        HirIntrinsic::VolatileLoad8 => Intrinsic::VolatileLoad8,
        HirIntrinsic::VolatileStore8 => Intrinsic::VolatileStore8,
        HirIntrinsic::VolatileLoad16 => Intrinsic::VolatileLoad16,
        HirIntrinsic::VolatileStore16 => Intrinsic::VolatileStore16,
        HirIntrinsic::VolatileLoad32 => Intrinsic::VolatileLoad32,
        HirIntrinsic::VolatileStore32 => Intrinsic::VolatileStore32,
        HirIntrinsic::IoOut8 => Intrinsic::IoOut8,
        HirIntrinsic::IoIn8 => Intrinsic::IoIn8,
        HirIntrinsic::CpuCli => Intrinsic::CpuCli,
        HirIntrinsic::CpuSti => Intrinsic::CpuSti,
        HirIntrinsic::CpuHlt => Intrinsic::CpuHlt,
        HirIntrinsic::CpuReadEflags => Intrinsic::ReadEflags,
        _ => return None,
    })
}
