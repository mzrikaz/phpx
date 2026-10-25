# Memory safety, leaks and security: known issues

Status as of 2026-10-25. Measured with `/usr/bin/time` max RSS over 200,000
loop iterations (script: objects created and dropped each iteration),
compared with PHP 8.5 `memory_get_peak_usage(true)`.

## Memory leaks (confirmed)

| Pattern | phpx max RSS | PHP 8.5 peak | Cause |
|---|---|---|---|
| Plain objects (no cycles) | 6.9 MB | 2 MB | none: freed by refcount |
| Two objects referencing each other | 169 MB → **12.7 MB (fixed)** | 4 MB | cycle collector added |
| Object referencing itself | 104 MB → **13 MB (fixed)** | — | cycle collector |
| Closure capturing `$this`/object stored on that object | 175 MB → **13 MB (fixed)** | — | cycle collector |
| `eval()` in a loop | 362 MB → **7 MB (fixed)** | — | parsed code cached per call site + source |
| `$$name` with ever-new names | 17 MB | — | interned variable names are never freed |

Impact: short CLI scripts are fine; long-running workers (queues, the future
`phpx serve`) would grow without bound on Laravel/WordPress object graphs,
which contain many cycles (container ↔ app, models ↔ relations, closures
bound to `$this`).

Fix plan:
- [x] Cycle collector (`gc.rs`): registry of live objects, trial deletion,
      destructors run before cycles are broken; `gc_collect_cycles()`,
      `gc_enable/disable`, `gc_status()`. Shared containers are treated
      conservatively (never frees reachable data, may miss some garbage).
      `gc_collect_cycles()` counts objects only (PHP also counts arrays).
- [x] `eval()` parsed code cached per call site + source.
- [x] Compact object layout: an object with one property is 112 bytes
      (PHP 87; was 333); constant array literals are shared (17 bytes, like
      PHP).
- [ ] Per-request arenas in server mode (drop everything a request allocated).

## Security issues

| Issue | Status |
|---|---|
| `random_int()` used a non-cryptographic xorshift generator (predictable tokens) | **fixed**: OS CSPRNG, rejection sampling |
| `random_bytes()` / `openssl_random_pseudo_bytes()` used xorshift | **fixed**: `/dev/urandom` |
| No `memory_limit` | **fixed**: counting allocator, PHP fatal error, pre-checks in `str_repeat`/`str_pad`/`array_fill`/`number_format`; `var_dump`/`print_r`/`var_export` stop at the limit instead of building output quadratic in nesting depth |
| `unserialize()`/`json_*`/`serialize` deep nesting overflowed the native stack | **fixed**: `max_depth` (4096) + stack checks raise PHP errors |
| Deeply nested source (also via `eval()`) overflowed the stack in the parser, checks, compiler or interpreter and aborted the process | **fixed**: stack budget; PHP's "Maximum call stack size ... reached during compilation" error (a catchable `ParseError` from `eval`) |
| Self-referencing structures (`$a[] = &$a`) segfaulted `count(COUNT_RECURSIVE)`, dumps, `==`, `sort`, `array_walk_recursive`, `array_replace_recursive`, `json_encode` | **fixed**: recursion detection with PHP's warnings/errors |
| Lexer panic on a trailing backslash in an unterminated string; `unpack()`, `intval()`, `base_convert()`, `number_format()` panics on hostile arguments | **fixed** (found by fuzzing) |
| `max_execution_time` not enforced | **fixed** (CPU-time watcher; compiled loops are interruptible) |
| Regex: fancy-regex backtracking; verify a backtrack limit equivalent to `pcre.backtrack_limit` (ReDoS) | to verify |
| `unserialize()` object injection on untrusted data | same as PHP (supports `allowed_classes`) |
| No `open_basedir` / `disable_functions` | open (PHP default is also unrestricted) |
| `unsafe` code (native compiler ABI, allocator, object layout, GC registry) | audit in progress |
| Native code relies on memory layouts (checked at startup by `probe_layout()` / `rc_layout_ok()`) | guarded |
| Runtime panics abort the script | **partly fixed**: panics inside built-ins become PHP `Error`s; panics elsewhere still abort |
| No HTTP server yet, so no network-facing attack surface beyond the runtime's demo | n/a until `phpx serve` |

## Fuzzing

`cargo test --release -p phpx-interp --test fuzz -- --ignored` runs three
randomized suites (seed: `PHPX_FUZZ_SEED`, amount: `PHPX_FUZZ_ITERS`):

- `parser`: mutated test sources (bit flips, token splices, deep nesting,
  truncation) through the lexer, parser and checks;
- `builtins`: random input for `unserialize`, `json_decode`, `unpack`,
  `pack`, `sprintf`, regexes, `str_getcsv`, `parse_url`;
- `all_builtins`: `tools/fuzz/all_builtins.php` calls every built-in
  without side effects with hostile arguments.

As of 2026-10-25: 180,000 parser mutations and ~700,000 built-in calls
without a panic after the fixes above.
