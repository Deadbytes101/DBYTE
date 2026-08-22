//! Deterministic, filesystem-only native module graph loading.

use crate::diagnostics::{Diagnostic, DiagnosticCategory, NativeResult};
use dbyte_ast::{Program, Span, Stmt};
use dbyte_lexer::Lexer;
use dbyte_parser::Parser;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const MAX_NATIVE_MODULES: usize = 256;
pub const MAX_NATIVE_SOURCE_SIZE: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct NativeImport {
    pub source_path: String,
    pub alias: String,
    pub target: usize,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct NativeSourceModule {
    pub id: usize,
    pub name: String,
    pub path: PathBuf,
    pub display_path: String,
    pub program: Program,
    pub imports: Vec<NativeImport>,
}

#[derive(Debug, Clone)]
pub struct NativeModuleGraph {
    pub root: usize,
    /// Canonical deterministic order: normalized absolute path text.
    pub modules: Vec<NativeSourceModule>,
}

impl NativeModuleGraph {
    pub fn module(&self, id: usize) -> Option<&NativeSourceModule> {
        self.modules.get(id)
    }

    pub fn root_module(&self) -> &NativeSourceModule {
        &self.modules[self.root]
    }
}

#[derive(Debug)]
struct PendingModule {
    canonical: PathBuf,
    display_path: String,
    name: String,
    program: Program,
    imports: Vec<PendingImport>,
}

#[derive(Debug)]
struct PendingImport {
    source_path: String,
    alias: String,
    canonical_target: PathBuf,
    span: Span,
}

pub fn load_native_module_graph(root: &Path) -> NativeResult<NativeModuleGraph> {
    let canonical_root = canonical_source(root, None)?;
    let mut pending = BTreeMap::<PathBuf, PendingModule>::new();
    load_recursive(&canonical_root, &mut pending)?;
    if pending.len() > MAX_NATIVE_MODULES {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidModuleImport,
            format!(
                "native module graph contains {} modules, exceeding bound {MAX_NATIVE_MODULES}",
                pending.len()
            ),
        ));
    }

    let ordered_paths: Vec<PathBuf> = pending.keys().cloned().collect();
    let ids: BTreeMap<PathBuf, usize> = ordered_paths
        .iter()
        .cloned()
        .enumerate()
        .map(|(id, path)| (path, id))
        .collect();
    let root_id = ids[&canonical_root];

    let mut names = BTreeMap::<String, String>::new();
    let mut modules = Vec::with_capacity(ordered_paths.len());
    for path in &ordered_paths {
        let pending_module = pending.remove(path).expect("ordered pending module");
        if let Some(previous) = names.insert(
            pending_module.name.clone(),
            pending_module.display_path.clone(),
        ) {
            return Err(Diagnostic::without_span(
                DiagnosticCategory::DuplicateModule,
                format!(
                    "duplicate native module name '{}': '{}' and '{}'",
                    pending_module.name, previous, pending_module.display_path
                ),
            ));
        }
        let mut imports = pending_module
            .imports
            .into_iter()
            .map(|import| NativeImport {
                source_path: import.source_path,
                alias: import.alias,
                target: ids[&import.canonical_target],
                span: import.span,
            })
            .collect::<Vec<_>>();
        imports.sort_by(|left, right| {
            left.alias
                .cmp(&right.alias)
                .then(left.source_path.cmp(&right.source_path))
        });
        modules.push(NativeSourceModule {
            id: ids[path],
            name: pending_module.name,
            path: pending_module.canonical,
            display_path: pending_module.display_path,
            program: pending_module.program,
            imports,
        });
    }

    detect_cycles(&modules)?;
    Ok(NativeModuleGraph {
        root: root_id,
        modules,
    })
}

