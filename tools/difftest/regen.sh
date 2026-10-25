#!/usr/bin/env bash
# Regenerate expected outputs for the interpreter's differential tests from
# real PHP. Run after adding or changing crates/interp/tests/php/*.php.
#   tools/difftest/regen.sh [php-binary]
set -uo pipefail
php=${1:-php}
dir=$(cd "$(dirname "$0")/../../crates/interp/tests/php" && pwd)
for f in "$dir"/*.php; do
    base=${f%.php}
    "$php" -n -d display_errors=stderr -d log_errors=0 -d error_reporting=E_ALL "$f" > "$base.out" 2>/dev/null
    code=$?
    if [ "$code" -ne 0 ]; then echo "$code" > "$base.exit"; else rm -f "$base.exit"; fi
    echo "$(basename "$f"): exit $code, $(wc -c < "$base.out") bytes"
done
"$php" --version | head -1 > "$dir/PHP_VERSION"
