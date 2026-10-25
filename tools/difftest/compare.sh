#!/usr/bin/env bash
# Run any PHP files with both php and phpx and diff stdout + exit code.
#   tools/difftest/compare.sh file.php...
set -uo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cargo build -q --release -p phpx-cli --manifest-path "$root/Cargo.toml" || exit 1
fail=0
for f in "$@"; do
    want=$(php -n -d display_errors=stderr -d log_errors=0 "$f" 2>/dev/null; echo "exit=$?")
    got=$("$root/target/release/phpx" run "$f" 2>/dev/null; echo "exit=$?")
    if [ "$want" == "$got" ]; then echo "PASS $f"; else echo "FAIL $f"; diff <(echo "$want") <(echo "$got") | head -20; fail=1; fi
done
exit $fail
