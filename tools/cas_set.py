#!/usr/bin/env python3
"""Exports the character customisation's 3D set (retail `DIST_CreateSakter` in `miscload.big`: the concrete floor and
wall the skater stands on in FE 15) as a render-only map, as skate-3-rust-engine's setup exports the Trick Guide's set.

The export is the reference checkout's `tools/asset_pipeline/scene_clips.export_set`, run on the customisation's model
with its decal and alpha-tested meshes kept (the set is mostly decals; the Trick Guide export keeps its two plain
shaders only). It writes `<assets>/private/customisation/CreateSakter.skate`, which `vita-bake cas` packs as
`cas/set.map`. The reference's scripts and its vendored parsers are imported from the checkout, read-only (no bytecode
is written there). The output is copyrighted data from the user's own disc: it stays in the private prepared folder.

    tools/cas_set.py --game ../game_data/skate3_x360 --assets $INSTALL/assets
"""
import argparse
import sys
from pathlib import Path

sys.dont_write_bytecode = True

MODEL = 'data/content/world/models/DIST_CreateSakter.rx2'
TEXTURES = 'data/content/world/models/DIST_CreateSakter_Textures.rx2'
# The set's floor and wall (environment), its stains and markings (decal) and its cut-out edges (alpha test).
SHADERS = ('environment.default', 'environmentsimple.default', 'environment.decal', 'environmentsimple.alphatest')


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--game', type=Path, required=True, help='the extracted disc')
    parser.add_argument('--assets', type=Path, required=True, help='the prepared install\'s assets folder')
    parser.add_argument('--reference', type=Path,
                        default=Path(__file__).resolve().parent.parent.parent / 'skate-3-rust-engine',
                        help='the skate-3-rust-engine checkout (default: next to this repo)')
    args = parser.parse_args()
    sys.path.insert(0, str(args.reference.resolve()))
    from tools.owned_game.big import BigArchive  # noqa: E402  (the reference's archive reader)
    from tools.asset_pipeline import scene_clips  # noqa: E402  (the reference's set export)
    scene_clips.SET_SHADERS = SHADERS
    archive = BigArchive(args.game.resolve() / 'data/big/miscload.big')
    entries = {e.path: e for e in archive.entries}
    missing = [p for p in (MODEL, TEXTURES) if p not in entries]
    if missing:
        sys.exit(f'cas_set: miscload.big has no {missing}')
    output = args.assets.resolve() / 'private/customisation/CreateSakter.skate'
    output.parent.mkdir(parents=True, exist_ok=True)
    if not scene_clips.export_set(archive.read(entries[MODEL]), archive.read(entries[TEXTURES]), output):
        sys.exit('cas_set: the set has no exportable meshes')
    print(f'cas set: {output} ({output.stat().st_size // 1024} KiB)')


if __name__ == '__main__':
    main()
