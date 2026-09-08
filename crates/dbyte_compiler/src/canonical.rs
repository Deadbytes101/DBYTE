//! Representation-only lowering from Concrete HIR to flat canonical bytecode.

use dbyte_ast::{BinOp, Span};
use dbyte_bytecode::*;
use dbyte_hir::{
    integer_semantics::{is_signed, IntegerBinaryOp, IntegerValue},
    CanonicalType, ConcreteHirProject, Diagnostic, FunctionInstanceId, FunctionInstanceKey,
    HirBlock, HirExpr, HirExprKind, HirFormatPart, HirPlace, HirPlaceBase, HirProgram,
    HirProjection, HirStatement, LocalId, TypeId,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy)]
struct Label(usize);

enum PatchKind {
    Jump,
    JumpFalse,
    ResultError,
    IteratorBegin,
    IteratorNext,
    MatchArm(usize),
}

struct Patch {
    instruction: usize,
    label: Label,
    kind: PatchKind,
}

#[derive(Clone)]
enum UnwindAction {
    Iterator(LocalId),
    Match,
}

struct LoopTarget {
    continue_label: Label,
    break_label: Label,
    unwind_depth: usize,
    continue_unwind_depth: usize,
}

struct DeferredError {
    label: Label,
    cleanup: Vec<HirBlock>,
    unwind: Vec<UnwindAction>,
    span: Span,
}

struct LoweringContext<'a> {
    instances: BTreeMap<FunctionInstanceKey, FunctionInstanceId>,
    semantic: &'a HirProgram,
    closures: BTreeMap<dbyte_hir::ClosureId, CanonicalBytecodeClosure>,
    optimization: VmOptimizationLevel,
}

impl LoweringContext<'_> {
    fn instance(
        &self,
        key: &FunctionInstanceKey,
        span: Span,
    ) -> Result<FunctionInstanceId, Diagnostic> {
        self.instances.get(key).copied().ok_or(Diagnostic {
            code: "E_VM_HIR_UNRESOLVED_INSTANCE",
            message: format!(
                "concrete function instance is missing for symbol#{}",
                key.symbol.0
            ),
            span,
        })
    }

    fn operand_type(&self, ty: TypeId) -> CanonicalOperandType {
        match self.semantic.type_of(ty) {
            CanonicalType::Integer(kind) => CanonicalOperandType::Integer(*kind),
            CanonicalType::Bool => CanonicalOperandType::Bool,
            CanonicalType::Float => CanonicalOperandType::Float,
            CanonicalType::Pointer { .. } | CanonicalType::Borrow { .. } => {
                CanonicalOperandType::Pointer
            }
            CanonicalType::LegacyStr | CanonicalType::StringView => CanonicalOperandType::String,
            _ => CanonicalOperandType::Other(ty),
        }
    }

    fn lower_body(
        &mut self,
        block: &HirBlock,
        locals: impl IntoIterator<Item = dbyte_hir::HirLocal>,
        terminal: CanonicalOpcode,
        optimization: VmOptimizationLevel,
    ) -> Result<CanonicalBytecodeBody, Diagnostic> {
        let locals = locals.into_iter().collect::<Vec<_>>();
        let mut builder = BodyBuilder::new(self, &locals);
        builder.lower_block(block)?;
        builder.emit(CanonicalOpcode::PushUnit, Span::zero());
        builder.emit(terminal, Span::zero());
        let mut body = builder.finish()?;
        if optimization == VmOptimizationLevel::O1 {
            optimize_body(&mut body, self.semantic.layout);
        }
        Ok(body)
    }
}

struct BodyBuilder<'context, 'semantic> {
    context: &'context mut LoweringContext<'semantic>,
    instructions: Vec<CanonicalInstruction>,
    labels: Vec<Option<usize>>,
    patches: Vec<Patch>,
    loops: Vec<LoopTarget>,
    unwind: Vec<UnwindAction>,
    deferred_errors: VecDeque<DeferredError>,
    local_count: usize,
    stable_cells: Vec<bool>,
}

impl<'context, 'semantic> BodyBuilder<'context, 'semantic> {
    fn new(
        context: &'context mut LoweringContext<'semantic>,
        locals: &[dbyte_hir::HirLocal],
    ) -> Self {
        let local_count = locals
            .iter()
            .map(|local| local.id.0 as usize + 1)
            .max()
            .unwrap_or(0);
        Self {
            context,
            instructions: Vec::new(),
            labels: Vec::new(),
            patches: Vec::new(),
            loops: Vec::new(),
            unwind: Vec::new(),
            deferred_errors: VecDeque::new(),
            local_count,
            stable_cells: vec![false; local_count],
        }
    }

    fn emit(&mut self, opcode: CanonicalOpcode, span: Span) -> usize {
        let index = self.instructions.len();
        self.instructions
            .push(CanonicalInstruction { opcode, span });
        index
    }

