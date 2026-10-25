//! phpx runtime: green threads + Laravel-style promises.
//!
//! Code inside a task reads as plain blocking code. Every wait parks the task,
//! never the OS thread. Concurrency inside one request is opt-in via `spawn`,
//! `concurrency::run` and the `Promise` combinators.

mod error;
pub mod net;
pub mod tls;
mod promise;
mod scheduler;

pub use error::{Error, Result};
pub use promise::{spawn, Promise};
pub use scheduler::{current_task, defer, set_stack_size, set_task_word, stack_remaining, stack_size, task_word, yield_now, TaskId};

use std::time::{Duration, Instant};

use promise::TimerGuard;

/// Run `main` as the root task (one "request") and drive the runtime until
/// every task is done, including deferred ones and un-awaited promises.
///
/// Deferred closures (`defer`) start after `main` returns, i.e. after the
/// response would be sent.
pub fn block_on<T, F>(main: F) -> Result<T>
where
    T: Clone + 'static,
    F: FnOnce() -> Result<T> + 'static,
{
    scheduler::assert_idle();
    let root = spawn(move || {
        let r = main();
        for job in scheduler::take_deferred() {
            spawn(move || {
                job();
                Ok(())
            });
        }
        r
    });
    scheduler::run();
    root.settled_result().expect("run() returned before the root task settled")
}

/// Low-level task control for runtime-level constructs built on tasks
/// (e.g. generators in the interpreter). Prefer `spawn`/`Promise` otherwise.
pub mod raw {
    pub use crate::scheduler::{cancel, park, spawn_task, wake};
}

/// PHP: `sleep()` / `usleep()`. Parks the current task, not the thread.
pub fn sleep(dur: Duration) -> Result<()> {
    let deadline = Instant::now() + dur;
    let me = current_task().expect("sleep() must be called inside a task");
    let _timer = TimerGuard(scheduler::add_timer(deadline, me));
    while Instant::now() < deadline {
        scheduler::park()?;
    }
    Ok(())
}

/// PHP: `Concurrency::run([...])` from Laravel.
pub mod concurrency {
    use super::*;

    pub type Job<T> = Box<dyn FnOnce() -> Result<T>>;

    /// Run every closure concurrently and wait for all results, in order.
    /// First failure cancels the others and is returned.
    pub fn run<T: Clone + 'static>(jobs: Vec<Job<T>>) -> Result<Vec<T>> {
        Promise::all(jobs.into_iter().map(spawn).collect()).wait()
    }
}
