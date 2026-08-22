use crate::diagnostics::{Diagnostic, DiagnosticCategory, NativeResult};
use crate::ir::{
    CompareOp, IrFunction, IrInst, IrParam, IrProgram, IrSlot, LabelId, LocalId, StringId,
};
use crate::types::{Intrinsic, NativeType};
use dbyte_ast::{BinOp, Expr, Param, Program, Span, Stmt, TypeAnnotation, UnaryOp};
use std::collections::{BTreeMap, BTreeSet};

const ENTRY_FUNCTION: &str = "kernel_main";

#[derive(Debug, Clone)]
struct FunctionSignature {
    params: Vec<NativeType>,
    return_type: NativeType,
    span: Span,
}

#[derive(Debug, Clone, Copy)]
struct Binding {
    slot: LocalId,
    ty: NativeType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExprResult {
    Value(NativeType),
    Void,
}

/// Validate and lower an existing DByte AST into typed freestanding native IR.
///
/// Analysis is deliberately separate from hosted `TypeChecker`: hosted `int`
/// remains i64 and hosted modules/process callbacks remain available there,
/// while this profile admits only deterministic i32/bool/static-string code.
pub fn analyze(program: &Program) -> Result<IrProgram, Diagnostic> {
    Analyzer::new().analyze(program)
}

pub struct Analyzer {
    signatures: BTreeMap<String, FunctionSignature>,
    strings: Vec<Vec<u8>>,
    string_ids: BTreeMap<Vec<u8>, StringId>,
}

impl Analyzer {
    pub fn new() -> Self {
        Self {
            signatures: BTreeMap::new(),
            strings: Vec::new(),
            string_ids: BTreeMap::new(),
        }
    }

    pub fn analyze(mut self, program: &Program) -> Result<IrProgram, Diagnostic> {
        self.collect_signatures(program)?;
        self.validate_entry()?;

        let mut functions = Vec::with_capacity(self.signatures.len());
        for stmt in &program.stmts {
            let Stmt::FnDef {
                name,
                params,
                body,
                span,
                ..
            } = stmt
            else {
                // The signature pass has already produced the source-facing
                // diagnostic for every non-function top-level statement.
                continue;
            };

            let signature = self
                .signatures
                .get(name)
                .expect("signature collected for every function")
                .clone();
            let builder = FunctionAnalyzer::new(
                &self.signatures,
                &mut self.strings,
                &mut self.string_ids,
                name,
                signature.return_type,
                *span,
            );
            functions.push(builder.compile(params, body)?);
        }

        Ok(IrProgram {
            functions,
            strings: self.strings,
        })
    }

    fn collect_signatures(&mut self, program: &Program) -> NativeResult<()> {
        for stmt in &program.stmts {
            let Stmt::FnDef {
                name,
                params,
                ret_ty,
                span,
                ..
            } = stmt
            else {
                return Err(Diagnostic::new(
                    DiagnosticCategory::UnsupportedFeature,
                    "top-level executable statements and globals are unsupported in the freestanding native profile",
                    stmt.span(),
                ));
            };

            if self.signatures.contains_key(name) {
                return Err(Diagnostic::new(
                    DiagnosticCategory::DuplicateFunction,
                    format!("duplicate function `{name}`"),
                    *span,
                ));
            }

            let mut parameter_names = BTreeSet::new();
            let mut parameter_types = Vec::with_capacity(params.len());
            for param in params {
                if !parameter_names.insert(param.name.clone()) {
                    return Err(Diagnostic::new(
                        DiagnosticCategory::DuplicateLocal,
                        format!("duplicate parameter `{}` in function `{name}`", param.name),
                        param.span,
                    ));
                }
                parameter_types.push(explicit_native_type(
                    &param.ty,
                    param.span,
                    &format!("parameter `{}`", param.name),
                )?);
            }

            let return_type =
                explicit_native_type(ret_ty, *span, &format!("return type of function `{name}`"))?;
            self.signatures.insert(
                name.clone(),
                FunctionSignature {
                    params: parameter_types,
                    return_type,
                    span: *span,
                },
            );
        }
        Ok(())
    }

    fn validate_entry(&self) -> NativeResult<()> {
        let Some(entry) = self.signatures.get(ENTRY_FUNCTION) else {
            return Err(Diagnostic::without_span(
                DiagnosticCategory::InvalidEntryPoint,
                "missing native entry function `kernel_main`",
            ));
        };
        if !entry.params.is_empty() || entry.return_type != NativeType::I32 {
            return Err(Diagnostic::new(
                DiagnosticCategory::InvalidEntryPoint,
                format!(
                    "native entry `kernel_main` must have signature `fn kernel_main() -> int`, found {} parameter(s) and return type {}",
                    entry.params.len(),
                    entry.return_type
                ),
                entry.span,
            ));
        }
        Ok(())
    }
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

struct FunctionAnalyzer<'a> {
    signatures: &'a BTreeMap<String, FunctionSignature>,
    strings: &'a mut Vec<Vec<u8>>,
    string_ids: &'a mut BTreeMap<Vec<u8>, StringId>,
    function_name: &'a str,
    return_type: NativeType,
    span: Span,
    scopes: Vec<BTreeMap<String, Binding>>,
    params: Vec<IrParam>,
    slots: Vec<IrSlot>,
    instructions: Vec<IrInst>,
    next_label: u32,
}

impl<'a> FunctionAnalyzer<'a> {
    fn new(
        signatures: &'a BTreeMap<String, FunctionSignature>,
        strings: &'a mut Vec<Vec<u8>>,
        string_ids: &'a mut BTreeMap<Vec<u8>, StringId>,
        function_name: &'a str,
        return_type: NativeType,
        span: Span,
    ) -> Self {
        Self {
            signatures,
            strings,
            string_ids,
            function_name,
            return_type,
            span,
            scopes: vec![BTreeMap::new()],
            params: Vec::new(),
            slots: Vec::new(),
            instructions: Vec::new(),
            next_label: 0,
        }
    }

