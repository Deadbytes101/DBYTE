use crate::archive::{write_archive, ArchiveMember};
use crate::dbup::{pack_user_bundle, BundleInput};
use crate::linker::{link_object_bytes, link_user_object_bytes, LinkInput};
use crate::native_mir::NativeOptimizationLevel;
use crate::object::parse_relocatable;
use crate::{Diagnostic, EmitKind, TARGET, USER_TARGET};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const USAGE: &str = "usage:\n  dbytec build <source.dby> --target i686-dbyte-none --emit elf|bin|obj|staticlib -o <output> [--opt-level 0|1] [--dump-ir] [--dump-native-mir] [--dump-machine-code]\n  dbytec build <source.dby> --target i686-dbyte-user --emit elf|obj -o <output> [--opt-level 0|1] [--dump-ir] [--dump-native-mir] [--dump-machine-code]\n  dbytec build <source.dby> --target x86_64-dbyte-none --emit obj|staticlib|elf|bin -o <output> [--opt-level 0|1] [--dump-ir] [--dump-native-mir] [--dump-machine-code]\n  dbytec build <source.dby> --target x86_64-dbyte-linux --emit obj|staticlib|elf -o <output> [--opt-level 0|1] [--dump-ir] [--dump-native-mir] [--dump-machine-code]\n  dbytec build <source.dby> --target x86_64-dbyte-windows --emit obj|staticlib|exe -o <output> [--opt-level 0|1] [--dump-ir] [--dump-native-mir] [--dump-machine-code]\n  dbytec assemble <source.dbtasm> --target x86_64-dbyte-none -o <object.o>\n  dbytec check-portable <source.dby>\n  dbytec pack-user NAME=ELF... -o <programs.dbup>\n  dbytec link --target <triple> <object.o|archive.a>... -o <output>\n  dbytec link --target x86_64-dbyte-none --flat-load-address <address> --flat-max-size <bytes> [--flat-section-alignment <bytes>] [--entry <symbol>] <object.o|archive.a>... -o <output>";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum CliError {
    Argument(String),
    Input {
        path: PathBuf,
        kind: io::ErrorKind,
    },
    Compile {
        path: PathBuf,
        diagnostic: String,
    },
    Link {
        diagnostic: String,
    },
    Output {
        path: PathBuf,
        action: &'static str,
        kind: io::ErrorKind,
    },
    Stdout(io::ErrorKind),
}

impl CliError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Argument(_) => 2,
            Self::Input { .. }
            | Self::Compile { .. }
            | Self::Link { .. }
            | Self::Output { .. }
            | Self::Stdout(_) => 1,
        }
    }

    fn argument(message: impl Into<String>) -> Self {
        Self::Argument(message.into())
    }

    fn compile(path: &Path, diagnostic: impl Into<String>) -> Self {
        Self::Compile {
            path: path.to_path_buf(),
            diagnostic: diagnostic.into(),
        }
    }

    fn link(diagnostic: impl Into<String>) -> Self {
        Self::Link {
            diagnostic: diagnostic.into(),
        }
    }

    fn output(path: &Path, action: &'static str, error: &io::Error) -> Self {
        Self::Output {
            path: path.to_path_buf(),
            action,
            kind: error.kind(),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Argument(message) => write!(formatter, "error[cli]: {message}\n{USAGE}"),
            Self::Input { path, kind } => write!(
                formatter,
                "error[input]: could not read '{}': {kind:?}",
                path.display()
            ),
            Self::Compile { path, diagnostic } => {
                let path_text = path.to_string_lossy();
                if diagnostic.contains(path_text.as_ref()) {
                    formatter.write_str(diagnostic)
                } else {
                    write!(formatter, "{}: {diagnostic}", path.display())
                }
            }
            Self::Link { diagnostic } => formatter.write_str(diagnostic),
            Self::Output { path, action, kind } => write!(
                formatter,
                "error[output]: could not {action} '{}': {kind:?}",
                path.display()
            ),
            Self::Stdout(kind) => {
                write!(formatter, "error[output]: could not write dump: {kind:?}")
            }
        }
    }
}

impl std::error::Error for CliError {}

#[derive(Debug)]
struct BuildOptions {
    source: PathBuf,
    target: BuildTarget,
    emit: EmitKind,
    output: PathBuf,
    optimization: NativeOptimizationLevel,
    dump_ir: bool,
    dump_native_mir: bool,
    dump_machine_code: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuildTarget {
    Kernel,
    User,
    X86_64None,
    X86_64User,
    X86_64Linux,
    X86_64Windows,
}

#[derive(Debug)]
struct LinkOptions {
    inputs: Vec<PathBuf>,
    output: PathBuf,
    target: Option<crate::target::TargetSpec>,
    flat_load_address: Option<u64>,
    flat_max_size: Option<u64>,
    flat_section_alignment: Option<u64>,
    entry: Option<String>,
}

#[derive(Debug)]
struct PackUserOptions {
    inputs: Vec<(String, PathBuf)>,
    output: PathBuf,
}

#[derive(Debug)]
struct AssembleOptions {
    source: PathBuf,
    output: PathBuf,
    target: crate::target::TargetSpec,
}

#[derive(Debug)]
struct BuildProduct {
    bytes: Vec<u8>,
    ir_dump: String,
    native_mir_dump: String,
    machine_dump: String,
}

/// Dispatch the standalone `dbytec` command while retaining the historical
/// direct-source form accepted by `run_build_args`.
pub fn run_args(args: &[String]) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("link") => run_link_args(args),
        Some("assemble") => run_assemble_args(args),
        Some("check-portable") => run_check_portable_args(args),
        Some("pack-user") => run_pack_user_args(args),
        _ => run_build_args(args),
    }
}

pub fn run_assemble_args(args: &[String]) -> Result<(), CliError> {
    let options = parse_assemble_args(args)?;
    reject_source_as_output(&options.source, &options.output)?;
    let source = fs::read_to_string(&options.source).map_err(|error| CliError::Input {
        path: options.source.clone(),
        kind: error.kind(),
    })?;
    let object = crate::dbtasm64::assemble(&source, options.target)
        .map_err(|failure| CliError::compile(&options.source, failure.0))?;
    replace_output_transactionally(&options.output, &object.bytes)
}

pub fn run_check_portable_args(args: &[String]) -> Result<(), CliError> {
    if args.len() != 2 {
        return Err(CliError::argument(
            "check-portable requires exactly one source path",
        ));
    }
    let source = PathBuf::from(&args[1]);
    fs::read_to_string(&source).map_err(|error| CliError::Input {
        path: source.clone(),
        kind: error.kind(),
    })?;
    let project = dbyte_hir::load_source_project(&source)
        .map_err(|diagnostics| CliError::compile(&source, render_hir_diagnostics(diagnostics)))?;
    let report = dbyte_hir::validate_portable_project(&project)
        .map_err(|diagnostics| CliError::compile(&source, render_hir_diagnostics(diagnostics)))?;
    writeln!(
        io::stdout().lock(),
        "DBYTE STRICT PORTABLE: PASS profiles={} interface={:016x}",
        report.profile_count,
        report.interface_fingerprint
    )
    .map_err(|error| CliError::Stdout(error.kind()))?;
    Ok(())
}

pub fn run_pack_user_args(args: &[String]) -> Result<(), CliError> {
    let options = parse_pack_user_args(args)?;
    let mut inputs = Vec::with_capacity(options.inputs.len());
    for (name, path) in &options.inputs {
        reject_source_as_output(path, &options.output)?;
        let payload = fs::read(path).map_err(|error| CliError::Input {
            path: path.clone(),
            kind: error.kind(),
        })?;
        validate_elf32_kind(path, &payload, ElfKind::Executable, "DBUP1 packer")?;
        validate_user_executable(path, &payload)?;
        inputs.push(BundleInput {
            name: name.clone(),
            payload,
        });
    }
    let bytes = pack_user_bundle(&inputs).map_err(|error| CliError::link(error.to_string()))?;
    replace_output_transactionally(&options.output, &bytes)
}

pub fn run_build_args(args: &[String]) -> Result<(), CliError> {
    let options = parse_build_args(args)?;
    // Preserve the original CLI's input-error contract before the systems
    // compiler canonicalizes and loads the complete module graph.
    let source_text = fs::read_to_string(&options.source).map_err(|error| CliError::Input {
        path: options.source.clone(),
        kind: error.kind(),
    })?;

    reject_source_as_output(&options.source, &options.output)?;

    let mut product = compile_build_product(&options, &source_text)?;
    if options.dump_native_mir {
        product.native_mir_dump = compile_native_mir_dump(&options)?;
    }
    validate_selected_output(
        &options.source,
        options.target,
        &options.emit,
        &product.bytes,
    )?;

    write_requested_dumps(&product, &options)?;
    if options.target == BuildTarget::X86_64Linux && options.emit == EmitKind::Elf {
        replace_executable_output_transactionally(&options.output, &product.bytes)
    } else {
        replace_output_transactionally(&options.output, &product.bytes)
    }
}

