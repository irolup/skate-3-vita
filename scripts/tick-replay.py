#!/usr/bin/env python3
"""M3 real-time gate: replay measured per-tick times through the sim thread's accumulator.

PLAN.md §8 (sim thread): a fixed 60 Hz accumulator with the stock clock.rs period
(10_000_000 / 60 = 166,666 units of 100 ns). Each wake-up runs the ticks that are due,
at most 1 + 2 catch-up ticks; any older debt is dropped. The simulation thread owns
core 1, so a tick's wall time is its measured time (sim-bench `*.ticks`, u32 LE µs).

Gate (M3, user decision): no simulated time is dropped in any scenario. Also reported:
the worst backlog (how far the simulation falls behind wall time) and the ticks that
start late by more than one period.

usage: scripts/tick-replay.py DIR [DIR ...]      (DIR holds sim-bench *.ticks files)
"""
import struct
import sys
from pathlib import Path

PERIOD = 10_000_000 // 60  # 100 ns units (clock.rs timer_period)
MAX_PER_WAKE = 3  # one tick plus at most 2 catch-up ticks


def replay(ticks_us):
    """Returns (dropped ticks, worst backlog in 100 ns units, late wake-ups)."""
    acc = PERIOD  # the first tick is due at start
    dropped = worst = late = 0
    i = 0
    while i < len(ticks_us):
        due = acc // PERIOD
        worst = max(worst, acc - PERIOD)
        if due > 1:
            late += 1
        if due > MAX_PER_WAKE:
            dropped += due - MAX_PER_WAKE
            acc -= (due - MAX_PER_WAKE) * PERIOD
            due = MAX_PER_WAKE
        for _ in range(due):
            if i == len(ticks_us):
                break
            acc += ticks_us[i] * 10 - PERIOD
            i += 1
        if acc < PERIOD:
            acc = PERIOD  # sleep until the next tick is due
    return dropped, worst, late


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    total_dropped = 0
    for d in sys.argv[1:]:
        files = sorted(Path(d).glob("*.ticks"))
        if not files:
            sys.exit(f"{d}: no *.ticks files")
        print(f"{d}: period {PERIOD / 10_000:.4f} ms, at most {MAX_PER_WAKE} ticks per wake-up")
        print(f"{'scenario':28} {'ticks':>6} {'dropped':>7} {'worst backlog ms':>16} {'late wakes':>10}")
        for f in files:
            data = f.read_bytes()
            ticks = list(struct.unpack(f"<{len(data) // 4}I", data))
            dropped, worst, late = replay(ticks)
            total_dropped += dropped
            print(f"{f.stem:28} {len(ticks):6} {dropped:7} {worst / 10_000:16.1f} {late:10}")
    print("GATE", "PASS" if total_dropped == 0 else f"FAIL ({total_dropped} ticks dropped)")
    sys.exit(0 if total_dropped == 0 else 1)


if __name__ == "__main__":
    main()
