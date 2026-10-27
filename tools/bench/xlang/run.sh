#!/usr/bin/env bash
# Same programs in PHP and JavaScript: PHP, phpx and Node side by side.
#   tools/bench/xlang/run.sh [php-binary] [node-binary]
# Each program prints "<name> <ms> ms <checksum>"; the checksums must agree.
set -uo pipefail
php=${1:-php}
node=${2:-node}
dir=$(cd "$(dirname "$0")" && pwd)
phpx="$dir/../../../target/release/phpx"
printf "%-10s %10s %10s %10s  %s\n" bench php phpx node "phpx/php phpx/node"
for f in "$dir"/*.php; do
    b=$(basename "$f" .php)
    p=$("$php" -d opcache.enable_cli=1 "$f")
    x=$("$phpx" "$f")
    n=$([ -f "$dir/$b.js" ] && "$node" "$dir/$b.js" || echo "- - - -")
    ok=$([ "${p#* ms }" = "${x#* ms }" ] && echo "" || echo "  CHECKSUM MISMATCH")
    pm=$(echo "$p" | awk '{print $2}'); xm=$(echo "$x" | awk '{print $2}'); nm=$(echo "$n" | awk '{print $2}')
    printf "%-10s %10s %10s %10s  %8.2f %10.1f%s\n" "$b" "$pm" "$xm" "$nm" "$(echo "$xm / $pm" | bc -l)" "$(echo "$xm / $nm" | bc -l 2>/dev/null || echo 0)" "$ok"
done
