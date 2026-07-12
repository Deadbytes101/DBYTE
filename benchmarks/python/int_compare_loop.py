import time


def main() -> None:
    start = time.perf_counter()
    i = 0
    hits = 0
    while i < 1_000_000:
        if i >= 10:
            hits = hits + 1
        if i <= 999_990:
            hits = hits + 1
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
