#!/usr/bin/env python3
"""Replay MoE expert trace: LRU vs Belady at varying cache budgets.

Trace format (binary): repeated u32 layer, u32 expert (little-endian).
Generate traces from host with SOSO_MOE_TRACE=1 and SOSO_PLANNER=1 on hostrun.

Usage:
  python3 tools/sim-moe-cache.py trace.bin [bytes_per_expert]
"""

from __future__ import annotations

import struct
import sys
from collections import OrderedDict
from typing import Iterable


def read_trace(path: str) -> list[tuple[int, int]]:
    data = open(path, "rb").read()
    if len(data) % 8 != 0:
        raise SystemExit(f"trace size {len(data)} not multiple of 8")
    out: list[tuple[int, int]] = []
    for i in range(0, len(data), 8):
        layer, expert = struct.unpack_from("<II", data, i)
        out.append((layer, expert))
    return out


def lru_sim(requests: Iterable[tuple[int, int]], capacity: int) -> tuple[int, int]:
    if capacity <= 0:
        return 0, sum(1 for _ in requests)
    cache: OrderedDict[tuple[int, int], None] = OrderedDict()
    hits = misses = 0
    for key in requests:
        if key in cache:
            hits += 1
            cache.move_to_end(key)
        else:
            misses += 1
            cache[key] = None
            while len(cache) > capacity:
                cache.popitem(last=False)
    return hits, misses


def belady_sim(requests: list[tuple[int, int]], capacity: int) -> tuple[int, int]:
    if capacity <= 0:
        return 0, len(requests)
    cache: set[tuple[int, int]] = set()
    hits = misses = 0
    for i, key in enumerate(requests):
        if key in cache:
            hits += 1
            continue
        misses += 1
        if len(cache) < capacity:
            cache.add(key)
            continue
        future = requests[i + 1 :]
        evict = min(
            cache,
            key=lambda k: future.index(k) if k in future else len(future) + 1,
        )
        cache.remove(evict)
        cache.add(key)
    return hits, misses


def main() -> None:
    if len(sys.argv) < 2:
        print(__doc__)
        raise SystemExit(2)
    path = sys.argv[1]
    bytes_per_expert = int(sys.argv[2]) if len(sys.argv) > 2 else 17_547_264 // 3
    trace = read_trace(path)
    if not trace:
        raise SystemExit("empty trace")
    distinct = len(set(trace))
    print(f"requests={len(trace)} distinct={distinct} bytes/expert={bytes_per_expert}")
    budgets = [
        8 * 1024**3,
        32 * 1024**3,
        64 * 1024**3,
        128 * 1024**3,
        192 * 1024**3,
    ]
    print("cache_GiB  slots   LRU_hit%  Belady_hit%  GB_read/token")
    for budget in budgets:
        slots = max(1, budget // bytes_per_expert)
        lh, lm = lru_sim(trace, slots)
        bh, bm = belady_sim(trace, slots)
        total = lh + lm
        l_pct = 100.0 * lh / total if total else 0.0
        b_pct = 100.0 * bh / total if total else 0.0
        gb_per_tok = (lm * bytes_per_expert) / (1024**3)
        print(
            f"{budget / 1024**3:8.0f}  {slots:6d}  {l_pct:8.1f}  {b_pct:11.1f}  {gb_per_tok:13.2f}"
        )


if __name__ == "__main__":
    main()
