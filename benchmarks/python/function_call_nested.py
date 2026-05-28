import time


def inc(n: int) -> int:
    return n + 1


def work(n: int) -> int:
    return inc(n)


def main() -> None:
    start = time.perf_counter()
    total = 0
    i = 0
    while i < 500_000:
        total = work(total)
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()