    fn label(&mut self) -> Label {
        let label = Label(self.labels.len());
        self.labels.push(None);
        label
    }

    fn mark(&mut self, label: Label) {
        assert!(self.labels[label.0]
            .replace(self.instructions.len())
            .is_none());
    }

    fn jump(&mut self, label: Label, span: Span) {
        let instruction = self.emit(CanonicalOpcode::Jump(usize::MAX), span);
        self.patches.push(Patch {
            instruction,
            label,
            kind: PatchKind::Jump,
        });
    }

    fn jump_false(&mut self, label: Label, span: Span) {
        let instruction = self.emit(CanonicalOpcode::JumpIfFalse(usize::MAX), span);
        self.patches.push(Patch {
            instruction,
            label,
            kind: PatchKind::JumpFalse,
        });
    }

    fn lower_block(&mut self, block: &HirBlock) -> Result<(), Diagnostic> {
        for statement in &block.statements {
            self.lower_statement(statement)?;
        }
        Ok(())
    }

    fn lower_statement(&mut self, statement: &HirStatement) -> Result<(), Diagnostic> {
        match statement {
            HirStatement::Let { local, value, span } => {
                self.ensure_local(*local);
                self.lower_expr(value)?;
                self.emit(CanonicalOpcode::StoreLocal(*local), *span);
            }
            HirStatement::Assign {
                place,
                value,
                cleanup_before_commit,
                span,
            } => {
                self.lower_expr(value)?;
                let flat = self.lower_place_operands(place)?;
                self.emit(CanonicalOpcode::ResolvePlace(flat), place.span);
                if let Some(cleanup) = cleanup_before_commit {
                    self.lower_block(cleanup)?;
                }
                self.emit(CanonicalOpcode::StorePlace, *span);
            }
            HirStatement::Expression(value) => {
                self.lower_expr(value)?;
                self.emit(CanonicalOpcode::Pop, value.span);
            }
            HirStatement::Return {
                value,
                cleanup,
                span,
            } => {
                if let Some(value) = value {
                    self.lower_expr(value)?;
                } else {
                    self.emit(CanonicalOpcode::PushUnit, *span);
                }
                for block in cleanup {
                    self.lower_block(block)?;
                }
                self.emit_unwind(0, *span);
                self.emit(CanonicalOpcode::Return, *span);
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                span,
            } => {
                let else_label = self.label();
                let end = self.label();
                self.lower_expr(condition)?;
                self.jump_false(else_label, condition.span);
                self.lower_block(then_block)?;
                self.jump(end, *span);
                self.mark(else_label);
                if let Some(block) = else_block {
                    self.lower_block(block)?;
                }
                self.mark(end);
            }
            HirStatement::While {
                condition,
                body,
                span,
            } => {
                let condition_label = self.label();
                let end = self.label();
                self.mark(condition_label);
                self.lower_expr(condition)?;
                self.jump_false(end, condition.span);
                let depth = self.unwind.len();
                self.loops.push(LoopTarget {
                    continue_label: condition_label,
                    break_label: end,
                    unwind_depth: depth,
                    continue_unwind_depth: depth,
                });
                self.lower_block(body)?;
                self.loops.pop();
                self.jump(condition_label, *span);
                self.mark(end);
            }
            HirStatement::For {
                local,
                iterable,
                body,
                span,
            } => {
                self.ensure_local(local.id);
                self.lower_expr(iterable)?;
                let next = self.label();
                let end = self.label();
                let instruction = self.emit(
                    CanonicalOpcode::IteratorBegin {
                        local: local.id,
                        end_target: usize::MAX,
                    },
                    *span,
                );
                self.patches.push(Patch {
                    instruction,
                    label: end,
                    kind: PatchKind::IteratorBegin,
                });
                self.unwind.push(UnwindAction::Iterator(local.id));
                let loop_unwind = self.unwind.len() - 1;
                self.loops.push(LoopTarget {
                    continue_label: next,
                    break_label: end,
                    unwind_depth: loop_unwind,
                    continue_unwind_depth: self.unwind.len(),
                });
                self.mark(next);
                let instruction = self.emit(
                    CanonicalOpcode::IteratorNext {
                        local: local.id,
                        end_target: usize::MAX,
                    },
                    *span,
                );
                self.patches.push(Patch {
                    instruction,
                    label: end,
                    kind: PatchKind::IteratorNext,
                });
                self.lower_block(body)?;
                self.jump(next, *span);
                self.loops.pop();
                self.mark(end);
                self.emit(CanonicalOpcode::IteratorEnd { local: local.id }, *span);
                self.unwind.pop();
            }
            HirStatement::Match {
                mode,
                value,
                arms,
                span,
                ..
            } => {
                self.lower_expr(value)?;
                let labels = arms.iter().map(|_| self.label()).collect::<Vec<_>>();
                let end = self.label();
                let targets = arms
                    .iter()
                    .map(|arm| CanonicalMatchTarget {
                        variant: arm.variant.map(|variant| variant.variant),
                        bindings: arm.bindings.clone(),
                        target: usize::MAX,
                    })
                    .collect::<Vec<_>>();
                let instruction = self.emit(
                    CanonicalOpcode::MatchDispatch {
                        mode: *mode,
                        arms: targets,
                    },
                    *span,
                );
                for (index, label) in labels.iter().copied().enumerate() {
                    self.patches.push(Patch {
                        instruction,
                        label,
                        kind: PatchKind::MatchArm(index),
                    });
                }
                for (arm, label) in arms.iter().zip(labels) {
                    self.mark(label);
                    for binding in &arm.bindings {
                        self.ensure_local(binding.id);
                    }
                    self.unwind.push(UnwindAction::Match);
                    self.lower_block(&arm.body)?;
                    self.emit(CanonicalOpcode::MatchRestore, arm.span);
                    self.unwind.pop();
                    self.jump(end, arm.span);
                }
                self.mark(end);
            }
            HirStatement::Defer { .. } => {}
            HirStatement::Break { cleanup, span } => {
                let Some(target) = self.loops.last() else {
                    return Err(internal("break has no loop target", *span));
                };
                let break_label = target.break_label;
                let depth = target.unwind_depth;
                for block in cleanup {
                    self.lower_block(block)?;
                }
                self.emit_unwind(depth, *span);
                self.jump(break_label, *span);
            }
            HirStatement::Continue { cleanup, span } => {
                let Some(target) = self.loops.last() else {
                    return Err(internal("continue has no loop target", *span));
                };
                let continue_label = target.continue_label;
                let depth = target.continue_unwind_depth;
                for block in cleanup {
                    self.lower_block(block)?;
                }
                self.emit_unwind(depth, *span);
                self.jump(continue_label, *span);
            }
        }
        Ok(())
    }

