#!/usr/bin/env python3
"""Decodes the decals of the stamps that the reference customiser setup leaves out, for `vita-bake cas`: the nine
uncategorised tattoos and the brand and team logos of the Graphics pages.

The reference project's customiser setup decodes the decal of each stamp that has a `TattooCategory` (170 of them). The
catalog also lists nine `tattoo_diffuse_N` stamps without a category (retail's first category takes them, 8253E4D8):
their materials have a `decal` texture too. The logos (`logo_*`, `team_logo_*`, `team_brand_logo_*`) keep their picture, with
its alpha, in the material's `diffuse` channel, or in `decal` when `diffuse` is the shared placeholder. This writes each one's picture beside the others (`decoded/<id>.png` of the
customisation set) and `extra-tattoos.json` and `extra-logos.json` (material id: name, texture, bounds), which the bake adds to the library's
`tattoos` and `logos`. The output is copyrighted data from the user's own disc and stays in the private prepared folder.

    tools/cas_stamps.py --game ../game_data/skate3_x360 --assets $INSTALL/assets
"""
import argparse
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--game', type=Path, required=True, help='the extracted disc')
    parser.add_argument('--assets', type=Path, required=True, help='the prepared install\'s assets folder')
    parser.add_argument('--reference', type=Path,
                        default=Path(__file__).resolve().parent.parent.parent / 'skate-3-rust-engine',
                        help='the skate-3-rust-engine checkout (default: next to this repo)')
    args = parser.parse_args()
    reference = args.reference.resolve()
    sys.path.insert(0, str(reference))
    from tools.owned_game.big import BigArchive  # noqa: E402
    from tools.extract_default_skater import import_rx2_parser, decode_texture  # noqa: E402

    assets = args.assets.resolve()
    sets = sorted((assets / 'private/customisation/sets').glob('*/catalog.json'))
    if not sets:
        sys.exit('no customisation set: run the reference customiser setup first')
    folder = sets[-1].parent
    catalog = json.loads((folder / 'catalog.json').read_text())
    archive = BigArchive(args.game.resolve() / 'data/big/createacharacter.big'
                         if (args.game / 'data/big/createacharacter.big').exists()
                         else args.game.resolve() / 'data/content/createacharacter.big')
    entries = {e.path.lower(): e for e in archive.entries}
    rx2 = import_rx2_parser(reference / 'tools/vendor/utt')
    decoded = folder / 'decoded'
    decoded.mkdir(exist_ok=True)
    scratch = folder / 'source' / 'data/content/createacharacter/texture'
    scratch.mkdir(parents=True, exist_ok=True)
    misc = next(p for p in catalog['components'] if p['slot'] == 'Misc')
    # The first material group only, as the bake's rows.
    variants = [v for m in misc['models'] for g in m['lods'][0]['material_instances'][:1] for v in g]
    extra, logos = {}, {}

    def decode(tid):
        png = decoded / (tid + '.png')
        if not png.exists():
            source = scratch / ('0x' + tid + '.rx2')
            path = f'data/content/createacharacter/texture/0x{tid}.rx2'
            source.write_bytes(archive.read(entries[path.lower()]))
            decode_texture(rx2, source, png)
        return png.relative_to(assets).as_posix()

    for v in variants:
        flags = catalog['materials'][v['id']]['flags']
        tex = {t['channel']: t['id'] for t in catalog['materials'][v['id']]['textures']}
        name = v['name']
        if name.startswith('tattoo_') and not flags.get('cas.TattooCategory') and 'decal' in tex:
            channel, out = 'decal', extra
        elif name.startswith(('logo_', 'team_logo_', 'team_brand_logo_')) and 'diffuse' in tex:
            # 176 logos share a 128 x 128 placeholder in `diffuse` (an arrow, id 0xAD03E38818); theirs is in `decal`.
            channel, out = ('decal' if tex['diffuse'] == '000000ad03e38818' and 'decal' in tex else 'diffuse'), logos
        else:
            continue
        out[v['id']] = {
            'name': name,
            'texture': decode(tex[channel]),
            'bounds': [float(x) for x in flags['cas.StampBorderConstraint'].split(',')],
        }
    (folder / 'extra-logos.json').write_text(json.dumps(logos, indent=1))
    print(f'{len(extra)} more tattoos and {len(logos)} logos decoded into {folder}')


if __name__ == '__main__':
    main()
