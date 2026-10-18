#!/bin/sh
# Copies the reference's small synthetic test fixtures into this repo. They are not
# retail data, but their extensions (.rwcmset, .skate) are on the blocked list, so they
# stay gitignored and every clone copies them here instead (PROVENANCE.md).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
ref="$root/../skate-3-rust-engine"
[ -d "$ref" ] || { echo "missing $ref; run scripts/setup-references.sh first"; exit 1; }
copy() { # src dst
    mkdir -p "$(dirname "$root/$2")"
    cp "$ref/$1" "$root/$2"
    echo "fixture: $2"
}
copy crates/skate-data/tests/fixtures/retail-collision.rwcmset crates/skate-data/tests/fixtures/retail-collision.rwcmset
copy maps/format-demo.skate maps/format-demo.skate
