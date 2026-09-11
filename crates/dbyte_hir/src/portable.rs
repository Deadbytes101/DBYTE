use crate::{
    analyze_project, CanonicalType, Diagnostic, IntegerType, Profile, ProjectInput,
    TargetDataLayout,
};
use dbyte_ast::{CallConvention, Expr, Repr, Span, Stmt, TypeAnnotation, UnaryOp};
use std::collections::BTreeSet;

pub const PORTABLE_PROFILE_COUNT: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableValidation {
    pub profile_count: usize,
    pub interface_fingerprint: u64,
}

pub fn validate_portable_project(
    project: &ProjectInput,
) -> Result<PortableValidation, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for module in &project.modules {
        validate_statements(&module.program.stmts, &mut diagnostics);
    }
    if !diagnostics.is_empty() {
        return Err(finish_diagnostics(diagnostics));
    }

    let matrix = portable_profile_matrix();
    let mut fingerprints = BTreeSet::new();
    for (profile, layout) in matrix {
        match analyze_project(project, profile, layout) {
            Ok(analyzed) => {
                fingerprints.insert(analyzed.executable.semantic.interface_fingerprint);
                for expression in &analyzed.executable.semantic.expressions {
                    if let Some((code, message)) = forbidden_canonical_type(
                        analyzed.executable.semantic.type_of(expression.ty),
                    ) {
                        diagnostics.push(Diagnostic {
                            code,
                            message: message.into(),
                            span: expression.span,
                        });
                    }
                }
            }
            Err(failures) => {
                for failure in failures {
                    diagnostics.push(Diagnostic {
                        code: "E_PORTABLE_PROFILE",
                        message: format!(
                            "strict portable source is rejected by {profile:?}/{layout:?}: {}: {}",
                            failure.code, failure.message
                        ),
                        span: failure.span,
                    });
                }
            }
        }
    }

    if fingerprints.len() > 1 {
        diagnostics.push(Diagnostic {
            code: "E_PORTABLE_INTERFACE_DRIFT",
            message: "public interface fingerprint changes across strict portable targets".into(),
            span: Span::zero(),
        });
    }
    if !diagnostics.is_empty() {
        return Err(finish_diagnostics(diagnostics));
    }
    Ok(PortableValidation {
        profile_count: PORTABLE_PROFILE_COUNT,
        interface_fingerprint: fingerprints.into_iter().next().unwrap_or(0),
    })
}

