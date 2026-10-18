#!/usr/bin/env python3
"""Links a prepared front-end screen with the bundles it imports (PLAN.md §14, M9).

A screen movie names characters that other bundles export (shared controls: buttons,
pickers, image renderers). The Vita's runtime loads one movie, so this merges a screen
and its whole import closure into one: every bundle gets its own id range (character
ids and action offsets are offset by `index * 10_000_000`), imported placeholders are
replaced by the exported characters they name, and the export table (linkage name ->
character id) is kept for `attachMovie`. Inputs are the `runtime/<bundle>.json` files of
`tools/prepare_ui_screen.py`; the output is `runtime/<screen>.linked.json`.

    tools/link_ui_screen.py --output ../game_data/ui-screens --screen core_menu \
        --game ../game_data/skate3_x360 --collections <assets>/private/stock/skater-collections.json
"""
import argparse
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))

BASE = 10_000_000


def load(runtime: Path, file: str, prepare=None) -> dict:
    """An import's `file` ("source/controls/menu_picker") is the bundle named
    `data/fe/<file>`; its prepared JSON is runtime/<last component>.json. A bundle that
    is not prepared yet is prepared with `prepare(bundle name)` (--game given)."""
    path = runtime / (Path(file).name + ".json")
    if not path.is_file() and prepare:
        prepare(f"data/fe/{file}")
    if not path.is_file():
        raise SystemExit(f"{file}: not prepared ({path}); run prepare_ui_screen.py --bundle data/fe/{file}")
    return json.loads(path.read_text(encoding="utf-8"))


def link(runtime: Path, screen: str, prepare=None) -> dict:
    main = json.loads((runtime / f"{screen}.json").read_text(encoding="utf-8"))
    order = [(screen, main)]          # bundle k has base k * BASE
    index = {screen: 0}
    queue = [main]
    while queue:
        bundle = queue.pop(0)
        for imp in bundle["imports"]:
            key = Path(imp["file"]).name
            if key not in index:
                index[key] = len(order)
                loaded = load(runtime, imp["file"], prepare)
                order.append((key, loaded))
                queue.append(loaded)
    names = [k for k, _ in order]

    def resolve(bundle_key: str, bundle: dict, character_id: int) -> int:
        """The linked id of `character_id` as referenced from this bundle."""
        for imp in bundle["imports"]:
            if imp["character_id"] == character_id:
                target_key = Path(imp["file"]).name
                target = order[index[target_key]][1]
                exported = target["exports"].get(imp["name"])
                if exported is None:
                    raise SystemExit(f"{target_key}: export {imp['name']!r} is absent")
                return index[target_key] * BASE + exported
        return index[bundle_key] * BASE + character_id

    characters, actions, shapes, fonts, exports = [], {}, {}, {}, {}
    language, mappings = {}, {}
    for key, bundle in order:
        base = index[key] * BASE
        for c in bundle["characters"]:
            c = json.loads(json.dumps(c))  # deep copy
            c["id"] = base + c["id"]
            text = c.get("text")
            if isinstance(text, dict) and "font_id" in text:
                text["font_id"] = resolve(key, bundle, text["font_id"])
            movie = c.get("movie")
            for frame in c.get("frames", []):
                for control in frame["controls"]:
                    if control.get("character_id") is not None:
                        control["character_id"] = resolve(key, bundle, control["character_id"])
                    if control.get("actions_offset"):
                        control["actions_offset"] += base
                    for event in control.get("clip_actions", []):
                        event["offset"] += base
            characters.append(c)
        for offset, code in bundle["actions"].items():
            blocks = json.loads(json.dumps(code))
            # Offsets inside blocks (instruction offsets, branch targets) stay file-local.
            actions[str(int(offset) + base)] = blocks
        for cid, prims in bundle["shapes"].items():
            shapes[str(base + int(cid))] = prims
        for family, asset in bundle["fonts"].items():
            fonts.setdefault(family, asset)
        for name, cid in bundle["exports"].items():
            exports.setdefault(name, base + cid)
        language.update(bundle["language"])
        mappings.update(bundle["font_mappings"])
    linked = {
        "format": "skate3-linked-screen", "version": 1,
        "source": {"screen": screen, "bundles": names},
        "characters": characters, "shapes": shapes, "fonts": fonts, "actions": actions,
        "font_mappings": mappings, "language": language, "exports": exports,
        "unresolved_fonts": sorted({f for _, b in order for f in b["unresolved_fonts"]}),
    }
    return linked


if __name__ == "__main__":
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--screen", required=True)
    p.add_argument("--game", type=Path, help="the extracted disc: prepares imported bundles that are missing")
    p.add_argument("--collections", type=Path, help="skater-collections.json (with --game)")
    a = p.parse_args()
    runtime = a.output.resolve() / "runtime"
    prepare = None
    if a.game:
        import prepare_ui_screen
        prepare = lambda name: prepare_ui_screen.prepare(a.game.resolve(), a.output.resolve(), a.collections.resolve(), name)
    linked = link(runtime, a.screen, prepare)
    target = runtime / f"{a.screen}.linked.json"
    target.write_text(json.dumps(linked, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"{a.screen}: {len(linked['source']['bundles'])} bundles {linked['source']['bundles']}, "
          f"{len(linked['characters'])} characters, {len(linked['actions'])} action blocks, "
          f"{len(linked['exports'])} exports -> {target}")
