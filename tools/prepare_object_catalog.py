#!/usr/bin/env python3
"""Exports retail's Object Dropper catalogue (step D2 of docs/object-dropper-survey.md) into the prepared install.

Every item of the two skate.Park recipes in `content/parkassets.big` (the static "terrain" pieces and the dynamic
"props") is converted with the reference's own skate.Park piece converter (`tools/asset_pipeline/park_layout.py`
`piece_template`, the one setup runs for the Fullpark layouts) and written with its `dynamic_props.write_placements`
as two template packages, one placement per item at the identity pose:

    <assets>/private/object-catalog/terrain.skate + terrain.json   static pieces (render, rails, collision)
    <assets>/private/object-catalog/props.skate + props.json       dynamic props (render, physics, collision)
    <assets>/private/object-catalog/catalog.json                    the tree, the items and the editor's tuning

`catalog.json`: the category tree of the vault class `fe_park_categories` (super categories have no parent in the
tree; retail reads their name at field 0x551FA5F0841F22A9, 825D16B8), every recipe item in the recipes' order with
its sub category (the recipe's `<comp>`), its template (None for the pieces without a model: teleporters and the spawner, which carry their
footprint `bounds` from the model's locator and the `indicator` row the editor draws them with, vault class
`0xD01B99210BA7323C` field `0x4D9046773E165576`, retail `82866AD8`; the teleporters their `teleport` [role: 0 start,
1 end; channel]) and its caps (FPSCost, Unlock, ...), its snap sockets (terrain pieces: the model's type-8 locators, docs/object-dropper-survey.md
"Snapping"), and the raw records of the editor's tuning classes (`objectdropper`, `editor_cursor`,
`editor_camera`, `skatepark_presentation`, `fe_park_itemtypes`, `skatepark_unlock_sets`), and each stock park
layout's player start (`park_starts`, the `.park` editor tail's first matrix: where the park's spawner stands) and
Park Setup's lots and templates (`park_lots`). The reference's scripts
are imported from a checkout at PR #52 `8df32c51` or later (it needs `park_layout.py`), read-only. The output is
copyrighted data from the user's own disc: it stays in the private prepared folder.

    tools/prepare_object_catalog.py --game ../game_data/skate3_x360 --assets $INSTALL/assets \
        --reference ~/tools/upstream-8df32c51
"""
import argparse
import json
import struct
import sys
import tempfile
import time
import xml.etree.ElementTree as ET
from pathlib import Path

sys.dont_write_bytecode = True

OUT = 'private/object-catalog'
# The stock park layouts (`.park`) setup copies from the disc.
PARKS = 'private/stock/data/skatepark'
TUNING_CLASSES = ('objectdropper', 'editor_cursor', 'editor_camera', 'skatepark_presentation', 'fe_park_itemtypes',
                  'skatepark_unlock_sets')
# fe_park_categories fields (vault, names unresolved): the name retail reads, the item type, the sort order and the
# bool set on the three super categories.
NAME, ITEM_TYPE, ORDER, SUPER = 'Hash_551FA5F0841F22A9', 'Hash_E110094B3121CE36', 'Hash_EBF090FD', 'Hash_24F106C9'


def text(field):
    data = field['data']
    return data if isinstance(data, str) and not all(c in '0123456789ABCDEFabcdef' for c in data) else \
        bytes.fromhex(data).split(b'\0')[0].decode('latin-1')


# A model's snap locators (retail 82865230): the part's EB001E points (0x70 bytes: frame at +0x00, type at +0x60,
# group at +0x64), type 8; group 0 = a snap point, points sharing another group = one snap face.
POINTS, POINT_STRIDE, SNAP = 0xEB001E, 0x70, 8


def sockets(raw, sections, matrix):
    """[[group, frame (4x4 row vectors, model space)]] of one piece model."""
    out = []
    for base, _, size, _, _, kind in sections(raw):
        if kind != POINTS:
            continue
        count, start = struct.unpack_from('>2I', raw, base)
        if start + count * POINT_STRIDE > size:
            raise ValueError('snap point section overruns')
        for k in range(count):
            at = base + start + k * POINT_STRIDE
            kind_of, group = struct.unpack_from('>2I', raw, at + 0x60)
            if kind_of == SNAP:
                out.append([group, [round(float(x), 6) for row in matrix(raw, at) for x in row]])
    return out


def piece_indicators(collections, rows, key_hash):
    """{piece id: indicator row key} of the vault class `0xD01B99210BA7323C` (the teleporter pieces and the spawner):
    its field `0x4D9046773E165576` is a reference to a `ribbon_indicator` row, which the editor draws at the piece
    (retail `82866AD8`). Keys as the visual indicators export names them (`Hash_<16 hex>`)."""
    cls = 'Hash_D01B99210BA7323C'
    piece_field, ref_field = key_hash('Hash_BCFAA75D7A7A5918'), key_hash('Hash_4D9046773E165576')
    result = {}
    for row in collections['collections']:
        if key_hash(row['class']) != key_hash(cls):
            continue
        fields, _ = rows.resolve(cls, row['key'])
        piece, ref = (fields.get(piece_field) or {}).get('data'), (fields.get(ref_field) or {}).get('data')
        if piece and int(piece, 16) and ref and int(ref[16:32], 16):
            result[f'{int(piece, 16):016X}'] = f'Hash_{ref[16:32].upper()}'
    return result