    fn emit_unwind(&mut self, depth: usize, span: Span) {
        let actions = self.unwind[depth..]
            .iter()
            .cloned()
            .rev()
            .collect::<Vec<_>>();
        for action in actions {
            match action {
                UnwindAction::Iterator(local) => {
                    self.emit(CanonicalOpcode::IteratorEnd { local }, span);
                }
                UnwindAction::Match => {
                    self.emit(CanonicalOpcode::MatchRestore, span);
                }
            }
        }
    }

    fn lower_expr(&mut self, expression: &HirExpr) -> Result<(), Diagnostic> {
        let span = expression.span;
        match &expression.kind {
            HirExprKind::Unit => {
                self.emit(CanonicalOpcode::PushUnit, span);
            }
            HirExprKind::Integer(value) => {
                self.emit(CanonicalOpcode::PushInteger(*value), span);
            }
            HirExprKind::LayoutOf { target, alignment } => {
                self.emit(
                    CanonicalOpcode::PushLayout {
                        target: *target,
                        alignment: *alignment,
                    },
                    span,
                );
            }
            HirExprKind::Length { value } => {
                self.lower_expr(value)?;
                self.emit(CanonicalOpcode::Length, span);
            }
            HirExprKind::Float(value) => {
                self.emit(CanonicalOpcode::PushFloat(*value), span);
            }
            HirExprKind::Bool(value) => {
                self.emit(CanonicalOpcode::PushBool(*value), span);
            }
            HirExprKind::String(value) => {
                self.emit(CanonicalOpcode::PushString(value.clone()), span);
            }
            HirExprKind::Bytes(value) => {
                self.emit(CanonicalOpcode::PushBytes(value.clone()), span);
            }
            HirExprKind::Null => {
                self.emit(CanonicalOpcode::PushNull, span);
            }
            HirExprKind::Local(local) => {
                self.ensure_local(*local);
                self.emit(
                    CanonicalOpcode::LoadLocal {
                        local: *local,
                        action: expression.ownership,
                    },
                    span,
                );
            }
            HirExprKind::Global(symbol) => {
                self.emit(
                    CanonicalOpcode::LoadGlobal {
                        symbol: *symbol,
                        action: expression.ownership,
                    },
                    span,
                );
            }
            HirExprKind::Constant { value, .. } => self.lower_expr(value)?,
            HirExprKind::Function(key) => {
                let instance = self.context.instance(key, span)?;
                self.emit(
                    CanonicalOpcode::PushFunction {
                        instance,
                        key: key.clone(),
                    },
                    span,
                );
            }
            HirExprKind::List(values) => {
                self.lower_many(values)?;
                self.emit(CanonicalOpcode::MakeList(values.len()), span);
            }
            HirExprKind::Binary { left, op, right } => {
                self.lower_expr(left)?;
                self.lower_expr(right)?;
                self.emit(
                    CanonicalOpcode::Binary {
                        op: op.clone(),
                        operand: self.context.operand_type(left.ty),
                    },
                    span,
                );
            }
            HirExprKind::Unary { op, value } => {
                self.lower_expr(value)?;
                self.emit(
                    CanonicalOpcode::Unary {
                        op: op.clone(),
                        operand: self.context.operand_type(value.ty),
                    },
                    span,
                );
            }
            HirExprKind::Cast { value, target } => {
                self.lower_expr(value)?;
                self.emit(CanonicalOpcode::Cast(*target), span);
            }
            HirExprKind::Bitcast { value, target } => {
                self.lower_expr(value)?;
                self.emit(
                    CanonicalOpcode::Bitcast {
                        source: value.ty,
                        target: *target,
                    },
                    span,
                );
            }
            HirExprKind::DirectCall {
                callee,
                convention,
                arguments,
            } => {
                self.lower_many(arguments)?;
                let instance = self.context.instance(callee, span)?;
                self.emit(
                    CanonicalOpcode::CallDirect {
                        callee: instance,
                        key: callee.clone(),
                        convention: *convention,
                        argument_count: arguments.len(),
                    },
                    span,
                );
            }
            HirExprKind::IndirectCall {
                callee,
                convention,
                arguments,
            } => {
                self.lower_expr(callee)?;
                self.lower_many(arguments)?;
                self.emit(
                    CanonicalOpcode::CallIndirect {
                        convention: *convention,
                        argument_count: arguments.len(),
                    },
                    span,
                );
            }
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } => {
                self.lower_many(arguments)?;
                self.emit(
                    CanonicalOpcode::Intrinsic {
                        intrinsic: *intrinsic,
                        argument_count: arguments.len(),
                        result: expression.ty,
                    },
                    span,
                );
            }
            HirExprKind::HostCall {
                capability,
                arguments,
            } => {
                self.lower_many(arguments)?;
                self.emit(
                    CanonicalOpcode::HostCall {
                        capability: *capability,
                        argument_count: arguments.len(),
                    },
                    span,
                );
            }
            HirExprKind::FormatString(parts) => {
                let mut descriptors = Vec::with_capacity(parts.len());
                let mut count = 0;
                for part in parts {
                    match part {
                        HirFormatPart::Literal(value) => {
                            descriptors.push(CanonicalFormatPart::Literal(value.clone()))
                        }
                        HirFormatPart::Value(value) => {
                            self.lower_expr(value)?;
                            descriptors.push(CanonicalFormatPart::Value);
                            count += 1;
                        }
                    }
                }
                self.emit(
                    CanonicalOpcode::Format {
                        parts: descriptors,
                        value_count: count,
                    },
                    span,
                );
            }
            HirExprKind::InterruptAddress { .. } | HirExprKind::TaskAddress { .. } => {
                return Err(Diagnostic {
                    code: "E_VM_HIR_UNSUPPORTED",
                    message: "target address metadata is unavailable in VM bytecode".into(),
                    span,
                })
            }
            HirExprKind::Struct { symbol, fields } => {
                for (_, value) in fields {
                    self.lower_expr(value)?;
                }
                self.emit(
                    CanonicalOpcode::MakeStruct {
                        symbol: *symbol,
                        fields: fields.iter().map(|(field, _)| *field).collect(),
                    },
                    span,
                );
            }
            HirExprKind::Enum {
                symbol,
                variant,
                discriminant,
                payload,
            } => {
                self.lower_many(payload)?;
                self.emit(
                    CanonicalOpcode::MakeEnum {
                        symbol: *symbol,
                        variant: *variant,
                        discriminant: *discriminant,
                        payload_count: payload.len(),
                    },
                    span,
                );
            }
            HirExprKind::Place(place) => {
                let place = self.lower_place_operands(place)?;
                self.emit(CanonicalOpcode::ResolvePlace(place), span);
                self.emit(CanonicalOpcode::ReadPlace(expression.ownership), span);
            }
            HirExprKind::Borrow { mutable, place } => {
                self.mark_stable_place(place);
                let place = self.lower_place_operands(place)?;
                self.emit(CanonicalOpcode::ResolvePlace(place), span);
                self.emit(CanonicalOpcode::BorrowPlace { mutable: *mutable }, span);
            }
            HirExprKind::Try {
                value,
                ok_variant,
                err_variant,
                cleanup,
            } => {
                self.lower_expr(value)?;
                let label = self.label();
                let instruction = self.emit(
                    CanonicalOpcode::ResultUnwrapOrJump {
                        ok_variant: *ok_variant,
                        err_variant: *err_variant,
                        error_target: usize::MAX,
                    },
                    span,
                );
                self.patches.push(Patch {
                    instruction,
                    label,
                    kind: PatchKind::ResultError,
                });
                self.deferred_errors.push_back(DeferredError {
                    label,
                    cleanup: cleanup.clone(),
                    unwind: self.unwind.clone(),
                    span,
                });
            }
            HirExprKind::Drop { place, cleanup } => {
                self.lower_block(cleanup)?;
                let place = self.lower_place_operands(place)?;
                self.emit(CanonicalOpcode::ResolvePlace(place), span);
                self.emit(CanonicalOpcode::ClearPlace, span);
                self.emit(CanonicalOpcode::PushUnit, span);
            }
            HirExprKind::Closure {
                closure,
                captures,
                parameters,
                result,
                body,
            } => {
                for capture in captures {
                    self.mark_stable(capture.local);
                }
                if !self.context.closures.contains_key(closure) {
                    let mut closure_locals = parameters.clone();
                    closure_locals.extend(captures.iter().map(|capture| dbyte_hir::HirLocal {
                        id: capture.local,
                        name: format!("capture#{}", capture.local.0),
                        ty: capture.ty,
                        mutable: capture.mode == dbyte_ast::CaptureMode::BorrowMut,
                        parameter: false,
                        span: capture.span,
                    }));
                    let body = self.context.lower_body(
                        body,
                        closure_locals,
                        CanonicalOpcode::Return,
                        self.context.optimization,
                    )?;
                    self.context.closures.insert(
                        *closure,
                        CanonicalBytecodeClosure {
                            closure: *closure,
                            parameters: parameters.clone(),
                            result: *result,
                            body,
                            span,
                        },
                    );
                }
                self.emit(
                    CanonicalOpcode::MakeClosure {
                        closure: *closure,
                        captures: captures
                            .iter()
                            .map(|capture| CanonicalCapture {
                                local: capture.local,
                                mode: capture.mode,
                                ty: capture.ty,
                                span: capture.span,
                            })
                            .collect(),
                    },
                    span,
                );
            }
        }
        Ok(())
    }

    fn lower_many(&mut self, values: &[HirExpr]) -> Result<(), Diagnostic> {
        for value in values {
            self.lower_expr(value)?;
        }
        Ok(())
    }

    fn lower_place_operands(&mut self, place: &HirPlace) -> Result<CanonicalPlace, Diagnostic> {
        let base = match &place.base {
            HirPlaceBase::Local(local) => {
                self.ensure_local(*local);
                CanonicalPlaceBase::Local(*local)
            }
            HirPlaceBase::Global(symbol) => CanonicalPlaceBase::Global(*symbol),
            HirPlaceBase::Pointer(pointer) => {
                self.lower_expr(pointer)?;
                CanonicalPlaceBase::StackPointer
            }
        };
        let mut projections = Vec::with_capacity(place.projections.len());
        for projection in &place.projections {
            projections.push(match projection {
                HirProjection::Field {
                    field, ty, offset, ..
                } => CanonicalProjection::Field {
                    field: *field,
                    ty: *ty,
                    offset: *offset,
                },
                HirProjection::Payload {
                    variant,
                    index,
                    ty,
                    offset,
                } => CanonicalProjection::Payload {
                    variant: *variant,
                    index: *index,
                    ty: *ty,
                    offset: *offset,
                },
                HirProjection::Index {
                    index,
                    element,
                    element_size,
                } => {
                    self.lower_expr(index)?;
                    CanonicalProjection::StackIndex {
                        element: *element,
                        element_size: *element_size,
                    }
                }
            });
        }
        Ok(CanonicalPlace {
            base,
            projections,
            ty: place.ty,
            mutable: place.mutable,
        })
    }

    fn mark_stable_place(&mut self, place: &HirPlace) {
        if let HirPlaceBase::Local(local) = place.base {
            self.mark_stable(local);
        }
    }

    fn ensure_local(&mut self, local: LocalId) {
        let needed = local.0 as usize + 1;
        if self.local_count < needed {
            self.local_count = needed;
            self.stable_cells.resize(needed, false);
        }
    }

    fn mark_stable(&mut self, local: LocalId) {
        self.ensure_local(local);
        self.stable_cells[local.0 as usize] = true;
    }

    fn finish(mut self) -> Result<CanonicalBytecodeBody, Diagnostic> {
        while let Some(error) = self.deferred_errors.pop_front() {
            self.mark(error.label);
            for block in &error.cleanup {
                self.lower_block(block)?;
            }
            for action in error.unwind.into_iter().rev() {
                match action {
                    UnwindAction::Iterator(local) => {
                        self.emit(CanonicalOpcode::IteratorEnd { local }, error.span);
                    }
                    UnwindAction::Match => {
                        self.emit(CanonicalOpcode::MatchRestore, error.span);
                    }
                }
            }
            self.emit(CanonicalOpcode::Return, error.span);
        }
        for patch in self.patches {
            let target = self
                .labels
                .get(patch.label.0)
                .and_then(|value| *value)
                .ok_or_else(|| internal("bytecode label was not finalized", Span::zero()))?;
            let opcode = &mut self.instructions[patch.instruction].opcode;
            match (&patch.kind, opcode) {
                (PatchKind::Jump, CanonicalOpcode::Jump(value))
                | (PatchKind::JumpFalse, CanonicalOpcode::JumpIfFalse(value))
                | (
                    PatchKind::ResultError,
                    CanonicalOpcode::ResultUnwrapOrJump {
                        error_target: value,
                        ..
                    },
                )
                | (
                    PatchKind::IteratorBegin,
                    CanonicalOpcode::IteratorBegin {
                        end_target: value, ..
                    },
                )
                | (
                    PatchKind::IteratorNext,
                    CanonicalOpcode::IteratorNext {
                        end_target: value, ..
                    },
                ) => *value = target,
                (PatchKind::MatchArm(index), CanonicalOpcode::MatchDispatch { arms, .. }) => {
                    arms[*index].target = target
                }
                _ => {
                    return Err(internal(
                        "bytecode label patch kind mismatch",
                        self.instructions[patch.instruction].span,
                    ))
                }
            }
        }
        Ok(CanonicalBytecodeBody {
            instructions: self.instructions,
            frame: CanonicalFrameLayout {
                local_count: self.local_count,
                stable_cells: self.stable_cells,
            },
        })
    }
}

