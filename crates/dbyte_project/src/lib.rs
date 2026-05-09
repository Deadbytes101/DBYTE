use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "Dbyte.toml";

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub package: Package,
}

#[derive(Debug, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub entry: String,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub entry_path: PathBuf,
}

#[derive(Debug, Clone)]
pub enum ProjectError {
    ManifestNotFound,
    ManifestReadFailed(PathBuf),
    ManifestParseFailed(PathBuf),
    MissingPackage,
    MissingName,
    MissingVersion,
    MissingEntry,
    EntryNotFound(PathBuf),
    ProjectAlreadyExists(PathBuf),
    ProjectCreateFailed(PathBuf),
}

impl std::fmt::Display for ProjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectError::ManifestNotFound => write!(f, "Dbyte.toml not found"),
            ProjectError::ManifestReadFailed(_) => write!(f, "failed to read Dbyte.toml"),
            ProjectError::ManifestParseFailed(_) => write!(f, "failed to parse Dbyte.toml"),
            ProjectError::MissingPackage => write!(f, "package not found"),
            ProjectError::MissingName => write!(f, "package.name not found"),
            ProjectError::MissingVersion => write!(f, "package.version not found"),
            ProjectError::MissingEntry => write!(f, "package.entry not found"),
            ProjectError::EntryNotFound(path) => {
                write!(f, "entry file not found: {}", path.display())
            }
            ProjectError::ProjectAlreadyExists(path) => {
                write!(f, "project already exists: {}", path.display())
            }
            ProjectError::ProjectCreateFailed(path) => {
                write!(f, "failed to create project: {}", path.display())
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawManifest {
    package: Option<RawPackage>,
}

#[derive(Debug, Deserialize)]
struct RawPackage {
    name: Option<String>,
    version: Option<String>,
    entry: Option<String>,
}

pub fn find_project_root(start_dir: &Path) -> Result<PathBuf, ProjectError> {
    let mut current = start_dir
        .canonicalize()
        .map_err(|_| ProjectError::ManifestNotFound)?;

    loop {
        if current.join(MANIFEST_FILE).is_file() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(ProjectError::ManifestNotFound);
        }
    }
}

pub fn load_manifest(root: &Path) -> Result<Manifest, ProjectError> {
    let manifest_path = root.join(MANIFEST_FILE);
    let src = std::fs::read_to_string(&manifest_path)
        .map_err(|_| ProjectError::ManifestReadFailed(manifest_path.clone()))?;
    let raw: RawManifest = toml::from_str(&src)
        .map_err(|_| ProjectError::ManifestParseFailed(manifest_path.clone()))?;
    let package = raw.package.ok_or(ProjectError::MissingPackage)?;
    let name = required_field(package.name, ProjectError::MissingName)?;
    let version = required_field(package.version, ProjectError::MissingVersion)?;
    let entry = required_field(package.entry, ProjectError::MissingEntry)?;
    Ok(Manifest {
        package: Package {
            name,
            version,
            entry,
        },
    })
}

pub fn load_project(start_dir: &Path) -> Result<Project, ProjectError> {
    let root = find_project_root(start_dir)?;
    let manifest_path = root.join(MANIFEST_FILE);
    let manifest = load_manifest(&root)?;
    let entry_path = root.join(&manifest.package.entry);
    if !entry_path.is_file() {
        return Err(ProjectError::EntryNotFound(PathBuf::from(
            &manifest.package.entry,
        )));
    }
    Ok(Project {
        root,
        manifest_path,
        entry_path,
    })
}

pub fn create_project(parent_dir: &Path, name: &str) -> Result<(), ProjectError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ProjectError::MissingName);
    }

    let root = parent_dir.join(name);
    if root.exists() {
        return Err(ProjectError::ProjectAlreadyExists(root));
    }

    std::fs::create_dir_all(root.join("src"))
        .map_err(|_| ProjectError::ProjectCreateFailed(root.clone()))?;
    std::fs::create_dir_all(root.join("tests"))
        .map_err(|_| ProjectError::ProjectCreateFailed(root.clone()))?;

    let manifest = format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nentry = \"src/main.dby\"\n",
        name
    );
    std::fs::write(root.join(MANIFEST_FILE), manifest)
        .map_err(|_| ProjectError::ProjectCreateFailed(root.clone()))?;
    std::fs::write(
        root.join("src").join("main.dby"),
        format!("print(\"hello from {}\")\n", name),
    )
    .map_err(|_| ProjectError::ProjectCreateFailed(root.clone()))?;
    std::fs::write(
        root.join("tests").join("smoke.dby"),
        format!("print(\"hello from {}\")\n", name),
    )
    .map_err(|_| ProjectError::ProjectCreateFailed(root.clone()))?;
    std::fs::write(
        root.join("tests").join("smoke.out"),
        format!("hello from {}\n", name),
    )
    .map_err(|_| ProjectError::ProjectCreateFailed(root))?;

    Ok(())
}

fn required_field(value: Option<String>, error: ProjectError) -> Result<String, ProjectError> {
    let value = value.ok_or_else(|| error.clone())?;
    if value.trim().is_empty() {
        Err(match error {
            ProjectError::MissingName => ProjectError::MissingName,
            ProjectError::MissingVersion => ProjectError::MissingVersion,
            ProjectError::MissingEntry => ProjectError::MissingEntry,
            _ => error,
        })
    } else {
        Ok(value)
    }
}
