# Memory safety, leaks and security: known issues

Status as of 2026-10-25. Measured with `/usr/bin/time` max RSS over 200,000
loop iterations (script: objects created and dropped each iteration),
compared with PHP 8.5 `memory_get_peak_usage(true)`.

## Memory leaks (confirmed)

| Pattern | phpx max RSS | PHP 8.5 peak | Cause |
|---|---|---|---|
| Plain objects (no cycles) | 6.9 MB | 2 MB | none: freed by refcount |
| Two objects referencing each other | 169 MB | 4 MB | **no cycle collector**: `Rc` cycles are never freed |
| Object referencing itself | 104 MB | — | same |
| Closure capturing `$this`/object stored on that object | 175 MB | — | same |
| `eval()` in a loop | 362 MB | — | each eval's source and AST are leaked (`Box::leak`) |
| `$$name` with ever-new names | 17 MB | — | interned variable names are never freed |

Impact: short CLI scripts are fine; long-running workers (queues, the future
`phpx serve`) would grow without bound on Laravel/WordPress object graphs,
which contain many cycles (container ↔ app, models ↔ relations, closures
bound to `$this`).

Fix plan:
- [ ] Cycle collector for objects/arrays/closures (trial deletion like PHP's
      `gc_collect_cycles`, triggered by root-buffer threshold), plus real
      `gc_collect_cycles()` / `gc_status()`.
- [ ] Cache `eval()` by source text; free unreferenced eval ASTs.
- [ ] Per-request arenas in server mode (drop everything a request allocated).

## Security issues

| Issue | Status |
|---|---|
| `random_int()` used a non-cryptographic xorshift generator (predictable tokens) | **fixed**: OS CSPRNG, rejection sampling |
| `random_bytes()` / `openssl_random_pseudo_bytes()` used xorshift | **fixed**: `/dev/urandom` |
| No `memory_limit`: a script (or input-driven allocation such as `str_repeat`, `pack("x999999999")`, huge `unserialize` lengths) can exhaust host memory | open |
| `unserialize()` recursion is unbounded: deeply nested input overflows the native stack (crash / DoS) | open: add depth limit |
| `max_execution_time` not enforced | open |
| Regex: fancy-regex backtracking; verify a backtrack limit equivalent to `pcre.backtrack_limit` (ReDoS) | to verify |
| `unserialize()` object injection on untrusted data | same as PHP (supports `allowed_classes`) |
| No `open_basedir` / `disable_functions` | open (PHP default is also unrestricted) |
| 79 `unsafe` blocks (native compiler ABI, raw frame pointers, `with_globals` aliasing the main frame) | needs audit; run test suite under Miri/ASan where possible |
| Native code relies on `Rc` memory layout (checked at startup by `rc_layout_ok()`) | guarded |
| Runtime panics abort the script with `phpx: runtime error` instead of a PHP error | open: map to `Error` exceptions |
| No HTTP server yet, so no network-facing attack surface beyond the runtime's demo | n/a until `phpx serve` |

Next steps: fuzz the lexer/parser and `unserialize`/`pack`/`parse_url` with
cargo-fuzz, run the differential suite under AddressSanitizer, add
`memory_limit` accounting.
