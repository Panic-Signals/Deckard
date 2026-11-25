"""Writes ../src/blue_noise_64.bin: a 64x64 blue-noise threshold map (void and cluster,
Ulichney 1993), one byte per pixel, row by row, each the pixel's rank scaled to 0..255.

    python blue_noise.py

Deterministic (seeded), so running it again writes the same file.
"""
import pathlib

import numpy as np

N, SIGMA = 64, 1.5


def kernel():
    d = np.minimum(np.arange(N), N - np.arange(N))
    return np.exp(-(d[:, None] ** 2 + d[None, :] ** 2) / (2 * SIGMA**2))


K = kernel()


def energy(p):
    return np.real(np.fft.ifft2(np.fft.fft2(p) * np.fft.fft2(K)))


def bump(e, i, sign):
    y, x = divmod(i, N)
    e += sign * np.roll(np.roll(K, y, 0), x, 1)


def main():
    rng = np.random.default_rng(1993)
    p = np.zeros((N, N))
    p.flat[rng.choice(N * N, N * N // 10, replace=False)] = 1
    e = energy(p)
    # The initial pattern: move the tightest cluster's 1 to the largest void until it is
    # the void itself.
    while True:
        c = np.argmax(np.where(p == 1, e, -np.inf))
        p.flat[c] = 0
        bump(e, c, -1)
        v = np.argmin(np.where(p == 0, e, np.inf))
        if v == c:
            p.flat[c] = 1
            bump(e, c, 1)
            break
        p.flat[v] = 1
        bump(e, v, 1)
    rank = np.zeros(N * N, dtype=np.int64)
    ones = int(p.sum())
    # Phase 1: rank the initial 1s, tightest cluster last.
    q, eq = p.copy(), e.copy()
    for r in range(ones - 1, -1, -1):
        c = np.argmax(np.where(q == 1, eq, -np.inf))
        q.flat[c] = 0
        bump(eq, c, -1)
        rank[c] = r
    # Phases 2 and 3: fill the largest void. (Past half, the tightest cluster of 0s: with
    # this energy, the same place.)
    for r in range(ones, N * N):
        v = np.argmin(np.where(p == 0, e, np.inf))
        p.flat[v] = 1
        bump(e, v, 1)
        rank[v] = r
    out = (rank * 256 // (N * N)).astype(np.uint8)
    path = pathlib.Path(__file__).resolve().parent.parent / "src" / "blue_noise_64.bin"
    path.write_bytes(out.tobytes())
    print(path, out.min(), out.max(), np.bincount(out, minlength=256).min())


main()
