const std = @import("std");

noinline fn benchIntegerLoop(seed: u64) c_int {
    var index: u64 = 0;
    var total: u64 = 0;
    while (index < 50_000_000) : (index += 1) {
        total += (index & 7) + seed;
    }
    return if (total == 225_000_000) 42 else 1;
}

noinline fn benchBranchLoop(seed: u64) c_int {
    var index: u64 = 0;
    var total: u64 = 0;
    while (index < 40_000_000) : (index += 1) {
        total += if ((index & 1) == 0) 2 + seed else 4 + seed;
    }
    return if (total == 160_000_000) 42 else 1;
}

noinline fn mix(value: u64) u64 {
    return ((value * 3) + 1) & 255;
}

noinline fn benchDirectCalls(seed: u64) c_int {
    var index: u64 = 0;
    var value: u64 = seed;
    while (index < 60_000_064) : (index += 1) {
        value = mix(value);
    }
    return if (value == 129) 42 else 1;
}

noinline fn fib(value: u64) u64 {
    if (value < 2) return value;
    return fib(value - 1) + fib(value - 2);
}

noinline fn benchRecursion(seed: u64) c_int {
    return if (fib(33 + seed) == 5_702_887) 42 else 1;
}

noinline fn benchWideArithmetic(seed: u64) c_int {
    var index: u64 = 0;
    var value: u64 = 0x1e3779b97f4a7c15 ^ (seed - 1);
    while (index < 30_000_000) : (index += 1) {
        value = (value ^ (value >> 13)) *% 0x3f58476d1ce4e5b9;
    }
    return if (value == 0xddf962871fd1d6c5) 42 else 1;
}

pub export fn main(argc: c_int, argv: [*][*:0]u8) c_int {
    if (argc != 2) return 2;
    const seed: u64 = @intCast(argc - 1);
    const arg = std.mem.span(argv[1]);
    if (std.mem.eql(u8, arg, "startup")) return 42;
    if (std.mem.eql(u8, arg, "integer_loop")) return benchIntegerLoop(seed);
    if (std.mem.eql(u8, arg, "branch_loop")) return benchBranchLoop(seed);
    if (std.mem.eql(u8, arg, "direct_calls")) return benchDirectCalls(seed);
    if (std.mem.eql(u8, arg, "recursion")) return benchRecursion(seed);
    if (std.mem.eql(u8, arg, "wide_arithmetic")) return benchWideArithmetic(seed);
    return 3;
}