pub fn run_link_args(args: &[String]) -> Result<(), CliError> {
    let options = parse_link_args(args)?;
    if options.flat_load_address.is_none()
        && (options.flat_max_size.is_some()
            || options.flat_section_alignment.is_some()
            || options.entry.is_some())
    {
        return Err(CliError::argument(
            "flat-link options require --flat-load-address",
        ));
    }
    let mut raw_inputs = Vec::with_capacity(options.inputs.len());
    for path in &options.inputs {
        reject_source_as_output(path, &options.output)?;
        let bytes = fs::read(path).map_err(|error| CliError::Input {
            path: path.clone(),
            kind: error.kind(),
        })?;
        raw_inputs.push((path.to_string_lossy().into_owned(), bytes));
    }

    if let Some(target) = options.target {
        if target.machine == crate::target::MachineTarget::X86_64 {
            let inputs: Vec<_> = raw_inputs
                .into_iter()
                .map(|(name, bytes)| crate::linker64::LinkInput::new(name, bytes))
                .collect();
            if let Some(load_address) = options.flat_load_address {
                if target != crate::target::X86_64_NONE {
                    return Err(CliError::argument(
                        "flat linking requires --target x86_64-dbyte-none",
                    ));
                }
                let maximum_size = options
                    .flat_max_size
                    .ok_or_else(|| CliError::argument("flat linking requires --flat-max-size"))?;
                let image = crate::linker64::link_flat_objects(
                    &inputs,
                    options.entry.as_deref().unwrap_or("kernel_main"),
                    load_address,
                    options.flat_section_alignment.unwrap_or(16),
                    maximum_size,
                )
                .map_err(|error| CliError::link(error.0))?;
                return replace_output_transactionally(&options.output, &image.bytes);
            }
            if options.flat_max_size.is_some() {
                return Err(CliError::argument(
                    "--flat-max-size requires --flat-load-address",
                ));
            }
            if target == crate::target::X86_64_WINDOWS {
                let image = crate::modern_native::link_windows_object_bytes(&inputs)
                    .map_err(|error| CliError::link(error.0))?;
                crate::pe64::inspect(&image.bytes).map_err(|error| CliError::link(error.0))?;
                return replace_output_transactionally(&options.output, &image.bytes);
            }
            let image = crate::modern_native::link_object_bytes(&inputs, target)
                .map_err(|error| CliError::link(error.0))?;
            crate::elf64::inspect(&image.bytes).map_err(|error| CliError::link(error.0))?;
            if target == crate::target::X86_64_LINUX {
                return replace_executable_output_transactionally(&options.output, &image.bytes);
            }
            return replace_output_transactionally(&options.output, &image.bytes);
        }
    }

    let inputs: Vec<_> = raw_inputs
        .into_iter()
        .map(|(name, bytes)| LinkInput::new(name, bytes))
        .collect();

    let linked =
        link_object_bytes(&inputs).map_err(|diagnostic| CliError::link(diagnostic.to_string()))?;
    validate_elf32_kind(&options.output, &linked.elf, ElfKind::Executable, "linker")?;
    replace_output_transactionally(&options.output, &linked.elf)
}

