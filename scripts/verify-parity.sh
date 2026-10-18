#!/bin/sh
# M3 optimisation check (PLAN.md §5.5: every performance change is followed by an
# oracle run). Rebuilds sim-oracle with both arithmetics and checks:
#   1. mul_add build: bit-identical to the reference dumps (oracle/ref);
#   2. no-fma build (the Vita arithmetic): bit-identical to oracle/nofma-base.
# usage: scripts/verify-parity.sh ASSETS        (--baseline: write oracle/nofma-base)
set -eu
cd "$(dirname "$0")/.."
assets=$1
cargo build -q --release -p sim-oracle --target-dir target/check
cargo build -q --release -p sim-oracle --features no-fma --target-dir target/nofma
rm -rf oracle/verify-fma oracle/verify-nofma
if [ "${2:-}" = "--baseline" ]; then
    rm -rf oracle/nofma-base
    target/nofma/release/sim-oracle run "$assets" oracle/scenarios oracle/nofma-base >/dev/null 2>&1 || true
    echo "baseline written: oracle/nofma-base"
    exit 0
fi
target/check/release/sim-oracle run "$assets" oracle/scenarios oracle/verify-fma >/dev/null 2>&1 || true
target/nofma/release/sim-oracle run "$assets" oracle/scenarios oracle/verify-nofma >/dev/null 2>&1 || true
fma=$(target/check/release/sim-oracle compare oracle/ref oracle/verify-fma --exact | grep -c 'PASS$' || true)
nofma=$(target/check/release/sim-oracle compare oracle/nofma-base oracle/verify-nofma --exact | grep -c 'PASS$' || true)
echo "mul_add vs reference: $fma/20   no-fma vs baseline: $nofma/20"
[ "$fma" = 20 ] && [ "$nofma" = 20 ]