fn load_recursive(
    canonical: &Path,
    modules: &mut BTreeMap<PathBuf, PendingModule>,
) -> NativeResult<()> {
    if modules.contains_key(canonical) {
        return Ok(());
    }
    if modules.len() >= MAX_NATIVE_MODULES {
        return Err(Diagnostic::without_span(
            DiagnosticCategory::InvalidModuleImport,
            format!("native module graph exceeds bound {MAX_NATIVE_MODULES}"),
        ));
    }

    let metadata = fs::metadata(canonical).map_err(|error| {
        module_error(
            canonical,
            None,
            format!("cannot inspect native module: {:?}", error.kind()),
        )
    })?;
    if metadata.len() > MAX_NATIVE_SOURCE_SIZE {
        return Err(module_error(
            canonical,
            None,
            format!(
                "native module is {} bytes, exceeding bound {MAX_NATIVE_SOURCE_SIZE}",
                metadata.len()
            ),
        ));
    }
    let source = fs::read_to_string(canonical).map_err(|error| {
        module_error(
            canonical,
            None,
            format!(
                "cannot read native module as UTF-8 text: {:?}",
                error.kind()
            ),
        )
    })?;
    let display_path = normalized_path(canonical);
    let tokens = Lexer::new(&source).tokenize().map_err(|error| {
        Diagnostic::new(DiagnosticCategory::Lex, error.msg, error.span)
            .with_path(display_path.clone())
    })?;
    let program = Parser::new(tokens).parse_program().map_err(|error| {
        Diagnostic::new(DiagnosticCategory::Parse, error.msg, error.span)
            .with_path(display_path.clone())
    })?;
    let name = canonical
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| module_error(canonical, None, "native module has no UTF-8 file stem"))?
        .to_owned();

    // Insert a placeholder before following edges so self/mutual imports do
    // not recurse indefinitely. Cycle classification is performed later with
    // deterministic path reporting.
    modules.insert(
        canonical.to_path_buf(),
        PendingModule {
            canonical: canonical.to_path_buf(),
            display_path: display_path.clone(),
            name,
            program: program.clone(),
            imports: Vec::new(),
        },
    );

    let mut aliases = BTreeSet::new();
    let mut imports = Vec::new();
    for statement in &program.stmts {
        if let Stmt::Import { path, alias, span } = statement {
            if alias == "native" {
                return Err(Diagnostic::new(
                    DiagnosticCategory::InvalidModuleImport,
                    "module alias 'native' is reserved for freestanding intrinsics",
                    *span,
                )
                .with_path(display_path));
            }
            if !aliases.insert(alias.as_str()) {
                return Err(Diagnostic::new(
                    DiagnosticCategory::InvalidModuleImport,
                    format!("duplicate native import alias '{alias}'"),
                    *span,
                )
                .with_path(display_path));
            }
            let target = resolve_import(canonical, path, *span)?;
            imports.push(PendingImport {
                source_path: path.clone(),
                alias: alias.clone(),
                canonical_target: target.clone(),
                span: *span,
            });
            load_recursive(&target, modules)?;
        }
    }
    imports.sort_by(|left, right| {
        left.alias
            .cmp(&right.alias)
            .then(left.source_path.cmp(&right.source_path))
    });
    modules
        .get_mut(canonical)
        .expect("inserted pending native module")
        .imports = imports;
    Ok(())
}

fn resolve_import(importer: &Path, requested: &str, span: Span) -> NativeResult<PathBuf> {
    if requested.contains("://") {
        return Err(module_error(
            importer,
            Some(span),
            "native imports never use network or URL resolution",
        ));
    }
    let requested_path = Path::new(requested);
    if requested_path.is_absolute()
        || requested_path
            .components()
            .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(module_error(
            importer,
            Some(span),
            format!("native import '{requested}' must be relative"),
        ));
    }

    let parent = importer.parent().unwrap_or_else(|| Path::new("."));
    let has_separator = requested.contains('/') || requested.contains('\\');
    let mut relative = if has_separator || requested.ends_with(".dby") {
        PathBuf::from(requested)
    } else {
        PathBuf::from(requested.replace('.', std::path::MAIN_SEPARATOR_STR))
    };
    if relative.extension().is_none() {
        relative.set_extension("dby");
    }
    canonical_source(&parent.join(relative), Some(span)).map_err(|mut error| {
        if error.path.is_none() {
            error.path = Some(normalized_path(importer));
        }
        error
    })
}

fn canonical_source(path: &Path, span: Option<Span>) -> NativeResult<PathBuf> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        module_error(
            path,
            span,
            format!("unresolved native module: {:?}", error.kind()),
        )
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| {
        module_error(
            path,
            span,
            format!("cannot inspect native module: {:?}", error.kind()),
        )
    })?;
    if !metadata.is_file() {
        return Err(module_error(path, span, "native module path is not a file"));
    }
    Ok(canonical)
}

fn detect_cycles(modules: &[NativeSourceModule]) -> NativeResult<()> {
    let mut state = vec![0u8; modules.len()];
    let mut stack = Vec::new();
    for module in modules {
        visit_cycle(module.id, modules, &mut state, &mut stack)?;
    }
    Ok(())
}

