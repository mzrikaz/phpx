# Native compiler and numeric JIT

phpx has two Cranelift-based code generators (Cranelift 0.137):

| | Native compiler | Numeric JIT |
|---|---|---|
| Location | `crates/interp/src/compiler/` | `crates/jit/` |
| Scope | Whole functions, methods, closures, arrow fns and file top-level code, **any value types** | Functions whose locals are all int, float or bool |
| Semantics | Inline fast paths for ints; everything else calls helpers that reuse interpreter methods | Pure native arithmetic; deopts on anything PHP-specific |
| When it gives up | At compile time: function stays interpreted. At call time: "decline" → interpreter | "Deopt" → interpreter re-executes the call |
| Disable | `PHPX_NATIVE=off` | `PHPX_NOJIT=1` |
| Log | `PHPX_NATIVE_LOG=1` | `PHPX_JIT_LOG=1` |

Both are invisible to PHP code: anything they cannot handle runs in the
interpreter with identical output. A panic inside the native compiler is
caught and treated as "not compiled".

## Tiering

For each call of a user function (`call.rs`, `compiler/mod.rs`):

1. **Numeric JIT** is tried first when every argument is int, float or bool
   and the function has not been rejected by the JIT. It compiles a
   specialisation per argument-type signature after 50 calls (on the first
   call if the function contains a loop).
2. **Native compiler**: compiles the function once it is **hot**, after
   1000 calls (`PHPX_NATIVE_THRESHOLD=n` changes it). Compiling costs
   about a millisecond per function, so code that runs a few times (most of
   a framework's boot) is cheaper to interpret: a Laravel artisan command
   compiles ~120 functions instead of ~2000.
   `PHPX_NATIVE=all` compiles everything on the first call.
3. **On-stack replacement (OSR)**: a `while`, `do` or `for` loop that runs
   500 iterations in one interpreted call moves to native code mid-call.
   The compiler builds an entry for that loop: it takes the interpreter
   frame's locals (`rt_load_locals`), jumps straight to the loop header and
   runs the rest of the function natively; the interpreter returns its
   result. Not for loops inside `foreach`, `switch` or `try`, nor when a
   local is shared (`static`, `global`, references): those keep
   interpreting. Generators never OSR.
4. **Interpreter**: everything else, plus functions that were rejected.

File top-level code (each namespace segment) is compiled when it contains a
loop, or always with `PHPX_NATIVE=all`.

Compilation results are cached per function (by AST address) and per OSR
loop, including negative results, so a rejected function is never
re-analysed. Cranelift's IR verifier is off (it took three quarters of
compile time); `PHPX_NATIVE_VERIFY=1` turns it on.

| `PHPX_NATIVE` | Threshold |
|---|---|
| unset / any other value | 1000 calls (`PHPX_NATIVE_THRESHOLD`), loops via OSR |
| `all` | 1st call |
| `off` or `0` | never (interpreter + numeric JIT only) |

Planned: compiling on a background thread while the interpreter keeps
running, and an on-disk cache of compiled code so a deploy compiles once.

## What compiles

The analysis (`compiler/analyze.rs`) accepts a function unless it contains
one of the constructs below; those functions run in the interpreter.

| Construct | Reject reason (as logged by `PHPX_NATIVE_LOG`) |
|---|---|
| By-reference parameter in the function's own signature | `by-reference parameter` |
| `foreach (... as &$v)` | `foreach by reference` |
| `foreach` with a `list()`/`[...]` or other non-variable target | `complex foreach target` |
| `function () use (&$x)` | `closure captures by reference` |
| `$a = &$b` | `assignment by reference` |
| `[&$x]`, `list()` with `&` or spread | `array item by reference`, `list() by reference` |
| `static $x` | `static variable` |
| `global $x` | `global` |
| `$GLOBALS` | `$GLOBALS` |
| `$$x`, `${expr}` | `variable variable / $GLOBALS` |
| `compact`, `extract`, `eval`, `get_defined_vars`, `func_get_args`, `func_num_args`, `get_called_class` | `scope-reading function` |
| `include` / `require` inside the function | `include` |
| `yield`, `yield from` | `generator` |
| `goto` / labels | `goto` |
| `return`, or `break`/`continue` leaving the block, inside `finally` | `return / break out of finally` |
| `break $n` with a non-constant level | `dynamic break level` |
| `new $class`, `$class::x`, anonymous classes | `dynamic class reference`, `dynamic/anonymous new` |
| `Foo::$prop = …` | `static property write` |
| `f(...)` first-class callable syntax | `first-class callable` |
| `[1, , 2]` holes | `array hole` |
| `$a[]` in read context | `[] read` |
| Function/class/const declarations nested inside a function or a conditional block | `nested declaration` / `conditional declaration` |
| Parse-error nodes | `parse error node` |