pub fn lower_concrete_project_to_bytecode(
    project: &ConcreteHirProject,
) -> Result<CanonicalBytecodeProject, Diagnostic> {
    lower_concrete_project_to_bytecode_at_level(project, VmOptimizationLevel::O1)
}

pub fn lower_concrete_project_to_bytecode_at_level(
    project: &ConcreteHirProject,
    optimization: VmOptimizationLevel,
) -> Result<CanonicalBytecodeProject, Diagnostic> {
    let mut context = LoweringContext {
        instances: project
            .executable
            .functions
            .iter()
            .map(|function| (function.key.clone(), function.instance))
            .collect(),
        semantic: &project.executable.semantic,
        closures: BTreeMap::new(),
        optimization,
    };
    let mut functions = Vec::with_capacity(project.executable.functions.len());
    for function in &project.executable.functions {
        let locals = function
            .parameters
            .iter()
            .chain(&function.locals)
            .cloned()
            .collect::<Vec<_>>();
        let body = context.lower_body(
            &function.body,
            locals,
            CanonicalOpcode::Return,
            optimization,
        )?;
        functions.push(CanonicalBytecodeFunction {
            instance: function.instance,
            key: function.key.clone(),
            declaration_only: function.declaration_only,
            parameters: function.parameters.clone(),
            result: function.result,
            body,
            span: function.span,
        });
    }
    let mut entry_hir = HirBlock::default();
    for global in &project.executable.globals {
        if let Some(initializer) = &global.initializer {
            entry_hir
                .statements
                .push(HirStatement::Expression(initializer.clone()));
        }
    }
    entry_hir
        .statements
        .extend(project.executable.top_level.statements.clone());
    let mut entry =
        context.lower_body(&entry_hir, Vec::new(), CanonicalOpcode::Halt, optimization)?;
    // Global initializer expressions are the prefix of entry. Replace their
    // expression POPs with canonical global stores in declaration order.
    let mut cursor = 0usize;
    for global in &project.executable.globals {
        if global.initializer.is_some() {
            while cursor < entry.instructions.len()
                && !matches!(entry.instructions[cursor].opcode, CanonicalOpcode::Pop)
            {
                cursor += 1;
            }
            if cursor >= entry.instructions.len() {
                return Err(internal("global initializer boundary missing", global.span));
            }
            entry.instructions[cursor].opcode = CanonicalOpcode::StoreGlobal(global.symbol);
            cursor += 1;
        }
    }
    let closures = context.closures.into_values().collect::<Vec<_>>();
    if closures
        .iter()
        .enumerate()
        .any(|(index, closure)| closure.closure.0 as usize != index)
    {
        return Err(internal(
            "closure identity table is not dense",
            Span::zero(),
        ));
    }
    Ok(CanonicalBytecodeProject {
        root: project.root,
        module_order: project.module_order.clone(),
        symbol_owners: project.symbol_owners.clone(),
        link_names: project.link_names.clone(),
        semantic: project.executable.semantic.clone(),
        functions,
        closures,
        globals: project
            .executable
            .globals
            .iter()
            .map(|global| CanonicalBytecodeGlobal {
                symbol: global.symbol,
                mutable: global.mutable,
                ty: global.ty,
                span: global.span,
            })
            .collect(),
        entry,
    })
}

