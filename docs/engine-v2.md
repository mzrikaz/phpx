# Engine v2: PHP at Node.js speed

Goal: general PHP code (not one framework) running at the speed of Node.js
(V8), with Node-level concurrency. phpx's concurrency model (green threads,
`async()`, promises, a reactor for sockets; see [async.md](async.md)) is
already there; what is missing is raw execution speed.

## Where phpx stands (2026-10-28)

Measured on a Laravel API request, steady state, user-space instructions
per request (median of 15 single requests, spread under 1%):

| Engine | Instructions/request |
|---|---|
| PHP 8.4 + opcache | 160 M |
| phpx, everything compiled | 443 M (2.8x) |

The gap has no hotspot. The largest single item is the allocator at ~6%;
the rest is spread over hundreds of functions at 0.5–2% each: argument
binding, call setup, built-in call glue, value copies and drops, name
lookups, hash probes. Spot fixes give about 1% each. Tiering doesn't fix it
either: interpreting everything costs 142 ms per request in-process,
compiling everything 60 ms, the default mix 66 ms, PHP 25 ms. Both tiers
sit on the same expensive runtime.

The cost comes from the shape of the core:

- **Variables** are `Rc<RefCell<Value>>` cells looked up by name in the
  interpreter; compiled code has slots but must convert to and from cells
  whenever it meets interpreter frames, references, `global`, closures.
- **Calls** build a `Frame` (six reference-counted fields, a variable
  table, an argument vector), push a trace record through thread-local
  storage, bind arguments one by one, and tear all of it down again.
- **The interpreter** walks the syntax tree: about 37 ns per loop
  iteration that PHP's VM runs in 2 ns.
- **Objects** are two or more allocations (object, property vector, native
  state box); closures four.
- **Machine code** is about 6 KB per PHP function, so code that runs once
  per request is not faster compiled than interpreted.

## Design

The model is the one PHP's VM and V8 share: one frame layout for every
tier, numbered slots for variables, per-instruction caches, a compact
default tier, and an optimizing tier with guards and deoptimization.

### 1. One frame layout (slots, VM stack)

