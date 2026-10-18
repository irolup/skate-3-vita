#!/usr/bin/env python3
"""Retail-audio stage S0: runs tools/audio-bench on the Vita and compares with the desktop.
1. builds and runs `audio-bench` on the desktop (the reference hashes, no-fma arithmetic);
2. builds it for the Vita and runs it as eboot.bin of an installed app (default: the
   skate-core test app SKTV00002);
3. prints the AUDIO lines (time per block per voice count, % of one core) and whether every
   output hash equals the desktop's.
Lines before this run's `AUDIO START` are stale PrincessLog output and are ignored.
Environment as scripts/vita-test.py (VITA_IP, VITA_LOG_PORT, VITASDK).
"""
import argparse
import importlib.util
import os
import queue
import re
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("vita_test", ROOT / "scripts" / "vita-test.py")
vt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vt)


def hashes(lines):
    return {m.group(1): m.group(2) for l in lines if (m := re.search(r"mode=ieee voices=(\d+) .* hash=([0-9a-f]+)", l))}


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--title", default="SKTV00002", help="installed app whose eboot.bin is replaced")
    ap.add_argument("--timeout", type=float, default=900)
    ap.add_argument("--features", help="audio-bench features (profile: time per render stage)")
    ap.add_argument("--ip", default=os.environ.get("VITA_IP"))
    ap.add_argument("--log-port", type=int, default=int(os.environ.get("VITA_LOG_PORT", 8888)))
    args = ap.parse_args()
    if not args.ip:
        sys.exit("VITA_IP is not set (source .envrc)")
    feats = ["--features", args.features] if args.features else []
    desktop = subprocess.run(["cargo", "run", "-q", "--release", "-p", "audio-bench"] + feats, cwd=ROOT, check=True,
                             capture_output=True, text=True).stdout.splitlines()
    for l in desktop:
        print("desktop:", l, flush=True)
    env = vt.sdk_env()
    cmd = ["cargo", "vita", "build", "eboot", "--", "--release", "-p", "audio-bench"] + feats
    print("build:", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, env=env, check=True, stdout=subprocess.DEVNULL)
    logs = vt.LogServer(args.log_port)
    vt.upload(ROOT / "target/armv7-sony-vita-newlibeabihf/release/audio-bench.self", args.title, args.ip, 1337)
    vt.launch(args.title, args.ip, 1338)
    start, started, lines = time.monotonic(), False, []
    while time.monotonic() - start < args.timeout:
        try:
            line = logs.lines.get(timeout=1)
        except queue.Empty:
            continue
        if not line.startswith("AUDIO"):
            continue
        started = started or line.startswith("AUDIO START")
        if not started:
            continue
        print("vita:", line, flush=True)
        lines.append(line)
        if line.startswith(("AUDIO DONE", "AUDIO ERROR", "AUDIO CANCELLED")):
            break
    else:
        sys.exit("timed out waiting for AUDIO DONE")
    d, v = hashes(desktop), hashes(lines)
    same = [n for n in d if v.get(n) == d[n]]
    print(f"output identical to the desktop: {len(same)}/{len(d)} voice counts", flush=True)


if __name__ == "__main__":
    main()
