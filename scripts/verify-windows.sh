#!/bin/sh
# M10 stage 1 (PLAN.md §15): streamed collision is exact. For each district the oracle corpus
# runs on the whole collision world and on streamed windows (ORACLE_WINDOW=HALF,TRIGGER, metres);
# every dumped word must be bit-identical.
# usage: scripts/verify-windows.sh INSTALL [DISTRICT…]   (default Industrial University DownTown)
#   windows run: 200,60, 60,4 (a stress: rebuilt every few metres, margin 56 m) and bg:120,30 (the
#   WINDOWS="…" / PACK_WINDOW=H,T override them (the device's is packs::WINDOW, 80,20).
#   device's: built on a thread, the tick waits only when the skater is within 40 m of the edge)
set -eu
cd "$(dirname "$0")/.."
install=$(realpath "$1"); shift
assets=$install/assets
cargo build -q --release -p sim-oracle --target-dir target/check
for d in ${*:-Industrial University DownTown}; do
    out=oracle/window-$d
    rm -rf "$out" && mkdir -p "$out"
    export ORACLE_MAP="$install/maps/$d.skate"
    target/check/release/sim-oracle run "$assets" oracle/scenarios "$out/full" >/dev/null 2>&1 || true
    for w in ${WINDOWS:-200,60 60,4 bg:120,30}; do
        spec=${w#bg:}; bg=""; [ "$w" != "$spec" ] && bg=1
        env ORACLE_WINDOW=$spec ${bg:+ORACLE_WINDOW_BG=1} WINDOW_LOG=1 target/check/release/sim-oracle run "$assets" oracle/scenarios "$out/w$w" >"$out/w$w.log" 2>&1 || true
        pass=$(target/check/release/sim-oracle compare "$out/full" "$out/w$w" --exact | grep -c 'PASS$' || true)
        builds=$(grep -c '^WINDOW ' "$out/w$w.log" || true)
        echo "$d window $w: $pass/20 identical ($builds rebuilds)"
    done
    # The district pack (stage 3): the collision is read from district_<d>.pak by offset.
    packs=$(realpath "${PACKS:-../game_data/SKATEVITA}")
    if [ -f "$packs/district_$d.pak" ]; then
        # The pack starts at its validated spawn (district_spawns.txt), which is not always the
        # map's authored one: the whole world runs from the same spawn for this comparison.
        spawn=$(awk -v d="$d" '$1 == d { print $2 }' "$packs/district_spawns.txt")
        ORACLE_SPAWN="$spawn" target/check/release/sim-oracle run "$assets" oracle/scenarios "$out/full-packspawn" >/dev/null 2>&1 || true
        unset ORACLE_MAP
        env ORACLE_WINDOW=${PACK_WINDOW:-120,30} ORACLE_WINDOW_BG=1 WINDOW_LOG=1 target/check/release/sim-oracle run-live --packs "$packs" --map "$d" oracle/scenarios "$out/wpack" >"$out/wpack.log" 2>&1 || true
        pass=$(target/check/release/sim-oracle compare "$out/full-packspawn" "$out/wpack" --exact | grep -c 'PASS$' || true)
        echo "$d district pack (${PACK_WINDOW:-120,30} background): $pass/20 identical"
    fi
done