fn optimize_body(body: &mut CanonicalBytecodeBody, layout: dbyte_hir::TargetDataLayout) {
    let targets = branch_targets(&body.instructions);
    let old = std::mem::take(&mut body.instructions);
    let mut new = Vec::with_capacity(old.len());
    let mut old_to_new = vec![0usize; old.len() + 1];
    let mut index = 0usize;
    while index < old.len() {
        old_to_new[index] = new.len();
        if index + 2 < old.len()
            && !targets.contains(&(index + 1))
            && !targets.contains(&(index + 2))
        {
            if let (
                CanonicalOpcode::PushInteger(left),
                CanonicalOpcode::PushInteger(right),
                CanonicalOpcode::Binary {
                    op,
                    operand: CanonicalOperandType::Integer(kind),
                },
            ) = (
                &old[index].opcode,
                &old[index + 1].opcode,
                &old[index + 2].opcode,
            ) {
                if let Some(opcode) = fold_integer(*left, *right, *kind, op, layout) {
                    new.push(CanonicalInstruction {
                        opcode,
                        span: old[index + 2].span,
                    });
                    old_to_new[index + 1] = new.len() - 1;
                    old_to_new[index + 2] = new.len() - 1;
                    index += 3;
                    continue;
                }
            }
        }
        new.push(old[index].clone());
        index += 1;
    }
    old_to_new[old.len()] = new.len();
    for instruction in &mut new {
        remap_targets(&mut instruction.opcode, &old_to_new);
    }
    body.instructions = new;
}

