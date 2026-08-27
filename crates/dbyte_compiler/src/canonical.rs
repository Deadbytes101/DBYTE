//! Representation-only lowering from Concrete HIR to canonical bytecode.

use dbyte_bytecode::*;
use dbyte_hir::{
    ConcreteHirProject, Diagnostic, HirBlock, HirExpr, HirExprKind, HirFormatPart, HirPlace,
    HirPlaceBase, HirProjection, HirStatement,
};

pub fn lower_concrete_project_to_bytecode(
    project: &ConcreteHirProject,
) -> Result<CanonicalBytecodeProject, Diagnostic> {
    Ok(CanonicalBytecodeProject {
        root: project.root,
        module_order: project.module_order.clone(),
        symbol_owners: project.symbol_owners.clone(),
        link_names: project.link_names.clone(),
        semantic: project.executable.semantic.clone(),
        functions: project
            .executable
            .functions
            .iter()
            .map(|function| {
                Ok(CanonicalBytecodeFunction {
                    instance: function.instance,
                    key: function.key.clone(),
                    declaration_only: function.declaration_only,
                    parameters: function.parameters.clone(),
                    locals: function.locals.clone(),
                    result: function.result,
                    body: lower_block(&function.body)?,
                    span: function.span,
                })
            })
            .collect::<Result<_, Diagnostic>>()?,
        globals: project
            .executable
            .globals
            .iter()
            .map(|global| {
                Ok(CanonicalBytecodeGlobal {
                    symbol: global.symbol,
                    mutable: global.mutable,
                    ty: global.ty,
                    initializer: global.initializer.as_ref().map(lower_expr).transpose()?,
                    span: global.span,
                })
            })
            .collect::<Result<_, Diagnostic>>()?,
        top_level: lower_block(&project.executable.top_level)?,
    })
}

fn lower_block(block: &HirBlock) -> Result<CanonicalBytecodeBlock, Diagnostic> {
    Ok(CanonicalBytecodeBlock {
        statements: block
            .statements
            .iter()
            .map(lower_statement)
            .collect::<Result<_, _>>()?,
    })
}

fn lower_statement(statement: &HirStatement) -> Result<CanonicalBytecodeStatement, Diagnostic> {
    Ok(match statement {
        HirStatement::Let { local, value, span } => CanonicalBytecodeStatement::Let {
            local: *local,
            value: lower_expr(value)?,
            span: *span,
        },
        HirStatement::Assign {
            place,
            value,
            cleanup_before_commit,
            span,
        } => CanonicalBytecodeStatement::Assign {
            place: lower_place(place)?,
            value: lower_expr(value)?,
            cleanup_before_commit: cleanup_before_commit
                .as_ref()
                .map(lower_block)
                .transpose()?,
            span: *span,
        },
        HirStatement::Expression(value) => {
            CanonicalBytecodeStatement::Expression(lower_expr(value)?)
        }
        HirStatement::Return {
            value,
            cleanup,
            span,
        } => CanonicalBytecodeStatement::Return {
            value: value.as_ref().map(lower_expr).transpose()?,
            cleanup: cleanup.iter().map(lower_block).collect::<Result<_, _>>()?,
            span: *span,
        },
        HirStatement::If {
            condition,
            then_block,
            else_block,
            span,
        } => CanonicalBytecodeStatement::If {
            condition: lower_expr(condition)?,
            then_block: lower_block(then_block)?,
            else_block: else_block.as_ref().map(lower_block).transpose()?,
            span: *span,
        },
        HirStatement::While {
            condition,
            body,
            span,
        } => CanonicalBytecodeStatement::While {
            condition: lower_expr(condition)?,
            body: lower_block(body)?,
            span: *span,
        },
        HirStatement::For {
            local,
            iterable,
            body,
            span,
        } => CanonicalBytecodeStatement::For {
            local: local.clone(),
            iterable: lower_expr(iterable)?,
            body: lower_block(body)?,
            span: *span,
        },
        HirStatement::Match {
            mode,
            value,
            arms,
            span,
            ..
        } => CanonicalBytecodeStatement::Match {
            mode: *mode,
            value: lower_expr(value)?,
            arms: arms
                .iter()
                .map(|arm| {
                    Ok(CanonicalMatchArm {
                        variant: arm.variant.map(|variant| variant.variant),
                        bindings: arm.bindings.clone(),
                        body: lower_block(&arm.body)?,
                        span: arm.span,
                    })
                })
                .collect::<Result<_, Diagnostic>>()?,
            span: *span,
        },
        HirStatement::Defer { .. } => CanonicalBytecodeStatement::RegisterDefer,
        HirStatement::Break { cleanup, span } => CanonicalBytecodeStatement::Break {
            cleanup: cleanup.iter().map(lower_block).collect::<Result<_, _>>()?,
            span: *span,
        },
        HirStatement::Continue { cleanup, span } => CanonicalBytecodeStatement::Continue {
            cleanup: cleanup.iter().map(lower_block).collect::<Result<_, _>>()?,
            span: *span,
        },
    })
}

