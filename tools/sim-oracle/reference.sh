#!/bin/sh
# Writes the reference dumps (PLAN.md §5.4): a scratch copy of skate-3-rust-engine with
# the shared harness (tools/sim-oracle/harness) injected, run as an ignored test. The
# reference folder itself is never modified.
#
# usage: tools/sim-oracle/reference.sh ASSETS SCENARIOS OUT
# env:   ORACLE_REFERENCE    the reference checkout (default ../skate-3-rust-engine)
#        ORACLE_REF_SCRATCH  scratch copy       (default target/oracle-reference/src)
#        ORACLE_REF_TARGET   its cargo target   (default target/oracle-reference/target)
#        ORACLE_MAP          optional .skate map to run on instead of the Course (M6)
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
ref=${ORACLE_REFERENCE:-$root/../skate-3-rust-engine}
assets=$(realpath "$1")
scenarios=$(realpath "$2")
out=$(realpath -m "$3")
scratch=${ORACLE_REF_SCRATCH:-$root/target/oracle-reference/src}
target=${ORACLE_REF_TARGET:-$root/target/oracle-reference/target}

commit=$(git -C "$ref" rev-parse --short HEAD)
[ "$commit" = "74c124a" ] || echo "warning: reference is at $commit, PROVENANCE.md pins 74c124a" >&2

mkdir -p "$scratch"
rsync -a --delete --exclude target --exclude .git "$ref/" "$scratch/"
game="$scratch/crates/skate-game/src"
cp "$root/tools/sim-oracle/harness/oracle_harness.rs" "$game/tests/oracle_harness.rs"
cp "$root/tools/sim-oracle/harness/oracle_input.rs" "$game/input/oracle_input.rs"
printf '\n#[cfg(test)]\nmacro_rules! skate_vita_only {\n    ($($t:tt)*) => {};\n}\n#[cfg(test)]\n#[path = "tests/oracle_harness.rs"]\npub(crate) mod oracle_harness;\n' >> "$game/physics.rs"
printf '\n#[cfg(test)]\n#[path = "input/oracle_input.rs"]\npub(crate) mod oracle_input;\n' >> "$game/input.rs"

# ORACLE_ITERATIONS=N: the reference solves with N constraint iterations (the port's
# "Fast" physics is 25), the port's harness and run-live apply the same value.
settings="$game/physics/settings.rs"
if [ -n "${ORACLE_ITERATIONS:-}" ]; then
    sed -i "s/pub(crate) const SIMULATION_ITERATIONS: u32 = 50;/pub(crate) const SIMULATION_ITERATIONS: u32 = $ORACLE_ITERATIONS;/" "$settings"
    grep -q "SIMULATION_ITERATIONS: u32 = $ORACLE_ITERATIONS;" "$settings" || { echo "reference.sh: cannot set the iterations" >&2; exit 1; }
fi

cd "$scratch"
ORACLE_ASSETS="$assets" ORACLE_SCENARIOS="$scenarios" ORACLE_OUT="$out" \
    cargo test --locked --target-dir "$target" -p skate-game \
    physics::oracle_harness::oracle_dump -- --ignored --exact --nocapture
