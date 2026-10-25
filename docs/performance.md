# Performance

Goal: faster than PHP 8.5 (with OPcache-less CLI as the baseline) on every
workload, while staying byte-identical. Today phpx is clearly faster on
numeric code, arithmetic, built-in calls, assignments and concatenation, and
still slower on user-function calls, method calls, property access,
indexing and iteration.

## Microbenchmarks

Nanoseconds per operation, lower is better. "phpx" is the default tiering
(native compiler + numeric JIT). The machine is noisy, so phpx figures are
given as observed ranges; differences under ~20% should not be read as
meaningful.

| Operation | PHP 8.5 (ns/op) | phpx (ns/op) | Verdict |
|---|---:|---:|---|
| Empty loop iteration | 3.7 | 3.2 – 3.8 | parity |
| Local assignment | 5.0 | ~2 | **phpx faster** |
| `$a[] = $x` (push) | 19 | 20 – 30 | slightly slower |
| `foreach` iteration | 8 | 18 – 24 | **slower** (~2.5x) |
| Built-in function call | 13 | ~2 | **phpx faster** |
| User function call | 13 | ~30 | **slower** (~2.3x) |
| String concatenation | 17 | ~12 | **phpx faster** |
| Array index read | 7 | ~17 | **slower** (~2.4x) |
| Property read | 6 | ~19 | **slower** (~3x) |
| Method call | 17 | ~55 | **slower** (~3x) |
| String interpolation | 52 | ~85 | **slower** (~1.6x) |

Note: the "user fn call" row is the general (non-numeric) case. Calls to
functions taken by the numeric JIT are much faster than PHP (see `fib`
below).

## Program benchmarks (`bench.php`)

| Section | PHP 8.5 | phpx |
|---|---:|---:|
| `fib` (recursive calls, numeric) | 10 ms | ~5 ms |
| array work | 8 ms | ~17 ms |
| string work | 5 ms | ~12 ms |

## Numeric code (`num.php` and earlier JIT benchmarks)

Numeric code that the JIT takes is 2–8x faster than PHP. Earlier
measurements of individual numeric functions:

| Benchmark | PHP 8.5 | phpx (JIT) |
|---|---:|---:|
| `fib(30)` | 28 ms | 8 ms |
| `loop_sum` 5M | 30 ms | 6 ms |
| `count_primes` 300k | 71 ms | 33 ms |
| `collatz` 300k | 550 ms | 82 ms |
| `mandelbrot` 400×400 | 169 ms | 19 ms |

## Where the time goes, and what's next

| Slow path | Cause | Planned fix |
|---|---|---|
| User function calls | Each call goes through `rt_call`: argument array, fresh `Frame`, return-type check, status handling | Direct native-to-native calls with a register-based fast ABI when caller and callee are both compiled |
| Method calls | `rt_method_call` + monomorphic cache check + frame setup | Same as above, plus inlined cache guard in generated code |
| Property reads | Helper call per read even on cache hit (`rt_prop_read`) | Inline the `PropSite` guard and load the slot directly |
| Array index | Helper call per read (`rt_index_get`) | Inline packed-array fast path (int key, in bounds) |
| `foreach` | Boxed iterator, helper call per element (`rt_iter_next`) | Inline packed-array iteration |
| String interpolation | Generic stringification of parts | Specialise per part type |
| Code that falls back to the interpreter | References, static vars, globals, `@`, generators (see [native-compiler.md](native-compiler.md#what-compiles)) | Extend compiler coverage (roadmap item 4) |

Target: beat PHP on every row of the microbenchmark table (roadmap item 5).

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
