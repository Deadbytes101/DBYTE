#include <stdint.h>

static uint32_t bits32(float value) {
    union { float f; uint32_t u; } x;
    x.f = value;
    return x.u;
}
static uint64_t bits64(double value) {
    union { double f; uint64_t u; } x;
    x.f = value;
    return x.u;
}
static float from32(uint32_t bits) {
    union { float f; uint32_t u; } x;
    x.u = bits;
    return x.f;
}
static double from64(uint64_t bits) {
    union { double f; uint64_t u; } x;
    x.u = bits;
    return x.f;
}

extern double dbyte_float_callback(int32_t tag, float a, double b, int64_t tail);
double c_float_contract(int32_t tag, float a, double b, int64_t tail) {
    if (tag != 7 || bits32(a) != 0x3fc00000u ||
        bits64(b) != UINT64_C(0x4002000000000000) || tail != 9) {
        return from64(0);
    }
    double callback = dbyte_float_callback(
        11, from32(0x40600000u), from64(UINT64_C(0x4011000000000000)), 13);
    if (bits64(callback) != UINT64_C(0x401a000000000000)) {
        return from64(0);
    }
    return from64(UINT64_C(0x4029000000000000));
}
