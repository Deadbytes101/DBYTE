import time


def choose(n: int) -> int:
    if n < 0:
        return 0
    return n + 1


def main() -> None:
    start = time.perf_counter()
    total = 0
    i = 0
    while i < 1_000_000:
        total = choose(total)
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()

