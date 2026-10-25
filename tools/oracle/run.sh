#!/usr/bin/env bash
# Diff phpx's operator trees against nikic/php-parser on a corpus.
# Usage: tools/oracle/run.sh <path-to-nikic/php-parser> <dir-with-php-files>...
set -euo pipefail
parser=$1; shift
root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
find "$@" -name '*.php' | sort > "$tmp/files"
cargo build -q --release -p phpx-cli --manifest-path "$root/Cargo.toml"
"$root/target/release/phpx" optrees < "$tmp/files" > "$tmp/ours"
php -d memory_limit=2G -d error_reporting=0 "$root/tools/oracle/optrees.php" "$parser" < "$tmp/files" > "$tmp/theirs"
python3 "$root/tools/oracle/compare.py" "$tmp/ours" "$tmp/theirs"
