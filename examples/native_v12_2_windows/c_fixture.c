#include <stdint.h>

struct c_pair {
    uint8_t tag;
    uint32_t value;
};

extern uint64_t dbyte_callback(uint64_t value, uint32_t *word);

uint64_t c_contract(
    int8_t signed_byte,
    uint8_t unsigned_byte,
    int16_t signed_word,
    uint16_t unsigned_word,
    uint32_t *word,
    struct c_pair *pair,
    uint64_t tail) {
    if (signed_byte != -7 || unsigned_byte != 250 ||
        signed_word != -300 || unsigned_word != 60000 ||
        (((uintptr_t)__builtin_frame_address(0)) & 15) != 0) {
        return 0;
    }
    *word += 2;
    pair->tag = 7;
    pair->value = 0x12345678u;
    uint32_t callback_word = 3;
    return dbyte_callback(tail, &callback_word) + callback_word;
}
