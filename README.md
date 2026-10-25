# phpx

phpx is PHP — same syntax, same semantics, same `.php` files — executed by a
new engine written in Rust. The goal is that existing PHP projects (Laravel
first; WordPress, Drupal, Joomla, WooCommerce and their plugins next) run
**unchanged**, produce **byte-identical output** to PHP 8.5, run **faster**
than PHP 8.5, and get **built-in concurrency** (green threads, Laravel-style
`async()` / `Concurrency::run` / `defer`) without changing how code reads.

Compatibility is checked continuously by differential tests: every test
script is run by real PHP 8.5 and by phpx, and the outputs must match byte
for byte.

> Status (2026-10-25): the language core is complete enough that WordPress
> (1,512 files) and Drupal 11.2.5 core (10,656 files) parse 100% clean, and
> Laravel probe scripts run byte-identical to PHP. Running WordPress
> end-to-end is in progress (blocked on stdlib breadth, MySQL drivers and an
> HTTP server mode). See [docs/roadmap.md](docs/roadmap.md).

## Quick start

```sh
cargo build --release -p phpx-cli

./target/release/phpx run hello.php [args...]     # run a script ($argv works)
./target/release/phpx check --summary path/to/app # parse-check a project
./target/release/phpx ast file.php                # dump the syntax tree
```

Useful environment variables:

| Variable | Effect |
|---|---|
| `PHPX_NATIVE=all` | Native-compile every eligible function on its first call |
| `PHPX_NATIVE=off` | Disable the native compiler (pure interpreter + numeric JIT) |
| `PHPX_NATIVE_LOG=1` | Log what the native compiler compiled or rejected (and why) |
| `PHPX_NOJIT=1` | Disable the type-specialised numeric JIT |
| `PHPX_JIT_LOG=1` | Log numeric JIT compilations/rejections |
| `PHPX_PROFILE=1` | Sampling profile report (needs `--features profile`) |

## Status at a glance

| Area | Status |
|---|---|
| PHP 8.x syntax | Complete except backticks (E1006, deliberately), property hooks (E1008, planned), pipe operator `\|>` (PHP 8.5, not yet parsed) |
| Language semantics (interpreter) | Full: classes/traits/enums/readonly, closures, references, generators, `goto`, `global`/`$GLOBALS`, `$$var`, `extract`, `eval`, destructors, superglobals, resources |
| Native compiler (Cranelift) | Most code compiles; functions using references, static vars, globals, `@`, generators etc. transparently fall back to the interpreter |
| Numeric JIT | Int/float-heavy functions, 2–8x faster than PHP |
| Built-in functions | ~400 implemented (snapshot; growing quickly) — see [docs/stdlib.md](docs/stdlib.md) |
| Built-in classes | Exceptions/SPL exception hierarchy, `ArrayIterator`, `Closure`, `Generator`, core interfaces; no Reflection/DateTime/SPL data structures yet |
| Async | Green threads + epoll reactor; `async()`, `Promise`, `Concurrency::run`, `defer`, non-blocking `sleep`/`usleep` |
| Laravel | Probe scripts byte-identical to PHP |
| WordPress | Parses 100%; boot in progress |
| Drupal core | Parses 100%; running not yet attempted |
| HTTP server mode, MySQL/PDO | Not started (roadmap) |

## Documentation

| Document | Contents |
|---|---|
| [docs/README.md](docs/README.md) | Index |
| [docs/architecture.md](docs/architecture.md) | Crates, execution pipeline, value representation |
| [docs/language-support.md](docs/language-support.md) | Feature matrix: interpreter vs native compiler |
| [docs/php-compatibility.md](docs/php-compatibility.md) | Differences from PHP 8.5, diagnostic codes |
| [docs/stdlib.md](docs/stdlib.md) | Built-in functions/classes by extension, what is missing |
| [docs/native-compiler.md](docs/native-compiler.md) | Tiering, ABI, helpers, inline caches, fallbacks |
| [docs/performance.md](docs/performance.md) | Benchmarks, methodology, profiling |
| [docs/async.md](docs/async.md) | Concurrency model and API |
| [docs/testing.md](docs/testing.md) | Differential testing workflow |
| [docs/cms-compatibility.md](docs/cms-compatibility.md) | Laravel / WordPress / Drupal / Joomla / WooCommerce status |
| [docs/roadmap.md](docs/roadmap.md) | Milestones with progress checkboxes |
| [docs/changelog.md](docs/changelog.md) | What has been done so far |

## Repository layout

```
crates/syntax/    lexer, parser, AST, diagnostics (`phpx check`)
crates/runtime/   green threads (corosensei), epoll reactor (polling), Promise, sleep, defer, TCP
crates/interp/    interpreter (semantic reference), stdlib, Composer autoload,
                  native compiler (src/compiler, Cranelift)
crates/jit/       type-specialised numeric JIT (Cranelift)
crates/cli/       the `phpx` binary: run | check | ast
stdlib/stubs/     PHP stubs so IDEs / PHPStan understand the async API
tools/difftest/   compare phpx with real php; regenerate expected outputs
tools/oracle/     compare parser operator trees with nikic/php-parser
```

## Other commands

```sh
cargo test                                         # all crates, incl. differential tests vs stored PHP output
tools/difftest/compare.sh file.php...              # run files with php and phpx, diff stdout + exit code
tools/difftest/regen.sh                            # regenerate crates/interp/tests/php/*.out from real php
tools/oracle/run.sh <nikic/php-parser dir> <php dirs>  # diff operator trees vs php-parser
cargo run --release -p phpx-runtime --example http     # runtime demo HTTP server on :8080 (Rust-level demo)
cargo run --release -p phpx-runtime --example profile  # runtime demo: 1000 concurrent simulated requests
```

## License

MIT (see workspace `Cargo.toml`).