fn branch_targets(instructions: &[CanonicalInstruction]) -> BTreeSet<usize> {
    let mut targets = BTreeSet::new();
    for instruction in instructions {
        match &instruction.opcode {
            CanonicalOpcode::Jump(target)
            | CanonicalOpcode::JumpIfFalse(target)
            | CanonicalOpcode::ResultUnwrapOrJump {
                error_target: target,
                ..
            }
            | CanonicalOpcode::IteratorBegin {
                end_target: target, ..
            }
            | CanonicalOpcode::IteratorNext {
                end_target: target, ..
            } => {
                targets.insert(*target);
            }
            CanonicalOpcode::MatchDispatch { arms, .. } => {
                targets.extend(arms.iter().map(|arm| arm.target));
            }
            _ => {}
        }
    }
    targets
}

fn remap_targets(opcode: &mut CanonicalOpcode, map: &[usize]) {
    match opcode {
        CanonicalOpcode::Jump(target)
        | CanonicalOpcode::JumpIfFalse(target)
        | CanonicalOpcode::ResultUnwrapOrJump {
            error_target: target,
            ..
        }
        | CanonicalOpcode::IteratorBegin {
            end_target: target, ..
        }
        | CanonicalOpcode::IteratorNext {
            end_target: target, ..
        } => *target = map[*target],
        CanonicalOpcode::MatchDispatch { arms, .. } => {
            for arm in arms {
                arm.target = map[arm.target];
            }
        }
        _ => {}
    }
}

