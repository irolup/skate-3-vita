"""Write the Extras > Licenses text (upstream `tools/gen_licenses.py` at b110bab, for the Vita): this project's license,
every crate skate-vita links on the Vita (`cargo metadata`, normal dependencies reachable from skate-vita; build and dev
dependencies left out) and the C libraries it links from VitaSDK.

Run after changing dependencies:  python3 tools/gen_licenses.py
Output: crates/skate-ui/src/fe/licenses.txt (embedded in the game; one line per entry, a blank line between sections).
"""
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / 'crates' / 'skate-ui' / 'src' / 'fe' / 'licenses.txt'
# The C libraries crates/skate-vita/build.rs links besides the Sce* stubs (their licenses are not shipped with
# VitaSDK: each project's own page has it).
C_LIBRARIES = [
    'vitaGL (github.com/Rinnegatamante/vitaGL)',
    'vitaShaRK (github.com/Rinnegatamante/vitaShaRK)',
    'math-neon (github.com/andrepuschmann/math-neon)',
    'newlib and libstdc++ (VitaSDK toolchain, vitasdk.org)',
]


def main():
    meta = json.loads(subprocess.run(
        ['cargo', 'metadata', '--format-version', '1', '--locked', '--offline',
         '--filter-platform', 'armv7-sony-vita-newlibeabihf'],
        cwd=ROOT, check=True, capture_output=True).stdout)
    packages = {p['id']: p for p in meta['packages']}
    nodes = {n['id']: n for n in meta['resolve']['nodes']}
    start = next(p['id'] for p in meta['packages'] if p['name'] == 'skate-vita')
    seen, todo = set(), [start]
    while todo:
        pid = todo.pop()
        if pid in seen:
            continue
        seen.add(pid)
        for dep in nodes[pid]['deps']:
            if any(k['kind'] is None for k in dep['dep_kinds']):
                todo.append(dep['pkg'])
    workspace = set(meta['workspace_members'])
    third = sorted({(packages[p]['name'], packages[p]['version'], packages[p].get('license') or 'see the crate')
                    for p in seen if p not in workspace})
    lines = [
        'Skate Vita',
        'This program is free software: GNU General Public License, version 3 (see LICENSE).',
        'Based on the Skate 3 Rust engine (github.com/SK8-ENGINE/skate-3-rust-engine), GNU GPL version 3.',
        'Skate 3 is a trademark of Electronic Arts. This project ships no game code or data: the game files come from your own copy.',
        '',
        f'Third-party software ({len(third)} crates, each under its own license):',
    ]
    lines += [f'{name} {version} ({lic})' for name, version, lic in third]
    lines += ['', 'C libraries (each under its own license):']
    lines += C_LIBRARIES
    OUT.write_text('\n'.join(lines) + '\n', encoding='utf-8', newline='\n')
    print(f'wrote {OUT} ({len(third)} crates)')


if __name__ == '__main__':
    sys.exit(main())
