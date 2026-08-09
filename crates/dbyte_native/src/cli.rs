use crate::{compile_source, EmitKind, NativeArtifact, TARGET};
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const USAGE: &str = "usage: dbytec build <source.dby> --target i686-dbyte-none --emit elf|bin -o <output> [--dump-ir] [--dump-machine-code]";

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
            Self::Input { .. } | Self::Compile { .. } | Self::Output { .. } | Self::Stdout(_) => 1,
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
    emit: EmitKind,
    output: PathBuf,
    dump_ir: bool,
    dump_machine_code: bool,
}

pub fn run_build_args(args: &[String]) -> Result<(), CliError> {
    let options = parse_build_args(args)?;
    let source_text = fs::read_to_string(&options.source).map_err(|error| CliError::Input {
        path: options.source.clone(),
        kind: error.kind(),
    })?;

    reject_source_as_output(&options.source, &options.output)?;

    let path_label = options.source.to_string_lossy();
    let artifact = compile_source(path_label.as_ref(), &source_text).map_err(|diagnostic| {
        CliError::compile(
            &options.source,
            diagnostic.render_with_path(path_label.as_ref()),
        )
    })?;

    let bytes = selected_output(&artifact, &options.emit);
    validate_selected_output(&options.source, &options.emit, bytes)?;

    write_requested_dumps(&artifact, &options)?;
    replace_output_transactionally(&options.output, bytes)
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
    let mut dump_ir = false;
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
            "--dump-ir" => {
                reject_duplicate(dump_ir, "--dump-ir")?;
                dump_ir = true;
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
    if target != TARGET {
        return Err(CliError::argument(format!(
            "unsupported target '{target}'; expected '{TARGET}'"
        )));
    }

    let emit = emit.ok_or_else(|| CliError::argument("missing required --emit"))?;
    let output = output.ok_or_else(|| CliError::argument("missing required -o/--output"))?;

    Ok(BuildOptions {
        source: PathBuf::from(source),
        emit,
        output,
        dump_ir,
        dump_machine_code,
    })
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
        _ => Err(CliError::argument(format!(
            "unsupported emit kind '{value}'; expected 'elf' or 'bin'"
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

fn selected_output<'a>(artifact: &'a NativeArtifact, emit: &EmitKind) -> &'a [u8] {
    match emit {
        EmitKind::Elf => &artifact.elf,
        EmitKind::Bin => &artifact.bin,
    }
}

fn validate_selected_output(source: &Path, emit: &EmitKind, bytes: &[u8]) -> Result<(), CliError> {
    match emit {
        EmitKind::Bin if bytes.is_empty() => Err(CliError::compile(
            source,
            "error[native-output]: compiler produced an empty binary",
        )),
        EmitKind::Bin => Ok(()),
        EmitKind::Elf => validate_elf32(source, bytes),
    }
}

fn validate_elf32(source: &Path, bytes: &[u8]) -> Result<(), CliError> {
    let valid = bytes.len() >= 52
        && bytes.starts_with(b"\x7fELF")
        && bytes[4] == 1
        && bytes[5] == 1
        && u16::from_le_bytes([bytes[16], bytes[17]]) == 2
        && u16::from_le_bytes([bytes[18], bytes[19]]) == 3;

    if valid {
        Ok(())
    } else {
        Err(CliError::compile(
            source,
            "error[elf32]: compiler produced an invalid ELF32/i386 executable",
        ))
    }
}

fn write_requested_dumps(
    artifact: &NativeArtifact,
    options: &BuildOptions,
) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if options.dump_ir {
        write_dump(&mut output, &artifact.ir_dump)?;
    }
    if options.dump_machine_code {
        write_dump(&mut output, &artifact.machine_dump)?;
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
    let (mut file, mut temporary) = create_temporary_sibling(output)?;
    let write_result = file.write_all(bytes).and_then(|()| file.sync_all());
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

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
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
            "--dump-ir",
            "--dump-machine-code",
        ]))
        .expect("options should parse");

        assert_eq!(parsed.source, PathBuf::from("kernel.dby"));
        assert!(matches!(parsed.emit, EmitKind::Elf));
        assert_eq!(parsed.output, PathBuf::from("kernel.elf"));
        assert!(parsed.dump_ir);
        assert!(parsed.dump_machine_code);
    }

    #[test]
    fn rejects_invalid_target() {
        let error = parse_build_args(&args(&[
            "kernel.dby",
            "--target",
            "x86_64-dbyte-none",
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
            "obj",
            "-o",
            "kernel.o",
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
    fn compile_failure_leaves_existing_output_untouched() {
        let test_dir = TestDirectory::new("compile-failure");
        let source = test_dir.path.join("invalid.dby");
        let output = test_dir.path.join("kernel.elf");
        fs::write(&source, "@").expect("write invalid source");
        fs::write(&output, b"existing output").expect("write sentinel output");

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
            fs::read(&output).expect("read sentinel output"),
            b"existing output"
        );
        let temporary_files = fs::read_dir(&test_dir.path)
            .expect("read test directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".dbytec-"))
            .count();
        assert_eq!(temporary_files, 0);
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
        let temporary_files = fs::read_dir(&test_dir.path)
            .expect("read test directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".dbytec-"))
            .count();
        assert_eq!(temporary_files, 0);
    }

    #[test]
    fn failed_replace_cleans_temporary_sibling() {
        let test_dir = TestDirectory::new("transactional-failure");
        let output = test_dir.path.join("existing-directory");
        fs::create_dir(&output).expect("create output directory");

        let result = replace_output_transactionally(&output, b"machine bytes");

        assert!(matches!(result, Err(CliError::Output { .. })));
        assert!(output.is_dir());
        let temporary_files = fs::read_dir(&test_dir.path)
            .expect("read test directory")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".dbytec-"))
            .count();
        assert_eq!(temporary_files, 0);
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