fn fold_integer(
    left: i64,
    right: i64,
    kind: dbyte_hir::IntegerType,
    operation: &BinOp,
    layout: dbyte_hir::TargetDataLayout,
) -> Option<CanonicalOpcode> {
    if kind == dbyte_hir::IntegerType::LegacyInt {
        return fold_legacy_integer(left, right, operation);
    }
    let left = IntegerValue::from_bits(left as u64, kind, layout);
    let right = IntegerValue::from_bits(right as u64, kind, layout);
    let comparison = match operation {
        BinOp::EqEq => Some(left.bits() == right.bits()),
        BinOp::NotEq => Some(left.bits() != right.bits()),
        BinOp::Lt if is_signed(kind) => Some(left.signed(layout) < right.signed(layout)),
        BinOp::LtEq if is_signed(kind) => Some(left.signed(layout) <= right.signed(layout)),
        BinOp::Gt if is_signed(kind) => Some(left.signed(layout) > right.signed(layout)),
        BinOp::GtEq if is_signed(kind) => Some(left.signed(layout) >= right.signed(layout)),
        BinOp::Lt => Some(left.bits() < right.bits()),
        BinOp::LtEq => Some(left.bits() <= right.bits()),
        BinOp::Gt => Some(left.bits() > right.bits()),
        BinOp::GtEq => Some(left.bits() >= right.bits()),
        _ => None,
    };
    if let Some(value) = comparison {
        return Some(CanonicalOpcode::PushBool(value));
    }
    let operation = match operation {
        BinOp::Add => IntegerBinaryOp::Add,
        BinOp::Sub => IntegerBinaryOp::Sub,
        BinOp::Mul => IntegerBinaryOp::Mul,
        BinOp::Div => IntegerBinaryOp::Div,
        BinOp::Rem => IntegerBinaryOp::Rem,
        BinOp::BitAnd => IntegerBinaryOp::BitAnd,
        BinOp::BitOr => IntegerBinaryOp::BitOr,
        BinOp::BitXor => IntegerBinaryOp::BitXor,
        BinOp::Shl => IntegerBinaryOp::ShiftLeft,
        BinOp::Shr => IntegerBinaryOp::ShiftRight,
        BinOp::EqEq | BinOp::NotEq | BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
            return None
        }
    };
    left.binary(operation, right, layout)
        .ok()
        .map(|value| CanonicalOpcode::PushInteger(value.bits() as i64))
}

