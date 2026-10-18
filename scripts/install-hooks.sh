#!/bin/sh
# Installs the pre-commit hook that runs check-no-retail.sh.
set -eu
root=$(git rev-parse --show-toplevel)
printf '#!/bin/sh\nexec "%s/scripts/check-no-retail.sh"\n' "$root" > "$root/.git/hooks/pre-commit"
chmod +x "$root/.git/hooks/pre-commit"
echo "pre-commit hook installed"
