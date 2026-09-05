//! Deterministic local source loading for the canonical HIR pipeline.

use crate::{Diagnostic, ModuleId, ProjectInput, ProjectModuleInput};
use dbyte_ast::{Program, Span, Stmt};
use dbyte_lexer::Lexer;
use dbyte_parser::Parser;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

const MAX_MODULES: usize = 256;
const MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
struct PendingModule {
    name: String,
    program: Program,
    imports: Vec<PendingImport>,
}

#[derive(Debug)]
struct PendingImport {
    alias: String,
    target: PathBuf,
}

/// Load a complete local-only project. Standard modules remain capability
/// declarations in their source modules and never become filesystem edges.
pub fn load_source_project(root: &Path) -> Result<ProjectInput, Vec<Diagnostic>> {
    let root = canonical_file(root, Span::zero())?;
    load_project_root(root, None)
}

pub fn load_source_project_paths(root: &Path) -> Result<Vec<PathBuf>, Vec<Diagnostic>> {
    let root = canonical_file(root, Span::zero())?;
    let mut pending = BTreeMap::new();
    let mut visiting = Vec::new();
    load_recursive(&root, None, &mut pending, &mut visiting)?;
    Ok(pending.keys().cloned().collect())
}

pub fn load_source_project_from_program(
    root: &Path,
    program: Program,
) -> Result<ProjectInput, Vec<Diagnostic>> {
    let root = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(root)
    };
    load_project_root(root, Some(program))
}

fn load_project_root(
    root: PathBuf,
    program: Option<Program>,
) -> Result<ProjectInput, Vec<Diagnostic>> {
    let mut pending = BTreeMap::new();
    let mut visiting = Vec::new();
    load_recursive(&root, program, &mut pending, &mut visiting)?;
    let ordered: Vec<_> = pending.keys().cloned().collect();
    let ids: BTreeMap<_, _> = ordered
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, path)| (path, ModuleId(index as u32)))
        .collect();
    let root_id = ids[&root];
    let modules = ordered
        .into_iter()
        .map(|path| {
            let module = pending.remove(&path).expect("ordered source module");
            ProjectModuleInput {
                id: ids[&path],
                name: module.name,
                program: module.program,
                imports: module
                    .imports
                    .into_iter()
                    .map(|import| (import.alias, ids[&import.target]))
                    .collect(),
            }
        })
        .collect();
    Ok(ProjectInput {
        root: root_id,
        modules,
    })
}

fn load_recursive(
    path: &Path,
    provided: Option<Program>,
    pending: &mut BTreeMap<PathBuf, PendingModule>,
    visiting: &mut Vec<PathBuf>,
) -> Result<(), Vec<Diagnostic>> {
    if let Some(start) = visiting.iter().position(|candidate| candidate == path) {
        let mut cycle: Vec<_> = visiting[start..]
            .iter()
            .map(|entry| display(entry))
            .collect();
        cycle.push(display(path));
        return Err(errors(
            "E_MODULE_CYCLE",
            format!(
                "ImportError: circular import detected; local module cycle: {}",
                cycle.join(" -> ")
            ),
            Span::zero(),
        ));
    }
    if pending.contains_key(path) {
        return Ok(());
    }
    if pending.len() >= MAX_MODULES {
        return Err(errors(
            "E_MODULE_LIMIT",
            format!("module graph exceeds {MAX_MODULES} modules"),
            Span::zero(),
        ));
    }
    let program = if let Some(program) = provided {
        program
    } else {
        let metadata = fs::metadata(path).map_err(|error| {
            errors(
                "E_MODULE_READ",
                format!("cannot inspect `{}`: {:?}", display(path), error.kind()),
                Span::zero(),
            )
        })?;
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(errors(
                "E_MODULE_SIZE",
                format!("`{}` exceeds {MAX_SOURCE_BYTES} bytes", display(path)),
                Span::zero(),
            ));
        }
        let source = fs::read_to_string(path).map_err(|error| {
            errors(
                "E_MODULE_READ",
                format!(
                    "cannot read `{}` as UTF-8: {:?}",
                    display(path),
                    error.kind()
                ),
                Span::zero(),
            )
        })?;
        let tokens = Lexer::new(&source).tokenize().map_err(|error| {
            errors(
                "E_LEX",
                format!("{}: {}", display(path), error.msg),
                error.span,
            )
        })?;
        Parser::new(tokens).parse_program().map_err(|error| {
            errors(
                "E_PARSE",
                format!("{}: {}", display(path), error.msg),
                error.span,
            )
        })?
    };

    visiting.push(path.to_path_buf());
    let mut aliases = BTreeMap::new();
    let mut imports = Vec::new();
    let mut discovered = Vec::new();
    collect_imports(&program.stmts, &mut discovered);
    for (requested, alias, span) in discovered {
        if let Some(existing) = aliases.get(alias) {
            if existing == requested {
                continue;
            }
            return Err(errors(
                "E_DUPLICATE_IMPORT_ALIAS",
                format!(
                    "ImportError: duplicate import alias: {alias}; duplicate import alias `{alias}` in `{}`",
                    display(path)
                ),
                span,
            ));
        }
        aliases.insert(alias.clone(), requested.to_owned());
        if requested.starts_with("std.") {
            if dbyte_module::stdlib_exports(requested).is_none() {
                return Err(errors(
                    "E_HOST_MODULE",
                    format!("ImportError: standard module not found: {requested}"),
                    span,
                ));
            }
            continue;
        }
        let target = resolve_local(path, requested, span)?;
        imports.push(PendingImport {
            alias: alias.clone(),
            target: target.clone(),
        });
        load_recursive(&target, None, pending, visiting)?;
    }
    visiting.pop();
    imports.sort_by(|left, right| {
        left.alias
            .cmp(&right.alias)
            .then(left.target.cmp(&right.target))
    });
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            errors(
                "E_MODULE_NAME",
                format!("`{}` has no UTF-8 module name", display(path)),
                Span::zero(),
            )
        })?
        .to_owned();
    pending.insert(
        path.to_path_buf(),
        PendingModule {
            name,
            program,
            imports,
        },
    );
    Ok(())
}

