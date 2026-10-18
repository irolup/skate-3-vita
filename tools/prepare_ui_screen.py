#!/usr/bin/env python3
"""Prepares one original front-end screen movie for the Vita's UI runtime (PLAN.md §14, M9).

The logic is skate-3-rust-engine's `tools/prepare_hud.py` (`cb79689`; scoring HUD of
`data/fe/source/screens/hud2/trickdisplay2`) with the bundle and the output name as
arguments instead of constants. It imports the reference's vendored UI extractor
(`tools/vendor/skate3_ui`, MIT, Custom Engine Layer preview.18) from the reference
checkout, read-only (no bytecode is written there). The output is copyrighted data from
the user's own disc: it goes to a private folder outside this repository.

    tools/prepare_ui_screen.py --game ../game_data/skate3_x360 \
        --collections <assets>/private/stock/skater-collections.json \
        --output ../game_data/ui-screens --bundle data/fe/source/screens/main/core_menu

writes OUTPUT/runtime/<bundle name>.json (the same format as trickdisplay.json).
"""
import argparse
import hashlib
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
REFERENCE_TOOLS = Path(__file__).resolve().parent.parent.parent / "skate-3-rust-engine" / "tools"
sys.path.insert(0, str(REFERENCE_TOOLS))

import prepare_hud  # noqa: E402  (the reference's script: font_mapping)
from vendor.skate3_ui.actions import Actions  # noqa: E402
from vendor.skate3_ui.project import extract_project  # noqa: E402
from vendor.skate3_ui.scene_graph import AssetCache, SceneFlattener  # noqa: E402


def prepare(game: Path, output: Path, collections: Path, name: str) -> dict:
    extract_project(game, output, prefixes=(name,), update=True)
    cache = AssetCache(output)
    mappings = prepare_hud.font_mapping(collections, cache)
    bundle = cache.load_bundle(name)
    apt_path = output / "raw" / (name + ".apt")
    const_path = apt_path.with_suffix(".const")
    actions = Actions(apt_path.read_bytes(), const_path.read_bytes())
    blocks = {}
    for c in bundle["characters"].values():
        for f in c.get("frames", []):
            for control in f["controls"]:
                if control["type_name"] in ("do_action", "do_init_action"):
                    offset = control.get("actions_offset", 0)
                    if offset:
                        blocks[str(offset)] = actions.stream(offset)
    # Placement clip actions (flag 0x80): `actions_offset` points to {count, events*}; an
    # event is {flags, key code, action stream*} (flags are SWF's ClipEvent bits, e.g.
    # 0x40000 Construct). Their streams join the action blocks.
    reader = actions.apt
    for c in bundle["characters"].values():
        for f in c.get("frames", []):
            for control in f["controls"]:
                if control["type_name"] in ("place_object2", "place_object3") and control.get("flags", 0) & 0x80 and control.get("actions_offset"):
                    base = control["actions_offset"]
                    count, events = reader.u32be(base), reader.u32be(base + 4)
                    control["clip_actions"] = []
                    for i in range(count):
                        flags, key, stream = (reader.u32be(events + 12 * i + k) for k in (0, 4, 8))
                        blocks[str(stream)] = actions.stream(stream)
                        control["clip_actions"].append({"flags": flags, "key": key, "offset": stream})
                    control["actions_offset"] = 0
    shapes, fonts, unresolved_shapes = {}, {}, {}
    for c in bundle["characters"].values():
        if c["type_name"] == "shape":
            scene = SceneFlattener(cache, lambda *_: {}).flatten(name, c["id"])
            if scene["unresolved"]:
                unresolved_shapes[str(c["id"])] = scene["unresolved"]
            shapes[str(c["id"])] = scene["primitives"]
        elif c["type_name"] == "font":
            family = c["font"]["name"]
            fonts[family] = cache.font_asset(family)
    result = {
        "format": "skate3-scoring-hud", "version": 1,
        "source": {"bundle": name,
                   "apt_sha256": hashlib.sha256(apt_path.read_bytes()).hexdigest(),
                   "const_sha256": hashlib.sha256(const_path.read_bytes()).hexdigest(),
                   "collections_sha256": hashlib.sha256(collections.read_bytes()).hexdigest()},
        "characters": list(bundle["characters"].values()),
        "shapes": shapes, "fonts": fonts, "actions": blocks,
        "font_mappings": mappings,
        "language": {row["label"].strip(): row["value"] for row in
                     json.loads((output / "metadata/languages/english_global.json").read_text(encoding="utf-8"))["entries"]},
        "unresolved_fonts": [family for family, asset in fonts.items() if asset is None],
        "unresolved_shapes": unresolved_shapes,
        # The movie's links to other bundles (linked by tools/link_ui_screen.py): imports
        # are placeholder character ids naming a character exported by another bundle.
        "imports": [{"character_id": int(k), "file": v["file"], "name": v["name"]} for k, v in bundle["imports"].items()],
        "exports": bundle["exports"],
    }
    target = output / "runtime" / (Path(name).name + ".json")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(result, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"{name}: {len(bundle['characters'])} characters, {len(shapes)} shapes ({len(unresolved_shapes)} with unresolved parts), "
          f"{len(blocks)} action blocks, {len(fonts)} fonts -> {target}")
    if result["unresolved_fonts"]:
        print("unresolved authored font families: " + ", ".join(result["unresolved_fonts"]))
    return result


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--game", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--collections", type=Path, required=True)
    p.add_argument("--bundle", required=True, help="e.g. data/fe/source/screens/main/core_menu")
    a = p.parse_args()
    prepare(a.game.resolve(), a.output.resolve(), a.collections.resolve(), a.bundle)
