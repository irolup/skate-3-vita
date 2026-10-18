#!/bin/sh
# Fails if retail-derived Skate 3 data is staged (or, with --all, tracked) in git.
# Used as the pre-commit hook (scripts/install-hooks.sh) and before every push.
set -eu

if [ "${1:-}" = "--all" ]; then
    files=$(git ls-files)
else
    files=$(git diff --cached --name-only --diff-filter=ACMR)
fi

bad=0
for f in $files; do
    case "$f" in
        game_data/*|SKATEVITA/*|assets/*|oracle/*|\
        *.pak|*.iso|*.xex|*.big|*.abin|*.stategraph|*.pat|*.skate|*.rwcm|*.rwcmset|*.irradiance|\
        */input.cfg|input.cfg|*skater-collections.json|*physics-skeletons.json|*skater.glb)
            echo "BLOCKED (retail data pattern): $f"; bad=1 ;;
    esac
    # Anything over 5 MB needs a deliberate review.
    if [ -f "$f" ] && [ "$(wc -c < "$f")" -gt 5242880 ]; then
        echo "BLOCKED (file over 5 MB, review it): $f"; bad=1
    fi
done

if [ "$bad" -ne 0 ]; then
    echo "Retail-derived or oversized files must not be committed: nothing from the game disc, or made from it, goes in this repository."
    exit 1
fi
echo "check-no-retail: OK"
