# Performance

Goal: faster than PHP 8.5 (OPcache-less CLI as the baseline) on every
workload, while staying byte-identical. Today phpx is faster on numeric
code, arithmetic, loops, indexing, iteration and string building, level on
property access and plain function calls, and still slower on method and
static calls, object creation and closures.

## Baseline (2026-10-25, fair comparison)

PHP 8.5.10 CLI run with `php -n` (no php.ini: **xdebug and pcov off**; the
lerd default config loads xdebug, which slows PHP 3-5x and made earlier
comparisons look better than they were). "PHP JIT" adds
`-d opcache.enable_cli=1 -d opcache.jit=tracing -d opcache.jit_buffer_size=64M`.
Microbenchmarks are pinned to one core (`taskset -c 5`), best of 3 runs;
absolute numbers depend on the machine, the ratio is what matters.

### Microbenchmarks (ns per operation, lower is better)

From `tools/bench/` (`micro.php`, `calls.php`, `objects.php`, `closures.php`; see Methodology).

| Operation | PHP 8.5 (`-n`) | phpx | Status |
|---|---|---|---|
| empty loop | 2.1 | 1.6 | faster |
| `$x = $x + 1` | 2.9 | 0.9 | faster |
| `$a[] = $i` | 12.1 | 13.8 | ~par |
| `foreach` over array | 4.6 | 3.7 | faster |
| `abs()` (numeric JIT inline) | 7.1 | 1.1 | faster |
| `strlen()` / `count()` / `is_*` | ~2 | ~2 | par (inline intrinsics) |
| `.=` concat | 9.4 | 7.1 | faster |
| `$a[$i]` read | 3.9 | 3.1 | faster |
| `$o->p` read | 3.5 | 3.5 | par (inline cache) |
| `"item $i"` | 29.9 | 28.2 | par |
| `f()` (empty user function) | 4.1 | 5.4 | slower (direct native call) |
| `$x = one("s")` | 7.4 | 7.5 | par |
| `$o->m()` (empty method) | 6.8 | 12.4 | slower (direct native call) |
| `A::s()` (static method) | 5.3 | 13.2 | slower (inline cache, helper call) |
| `new C` (no constructor) | 20.1 | 36.6 | slower |
| `new C($i)` (constructor) | 35.0 | 61.2 | slower |
| `new C(int $a, string $b)` promoted | 57.2 | 83.4 | slower |
| `$f($x)` closure call | 16.4 | 30.5 | slower |
| `array_map(fn ..., $a)` per element | 19.0 | 72.5 | slower |
| create closure + call | 85.2 | 195.7 | slower |

Progress on the slow paths this round (phpx before -> after): function
call 31 -> 5.4 ns (calls with no or numeric arguments were 130-150 ns:
the numeric JIT never gave up on them), method call 55 -> 12.4, static
call 524 -> 13.2, `new` with constructor 270 -> 61, promoted typed
constructor 420 -> 83, closure call 150 -> 30.5, `array_map` callback 190
-> 72.

