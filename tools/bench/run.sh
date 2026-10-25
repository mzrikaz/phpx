#!/usr/bin/env bash
# Compare phpx against PHP on the microbenchmarks, side by side.
#   tools/bench/run.sh [runs] [php-binary]
# Each script prints "<name> <value> ns" lines; the best of `runs` runs is
# kept per line. Runs are pinned to one core (BENCH_CPU, default 1) when
# taskset is available, and the two engines never run at the same time.
set -euo pipefail
runs=${1:-3}
php=${2:-php}
dir=$(cd "$(dirname "$0")" && pwd)
phpx="$dir/../../target/release/phpx"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
pin=()
if command -v taskset >/dev/null; then pin=(taskset -c "${BENCH_CPU:-1}"); fi
best() {
    for _ in $(seq "$runs"); do "$@"; done | awk '
        / ns$/ { v = $(NF-1); k = $0; sub(/ +[0-9.]+ ns$/, "", k)
                 if (!(k in m) || v < m[k]) m[k] = v
                 if (!(k in o)) { o[k] = ++n; ks[n] = k } }
        END { for (i = 1; i <= n; i++) printf "%s\t%.1f\n", ks[i], m[ks[i]] }'
}
printf "%-22s %8s %8s\n" "ns per operation" "phpx" "php -n"
for b in micro calls objects closures; do
    best "${pin[@]}" "$phpx" run -d memory_limit=-1 "$dir/$b.php" > "$tmp/x"
    best "${pin[@]}" "$php" -n -d memory_limit=-1 "$dir/$b.php" > "$tmp/p"
    paste "$tmp/x" "$tmp/p" | awk -F'\t' '{ printf "%-22s %8s %8s\n", $1, $2, $4 }'
done
echo
printf "%-22s %8s %8s\n" "bytes per item" "phpx" "php -n"
"$phpx" run -d memory_limit=-1 "$dir/memory.php" > "$tmp/x"
"$php" -n -d memory_limit=-1 "$dir/memory.php" > "$tmp/p"
paste "$tmp/x" "$tmp/p" | awk '{ printf "%-22s %8s %8s\n", $1, $2, $4 }'
