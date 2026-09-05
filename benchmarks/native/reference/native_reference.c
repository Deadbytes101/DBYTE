#include <stdint.h>
#include <string.h>

#if defined(_MSC_VER)
#define NOINLINE __declspec(noinline)
#else
#define NOINLINE __attribute__((noinline))
#endif

NOINLINE int bench_integer_loop(uint64_t seed) {
    uint64_t index = 0;
    uint64_t total = 0;
    while (index < UINT64_C(50000000)) {
        total += (index & UINT64_C(7)) + seed;
        index += UINT64_C(1);
    }
    return total == UINT64_C(225000000) ? 42 : 1;
}

NOINLINE int bench_branch_loop(uint64_t seed) {
    uint64_t index = 0;
    uint64_t total = 0;
    while (index < UINT64_C(40000000)) {
        total += (index & UINT64_C(1)) == 0 ? UINT64_C(2) + seed : UINT64_C(4) + seed;
        index += UINT64_C(1);
    }
    return total == UINT64_C(160000000) ? 42 : 1;
}

NOINLINE uint64_t mix(uint64_t value) {
    return ((value * UINT64_C(3)) + UINT64_C(1)) & UINT64_C(255);
}

NOINLINE int bench_direct_calls(uint64_t seed) {
    uint64_t index = 0;
    uint64_t value = seed;
    while (index < UINT64_C(60000064)) {
        value = mix(value);
        index += UINT64_C(1);
    }
    return value == UINT64_C(129) ? 42 : 1;
}

NOINLINE uint64_t fib(uint64_t value) {
    if (value < UINT64_C(2)) return value;
    return fib(value - UINT64_C(1)) + fib(value - UINT64_C(2));
}

NOINLINE int bench_recursion(uint64_t seed) {
    return fib(UINT64_C(33) + seed) == UINT64_C(5702887) ? 42 : 1;
}

NOINLINE int bench_wide_arithmetic(uint64_t seed) {
    uint64_t index = 0;
    uint64_t value = UINT64_C(0x1e3779b97f4a7c15) ^ (seed - UINT64_C(1));
    while (index < UINT64_C(30000000)) {
        value = (value ^ (value >> 13)) * UINT64_C(0x3f58476d1ce4e5b9);
        index += UINT64_C(1);
    }
    return value == UINT64_C(0xddf962871fd1d6c5) ? 42 : 1;
}

int main(int argc, char **argv) {
    if (argc != 2) return 2;
    const uint64_t seed = (uint64_t)(argc - 1);
    if (strcmp(argv[1], "startup") == 0) return 42;
    if (strcmp(argv[1], "integer_loop") == 0) return bench_integer_loop(seed);
    if (strcmp(argv[1], "branch_loop") == 0) return bench_branch_loop(seed);
    if (strcmp(argv[1], "direct_calls") == 0) return bench_direct_calls(seed);
    if (strcmp(argv[1], "recursion") == 0) return bench_recursion(seed);
    if (strcmp(argv[1], "wide_arithmetic") == 0) return bench_wide_arithmetic(seed);
    return 3;
}
