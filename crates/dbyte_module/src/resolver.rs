use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportTarget {
    File(PathBuf),
    Std(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleError {
    LocalImportWithoutSource {
        requested: String,
    },
    LocalModuleNotFound {
        requested: String,
        resolved_path: PathBuf,
    },
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModuleError::LocalImportWithoutSource { requested } => {
                write!(f, "local import requires a source file path: {}", requested)
            }
            ModuleError::LocalModuleNotFound {
                requested,
                resolved_path,
            } => write!(
                f,
                "local module not found: {} (resolved: {})",
                requested,
                resolved_path.display()
            ),
        }
    }
}

pub fn resolve_import(
    path: &str,
    current_file: Option<&Path>,
) -> Result<ImportTarget, ModuleError> {
    if path.starts_with("std.") {
        return Ok(ImportTarget::Std(path.to_string()));
    }

    let base_dir = current_file.and_then(Path::parent).ok_or_else(|| {
        ModuleError::LocalImportWithoutSource {
            requested: path.to_string(),
        }
    })?;
    let resolved = base_dir.join(path);
    let canonical = resolved
        .canonicalize()
        .map_err(|_| ModuleError::LocalModuleNotFound {
            requested: path.to_string(),
            resolved_path: resolved,
        })?;
    Ok(ImportTarget::File(canonical))
}