fn lower_expr(expression: &HirExpr) -> Result<CanonicalExprCode, Diagnostic> {
    let mut ops = Vec::new();
    lower_expr_ops(expression, &mut ops)?;
    Ok(CanonicalExprCode {
        ty: expression.ty,
        span: expression.span,
        ops,
    })
}

fn lower_expr_ops(expression: &HirExpr, ops: &mut Vec<CanonicalOp>) -> Result<(), Diagnostic> {
    match &expression.kind {
        HirExprKind::Unit => ops.push(CanonicalOp::PushUnit),
        HirExprKind::Integer(value) => ops.push(CanonicalOp::PushInteger(*value)),
        HirExprKind::LayoutOf { target, alignment } => ops.push(CanonicalOp::PushLayout {
            target: *target,
            alignment: *alignment,
        }),
        HirExprKind::Length { value } => {
            lower_expr_ops(value, ops)?;
            ops.push(CanonicalOp::Length);
        }
        HirExprKind::Float(value) => ops.push(CanonicalOp::PushFloat(*value)),
        HirExprKind::Bool(value) => ops.push(CanonicalOp::PushBool(*value)),
        HirExprKind::String(value) => ops.push(CanonicalOp::PushString(value.clone())),
        HirExprKind::Bytes(value) => ops.push(CanonicalOp::PushBytes(value.clone())),
        HirExprKind::Null => ops.push(CanonicalOp::PushNull),
        HirExprKind::Local(local) => ops.push(CanonicalOp::LoadLocal {
            local: *local,
            action: expression.ownership,
        }),
        HirExprKind::Global(symbol) => ops.push(CanonicalOp::LoadGlobal {
            symbol: *symbol,
            action: expression.ownership,
        }),
        HirExprKind::Constant { value, .. } => lower_expr_ops(value, ops)?,
        HirExprKind::Function(key) => ops.push(CanonicalOp::PushFunction(key.clone())),
        HirExprKind::List(values) => {
            lower_many(values, ops)?;
            ops.push(CanonicalOp::MakeList(values.len()));
        }
        HirExprKind::Binary { left, op, right } => {
            lower_expr_ops(left, ops)?;
            lower_expr_ops(right, ops)?;
            ops.push(CanonicalOp::Binary(op.clone()));
        }
        HirExprKind::Unary { op, value } => {
            lower_expr_ops(value, ops)?;
            ops.push(CanonicalOp::Unary(op.clone()));
        }
        HirExprKind::Cast { value, target } => {
            lower_expr_ops(value, ops)?;
            ops.push(CanonicalOp::Cast(*target));
        }
        HirExprKind::DirectCall {
            callee,
            convention,
            arguments,
        } => {
            lower_many(arguments, ops)?;
            ops.push(CanonicalOp::CallDirect {
                callee: callee.clone(),
                convention: *convention,
                argument_count: arguments.len(),
            });
        }
        HirExprKind::IndirectCall {
            callee,
            convention,
            arguments,
        } => {
            lower_expr_ops(callee, ops)?;
            lower_many(arguments, ops)?;
            ops.push(CanonicalOp::CallIndirect {
                convention: *convention,
                argument_count: arguments.len(),
            });
        }
        HirExprKind::IntrinsicCall {
            intrinsic,
            arguments,
        } => {
            lower_many(arguments, ops)?;
            ops.push(CanonicalOp::Intrinsic {
                intrinsic: *intrinsic,
                argument_count: arguments.len(),
                result: expression.ty,
            });
        }
        HirExprKind::HostCall {
            capability,
            arguments,
        } => {
            lower_many(arguments, ops)?;
            ops.push(CanonicalOp::HostCall {
                capability: *capability,
                argument_count: arguments.len(),
            });
        }
        HirExprKind::FormatString(parts) => {
            let mut descriptors = Vec::with_capacity(parts.len());
            let mut value_count = 0;
            for part in parts {
                match part {
                    HirFormatPart::Literal(value) => {
                        descriptors.push(CanonicalFormatPart::Literal(value.clone()));
                    }
                    HirFormatPart::Value(value) => {
                        lower_expr_ops(value, ops)?;
                        descriptors.push(CanonicalFormatPart::Value);
                        value_count += 1;
                    }
                }
            }
            ops.push(CanonicalOp::Format {
                parts: descriptors,
                value_count,
            });
        }
        HirExprKind::InterruptAddress { .. } | HirExprKind::TaskAddress { .. } => {
            return Err(Diagnostic {
                code: "E_VM_HIR_UNSUPPORTED",
                message: "target address metadata is unavailable in VM bytecode".into(),
                span: expression.span,
            });
        }
        HirExprKind::Struct { symbol, fields } => {
            for (_, value) in fields {
                lower_expr_ops(value, ops)?;
            }
            ops.push(CanonicalOp::MakeStruct {
                symbol: *symbol,
                fields: fields.iter().map(|(field, _)| *field).collect(),
            });
        }
        HirExprKind::Enum {
            symbol,
            variant,
            discriminant,
            payload,
        } => {
            lower_many(payload, ops)?;
            ops.push(CanonicalOp::MakeEnum {
                symbol: *symbol,
                variant: *variant,
                discriminant: *discriminant,
                payload_count: payload.len(),
            });
        }
        HirExprKind::Place(place) => ops.push(CanonicalOp::LoadPlace {
            place: lower_place(place)?,
            action: expression.ownership,
        }),
        HirExprKind::Borrow { mutable, place } => ops.push(CanonicalOp::BorrowPlace {
            place: lower_place(place)?,
            mutable: *mutable,
        }),
        HirExprKind::Try {
            value,
            ok_variant,
            err_variant,
            cleanup,
        } => {
            lower_expr_ops(value, ops)?;
            ops.push(CanonicalOp::PropagateResult {
                ok_variant: *ok_variant,
                err_variant: *err_variant,
                cleanup: cleanup.iter().map(lower_block).collect::<Result<_, _>>()?,
            });
        }
        HirExprKind::Drop { place, cleanup } => ops.push(CanonicalOp::DropPlace {
            place: lower_place(place)?,
            cleanup: lower_block(cleanup)?,
        }),
        HirExprKind::Closure {
            closure,
            captures,
            parameters,
            result,
            body,
        } => ops.push(CanonicalOp::MakeClosure {
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
            parameters: parameters.clone(),
            result: *result,
            body: lower_block(body)?,
        }),
    }
    Ok(())
}

