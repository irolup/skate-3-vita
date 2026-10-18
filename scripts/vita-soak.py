#!/usr/bin/env python3
"""M7 soak: 30 minutes of the real game loop on scripted input, nobody holding the Vita.

1. uploads the oracle scenarios to ux0:data/SKATEVITA/autoplay/ (the soak's pad input);
2. runs scripts/vita-run.py with the skate-vita `soak` feature (src/soak.rs: scenarios
   back to back, respawn and difficulty change between them, another world every 3,
   the pause menu every 2 minutes) and keeps its log in LOG;
3. removes the scenarios from the card and prints the heap and frame summary.

The app must be closed on the Vita first. Usage: scripts/vita-soak.py [--upload-packs DIR]
"""
import argparse
import ftplib
import io
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REMOTE = "ux0:/data/SKATEVITA"


def ftp_session(ip):
    ftp = ftplib.FTP()
    ftp.connect(ip, 1337, timeout=60)
    return ftp


def clear(ftp, path):
    try:
        ftp.mkd(path)
    except ftplib.error_perm:
        pass
    for n in ftp.nlst(path):
        try:
            ftp.delete(f"{path}/{n.rsplit('/', 1)[-1]}")
        except ftplib.error_perm:
            pass


def summarize(log):
    text = Path(log).read_text(errors="replace")
    heap = [(int(m[1]), int(m[2]), int(m[3]), int(m[4])) for m in
            re.finditer(r"HEAP t=(\d+)s in_use=(\d+) peak=(\d+) arena=\d+ anim=(\d+)", text)]
    fps = [float(m[1]) for m in re.finditer(r"^RENDER fps=([\d.]+)", text, re.M)]
    ready = [(m[1], float(m[2]), float(m[3])) for m in re.finditer(r"world (\w+): textures ready after ([\d.]+) s \((\d+) ms of upload work\)", text)]
    print(f"\nsoak summary ({log})")
    if heap:
        mb = lambda v: v / 1048576
        print(f"  heap lines {len(heap)}, last t={heap[-1][0]} s")
        # The in-use level at the end of each park (right after a reload) shows growth.
        thirds = [heap[i * len(heap) // 3:(i + 1) * len(heap) // 3] for i in range(3)]
        for i, part in enumerate(thirds):
            if part:
                print(f"  third {i + 1}: in_use min {mb(min(p[1] for p in part)):.1f} max {mb(max(p[1] for p in part)):.1f} MB, anim cache max {mb(max(p[3] for p in part)):.1f} MB")
        print(f"  peak in_use {mb(heap[-1][2]):.1f} MB")
    if fps:
        print(f"  {len(fps)} s rendered, mean {sum(fps) / len(fps):.2f} fps, min {min(fps):.1f}, seconds under 55 fps: {sum(f < 55 for f in fps)}")
    if ready:
        print(f"  {len(ready)} park texture uploads: ready after {min(r[1] for r in ready):.1f}-{max(r[1] for r in ready):.1f} s, {min(r[2] for r in ready):.0f}-{max(r[2] for r in ready):.0f} ms of upload work")
    for key in ("dropped", "holds"):
        v = re.findall(rf"{key}=(\d+)", text)
        if v:
            print(f"  last {key} counter {v[-1]}")
    bad = [l for l in text.splitlines() if re.search(r"panic|simulation error|reloading after|load failed|SOAK .*: |recording saved", l)]
    print(f"  errors: {len(bad)}")
    for l in bad[:10]:
        print("   ", l[:160])
    print("  " + ("SOAK DONE" if "SOAK DONE" in text else "NO 'SOAK DONE' LINE (did not finish)"))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--scenarios", default=str(ROOT / "oracle/scenarios"))
    ap.add_argument("--log", default=str(ROOT / "target/device-logs/m7-soak-unattended.log"))
    ap.add_argument("--upload-packs", metavar="DIR")
    ap.add_argument("--ip", default=os.environ.get("VITA_IP"))
    ap.add_argument("--log-port", default=os.environ.get("VITA_LOG_PORT", "9999"))
    ap.add_argument("--duration", type=int, help="seconds (default 1800)")
    ap.add_argument("--switch-every", type=int, help="change world every N seconds (default: every 3 scenarios)")
    ap.add_argument("--districts", action="store_true", help="rotate the three districts only (M10 gate)")
    ap.add_argument("--summary-only", action="store_true", help="only summarize an existing log")
    args = ap.parse_args()
    if args.summary_only:
        return summarize(args.log)
    if not args.ip:
        sys.exit("VITA_IP is not set (source .envrc)")
    inputs = sorted(Path(args.scenarios).glob("*.input"))
    if not inputs:
        sys.exit(f"no scenarios in {args.scenarios}")
    with ftp_session(args.ip) as ftp:
        clear(ftp, f"{REMOTE}/autoplay")
        for p in inputs:
            with open(p, "rb") as f:
                ftp.storbinary(f"STOR {REMOTE}/autoplay/{p.name}", f)
        config = "".join(f"{k}={v}\n" for k, v in (("duration", args.duration), ("switch_every", args.switch_every), ("districts", int(args.districts))) if v)
        if config:
            ftp.storbinary(f"STOR {REMOTE}/autoplay/soak.txt", io.BytesIO(config.encode()))
        print(f"uploaded {len(inputs)} scenarios" + (f" and soak.txt ({config.strip()!r})" if config else ""), flush=True)
    cmd = [sys.executable, str(ROOT / "scripts/vita-run.py"), "--features", "soak",
           "--log-port", str(args.log_port), "--timeout", "3000"]
    if args.upload_packs:
        cmd += ["--upload-packs", args.upload_packs]
    with open(args.log, "w") as log:
        subprocess.run(cmd, cwd=ROOT, stdout=log, stderr=subprocess.STDOUT)
    with ftp_session(args.ip) as ftp:
        clear(ftp, f"{REMOTE}/autoplay")
    summarize(args.log)


if __name__ == "__main__":
    main()
