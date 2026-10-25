#!/usr/bin/env bash
# Compare phpx's Redis class with phpredis on a live server (database 15 is
# flushed). Reference PHP connects with RH/RP (default lerd-redis:6379),
# phpx with PHPX_RH/PHPX_RP (default 127.0.0.1:6380): the same server seen
# from inside and outside the lerd containers.
#   tools/redis/compare.sh [php-binary]
set -uo pipefail
php=${1:-php}
dir=$(cd "$(dirname "$0")" && pwd)
phpx="$dir/../../target/release/phpx"
status=0
for f in "$dir"/*.php; do
    want=$("$php" "$f" 2>&1)
    got=$(RH=${PHPX_RH:-127.0.0.1} RP=${PHPX_RP:-6380} "$phpx" "$f" 2>&1)
    if [ "$want" == "$got" ]; then
        echo "$(basename "$f"): same"
    else
        echo "$(basename "$f"): differs"
        diff <(echo "$want") <(echo "$got") | head -20
        status=1
    fi
done
exit $status