fn collect_imports<'a>(statements: &'a [Stmt], imports: &mut Vec<(&'a str, &'a String, Span)>) {
    for statement in statements {
        match statement {
            Stmt::Import { path, alias, span } => imports.push((path, alias, *span)),
            Stmt::FnDef { body, .. }
            | Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::Defer { body, .. } => collect_imports(body, imports),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_imports(then_body, imports);
                if let Some(body) = else_body {
                    collect_imports(body, imports);
                }
            }
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    collect_imports(&arm.body, imports);
                }
            }
            _ => {}
        }
    }
}

fn resolve_local(importer: &Path, requested: &str, span: Span) -> Result<PathBuf, Vec<Diagnostic>> {
    if requested.contains("://") {
        return Err(errors(
            "E_NETWORK_IMPORT",
            "network and URL imports are forbidden",
            span,
        ));
    }
    let requested_path = Path::new(requested);
    if requested_path.is_absolute()
        || requested_path
            .components()
            .any(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
    {
        return Err(errors(
            "E_ABSOLUTE_IMPORT",
            "module imports must be relative",
            span,
        ));
    }
    let mut relative =
        if requested.contains('/') || requested.contains('\\') || requested.ends_with(".dby") {
            PathBuf::from(requested)
        } else {
            PathBuf::from(requested.replace('.', std::path::MAIN_SEPARATOR_STR))
        };
    if relative.extension().is_none() {
        relative.set_extension("dby");
    }
    canonical_file(
        &importer
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(relative),
        span,
    )
    .map_err(|_| {
        errors(
            "E_UNRESOLVED_IMPORT",
            format!("ImportError: local module not found: {requested}"),
            span,
        )
    })
}

fn canonical_file(path: &Path, span: Span) -> Result<PathBuf, Vec<Diagnostic>> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        errors(
            "E_UNRESOLVED_IMPORT",
            format!("cannot resolve `{}`: {:?}", display(path), error.kind()),
            span,
        )
    })?;
    if !canonical.is_file() {
        return Err(errors(
            "E_MODULE_NOT_FILE",
            format!("`{}` is not a file", display(&canonical)),
            span,
        ));
    }
    Ok(canonical)
}

fn errors(code: &'static str, message: impl Into<String>, span: Span) -> Vec<Diagnostic> {
    vec![Diagnostic {
        code,
        message: message.into(),
        span,
    }]
}

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn std_capabilities_are_not_local_module_edges() {
        let root = std::env::temp_dir().join(format!(
            "dbyte-hir-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::write(
            root.join("main.dby"),
            "import std.math as math\nimport \"dep.dby\" as dep\nprint(math.abs(dep.answer()))\n",
        )
        .unwrap();
        fs::write(
            root.join("dep.dby"),
            "pub fn answer() -> int:\n    return -42\n",
        )
        .unwrap();
        let project = load_source_project(&root.join("main.dby")).unwrap();
        assert_eq!(project.modules.len(), 2);
        assert_eq!(project.modules[project.root.0 as usize].imports.len(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
