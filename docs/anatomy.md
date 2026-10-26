# Engine anatomy

A map of the whole engine for people (and agents) who need to fix a bug,
speed something up, or add a PHP feature. [architecture.md](architecture.md)
is the one-page overview; this page goes module by module and follows a
script from source bytes to exit code. Step-by-step procedures are in
[contributing-recipes.md](contributing-recipes.md); bringing phpx to a new
PHP release is in [php-upgrade-guide.md](php-upgrade-guide.md).

Everything named here exists in the tree as of 2026-10-26. Line numbers are
deliberately left out (files change daily); grep for the names.

## Contents

1. [Crates](#crates)
2. [Life of a script](#life-of-a-script)
3. [Core data types](#core-data-types)
4. [How calls work](#how-calls-work)
5. [Classes: declaration, layout and reuse](#classes-declaration-layout-and-reuse)
6. [Built-in functions](#built-in-functions)
7. [Built-in classes: prelude PHP and native methods](#built-in-classes-prelude-php-and-native-methods)
8. [Errors, exceptions and output](#errors-exceptions-and-output)
9. [Tiering: native compiler and numeric JIT](#tiering-native-compiler-and-numeric-jit)
10. [Memory: allocator, refcounts, cycle collector](#memory-allocator-refcounts-cycle-collector)
11. [Per-process, per-thread and per-request state](#per-process-per-thread-and-per-request-state)
12. [Extensions](#extensions)
13. [Module reference: `crates/interp/src`](#module-reference-cratesinterpsrc)
14. [Debugging switches](#debugging-switches)
15. [Generated files and their generators](#generated-files-and-their-generators)

## Crates

| Crate | Package | What it owns |
|---|---|---|
| `crates/syntax` | `phpx-syntax` | Source → AST. `lexer.rs` (`Tok`, `Token`, `PUNCTS`; keywords are plain `Name`s, the parser decides by context), `parser.rs` (recursive descent for statements, Pratt parser for expressions; `binary_op()` is the operator/precedence table; errors never abort, each becomes a `Diagnostic` and parsing resumes at the next `;`/`}`), `ast.rs` (`File`, `Stmt`/`StmtKind`, `Expr`/`ExprKind`, `BinOp`, `UnOp`, `Class`, `Member`, `Function`, `Param`, `Type`, ...), `checks.rs` (tree-level rules: `E1007` `create_function`, `E1008` property hooks), `diag.rs` (rustc-style rendering), `stack.rs` (stack budget so deeply nested source fails with PHP's "Maximum call stack size ... reached during compilation" instead of crashing). Entry point: `phpx_syntax::check(src) -> (ast::File, Vec<Diagnostic>)` in `lib.rs`. |
| `crates/interp` | `phpx-interp` | Everything else PHP: interpreter, values, classes, every built-in function and class, the HTTP server, and the native compiler (`src/compiler/`). See [module reference](#module-reference-cratesinterpsrc). |
| `crates/jit` | `phpx-jit` | Numeric JIT: `lib.rs` (`Jit`, `Jit::call`, `Outcome::{Done, Deopt, Skip}`, `JitVal`, the `Resolver` trait the interpreter implements to resolve calls), `infer.rs` (one type per local: int, float or bool, or reject). |
| `crates/runtime` | `phpx-runtime` | Green threads and I/O. `scheduler.rs` (stackful `corosensei` tasks; `spawn_task`, `park`, `wake`, `yield_now`, `defer`, per-task "task word" used by `trace.rs`, `stack_remaining`, `set_stack_size`), `promise.rs` (`Promise`, `spawn`), `net.rs` (non-blocking sockets on an epoll reactor; a would-block parks the task), `tls.rs` (rustls client streams), `lib.rs` (`block_on`, `sleep`, `concurrency::run`). |
| `crates/cli` | `phpx-cli` (binary `phpx`) | `main.rs`: PHP's command line (`-r`, `-f`, `-S`, `-t`, `-d`, `-n`, `-c`, `-v`, `-m`, `-l`) plus `phpx check`, `phpx ast`, `phpx run`; `PHPX_PROFILE` (pprof) reports. `optrees.rs`: dumps operator trees for the parser oracle (`tools/oracle`). |

Dependency direction: `cli → interp → {syntax, runtime, jit}`; `jit →
syntax`. Nothing depends on `cli`.

## Life of a script

### CLI (`phpx file.php`)

1. **Command line** — `crates/cli/src/main.rs` `php_cli()` parses options,
   calls `phpx_interp::set_ini_options(no_ini, overrides)` (stored in the
   process-wide `INI_OPTS`), then `run_file_args()` (`crates/interp/src/lib.rs`).
2. **Read** — `run_file_args` reads the file (lossy UTF-8), canonicalizes
   the path and builds `$argv`. `run_source_args` → `run_main(path, src,
   out, setup)` where `setup` is `cli_globals` (fills `$_SERVER` from the
   environment, `$argv`, `$argc`, empty `$_GET`/`$_POST`/...).
3. **`Interp::new(writer)`** (`interp.rs`) — first resets everything a
   previous interpreter on this thread left behind (see
   [state](#per-process-per-thread-and-per-request-state)):
   `compiler::for_new_interp()`, `call::reset_classes()`,
   `gc::new_request()`, `reset_request_state()`, `builtin_shadowed()`.
   Then builds the `Interp`: built-in function table
   (`builtins::table()`), native method table (`native::table()`),
   constants (`builtins::constants()`), output stack, numeric JIT
   (unless `PHPX_NOJIT`), and finally `load_prelude()`: the prelude PHP
   (all `prelude*.php` files concatenated, parsed once per thread, file id
   `PRELUDE_FILE` = 0) is executed so every built-in class is declared.
4. **ini** — `Ini::load` (php.ini lookup + `-d`), then
   `Interp::ini_apply_all()` applies the settings with side effects
   (`memory_limit`, `include_path`, `zend.enable_gc`, `max_execution_time`).
5. **Parse** — `Interp::parse_file(path, src)`: the thread's `PARSED`
   cache returns the same leaked `&'static ast::File` for identical source;
   otherwise `phpx_syntax::check` runs (with the remaining task stack as
   budget). Errors render rustc-style and the script exits with 255. ASTs
   are leaked on purpose (`Box::leak`): they live for the process like
   opcache, are excluded from `memory_limit` (`alloc::exclude`), and every
   cache keys on AST node addresses.
6. **Run** — `phpx_runtime::block_on` starts the root green thread (256 MB
   lazily committed stack, `MAIN_STACK`; other tasks get `INTERP_STACK` =
   8 MB). In it: `Frame::new` for the global scope, `setup`, `globals` is
   pointed at that frame, `auto_prepend_file`, then
   `Interp::run_file_in` → `run_stmts_top`:
   - detects `declare(strict_types=1)` (`file_strict`);
   - `namespace_segments` splits the file into `(statements, NsCtx)` runs
     (one per `namespace` block, with its `use` imports), cached per file id;
   - `hoist` declares top-level functions, then classes whose parent,
     interfaces and traits are already declared (PHP's early binding;
     `deps_ready`), in as many passes as needed; the rest are declared when
     execution reaches them (`exec_rest`, `StmtKind::Class`);
   - each segment is offered to the native compiler (`native_top_level`),
     else `exec_block` interprets it.
7. **Statements** — `Interp::exec` handles the hot kinds (expression,
   `return`, simple `if`) and hands the rest to `exec_rest`. Before each
   statement it checks the per-thread `destruct_pending()` flag (queued
   destructors, due GC, memory/time limit) and records the position
   (`Interp::pos`, a `Pos { file, off }` used for every error line).
   `exec_block` runs a statement list and resolves `goto` within it.
   Control flow is the `Flow` enum (`Normal`, `Break(n)`, `Continue(n)`,
   `Return(v)`, `Goto(label)`); non-local exits are `Err(Ctrl)`.
8. **Expressions** — `Interp::eval` (`expr.rs`) handles literals and
   variable reads inline and sends everything else to `eval_rest`. Writes
   go through "places": `$a['x'][] = 1` resolves to a root plus
   pre-evaluated keys (`write_path`, `make_ref`, `bind_ref`) so no user code
   (`offsetSet`, `__set`, `__toString`) runs while a `RefCell` is borrowed.
   Operators and conversions live in `ops.rs` (`binop`, `to_str`,
   `to_int`, `to_float`) and `value.rs` (`loose_eq`, `compare`, `identical`).
9. **Shutdown** (still inside `block_on`, in this order): `auto_append_file`
   (only on normal completion) → `run_shutdown()`
   (`register_shutdown_function` callbacks) → the global frame is dropped
   → `close_all_generators()` → `destruct_remaining()` (queued destructors,
   then every live object with `__destruct` in handle order, like
   `zend_objects_store_call_destructors`) → `session::finish` (write the
   session) → `ob::end_all` (flush buffers through their handlers). Then
   `Interp::flush` writes the output and `phpprof::finish` reports.
   Exit code: 0, the `exit()` code, or 255 for an uncaught exception
   (`errors::uncaught`, which also runs `set_exception_handler`).

### Built-in server (`phpx -S host:port [-t docroot] [router.php]`)

`server.rs`: `serve()` sets `sapi()` to `"cli-server"`, binds the listener
and starts `PHP_CLI_SERVER_WORKERS` worker threads (default 1, 64 MB OS
stack each). Each connection goes to one worker: `handle()` reads the
request, resolves the target (`resolve`), runs the router if any, serves
static files itself (`serve_static`), or calls `run_php()`. `run_php` calls
the same `run_main` with an `HttpOut` writer (headers go out before the
first body byte) and a `setup` closure that calls `request_globals`
(`$_SERVER`, `$_GET`, `$_POST`, `$_COOKIE`, `$_FILES` incl. multipart
uploads, `php://input` body in the thread-local `BODY`). After each request
the worker calls `reset_request_state()`. Log lines match `php -S`
(Laravel parses them). `PHPX_SERVE_MAX_REQUESTS=n` stops after `n`
connections.

What a worker thread keeps between requests (each request still gets a
fresh `Interp`): parsed files (`PARSED`, `FILE_REG`, so file ids are
stable), compiled native code (`THREAD_NATIVE`), built classes (`BUILT`,
see [class reuse](#reuse-across-requests)), resolved includes for 2 s
(`RESOLVED` in `call.rs`, like `opcache.revalidate_freq`), the realpath
cache (`fs.rs` `REALPATH`), compiled regexes (`preg.rs` `CACHE`) and
Reflection descriptors (`reflect.rs`).

### `include` / `require`

`Interp::include` (`call.rs`): resolves relative paths against the
including file's directory, then the CWD; on a server worker, a path
resolved less than 2 s ago is reused without touching the disk. `*_once`
checks `Interp::included`. `load_file` reuses the parsed AST when the
file's (mtime, size, inode) stamp is unchanged. The file runs in the
caller's frame (`run_file_in`) — same scope, like PHP. `vendor/autoload.php`
runs Composer's real autoloader unless `PHPX_NATIVE_COMPOSER=1`
(`composer.rs` `composer_boot` then loads the generated maps directly).

## Core data types

### `Value` (`value.rs`)

`#[repr(C, u8)]`, 16 bytes: tag byte at offset 0, payload at offset 8
(`value::tag` lists the tags; generated code depends on this layout).

| Variant | Payload | Notes |
|---|---|---|
| `Null`, `Bool`, `Int(i64)`, `Float(f64)` | inline | |
| `Str(PStr)` | `pstr.rs` | One allocation: `Header { strong, len, cap }` + bytes, like `zend_string`. Bytes, not UTF-8. The strong count is the first word so native code bumps it inline. Build with `Value::str(&str)`, `Value::bytes(..)`, `value::pstr`, `value::pbytes`. |
| `Array(Rc<PhpArray>)` | copy-on-write | A shared array (refcount > 1) is copied before it is written. |
| `Object(Rc<Obj>)` | | |
| `Ref(Slot)` | `Slot = Rc<RefCell<Value>>` | A PHP reference stored **inside** an array element or property. Variables never hold `Ref` directly: two variables that are references share one `Slot` in `Vars`. Reads dereference (`deref`, `into_deref`). |
| `Resource(Rc<Resource>)` | `Resource { id, kind, data }` | Streams, dir handles, contexts, processes. Closing sets `kind` to `"Unknown"`. |

Conversions with PHP 8 semantics: `Value::truthy`, `Interp::to_str`/
`to_int`/`to_float` (may warn or throw, so they take `&Interp`),
`value::parse_numeric`, `fmt_float_str`/`fmt_float_repr` (precision /
serialize_precision formatting). Dumping: `var_dump_item`, `print_r_body`,
`var_export_checked` in `value.rs`.

### `PhpArray` (`value.rs`)

`#[repr(C)]` with mirrored fields `pk` (packed buffer pointer), `pk_len`
and `count` at fixed offsets so native code can index packed arrays and
read `count()` without a call. Storage is `Store::Packed(Vec<Value>)` while
keys are exactly `0..n` in order, `Store::Hash(FxIndexMap<Key, Value>)`
otherwise (converted the first time a key breaks the shape). `next_free`
is PHP's next integer key; `pos` is the internal pointer
(`current`/`next`/`reset`). `Key` is `Int(i64) | Str(PStr)`; numeric
strings are canonicalized to ints (`canonical_int`). Cloning drops
references only the source held (`copy_elem`), as PHP does.

### `Obj`, properties and shapes (`value.rs`, `props.rs`)

`Obj { id, gc, class: Rc<Class>, props: RefCell<Props> }`. `id` is the
object handle (`spl_object_id`) and also its slot in the collector's
registry (`gc.rs`). Native state (closures, generators, promises,
`DateTime`, `PDO`, DOM nodes, ...) is **not** in the object: it lives in
the thread-local `INTERNALS` map keyed by handle and is reached with
`Obj::internal()`, `internal_mut()`, `set_internal()`; the type is
`Internal::{None, Closure, Promise, Generator, Data(Box<dyn NativeData>)}`.
`NativeData` lets a class supply clone behaviour (`clone_data`), fake
properties for dumps (`view_props`, e.g. `DateTime::$date`) and comparison.

`Props` = a shared `Shape` (property names in order, like V8 hidden
classes) + a flat `Vec<Option<Value>>`. All instances of a class start from
`Class::shape`; adding a dynamic property moves the object to a child shape
(shared transition), beyond 64 properties/transitions it goes to a private
"dictionary" shape. `None` = unset or uninitialized typed property.
Properties whose name starts with `props::HIDDEN_PREFIX` (`__phpx_`) are
internal state of prelude classes: they behave like properties but are
invisible to dumps, casts, `foreach`, serialization and comparison.

### `Class`, `Method`, `PropDecl` (`class.rs`)

Classes are **flattened at declaration**: inherited methods, properties and
constants are copied in, so lookups never walk the parent chain.
Important fields: `name`/`lname` (lowercase key), `parent`, `ancestors`
(lowercase names of every ancestor and interface: `is_a()` is a scan),
`methods: FxIndexMap<lname, Rc<Method>>`, `props: Vec<PropDecl>`,
`static_props` (shared with the parent unless redeclared), `consts`
(`ConstSlot::{Pending, Evaluating, Ready, EnumCase}`: evaluated lazily)
with `consts_init` (pristine copy for reuse), `ctor`, `magic`
(`__get/__set/__unset/__isset`), `shape`, `prop_template`, `has_destruct`,
`plain_new` (fast `new` path), `decl` (the AST, for Reflection),
`decl_ctx`, `deps` (resolved interfaces/traits, for reuse checks),
`anon_name` (`Base@anonymous\0file:line$n`). `Method.body` is
`MethodBody::User(&ast::Function)` or `MethodBody::Native(NativeMethod)`.

### Interpreter state: `Interp`, `Frame`, `Vars`, `NsCtx` (`interp.rs`)

- **`Interp`** — one per script/request, shared as `Rc<Interp>` (`Interp::rc()`).
  Holds the function table (`functions`), built-ins (`builtins`), native
  methods (`natives`), declared classes (`classes`), constants, `static`
  variables (`statics`), output buffer stack (`out`, level 0 =
  unbuffered), call-site caches (`call_cache`, `method_sites`,
  `class_cache`, `str_callables`), `included` files, `ini`, the current
  position (`pos`, `builtin_pos`), `@` depth (`silence`), `pending` (an
  exception raised inside native code), `want_ref`/`ref_return_depth`
  (return-by-reference plumbing), `magic_guards` (recursion guards for
  `__get` & co), `native` (the thread's native compiler), `jit`.
- **`Frame`** — one per running function: `vars: Vars`, `this`, `class`
  (`self`), `static_class` (`static`), `ctx: Rc<NsCtx>`, `func` (name),
  `args` (as passed, for `func_get_args`), `depth`, `fn_id` (AST address,
  keys `static $x`), `gen` (set inside a generator body), `closure`.
- **`Vars`** — `Vec<(&'static str, Slot)>`, linear scan up to 24 entries,
  then an `FxHashMap` index. Names borrow from the leaked AST
  (`Interp::intern` for names made at run time). `get_hinted` uses the
  per-AST-node `Expr::hint` cell as an inline cache. Rebinding or removing
  a variable bumps `SCOPE_EPOCH` so compiled top-level code re-reads its
  cells.
- **`NsCtx`** — name-resolution context of one namespace block: `ns`,
  `uses`/`use_fns`/`use_consts`, `file`, `strict`.
- **`Ctrl`** — `Throw(Rc<Obj>)`, `Exit(i32)`, `GenClose`. `R<T> =
  Result<T, Ctrl>` is the return type of almost everything. Build PHP
  exceptions with `Interp::error("TypeError", msg)` (sets file, line and
  trace; `PHPX_TRACE_THROW` logs it).

### `Callable` (`interp.rs`)

```rust
pub enum Callable {
    User(Rc<UserFn>),                       // declared function
    Builtin(&'static str, BuiltinFn),       // built-in function
    Closure(Rc<ClosureData>),               // closure / arrow fn / first-class callable
    Method { this, class, method, static_class },
    Magic { this, class, name },            // answered by __call / __callStatic
}
```

`ClosureData` holds the AST (`ClosureKind::{Closure, Arrow, Wrap}`),
captured values (`use (&$x)` captures hold `Value::Ref` of the caller's
slot), bound `$this`, scope, static class and PHP 8.4+ name
(`{closure:file:line}`). `Interp::to_callable(frame, &Value)` turns any PHP
callable value into a `Callable`; `Interp::closure_object(Callable)` wraps
one in a `Closure` object.

## How calls work

All in `call.rs` unless noted.

```
foo($a)            eval_rest → call_named(name, args)
                     resolve_call  (call_cache by name AST address → find_function)
                     ├─ Builtin → call_builtin_named
                     │            ├─ scope readers: compact, extract, eval, func_get_args,
                     │            │  get_defined_vars, func_num_args, get_called_class, ...
                     │            ├─ by-ref params (by_ref_params: arginfo flag 2 + BY_REF) → call_by_ref_builtin
                     │            └─ builtins::invoke(spec check → guarded(fun))
                     └─ user → eval_args_for (by-ref args as Value::Ref) → call_callable
call_callable(c, pos, named)
   User     → try_jit (all int/float/bool args) → invoke(Invoke{...})
   Builtin  → call_builtin (named args via arginfo Spec::bind_named)
   Closure  → call_closure → invoke
   Method   → invoke_method → Native: fun(it, this, static_class, args) | User: invoke
   Magic    → __call / __callStatic with (name, args array)
invoke(inv, pos, named)
   stack check (STACK_RESERVE) · return-by-ref? · native_entry_of (compiled?) →
   native_call_args (bind straight into compiled frame) or
   bind_args (defaults, named, variadics, by-ref, type coercion check_param/coerce, promoted ctor params) →
   generator? start_generator · by-ref return? invoke_ref ·
   run_native_entry or trace::enter + exec_block/eval → check_return
```

- **Methods**: `$o->m()` → `Interp::method_call` (visibility via
  `check_method_visible`, `Class::find_method_from` for private-method
  scoping, `method_sites` cache) → `invoke_method`. Static calls →
  `static_invoke` (`parent::`/`self::`/`static::` forward the late static binding class; a non-static method called statically needs a compatible `$this`).
  From Rust, call PHP methods with `Interp::call_method(&obj, "name", args)`.
- **Callbacks from built-ins** (`array_map`, `usort`, ...): use
  `Interp::callback(callable)` (`callback.rs`), which resolves the target
  once to its compiled entry and reuses one frame per call; falls back to
  `call_callable` for anything unusual.
- **`new`**: `instantiate_begin` ("Cannot instantiate ..." checks,
  `new_instance` creates the object, constructor visibility) → constructor
  → `instantiate_finish`.
- **Type checks**: parameter and return types use `type_matches` and
  `coerce` (scalar juggling in weak mode, `strict_types` per calling file).
  Type names in messages come from `type_str` (PHP's ordering).
- **Stack traces**: every running body (interpreted or compiled) links a
  `trace::CallRec` on its own native stack (`trace::enter` returns a
  `Guard`); `trace::entries` walks the list for `debug_backtrace()` and
  exception traces. The list head is the green thread's task word, so each
  task has its own stack.
- **Generators** (`generator.rs`): the body runs as its own green-thread
  task; `next()/send()/throw()` pass a `Resume` and park the caller until
  the body yields an `Event`. **Fibers** (`fiber.rs`) use the same idea.

## Classes: declaration, layout and reuse

`Interp::declare_class(ast, ctx)` → `build_class` (or a reused build) →
inserted into `Interp::classes` → recorded in `IN_USE` and `declared`.
`build_class` resolves the parent (`lookup_class`, which autoloads via
`spl_autoload_register` callbacks in `Interp::autoloaders`), interfaces
and traits; copies inherited members; applies trait adaptations; adds
**native methods** from `Interp::natives` whose key matches the class's
lowercase name (or `*enum` for every enum: `cases`/`from`/`tryFrom`); checks
abstract methods; builds the shape. Anonymous classes:
`declare_anon_class` (cached per AST node in `anon_classes`).

### Reuse across requests

A server worker declares the same classes on every request. `BUILT`
(thread-local, keyed by the class AST address) keeps each built class.
`declare_class` reuses it when `can_reuse` confirms the same namespace
context and that the parent, interfaces and traits resolve to the very
same `Rc<Class>` objects (resolved in build order, so autoloading happens
identically). At the start of the next request `call::reset_classes()`
returns every class the last request used to its as-built state: constants
back to `consts_init`, static properties unset, `prop_template` cleared,
and the destructor queue dropped (PHP runs no destructors after shutdown).
If you add mutable per-request state to `Class`, reset it there.

## Built-in functions

- **Signature**: `type BuiltinFn = fn(&Interp, &mut Frame, &mut [Value]) -> R<Value>`
  (`interp.rs`). Arguments arrive dereferenced; for by-reference parameters
  the current value is in the slot and the built-in writes the result back
  into it (see `arrays.rs` header and `call_by_ref_builtin`).
- **Registration**: each module has `pub fn register(t: &mut HashMap<&'static str, BuiltinFn>)`
  with `t.insert("name", |it, f, a| { ... })`. `builtins::table()` calls
  every module's `register`; the table is copied into `Interp::builtins`.
  Names are lowercase; namespaced built-ins use their full name
  (`"Dom\\import_simplexml"`).
- **Argument helpers** (`builtins.rs`): `arg`, `has`, `s` (string), `int`,
  `float`, `arr`, `callable`, `reserve` (check `memory_limit` before a big
  allocation).
- **Signatures and errors** (`arginfo.rs` + generated `arginfo_table.rs`):
  `arginfo::spec(name)` returns PHP's own signature (from Reflection of the
  reference PHP build). `builtins::invoke` → `Spec::check` raises
  `ArgumentCountError`, `TypeError` and the "Passing null to parameter ...
  is deprecated" deprecation with PHP's exact text, and performs PHP's
  scalar coercions *before* the built-in runs; `Spec::bind_named` maps
  named arguments. A built-in without a spec (phpx-only, `__phpx_*`) gets
  no checks and no named arguments.
- **Panic safety**: `builtins::guarded` turns a Rust panic into
  `Error: phpx internal error in name(): ...` instead of aborting.
- **Scope-reading built-ins** (`compact`, `extract`, `eval`,
  `func_get_args`, `get_defined_vars`, ...) are intercepted in
  `call_builtin_named` and cannot be called dynamically.
- **Hidden helpers**: names starting with `__phpx` are filtered out of
  `function_exists()`/`get_defined_functions()`; they back prelude classes.
- **Constants**: `builtins::constants()` starts from the generated
  `consts_table::CONSTS` (PHP's values for implemented extensions), then
  phpx overrides (`PHP_VERSION`, `PHP_OS`, `PHP_SAPI`, ...).
  `consts::deprecation(name)` serves `consts_table::DEPRECATED` messages.
- **Compiled fast paths**: native code calls some built-ins through
  `compiler/fastbi.rs` (`lookup` → `rt_fast_*`) instead of the generic
  path; see [native-compiler.md](native-compiler.md).

## Built-in classes: prelude PHP and native methods

Every built-in class and interface is **declared in PHP** in one of the
prelude files, concatenated by `prelude_source()` in `interp.rs`:

| File | Contents |
|---|---|
| `prelude.php` | Core interfaces (`Traversable`, `Iterator`, `ArrayAccess`, ...), `stdClass`, `Closure`, `Generator`, `Fiber`, `WeakMap`, `Promise`, exception hierarchy, `finfo`, ... |
| `prelude_spl.php` | SPL data structures and iterators, `ArrayObject`, `ArrayIterator` |
| `prelude_reflection.php` | The Reflection API, on `__phpx_refl_*` built-ins (`reflect.rs`) |
| `prelude_date.php` | `DateTime*`, `DateTimeZone`, `DateInterval`, `DatePeriod` shells |
| `prelude_splfs.php` | `SplFileInfo`, `DirectoryIterator` & co (hot methods native in `splfs.rs`) |
| `prelude_tokenizer.php` | `PhpToken` |
| `prelude_dom.php` | DOM class shells (`dom/`) |
| `prelude_redis.php` | `Redis` shell (`redis/`) |
| `prelude_pdo.php` | `PDO`, `PDOStatement`, constants, by-reference binders |
| `prelude_random.php` | `Random\*` engines and `Randomizer` |
| `prelude_intl.php` | ext/intl classes (generated by `tools/intl/prelude.php`; unimplemented methods throw "not supported by phpx yet") |

Two ways to implement behaviour:

1. **Prelude PHP calling hidden built-ins.** The method body is PHP; it
   calls `__phpx_*` functions registered in a Rust module (e.g.
   `ReflectionClass` → `__phpx_refl_class`). Internal state goes in
   `__phpx_`-prefixed properties (invisible to PHP code). Use for
   rarely-hot, logic-heavy classes.
2. **Native methods.** The prelude declares the class (and usually empty
   method bodies for Reflection/signatures); Rust provides
   `fn(&Interp, Option<&Rc<Obj>>, &Rc<Class>, Vec<Value>) -> R<Value>`
   (`NativeMethod`) registered under `(lowercase class, lowercase method)`
   — prefix the method with `"static "` for static methods — either in
   `native::table()` directly or in a module's `natives()` list chained
   there (`date`, `weak`, `pdo`, `fiber`, `dom`, `xmlwriter`, `redis`,
   `random`, `intl`, `fileinfo`, `splfs`, plus `generator::natives()`).
   `build_class` replaces the prelude's method with the native one. Native
   state goes in `Internal::Data(Box<dyn NativeData>)`. Named arguments to
   native methods are not supported yet (an `Error` says so).

Code in the prelude (file 0) does not update `Interp::pos`, so warnings
and exceptions point at the user's calling line, as for PHP's internal
classes. A deprecation from a prelude method uses `__phpx_deprecated(msg)`;
`#[\Deprecated]` on prelude constants is honoured (`const_deprecation`).

## Errors, exceptions and output

- **Warnings/notices/deprecations** (`errors.rs`): `Interp::warn`,
  `notice`, `deprecated` → `errors::raise(level, msg)`: records
  `error_get_last()`, calls the `set_error_handler` callback if its level
  mask matches, honours `@` (`Interp::silence`) and `error_reporting`,
  otherwise prints `PHP Warning:  msg in file on line N` to **stderr**
  (PHP CLI prints to stdout; tests run PHP with `display_errors=stderr`).
  If a handler throws, the exception is deferred (`errors::defer`) and
  surfaces at the next `errors::check` (built-in return, compiled helper
  status, statement end) — PHP's behaviour.
- **Exceptions**: `Interp::error(class, msg)` → `Ctrl::Throw`. Uncaught:
  `errors::uncaught` → `set_exception_handler` or
  `Interp::uncaught_message` ("PHP Fatal error:  Uncaught ...") and exit
  255. Fatal limits: `Interp::memory_fatal` ("Allowed memory size ...
  exhausted") and the `max_execution_time` message in `run_destructors`.
- **Output**: `Interp::echo` appends to the top of `Interp::out` (the
  `ob_start` stack; `ob.rs` keeps the handlers in its `HANDLERS`
  thread-local). Level 0 is written to the `writer` once it exceeds 64 KB
  and at `flush()`. `output_started()` feeds `headers_sent()`.
  `php://stdout` (`write_stdout`) bypasses buffers. Headers, response code
  and shutdown functions live in `Interp::response` (`misc::Response`).

## Tiering: native compiler and numeric JIT

Full details: [native-compiler.md](native-compiler.md). In short:

- `call_callable` → `try_jit` first when every argument is int/float/bool
  (`crates/jit`; deopts re-run the call in the interpreter).
- `invoke` → `native_entry_of` asks the native compiler
  (`compiler/mod.rs`, `Native`) for a compiled `Entry`; a function compiles
  after 1000 calls (`DEFAULT_THRESHOLD`, `PHPX_NATIVE_THRESHOLD`), loops
  move mid-call after `OSR_ITERATIONS` = 500 iterations (`try_osr`, called
  from the loop statements in `interp.rs`), top-level segments compile when
  they contain a loop (`native_top_level`).
- `compiler/analyze.rs` decides what compiles (reject reasons are strings,
  logged by `PHPX_NATIVE_LOG=1`); `lower.rs` emits Cranelift IR with inline
  int, refcount, packed-array and property fast paths; everything else
  calls `rt_*` helpers in `rt.rs` (registered in `HELPERS` in `mod.rs`),
  which call back into the same interpreter methods — that is what keeps
  output identical. Top-level file code can hand single statements or
  expressions back to the interpreter (`rt_exec_ast`, `rt_eval_ast`).
- Compiled code lives per thread (`THREAD_NATIVE`) and survives across a
  worker's requests; `for_new_interp()` only empties the inline caches
  (`CallSite`, `MethodSite`, `PropSite`, `ClassSite`), which point at the
  previous request's classes.
- The interpreter is the reference. If `PHPX_NATIVE=all` and
  `PHPX_NATIVE=off PHPX_NOJIT=1` disagree, the compiler is wrong.

## Memory: allocator, refcounts, cycle collector

- **Allocator** (`alloc.rs`): `#[global_allocator] Counting` wraps the
  system allocator with per-thread counters (`used`, `peak`) for
  `memory_get_usage()` and `memory_limit`, plus per-thread free lists for
  16 size classes up to 256 bytes (32 KB chunks). Crossing the limit sets
  a flag; the fatal error is raised at the next statement boundary
  (`take_over` in `run_destructors`). Parsed code is excluded
  (`alloc::exclude`). Built-ins that allocate a lot up front call
  `builtins::reserve`.
- **Refcounting**: `Rc` everywhere. When the last reference to an object
  with `__destruct` dies (`impl Drop for Obj`), it is pushed to
  `DESTRUCT_QUEUE` and `destruct_pending()` is raised; the queue drains at
  the next statement boundary (`run_destructors`) or right after a call on
  a temporary (`destruct_now`).
- **Cycle collector** (`gc.rs`): every object is in a registry (`State`,
  handle = index, free list reuses handles like PHP). `collect()` counts,
  per object, references held by other registered objects; objects with
  more strong references than that are externally reachable. Unreachable
  cycles get destructors first, then their properties are cleared on the
  next pass. Becomes due when the objects created since the last run pass an
  adaptive threshold (min 10,000), checked at statement boundaries via `take_due()`, or by `gc_collect_cycles()`.
  Shared containers are not traversed (conservative: may miss garbage,
  never frees live objects). `gc::new_request` restarts handles at 1 and
  frees what an earlier request's cycles left (`free_leftovers`).
- Known leaks and risks: [security-and-memory.md](security-and-memory.md).

## Per-process, per-thread and per-request state

phpx is single-threaded per script: everything PHP-visible is either in
`Interp` (dies with the request) or in a `thread_local!`. A server worker
thread runs many interpreters in sequence, so **every thread-local that
holds PHP-visible state must be reset between requests**.

| Scope | State | Reset by |
|---|---|---|
| Process | `INI_OPTS` (`lib.rs`), `server::SAPI`, `arginfo` specs, `consts` maps, `PROCESS_START`, `value::ALL_PENDING`, `timeout` watcher | never |
| Thread, kept across requests (caches keyed by immutable data) | `FILE_REG`, `PARSED`, `PRELUDE` (`interp.rs`), `BUILT` (`call.rs`), `RESOLVED` (`call.rs`, 2 s TTL), `THREAD_NATIVE` (`compiler/mod.rs`), `fs.rs` `REALPATH`, `preg.rs` `CACHE`, `reflect.rs` descriptors (except defaults), `strings.rs` `ONE_BYTE`, `props.rs` empty shape, `alloc.rs` free lists | n/a |
| Thread, per request | `IN_USE` + class constants/statics | `call::reset_classes()` (from `Interp::new`) |
| | GC registry, handles | `gc::new_request()` (from `Interp::new`) |
| | compiler inline caches | `compiler::for_new_interp()` (from `Interp::new`) |
| | `builtin_shadowed` flag | `Interp::new` |
| | error/exception handlers, last error (`errors`), libxml errors (`dom`), mbstring settings (`mbext`), reflection defaults (`reflect`), default RNG + LCG (`random`), intl default locale/error (`intl`), user stream wrappers (`wrappers`), user filters (`filters`), weak refs (`weak`), date last errors (`date`), session state (`session`), autoload extensions (`ext_more`) | `reset_request_state()` in `lib.rs` (from `Interp::new` and after each server request) |
| Request | everything in `Interp`, the main task's stack | dropped with the interpreter |

Thread-locals **not** in `reset_request_state` today, worth checking when
a server request shows another request's state: `json.rs` `LAST_ERROR`,
`preg.rs` `LAST_ERROR`, `openssl.rs` `ERRORS`, `iconv_conv.rs` `ENC`,
`ext.rs` `strtok` state, `ob.rs` `HANDLERS` (emptied by `ob::end_all` at
shutdown), `fiber.rs` `RUNNING`, `misc_more.rs` `PROCESS_TITLE`,
`process.rs` backtick-deprecation `SEEN`. When you add a thread-local,
either key it by immutable data or add a `reset()` and call it from
`reset_request_state`. The differential test
`matches_php_output_on_one_thread` runs every test file twice on one
thread to catch leaks of this kind.

## Extensions

| Extension | Files | Notes |
|---|---|---|
| Core / standard | `builtins.rs`, `core_more.rs`, `strings.rs`, `strings_more.rs`, `arrays.rs`, `arrays_more.rs`, `zsort.rs`, `math_more.rs`, `misc.rs`, `misc_more.rs`, `ext.rs`, `ext_more.rs`, `ops.rs` | `zsort.rs` is a port of `zend_sort.c` so inconsistent comparators order like PHP |
| date | `date/` (`mod.rs`, `parse.rs` strtotime formats, `fromformat.rs` timelib port, `format.rs`, `interval.rs`, `zone.rs`, `civil.rs`, `more.rs`, generated `zones_table.rs`, `abbr_map.rs`), `prelude_date.php` | |
| pcre | `preg.rs` | PCRE2 with JIT, byte subjects, compiled-pattern cache |
| json | `json.rs` | |
| mbstring | `strings.rs` (`mb_*` basics), `mbext.rs`, `mbcase.rs` + generated `mbcase_table.rs` | `mb_ereg` maps Oniguruma syntax onto PCRE2 |
| iconv | `iconv_conv.rs` | libc `iconv(3)` |
| hash, openssl (symmetric), random | `hash_algos.rs`, `openssl.rs`, `random.rs` + `prelude_random.php` | Seeded sequences identical to PHP |
| streams / fs | `streams.rs` (`fopen`, `php://`, `stat`, contexts), `stream_more.rs`, `fs.rs`, `glob.rs`, `wrappers.rs` (user wrappers), `filters.rs` (stream filters), `zlib_more.rs` (`gz*`, `compress.zlib://`), `splfs.rs` | File I/O blocks the OS thread (TODO: blocking pool) |
| sockets, processes | `sockets.rs` (`fsockopen`, `stream_socket_*`, TLS), `process.rs` (`proc_open`, `exec`, backticks), `sysinfo.rs` (DNS, services) | Non-blocking: park the green thread |
| PDO / MySQL | `pdo.rs` + `prelude_pdo.php`, `db/mod.rs` (shared result model, `DbError`), `db/mysql.rs` (MySQL wire protocol: auth incl. caching_sha2, text and binary prepared protocol), `db/sql.rs` (placeholder parsing) | Own wire protocol on runtime sockets; a query parks only its task |
| redis | `redis/mod.rs` (phpredis 6 API; a spec table per command), `redis/conn.rs` (RESP2 over TCP/TLS/Unix) | |
| dom / libxml / xml / xmlwriter | `dom/` (arena tree `tree.rs`, `html_parse.rs` + `tables.rs` port of libxml2's HTML parser, `xml_parse.rs`, `save.rs`, `xpath.rs`, `node.rs`, `document.rs`, `props.rs`), `xml_ext.rs` (SAX API), `xmlwriter.rs` | Output byte-identical to libxml2 2.13 |
| intl | `intl/` (`mod.rs`, `numfmt.rs` NumberFormatter, `grapheme.rs`, generated `locales_table.rs`), `prelude_intl.php` | Partial: see [php-compatibility.md](php-compatibility.md) |
| session | `session.rs` | Files and user save handlers; written at shutdown |
| fileinfo | `fileinfo.rs` | libmagic-compatible answers for common formats |
| tokenizer | `tokenizer.rs`, `prelude_tokenizer.php` | Own lossless scanner modelled on Zend's |
| Reflection | `reflect.rs`, `prelude_reflection.php` | |
| phpx-only | `native.rs` (`Promise`, `Concurrency`), `async()`/`defer()` in `builtins.rs` | [async.md](async.md) |

`builtins::EXTENSIONS` is what `extension_loaded()`, `get_loaded_extensions()`
and `phpx -m` report.

## Module reference: `crates/interp/src`

Execution core:

| File | Owns |
|---|---|
| `lib.rs` | Crate root; `run_file`, `run_file_args`, `run_source`, `run_source_args`, `run_main` (the request lifecycle above), `run_capture` (tests), `cli_globals`, `reset_request_state`, `set_ini_options`, `extensions`, stack size constants |
| `interp.rs` | `Interp`, `Frame`, `Vars`, `NsCtx`, `Pos`, `FileInfo`, `Ctrl`, `Flow`, `Callable`, `UserFn`, `ClosureData`, `BuiltinFn`, `NativeMethod`; `Interp::new`, prelude loading, file registry and parse cache (`parse_file`, `load_file`, `parsed_file`), `run_file_in`, `run_stmts_top`, `namespace_segments`, `hoist`, `declare_function`, `exec`/`exec_rest`/`exec_block`, loops (with OSR counters), `exec_try`, `exec_foreach`, shutdown (`run_shutdown`, `destruct_now`, `destruct_remaining`, `run_destructors`), `echo`/`flush`, `error`, `uncaught_message` |
| `expr.rs` | `eval`/`eval_rest`, places and writes (`write_path`, `make_ref`, `bind_ref`, `destructure`), property access and magic methods (`get_prop`, `call_magic`, `magic_isset`, guards), closures (`closure_object`, `arrow_captures`, closure naming), `eval_quiet_mode` (`isset`/`??`/`empty`) |
| `call.rs` | Name resolution (`resolve_class_name`, `find_function`, `find_class`, `lookup_class`), calls (`call_named`, `resolve_call`, `call_callable`, `call_value`, `invoke`, `invoke_method`, `bind_args`, `try_jit`, `method_call`, `static_invoke`, `call_method`), type checks (`check_param`, `coerce`, `type_matches`, `check_return_type`, `type_str`), classes (`declare_class`, `can_reuse`, `build_class`, `declare_anon_class`, `deps_ready`, `reset_classes`, `init_props`, `instantiate_begin`/`finish`), `include`, `by_ref_params` + `BY_REF` |
| `class.rs` | `Class`, `Method`, `MethodBody`, `PropDecl`, `PropDefault`, `ConstSlot`, `StaticProp`, `Vis` |
| `value.rs` | `Value`, `PhpArray`, `Key`, `Obj`, `Internal`, `NativeData`, `Resource`, comparisons, number parsing/formatting, `var_dump`/`print_r`/`var_export`, `DESTRUCT_QUEUE`, `destruct_pending`, `builtin_shadowed`, `Lower` (lowercase-name interner) |
| `pstr.rs` | `PStr` byte strings |
| `props.rs` | `Shape`, `Props`, `HIDDEN_PREFIX` |
| `ops.rs` | `binop`, conversions (`to_str`, `to_int`, `to_float`), array-offset rules |
| `callback.rs` | `Interp::callback` / `Callback::call` for repeated callbacks from built-ins |
| `generator.rs`, `fiber.rs` | Generators and Fibers as green-thread tasks |
| `trace.rs` | Call records, `debug_backtrace`, exception traces |
| `errors.rs` | Error levels, handlers, `raise`, `check`, `uncaught` |
| `ob.rs` | Output-buffer handlers and `ob_*` |
| `ini.rs` | `Ini`, `DEFAULTS`, php.ini lookup, `ini_apply`, `ini_*` built-ins |
| `alloc.rs`, `gc.rs` | Memory accounting/allocator, cycle collector |
| `timeout.rs` | `max_execution_time` watcher thread (CPU time) |
| `phpprof.rs` | `PHPX_PROFILE_PHP` sampling profiler of PHP stacks |
| `native.rs` | `native::table()`: native methods registry; `Closure`, enum helpers, `Promise`, `Concurrency` |
| `builtins.rs` | `table()`, `constants()`, `invoke`, `guarded`, `EXTENSIONS`, argument helpers, core built-ins |
| `arginfo.rs`, `arginfo_table.rs` | Built-in signatures (generated table) and their checks |
| `consts.rs`, `consts_table.rs` | Built-in constant values and deprecations (generated table) |
| `composer.rs` | Opt-in native Composer autoloader |
| `server.rs` | `phpx -S` |
| `reflect.rs` | Native side of Reflection |
| `tokenizer.rs` | ext/tokenizer |

Compiler (`compiler/`): `mod.rs` (driver, `Native`, `Target`, `Entry`,
`HELPERS`, thresholds, layout probes, OSR, `native_top_level`,
`run_native_entry`), `analyze.rs` (accept/reject, local slots), `lower.rs`
(AST → Cranelift IR), `rt.rs` (helpers, `CallSite`, `MethodSite`,
`PropSite`, `ClassSite`), `fastbi.rs` (built-in fast paths).

Library modules not listed above are described in the
[Extensions](#extensions) table; every file starts with a `//!` comment
saying what it implements and which PHP/C source it follows.

## Debugging switches

| Variable | Effect | Read in |
|---|---|---|
| `PHPX_NATIVE=off\|0\|all` | Disable the native compiler / compile everything on first call | `compiler/mod.rs` |
| `PHPX_NATIVE_THRESHOLD=n` | Calls before a function compiles (default 1000) | `compiler/mod.rs` |
| `PHPX_NATIVE_LOG=1` | Log compile / reject decisions and compile time | `compiler/mod.rs` |
| `PHPX_NATIVE_MAX=n`, `PHPX_NATIVE_SKIP=a,b` | Bisect miscompiles: compile at most `n` functions / skip names containing any of these | `compiler/mod.rs` |
| `PHPX_NATIVE_DUMP=name` | Print IR and machine code of functions whose name contains `name` | `compiler/mod.rs` |
| `PHPX_NATIVE_VERIFY=1` | Turn on Cranelift's IR verifier | `compiler/mod.rs` |
| `PHPX_PERF_MAP=1` | Write `/tmp/perf-PID.map` for `perf` symbolization | `compiler/mod.rs` |
| `PHPX_NOJIT=1` | Disable the numeric JIT | `interp.rs` |
| `PHPX_JIT_LOG=1` | Numeric JIT decisions | `crates/jit` |
| `PHPX_TRACE_THROW=1` (or `2`) | Log every exception thrown, with file:line (`2`: plus 12 frames) — finds swallowed exceptions | `interp.rs`, `expr.rs` |
| `PHPX_PROFILE=1`, `PHPX_PROFILE_FOLDED=path` | Native (Rust-level) sampling profile via pprof; folded stacks for flamegraphs | `crates/cli/src/main.rs` |
| `PHPX_PROFILE_PHP=path` | PHP-level sampling profiler (hottest PHP functions; folded stacks to `path`) | `phpprof.rs` |
| `PHPX_NATIVE_COMPOSER=1` | Use the built-in Composer autoloader | `call.rs` |
| `PHPX_SERVE_MAX_REQUESTS=n` | `-S` stops after `n` connections | `server.rs` |
| `PHP_CLI_SERVER_WORKERS=n` | `-S` worker threads (as in PHP) | `server.rs` |
| `PHPX_TEST=substr` | Run a subset of differential tests | `tests/differential.rs` |
| `PHPX_FUZZ_SEED`, `PHPX_FUZZ_ITERS` | Fuzz test seed/iterations | `tests/fuzz.rs` |

See also [performance.md](performance.md#profiling) for profiling recipes.

## Generated files and their generators

Never edit these by hand; regenerate them with the reference `php`.

| File | Generator | Source of truth |
|---|---|---|
| `crates/interp/src/arginfo_table.rs` | `php -n tools/arginfo/dump.php > ~/.cache/arginfo.json && tools/arginfo/gen.py ~/.cache/arginfo.json` | Reflection of every internal function |
| `crates/interp/src/consts_table.rs` | `php -n -d extension=intl tools/consts/gen.php > crates/interp/src/consts_table.rs` | `get_defined_constants(true)` for the implemented extensions |
| `crates/interp/src/date/zones_table.rs` | `php -n tools/date/zones.php > ...` | PHP's timezonedb index |
| `crates/interp/src/mbcase_table.rs` | `php -n -d extension=intl tools/mbstring/case.php > ...` | ICU Unicode properties |
| `crates/interp/src/intl/locales_table.rs` | `php tools/intl/numfmt.php > ...` | PHP's ICU locale data |
| `crates/interp/src/prelude_intl.php` | `php -n -d extension=intl tools/intl/prelude.php > ...` | ext/intl class signatures |
| `crates/interp/tests/php/*.out`, `.exit`, `PHP_VERSION` | `tools/difftest/regen.sh` | Running each test with real PHP |
| `docs/stdlib.md` coverage numbers | `tools/stdlib_coverage.py` | [stdlib.md](stdlib.md#regenerating-the-coverage-data) |

The lerd `php` wrapper on the development machine cannot see `/tmp`; keep
generator inputs and outputs under `$HOME` ([testing.md](testing.md#the-php-command-in-this-environment)).
