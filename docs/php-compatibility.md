# Compatibility with PHP 8.5

The target is **byte-identical output to PHP 8.5** (the reference build used
for tests is recorded in `crates/interp/tests/php/PHP_VERSION`; currently
`PHP 8.5.10 (cli)`). Any difference is a bug unless it is listed under
[Intentional differences](#intentional-differences).

## Intentional differences

| Difference | Why |
|---|---|
| Backtick shell execution is rejected (E1006) | Hidden process spawning; use an explicit function call instead |
| `create_function()` is rejected (E1007) | Removed in PHP 8; the diagnostic suggests a closure |
| `async()`, `defer()`, `Promise`, `Concurrency`, `TimeoutException`, `CancelledException`, `AggregateException` exist as built-ins | phpx concurrency API ([async.md](async.md)). Code that defines its own `async()`/`defer()` functions or `Promise` class will collide — to verify how common this is in the target CMSs |
| `sleep()` / `usleep()` park the green thread instead of blocking the process | Same observable behaviour for a single request; other tasks keep running |
| `require 'vendor/autoload.php'` is handled natively | Composer's own bootstrap is not executed; class loading behaviour is the same |

## Diagnostic codes

`phpx check` and `phpx run` share the parser. Codes are stable.
`E0xxx` = syntax error (PHP would also refuse the file). `E1xxx` = valid PHP
that phpx does not support (counted as "use unsupported features" by
`phpx check`).

| Code | Kind | Message (abridged) | Fix |
|---|---|---|---|
| E0001 | syntax | Lexer error: unterminated comment / string / backtick string / heredoc, unexpected character | Fix the source |
| E0002 | syntax | `expected X, found Y`; `cannot assign to this expression`; `key element cannot be a reference`; `the (unset) cast was removed in PHP 8`; `call-time pass-by-reference was removed in PHP 5.4`; `` `&` is only valid in parameters, foreach, `=&` assignments and array items``; unclosed `{` / `${` in string | Fix the source |
| E1006 | unsupported | backtick shell execution is not supported | Call a process function explicitly (note: `shell_exec()` itself is not implemented yet) |
| E1007 | unsupported | `create_function()` was removed in PHP 8 | Use a closure |
| E1008 | unsupported | property hooks are not supported yet | Use getter/setter methods until hooks land |

Earlier versions of phpx also rejected `eval`, `$$var` / `${expr}`,
`extract()`, `global` and `$GLOBALS` (on the theory that a compiler cannot
resolve variables by runtime name). All of these are now supported — the
native compiler simply leaves such functions to the interpreter — and no
codes are emitted for them any more. Codes below E1006 in the `E1xxx`
range are therefore unused.

### `phpx check`

```sh
phpx check [--summary] <file-or-dir>...
```

Recursively collects `*.php` files, parses each one (non-UTF-8 files are
read lossily, spans stay correct) and prints rendered diagnostics (unless
`--summary`), then a summary line:

```
1512 files (… MB) in 0.xx s: 1512 ok, 0 use unsupported features, 0 with syntax errors
diagnostics: E1006×n …
```

The exit status is non-zero if any file has an error.

## Semantic gaps

Known places where phpx output can differ from PHP 8.5 today.

| Gap | Detail | Plan |
|---|---|---|
| Destructor timing | `__destruct` runs at the next statement boundary after the last reference dies, not in the middle of an expression. Output order differs only if a destructor prints during an expression that also prints | Acceptable for now; revisit if real code depends on it |
| Warning/notice destination | phpx writes warnings, notices and deprecations to **stderr**; PHP CLI prints them to **stdout** (with default `display_errors`). Tests run PHP with `display_errors=stderr` so stdout compares cleanly | HTTP mode and `display_errors` ini handling |
| `${var}` deprecation | PHP 8.2+ prints `Deprecated: Using ${var} in strings is deprecated`; phpx does not | Emit it |
| `PHP_VERSION` constant | phpx reports `8.4.0` / `PHP_VERSION_ID` `80400` (set in `builtins.rs`) while the reference is 8.5.x | Bump to 8.5 so version checks in CMSs take 8.5 code paths |
| Pipe operator `\|>` | PHP 8.5 syntax not parsed yet | Implement in parser + interpreter |
| Resource IDs | `var_dump($fp)` shows `resource(4)` where PHP shows `resource(5)` in the same script (PHP allocates STDIN/STDOUT/STDERR and other resources first) | Align the initial resource counter |
| Non-PHP functions visible | `function_exists('log2')`, `function_exists('lcg')` and `__phpx_stack_remaining` return true; PHP has no such functions | Remove or rename internal helpers |
| Non-UTF-8 source files | Source is decoded with `from_utf8_lossy`, so Latin-1 bytes inside string literals become U+FFFD | Keep sources as bytes |
| Stack traces | `getTrace()` returns `[]` and `getTraceAsString()` returns `#0 {main}` | Real backtraces (`debug_backtrace` is also missing) |
| Missing built-ins | Calling an unimplemented function throws `Error: Call to undefined function` | [stdlib.md](stdlib.md) lists what is missing |
| Missing classes | No `Reflection*`, `DateTime*`, `ArrayObject`, `SplStack`/SPL data structures, `WeakMap`, DOM/XML classes, `PDO`, `mysqli` | Roadmap items 2 and 6 |
| Superglobals in CLI | `$_GET`, `$_POST`, `$_COOKIE`, `$_FILES`, `$_REQUEST` are empty arrays; `$_SESSION` is not set (like PHP CLI without a session) | HTTP server mode |
| `ini_get` / `ini_set` | Minimal; most directives are not modelled | To verify per directive as CMSs need them |

## How compatibility is verified

- Differential tests: `crates/interp/tests/php/*.php` with expected output
  generated by real PHP ([testing.md](testing.md)). They pass in all three
  engine modes (default tiering, `PHPX_NATIVE=all`, and
  `PHPX_NATIVE=off PHPX_NOJIT=1`).
- Ad-hoc comparison of any file: `tools/difftest/compare.sh file.php`.
- Parser cross-check against `nikic/php-parser`: `tools/oracle/run.sh`.
- Real-world corpora: `phpx check` over WordPress and Drupal core; Laravel
  probe scripts run and diffed against PHP
  ([cms-compatibility.md](cms-compatibility.md)).
