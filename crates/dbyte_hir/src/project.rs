//! Canonical project name resolution before executable-HIR construction.
//!
//! The resolver rewrites module-qualified source references to unique internal
//! identities in one deterministic project type universe. It is deliberately
//! backend-neutral: no object symbol, relocation or machine concept appears
//! here.

use super::{Diagnostic, ModuleId, ProjectInput};
use dbyte_ast::{Expr, MatchArm, Param, Pattern, Program, Span, Stmt, TypeAnnotation};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedProjectAst {
    pub program: Program,
    pub owners: BTreeMap<String, (ModuleId, String)>,
}

#[derive(Debug, Clone)]
struct Decl {
    internal: String,
    source: String,
    public: bool,
    fields: Option<Vec<String>>,
    value: bool,
}

pub(crate) fn resolve_project_ast(
    project: &ProjectInput,
) -> Result<ResolvedProjectAst, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut declarations = Vec::<BTreeMap<String, Decl>>::with_capacity(project.modules.len());
    let captured_root_values = project
        .modules
        .iter()
        .find(|module| module.id == project.root)
        .map_or_else(BTreeSet::new, |module| {
            let root_values: BTreeMap<_, _> = module
                .program
                .stmts
                .iter()
                .filter_map(|statement| match statement {
                    Stmt::Let { name, value, .. } => Some((name.clone(), value)),
                    _ => None,
                })
                .collect();
            let mut referenced = BTreeSet::new();
            for statement in &module.program.stmts {
                if let Stmt::FnDef { params, body, .. } = statement {
                    let mut function_references = BTreeSet::new();
                    collect_referenced_names_in_statements(body, &mut function_references);
                    let mut function_locals: BTreeSet<_> = params
                        .iter()
                        .map(|parameter| parameter.name.clone())
                        .collect();
                    collect_declared_names_in_statements(body, &mut function_locals);
                    referenced.extend(function_references.difference(&function_locals).cloned());
                }
            }
            let mut captured: BTreeSet<_> = root_values
                .keys()
                .filter(|name| referenced.contains(*name))
                .cloned()
                .collect();
            loop {
                let mut dependencies = BTreeSet::new();
                for name in &captured {
                    if let Some(initializer) = root_values.get(name) {
                        collect_referenced_names_in_expression(initializer, &mut dependencies);
                    }
                }
                let before = captured.len();
                captured.extend(
                    root_values
                        .keys()
                        .filter(|name| dependencies.contains(*name))
                        .cloned(),
                );
                if captured.len() == before {
                    break;
                }
            }
            captured
        });
    let mut public_counts = BTreeMap::<String, usize>::new();
    for module in &project.modules {
        for statement in &module.program.stmts {
            if let Some((name, true, _)) = declaration(statement) {
                *public_counts.entry(name.to_owned()).or_default() += 1;
            }
        }
    }
    let mut owners = BTreeMap::new();
    for module in &project.modules {
        let mut module_declarations = BTreeMap::new();
        for statement in &module.program.stmts {
            if module.id == project.root
                && matches!(statement, Stmt::Let { name, .. } if !captured_root_values.contains(name))
            {
                continue;
            }
            let Some((name, public, _span)) = declaration(statement) else {
                continue;
            };
            let entry = name == "kernel_main" || name == "user_main";
            let internal = if public && public_counts.get(name).copied().unwrap_or(0) > 1 {
                format!("__dbyte_pub_m{}_{}", module.id.0, name)
            } else if public || (module.id == project.root && entry) {
                name.to_owned()
            } else {
                format!("__dbyte_m{}_{}", module.id.0, name)
            };
            owners.insert(internal.clone(), (module.id, name.to_owned()));
            let fields = match statement {
                Stmt::StructDef { fields, .. } => {
                    Some(fields.iter().map(|field| field.name.clone()).collect())
                }
                _ => None,
            };
            let value = matches!(
                statement,
                Stmt::Let { .. } | Stmt::Const { .. } | Stmt::Static { .. }
            );
            module_declarations.insert(
                name.to_owned(),
                Decl {
                    internal,
                    source: name.to_owned(),
                    public,
                    fields,
                    value,
                },
            );
        }
        declarations.push(module_declarations);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut statements = Vec::new();
    for module in &project.modules {
        let aliases: BTreeMap<_, _> = module.imports.iter().cloned().collect();
        let mut host_imports = BTreeMap::new();
        collect_host_imports(&module.program.stmts, &mut host_imports);
        let host_aliases: BTreeMap<_, _> = host_imports
            .keys()
            .map(|alias| {
                (
                    alias.clone(),
                    format!("__dbyte_host_m{}_{}", module.id.0, alias),
                )
            })
            .collect();
        let mut rewriter = Rewriter {
            module: module.id,
            root: project.root,
            declarations: &declarations,
            aliases,
            host_aliases,
            locals: Vec::new(),
            diagnostics: Vec::new(),
        };
        for (alias, path) in &host_imports {
            statements.push(Stmt::Import {
                path: path.clone(),
                alias: rewriter.host_aliases[alias].clone(),
                span: Span::zero(),
            });
        }
        for statement in &module.program.stmts {
            if let Stmt::Import { path, alias, span } = statement {
                if path.starts_with("std.") {
                    let _ = (alias, span);
                }
                continue;
            }
            let mut statement = rewriter.statement(statement.clone(), true);
            shift_statement_span(&mut statement, module.id.0 as usize * 1_000_000);
            statements.push(statement);
        }
        diagnostics.append(&mut rewriter.diagnostics);
    }
    diagnostics
        .sort_by_key(|diagnostic| (diagnostic.span.line, diagnostic.span.col, diagnostic.code));
    if diagnostics.is_empty() {
        Ok(ResolvedProjectAst {
            program: Program { stmts: statements },
            owners,
        })
    } else {
        Err(diagnostics)
    }
}

