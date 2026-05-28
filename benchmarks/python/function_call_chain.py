import time


def a(n: int) -> int:
    return n + 1


def b(n: int) -> int:
    return a(n) + 1


def c(n: int) -> int:
    return b(n) + 1


start = time.perf_counter()
total = 0
i = 0

while i < 500_000:
    total = total + c(i)
    i = i + 1

elapsed = (time.perf_counter() - start) * 1000
print(f"{elapsed:.2f}")