fn compile_build_product(
    options: &BuildOptions,
    _source_text: &str,
) -> Result<BuildProduct, CliError> {
    if matches!(
        options.target,
        BuildTarget::X86_64None
            | BuildTarget::X86_64User
            | BuildTarget::X86_64Linux
            | BuildTarget::X86_64Windows
    ) {
        let target = match options.target {
            BuildTarget::X86_64None => crate::target::X86_64_NONE,
            BuildTarget::X86_64User => crate::target::X86_64_USER,
            BuildTarget::X86_64Linux => crate::target::X86_64_LINUX,
            BuildTarget::X86_64Windows => crate::target::X86_64_WINDOWS,
            BuildTarget::Kernel | BuildTarget::User => unreachable!(),
        };
        return match options.emit {
            EmitKind::Elf if options.target != BuildTarget::X86_64Windows => {
                let artifact = crate::modern_native::compile_mir_source(
                    &options.source,
                    target,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                Ok(BuildProduct {
                    bytes: artifact.elf.bytes,
                    ir_dump: artifact.hir_dump,
                    native_mir_dump: String::new(),
                    machine_dump: artifact.machine_dump,
                })
            }
            EmitKind::Elf => {
                let object = crate::modern_native::compile_mir_executable_object_source(
                    &options.source,
                    target,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                let artifact = crate::modern_native::link_windows_object_bytes(&[
                    crate::linker64::LinkInput::new("m0000.obj", object.object.bytes),
                ])
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                Ok(BuildProduct {
                    bytes: artifact.bytes,
                    ir_dump: object.hir_dump,
                    native_mir_dump: String::new(),
                    machine_dump: format!(
                        "{}PE32+ entry=0x{:016x} imports={}\n",
                        object.machine_dump, artifact.entry, artifact.import_count
                    ),
                })
            }
            EmitKind::Obj => {
                let artifact = crate::modern_native::compile_mir_object_source(
                    &options.source,
                    target,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                Ok(BuildProduct {
                    bytes: artifact.object.bytes,
                    ir_dump: artifact.hir_dump,
                    native_mir_dump: String::new(),
                    machine_dump: artifact.machine_dump,
                })
            }
            EmitKind::StaticLib if options.target == BuildTarget::X86_64User => Err(
                CliError::argument("x86_64-dbyte-user does not support --emit staticlib"),
            ),
            EmitKind::StaticLib => {
                let artifact = crate::modern_native::compile_mir_object_source(
                    &options.source,
                    target,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                let bytes = crate::archive::write_archive(&[crate::archive::ArchiveMember {
                    name: "m0000.o".into(),
                    bytes: artifact.object.bytes,
                }])
                .map_err(|error| CliError::compile(&options.source, error.to_string()))?;
                Ok(BuildProduct {
                    bytes,
                    ir_dump: artifact.hir_dump,
                    native_mir_dump: String::new(),
                    machine_dump: artifact.machine_dump,
                })
            }
            EmitKind::Bin if options.target == BuildTarget::X86_64None => {
                let artifact = crate::modern_native::compile_mir_source(
                    &options.source,
                    target,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                Ok(BuildProduct {
                    // The address tag describes the same deterministic bytes
                    // as a flat load at 1 MiB, with no external objcopy step.
                    bytes: artifact.elf.bytes,
                    ir_dump: artifact.hir_dump,
                    native_mir_dump: String::new(),
                    machine_dump: artifact.machine_dump,
                })
            }
            EmitKind::Bin => Err(CliError::argument(
                "DBYTE64 BIN emission is supported only for x86_64-dbyte-none",
            )),
        };
    }
    if options.target == BuildTarget::Kernel {
        return match options.emit {
            EmitKind::Obj | EmitKind::StaticLib => {
                let artifact = crate::modern_native::compile_mir_i686_object_source(
                    &options.source,
                    crate::target::I686_NONE,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                if options.emit == EmitKind::Obj {
                    Ok(BuildProduct {
                        bytes: artifact.object.bytes,
                        ir_dump: artifact.hir_dump,
                        native_mir_dump: String::new(),
                        machine_dump: artifact.machine_dump,
                    })
                } else {
                    let bytes = write_archive(&[ArchiveMember {
                        name: "m0000.o".into(),
                        bytes: artifact.object.bytes,
                    }])
                    .map_err(|diagnostic| compile_diagnostic(&options.source, diagnostic))?;
                    Ok(BuildProduct {
                        bytes,
                        ir_dump: artifact.hir_dump,
                        native_mir_dump: String::new(),
                        machine_dump: artifact.machine_dump,
                    })
                }
            }
            EmitKind::Elf | EmitKind::Bin => {
                let objects = crate::modern_native::compile_mir_i686_project_objects(
                    &options.source,
                    crate::target::I686_NONE,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                let mut inputs = Vec::with_capacity(objects.len());
                let mut ir_dump = String::new();
                let mut machine_dump = String::new();
                for (source, artifact) in objects {
                    let stem = source
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .ok_or_else(|| {
                            CliError::compile(&source, "source module has no UTF-8 file stem")
                        })?;
                    inputs.push(LinkInput::new(format!("{stem}.o"), artifact.object.bytes));
                    append_dump(&mut ir_dump, &artifact.hir_dump);
                    append_dump(&mut machine_dump, &artifact.machine_dump);
                }
                let linked = link_object_bytes(&inputs).map_err(|diagnostic| {
                    if diagnostic.category
                        == crate::diagnostics::DiagnosticCategory::UndefinedSymbol
                        && diagnostic.message == "undefined entry symbol 'kernel_main'"
                    {
                        CliError::compile(&options.source, "E_ENTRY_MISSING: missing `kernel_main`")
                    } else {
                        CliError::compile(&options.source, diagnostic.to_string())
                    }
                })?;
                append_dump(&mut machine_dump, &linked.dump);
                Ok(BuildProduct {
                    bytes: if options.emit == EmitKind::Elf {
                        linked.elf
                    } else {
                        linked.bin
                    },
                    ir_dump,
                    native_mir_dump: String::new(),
                    machine_dump,
                })
            }
        };
    }
    if options.target == BuildTarget::User {
        return match options.emit {
            EmitKind::Bin => Err(CliError::argument(
                "i686-dbyte-user does not support --emit bin",
            )),
            EmitKind::StaticLib => Err(CliError::argument(
                "i686-dbyte-user does not support --emit staticlib",
            )),
            EmitKind::Obj => {
                let artifact = crate::modern_native::compile_mir_i686_object_source(
                    &options.source,
                    crate::target::I686_USER,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                Ok(BuildProduct {
                    bytes: artifact.object.bytes,
                    ir_dump: artifact.hir_dump,
                    native_mir_dump: String::new(),
                    machine_dump: artifact.machine_dump,
                })
            }
            EmitKind::Elf => {
                let artifact = crate::modern_native::compile_mir_i686_executable_object_source(
                    &options.source,
                    crate::target::I686_USER,
                    options.optimization,
                )
                .map_err(|error| CliError::compile(&options.source, error.0))?;
                let linked =
                    link_user_object_bytes(&[LinkInput::new("m0000.o", artifact.object.bytes)])
                        .map_err(|diagnostic| compile_diagnostic(&options.source, diagnostic))?;
                let mut machine_dump = artifact.machine_dump;
                append_dump(&mut machine_dump, &linked.dump);
                Ok(BuildProduct {
                    bytes: linked.elf,
                    ir_dump: artifact.hir_dump,
                    native_mir_dump: String::new(),
                    machine_dump,
                })
            }
        };
    }
    unreachable!("every native build target is routed through Native MIR")
}

fn compile_native_mir_dump(options: &BuildOptions) -> Result<String, CliError> {
    let target = match options.target {
        BuildTarget::Kernel => crate::target::I686_NONE,
        BuildTarget::User => crate::target::I686_USER,
        BuildTarget::X86_64None => crate::target::X86_64_NONE,
        BuildTarget::X86_64User => crate::target::X86_64_USER,
        BuildTarget::X86_64Linux => crate::target::X86_64_LINUX,
        BuildTarget::X86_64Windows => crate::target::X86_64_WINDOWS,
    };
    let input = dbyte_hir::load_source_project(&options.source).map_err(|diagnostics| {
        CliError::compile(&options.source, render_hir_diagnostics(diagnostics))
    })?;
    let analyzed = dbyte_hir::analyze_project(&input, target.profile, target.layout).map_err(
        |diagnostics| CliError::compile(&options.source, render_hir_diagnostics(diagnostics)),
    )?;
    let concrete = dbyte_hir::monomorphize_project(&analyzed)
        .map_err(|diagnostic| CliError::compile(&options.source, diagnostic.to_string()))?;
    dbyte_hir::validate_project_profile(&concrete, target.profile)
        .map_err(|diagnostic| CliError::compile(&options.source, diagnostic.to_string()))?;
    let mut mir = crate::native_mir_lower::lower_program_to_native_mir_with_link_names(
        &concrete.executable,
        Some(&concrete.link_names),
    )
    .map_err(|error| CliError::compile(&options.source, error.to_string()))?;
    mir.optimize(options.optimization)
        .map_err(|error| CliError::compile(&options.source, error.to_string()))?;
    Ok(format!(
        "optimization={}\n{}",
        options.optimization,
        mir.dump()
    ))
}

fn render_hir_diagnostics(diagnostics: Vec<dbyte_hir::Diagnostic>) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn compile_diagnostic(source: &Path, diagnostic: Diagnostic) -> CliError {
    let path_label = source.to_string_lossy();
    let diagnostic = if diagnostic.path.is_some() {
        diagnostic
    } else {
        diagnostic.with_path(path_label.as_ref())
    };
    CliError::compile(source, diagnostic.to_string())
}

fn append_dump(output: &mut String, dump: &str) {
    if !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }
    output.push_str(dump);
    if !output.ends_with('\n') {
        output.push('\n');
    }
}

fn parse_build_args(args: &[String]) -> Result<BuildOptions, CliError> {
    let args = match args.first().map(String::as_str) {
        Some("build") => &args[1..],
        _ => args,
    };

    let source = args
        .first()
        .filter(|argument| !argument.starts_with('-'))
        .ok_or_else(|| CliError::argument("missing source file"))?;

    let mut target: Option<String> = None;
    let mut emit: Option<EmitKind> = None;
    let mut output: Option<PathBuf> = None;
    let mut optimization: Option<NativeOptimizationLevel> = None;
    let mut dump_ir = false;
    let mut dump_native_mir = false;
    let mut dump_machine_code = false;
    let mut index = 1;

    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "--target" => {
                reject_duplicate(target.is_some(), "--target")?;
                target = Some(take_option_value(args, &mut index, "--target")?.to_owned());
            }
            "--emit" => {
                reject_duplicate(emit.is_some(), "--emit")?;
                let value = take_option_value(args, &mut index, "--emit")?;
                emit = Some(parse_emit(value)?);
            }
            "-o" | "--output" => {
                reject_duplicate(output.is_some(), "-o/--output")?;
                output = Some(PathBuf::from(take_option_value(
                    args, &mut index, argument,
                )?));
            }
            "--opt-level" => {
                reject_duplicate(optimization.is_some(), "--opt-level")?;
                let value = take_option_value(args, &mut index, "--opt-level")?;
                optimization = Some(
                    NativeOptimizationLevel::parse(value)
                        .map_err(|error| CliError::argument(error.to_string()))?,
                );
            }
            "--dump-ir" => {
                reject_duplicate(dump_ir, "--dump-ir")?;
                dump_ir = true;
            }
            "--dump-native-mir" => {
                reject_duplicate(dump_native_mir, "--dump-native-mir")?;
                dump_native_mir = true;
            }
            "--dump-machine-code" => {
                reject_duplicate(dump_machine_code, "--dump-machine-code")?;
                dump_machine_code = true;
            }
            _ if argument.starts_with("--target=") => {
                reject_duplicate(target.is_some(), "--target")?;
                target = Some(argument["--target=".len()..].to_owned());
            }
            _ if argument.starts_with("--emit=") => {
                reject_duplicate(emit.is_some(), "--emit")?;
                emit = Some(parse_emit(&argument["--emit=".len()..])?);
            }
            _ if argument.starts_with("--opt-level=") => {
                reject_duplicate(optimization.is_some(), "--opt-level")?;
                optimization = Some(
                    NativeOptimizationLevel::parse(&argument["--opt-level=".len()..])
                        .map_err(|error| CliError::argument(error.to_string()))?,
                );
            }
            _ if argument.starts_with('-') => {
                return Err(CliError::argument(format!("unknown option '{argument}'")));
            }
            _ => {
                return Err(CliError::argument(format!(
                    "unexpected argument '{argument}'"
                )));
            }
        }
        index += 1;
    }

    let target = target.ok_or_else(|| CliError::argument("missing required --target"))?;
    let target = match target.as_str() {
        TARGET => BuildTarget::Kernel,
        USER_TARGET => BuildTarget::User,
        "x86_64-dbyte-none" => BuildTarget::X86_64None,
        "x86_64-dbyte-user" => BuildTarget::X86_64User,
        "x86_64-dbyte-linux" => BuildTarget::X86_64Linux,
        "x86_64-dbyte-windows" => BuildTarget::X86_64Windows,
        _ => {
            return Err(CliError::argument(format!(
                "unsupported target '{target}'; expected '{TARGET}', '{USER_TARGET}', 'x86_64-dbyte-none', 'x86_64-dbyte-user', 'x86_64-dbyte-linux', or 'x86_64-dbyte-windows'"
            )))
        }
    };

    let emit = emit.ok_or_else(|| CliError::argument("missing required --emit"))?;
    let output = output.ok_or_else(|| CliError::argument("missing required -o/--output"))?;

    Ok(BuildOptions {
        source: PathBuf::from(source),
        target,
        emit,
        output,
        optimization: optimization.unwrap_or_default(),
        dump_ir,
        dump_native_mir,
        dump_machine_code,
    })
}

fn parse_link_args(args: &[String]) -> Result<LinkOptions, CliError> {
    let args = match args.first().map(String::as_str) {
        Some("link") => &args[1..],
        _ => args,
    };

    let mut inputs = Vec::new();
    let mut output: Option<PathBuf> = None;
    let mut target = None;
    let mut flat_load_address = None;
    let mut flat_max_size = None;
    let mut flat_section_alignment = None;
    let mut entry = None;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "--target" => {
                reject_duplicate(target.is_some(), "--target")?;
                let value = take_option_value(args, &mut index, "--target")?;
                target = Some(crate::target::parse_target(value).ok_or_else(|| {
                    CliError::argument(format!("unsupported link target '{value}'"))
                })?);
            }
            _ if argument.starts_with("--target=") => {
                reject_duplicate(target.is_some(), "--target")?;
                let value = &argument["--target=".len()..];
                target = Some(crate::target::parse_target(value).ok_or_else(|| {
                    CliError::argument(format!("unsupported link target '{value}'"))
                })?);
            }
            "--flat-load-address" => {
                reject_duplicate(flat_load_address.is_some(), "--flat-load-address")?;
                flat_load_address = Some(parse_u64_option(
                    take_option_value(args, &mut index, "--flat-load-address")?,
                    "--flat-load-address",
                )?);
            }
            "--flat-max-size" => {
                reject_duplicate(flat_max_size.is_some(), "--flat-max-size")?;
                flat_max_size = Some(parse_u64_option(
                    take_option_value(args, &mut index, "--flat-max-size")?,
                    "--flat-max-size",
                )?);
            }
            "--flat-section-alignment" => {
                reject_duplicate(flat_section_alignment.is_some(), "--flat-section-alignment")?;
                flat_section_alignment = Some(parse_u64_option(
                    take_option_value(args, &mut index, "--flat-section-alignment")?,
                    "--flat-section-alignment",
                )?);
            }
            "--entry" => {
                reject_duplicate(entry.is_some(), "--entry")?;
                entry = Some(take_option_value(args, &mut index, "--entry")?.to_owned());
            }
            "-o" | "--output" => {
                reject_duplicate(output.is_some(), "-o/--output")?;
                output = Some(PathBuf::from(take_option_value(
                    args, &mut index, argument,
                )?));
            }
            _ if argument.starts_with('-') => {
                return Err(CliError::argument(format!("unknown option '{argument}'")));
            }
            _ => inputs.push(PathBuf::from(argument)),
        }
        index += 1;
    }

    if inputs.is_empty() {
        return Err(CliError::argument(
            "link requires at least one ELF32 object input",
        ));
    }
    let output = output.ok_or_else(|| CliError::argument("missing required -o/--output"))?;
    Ok(LinkOptions {
        inputs,
        output,
        target,
        flat_load_address,
        flat_max_size,
        flat_section_alignment,
        entry,
    })
}

fn parse_assemble_args(args: &[String]) -> Result<AssembleOptions, CliError> {
    let args = match args.first().map(String::as_str) {
        Some("assemble") => &args[1..],
        _ => args,
    };
    let mut source = None;
    let mut output = None;
    let mut target = None;
    let mut index = 0;
    while index < args.len() {
        let argument = args[index].as_str();
        match argument {
            "--target" => {
                reject_duplicate(target.is_some(), "--target")?;
                let value = take_option_value(args, &mut index, "--target")?;
                target = Some(crate::target::parse_target(value).ok_or_else(|| {
                    CliError::argument(format!("unsupported assemble target '{value}'"))
                })?);
            }
            "-o" | "--output" => {
                reject_duplicate(output.is_some(), "-o/--output")?;
                output = Some(PathBuf::from(take_option_value(
                    args, &mut index, argument,
                )?));
            }
            _ if argument.starts_with('-') => {
                return Err(CliError::argument(format!("unknown option '{argument}'")));
            }
            _ => {
                if source.replace(PathBuf::from(argument)).is_some() {
                    return Err(CliError::argument(
                        "assemble accepts exactly one source input",
                    ));
                }
            }
        }
        index += 1;
    }
    let source = source.ok_or_else(|| CliError::argument("assemble requires a source input"))?;
    let output = output.ok_or_else(|| CliError::argument("missing required -o/--output"))?;
    let target = target.ok_or_else(|| CliError::argument("assemble requires --target"))?;
    if target != crate::target::X86_64_NONE {
        return Err(CliError::argument(
            "assemble currently supports only x86_64-dbyte-none",
        ));
    }
    Ok(AssembleOptions {
        source,
        output,
        target,
    })
}

fn parse_u64_option(value: &str, option: &str) -> Result<u64, CliError> {
    let parsed = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        value.parse()
    };
    parsed.map_err(|_| CliError::argument(format!("{option} requires a u64 value")))
}

fn parse_pack_user_args(args: &[String]) -> Result<PackUserOptions, CliError> {
    let args = match args.first().map(String::as_str) {
        Some("pack-user") => &args[1..],
        _ => args,
    };
    let mut inputs = Vec::new();
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if matches!(argument.as_str(), "-o" | "--output") {
            reject_duplicate(output.is_some(), "-o/--output")?;
            output = Some(PathBuf::from(take_option_value(
                args, &mut index, argument,
            )?));
        } else if argument.starts_with('-') {
            return Err(CliError::argument(format!("unknown option '{argument}'")));
        } else {
            let (name, path) = argument.split_once('=').ok_or_else(|| {
                CliError::argument(format!("pack-user input '{argument}' must be NAME=ELF"))
            })?;
            if name.is_empty() || path.is_empty() {
                return Err(CliError::argument(format!(
                    "pack-user input '{argument}' must be NAME=ELF"
                )));
            }
            inputs.push((name.to_owned(), PathBuf::from(path)));
        }
        index += 1;
    }
    if inputs.is_empty() {
        return Err(CliError::argument(
            "pack-user requires at least one NAME=ELF input",
        ));
    }
    let output = output.ok_or_else(|| CliError::argument("missing required -o/--output"))?;
    Ok(PackUserOptions { inputs, output })
}

fn reject_duplicate(already_seen: bool, option: &str) -> Result<(), CliError> {
    if already_seen {
        Err(CliError::argument(format!(
            "option '{option}' specified more than once"
        )))
    } else {
        Ok(())
    }
}

fn take_option_value<'a>(
    args: &'a [String],
    index: &mut usize,
    option: &str,
) -> Result<&'a str, CliError> {
    *index += 1;
    args.get(*index)
        .map(String::as_str)
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| CliError::argument(format!("option '{option}' requires a value")))
}

