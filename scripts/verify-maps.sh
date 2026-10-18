#!/bin/sh
# M6 park check (PLAN.md §7: "the oracle scenarios run on the park map collision"):
# for each park baked into PACKS (vita-bake maps; spawns in PACKS/spawns.txt),
#   1. reference vs port on the converted .skate with that spawn (mul_add): exact;
#   2. port, no-fma, on the .skate: the baseline (oracle/map-<Park>/nofma);
#   3. port, no-fma, from the packs only (sim.pak, anim.pak, map_<Park>.pak, strict):
#      must equal 2.
# usage: scripts/verify-maps.sh INSTALL PACKS [PARK…]
#   INSTALL = game_data/prepared/installations/<id>, PACKS = game_data/SKATEVITA
set -eu
cd "$(dirname "$0")/.."
install=$(realpath "$1"); packs=$(realpath "$2"); shift 2
assets=$install/assets
cargo build -q --release -p sim-oracle --target-dir target/check
cargo build -q --release -p sim-oracle --features no-fma --target-dir target/nofma
parks=${*:-$(cut -d' ' -f1 "$packs/spawns.txt")}
for park in $parks; do
    spawn=$(awk -v p="$park" '$1 == p {print $2}' "$packs/spawns.txt")
    out=oracle/map-$park
    rm -rf "$out" && mkdir -p "$out"
    export ORACLE_MAP="$install/maps/$park.skate" ORACLE_SPAWN="$spawn"
    tools/sim-oracle/reference.sh "$assets" oracle/scenarios "$out/ref" >"$out/ref.log" 2>&1
    target/check/release/sim-oracle run "$assets" oracle/scenarios "$out/fma" >/dev/null 2>&1 || true
    target/nofma/release/sim-oracle run "$assets" oracle/scenarios "$out/nofma" >/dev/null 2>&1 || true
    unset ORACLE_MAP ORACLE_SPAWN
    target/nofma/release/sim-oracle run-live --packs "$packs" --map "$park" oracle/scenarios "$out/packs" >/dev/null 2>&1 || true
    ref=$(target/check/release/sim-oracle compare "$out/ref" "$out/fma" --exact | grep -c 'PASS$' || true)
    packed=$(target/check/release/sim-oracle compare "$out/nofma" "$out/packs" --exact | grep -c 'PASS$' || true)
    echo "$park: port vs reference $ref/20, packs vs .skate (no-fma) $packed/20"
done
