# Changelog

What has been built so far, grouped by milestone (most recent first).
Entries before the first commit (2026-10-25) are reconstructed from the code
and project notes; later entries follow the git history.

## 2026-10-25 — Compatibility and hardening

- **Built-in signatures** from PHP's Reflection (`tools/arginfo`): PHP's
  `ArgumentCountError`/`TypeError` for built-ins, coercive-mode juggling,
  `strict_types`, invalid-callback errors, named arguments.
- **SPL** written in PHP: `ArrayObject`, `ArrayIterator`,
  `SplDoublyLinkedList`/`SplStack`/`SplQueue`, heaps, `SplPriorityQueue`,
  `SplFixedArray`, `SplObjectStorage`, and the standard iterators.
- `__debugInfo()` in `var_dump`/`print_r`; `get_defined_functions()`;
  `ArrayAccess` offsets of any type; `round()` uses PHP 8.4's algorithm;
  `json_encode` honours `$depth`; `var_export` of `stdClass` and nested
  properties; private methods bind to the calling class.
- **Native-code fixes**: by-reference arguments that are properties or
  array elements (`sort($this->items)`) were silently copied; a temp
  double free in the `count()`/`strlen()` intrinsic (seen when a
  namespaced function shadows a built-in, as Laravel does).
- **Hardening**: recursion detection for self-referencing values,
  stack budget for deeply nested source, dumps stop at `memory_limit`,
  fuzz harness and the crashes it found (lexer, `unpack`, `intval`,
  `base_convert`, `number_format`).

## 2026-10-25 — Calls, objects and memory

- **Direct calls**: compiled code calls cached compiled functions and
  instance methods through their entry with a reusable frame (no helper
  round trip); compiled functions check their own return types.
- **Inline caches** for static calls (`A::m()`, `self::`, `parent::`,
  `static::`), `new` with constructors (promoted parameters bind
  natively), and numeric-JIT entries at call sites.
- **Closures** run natively when called directly or from built-ins
  (`array_map`, `usort`, ...); creating one allocates less.
- Fixed: the numeric JIT never gave up on functions whose specialisation
  failed, so every call to them took the slow path (~130 ns).
- **Objects**: 48-byte header (was 96); native state in a side table;
  written in place. **Allocator**: per-thread size classes for blocks up
  to 256 bytes (no per-block header); `memory_get_usage()` counts real
  block sizes. An object with one property: 160 -> 112 bytes.
- **PHP compatibility**: closures are named `{closure:<where>:<line>}`
  (PHP 8.4+); argument TypeErrors add ", called in <file> on line <n>"
  and point at the parameter; return-type TypeErrors point at the
  `return`; `$this->m()` binds a private `m()` to the calling class; a
  temporary object (`(new C)->m()`) is destroyed right after the call.
- Benchmark scripts and a side-by-side runner in `tools/bench/`.

## 2026-10-25 — WordPress boot work (in progress)

- **Streams** (`streams.rs`, new): `fopen`/`fclose`/`fread`/`fwrite`/`fputs`/
  `fgets`/`fgetc`/`feof`/`fseek`/`ftell`/`rewind`/`ftruncate`/`fflush`/
  `flock`/`fstat`/`fpassthru`/`stream_get_contents`/`tmpfile`, `php://`
  streams, directory handles (`opendir`/`readdir`/`rewinddir`/`closedir`),
  `stat`/`lstat` and `file*` metadata functions, `chmod`/`chown`/`chgrp`,
  `link`/`symlink`/`readlink`, `umask`, `fnmatch`, disk space, stream
  contexts and meta data. Resources (`Value::Resource`) with PHP-exact
  `var_dump` / `gettype` output, including closed resources.
- **ext.rs** (new): `serialize`/`unserialize` (`__serialize`, `__sleep`,
  `__wakeup`, enums, `allowed_classes`), `pack`/`unpack`, `hash`/`hash_file`/
  `hash_hmac`/`hash_init`/`hash_update`/`hash_final`/`hash_copy`
  (md5, sha1, sha224/256/384/512, crc32b), `sha1`, `md5_file`, `hash_equals`,
  `parse_url`, `parse_str`, `version_compare`, `strspn`, `strcspn`,
  `strpbrk`, `substr_compare`, `stristr`, `strtok`, `addcslashes`,
  `stripcslashes`, `chunk_split`, `utf8_encode`/`decode`, IP helpers.
- **misc.rs** (new): `header`, `header_remove`, `headers_list`,
  `headers_sent` (tracks where output started), `http_response_code`,
  `setcookie`, `register_shutdown_function`, `assert`, `class_alias`,
  `func_get_arg`, `next`/`prev`/`reset`/`end`/`current`/`key` with a real
  internal pointer, `html_entity_decode`/`htmlentities`, `filter_var`,
  bcrypt `password_hash`/`password_verify`/`crypt`, `gz*`/`zlib_*`,
  `str_getcsv`/`fputcsv`/`fgetcsv`, `sscanf`, `levenshtein`,
  `similar_text`, `soundex`, `get_class_methods`, `get_class_vars`,
  `class_parents`, `class_uses`, `vprintf`/`fprintf`, include path.
- Built-ins can take several by-reference arguments (`sscanf`,
  `headers_sent`).
- Fixed: `"\xE9"`-style escapes now produce raw bytes (was UTF-8 encoded);
  `random_bytes` uses the OS CSPRNG; `PHP_VERSION` reports 8.5.10;
  `extension_loaded` reports implemented extensions.
- WordPress `wp-admin/setup-config.php` probe now passes PHP version and
  extension checks; next blocker is the mysqli extension.
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
