//! Concrete-HIR-only staging seam for DBYTE64 Linux execution.

use crate::elf64::{
    user_executable_layout, write_static_executable, write_user_executable, Elf64Executable,
    ExecutableInput, UserElf64Layout,
};
use crate::object64::{
    write_relocatable, Binding, Contents, Definition, ObjectArtifact, ObjectImage, Relocation,
    RelocationKind, Section, SectionData, Symbol, SymbolKind,
};
use crate::target::{TargetSpec, X86_64_LINUX, X86_64_NONE, X86_64_USER};
use crate::x86_64::{Condition, Encoder, Register, ShiftKind, Width};
use dbyte_ast::{BinOp, CaptureMode, UnaryOp};
use dbyte_hir::{
    CanonicalType, ClosureId, ConcreteHirFunction, ConcreteHirProject, HirBlock, HirCapture,
    HirExpr, HirExprKind, HirPlaceBase, HirProjection, HirStatement, IntegerType, LocalId,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::native_mir::NativeOptimizationLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModernNativeError(pub String);

impl std::fmt::Display for ModernNativeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ModernNativeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModernExecutable {
    pub elf: Elf64Executable,
    pub hir_dump: String,
    pub machine_dump: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModernObject {
    pub object: ObjectArtifact,
    pub hir_dump: String,
    pub machine_dump: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModernObject32 {
    pub object: crate::object::ObjectArtifact,
    pub hir_dump: String,
    pub machine_dump: String,
}

pub fn i686_project_source_paths(root: &Path) -> Result<Vec<PathBuf>, ModernNativeError> {
    dbyte_hir::load_source_project_paths(root).map_err(diagnostics)
}

pub fn compile_mir_i686_project_objects(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<Vec<(PathBuf, ModernObject32)>, ModernNativeError> {
    if target.machine != crate::target::MachineTarget::X86I686
        || target.layout != dbyte_hir::TargetDataLayout::I686
    {
        return Err(error(
            "E_NATIVE_MIR_TARGET_MISMATCH",
            "Native MIR DBYTE32 project compilation requires an i686 target layout",
        ));
    }
    let paths = dbyte_hir::load_source_project_paths(root).map_err(diagnostics)?;
    let input = dbyte_hir::load_source_project(root).map_err(diagnostics)?;
    let analyzed =
        dbyte_hir::analyze_project(&input, target.profile, target.layout).map_err(diagnostics)?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    dbyte_hir::validate_project_profile(&concrete, target.profile)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    let entry_name = if target == crate::target::I686_USER {
        "user_main"
    } else {
        "kernel_main"
    };
    let entry = concrete
        .executable
        .functions
        .iter()
        .find(|function| {
            concrete.symbol_owners[&function.key.symbol] == concrete.root
                && concrete.link_names[&function.key.symbol] == entry_name
        })
        .ok_or_else(|| error("E_ENTRY_MISSING", &format!("missing `{entry_name}`")))?;
    validate_i686_entry(&concrete, entry, entry_name, target)?;

    let mut executable = concrete.executable.clone();
    for function in &mut executable.functions {
        if concrete.symbol_owners[&function.key.symbol] == concrete.root
            && concrete.link_names[&function.key.symbol] == entry_name
        {
            function.symbol_name = entry_name.into();
        }
    }
    let full_mir = crate::native_mir_lower::lower_program_to_native_mir_with_link_names(
        &executable,
        Some(&concrete.link_names),
    )
    .map_err(|failure| error(failure.code, &failure.message))?;

    let mut objects = Vec::with_capacity(paths.len());
    for (index, path) in paths.into_iter().enumerate() {
        let module = dbyte_hir::ModuleId(
            u32::try_from(index)
                .map_err(|_| error("E_MODULE_LIMIT", "module index exceeds u32"))?,
        );
        let mut mir = full_mir.clone();
        retain_module_mir_definitions(&concrete, module, &mut mir);
        let mut local = i686_local_object_symbols_for_module(&concrete, module, &mir);
        if module == concrete.root {
            local.remove(entry_name);
        }
        let mut metadata = i686_modern_metadata_markers_for_module(&analyzed, module);
        if target == crate::target::I686_USER {
            metadata.push(format!("{}user", crate::object::PROFILE_METADATA_PREFIX));
        }
        let selected = crate::native_select_i686::select_project_object_with_metadata_and_locals(
            &mir, level, &metadata, &local,
        )
        .map_err(|failure| error(failure.code, &failure.message))?;
        objects.push((
            path,
            ModernObject32 {
                object: selected.object,
                hir_dump: selected.mir_dump,
                machine_dump: selected.machine_dump,
            },
        ));
    }
    Ok(objects)
}

fn retain_module_mir_definitions(
    project: &ConcreteHirProject,
    module: dbyte_hir::ModuleId,
    mir: &mut crate::native_mir::NativeMirProject,
) {
    let mut module_instances = project
        .executable
        .functions
        .iter()
        .filter(|function| {
            let owner = if function.key.arguments.is_empty() {
                project.symbol_owners[&function.key.symbol]
            } else {
                project.root
            };
            owner == module
        })
        .map(|function| function.instance)
        .collect::<BTreeSet<_>>();

    let private_by_symbol = mir
        .functions
        .iter()
        .filter_map(|function| {
            let source = project
                .executable
                .functions
                .iter()
                .find(|source| source.instance == function.instance)?;
            (!source.public && source.kind != dbyte_ast::FunctionKind::Drop)
                .then_some((function.symbol.clone(), function.instance))
        })
        .collect::<BTreeMap<_, _>>();
    loop {
        let required = mir
            .functions
            .iter()
            .filter(|function| module_instances.contains(&function.instance))
            .flat_map(|function| function.blocks.iter())
            .flat_map(|block| block.instructions.iter())
            .filter_map(|instruction| match &instruction.instruction {
                crate::native_mir::NativeMirInst::Call {
                    callee: crate::native_mir::NativeCallee::Direct { symbol },
                    ..
                } => private_by_symbol.get(symbol).copied(),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        let before = module_instances.len();
        module_instances.extend(required);
        if module_instances.len() == before {
            break;
        }
    }
    mir.functions
        .retain(|function| module_instances.contains(&function.instance));

    let private_globals = project
        .executable
        .globals
        .iter()
        .filter(|global| !global.public)
        .map(|global| global.name.clone())
        .collect::<BTreeSet<_>>();
    let mut module_globals = project
        .executable
        .globals
        .iter()
        .filter(|global| project.symbol_owners[&global.symbol] == module)
        .map(|global| global.name.clone())
        .collect::<BTreeSet<_>>();
    for function in &mir.functions {
        for block in &function.blocks {
            for instruction in &block.instructions {
                if let crate::native_mir::NativeMirInst::GlobalAddress { symbol, .. } =
                    &instruction.instruction
                {
                    if symbol.starts_with("__dbyte_string_") || private_globals.contains(symbol) {
                        module_globals.insert(symbol.clone());
                    }
                }
            }
        }
    }
    mir.globals
        .retain(|global| module_globals.contains(&global.symbol));
}

fn i686_modern_metadata_markers_for_module(
    project: &dbyte_hir::CanonicalHirProject,
    module: dbyte_hir::ModuleId,
) -> Vec<String> {
    let mut markers = crate::systems_compiler::object_metadata_markers_for_module(project, module)
        .into_iter()
        .filter(|marker| !marker.starts_with(crate::object::DROP_METADATA_PREFIX))
        .collect::<Vec<_>>();
    for definition in &project.executable.semantic.definitions {
        if !definition.public || project.symbol_owners.get(&definition.symbol) != Some(&module) {
            continue;
        }
        let Some(drop_function) = project
            .executable
            .semantic
            .functions
            .iter()
            .find(|function| {
                function.kind == dbyte_ast::FunctionKind::Drop
                    && function.drop_target == Some(definition.symbol)
            })
        else {
            continue;
        };
        markers.push(format!(
            "{}{}:{}:{}:{}",
            crate::object::DROP_METADATA_PREFIX,
            definition.name,
            drop_function.name,
            definition.symbol.0,
            definition.generic_params.len()
        ));
    }
    markers.sort();
    markers.dedup();
    markers
}

fn i686_modern_metadata_markers(project: &dbyte_hir::CanonicalHirProject) -> Vec<String> {
    let mut markers = Vec::new();
    for module in &project.module_order {
        markers.extend(i686_modern_metadata_markers_for_module(project, *module));
    }
    markers.sort();
    markers.dedup();
    markers
}

fn i686_local_object_symbols_for_module(
    project: &ConcreteHirProject,
    _module: dbyte_hir::ModuleId,
    mir: &crate::native_mir::NativeMirProject,
) -> BTreeSet<String> {
    let mut local = BTreeSet::new();
    for function in &mir.functions {
        if function.symbol.starts_with("__dbyte_closure_") {
            local.insert(function.symbol.clone());
            continue;
        }
        if let Some(source) = project
            .executable
            .functions
            .iter()
            .find(|source| source.instance == function.instance)
        {
            if source.kind != dbyte_ast::FunctionKind::Drop
                && (!source.public || !source.key.arguments.is_empty())
            {
                local.insert(function.symbol.clone());
            }
        }
    }

    let private_globals = project
        .executable
        .globals
        .iter()
        .filter(|global| !global.public)
        .map(|global| global.name.clone())
        .collect::<BTreeSet<_>>();
    for global in &mir.globals {
        if global.symbol.starts_with("__dbyte_string_") || private_globals.contains(&global.symbol)
        {
            local.insert(global.symbol.clone());
        }
    }
    local
}

pub fn compile_mir_i686_object_source(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<ModernObject32, ModernNativeError> {
    compile_mir_i686_object_source_inner(root, target, level, false)
}

pub fn compile_mir_i686_executable_object_source(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<ModernObject32, ModernNativeError> {
    compile_mir_i686_object_source_inner(root, target, level, true)
}

fn compile_mir_i686_object_source_inner(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
    require_entry: bool,
) -> Result<ModernObject32, ModernNativeError> {
    if target.machine != crate::target::MachineTarget::X86I686
        || target.layout != dbyte_hir::TargetDataLayout::I686
    {
        return Err(error(
            "E_NATIVE_MIR_TARGET_MISMATCH",
            "Native MIR DBYTE32 compilation requires an i686 target layout",
        ));
    }
    let input = dbyte_hir::load_source_project(root).map_err(diagnostics)?;
    let analyzed =
        dbyte_hir::analyze_project(&input, target.profile, target.layout).map_err(diagnostics)?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    dbyte_hir::validate_project_profile(&concrete, target.profile)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    let entry_name = if target == crate::target::I686_USER {
        "user_main"
    } else {
        "kernel_main"
    };
    let entry = concrete.executable.functions.iter().find(|function| {
        concrete.symbol_owners[&function.key.symbol] == concrete.root
            && concrete.link_names[&function.key.symbol] == entry_name
    });
    if let Some(entry) = entry {
        validate_i686_entry(&concrete, entry, entry_name, target)?;
    } else if require_entry {
        return Err(error("E_ENTRY_MISSING", &format!("missing `{entry_name}`")));
    }
    let mut metadata_symbols = if require_entry {
        i686_modern_metadata_markers(&analyzed)
    } else {
        i686_modern_metadata_markers_for_module(&analyzed, concrete.root)
    };
    if target == crate::target::I686_USER {
        metadata_symbols.push(format!("{}user", crate::object::PROFILE_METADATA_PREFIX));
    }
    let mut executable = concrete.executable.clone();
    if entry.is_some() {
        for function in &mut executable.functions {
            if concrete.symbol_owners[&function.key.symbol] == concrete.root
                && concrete.link_names[&function.key.symbol] == entry_name
            {
                function.symbol_name = entry_name.into();
            }
        }
    }
    let mut mir = crate::native_mir_lower::lower_program_to_native_mir_with_link_names(
        &executable,
        Some(&concrete.link_names),
    )
    .map_err(|failure| error(failure.code, &failure.message))?;
    let local_symbols = if require_entry {
        BTreeSet::new()
    } else {
        retain_module_mir_definitions(&concrete, concrete.root, &mut mir);
        let mut local = i686_local_object_symbols_for_module(&concrete, concrete.root, &mir);
        if entry.is_some() {
            local.remove(entry_name);
        }
        local
    };
    let selected = crate::native_select_i686::select_project_object_with_metadata_and_locals(
        &mir,
        level,
        &metadata_symbols,
        &local_symbols,
    )
    .map_err(|failure| error(failure.code, &failure.message))?;
    Ok(ModernObject32 {
        object: selected.object,
        hir_dump: selected.mir_dump,
        machine_dump: selected.machine_dump,
    })
}

pub fn compile_source(
    root: &Path,
    target: TargetSpec,
) -> Result<ModernExecutable, ModernNativeError> {
    let input = dbyte_hir::load_source_project(root).map_err(diagnostics)?;
    let analyzed =
        dbyte_hir::analyze_project(&input, target.profile, target.layout).map_err(diagnostics)?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    dbyte_hir::validate_project_profile(&concrete, target.profile)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    compile_concrete(&concrete, target)
}

pub fn compile_object_source(
    root: &Path,
    target: TargetSpec,
) -> Result<ModernObject, ModernNativeError> {
    let input = dbyte_hir::load_source_project(root).map_err(diagnostics)?;
    let analyzed =
        dbyte_hir::analyze_project(&input, target.profile, target.layout).map_err(diagnostics)?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    dbyte_hir::validate_project_profile(&concrete, target.profile)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    compile_concrete_object(&concrete, target)
}

/// Compile a DBYTE64 source project through the production Native MIR seam.
///
/// The adapter consumes only validated Concrete HIR. It never falls back to
/// the legacy direct-HIR encoder when MIR representation is unsupported.
pub fn compile_mir_object_source(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<ModernObject, ModernNativeError> {
    compile_mir_object_source_inner(root, target, level, false)
}

pub fn compile_mir_executable_object_source(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<ModernObject, ModernNativeError> {
    compile_mir_object_source_inner(root, target, level, true)
}

fn compile_mir_object_source_inner(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
    require_entry: bool,
) -> Result<ModernObject, ModernNativeError> {
    let input = dbyte_hir::load_source_project(root).map_err(diagnostics)?;
    let analyzed =
        dbyte_hir::analyze_project(&input, target.profile, target.layout).map_err(diagnostics)?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    dbyte_hir::validate_project_profile(&concrete, target.profile)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    if target.machine != crate::target::MachineTarget::X86_64
        || concrete.executable.semantic.layout != dbyte_hir::TargetDataLayout::X86_64
    {
        return Err(error(
            "E_NATIVE_MIR_TARGET_MISMATCH",
            "Native MIR DBYTE64 compilation requires an x86_64 target layout",
        ));
    }
    let entry_name = if target == X86_64_NONE {
        "kernel_main"
    } else if target == X86_64_USER {
        "user_main"
    } else {
        "main"
    };
    let entry = concrete
        .executable
        .functions
        .iter()
        .find(|function| function.symbol_name == entry_name);
    if let Some(entry) = entry {
        validate_main(&concrete, entry, entry_name, target)?;
    } else if require_entry {
        return Err(error("E_ENTRY_MISSING", &format!("missing `{entry_name}`")));
    }
    let mir = crate::native_mir_lower::lower_program_to_native_mir_with_link_names(
        &concrete.executable,
        Some(&concrete.link_names),
    )
    .map_err(|failure| error(failure.code, &failure.message))?;
    let selected = crate::native_select_x86_64::select_project_object(&mir, target, level)
        .map_err(|failure| error(failure.code, &failure.message))?;
    Ok(ModernObject {
        object: selected.object,
        hir_dump: selected.mir_dump,
        machine_dump: selected.machine_dump,
    })
}

pub fn compile_mir_source(
    root: &Path,
    target: TargetSpec,
    level: NativeOptimizationLevel,
) -> Result<ModernExecutable, ModernNativeError> {
    if !matches!(target, X86_64_LINUX | X86_64_NONE | X86_64_USER) {
        return Err(error(
            "E_NATIVE_MIR_TARGET_UNSUPPORTED",
            "Native MIR executable emission currently supports x86_64 Linux, user, and none",
        ));
    }
    let object = compile_mir_executable_object_source(root, target, level)?;
    let elf = link_object_bytes(
        &[crate::linker64::LinkInput::new(
            "m0000.o",
            object.object.bytes,
        )],
        target,
    )?;
    Ok(ModernExecutable {
        elf,
        hir_dump: object.hir_dump,
        machine_dump: object.machine_dump,
    })
}

pub fn compile_mir_windows_source(
    root: &Path,
    level: NativeOptimizationLevel,
) -> Result<crate::pe64::PeArtifact, ModernNativeError> {
    let object = compile_mir_object_source(root, crate::target::X86_64_WINDOWS, level)?;
    link_windows_object_bytes(&[crate::linker64::LinkInput::new(
        "m0000.obj",
        object.object.bytes,
    )])
}

pub fn compile_linux_source(root: &Path) -> Result<ModernExecutable, ModernNativeError> {
    compile_source(root, X86_64_LINUX)
}

pub fn compile_windows_source(root: &Path) -> Result<crate::pe64::PeArtifact, ModernNativeError> {
    let object = compile_object_source(root, crate::target::X86_64_WINDOWS)?;
    link_windows_object_bytes(&[crate::linker64::LinkInput::new(
        "m0000.obj",
        object.object.bytes,
    )])
}

pub fn link_windows_object_bytes(
    inputs: &[crate::linker64::LinkInput],
) -> Result<crate::pe64::PeArtifact, ModernNativeError> {
    let mut linked_inputs =
        vec![crate::pe64::import_object().map_err(|failure| ModernNativeError(failure.0))?];
    linked_inputs.extend_from_slice(inputs);
    let legacy = crate::pe64::LEGACY_LAYOUT;
    let mut linked = crate::linker64::link_objects(
        &linked_inputs,
        crate::target::X86_64_WINDOWS,
        crate::linker64::SectionAddresses {
            text: crate::pe64::IMAGE_BASE + u64::from(legacy.text_rva),
            rodata: crate::pe64::IMAGE_BASE + u64::from(legacy.rodata_rva),
            data: crate::pe64::IMAGE_BASE + u64::from(legacy.data_rva),
            bss: crate::pe64::IMAGE_BASE + u64::from(legacy.bss_rva),
        },
        "main",
    )
    .map_err(|failure| ModernNativeError(failure.0))?;
    let layout =
        crate::pe64::layout_for(&linked).map_err(|failure| ModernNativeError(failure.0))?;
    if layout != legacy {
        linked_inputs[0] = crate::pe64::import_object_at(layout.rodata_rva)
            .map_err(|failure| ModernNativeError(failure.0))?;
        linked = crate::linker64::link_objects(
            &linked_inputs,
            crate::target::X86_64_WINDOWS,
            crate::linker64::SectionAddresses {
                text: crate::pe64::IMAGE_BASE + u64::from(layout.text_rva),
                rodata: crate::pe64::IMAGE_BASE + u64::from(layout.rodata_rva),
                data: crate::pe64::IMAGE_BASE + u64::from(layout.data_rva),
                bss: crate::pe64::IMAGE_BASE + u64::from(layout.bss_rva),
            },
            "main",
        )
        .map_err(|failure| ModernNativeError(failure.0))?;
    }
    crate::pe64::write_console_image_at(&linked, layout)
        .map_err(|failure| ModernNativeError(failure.0))
}

pub fn compile_concrete_linux(
    project: &ConcreteHirProject,
) -> Result<ModernExecutable, ModernNativeError> {
    compile_concrete(project, X86_64_LINUX)
}

pub fn compile_concrete(
    project: &ConcreteHirProject,
    target: TargetSpec,
) -> Result<ModernExecutable, ModernNativeError> {
    if project.executable.semantic.layout != dbyte_hir::TargetDataLayout::X86_64
        || project.executable.semantic.profile != target.profile
        || target.machine != crate::target::MachineTarget::X86_64
    {
        return Err(error(
            "E_MODERN_TARGET_MISMATCH",
            "DBYTE64 lowering requires the requested X86_64 layout and profile",
        ));
    }
    if !matches!(target, X86_64_LINUX | X86_64_NONE) {
        return Err(error(
            "E_MODERN_TARGET_UNSUPPORTED",
            "DBYTE64 staging lowerer currently supports Linux and none targets",
        ));
    }
    let entry_name = if target == X86_64_NONE {
        "kernel_main"
    } else {
        "main"
    };
    let main = project
        .executable
        .functions
        .iter()
        .find(|function| function.symbol_name == entry_name)
        .ok_or_else(|| {
            error(
                "E_ENTRY_MISSING",
                &format!("{} requires `pub fn {entry_name}()`", target.triple),
            )
        })?;
    let entry_arity = validate_main(project, main, entry_name, target)?;
    let lowered = lower_project(project)?;
    let object = build_object(target, project, lowered)?;
    let inputs = vec![crate::linker64::LinkInput::new("m0000.o", object.bytes)];

    if target == X86_64_NONE {
        let linked = link_sections(
            &inputs,
            target,
            entry_name,
            crate::multiboot2::DBYTE_TEXT_ADDRESS,
        )?;
        let entry_offset = linked
            .entry
            .checked_sub(crate::multiboot2::DBYTE_TEXT_ADDRESS)
            .ok_or_else(|| error("E_LINK64_ENTRY", "entry precedes linked text"))?;
        let image = crate::multiboot2::wrap_sections(
            &linked.text,
            &linked.rodata,
            &linked.data,
            linked.bss_size,
            entry_offset,
        )
        .map_err(|error| ModernNativeError(error.0))?;
        let info = crate::elf64::inspect(&image.elf).map_err(|error| ModernNativeError(error.0))?;
        return Ok(ModernExecutable {
            hir_dump: format!(
                "profile {}\nlayout pointer=8 usize=8\nentry {}\n",
                target.triple, main.symbol_name
            ),
            machine_dump: format!(
                "DBYTE64 multiboot2 bytes={} entry=0x{:016x} payload=0x{:016x}\n",
                image.elf.len(),
                image.entry,
                image.payload_entry
            ),
            elf: Elf64Executable {
                bytes: image.elf,
                layout: crate::elf64::Elf64Layout {
                    entry: info.entry,
                    text_address: image.payload_entry,
                    rodata_address: 0,
                    data_address: 0,
                    bss_address: 0,
                    file_size: 0,
                },
            },
        });
    }

    let mut linked_inputs = vec![linux_start_object(target, entry_name, entry_arity)?];
    linked_inputs.extend(inputs);
    let linked = link_sections(&linked_inputs, target, "_start", 0x0040_1000)?;
    let entry_offset = linked
        .entry
        .checked_sub(0x0040_1000)
        .ok_or_else(|| error("E_LINK64_ENTRY", "Linux entry precedes linked text"))?;
    let elf = write_static_executable(&ExecutableInput {
        text: linked.text,
        rodata: linked.rodata,
        data: linked.data,
        bss_size: linked.bss_size,
        entry_offset,
    })
    .map_err(|error| ModernNativeError(error.0))?;
    Ok(ModernExecutable {
        hir_dump: format!(
            "profile x86_64-dbyte-linux\nlayout pointer=8 usize=8\nentry {}\n",
            main.symbol_name
        ),
        machine_dump: format!(
            "DBYTE64 text={} entry=0x{:016x} main_offset={}\n",
            elf.bytes.len(),
            elf.layout.entry,
            linked.symbols[entry_name] - 0x0040_1000
        ),
        elf,
    })
}

pub fn link_object_bytes(
    inputs: &[crate::linker64::LinkInput],
    target: TargetSpec,
) -> Result<Elf64Executable, ModernNativeError> {
    match target {
        X86_64_NONE => {
            let linked = link_sections(
                inputs,
                target,
                "kernel_main",
                crate::multiboot2::DBYTE_TEXT_ADDRESS,
            )?;
            let entry_offset = linked
                .entry
                .checked_sub(crate::multiboot2::DBYTE_TEXT_ADDRESS)
                .ok_or_else(|| error("E_LINK64_ENTRY", "entry precedes linked text"))?;
            let image = crate::multiboot2::wrap_sections(
                &linked.text,
                &linked.rodata,
                &linked.data,
                linked.bss_size,
                entry_offset,
            )
            .map_err(|error| ModernNativeError(error.0))?;
            let info =
                crate::elf64::inspect(&image.elf).map_err(|error| ModernNativeError(error.0))?;
            Ok(Elf64Executable {
                bytes: image.elf,
                layout: crate::elf64::Elf64Layout {
                    entry: info.entry,
                    text_address: image.payload_entry,
                    rodata_address: 0,
                    data_address: 0,
                    bss_address: 0,
                    file_size: 0,
                },
            })
        }
        X86_64_USER => {
            let mut linked_inputs = vec![user_start_object(target, "user_main")?];
            linked_inputs.extend_from_slice(inputs);
            let (linked, layout) = link_user_sections(&linked_inputs, target, "_start")?;
            let entry_offset = linked
                .entry
                .checked_sub(layout.text_address)
                .ok_or_else(|| error("E_LINK64_ENTRY", "user entry precedes linked text"))?;
            write_user_executable(&ExecutableInput {
                text: linked.text,
                rodata: linked.rodata,
                data: linked.data,
                bss_size: linked.bss_size,
                entry_offset,
            })
            .map_err(|error| ModernNativeError(error.0))
        }
        X86_64_LINUX => {
            let inspected = link_sections(inputs, target, "main", 0x0040_1000)?;
            let entry_arity = match inspected.entry_abi {
                Some(crate::object64::EntryAbi::LinuxProcessArguments) => 3,
                Some(crate::object64::EntryAbi::ZeroArguments) | None => 0,
            };
            let mut linked_inputs = vec![linux_start_object(target, "main", entry_arity)?];
            linked_inputs.extend_from_slice(inputs);
            let linked = link_sections(&linked_inputs, target, "_start", 0x0040_1000)?;
            let entry_offset = linked
                .entry
                .checked_sub(0x0040_1000)
                .ok_or_else(|| error("E_LINK64_ENTRY", "Linux entry precedes linked text"))?;
            write_static_executable(&ExecutableInput {
                text: linked.text,
                rodata: linked.rodata,
                data: linked.data,
                bss_size: linked.bss_size,
                entry_offset,
            })
            .map_err(|error| ModernNativeError(error.0))
        }
        _ => Err(error(
            "E_MODERN_TARGET_UNSUPPORTED",
            "ELF64 object linking supports none, user, and Linux targets",
        )),
    }
}

fn link_user_sections(
    inputs: &[crate::linker64::LinkInput],
    target: TargetSpec,
    entry: &str,
) -> Result<(crate::linker64::LinkedSections, UserElf64Layout), ModernNativeError> {
    let sizing = crate::linker64::link_objects(
        inputs,
        target,
        crate::linker64::SectionAddresses {
            text: 0,
            rodata: 0,
            data: 0,
            bss: 0,
        },
        entry,
    )
    .map_err(|error| ModernNativeError(error.0))?;
    let layout = user_executable_layout(
        sizing.text.len() as u64,
        sizing.rodata.len() as u64,
        sizing.data.len() as u64,
        sizing.bss_size,
    )
    .map_err(|error| ModernNativeError(error.0))?;
    let linked = crate::linker64::link_objects(
        inputs,
        target,
        crate::linker64::SectionAddresses {
            text: layout.text_address,
            rodata: layout.rodata_address,
            data: layout.data_address,
            bss: layout.bss_address,
        },
        entry,
    )
    .map_err(|error| ModernNativeError(error.0))?;
    Ok((linked, layout))
}

fn link_sections(
    inputs: &[crate::linker64::LinkInput],
    target: TargetSpec,
    entry: &str,
    text_address: u64,
) -> Result<crate::linker64::LinkedSections, ModernNativeError> {
    // Section sizes do not depend on addresses. The sizing pass lets the final
    // pass apply cross-section relocations against the exact ELF layout.
    let sizing = crate::linker64::link_objects(
        inputs,
        target,
        crate::linker64::SectionAddresses {
            text: 0,
            rodata: 0,
            data: 0,
            bss: 0,
        },
        entry,
    )
    .map_err(|error| ModernNativeError(error.0))?;
    let rodata_address = text_address
        .checked_add(sizing.text.len() as u64)
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "rodata address overflows"))?;
    let rx_end = 0x1000u64
        .checked_add(sizing.text.len() as u64)
        .and_then(|value| value.checked_add(sizing.rodata.len() as u64))
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "RX layout overflows"))?;
    let data_offset = rx_end
        .checked_add(0xfff)
        .map(|value| value & !0xfff)
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "data alignment overflows"))?;
    let image_base = text_address
        .checked_sub(0x1000)
        .ok_or_else(|| error("E_LINK64_LAYOUT", "text address precedes ELF text offset"))?;
    let data_address = image_base
        .checked_add(data_offset)
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "data address overflows"))?;
    let bss_address = data_address
        .checked_add(sizing.data.len() as u64)
        .ok_or_else(|| error("E_LINK64_OVERFLOW", "BSS address overflows"))?;
    crate::linker64::link_objects(
        inputs,
        target,
        crate::linker64::SectionAddresses {
            text: text_address,
            rodata: rodata_address,
            data: data_address,
            bss: bss_address,
        },
        entry,
    )
    .map_err(|error| ModernNativeError(error.0))
}

fn user_start_object(
    target: TargetSpec,
    entry_name: &str,
) -> Result<crate::linker64::LinkInput, ModernNativeError> {
    if target != X86_64_USER {
        return Err(error(
            "E_USER64_TARGET",
            "user start requires x86_64-dbyte-user",
        ));
    }
    let mut start = Encoder::new(64);
    let call_offset = start.position();
    start.call_relative(0).map_err(encode_error)?;
    start
        .mov_reg(Register::Rdx, Register::Rax, Width::Qword)
        .map_err(encode_error)?;
    start.mov_imm64(Register::Rax, 0).map_err(encode_error)?;
    start.software_interrupt(0x80).map_err(encode_error)?;
    start.ud2().map_err(encode_error)?;
    let text = start.into_bytes();
    let artifact = write_relocatable(&ObjectImage {
        target,
        entry_abi: None,
        sections: std::collections::BTreeMap::from([(
            Section::Text,
            SectionData {
                contents: Contents::Bytes(text.clone()),
                alignment: 16,
            },
        )]),
        symbols: vec![
            Symbol {
                name: "_start".into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Defined {
                    section: Section::Text,
                    value: 0,
                },
                size: text.len() as u64,
            },
            Symbol {
                name: entry_name.into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Undefined,
                size: 0,
            },
        ],
        relocations: vec![Relocation {
            section: Section::Text,
            offset: call_offset as u64 + 1,
            symbol: entry_name.into(),
            kind: RelocationKind::Plt32,
            addend: -4,
        }],
    })
    .map_err(|error| ModernNativeError(error.0))?;
    Ok(crate::linker64::LinkInput::new(
        "__dbyte_user_start.o",
        artifact.bytes,
    ))
}

fn linux_start_object(
    target: TargetSpec,
    entry_name: &str,
    entry_arity: usize,
) -> Result<crate::linker64::LinkInput, ModernNativeError> {
    let mut start = Encoder::new(160);
    if entry_arity == 3 {
        // Linux initial stack: argc, argv[0..argc], NULL, envp[0..]. Map it
        // into the DBYTE64 scalar slots RAX/RDX/RCX without host assistance.
        start
            .mov_reg_from_base_disp32(Register::Rax, Register::Rsp, 0, Width::Qword)
            .map_err(encode_error)?;
        start
            .lea_base_disp32(Register::Rdx, Register::Rsp, 8)
            .map_err(encode_error)?;
        start
            .mov_reg(Register::R8, Register::Rax, Width::Qword)
            .map_err(encode_error)?;
        start.mov_imm64(Register::Rcx, 3).map_err(encode_error)?;
        start
            .shift_reg_by_cl(Register::R8, ShiftKind::Left, Width::Qword)
            .map_err(encode_error)?;
        start
            .mov_reg(Register::Rcx, Register::Rdx, Width::Qword)
            .map_err(encode_error)?;
        start
            .add_reg(Register::Rcx, Register::R8, Width::Qword)
            .map_err(encode_error)?;
        start.add_imm32(Register::Rcx, 8).map_err(encode_error)?;
    }
    let call_offset = start.position();
    start.call_relative(0).map_err(encode_error)?;
    start
        .mov_reg(Register::Rdi, Register::Rax, Width::Qword)
        .map_err(encode_error)?;
    start.mov_imm64(Register::Rax, 60).map_err(encode_error)?;
    start.syscall().map_err(encode_error)?;
    let text = start.into_bytes();
    let artifact = write_relocatable(&ObjectImage {
        target,
        entry_abi: None,
        sections: std::collections::BTreeMap::from([(
            Section::Text,
            SectionData {
                contents: Contents::Bytes(text.clone()),
                alignment: 16,
            },
        )]),
        symbols: vec![
            Symbol {
                name: "_start".into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Defined {
                    section: Section::Text,
                    value: 0,
                },
                size: text.len() as u64,
            },
            Symbol {
                name: entry_name.into(),
                binding: Binding::Global,
                kind: SymbolKind::Function,
                definition: Definition::Undefined,
                size: 0,
            },
        ],
        relocations: vec![Relocation {
            section: Section::Text,
            offset: call_offset as u64 + 1,
            symbol: entry_name.into(),
            kind: RelocationKind::Plt32,
            addend: -4,
        }],
    })
    .map_err(|error| ModernNativeError(error.0))?;
    Ok(crate::linker64::LinkInput::new(
        // NUL is impossible in a filesystem path and keeps the synthetic
        // entry first under the linker's canonical name ordering.
        "\0__dbyte_start.o",
        artifact.bytes,
    ))
}

pub fn compile_concrete_object(
    project: &ConcreteHirProject,
    target: TargetSpec,
) -> Result<ModernObject, ModernNativeError> {
    if project.executable.semantic.layout != dbyte_hir::TargetDataLayout::X86_64
        || project.executable.semantic.profile != target.profile
        || target.machine != crate::target::MachineTarget::X86_64
    {
        return Err(error(
            "E_MODERN_TARGET_MISMATCH",
            "DBYTE64 object lowering requires the requested X86_64 layout and profile",
        ));
    }
    let entry_name = if target == X86_64_NONE {
        "kernel_main"
    } else {
        "main"
    };
    let main = project
        .executable
        .functions
        .iter()
        .find(|function| function.symbol_name == entry_name)
        .ok_or_else(|| error("E_ENTRY_MISSING", &format!("missing `{entry_name}`")))?;
    validate_main(project, main, entry_name, target)?;
    let lowered = lower_project(project)?;
    let machine_size = lowered.text.len();
    let object = build_object(target, project, lowered)?;
    Ok(ModernObject {
        hir_dump: format!(
            "profile {}\nlayout pointer=8 usize=8\nentry {}\n",
            target.triple, main.symbol_name
        ),
        machine_dump: format!(
            "DBYTE64 object text={} symbols={} relocations={}\n",
            machine_size, object.symbol_count, object.relocation_count
        ),
        object,
    })
}

fn build_object(
    target: TargetSpec,
    project: &ConcreteHirProject,
    lowered: LoweredProject,
) -> Result<ObjectArtifact, ModernNativeError> {
    let mut sections = std::collections::BTreeMap::from([(
        Section::Text,
        SectionData {
            contents: Contents::Bytes(lowered.text),
            alignment: 16,
        },
    )]);
    if !lowered.rodata.is_empty() {
        sections.insert(
            Section::Rodata,
            SectionData {
                contents: Contents::Bytes(lowered.rodata),
                alignment: 8,
            },
        );
    }
    if !lowered.data.is_empty() {
        sections.insert(
            Section::Data,
            SectionData {
                contents: Contents::Bytes(lowered.data),
                alignment: 16,
            },
        );
    }
    if lowered.bss_size != 0 {
        sections.insert(
            Section::Bss,
            SectionData {
                contents: Contents::Uninitialized(lowered.bss_size),
                alignment: 16,
            },
        );
    }
    let mut symbols = Vec::new();
    for function in &project.executable.functions {
        let link_name = function_link_name(project, function);
        let definition =
            lowered
                .functions
                .get(&link_name)
                .map_or(Definition::Undefined, |(offset, _)| Definition::Defined {
                    section: Section::Text,
                    value: *offset,
                });
        let size = lowered
            .functions
            .get(&link_name)
            .map_or(0, |(_, size)| *size);
        symbols.push(Symbol {
            name: link_name,
            binding: Binding::Global,
            kind: SymbolKind::Function,
            definition,
            size,
        });
    }
    let canonical_function_names = symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for (name, (offset, size)) in &lowered.functions {
        if canonical_function_names.contains(name) {
            continue;
        }
        symbols.push(Symbol {
            name: name.clone(),
            binding: Binding::Local,
            kind: SymbolKind::Function,
            definition: Definition::Defined {
                section: Section::Text,
                value: *offset,
            },
            size: *size,
        });
    }
    for (name, offset, size) in lowered.string_symbols {
        symbols.push(Symbol {
            name,
            binding: Binding::Local,
            kind: SymbolKind::Object,
            definition: Definition::Defined {
                section: Section::Rodata,
                value: offset,
            },
            size,
        });
    }
    for (_, (name, section, offset, size)) in lowered.global_symbols {
        symbols.push(Symbol {
            name,
            binding: Binding::Global,
            kind: SymbolKind::Object,
            definition: Definition::Defined {
                section,
                value: offset,
            },
            size,
        });
    }
    let defined_names = symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let external_names = lowered
        .relocations
        .iter()
        .map(|relocation| relocation.symbol.clone())
        .filter(|name| !defined_names.contains(name))
        .collect::<std::collections::BTreeSet<_>>();
    for name in external_names {
        symbols.push(Symbol {
            name,
            binding: Binding::Global,
            kind: SymbolKind::Object,
            definition: Definition::Undefined,
            size: 0,
        });
    }
    let entry_name = if target == X86_64_NONE {
        "kernel_main"
    } else {
        "main"
    };
    let entry_abi = project
        .executable
        .functions
        .iter()
        .find(|function| function.symbol_name == entry_name && function.public)
        .map(|function| {
            if function.parameters.len() == 3 {
                crate::object64::EntryAbi::LinuxProcessArguments
            } else {
                crate::object64::EntryAbi::ZeroArguments
            }
        });
    let image = ObjectImage {
        target,
        entry_abi,
        sections,
        symbols,
        relocations: lowered.relocations,
    };
    if target == crate::target::X86_64_WINDOWS {
        crate::coff64::write_object(&image).map_err(|error| ModernNativeError(error.0))
    } else {
        write_relocatable(&image).map_err(|error| ModernNativeError(error.0))
    }
}

#[derive(Debug)]
struct LoweredProject {
    text: Vec<u8>,
    rodata: Vec<u8>,
    data: Vec<u8>,
    bss_size: u64,
    functions: BTreeMap<String, (u64, u64)>,
    global_symbols: BTreeMap<dbyte_hir::SymbolId, (String, Section, u64, u64)>,
    string_symbols: Vec<(String, u64, u64)>,
    relocations: Vec<Relocation>,
}

#[derive(Clone)]
struct ClosureInfo64 {
    id: ClosureId,
    captures: Vec<HirCapture>,
    parameters: Vec<dbyte_hir::HirLocal>,
    result: dbyte_hir::TypeId,
    body: HirBlock,
    parent: ConcreteHirFunction,
    span: dbyte_ast::Span,
}

#[derive(Clone)]
struct CaptureAccess64 {
    offset: u32,
    mode: CaptureMode,
    ty: dbyte_hir::TypeId,
}

#[derive(Clone)]
struct CaptureEnvironment64 {
    slot: i32,
    captures: BTreeMap<LocalId, CaptureAccess64>,
}

fn collect_closures_block64(
    block: &HirBlock,
    parent: &ConcreteHirFunction,
    output: &mut Vec<ClosureInfo64>,
) {
    for statement in &block.statements {
        match statement {
            HirStatement::Let { value, .. } | HirStatement::Expression(value) => {
                collect_closures_expr64(value, parent, output);
            }
            HirStatement::Assign {
                value,
                cleanup_before_commit,
                ..
            } => {
                collect_closures_expr64(value, parent, output);
                if let Some(cleanup) = cleanup_before_commit {
                    collect_closures_block64(cleanup, parent, output);
                }
            }
            HirStatement::Return { value, cleanup, .. } => {
                if let Some(value) = value {
                    collect_closures_expr64(value, parent, output);
                }
                for cleanup in cleanup {
                    collect_closures_block64(cleanup, parent, output);
                }
            }
            HirStatement::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                collect_closures_expr64(condition, parent, output);
                collect_closures_block64(then_block, parent, output);
                if let Some(else_block) = else_block {
                    collect_closures_block64(else_block, parent, output);
                }
            }
            HirStatement::While {
                condition, body, ..
            } => {
                collect_closures_expr64(condition, parent, output);
                collect_closures_block64(body, parent, output);
            }
            HirStatement::For { iterable, body, .. } => {
                collect_closures_expr64(iterable, parent, output);
                collect_closures_block64(body, parent, output);
            }
            HirStatement::Match { value, arms, .. } => {
                collect_closures_expr64(value, parent, output);
                for arm in arms {
                    collect_closures_block64(&arm.body, parent, output);
                }
            }
            HirStatement::Defer { body, .. } => collect_closures_block64(body, parent, output),
            HirStatement::Break { cleanup, .. } | HirStatement::Continue { cleanup, .. } => {
                for cleanup in cleanup {
                    collect_closures_block64(cleanup, parent, output);
                }
            }
        }
    }
}

fn collect_closures_expr64(
    expression: &HirExpr,
    parent: &ConcreteHirFunction,
    output: &mut Vec<ClosureInfo64>,
) {
    match &expression.kind {
        HirExprKind::Closure {
            closure,
            captures,
            parameters,
            result,
            body,
        } => {
            output.push(ClosureInfo64 {
                id: *closure,
                captures: captures.clone(),
                parameters: parameters.clone(),
                result: *result,
                body: body.clone(),
                parent: parent.clone(),
                span: expression.span,
            });
            collect_closures_block64(body, parent, output);
        }
        HirExprKind::Binary { left, right, .. } => {
            collect_closures_expr64(left, parent, output);
            collect_closures_expr64(right, parent, output);
        }
        HirExprKind::Unary { value, .. }
        | HirExprKind::Cast { value, .. }
        | HirExprKind::Bitcast { value, .. }
        | HirExprKind::Length { value }
        | HirExprKind::Try { value, .. }
        | HirExprKind::Constant { value, .. } => collect_closures_expr64(value, parent, output),
        HirExprKind::Drop { cleanup, .. } => collect_closures_block64(cleanup, parent, output),
        HirExprKind::DirectCall { arguments, .. }
        | HirExprKind::IntrinsicCall { arguments, .. }
        | HirExprKind::HostCall { arguments, .. }
        | HirExprKind::List(arguments)
        | HirExprKind::Enum {
            payload: arguments, ..
        } => {
            for argument in arguments {
                collect_closures_expr64(argument, parent, output);
            }
        }
        HirExprKind::IndirectCall {
            callee, arguments, ..
        } => {
            collect_closures_expr64(callee, parent, output);
            for argument in arguments {
                collect_closures_expr64(argument, parent, output);
            }
        }
        HirExprKind::Struct { fields, .. } => {
            for (_, value) in fields {
                collect_closures_expr64(value, parent, output);
            }
        }
        HirExprKind::Place(place) | HirExprKind::Borrow { place, .. } => {
            if let HirPlaceBase::Pointer(pointer) = &place.base {
                collect_closures_expr64(pointer, parent, output);
            }
            for projection in &place.projections {
                if let HirProjection::Index { index, .. } = projection {
                    collect_closures_expr64(index, parent, output);
                }
            }
        }
        HirExprKind::FormatString(parts) => {
            for part in parts {
                if let dbyte_hir::HirFormatPart::Value(value) = part {
                    collect_closures_expr64(value, parent, output);
                }
            }
        }
        HirExprKind::Unit
        | HirExprKind::Integer(..)
        | HirExprKind::LayoutOf { .. }
        | HirExprKind::Float(..)
        | HirExprKind::Bool(..)
        | HirExprKind::String(..)
        | HirExprKind::Bytes(..)
        | HirExprKind::Null
        | HirExprKind::Local(..)
        | HirExprKind::Global(..)
        | HirExprKind::Function(..)
        | HirExprKind::InterruptAddress { .. }
        | HirExprKind::TaskAddress { .. } => {}
    }
}

fn closure_environment_layout64(
    project: &ConcreteHirProject,
    captures: &[HirCapture],
) -> Result<(dbyte_hir::Layout, BTreeMap<LocalId, CaptureAccess64>), ModernNativeError> {
    let mut size = 0u32;
    let mut align = 1u32;
    let mut accesses = BTreeMap::new();
    for capture in captures {
        let layout = if capture.mode == CaptureMode::Move {
            dbyte_hir::layout_of(&project.executable.semantic, capture.ty)
                .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?
        } else {
            dbyte_hir::Layout { size: 8, align: 8 }
        };
        size = align_u32(size, layout.align)?;
        accesses.insert(
            capture.local,
            CaptureAccess64 {
                offset: size,
                mode: capture.mode,
                ty: capture.ty,
            },
        );
        size = size
            .checked_add(layout.size)
            .ok_or_else(|| error("E_AMD64_CLOSURE", "closure environment size overflows"))?;
        align = align.max(layout.align);
    }
    size = align_u32(size, align)?;
    Ok((dbyte_hir::Layout { size, align }, accesses))
}

fn lower_project(project: &ConcreteHirProject) -> Result<LoweredProject, ModernNativeError> {
    let mut text = Vec::new();
    let mut rodata = Vec::new();
    let mut data = Vec::new();
    let mut bss_size = 0u64;
    let mut strings = BTreeMap::<Vec<u8>, String>::new();
    let mut string_symbols = Vec::new();
    let mut functions = BTreeMap::new();
    let mut relocations = Vec::new();
    let function_symbols = project
        .executable
        .functions
        .iter()
        .map(|function| (function.key.clone(), function_link_name(project, function)))
        .collect::<BTreeMap<_, _>>();
    let mut closure_infos = Vec::new();
    for function in project
        .executable
        .functions
        .iter()
        .filter(|function| !function.declaration_only)
    {
        collect_closures_block64(&function.body, function, &mut closure_infos);
    }
    closure_infos.sort_by_key(|closure| (closure.parent.instance, closure.id));
    let closure_symbols = closure_infos
        .iter()
        .map(|closure| {
            (
                (closure.parent.instance, closure.id),
                format!(
                    "__dbyte_closure_{}_c{}",
                    closure.parent.symbol_name, closure.id.0
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut global_symbols = BTreeMap::new();
    for global in &project.executable.globals {
        let layout = dbyte_hir::layout_of(&project.executable.semantic, global.ty)
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        let name = project
            .link_names
            .get(&global.symbol)
            .cloned()
            .unwrap_or_else(|| global.name.clone());
        if let Some(initializer) = &global.initializer {
            let aligned = align_u64(data.len() as u64, u64::from(layout.align))? as usize;
            data.resize(aligned, 0);
            let offset = data.len() as u64;
            append_static_initializer(&mut data, initializer, layout.size)?;
            global_symbols.insert(
                global.symbol,
                (name, Section::Data, offset, u64::from(layout.size)),
            );
        } else {
            bss_size = align_u64(bss_size, u64::from(layout.align))?;
            let offset = bss_size;
            bss_size = bss_size
                .checked_add(u64::from(layout.size))
                .ok_or_else(|| error("E_AMD64_GLOBAL", "BSS layout overflows"))?;
            global_symbols.insert(
                global.symbol,
                (name, Section::Bss, offset, u64::from(layout.size)),
            );
        }
    }
    for function in project
        .executable
        .functions
        .iter()
        .filter(|function| !function.declaration_only)
    {
        while text.len() & 15 != 0 {
            text.push(0x90);
        }
        let start = text.len() as u64;
        let mut lowerer = Amd64FunctionLowerer::new(
            project,
            function,
            &function_symbols,
            &closure_symbols,
            &closure_infos,
            &global_symbols,
            &mut rodata,
            &mut strings,
            &mut string_symbols,
        )?;
        lowerer.lower()?;
        let machine = lowerer.encoder.into_bytes();
        for mut relocation in lowerer.relocations {
            relocation.offset = relocation
                .offset
                .checked_add(start)
                .ok_or_else(|| error("E_LINK64_OVERFLOW", "function relocation overflows"))?;
            relocations.push(relocation);
        }
        functions.insert(
            function_link_name(project, function),
            (start, machine.len() as u64),
        );
        text.extend_from_slice(&machine);
    }
    for closure in &closure_infos {
        while text.len() & 15 != 0 {
            text.push(0x90);
        }
        let start = text.len() as u64;
        let mut lowerer = Amd64FunctionLowerer::new_closure(
            project,
            closure,
            &function_symbols,
            &closure_symbols,
            &closure_infos,
            &global_symbols,
            &mut rodata,
            &mut strings,
            &mut string_symbols,
        )?;
        lowerer.lower()?;
        let machine = lowerer.encoder.into_bytes();
        for mut relocation in lowerer.relocations {
            relocation.offset = relocation
                .offset
                .checked_add(start)
                .ok_or_else(|| error("E_LINK64_OVERFLOW", "closure relocation overflows"))?;
            relocations.push(relocation);
        }
        let symbol = closure_symbols[&(closure.parent.instance, closure.id)].clone();
        functions.insert(symbol, (start, machine.len() as u64));
        text.extend_from_slice(&machine);
    }
    Ok(LoweredProject {
        text,
        rodata,
        data,
        bss_size,
        functions,
        global_symbols,
        string_symbols,
        relocations,
    })
}

fn function_link_name(project: &ConcreteHirProject, function: &ConcreteHirFunction) -> String {
    if function.convention == dbyte_ast::CallConvention::C && function.key.arguments.is_empty() {
        project
            .link_names
            .get(&function.key.symbol)
            .cloned()
            .unwrap_or_else(|| function.symbol_name.clone())
    } else {
        function.symbol_name.clone()
    }
}

struct Amd64FunctionLowerer<'a, 's> {
    project: &'a ConcreteHirProject,
    function: ConcreteHirFunction,
    function_symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
    closure_symbols: &'a BTreeMap<(dbyte_hir::FunctionInstanceId, ClosureId), String>,
    global_symbols: &'a BTreeMap<dbyte_hir::SymbolId, (String, Section, u64, u64)>,
    encoder: Encoder,
    slots: BTreeMap<LocalId, i32>,
    argument_slots: Vec<i32>,
    aggregate_argument_slots: Vec<i32>,
    callee_slot: i32,
    return_slot: i32,
    sret_slot: Option<i32>,
    try_slot: Option<i32>,
    relocations: Vec<Relocation>,
    rodata: &'s mut Vec<u8>,
    strings: &'s mut BTreeMap<Vec<u8>, String>,
    string_symbols: &'s mut Vec<(String, u64, u64)>,
    labels: BTreeMap<u32, usize>,
    fixups: Vec<(usize, u32)>,
    next_label: u32,
    loops: Vec<(u32, u32)>,
    closure_env_slots: BTreeMap<ClosureId, i32>,
    capture_env: Option<CaptureEnvironment64>,
}

impl<'a, 's> Amd64FunctionLowerer<'a, 's> {
    fn load_physical(
        &mut self,
        dst: Register,
        base: Register,
        displacement: i32,
        width: Width,
    ) -> Result<(), ModernNativeError> {
        match width {
            Width::Byte | Width::Word => self
                .encoder
                .movzx_reg_from_base_disp32(dst, base, displacement, width)
                .map_err(encode_error),
            Width::Dword | Width::Qword => self
                .encoder
                .mov_reg_from_base_disp32(dst, base, displacement, width)
                .map_err(encode_error),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        project: &'a ConcreteHirProject,
        function: &ConcreteHirFunction,
        function_symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<(dbyte_hir::FunctionInstanceId, ClosureId), String>,
        closure_infos: &[ClosureInfo64],
        global_symbols: &'a BTreeMap<dbyte_hir::SymbolId, (String, Section, u64, u64)>,
        rodata: &'s mut Vec<u8>,
        strings: &'s mut BTreeMap<Vec<u8>, String>,
        string_symbols: &'s mut Vec<(String, u64, u64)>,
    ) -> Result<Self, ModernNativeError> {
        Self::new_inner(
            project,
            function.clone(),
            function_symbols,
            closure_symbols,
            closure_infos,
            global_symbols,
            rodata,
            strings,
            string_symbols,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        project: &'a ConcreteHirProject,
        function: ConcreteHirFunction,
        function_symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<(dbyte_hir::FunctionInstanceId, ClosureId), String>,
        closure_infos: &[ClosureInfo64],
        global_symbols: &'a BTreeMap<dbyte_hir::SymbolId, (String, Section, u64, u64)>,
        rodata: &'s mut Vec<u8>,
        strings: &'s mut BTreeMap<Vec<u8>, String>,
        string_symbols: &'s mut Vec<(String, u64, u64)>,
        capture_layout: Option<BTreeMap<LocalId, CaptureAccess64>>,
    ) -> Result<Self, ModernNativeError> {
        let mut slots = BTreeMap::new();
        let mut frame = 0u32;
        for local in &function.locals {
            let layout = dbyte_hir::layout_of(&project.executable.semantic, local.ty)
                .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
            let allocation = if abi_scalar(&project.executable.semantic, local.ty) {
                8
            } else {
                layout.size
            };
            frame = align_u32(frame, layout.align.max(1))?;
            frame = frame
                .checked_add(allocation)
                .ok_or_else(|| error("E_AMD64_FRAME", "local frame overflows"))?;
            slots.insert(local.id, -(frame as i32));
        }
        let capture_env = if let Some(captures) = capture_layout {
            frame = frame
                .checked_add(8)
                .ok_or_else(|| error("E_AMD64_FRAME", "closure env parameter overflows"))?;
            Some(CaptureEnvironment64 {
                slot: -(frame as i32),
                captures,
            })
        } else {
            None
        };
        let mut closure_env_slots = BTreeMap::new();
        for closure in closure_infos
            .iter()
            .filter(|closure| closure.parent.instance == function.instance)
        {
            let (layout, _) = closure_environment_layout64(project, &closure.captures)?;
            if layout.size == 0 {
                continue;
            }
            frame = align_u32(frame, layout.align)?;
            frame = frame
                .checked_add(layout.size)
                .ok_or_else(|| error("E_AMD64_FRAME", "closure environment overflows"))?;
            closure_env_slots.insert(closure.id, -(frame as i32));
        }
        const MAX_DBYTE64_ARGUMENTS: usize = 16;
        let mut argument_slots = Vec::with_capacity(MAX_DBYTE64_ARGUMENTS);
        for _ in 0..MAX_DBYTE64_ARGUMENTS {
            frame = frame
                .checked_add(8)
                .ok_or_else(|| error("E_AMD64_FRAME", "argument scratch frame overflows"))?;
            argument_slots.push(-(frame as i32));
        }
        frame = frame
            .checked_add(8)
            .ok_or_else(|| error("E_AMD64_FRAME", "callee scratch frame overflows"))?;
        let callee_slot = -(frame as i32);
        frame = frame
            .checked_add(8)
            .ok_or_else(|| error("E_AMD64_FRAME", "return scratch frame overflows"))?;
        let return_slot = -(frame as i32);
        let sret_slot = if abi_scalar(&project.executable.semantic, function.result) {
            None
        } else {
            frame = frame
                .checked_add(8)
                .ok_or_else(|| error("E_AMD64_FRAME", "sret frame slot overflows"))?;
            Some(-(frame as i32))
        };
        let aggregate_types = project.executable.functions.iter().flat_map(|candidate| {
            std::iter::once(candidate.result)
                .chain(candidate.parameters.iter().map(|parameter| parameter.ty))
                .chain(candidate.locals.iter().map(|local| local.ty))
        });
        let aggregate_temporary = aggregate_types
            .filter(|ty| !abi_scalar(&project.executable.semantic, *ty))
            .map(|ty| {
                dbyte_hir::layout_of(&project.executable.semantic, ty)
                    .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .max_by_key(|layout| (layout.size, layout.align));
        let try_slot = if let Some(layout) = aggregate_temporary {
            frame = align_u32(frame, layout.align.max(1))?;
            frame = frame
                .checked_add(layout.size)
                .ok_or_else(|| error("E_AMD64_FRAME", "aggregate temporary overflows"))?;
            Some(-(frame as i32))
        } else {
            None
        };
        let mut aggregate_argument_slots = Vec::with_capacity(MAX_DBYTE64_ARGUMENTS);
        if let Some(layout) = aggregate_temporary {
            for _ in 0..MAX_DBYTE64_ARGUMENTS {
                frame = align_u32(frame, layout.align.max(1))?;
                frame = frame.checked_add(layout.size).ok_or_else(|| {
                    error("E_AMD64_FRAME", "aggregate argument scratch overflows")
                })?;
                aggregate_argument_slots.push(-(frame as i32));
            }
        }
        let frame = (frame + 15) & !15;
        let mut encoder = Encoder::new(4 * 1024 * 1024);
        encoder.push(Register::Rbp).map_err(encode_error)?;
        encoder
            .mov_reg(Register::Rbp, Register::Rsp, Width::Qword)
            .map_err(encode_error)?;
        if frame != 0 {
            encoder
                .sub_imm32(Register::Rsp, frame)
                .map_err(encode_error)?;
        }
        let (registers, shadow): (&[Register], i32) = match function.convention {
            dbyte_ast::CallConvention::DByte => (&dbyte64_argument_registers(), 0),
            dbyte_ast::CallConvention::C => {
                c_argument_registers(project.executable.semantic.profile)?
            }
        };
        if function.parameters.len() > MAX_DBYTE64_ARGUMENTS {
            return Err(unsupported("DBYTE64 function", "more than 16 parameters"));
        }
        let mut register_index = 0usize;
        if let Some(capture_env) = &capture_env {
            let register = registers
                .get(register_index)
                .copied()
                .ok_or_else(|| error("E_AMD64_ABI", "closure env has no argument register"))?;
            encoder
                .mov_base_disp32_from_reg(Register::Rbp, capture_env.slot, register, Width::Qword)
                .map_err(encode_error)?;
            register_index += 1;
        }
        if let Some(sret_slot) = sret_slot {
            let register = registers
                .get(register_index)
                .copied()
                .ok_or_else(|| error("E_AMD64_ABI", "sret ABI has no argument register"))?;
            encoder
                .mov_base_disp32_from_reg(Register::Rbp, sret_slot, register, Width::Qword)
                .map_err(encode_error)?;
            register_index = 1;
        }
        // Snapshot every register argument before aggregate stack copies use
        // RAX as a transfer register. Source-order lowering may place an
        // aggregate before a scalar without consuming a scalar register.
        let register_prefix = register_index;
        for parameter in &function.parameters {
            if !abi_scalar(&project.executable.semantic, parameter.ty) {
                continue;
            }
            if let Some(register) = registers.get(register_index).copied() {
                encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        slots[&parameter.id],
                        register,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
            }
            register_index += 1;
        }

        let mut stack_offset = 16i32
            .checked_add(shadow)
            .ok_or_else(|| error("E_AMD64_FRAME", "shadow-space offset overflows"))?;
        register_index = register_prefix;
        for parameter in &function.parameters {
            let destination = slots[&parameter.id];
            if abi_scalar(&project.executable.semantic, parameter.ty) {
                if registers.get(register_index).is_some() {
                    register_index += 1;
                    continue;
                }
                encoder
                    .mov_reg_from_base_disp32(
                        Register::Rax,
                        Register::Rbp,
                        stack_offset,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        destination,
                        Register::Rax,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                stack_offset = stack_offset
                    .checked_add(8)
                    .ok_or_else(|| error("E_AMD64_FRAME", "stack parameter offset overflows"))?;
                register_index += 1;
            } else {
                if function.convention == dbyte_ast::CallConvention::C {
                    return Err(unsupported("Win64/SysV64 parameter", "aggregate by value"));
                }
                let layout = dbyte_hir::layout_of(&project.executable.semantic, parameter.ty)
                    .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                stack_offset = align_i32(stack_offset, layout.align.max(8))?;
                for (chunk, width) in physical_chunks(layout.size) {
                    let chunk = i32::try_from(chunk)
                        .map_err(|_| error("E_AMD64_FRAME", "aggregate parameter is too large"))?;
                    encoder
                        .mov_reg_from_base_disp32(
                            Register::Rax,
                            Register::Rbp,
                            stack_offset + chunk,
                            width,
                        )
                        .map_err(encode_error)?;
                    encoder
                        .mov_base_disp32_from_reg(
                            Register::Rbp,
                            destination + chunk,
                            Register::Rax,
                            width,
                        )
                        .map_err(encode_error)?;
                }
                let physical_size = align_u32(layout.size, 8)?;
                stack_offset = stack_offset
                    .checked_add(physical_size as i32)
                    .ok_or_else(|| error("E_AMD64_FRAME", "aggregate parameter overflows"))?;
            }
        }
        Ok(Self {
            project,
            function,
            function_symbols,
            closure_symbols,
            global_symbols,
            encoder,
            slots,
            argument_slots,
            aggregate_argument_slots,
            callee_slot,
            return_slot,
            sret_slot,
            try_slot,
            relocations: Vec::new(),
            rodata,
            strings,
            string_symbols,
            labels: BTreeMap::new(),
            fixups: Vec::new(),
            next_label: 0,
            loops: Vec::new(),
            closure_env_slots,
            capture_env,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn new_closure(
        project: &'a ConcreteHirProject,
        closure: &ClosureInfo64,
        function_symbols: &'a BTreeMap<dbyte_hir::FunctionInstanceKey, String>,
        closure_symbols: &'a BTreeMap<(dbyte_hir::FunctionInstanceId, ClosureId), String>,
        _closure_infos: &[ClosureInfo64],
        global_symbols: &'a BTreeMap<dbyte_hir::SymbolId, (String, Section, u64, u64)>,
        rodata: &'s mut Vec<u8>,
        strings: &'s mut BTreeMap<Vec<u8>, String>,
        string_symbols: &'s mut Vec<(String, u64, u64)>,
    ) -> Result<Self, ModernNativeError> {
        let parameter_ids = closure
            .parameters
            .iter()
            .map(|parameter| parameter.id)
            .collect::<std::collections::BTreeSet<_>>();
        let mut locals = closure.parent.locals.clone();
        for local in &mut locals {
            local.parameter = parameter_ids.contains(&local.id);
        }
        let function = ConcreteHirFunction {
            instance: closure.parent.instance,
            key: closure.parent.key.clone(),
            symbol_name: closure_symbols[&(closure.parent.instance, closure.id)].clone(),
            public: false,
            kind: dbyte_ast::FunctionKind::Normal,
            drop_target: None,
            convention: dbyte_ast::CallConvention::DByte,
            interrupt: None,
            declaration_only: false,
            parameters: closure.parameters.clone(),
            result: closure.result,
            locals,
            body: closure.body.clone(),
            span: closure.span,
        };
        let (_, accesses) = closure_environment_layout64(project, &closure.captures)?;
        Self::new_inner(
            project,
            function,
            function_symbols,
            closure_symbols,
            &[],
            global_symbols,
            rodata,
            strings,
            string_symbols,
            Some(accesses),
        )
    }

    fn lower(&mut self) -> Result<(), ModernNativeError> {
        let body = self.function.body.clone();
        self.lower_block(&body)?;
        if !self
            .function
            .body
            .statements
            .iter()
            .any(|statement| matches!(statement, HirStatement::Return { .. }))
        {
            self.encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error)?;
            self.emit_return()?;
        }
        for (offset, label) in self.fixups.clone() {
            let destination = *self
                .labels
                .get(&label)
                .ok_or_else(|| error("E_AMD64_LABEL", "branch references an unbound label"))?;
            self.encoder
                .patch_relative_i32(offset, destination)
                .map_err(encode_error)?;
        }
        Ok(())
    }

    fn lower_block(&mut self, block: &HirBlock) -> Result<(), ModernNativeError> {
        for statement in &block.statements {
            match statement {
                HirStatement::Let { local, value, .. } => {
                    if matches!(value.kind, HirExprKind::Struct { .. }) {
                        self.initialize_struct_local(*local, value)?;
                    } else if matches!(value.kind, HirExprKind::Enum { .. }) {
                        self.initialize_enum_local(*local, value)?;
                    } else if matches!(
                        value.kind,
                        HirExprKind::DirectCall { .. } | HirExprKind::IndirectCall { .. }
                    ) && !abi_scalar(&self.project.executable.semantic, value.ty)
                    {
                        self.lower_aggregate_call_into(*local, value)?;
                    } else if abi_scalar(&self.project.executable.semantic, value.ty) {
                        self.lower_expr(value)?;
                        self.store_local(*local)?;
                    } else {
                        self.lower_expr(value)?;
                        self.copy_from_pointer_to_local(*local, value.ty)?;
                    }
                }
                HirStatement::Assign {
                    place,
                    value,
                    cleanup_before_commit: None,
                    ..
                } if place.projections.is_empty() => {
                    if let dbyte_hir::HirPlaceBase::Local(local) = place.base {
                        if abi_scalar(&self.project.executable.semantic, value.ty) {
                            self.lower_expr(value)?;
                            self.store_local(local)?;
                        } else {
                            self.lower_expr(value)?;
                            self.copy_from_pointer_to_local(local, value.ty)?;
                        }
                    } else {
                        self.lower_place_assignment(place, value)?;
                    }
                }
                HirStatement::Assign {
                    place,
                    value,
                    cleanup_before_commit: None,
                    ..
                } => self.lower_place_assignment(place, value)?,
                HirStatement::Expression(expression) => self.lower_expr(expression)?,
                HirStatement::Return { value, cleanup, .. } => {
                    if let Some(value) = value {
                        if self.sret_slot.is_some() {
                            self.lower_aggregate_return(value)?;
                        } else if matches!(
                            self.project.executable.semantic.type_of(value.ty),
                            CanonicalType::Unit
                        ) {
                            self.encoder
                                .mov_imm64(Register::Rax, 0)
                                .map_err(encode_error)?;
                        } else {
                            self.lower_expr(value)?;
                        }
                    } else {
                        self.encoder
                            .mov_imm64(Register::Rax, 0)
                            .map_err(encode_error)?;
                    }
                    if !cleanup.is_empty() {
                        if self.sret_slot.is_none()
                            && value.as_ref().is_some_and(|value| {
                                !matches!(
                                    self.project.executable.semantic.type_of(value.ty),
                                    CanonicalType::Unit
                                )
                            })
                        {
                            self.encoder
                                .mov_base_disp32_from_reg(
                                    Register::Rbp,
                                    self.return_slot,
                                    Register::Rax,
                                    Width::Qword,
                                )
                                .map_err(encode_error)?;
                        }
                        for block in cleanup {
                            self.lower_block(block)?;
                        }
                        if let Some(sret_slot) = self.sret_slot {
                            self.encoder
                                .mov_reg_from_base_disp32(
                                    Register::Rax,
                                    Register::Rbp,
                                    sret_slot,
                                    Width::Qword,
                                )
                                .map_err(encode_error)?;
                        } else if value.as_ref().is_some_and(|value| {
                            !matches!(
                                self.project.executable.semantic.type_of(value.ty),
                                CanonicalType::Unit
                            )
                        }) {
                            self.encoder
                                .mov_reg_from_base_disp32(
                                    Register::Rax,
                                    Register::Rbp,
                                    self.return_slot,
                                    Width::Qword,
                                )
                                .map_err(encode_error)?;
                        } else {
                            self.encoder
                                .mov_imm64(Register::Rax, 0)
                                .map_err(encode_error)?;
                        }
                    }
                    self.emit_return()?;
                }
                HirStatement::If {
                    condition,
                    then_block,
                    else_block,
                    ..
                } => {
                    let otherwise = self.new_label();
                    self.lower_expr(condition)?;
                    self.encoder
                        .test_reg(Register::Rax, Register::Rax, Width::Qword)
                        .map_err(encode_error)?;
                    self.jump_if(Condition::Equal, otherwise)?;
                    self.lower_block(then_block)?;
                    if let Some(else_block) = else_block {
                        let done = self.new_label();
                        self.jump(done)?;
                        self.bind(otherwise)?;
                        self.lower_block(else_block)?;
                        self.bind(done)?;
                    } else {
                        self.bind(otherwise)?;
                    }
                }
                HirStatement::While {
                    condition, body, ..
                } => {
                    let head = self.new_label();
                    let done = self.new_label();
                    self.bind(head)?;
                    self.lower_expr(condition)?;
                    self.encoder
                        .test_reg(Register::Rax, Register::Rax, Width::Qword)
                        .map_err(encode_error)?;
                    self.jump_if(Condition::Equal, done)?;
                    self.loops.push((head, done));
                    self.lower_block(body)?;
                    self.loops.pop();
                    self.jump(head)?;
                    self.bind(done)?;
                }
                HirStatement::Match { value, arms, .. } => {
                    self.lower_match(value, arms)?;
                }
                HirStatement::Defer { .. } => {}
                HirStatement::Break { cleanup, .. } => {
                    for block in cleanup {
                        self.lower_block(block)?;
                    }
                    let (_, done) = self
                        .loops
                        .last()
                        .copied()
                        .ok_or_else(|| error("E_AMD64_LOOP", "break has no loop"))?;
                    self.jump(done)?;
                }
                HirStatement::Continue { cleanup, .. } => {
                    for block in cleanup {
                        self.lower_block(block)?;
                    }
                    let (head, _) = self
                        .loops
                        .last()
                        .copied()
                        .ok_or_else(|| error("E_AMD64_LOOP", "continue has no loop"))?;
                    self.jump(head)?;
                }
                _ => return Err(unsupported("DBYTE64 function", "control-flow statement")),
            }
        }
        Ok(())
    }

    fn lower_expr(&mut self, expression: &HirExpr) -> Result<(), ModernNativeError> {
        match &expression.kind {
            HirExprKind::Length { value } => {
                match self.project.executable.semantic.type_of(value.ty) {
                    CanonicalType::Array { length, .. } => self
                        .encoder
                        .mov_imm64(Register::Rax, u64::from(*length))
                        .map_err(encode_error),
                    CanonicalType::Slice { .. } | CanonicalType::StringView => {
                        self.lower_expr(value)?;
                        self.load_physical(
                            Register::Rax,
                            Register::Rax,
                            self.project.executable.semantic.layout.pointer_size as i32,
                            Width::Qword,
                        )
                    }
                    _ => Err(unsupported(
                        "DBYTE64 length",
                        "operand without a canonical bounded representation",
                    )),
                }
            }
            HirExprKind::Integer(value) => self
                .encoder
                .mov_imm64(Register::Rax, *value as u64)
                .map_err(encode_error),
            HirExprKind::Bool(value) => self
                .encoder
                .mov_imm64(Register::Rax, u64::from(*value))
                .map_err(encode_error),
            HirExprKind::Null | HirExprKind::Unit => self
                .encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error),
            HirExprKind::Cast { value, .. } | HirExprKind::Constant { value, .. } => {
                self.lower_expr(value)
            }
            HirExprKind::Local(local) => self.load_local(*local),
            HirExprKind::Global(symbol) => {
                self.lower_global_address(*symbol)?;
                if matches!(
                    self.project.executable.semantic.type_of(expression.ty),
                    CanonicalType::Unit
                ) {
                    self.encoder
                        .mov_imm64(Register::Rax, 0)
                        .map_err(encode_error)?;
                } else if abi_scalar(&self.project.executable.semantic, expression.ty) {
                    self.load_physical(
                        Register::Rax,
                        Register::Rax,
                        0,
                        scalar_width(&self.project.executable.semantic, expression.ty)?,
                    )?;
                }
                Ok(())
            }
            HirExprKind::Place(place) => self.lower_place_value(place),
            HirExprKind::Borrow { place, .. } => self.lower_place_address(place),
            HirExprKind::String(bytes) => self.lower_string(bytes, expression.ty),
            HirExprKind::Function(function) => {
                let symbol = self.function_symbols.get(function).ok_or_else(|| {
                    error("E_AMD64_CALL", "function value has no resolved link symbol")
                })?;
                let offset = self
                    .encoder
                    .lea_rip_relative_placeholder(Register::Rax)
                    .map_err(encode_error)?;
                self.relocations.push(Relocation {
                    section: Section::Text,
                    offset: offset as u64,
                    symbol: symbol.clone(),
                    kind: RelocationKind::Pc32,
                    addend: -4,
                });
                Ok(())
            }
            HirExprKind::Struct { .. } => {
                let storage = self.try_slot.ok_or_else(|| {
                    unsupported("DBYTE64 struct expression", "temporary aggregate storage")
                })?;
                self.initialize_struct_at_offset(storage, expression)?;
                self.encoder
                    .lea_base_disp32(Register::Rax, Register::Rbp, storage)
                    .map_err(encode_error)
            }
            HirExprKind::Enum { .. } => {
                let storage = self.try_slot.ok_or_else(|| {
                    unsupported("DBYTE64 enum expression", "temporary aggregate storage")
                })?;
                self.initialize_enum_at_offset(storage, expression)?;
                self.encoder
                    .lea_base_disp32(Register::Rax, Register::Rbp, storage)
                    .map_err(encode_error)
            }
            HirExprKind::Try {
                value,
                ok_variant,
                cleanup,
                ..
            } => self.lower_try(value, *ok_variant, cleanup),
            HirExprKind::Drop { cleanup, .. } => {
                self.lower_block(cleanup)?;
                self.encoder
                    .mov_imm64(Register::Rax, 0)
                    .map_err(encode_error)
            }
            HirExprKind::Unary {
                op: UnaryOp::Neg,
                value,
            } => {
                self.lower_expr(value)?;
                self.encoder
                    .mov_imm64(Register::Rdx, 0)
                    .map_err(encode_error)?;
                self.encoder
                    .sub_reg(Register::Rdx, Register::Rax, Width::Qword)
                    .map_err(encode_error)?;
                self.encoder
                    .mov_reg(Register::Rax, Register::Rdx, Width::Qword)
                    .map_err(encode_error)
            }
            HirExprKind::Unary {
                op: UnaryOp::BitNot,
                value,
            } => {
                self.lower_expr(value)?;
                self.encoder
                    .not_reg(Register::Rax, Width::Qword)
                    .map_err(encode_error)
            }
            HirExprKind::Unary {
                op: UnaryOp::Not,
                value,
            } => {
                self.lower_expr(value)?;
                self.encoder
                    .test_reg(Register::Rax, Register::Rax, Width::Qword)
                    .map_err(encode_error)?;
                self.encoder
                    .set_condition(Condition::Equal, Register::Rax)
                    .map_err(encode_error)?;
                self.encoder
                    .movzx_byte_to_qword(Register::Rax, Register::Rax)
                    .map_err(encode_error)
            }
            HirExprKind::Unary {
                op: UnaryOp::Deref,
                value,
            } => {
                self.lower_expr(value)?;
                if matches!(
                    self.project.executable.semantic.type_of(expression.ty),
                    CanonicalType::Unit
                ) {
                    self.encoder
                        .mov_imm64(Register::Rax, 0)
                        .map_err(encode_error)?;
                } else if abi_scalar(&self.project.executable.semantic, expression.ty) {
                    self.load_physical(
                        Register::Rax,
                        Register::Rax,
                        0,
                        scalar_width(&self.project.executable.semantic, expression.ty)?,
                    )?;
                }
                Ok(())
            }
            HirExprKind::Binary { left, op, right } => {
                self.lower_expr(left)?;
                self.encoder.push(Register::Rax).map_err(encode_error)?;
                self.lower_expr(right)?;
                self.encoder
                    .mov_reg(Register::Rcx, Register::Rax, Width::Qword)
                    .map_err(encode_error)?;
                self.encoder.pop(Register::Rax).map_err(encode_error)?;
                match op {
                    BinOp::Add => self
                        .encoder
                        .add_reg(Register::Rax, Register::Rcx, Width::Qword)
                        .map_err(encode_error),
                    BinOp::Sub => self
                        .encoder
                        .sub_reg(Register::Rax, Register::Rcx, Width::Qword)
                        .map_err(encode_error),
                    BinOp::Mul => self
                        .encoder
                        .imul_reg(Register::Rax, Register::Rcx, Width::Qword)
                        .map_err(encode_error),
                    BinOp::Div | BinOp::Rem => {
                        lower_division(self.project, expression, op.clone(), &mut self.encoder)
                    }
                    BinOp::BitAnd => self
                        .encoder
                        .and_reg(Register::Rax, Register::Rcx, Width::Qword)
                        .map_err(encode_error),
                    BinOp::BitOr => self
                        .encoder
                        .or_reg(Register::Rax, Register::Rcx, Width::Qword)
                        .map_err(encode_error),
                    BinOp::BitXor => self
                        .encoder
                        .xor_reg(Register::Rax, Register::Rcx, Width::Qword)
                        .map_err(encode_error),
                    BinOp::Shl => self
                        .encoder
                        .shift_reg_by_cl(Register::Rax, ShiftKind::Left, Width::Qword)
                        .map_err(encode_error),
                    BinOp::Shr => self
                        .encoder
                        .shift_reg_by_cl(
                            Register::Rax,
                            if is_signed_type(self.project, left.ty) {
                                ShiftKind::ArithmeticRight
                            } else {
                                ShiftKind::LogicalRight
                            },
                            Width::Qword,
                        )
                        .map_err(encode_error),
                    BinOp::EqEq
                    | BinOp::NotEq
                    | BinOp::Lt
                    | BinOp::Gt
                    | BinOp::LtEq
                    | BinOp::GtEq => {
                        self.encoder
                            .cmp_reg(Register::Rax, Register::Rcx, Width::Qword)
                            .map_err(encode_error)?;
                        let signed = is_signed_type(self.project, left.ty);
                        let condition = match (op, signed) {
                            (BinOp::EqEq, _) => Condition::Equal,
                            (BinOp::NotEq, _) => Condition::NotEqual,
                            (BinOp::Lt, true) => Condition::Less,
                            (BinOp::LtEq, true) => Condition::LessOrEqual,
                            (BinOp::Gt, true) => Condition::Greater,
                            (BinOp::GtEq, true) => Condition::GreaterOrEqual,
                            (BinOp::Lt, false) => Condition::Below,
                            (BinOp::LtEq, false) => Condition::BelowOrEqual,
                            (BinOp::Gt, false) => Condition::Above,
                            (BinOp::GtEq, false) => Condition::AboveOrEqual,
                            _ => unreachable!(),
                        };
                        self.encoder
                            .set_condition(condition, Register::Rax)
                            .map_err(encode_error)?;
                        self.encoder
                            .movzx_byte_to_qword(Register::Rax, Register::Rax)
                            .map_err(encode_error)
                    }
                }
            }
            HirExprKind::DirectCall {
                callee,
                arguments,
                convention,
            } => {
                if !abi_scalar(&self.project.executable.semantic, expression.ty) {
                    let storage = self.try_slot.ok_or_else(|| {
                        unsupported("DBYTE64 aggregate call", "temporary result storage")
                    })?;
                    self.lower_aggregate_call_into_offset(storage, expression)?;
                    return self
                        .encoder
                        .lea_base_disp32(Register::Rax, Register::Rbp, storage)
                        .map_err(encode_error);
                }
                let cleanup = self.lower_call_arguments(arguments, *convention)?;
                let symbol = self
                    .function_symbols
                    .get(callee)
                    .ok_or_else(|| error("E_AMD64_CALL", "resolved function has no link symbol"))?;
                let offset = self
                    .encoder
                    .call_relative_placeholder()
                    .map_err(encode_error)?;
                self.relocations.push(Relocation {
                    section: Section::Text,
                    offset: offset as u64,
                    symbol: symbol.clone(),
                    kind: RelocationKind::Plt32,
                    addend: -4,
                });
                if cleanup != 0 {
                    self.encoder
                        .add_imm32(Register::Rsp, cleanup)
                        .map_err(encode_error)?;
                }
                Ok(())
            }
            HirExprKind::IndirectCall {
                callee,
                arguments,
                convention,
            } => {
                let closure_call = matches!(
                    self.project.executable.semantic.type_of(callee.ty),
                    CanonicalType::ClosureShape { .. }
                );
                if !abi_scalar(&self.project.executable.semantic, expression.ty) {
                    let storage = self.try_slot.ok_or_else(|| {
                        unsupported("DBYTE64 aggregate call", "temporary result storage")
                    })?;
                    self.lower_aggregate_call_into_offset(storage, expression)?;
                    return self
                        .encoder
                        .lea_base_disp32(Register::Rax, Register::Rbp, storage)
                        .map_err(encode_error);
                }
                self.lower_expr(callee)?;
                self.encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        self.callee_slot,
                        Register::Rax,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                let cleanup = self.lower_call_arguments_with_hidden(
                    arguments,
                    *convention,
                    None,
                    closure_call,
                )?;
                self.encoder
                    .mov_reg_from_base_disp32(
                        Register::R10,
                        Register::Rbp,
                        self.callee_slot,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                if closure_call {
                    self.load_physical(Register::R11, Register::R10, 0, Width::Qword)?;
                } else {
                    self.encoder
                        .mov_reg(Register::R11, Register::R10, Width::Qword)
                        .map_err(encode_error)?;
                }
                self.encoder
                    .call_indirect(Register::R11)
                    .map_err(encode_error)?;
                if cleanup != 0 {
                    self.encoder
                        .add_imm32(Register::Rsp, cleanup)
                        .map_err(encode_error)?;
                }
                Ok(())
            }
            HirExprKind::Closure {
                closure, captures, ..
            } => self.materialize_closure(*closure, captures),
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } if intrinsic.is_linux() => self.lower_linux_intrinsic(*intrinsic, arguments),
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } if intrinsic.is_windows() => self.lower_windows_intrinsic(*intrinsic, arguments),
            HirExprKind::IntrinsicCall {
                intrinsic,
                arguments,
            } if intrinsic.is_runtime_view() => {
                let [pointer, length] = arguments.as_slice() else {
                    return Err(error(
                        "E_AMD64_HIR",
                        "runtime view constructor lost pointer or length",
                    ));
                };
                let storage = self.try_slot.ok_or_else(|| {
                    unsupported("DBYTE64 runtime view", "aggregate temporary storage")
                })?;
                self.lower_expr(pointer)?;
                self.encoder
                    .mov_base_disp32_from_reg(Register::Rbp, storage, Register::Rax, Width::Qword)
                    .map_err(encode_error)?;
                self.lower_expr(length)?;
                self.encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        storage + self.project.executable.semantic.layout.pointer_size as i32,
                        Register::Rax,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                self.encoder
                    .lea_base_disp32(Register::Rax, Register::Rbp, storage)
                    .map_err(encode_error)
            }
            HirExprKind::IntrinsicCall {
                intrinsic: dbyte_hir::HirIntrinsic::IoOut8,
                arguments,
            } if arguments.len() == 2 => {
                self.lower_expr(&arguments[0])?;
                self.encoder.push(Register::Rax).map_err(encode_error)?;
                self.lower_expr(&arguments[1])?;
                self.encoder.pop(Register::Rdx).map_err(encode_error)?;
                self.encoder.out8().map_err(encode_error)
            }
            HirExprKind::IntrinsicCall {
                intrinsic: dbyte_hir::HirIntrinsic::IoIn8,
                arguments,
            } if arguments.len() == 1 => {
                self.lower_expr(&arguments[0])?;
                self.encoder
                    .mov_reg(Register::Rdx, Register::Rax, Width::Qword)
                    .map_err(encode_error)?;
                self.encoder.in8().map_err(encode_error)?;
                self.encoder
                    .movzx_byte_to_qword(Register::Rax, Register::Rax)
                    .map_err(encode_error)
            }
            HirExprKind::IntrinsicCall {
                intrinsic: dbyte_hir::HirIntrinsic::CpuCli,
                arguments,
            } if arguments.is_empty() => self.encoder.cli().map_err(encode_error),
            HirExprKind::IntrinsicCall {
                intrinsic: dbyte_hir::HirIntrinsic::CpuHlt,
                arguments,
            } if arguments.is_empty() => self.encoder.hlt().map_err(encode_error),
            other => Err(unsupported("DBYTE64 expression", &format!("{other:?}"))),
        }
    }

    fn lower_arguments(
        &mut self,
        arguments: &[HirExpr],
        registers: &[Register],
    ) -> Result<(), ModernNativeError> {
        if arguments.len() > registers.len() {
            return Err(unsupported("DBYTE64 call", "overflow stack arguments"));
        }
        for argument in arguments {
            self.lower_expr(argument)?;
            self.encoder.push(Register::Rax).map_err(encode_error)?;
        }
        for register in registers[..arguments.len()].iter().rev() {
            self.encoder.pop(*register).map_err(encode_error)?;
        }
        Ok(())
    }

    fn lower_call_arguments(
        &mut self,
        arguments: &[HirExpr],
        convention: dbyte_ast::CallConvention,
    ) -> Result<u32, ModernNativeError> {
        self.lower_call_arguments_with_sret(arguments, convention, None)
    }

    fn lower_call_arguments_with_sret(
        &mut self,
        arguments: &[HirExpr],
        convention: dbyte_ast::CallConvention,
        sret_offset: Option<i32>,
    ) -> Result<u32, ModernNativeError> {
        self.lower_call_arguments_with_hidden(arguments, convention, sret_offset, false)
    }

    fn lower_call_arguments_with_hidden(
        &mut self,
        arguments: &[HirExpr],
        convention: dbyte_ast::CallConvention,
        sret_offset: Option<i32>,
        closure_env: bool,
    ) -> Result<u32, ModernNativeError> {
        if arguments.len() > self.argument_slots.len() {
            return Err(unsupported("DBYTE64 call", "more than 16 arguments"));
        }
        for (index, argument) in arguments.iter().enumerate() {
            self.lower_expr(argument)?;
            if !abi_scalar(&self.project.executable.semantic, argument.ty) {
                let destination = *self.aggregate_argument_slots.get(index).ok_or_else(|| {
                    error("E_AMD64_CALL", "aggregate argument scratch is unavailable")
                })?;
                let layout = dbyte_hir::layout_of(&self.project.executable.semantic, argument.ty)
                    .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                self.encoder
                    .mov_reg(Register::R11, Register::Rax, Width::Qword)
                    .map_err(encode_error)?;
                for (chunk, width) in physical_chunks(layout.size) {
                    self.load_physical(Register::Rax, Register::R11, chunk as i32, width)?;
                    self.encoder
                        .mov_base_disp32_from_reg(
                            Register::Rbp,
                            destination + chunk as i32,
                            Register::Rax,
                            width,
                        )
                        .map_err(encode_error)?;
                }
                self.encoder
                    .lea_base_disp32(Register::Rax, Register::Rbp, destination)
                    .map_err(encode_error)?;
            }
            self.encoder
                .mov_base_disp32_from_reg(
                    Register::Rbp,
                    self.argument_slots[index],
                    Register::Rax,
                    Width::Qword,
                )
                .map_err(encode_error)?;
        }

        let (registers, shadow): (&[Register], u32) = match convention {
            dbyte_ast::CallConvention::DByte => (&dbyte64_argument_registers(), 0),
            dbyte_ast::CallConvention::C => {
                let (registers, shadow) =
                    c_argument_registers(self.project.executable.semantic.profile)?;
                (registers, shadow as u32)
            }
        };
        if sret_offset.is_some() && convention == dbyte_ast::CallConvention::C {
            return Err(unsupported("DBYTE64 C call", "aggregate return"));
        }
        let hidden_count = usize::from(closure_env) + usize::from(sret_offset.is_some());
        let mut register_index = hidden_count;
        let mut stack_cursor = shadow;
        let mut stack_items = Vec::<(usize, u32, u32)>::new();
        for (index, argument) in arguments.iter().enumerate() {
            if abi_scalar(&self.project.executable.semantic, argument.ty) {
                if register_index < registers.len() {
                    register_index += 1;
                } else {
                    stack_items.push((index, stack_cursor, 8));
                    stack_cursor = stack_cursor
                        .checked_add(8)
                        .ok_or_else(|| error("E_AMD64_CALL", "argument stack overflows"))?;
                }
            } else {
                if convention == dbyte_ast::CallConvention::C {
                    return Err(unsupported("Win64/SysV64 call", "aggregate by value"));
                }
                let layout = dbyte_hir::layout_of(&self.project.executable.semantic, argument.ty)
                    .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                stack_cursor = align_u32(stack_cursor, layout.align.max(8))?;
                stack_items.push((index, stack_cursor, layout.size));
                stack_cursor = stack_cursor
                    .checked_add(align_u32(layout.size, 8)?)
                    .ok_or_else(|| error("E_AMD64_CALL", "aggregate argument overflows"))?;
            }
        }
        let cleanup = align_u32(stack_cursor, 16)?;
        if cleanup != 0 {
            self.encoder
                .sub_imm32(Register::Rsp, cleanup)
                .map_err(encode_error)?;
        }
        for (index, offset, size) in stack_items {
            self.encoder
                .mov_reg_from_base_disp32(
                    Register::R11,
                    Register::Rbp,
                    self.argument_slots[index],
                    Width::Qword,
                )
                .map_err(encode_error)?;
            if abi_scalar(&self.project.executable.semantic, arguments[index].ty) {
                self.encoder
                    .mov_base_disp32_from_reg(
                        Register::Rsp,
                        offset as i32,
                        Register::R11,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
            } else {
                for (chunk, width) in physical_chunks(size) {
                    self.encoder
                        .mov_reg_from_base_disp32(Register::Rax, Register::R11, chunk as i32, width)
                        .map_err(encode_error)?;
                    self.encoder
                        .mov_base_disp32_from_reg(
                            Register::Rsp,
                            (offset + chunk) as i32,
                            Register::Rax,
                            width,
                        )
                        .map_err(encode_error)?;
                }
            }
        }
        let mut scalar_register = 0usize;
        let register_prefix = hidden_count;
        for (index, argument) in arguments.iter().enumerate() {
            if !abi_scalar(&self.project.executable.semantic, argument.ty) {
                continue;
            }
            if let Some(register) = registers.get(register_prefix + scalar_register).copied() {
                self.encoder
                    .mov_reg_from_base_disp32(
                        register,
                        Register::Rbp,
                        self.argument_slots[index],
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
            }
            scalar_register += 1;
        }
        if closure_env {
            self.encoder
                .mov_reg_from_base_disp32(
                    Register::R11,
                    Register::Rbp,
                    self.callee_slot,
                    Width::Qword,
                )
                .map_err(encode_error)?;
            self.load_physical(Register::Rax, Register::R11, 8, Width::Qword)?;
        }
        if let Some(offset) = sret_offset {
            self.encoder
                .lea_base_disp32(
                    if closure_env {
                        Register::Rdx
                    } else {
                        Register::Rax
                    },
                    Register::Rbp,
                    offset,
                )
                .map_err(encode_error)?;
        }
        Ok(cleanup)
    }

    fn lower_linux_intrinsic(
        &mut self,
        intrinsic: dbyte_hir::HirIntrinsic,
        arguments: &[HirExpr],
    ) -> Result<(), ModernNativeError> {
        let (number, registers): (u64, &[Register]) = match intrinsic {
            dbyte_hir::HirIntrinsic::LinuxRead => {
                (0, &[Register::Rdi, Register::Rsi, Register::Rdx])
            }
            dbyte_hir::HirIntrinsic::LinuxWrite => {
                (1, &[Register::Rdi, Register::Rsi, Register::Rdx])
            }
            dbyte_hir::HirIntrinsic::LinuxClose => (3, &[Register::Rdi]),
            dbyte_hir::HirIntrinsic::LinuxMmap => (
                9,
                &[
                    Register::Rdi,
                    Register::Rsi,
                    Register::Rdx,
                    Register::R10,
                    Register::R8,
                    Register::R9,
                ],
            ),
            dbyte_hir::HirIntrinsic::LinuxMunmap => (11, &[Register::Rdi, Register::Rsi]),
            dbyte_hir::HirIntrinsic::LinuxExit => (60, &[Register::Rdi]),
            dbyte_hir::HirIntrinsic::LinuxClockGettime => (228, &[Register::Rdi, Register::Rsi]),
            dbyte_hir::HirIntrinsic::LinuxOpenAt => (
                257,
                &[Register::Rdi, Register::Rsi, Register::Rdx, Register::R10],
            ),
            _ => unreachable!(),
        };
        self.lower_arguments(arguments, registers)?;
        self.encoder
            .mov_imm64(Register::Rax, number)
            .map_err(encode_error)?;
        self.encoder.syscall().map_err(encode_error)
    }

    fn lower_windows_intrinsic(
        &mut self,
        intrinsic: dbyte_hir::HirIntrinsic,
        arguments: &[HirExpr],
    ) -> Result<(), ModernNativeError> {
        let symbol = match intrinsic {
            dbyte_hir::HirIntrinsic::WindowsGetStdHandle => "__imp_GetStdHandle",
            dbyte_hir::HirIntrinsic::WindowsReadFile => "__imp_ReadFile",
            dbyte_hir::HirIntrinsic::WindowsWriteFile => "__imp_WriteFile",
            dbyte_hir::HirIntrinsic::WindowsCloseHandle => "__imp_CloseHandle",
            dbyte_hir::HirIntrinsic::WindowsVirtualAlloc => "__imp_VirtualAlloc",
            dbyte_hir::HirIntrinsic::WindowsVirtualFree => "__imp_VirtualFree",
            dbyte_hir::HirIntrinsic::WindowsQueryPerformanceCounter => {
                "__imp_QueryPerformanceCounter"
            }
            _ => unreachable!(),
        };
        let cleanup = self.lower_call_arguments(arguments, dbyte_ast::CallConvention::C)?;
        let offset = self
            .encoder
            .call_rip_relative_placeholder()
            .map_err(encode_error)?;
        self.relocations.push(Relocation {
            section: Section::Text,
            offset: offset as u64,
            symbol: symbol.to_string(),
            kind: RelocationKind::Pc32,
            addend: -4,
        });
        if cleanup != 0 {
            self.encoder
                .add_imm32(Register::Rsp, cleanup)
                .map_err(encode_error)?;
        }
        Ok(())
    }

    fn lower_string(
        &mut self,
        bytes: &[u8],
        ty: dbyte_hir::TypeId,
    ) -> Result<(), ModernNativeError> {
        let symbol = if let Some(symbol) = self.strings.get(bytes) {
            symbol.clone()
        } else {
            let symbol = format!("__dbyte_string_{:04}", self.strings.len());
            let offset = self.rodata.len() as u64;
            self.rodata.extend_from_slice(bytes);
            self.string_symbols
                .push((symbol.clone(), offset, bytes.len() as u64));
            self.strings.insert(bytes.to_vec(), symbol.clone());
            symbol
        };
        let offset = self
            .encoder
            .lea_rip_relative_placeholder(Register::Rax)
            .map_err(encode_error)?;
        self.relocations.push(Relocation {
            section: Section::Text,
            offset: offset as u64,
            symbol,
            kind: RelocationKind::Pc32,
            addend: -4,
        });
        if matches!(
            self.project.executable.semantic.type_of(ty),
            CanonicalType::StringView
        ) {
            let storage = self.try_slot.ok_or_else(|| {
                unsupported("DBYTE64 string literal", "StringView temporary storage")
            })?;
            self.encoder
                .mov_base_disp32_from_reg(Register::Rbp, storage, Register::Rax, Width::Qword)
                .map_err(encode_error)?;
            self.encoder
                .mov_imm64(Register::Rax, bytes.len() as u64)
                .map_err(encode_error)?;
            self.encoder
                .mov_base_disp32_from_reg(
                    Register::Rbp,
                    storage + self.project.executable.semantic.layout.pointer_size as i32,
                    Register::Rax,
                    Width::Qword,
                )
                .map_err(encode_error)?;
            self.encoder
                .lea_base_disp32(Register::Rax, Register::Rbp, storage)
                .map_err(encode_error)?;
        }
        Ok(())
    }

    fn initialize_struct_local(
        &mut self,
        local: LocalId,
        expression: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        self.initialize_struct_at_offset(self.local_offset(local)?, expression)
    }

    fn initialize_struct_at_offset(
        &mut self,
        base: i32,
        expression: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        let HirExprKind::Struct { fields, .. } = &expression.kind else {
            unreachable!()
        };
        let layout = dbyte_hir::layout_of(&self.project.executable.semantic, expression.ty)
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        self.encoder
            .mov_imm64(Register::Rax, 0)
            .map_err(encode_error)?;
        for (chunk, width) in physical_chunks(layout.size) {
            self.encoder
                .mov_base_disp32_from_reg(Register::Rbp, base + chunk as i32, Register::Rax, width)
                .map_err(encode_error)?;
        }
        for (field, value) in fields {
            let (_, offset) = dbyte_hir::struct_field_layout(
                &self.project.executable.semantic,
                expression.ty,
                *field,
            )
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
            self.initialize_value_at_offset(base + offset as i32, value)?;
        }
        Ok(())
    }

    fn initialize_value_at_offset(
        &mut self,
        base: i32,
        value: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        if matches!(
            self.project.executable.semantic.type_of(value.ty),
            CanonicalType::Unit
        ) {
            return Ok(());
        }
        if abi_scalar(&self.project.executable.semantic, value.ty) {
            self.lower_expr(value)?;
            return self
                .encoder
                .mov_base_disp32_from_reg(
                    Register::Rbp,
                    base,
                    Register::Rax,
                    scalar_width(&self.project.executable.semantic, value.ty)?,
                )
                .map_err(encode_error);
        }
        match &value.kind {
            HirExprKind::Struct { .. } => self.initialize_struct_at_offset(base, value),
            HirExprKind::Enum { .. } => self.initialize_enum_at_offset(base, value),
            HirExprKind::DirectCall { .. } | HirExprKind::IndirectCall { .. } => {
                self.lower_aggregate_call_into_offset(base, value)
            }
            _ => {
                self.lower_expr(value)?;
                let layout = dbyte_hir::layout_of(&self.project.executable.semantic, value.ty)
                    .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                self.encoder
                    .mov_reg(Register::R11, Register::Rax, Width::Qword)
                    .map_err(encode_error)?;
                for (chunk, width) in physical_chunks(layout.size) {
                    self.load_physical(Register::Rax, Register::R11, chunk as i32, width)?;
                    self.encoder
                        .mov_base_disp32_from_reg(
                            Register::Rbp,
                            base + chunk as i32,
                            Register::Rax,
                            width,
                        )
                        .map_err(encode_error)?;
                }
                Ok(())
            }
        }
    }

    fn initialize_enum_local(
        &mut self,
        local: LocalId,
        expression: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        self.initialize_enum_at_offset(self.local_offset(local)?, expression)
    }

    fn initialize_enum_at_offset(
        &mut self,
        base: i32,
        expression: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        let HirExprKind::Enum {
            variant,
            discriminant,
            payload,
            ..
        } = &expression.kind
        else {
            unreachable!()
        };
        let layout = dbyte_hir::layout_of(&self.project.executable.semantic, expression.ty)
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        self.encoder
            .mov_imm64(Register::Rax, 0)
            .map_err(encode_error)?;
        for (chunk, width) in physical_chunks(layout.size) {
            self.encoder
                .mov_base_disp32_from_reg(Register::Rbp, base + chunk as i32, Register::Rax, width)
                .map_err(encode_error)?;
        }
        self.encoder
            .mov_imm64(Register::Rax, u64::from(*discriminant))
            .map_err(encode_error)?;
        self.encoder
            .mov_base_disp32_from_reg(Register::Rbp, base, Register::Rax, Width::Dword)
            .map_err(encode_error)?;
        let (_, _, fields) = dbyte_hir::enum_variant_layout(
            &self.project.executable.semantic,
            expression.ty,
            *variant,
        )
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        for (value, (_, offset)) in payload.iter().zip(fields) {
            self.initialize_value_at_offset(base + offset as i32, value)?;
        }
        Ok(())
    }

    fn lower_match(
        &mut self,
        value: &HirExpr,
        arms: &[dbyte_hir::HirMatchArm],
    ) -> Result<(), ModernNativeError> {
        self.lower_expr(value)?;
        self.encoder
            .mov_base_disp32_from_reg(Register::Rbp, self.callee_slot, Register::Rax, Width::Qword)
            .map_err(encode_error)?;
        let done = self.new_label();
        for arm in arms {
            let next = self.new_label();
            if let Some(variant) = arm.variant {
                self.encoder
                    .mov_reg_from_base_disp32(
                        Register::R11,
                        Register::Rbp,
                        self.callee_slot,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                self.encoder
                    .mov_reg_from_base_disp32(Register::Rax, Register::R11, 0, Width::Dword)
                    .map_err(encode_error)?;
                self.encoder
                    .mov_imm64(Register::Rcx, u64::from(variant.discriminant))
                    .map_err(encode_error)?;
                self.encoder
                    .cmp_reg(Register::Rax, Register::Rcx, Width::Dword)
                    .map_err(encode_error)?;
                self.jump_if(Condition::NotEqual, next)?;
                let (_, _, payload) = dbyte_hir::enum_variant_layout(
                    &self.project.executable.semantic,
                    value.ty,
                    variant.variant,
                )
                .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                for (binding, (ty, offset)) in arm.bindings.iter().zip(payload) {
                    self.encoder
                        .mov_reg_from_base_disp32(
                            Register::R11,
                            Register::Rbp,
                            self.callee_slot,
                            Width::Qword,
                        )
                        .map_err(encode_error)?;
                    if abi_scalar(&self.project.executable.semantic, ty) {
                        if !matches!(
                            self.project.executable.semantic.type_of(ty),
                            CanonicalType::Unit
                        ) {
                            self.load_physical(
                                Register::Rax,
                                Register::R11,
                                offset as i32,
                                scalar_width(&self.project.executable.semantic, ty)?,
                            )?;
                            self.store_local(binding.id)?;
                        }
                    } else {
                        let layout = dbyte_hir::layout_of(&self.project.executable.semantic, ty)
                            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                        for (chunk, width) in physical_chunks(layout.size) {
                            self.load_physical(
                                Register::Rax,
                                Register::R11,
                                offset as i32 + chunk as i32,
                                width,
                            )?;
                            self.encoder
                                .mov_base_disp32_from_reg(
                                    Register::Rbp,
                                    self.local_offset(binding.id)? + chunk as i32,
                                    Register::Rax,
                                    width,
                                )
                                .map_err(encode_error)?;
                        }
                    }
                }
            }
            self.lower_block(&arm.body)?;
            self.jump(done)?;
            self.bind(next)?;
        }
        self.bind(done)
    }

    fn lower_aggregate_call_into(
        &mut self,
        local: LocalId,
        expression: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        self.lower_aggregate_call_into_offset(self.local_offset(local)?, expression)
    }

    fn lower_aggregate_call_into_offset(
        &mut self,
        destination: i32,
        expression: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        let cleanup = match &expression.kind {
            HirExprKind::DirectCall {
                callee,
                arguments,
                convention,
            } => {
                let cleanup =
                    self.lower_call_arguments_with_sret(arguments, *convention, Some(destination))?;
                let symbol = self
                    .function_symbols
                    .get(callee)
                    .ok_or_else(|| error("E_AMD64_CALL", "resolved function has no link symbol"))?;
                let offset = self
                    .encoder
                    .call_relative_placeholder()
                    .map_err(encode_error)?;
                self.relocations.push(Relocation {
                    section: Section::Text,
                    offset: offset as u64,
                    symbol: symbol.clone(),
                    kind: RelocationKind::Plt32,
                    addend: -4,
                });
                cleanup
            }
            HirExprKind::IndirectCall {
                callee,
                arguments,
                convention,
            } => {
                self.lower_expr(callee)?;
                self.encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        self.callee_slot,
                        Register::Rax,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                let cleanup =
                    self.lower_call_arguments_with_sret(arguments, *convention, Some(destination))?;
                self.encoder
                    .mov_reg_from_base_disp32(
                        Register::R11,
                        Register::Rbp,
                        self.callee_slot,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                self.encoder
                    .call_indirect(Register::R11)
                    .map_err(encode_error)?;
                cleanup
            }
            _ => unreachable!(),
        };
        if cleanup != 0 {
            self.encoder
                .add_imm32(Register::Rsp, cleanup)
                .map_err(encode_error)?;
        }
        Ok(())
    }

    fn lower_aggregate_return(&mut self, value: &HirExpr) -> Result<(), ModernNativeError> {
        let sret_slot = self
            .sret_slot
            .ok_or_else(|| error("E_AMD64_SRET", "aggregate return has no sret slot"))?;
        let layout = dbyte_hir::layout_of(&self.project.executable.semantic, value.ty)
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        if let HirExprKind::Struct { fields, .. } = &value.kind {
            self.encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error)?;
            for (chunk, width) in physical_chunks(layout.size) {
                self.encoder
                    .mov_reg_from_base_disp32(Register::R11, Register::Rbp, sret_slot, Width::Qword)
                    .map_err(encode_error)?;
                self.encoder
                    .mov_base_disp32_from_reg(Register::R11, chunk as i32, Register::Rax, width)
                    .map_err(encode_error)?;
            }
            for (field, field_value) in fields {
                let (_, offset) = dbyte_hir::struct_field_layout(
                    &self.project.executable.semantic,
                    value.ty,
                    *field,
                )
                .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                if matches!(
                    self.project.executable.semantic.type_of(field_value.ty),
                    CanonicalType::Unit
                ) {
                    continue;
                }
                self.lower_expr(field_value)?;
                self.encoder
                    .mov_reg_from_base_disp32(Register::R11, Register::Rbp, sret_slot, Width::Qword)
                    .map_err(encode_error)?;
                if abi_scalar(&self.project.executable.semantic, field_value.ty) {
                    self.encoder
                        .mov_base_disp32_from_reg(
                            Register::R11,
                            offset as i32,
                            Register::Rax,
                            scalar_width(&self.project.executable.semantic, field_value.ty)?,
                        )
                        .map_err(encode_error)?;
                } else {
                    let field_layout =
                        dbyte_hir::layout_of(&self.project.executable.semantic, field_value.ty)
                            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                    self.encoder
                        .mov_reg(Register::R10, Register::Rax, Width::Qword)
                        .map_err(encode_error)?;
                    for (chunk, width) in physical_chunks(field_layout.size) {
                        self.load_physical(Register::Rax, Register::R10, chunk as i32, width)?;
                        self.encoder
                            .mov_reg_from_base_disp32(
                                Register::R11,
                                Register::Rbp,
                                sret_slot,
                                Width::Qword,
                            )
                            .map_err(encode_error)?;
                        self.encoder
                            .mov_base_disp32_from_reg(
                                Register::R11,
                                offset as i32 + chunk as i32,
                                Register::Rax,
                                width,
                            )
                            .map_err(encode_error)?;
                    }
                }
            }
        } else if let HirExprKind::Enum {
            variant,
            discriminant,
            payload,
            ..
        } = &value.kind
        {
            self.encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error)?;
            for (chunk, width) in physical_chunks(layout.size) {
                self.encoder
                    .mov_reg_from_base_disp32(Register::R11, Register::Rbp, sret_slot, Width::Qword)
                    .map_err(encode_error)?;
                self.encoder
                    .mov_base_disp32_from_reg(Register::R11, chunk as i32, Register::Rax, width)
                    .map_err(encode_error)?;
            }
            self.encoder
                .mov_imm64(Register::Rax, u64::from(*discriminant))
                .map_err(encode_error)?;
            self.encoder
                .mov_reg_from_base_disp32(Register::R11, Register::Rbp, sret_slot, Width::Qword)
                .map_err(encode_error)?;
            self.encoder
                .mov_base_disp32_from_reg(Register::R11, 0, Register::Rax, Width::Dword)
                .map_err(encode_error)?;
            let (_, _, fields) = dbyte_hir::enum_variant_layout(
                &self.project.executable.semantic,
                value.ty,
                *variant,
            )
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
            for (field_value, (_, offset)) in payload.iter().zip(fields) {
                self.lower_expr(field_value)?;
                if matches!(
                    self.project.executable.semantic.type_of(field_value.ty),
                    CanonicalType::Unit
                ) {
                    continue;
                }
                if abi_scalar(&self.project.executable.semantic, field_value.ty) {
                    self.encoder
                        .mov_reg_from_base_disp32(
                            Register::R11,
                            Register::Rbp,
                            sret_slot,
                            Width::Qword,
                        )
                        .map_err(encode_error)?;
                    self.encoder
                        .mov_base_disp32_from_reg(
                            Register::R11,
                            offset as i32,
                            Register::Rax,
                            scalar_width(&self.project.executable.semantic, field_value.ty)?,
                        )
                        .map_err(encode_error)?;
                } else {
                    let field_layout =
                        dbyte_hir::layout_of(&self.project.executable.semantic, field_value.ty)
                            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
                    self.encoder
                        .mov_reg(Register::R10, Register::Rax, Width::Qword)
                        .map_err(encode_error)?;
                    for (chunk, width) in physical_chunks(field_layout.size) {
                        self.load_physical(Register::Rax, Register::R10, chunk as i32, width)?;
                        self.encoder
                            .mov_reg_from_base_disp32(
                                Register::R11,
                                Register::Rbp,
                                sret_slot,
                                Width::Qword,
                            )
                            .map_err(encode_error)?;
                        self.encoder
                            .mov_base_disp32_from_reg(
                                Register::R11,
                                offset as i32 + chunk as i32,
                                Register::Rax,
                                width,
                            )
                            .map_err(encode_error)?;
                    }
                }
            }
        } else {
            self.lower_expr(value)?;
            self.encoder
                .mov_reg(Register::R10, Register::Rax, Width::Qword)
                .map_err(encode_error)?;
            self.encoder
                .mov_reg_from_base_disp32(Register::R11, Register::Rbp, sret_slot, Width::Qword)
                .map_err(encode_error)?;
            for (chunk, width) in physical_chunks(layout.size) {
                self.encoder
                    .mov_reg_from_base_disp32(Register::Rax, Register::R10, chunk as i32, width)
                    .map_err(encode_error)?;
                self.encoder
                    .mov_base_disp32_from_reg(Register::R11, chunk as i32, Register::Rax, width)
                    .map_err(encode_error)?;
            }
        }
        self.encoder
            .mov_reg_from_base_disp32(Register::Rax, Register::Rbp, sret_slot, Width::Qword)
            .map_err(encode_error)
    }

    fn materialize_closure(
        &mut self,
        closure: ClosureId,
        captures: &[HirCapture],
    ) -> Result<(), ModernNativeError> {
        let storage = self.try_slot.ok_or_else(|| {
            unsupported("DBYTE64 closure", "closure descriptor temporary storage")
        })?;
        let environment = self.closure_env_slots.get(&closure).copied();
        let (_, accesses) = closure_environment_layout64(self.project, captures)?;
        for capture in captures {
            let access = &accesses[&capture.local];
            let Some(environment) = environment else {
                return Err(error(
                    "E_AMD64_CLOSURE",
                    "capturing closure has no environment storage",
                ));
            };
            if capture.mode == CaptureMode::Move {
                if !abi_scalar(&self.project.executable.semantic, capture.ty) {
                    return Err(unsupported(
                        "DBYTE64 closure capture",
                        "aggregate move capture",
                    ));
                }
                self.load_local(capture.local)?;
                self.encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        environment + access.offset as i32,
                        Register::Rax,
                        scalar_width(&self.project.executable.semantic, access.ty)?,
                    )
                    .map_err(encode_error)?;
            } else {
                self.lower_local_address(capture.local)?;
                self.encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        environment + access.offset as i32,
                        Register::Rax,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
            }
        }
        let symbol = self
            .closure_symbols
            .get(&(self.function.instance, closure))
            .cloned()
            .ok_or_else(|| error("E_AMD64_CLOSURE", "closure helper symbol is missing"))?;
        let offset = self
            .encoder
            .lea_rip_relative_placeholder(Register::Rax)
            .map_err(encode_error)?;
        self.relocations.push(Relocation {
            section: Section::Text,
            offset: offset as u64,
            symbol,
            kind: RelocationKind::Pc32,
            addend: -4,
        });
        self.encoder
            .mov_base_disp32_from_reg(Register::Rbp, storage, Register::Rax, Width::Qword)
            .map_err(encode_error)?;
        if let Some(environment) = environment {
            self.encoder
                .lea_base_disp32(Register::Rax, Register::Rbp, environment)
                .map_err(encode_error)?;
        } else {
            self.encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error)?;
        }
        self.encoder
            .mov_base_disp32_from_reg(Register::Rbp, storage + 8, Register::Rax, Width::Qword)
            .map_err(encode_error)?;
        self.encoder
            .lea_base_disp32(Register::Rax, Register::Rbp, storage)
            .map_err(encode_error)
    }

    fn lower_try(
        &mut self,
        value: &HirExpr,
        ok_variant: dbyte_hir::VariantId,
        cleanup: &[HirBlock],
    ) -> Result<(), ModernNativeError> {
        let storage = self
            .try_slot
            .ok_or_else(|| error("E_AMD64_TRY", "Result propagation has no try storage"))?;
        let sret = self
            .sret_slot
            .ok_or_else(|| error("E_AMD64_TRY", "Result propagation requires sret"))?;
        if matches!(
            value.kind,
            HirExprKind::DirectCall { .. } | HirExprKind::IndirectCall { .. }
        ) {
            self.lower_aggregate_call_into_offset(storage, value)?;
        } else {
            self.lower_expr(value)?;
            let layout = dbyte_hir::layout_of(&self.project.executable.semantic, value.ty)
                .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
            if !layout.size.is_multiple_of(8) {
                return Err(unsupported("DBYTE64 Result", "non-qword layout"));
            }
            self.encoder
                .mov_reg(Register::R11, Register::Rax, Width::Qword)
                .map_err(encode_error)?;
            for chunk in (0..layout.size).step_by(8) {
                self.encoder
                    .mov_reg_from_base_disp32(
                        Register::Rax,
                        Register::R11,
                        chunk as i32,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
                self.encoder
                    .mov_base_disp32_from_reg(
                        Register::Rbp,
                        storage + chunk as i32,
                        Register::Rax,
                        Width::Qword,
                    )
                    .map_err(encode_error)?;
            }
        }
        let (ok_discriminant, _, payload) =
            dbyte_hir::enum_variant_layout(&self.project.executable.semantic, value.ty, ok_variant)
                .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        self.encoder
            .mov_reg_from_base_disp32(Register::Rax, Register::Rbp, storage, Width::Dword)
            .map_err(encode_error)?;
        self.encoder
            .mov_imm64(Register::Rcx, u64::from(ok_discriminant))
            .map_err(encode_error)?;
        self.encoder
            .cmp_reg(Register::Rax, Register::Rcx, Width::Dword)
            .map_err(encode_error)?;
        let success = self.new_label();
        self.jump_if(Condition::Equal, success)?;

        let layout = dbyte_hir::layout_of(&self.project.executable.semantic, value.ty)
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        for chunk in (0..layout.size).step_by(8) {
            self.encoder
                .mov_reg_from_base_disp32(
                    Register::Rax,
                    Register::Rbp,
                    storage + chunk as i32,
                    Width::Qword,
                )
                .map_err(encode_error)?;
            self.encoder
                .mov_reg_from_base_disp32(Register::R11, Register::Rbp, sret, Width::Qword)
                .map_err(encode_error)?;
            self.encoder
                .mov_base_disp32_from_reg(Register::R11, chunk as i32, Register::Rax, Width::Qword)
                .map_err(encode_error)?;
        }
        for block in cleanup {
            self.lower_block(block)?;
        }
        self.encoder
            .mov_reg_from_base_disp32(Register::Rax, Register::Rbp, sret, Width::Qword)
            .map_err(encode_error)?;
        self.emit_return()?;

        self.bind(success)?;
        let Some((payload_ty, payload_offset)) = payload.first().copied() else {
            return self
                .encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error);
        };
        if matches!(
            self.project.executable.semantic.type_of(payload_ty),
            CanonicalType::Unit
        ) {
            return self
                .encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error);
        }
        if !abi_scalar(&self.project.executable.semantic, payload_ty) {
            return Err(unsupported("DBYTE64 Result Ok", "aggregate payload"));
        }
        self.load_physical(
            Register::Rax,
            Register::Rbp,
            storage + payload_offset as i32,
            scalar_width(&self.project.executable.semantic, payload_ty)?,
        )
    }

    fn copy_from_pointer_to_local(
        &mut self,
        local: LocalId,
        ty: dbyte_hir::TypeId,
    ) -> Result<(), ModernNativeError> {
        let layout = dbyte_hir::layout_of(&self.project.executable.semantic, ty)
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        let destination = self.local_offset(local)?;
        self.encoder
            .mov_reg(Register::R11, Register::Rax, Width::Qword)
            .map_err(encode_error)?;
        for (chunk, width) in physical_chunks(layout.size) {
            self.encoder
                .mov_reg_from_base_disp32(Register::Rax, Register::R11, chunk as i32, width)
                .map_err(encode_error)?;
            self.encoder
                .mov_base_disp32_from_reg(
                    Register::Rbp,
                    destination + chunk as i32,
                    Register::Rax,
                    width,
                )
                .map_err(encode_error)?;
        }
        Ok(())
    }

    fn lower_place_address(
        &mut self,
        place: &dbyte_hir::HirPlace,
    ) -> Result<(), ModernNativeError> {
        let mut current_ty = self.place_base_type(&place.base)?;
        match &place.base {
            dbyte_hir::HirPlaceBase::Local(local) => self.lower_local_address(*local)?,
            dbyte_hir::HirPlaceBase::Pointer(pointer) => self.lower_expr(pointer)?,
            dbyte_hir::HirPlaceBase::Global(symbol) => self.lower_global_address(*symbol)?,
        }
        for projection in &place.projections {
            match projection {
                dbyte_hir::HirProjection::Field { offset, .. }
                | dbyte_hir::HirProjection::Payload { offset, .. } => {
                    if *offset != 0 {
                        self.encoder
                            .add_imm32(Register::Rax, *offset)
                            .map_err(encode_error)?;
                    }
                    current_ty = match projection {
                        dbyte_hir::HirProjection::Field { ty, .. }
                        | dbyte_hir::HirProjection::Payload { ty, .. } => *ty,
                        _ => unreachable!(),
                    };
                }
                dbyte_hir::HirProjection::Index {
                    index,
                    element,
                    element_size,
                    ..
                } => {
                    if matches!(
                        self.project.executable.semantic.type_of(current_ty),
                        CanonicalType::Slice { .. } | CanonicalType::StringView
                    ) {
                        self.load_physical(Register::Rax, Register::Rax, 0, Width::Qword)?;
                    }
                    self.encoder.push(Register::Rax).map_err(encode_error)?;
                    self.lower_expr(index)?;
                    self.encoder
                        .mov_imm64(Register::Rcx, u64::from(*element_size))
                        .map_err(encode_error)?;
                    self.encoder
                        .imul_reg(Register::Rax, Register::Rcx, Width::Qword)
                        .map_err(encode_error)?;
                    self.encoder.pop(Register::R11).map_err(encode_error)?;
                    self.encoder
                        .add_reg(Register::Rax, Register::R11, Width::Qword)
                        .map_err(encode_error)?;
                    current_ty = *element;
                }
            }
        }
        Ok(())
    }

    fn place_base_type(
        &self,
        base: &dbyte_hir::HirPlaceBase,
    ) -> Result<dbyte_hir::TypeId, ModernNativeError> {
        match base {
            dbyte_hir::HirPlaceBase::Local(local) => self
                .function
                .parameters
                .iter()
                .chain(self.function.locals.iter())
                .find(|candidate| candidate.id == *local)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| error("E_AMD64_LOCAL", "place base local has no canonical type")),
            dbyte_hir::HirPlaceBase::Global(symbol) => self
                .project
                .executable
                .globals
                .iter()
                .find(|candidate| candidate.symbol == *symbol)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| error("E_AMD64_GLOBAL", "place base global has no canonical type")),
            dbyte_hir::HirPlaceBase::Pointer(pointer) => {
                match self.project.executable.semantic.type_of(pointer.ty) {
                    CanonicalType::Pointer { pointee, .. }
                    | CanonicalType::Borrow { pointee, .. } => Ok(*pointee),
                    CanonicalType::Slice { .. }
                    | CanonicalType::StringView
                    | CanonicalType::Array { .. }
                    | CanonicalType::LegacyStr
                    | CanonicalType::LegacyBytes
                    | CanonicalType::LegacyBuffer => Ok(pointer.ty),
                    other => Err(error(
                        "E_AMD64_PLACE",
                        &format!("pointer place base has canonical type {other:?}"),
                    )),
                }
            }
        }
    }

    fn lower_place_value(&mut self, place: &dbyte_hir::HirPlace) -> Result<(), ModernNativeError> {
        self.lower_place_address(place)?;
        if matches!(
            self.project.executable.semantic.type_of(place.ty),
            CanonicalType::Unit
        ) {
            self.encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error)?;
        } else if abi_scalar(&self.project.executable.semantic, place.ty) {
            self.load_physical(
                Register::Rax,
                Register::Rax,
                0,
                scalar_width(&self.project.executable.semantic, place.ty)?,
            )?;
        }
        Ok(())
    }

    fn lower_place_assignment(
        &mut self,
        place: &dbyte_hir::HirPlace,
        value: &HirExpr,
    ) -> Result<(), ModernNativeError> {
        self.lower_place_address(place)?;
        self.encoder
            .mov_base_disp32_from_reg(Register::Rbp, self.return_slot, Register::Rax, Width::Qword)
            .map_err(encode_error)?;
        self.lower_expr(value)?;
        if matches!(
            self.project.executable.semantic.type_of(value.ty),
            CanonicalType::Unit
        ) {
            return Ok(());
        }
        if abi_scalar(&self.project.executable.semantic, value.ty) {
            self.encoder
                .mov_reg_from_base_disp32(
                    Register::R11,
                    Register::Rbp,
                    self.return_slot,
                    Width::Qword,
                )
                .map_err(encode_error)?;
            return self
                .encoder
                .mov_base_disp32_from_reg(
                    Register::R11,
                    0,
                    Register::Rax,
                    scalar_width(&self.project.executable.semantic, value.ty)?,
                )
                .map_err(encode_error);
        }
        let layout = dbyte_hir::layout_of(&self.project.executable.semantic, value.ty)
            .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
        self.encoder
            .mov_reg(Register::R10, Register::Rax, Width::Qword)
            .map_err(encode_error)?;
        for (chunk, width) in physical_chunks(layout.size) {
            self.load_physical(Register::Rax, Register::R10, chunk as i32, width)?;
            self.encoder
                .mov_reg_from_base_disp32(
                    Register::R11,
                    Register::Rbp,
                    self.return_slot,
                    Width::Qword,
                )
                .map_err(encode_error)?;
            self.encoder
                .mov_base_disp32_from_reg(Register::R11, chunk as i32, Register::Rax, width)
                .map_err(encode_error)?;
        }
        Ok(())
    }

    fn lower_global_address(
        &mut self,
        symbol: dbyte_hir::SymbolId,
    ) -> Result<(), ModernNativeError> {
        let name = self
            .global_symbols
            .get(&symbol)
            .map(|(name, ..)| name.clone())
            .ok_or_else(|| error("E_AMD64_GLOBAL", "global symbol is unresolved"))?;
        let offset = self
            .encoder
            .lea_rip_relative_placeholder(Register::Rax)
            .map_err(encode_error)?;
        self.relocations.push(Relocation {
            section: Section::Text,
            offset: offset as u64,
            symbol: name,
            kind: RelocationKind::Pc32,
            addend: -4,
        });
        Ok(())
    }

    fn local_offset(&self, local: LocalId) -> Result<i32, ModernNativeError> {
        self.slots
            .get(&local)
            .copied()
            .ok_or_else(|| error("E_AMD64_LOCAL", "local has no frame slot"))
    }

    fn lower_local_address(&mut self, local: LocalId) -> Result<(), ModernNativeError> {
        if let Some(access) = self
            .capture_env
            .as_ref()
            .and_then(|environment| environment.captures.get(&local).cloned())
        {
            let slot = self.capture_env.as_ref().expect("capture env exists").slot;
            self.encoder
                .mov_reg_from_base_disp32(Register::Rax, Register::Rbp, slot, Width::Qword)
                .map_err(encode_error)?;
            if access.offset != 0 {
                self.encoder
                    .add_imm32(Register::Rax, access.offset)
                    .map_err(encode_error)?;
            }
            if access.mode != CaptureMode::Move {
                self.load_physical(Register::Rax, Register::Rax, 0, Width::Qword)?;
            }
            return Ok(());
        }
        self.encoder
            .lea_base_disp32(Register::Rax, Register::Rbp, self.local_offset(local)?)
            .map_err(encode_error)
    }

    fn load_local(&mut self, local: LocalId) -> Result<(), ModernNativeError> {
        let ty = self
            .function
            .locals
            .iter()
            .find(|candidate| candidate.id == local)
            .map(|candidate| candidate.ty)
            .ok_or_else(|| error("E_AMD64_LOCAL", "local type is missing"))?;
        self.lower_local_address(local)?;
        if matches!(
            self.project.executable.semantic.type_of(ty),
            CanonicalType::Unit
        ) {
            self.encoder
                .mov_imm64(Register::Rax, 0)
                .map_err(encode_error)
        } else if abi_scalar(&self.project.executable.semantic, ty) {
            self.load_physical(
                Register::Rax,
                Register::Rax,
                0,
                scalar_width(&self.project.executable.semantic, ty)?,
            )
        } else {
            Ok(())
        }
    }

    fn store_local(&mut self, local: LocalId) -> Result<(), ModernNativeError> {
        if self
            .capture_env
            .as_ref()
            .is_some_and(|environment| environment.captures.contains_key(&local))
        {
            let ty = self
                .function
                .locals
                .iter()
                .find(|candidate| candidate.id == local)
                .map(|candidate| candidate.ty)
                .ok_or_else(|| error("E_AMD64_LOCAL", "captured local type is missing"))?;
            self.encoder
                .mov_base_disp32_from_reg(
                    Register::Rbp,
                    self.return_slot,
                    Register::Rax,
                    Width::Qword,
                )
                .map_err(encode_error)?;
            self.lower_local_address(local)?;
            self.encoder
                .mov_reg(Register::R11, Register::Rax, Width::Qword)
                .map_err(encode_error)?;
            self.encoder
                .mov_reg_from_base_disp32(
                    Register::Rax,
                    Register::Rbp,
                    self.return_slot,
                    Width::Qword,
                )
                .map_err(encode_error)?;
            return self
                .encoder
                .mov_base_disp32_from_reg(
                    Register::R11,
                    0,
                    Register::Rax,
                    scalar_width(&self.project.executable.semantic, ty)?,
                )
                .map_err(encode_error);
        }
        let offset = *self
            .slots
            .get(&local)
            .ok_or_else(|| error("E_AMD64_LOCAL", "local has no frame slot"))?;
        self.encoder
            .mov_base_disp32_from_reg(Register::Rbp, offset, Register::Rax, Width::Qword)
            .map_err(encode_error)
    }

    fn emit_return(&mut self) -> Result<(), ModernNativeError> {
        self.encoder
            .mov_reg(Register::Rsp, Register::Rbp, Width::Qword)
            .map_err(encode_error)?;
        self.encoder.pop(Register::Rbp).map_err(encode_error)?;
        self.encoder.ret().map_err(encode_error)
    }

    fn new_label(&mut self) -> u32 {
        let label = self.next_label;
        self.next_label += 1;
        label
    }

    fn bind(&mut self, label: u32) -> Result<(), ModernNativeError> {
        if self.labels.insert(label, self.encoder.position()).is_some() {
            return Err(error("E_AMD64_LABEL", "label was bound twice"));
        }
        Ok(())
    }

    fn jump(&mut self, label: u32) -> Result<(), ModernNativeError> {
        let offset = self
            .encoder
            .jump_relative_placeholder()
            .map_err(encode_error)?;
        self.fixups.push((offset, label));
        Ok(())
    }

    fn jump_if(&mut self, condition: Condition, label: u32) -> Result<(), ModernNativeError> {
        let offset = self
            .encoder
            .jump_if_placeholder(condition)
            .map_err(encode_error)?;
        self.fixups.push((offset, label));
        Ok(())
    }
}

fn is_signed_type(project: &ConcreteHirProject, ty: dbyte_hir::TypeId) -> bool {
    matches!(
        project.executable.semantic.type_of(ty),
        CanonicalType::Integer(
            IntegerType::I8
                | IntegerType::I16
                | IntegerType::I32
                | IntegerType::I64
                | IntegerType::LegacyInt
        )
    )
}

fn abi_scalar(program: &dbyte_hir::HirProgram, ty: dbyte_hir::TypeId) -> bool {
    matches!(
        program.type_of(ty),
        CanonicalType::Unit
            | CanonicalType::Integer(_)
            | CanonicalType::Bool
            | CanonicalType::Float
            | CanonicalType::Pointer { .. }
            | CanonicalType::Borrow { .. }
            | CanonicalType::Function { .. }
            | CanonicalType::LegacyStr
    )
}

fn scalar_width(
    program: &dbyte_hir::HirProgram,
    ty: dbyte_hir::TypeId,
) -> Result<Width, ModernNativeError> {
    let layout = dbyte_hir::layout_of(program, ty)
        .map_err(|diagnostic| error(diagnostic.code, &diagnostic.message))?;
    match layout.size {
        1 => Ok(Width::Byte),
        2 => Ok(Width::Word),
        4 => Ok(Width::Dword),
        8 => Ok(Width::Qword),
        _ => Err(unsupported(
            "DBYTE64 scalar",
            &format!(
                "unsupported physical width {} for {:?}",
                layout.size,
                program.type_of(ty)
            ),
        )),
    }
}

fn align_u32(value: u32, alignment: u32) -> Result<u32, ModernNativeError> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(error("E_AMD64_ALIGN", "alignment is not a power of two"));
    }
    value
        .checked_add(alignment - 1)
        .map(|aligned| aligned & !(alignment - 1))
        .ok_or_else(|| error("E_AMD64_ALIGN", "alignment overflows"))
}

fn align_i32(value: i32, alignment: u32) -> Result<i32, ModernNativeError> {
    let value = u32::try_from(value)
        .map_err(|_| error("E_AMD64_ALIGN", "negative stack alignment input"))?;
    i32::try_from(align_u32(value, alignment)?)
        .map_err(|_| error("E_AMD64_ALIGN", "aligned stack offset exceeds i32"))
}

fn align_u64(value: u64, alignment: u64) -> Result<u64, ModernNativeError> {
    if alignment == 0 || !alignment.is_power_of_two() {
        return Err(error("E_AMD64_ALIGN", "global alignment is invalid"));
    }
    value
        .checked_add(alignment - 1)
        .map(|aligned| aligned & !(alignment - 1))
        .ok_or_else(|| error("E_AMD64_ALIGN", "global alignment overflows"))
}

fn append_static_initializer(
    output: &mut Vec<u8>,
    initializer: &HirExpr,
    size: u32,
) -> Result<(), ModernNativeError> {
    let bits = match &initializer.kind {
        HirExprKind::Integer(value) => *value as u64,
        HirExprKind::Bool(value) => u64::from(*value),
        HirExprKind::Null | HirExprKind::Unit => 0,
        HirExprKind::Cast { value, .. } | HirExprKind::Constant { value, .. } => {
            return append_static_initializer(output, value, size)
        }
        _ => return Err(unsupported("DBYTE64 global", "non-scalar initializer")),
    };
    if size > 8 {
        return Err(unsupported("DBYTE64 global", "aggregate initializer"));
    }
    output.extend_from_slice(&bits.to_le_bytes()[..size as usize]);
    Ok(())
}

fn physical_chunks(size: u32) -> Vec<(u32, Width)> {
    let mut chunks = Vec::new();
    let mut offset = 0u32;
    while size - offset >= 8 {
        chunks.push((offset, Width::Qword));
        offset += 8;
    }
    if size - offset >= 4 {
        chunks.push((offset, Width::Dword));
        offset += 4;
    }
    if size - offset >= 2 {
        chunks.push((offset, Width::Word));
        offset += 2;
    }
    if size - offset == 1 {
        chunks.push((offset, Width::Byte));
    }
    chunks
}

fn dbyte64_argument_registers() -> [Register; 6] {
    [
        Register::Rax,
        Register::Rdx,
        Register::Rcx,
        Register::R8,
        Register::R9,
        Register::R10,
    ]
}

const SYSV64_C_ARGUMENT_REGISTERS: [Register; 6] = [
    Register::Rdi,
    Register::Rsi,
    Register::Rdx,
    Register::Rcx,
    Register::R8,
    Register::R9,
];
const WIN64_C_ARGUMENT_REGISTERS: [Register; 4] =
    [Register::Rcx, Register::Rdx, Register::R8, Register::R9];

fn c_argument_registers(
    profile: dbyte_hir::Profile,
) -> Result<(&'static [Register], i32), ModernNativeError> {
    match profile {
        dbyte_hir::Profile::X86_64Linux => Ok((&SYSV64_C_ARGUMENT_REGISTERS, 0)),
        dbyte_hir::Profile::X86_64Windows => Ok((&WIN64_C_ARGUMENT_REGISTERS, 32)),
        _ => Err(unsupported("DBYTE64 C call", "C ABI for this target")),
    }
}

fn validate_main(
    project: &ConcreteHirProject,
    function: &ConcreteHirFunction,
    entry_name: &str,
    target: TargetSpec,
) -> Result<usize, ModernNativeError> {
    if !function.public || function.declaration_only {
        return Err(error(
            "E_ENTRY_SIGNATURE",
            &format!("entry must be a defined public function `{entry_name}`"),
        ));
    }
    let entry_arity = function.parameters.len();
    let valid_parameters = if entry_arity == 0 {
        true
    } else if target == X86_64_NONE && entry_arity == 2 {
        let types = &project.executable.semantic;
        matches!(
            types.type_of(function.parameters[0].ty),
            CanonicalType::Integer(IntegerType::U32)
        ) && matches!(
            types.type_of(function.parameters[1].ty),
            CanonicalType::Integer(IntegerType::Usize)
        )
    } else if target == X86_64_LINUX && entry_arity == 3 {
        let types = &project.executable.semantic;
        matches!(
            types.type_of(function.parameters[0].ty),
            CanonicalType::Integer(IntegerType::Usize)
        ) && is_const_pointer_to_const_u8_pointer(types, function.parameters[1].ty)
            && is_const_pointer_to_const_u8_pointer(types, function.parameters[2].ty)
    } else {
        false
    };
    if !valid_parameters {
        return Err(error(
            "E_ENTRY_SIGNATURE",
            &format!(
                "{entry_name} must use (), the checked x86_64 Multiboot (u32, usize) entry, or Linux (usize, const_ptr[const_ptr[u8]], const_ptr[const_ptr[u8]]) entry parameters"
            ),
        ));
    }
    match project.executable.semantic.type_of(function.result) {
        CanonicalType::Integer(
            IntegerType::LegacyInt
            | IntegerType::I32
            | IntegerType::U32
            | IntegerType::I64
            | IntegerType::U64,
        ) => Ok(entry_arity),
        _ => Err(error(
            "E_ENTRY_SIGNATURE",
            "native entry must return int, i32, u32, i64, or u64",
        )),
    }
}

fn validate_i686_entry(
    project: &ConcreteHirProject,
    function: &ConcreteHirFunction,
    entry_name: &str,
    target: TargetSpec,
) -> Result<(), ModernNativeError> {
    if function.declaration_only || (target == crate::target::I686_USER && !function.public) {
        return Err(error(
            "E_ENTRY_SIGNATURE",
            &format!("entry must be a defined function `{entry_name}` for this profile"),
        ));
    }
    let parameters_valid = if target == crate::target::I686_USER {
        function.parameters.is_empty()
    } else if function.parameters.is_empty() {
        true
    } else {
        function.parameters.len() == 2
            && matches!(
                project
                    .executable
                    .semantic
                    .type_of(function.parameters[0].ty),
                CanonicalType::Integer(IntegerType::U32)
            )
            && matches!(
                project
                    .executable
                    .semantic
                    .type_of(function.parameters[1].ty),
                CanonicalType::Integer(IntegerType::Usize)
            )
    };
    if !parameters_valid {
        return Err(error(
            "E_ENTRY_SIGNATURE",
            &format!("{entry_name} must use () or the checked i686 Multiboot (u32, usize) entry"),
        ));
    }
    if !matches!(
        project.executable.semantic.type_of(function.result),
        CanonicalType::Integer(IntegerType::LegacyInt | IntegerType::I32 | IntegerType::U32)
    ) {
        return Err(error(
            "E_ENTRY_SIGNATURE",
            "i686 native entry must return int, i32, or u32",
        ));
    }
    Ok(())
}

fn is_const_pointer_to_const_u8_pointer(
    project: &dbyte_hir::HirProgram,
    ty: dbyte_hir::TypeId,
) -> bool {
    let CanonicalType::Pointer {
        mutable: false,
        pointee,
    } = project.type_of(ty)
    else {
        return false;
    };
    let CanonicalType::Pointer {
        mutable: false,
        pointee,
    } = project.type_of(*pointee)
    else {
        return false;
    };
    matches!(
        project.type_of(*pointee),
        CanonicalType::Integer(IntegerType::U8)
    )
}

fn lower_division(
    project: &ConcreteHirProject,
    expression: &HirExpr,
    op: BinOp,
    encoder: &mut Encoder,
) -> Result<(), ModernNativeError> {
    let signed = matches!(
        project.executable.semantic.type_of(expression.ty),
        CanonicalType::Integer(
            IntegerType::I8
                | IntegerType::I16
                | IntegerType::I32
                | IntegerType::I64
                | IntegerType::LegacyInt
        )
    );
    if signed {
        encoder.cqo().map_err(encode_error)?;
    } else {
        encoder
            .xor_reg(Register::Rdx, Register::Rdx, Width::Qword)
            .map_err(encode_error)?;
    }
    encoder
        .div_reg(Register::Rcx, signed, Width::Qword)
        .map_err(encode_error)?;
    if op == BinOp::Rem {
        encoder
            .mov_reg(Register::Rax, Register::Rdx, Width::Qword)
            .map_err(encode_error)?;
    }
    Ok(())
}

fn encode_error(error: crate::x86_64::EncodeError) -> ModernNativeError {
    ModernNativeError(format!("E_AMD64_ENCODE: {error}"))
}

fn diagnostics(diagnostics: Vec<dbyte_hir::Diagnostic>) -> ModernNativeError {
    ModernNativeError(
        diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn unsupported(context: &str, operation: &str) -> ModernNativeError {
    error(
        "E_NATIVE_HIR_UNSUPPORTED",
        &format!("DBYTE64 {context} does not yet lower {operation}"),
    )
}

fn error(code: &str, message: &str) -> ModernNativeError {
    ModernNativeError(format!("{code}: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbyte_hir::{ModuleId, ProjectInput, ProjectModuleInput};
    use dbyte_lexer::Lexer;
    use dbyte_parser::Parser;

    fn project(source: &str) -> ConcreteHirProject {
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let input = ProjectInput {
            root: ModuleId(0),
            modules: vec![ProjectModuleInput {
                id: ModuleId(0),
                name: "main".into(),
                program,
                imports: vec![],
            }],
        };
        let analyzed = dbyte_hir::analyze_project(
            &input,
            dbyte_hir::Profile::X86_64Linux,
            dbyte_hir::TargetDataLayout::X86_64,
        )
        .unwrap();
        dbyte_hir::monomorphize_project(&analyzed).unwrap()
    }

    fn project_none(source: &str) -> ConcreteHirProject {
        let program = Parser::new(Lexer::new(source).tokenize().unwrap())
            .parse_program()
            .unwrap();
        let input = ProjectInput {
            root: ModuleId(0),
            modules: vec![ProjectModuleInput {
                id: ModuleId(0),
                name: "main".into(),
                program,
                imports: vec![],
            }],
        };
        let analyzed = dbyte_hir::analyze_project(
            &input,
            dbyte_hir::Profile::X86_64None,
            dbyte_hir::TargetDataLayout::X86_64,
        )
        .unwrap();
        dbyte_hir::monomorphize_project(&analyzed).unwrap()
    }

    #[test]
    fn x86_64_none_accepts_checked_multiboot_entry_and_rejects_wide_magic() {
        let valid = project_none(
            "pub fn kernel_main(magic: u32, info: usize) -> int:\n    return (magic as int) + (info as int)\n",
        );
        let entry = valid
            .executable
            .functions
            .iter()
            .find(|function| function.symbol_name == "kernel_main")
            .unwrap();
        assert_eq!(
            validate_main(&valid, entry, "kernel_main", X86_64_NONE).unwrap(),
            2
        );

        let invalid = project_none(
            "pub fn kernel_main(magic: u64, info: usize) -> int:\n    return (magic as int) + (info as int)\n",
        );
        let entry = invalid
            .executable
            .functions
            .iter()
            .find(|function| function.symbol_name == "kernel_main")
            .unwrap();
        assert!(validate_main(&invalid, entry, "kernel_main", X86_64_NONE).is_err());
    }
    #[test]
    fn concrete_hir_produces_static_linux_elf64() {
        let executable = compile_concrete_linux(&project(
            "pub fn main() -> i64:\n    return ((40 as i64) + 2) * 1\n",
        ))
        .unwrap();
        let info = crate::elf64::inspect(&executable.elf.bytes).unwrap();
        assert_eq!(info.machine, 62);
        assert!(!info.has_interp);
        assert!(!info.has_dynamic);
        assert!(executable.hir_dump.contains("pointer=8 usize=8"));
    }

    #[test]
    fn linux_three_argument_entry_maps_the_initial_stack_to_dbyte64_slots() {
        let executable = compile_concrete_linux(&project(
            "pub fn main(argc: usize, argv: const_ptr[const_ptr[u8]], envp: const_ptr[const_ptr[u8]]) -> int:\n    return argc as int\n",
        ))
        .unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(8)
            .any(|bytes| { bytes == [0x48, 0x8b, 0x84, 0x24, 0x00, 0x00, 0x00, 0x00] }));
        assert!(executable
            .elf
            .bytes
            .windows(8)
            .any(|bytes| bytes == [0x48, 0x8d, 0x94, 0x24, 0x08, 0x00, 0x00, 0x00]));
    }

    #[test]
    fn dbyte64_overflow_arguments_use_aligned_stack_slots() {
        let source = "fn sum8(a: i64, b: i64, c: i64, d: i64, e: i64, f: i64, g: i64, h: i64) -> i64:\n    return a + b + c + d + e + f + g + h\n\npub fn main() -> i64:\n    return sum8(1 as i64, 2 as i64, 3 as i64, 4 as i64, 5 as i64, 6 as i64, 7 as i64, 8 as i64)\n";
        let executable = compile_concrete_linux(&project(source)).unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(7)
            .any(|bytes| { bytes == [0x48, 0x8b, 0x85, 0x10, 0x00, 0x00, 0x00] }));
        assert!(executable
            .elf
            .bytes
            .windows(7)
            .any(|bytes| { bytes == [0x48, 0x81, 0xc4, 0x10, 0x00, 0x00, 0x00] }));
    }

    #[test]
    fn dbyte64_function_values_lower_to_checked_indirect_calls() {
        let source = "fn add_one(value: i64) -> i64:\n    return value + 1\n\npub fn main() -> i64:\n    let callback: fn(i64) -> i64 = add_one\n    return callback(41 as i64)\n";
        let executable = compile_concrete_linux(&project(source)).unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(3)
            .any(|bytes| bytes == [0x49, 0xff, 0xd3]));
    }

    #[test]
    fn dbyte64_capturing_closure_uses_hidden_environment() {
        let source = "pub fn main() -> i64:\n    let total: i64 = 1\n    let add = fn [borrow mut total](value: i64) -> i64:\n        total = total + value\n        return total\n    return add(41 as i64)\n";
        let executable = compile_concrete_linux(&project(source)).unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(3)
            .any(|bytes| bytes == [0x49, 0xff, 0xd3]));
        assert!(executable
            .elf
            .bytes
            .windows(7)
            .any(|bytes| bytes[..3] == [0x48, 0x8d, 0x85]));
    }

    #[test]
    fn dbyte64_aggregate_argument_is_an_aligned_stack_copy() {
        let executable = compile_concrete_linux(&project(
            "struct Pair:\n    left: u64\n    right: u64\n\nfn sum(pair: Pair) -> u64:\n    return pair.left + pair.right\n\npub fn main() -> i64:\n    let pair = Pair(left = 40 as u64, right = 2 as u64)\n    return sum(pair) as i64\n",
        ))
        .unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(7)
            .any(|bytes| bytes == [0x48, 0x81, 0xec, 0x10, 0, 0, 0]));
        assert!(executable
            .elf
            .bytes
            .windows(8)
            .any(|bytes| bytes == [0x48, 0x89, 0x84, 0x24, 0x08, 0, 0, 0]));
    }

    #[test]
    fn dbyte64_snapshots_scalar_registers_before_aggregate_stack_copies() {
        let executable = compile_concrete_linux(&project(
            "struct Pair:\n    left: u64\n    right: u64\n\nfn choose(pair: Pair, marker: u64) -> u64:\n    return marker\n\npub fn main() -> i64:\n    let pair = Pair(left = 1 as u64, right = 2 as u64)\n    return choose(pair, 42 as u64) as i64\n",
        ))
        .unwrap();
        assert!(executable.elf.bytes.windows(14).any(|bytes| {
            bytes[..3] == [0x48, 0x89, 0x85]
                && bytes[7..] == [0x48, 0x8b, 0x85, 0x10, 0x00, 0x00, 0x00]
        }));
    }

    #[test]
    fn dbyte64_slice_index_loads_the_descriptor_data_pointer() {
        let executable = compile_concrete_linux(&project(
            "pub fn first(values: slice[u32]) -> u32:\n    return values[0 as usize]\n\npub fn main() -> i64:\n    return 0 as i64\n",
        ))
        .unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(8)
            .any(|bytes| { bytes == [0x48, 0x8b, 0x80, 0x00, 0x00, 0x00, 0x00, 0x50] }));
    }

    #[test]
    fn dbyte64_raw_pointer_dereference_loads_the_pointee_width() {
        let executable = compile_concrete_linux(&project(
            "fn increment(value: ptr[u32]) -> u32:\n    *value = *value + (1 as u32)\n    return *value\n\npub fn main() -> i64:\n    let value: u32 = 41 as u32\n    return increment(&value) as i64\n",
        ))
        .unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(6)
            .any(|bytes| bytes == [0x8b, 0x80, 0, 0, 0, 0]));
    }

    #[test]
    fn dbyte64_aggregate_return_uses_hidden_first_register_sret() {
        let executable = compile_concrete_linux(&project(
            "struct Pair:\n    left: u64\n    right: u64\n\nfn make(left: u64, right: u64) -> Pair:\n    return Pair(left = left, right = right)\n\npub fn main() -> i64:\n    let pair = make(40 as u64, 2 as u64)\n    return (pair.left + pair.right) as i64\n",
        ))
        .unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(7)
            .any(|bytes| bytes == [0x48, 0x8d, 0x85, 0xf0, 0xff, 0xff, 0xff]));
    }

    #[test]
    fn dbyte64_indirect_aggregate_return_passes_hidden_sret() {
        let executable = compile_concrete_linux(&project(
            "struct Pair:\n    left: u64\n    right: u64\n\nfn make(left: u64, right: u64) -> Pair:\n    return Pair(left = left, right = right)\n\npub fn main() -> i64:\n    let callback: fn(u64, u64) -> Pair = make\n    let pair = callback(40 as u64, 2 as u64)\n    return (pair.left + pair.right) as i64\n",
        ))
        .unwrap();
        assert!(executable
            .elf
            .bytes
            .windows(3)
            .any(|bytes| bytes == [0x49, 0xff, 0xd3]));
        assert!(executable
            .elf
            .bytes
            .windows(7)
            .any(|bytes| bytes[..3] == [0x48, 0x8d, 0x85]));
    }

    #[test]
    fn dbyte64_nested_and_non_qword_aggregates_use_the_general_sret_path() {
        let executable = compile_concrete_linux(&project(
            "struct Tiny:\n    value: u32\n\nstruct Wrapper:\n    inner: Tiny\n    marker: u8\n\nfn identity(value: Tiny) -> Tiny:\n    return value\n\nfn make() -> Result[Wrapper, u8]:\n    return Result[Wrapper, u8].Ok(Wrapper(inner = identity(Tiny(value = 42 as u32)), marker = 1 as u8))\n\npub fn main() -> i64:\n    let made = make()\n    match made:\n        case Result.Ok(value):\n            return value.inner.value as i64\n        case Result.Err(error):\n            return error as i64\n",
        ))
        .unwrap();
        assert!(executable.elf.bytes.starts_with(b"\x7fELF"));
        assert!(!executable.elf.bytes.is_empty());
    }

    #[test]
    fn native_mir_x86_64_object_does_not_require_executable_entry() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dbyte-x64-object-no-entry-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("module.dby");
        std::fs::write(
            &source,
            "pub fn arch_value() -> u64:\n    return 42 as u64\n",
        )
        .unwrap();
        for target in [
            crate::target::X86_64_NONE,
            crate::target::X86_64_LINUX,
            crate::target::X86_64_WINDOWS,
        ] {
            let result = compile_mir_object_source(&source, target, NativeOptimizationLevel::O1);
            let object = result.unwrap();
            assert!(!object.object.bytes.is_empty());
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn native_mir_x86_64_executable_objects_require_entry() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dbyte-x64-executable-entry-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("module.dby");
        std::fs::write(
            &source,
            "pub fn arch_value() -> u64:\n    return 42 as u64\n",
        )
        .unwrap();
        for (target, entry_name) in [
            (crate::target::X86_64_NONE, "kernel_main"),
            (crate::target::X86_64_LINUX, "main"),
            (crate::target::X86_64_WINDOWS, "main"),
        ] {
            let error =
                compile_mir_executable_object_source(&source, target, NativeOptimizationLevel::O1)
                    .expect_err("executable object must require its target entry");
            assert_eq!(error.0, format!("E_ENTRY_MISSING: missing `{entry_name}`"));
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn native_mir_x86_64_interrupt_objects_preserve_stubs_metadata_and_addresses() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dbyte-x64-interrupt-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("module.dby");
        std::fs::write(
            &source,
            "pub struct InterruptFrame:\n    rip: u64\n    cs: u64\n    rflags: u64\n\npub interrupt(no_error_code) fn breakpoint(frame: ptr[InterruptFrame]):\n    return\n\npub fn handler_address() -> usize:\n    return interrupt_address(3, breakpoint)\n",
        )
        .unwrap();
        let object = compile_mir_object_source(
            &source,
            crate::target::X86_64_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object64::parse_relocatable(&object.object.bytes).unwrap();
        let public = image
            .symbols
            .iter()
            .find(|symbol| symbol.name == "breakpoint")
            .unwrap();
        assert_eq!(public.binding, crate::object64::Binding::Global);
        assert!(image.symbols.iter().any(|symbol| {
            symbol.name == "__dbyte_interrupt_breakpoint_body"
                && symbol.binding == crate::object64::Binding::Local
        }));
        assert!(image.symbols.iter().any(|symbol| {
            symbol.name == "__dbyte_interrupt:def:no_error_code:breakpoint"
                && symbol.binding == crate::object64::Binding::Local
        }));
        assert!(image.symbols.iter().any(|symbol| {
            symbol.name == "__dbyte_interrupt:use:003:no_error_code:breakpoint"
                && symbol.binding == crate::object64::Binding::Local
        }));
        assert!(image.relocations.iter().any(|relocation| {
            relocation.symbol == "breakpoint"
                && relocation.kind == crate::object64::RelocationKind::Pc32
        }));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn native_mir_i686_drop_metadata_function_is_materialized() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/v12.1.0/string.dby");
        let input = dbyte_hir::load_source_project(&source).unwrap();
        let analyzed = dbyte_hir::analyze_project(
            &input,
            crate::target::I686_NONE.profile,
            crate::target::I686_NONE.layout,
        )
        .unwrap();
        let source_drop = analyzed
            .executable
            .semantic
            .functions
            .iter()
            .find(|function| {
                function.kind == dbyte_ast::FunctionKind::Drop
                    && analyzed
                        .link_names
                        .get(&function.symbol)
                        .is_some_and(|name| name == "destroy")
            })
            .unwrap();
        assert!(source_drop.generic.is_none());
        let drop_target = source_drop.drop_target.unwrap();
        assert!(analyzed
            .executable
            .semantic
            .definitions
            .iter()
            .any(|definition| definition.symbol == drop_target && definition.public));
        let concrete = dbyte_hir::monomorphize_project(&analyzed).unwrap();
        assert!(concrete
            .executable
            .functions
            .iter()
            .any(|function| function.key.symbol == source_drop.symbol));
        assert_eq!(concrete.link_names[&source_drop.symbol], "destroy");
        let concrete_drop = concrete
            .executable
            .functions
            .iter()
            .find(|function| function.key.symbol == source_drop.symbol)
            .unwrap();
        assert_ne!(concrete_drop.symbol_name, "destroy");
        let expected_symbol = concrete_drop.symbol_name.clone();
        let mir = crate::native_mir_lower::lower_program_to_native_mir_with_link_names(
            &concrete.executable,
            Some(&concrete.link_names),
        )
        .unwrap();
        assert!(mir
            .functions
            .iter()
            .any(|function| function.symbol == expected_symbol));
    }

    #[test]
    fn native_mir_i686_preserves_canonical_link_metadata() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/v12.1.0/array.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        assert!(object
            .object
            .bytes
            .windows(b".dbyte.drop".len())
            .any(|bytes| bytes == b".dbyte.drop"));
        assert!(object
            .object
            .bytes
            .windows(b".dbyte.generics".len())
            .any(|bytes| bytes == b".dbyte.generics"));
    }

    #[test]
    fn native_mir_i686_gdt_object_selects_segment_reload() {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native_kernel_core/gdt.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        assert!(image
            .symbols
            .iter()
            .any(|symbol| symbol.name == "gdt_install"));
    }

    #[test]
    fn native_mir_i686_task_control_addresses_preserve_checked_metadata() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_scheduler_core/scheduler.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        for marker in [
            "__dbyte_task:def:task_returned",
            "__dbyte_task:use:task_returned",
            "__dbyte_task:trampoline",
        ] {
            assert!(image.symbols.iter().any(|symbol| symbol.name == marker));
        }
        for target in ["task_returned", "__dbyte_task_trampoline"] {
            assert!(image.relocations.iter().any(|relocation| {
                image.symbols[relocation.symbol.0 as usize].name == target
                    && relocation.kind == crate::object::RelocationKind::Abs32
            }));
        }
    }

    #[test]
    fn native_mir_i686_kernel_entry_preserves_multiboot_metadata() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_memory_core/kernel.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        assert!(image
            .symbols
            .iter()
            .any(|symbol| symbol.name == "__dbyte_entry:multiboot_v1"));
        let entry = image
            .symbols
            .iter()
            .find(|symbol| symbol.name == "kernel_main")
            .expect("kernel entry symbol");
        assert_eq!(entry.binding, crate::object::SymbolBinding::Global);
    }

    #[test]
    fn native_mir_i686_memory_core_linker_addresses_are_object_relocations() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_memory_core/frames.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        for name in [
            "__dbyte_image_start",
            "__dbyte_image_end",
            "__dbyte_stack_bottom",
            "__dbyte_stack_top",
        ] {
            let symbol = image
                .symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap();
            assert_eq!(symbol.kind, crate::object::SymbolKind::Object);
            assert!(matches!(
                symbol.definition,
                crate::object::SymbolDefinition::Undefined
            ));
            assert!(image.relocations.iter().any(|relocation| {
                image.symbols[relocation.symbol.0 as usize].name == name
                    && relocation.kind == crate::object::RelocationKind::Abs32
            }));
        }
    }

    #[test]
    fn native_mir_i686_user_syscalls_emit_int80() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_user_process_core/programs/hello.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_USER,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        let text = image
            .sections
            .get(&crate::object::ObjectSection::Text)
            .unwrap();
        let crate::object::SectionContents::Bytes(text) = &text.contents else {
            panic!("i686 user text section must contain bytes");
        };
        let entry = image
            .symbols
            .iter()
            .find(|symbol| symbol.name == "user_main")
            .expect("user entry symbol");
        assert_eq!(entry.binding, crate::object::SymbolBinding::Global);
        assert!(image
            .symbols
            .iter()
            .any(|symbol| symbol.name == "__dbyte_profile:user"));
        assert_eq!(
            text.windows(2)
                .filter(|window| *window == [0xcd, 0x80])
                .count(),
            7
        );
    }

    #[test]
    fn native_mir_i686_private_root_symbols_are_local_across_objects() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native_filesystem_runtime");
        for (source_name, public_name) in [
            ("path.dby", "path_cursor_initialize"),
            ("file_handles.dby", "handles_initialize"),
        ] {
            let object = compile_mir_i686_object_source(
                &root.join(source_name),
                crate::target::I686_NONE,
                NativeOptimizationLevel::O1,
            )
            .unwrap();
            let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
            let private = image
                .symbols
                .iter()
                .find(|symbol| symbol.name == "__dbyte_m0_OK")
                .unwrap();
            assert_eq!(private.binding, crate::object::SymbolBinding::Local);
            let public = image
                .symbols
                .iter()
                .find(|symbol| symbol.name == public_name)
                .unwrap();
            assert_eq!(public.binding, crate::object::SymbolBinding::Global);
        }
    }

    #[test]
    fn native_mir_i686_task_block_trap_uses_int81_register_abi() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_blocking_core/scheduler.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        let text = image
            .sections
            .get(&crate::object::ObjectSection::Text)
            .unwrap();
        let crate::object::SectionContents::Bytes(text) = &text.contents else {
            panic!("i686 text section must contain bytes");
        };
        assert!(text.windows(2).any(|window| window == [0xcd, 0x81]));
    }

    #[test]
    fn native_mir_i686_interrupt_objects_preserve_stubs_and_metadata() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native_kernel_core");
        let exceptions = compile_mir_i686_object_source(
            &root.join("exceptions.dby"),
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let exception_image = crate::object::parse_relocatable(&exceptions.object.bytes).unwrap();
        assert!(exception_image
            .symbols
            .iter()
            .any(|symbol| symbol.name == "__dbyte_interrupt_breakpoint_body"));
        assert!(exception_image
            .symbols
            .iter()
            .any(|symbol| { symbol.name == "__dbyte_interrupt:def:no_error_code:breakpoint" }));

        let idt = compile_mir_i686_object_source(
            &root.join("idt.dby"),
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let idt_image = crate::object::parse_relocatable(&idt.object.bytes).unwrap();
        assert!(idt_image
            .symbols
            .iter()
            .any(|symbol| { symbol.name == "__dbyte_interrupt:use:003:no_error_code:breakpoint" }));
        assert!(idt_image.relocations.iter().any(|relocation| {
            idt_image.symbols[relocation.symbol.0 as usize].name == "breakpoint"
                && relocation.kind == crate::object::RelocationKind::Abs32
        }));
    }

    #[test]
    fn native_mir_i686_object_emits_only_root_module_definitions() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/serial.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        let out8 = image
            .symbols
            .iter()
            .find(|symbol| symbol.name == "out8")
            .unwrap();
        assert!(matches!(
            out8.definition,
            crate::object::SymbolDefinition::Undefined
        ));
        assert!(!image.symbols.iter().any(|symbol| {
            symbol.name == "halt"
                && matches!(
                    symbol.definition,
                    crate::object::SymbolDefinition::Defined { .. }
                )
        }));
        let init = image
            .symbols
            .iter()
            .find(|symbol| symbol.name == "init")
            .unwrap();
        assert!(matches!(
            init.definition,
            crate::object::SymbolDefinition::Defined { .. }
        ));
    }

    #[test]
    fn native_mir_i686_object_keeps_root_string_definitions() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let object = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let image = crate::object::parse_relocatable(&object.object.bytes).unwrap();
        let strings = image
            .symbols
            .iter()
            .filter(|symbol| symbol.name.starts_with("__dbyte_string_"))
            .collect::<Vec<_>>();
        assert!(!strings.is_empty());
        assert!(strings.iter().all(|symbol| matches!(
            symbol.definition,
            crate::object::SymbolDefinition::Defined { .. }
        )));
        assert!(strings
            .iter()
            .all(|symbol| symbol.binding == crate::object::SymbolBinding::Local));
    }

    #[test]
    fn native_mir_i686_project_objects_share_public_link_names() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/v12.1.0/kernel.dby");
        let objects = compile_mir_i686_project_objects(
            &root,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap();
        let string = objects
            .iter()
            .find(|(path, _)| path.file_name().is_some_and(|name| name == "string.dby"))
            .expect("string project object");
        let string_image = crate::object::parse_relocatable(&string.1.object.bytes).unwrap();
        let clone = string_image
            .symbols
            .iter()
            .find(|symbol| {
                symbol.name.starts_with("__dbyte_pub_m")
                    && symbol.name.ends_with("_clone")
                    && matches!(
                        symbol.definition,
                        crate::object::SymbolDefinition::Defined { .. }
                    )
            })
            .expect("module-scoped public string clone definition");
        assert_eq!(clone.binding, crate::object::SymbolBinding::Global);
        let clone_name = clone.name.clone();

        let kernel = objects
            .iter()
            .find(|(path, _)| path.file_name().is_some_and(|name| name == "kernel.dby"))
            .expect("kernel project object");
        let kernel_image = crate::object::parse_relocatable(&kernel.1.object.bytes).unwrap();
        let imported = kernel_image
            .symbols
            .iter()
            .find(|symbol| symbol.name == clone_name)
            .expect("kernel import of string clone");
        assert!(matches!(
            imported.definition,
            crate::object::SymbolDefinition::Undefined
        ));
        assert!(kernel_image.relocations.iter().any(|relocation| {
            kernel_image.symbols[relocation.symbol.0 as usize].name == clone_name
        }));

        let destroy = string_image
            .symbols
            .iter()
            .find(|symbol| {
                symbol.name.ends_with("destroy")
                    && matches!(
                        symbol.definition,
                        crate::object::SymbolDefinition::Defined { .. }
                    )
            })
            .expect("string destructor definition");
        assert_eq!(destroy.binding, crate::object::SymbolBinding::Global);
        let destroy_name = destroy.name.clone();
        let imported_destroy = kernel_image
            .symbols
            .iter()
            .find(|symbol| symbol.name == destroy_name)
            .expect("kernel import of string destructor");
        assert!(matches!(
            imported_destroy.definition,
            crate::object::SymbolDefinition::Undefined
        ));
        assert!(kernel_image.relocations.iter().any(|relocation| {
            kernel_image.symbols[relocation.symbol.0 as usize].name == destroy_name
        }));
    }

    #[test]
    fn native_mir_i686_object_does_not_require_kernel_entry() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("dbyte-i686-object-no-entry-{nonce}"));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("module.dby");
        std::fs::write(
            &source,
            "pub fn arch_value() -> u32:\n    return 42 as u32\n",
        )
        .unwrap();
        let result = compile_mir_i686_object_source(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        );
        std::fs::remove_dir_all(&root).unwrap();
        let object = result.unwrap();
        assert!(object.object.bytes.starts_with(b"\x7fELF"));
    }
}
