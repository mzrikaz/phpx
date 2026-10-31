#!/usr/bin/env bash
# Profile-guided build of phpx: an instrumented `dist` build runs a
# training workload, then the final build optimizes for what it measured
# (hot/cold code splitting and function order matter most: phpx's runtime
# is instruction-cache bound on framework code).
#
#   tools/pgo/build.sh TRAIN_SCRIPT
#
# TRAIN_SCRIPT gets the instrumented binary's path as its argument and
# must run it on representative PHP (servers must exit normally, e.g. with
# PHPX_SERVE_MAX_REQUESTS, for their profiles to be written). The result
# is target-pgo/dist/phpx.
set -euo pipefail
train=${1:?training script}
root=$(cd "$(dirname "$0")/../.." && pwd)
prof=$root/target-pgo/profiles
profdata=$(find "$(rustc --print sysroot)" -name llvm-profdata -type f | head -1)
[ -n "$profdata" ] || { echo "llvm-profdata missing: rustup component add llvm-tools" >&2; exit 1; }
rm -rf "$prof"; mkdir -p "$prof"
cd "$root"
RUSTFLAGS="-Cprofile-generate=$prof" CARGO_TARGET_DIR=target-pgo/gen cargo build --profile dist -p phpx-cli
"$train" "$root/target-pgo/gen/dist/phpx"
"$profdata" merge -o "$prof/merged.profdata" "$prof"/*.profraw
RUSTFLAGS="-Cprofile-use=$prof/merged.profdata" CARGO_TARGET_DIR=target-pgo cargo build --profile dist -p phpx-cli
echo "built target-pgo/dist/phpx"