fn shift(span: &mut Span, lines: usize) {
    span.line = span.line.saturating_add(lines);
}

fn shift_statement_span(statement: &mut Stmt, lines: usize) {
    // An expression statement borrows its span directly from the expression.
    // Its recursive arm below shifts that span together with every child, so
    // shifting it here as well would move the outer expression twice and can
    // alias facts from another module's canonical line partition.
    if !matches!(statement, Stmt::Expr(_)) {
        shift(statement_span_mut(statement), lines);
    }
    match statement {
        Stmt::Let { value, .. } | Stmt::Expr(value) => shift_expression_span(value, lines),
        Stmt::Assign { value, .. } => shift_expression_span(value, lines),
        Stmt::Store { target, value, .. } => {
            shift_expression_span(target, lines);
            shift_expression_span(value, lines);
        }
        Stmt::FnDef { params, body, .. } => {
            for param in params {
                shift(&mut param.span, lines);
            }
            for child in body {
                shift_statement_span(child, lines);
            }
        }
        Stmt::Return { value, .. } => {
            if let Some(value) = value {
                shift_expression_span(value, lines);
            }
        }
        Stmt::If {
            cond,
            then_body,
            else_body,
            ..
        } => {
            shift_expression_span(cond, lines);
            for child in then_body {
                shift_statement_span(child, lines);
            }
            if let Some(body) = else_body {
                for child in body {
                    shift_statement_span(child, lines);
                }
            }
        }
        Stmt::While { cond, body, .. } => {
            shift_expression_span(cond, lines);
            for child in body {
                shift_statement_span(child, lines);
            }
        }
        Stmt::For { iterable, body, .. } => {
            shift_expression_span(iterable, lines);
            for child in body {
                shift_statement_span(child, lines);
            }
        }
        Stmt::StructDef { fields, .. } => {
            for field in fields {
                shift(&mut field.span, lines);
            }
        }
        Stmt::EnumDef { variants, .. } => {
            for variant in variants {
                shift(&mut variant.span, lines);
            }
        }
        Stmt::Match { value, arms, .. } => {
            shift_expression_span(value, lines);
            for arm in arms {
                shift(&mut arm.span, lines);
                shift_pattern_span(&mut arm.pattern, lines);
                for child in &mut arm.body {
                    shift_statement_span(child, lines);
                }
            }
        }
        Stmt::Defer { captures, body, .. } => {
            for capture in captures {
                shift(&mut capture.span, lines);
            }
            for child in body {
                shift_statement_span(child, lines);
            }
        }
        Stmt::Const { value, .. } => shift_expression_span(value, lines),
        Stmt::Static { value, .. } => {
            if let Some(value) = value {
                shift_expression_span(value, lines);
            }
        }
        Stmt::Import { .. } | Stmt::Break { .. } | Stmt::Continue { .. } => {}
    }
}