    fn compile(mut self, params: &[Param], body: &[Stmt]) -> NativeResult<IrFunction> {
        for param in params {
            let ty = explicit_native_type(
                &param.ty,
                param.span,
                &format!("parameter `{}`", param.name),
            )?;
            let slot = self.allocate_slot(&param.name, ty, true, param.span)?;
            self.params.push(IrParam {
                name: param.name.clone(),
                ty,
                slot,
            });
        }

        self.compile_block(body)?;
        if !block_always_returns(body) {
            return Err(Diagnostic::new(
                DiagnosticCategory::InvalidReturn,
                format!(
                    "function `{}` does not return on every control-flow path",
                    self.function_name
                ),
                self.span,
            ));
        }

        Ok(IrFunction {
            name: self.function_name.to_string(),
            params: self.params,
            return_type: self.return_type,
            slots: self.slots,
            instructions: self.instructions,
            span: self.span,
        })
    }

    fn compile_block(&mut self, statements: &[Stmt]) -> NativeResult<()> {
        for statement in statements {
            self.compile_stmt(statement)?;
        }
        Ok(())
    }

    fn compile_scoped_block(&mut self, statements: &[Stmt]) -> NativeResult<()> {
        self.scopes.push(BTreeMap::new());
        let result = self.compile_block(statements);
        self.scopes.pop();
        result
    }

    fn compile_stmt(&mut self, statement: &Stmt) -> NativeResult<()> {
        match statement {
            Stmt::Let {
                is_pub,
                name,
                ty,
                value,
                span,
            } => {
                if *is_pub {
                    return Err(Diagnostic::new(
                        DiagnosticCategory::UnsupportedFeature,
                        "public local variables are unsupported in the freestanding native profile",
                        *span,
                    ));
                }
                self.ensure_local_available(name, *span)?;
                let inferred =
                    require_value(self.compile_expr(value)?, value.span(), "local initializer")?;
                let local_type = match ty {
                    TypeAnnotation::Inferred => inferred,
                    _ => {
                        let declared = explicit_native_type(ty, *span, &format!("local `{name}`"))?;
                        if declared != inferred {
                            return Err(type_mismatch(
                                format!("local `{name}` expects {declared}, found {inferred}"),
                                value.span(),
                            ));
                        }
                        declared
                    }
                };
                let slot = self.allocate_slot(name, local_type, false, *span)?;
                self.instructions.push(IrInst::LocalStore(slot));
            }
            Stmt::Assign { name, value, span } => {
                let binding = self.lookup(name).ok_or_else(|| {
                    Diagnostic::new(
                        DiagnosticCategory::UnknownSymbol,
                        format!("unknown symbol `{name}`"),
                        *span,
                    )
                })?;
                let value_type =
                    require_value(self.compile_expr(value)?, value.span(), "assignment value")?;
                if value_type != binding.ty {
                    return Err(type_mismatch(
                        format!(
                            "cannot assign {value_type} to local `{name}` of type {}",
                            binding.ty
                        ),
                        *span,
                    ));
                }
                self.instructions.push(IrInst::LocalStore(binding.slot));
            }
            Stmt::Store { span, .. }
            | Stmt::StructDef { span, .. }
            | Stmt::Const { span, .. }
            | Stmt::Static { span, .. } => {
                return Err(unsupported(
                    "systems declarations and lvalue stores in the v11.0 single-image profile",
                    *span,
                ));
            }
            Stmt::FnDef { span, .. } => {
                return Err(Diagnostic::new(
                    DiagnosticCategory::UnsupportedFeature,
                    "nested functions are unsupported in the freestanding native profile",
                    *span,
                ));
            }
            Stmt::Return { value, span } => {
                let Some(value) = value else {
                    return Err(Diagnostic::new(
                        DiagnosticCategory::InvalidReturn,
                        format!(
                            "function `{}` must return a {} value",
                            self.function_name, self.return_type
                        ),
                        *span,
                    ));
                };
                let value_type =
                    require_value(self.compile_expr(value)?, value.span(), "return value")?;
                if value_type != self.return_type {
                    return Err(Diagnostic::new(
                        DiagnosticCategory::InvalidReturn,
                        format!(
                            "function `{}` must return {}, found {}",
                            self.function_name, self.return_type, value_type
                        ),
                        *span,
                    ));
                }
                self.instructions.push(IrInst::Return);
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                span,
            } => {
                let condition_type =
                    require_value(self.compile_expr(cond)?, cond.span(), "if condition")?;
                if condition_type != NativeType::Bool {
                    return Err(type_mismatch(
                        format!("if condition must be bool, found {condition_type}"),
                        *span,
                    ));
                }

                if let Some(else_body) = else_body {
                    let else_label = self.new_label(*span)?;
                    let end_label = self.new_label(*span)?;
                    self.instructions.push(IrInst::JumpIfFalse(else_label));
                    self.compile_scoped_block(then_body)?;
                    self.instructions.push(IrInst::Jump(end_label));
                    self.instructions.push(IrInst::Label(else_label));
                    self.compile_scoped_block(else_body)?;
                    self.instructions.push(IrInst::Label(end_label));
                } else {
                    let end_label = self.new_label(*span)?;
                    self.instructions.push(IrInst::JumpIfFalse(end_label));
                    self.compile_scoped_block(then_body)?;
                    self.instructions.push(IrInst::Label(end_label));
                }
            }
            Stmt::While { cond, body, span } => {
                let loop_label = self.new_label(*span)?;
                let end_label = self.new_label(*span)?;
                self.instructions.push(IrInst::Label(loop_label));
                let condition_type =
                    require_value(self.compile_expr(cond)?, cond.span(), "while condition")?;
                if condition_type != NativeType::Bool {
                    return Err(type_mismatch(
                        format!("while condition must be bool, found {condition_type}"),
                        *span,
                    ));
                }
                self.instructions.push(IrInst::JumpIfFalse(end_label));
                self.compile_scoped_block(body)?;
                self.instructions.push(IrInst::Jump(loop_label));
                self.instructions.push(IrInst::Label(end_label));
            }
            Stmt::For { span, .. } => {
                return Err(Diagnostic::new(
                    DiagnosticCategory::UnsupportedFeature,
                    "for loops are unsupported in the freestanding native profile",
                    *span,
                ));
            }
            Stmt::Import { span, .. } => {
                return Err(Diagnostic::new(
                    DiagnosticCategory::UnsupportedFeature,
                    "imports are unsupported in the freestanding native profile; use the built-in `native.*` namespace directly",
                    *span,
                ));
            }
            Stmt::Expr(expr) => match self.compile_expr(expr)? {
                ExprResult::Value(_) => self.instructions.push(IrInst::Pop),
                ExprResult::Void => {}
            },
        }
        Ok(())
    }