fn lower_many(expressions: &[HirExpr], ops: &mut Vec<CanonicalOp>) -> Result<(), Diagnostic> {
    for expression in expressions {
        lower_expr_ops(expression, ops)?;
    }
    Ok(())
}

fn lower_place(place: &HirPlace) -> Result<CanonicalPlace, Diagnostic> {
    Ok(CanonicalPlace {
        base: match &place.base {
            HirPlaceBase::Local(local) => CanonicalPlaceBase::Local(*local),
            HirPlaceBase::Global(symbol) => CanonicalPlaceBase::Global(*symbol),
            HirPlaceBase::Pointer(pointer) => {
                CanonicalPlaceBase::Pointer(Box::new(lower_expr(pointer)?))
            }
        },
        projections: place
            .projections
            .iter()
            .map(|projection| {
                Ok(match projection {
                    HirProjection::Field { field, ty, offset } => CanonicalProjection::Field {
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
                    } => CanonicalProjection::Index {
                        index: lower_expr(index)?,
                        element: *element,
                        element_size: *element_size,
                    },
                })
            })
            .collect::<Result<_, Diagnostic>>()?,
        ty: place.ty,
        mutable: place.mutable,
        span: place.span,
    })
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

    fn lower(source: &str) -> CanonicalBytecodeProject {
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
        let concrete = monomorphize_project(&analyzed).unwrap();
        lower_concrete_project_to_bytecode(&concrete).unwrap()
    }

    #[test]
    fn canonical_bytecode_uses_ids_and_explicit_stack_operations() {
        let bytecode = lower(
            "fn id[T](value: T) -> T:\n    return value\n\nlet answer = id[int](40) + 2\nprint(answer)\n",
        );
        let CanonicalBytecodeStatement::Let { local, value, .. } =
            &bytecode.top_level.statements[0]
        else {
            panic!("expected canonical let");
        };
        assert_eq!(local.0, 0);
        assert!(value
            .ops
            .iter()
            .any(|op| matches!(op, CanonicalOp::CallDirect { .. })));
        assert!(value
            .ops
            .iter()
            .any(|op| matches!(op, CanonicalOp::Binary(dbyte_ast::BinOp::Add))));
        assert!(matches!(
            bytecode.top_level.statements[1],
            CanonicalBytecodeStatement::Expression(CanonicalExprCode { ref ops, .. })
                if matches!(ops.last(), Some(CanonicalOp::HostCall { capability: dbyte_hir::HostCapabilityId::Print, .. }))
        ));
    }

    #[test]
    fn canonical_bytecode_carries_cleanup_order_from_hir() {
        let bytecode = lower(
            "struct Resource:\n    value: int\n\ndrop fn destroy(value: borrow mut Resource):\n    print((*value).value)\n\nlet resource = Resource(value = 1)\nresource = Resource(value = 2)\ndrop(resource)\n",
        );
        let CanonicalBytecodeStatement::Assign {
            cleanup_before_commit: Some(cleanup),
            ..
        } = &bytecode.top_level.statements[1]
        else {
            panic!("reassignment cleanup must be explicit bytecode");
        };
        assert!(!cleanup.statements.is_empty());
        let CanonicalBytecodeStatement::Expression(CanonicalExprCode { ops, .. }) =
            &bytecode.top_level.statements[2]
        else {
            panic!("expected explicit drop expression");
        };
        assert!(matches!(ops.last(), Some(CanonicalOp::DropPlace { .. })));
    }
}