Everything else compiles: arithmetic, string, comparison and bitwise ops;
concatenation (including in-place `.=`); string interpolation (single
allocation); array literals with spread; index and property reads/writes at
any depth; `isset`/`empty`/`??`/`unset`; all loops; `foreach` by value;
`switch`; `match`; `try`/`catch`/`finally`; `throw`; closures and arrow
functions (by-value captures); function, method, static and dynamic calls
(`$f()`, `$obj->$name()`, `$obj->$name`); named and spread arguments;
`new`, `clone`, `instanceof`; class constants and static property reads;
`exit`; `echo`/`print`; inline HTML. At top level, function and class
declarations, `const`, `use` and `declare` are also accepted.

Some calls are additionally **declined** at runtime (status 2) when the
native frame cannot represent them — e.g. by-ref/variadic/promoted
parameters or missing required arguments on the fast argument-binding path
(`rt_bind_args`). The interpreter then binds the arguments and the compiled
body still runs via `rt_load_locals` when possible.

## Code generation model (`lower.rs`)

- Every local and temporary is a 16-byte stack slot holding a `Value` in its
  native layout (tag byte at offset 0, payload at offset 8). Empty = `Null`.
- Expressions produce operands: unboxed int/float/bool in registers, a
  borrowed local or constant, or an owned temporary.
- **Int fast paths are inline**: arithmetic with overflow checks, comparisons,
  increments, truthiness. Anything else (floats mixed with strings, arrays,
  objects, …) calls an `rt_*` helper.
- **Refcounting is inline**: generated code increments/decrements the `Rc`
  strong count directly (layout verified at startup by `rc_layout_ok`;
  helper calls otherwise). Only the final release calls into Rust.
- **Exits**: every exit path (normal, exception unwind, top-level return)
  drops all locals, temporaries, foreach iterators and stashed exceptions.
- **try/finally**: `finally` bodies are inlined on each `return`, `break`
  and `continue` path; in-flight exceptions are stashed while `finally`
  runs; `exit` skips `finally`, as in PHP.
- **Destructors**: compiled code checks the destructor queue at each
  statement boundary (`rt_destruct`), matching the interpreter.
- **Error positions**: generated code updates the interpreter's current
  position so warnings and exceptions report the right file and line.
- **Top-level code**: locals are not copied — each slot points directly at
  the frame's variable cell (`rt_bind_slots`/`rt_unbind_slots`), so
  `global $x` and `$GLOBALS` in called functions see live values, and later
  segments, includes and catch blocks see the variables like in PHP.

## ABI

Compiled functions have one signature:

```rust
pub type Entry = unsafe extern "C" fn(cx: *mut Cx, args: *mut Value, nargs: u64, out: *mut Value) -> u64;
```

| Argument | Meaning |
|---|---|
| `cx` | `Cx { it: *const Interp, frame: *mut Frame }` |
| `args`, `nargs` | Argument array from a compiled caller (moved in). `args == null` means "parameters are already bound in the interpreter frame" |
| `out` | Return value slot |

| Return status | Meaning |
|---|---|
| 0 | OK, result in `out` |
| 1 | Exception; the exception is in `Interp::pending` |
| 2 | Declined before touching anything; run the interpreter instead |
| 3 | Top-level `return` (value in `out`) |

Helper conventions (`rt.rs`): every `*mut Value` slot always holds a valid
value; `dst` slots are empty on entry and receive the result; "consumed"
arguments are taken and left `Null`; helpers return 0 = ok, 1 = error.

When the parameters are plain (no types, by-ref, variadic or promotion) and
the argument count matches exactly, compiled callers move arguments straight
into the callee's parameter slots without any helper call.

## Helpers (`HELPERS` in `compiler/mod.rs`)

