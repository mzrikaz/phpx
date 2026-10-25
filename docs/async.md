# Async and concurrency

phpx code reads as ordinary blocking PHP. Under the hood every request (and
every script run) is a **green thread** on a single-threaded scheduler, and
anything that waits parks the green thread instead of the OS thread.
Concurrency *inside* a request is opt-in, through a Laravel-style API.

Nothing here changes the meaning of existing PHP code: a script that never
calls these APIs behaves exactly as under PHP.

## Model

| Concept | Implementation |
|---|---|
| Task (green thread) | Stackful coroutine (`corosensei`), lazily committed stack: 8 MB virtual per task for interpreted code, 256 MB for the main task |
| Scheduler | `crates/runtime/src/scheduler.rs`: single OS thread, run queue + timer heap + epoll reactor (`polling`) |
| Waiting | `park()` / `wake()`. `sleep()`, `usleep()`, `Promise::wait()` and runtime socket I/O park the task |
| Script lifetime | `block_on`: run the script as the root task, then run deferred closures, then keep driving the scheduler until **every** task (including un-awaited promises) has finished |
| Generators | Each generator body is also a task, so it can sleep or wait between yields |
| Parallelism | None yet: one core. An M:N work-stealing scheduler is planned with the same API |

## PHP API

IDE/PHPStan stubs live in `stdlib/stubs/` (`functions.php`,
`Promise.php`, `Concurrency.php`). Implementation: `async`/`defer` in
`crates/interp/src/builtins.rs`, classes in `native.rs`, exceptions in
`prelude.php`.

### Functions

| Function | Behaviour |
|---|---|
| `async(callable $fn): Promise` | Starts `$fn` in a new green thread immediately and returns its `Promise` |
| `defer(callable $fn): void` | Runs `$fn` after the main script finishes ("after the response is sent"). An exception thrown by a deferred closure is printed to stderr as an uncaught exception |
| `sleep(int $s)`, `usleep(int $us)` | Park the current task only; other tasks keep running |

### `Promise`

| Method | Behaviour |
|---|---|
| `wait(): mixed` | Park until settled; return the value or rethrow the exception |
| `then(callable $onFulfilled): Promise` | Chain on success |
| `catch(callable $onRejected): Promise` | Recover from an exception |
| `finally(callable $onSettled): Promise` | Run on either outcome, keep the original result |
| `timeout(float $seconds): Promise` | Reject with `TimeoutException` ("Promise timed out") after `$seconds` and cancel the work |
| `cancel(): void` | Stop the underlying task at its next wait point (`CancelledException`) |
| `isSettled(): bool` | Non-blocking state check |
| `Promise::all(array $ps): Promise` | All values, keys preserved; rejects with the first failure and cancels the rest |
| `Promise::allSettled(array $ps): Promise` | Never rejects; each entry is `['state' => 'fulfilled', 'value' => …]` or `['state' => 'rejected', 'reason' => Throwable]` |
| `Promise::any(array $ps): Promise` | First fulfilment wins; if all fail, rejects with `AggregateException` (`getErrors()`) |
| `Promise::race(array $ps): Promise` | First to settle wins, success or failure |

`Promise` cannot be constructed directly; promises come from `async()` and
the combinators.

### `Concurrency` (same shape as Laravel's facade)

| Method | Behaviour |
|---|---|
| `Concurrency::run(array $tasks): array` | Run closures concurrently, wait for all, return results with the same keys. The first exception cancels the others and is rethrown |
| `Concurrency::defer(array $tasks): void` | `defer()` for each closure |

Unlike Laravel's process/fork drivers, tasks run as green threads in the same
process: no forking, no closure serialization, shared memory.

### Exceptions

`TimeoutException`, `CancelledException` and `AggregateException` (with
`public array $errors` and `getErrors()`) all extend `RuntimeException`.

## Example

```php
<?php
function fetch(string $what, int $ms) { usleep($ms * 1000); return strtoupper($what); }

$t = hrtime(true);
[$users, $posts] = Concurrency::run([
    fn() => fetch('users', 100),
    fn() => fetch('posts', 100),
]);
printf("%s %s in ~%d ms\n", $users, $posts, (hrtime(true) - $t) / 1e6); // ~100 ms, not 200

$p = async(fn() => 21)->then(fn($x) => $x * 2);
echo $p->wait(), "\n";                                   // 42

try {
    async(function () { usleep(200_000); return 1; })->timeout(0.05)->wait();
} catch (TimeoutException $e) {
    echo "timed out\n";
}

defer(fn() => print("runs last\n"));
```

The differential test `crates/interp/tests/php/08_async.php` shows how to
write async code that also runs on stock PHP (with a sequential polyfill),
so its output can be compared with real PHP.

## Current limitations

| Limitation | Notes |
|---|---|
| Only timers park today at PHP level | PHP file and stream functions (`fopen`, `file_get_contents`, …) use blocking OS calls. Non-blocking TCP exists in the Rust runtime (`runtime/src/net.rs`) but is not yet exposed to PHP (no `fsockopen`/`stream_socket_client`, curl or database drivers yet) |
| Single core | M:N scheduler is on the roadmap |
| `DB::query`, `Http::get`, `Http::pool` | Appear in the Rust demo (`crates/runtime/examples/http.rs`) as simulated latency only; they are **not** PHP APIs yet. Async-capable mysqli/PDO and an HTTP client are roadmap items 2 and 6 |
| No HTTP server mode | `phpx serve` (per-request isolation, headers, cookies, sessions) is roadmap item 3; the runtime example server is a Rust-level demo |

## Rust-level runtime API (`phpx-runtime`)

For engine work: `block_on`, `spawn` → `Promise<T>` (`wait`,
`wait_timeout`, `then`, `catch`, `finally`, `timeout`, `cancel`,
`is_settled`, `all`, `all_settled`, `any`, `race`), `sleep`, `defer`,
`yield_now`, `current_task`, `stack_remaining`, `concurrency::run`,
`net::{TcpListener, TcpStream}`, and `raw::{park, wake, spawn_task,
cancel}` for runtime-level constructs such as generators.

Demos:

```sh
cargo run --release -p phpx-runtime --example http      # HTTP server on :8080 (/, /slow, /profile)
wrk -t4 -c1000 -d5s http://127.0.0.1:8080/slow
cargo run --release -p phpx-runtime --example profile   # 1000 concurrent simulated requests
```
