typedef int (*dbyte_callback)(int);

extern int archive_bias(int value);

int c_roundtrip(dbyte_callback callback, int value) {
    return archive_bias(callback(value));
}
