#!/usr/bin/env python3
"""M5 render-active benchmark: the scenario corpus played by the real app, rendering.

1. uploads the scenarios to ux0:data/SKATEVITA/autoplay/ (and clears autoplay-out/);
2. builds skate-vita with --features replay and runs it as eboot.bin of SKTV00001;
3. prints the AUTOPLAY lines and the per-second RENDER statistics from PrincessLog
   until AUTOPLAY DONE (all of them go to OUT/render-bench.log);
4. downloads each scenario's .ticks (per-tick simulation µs) and .dump (oracle dump)
   to OUT, then removes the autoplay inputs so the next launch is live play again.

Check afterwards with scripts/tick-replay.py OUT (the M3 real-time gate) and
`sim-oracle compare oracle/nofma-base OUT --exact` (rendering must not change the sim).
Assets must already be on the Vita (scripts/vita-bench.py --upload-assets) and the
skater pack too (scripts/vita-run.py --upload-packs ../game_data/SKATEVITA).

Environment as scripts/vita-test.py (VITA_IP, VITA_LOG_PORT, VITASDK).
"""
import argparse
import ftplib
import importlib.util
import io
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


def listing(ftp, path):
    try:
        return [n.rsplit("/", 1)[-1] for n in ftp.nlst(path)]
    except ftplib.error_perm:
        return []


def clear(ftp, path):
    try:
        ftp.mkd(path)
    except ftplib.error_perm:
        pass
    for name in listing(ftp, path):
        try:
            ftp.delete(f"{path}/{name}")
        except ftplib.error_perm:
            pass


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--scenarios", default=str(ROOT / "oracle" / "scenarios"))
    ap.add_argument("--only", nargs="*", help="scenario names (without .input)")
    ap.add_argument("--out", default=str(ROOT / "oracle" / "vita-render"))
    ap.add_argument("--park", help="run on this park's map pack (map_<PARK>.pak) instead of the Course")
    ap.add_argument("--upload-packs", metavar="DIR", help="upload changed DIR/*.pak first (vita-run.py)")
    ap.add_argument("--features", help="more skate-vita cargo features (e.g. stage-profile)")
    ap.add_argument("--title", default="SKTV00001")
    ap.add_argument("--timeout", type=float, default=7200)
    ap.add_argument("--ip", default=os.environ.get("VITA_IP"))
    ap.add_argument("--log-port", type=int, default=int(os.environ.get("VITA_LOG_PORT", 8888)))
    args = ap.parse_args()
    if not args.ip:
        sys.exit("VITA_IP is not set (source .envrc)")

    if args.upload_packs:
        run_spec = importlib.util.spec_from_file_location("vita_run", ROOT / "scripts" / "vita-run.py")
        vr = importlib.util.module_from_spec(run_spec)
        run_spec.loader.exec_module(vr)
        vr.upload_packs(Path(args.upload_packs), args.ip)
    env = vt.sdk_env()
    cmd = ["cargo", "vita", "build", "eboot", "--", "--release", "-p", "skate-vita", "--features", ",".join(["replay"] + ([args.features] if args.features else []))]
    print("build:", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, env=env, check=True, stdout=subprocess.DEVNULL)
    self_path = ROOT / "target/armv7-sony-vita-newlibeabihf/release/skate-vita.self"

    inputs = sorted(Path(args.scenarios).glob("*.input"))
    if args.only:
        inputs = [p for p in inputs if p.stem in args.only]
    with ftplib.FTP() as ftp:
        ftp.connect(args.ip, 1337, timeout=60)
        clear(ftp, f"{REMOTE}/autoplay")
        clear(ftp, f"{REMOTE}/autoplay-out")
        for p in inputs:
            with open(p, "rb") as f:
                ftp.storbinary(f"STOR {REMOTE}/autoplay/{p.name}", f)
        if args.park:
            ftp.storbinary(f"STOR {REMOTE}/autoplay/park.txt", io.BytesIO(args.park.encode()))
    print(f"scenarios: {len(inputs)} uploaded" + (f", park {args.park}" if args.park else ""), flush=True)

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    logs = vt.LogServer(args.log_port)
    vt.upload(self_path, args.title, args.ip, 1337)
    # PrincessLog delivers lines it buffered while nobody listened (e.g. from an
    # earlier run): drop them before this launch.
    time.sleep(2)
    logs.drain()
    vt.launch(args.title, args.ip, 1338)
    start = time.monotonic()
    lines = []
    while time.monotonic() - start < args.timeout:
        try:
            line = logs.lines.get(timeout=1)
        except queue.Empty:
            continue
        lines.append(line)
        if line.startswith("AUTOPLAY") or "fail" in line or "panicked" in line:
            print(line, flush=True)
        if line.startswith("AUTOPLAY DONE"):
            break
    else:
        print("timed out waiting for AUTOPLAY DONE", flush=True)
    (out / "render-bench.log").write_text("\n".join(lines) + "\n")
    time.sleep(2)
    with ftplib.FTP() as ftp:
        ftp.connect(args.ip, 1337, timeout=60)
        for p in inputs:
            for name in (p.stem + ".ticks", p.stem + ".dump"):
                try:
                    with open(out / name, "wb") as f:
                        ftp.retrbinary(f"RETR {REMOTE}/autoplay-out/{name}", f.write)
                except ftplib.error_perm:
                    (out / name).unlink()
                    print(f"missing on the Vita: {name}", flush=True)
        clear(ftp, f"{REMOTE}/autoplay")
    render = [l for l in lines if l.startswith("RENDER")]
    fps = [float(l.split("fps=")[1].split()[0]) for l in render]
    holds = sum(int(l.split("holds=")[1].split()[0]) for l in render)
    if fps:
        print(f"render: {len(fps)} s logged, fps min {min(fps):.1f} mean {sum(fps) / len(fps):.2f}, holds {holds}", flush=True)
    print(f"results: {out}", flush=True)


if __name__ == "__main__":
    main()
