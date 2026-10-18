#!/usr/bin/env python3
"""Build skate-vita, deploy it as eboot.bin of the installed app and show its log.

1. builds `cargo vita build eboot -- --release -p skate-vita` with the Vita flags
   (scripts/vita-test.py sdk_env: list-ilp scheduler);
2. uploads the SELF as ux0:/app/SKTV00001/eboot.bin (binary FTP) and launches it;
3. prints PrincessLog lines until the app logs that it is exiting, or --timeout.

--upload-packs DIR first uploads DIR/*.pak (vita-bake writes them to
../game_data/SKATEVITA/, PLAN.md §0.5) to ux0:data/SKATEVITA/: only the packs whose
SHA-256 differs from the manifest left there by the previous upload.

The app must be closed on the Vita first (vitacompanion cannot replace a running
eboot). Assets are read from ux0:data/SKATEVITA/assets (upload them once with
scripts/vita-bench.py --upload-assets).

Environment as scripts/vita-test.py (VITA_IP, VITA_LOG_PORT, VITASDK).
"""
import argparse
import ftplib
import hashlib
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


def upload_packs(folder, ip):
    """Uploads the packs that changed since the last upload (manifest of SHA-256s)."""
    remote = "ux0:/data/SKATEVITA"
    with ftplib.FTP() as ftp:
        ftp.connect(ip, 1337, timeout=60)
        try:
            ftp.mkd(remote)
        except ftplib.error_perm:
            pass
        old = {}
        buf = io.BytesIO()
        try:
            ftp.retrbinary(f"RETR {remote}/packs.sha256", buf.write)
            for line in buf.getvalue().decode().splitlines():
                digest, name = line.split(maxsplit=1)
                old[name] = digest
        except ftplib.error_perm:
            pass
        manifest = []
        for pack in sorted([*folder.glob("*.pak"), *folder.glob("*.pak.idx")]):
            digest = hashlib.sha256(pack.read_bytes()).hexdigest()
            manifest.append(f"{digest} {pack.name}")
            if old.get(pack.name) == digest:
                continue
            with open(pack, "rb") as f:
                ftp.storbinary(f"STOR {remote}/{pack.name}", f)
            print(f"uploaded {pack.name} ({pack.stat().st_size // 1024} KiB)", flush=True)
        ftp.storbinary(f"STOR {remote}/packs.sha256", io.BytesIO(("\n".join(manifest) + "\n").encode()))


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--title", default="SKTV00001")
    ap.add_argument("--timeout", type=float, default=3600, help="seconds of log to show")
    ap.add_argument("--upload-packs", metavar="DIR", help="upload DIR/*.pak to ux0:data/SKATEVITA/")
    ap.add_argument("--features", help="skate-vita cargo features (e.g. soak)")
    ap.add_argument("--ip", default=os.environ.get("VITA_IP"))
    ap.add_argument("--log-port", type=int, default=int(os.environ.get("VITA_LOG_PORT", 8888)))
    args = ap.parse_args()
    if not args.ip:
        sys.exit("VITA_IP is not set (source .envrc)")
    env = vt.sdk_env()
    cmd = ["cargo", "vita", "build", "eboot", "--", "--release", "-p", "skate-vita"]
    if args.features:
        cmd += ["--features", args.features]
    print("build:", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, env=env, check=True, stdout=subprocess.DEVNULL)
    self_path = ROOT / "target/armv7-sony-vita-newlibeabihf/release/skate-vita.self"
    if args.upload_packs:
        upload_packs(Path(args.upload_packs), args.ip)
    logs = vt.LogServer(args.log_port)
    vt.upload(self_path, args.title, args.ip, 1337)
    vt.launch(args.title, args.ip, 1338)
    print("launched; log follows", flush=True)
    start = time.monotonic()
    # PrincessLog flushes the lines it buffered while nobody listened (the end of an earlier
    # session, its "exiting" included) when the new process first prints: skip everything
    # before the new process's startup line.
    started, stale = False, 0
    while time.monotonic() - start < args.timeout:
        try:
            line = logs.lines.get(timeout=1)
        except queue.Empty:
            continue
        if not started:
            if line.startswith("skate vita ") and "clocks" in line:
                started = True
                if stale:
                    print(f"(dropped {stale} stale log lines from an earlier session)", flush=True)
            else:
                stale += 1
                continue
        print(line, flush=True)
        if "exiting" in line:
            break


if __name__ == "__main__":
    main()
