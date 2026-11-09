#!/usr/bin/env python3
"""Exports Slappy's shop thumbnails (retail `fedynamic.big` `data/fe/cas/**`: one picture per item, the remove tile in
`general`) for `vita-bake cas`, which packs them as `cas/thumbs/<name>.tex`.

Each thumbnail is one DXT5 128 x 128 level. Its blocks are kept as retail stores them, only made readable for the Vita:
untiled from the Xbox 360 GPU layout when the texture is tiled and every 16-bit word swapped (the console's byte
order). Nothing is decoded or compressed again. The output is `<assets>/private/customisation/thumbnails/` with one
`<name>.dxt5` per picture (name: the path under `data/` without `.rx2`, `fe/cas/male/hat/...`) and `thumbnails.json`
(name, width, height). The tattoos' and logos' pictures (`logos/`, the Tattoo and Graphics pages' thumbnails) come
with them.

The archive reader and the RX2 parser are the reference checkout's (`tools/owned_game/big.py`,
`tools/vendor/utt/rx2_parser.py`), imported read-only (no bytecode is written there). The output is copyrighted data
from the user's own disc: it stays in the private prepared folder.

    tools/cas_thumbs.py --game ../game_data/skate3_x360 --assets $INSTALL/assets
"""
import argparse
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True

PREFIX = 'data/fe/cas/'
# The part folders Slappy's reads (825FF7D0 with 82604958: a gender folder, or the board parts, and `general`).
FOLDERS = ('male/', 'female/', 'skateboard/', 'skatetruck/', 'skatewheel/', 'general/', 'logos/')
OUT = 'private/customisation/thumbnails'


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
    from tools.vendor.utt import rx2_parser as rx2  # noqa: E402  (the reference's RX2 parser)

    archive = BigArchive(args.game.resolve() / 'data/big/fedynamic.big')
    out = args.assets.resolve() / OUT
    out.mkdir(parents=True, exist_ok=True)
    index, skipped = [], []
    for entry in archive.entries:
        path = entry.path.lower()
        if not path.startswith(PREFIX) or not path[len(PREFIX):].startswith(FOLDERS) or not path.endswith('.rx2'):
            continue
        name = path[len('data/'):-len('.rx2')]
        file = rx2.RX2File(archive.read(entry))
        file.parse()
        texture = next((e for e in file.entries if e.is_texture), None)
        if texture is None:
            skipped.append((name, 'no texture'))
            continue
        header = file.data[texture.info_offset:texture.info_offset + 40]
        fmt = header[35]
        size = int.from_bytes(header[36:40], 'big')
        width, height = (size & 0x1FFF) + 1, ((size >> 13) & 0x1FFF) + 1
        if fmt != rx2.FMT_DXT5:
            skipped.append((name, f'format {rx2.FMT_NAMES.get(fmt, fmt)}'))
            continue
        level = ((width + 3) // 4) * ((height + 3) // 4) * 16
        base = file.data_base + texture.data_offset
        blocks = file.data[base:base + (texture.buffer_size or 0)]
        # The variant byte says whether the level is tiled (the reference's decode_dxt5).
        if header[28] not in (1, 2, 84):
            blocks = rx2._untile360_dxt(blocks, width, height, 16)
        blocks = rx2._swap16(blocks[:level])
        if len(blocks) != level:
            skipped.append((name, f'{len(blocks)} bytes, a level is {level}'))
            continue
        target = out / (name + '.dxt5')
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(blocks)
        index.append({'name': name, 'width': width, 'height': height})
    (out / 'thumbnails.json').write_text(json.dumps({'version': 1, 'thumbnails': index}, indent=1))
    total = sum(t['width'] * t['height'] for t in index)
    print(f'{len(index)} thumbnails ({total // 1024} KiB of DXT5) in {out}')
    for name, why in skipped:
        print(f'  skipped {name}: {why}')


if __name__ == '__main__':
    main()