All parameters are `i64`. Grouped by purpose:

| Group | Helpers |
|---|---|
| Frame / locals | `rt_load_locals`, `rt_bind_args`, `rt_store_locals`, `rt_bind_slots`, `rt_unbind_slots` |
| Declarations | `rt_declare_class`, `rt_define_const` |
| Memory | `rt_release`, `rt_release_n`, `rt_copy` |
| Operators | `rt_binop`, `rt_compare`, `rt_truthy`, `rt_identical`, `rt_loose_eq`, `rt_unary`, `rt_concat_assign`, `rt_interp`, `rt_incdec`, `rt_cast` |
| Output | `rt_echo` |
| Reads | `rt_index_get`, `rt_quiet_index`, `rt_prop_get`, `rt_prop_read` (inline cache), `rt_quiet_prop`, `rt_prop_get_dyn`, `rt_const`, `rt_magic`, `rt_class_const`, `rt_static_prop_get`, `rt_instanceof` |
| Writes | `rt_place_assign`, `rt_place_update`, `rt_local_store`, `rt_unset_place` |
| Calls | `rt_call`, `rt_call_value`, `rt_method_call`, `rt_method_call_dyn`, `rt_static_call`, `rt_new`, `rt_make_closure`, `rt_clone` |
| Arrays / iteration | `rt_array_new`, `rt_array_add`, `rt_iter_new`, `rt_iter_next`, `rt_iter_free` |
| Control | `rt_throw`, `rt_destruct`, `rt_catch`, `rt_pending_save`, `rt_pending_restore`, `rt_stash_free`, `rt_exit`, `rt_unhandled_match` |

Adding a helper: write `pub unsafe extern "C" fn rt_x(...) -> u64` in
`rt.rs` following the conventions above, add `rt_x: N => true/false` to
`HELPERS`, and call it from `lower.rs` with `l.call("rt_x", &[...])`.

## Inline caches

| Cache | Site | What it remembers |
|---|---|---|
| `CallSite` | `foo(...)` | The resolved target: either a plain built-in function pointer (no by-ref params, no caller-scope access) or a compiled user function `(Entry, *const UserFn)`. Functions cannot be redeclared, so the cache never needs invalidation. Not cached while the numeric JIT may still take the function (all-numeric arguments). |
| `MethodSite` | `$obj->m(...)` | Monomorphic: class, calling scope that passed the visibility check, compiled entry, method and declaring class |
| `PropSite` | `$obj->name` read | Per (class, scope): that the property is visible and its index in the object's property table |

Calls between compiled functions currently go through `rt_call` /
`rt_method_call`, which use the cached entry to call the target directly
with a fresh frame (no name lookup, no interpreter dispatch). Emitting a
direct machine-level call from one compiled function to another is a
planned optimisation ([performance.md](performance.md)).

## Numeric JIT

`crates/jit` compiles functions that stay within a pure numeric subset:

- Locals are int, float or bool, and each local has **one type** for the
  whole function (`infer.rs`); reads must be of definitely-assigned
  variables.
- Arithmetic, comparisons, control flow, calls to other numeric functions,
  and a few inline built-ins: `abs`, two-argument `min`/`max`, `intdiv`,
  `sqrt`, `floor`, `ceil`.
- Specialised per argument-type tuple (key = function address + packed
  types, up to 28 parameters). An int passed to a `float` parameter is
  widened first.
- **Deopt**: integer overflow, division by zero, deep recursion, or
  anything else PHP would handle specially makes compiled code return
  "deopt"; the interpreter then re-runs the call. This is safe because JIT
  code has no side effects besides its locals. After 20 deopts a
  specialisation is abandoned.

## Debugging

```sh
PHPX_NATIVE_LOG=1 phpx run app.php      # "[native] foo compiled" / "[native] bar: not compiled: global", total compile CPU at exit
PHPX_JIT_LOG=1 phpx run app.php         # numeric JIT decisions
PHPX_NATIVE=all phpx run app.php        # compile everything eagerly (good for finding compiler bugs)
PHPX_NATIVE=off PHPX_NOJIT=1 phpx run app.php   # pure interpreter (semantic reference)
```

If outputs differ between these modes, the compiler is wrong: the
interpreter is the reference. The differential test suite runs in all three
modes ([testing.md](testing.md)).