fn portable_profile_matrix() -> [(Profile, TargetDataLayout); PORTABLE_PROFILE_COUNT] {
    [
        (Profile::Hosted, TargetDataLayout::I686),
        (Profile::Hosted, TargetDataLayout::X86_64),
        (Profile::Vm, TargetDataLayout::I686),
        (Profile::Vm, TargetDataLayout::X86_64),
        (Profile::I686None, TargetDataLayout::I686),
        (Profile::I686User, TargetDataLayout::I686),
        (Profile::X86_64None, TargetDataLayout::X86_64),
        (Profile::X86_64User, TargetDataLayout::X86_64),
        (Profile::X86_64Linux, TargetDataLayout::X86_64),
        (Profile::X86_64Windows, TargetDataLayout::X86_64),
    ]
}
fn forbidden_canonical_type(ty: &CanonicalType) -> Option<(&'static str, &'static str)> {
    match ty {
        CanonicalType::Integer(IntegerType::Usize) => Some((
            "E_PORTABLE_TARGET_WIDTH",
            "usize is target-width-dependent in strict portable code",
        )),
        CanonicalType::Integer(IntegerType::LegacyInt) => Some((
            "E_PORTABLE_LEGACY_INT",
            "legacy int is not admitted by the strict portable contract",
        )),
        CanonicalType::Float => Some((
            "E_PORTABLE_LEGACY_FLOAT",
            "legacy float is not admitted by the strict portable contract",
        )),
        CanonicalType::LegacyStr
        | CanonicalType::LegacyBytes
        | CanonicalType::LegacyBuffer
        | CanonicalType::LegacyList(_) => Some((
            "E_PORTABLE_LEGACY_REPRESENTATION",
            "legacy container/string representation is not portable across DByte backends",
        )),
        CanonicalType::Pointer { .. }
        | CanonicalType::Borrow { .. }
        | CanonicalType::Slice { .. }
        | CanonicalType::StringView => Some((
            "E_PORTABLE_ADDRESS_REPRESENTATION",
            "address-bearing representation is outside strict portable code",
        )),
        _ => None,
    }
}
fn validate_type(ty: &TypeAnnotation, span: Span, diagnostics: &mut Vec<Diagnostic>) {
    match ty {
        TypeAnnotation::Int => push(
            diagnostics,
            "E_PORTABLE_LEGACY_INT",
            "legacy int is outside strict portable code",
            span,
        ),
        TypeAnnotation::Usize => push(
            diagnostics,
            "E_PORTABLE_TARGET_WIDTH",
            "usize is target-width-dependent",
            span,
        ),
        TypeAnnotation::Float => push(
            diagnostics,
            "E_PORTABLE_LEGACY_FLOAT",
            "legacy float is outside strict portable code",
            span,
        ),
        TypeAnnotation::Str
        | TypeAnnotation::Bytes
        | TypeAnnotation::Buffer
        | TypeAnnotation::List(_) => push(
            diagnostics,
            "E_PORTABLE_LEGACY_REPRESENTATION",
            "legacy container/string representation is outside strict portable code",
            span,
        ),
        TypeAnnotation::Pointer(_)
        | TypeAnnotation::ConstPointer(_)
        | TypeAnnotation::Borrow { .. }
        | TypeAnnotation::Slice(_)
        | TypeAnnotation::MutSlice(_)
        | TypeAnnotation::StringView => push(
            diagnostics,
            "E_PORTABLE_ADDRESS_REPRESENTATION",
            "address-bearing representation is outside strict portable code",
            span,
        ),
        TypeAnnotation::Array { element, .. } => validate_type(element, span, diagnostics),
        TypeAnnotation::Generic { args, .. } => {
            for argument in args {
                validate_type(argument, span, diagnostics);
            }
        }
        TypeAnnotation::Function {
            convention,
            params,
            ret,
        } => {
            if *convention != CallConvention::DByte {
                push(
                    diagnostics,
                    "E_PORTABLE_C_ABI",
                    "extern C function types are target ABI contracts",
                    span,
                );
            }
            for parameter in params {
                validate_type(parameter, span, diagnostics);
            }
            validate_type(ret, span, diagnostics);
        }
        TypeAnnotation::Closure { params, ret } => {
            for parameter in params {
                validate_type(parameter, span, diagnostics);
            }
            validate_type(ret, span, diagnostics);
        }
        TypeAnnotation::Unit
        | TypeAnnotation::I8
        | TypeAnnotation::U8
        | TypeAnnotation::I16
        | TypeAnnotation::U16
        | TypeAnnotation::I32
        | TypeAnnotation::U32
        | TypeAnnotation::I64
        | TypeAnnotation::U64
        | TypeAnnotation::F32
        | TypeAnnotation::F64
        | TypeAnnotation::Bool
        | TypeAnnotation::Named(_)
        | TypeAnnotation::Inferred => {}
    }
}