fn fold_legacy_integer(left: i64, right: i64, operation: &BinOp) -> Option<CanonicalOpcode> {
    let value = match operation {
        BinOp::Add => left.checked_add(right)?,
        BinOp::Sub => left.checked_sub(right)?,
        BinOp::Mul => left.checked_mul(right)?,
        BinOp::Div => left.checked_div(right)?,
        BinOp::Rem => left.checked_rem(right)?,
        BinOp::BitAnd => left & right,
        BinOp::BitOr => left | right,
        BinOp::BitXor => left ^ right,
        BinOp::Shl => left.checked_shl(right as u32)?,
        BinOp::Shr => left.checked_shr(right as u32)?,
        BinOp::EqEq => return Some(CanonicalOpcode::PushBool(left == right)),
        BinOp::NotEq => return Some(CanonicalOpcode::PushBool(left != right)),
        BinOp::Lt => return Some(CanonicalOpcode::PushBool(left < right)),
        BinOp::LtEq => return Some(CanonicalOpcode::PushBool(left <= right)),
        BinOp::Gt => return Some(CanonicalOpcode::PushBool(left > right)),
        BinOp::GtEq => return Some(CanonicalOpcode::PushBool(left >= right)),
    };
    Some(CanonicalOpcode::PushInteger(value))
}

fn internal(message: &str, span: Span) -> Diagnostic {
    Diagnostic {
        code: "E_VM_HIR_INTERNAL",
        message: message.into(),
        span,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_hir::{
        analyze_project, monomorphize_project, ModuleId, Profile, ProjectInput, ProjectModuleInput,
        TargetDataLayout,
    };
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn lower(source: &str, optimization: VmOptimizationLevel) -> CanonicalBytecodeProject {
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let analyzed = analyze_project(
            &ProjectInput {
                root: ModuleId(0),
                modules: vec![ProjectModuleInput {
                    id: ModuleId(0),
                    name: "root".into(),
                    program,
                    imports: Vec::new(),
                }],
            },
            Profile::Vm,
            TargetDataLayout::I686,
        )
        .unwrap();
        lower_concrete_project_to_bytecode_at_level(
            &monomorphize_project(&analyzed).unwrap(),
            optimization,
        )
        .unwrap()
    }

    #[test]
    fn bytecode_is_flat_and_targets_are_finalized() {
        let bytecode = lower(
            "let total = 0\nwhile total < 2:\n    total = total + 1\nprint(total)\n",
            VmOptimizationLevel::O0,
        );
        assert!(!bytecode.entry.instructions.is_empty());
        for instruction in &bytecode.entry.instructions {
            match &instruction.opcode {
                CanonicalOpcode::Jump(target) | CanonicalOpcode::JumpIfFalse(target) => {
                    assert!(*target < bytecode.entry.instructions.len())
                }
                _ => {}
            }
        }
    }

    #[test]
    fn optimizer_keeps_o0_and_folds_only_successful_integer_constants() {
        let o0 = lower("print(40 + 2)\n", VmOptimizationLevel::O0);
        let o1 = lower("print(40 + 2)\n", VmOptimizationLevel::O1);
        assert!(o0.entry.instructions.len() > o1.entry.instructions.len());
        assert!(o1
            .entry
            .instructions
            .iter()
            .any(|instruction| matches!(instruction.opcode, CanonicalOpcode::PushInteger(42))));
        let zero = lower("print(1 / 0)\n", VmOptimizationLevel::O1);
        assert!(zero.entry.instructions.iter().any(|instruction| matches!(
            instruction.opcode,
            CanonicalOpcode::Binary { op: BinOp::Div, .. }
        )));
        let overflow = lower(
            "let maximum: int = 9223372036854775807\nprint(maximum + 1)\n",
            VmOptimizationLevel::O1,
        );
        assert!(overflow
            .entry
            .instructions
            .iter()
            .any(|instruction| matches!(
                instruction.opcode,
                CanonicalOpcode::Binary { op: BinOp::Add, .. }
            )));
        let minimum = lower(
            "let minimum: int = -9223372036854775807 - 1\nprint(minimum)\n",
            VmOptimizationLevel::O1,
        );
        assert!(minimum
            .entry
            .instructions
            .iter()
            .any(|instruction| matches!(
                instruction.opcode,
                CanonicalOpcode::PushInteger(i64::MIN)
            )));
    }

    #[test]
    fn flat_lowering_and_disassembly_are_deterministic() {
        let source = "fn select(value: bool) -> int:\n    if value:\n        return 40 + 2\n    return 0\n\nprint(select(true))\n";
        let first = lower(source, VmOptimizationLevel::O1);
        let second = lower(source, VmOptimizationLevel::O1);
        assert_eq!(
            disassemble_canonical_bytecode(&first, "deterministic"),
            disassemble_canonical_bytecode(&second, "deterministic")
        );
    }
}
