# Architecture

phpx is a Cargo workspace of five crates. A PHP file goes through the
parser, then the interpreter, which hands hot functions to the native
compiler or the numeric JIT. Everything runs inside a green-thread runtime.

```
             source .php
                  │
          crates/syntax ── lexer → parser → AST (+ diagnostics, `phpx check`)
                  │
          crates/interp ── Interp (tree-walker, semantic reference)
             │        │         │
             │        │         └─ stdlib: builtins/strings/arrays/json/preg/fs/streams,
             │        │            prelude.php (built-in classes), native.rs (native methods),
             │        │            composer.rs (opt-in native Composer autoload)
             │        │
             │        ├─ compiler/ (native compiler, Cranelift): whole functions,
             │        │   any value types, helpers in rt.rs, falls back to the interpreter
             │        │
             │        └─ crates/jit (numeric JIT, Cranelift): int/float/bool functions,
             │            type-specialised per argument types, deopt = re-execute
             │
          crates/runtime ── green threads (corosensei) + epoll reactor (polling):
                            block_on, spawn, Promise, sleep, defer, TCP
                  │
          crates/cli ── `phpx run | check | ast`
```

## Crates

| Crate | Package | Role | Key files |
|---|---|---|---|
| `crates/syntax` | `phpx-syntax` | Lexer, Pratt parser, AST, diagnostics | `lexer.rs`, `parser.rs`, `ast.rs`, `checks.rs` (tree-level rules), `diag.rs` (rustc-style rendering, codes) |
| `crates/runtime` | `phpx-runtime` | Single-threaded green-thread scheduler, timers, epoll reactor, promises, non-blocking TCP | `scheduler.rs`, `promise.rs`, `net.rs`, `lib.rs` (`block_on`, `sleep`, `concurrency::run`) |
| `crates/interp` | `phpx-interp` | Interpreter, values, stdlib, native compiler | see below |
| `crates/jit` | `phpx-jit` | Numeric JIT | `lib.rs` (codegen, specialisation cache, deopt), `infer.rs` (type inference) |
| `crates/cli` | `phpx-cli` (binary `phpx`) | Command line | `main.rs` (`run`, `check`, `ast`, profiler report), `optrees.rs` (parser oracle support) |

### `crates/interp/src`

| File | Contents |
|---|---|
| `lib.rs` | Entry points (`run_file_args`, `run_source`, `run_capture`), CLI superglobals (`$_SERVER`, `$argv`, `$argc`, …), stack sizes (8 MB per task, 256 MB for the main task, committed lazily) |
| `interp.rs` | `Interp` state, file execution, namespace segments, hoisting, includes |
| `expr.rs` | Expression evaluation |
| `call.rs` | Function/method calls, argument binding, named args, spread, by-ref binding, strict types, numeric JIT entry (`try_jit`) |
| `class.rs` | Class model (properties, methods, constants, traits, enums) |
| `value.rs` | `Value`, `PhpArray`, `Obj`, `Resource`, keys, destructor queue |
| `ops.rs` | Operators, comparisons, conversions |
| `builtins.rs` | Core / standard / math / output buffering / class introspection builtins, constants, scope-reading functions |
| `strings.rs` | String, ctype, mbstring, iconv, URL, HTML builtins, `sprintf` family |
| `arrays.rs` | Array builtins and sorting |
| `json.rs` | `json_encode` / `json_decode` / `json_validate` |
| `preg.rs` | PCRE functions on top of `fancy-regex` |
| `fs.rs` | Path and whole-file filesystem functions |
| `streams.rs` | `fopen` family, `php://` streams, directory handles, `stat` family, stream contexts |
| `generator.rs` | Generators, each running as its own green-thread task |
| `composer.rs` | Native Composer autoloader (classmap, PSR-4, PSR-0) triggered by `require 'vendor/autoload.php'` |
| `native.rs` | Native methods of built-in classes: `Closure`, enums (`cases`/`from`/`tryFrom`), `Promise`, `Concurrency`, `Generator` |
| `prelude.php` | Built-in classes and interfaces written in PHP (exception hierarchy, `ArrayIterator`, core interfaces) |
| `compiler/mod.rs` | Native compiler driver: `HELPERS` table, tiering, `compile_mode`, top-level compilation |
| `compiler/analyze.rs` | Decides what can compile; assigns local slots |
| `compiler/lower.rs` | AST → Cranelift IR |
| `compiler/rt.rs` | Runtime helpers called from native code, inline caches (`CallSite`, `MethodSite`, `PropSite`) |

