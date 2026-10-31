#!/bin/bash
# Example training run on this machine's apps (Wit-ERP pages, the iDriver API, the
# cross-language suite and the differential tests): tools/pgo/build.sh tools/pgo/train-local.sh
BIN=$1
COOKIE=$(cat ~/pxbench/cookie.org); T=/app/a6b27945-bb4b-4873-8c14-f2f24d945cf2
PAGES="$T $T/accounts $T/brands $T/expenses $T/products $T/purchases $T/payments $T/products/create $T/purchases/create"
R=25; NP=$(echo $PAGES | wc -w)
cd /home/rikaz/Projects/Wit-ERP
env PHPX_SERVE_MAX_REQUESTS=$((R*NP)) DB_HOST=127.0.0.1 DB_PORT=3307 REDIS_HOST=127.0.0.1 REDIS_PORT=6380 $BIN -S 127.0.0.1:8991 -t public >/dev/null 2>&1 & P=$!
sleep 2
for r in $(seq $R); do for p in $PAGES; do curl -s -o /dev/null -b "$COOKIE" "http://127.0.0.1:8991$p"; done; done
wait $P
cd /home/rikaz/Projects/iDriver/iDriver-Backend
[ "$(redis-cli -p 6379 -n 14 exists 'idriver_database_idriver_cacheapi:api/sms/status')" = 1 ] || { echo "cache key missing"; exit 1; }
~/pxbench/idrclean.sh
env PHPX_SERVE_MAX_REQUESTS=200 DB_HOST=127.0.0.1 DB_PORT=3307 REDIS_HOST=127.0.0.1 REDIS_DB=13 REDIS_CACHE_DB=14 REDIS_CLIENT=predis $BIN -S 127.0.0.1:8992 server.php >/dev/null 2>&1 & P=$!
sleep 1.5
for i in $(seq 200); do [ $((i % 40)) = 0 ] && ~/pxbench/idrclean.sh; curl -s -o /dev/null http://127.0.0.1:8992/api/sms/status; done
wait $P; ~/pxbench/idrclean.sh
for f in /home/rikaz/Projects/php/tools/bench/xlang/*.php; do $BIN $f >/dev/null; done
cd /home/rikaz/Projects/php/crates/interp/tests/php; for f in *.php; do timeout 20 $BIN $f >/dev/null 2>&1; done
echo trained
