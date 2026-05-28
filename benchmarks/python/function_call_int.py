import time


def add(a: int, b: int) -> int:
    return a + b


def main() -> None:
    start = time.perf_counter()
    total = 0
    i = 0
    while i < 1_000_000:
        total = add(total, 1)
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()