def park_lots(collections, rows, key_hash, assets):
    """Park Setup's lots and templates (retail FE state 0x3A, OnEnter 825D1BF8): the `world` rows under
    `default_skateparks`, and each lot's selectable children (flag 0xFC96AC5110FA08F6; thumbnail, field
    0x6327D79EE89200D8): key, display name (field
    0x095E1D9778F0C6F4), thumbnail, layout file stem (field 0x79B248E8469DC5DA; none for a bare lot), origin hash (the
    key's, as a `.park` header names its template world) and map (the row's stream, `park-layouts/worlds.json`); the
    lot's unlock key (field 0x3A3029D4DCA2E714). Sorted as retail (field 0xE69A80EE0520102D)."""
    worlds = json.loads((assets / 'private/park-layouts/worlds.json').read_text())['worlds']
    field = lambda fields, name: (fields.get(key_hash(name)) or {}).get('data')
    world_rows = [r for r in collections['collections'] if r['class'] == 'world']
    lots = []
    for lot in world_rows:
        if lot['parent'] != 'default_skateparks':
            continue
        fields, _ = rows.resolve('world', lot['key'])
        templates = []
        for row in world_rows:
            if row['parent'] != lot['key']:
                continue
            f, _ = rows.resolve('world', row['key'])
            thumb = field(f, 'Hash_6327D79EE89200D8')
            selectable = (field(f, 'Hash_FC96AC5110FA08F6') or '00')[:2] == '01'
            if not thumb or not selectable:
                continue
            layout = field(f, 'Hash_79B248E8469DC5DA')
            stream = (worlds.get(row['key']) or {}).get('stream') or ''
            templates.append(dict(world=row['key'], name=field(f, 'Hash_095E1D9778F0C6F4'), thumb=thumb,
                                  layout=Path(layout.replace('\\', '/')).stem if layout else None,
                                  origin=f'{key_hash(row["key"]):016X}',
                                  map=stream[5:] if stream.lower().startswith('dist_') else stream))
        # A bare lot's template has no stream of its own: its lot's (a sibling's).
        known = next((t['map'] for t in templates if t['map']), '')
        for t in templates:
            t['map'] = t['map'] or known
        unlock = field(fields, 'Hash_3A3029D4DCA2E714')
        order = int(field(fields, 'Hash_E69A80EE0520102D') or '0', 16)
        lots.append(dict(lot=lot['key'], name=field(fields, 'Hash_095E1D9778F0C6F4'), unlock=unlock, templates=templates, order=order))
    # Retail sorts the lots by this field, smallest first (comparator 825D1AD0): Starter 1000, Industrial 2000, Downtown
    # 3000, the Stadium 4000.
    lots.sort(key=lambda l: l.pop('order'))
    return lots


