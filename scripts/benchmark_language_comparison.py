#!/usr/bin/env python3
import hashlib
import json
import os
import platform
import shutil
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "tmp" / "native_v12.4.0" / "references"
MEASURE = ROOT / "scripts" / "native_perf_measure.py"
CPU = 10
CASES = {
    "integer_loop": ("integer_loop.dby", 400_000_000),
    "branch_loop": ("branch_loop.dby", 320_000_000),
    "direct_calls": ("direct_calls.dby", 60_000_064),
    "recursion": ("recursion.dby", 34),
    "wide_arithmetic": ("wide_arithmetic.dby", 120_000_000),
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, cwd=ROOT, env=None):
    started = time.perf_counter_ns()
    result = subprocess.run(command, cwd=cwd, env=env, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    elapsed_ms = (time.perf_counter_ns() - started) / 1_000_000
    if result.returncode != 0:
        raise RuntimeError(f"command failed ({result.returncode}): {command}\n{result.stdout}")
    return result.stdout.strip(), elapsed_ms


def barrier_c(case: str, value: int) -> str:
    common = f"""#include <stdint.h>\nstatic volatile uint64_t runtime_value = {value}ULL;\n#if defined(__GNUC__)\n__attribute__((noinline,noipa))\n#endif\nstatic uint64_t runtime_input(void) {{ return runtime_value; }}\n"""
    if case == "integer_loop":
        body = "uint64_t n=runtime_input(),i=0,total=0; while(i<n){ total += (i&7ULL)+1ULL; ++i; } return total==1800000000ULL?42:1;"
    elif case == "branch_loop":
        body = "uint64_t n=runtime_input(),i=0,total=0; while(i<n){ total += ((i&1ULL)==0)?3ULL:5ULL; ++i; } return total==1280000000ULL?42:1;"
    elif case == "direct_calls":
        common += "#if defined(__GNUC__)\n__attribute__((noinline,noipa))\n#endif\nstatic uint64_t mix(uint64_t v){ return (v*3ULL+1ULL)&255ULL; }\n"
        body = "uint64_t n=runtime_input(),i=0,v=1; while(i<n){ v=mix(v); ++i; } return v==129ULL?42:1;"
    elif case == "recursion":
        common += "#if defined(__GNUC__)\n__attribute__((noinline,noipa))\n#endif\nstatic uint64_t fib(uint64_t v){ return v<2?v:fib(v-1)+fib(v-2); }\n"
        body = "uint64_t i=0,total=0; while(i<4){ total+=fib(runtime_input()); ++i; } return total==22811548ULL?42:1;"
    else:
        body = "uint64_t n=runtime_input(),i=0,v=0x1e3779b97f4a7c15ULL; while(i<n){ v=(v^(v>>13))*0x3f58476d1ce4e5b9ULL; ++i; } return v==0x3e38752fcbc19ffcULL?42:1;"
    return common + "int main(void){ " + body + " }\n"


def barrier_rust(case: str, value: int) -> str:
    head = f"static mut RUNTIME_VALUE:u64={value};\n#[inline(never)] fn runtime_input()->u64{{unsafe{{std::ptr::read_volatile(std::ptr::addr_of!(RUNTIME_VALUE))}}}}\n"
    if case == "integer_loop":
        body = "let n=runtime_input(); let(mut i,mut total)=(0u64,0u64); while i<n{total=total.wrapping_add((i&7)+1);i+=1;} total==1800000000"
    elif case == "branch_loop":
        body = "let n=runtime_input(); let(mut i,mut total)=(0u64,0u64); while i<n{total=total.wrapping_add(if i&1==0{3}else{5});i+=1;} total==1280000000"
    elif case == "direct_calls":
        head += "#[inline(never)] fn mix(v:u64)->u64{v.wrapping_mul(3).wrapping_add(1)&255}\n"
        body = "let n=runtime_input(); let(mut i,mut v)=(0u64,1u64); while i<n{v=mix(v);i+=1;} v==129"
    elif case == "recursion":
        head += "#[inline(never)] fn fib(v:u64)->u64{if v<2{v}else{fib(v-1)+fib(v-2)}}\n"
        body = "let(mut i,mut total)=(0u64,0u64); while i<4{total=total.wrapping_add(fib(runtime_input()));i+=1;} total==22811548"
    else:
        body = "let n=runtime_input(); let(mut i,mut v)=(0u64,0x1e3779b97f4a7c15u64); while i<n{v=(v^(v>>13)).wrapping_mul(0x3f58476d1ce4e5b9);i+=1;} v==0x3e38752fcbc19ffc"
    return head + "fn main(){ let ok={" + body + "}; std::process::exit(if ok{42}else{1}); }\n"


def barrier_zig(case: str, value: int) -> str:
    head = f"const std=@import(\"std\");\nvar runtime_value:u64={value};\nnoinline fn runtimeInput() u64{{const p:*volatile u64=&runtime_value; return p.*;}}\n"
    if case == "integer_loop":
        body = "const n=runtimeInput(); var i:u64=0; var total:u64=0; while(i<n):(i+=1){total +%= (i&7)+1;} const ok=total==1800000000;"
    elif case == "branch_loop":
        body = "const n=runtimeInput(); var i:u64=0; var total:u64=0; while(i<n):(i+=1){total +%= if((i&1)==0) 3 else 5;} const ok=total==1280000000;"
    elif case == "direct_calls":
        head += "noinline fn mix(v:u64) u64{return (v*%3+%1)&255;}\n"
        body = "const n=runtimeInput(); var i:u64=0; var v:u64=1; while(i<n):(i+=1){v=mix(v);} const ok=v==129;"
    elif case == "recursion":
        head += "noinline fn fib(v:u64) u64{return if(v<2) v else fib(v-1)+fib(v-2);}\n"
        body = "var i:u64=0; var total:u64=0; while(i<4):(i+=1){total +%= fib(runtimeInput());} const ok=total==22811548;"
    else:
        body = "const n=runtimeInput(); var i:u64=0; var v:u64=0x1e3779b97f4a7c15; while(i<n):(i+=1){v=(v^(v>>13))*%0x3f58476d1ce4e5b9;} const ok=v==0x3e38752fcbc19ffc;"
    return head + "pub fn main() void{" + body + " std.process.exit(if(ok) 42 else 1);}\n"


def tool_version(command):
    try:
        out = subprocess.run(command, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT, timeout=15).stdout.strip()
        return out.splitlines()[0] if out else "unknown"
    except Exception as error:
        return f"unavailable: {error}"


def build_dbyte_compiler() -> Path:
    target = BUILD / "compiler-dbyte"
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target)
    run([str(Path.home()/".cargo"/"bin"/"cargo.exe"), "build", "-q", "-p", "dbyte_cli", "--bin", "dbytec", "--release"], env=env)
    compiler = target / "release" / "dbytec.exe"
    if not compiler.exists():
        raise RuntimeError("DByte compiler build did not produce dbytec.exe")
    return compiler


