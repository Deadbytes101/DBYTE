import time


def main() -> None:
    data = bytearray(b"\x00\x11\x22\x33\xDE\xAD\xBE\xEF\x44\x55\x66\x77")
    pattern = b"\xDE\xAD\xBE\xEF"
    patch = b"\x90\x90\x90\x90"
    start = time.perf_counter()
    i = 0
    while i < 100_000:
        pos = data.find(pattern)
        if pos >= 0:
            data[pos : pos + 4] = patch
            data[pos : pos + 4] = pattern
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
