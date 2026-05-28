import time


def sum6(a: int, b: int, c: int, d: int, e: int, f: int) -> int:
    return a + b + c + d + e + f


start = time.perf_counter()
total = 0
i = 0

while i < 500_000:
    total = total + sum6(i, 1, 2, 3, 4, 5)
    i = i + 1

elapsed = (time.perf_counter() - start) * 1000
print(f"{elapsed:.2f}")