fn parse_emit(value: &str) -> Result<EmitKind, CliError> {
    match value {
        "elf" => Ok(EmitKind::Elf),
        "bin" => Ok(EmitKind::Bin),
        "obj" => Ok(EmitKind::Obj),
        "staticlib" => Ok(EmitKind::StaticLib),
        "exe" => Ok(EmitKind::Elf),
        _ => Err(CliError::argument(format!(
            "unsupported emit kind '{value}'; expected 'elf', 'bin', 'obj', 'staticlib', or 'exe'"
        ))),
    }
}

fn reject_source_as_output(source: &Path, output: &Path) -> Result<(), CliError> {
    if let (Ok(source), Ok(output)) = (fs::canonicalize(source), fs::canonicalize(output)) {
        if source == output {
            return Err(CliError::argument(
                "source and output paths must be different",
            ));
        }
    }
    Ok(())
}

fn validate_selected_output(
    source: &Path,
    target: BuildTarget,
    emit: &EmitKind,
    bytes: &[u8],
) -> Result<(), CliError> {
    if matches!(
        target,
        BuildTarget::X86_64None
            | BuildTarget::X86_64User
            | BuildTarget::X86_64Linux
            | BuildTarget::X86_64Windows
    ) {
        return match emit {
            EmitKind::Elf if target == BuildTarget::X86_64Windows => {
                crate::pe64::inspect(bytes).map_err(|error| CliError::compile(source, error.0))
            }
            EmitKind::Elf => crate::elf64::inspect(bytes)
                .and_then(|info| {
                    if info.object_type == 2 && !info.has_interp && !info.has_dynamic {
                        Ok(info)
                    } else {
                        Err(crate::elf64::Elf64Error(
                            "E_ELF64_STATIC: expected static ET_EXEC".into(),
                        ))
                    }
                })
                .map(|_| ())
                .map_err(|error| CliError::compile(source, error.0)),
            EmitKind::Obj if target == BuildTarget::X86_64Windows => {
                crate::coff64::parse_object(bytes)
                    .map(|_| ())
                    .map_err(|error| CliError::compile(source, error.0))
            }
            EmitKind::Obj => crate::object64::parse_relocatable(bytes)
                .and_then(|object| {
                    if object.target
                        == match target {
                            BuildTarget::X86_64None => crate::target::X86_64_NONE,
                            BuildTarget::X86_64User => crate::target::X86_64_USER,
                            BuildTarget::X86_64Linux => crate::target::X86_64_LINUX,
                            BuildTarget::X86_64Windows => crate::target::X86_64_WINDOWS,
                            BuildTarget::Kernel | BuildTarget::User => unreachable!(),
                        }
                    {
                        Ok(object)
                    } else {
                        Err(crate::object64::ObjectError(
                            "E_TARGET_MISMATCH: emitted object target differs from request".into(),
                        ))
                    }
                })
                .map(|_| ())
                .map_err(|error| CliError::compile(source, error.0)),
            EmitKind::StaticLib => crate::archive::parse_archive(bytes)
                .map(|_| ())
                .map_err(|error| CliError::compile(source, error.to_string())),
            EmitKind::Bin if target == BuildTarget::X86_64None => crate::elf64::inspect(bytes)
                .map(|_| ())
                .map_err(|error| CliError::compile(source, error.0)),
            EmitKind::Bin => Err(CliError::compile(
                source,
                "E_MODERN_EMIT: BIN is supported only for x86_64-dbyte-none",
            )),
        };
    }
    match emit {
        EmitKind::Bin if bytes.is_empty() => Err(CliError::compile(
            source,
            "error[native-output]: compiler produced an empty binary",
        )),
        EmitKind::Bin => Ok(()),
        EmitKind::Elf => validate_elf32_kind(source, bytes, ElfKind::Executable, "compiler"),
        EmitKind::Obj => {
            validate_elf32_kind(source, bytes, ElfKind::Relocatable, "compiler")?;
            parse_relocatable(bytes)
                .map(|_| ())
                .map_err(|diagnostic| compile_diagnostic(source, diagnostic))
        }
        EmitKind::StaticLib => crate::archive::parse_archive(bytes)
            .map(|_| ())
            .map_err(|error| {
                CliError::compile(source, format!("compiler emitted invalid archive: {error}"))
            }),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ElfKind {
    Relocatable,
    Executable,
}

impl ElfKind {
    const fn elf_type(self) -> u16 {
        match self {
            Self::Relocatable => 1,
            Self::Executable => 2,
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Relocatable => "relocatable object",
            Self::Executable => "executable",
        }
    }
}

fn validate_elf32_kind(
    path: &Path,
    bytes: &[u8],
    expected: ElfKind,
    producer: &str,
) -> Result<(), CliError> {
    let valid = bytes.len() >= 52
        && bytes.starts_with(b"\x7fELF")
        && bytes[4] == 1
        && bytes[5] == 1
        && bytes[6] == 1
        && u16::from_le_bytes([bytes[16], bytes[17]]) == expected.elf_type()
        && u16::from_le_bytes([bytes[18], bytes[19]]) == 3
        && u32::from_le_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]) == 1
        && u16::from_le_bytes([bytes[40], bytes[41]]) == 52;

    if valid {
        Ok(())
    } else {
        Err(CliError::compile(
            path,
            format!(
                "error[elf32]: {producer} produced an invalid ELF32/i386 {}",
                expected.description()
            ),
        ))
    }
}

