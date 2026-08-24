//! End-to-end systems project compilation through real ELF32 objects and the
//! in-process DByte static linker.

use crate::diagnostics::{Diagnostic, NativeResult};
use crate::linker::{link_object_bytes, link_user_object_bytes, LinkArtifact, LinkInput};
use crate::modules::load_native_module_graph;
use crate::object::write_relocatable;
use crate::systems_lower::lower_systems_module;
use crate::systems_semantic::{
    analyze_systems_objects, analyze_systems_project, analyze_user_objects, analyze_user_project,
    SystemsProfile,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledModuleObject {
    pub module_id: usize,
    pub module_name: String,
    pub source_path: PathBuf,
    pub display_path: String,
    pub bytes: Vec<u8>,
    pub ir_dump: String,
    pub machine_dump: String,
    pub object_dump: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledProjectObjects {
    pub root_module: usize,
    pub modules: Vec<CompiledModuleObject>,
}

impl CompiledProjectObjects {
    pub fn root_object(&self) -> &CompiledModuleObject {
        &self.modules[self.root_module]
    }

    pub fn link_inputs(&self) -> Vec<LinkInput> {
        self.modules
            .iter()
            .map(|module| LinkInput {
                name: format!("{}.o", module.module_name),
                bytes: module.bytes.clone(),
            })
            .collect()
    }

    pub fn combined_ir_dump(&self) -> String {
        let mut dump = String::new();
        for module in &self.modules {
            dump.push_str(&module.ir_dump);
            if !dump.ends_with('\n') {
                dump.push('\n');
            }
        }
        dump
    }

    pub fn combined_machine_dump(&self) -> String {
        let mut dump = String::new();
        for module in &self.modules {
            dump.push_str(&module.machine_dump);
            if !dump.ends_with('\n') {
                dump.push('\n');
            }
            dump.push_str(&module.object_dump);
            if !dump.ends_with('\n') {
                dump.push('\n');
            }
        }
        dump
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemsBuildArtifact {
    pub objects: CompiledProjectObjects,
    pub linked: LinkArtifact,
    pub ir_dump: String,
    pub machine_dump: String,
}

pub fn compile_systems_objects(root: &Path) -> NativeResult<CompiledProjectObjects> {
    compile_systems_objects_with_mode(root, false, SystemsProfile::Kernel)
}

pub fn compile_user_objects(root: &Path) -> NativeResult<CompiledProjectObjects> {
    compile_systems_objects_with_mode(root, false, SystemsProfile::User)
}

fn compile_systems_objects_with_mode(
    root: &Path,
    require_entry: bool,
    profile: SystemsProfile,
) -> NativeResult<CompiledProjectObjects> {
    let graph = load_native_module_graph(root)?;
    let project = match (profile, require_entry) {
        (SystemsProfile::Kernel, true) => analyze_systems_project(&graph)?,
        (SystemsProfile::Kernel, false) => analyze_systems_objects(&graph)?,
        (SystemsProfile::User, true) => analyze_user_project(&graph)?,
        (SystemsProfile::User, false) => analyze_user_objects(&graph)?,
    };
    if project.modules.len() != graph.modules.len() {
        return Err(Diagnostic::without_span(
            crate::diagnostics::DiagnosticCategory::InvalidModuleImport,
            "systems semantic output module count differs from module graph",
        ));
    }

    let mut modules = Vec::with_capacity(graph.modules.len());
    for source_module in &graph.modules {
        let ir_module = project.module(source_module.id).ok_or_else(|| {
            Diagnostic::without_span(
                crate::diagnostics::DiagnosticCategory::InvalidModuleImport,
                format!(
                    "systems semantic output is missing module '{}'",
                    source_module.name
                ),
            )
        })?;
        let lowered = lower_systems_module(ir_module)
            .map_err(|error| attach_path(error, &source_module.display_path))?;
        let object = write_relocatable(&lowered.image)
            .map_err(|error| attach_path(error, &source_module.display_path))?;
        modules.push(CompiledModuleObject {
            module_id: source_module.id,
            module_name: source_module.name.clone(),
            source_path: source_module.path.clone(),
            display_path: source_module.display_path.clone(),
            bytes: object.bytes,
            ir_dump: ir_module.dump(),
            machine_dump: lowered.machine_dump,
            object_dump: object.dump,
        });
    }
    Ok(CompiledProjectObjects {
        root_module: graph.root,
        modules,
    })
}

pub fn link_compiled_project(objects: &CompiledProjectObjects) -> NativeResult<LinkArtifact> {
    link_object_bytes(&objects.link_inputs())
}

pub fn compile_systems_project(root: &Path) -> NativeResult<SystemsBuildArtifact> {
    let objects = compile_systems_objects_with_mode(root, true, SystemsProfile::Kernel)?;
    let ir_dump = objects.combined_ir_dump();
    let mut machine_dump = objects.combined_machine_dump();
    let linked = link_compiled_project(&objects)?;
    machine_dump.push_str(&linked.dump);
    if !machine_dump.ends_with('\n') {
        machine_dump.push('\n');
    }
    Ok(SystemsBuildArtifact {
        objects,
        linked,
        ir_dump,
        machine_dump,
    })
}

pub fn compile_user_project(root: &Path) -> NativeResult<SystemsBuildArtifact> {
    let objects = compile_systems_objects_with_mode(root, true, SystemsProfile::User)?;
    let ir_dump = objects.combined_ir_dump();
    let mut machine_dump = objects.combined_machine_dump();
    let linked = link_user_object_bytes(&objects.link_inputs())?;
    machine_dump.push_str(&linked.dump);
    if !machine_dump.ends_with('\n') {
        machine_dump.push('\n');
    }
    Ok(SystemsBuildArtifact {
        objects,
        linked,
        ir_dump,
        machine_dump,
    })
}

fn attach_path(error: Diagnostic, path: &str) -> Diagnostic {
    if error.path.is_some() {
        error
    } else {
        error.with_path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_project_emits_one_object_per_module_and_links_same_bytes_twice() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let objects = compile_systems_objects(&root).unwrap();
        assert_eq!(objects.modules.len(), 5);
        assert!(objects
            .modules
            .iter()
            .all(|module| module.bytes.starts_with(b"\x7fELF")));
        let first = link_compiled_project(&objects).unwrap();
        let second = link_compiled_project(&objects).unwrap();
        assert_eq!(first.elf, second.elf);
        assert_eq!(first.bin, second.bin);
    }

    #[test]
    fn object_and_integrated_paths_share_exact_object_bytes() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let direct = compile_systems_objects(&root).unwrap();
        let integrated = compile_systems_project(&root).unwrap();
        let direct_bytes: Vec<&[u8]> = direct
            .modules
            .iter()
            .map(|module| module.bytes.as_slice())
            .collect();
        let integrated_bytes: Vec<&[u8]> = integrated
            .objects
            .modules
            .iter()
            .map(|module| module.bytes.as_slice())
            .collect();
        assert_eq!(direct_bytes, integrated_bytes);
    }

    #[test]
    fn standalone_object_does_not_require_entry_but_integrated_project_does() {
        let module = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/arch_x86.dby");
        let objects = compile_systems_objects(&module).unwrap();
        assert_eq!(objects.modules.len(), 1);
        assert!(objects.root_object().bytes.starts_with(b"\x7fELF"));

        let error = compile_systems_project(&module).unwrap_err();
        assert_eq!(
            error.category,
            crate::diagnostics::DiagnosticCategory::InvalidEntryPoint
        );
        assert!(error.message.contains("kernel_main"));
    }
}
