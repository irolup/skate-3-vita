#!/usr/bin/env python3
"""Exports the original front-end movies the Vita's menus use (R2), as skate-3-rust-engine's setup does.

The export is the reference checkout's `tools/prepare_menu_movies.py` (`prepare` and `install`),
run with its menu roots plus the screens the Vita's own hosts need: the manual meter, the trick
analyzer, the replay editor and its help. It follows every cross-movie import, as retail's load
queue does, and installs the verified set as `<assets>/private/menu-movies/` (the format the
engine's APT player reads; `vita-bake ui` packs it). The reference's scripts and its vendored UI
extractor are imported from the checkout, read-only (no bytecode is written there). The output is
copyrighted data from the user's own disc: it stays in the private prepared folder.

    tools/prepare_menu_screens.py --game ../game_data/skate3_x360 --assets $INSTALL/assets \
        --work ../game_data/menu-movies-work
"""
import argparse
import sys
from pathlib import Path

sys.dont_write_bytecode = True

# The screens the Vita's hosts drive beside the reference's menu roots (data/fe/ relative).
VITA_ROOTS = (
    'source/screens/hud2/manualmeter',
    'source/screens/hud2/trickanalyser',
    'source/screens/replay2/replay_editor2',
    'source/screens/replay2/replay_help',
)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--game', type=Path, required=True, help='the extracted disc')
    parser.add_argument('--assets', type=Path, required=True, help='the prepared install\'s assets folder')
    parser.add_argument('--work', type=Path, required=True, help='a work folder (extraction cache)')
    parser.add_argument('--reference', type=Path,
                        default=Path(__file__).resolve().parent.parent.parent / 'skate-3-rust-engine',
                        help='the skate-3-rust-engine checkout (default: next to this repo)')
    args = parser.parse_args()
    sys.path.insert(0, str(args.reference.resolve() / 'tools'))
    import prepare_menu_movies as menus  # noqa: E402  (the reference's export)

    assets = args.assets.resolve()
    collections = assets / 'private/stock/skater-collections.json'
    roots = tuple(menus.MENU_ROOTS) + tuple(r for r in VITA_ROOTS if r not in menus.MENU_ROOTS)
    manifest = menus.prepare(args.game.resolve(), args.work.resolve(), collections, roots=roots)
    missing = [r for r in VITA_ROOTS if menus.library_key(r) not in manifest['movies']]
    if missing:
        sys.exit(f'prepare_menu_screens: no movie for {missing}')
    menus.install(assets, args.work.resolve() / 'runtime')
    print(f"menu-movies: {len(manifest['movies'])} movies installed in {assets / 'private/menu-movies'}")


if __name__ == '__main__':
    main()
