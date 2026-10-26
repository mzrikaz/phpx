# Contributing recipes

Step-by-step procedures for the changes that come up most. Read
[anatomy.md](anatomy.md) first for the pieces involved, and
[testing.md](testing.md) for how output is checked against PHP.

Ground rules that apply to every recipe:

- **PHP is the spec.** Expected output always comes from the real `php`
  binary (the reference build is in `crates/interp/tests/php/PHP_VERSION`),
  never from reading the PHP manual or from what phpx prints.
- **Messages are byte-exact.** Copy warning, deprecation and exception
  texts from PHP's output (or from php-src), including `Argument #1 ($x)`,
  type names and punctuation.
- **The interpreter is the reference.** Make it right there first; the
  native compiler either reuses the same Rust code through an `rt_*`
  helper or rejects the construct (`compiler/analyze.rs`) so the
  interpreter runs it.
- **Finish in all three engine modes** (default, `PHPX_NATIVE=all`,
  `PHPX_NATIVE=off PHPX_NOJIT=1`), and update the docs that list the
  feature ([stdlib.md](stdlib.md), [language-support.md](language-support.md),
  [php-compatibility.md](php-compatibility.md), [changelog.md](changelog.md)).

Contents:

1. [Write a differential test](#1-write-a-differential-test)
2. [Fix a behaviour difference](#2-fix-a-behaviour-difference)
3. [Add a built-in function](#3-add-a-built-in-function)
4. [Add a built-in class](#4-add-a-built-in-class)
5. [Add a constant](#5-add-a-constant)
6. [Add an ini setting](#6-add-an-ini-setting)
7. [Add a syntax feature](#7-add-a-syntax-feature)
8. [Add a fast path to the native compiler](#8-add-a-fast-path-to-the-native-compiler)
9. [Debug a native-compiler miscompile](#9-debug-a-native-compiler-miscompile)
10. [Fix state leaking between server requests](#10-fix-state-leaking-between-server-requests)

## 1. Write a differential test

1. Pick the next free number: `ls crates/interp/tests/php/*.php | sed 's#.*/##' | sort -V | tail -1`
   (files are `NNN_topic.php`; the newest is currently `199_...`).
2. Write `crates/interp/tests/php/NNN_topic.php`. Print everything you
   want compared (`echo`, `var_dump`, `print_r`). Keep it deterministic: no
   time, randomness without a seed, absolute paths, `__FILE__`/`__DIR__`
   output (tests run as `/test.php` under phpx via `run_capture`), or
   resource ids. Warnings go to stderr and are **not** compared — if the
   text of a warning matters, capture it with `set_error_handler` and echo
   it.
3. Generate the expected output with real PHP. `tools/difftest/regen.sh`
   regenerates **every** `.out`; for one file the equivalent is:

   ```sh
   cd crates/interp/tests/php
   php -n -d display_errors=stderr -d log_errors=0 -d error_reporting=E_ALL NNN_topic.php > NNN_topic.out 2>/dev/null
   code=$?; [ $code -ne 0 ] && echo $code > NNN_topic.exit
   ```

   (`php` here is the lerd wrapper; it only sees files under `$HOME`, which
   includes the repository.)
4. Run it in all modes:

   ```sh
   PHPX_TEST=NNN_topic cargo test -p phpx-interp --test differential
   PHPX_TEST=NNN_topic PHPX_NATIVE=all cargo test -p phpx-interp --test differential
   PHPX_TEST=NNN_topic PHPX_NATIVE=off PHPX_NOJIT=1 cargo test -p phpx-interp --test differential
   ```

   The suite also runs every file twice on one thread
   (`matches_php_output_on_one_thread`), which catches state that leaks
   from one request to the next.
5. Commit the `.php`, `.out` and `.exit` together.

## 2. Fix a behaviour difference

1. **Reproduce** with a minimal script under `~/.cache/phpx-scratch/`:

   ```sh
   f=~/.cache/phpx-scratch/t.php
   php -n -d display_errors=stderr "$f" > want.txt 2>want.err; echo "php exit $?"
   for mode in "" "PHPX_NATIVE=all" "PHPX_NATIVE=off PHPX_NOJIT=1"; do
     env $mode ./target/release/phpx "$f" 2>got.err | diff want.txt - && echo "ok [$mode]"
   done
   ```

   Compare stderr too when the bug is about a warning (PHP's line starts
   `PHP Warning:  ` in this mode, like phpx's).
2. **Classify.**
   - Differs only with the native compiler or JIT → compiler bug; see
     [recipe 9](#9-debug-a-native-compiler-miscompile).
   - Differs in every mode → interpreter or library bug.
   - Differs only on the second request of a `-S` worker → per-request
     state; see [recipe 10](#10-fix-state-leaking-between-server-requests).
3. **Locate.** Grep for the message or the built-in name:
   `grep -rn '"array_splice"' crates/interp/src` finds its registration;
   `grep -rn 'must be of type' crates/interp/src` finds message builders.
   Language semantics live in `expr.rs` (expressions, places, properties),
   `interp.rs` (statements), `call.rs` (calls, classes, types), `ops.rs`
   (operators, conversions), `value.rs` (comparison, dumping). To see which
   PHP code path phpx is in, `PHPX_TRACE_THROW=2` prints every exception
   with a short stack.
4. **Read PHP's implementation** for the exact rule (php-src
   `Zend/zend_operators.c`, `Zend/zend_execute.c`, `ext/standard/*.c`, ...),
   and port the rule rather than tuning to one example. Several phpx
   modules are line-by-line ports for this reason (`zsort.rs`,
   `date/fromformat.rs`, `dom/html_parse.rs`, `xmlwriter.rs`, `random.rs`).
5. **Test** ([recipe 1](#1-write-a-differential-test)), including the
   neighbouring cases (empty input, negative offsets, references, objects
   with `__toString`, `strict_types=1`).
6. If the difference was listed in
   [php-compatibility.md](php-compatibility.md#semantic-gaps), remove it;
   add a [changelog.md](changelog.md) line.

## 3. Add a built-in function

1. **Find PHP's signature.** If the function exists in the reference PHP,
   `crates/interp/src/arginfo_table.rs` already has it (generated from
   Reflection of every internal function):
   `grep -n '("my_func"' crates/interp/src/arginfo_table.rs`. That entry
   gives you argument-count errors, `TypeError`s, scalar coercion, the null
   deprecation and named arguments for free — the Rust body only runs with
   valid arguments. If it is missing (a newer PHP), regenerate the table
   ([php-upgrade-guide.md](php-upgrade-guide.md#4-regenerate-generated-tables)).
2. **Choose the module** by PHP extension (see the
   [Extensions table](anatomy.md#extensions)): e.g. string functions in
   `strings.rs`/`strings_more.rs`, array functions in
   `arrays.rs`/`arrays_more.rs`. A large new extension gets its own module:
   add `mod x;` in `lib.rs`, a `pub fn register(t: &mut HashMap<&'static str, BuiltinFn>)`
   in it, and a `crate::x::register(&mut t);` line in `builtins::table()`.
3. **Register it** inside that module's `register`:

   ```rust
   t.insert("str_example", |it, _f, a| {
       let s = crate::builtins::s(it, a, 0)?;          // string arg (already type-checked)
       let n = if has(a, 1) { int(it, a, 1)? } else { 1 };
       if n < 0 {
           return Err(it.error("ValueError", "str_example(): Argument #2 ($times) must be greater than or equal to 0"));
       }
       Ok(Value::bytes(...))
   });
   ```

   - Signature: `fn(&Interp, &mut Frame, &mut [Value]) -> R<Value>`. The
     frame is the caller's for direct calls (scope-dependent built-ins can
     read `f.class`).
   - Helpers in `builtins.rs`: `arg`, `has`, `s`, `int`, `float`, `arr`,
     `callable`, `reserve` (check `memory_limit` before a large allocation).
   - Warnings: `it.warn(&format!("str_example(): ..."))`; deprecations:
     `it.deprecated(...)` — deprecated *functions* are not flagged by the
     generated table, so emit `Function x() is deprecated since 8.N...`
     yourself (see `utf8_encode` in `ext.rs`).
   - Exceptions: `Err(it.error("ValueError", msg))`.
   - Calling back into PHP: `it.call_callable(&c, args, vec![])`, or for
     many calls `it.callback(c)` (`callback.rs`).
   - Never panic on user input: a panic becomes
     `Error: phpx internal error in name()` (`builtins::guarded`) — that is
     a bug, and `tools/fuzz/all_builtins.php` hunts for them.
4. **By-reference parameters** are taken from the arginfo flags
   automatically (`call::by_ref_params`): the built-in receives the current
   value in `a[i]` and writes the new value back into `a[i]`. Variadic
   by-ref positions and phpx-only built-ins need an entry in `BY_REF`
   (`call.rs`).
5. **Functions that read or write the caller's variables by name** (like
   `compact`, `extract`) must be intercepted in `call_builtin_named`
   (`call.rs`) and listed in `SCOPE_FNS` (`compiler/analyze.rs`) so
   functions that call them stay interpreted.
6. **phpx-internal helpers** for prelude classes are named `__phpx_*`;
   they are hidden from `function_exists()`/`get_defined_functions()` and
   have no arginfo checks.
7. **Hot?** Consider a compiled fast path ([recipe 8](#8-add-a-fast-path-to-the-native-compiler)).
8. **Test and document**: differential test; update the extension's
   list in [stdlib.md](stdlib.md) (or rerun `tools/stdlib_coverage.py`,
   see [stdlib.md](stdlib.md#regenerating-the-coverage-data)).

## 4. Add a built-in class

Every built-in class is declared in PHP in a prelude file; the question is
where the behaviour lives.

| Use | When |
|---|---|
| **Pure prelude PHP** | Interfaces, exceptions, simple value classes, rarely hot logic. Easiest to get PHP-identical. |
| **Prelude PHP + `__phpx_*` built-ins** | The class needs something PHP code can't do (read engine state, bypass visibility, call into a Rust library) but most logic is PHP. Example: Reflection (`prelude_reflection.php` + `reflect.rs`). |
| **Native methods** | Hot methods, or state that must not be visible as properties (`DateTime`, `PDO`, DOM, `Redis`, `Random\Randomizer`). |

Steps:

1. **Declare the class** in the matching `crates/interp/src/prelude_*.php`
   (or `prelude.php`) with PHP's exact name, parent, interfaces,
   constants, property declarations and method signatures (`php
   --rc ClassName` or Reflection gives them). Namespaced classes go in a
   bracketed block, as in `prelude_random.php`:

   ```php
   namespace Foo {
       final class Bar implements \Countable
       {
           public const MODE = 1;
           public function __construct(string $x) {}   // native, see below
           public function count(): int {}              // native
           public function label(): string { return __phpx_bar_label($this); }
       }
   }
   ```

   A new prelude file must be added to the `concat!` in `prelude_source()`
   (`interp.rs`). The prelude must parse without errors (asserted at
   startup) and is declared again by every interpreter, so keep top-level
   code to declarations.
2. **Internal state.** In prelude PHP, use properties named `__phpx_...`
   (`props::HIDDEN_PREFIX`): invisible to `var_dump`, casts, `foreach`,
   `serialize` and comparison. In native code, implement
   `value::NativeData` for a Rust struct and store it with
   `obj.set_internal(Internal::Data(Box::new(state)))`; read it back with
   `obj.internal()` and `as_any().downcast_ref::<T>()`. Implement
   `clone_data` (return `None` for uncloneable classes), `view_props` for
   properties `var_dump` shows but PHP code can't write (like
   `DateTime::$date`), and `compare` if `==`/`<` compare state.
3. **Native methods.** Write
   `fn m(it: &Interp, this: Option<&Rc<Obj>>, cls: &Rc<Class>, args: Vec<Value>) -> R<Value>`
   and register it under the lowercase class name (full namespaced name,
   e.g. `"random\\engine\\mt19937"`) and lowercase method name, with a
   `"static "` prefix for static methods. Either insert it in
   `native::table()` (`native.rs`) or, for a module, return it from the
   module's `pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)>`
   and chain that into `native::table()`. `build_class` (`call.rs`)
   replaces the prelude method of the same name. Native methods get no
   automatic argument checks: validate arguments and raise PHP's
   `TypeError`/`ArgumentCountError` texts yourself. Named arguments to
   native methods are not supported yet.
4. **Hidden built-ins** for the prelude: register `__phpx_<ext>_<what>` in
   the module's `register` (see [recipe 3](#3-add-a-built-in-function)).
   Deprecations from prelude methods: `__phpx_deprecated("Method X::y() is deprecated since 8.N, ...")`.
   Prelude constants can carry `#[\Deprecated(message: ..., since: ...)]`
   (see `prelude_pdo.php`).
5. **Extension name.** If this is a new extension, add it to
   `builtins::EXTENSIONS` (drives `extension_loaded()` and `phpx -m`) and
   its constants to `tools/consts/gen.php` ([recipe 5](#5-add-a-constant)).
6. **Test**: construction, every method, `var_dump`/`print_r`/
   `json_encode` of an instance, `clone`, `serialize` (PHP often forbids
   it for internal classes), Reflection (`getMethods()`, parameter names),
   and subclassing by user code.

## 5. Add a constant

- **A constant of an extension phpx implements** (it exists in the
  reference PHP): add the extension to `$exts` in `tools/consts/gen.php`
  if needed and regenerate `crates/interp/src/consts_table.rs`
  ([anatomy.md](anatomy.md#generated-files-and-their-generators)). This
  also refreshes `DEPRECATED` (PHP 8.4+ `#[\Deprecated]` constants and
  their exact messages, served by `consts::deprecation`).
- **A constant whose value phpx must decide** (version, OS, SAPI, paths):
  `add("NAME", Value::...)` in `builtins::constants()`; it overrides the
  generated value. Module-specific groups use a `constants(add)` function
  called from there (`streams::std_constants`, `sockets::constants`,
  `ext::constants`).
- **Class constants** go in the prelude class declaration.

## 6. Add an ini setting

1. Add `("name", "default")` to `DEFAULTS` in `crates/interp/src/ini.rs`
   (kept sorted; values are PHP 8.5 CLI defaults without a php.ini —
   check with `php -n -r 'var_dump(ini_get("name"));'`). Keys missing from
   `DEFAULTS` make `ini_set()` return `false`, like an unknown directive in
   PHP.
2. Read it where needed with `it.ini_get("name")` (a `String`).
3. If changing it must have an immediate effect (a limit, a cache, a mode),
   add a branch to `Interp::ini_apply` and the key to the list in
   `ini_apply_all` (startup).
4. Per-request: values live in `Interp::ini`, so nothing to reset. If the
   effect is stored in a thread-local, reset it in `reset_request_state`.
5. Document it in [configuration.md](configuration.md#settings-with-effect-today).

## 7. Add a syntax feature

Worked example: PHP 8.5's pipe operator `$x |> f(...)` (not implemented
yet; see [language-support.md](language-support.md)).

1. **Lexer** (`crates/syntax/src/lexer.rs`): new punctuation goes in
   `PUNCTS`, longest tokens first (`"|>"` must come before `"|"` is
   tried). Keywords need no lexer change: every word is a `Tok::Name`.
2. **Parser** (`crates/syntax/src/parser.rs`):
   - binary operators: add an entry to `binary_op()` with PHP's
     precedence and associativity as binding powers (left-assoc:
     `(n, n+1)`; right-assoc: `(n+1, n)`); check against
     `Zend/zend_language_parser.y`'s `%left`/`%right` order;
   - prefix forms and new expressions: `prefix()` / `primary()`;
   - statements: `statement()`; class members: the class body parser;
   - syntax errors: `Diagnostic::error("E0002", span, msg)`.
3. **AST** (`crates/syntax/src/ast.rs`): either add a variant
   (`ExprKind`, `StmtKind`, `BinOp`, `Member`, ...) or **desugar** in the
   parser into existing nodes when PHP's semantics are exactly a rewrite.
   Desugaring means no interpreter or compiler work, but check that error
   messages, evaluation order and line numbers still match PHP (for `|>`,
   PHP evaluates the left side first and passes it as the only argument).
4. **Every match on the enum** must handle a new variant. Exhaustive
   matches fail to compile (`cargo build` lists them); matches with a
   `_ =>` arm silently ignore it, so review those too. Files that match on
   `ExprKind` today: `crates/interp/src/expr.rs` (`eval_rest`),
   `interp.rs`, `call.rs`, `reflect.rs`, `compiler/analyze.rs`
   (`expr_inner`, exhaustive), `compiler/lower.rs` (`expr_kind`),
   `compiler/rt.rs`, `crates/jit/src/{infer,lib}.rs`,
   `crates/syntax/src/{parser,checks}.rs`, `crates/cli/src/optrees.rs`
   (operator dumps for the parser oracle). Find them with
   `grep -rln 'ExprKind::' crates --include=*.rs`.
5. **Interpreter**: implement the semantics in `eval_rest`/`exec_rest`,
   reusing existing helpers (`call_callable`, `binop`, places).
6. **Native compiler**: simplest correct option first — in
   `compiler/analyze.rs` return `Err("reason")` for the new node so
   functions using it stay interpreted (the reason shows in
   `PHPX_NATIVE_LOG=1`). Later, compile it: accept it in `analyze.rs` and
   emit IR in `lower.rs` (`expr_kind`/`stmt_body`), calling an `rt_*`
   helper that runs the interpreter's code for anything non-trivial
   ([native-compiler.md](native-compiler.md#helpers-helpers-in-compilermodrs)).
   The numeric JIT (`crates/jit/src/infer.rs`) should reject unknown nodes.
7. **Unsupported-for-now features** that PHP accepts get an `E1xxx`
   diagnostic in `crates/syntax/src/checks.rs` (like `E1008` for property
   hooks) and a row in
   [php-compatibility.md](php-compatibility.md#diagnostic-codes).
   Remove the diagnostic when the feature lands.
8. **Tests**: a parser test (`crates/syntax/tests/parse.rs` with
   fixtures), a differential test, and the parser oracle for precedence
   (`tools/oracle/run.sh`, needs a nikic/php-parser checkout that knows
   the syntax). Re-run `phpx check --summary` over the WordPress/Drupal
   corpora ([cms-compatibility.md](cms-compatibility.md)) to make sure
   nothing else stopped parsing.
9. Update [language-support.md](language-support.md) (both the
   interpreter and native-compiler columns).

## 8. Add a fast path to the native compiler

Measure first (`tools/bench/run.sh`, [performance.md](performance.md));
fast paths are only worth it on hot code.

### A built-in called from compiled code (`compiler/fastbi.rs`)

1. Add `("name", nargs) => "rt_fast_name"` to `fastbi::lookup`.
2. Write the helper in `fastbi.rs`:

   ```rust
   pub unsafe extern "C" fn rt_fast_name(args: *mut Value, n: u64, dst: *mut Value) -> u64 {
       match arg(args, 0) {
           Value::Str(s) => done(args, n, dst, /* result */),
           _ => 2, // anything unusual: decline, the generic path runs
       }
   }
   ```

   Return `0` after writing `dst` and consuming the arguments (`done`
   does both), or `2` **without touching anything** for every case you do
   not handle exactly like PHP — conversions, warnings, deprecations and
   errors are left to the generic built-in. Fast helpers run outside the
   panic guard: they must not panic.
3. Re-export it in `compiler/rt.rs` (the `pub use super::fastbi::{...}`
   list) and register it in `HELPERS` (`compiler/mod.rs`):
   `rt_fast_name: 3 => true,`.
4. Fast paths are skipped automatically for namespaced calls when a
   namespace function shadows a built-in (`builtin_shadowed`).
5. Test with `PHPX_NATIVE=all` including the declined cases (wrong types,
   `null`, objects with `__toString`, `strict_types=1`).

### A new runtime helper or inline IR

- Helpers: `pub unsafe extern "C" fn rt_x(...) -> u64` in `rt.rs` following
  the slot conventions at the top of that file (every slot holds a valid
  `Value`; `dst` empty on entry; consumed arguments left `Null`; return 0
  ok, 1 exception in `Interp::pending`), add `rt_x: N => true/false` to
  `HELPERS`, call it from `lower.rs`.
- Inline IR in `lower.rs`: follow the existing int fast paths
  (`int_check`/`int_payload`), inline refcounting (`rc_inc`, gated by
  `inline_rc()`), packed-array reads (`packed_elem`, offsets probed by
  `compiler::layout()`), property inline caches (`prop_guard`,
  `prop_slot`). Every inline path needs a slow-path block that calls the
  general helper.
- Making a rejected construct compile: remove its `Err(...)` in
  `analyze.rs`, then lower it. The list of reject reasons is in
  [native-compiler.md](native-compiler.md#what-compiles).
- Record the before/after numbers in [performance.md](performance.md).

## 9. Debug a native-compiler miscompile

1. Confirm: output differs with `PHPX_NATIVE=all` but not with
   `PHPX_NATIVE=off PHPX_NOJIT=1`. If it differs with the JIT only
   (`PHPX_NATIVE=off` alone still wrong), suspect `crates/jit`.
2. `PHPX_NATIVE_LOG=1` lists what compiled.
3. Bisect: `PHPX_NATIVE_MAX=n` compiles only the first `n` functions;
   halve `n` until the output flips, then `PHPX_NATIVE_SKIP=name` to
   confirm the culprit.
4. `PHPX_NATIVE_DUMP=name` prints its Cranelift IR and machine code;
   `PHPX_NATIVE_VERIFY=1` runs Cranelift's verifier (catches malformed IR).
5. Typical causes: a helper's slot convention broken (a value released
   twice or leaked), a fast path that doesn't decline on an edge case, a
   missing destructor/statement-boundary check, `Interp::pos` not updated
   before a call that can warn.
6. Add the minimal case as a differential test.

## 10. Fix state leaking between server requests

Symptom: a `-S` worker's second request behaves differently from the
first (or from `php -S`), or `matches_php_output_on_one_thread` fails in
round 1.

1. Find the `thread_local!` holding the state:
   `grep -rn 'thread_local!' crates/interp/src`. The table in
   [anatomy.md](anatomy.md#per-process-per-thread-and-per-request-state)
   says which are reset today.
2. Give the module a `pub fn reset()` (or `pub(crate)`) and call it from
   `reset_request_state()` in `lib.rs`. State tied to classes belongs in
   `call::reset_classes`; compiler caches in `Native::reset_caches`.
3. Caches are fine to keep only if keyed by immutable data (AST
   addresses, file content stamps) and holding nothing a PHP script can
   observe or mutate.
