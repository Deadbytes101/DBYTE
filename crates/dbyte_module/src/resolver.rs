use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ImportTarget {
    File(PathBuf),
    Std(String),
}

pub fn resolve_import(path: &str, current_file: Option<&Path>) -> Result<ImportTarget, String> {
    if path.starts_with("std.") {
        return Ok(ImportTarget::Std(path.to_string()));
    }

    let base_dir = current_file
        .and_then(Path::parent)
        .ok_or_else(|| "local imports require a source file path".to_string())?;
    let resolved = base_dir.join(path);
    let canonical = resolved
        .canonicalize()
        .map_err(|e| format!("cannot resolve import `{}`: {}", path, e))?;
    Ok(ImportTarget::File(canonical))
}
