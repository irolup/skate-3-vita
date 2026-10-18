#!/usr/bin/env python3
"""Run a crate's test binaries on the PS Vita, one after another (PLAN.md §5.1).

Install the test VPK for the title once (e.g. the lib test VPK from
`cargo vita build vpk -- --profile vita-test --tests -p skate-core`). Then this script:

1. builds every test binary of the package (profile `vita-test`, panic = unwind), linked
   with tools/vita-fpu/fpscr_ieee.c so every thread runs in IEEE mode like the desktop
   (the Vita default flushes subnormals to zero; --no-ieee keeps it),
2. turns each ELF into a SELF (vita-elf-create, vita-make-fself),
3. for each one: uploads it as ux0:/app/<title>/eboot.bin (binary FTP), launches it through
   vitacompanion, and reads PrincessLog output until libtest prints `test result:`,
4. writes one log per binary under target/vita-test/<package>/ and prints a summary.

It listens for PrincessLog itself, so stop any other `cargo vita logs` first.
Environment: VITASDK, VITA_IP, and optionally VITA_LOG_PORT (8888), VITA_FTP_PORT (1337)
and VITA_CMD_PORT (1338).
"""

import argparse
import ftplib
import json
import os
import queue
import re
import socket
import subprocess
import sys
import threading
import time
from pathlib import Path

TARGET = "armv7-sony-vita-newlibeabihf"
PROFILE = "vita-test"
# Must match [package.metadata.vita] build_std (see crates/skate-vita/Cargo.toml).
BUILD_STD = "std,panic_abort,panic_unwind"
# Same RUSTFLAGS as cargo-vita, so both share one build cache.
CARGO_VITA_RUSTFLAGS = " --cfg mio_unsupported_force_poll_poll --cfg mio_unsupported_force_waker_pipe"
# M3: LLVM's ILP list scheduler before register allocation. Codegen only (no
# arithmetic change; the Vita dumps stay bit-identical): the solver has 85 spill
# stores instead of 223, and the heavy scenarios run ~10% faster.
VITA_CODEGEN_RUSTFLAGS = " -C llvm-args=-pre-RA-sched=list-ilp"
RESULT_RE = re.compile(
    r"test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
    r"(\d+) measured; (\d+) filtered out; finished in ([\d.]+)s"
)
ROOT = Path(__file__).resolve().parent.parent


def sdk_env():
    sdk = os.environ.get("VITASDK")
    if not sdk:
        sys.exit("VITASDK is not set")
    env = dict(os.environ)
    env["PATH"] = f"{sdk}/bin:{env.get('PATH', '')}"
    flags = env.get("RUSTFLAGS", "")
    if "pre-RA-sched" not in flags:
        flags += VITA_CODEGEN_RUSTFLAGS
    env["RUSTFLAGS"] = flags + CARGO_VITA_RUSTFLAGS
    env["TARGET_CC"] = "arm-vita-eabi-gcc"
    env["TARGET_CXX"] = "arm-vita-eabi-g++"
    return env


def ieee_rustflags(env):
    """Compiles fpscr_ieee.c and returns the RUSTFLAGS that link it into every test binary."""
    src = ROOT / "tools" / "vita-fpu" / "fpscr_ieee.c"
    obj = ROOT / "target" / "vita-test" / "fpscr_ieee.o"
    obj.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["arm-vita-eabi-gcc", "-O2", "-mcpu=cortex-a9", "-mfpu=neon",
                    "-mfloat-abi=hard", "-c", str(src), "-o", str(obj)], env=env, check=True)
    # link-args land after the libraries, so pthread must be listed again after the object.
    return f" -C link-arg={obj} -C link-arg=-lpthread -C link-arg=-Wl,--wrap=pthread_create"


def build(package, env):
    """Builds the package's test binaries and returns [(name, elf_path)]."""
    cmd = [
        "cargo", "build", "-Z", f"build-std={BUILD_STD}", "--target", TARGET,
        "--profile", PROFILE, "-p", package, "--tests",
        "--message-format=json-render-diagnostics",
    ]
    print("build:", " ".join(cmd), flush=True)
    proc = subprocess.run(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True)
    if proc.returncode != 0:
        sys.exit("cargo build failed")
    bins = []
    for line in proc.stdout.splitlines():
        msg = json.loads(line)
        if msg.get("reason") == "compiler-artifact" and msg.get("executable"):
            kind = msg["target"]["kind"]
            name = "lib" if "lib" in kind else msg["target"]["name"]
            bins.append((name, Path(msg["executable"])))
    # Unit tests first, then integration tests by name.
    bins.sort(key=lambda b: (b[0] != "lib", b[0]))
    return bins


def make_self(elf, env):
    velf, self_ = elf.with_suffix(".velf"), elf.with_suffix(".self")
    if self_.exists() and self_.stat().st_mtime >= elf.stat().st_mtime:
        return self_
    subprocess.run(["vita-elf-create", str(elf), str(velf)], env=env, check=True,
                   stdout=subprocess.DEVNULL)
    subprocess.run(["vita-make-fself", "-s", str(velf), str(self_)], env=env, check=True,
                   stdout=subprocess.DEVNULL)
    return self_