Remaining gaps: static calls and constructors still go through a Rust
helper per call (method and function calls don't); objects take two
allocations plus cycle-collector registration; closure creation allocates
several blocks (PHP allocates one).

### Numeric code (`num.php`, ms)

| Benchmark | PHP `-n` | PHP JIT | phpx |
|---|---|---|---|
| fib(30) | 54.6 | 19.2 | 17.6 |
| loop_sum | 59.2 | 31.5 | 17.3 |
| count_primes | 141.4 | 67.7 | 73.3 |
| collatz | 1107.3 | 382.2 | 162.0 |
| mandelbrot | 338.9 | 30.1 | 40.6 |

### Memory (bytes per item, 100k items, `memory_get_usage()` deltas)

phpx counts the size class each block really occupies, like PHP's
allocator does.

| Item | PHP 8.5 | phpx |
|---|---|---|
| object with 1 property | 87 | 112 (was 160; 333 before shapes) |
| object with 3 properties, set in the constructor | 205 | 245 |
| `['id' => $i, 'name' => 'n', 'x' => 1]` | 397 | 421 |
| `[$i, $i]` | 237 | 197 |
| 10-byte string | 69 | 69 |
| int in a list | 21 | 21 |
| sparse int keys | 52 | 58 |
| string keys | 84 | 90 |
| constant array literal `[1,2]` | 17 | 17 |

Resident memory for a mixed script (1M small objects, 300k objects with
strings, 300k nested arrays, 500k strings): PHP 322 MB, phpx 357 MB (was
493 MB). Objects are 48 bytes plus their property slots: properties are a
two-word vector, the destructed flag shares the GC index word, and native
state (closures, generators) lives in a side table; small blocks come from
size-class free lists without per-block headers.

## Plan: beat PHP everywhere, approach Node/Go

Targets: every microbenchmark at or below PHP 8.5 with JIT; numeric and
call-heavy code near Node; memory per value at or below PHP's.

1. **Inline fast paths in generated code** (biggest gap):
   property read/write by (shape, slot) with an inline guard; packed-array
   index load; `foreach` over packed arrays without the iterator helper.
   Needs `#[repr(C)]` object/props layout so generated code can address
   fields directly.
2. **Direct native-to-native calls** (done for functions and instance
   methods): compiled callers call the cached entry with the site's
   reusable frame, no helper round trip. Still to do: static calls,
   constructors and closures through the same path; arguments in
   registers.
3. **Objects**: one allocation per object (values inline after the header),
   no per-object `RefCell`, cheaper registration with the cycle collector;
   `new` target ~40 ns.
4. **Strings**: interpolation builds the result in one buffer of the right
   size; small-string optimisation.
5. **Type feedback**: specialise native code on observed int/float/string
   types (the numeric JIT already does this for pure-number functions).
6. **Throughput beyond single-thread speed**: `phpx serve` runs requests on
   green threads (no FPM process per request), and later an M:N scheduler
   across cores. This is where Node/Go-style throughput comes from.

## Methodology

- Compare against the same PHP 8.5 build used for differential tests
  (`crates/interp/tests/php/PHP_VERSION`), run with `php -n` (no ini, no
  OPcache/JIT) unless stated otherwise.
- Build phpx with `cargo build --release -p phpx-cli`.
- Microbenchmarks time a tight loop of N iterations of one operation in
  both engines, subtract the empty-loop time where noted, and divide by N.
  Use `hrtime(true)` for timing; run each benchmark several times and take
  the minimum (the machine is noisy).
- Always confirm the benchmark's output is identical in both engines
  before trusting its timing.
- Check which engine ran the hot code:
  `PHPX_NATIVE_LOG=1` and `PHPX_JIT_LOG=1` show whether the function
  compiled; compare with `PHPX_NATIVE=off` / `PHPX_NOJIT=1` to see each
  tier's contribution.

The microbenchmarks live in `tools/bench/` (`micro.php`, `calls.php`,
`objects.php`, `closures.php`, `memory.php`, `num.php`, `rss.php`).
`tools/bench/run.sh [runs] [php]` builds nothing; it runs the release
binary and PHP one after the other, pinned to `BENCH_CPU`, and prints the
best of `runs` side by side.

## Profiling

The CLI has an optional in-process sampling profiler (pprof, ~1 kHz):

```sh
cargo build --release -p phpx-cli --features profile
PHPX_PROFILE=1 ./target/release/phpx run bench.php
```

At exit it prints the 25 hottest Rust functions by self time and by
inclusive time to stderr. Native-compiled PHP code appears as anonymous
JIT frames; the `rt_*` helpers it calls show up by name, which is usually
what you want to optimise.

`perf` also works on the release binary (`perf record -g ./target/release/phpx run x.php`),
with the same caveat about anonymous JIT frames.