fn validate_statements(statements: &[Stmt], diagnostics: &mut Vec<Diagnostic>) {
    for statement in statements {
        match statement {
            Stmt::Let {
                ty, value, span, ..
            } => {
                validate_type(ty, *span, diagnostics);
                validate_expr(value, diagnostics);
            }
            Stmt::Assign { value, .. } => validate_expr(value, diagnostics),
            Stmt::Store { target, value, .. } => {
                validate_expr(target, diagnostics);
                validate_expr(value, diagnostics);
            }
            Stmt::FnDef {
                interrupt,
                call_convention,
                params,
                ret_ty,
                body,
                span,
                ..
            } => {
                if interrupt.is_some() {
                    push(
                        diagnostics,
                        "E_PORTABLE_INTERRUPT",
                        "interrupt functions are target execution contracts",
                        *span,
                    );
                }
                if *call_convention != CallConvention::DByte {
                    push(
                        diagnostics,
                        "E_PORTABLE_C_ABI",
                        "extern C functions are target ABI contracts",
                        *span,
                    );
                }
                for parameter in params {
                    validate_type(&parameter.ty, parameter.span, diagnostics);
                }
                validate_type(ret_ty, *span, diagnostics);
                validate_statements(body, diagnostics);
            }
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    validate_expr(value, diagnostics);
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                validate_expr(cond, diagnostics);
                validate_statements(then_body, diagnostics);
                if let Some(other) = else_body {
                    validate_statements(other, diagnostics);
                }
            }
            Stmt::While { cond, body, .. } => {
                validate_expr(cond, diagnostics);
                validate_statements(body, diagnostics);
            }
            Stmt::For { iterable, body, .. } => {
                validate_expr(iterable, diagnostics);
                validate_statements(body, diagnostics);
            }
            Stmt::Import { .. } | Stmt::Break { .. } | Stmt::Continue { .. } => {}
            Stmt::StructDef {
                repr, fields, span, ..
            } => {
                if *repr != Repr::DByte {
                    push(
                        diagnostics,
                        "E_PORTABLE_C_ABI",
                        "repr(C) is a target ABI layout contract",
                        *span,
                    );
                }
                for field in fields {
                    validate_type(&field.ty, field.span, diagnostics);
                }
            }
            Stmt::EnumDef { variants, span, .. } => {
                for variant in variants {
                    for payload in &variant.payload {
                        validate_type(payload, *span, diagnostics);
                    }
                }
            }
            Stmt::Match { value, arms, .. } => {
                validate_expr(value, diagnostics);
                for arm in arms {
                    validate_statements(&arm.body, diagnostics);
                }
            }
            Stmt::Defer { body, .. } => validate_statements(body, diagnostics),
            Stmt::Const {
                ty, value, span, ..
            } => {
                validate_type(ty, *span, diagnostics);
                validate_expr(value, diagnostics);
            }
            Stmt::Static {
                ty, value, span, ..
            } => {
                validate_type(ty, *span, diagnostics);
                if let Some(value) = value {
                    validate_expr(value, diagnostics);
                }
            }
            Stmt::Expr(expression) => validate_expr(expression, diagnostics),
        }
    }
}
fn validate_expr(expression: &Expr, diagnostics: &mut Vec<Diagnostic>) {
    match expression {
        Expr::FloatLit(_, span) => push(
            diagnostics,
            "E_PORTABLE_LEGACY_FLOAT",
            "float literals currently use the legacy float representation",
            *span,
        ),
        Expr::StrLit(_, span) | Expr::BytesLit(_, span) | Expr::FStr(_, span) => push(
            diagnostics,
            "E_PORTABLE_LEGACY_REPRESENTATION",
            "string/bytes literals are outside the strict portable representation set",
            *span,
        ),
        Expr::NullLit(span) => push(
            diagnostics,
            "E_PORTABLE_ADDRESS_REPRESENTATION",
            "null is meaningful only for address-bearing representations",
            *span,
        ),
        Expr::List(values, _) => {
            for value in values {
                validate_expr(value, diagnostics);
            }
        }
        Expr::Binary { left, right, .. } => {
            validate_expr(left, diagnostics);
            validate_expr(right, diagnostics);
        }
        Expr::Unary { op, expr, span } => {
            if matches!(op, UnaryOp::AddressOf | UnaryOp::Deref) {
                push(
                    diagnostics,
                    "E_PORTABLE_ADDRESS_OPERATION",
                    "address-of and dereference are outside strict portable code",
                    *span,
                );
            }
            validate_expr(expr, diagnostics);
        }
        Expr::Call { name, args, span } => {
            if matches!(
                name.as_str(),
                "size_of" | "align_of" | "interrupt_address" | "task_address"
            ) {
                push(
                    diagnostics,
                    "E_PORTABLE_TARGET_OBSERVATION",
                    "target layout/address observation is outside strict portable code",
                    *span,
                );
            }
            for argument in args {
                validate_expr(argument, diagnostics);
            }
        }
        Expr::Index {
            target,
            index,
            span,
        } => {
            push(
                diagnostics,
                "E_PORTABLE_INDEX_SEMANTICS",
                "indexing is excluded until every backend enforces one bounds contract",
                *span,
            );
            validate_expr(target, diagnostics);
            validate_expr(index, diagnostics);
        }
        Expr::Member { object, span, .. } => {
            if target_namespace(object).is_some() {
                push(
                    diagnostics,
                    "E_PORTABLE_TARGET_CAPABILITY",
                    "target/host namespaces are outside strict portable code",
                    *span,
                );
            }
            validate_expr(object, diagnostics);
        }
        Expr::MemberCall {
            object, args, span, ..
        } => {
            if target_namespace(object).is_some() {
                push(
                    diagnostics,
                    "E_PORTABLE_TARGET_CAPABILITY",
                    "target/host namespaces are outside strict portable code",
                    *span,
                );
            }
            validate_expr(object, diagnostics);
            for argument in args {
                validate_expr(argument, diagnostics);
            }
        }
        Expr::GenericApply { target, args, span } => {
            validate_expr(target, diagnostics);
            for argument in args {
                validate_type(argument, *span, diagnostics);
            }
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
            ..
        } => {
            validate_expr(callee, diagnostics);
            for argument in args {
                validate_expr(argument, diagnostics);
            }
            for argument in named_args {
                validate_expr(&argument.value, diagnostics);
            }
        }
        Expr::Borrow { expr, span, .. } => {
            push(
                diagnostics,
                "E_PORTABLE_ADDRESS_OPERATION",
                "borrowing is outside the first strict portable contract",
                *span,
            );
            validate_expr(expr, diagnostics);
        }
        Expr::Try { expr, .. } | Expr::Drop { expr, .. } => validate_expr(expr, diagnostics),
        Expr::Closure {
            params,
            ret_ty,
            body,
            span,
            ..
        } => {
            for parameter in params {
                validate_type(&parameter.ty, parameter.span, diagnostics);
            }
            validate_type(ret_ty, *span, diagnostics);
            validate_statements(body, diagnostics);
        }
        Expr::Cast { expr, ty, span } | Expr::Bitcast { expr, ty, span } => {
            validate_expr(expr, diagnostics);
            validate_type(ty, *span, diagnostics);
        }
        Expr::UnitLit(_) | Expr::IntLit(_, _) | Expr::BoolLit(_, _) | Expr::Ident(_, _) => {}
    }
}

fn target_namespace(expression: &Expr) -> Option<&str> {
    match expression {
        Expr::Ident(name, _)
            if matches!(
                name.as_str(),
                "native" | "linux" | "windows" | "user" | "__dbyte_runtime"
            ) =>
        {
            Some(name.as_str())
        }
        Expr::Member { object, .. } => target_namespace(object),
        _ => None,
    }
}
fn push(diagnostics: &mut Vec<Diagnostic>, code: &'static str, message: &'static str, span: Span) {
    diagnostics.push(Diagnostic {
        code,
        message: message.into(),
        span,
    });
}

fn finish_diagnostics(mut diagnostics: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diagnostics.sort_by(|left, right| {
        (
            left.span.line,
            left.span.col,
            left.code,
            left.message.as_str(),
        )
            .cmp(&(
                right.span.line,
                right.span.col,
                right.code,
                right.message.as_str(),
            ))
    });
    diagnostics.dedup_by(|left, right| {
        left.span == right.span && left.code == right.code && left.message == right.message
    });
    diagnostics
}
