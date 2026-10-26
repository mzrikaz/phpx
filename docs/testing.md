# Testing

phpx's correctness criterion is simple: **for the same script, phpx must
print exactly what PHP 8.5 prints and exit with the same status.** Tests are
therefore differential: expected output is produced by real PHP, never
written by hand.

## Test suites

| Suite | Location | What it checks |
|---|---|---|
| Differential (main) | `crates/interp/tests/differential.rs` + `crates/interp/tests/php/*.php` | Each `.php` file's stdout must equal the stored `.out`; exit code must equal `.exit` (absent = 0) |
| Parser | `crates/syntax/tests/parse.rs` + `crates/syntax/tests/fixtures/` | Parsing and diagnostics |
| Runtime | `crates/runtime/tests/promise.rs`, `net.rs` | Scheduler, promises, timers, TCP |
| Parser oracle | `tools/oracle/` | Operator trees compared with `nikic/php-parser` |

Run everything:

```sh
cargo test
```

### Differential test files

| File | Area |
|---|---|
| `01_basics.php` | Variables, operators, control flow, output |
| `02_strings.php` | String functions, formatting |
| `03_arrays.php` | Array functions, sorting, copy-on-write |
| `04_classes.php` | Classes, interfaces, traits, enums, magic methods |
| `05_functions.php` | Functions, closures, named args, spread |
| `06_exceptions.php` | Exceptions, uncaught exception output (non-zero exit) |
| `07_json_regex.php` | `json_*`, `preg_*` |
| `08_async.php` | `async`, promises (with a sequential polyfill so real PHP prints the same) |
| `09_references.php` | References everywhere |
| `10_goto.php` | `goto` |
| `11_generators.php` | Generators |
| `12_jit_edges.php` | Numeric JIT edge cases (overflow, division, deopt) |
| `13_globals_dynamic.php` | `global`, `$GLOBALS`, variable variables, `extract`, `eval` |
| `14_try_native.php` | `try`/`catch`/`finally` in compiled code (non-zero exit) |
| `15_destructors.php` | `__destruct` timing |
| `16_dynamic_calls.php` | Dynamic calls `$f()`, `$obj->$m()`, `$obj->$p` |

Expected outputs were generated with the PHP build named in
`crates/interp/tests/php/PHP_VERSION` (currently PHP 8.5.10 CLI, NTS).

### Run in all engine modes

The suite must pass in all three modes; run all of them before declaring a
change done:

```sh
cargo test -p phpx-interp --test differential                       # default tiering
PHPX_NATIVE=all cargo test -p phpx-interp --test differential       # compile everything on first call
PHPX_NATIVE=off PHPX_NOJIT=1 cargo test -p phpx-interp --test differential   # pure interpreter
```

Run a subset with `PHPX_TEST=<substring of file name>`, e.g.
`PHPX_TEST=09_ref cargo test -p phpx-interp --test differential`.
Failures report the first differing line with the expected and actual text.

Each file runs on its own thread because the runtime is thread-local.

## Adding a differential test

1. Write `crates/interp/tests/php/NN_topic.php`. Print everything you want
   checked (`echo`, `var_dump`, `print_r`); keep it deterministic (no
   timestamps, random numbers, absolute paths, or timings in the output).
2. Generate the expected output with real PHP:

   ```sh
   tools/difftest/regen.sh            # uses `php` from PATH; or: tools/difftest/regen.sh /path/to/php
   ```

   This runs every test file with
   `php -n -d display_errors=stderr -d log_errors=0 -d error_reporting=E_ALL`,
   writes `NN_topic.out`, writes `NN_topic.exit` for non-zero exit codes
   (and deletes stale ones), and updates `PHP_VERSION`. Warnings go to
   stderr and are not compared (phpx also prints them to stderr).
3. Run the suite in all three modes (above).
4. Commit the `.php`, `.out` and (if any) `.exit` files together.

Note that `regen.sh` regenerates **all** expected outputs; review the diff
of the other `.out` files — they should not change unless the PHP version
changed.

### The `php` command in this environment

`php` here is a **lerd** wrapper (`~/.local/share/lerd/bin/php`) that runs
PHP inside a container. The container cannot see `/tmp`, so scripts you
want to run with it must live under `$HOME` (the repository itself is fine;
for scratch files use something like `~/.cache/phpx-scratch/`). A script in
`/tmp` fails with "Could not open input file".

## Ad-hoc comparison

```sh
tools/difftest/compare.sh some/file.php other.php
```

Builds the release binary, runs each file with real `php` and with phpx,
and compares stdout plus exit code (prints `PASS` / `FAIL` with the first
20 lines of diff). Useful for reproducing a bug before turning it into a
test.

Quick manual diff, including all three engine modes:

```sh
f=~/.cache/phpx-scratch/t.php
php -n -d display_errors=stderr "$f" > want.txt 2>/dev/null
for mode in "" "PHPX_NATIVE=all" "PHPX_NATIVE=off PHPX_NOJIT=1"; do
  env $mode ./target/release/phpx run "$f" 2>/dev/null | diff -q want.txt - && echo "ok [$mode]"
done
```

## Parser checks on real code

```sh
./target/release/phpx check --summary ~/src/wordpress
./target/release/phpx check --summary ~/src/drupal/core
```

The summary line counts files that are OK, that use unsupported features
(`E1xxx`) and that have syntax errors (`E0xxx`), followed by a histogram of
diagnostic codes. See [cms-compatibility.md](cms-compatibility.md) for the
current results.

## Parser oracle

```sh
tools/oracle/run.sh <path to nikic/php-parser checkout> <php dirs...>
```

Dumps operator trees (precedence/associativity) from both phpx
(`phpx optrees`) and nikic/php-parser and diffs them, which catches
precedence bugs that output-level tests may miss. With the lerd `php`
wrapper, both the php-parser checkout and the corpus must be under `$HOME`
(the script's temp files in `/tmp` are only read and written by the shell,
so they are fine).

## Writing tests that also run on stock PHP

phpx-only features (async API) need a polyfill guarded by
`if (!function_exists('async'))` so PHP produces the reference output;
see `08_async.php`.

## Memory errors (AddressSanitizer)

phpx has unsafe code (compiled-code helpers, raw object construction,
the size-class allocator). A crash in drop glue or a value that changes
by itself is usually memory corruption. Build with ASan on nightly:

```sh
RUSTFLAGS="--cfg phpx_asan -Zsanitizer=address" CARGO_TARGET_DIR=target-asan \
  cargo +nightly build -p phpx-cli --target x86_64-unknown-linux-gnu
ASAN_OPTIONS=detect_leaks=0 target-asan/x86_64-unknown-linux-gnu/debug/phpx script.php
```

`--cfg phpx_asan` makes the allocator hand every block to the system
allocator so ASan sees it. If ASan reports nothing yet the process still
crashes, look for objects built field by field (`Obj::from_template`)
that miss a newly added field.

Expected under ASan: instrumented frames are much larger, so the
parser's stack-depth guard can trip on small inputs ("Maximum call stack
size reached during compilation"); in the differential suite this shows
as `str_pad()`'s default argument being "not known" (its default is
parsed from source). Only memory-error reports matter there.