def compile_case(compiler: Path, case: str, source_name: str, value: int):
    case_root = BUILD / "cases" / case
    case_root.mkdir(parents=True, exist_ok=True)
    outputs = {}
    compile_ms = {}

    dbyte_out = case_root / "dbyte.exe"
    dbyte_source = ROOT / "benchmarks" / "native" / source_name
    if case == "branch_loop":
        dbyte_source = case_root / "branch_compare.dby"
        dbyte_source.write_text(
            "pub fn main() -> i64:\n    let i: u64 = 0 as u64\n    let total: u64 = 0 as u64\n"
            "    while i < (320000000 as u64):\n        if (i & (1 as u64)) == (0 as u64):\n            total = total + (3 as u64)\n        else:\n            total = total + (5 as u64)\n        i = i + (1 as u64)\n"
            "    if total == (1280000000 as u64):\n        return 42 as i64\n    return 1 as i64\n",
            encoding="utf-8",
        )
    elif case == "recursion":
        dbyte_source = case_root / "recursion_compare.dby"
        dbyte_source.write_text(
            "fn fib(value: u64) -> u64:\n"
            "    if value < (2 as u64):\n        return value\n"
            "    return fib(value - (1 as u64)) + fib(value - (2 as u64))\n\n"
            "pub fn main() -> i64:\n    let i: u64 = 0 as u64\n    let total: u64 = 0 as u64\n"
            "    while i < (4 as u64):\n        total = total + fib(34 as u64)\n        i = i + (1 as u64)\n"
            "    if total == (22811548 as u64):\n        return 42 as i64\n    return 1 as i64\n",
            encoding="utf-8",
        )
    _, compile_ms["dbyte"] = run([str(compiler), "build", str(dbyte_source), "--target", "x86_64-dbyte-windows", "--emit", "exe", "-o", str(dbyte_out), "--opt-level", "1"])
    outputs["dbyte"] = dbyte_out

    c_src = case_root / f"{case}.c"
    c_src.write_text(barrier_c(case, value), encoding="utf-8")
    c_out = case_root / "c.exe"
    _, compile_ms["c"] = run(["gcc", "-O3", "-DNDEBUG", "-std=c17", "-march=x86-64", "-mtune=generic", "-fno-lto", str(c_src), "-o", str(c_out)])
    outputs["c"] = c_out

    cpp_src = case_root / f"{case}.cpp"
    cpp_src.write_text(barrier_c(case, value), encoding="utf-8")
    cpp_out = case_root / "cpp.exe"
    _, compile_ms["cpp"] = run(["g++", "-O3", "-DNDEBUG", "-std=c++20", "-march=x86-64", "-mtune=generic", "-fno-lto", str(cpp_src), "-o", str(cpp_out)])
    outputs["cpp"] = cpp_out

    rust_src = case_root / f"{case}.rs"
    rust_src.write_text(barrier_rust(case, value), encoding="utf-8")
    rust_out = case_root / "rust.exe"
    _, compile_ms["rust"] = run(["rustc", str(rust_src), "-C", "opt-level=3", "-C", "target-cpu=x86-64", "-C", "codegen-units=1", "-C", "lto=off", "-C", "panic=abort", "-o", str(rust_out)])
    outputs["rust"] = rust_out

    zig_src = case_root / f"{case}.zig"
    zig_src.write_text(barrier_zig(case, value), encoding="utf-8")
    zig_out = case_root / "zig.exe"
    _, compile_ms["zig"] = run(["zig", "build-exe", str(zig_src), "-O", "ReleaseFast", "-target", "x86_64-windows-gnu", "-mcpu", "baseline", f"-femit-bin={zig_out}"])
    outputs["zig"] = zig_out
    return outputs, compile_ms