    fn compile_expr(&mut self, expression: &Expr) -> NativeResult<ExprResult> {
        match expression {
            Expr::IntLit(value, span) => {
                let value = i32::try_from(*value).map_err(|_| integer_range(*value, *span))?;
                self.instructions.push(IrInst::ConstI32(value));
                Ok(ExprResult::Value(NativeType::I32))
            }
            Expr::FloatLit(_, span) => Err(unsupported("float literals", *span)),
            Expr::BoolLit(value, _) => {
                self.instructions.push(IrInst::ConstBool(*value));
                Ok(ExprResult::Value(NativeType::Bool))
            }
            Expr::StrLit(value, span) => {
                if !value.is_ascii() {
                    return Err(unsupported(
                        "non-ASCII string literals in the freestanding native profile",
                        *span,
                    ));
                }
                let id = self.intern_string(value.as_bytes(), *span)?;
                self.instructions.push(IrInst::ConstStr(id));
                Ok(ExprResult::Value(NativeType::Str))
            }
            Expr::BytesLit(_, span) => Err(unsupported("byte literals", *span)),
            Expr::FStr(_, span) => Err(unsupported("formatted strings", *span)),
            Expr::Ident(name, span) => {
                let binding = self.lookup(name).ok_or_else(|| {
                    Diagnostic::new(
                        DiagnosticCategory::UnknownSymbol,
                        format!("unknown symbol `{name}`"),
                        *span,
                    )
                })?;
                self.instructions.push(IrInst::LocalLoad(binding.slot));
                Ok(ExprResult::Value(binding.ty))
            }
            Expr::List(_, span) => Err(unsupported("list values", *span)),
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => self.compile_binary(left, op, right, *span),
            Expr::Unary { op, expr, span } => self.compile_unary(op, expr, *span),
            Expr::Call { name, args, span } => self.compile_call(name, args, *span),
            Expr::Index {
                target,
                index,
                span,
            } => {
                let target_type =
                    require_value(self.compile_expr(target)?, target.span(), "index target")?;
                let index_type =
                    require_value(self.compile_expr(index)?, index.span(), "string index")?;
                if target_type != NativeType::Str || index_type != NativeType::I32 {
                    return Err(type_mismatch(
                        format!(
                            "native indexing requires `str[int]`, found {target_type}[{index_type}]"
                        ),
                        *span,
                    ));
                }
                self.instructions.push(IrInst::StrIndex8);
                Ok(ExprResult::Value(NativeType::I32))
            }
            Expr::Member { span, .. } => Err(unsupported("member values", *span)),
            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => self.compile_intrinsic(object, property, args, *span),
            Expr::Cast { span, .. } => Err(unsupported(
                "explicit systems casts in the v11.0 single-image profile",
                *span,
            )),
        }
    }

