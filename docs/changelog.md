# Changelog

What has been built so far, grouped by milestone (most recent first).
The repository has no commit history yet, so this log is reconstructed from
the code and project notes; entries are not individually dated.

## 2026-10-25 — WordPress boot work (in progress)

- **Streams** (`streams.rs`, new): `fopen`/`fclose`/`fread`/`fwrite`/`fputs`/
  `fgets`/`fgetc`/`feof`/`fseek`/`ftell`/`rewind`/`ftruncate`/`fflush`/
  `flock`/`fstat`/`fpassthru`/`stream_get_contents`/`tmpfile`, `php://`
  streams, directory handles (`opendir`/`readdir`/`rewinddir`/`closedir`),
  `stat`/`lstat` and `file*` metadata functions, `chmod`/`chown`/`chgrp`,
  `link`/`symlink`/`readlink`, `umask`, `fnmatch`, disk space, stream
  contexts and meta data. Resources (`Value::Resource`) with PHP-exact
  `var_dump` / `gettype` output, including closed resources.
- Started the batch of ~150 missing built-ins (headers/cookies,
  serialization, `pack`, URL parsing, `version_compare`, string scanners,
  HTML entities, hashing, date/time, array gaps) — see
  [roadmap.md](roadmap.md#1-stdlib-for-wordpress-boot-in-progress).
- Documentation set (`docs/`) written.

## Dynamic PHP support — WordPress and Drupal parse 100%

- `global` statement, `$GLOBALS` (read returns a copy; write/unset through
  `$GLOBALS['x']`).
- Variable variables `$$x`, `${expr}`, `"${name}"` in strings.
- `extract()` with `EXTR_OVERWRITE`, `EXTR_SKIP`, `EXTR_PREFIX_SAME`,
  `EXTR_PREFIX_ALL`.
- `eval()` (throws `ParseError` on invalid code).
- `@` suppresses warnings.
- Superglobals `$_SERVER` (environment + `argv`, `argc`, `PHP_SELF`,
  `SCRIPT_NAME`, `REQUEST_TIME`, `REQUEST_TIME_FLOAT`, …), `$_GET`,
  `$_POST`, `$_COOKIE`, `$_FILES`, `$_REQUEST`, `$_ENV`; `$argv`, `$argc`.
- Removed the `E1xxx` diagnostics that used to reject these features; only
  backticks (E1006), `create_function` (E1007) and property hooks (E1008)
  remain.
- Result: WordPress went from 251 rejected files (global ×1,147,
  `$GLOBALS` ×490) to 1,512/1,512 parse-OK; Drupal 11.2.5 core
  10,656/10,656 parse-OK.
- Native compiler: top-level code binds its locals directly to the
  frame's variable cells, so `global` / `$GLOBALS` in called functions see
  live values.
- Tests `13_globals_dynamic.php`, `16_dynamic_calls.php`.

## Destructors

- `__destruct` runs when the last reference dies (queued, drained at the
  next statement boundary) and at shutdown. Compiled code checks the queue
  per statement.
- Test `15_destructors.php`.

## Native compiler (Cranelift) for general code

- Whole-function compilation of arbitrary PHP (not only numeric): values in
  16-byte stack slots, inline int fast paths, inline refcounting, helpers
  for everything else (`rt.rs`).
- Tiering: compile on 2nd call (1st with a loop); `PHPX_NATIVE=all|off`,
  `PHPX_NATIVE_LOG=1`.
- Inline caches: `CallSite` (built-ins and compiled user functions),
  `MethodSite` (monomorphic), `PropSite`.
- Coverage: all loops, `foreach` by value, `switch`, `match`,
  `try`/`catch`/`finally` (finally inlined on return/break/continue), closures
  and arrow functions, calls incl. dynamic `$f()`, spread and named
  arguments, `$obj->$name` / `$obj->$name()`.
- Top-level file code compiles too.
- Transparent fallback to the interpreter for anything unsupported.
- Tests `14_try_native.php`, `16_dynamic_calls.php`; whole suite passes in
  default, `PHPX_NATIVE=all` and `PHPX_NATIVE=off PHPX_NOJIT=1` modes.

## Interpreter: full language

- References everywhere (locals, arrays, properties, `foreach` by ref,
  `use (&$x)`, by-ref params), `goto`, generators (as green-thread tasks),
  static variables, first-class callables, named arguments, spread, enums,
  readonly, traits, magic methods, `Closure::bind`.
- Value representation: 16-byte `#[repr(C, u8)]` `Value`, hybrid packed/hash
  `PhpArray`, objects with ordered property maps.
- Stdlib: strings, arrays, math, JSON, PCRE (fancy-regex), ctype, partial
  mbstring/iconv, output buffering, class introspection, filesystem.
- Native Composer autoloader.
- Tests `01`–`12`.

## Numeric JIT

- Cranelift JIT for int/float/bool functions, type-specialised per
  argument types, deopt by re-execution on overflow / division by zero /
  deep recursion. 2–8x faster than PHP on numeric code (`fib(30)` 28 ms →
  8 ms, `mandelbrot` 169 ms → 19 ms).
- `PHPX_NOJIT`, `PHPX_JIT_LOG`.

## Parser

- Hand-written lexer and Pratt parser for PHP 8.x, rustc-style diagnostics
  with stable codes, `phpx check` (with `--summary`) and `phpx ast`.
- Parser oracle against nikic/php-parser (`tools/oracle`).

## Runtime

- Single-threaded green-thread scheduler (corosensei), timers, epoll reactor
  (polling), non-blocking TCP, `Promise` with `then/catch/finally/timeout/
  cancel/all/allSettled/any/race`, `Concurrency::run`, `defer`.
- Demo HTTP server and concurrency profile examples.
