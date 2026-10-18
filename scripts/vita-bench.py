#!/usr/bin/env python3
"""M3 simulation benchmark + on-device parity oracle (PLAN.md §5.5).

1. (--upload-assets ASSETS) copies the 17 private files the production load opens to
   ux0:data/SKATEVITA/assets/ (your own data, on your own device; never committed);
2. uploads the scenario corpus to ux0:data/SKATEVITA/scenarios/;
3. builds tools/sim-bench for the Vita and runs it as eboot.bin of an installed app
   (default: the skate-core test app SKTV00002, so nothing new has to be installed);
4. prints the BENCH lines from PrincessLog and downloads the dumps to OUT
   (default oracle/vita-soft) for `sim-oracle compare`, and the per-tick times
   (`*.ticks`) for scripts/tick-replay.py.

Environment as scripts/vita-test.py (VITA_IP, VITA_LOG_PORT, VITASDK).
"""
import argparse
import ftplib
import importlib.util
import os
import queue
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("vita_test", ROOT / "scripts" / "vita-test.py")
vt = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vt)

REMOTE = "ux0:/data/SKATEVITA"
# Files the production load opens or checks (strace -e trace=file of `sim-oracle run`, M3).
ASSET_FILES = [
    "private/game.json",
    "private/skater.glb",  # only checked for existence by the manifest
    "private/stock/skater-collections.json",
    "private/stock/physics-skeletons.json",
    "private/stock/data/anim/OnBoard.abin",
    "private/stock/data/anim/OffBoard.abin",
    "private/stock/data/camera/1.shk",
    "private/stock/data/camera/2.shk",
    "private/stock/data/joystick/skater.pat",
    "private/stock/data/joystick/skater90.pat",
    "private/stock/data/joystick/skaterN90.pat",
    "private/stock/data/joystick/skater_air.pat",
    "private/stock/data/joystick/skater_fingerflip.pat",
    "private/stock/data/joystick/skaterls.pat",
    "private/stock/data/joystick/skaterstep.pat",
    "private/stock/data/script/camera/Default_cameragraph.stategraph",
    "private/stock/data/state/ActionGraph_OnBoard.stategraph",
    "private/stock/data/state/MotionGraph_OnBoard.stategraph",
]


def mkdirs(ftp, path):
    parts = path.split("/")
    for i in range(2, len(parts) + 1):
        try:
            ftp.mkd("/".join(parts[:i]))
        except ftplib.error_perm:
            pass  # exists


def listing(ftp, path):
    try:
        return [n.rsplit("/", 1)[-1] for n in ftp.nlst(path)]
    except ftplib.error_perm:
        return []


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--upload-assets", metavar="ASSETS", help="assets folder of the converted data")
    ap.add_argument("--scenarios", default=str(ROOT / "oracle" / "scenarios"))
    ap.add_argument("--only", nargs="*", help="scenario names (without .input)")
    ap.add_argument("--out", default=str(ROOT / "oracle" / "vita-soft"))
    ap.add_argument("--title", default="SKTV00002", help="installed app whose eboot.bin is replaced")
    ap.add_argument("--timeout", type=float, default=3600)
    ap.add_argument("--features", help="extra cargo features, e.g. skate-sim/no-fma (timing only)")
    ap.add_argument("--ip", default=os.environ.get("VITA_IP"))
    ap.add_argument("--log-port", type=int, default=int(os.environ.get("VITA_LOG_PORT", 8888)))
    args = ap.parse_args()
    if not args.ip:
        sys.exit("VITA_IP is not set (source .envrc)")

    env = vt.sdk_env()
    cmd = ["cargo", "vita", "build", "eboot", "--", "--release", "-p", "sim-bench"]
    if args.features:
        cmd += ["--features", args.features]
    print("build:", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, env=env, check=True, stdout=subprocess.DEVNULL)
    self_path = ROOT / "target/armv7-sony-vita-newlibeabihf/release/sim-bench.self"

    with ftplib.FTP() as ftp:
        ftp.connect(args.ip, 1337, timeout=60)
        if args.upload_assets:
            for rel in ASSET_FILES:
                remote = f"{REMOTE}/assets/{rel}"
                mkdirs(ftp, remote.rsplit("/", 1)[0])
                with open(Path(args.upload_assets) / rel, "rb") as f:
                    ftp.storbinary(f"STOR {remote}", f)
                print(f"asset: {rel}", flush=True)
        mkdirs(ftp, f"{REMOTE}/scenarios")
        mkdirs(ftp, f"{REMOTE}/oracle")
        for old in listing(ftp, f"{REMOTE}/scenarios") + listing(ftp, f"{REMOTE}/oracle"):
            for d in ("scenarios", "oracle"):
                try:
                    ftp.delete(f"{REMOTE}/{d}/{old}")
                except ftplib.error_perm:
                    pass
        inputs = sorted(Path(args.scenarios).glob("*.input"))
        if args.only:
            inputs = [p for p in inputs if p.stem in args.only]
        for p in inputs:
            with open(p, "rb") as f:
                ftp.storbinary(f"STOR {REMOTE}/scenarios/{p.name}", f)
        print(f"scenarios: {len(inputs)} uploaded", flush=True)

    logs = vt.LogServer(args.log_port)
    vt.upload(self_path, args.title, args.ip, 1337)
    vt.launch(args.title, args.ip, 1338)
    start = time.monotonic()
    lines = []
    while time.monotonic() - start < args.timeout:
        try:
            line = logs.lines.get(timeout=1)
        except queue.Empty:
            continue
        if line.startswith("BENCH") or line.startswith("ORACLE"):
            print(line, flush=True)
            lines.append(line)
        if line.startswith(("BENCH DONE", "BENCH ERROR", "BENCH CANCELLED")):
            break
    else:
        sys.exit("timed out waiting for BENCH DONE")

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    (out / "bench.log").write_text("\n".join(lines) + "\n")
    time.sleep(2)
    with ftplib.FTP() as ftp:
        ftp.connect(args.ip, 1337, timeout=60)
        for p in inputs:
            for name in (p.stem + ".dump", p.stem + ".ticks", p.stem + ".stages"):
                try:
                    with open(out / name, "wb") as f:
                        ftp.retrbinary(f"RETR {REMOTE}/oracle/{name}", f.write)
                except ftplib.error_perm:
                    (out / name).unlink()  # .stages only with --features tick-stages
    print(f"dumps: {out}", flush=True)


if __name__ == "__main__":
    main()