    fn compile_binary(
        &mut self,
        left: &Expr,
        operator: &BinOp,
        right: &Expr,
        span: Span,
    ) -> NativeResult<ExprResult> {
        if matches!(operator, BinOp::Div) {
            return Err(unsupported("integer division", span));
        }

        let left_type = require_value(self.compile_expr(left)?, left.span(), "left operand")?;
        let right_type = require_value(self.compile_expr(right)?, right.span(), "right operand")?;

        match operator {
            BinOp::Add | BinOp::Sub | BinOp::Mul => {
                if left_type != NativeType::I32 || right_type != NativeType::I32 {
                    return Err(type_mismatch(
                        format!(
                            "operator `{operator}` requires i32 operands, found {left_type} and {right_type}"
                        ),
                        span,
                    ));
                }
                self.instructions.push(match operator {
                    BinOp::Add => IrInst::AddI32,
                    BinOp::Sub => IrInst::SubI32,
                    BinOp::Mul => IrInst::MulI32,
                    _ => unreachable!(),
                });
                Ok(ExprResult::Value(NativeType::I32))
            }
            BinOp::EqEq | BinOp::NotEq => {
                if left_type != right_type
                    || !matches!(left_type, NativeType::I32 | NativeType::Bool)
                {
                    return Err(type_mismatch(
                        format!(
                            "operator `{operator}` requires matching i32 or bool operands, found {left_type} and {right_type}"
                        ),
                        span,
                    ));
                }
                let compare = if matches!(operator, BinOp::EqEq) {
                    CompareOp::Eq
                } else {
                    CompareOp::NotEq
                };
                self.instructions.push(IrInst::Compare(compare));
                Ok(ExprResult::Value(NativeType::Bool))
            }
            BinOp::Lt | BinOp::LtEq | BinOp::Gt | BinOp::GtEq => {
                if left_type != NativeType::I32 || right_type != NativeType::I32 {
                    return Err(type_mismatch(
                        format!(
                            "operator `{operator}` requires i32 operands, found {left_type} and {right_type}"
                        ),
                        span,
                    ));
                }
                let compare = match operator {
                    BinOp::Lt => CompareOp::Lt,
                    BinOp::LtEq => CompareOp::LtEq,
                    BinOp::Gt => CompareOp::Gt,
                    BinOp::GtEq => CompareOp::GtEq,
                    _ => unreachable!(),
                };
                self.instructions.push(IrInst::Compare(compare));
                Ok(ExprResult::Value(NativeType::Bool))
            }
            BinOp::Div => unreachable!(),
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                Err(unsupported(
                    "bitwise and shift operators in the v11.0 single-image profile",
                    span,
                ))
            }
        }
    }

    fn compile_unary(
        &mut self,
        operator: &UnaryOp,
        expression: &Expr,
        span: Span,
    ) -> NativeResult<ExprResult> {
        match operator {
            UnaryOp::Neg => {
                // The parser represents -2147483648 as Neg(IntLit(2147483648)).
                // Accept that one source literal without first trying to place
                // its positive magnitude in an i32 instruction.
                if let Expr::IntLit(value, _) = expression {
                    if *value == i32::MAX as i64 + 1 {
                        self.instructions.push(IrInst::ConstI32(i32::MIN));
                        return Ok(ExprResult::Value(NativeType::I32));
                    }
                }

                let operand_type = require_value(
                    self.compile_expr(expression)?,
                    expression.span(),
                    "unary negation operand",
                )?;
                if operand_type != NativeType::I32 {
                    return Err(type_mismatch(
                        format!("unary `-` requires i32, found {operand_type}"),
                        span,
                    ));
                }
                self.instructions.push(IrInst::NegI32);
                Ok(ExprResult::Value(NativeType::I32))
            }
            UnaryOp::Not => {
                let operand_type = require_value(
                    self.compile_expr(expression)?,
                    expression.span(),
                    "boolean negation operand",
                )?;
                if operand_type != NativeType::Bool {
                    return Err(type_mismatch(
                        format!("unary `!` requires bool, found {operand_type}"),
                        span,
                    ));
                }
                self.instructions.push(IrInst::BoolNot);
                Ok(ExprResult::Value(NativeType::Bool))
            }
            UnaryOp::BitNot | UnaryOp::AddressOf | UnaryOp::Deref => Err(unsupported(
                "systems unary operators in the v11.0 single-image profile",
                span,
            )),
        }
    }