fn validate_user_executable(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let read_u16 = |offset: usize| {
        bytes
            .get(offset..offset + 2)
            .map(|value| u16::from_le_bytes([value[0], value[1]]))
    };
    let read_u32 = |offset: usize| {
        bytes
            .get(offset..offset + 4)
            .map(|value| u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
    };
    let entry = read_u32(24);
    let mut valid = bytes.len() <= 4096
        && read_u32(28) == Some(52)
        && read_u32(32) == Some(0)
        && read_u32(36) == Some(0)
        && read_u16(42) == Some(32)
        && read_u16(44) == Some(2)
        && read_u16(46) == Some(0)
        && read_u16(48) == Some(0)
        && read_u16(50) == Some(0)
        && entry.is_some_and(|value| (0x0040_0000..0x0080_0000).contains(&value));
    let mut last_file_end = 116u32;
    for index in 0..2usize {
        let header = 52 + index * 32;
        let offset = read_u32(header + 4);
        let address = read_u32(header + 8);
        let physical = read_u32(header + 12);
        let file_size = read_u32(header + 16);
        let memory_size = read_u32(header + 20);
        let flags = read_u32(header + 24);
        let alignment = read_u32(header + 28);
        let expected_flags = if index == 0 { 5 } else { 6 };
        valid &= read_u32(header) == Some(1)
            && address == physical
            && flags == Some(expected_flags)
            && alignment == Some(1);
        let Some((offset, address, file_size, memory_size)) =
            offset.zip(address).zip(file_size).zip(memory_size).map(
                |(((offset, address), file_size), memory_size)| {
                    (offset, address, file_size, memory_size)
                },
            )
        else {
            valid = false;
            continue;
        };
        let file_end = offset.checked_add(file_size);
        let memory_end = address.checked_add(memory_size);
        valid &= file_size <= memory_size
            && offset == last_file_end
            && file_end.is_some_and(|end| end as usize <= bytes.len())
            && memory_end.is_some_and(|end| end <= 0x0080_0000)
            && address >= 0x0040_0000;
        if index == 0 {
            valid &= address == 0x0040_0000
                && memory_size != 0
                && entry.is_some_and(|entry| {
                    memory_end.is_some_and(|end| (address..end).contains(&entry))
                });
        }
        last_file_end = file_end.unwrap_or(u32::MAX);
    }
    valid &= last_file_end as usize == bytes.len();
    if valid {
        Ok(())
    } else {
        Err(CliError::compile(
            path,
            "error[elf32]: DBUP1 packer requires the canonical compact i686-dbyte-user ELF",
        ))
    }
}

fn write_requested_dumps(product: &BuildProduct, options: &BuildOptions) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if options.dump_ir {
        write_dump(&mut output, &product.ir_dump)?;
    }
    if options.dump_native_mir {
        write_dump(&mut output, &product.native_mir_dump)?;
    }
    if options.dump_machine_code {
        write_dump(&mut output, &product.machine_dump)?;
    }
    output
        .flush()
        .map_err(|error| CliError::Stdout(error.kind()))?;
    Ok(())
}

fn write_dump(output: &mut impl Write, dump: &str) -> Result<(), CliError> {
    output
        .write_all(dump.as_bytes())
        .map_err(|error| CliError::Stdout(error.kind()))?;
    if !dump.ends_with('\n') {
        output
            .write_all(b"\n")
            .map_err(|error| CliError::Stdout(error.kind()))?;
    }
    Ok(())
}

fn replace_output_transactionally(output: &Path, bytes: &[u8]) -> Result<(), CliError> {
    replace_output_transactionally_with_mode(output, bytes, false)
}

fn replace_executable_output_transactionally(output: &Path, bytes: &[u8]) -> Result<(), CliError> {
    replace_output_transactionally_with_mode(output, bytes, true)
}

fn replace_output_transactionally_with_mode(
    output: &Path,
    bytes: &[u8],
    executable: bool,
) -> Result<(), CliError> {
    let (mut file, mut temporary) = create_temporary_sibling(output)?;
    let write_result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        #[cfg(unix)]
        if executable {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o755))?;
        }
        #[cfg(not(unix))]
        let _ = executable;
        file.sync_all()
    })();
    drop(file);
    if let Err(error) = write_result {
        return Err(CliError::output(
            output,
            "write and flush temporary output for",
            &error,
        ));
    }

    replace_file(temporary.path(), output)
        .map_err(|error| CliError::output(output, "replace", &error))?;
    temporary.keep();
    Ok(())
}

fn create_temporary_sibling(output: &Path) -> Result<(File, TemporaryOutput), CliError> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = output
        .file_name()
        .ok_or_else(|| CliError::argument("output path must name a file"))?;

    for _ in 0..128 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(file_name);
        temporary_name.push(format!(".dbytec-{}-{sequence}.tmp", std::process::id()));
        let path = parent.join(temporary_name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, TemporaryOutput::new(path))),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(CliError::output(
                    output,
                    "create temporary output for",
                    &error,
                ));
            }
        }
    }

    Err(CliError::Output {
        path: output.to_path_buf(),
        action: "allocate temporary output for",
        kind: io::ErrorKind::AlreadyExists,
    })
}

struct TemporaryOutput {
    path: PathBuf,
    keep: bool,
}