fn visit_cycle(
    id: usize,
    modules: &[NativeSourceModule],
    state: &mut [u8],
    stack: &mut Vec<usize>,
) -> NativeResult<()> {
    match state[id] {
        2 => return Ok(()),
        1 => {
            let start = stack
                .iter()
                .position(|candidate| *candidate == id)
                .unwrap_or(0);
            let mut names: Vec<&str> = stack[start..]
                .iter()
                .map(|module_id| modules[*module_id].name.as_str())
                .collect();
            names.push(modules[id].name.as_str());
            return Err(Diagnostic::without_span(
                DiagnosticCategory::InvalidModuleImport,
                format!("native import cycle: {}", names.join(" -> ")),
            ));
        }
        _ => {}
    }
    state[id] = 1;
    stack.push(id);
    for import in &modules[id].imports {
        visit_cycle(import.target, modules, state, stack)?;
    }
    stack.pop();
    state[id] = 2;
    Ok(())
}

fn module_error(path: &Path, span: Option<Span>, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(DiagnosticCategory::InvalidModuleImport, message, span)
        .with_path(normalized_path(path))
}

fn normalized_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dbyte-native-modules-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, source: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, source).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn graph_order_and_alias_order_are_deterministic() {
        let directory = TestDirectory::new();
        directory.write("zeta.dby", "pub fn z() -> int:\n    return 1\n");
        directory.write("alpha.dby", "pub fn a() -> int:\n    return 2\n");
        let root = directory.write(
            "kernel.dby",
            "import \"zeta.dby\" as z\nimport \"alpha.dby\" as a\nfn kernel_main() -> int:\n    return a.a() + z.z()\n",
        );
        let first = load_native_module_graph(&root).unwrap();
        let second = load_native_module_graph(&root).unwrap();
        let first_names: Vec<&str> = first
            .modules
            .iter()
            .map(|module| module.name.as_str())
            .collect();
        let second_names: Vec<&str> = second
            .modules
            .iter()
            .map(|module| module.name.as_str())
            .collect();
        assert_eq!(first_names, second_names);
        let aliases: Vec<&str> = first
            .root_module()
            .imports
            .iter()
            .map(|import| import.alias.as_str())
            .collect();
        assert_eq!(aliases, ["a", "z"]);
    }

    #[test]
    fn unresolved_import_has_category_path_and_span() {
        let directory = TestDirectory::new();
        let root = directory.write(
            "kernel.dby",
            "import \"missing.dby\" as missing\nfn kernel_main() -> int:\n    return 0\n",
        );
        let error = load_native_module_graph(&root).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InvalidModuleImport);
        assert!(error.path.is_some());
        assert!(error.span.is_some());
    }

    #[test]
    fn cycles_are_rejected_with_stable_path() {
        let directory = TestDirectory::new();
        let first = directory.write(
            "first.dby",
            "import \"second.dby\" as second\nfn kernel_main() -> int:\n    return 0\n",
        );
        directory.write(
            "second.dby",
            "import \"first.dby\" as first\npub fn value() -> int:\n    return 1\n",
        );
        let error = load_native_module_graph(&first).unwrap_err();
        assert_eq!(error.category, DiagnosticCategory::InvalidModuleImport);
        assert!(error.message.contains("first -> second -> first"));
    }

    #[test]
    fn duplicate_alias_and_network_imports_are_rejected() {
        let directory = TestDirectory::new();
        directory.write("one.dby", "pub fn one() -> int:\n    return 1\n");
        let duplicate = directory.write(
            "duplicate.dby",
            "import \"one.dby\" as same\nimport \"one.dby\" as same\nfn kernel_main() -> int:\n    return 0\n",
        );
        assert_eq!(
            load_native_module_graph(&duplicate).unwrap_err().category,
            DiagnosticCategory::InvalidModuleImport
        );

        let network = directory.write(
            "network.dby",
            "import \"https://example.com/code.dby\" as code\nfn kernel_main() -> int:\n    return 0\n",
        );
        assert!(load_native_module_graph(&network)
            .unwrap_err()
            .message
            .contains("network"));
    }

    #[test]
    fn real_multimodule_kernel_has_five_meaningful_modules() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let graph = load_native_module_graph(&root).unwrap();
        let names: BTreeSet<&str> = graph
            .modules
            .iter()
            .map(|module| module.name.as_str())
            .collect();
        assert_eq!(
            names,
            BTreeSet::from(["arch_x86", "kernel", "memory", "numbers", "serial"])
        );
        assert_eq!(graph.root_module().name, "kernel");
        assert_eq!(graph.root_module().imports.len(), 3);
        assert!(graph
            .modules
            .iter()
            .all(|module| !module.program.stmts.is_empty()));
    }
}
