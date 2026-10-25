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
- [ ] Compact object layout (declared props as slots): objects are 333 bytes
      vs PHP's 105; constant array literals are rebuilt each evaluation
      (169 vs 17 bytes).
- [ ] Per-request arenas in server mode (drop everything a request allocated).

## Security issues

| Issue | Status |
|---|---|
| `random_int()` used a non-cryptographic xorshift generator (predictable tokens) | **fixed**: OS CSPRNG, rejection sampling |
| `random_bytes()` / `openssl_random_pseudo_bytes()` used xorshift | **fixed**: `/dev/urandom` |
| No `memory_limit` | **fixed**: counting allocator, PHP fatal error, pre-checks in `str_repeat`/`str_pad`/`array_fill` (enforced at 3x until compact objects) |
| `unserialize()`/`json_*`/`serialize` deep nesting overflowed the native stack | **fixed**: `max_depth` (4096) + stack checks raise PHP errors |
| `max_execution_time` not enforced | open |
| Regex: fancy-regex backtracking; verify a backtrack limit equivalent to `pcre.backtrack_limit` (ReDoS) | to verify |
| `unserialize()` object injection on untrusted data | same as PHP (supports `allowed_classes`) |
| No `open_basedir` / `disable_functions` | open (PHP default is also unrestricted) |
| 79 `unsafe` blocks (native compiler ABI, raw frame pointers, `with_globals` aliasing the main frame) | needs audit; run test suite under Miri/ASan where possible |
| Native code relies on `Rc` memory layout (checked at startup by `rc_layout_ok()`) | guarded |
| Runtime panics abort the script | **partly fixed**: panics inside built-ins become PHP `Error`s; panics elsewhere still abort |
| No HTTP server yet, so no network-facing attack surface beyond the runtime's demo | n/a until `phpx serve` |

Next steps: fuzz the lexer/parser and `unserialize`/`pack`/`parse_url` with
cargo-fuzz, run the differential suite under AddressSanitizer, add
`memory_limit` accounting.