impl TemporaryOutput {
    fn new(path: PathBuf) -> Self {
        Self { path, keep: false }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let source: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linker::{link_user_object_bytes, LINK_ENTRY_SYMBOL, LINK_USER_ENTRY_SYMBOL};
    use crate::object::{
        write_relocatable, ObjectImage, ObjectSection, ObjectSectionData, ObjectSymbol,
        SymbolBinding, SymbolDefinition, SymbolKind,
    };

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn function_object(name: &str) -> ObjectImage {
        let text = vec![0xc3];
        let mut image = ObjectImage::empty();
        image.sections.insert(
            ObjectSection::Text,
            ObjectSectionData::bytes(text.clone(), 16),
        );
        image.symbols.push(ObjectSymbol {
            name: name.to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: text.len() as u32,
        });
        image
    }

    fn object_bytes(image: &ObjectImage) -> Vec<u8> {
        write_relocatable(image)
            .expect("serialize test object")
            .bytes
    }

    fn valid_kernel_object_bytes() -> Vec<u8> {
        object_bytes(&function_object(LINK_ENTRY_SYMBOL))
    }

    fn known_good_elf() -> Vec<u8> {
        link_object_bytes(&[LinkInput::new("known-good.o", valid_kernel_object_bytes())])
            .expect("link known-good ELF")
            .elf
    }

    fn known_good_user_elf() -> Vec<u8> {
        let mut image = function_object(LINK_USER_ENTRY_SYMBOL);
        image.symbols.push(ObjectSymbol {
            name: "__dbyte_profile:user".to_owned(),
            binding: SymbolBinding::Local,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Defined {
                section: ObjectSection::Text,
                value: 0,
            },
            size: 0,
        });
        link_user_object_bytes(&[LinkInput::new("user.o", object_bytes(&image))])
            .expect("link known-good user ELF")
            .elf
    }

    fn temporary_file_count(path: &Path) -> usize {
        fs::read_dir(path)
            .expect("read test directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".dbytec-"))
            .count()
    }

    fn assert_no_temporary_files(path: &Path) {
        assert_eq!(temporary_file_count(path), 0);
    }

    #[test]
    fn parses_complete_build_options() {
        let parsed = parse_build_args(&args(&[
            "build",
            "kernel.dby",
            "--target",
            TARGET,
            "--emit",
            "elf",
            "-o",
            "kernel.elf",
            "--opt-level",
            "0",
            "--dump-ir",
            "--dump-native-mir",
            "--dump-machine-code",
        ]))
        .expect("options should parse");

        assert_eq!(parsed.source, PathBuf::from("kernel.dby"));
        assert!(matches!(parsed.emit, EmitKind::Elf));
        assert_eq!(parsed.output, PathBuf::from("kernel.elf"));
        assert_eq!(parsed.optimization, NativeOptimizationLevel::O0);
        assert!(parsed.dump_ir);
        assert!(parsed.dump_native_mir);
        assert!(parsed.dump_machine_code);
    }

    #[test]
    fn parses_object_emit_and_direct_link_contract() {
        let object = parse_build_args(&args(&[
            "kernel.dby",
            "--target",
            TARGET,
            "--emit",
            "obj",
            "-o",
            "kernel.o",
        ]))
        .expect("object build options should parse");
        assert_eq!(object.emit, EmitKind::Obj);
        assert_eq!(object.optimization, NativeOptimizationLevel::O1);
        assert_eq!(object.output, PathBuf::from("kernel.o"));

        let link = parse_link_args(&args(&["link", "kernel.o", "serial.o", "-o", "kernel.elf"]))
            .expect("link options should parse");
        assert_eq!(
            link.inputs,
            vec![PathBuf::from("kernel.o"), PathBuf::from("serial.o")]
        );
        assert_eq!(link.output, PathBuf::from("kernel.elf"));
    }

    #[test]
    fn native_mir_dump_is_real_optimized_ir_and_not_a_placeholder() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_v12_2_linux/exit42.dby");
        let options = BuildOptions {
            source,
            target: BuildTarget::X86_64Linux,
            emit: EmitKind::Elf,
            output: PathBuf::from("unused.elf"),
            optimization: NativeOptimizationLevel::O1,
            dump_ir: false,
            dump_native_mir: true,
            dump_machine_code: false,
        };
        let dump = compile_native_mir_dump(&options).unwrap();
        assert!(dump.starts_with("optimization=O1\nnative-mir pointer-bytes=8 globals=0\n"));
        assert!(dump.contains("function main"));
        assert!(dump.contains("Const"));
        assert!(dump.contains("Return"));
    }

    #[test]
    fn linux_build_product_uses_native_mir_for_o0_and_o1_machine_bytes() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_v12_2_linux/exit42.dby");
        let options = |optimization| BuildOptions {
            source: source.clone(),
            target: BuildTarget::X86_64Linux,
            emit: EmitKind::Elf,
            output: PathBuf::from("unused.elf"),
            optimization,
            dump_ir: false,
            dump_native_mir: false,
            dump_machine_code: true,
        };
        let o0 = compile_build_product(&options(NativeOptimizationLevel::O0), "").unwrap();
        let o1 = compile_build_product(&options(NativeOptimizationLevel::O1), "").unwrap();
        assert!(o0.machine_dump.starts_with("native-codegen opt=O0 "));
        assert!(o1.machine_dump.starts_with("native-codegen opt=O1 "));
        assert_ne!(o0.bytes, o1.bytes);
        for bytes in [&o0.bytes, &o1.bytes] {
            let inspection = crate::elf64::inspect(bytes).unwrap();
            assert_eq!(inspection.machine, 62);
            assert!(!inspection.has_dynamic);
        }
    }

    #[test]
    fn windows_build_product_uses_native_mir_and_internal_pe_linking() {
        let test_dir = TestDirectory::new("windows-mir-product");
        let source = test_dir.path.join("main.dby");
        fs::write(
            &source,
            "fn windows_get_std_handle_raw(kind: i32) -> usize:\n    return windows.get_std_handle(kind)\n\nfn windows_write_file_raw(handle: usize, text: str, length: u32) -> i32:\n    return windows.write_file(handle, text, length, 0 as usize, 0 as usize)\n\npub fn main() -> int:\n    let output = windows_get_std_handle_raw(-11 as i32)\n    windows_write_file_raw(output, \"WIN MIR OK\\r\\n\", 12 as u32)\n    return 42\n",
        )
        .unwrap();
        let options = |optimization| BuildOptions {
            source: source.clone(),
            target: BuildTarget::X86_64Windows,
            emit: EmitKind::Elf,
            output: test_dir.path.join("unused.exe"),
            optimization,
            dump_ir: false,
            dump_native_mir: false,
            dump_machine_code: true,
        };
        let o0 = compile_build_product(&options(NativeOptimizationLevel::O0), "").unwrap();
        let o1 = compile_build_product(&options(NativeOptimizationLevel::O1), "").unwrap();
        assert_eq!(&o0.bytes[..2], b"MZ");
        assert_eq!(&o1.bytes[..2], b"MZ");
        assert!(o0.machine_dump.starts_with("native-codegen opt=O0 "));
        assert!(o1.machine_dump.starts_with("native-codegen opt=O1 "));
        assert!(o1.machine_dump.contains("PE32+"));
        assert_ne!(o0.bytes, o1.bytes);
    }

    #[test]
    fn kernel_object_forms_do_not_require_entry_but_executable_forms_do() {
        let test_dir = TestDirectory::new("kernel-entry-routing");
        let source = test_dir.path.join("module.dby");
        fs::write(
            &source,
            "pub fn arch_value() -> u32:\n    return 42 as u32\n",
        )
        .unwrap();

        for emit in [EmitKind::Obj, EmitKind::StaticLib] {
            let options = BuildOptions {
                source: source.clone(),
                target: BuildTarget::Kernel,
                emit,
                output: test_dir.path.join("unused-output"),
                optimization: NativeOptimizationLevel::O1,
                dump_ir: false,
                dump_native_mir: false,
                dump_machine_code: false,
            };
            compile_build_product(&options, "").expect("object forms must not require kernel_main");
        }

        for emit in [EmitKind::Elf, EmitKind::Bin] {
            let options = BuildOptions {
                source: source.clone(),
                target: BuildTarget::Kernel,
                emit,
                output: test_dir.path.join("unused-output"),
                optimization: NativeOptimizationLevel::O1,
                dump_ir: false,
                dump_native_mir: false,
                dump_machine_code: false,
            };
            let error = compile_build_product(&options, "")
                .expect_err("executable forms must require kernel_main");
            assert!(error.to_string().contains("E_ENTRY_MISSING"));
        }
    }

