#[inline(never)]
fn bench_integer_loop(seed: u64) -> i32 {
    let mut index: u64 = 0;
    let mut total: u64 = 0;
    while index < 50_000_000 {
        total += (index & 7) + seed;
        index += 1;
    }
    if total == 225_000_000 { 42 } else { 1 }
}

#[inline(never)]
fn bench_branch_loop(seed: u64) -> i32 {
    let mut index: u64 = 0;
    let mut total: u64 = 0;
    while index < 40_000_000 {
        total += if (index & 1) == 0 { 2 + seed } else { 4 + seed };
        index += 1;
    }
    if total == 160_000_000 { 42 } else { 1 }
}

#[inline(never)]
fn mix(value: u64) -> u64 {
    ((value * 3) + 1) & 255
}

#[inline(never)]
fn bench_direct_calls(seed: u64) -> i32 {
    let mut index: u64 = 0;
    let mut value: u64 = seed;
    while index < 60_000_064 {
        value = mix(value);
        index += 1;
    }
    if value == 129 { 42 } else { 1 }
}

#[inline(never)]
fn fib(value: u64) -> u64 {
    if value < 2 {
        return value;
    }
    fib(value - 1) + fib(value - 2)
}

#[inline(never)]
fn bench_recursion(seed: u64) -> i32 {
    if fib(33 + seed) == 5_702_887 { 42 } else { 1 }
}

#[inline(never)]
fn bench_wide_arithmetic(seed: u64) -> i32 {
    let mut index: u64 = 0;
    let mut value: u64 = 0x1e3779b97f4a7c15 ^ (seed - 1);
    while index < 30_000_000 {
        value = (value ^ (value >> 13)).wrapping_mul(0x3f58476d1ce4e5b9);
        index += 1;
    }
    if value == 0xddf962871fd1d6c5 { 42 } else { 1 }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        std::process::exit(2);
    }
    let seed = (args.len() - 1) as u64;
    let code = match args[1].as_str() {
        "startup" => 42,
        "integer_loop" => bench_integer_loop(seed),
        "branch_loop" => bench_branch_loop(seed),
        "direct_calls" => bench_direct_calls(seed),
        "recursion" => bench_recursion(seed),
        "wide_arithmetic" => bench_wide_arithmetic(seed),
        _ => 3,
    };
    std::process::exit(code);
}