fn statement_span_mut(statement: &mut Stmt) -> &mut Span {
    match statement {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Store { span, .. }
        | Stmt::FnDef { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Import { span, .. }
        | Stmt::StructDef { span, .. }
        | Stmt::EnumDef { span, .. }
        | Stmt::Match { span, .. }
        | Stmt::Defer { span, .. }
        | Stmt::Break { span }
        | Stmt::Continue { span }
        | Stmt::Const { span, .. }
        | Stmt::Static { span, .. } => span,
        Stmt::Expr(expression) => expression_span_mut(expression),
    }
}

fn shift_pattern_span(pattern: &mut Pattern, lines: usize) {
    match pattern {
        Pattern::Wildcard(span) | Pattern::Variant { span, .. } => shift(span, lines),
    }
}

fn shift_expression_span(expression: &mut Expr, lines: usize) {
    shift(expression_span_mut(expression), lines);
    match expression {
        Expr::List(values, _) => {
            for value in values {
                shift_expression_span(value, lines);
            }
        }
        Expr::Binary { left, right, .. } => {
            shift_expression_span(left, lines);
            shift_expression_span(right, lines);
        }
        Expr::Unary { expr, .. }
        | Expr::Borrow { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Drop { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Bitcast { expr, .. } => shift_expression_span(expr, lines),
        Expr::Call { args, .. } | Expr::MemberCall { args, .. } => {
            for arg in args {
                shift_expression_span(arg, lines);
            }
            if let Expr::MemberCall { object, .. } = expression {
                shift_expression_span(object, lines);
            }
        }
        Expr::Index { target, index, .. } => {
            shift_expression_span(target, lines);
            shift_expression_span(index, lines);
        }
        Expr::Member { object, .. } | Expr::GenericApply { target: object, .. } => {
            shift_expression_span(object, lines);
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
            ..
        } => {
            shift_expression_span(callee, lines);
            for arg in args {
                shift_expression_span(arg, lines);
            }
            for arg in named_args {
                shift(&mut arg.span, lines);
                shift_expression_span(&mut arg.value, lines);
            }
        }
        Expr::Closure {
            captures,
            params,
            body,
            ..
        } => {
            for capture in captures {
                shift(&mut capture.span, lines);
            }
            for param in params {
                shift(&mut param.span, lines);
            }
            for statement in body {
                shift_statement_span(statement, lines);
            }
        }
        Expr::UnitLit(..)
        | Expr::IntLit(..)
        | Expr::FloatLit(..)
        | Expr::BoolLit(..)
        | Expr::StrLit(..)
        | Expr::BytesLit(..)
        | Expr::FStr(..)
        | Expr::NullLit(..)
        | Expr::Ident(..) => {}
    }
}

fn expression_span_mut(expression: &mut Expr) -> &mut Span {
    match expression {
        Expr::UnitLit(span)
        | Expr::IntLit(_, span)
        | Expr::FloatLit(_, span)
        | Expr::BoolLit(_, span)
        | Expr::StrLit(_, span)
        | Expr::BytesLit(_, span)
        | Expr::FStr(_, span)
        | Expr::NullLit(span)
        | Expr::Ident(_, span)
        | Expr::List(_, span) => span,
        Expr::Binary { span, .. }
        | Expr::Unary { span, .. }
        | Expr::Call { span, .. }
        | Expr::Index { span, .. }
        | Expr::Member { span, .. }
        | Expr::MemberCall { span, .. }
        | Expr::GenericApply { span, .. }
        | Expr::Invoke { span, .. }
        | Expr::Borrow { span, .. }
        | Expr::Try { span, .. }
        | Expr::Drop { span, .. }
        | Expr::Closure { span, .. }
        | Expr::Cast { span, .. }
        | Expr::Bitcast { span, .. } => span,
    }
}

fn collect_declared_names_in_statements(statements: &[Stmt], declared: &mut BTreeSet<String>) {
    for statement in statements {
        match statement {
            Stmt::Let { name, .. } | Stmt::Const { name, .. } | Stmt::Static { name, .. } => {
                declared.insert(name.clone());
            }
            Stmt::FnDef {
                name, params, body, ..
            } => {
                declared.insert(name.clone());
                declared.extend(params.iter().map(|parameter| parameter.name.clone()));
                collect_declared_names_in_statements(body, declared);
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_declared_names_in_statements(then_body, declared);
                if let Some(else_body) = else_body {
                    collect_declared_names_in_statements(else_body, declared);
                }
            }
            Stmt::While { body, .. } | Stmt::Defer { body, .. } => {
                collect_declared_names_in_statements(body, declared);
            }
            Stmt::For { var, body, .. } => {
                declared.insert(var.clone());
                collect_declared_names_in_statements(body, declared);
            }
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    if let Pattern::Variant { bindings, .. } = &arm.pattern {
                        declared.extend(bindings.iter().cloned());
                    }
                    collect_declared_names_in_statements(&arm.body, declared);
                }
            }
            Stmt::Import { alias, .. } => {
                declared.insert(alias.clone());
            }
            Stmt::StructDef { name, .. } | Stmt::EnumDef { name, .. } => {
                declared.insert(name.clone());
            }
            Stmt::Assign { .. }
            | Stmt::Store { .. }
            | Stmt::Return { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Expr(_) => {}
        }
    }
}

fn collect_referenced_names_in_statements(statements: &[Stmt], referenced: &mut BTreeSet<String>) {
    for statement in statements {
        match statement {
            Stmt::Let { value, .. } | Stmt::Const { value, .. } | Stmt::Expr(value) => {
                collect_referenced_names_in_expression(value, referenced);
            }
            Stmt::Assign { name, value, .. } => {
                referenced.insert(name.clone());
                collect_referenced_names_in_expression(value, referenced);
            }
            Stmt::Store { target, value, .. } => {
                collect_referenced_names_in_expression(target, referenced);
                collect_referenced_names_in_expression(value, referenced);
            }
            Stmt::FnDef { body, .. } | Stmt::While { body, .. } | Stmt::Defer { body, .. } => {
                collect_referenced_names_in_statements(body, referenced);
            }
            Stmt::Return { value, .. } => {
                if let Some(value) = value {
                    collect_referenced_names_in_expression(value, referenced);
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
                ..
            } => {
                collect_referenced_names_in_expression(cond, referenced);
                collect_referenced_names_in_statements(then_body, referenced);
                if let Some(else_body) = else_body {
                    collect_referenced_names_in_statements(else_body, referenced);
                }
            }
            Stmt::For { iterable, body, .. } => {
                collect_referenced_names_in_expression(iterable, referenced);
                collect_referenced_names_in_statements(body, referenced);
            }
            Stmt::Match { value, arms, .. } => {
                collect_referenced_names_in_expression(value, referenced);
                for arm in arms {
                    collect_referenced_names_in_statements(&arm.body, referenced);
                }
            }
            Stmt::Static { value, .. } => {
                if let Some(value) = value {
                    collect_referenced_names_in_expression(value, referenced);
                }
            }
            Stmt::Import { .. }
            | Stmt::StructDef { .. }
            | Stmt::EnumDef { .. }
            | Stmt::Break { .. }
            | Stmt::Continue { .. } => {}
        }
    }
}

fn collect_referenced_names_in_expression(expression: &Expr, referenced: &mut BTreeSet<String>) {
    match expression {
        Expr::Ident(name, _) => {
            referenced.insert(name.clone());
        }
        Expr::Call { name, args, .. } => {
            referenced.insert(name.clone());
            for argument in args {
                collect_referenced_names_in_expression(argument, referenced);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_referenced_names_in_expression(left, referenced);
            collect_referenced_names_in_expression(right, referenced);
        }
        Expr::Unary { expr, .. }
        | Expr::Borrow { expr, .. }
        | Expr::Try { expr, .. }
        | Expr::Drop { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Bitcast { expr, .. }
        | Expr::GenericApply { target: expr, .. }
        | Expr::Member { object: expr, .. } => {
            collect_referenced_names_in_expression(expr, referenced);
        }
        Expr::List(values, _) => {
            for value in values {
                collect_referenced_names_in_expression(value, referenced);
            }
        }
        Expr::Index { target, index, .. } => {
            collect_referenced_names_in_expression(target, referenced);
            collect_referenced_names_in_expression(index, referenced);
        }
        Expr::MemberCall { object, args, .. } => {
            collect_referenced_names_in_expression(object, referenced);
            for argument in args {
                collect_referenced_names_in_expression(argument, referenced);
            }
        }
        Expr::Invoke {
            callee,
            args,
            named_args,
            ..
        } => {
            collect_referenced_names_in_expression(callee, referenced);
            for argument in args {
                collect_referenced_names_in_expression(argument, referenced);
            }
            for argument in named_args {
                collect_referenced_names_in_expression(&argument.value, referenced);
            }
        }
        Expr::Closure { body, .. } => {
            collect_referenced_names_in_statements(body, referenced);
        }
        Expr::UnitLit(..)
        | Expr::IntLit(..)
        | Expr::FloatLit(..)
        | Expr::BoolLit(..)
        | Expr::StrLit(..)
        | Expr::BytesLit(..)
        | Expr::FStr(..)
        | Expr::NullLit(..) => {}
    }
}

fn declaration(statement: &Stmt) -> Option<(&str, bool, Span)> {
    match statement {
        Stmt::FnDef {
            name, is_pub, span, ..
        }
        | Stmt::StructDef {
            name, is_pub, span, ..
        }
        | Stmt::EnumDef {
            name, is_pub, span, ..
        }
        | Stmt::Const {
            name, is_pub, span, ..
        }
        | Stmt::Let {
            name, is_pub, span, ..
        }
        | Stmt::Static {
            name, is_pub, span, ..
        } => Some((name, *is_pub, *span)),
        _ => None,
    }
}

struct Rewriter<'a> {
    module: ModuleId,
    root: ModuleId,
    declarations: &'a [BTreeMap<String, Decl>],
    aliases: BTreeMap<String, ModuleId>,
    host_aliases: BTreeMap<String, String>,
    locals: Vec<BTreeSet<String>>,
    diagnostics: Vec<Diagnostic>,
}

impl Rewriter<'_> {
    fn local(&self, name: &str) -> bool {
        self.locals.iter().rev().any(|scope| scope.contains(name))
    }

    fn local_name(&self, name: &str) -> String {
        if self.local(name) {
            return name.to_owned();
        }
        if let Some(alias) = self.host_aliases.get(name) {
            return alias.clone();
        }
        self.declarations[self.module.0 as usize]
            .get(name)
            .map_or_else(|| name.to_owned(), |decl| decl.internal.clone())
    }

    fn local_decl(&self, name: &str) -> Option<&Decl> {
        if self.local(name) {
            None
        } else {
            self.declarations[self.module.0 as usize].get(name)
        }
    }

    fn value_target(&self, expression: &Expr) -> bool {
        match expression {
            Expr::Ident(name, _) => {
                self.local(name)
                    || self.declarations[self.module.0 as usize]
                        .get(name)
                        .is_some_and(|decl| decl.value)
            }
            Expr::Member {
                object, property, ..
            } => {
                let Expr::Ident(alias, _) = object.as_ref() else {
                    return false;
                };
                self.aliases
                    .get(alias)
                    .and_then(|module| self.declarations[module.0 as usize].get(property))
                    .is_some_and(|decl| decl.public && decl.value)
            }
            _ => false,
        }
    }

    fn imported_name(&mut self, alias: &str, name: &str, span: Span) -> Option<String> {
        let target = self.aliases.get(alias).copied()?;
        let Some(decl) = self.declarations[target.0 as usize].get(name) else {
            self.diagnostics.push(Diagnostic {
                code: "E_UNRESOLVED_IMPORT_SYMBOL",
                message: format!("module alias `{alias}` has no symbol `{name}`"),
                span,
            });
            return None;
        };
        if !decl.public {
            self.diagnostics.push(Diagnostic {
                code: "E_PRIVATE_IMPORT",
                message: format!(
                    "TypeError: module '{alias}' has no public member '{}' (`{alias}.{}` is module-private)",
                    decl.source, decl.source
                ),
                span,
            });
            return None;
        }
        Some(decl.internal.clone())
    }

    fn annotation(&mut self, annotation: TypeAnnotation, span: Span) -> TypeAnnotation {
        match annotation {
            TypeAnnotation::Named(name) => {
                TypeAnnotation::Named(self.resolve_type_name(&name, span))
            }
            TypeAnnotation::Generic { base, args } => TypeAnnotation::Generic {
                base: self.resolve_type_name(&base, span),
                args: args
                    .into_iter()
                    .map(|arg| self.annotation(arg, span))
                    .collect(),
            },
            TypeAnnotation::List(inner) => {
                TypeAnnotation::List(Box::new(self.annotation(*inner, span)))
            }
            TypeAnnotation::Pointer(inner) => {
                TypeAnnotation::Pointer(Box::new(self.annotation(*inner, span)))
            }
            TypeAnnotation::ConstPointer(inner) => {
                TypeAnnotation::ConstPointer(Box::new(self.annotation(*inner, span)))
            }
            TypeAnnotation::Borrow { mutable, pointee } => TypeAnnotation::Borrow {
                mutable,
                pointee: Box::new(self.annotation(*pointee, span)),
            },
            TypeAnnotation::Slice(inner) => {
                TypeAnnotation::Slice(Box::new(self.annotation(*inner, span)))
            }
            TypeAnnotation::MutSlice(inner) => {
                TypeAnnotation::MutSlice(Box::new(self.annotation(*inner, span)))
            }
            TypeAnnotation::Array { element, length } => TypeAnnotation::Array {
                element: Box::new(self.annotation(*element, span)),
                length,
            },
            TypeAnnotation::Function {
                convention,
                params,
                ret,
            } => TypeAnnotation::Function {
                convention,
                params: params
                    .into_iter()
                    .map(|param| self.annotation(param, span))
                    .collect(),
                ret: Box::new(self.annotation(*ret, span)),
            },
            TypeAnnotation::Closure { params, ret } => TypeAnnotation::Closure {
                params: params
                    .into_iter()
                    .map(|param| self.annotation(param, span))
                    .collect(),
                ret: Box::new(self.annotation(*ret, span)),
            },
            other => other,
        }
    }

    fn resolve_type_name(&mut self, name: &str, span: Span) -> String {
        if let Some((alias, property)) = name.split_once('.') {
            return self
                .imported_name(alias, property, span)
                .unwrap_or_else(|| name.to_owned());
        }
        self.local_name(name)
    }

    fn params(&mut self, params: Vec<Param>) -> Vec<Param> {
        params
            .into_iter()
            .map(|param| Param {
                ty: self.annotation(param.ty, param.span),
                ..param
            })
            .collect()
    }

    fn block(&mut self, statements: Vec<Stmt>) -> Vec<Stmt> {
        self.locals.push(BTreeSet::new());
        let output = statements
            .into_iter()
            .map(|statement| self.statement(statement, false))
            .collect();
        self.locals.pop();
        output
    }

    fn statement(&mut self, statement: Stmt, top_level: bool) -> Stmt {
        match statement {
            Stmt::Import { path, alias, span } if path.starts_with("std.") => Stmt::Import {
                path,
                alias: self.host_aliases.get(&alias).cloned().unwrap_or(alias),
                span,
            },
            Stmt::FnDef {
                is_pub,
                kind,
                interrupt,
                call_convention,
                name,
                generic_params,
                params,
                ret_ty,
                body,
                span,
            } => {
                let name = if top_level {
                    self.local_name(&name)
                } else {
                    name
                };
                let params = self.params(params);
                self.locals
                    .push(params.iter().map(|param| param.name.clone()).collect());
                let body = body
                    .into_iter()
                    .map(|statement| self.statement(statement, false))
                    .collect();
                self.locals.pop();
                Stmt::FnDef {
                    is_pub,
                    kind,
                    interrupt,
                    call_convention,
                    name,
                    generic_params,
                    params,
                    ret_ty: self.annotation(ret_ty, span),
                    body,
                    span,
                }
            }
            Stmt::StructDef {
                is_pub,
                is_packed,
                repr,
                name,
                generic_params,
                fields,
                span,
            } => Stmt::StructDef {
                is_pub,
                is_packed,
                repr,
                name: if top_level {
                    self.local_name(&name)
                } else {
                    name
                },
                generic_params,
                fields: fields
                    .into_iter()
                    .map(|field| dbyte_ast::StructField {
                        ty: self.annotation(field.ty, field.span),
                        ..field
                    })
                    .collect(),
                span,
            },
            Stmt::EnumDef {
                is_pub,
                name,
                generic_params,
                variants,
                span,
            } => Stmt::EnumDef {
                is_pub,
                name: if top_level {
                    self.local_name(&name)
                } else {
                    name
                },
                generic_params,
                variants: variants
                    .into_iter()
                    .map(|variant| dbyte_ast::EnumVariant {
                        payload: variant
                            .payload
                            .into_iter()
                            .map(|payload| self.annotation(payload, variant.span))
                            .collect(),
                        ..variant
                    })
                    .collect(),
                span,
            },
            Stmt::Let {
                is_pub,
                name,
                ty,
                value,
                span,
            } => {
                if top_level
                    && (self.module != self.root
                        || self.declarations[self.module.0 as usize].contains_key(&name))
                {
                    return Stmt::Static {
                        is_pub,
                        is_mut: true,
                        name: self.local_name(&name),
                        ty: self.annotation(ty, span),
                        value: Some(self.expression(value)),
                        span,
                    };
                }
                let output = Stmt::Let {
                    is_pub,
                    name: name.clone(),
                    ty: self.annotation(ty, span),
                    value: self.expression(value),
                    span,
                };
                if let Some(scope) = self.locals.last_mut() {
                    scope.insert(name);
                }
                output
            }
            Stmt::Assign { name, value, span } => Stmt::Assign {
                name: self.local_name(&name),
                value: self.expression(value),
                span,
            },
            Stmt::Store {
                target,
                value,
                span,
            } => Stmt::Store {
                target: self.expression(target),
                value: self.expression(value),
                span,
            },
            Stmt::Return { value, span } => Stmt::Return {
                value: value.map(|value| self.expression(value)),
                span,
            },
            Stmt::If {
                cond,
                then_body,
                else_body,
                span,
            } => Stmt::If {
                cond: self.expression(cond),
                then_body: self.block(then_body),
                else_body: else_body.map(|body| self.block(body)),
                span,
            },
            Stmt::While { cond, body, span } => Stmt::While {
                cond: self.expression(cond),
                body: self.block(body),
                span,
            },
            Stmt::For {
                var,
                iterable,
                body,
                span,
            } => {
                self.locals.push(BTreeSet::from([var.clone()]));
                let body = body
                    .into_iter()
                    .map(|statement| self.statement(statement, false))
                    .collect();
                self.locals.pop();
                Stmt::For {
                    var,
                    iterable: self.expression(iterable),
                    body,
                    span,
                }
            }
            Stmt::Match {
                mode,
                value,
                arms,
                span,
            } => Stmt::Match {
                mode,
                value: self.expression(value),
                arms: arms.into_iter().map(|arm| self.match_arm(arm)).collect(),
                span,
            },
            Stmt::Defer {
                captures,
                body,
                span,
            } => Stmt::Defer {
                captures,
                body: self.block(body),
                span,
            },
            Stmt::Const {
                is_pub,
                name,
                ty,
                value,
                span,
            } => Stmt::Const {
                is_pub,
                name: if top_level {
                    self.local_name(&name)
                } else {
                    name
                },
                ty: self.annotation(ty, span),
                value: self.expression(value),
                span,
            },
            Stmt::Static {
                is_pub,
                is_mut,
                name,
                ty,
                value,
                span,
            } => Stmt::Static {
                is_pub,
                is_mut,
                name: if top_level {
                    self.local_name(&name)
                } else {
                    name
                },
                ty: self.annotation(ty, span),
                value: value.map(|value| self.expression(value)),
                span,
            },
            Stmt::Expr(value) => Stmt::Expr(self.expression(value)),
            other => other,
        }
    }

    fn match_arm(&mut self, arm: MatchArm) -> MatchArm {
        let (pattern, bindings) = match arm.pattern {
            Pattern::Variant {
                path,
                bindings,
                span,
            } => {
                let path = if let Some((base, variant)) = path.rsplit_once('.') {
                    format!("{}.{}", self.resolve_type_name(base, span), variant)
                } else {
                    path
                };
                (
                    Pattern::Variant {
                        path,
                        bindings: bindings.clone(),
                        span,
                    },
                    bindings,
                )
            }
            pattern => (pattern, Vec::new()),
        };
        self.locals.push(bindings.into_iter().collect());
        let body = arm
            .body
            .into_iter()
            .map(|statement| self.statement(statement, false))
            .collect();
        self.locals.pop();
        MatchArm {
            pattern,
            body,
            ..arm
        }
    }

    fn expression(&mut self, expression: Expr) -> Expr {
        match expression {
            Expr::Ident(name, span) => Expr::Ident(self.local_name(&name), span),
            Expr::Call { name, args, span } => {
                let fields = self.local_decl(&name).and_then(|decl| decl.fields.clone());
                let name = self.local_name(&name);
                let args: Vec<_> = args.into_iter().map(|arg| self.expression(arg)).collect();
                if let Some(fields) = fields {
                    Expr::Invoke {
                        callee: Box::new(Expr::Ident(name, span)),
                        args: Vec::new(),
                        named_args: fields
                            .into_iter()
                            .zip(args)
                            .map(|(name, value)| dbyte_ast::NamedArgument {
                                name,
                                span: value.span(),
                                value,
                            })
                            .collect(),
                        span,
                    }
                } else {
                    Expr::Call { name, args, span }
                }
            }
            Expr::MemberCall {
                object,
                property,
                args,
                span,
            } => {
                if let Expr::Ident(alias, _) = object.as_ref() {
                    if self.aliases.contains_key(alias) {
                        if let Some(name) = self.imported_name(alias, &property, span) {
                            let target = self.aliases[alias];
                            let fields = self.declarations[target.0 as usize]
                                .get(&property)
                                .and_then(|decl| decl.fields.clone());
                            let args: Vec<_> =
                                args.into_iter().map(|arg| self.expression(arg)).collect();
                            if let Some(fields) = fields {
                                return Expr::Invoke {
                                    callee: Box::new(Expr::Ident(name, span)),
                                    args: Vec::new(),
                                    named_args: fields
                                        .into_iter()
                                        .zip(args)
                                        .map(|(name, value)| dbyte_ast::NamedArgument {
                                            name,
                                            span: value.span(),
                                            value,
                                        })
                                        .collect(),
                                    span,
                                };
                            }
                            return Expr::Call { name, args, span };
                        }
                    }
                }
                Expr::MemberCall {
                    object: Box::new(self.expression(*object)),
                    property,
                    args: args.into_iter().map(|arg| self.expression(arg)).collect(),
                    span,
                }
            }
            Expr::Member {
                object,
                property,
                span,
            } => {
                if let Expr::Ident(alias, _) = object.as_ref() {
                    if self.aliases.contains_key(alias) {
                        if let Some(name) = self.imported_name(alias, &property, span) {
                            return Expr::Ident(name, span);
                        }
                    }
                }
                Expr::Member {
                    object: Box::new(self.expression(*object)),
                    property,
                    span,
                }
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => Expr::Binary {
                left: Box::new(self.expression(*left)),
                op,
                right: Box::new(self.expression(*right)),
                span,
            },
            Expr::Unary { op, expr, span } => Expr::Unary {
                op,
                expr: Box::new(self.expression(*expr)),
                span,
            },
            Expr::List(values, span) => Expr::List(
                values
                    .into_iter()
                    .map(|value| self.expression(value))
                    .collect(),
                span,
            ),
            Expr::Index {
                target,
                index,
                span,
            } => Expr::Index {
                target: Box::new(self.expression(*target)),
                index: Box::new(self.expression(*index)),
                span,
            },
            Expr::GenericApply { target, args, span } => {
                // `value[index].field` and `Generic[T].member` share the
                // bracket token family. Resolve the ambiguity once from the
                // canonical value namespace, before type checking and HIR
                // construction. The backend therefore never sees or guesses
                // this source-level distinction.
                if self.value_target(&target) && args.len() == 1 {
                    if let TypeAnnotation::Named(index) = &args[0] {
                        // GenericApply historically records the target span,
                        // while a real Index records the bracket/index span.
                        // Keeping the same span for both synthesized children
                        // aliases their canonical expression facts and can make
                        // the index inherit the aggregate target type. Give the
                        // recovered value expression a deterministic child span
                        // inside the target token; source tokens only record
                        // their starting column, so this cannot collide with a
                        // sibling token on the same line.
                        let index_span = Span::new(span.line, span.col.saturating_add(1));
                        let index = Expr::Ident(index.clone(), index_span);
                        return Expr::Index {
                            target: Box::new(self.expression(*target)),
                            index: Box::new(self.expression(index)),
                            span,
                        };
                    }
                }
                Expr::GenericApply {
                    target: Box::new(self.expression(*target)),
                    args: args
                        .into_iter()
                        .map(|arg| self.annotation(arg, span))
                        .collect(),
                    span,
                }
            }
            Expr::Invoke {
                callee,
                args,
                named_args,
                span,
            } => Expr::Invoke {
                callee: Box::new(self.expression(*callee)),
                args: args.into_iter().map(|arg| self.expression(arg)).collect(),
                named_args: named_args
                    .into_iter()
                    .map(|arg| dbyte_ast::NamedArgument {
                        value: self.expression(arg.value),
                        ..arg
                    })
                    .collect(),
                span,
            },
            Expr::Borrow {
                mutable,
                expr,
                span,
            } => Expr::Borrow {
                mutable,
                expr: Box::new(self.expression(*expr)),
                span,
            },
            Expr::Try { expr, span } => Expr::Try {
                expr: Box::new(self.expression(*expr)),
                span,
            },
            Expr::Closure {
                captures,
                params,
                ret_ty,
                body,
                span,
            } => {
                let params = self.params(params);
                self.locals
                    .push(params.iter().map(|param| param.name.clone()).collect());
                let body = body
                    .into_iter()
                    .map(|statement| self.statement(statement, false))
                    .collect();
                self.locals.pop();
                Expr::Closure {
                    captures,
                    params,
                    ret_ty: self.annotation(ret_ty, span),
                    body,
                    span,
                }
            }
            Expr::Cast { expr, ty, span } => Expr::Cast {
                expr: Box::new(self.expression(*expr)),
                ty: self.annotation(ty, span),
                span,
            },
            Expr::Bitcast { expr, ty, span } => Expr::Bitcast {
                expr: Box::new(self.expression(*expr)),
                ty: self.annotation(ty, span),
                span,
            },
            other => other,
        }
    }
}

fn collect_host_imports(statements: &[Stmt], imports: &mut BTreeMap<String, String>) {
    for statement in statements {
        match statement {
            Stmt::Import { path, alias, .. } if path.starts_with("std.") => {
                imports.entry(alias.clone()).or_insert_with(|| path.clone());
            }
            Stmt::FnDef { body, .. }
            | Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Defer { body, .. } => collect_host_imports(body, imports),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_host_imports(then_body, imports);
                if let Some(body) = else_body {
                    collect_host_imports(body, imports);
                }
            }
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    collect_host_imports(&arm.body, imports);
                }
            }
            _ => {}
        }
    }
}
