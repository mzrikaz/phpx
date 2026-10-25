# Performance

Goal: faster than PHP 8.5 (with OPcache-less CLI as the baseline) on every
workload, while staying byte-identical. Today phpx is clearly faster on
numeric code, arithmetic, built-in calls, assignments and concatenation, and
still slower on user-function calls, method calls, property access,
indexing and iteration.

## Baseline (2026-10-25, fair comparison)

PHP 8.5.10 CLI run with `php -n` (no php.ini: **xdebug and pcov off**; the
lerd default config loads xdebug, which slows PHP 3-5x and made earlier
comparisons look better than they were). "PHP JIT" adds
`-d opcache.enable_cli=1 -d opcache.jit=tracing -d opcache.jit_buffer_size=64M`.

### Microbenchmarks (ns per operation, `microf.php`, lower is better)

| Operation | PHP 8.5 (`-n`) | phpx (2026-10-25 start) | phpx now | Status |
|---|---|---|---|---|
| empty loop | 5.5 | 3.1 | 3.2 | faster |
| `$x = $x + 1` | 6.1 | 1.9 | 2.0 | faster |
| `$a[] = $i` | 22.3 | 25 | 25-30 | ~par (noisy) |
| `foreach` over array | 10.3 | 25 | 7.5 | **faster** (inline packed iteration) |
| `abs()` (numeric JIT inline) | 20.1 | 2.2 | 2.3 | faster |
| `strlen()` / `count()` / `is_*` | 5.1 | 70 | 4-6 | **par** (inline intrinsics) |
| other built-in call (`chr`) | 7.2 | 75 | 30 | slower |
| user function call | 19.1 | 31 | 26 | slower |
| `.=` concat | 19.7 | 12.5 | 14 | faster |
| `$a[$i]` read | 8.2 | 18 | 6.7 | **faster** (inline packed read) |
| `$o->p` read | 6.8 | 21 | 7.1 | **par** (inline cache) |
| method call | 19.4 | 55 | 38 | slower |
| `"item $i"` | 60.3 | 87 | 57 | **par** (single-alloc strings) |
| `new C` (no ctor) | 38.7 | 147 | 114 | slower |

Remaining gaps are call overhead (frames, helper round trips, the
per-built-in panic guard) and object allocation; see the plan below.

### Numeric code (`num.php`, ms)

| Benchmark | PHP `-n` | PHP JIT | phpx |
|---|---|---|---|
| fib(30) | 54.6 | 19.2 | 17.6 |
| loop_sum | 59.2 | 31.5 | 17.3 |
| count_primes | 141.4 | 67.7 | 73.3 |
| collatz | 1107.3 | 382.2 | 162.0 |
| mandelbrot | 338.9 | 30.1 | 40.6 |

### Memory (bytes per item, 500k items)

| Item | PHP 8.5 | phpx |
|---|---|---|
| object with 2 props | 105 | 169 (was 333 before shapes) |
| constant array literal `[1,2]` | 17 | 17 (was 169 before sharing) |
| short string | 57 | 64 (was 68 before single-allocation strings) |

## Plan: beat PHP everywhere, approach Node/Go

Targets: every microbenchmark at or below PHP 8.5 with JIT; numeric and
call-heavy code near Node; memory per value at or below PHP's.

1. **Inline fast paths in generated code** (biggest gap):
   property read/write by (shape, slot) with an inline guard; packed-array
   index load; `foreach` over packed arrays without the iterator helper.
   Needs `#[repr(C)]` object/props layout so generated code can address
   fields directly.
2. **Direct native-to-native calls**: compiled callers call compiled callees
   through the entry pointer with arguments in registers, no `Frame`
   allocation, no helper round trip; method calls via a monomorphic inline
   cache guard in generated code.
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

The microbenchmark scripts (`bench.php`, `num.php`) are not checked into
the repository yet — to do: add them under `tools/bench/` with a runner
script so these numbers can be reproduced.

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
