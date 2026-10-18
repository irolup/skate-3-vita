#!/usr/bin/env python3
"""Stage skate-vita for another Vita: the VPK plus a folder laid out like ux0:data/SKATEVITA/.

Copies every pack vita-bake wrote (sim.pak, anim.pak, skater.pak, map_<park>.pak; the
app reads nothing else, PLAN.md §7) to OUT (default ../game_data/vita-install/SKATEVITA),
and the last built skate-vita.vpk next to it. OUT is replaced on each run. On the Vita,
install the VPK with VitaShell and copy OUT as a whole into ux0:data/.

Build the VPK first (`cargo vita build vpk -- --release -p skate-vita` with the flags of
scripts/vita-test.py sdk_env). The packs are retail-derived: keep OUT outside the repo,
on your own devices.
"""
import argparse
import shutil
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GAME_DATA = ROOT.parent / "game_data"
VPK = ROOT / "target/armv7-sony-vita-newlibeabihf/release/skate-vita.vpk"


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--packs-dir", type=Path, default=GAME_DATA / "SKATEVITA",
                    help="where vita-bake wrote the packs")
    ap.add_argument("--out", type=Path, default=GAME_DATA / "vita-install" / "SKATEVITA")
    args = ap.parse_args()
    if args.out.name != "SKATEVITA":
        sys.exit("--out must end in SKATEVITA (it is replaced, then copied into ux0:data/)")

    packs = sorted(args.packs_dir.glob("*.pak"))
    if not packs:
        sys.exit(f"no packs in {args.packs_dir}: run vita-bake all INSTALL {args.packs_dir}")
    if not VPK.is_file():
        sys.exit(f"missing {VPK}: build the VPK first")

    if args.out.exists():
        shutil.rmtree(args.out)
    args.out.mkdir(parents=True)
    total = 0
    for pack in packs:
        shutil.copy2(pack, args.out / pack.name)
        total += pack.stat().st_size
        print(f"{args.out.name}/{pack.name}", flush=True)
    shutil.copy2(VPK, args.out.parent / VPK.name)
    print(VPK.name)
    print(f"staged {len(packs)} packs ({total / 2**20:.1f} MiB) and the VPK in {args.out.parent}")
    print("on the Vita: install skate-vita.vpk, copy SKATEVITA into ux0:data/")


if __name__ == "__main__":
    main()
