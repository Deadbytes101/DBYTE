import time


def main() -> None:
    start = time.perf_counter()
    i = 0
    total = 0
    while i < 2_000_000:
        total = total + i
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
