import time


def main() -> None:
    start = time.perf_counter()
    outer = 0
    total = 0
    while outer < 1000:
        inner = 0
        while inner < 1000:
            total = total + inner
            inner = inner + 1
        outer = outer + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
