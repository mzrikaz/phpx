# Language support

This page lists PHP language features and how phpx handles each one in its
two general execution engines:

- **Interpreter**: the tree-walking reference implementation. Every
  feature marked "yes" here is expected to produce output byte-identical to
  PHP 8.5.
- **Native compiler**: whole-function Cranelift compilation
  ([native-compiler.md](native-compiler.md)). "fallback" means a function
  (or top-level segment) that uses the feature is not compiled and runs in
  the interpreter instead — same output, lower speed. Fallback is per
  function, not per program.

The numeric JIT ([native-compiler.md](native-compiler.md#numeric-jit)) only
handles a small int/float/bool subset and is not listed per feature.

Legend: **yes** = supported; **fallback** = interpreter only; **rejected** =
`phpx check` / `phpx run` reports an error; **no** = not implemented.

Sources: `crates/syntax/src/parser.rs`, `crates/syntax/src/checks.rs`,
`crates/interp/src/compiler/analyze.rs`, and the differential tests in
`crates/interp/tests/php/`.

## Syntax-level status

| Feature | Status | Notes |
|---|---|---|
| All PHP 8.x syntax used by WordPress and Drupal core | yes | 100% of 1,512 WordPress files and 10,656 Drupal 11.2.5 core files parse |
| Backtick shell execution `` `cmd` `` | yes | Runs `shell_exec()`, with PHP 8.5's deprecation |
| `create_function()` | **rejected** (E1007) | Removed in PHP 8 anyway |
| Property hooks (PHP 8.4) | **rejected** (E1008) | Planned (roadmap item 7) |
| Pipe operator `\|>` (PHP 8.5) | **no** | Parse error E0002 today; needs implementing for full 8.5 parity |
| `(unset)` cast, call-time `&$x` | rejected (E0002) | Removed from PHP itself (PHP 8 / PHP 5.4) |
| Inline HTML, `<?=`, `?>` | yes | |
| Heredoc / nowdoc (incl. flexible closing marker) | yes | |
| Attributes `#[...]` | yes | Read through Reflection (`getAttributes()`, `newInstance()`) |
| `declare(strict_types=1)` | yes | Per-file strict mode for scalar parameter/return types |

## Control flow

| Feature | Interpreter | Native compiler | Notes |
|---|---|---|---|
| `if` / `elseif` / `else`, alternative syntax | yes | yes | |
| `while`, `do … while`, `for` | yes | yes | |
| `foreach` by value, `$k => $v` | yes | yes | |
| `foreach` with `list()` / `[...]` destructuring target | yes | fallback | Compiler only accepts a plain variable as key/value target |
| `foreach` by reference `&$v` | yes | fallback | |
| `switch` | yes | yes | |
| `match` (incl. `UnhandledMatchError`) | yes | yes | |
| `break N` / `continue N` (constant N) | yes | yes | Non-constant levels fall back |
| `goto` / labels | yes | fallback | |
| `try` / `catch` / `finally` | yes | yes | Multi-catch, catch without variable. `finally` is inlined on `return`/`break`/`continue`; `exit` skips `finally` like PHP |
| `return` / `break` out of a `finally` block | yes | fallback | |
| `throw` as expression | yes | yes | |
| `exit` / `die` | yes | yes | |
| `declare(ticks)` | to verify | — | |

## Functions and closures

| Feature | Interpreter | Native compiler | Notes |
|---|---|---|---|
| User functions, default params, type declarations | yes | yes | Return types checked on exit from native code |
| Union / nullable / intersection / DNF types, `mixed`, `never`, `void`, `static` | yes | yes | |
| Named arguments | yes | yes | |
| Spread `...$args` (positional and string keys) | yes | yes | |
| Variadic parameters | yes | yes | Bound by the general path |
| By-reference parameters (declared in the compiled function) | yes | fallback | |
| Calling a by-ref function from compiled code (`sort($a)`, user `f(&$x)`) | — | yes | Copy-in/copy-out of the local |
| `static` local variables | yes | fallback | |
| Closures `function () use ($x)` | yes | yes | |
| `use (&$x)` | yes | fallback | |
| Arrow functions `fn() =>` | yes | yes | |
| `static function` / `static fn` | yes | yes | |
| First-class callable syntax `f(...)`, `$obj->m(...)` | yes | fallback | |
| Dynamic calls `$f()`, `'name'()`, `[$obj, 'm']()` | yes | yes | |
| `call_user_func(_array)` | yes | yes | |
| `func_get_args()` / `func_num_args()` | yes | fallback | |
| `Closure::bind` / `bindTo` / `call` / `fromCallable` | yes | yes | |
| Recursion depth | yes | yes | 256 MB main-task stack; native calls check remaining stack |

## Classes and objects

| Feature | Interpreter | Native compiler | Notes |
|---|---|---|---|
| Classes, inheritance, `abstract`, `final` | yes | yes (code inside methods) | Declarations themselves are processed by the interpreter/helpers |
| Interfaces, interface constants | yes | yes | |
| Traits (incl. static props, `insteadof`/`as`) | yes | yes | |
| Enums (pure and backed), `cases`/`from`/`tryFrom`, enum constants and methods | yes | yes | |
| `readonly` properties and classes | yes | yes | |
| Constructor property promotion | yes | yes | Promoted params bound by the general path |
| Static properties / methods, `self` / `parent` / `static` | yes | yes | Static property **writes** fall back |
| Late static binding | yes | yes | |
| Class constants, `::class` | yes | yes | |
| `new $className`, `$class::method()`, `$class::CONST` | yes | fallback | Dynamic class references |
| Anonymous classes | yes | fallback | |
| `$obj->$name`, `$obj->$name()` | yes | yes | |
| Nullsafe `?->` | yes | yes (reads) | Nullsafe as a write base falls back |
| `clone`, `__clone`, `clone()` function form (8.5) | yes | yes | |
| Magic methods `__get` `__set` `__isset` `__unset` `__call` `__callStatic` `__invoke` `__toString` | yes | yes | |
| `ArrayAccess`, `IteratorAggregate`, `Iterator`, `Countable`, `Stringable`, `JsonSerializable` | yes | yes | |
| `__destruct` | yes | yes | Runs when the last reference dies, at the next statement boundary; compiled code checks per statement |
| `instanceof` | yes | yes | |
| Property hooks (8.4) | rejected | — | E1008 |
| `__serialize`/`__unserialize`, `__sleep`/`__wakeup`, `__set_state` | no | — | Need `serialize`/`unserialize`/`var_export` object support (in progress) |
| Reflection API | yes | yes | Classes, methods, functions, closures, parameters, properties, constants, enums, types, attributes |

## Variables and scope

| Feature | Interpreter | Native compiler | Notes |
|---|---|---|---|
| References `$a = &$b` | yes | fallback | |
| References in arrays/properties (`$a[0] = &$x`) | yes | fallback | Stored as `Value::Ref` |
| `list()` / `[...]` destructuring, keyed, nested | yes | yes | By-ref/spread items fall back |
| `global $x` | yes | fallback | |
| `$GLOBALS` (read = copy, write/unset via `$GLOBALS['x']`) | yes | fallback | |
| Variable variables `$$x`, `${expr}` | yes | fallback | |
| `"${name}"` string interpolation | yes | yes | PHP 8.2+ deprecation notice is not emitted (see compatibility) |
| `extract()` (EXTR_OVERWRITE / SKIP / PREFIX_SAME / PREFIX_ALL) | yes | fallback | `EXTR_REFS` constant defined; behaviour to verify |
| `compact()`, `get_defined_vars()` | yes | fallback | |
| `eval()` (throws `ParseError` on bad code) | yes | fallback | |
| `@` error suppression | yes | fallback | |
| Superglobals `$_SERVER`, `$_GET`, `$_POST`, `$_COOKIE`, `$_FILES`, `$_REQUEST`, `$_ENV`, `$_SESSION` | yes | fallback | CLI values: `$_SERVER` = environment + `argv`/`argc`/`PHP_SELF`/`SCRIPT_NAME`/`REQUEST_TIME(_FLOAT)`…; the request arrays are empty until the HTTP server mode exists |
| `$argv` / `$argc` | yes | yes | |
| Top-level variables shared with `global` / `$GLOBALS` in called functions | yes | yes | Compiled top-level code binds its locals directly to the frame's variable cells |
| Constants: `define`, `const`, namespaced constants | yes | yes | `const` inside conditional blocks at top level falls back |
| Magic constants `__LINE__`, `__FILE__`, `__DIR__`, `__FUNCTION__`, `__CLASS__`, `__METHOD__`, `__NAMESPACE__` | yes | yes | |

## Expressions and operators

| Feature | Interpreter | Native compiler | Notes |
|---|---|---|---|
| Arithmetic, int overflow to float, `**`, `%`, `intdiv` | yes | yes | Inline int fast paths with overflow checks |
| String concatenation, `.=` in place | yes | yes | |
| Comparison (`==` PHP 8 semantics, `===`, `<=>`) | yes | yes | |
| Bitwise / logical operators | yes | yes | |
| `??`, `??=`, `?:`, ternary | yes | yes | |
| `isset`, `empty`, `unset` | yes | yes | |
| Casts `(int)` `(float)` `(string)` `(bool)` `(array)` `(object)` | yes | yes | |
| Increment/decrement incl. string increment (`'a'++`) | yes | yes | |
| Array literals, spread in arrays | yes | yes | Array items by reference fall back |
| `$a[] = …`, nested index writes, string offsets | yes | yes | `$a[]` in read context is an error, as in PHP |
| `print`, `echo` | yes | yes | |
| `include` / `require` (`_once`) | yes | fallback | A function containing `include` runs interpreted; files themselves are compiled per segment |
| Generators: `yield`, `yield $k => $v`, `yield from`, `send`, `throw`, `getReturn` | yes | fallback | Generators run as green-thread tasks |
| Nested function/class declarations (inside functions or conditionals) | yes | fallback | |

## Namespaces and autoloading

| Feature | Status | Notes |
|---|---|---|
| `namespace` (braced and unbraced, multiple per file) | yes | Each segment compiled separately |
| `use` (classes, functions, constants, group use) | yes | |
| Global function/constant fallback | yes | |
| `spl_autoload_register` | yes | |
| Composer `vendor/autoload.php` | yes | Handled natively (classmap, PSR-4, PSR-0, `files`) |

## Async extensions (phpx only)

These are additions; they do not change the meaning of any valid PHP
program. See [async.md](async.md).

| Feature | Status |
|---|---|
| `async(callable): Promise` | yes |
| `Promise` `wait/then/catch/finally/timeout/cancel/isSettled`, `Promise::all/allSettled/any/race` | yes |
| `Concurrency::run([...])`, `Concurrency::defer([...])` | yes |
| `defer(callable)` | yes |
| Non-blocking `sleep()` / `usleep()` (park the task, not the thread) | yes |