def comps(raw):
    """{item id: sub category key} of one recipe XML (the `<comp n=...>` around each `<mod>`)."""
    data = raw.decode('utf-8')
    root = ET.fromstring(data.split('?>', 1)[1] if data.lstrip().startswith('<?') else data)
    return {f"{int(mod.get('id'), 16):016X}": comp.get('n', '').lower() for comp in root.iter('comp') for mod in comp.iter('mod')}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--game', type=Path, required=True, help='the extracted disc')
    parser.add_argument('--assets', type=Path, required=True, help="the prepared install's assets folder")
    parser.add_argument('--reference', type=Path,
                        default=Path(__file__).resolve().parent.parent.parent / 'skate-3-rust-engine',
                        help='a skate-3-rust-engine checkout at PR #52 8df32c51 or later (default: next to this repo)')
    args = parser.parse_args()
    reference = args.reference.resolve()
    if not (reference / 'tools/asset_pipeline/park_layout.py').is_file():
        sys.exit(f'prepare_object_catalog: {reference} has no tools/asset_pipeline/park_layout.py (needs PR #52 8df32c51+)')
    sys.path.insert(0, str(reference))
    from tools.owned_game.big import BigArchive  # noqa: E402  (the reference's readers and converters)
    from tools.asset_pipeline.park_layout import Sources, instance_id, parse, piece_template, teleporter_roles  # noqa: E402
    from tools.asset_pipeline.dynamic_props import locators, matrix, sections, write_placements  # noqa: E402
    from tools.asset_pipeline.environment import Collections, key_hash  # noqa: E402

    started = time.time()
    assets = args.assets.resolve()
    archive = BigArchive(args.game.resolve() / 'data/content/parkassets.big')
    sources = Sources([archive])
    sub_of = {}
    for entry in archive.entries:
        path = entry.path.replace('\\', '/').lower()
        if path.endswith('.xml') and path.startswith('data/content/recipe/'):
            sub_of.update(comps(archive.read(entry)))
    collections = json.loads((assets / 'private/stock/skater-collections.json').read_text())
    rows = Collections(collections)
    roles = teleporter_roles(rows)
    indicators = piece_indicators(collections, rows, key_hash)

    # The tree: fe_park_categories records, by parent.
    categories = {}
    for row in collections['collections']:
        if key_hash(row['class']) != key_hash('fe_park_categories'):
            continue
        fields, _ = rows.resolve('fe_park_categories', row['key'])
        get = lambda name: next((v for k, v in fields.items() if f'{k:016X}'.startswith(name[5:].upper())), None)
        categories[row['key']] = dict(
            name=text(get(NAME)) if get(NAME) else row['key'], parent=row['parent'] or None,
            item_type=text(get(ITEM_TYPE)) if get(ITEM_TYPE) else None,
            order=struct.unpack('>i', bytes.fromhex(get(ORDER)['data'][:8]))[0] if get(ORDER) else 0,
            super=bool(get(SUPER) and get(SUPER)['data'][:2] == '01'))
    by_key = {key: c['name'] for key, c in categories.items()}
    super_categories = sorted(({'key': c['name'], 'order': c['order']} for c in categories.values() if c['super']),
                              key=lambda c: c['order'])
    sub_categories = sorted(({'key': c['name'], 'parent': by_key.get(c['parent']), 'order': c['order'],
                              'item_type': c['item_type']} for c in categories.values() if not c['super']),
                            key=lambda c: (c['parent'] or '', c['order']))

    out = assets / OUT
    out.mkdir(parents=True, exist_ok=True)
    items, report_lines = [], []
    with tempfile.TemporaryDirectory(prefix='skate-catalog-') as temporary:
        work = Path(temporary)
        for kind, rows_of_kind, recipe_rows in (('terrain', sources.static, None), ('props', sources.dynamic, sources.dynamic)):
            templates, textures, placements = {}, {}, []
            (work / kind).mkdir()
            for key in sorted(rows_of_kind):
                row = rows_of_kind[key]
                template = None
                if key not in roles:
                    try:
                        found = piece_template(sources, key, work / kind, recipe_rows)
                    except (ValueError, KeyError, IndexError, struct.error) as error:
                        report_lines.append(f'{kind} {key} ({row["name"]}): {error}')
                        found = None
                    if found is not None:
                        # Props are keyed by their DMO template (the arena id), as the park's movable pieces.
                        template = row['arena'] if kind == 'props' else key
                        templates[template], more = found
                        textures.update(more)
                        # The MOBJ identity is the id's low 32 bits, which recipe ids share (`...03E38703`): a
                        # hashed instance id as park_layout.instance_id, the item id beside it.
                        placements.append(dict(instance_id=instance_id('object-catalog', kind, key), item_id=key,
                                               template_id=template, name=row['name'],
                                               matrix=[[1.0, 0, 0, 0], [0, 1.0, 0, 0], [0, 0, 1.0, 0], [0, 0, 0, 1.0]]))
                snaps = []
                if kind == 'terrain' and template is not None:
                    snaps = sockets(sources.read(sources.models, row['arena']), sections, matrix)
                item = dict(id=key, name=row['name'], kind=kind, sub=sub_of.get(key), template=template,
                            teleporter=key in roles, caps=row['caps'], sockets=snaps)
                if key in roles:
                    item['teleport'] = list(roles[key])
                if template is None and key in indicators and row['arena'] in sources.models:
                    found = locators(sources.read(sources.models, row['arena']))
                    if found:
                        item.update(bounds=found[0]['bounds'], indicator=indicators[key])
                items.append(item)
            write_placements(f'object-catalog-{kind}', '', placements, templates, textures, out / f'{kind}.skate')
            print(f'{kind}: {len(placements)} templates written, {len(textures)} textures')

    tuning = {}
    for row in collections['collections']:
        for name in TUNING_CLASSES:
            if key_hash(row['class']) == key_hash(name):
                tuning.setdefault(name, {})[row['key']] = dict(parent=row['parent'] or None, fields=row['fields'])
    # The catalogue pages list items in the recipes' order (each `<comp>`'s `<mod>`s: the Teleports page is Start 1,
    # End 1, Start 2, ...), not by id.
    position = {key: i for i, key in enumerate(sub_of)}
    items.sort(key=lambda i: position.get(i['id'], len(position)))
    lots = park_lots(collections, rows, key_hash, assets)
    park_starts = {}
    for path in sorted((assets / PARKS).glob('*.park')):
        start = parse(path.read_bytes())['start']
        if start is not None:
            park_starts[path.stem] = start
    catalog = dict(version=1, source=dict(reference='park_layout.py piece_template', archive='content/parkassets.big'),
                   super_categories=super_categories, sub_categories=sub_categories, items=items, tuning=tuning,
                   park_starts=park_starts, park_lots=lots, failed=report_lines)
    (out / 'catalog.json').write_text(json.dumps(catalog, indent=1))
    missing = sum(1 for i in items if i['template'] is None)
    print(f'object-catalog: {len(items)} items ({missing} without a model), {len(super_categories)} super / '
          f'{len(sub_categories)} sub categories, {len(report_lines)} failures, {time.time() - started:.0f} s -> {out}')
    for line in report_lines:
        print('  ' + line)


if __name__ == '__main__':
    main()
