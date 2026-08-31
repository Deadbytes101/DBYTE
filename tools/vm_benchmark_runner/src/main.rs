use dbyte_compiler::lower_concrete_project_to_bytecode;
use dbyte_hir::{
    analyze_project, load_source_project, monomorphize_project, Profile, TargetDataLayout,
};
use dbyte_vm::execute_canonical_bytecode;
#[cfg(feature = "candidate-profile")]
use dbyte_vm::execute_canonical_bytecode_profiled;
use sha2::{Digest, Sha256};
use std::env;
use std::path::PathBuf;
use std::process;
use std::time::Instant;

fn main() {
    if let Err(message) = run() {
        eprintln!("VM_BENCHMARK_ERROR: {message}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let source =
        PathBuf::from(args.next().ok_or(
            "usage: dbyte_vm_benchmark_runner <source.dby> [--warmups N] [--measurements N]",
        )?);
    let mut warmups = 3usize;
    let mut measurements = 9usize;
    let mut iterations = 1usize;
    while let Some(argument) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {argument}"))?;
        let parsed = value
            .parse::<usize>()
            .map_err(|_| format!("invalid count `{value}`"))?;
        match argument.as_str() {
            "--warmups" => warmups = parsed,
            "--measurements" => measurements = parsed,
            "--iterations" => iterations = parsed,
            _ => return Err(format!("unknown argument `{argument}`")),
        }
    }
    if measurements == 0 || iterations == 0 {
        return Err("measurements and iterations must be greater than zero".into());
    }

    let source_bytes = std::fs::read(&source)
        .map_err(|error| format!("cannot read `{}`: {error}", source.display()))?;
    let source_sha256 = hex_digest(&source_bytes);

    let analyze_start = Instant::now();
    let input = load_source_project(&source).map_err(format_diagnostics)?;
    let analyzed =
        analyze_project(&input, Profile::Vm, TargetDataLayout::I686).map_err(format_diagnostics)?;
    let concrete = monomorphize_project(&analyzed).map_err(|diagnostic| diagnostic.to_string())?;
    let analyze_ns = analyze_start.elapsed().as_nanos();

    let lower_start = Instant::now();
    let bytecode = lower_concrete_project_to_bytecode(&concrete)
        .map_err(|diagnostic| diagnostic.to_string())?;
    let lower_ns = lower_start.elapsed().as_nanos();

    let mut expected = None;
    for _ in 0..warmups {
        execute_batch(&bytecode, iterations, &mut expected)?;
    }

    let mut samples_ns = Vec::with_capacity(measurements);
    for _ in 0..measurements {
        let started = Instant::now();
        execute_batch(&bytecode, iterations, &mut expected)?;
        let elapsed = started.elapsed().as_nanos();
        samples_ns.push(elapsed);
    }
    samples_ns.sort_unstable();
    let median_ns = samples_ns[samples_ns.len() / 2];
    let mut deviations = samples_ns
        .iter()
        .map(|sample| sample.abs_diff(median_ns))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad_ns = deviations[deviations.len() / 2];
    let relative_mad = if median_ns == 0 {
        0.0
    } else {
        mad_ns as f64 / median_ns as f64
    };
    let output_sha256 = hex_digest(expected.as_deref().unwrap_or_default().as_bytes());

    print!(
        "{{\"source\":\"{}\",\"source_sha256\":\"{}\",\"output_sha256\":\"{}\",\"warmups\":{},\"measurements\":{},\"iterations\":{},\"analyze_ns\":{},\"lower_ns\":{},\"median_ns\":{},\"min_ns\":{},\"max_ns\":{},\"mad_ns\":{},\"relative_mad\":{:.8},\"samples_ns\":[{}]",
        json_escape(&source.to_string_lossy()),
        source_sha256,
        output_sha256,
        warmups,
        measurements,
        iterations,
        analyze_ns,
        lower_ns,
        median_ns,
        samples_ns[0],
        samples_ns[samples_ns.len() - 1],
        mad_ns,
        relative_mad,
        samples_ns
            .iter()
            .map(u128::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    print_profile(&bytecode)?;
    println!("}}");
    Ok(())
}

fn execute_batch(
    bytecode: &dbyte_bytecode::CanonicalBytecodeProject,
    iterations: usize,
    expected: &mut Option<String>,
) -> Result<(), String> {
    for _ in 0..iterations {
        let execution = execute_canonical_bytecode(bytecode).map_err(|error| error.msg)?;
        verify_execution(expected, &execution)?;
    }
    Ok(())
}

fn verify_execution(
    expected: &mut Option<String>,
    execution: &dbyte_vm::CanonicalVmExecution,
) -> Result<(), String> {
    if execution.live_allocations != 0 {
        return Err(format!(
            "benchmark leaked {} allocations",
            execution.live_allocations
        ));
    }
    if let Some(expected) = expected {
        if expected != &execution.output {
            return Err("benchmark output changed between runs".into());
        }
    } else {
        *expected = Some(execution.output.clone());
    }
    Ok(())
}

#[cfg(feature = "candidate-profile")]
fn print_profile(bytecode: &dbyte_bytecode::CanonicalBytecodeProject) -> Result<(), String> {
    let profiled = execute_canonical_bytecode_profiled(bytecode).map_err(|error| error.msg)?;
    let metrics = profiled.metrics;
    print!(
        ",\"profile\":{{\"total_opcodes\":{},\"function_calls\":{},\"allocations\":{},\"bytecode_ops\":{},\"peak_frames\":{},\"peak_value_stack\":{},\"peak_live_allocations\":{},\"per_opcode\":{{",
        metrics.total_opcodes,
        metrics.function_calls,
        metrics.allocations,
        metrics.bytecode_ops,
        metrics.peak_frames,
        metrics.peak_value_stack,
        metrics.peak_live_allocations
    );
    for (index, (opcode, count)) in metrics.per_opcode.iter().enumerate() {
        if index != 0 {
            print!(",");
        }
        print!("\"{}\":{}", json_escape(opcode), count);
    }
    print!("}}}}");
    Ok(())
}

#[cfg(not(feature = "candidate-profile"))]
fn print_profile(_: &dbyte_bytecode::CanonicalBytecodeProject) -> Result<(), String> {
    print!(",\"profile\":null");
    Ok(())
}

fn format_diagnostics(diagnostics: Vec<dbyte_hir::Diagnostic>) -> String {
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
}