def measure_pair(dbyte: Path, other: Path):
    command = [
        os.fspath(Path(os.sys.executable)), os.fspath(MEASURE), os.fspath(dbyte), os.fspath(other),
        "--expected-exit", "42", "--warmups", "3", "--measurements", "9",
        "--attempts", "3", "--maximum-relative-mad", "0.05",
        "--cpu-affinity", str(CPU),
    ]
    output, _ = run(command)
    return json.loads(output.splitlines()[-1])


def git_text(*args):
    return run(["git", *args])[0]


def main():
    if BUILD.exists():
        shutil.rmtree(BUILD)
    BUILD.mkdir(parents=True)
    compiler = build_dbyte_compiler()
    receipt = {
        "schema": 1,
        "contract": "DByte cross-language performance telemetry v12.4",
        "authority": "telemetry-only; DByte release gates remain authoritative",
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "platform": platform.platform(),
        "cpu_affinity": CPU,
        "candidate_head": git_text("rev-parse", "HEAD"),
        "candidate_dirty": bool(git_text("status", "--porcelain=v1", "--untracked-files=all")),
        "dbyte_compiler_sha256": sha256(compiler),
        "toolchains": {
            "dbyte": "working-tree dbytec O1",
            "c": tool_version(["gcc", "--version"]),
            "cpp": tool_version(["g++", "--version"]),
            "rust": tool_version(["rustc", "--version"]),
            "zig": tool_version(["zig", "version"]),
        },
        "flags": {
            "dbyte": "--target x86_64-dbyte-windows --emit exe --opt-level 1",
            "c": "gcc -O3 -DNDEBUG -std=c17 -march=x86-64 -mtune=generic -fno-lto",
            "cpp": "g++ -O3 -DNDEBUG -std=c++20 -march=x86-64 -mtune=generic -fno-lto",
            "rust": "rustc -C opt-level=3 -C target-cpu=x86-64 -C codegen-units=1 -C lto=off -C panic=abort",
            "zig": "zig build-exe -O ReleaseFast -target x86_64-windows-gnu -mcpu baseline",
        },
        "barrier_policy": "C/C++ volatile+noipa; Rust volatile read; Zig volatile load; barrier invoked once per case",
        "workloads": [],
    }
    holyc = None
    for candidate in ("holyc", "HolyC", "templeos", "tcc"):
        found = shutil.which(candidate)
        if found:
            holyc = found
            break
    receipt["holyc"] = {
        "available": holyc is not None,
        "path": holyc,
        "status": "available-but-not-benchmarked" if holyc else "UNAVAILABLE: no HolyC/TempleOS compiler command in PATH",
    }

    for case, (source_name, value) in CASES.items():
        print(f"language-compare: build {case}", flush=True)
        outputs, compile_ms = compile_case(compiler, case, source_name, value)
        artifacts = {
            language: {
                "sha256": sha256(path),
                "bytes": path.stat().st_size,
                "compile_ms": round(compile_ms[language], 3),
            }
            for language, path in outputs.items()
        }
        comparisons = {}
        for language in ("c", "cpp", "rust", "zig"):
            print(f"language-compare: measure {case} DByte vs {language}", flush=True)
            measurement = measure_pair(outputs["dbyte"], outputs[language])
            comparisons[language] = measurement
        receipt["workloads"].append({
            "name": case,
            "dbyte_source": source_name,
            "artifacts": artifacts,
            "comparisons": comparisons,
        })

    receipt_path = BUILD / "receipt.json"
    receipt_path.write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    print(f"DBYTE CROSS-LANGUAGE TELEMETRY: PASS\nReceipt: {receipt_path}")


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"DBYTE CROSS-LANGUAGE TELEMETRY: FAIL: {error}", file=os.sys.stderr)
        raise
