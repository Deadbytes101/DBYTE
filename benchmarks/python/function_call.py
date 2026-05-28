import time


def work(n: int) -> int:
    return n * 2


def main() -> None:
    start = time.perf_counter()
    i = 0
    while i < 1_000_000:
        work(i)
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
