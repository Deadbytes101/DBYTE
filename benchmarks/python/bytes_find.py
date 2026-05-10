import time


def main() -> None:
    data = bytearray(10004)
    data[10000:10004] = b"\xDE\xAD\xBE\xEF"
    pattern = b"\xDE\xAD\xBE\xEF"
    start = time.perf_counter()
    i = 0
    while i < 1_000:
        data.find(pattern)
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
