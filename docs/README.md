# phpx documentation

Last updated: 2026-10-25.

phpx runs PHP code unchanged on a Rust engine: a tree-walking interpreter that
defines the semantics, a Cranelift native compiler for whole functions, a
type-specialised numeric JIT, and a green-thread runtime for concurrency.
Output is required to be byte-identical to PHP 8.5.

## Start here

| If you want to… | Read |
|---|---|
| Understand how the engine is put together | [architecture.md](architecture.md) |
| Know whether a PHP feature works (and whether it compiles natively) | [language-support.md](language-support.md) |
| Know where phpx behaves differently from PHP 8.5 | [php-compatibility.md](php-compatibility.md) |
| Check whether a built-in function or class exists | [stdlib.md](stdlib.md) |
| Work on the native compiler | [native-compiler.md](native-compiler.md) |
| Measure or improve speed | [performance.md](performance.md) |
| Use concurrency (`async`, `Promise`, `Concurrency::run`, `defer`) | [async.md](async.md) |
| Add a test or verify a change against real PHP | [testing.md](testing.md) |
| See how close Laravel / WordPress / Drupal / Joomla / WooCommerce are | [cms-compatibility.md](cms-compatibility.md) |
| See what is planned and track progress | [roadmap.md](roadmap.md) |
| See what has been done | [changelog.md](changelog.md) |

## Principles

1. **PHP is the spec.** Files stay `.php`; anything phpx accepts is valid PHP.
   When phpx and PHP 8.5 disagree, phpx is wrong.
2. **Compatibility over strictness.** Laravel must run unmodified, so
   references, `goto`, generators, `global`, `$GLOBALS`, variable variables,
   `extract()` and `eval()` are all supported. Only backtick shell execution
   (and the long-removed `create_function`) are rejected.
3. **Speed comes from the compiler, not from restricting the language.**
   Code the native compiler can't handle runs in the interpreter with the
   same output — never an error.
4. **Concurrency is opt-in and looks blocking.** Code runs top to bottom;
   waits park a green thread, never the OS thread.

## Conventions in these docs

- "Interpreter" = `crates/interp` tree-walker (always available, the
  semantic reference).
- "Native compiler" = `crates/interp/src/compiler` (whole-function Cranelift
  compilation of general PHP code).
- "Numeric JIT" = `crates/jit` (type-specialised int/float/bool functions).
- "to verify" marks statements that have not been confirmed against code or
  a test.