## Execution pipeline

1. **Parse.** `phpx_syntax::check(src)` returns the AST plus diagnostics.
   Any error (syntax `E0xxx` or unsupported feature `E1xxx`) stops the run
   with exit code 255 and a rustc-style message. Source bytes are decoded
   lossily as UTF-8.
2. **Segment and hoist.** A file is split into namespace segments; functions
   and classes in each segment are hoisted before execution, as in PHP.
3. **Run top-level code.** Each segment is offered to the native compiler
   (`native_top_level`); top-level code compiles when it contains a loop
   (or always with `PHPX_NATIVE=all`). Otherwise the interpreter runs it.
4. **Call functions.** On each user-function call:
   - the numeric JIT is tried first when all arguments are int/float/bool;
   - otherwise the native compiler compiles the function on its 2nd call
     (1st if it contains a loop; 1st for everything with `PHPX_NATIVE=all`);
   - anything rejected or declined runs in the interpreter.
5. **Shutdown.** Pending destructors run, deferred closures (`defer`) run,
   and the runtime waits for every un-awaited task before exiting.

All three execution engines share the same `Value` representation and the
same interpreter methods for anything non-trivial, which is what keeps their
output identical.

## Values

`Value` (`value.rs`) is `#[repr(C, u8)]`, 16 bytes: a one-byte tag and an
8-byte payload at offset 8. Native code relies on this layout.

| Tag | Variant | Payload |
|---:|---|---|
| 0 | `Null` | — |
| 1 | `Bool` | `bool` |
| 2 | `Int` | `i64` |
| 3 | `Float` | `f64` |
| 4 | `Str` | `PStr` (refcounted byte string; PHP strings are bytes, not UTF-8) |
| 5 | `Array` | `Rc<PhpArray>` (copy-on-write) |
| 6 | `Object` | `Rc<Obj>` |
| 7 | `Ref` | `Slot` — a reference stored inside an array element or property (`$a[0] = &$x`, `foreach ($a as &$v)`); never stored directly in a variable, dereferenced on read |
| 8 | `Resource` | `Rc<Resource>` (streams, directory handles, stream contexts) |

- **`PhpArray`** is a hybrid like PHP's: a packed `Vec<Value>` for lists
  with keys `0..n`, converted on demand to an insertion-ordered hash
  (`FxIndexMap`). `next_free` tracks the next integer key.
- **`Obj`** holds its class and an `FxIndexMap` of properties (declared
  order preserved, as `var_dump`/`foreach` require).
- **Destructors.** When the last reference to an object dies its
  `__destruct` is queued, and the queue is drained at the next statement
  boundary (and at shutdown). See
  [php-compatibility.md](php-compatibility.md#semantic-gaps).
- The native compiler inlines `Rc` strong-count updates; it verifies the
  `RcInner` layout at startup and falls back to helper calls if it ever
  differs.

## Runtime

- **Scheduler** (`runtime/src/scheduler.rs`): single-threaded; every task is
  a stackful `corosensei` coroutine with lazily committed stack. `park()` /
  `wake()` are the only primitives; timers live in a binary heap.
- **Reactor**: `polling` (epoll on Linux) for non-blocking sockets.
- **`block_on(main)`** runs the script as the root task, then starts
  deferred jobs, then drives the scheduler until every task has finished.
- An M:N scheduler across cores is planned; the public API (`spawn`, `park`,
  `wake`) is designed to stay the same.

See [async.md](async.md) for the PHP-level API.

## Generators

Each generator body runs as its own green-thread task. `current()`,
`next()`, `send()`, `throw()` hand the task a resume value and park the
caller until the generator yields. Because a generator is an ordinary task,
it can sleep or do I/O between yields.

## Composer

`require .../vendor/autoload.php` is intercepted: phpx loads Composer's
generated maps (`autoload_classmap.php`, `autoload_psr4.php`,
`autoload_namespaces.php`, files) directly and registers a built-in
autoloader, instead of executing Composer's bootstrap code.