    #[test]
    fn kernel_integrated_build_matches_per_module_object_link() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_scheduler_core/kernel.dby");
        let mut inputs = Vec::new();
        for (module, artifact) in crate::modern_native::compile_mir_i686_project_objects(
            &source,
            crate::target::I686_NONE,
            NativeOptimizationLevel::O1,
        )
        .unwrap()
        {
            let stem = module.file_stem().unwrap().to_str().unwrap();
            inputs.push(LinkInput::new(format!("{stem}.o"), artifact.object.bytes));
        }
        let linked = link_object_bytes(&inputs).unwrap();
        let integrated = compile_build_product(
            &BuildOptions {
                source,
                target: BuildTarget::Kernel,
                emit: EmitKind::Elf,
                output: PathBuf::from("unused.elf"),
                optimization: NativeOptimizationLevel::O1,
                dump_ir: false,
                dump_native_mir: false,
                dump_machine_code: false,
            },
            "",
        )
        .unwrap();
        assert_eq!(integrated.bytes, linked.elf);
    }

    #[test]
    fn pack_user_accepts_only_canonical_user_executables() {
        assert!(validate_user_executable(Path::new("user.elf"), &known_good_user_elf()).is_ok());
        assert!(validate_user_executable(Path::new("kernel.elf"), &known_good_elf()).is_err());
        let mut writable_text = known_good_user_elf();
        writable_text[52 + 24..52 + 28].copy_from_slice(&7u32.to_le_bytes());
        assert!(validate_user_executable(Path::new("wx.elf"), &writable_text).is_err());
    }

    #[test]
    fn direct_link_requires_inputs_and_one_output() {
        let no_inputs = parse_link_args(&args(&["link", "-o", "kernel.elf"]))
            .expect_err("empty link should fail");
        assert!(no_inputs.to_string().contains("at least one"));

        let no_output =
            parse_link_args(&args(&["link", "kernel.o"])).expect_err("output is required");
        assert!(no_output
            .to_string()
            .contains("missing required -o/--output"));

        let duplicate_output = parse_link_args(&args(&[
            "link",
            "kernel.o",
            "-o",
            "first.elf",
            "--output",
            "second.elf",
        ]))
        .expect_err("duplicate output should fail");
        assert!(duplicate_output.to_string().contains("more than once"));

        let targeted = parse_link_args(&args(&[
            "link",
            "kernel.o",
            "--target",
            TARGET,
            "-o",
            "kernel.elf",
        ]))
        .expect("link target should parse");
        assert_eq!(targeted.target, Some(crate::target::I686_NONE));
    }

    #[test]
    fn parses_explicit_flat_machine_payload_contract() {
        let parsed = parse_link_args(&args(&[
            "link",
            "boundary.o",
            "body.o",
            "--target",
            "x86_64-dbyte-none",
            "--flat-load-address",
            "0x10000",
            "--flat-max-size",
            "129024",
            "--flat-section-alignment",
            "16",
            "--entry",
            "hoppys_entry",
            "-o",
            "kernel.bin",
        ]))
        .unwrap();
        assert_eq!(parsed.target, Some(crate::target::X86_64_NONE));
        assert_eq!(parsed.flat_load_address, Some(0x1_0000));
        assert_eq!(parsed.flat_max_size, Some(252 * 512));
        assert_eq!(parsed.flat_section_alignment, Some(16));
        assert_eq!(parsed.entry.as_deref(), Some("hoppys_entry"));
    }

    #[test]
    fn parses_constrained_dbtasm64_command() {
        let parsed = parse_assemble_args(&args(&[
            "assemble",
            "entry.dbtasm",
            "--target",
            "x86_64-dbyte-none",
            "-o",
            "entry.o",
        ]))
        .unwrap();
        assert_eq!(parsed.source, PathBuf::from("entry.dbtasm"));
        assert_eq!(parsed.output, PathBuf::from("entry.o"));
        assert_eq!(parsed.target, crate::target::X86_64_NONE);
        assert!(parse_assemble_args(&args(&[
            "assemble",
            "entry.dbtasm",
            "--target",
            "x86_64-dbyte-linux",
            "-o",
            "entry.o",
        ]))
        .unwrap_err()
        .to_string()
        .contains("only x86_64-dbyte-none"));
    }

    #[test]
    fn rejects_invalid_target() {
        let error = parse_build_args(&args(&[
            "kernel.dby",
            "--target",
            "x86_64-dbyte-macos",
            "--emit",
            "elf",
            "-o",
            "kernel.elf",
        ]))
        .expect_err("target should fail");
        assert!(error.to_string().contains("unsupported target"));
    }

    #[test]
    fn rejects_invalid_emit_kind() {
        let error = parse_build_args(&args(&[
            "kernel.dby",
            "--target",
            TARGET,
            "--emit",
            "wasm",
            "-o",
            "kernel.wasm",
        ]))
        .expect_err("emit kind should fail");
        assert!(error.to_string().contains("unsupported emit kind"));
    }

    #[test]
    fn rejects_missing_output() {
        let error = parse_build_args(&args(&["kernel.dby", "--target", TARGET, "--emit", "elf"]))
            .expect_err("missing output should fail");
        assert!(error.to_string().contains("missing required -o/--output"));
    }

    #[test]
    fn elf_validation_distinguishes_relocatable_and_executable_outputs() {
        let object = valid_kernel_object_bytes();
        validate_elf32_kind(
            Path::new("kernel.o"),
            &object,
            ElfKind::Relocatable,
            "compiler",
        )
        .expect("ET_REL should validate as an object");
        parse_relocatable(&object).expect("ET_REL should pass the real object parser");
        assert!(validate_elf32_kind(
            Path::new("kernel.o"),
            &object,
            ElfKind::Executable,
            "linker",
        )
        .is_err());

        let executable = known_good_elf();
        validate_elf32_kind(
            Path::new("kernel.elf"),
            &executable,
            ElfKind::Executable,
            "linker",
        )
        .expect("ET_EXEC should validate as an executable");
        assert!(validate_elf32_kind(
            Path::new("kernel.elf"),
            &executable,
            ElfKind::Relocatable,
            "compiler",
        )
        .is_err());
    }

    #[test]
    fn production_build_has_no_legacy_ast_pipeline_selector() {
        let source = include_str!("cli.rs");
        assert!(!source.contains(&["fn requires_", "systems_pipeline"].concat()));
        assert!(!source.contains(&["compile_source", "(path_label"].concat()));
    }

    #[test]
    fn legacy_v11_source_still_builds_transactionally() {
        let test_dir = TestDirectory::new("legacy-v11-build");
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native_kernel/kernel.dby");
        let output = test_dir.path.join("legacy.elf");

        run_build_args(&[
            "build".to_owned(),
            source.to_string_lossy().into_owned(),
            "--target".to_owned(),
            TARGET.to_owned(),
            "--emit".to_owned(),
            "elf".to_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .expect("v11.0.0 native source should remain buildable");

        let bytes = fs::read(&output).expect("read legacy native ELF");
        validate_elf32_kind(&output, &bytes, ElfKind::Executable, "compiler")
            .expect("legacy pipeline should still emit ET_EXEC");
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn object_build_writes_root_et_rel_that_parses() {
        let test_dir = TestDirectory::new("object-build");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let output = test_dir.path.join("kernel.o");

        run_build_args(&[
            "build".to_owned(),
            source.to_string_lossy().into_owned(),
            "--target".to_owned(),
            TARGET.to_owned(),
            "--emit".to_owned(),
            "obj".to_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .expect("systems object build should succeed");

        let bytes = fs::read(&output).expect("read object output");
        assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 1);
        let image = parse_relocatable(&bytes).expect("CLI object should parse as ET_REL");
        assert!(image
            .symbols
            .iter()
            .any(|symbol| symbol.name == LINK_ENTRY_SYMBOL));
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn build_forms_emit_linked_elf_and_nonempty_bin() {
        let test_dir = TestDirectory::new("integrated-build");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_kernel_multimodule/kernel.dby");
        let elf_output = test_dir.path.join("kernel.elf");
        let bin_output = test_dir.path.join("kernel.bin");

        // `dbyte build` passes the direct source form, while standalone
        // `dbytec build` includes the command token. Keep both contracts.
        run_build_args(&[
            source.to_string_lossy().into_owned(),
            "--target".to_owned(),
            TARGET.to_owned(),
            "--emit".to_owned(),
            "elf".to_owned(),
            "-o".to_owned(),
            elf_output.to_string_lossy().into_owned(),
        ])
        .expect("integrated ELF build should succeed");
        run_args(&[
            "build".to_owned(),
            source.to_string_lossy().into_owned(),
            "--target".to_owned(),
            TARGET.to_owned(),
            "--emit".to_owned(),
            "bin".to_owned(),
            "-o".to_owned(),
            bin_output.to_string_lossy().into_owned(),
        ])
        .expect("integrated flat-binary build should succeed");

        let elf = fs::read(&elf_output).expect("read integrated ELF");
        validate_elf32_kind(&elf_output, &elf, ElfKind::Executable, "compiler")
            .expect("integrated build should emit ET_EXEC");
        assert!(!fs::read(&bin_output)
            .expect("read integrated binary")
            .is_empty());
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn x86_64_none_bin_is_the_checked_multiboot2_load_image() {
        let test_dir = TestDirectory::new("modern-none-bin");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/native_v12_2_none/bootstrap.dby");
        let elf_output = test_dir.path.join("kernel.elf");
        let bin_output = test_dir.path.join("kernel.bin");
        for (emit, output) in [("elf", &elf_output), ("bin", &bin_output)] {
            run_args(&[
                "build".to_owned(),
                source.to_string_lossy().into_owned(),
                "--target".to_owned(),
                "x86_64-dbyte-none".to_owned(),
                "--emit".to_owned(),
                emit.to_owned(),
                "-o".to_owned(),
                output.to_string_lossy().into_owned(),
            ])
            .expect("x86_64-none output should build");
        }
        let elf = fs::read(&elf_output).expect("read modern ELF");
        let bin = fs::read(&bin_output).expect("read modern BIN");
        assert_eq!(bin, elf);
        crate::elf64::inspect(&bin).expect("flat load image retains checked ELF transport");
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn direct_link_dispatch_writes_valid_et_exec() {
        let test_dir = TestDirectory::new("direct-link-success");
        let input = test_dir.path.join("kernel.o");
        let output = test_dir.path.join("kernel.elf");
        fs::write(&input, valid_kernel_object_bytes()).expect("write kernel object");

        run_args(&[
            "link".to_owned(),
            input.to_string_lossy().into_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .expect("direct link should succeed");

        let bytes = fs::read(&output).expect("read linked executable");
        assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 2);
        validate_elf32_kind(&output, &bytes, ElfKind::Executable, "linker")
            .expect("linked output should be ET_EXEC");
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn compile_failure_leaves_existing_output_untouched() {
        let test_dir = TestDirectory::new("compile-failure");
        let source = test_dir.path.join("invalid.dby");
        let output = test_dir.path.join("kernel.elf");
        fs::write(&source, "@").expect("write invalid source");
        let known_good = known_good_elf();
        fs::write(&output, &known_good).expect("write known-good ELF");

        let result = run_build_args(&[
            "build".to_owned(),
            source.to_string_lossy().into_owned(),
            "--target".to_owned(),
            TARGET.to_owned(),
            "--emit".to_owned(),
            "elf".to_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ]);

        assert!(matches!(result, Err(CliError::Compile { .. })));
        assert_eq!(
            fs::read(&output).expect("read preserved output"),
            known_good
        );
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn malformed_object_link_failure_preserves_known_good_output() {
        let test_dir = TestDirectory::new("malformed-link-failure");
        let input = test_dir.path.join("malformed.o");
        let output = test_dir.path.join("kernel.elf");
        let known_good = known_good_elf();
        fs::write(&input, vec![0; 52]).expect("write malformed object");
        fs::write(&output, &known_good).expect("write known-good ELF");

        let error = run_link_args(&[
            "link".to_owned(),
            input.to_string_lossy().into_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .expect_err("malformed object link should fail");

        assert!(matches!(&error, CliError::Link { .. }));
        assert!(error.to_string().contains("InvalidObject"));
        assert_eq!(fs::read(&output).expect("read output"), known_good);
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn undefined_symbol_link_failure_preserves_known_good_output() {
        let test_dir = TestDirectory::new("undefined-link-failure");
        let input = test_dir.path.join("undefined.o");
        let output = test_dir.path.join("kernel.elf");
        let known_good = known_good_elf();
        let mut image = function_object(LINK_ENTRY_SYMBOL);
        image.symbols.push(ObjectSymbol {
            name: "missing".to_owned(),
            binding: SymbolBinding::Global,
            kind: SymbolKind::Function,
            definition: SymbolDefinition::Undefined,
            size: 0,
        });
        fs::write(&input, object_bytes(&image)).expect("write undefined-symbol object");
        fs::write(&output, &known_good).expect("write known-good ELF");

        let error = run_link_args(&[
            input.to_string_lossy().into_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .expect_err("undefined symbol link should fail");

        assert!(matches!(&error, CliError::Link { .. }));
        assert!(error.to_string().contains("UndefinedSymbol"));
        assert_eq!(fs::read(&output).expect("read output"), known_good);
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn duplicate_symbol_link_failure_preserves_known_good_output() {
        let test_dir = TestDirectory::new("duplicate-link-failure");
        let first = test_dir.path.join("first.o");
        let second = test_dir.path.join("second.o");
        let output = test_dir.path.join("kernel.elf");
        let known_good = known_good_elf();
        let duplicate = valid_kernel_object_bytes();
        fs::write(&first, &duplicate).expect("write first duplicate object");
        fs::write(&second, &duplicate).expect("write second duplicate object");
        fs::write(&output, &known_good).expect("write known-good ELF");

        let error = run_link_args(&[
            "link".to_owned(),
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .expect_err("duplicate symbol link should fail");

        assert!(matches!(&error, CliError::Link { .. }));
        assert!(error.to_string().contains("DuplicateSymbol"));
        assert_eq!(fs::read(&output).expect("read output"), known_good);
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn check_portable_accepts_fixed_width_target_independent_source() {
        let test_dir = TestDirectory::new("portable-fixed-width");
        let source = test_dir.path.join("portable.dby");
        fs::write(
            &source,
            "pub fn add(left: i32, right: i32) -> i32:\n    return left + right\n",
        )
        .expect("write portable source");

        run_args(&[
            "check-portable".to_owned(),
            source.to_string_lossy().into_owned(),
        ])
        .expect("fixed-width source should satisfy strict portable contract");
    }

    fn portable_error(label: &str, source_text: &str) -> String {
        let test_dir = TestDirectory::new(label);
        let source = test_dir.path.join("portable.dby");
        fs::write(&source, source_text).expect("write portable negative fixture");
        run_args(&[
            "check-portable".to_owned(),
            source.to_string_lossy().into_owned(),
        ])
        .expect_err("fixture must violate strict portable contract")
        .to_string()
    }

    #[test]
    fn check_portable_rejects_target_width_target_capability_c_abi_and_indexing() {
        let width = portable_error(
            "portable-usize",
            "pub fn width(value: usize) -> usize:\n    return value\n",
        );
        assert!(width.contains("E_PORTABLE_TARGET_WIDTH"));

        let target = portable_error("portable-linux", "fn raw(fd: i32, text: str, length: usize) -> i64:\n    return linux.write(fd, text, length)\n");
        assert!(target.contains("E_PORTABLE_TARGET_CAPABILITY"));

        let c_abi = portable_error(
            "portable-c-abi",
            "extern \"C\" fn c_add(left: i32, right: i32) -> i32\n",
        );
        assert!(c_abi.contains("E_PORTABLE_C_ABI"));

        let indexing = portable_error(
            "portable-index",
            "pub fn first(values: array[u32, 2]) -> u32:\n    return values[0]\n",
        );
        assert!(indexing.contains("E_PORTABLE_INDEX_SEMANTICS"));
    }

    #[test]
    fn check_portable_accepts_explicit_float_bits_but_not_legacy_float_literals() {
        let test_dir = TestDirectory::new("portable-f32-bits");
        let source = test_dir.path.join("portable.dby");
        fs::write(
            &source,
            "pub fn bits(value: u32) -> f32:\n    return value as! f32\n",
        )
        .unwrap();
        run_args(&[
            "check-portable".to_owned(),
            source.to_string_lossy().into_owned(),
        ])
        .expect("exact f32 representation is portable before arithmetic is enabled");

        let legacy = portable_error(
            "portable-legacy-float",
            "pub fn bad() -> float:\n    return 1.5\n",
        );
        assert!(legacy.contains("E_PORTABLE_LEGACY_FLOAT"));
    }

    #[cfg(unix)]
    #[test]
    fn linux_elf_build_output_is_executable() {
        use std::os::unix::fs::PermissionsExt;

        let test_dir = TestDirectory::new("linux-elf-executable");
        let source = test_dir.path.join("main.dby");
        let output = test_dir.path.join("main.elf");
        fs::write(&source, "pub fn main() -> i32:\n    return 42\n")
            .expect("write Linux executable fixture");

        run_build_args(&[
            source.to_string_lossy().into_owned(),
            "--target".to_owned(),
            "x86_64-dbyte-linux".to_owned(),
            "--emit".to_owned(),
            "elf".to_owned(),
            "-o".to_owned(),
            output.to_string_lossy().into_owned(),
        ])
        .expect("build Linux ELF");

        let mode = fs::metadata(&output)
            .expect("read Linux ELF metadata")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "Linux ELF output must be executable");
    }

    #[cfg(unix)]
    #[test]
    fn linux_link_output_is_executable() {
        use std::os::unix::fs::PermissionsExt;

        let test_dir = TestDirectory::new("linux-link-executable");
        let source = test_dir.path.join("main.dby");
        let object = test_dir.path.join("main.o");
        let output = test_dir.path.join("main.elf");
        fs::write(&source, "pub fn main() -> i32:\n    return 42\n").expect("write fixture");
        run_build_args(&[
            source.to_string_lossy().into_owned(),
            "--target".into(),
            "x86_64-dbyte-linux".into(),
            "--emit".into(),
            "obj".into(),
            "-o".into(),
            object.to_string_lossy().into_owned(),
        ])
        .expect("emit Linux object");
        run_link_args(&[
            "link".into(),
            "--target".into(),
            "x86_64-dbyte-linux".into(),
            object.to_string_lossy().into_owned(),
            "-o".into(),
            output.to_string_lossy().into_owned(),
        ])
        .expect("link Linux ELF");
        let mode = fs::metadata(&output)
            .expect("read linked ELF metadata")
            .permissions()
            .mode();
        assert_ne!(
            mode & 0o111,
            0,
            "linked Linux ELF output must be executable"
        );
    }

    #[test]
    fn transactional_write_replaces_existing_output() {
        let test_dir = TestDirectory::new("transactional-replace");
        let output = test_dir.path.join("kernel.bin");
        fs::write(&output, b"old bytes").expect("write old output");

        replace_output_transactionally(&output, b"new machine bytes")
            .expect("replace output transactionally");

        assert_eq!(
            fs::read(&output).expect("read replaced output"),
            b"new machine bytes"
        );
        assert_no_temporary_files(&test_dir.path);
    }

    #[test]
    fn failed_replace_cleans_temporary_sibling() {
        let test_dir = TestDirectory::new("transactional-failure");
        let output = test_dir.path.join("existing-directory");
        fs::create_dir(&output).expect("create output directory");

        let result = replace_output_transactionally(&output, b"machine bytes");

        assert!(matches!(result, Err(CliError::Output { .. })));
        assert!(output.is_dir());
        assert_no_temporary_files(&test_dir.path);
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dbyte-native-cli-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