class LogServer:
    """Accepts PrincessLog connections and queues complete lines."""

    def __init__(self, port):
        self.lines = queue.Queue()
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind(("0.0.0.0", port))
        self.sock.listen(4)
        threading.Thread(target=self._accept, daemon=True).start()

    def _accept(self):
        while True:
            conn, _ = self.sock.accept()
            threading.Thread(target=self._read, args=(conn,), daemon=True).start()

    def _read(self, conn):
        buf = b""
        while chunk := conn.recv(65536):
            buf += chunk
            *done, buf = buf.split(b"\n")
            for raw in done:
                self.lines.put(raw.decode("utf-8", "replace").rstrip("\r"))

    def drain(self):
        while not self.lines.empty():
            self.lines.get_nowait()


def upload(self_path, title, ip, port):
    with ftplib.FTP() as ftp:
        ftp.connect(ip, port, timeout=30)
        with open(self_path, "rb") as f:
            ftp.storbinary(f"STOR ux0:/app/{title}/eboot.bin", f)  # TYPE I


def launch(title, ip, port):
    with socket.create_connection((ip, port), timeout=10) as s:
        s.sendall(f"launch {title}\n".encode())
        reply = s.recv(256).decode(errors="replace").strip()
    if "Launched" not in reply:
        raise RuntimeError(f"launch {title}: {reply!r}")


def run_one(name, self_path, args, logs, log_dir):
    logs.drain()
    upload(self_path, args.title, args.ip, args.ftp_port)
    start = time.monotonic()
    launch(args.title, args.ip, args.cmd_port)
    captured, result = [], None
    deadline = start + args.timeout
    while time.monotonic() < deadline:
        try:
            line = logs.lines.get(timeout=1)
        except queue.Empty:
            continue
        captured.append(line)
        if m := RESULT_RE.search(line):
            result = m
            break
    wall = time.monotonic() - start
    (log_dir / f"{name}.log").write_text("\n".join(captured) + "\n")
    if result is None:
        return {"name": name, "status": "TIMEOUT/CRASH", "wall_s": round(wall, 1)}
    status, passed, failed, ignored, _, filtered, secs = result.groups()
    failures = [l.split(" ... ")[0].removeprefix("test ") for l in captured
                if l.endswith(" ... FAILED")]
    return {
        "name": name, "status": status, "passed": int(passed), "failed": int(failed),
        "ignored": int(ignored), "filtered": int(filtered), "test_s": float(secs),
        "wall_s": round(wall, 1), "failures": failures,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("-p", "--package", default="skate-core")
    ap.add_argument("--title", default="SKTV00002", help="title id of the installed test VPK")
    ap.add_argument("--only", nargs="*", help="run only these binaries (lib = unit tests)")
    ap.add_argument("--no-ieee", action="store_true",
                    help="keep the Vita's default flush-to-zero/default-NaN FPSCR")
    ap.add_argument("--timeout", type=float, default=900, help="seconds per binary")
    ap.add_argument("--ip", default=os.environ.get("VITA_IP"))
    ap.add_argument("--ftp-port", type=int, default=int(os.environ.get("VITA_FTP_PORT", 1337)))
    ap.add_argument("--cmd-port", type=int, default=int(os.environ.get("VITA_CMD_PORT", 1338)))
    ap.add_argument("--log-port", type=int, default=int(os.environ.get("VITA_LOG_PORT", 8888)))
    args = ap.parse_args()
    if not args.ip:
        sys.exit("VITA_IP is not set (source .envrc)")

    env = sdk_env()
    if not args.no_ieee:
        env["RUSTFLAGS"] += ieee_rustflags(env)
    bins = build(args.package, env)
    if args.only:
        bins = [b for b in bins if b[0] in args.only]
    selfs = [(name, make_self(elf, env)) for name, elf in bins]
    log_dir = ROOT / "target" / "vita-test" / args.package
    log_dir.mkdir(parents=True, exist_ok=True)

    logs = LogServer(args.log_port)
    results = []
    for i, (name, self_path) in enumerate(selfs, 1):
        print(f"[{i}/{len(selfs)}] {name} ...", end=" ", flush=True)
        r = run_one(name, self_path, args, logs, log_dir)
        results.append(r)
        if "passed" in r:
            print(f"{r['status']}: {r['passed']} passed, {r['failed']} failed "
                  f"({r['test_s']} s on device, {r['wall_s']} s wall)", flush=True)
            for f in r["failures"]:
                print(f"    FAILED {f}", flush=True)
        else:
            print(f"{r['status']} after {r['wall_s']} s, see {log_dir / (name + '.log')}",
                  flush=True)
        time.sleep(3)  # let the process exit before eboot.bin is replaced

    (log_dir / "summary.json").write_text(json.dumps(results, indent=2) + "\n")
    passed = sum(r.get("passed", 0) for r in results)
    failed = sum(r.get("failed", 0) for r in results)
    broken = [r["name"] for r in results if "passed" not in r]
    print(f"\nTOTAL {len(results)} binaries: {passed} passed, {failed} failed, "
          f"{len(broken)} timed out/crashed {broken if broken else ''}")
    print(f"logs: {log_dir}")
    sys.exit(0 if failed == 0 and not broken else 1)


if __name__ == "__main__":
    main()