    fn compile_call(&mut self, name: &str, args: &[Expr], span: Span) -> NativeResult<ExprResult> {
        if matches!(name, "print" | "len") {
            return Err(Diagnostic::new(
                DiagnosticCategory::UnsupportedFeature,
                format!(
                    "hosted builtin `{name}` is unavailable in the freestanding native profile"
                ),
                span,
            ));
        }

        let signature = self.signatures.get(name).cloned().ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::UnresolvedFunction,
                format!("unresolved function `{name}`"),
                span,
            )
        })?;
        if args.len() != signature.params.len() {
            return Err(Diagnostic::new(
                DiagnosticCategory::WrongArity,
                format!(
                    "function `{name}` expects {} arguments, got {}",
                    signature.params.len(),
                    args.len()
                ),
                span,
            ));
        }

        // Preserve the tree interpreter's observable left-to-right argument
        // evaluation order. The backend is responsible for arranging these
        // values according to DBYTE32 without re-evaluating them.
        for (index, (argument, expected)) in args.iter().zip(&signature.params).enumerate() {
            let actual = require_value(
                self.compile_expr(argument)?,
                argument.span(),
                "function argument",
            )?;
            if actual != *expected {
                return Err(type_mismatch(
                    format!(
                        "argument {} to `{name}` expects {expected}, found {actual}",
                        index + 1
                    ),
                    argument.span(),
                ));
            }
        }
        self.instructions.push(IrInst::Call {
            function: name.to_string(),
            argument_count: args.len(),
            result: signature.return_type,
        });
        Ok(ExprResult::Value(signature.return_type))
    }

    fn compile_intrinsic(
        &mut self,
        object: &Expr,
        property: &str,
        args: &[Expr],
        span: Span,
    ) -> NativeResult<ExprResult> {
        if !matches!(object, Expr::Ident(namespace, _) if namespace == "native") {
            return Err(unsupported(
                "member calls outside the `native.*` intrinsic namespace",
                span,
            ));
        }

        let intrinsic = Intrinsic::from_name(property).ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::InvalidIntrinsic,
                format!("unknown native intrinsic `native.{property}`"),
                span,
            )
        })?;
        let expected_types = intrinsic.parameter_types();
        if args.len() != expected_types.len() {
            return Err(Diagnostic::new(
                DiagnosticCategory::WrongArity,
                format!(
                    "intrinsic `{intrinsic}` expects {} arguments, got {}",
                    expected_types.len(),
                    args.len()
                ),
                span,
            ));
        }

        for (index, (argument, expected)) in args.iter().zip(expected_types).enumerate() {
            let actual = require_value(
                self.compile_expr(argument)?,
                argument.span(),
                "intrinsic argument",
            )?;
            if actual != *expected {
                return Err(Diagnostic::new(
                    DiagnosticCategory::InvalidIntrinsic,
                    format!(
                        "argument {} to `{intrinsic}` expects {expected}, found {actual}",
                        index + 1
                    ),
                    argument.span(),
                ));
            }
        }

        let result = intrinsic.result_type();
        self.instructions.push(IrInst::Intrinsic {
            intrinsic,
            argument_count: args.len(),
            result,
        });
        Ok(match result {
            Some(ty) => ExprResult::Value(ty),
            None => ExprResult::Void,
        })
    }

    fn ensure_local_available(&self, name: &str, span: Span) -> NativeResult<()> {
        if self
            .scopes
            .last()
            .is_some_and(|scope| scope.contains_key(name))
        {
            return Err(Diagnostic::new(
                DiagnosticCategory::DuplicateLocal,
                format!("duplicate local `{name}`"),
                span,
            ));
        }
        Ok(())
    }

    fn allocate_slot(
        &mut self,
        name: &str,
        ty: NativeType,
        is_param: bool,
        span: Span,
    ) -> NativeResult<LocalId> {
        self.ensure_local_available(name, span)?;
        let index = u32::try_from(self.slots.len()).map_err(|_| {
            Diagnostic::new(
                DiagnosticCategory::UnsupportedFeature,
                "native function exceeds the u32 local-slot limit",
                span,
            )
        })?;
        let slot = LocalId(index);
        self.slots.push(IrSlot {
            name: name.to_string(),
            ty,
            is_param,
        });
        self.scopes
            .last_mut()
            .expect("function always has a lexical scope")
            .insert(name.to_string(), Binding { slot, ty });
        Ok(slot)
    }

    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn new_label(&mut self, span: Span) -> NativeResult<LabelId> {
        let label = LabelId(self.next_label);
        self.next_label = self.next_label.checked_add(1).ok_or_else(|| {
            Diagnostic::new(
                DiagnosticCategory::UnsupportedFeature,
                "native function exceeds the u32 label limit",
                span,
            )
        })?;
        Ok(label)
    }

    fn intern_string(&mut self, bytes: &[u8], span: Span) -> NativeResult<StringId> {
        if let Some(id) = self.string_ids.get(bytes) {
            return Ok(*id);
        }
        let index = u32::try_from(self.strings.len()).map_err(|_| {
            Diagnostic::new(
                DiagnosticCategory::UnsupportedFeature,
                "native program exceeds the u32 static-string limit",
                span,
            )
        })?;
        let id = StringId(index);
        let owned = bytes.to_vec();
        self.strings.push(owned.clone());
        self.string_ids.insert(owned, id);
        Ok(id)
    }
}

fn explicit_native_type(
    annotation: &TypeAnnotation,
    span: Span,
    owner: &str,
) -> NativeResult<NativeType> {
    match annotation {
        TypeAnnotation::Int | TypeAnnotation::I32 => Ok(NativeType::I32),
        TypeAnnotation::Bool => Ok(NativeType::Bool),
        TypeAnnotation::Str => Ok(NativeType::Str),
        TypeAnnotation::Inferred => Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("{owner} requires an explicit native type"),
            span,
        )),
        TypeAnnotation::Float => Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("{owner} uses unsupported native type `float`"),
            span,
        )),
        TypeAnnotation::Bytes => Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("{owner} uses unsupported native type `bytes`"),
            span,
        )),
        TypeAnnotation::Buffer => Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("{owner} uses unsupported native type `buffer`"),
            span,
        )),
        TypeAnnotation::List(inner) => Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("{owner} uses unsupported native type `list[{inner}]`"),
            span,
        )),
        TypeAnnotation::I8
        | TypeAnnotation::U8
        | TypeAnnotation::I16
        | TypeAnnotation::U16
        | TypeAnnotation::U32
        | TypeAnnotation::Usize
        | TypeAnnotation::Pointer(_)
        | TypeAnnotation::Array { .. }
        | TypeAnnotation::Named(_) => Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("{owner} uses systems type `{annotation}` outside the v11.1 systems pipeline"),
            span,
        )),
    }
}

fn require_value(result: ExprResult, span: Span, context: &str) -> NativeResult<NativeType> {
    match result {
        ExprResult::Value(ty) => Ok(ty),
        ExprResult::Void => Err(Diagnostic::new(
            DiagnosticCategory::InvalidType,
            format!("{context} requires a value, found void intrinsic result"),
            span,
        )),
    }
}

fn type_mismatch(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::new(DiagnosticCategory::InvalidType, message, span)
}

