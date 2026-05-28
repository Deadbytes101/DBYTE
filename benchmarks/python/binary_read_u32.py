import time


def main() -> None:
    data = b"\x78\x56\x34\x12\x78\x56\x34\x12\x78\x56\x34\x12\x78\x56\x34\x12"
    start = time.perf_counter()
    total = 0
    i = 0
    while i < 200_000:
        total = total + int.from_bytes(data[0:4], "little", signed=False)
        total = total + int.from_bytes(data[4:8], "little", signed=False)
        total = total + int.from_bytes(data[8:12], "little", signed=False)
        total = total + int.from_bytes(data[12:16], "little", signed=False)
        i = i + 1
    elapsed = (time.perf_counter() - start) * 1000.0
    print(f"{elapsed:.2f}")


if __name__ == "__main__":
    main()