- Every function gets a fixed slot layout at compile time: parameters,
  then named locals (PHP's CVs), then temporaries. The analysis already
  exists for the native compiler (`compiler::analyze::Locals`); the interpreter
  uses the same one.
- Frames live on a per-task VM stack: a header (function, `$this`, class,
  static class, caller position, argument count) followed by the slots.
  A call claims the next block; the caller writes arguments straight into
  the callee's parameter slots. No per-call heap allocation.
- References stay `Value::Ref` in a slot, as compiled code already does.
- Dynamic scope (`$$name`, `extract()`, `compact()`, `get_defined_vars()`,
  `include` inside a function, `global`) attaches a name table to the
  frame on first use, mapping names to slots plus extra dynamic variables,
  like PHP's symbol table attached to CVs.
- Backtraces read the frame chain; nothing is pushed to thread-local
  storage per call.

### 2. Bytecode tier (replaces the tree-walking interpreter)

- A register bytecode over the slot layout: operands are slot numbers,
  constants are resolved at compile time (names lowercased, literals and
  constant arrays built once), jumps are offsets.
- Compiled once per function per worker, kept across requests (like
  opcache).
- One dispatch loop small enough to stay in the instruction cache.
- Each instruction that looks something up (call, method call, property
  access, constant, static property) owns an inline-cache entry in a side
  table (V8's feedback vector). Entries hold strong references to what
  they are keyed on, so they can outlive a request.
- Built-in calls bind straight from slots, with arginfo checked once per
  call site where the argument types are known.

### 3. Optimizing tier

- Hot functions are compiled from bytecode plus the recorded feedback:
  types seen at each instruction, call targets, object shapes.
- Values whose type is known are kept unboxed (ints, floats, bools in
  registers); property accesses on a known shape become loads at fixed
  offsets; small monomorphic callees are inlined.
- Guards check each assumption; a failed guard deoptimizes: the frame is
  rebuilt in the shared layout and execution continues in bytecode at the
  same point. The common frame layout is what makes this cheap.
- Machine code is emitted only for code that is hot, so its size stops
  mattering.

First version, before the bytecode tier exists (2026-10-28): speculation
inside the native compiler, per loop, with fallback per statement.

- **Why speculation.** Declared types alone don't cover real hot loops:
  in n-body `$b = $bs[$i]` comes from an untyped array, so `$b->x` has no
  static type; in spectral-norm `$v[$j]` is an untyped element; int
  arithmetic isn't closed (overflow gives a float).
- **Observe.** A loop compiled for the first time runs as generic code
  that records, per local and per property/element read, the types it
  saw (and the shape behind each property read).
- **Specialise.** When the loop is recompiled, it gets a specialised body:
  locals of one observed type live unboxed in registers, property reads
  on the observed shape are direct loads, float/int arithmetic runs in
  registers, and each object's shape is checked once per iteration.
- **Fall back per statement.** In the specialised body every check of a
  statement comes before its side effect (the final store, a call, an
  echo). A failed check writes the unboxed locals back to their slots and
  jumps to the generic code for that same statement, which runs it again
  from the start; nothing has happened yet, so that is safe. Statements
  that can't be ordered that way stay generic.
- **Widen.** A loop whose checks keep failing is recompiled with the wider
  types (or left generic).

### 4. Values, objects, arrays

- Objects in one allocation: header, class, inline property slots
  (dynamic properties in an overflow table), native state only for
  built-in classes.
- Closures in one allocation with their captured values inline.
- Strings carry their hash once computed (like `zend_string`); property
  and method names are interned, so lookups compare pointers first.
- Arrays keep the packed/hash split; hash lookups use cached string hashes.

## Phases

Each phase lands in small commits. Every commit keeps the differential
tests passing in all three modes (`PHPX_NATIVE=0`, default,
`PHPX_NATIVE_THRESHOLD=1`) and is measured with the instruction-count
yardstick and the cross-language suite (`tools/bench/xlang`).

| Phase | What | Done when |
|---|---|---|
| A | Cross-language benchmark suite (PHP, phpx, Node) and the per-request instruction yardstick | Baseline recorded below |
| B | One frame layout: slot-indexed locals in the interpreter, frames on the VM stack, arguments written in place, trace from the frame chain | Interpreter calls ≥5x faster; no `Rc<RefCell>` per variable |
| C | Bytecode tier with per-instruction caches, replacing the tree walker | Interpreted code within 2x of PHP's VM |
| D | Optimizing tier from bytecode feedback, with deoptimization | Numeric and OOP benchmarks within 2x of Node |
| E | One-allocation objects and closures, hashed strings, interned names | Allocation under 3% of a framework request |

## Baseline

`tools/bench/xlang/run.sh php8.4` (ms, one run, 2026-10-28; PHP 8.4 with
opcache, JIT off; Node 22). Short runs (under ~100 ms) include phpx's
compile warm-up.

| Benchmark | PHP | phpx | Node | phpx/PHP | phpx/Node |
|---|---|---|---|---|---|
| assoc | 33 | 47 | 34 | 1.42 | 1.4 |
| binary-trees | 1466 | 3803 | 142 | 2.59 | 27 |
| closures | 24 | 55 | 8 | 2.29 | 6.9 |
| fannkuch-redux | 1186 | 698 | 176 | 0.59 | 4.0 |
| json | 47 | 77 | 45 | 1.64 | 1.7 |
| n-body | 1538 | 2822 | 64 | 1.83 | 44 |
| oop | 54 | 120 | 11 | 2.22 | 11 |
| spectral-norm | 183 | 307 | 13 | 1.68 | 24 |
| strings | 41 | 52 | 33 | 1.27 | 1.6 |

Before this suite existed n-body took 15.7 s in phpx (no inline float
arithmetic, compound assignment to properties through the generic place
path), spectral-norm 1.5 s (short loops never reached on-stack
replacement) and oop 213 ms (one-entry caches swapping on code shared by
several classes).

## Progress (2026-10-29)

Same suite, after speculation (phase D, first version) and the call-path
work of phase E. The machine was faster than for the baseline, so compare
ratios:

| Benchmark | phpx/PHP before | phpx/PHP now | phpx/Node now |
|---|---|---|---|
| assoc | 1.42 | 1.60 | 1.5 |
| binary-trees | 2.59 | 1.1–1.3 | 10 |
| closures | 2.29 | 1.11 | 3.9 |
| fannkuch-redux | 0.59 | 0.59 | 3.9 |
| json | 1.64 | 1.54 | 1.7 |
| n-body | 1.83 | 0.58 | 15 |
| oop | 2.22 | 1.45 | 4.8 |
| spectral-norm | 1.68 | 0.18 | 2.7 |
| strings | 1.27 | 1.15 | 1.4 |

What changed for calls and objects:

- Compiled prologues bind without the general helper for class-typed
  parameters (the object's class compared with the two classes last
  accepted), scalar literal defaults, promoted constructor parameters
  (`$this->p = $p` statements ahead of the body, as PHP compiles them)
  and closures' captured values.
- Return types are checked inline the same way.
- A call site keeps a frame per call depth, so recursion calls compiled
  code directly; frames carry an in-use flag because fibers, generators
  and green threads release them out of call order.
- The call-stack record is linked inline (compiled code belongs to one
  thread, so it writes the thread's task word directly).
- Typed property writes stay inline for null into nullable types, objects
  of the class last stored, and uninitialized slots (constructors).
- The cycle collector backs off after runs that find no garbage (it scans
  the whole heap; for binary-trees it was 42% of the time) and indexes
  objects by handle instead of a hash map.
- Callbacks from built-ins set their frame up once per callable.
- `parent::m()`/`self::m()` on instance methods call directly (the
  caller's `$this` lent), and method sites keep a second direct entry for
  a second class (interface-typed code).
- `static` and `void` return types check inline; `<=>` on ints and
  `=== null` are inline.

Per-construct cost now (instructions, compiled code, vs PHP 8.4 with
opcache): a method call is PHP's plus 45–65, a property read 69 vs 52,
`new` of a 3-property class 667 vs 423. Timings on this machine vary
by up to 30% between runs; instruction counts (callgrind) are the
reliable measure.

## Progress (2026-10-30)

Same suite, one run (ms timings are noisy; ratios matter):

| Benchmark | phpx/PHP 10-29 | phpx/PHP now | phpx/Node now |
|---|---|---|---|
| assoc | 1.60 | 1.20 | 1.4 |
| binary-trees | 1.1–1.3 | 1.17 | 10 |
| closures | 1.11 | 1.14 | 4.0 |
| fannkuch-redux | 0.59 | 0.58 | 4.3 |
| json | 1.54 | 0.92 | 1.1 |
| n-body | 0.58 | 0.58 | 15 |
| oop | 1.45 | 1.36 | 6.5 |
| spectral-norm | 0.18 | 0.19 | 2.4 |
| strings | 1.15 | 1.05 | 1.2 |

What changed:

- Trivial getters and setters (`return $this->p;`, a literal,
  `$this->p = $x;` optionally followed by `return $this;`) are answered at
  the call site: the site caches, for up to two classes, the slot, shape
  and the value tags that pass the method's types unchanged, and
  generated code reads or writes the slot instead of calling. A getter
  call costs 75 instructions per loop iteration instead of 228.
- A local method receiver is read in place when the arguments run no PHP
  code (no temporary copy and release per call).
- `===` falls back to the plain identity check, not the general
  comparison.
- `$a[k1][k2] = v` walks the levels in place.
- Integer-only arrays sort with a direct comparison.
- json_encode/json_decode work without intermediate copies (decode builds
  values while parsing). json_decode's depth limit and object creation
  order now match PHP.
- String interpolation of strings and integers allocates once at the
  exact size; integers format two digits at a time.

Still open: `new` (two allocations per object, registry bookkeeping:
~430 instructions vs PHP's ~300 for a 3-property class), ~165
instructions of generated code per non-accessor method call, inlining
of non-trivial small methods.

### Later on 2026-10-30

Suite (this machine ran about half as fast as in the morning; ratios):

| Benchmark | phpx/PHP | phpx/Node |
|---|---|---|
| assoc | 1.2–1.4 | 1.0 |
| binary-trees | 1.07 | 8.9 |
| closures | 1.06 | 2.8 |
| fannkuch-redux | 0.59 | 3.9 |
| json | 0.87 | 1.0 |
| n-body | 0.29 | 4.8 |
| oop | 0.72–0.93 | 2.8–4.8 |
| spectral-norm | 0.20 | 1.9 |
| strings | 1.1–1.2 | 1.2 |

- Leaf methods (`compiler/leaf.rs`): a method whose body is `return
  <expr>;` over int/float/bool parameters, typed properties of `$this`
  and literals is compiled a second time as a frameless function; call
  sites cache it per class (with that class's slots) beside accessors
  and call it directly. It declines (the real call follows) on anything
  it can't compute exactly. `area()` in a loop: 88 instructions per
  iteration instead of 365.
- `$obj->p op= v` on int/float properties updates the slot in place
  (write cache), and `$this` is no longer copied for property writes.
- Typed property write sites keep a second shape (base-class
  constructors run for two subclasses).
- Speculated loops: float property updates in place, and each object
  local is checked once per run of speculated statements (shape, slot
  storage, scope), so a property access costs ~13 instructions.
- A function whose short loop is hot across many calls compiles once on
  its next call instead of first compiling a loop entry.

Known difference left: PHP sets a typed property to 0 when `op=` on it
throws (`$o->f /= 0`); phpx leaves the old value.
