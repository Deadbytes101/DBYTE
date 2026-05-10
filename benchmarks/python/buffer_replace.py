import time


def main() -> None:
    data = bytearray(10000)
    patch = b"\xAA\xBB\xCC\xDD"
    start = time.perf_counter()
    i = 0
    while i < 100_000:
        data[5000:5004] = patch
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