fn unsupported(feature: &str, span: Span) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCategory::UnsupportedFeature,
        format!("{feature} are unsupported in the freestanding native profile"),
        span,
    )
}

fn integer_range(value: i64, span: Span) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCategory::IntegerOutOfRange,
        format!("integer literal `{value}` is outside native i32 range"),
        span,
    )
}

fn block_always_returns(statements: &[Stmt]) -> bool {
    for statement in statements {
        if statement_always_returns(statement) {
            return true;
        }
    }
    false
}

fn statement_always_returns(statement: &Stmt) -> bool {
    match statement {
        Stmt::Return { .. } => true,
        Stmt::If {
            then_body,
            else_body: Some(else_body),
            ..
        } => block_always_returns(then_body) && block_always_returns(else_body),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn parse(source: &str) -> Program {
        let mut lexer = Lexer::new(source);
        let tokens = lexer.tokenize().expect("test source must lex");
        let mut parser = Parser::new(tokens);
        parser.parse_program().expect("test source must parse")
    }

    fn analyze_source(source: &str) -> IrProgram {
        analyze(&parse(source)).expect("native analysis must succeed")
    }

    fn analyze_error(source: &str) -> Diagnostic {
        analyze(&parse(source)).expect_err("native analysis must reject source")
    }

    fn kernel(program: &IrProgram) -> &IrFunction {
        program
            .function(ENTRY_FUNCTION)
            .expect("test program must contain kernel_main")
    }

    #[test]
    fn lowers_arithmetic_local_assignment_and_return() {
        let program = analyze_source(
            "fn kernel_main() -> int:\n    let value: int = 40\n    value = value + 2 * 1\n    return value\n",
        );
        let function = kernel(&program);
        assert_eq!(
            function.slots,
            vec![IrSlot {
                name: "value".to_owned(),
                ty: NativeType::I32,
                is_param: false,
            }]
        );
        assert_eq!(
            function.instructions,
            vec![
                IrInst::ConstI32(40),
                IrInst::LocalStore(LocalId(0)),
                IrInst::LocalLoad(LocalId(0)),
                IrInst::ConstI32(2),
                IrInst::ConstI32(1),
                IrInst::MulI32,
                IrInst::AddI32,
                IrInst::LocalStore(LocalId(0)),
                IrInst::LocalLoad(LocalId(0)),
                IrInst::Return,
            ]
        );
    }

    #[test]
    fn lowers_if_else_and_ordered_comparison() {
        let program = analyze_source(
            "fn choose(value: int) -> int:\n    if value >= 42:\n        return 1\n    else:\n        return 0\n\nfn kernel_main() -> int:\n    return choose(42)\n",
        );
        let choose = program.function("choose").unwrap();
        assert!(choose
            .instructions
            .contains(&IrInst::Compare(CompareOp::GtEq)));
        assert!(choose
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, IrInst::JumpIfFalse(_))));
        assert!(choose
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, IrInst::Jump(_))));
        assert_eq!(
            kernel(&program).instructions,
            vec![
                IrInst::ConstI32(42),
                IrInst::Call {
                    function: "choose".to_owned(),
                    argument_count: 1,
                    result: NativeType::I32,
                },
                IrInst::Return,
            ]
        );
    }

    #[test]
    fn lowers_while_loop_with_back_edge() {
        let program = analyze_source(
            "fn kernel_main() -> int:\n    let value: int = 0\n    while value < 3:\n        value = value + 1\n    return value\n",
        );
        let instructions = &kernel(&program).instructions;
        assert!(instructions.contains(&IrInst::Label(LabelId(0))));
        assert!(instructions.contains(&IrInst::Compare(CompareOp::Lt)));
        assert!(instructions.contains(&IrInst::JumpIfFalse(LabelId(1))));
        assert!(instructions.contains(&IrInst::Jump(LabelId(0))));
        assert!(instructions.contains(&IrInst::Label(LabelId(1))));
    }

    #[test]
    fn resolves_forward_calls_and_preserves_argument_order() {
        let program = analyze_source(
            "fn kernel_main() -> int:\n    return subtract(40, 2)\n\nfn subtract(left: int, right: int) -> int:\n    return left - right\n",
        );
        assert_eq!(
            kernel(&program).instructions,
            vec![
                IrInst::ConstI32(40),
                IrInst::ConstI32(2),
                IrInst::Call {
                    function: "subtract".to_owned(),
                    argument_count: 2,
                    result: NativeType::I32,
                },
                IrInst::Return,
            ]
        );
        let subtract = program.function("subtract").unwrap();
        assert_eq!(subtract.params[0].slot, LocalId(0));
        assert_eq!(subtract.params[1].slot, LocalId(1));
    }

    #[test]
    fn lowers_bool_values_not_and_equality() {
        let program = analyze_source(
            "fn invert(value: bool) -> bool:\n    return !value\n\nfn kernel_main() -> int:\n    if invert(false) == true:\n        return 1\n    else:\n        return 0\n",
        );
        assert!(program
            .function("invert")
            .unwrap()
            .instructions
            .contains(&IrInst::BoolNot));
        assert!(kernel(&program)
            .instructions
            .contains(&IrInst::Compare(CompareOp::Eq)));
    }

    #[test]
    fn interns_ascii_strings_once_and_lowers_indexing() {
        let program = analyze_source(
            "fn first(text: str) -> int:\n    return text[0]\n\nfn kernel_main() -> int:\n    let left: str = \"AA\"\n    let right: str = \"AA\"\n    return first(left) + right[1]\n",
        );
        assert_eq!(program.strings, vec![b"AA".to_vec()]);
        let instructions = &kernel(&program).instructions;
        assert_eq!(
            instructions
                .iter()
                .filter(|instruction| matches!(instruction, IrInst::ConstStr(StringId(0))))
                .count(),
            2
        );
        assert!(instructions.contains(&IrInst::StrIndex8));
    }

    #[test]
    fn lowers_the_explicit_native_intrinsic_set() {
        let program = analyze_source(
            "fn kernel_main() -> int:\n    let byte: int = native.volatile_load8(753664)\n    native.volatile_store8(753664, byte)\n    let word: int = native.volatile_load32(753664)\n    native.volatile_store32(753664, word)\n    native.io_out8(1016, byte)\n    let input: int = native.io_in8(1016)\n    native.cpu_hlt()\n    return byte + word + input\n",
        );
        let instructions = &kernel(&program).instructions;
        for intrinsic in [
            Intrinsic::VolatileLoad8,
            Intrinsic::VolatileStore8,
            Intrinsic::VolatileLoad32,
            Intrinsic::VolatileStore32,
            Intrinsic::IoOut8,
            Intrinsic::IoIn8,
            Intrinsic::CpuHlt,
        ] {
            assert!(instructions.iter().any(|instruction| {
                matches!(instruction, IrInst::Intrinsic { intrinsic: found, .. } if *found == intrinsic)
            }));
        }
    }

    #[test]
    fn accepts_both_i32_literal_boundaries() {
        let program = analyze_source(
            "fn kernel_main() -> int:\n    let low: int = -2147483648\n    let high: int = 2147483647\n    return low + high\n",
        );
        let instructions = &kernel(&program).instructions;
        assert!(instructions.contains(&IrInst::ConstI32(i32::MIN)));
        assert!(instructions.contains(&IrInst::ConstI32(i32::MAX)));
    }

    #[test]
    fn permits_inner_scope_shadowing_with_distinct_slots() {
        let program = analyze_source(
            "fn kernel_main() -> int:\n    let value: int = 1\n    if true:\n        let value: int = 2\n        native.io_out8(1016, value)\n    return value\n",
        );
        let function = kernel(&program);
        assert_eq!(function.slots.len(), 2);
        assert_eq!(function.slots[0].name, "value");
        assert_eq!(function.slots[1].name, "value");
        assert_eq!(function.instructions.last(), Some(&IrInst::Return));
    }

    #[test]
    fn analysis_and_ir_dump_are_deterministic() {
        let source = "fn helper(value: int) -> int:\n    return value * 2\n\nfn kernel_main() -> int:\n    return helper(21)\n";
        let first = analyze_source(source);
        let second = analyze_source(source);
        assert_eq!(first, second);
        assert_eq!(first.dump(), second.dump());
    }

    #[test]
    fn rejects_missing_native_entry() {
        let error = analyze_error("fn helper() -> int:\n    return 0\n");
        assert_eq!(error.category, DiagnosticCategory::InvalidEntryPoint);
        assert_eq!(error.span, None);
    }

    #[test]
    fn rejects_invalid_native_entry_signatures() {
        for source in [
            "fn kernel_main(value: int) -> int:\n    return value\n",
            "fn kernel_main() -> bool:\n    return true\n",
        ] {
            let error = analyze_error(source);
            assert_eq!(error.category, DiagnosticCategory::InvalidEntryPoint);
            assert!(error.span.is_some());
        }
    }

    #[test]
    fn rejects_duplicate_functions() {
        let error = analyze_error(
            "fn kernel_main() -> int:\n    return 0\n\nfn kernel_main() -> int:\n    return 1\n",
        );
        assert_eq!(error.category, DiagnosticCategory::DuplicateFunction);
        assert!(error.message.contains("kernel_main"));
    }

    #[test]
    fn rejects_duplicate_parameters() {
        let error = analyze_error(
            "fn bad(value: int, value: int) -> int:\n    return value\n\nfn kernel_main() -> int:\n    return 0\n",
        );
        assert_eq!(error.category, DiagnosticCategory::DuplicateLocal);
        assert!(error.message.contains("duplicate parameter"));
    }

    #[test]
    fn rejects_duplicate_locals_in_one_scope() {
        let error = analyze_error(
            "fn kernel_main() -> int:\n    let value: int = 1\n    let value: int = 2\n    return value\n",
        );
        assert_eq!(error.category, DiagnosticCategory::DuplicateLocal);
        assert!(error.message.contains("duplicate local"));
    }

    #[test]
    fn rejects_unknown_local_symbols() {
        let error = analyze_error("fn kernel_main() -> int:\n    return missing\n");
        assert_eq!(error.category, DiagnosticCategory::UnknownSymbol);
        assert!(error.message.contains("missing"));
    }

    #[test]
    fn rejects_unresolved_function_calls() {
        let error = analyze_error("fn kernel_main() -> int:\n    return missing()\n");
        assert_eq!(error.category, DiagnosticCategory::UnresolvedFunction);
        assert!(error.message.contains("missing"));
    }

    #[test]
    fn rejects_wrong_function_argument_count() {
        let error = analyze_error(
            "fn helper(value: int) -> int:\n    return value\n\nfn kernel_main() -> int:\n    return helper()\n",
        );
        assert_eq!(error.category, DiagnosticCategory::WrongArity);
        assert!(error.message.contains("expects 1 arguments, got 0"));
    }

    #[test]
    fn rejects_wrong_intrinsic_argument_count() {
        let error =
            analyze_error("fn kernel_main() -> int:\n    native.io_out8(1016)\n    return 0\n");
        assert_eq!(error.category, DiagnosticCategory::WrongArity);
        assert!(error.message.contains("native.io_out8"));
    }

    #[test]
    fn rejects_unknown_and_mistyped_intrinsics() {
        let unknown = analyze_error(
            "fn kernel_main() -> int:\n    native.no_such_intrinsic()\n    return 0\n",
        );
        assert_eq!(unknown.category, DiagnosticCategory::InvalidIntrinsic);

        let mistyped = analyze_error(
            "fn kernel_main() -> int:\n    native.io_out8(1016, true)\n    return 0\n",
        );
        assert_eq!(mistyped.category, DiagnosticCategory::InvalidIntrinsic);
        assert!(mistyped.message.contains("expects i32, found bool"));
    }

    #[test]
    fn rejects_invalid_return_forms() {
        let cases = [
            "fn kernel_main() -> int:\n    return\n",
            "fn kernel_main() -> int:\n    return true\n",
            "fn kernel_main() -> int:\n    if true:\n        return 1\n",
        ];
        for source in cases {
            let error = analyze_error(source);
            assert_eq!(error.category, DiagnosticCategory::InvalidReturn);
        }
    }

    #[test]
    fn rejects_incompatible_native_types() {
        let cases = [
            "fn kernel_main() -> int:\n    return 1 + true\n",
            "fn kernel_main() -> int:\n    if 1:\n        return 1\n    else:\n        return 0\n",
            "fn kernel_main() -> int:\n    let value: int = 1\n    value = false\n    return value\n",
            "fn kernel_main() -> int:\n    let text: str = \"A\"\n    return text[false]\n",
            "fn kernel_main() -> int:\n    return !1\n",
        ];
        for source in cases {
            let error = analyze_error(source);
            assert_eq!(error.category, DiagnosticCategory::InvalidType);
        }
    }

    #[test]
    fn rejects_unsupported_native_type_annotations() {
        let cases = [
            "fn helper(value: float) -> int:\n    return 0\n\nfn kernel_main() -> int:\n    return 0\n",
            "fn helper(value: bytes) -> int:\n    return 0\n\nfn kernel_main() -> int:\n    return 0\n",
            "fn helper(value: buffer) -> int:\n    return 0\n\nfn kernel_main() -> int:\n    return 0\n",
            "fn helper(value: list[int]) -> int:\n    return 0\n\nfn kernel_main() -> int:\n    return 0\n",
            "fn helper() :\n    return 0\n\nfn kernel_main() -> int:\n    return 0\n",
        ];
        for source in cases {
            let error = analyze_error(source);
            assert_eq!(error.category, DiagnosticCategory::InvalidType);
        }
    }

    #[test]
    fn rejects_hosted_or_dynamic_constructs_before_lowering() {
        let cases = [
            "fn kernel_main() -> int:\n    print(1)\n    return 0\n",
            "fn kernel_main() -> int:\n    len(\"A\")\n    return 0\n",
            "fn kernel_main() -> int:\n    return 4 / 2\n",
            "fn kernel_main() -> int:\n    let value = 1.5\n    return 0\n",
            "fn kernel_main() -> int:\n    let value = b\"A\"\n    return 0\n",
            "fn kernel_main() -> int:\n    let value = \"answer {missing}\"\n    return 0\n",
            "fn kernel_main() -> int:\n    let value = [1, 2]\n    return 0\n",
            "fn kernel_main() -> int:\n    for value in [1]:\n        native.cpu_hlt()\n    return 0\n",
            "fn kernel_main() -> int:\n    import hosted as hosted\n    return 0\n",
            "fn kernel_main() -> int:\n    hosted.call()\n    return 0\n",
            "fn kernel_main() -> int:\n    pub let value: int = 1\n    return value\n",
            "fn kernel_main() -> int:\n    fn nested() -> int:\n        return 0\n    return 0\n",
            "let global: int = 1\n\nfn kernel_main() -> int:\n    return global\n",
        ];
        for source in cases {
            let error = analyze_error(source);
            assert_eq!(
                error.category,
                DiagnosticCategory::UnsupportedFeature,
                "unexpected diagnostic for source:\n{source}\n{error}"
            );
        }
    }

    #[test]
    fn rejects_non_ascii_static_strings() {
        let error =
            analyze_error("fn kernel_main() -> int:\n    let text: str = \"café\"\n    return 0\n");
        assert_eq!(error.category, DiagnosticCategory::UnsupportedFeature);
        assert!(error.message.contains("non-ASCII"));
    }

    #[test]
    fn rejects_literals_outside_native_i32_range() {
        for source in [
            "fn kernel_main() -> int:\n    return 2147483648\n",
            "fn kernel_main() -> int:\n    return -2147483649\n",
        ] {
            let error = analyze_error(source);
            assert_eq!(error.category, DiagnosticCategory::IntegerOutOfRange);
        }
    }

    #[test]
    fn rejects_void_intrinsic_where_a_value_is_required() {
        let error = analyze_error(
            "fn kernel_main() -> int:\n    let value = native.cpu_hlt()\n    return 0\n",
        );
        assert_eq!(error.category, DiagnosticCategory::InvalidType);
        assert!(error.message.contains("void intrinsic result"));
    }
}
