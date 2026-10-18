#!/usr/bin/env python3
"""Screen inventory for M9 (PLAN.md §14): prepares, links and probes every front-end screen.

For each movie under `data/fe/source/screens/` (except the shared `controls`, the cut
scenes `nis` and the demo/debug movies) it runs `prepare_ui_screen.py`, `link_ui_screen.py`
and `sim-oracle ui-probe`, and writes a Markdown table of the result (stdout): the
movie's size, whether the runtime runs it, the first error, and the native calls it makes.
The output folder is private (disc data); only the table is committed.

    tools/ui_inventory.py --game ../game_data/skate3_x360 --collections <assets>/private/stock/skater-collections.json \
        --output ../game_data/ui-inventory --apts apts.txt --oracle target/release/sim-oracle
"""
import argparse, collections, json, re, subprocess, sys
from pathlib import Path

# Tier A (PLAN.md §14): views over data free skate already has; the rest is Tier B (M12).
TIER_A = {"dpo", "homscoring", "hudbase", "hudcentral", "hudcountdown", "hudminimap", "hudtimer", "manualmeter", "trickdisplay2",
          "hudabd", "huddistance", "actionoverlay2", "trickanalyser", "tutorial", "hudtutorial", "centralhud", "FE_root", "chyron",
          "dimmer", "subtitle", "tiploader", "core_menu", "calibration_internal", "freeskate_options", "fmv_popup", "languageselect",
          "options", "videocal", "save_warning", "sk8popup", "small_popup", "starratingpopup", "challengemap", "replay_editor2",
          "replay_help", "trickguide", "skate101", "hud_freeskateactivity_overlay", "hudphonelist", "main3dhud", "mainmisc", "main"}
SKIP = ("/controls/", "/nis/", "/demo/", "/debug/", "/musicplayer/", "/mags_promo/", "/tutorial_videos/")
HERE = Path(__file__).resolve().parent


def main():
    p = argparse.ArgumentParser()
    for a in ("game", "collections", "output", "apts", "oracle"):
        p.add_argument("--" + a, type=Path, required=True)
    p.add_argument("--frames", default="90")
    a = p.parse_args()
    names = sorted({x.strip()[:-4] for x in a.apts.read_text().split() if x.startswith("data/fe/source/screens/") and not any(s in x for s in SKIP)})
    rows = []
    for name in names:
        short = Path(name).name
        group = name.split("screens/")[1].rsplit("/", 1)[0] if name.count("/") > 5 else "(root)"
        prep = subprocess.run([sys.executable, HERE / "prepare_ui_screen.py", "--game", a.game, "--collections", a.collections, "--output", a.output, "--bundle", name], capture_output=True, text=True)
        if prep.returncode:
            rows.append((group, short, "-", "prepare failed: " + (prep.stderr.strip().splitlines() or ["?"])[-1][:90], "")); continue
        info = re.search(r"(\d+) characters, (\d+) shapes.*?(\d+) action blocks", prep.stdout)
        size = f"{info[1]} ch / {info[2]} sh / {info[3]} act" if info else "?"
        lk = subprocess.run([sys.executable, HERE / "link_ui_screen.py", "--output", a.output, "--screen", short, "--game", a.game, "--collections", a.collections], capture_output=True, text=True)
        if lk.returncode:
            rows.append((group, short, size, "link failed: " + (lk.stderr.strip().splitlines() or ["?"])[-1][:90], "")); continue
        pr = subprocess.run([a.oracle, "ui-probe", a.output / "runtime" / (short + ".linked.json"), a.frames], capture_output=True, text=True)
        out = pr.stdout + pr.stderr
        calls = collections.Counter(m.group(1) for m in re.finditer(r"^  ([A-Za-z_0-9]+\.[A-Za-z_0-9]+)\s+x\d+", out, re.M))
        status = "runs" if pr.returncode == 0 else "error: " + (pr.stderr.strip().splitlines() or ["?"])[-1][:80]
        rows.append((group, short, size, status, ", ".join(sorted(calls))))
        print(f"{name}: {status}", file=sys.stderr)
    head = "| Group | Screen | Size | Runtime | Native calls |\n|---|---|---|---|---|"
    rows_a = [r for r in rows if r[1] in TIER_A]
    rows_b = [r for r in rows if r[1] not in TIER_A]
    for title, part in (("Tier A (M9)", rows_a), ("Tier B (M12, stubbed, or cut)", rows_b)):
        print(f"\n## {title}: {sum(r[3] == 'runs' for r in part)} of {len(part)} run\n\n{head}")
        for r in part:
            print("| " + " | ".join(x.replace("|", "/") for x in r) + " |")

main()
