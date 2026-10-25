# Roadmap

Tick boxes as work lands, and add a line to [changelog.md](changelog.md).
Last updated: 2026-10-25.

Legend: `[x]` done · `[ ]` pending · *(in progress)* = actively being worked on.

## 0. Foundations (done)

- [x] Green-thread scheduler (corosensei), timers, `Promise`, `sleep`, `defer`
- [x] I/O reactor (epoll via `polling`), non-blocking TCP, demo HTTP server
- [x] Full PHP 8.x parser with diagnostics (`phpx check`, `phpx ast`)
- [x] Tree-walking interpreter as semantic reference, byte-identical on the differential suite
- [x] Numeric JIT (Cranelift), type-specialised, deopt by re-execution
- [x] Native compiler (Cranelift) for general code with transparent interpreter fallback
- [x] Native compilation of file top-level code
- [x] Native Composer autoloader
- [x] Full dynamic-PHP support: references, `goto`, generators, `global`, `$GLOBALS`, `$$var`, `extract`, `eval`, `@`, destructors, superglobals, resources
- [x] WordPress and Drupal core parse 100%
- [x] Laravel probe scripts byte-identical to PHP

## 1. Stdlib for WordPress boot *(in progress)*

Target: `wp-admin/setup-config.php` output identical to PHP.

- [x] File streams: `fopen` family, `php://` streams, directories, `stat` family, stream contexts
- [x] `header`, `headers_sent`, `setcookie`, `http_response_code`
- [x] `serialize` / `unserialize`
- [x] `pack` / `unpack`
- [x] `parse_url` / `parse_str`
- [x] `version_compare`
- [x] `strspn`, `strcspn`, `sscanf` and other string gaps
- [x] `htmlentities` / `html_entity_decode`
- [x] `hash`, `hash_hmac`, `sha1`, `hash_equals`
- [x] `array_replace_recursive` and other array gaps
- [x] `filter_var`, bcrypt `password_*`, `gz*`, CSV, `next`/`prev` array pointer, class introspection
- [x] `extension_loaded` reports implemented extensions; `PHP_VERSION` = 8.5.10
- [x] Secure `random_bytes` (OS CSPRNG)
- [ ] Reflection API (`ReflectionClass`, `ReflectionMethod`, ...)
- [ ] DateTime family (`DateTime`, `DateTimeImmutable`, `DateTimeZone`, `DateInterval`, `DatePeriod`) and `strtotime`/`gmdate`
- [ ] SPL data structures and iterators (`ArrayObject`, `SplObjectStorage`, `SplStack`, iterators)
- [ ] `set_error_handler` invoking user handlers (Laravel converts warnings to exceptions)
- [ ] WordPress setup page byte-identical to PHP (currently blocked on mysqli check)

## 2. Databases

- [ ] mysqli (procedural + OO), async-capable (parks the green thread)
- [ ] PDO core + MySQL driver
- [ ] PDO SQLite
- [ ] PDO Postgres
- [ ] WordPress install + front page against MySQL

## 3. HTTP server mode (`phpx serve`)

- [ ] Request → green thread, per-request isolated globals and superglobals (`$_GET`, `$_POST`, `$_COOKIE`, `$_FILES`, `$_REQUEST`, `$_SERVER`)
- [ ] `header()` / status codes / `headers_sent()` semantics
- [ ] Cookies, sessions (`session_*`, crypto via RustCrypto)
- [ ] Output buffering integrated with the response
- [ ] Static file serving
- [ ] Front-controller routing (Laravel `public/index.php`, WordPress `index.php`)
- [ ] M:N scheduler across cores (work stealing)

## 4. Native compiler coverage

Constructs that currently force interpreter fallback
([native-compiler.md](native-compiler.md#what-compiles)):

- [ ] References in locals (`$a = &$b`)
- [ ] By-reference parameters
- [ ] `foreach` by reference
- [ ] `use (&$x)` captures
- [ ] `static` variables
- [ ] `global` / `$GLOBALS` / superglobals via helpers
- [ ] `@` operator
- [ ] `foreach` with `list()` targets
- [ ] Dynamic class references (`new $class`, `$class::m()`), anonymous classes
- [ ] First-class callable syntax `f(...)`
- [ ] Static property writes
- [ ] Generators
- [ ] `goto`

## 5. Performance: beat PHP 8.5 on every microbenchmark

See [performance.md](performance.md).

- [x] Inline int fast paths, inline refcounting
- [x] Inline caches for calls, methods, property reads
- [x] Built-in calls faster than PHP
- [x] Property read/write fast path (inline `PropSite` guard)
- [x] Packed-array index fast path
- [x] Inline `foreach` over packed arrays
- [x] Inline `strlen`/`count`/`is_*`
- [x] Direct native-to-native calls for functions and instance methods
- [x] Inline caches for static calls and `new` with constructors
- [x] Native closure calls (direct `$f()` and callbacks from built-ins)
- [x] Compiled code checks its own return types
- [x] Smaller objects (48-byte header, side table for native state)
- [x] Size-class allocator for small blocks
- [x] Benchmark scripts and runner (`tools/bench/run.sh`)
- [ ] Direct calls for static methods, constructors and closures
- [ ] Single allocation per object; cheaper closure creation
- [ ] Arguments in registers for direct calls
- [ ] Memory per object at or below PHP's (112 vs 87 bytes)
- [ ] String interpolation specialisation

## 6. Stdlib breadth

- [ ] DateTime family (`DateTime`, `DateTimeImmutable`, `DateTimeZone`, `DateInterval`, `DatePeriod`)
- [ ] Reflection (classes, methods, properties, parameters, attributes)
- [ ] SPL data structures and iterators (`ArrayObject`, `SplObjectStorage`, `SplStack`, …)
- [ ] curl (async-capable)
- [ ] gd / imagick
- [ ] xml parser, DOM, SimpleXML, XMLReader/XMLWriter
- [ ] sodium, openssl
- [ ] hash extension (full)
- [ ] intl, mbstring completeness
- [ ] zip, zlib
- [ ] filter, fileinfo, session
- [ ] Process functions (`exec`, `proc_open`, `shell_exec`) — `shell_exec` is missing today

## 7. Language

- [ ] Property hooks (PHP 8.4) — currently E1008
- [ ] Pipe operator `|>` (PHP 8.5)
- [ ] `PHP_VERSION` reports 8.5
- [ ] Real stack traces (`getTrace`, `debug_backtrace`)
- [ ] Generics from `@template` docblocks (type checking/compilation hints)
- [ ] `phpx fix` migration tool
- [ ] `phpx.json` project configuration

## 8. Real applications end-to-end in CI

- [ ] WordPress (install, front page, admin) vs PHP 8.5 output
- [ ] WooCommerce (catalog, cart, checkout)
- [ ] Drupal (install, front page)
- [ ] Joomla (install, front page)
- [ ] Laravel application (routes, Eloquent, Blade, queues)
