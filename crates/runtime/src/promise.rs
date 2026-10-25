//! Laravel/Guzzle-style promises on top of green threads.
//!
//! A promise is a handle to a task that started immediately (eager, like
//! `Http::async()`). `wait()` parks only the calling task, never the thread.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::scheduler::{self, TaskId};

struct Inner<T> {
    result: Option<Result<T>>,
    waiters: Vec<TaskId>,
    task: Option<TaskId>,
}

pub struct Promise<T> {
    inner: Rc<RefCell<Inner<T>>>,
}

impl<T> Clone for Promise<T> {
    fn clone(&self) -> Self {
        Promise { inner: Rc::clone(&self.inner) }
    }
}

/// PHP: `async(fn() => ...)`. Starts `f` in a new green thread right away.
pub fn spawn<T, F>(f: F) -> Promise<T>
where
    T: 'static,
    F: FnOnce() -> Result<T> + 'static,
{
    let p = Promise::<T>::pending();
    let settle = p.clone();
    let id = scheduler::spawn_task(move || {
        let me = scheduler::current_task().unwrap();
        let r = if scheduler::is_cancelled(me) {
            Err(Error::Cancelled)
        } else {
            panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|p| Err(panic_error(p)))
        };
        settle.settle(r);
    });
    p.inner.borrow_mut().task = Some(id);
    p
}

fn panic_error(payload: Box<dyn std::any::Any + Send>) -> Error {
    let msg = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into());
    Error::Panic(msg)
}

impl<T> Promise<T> {
    fn pending() -> Self {
        Promise { inner: Rc::new(RefCell::new(Inner { result: None, waiters: vec![], task: None })) }
    }

    fn settle(&self, r: Result<T>) {
        let waiters = {
            let mut inner = self.inner.borrow_mut();
            debug_assert!(inner.result.is_none(), "promise settled twice");
            inner.result = Some(r);
            std::mem::take(&mut inner.waiters)
        };
        waiters.into_iter().for_each(scheduler::wake);
    }

    pub fn is_settled(&self) -> bool {
        self.inner.borrow().result.is_some()
    }

    fn is_fulfilled(&self) -> bool {
        matches!(self.inner.borrow().result, Some(Ok(_)))
    }

    fn error(&self) -> Option<Error> {
        match &self.inner.borrow().result {
            Some(Err(e)) => Some(e.clone()),
            _ => None,
        }
    }

    /// Register the current task to be woken when this promise settles.
    fn subscribe(&self) {
        let me = scheduler::current_task().expect("wait() must be called inside a task");
        let mut inner = self.inner.borrow_mut();
        if inner.result.is_none() && !inner.waiters.contains(&me) {
            inner.waiters.push(me);
        }
    }

    /// PHP: `$promise->cancel()`. Stops the underlying task at its next park point.
    pub fn cancel(&self) {
        if self.is_settled() {
            return;
        }
        if let Some(id) = self.inner.borrow().task {
            scheduler::cancel(id);
        }
    }
}

impl<T: Clone + 'static> Promise<T> {
    /// PHP: `$promise->wait()`. Parks the current task until settled.
    pub fn wait(&self) -> Result<T> {
        loop {
            if let Some(r) = &self.inner.borrow().result {
                return r.clone();
            }
            self.subscribe();
            if let Err(e) = scheduler::park() {
                // We were cancelled while waiting: take the child down with us.
                self.cancel();
                return Err(e);
            }
        }
    }

    /// Result of an already-settled promise, readable from outside any task.
    pub(crate) fn settled_result(&self) -> Option<Result<T>> {
        self.inner.borrow().result.clone()
    }

    /// Like `wait()` but gives up after `dur`, cancelling the task.
    pub fn wait_timeout(&self, dur: Duration) -> Result<T> {
        let deadline = Instant::now() + dur;
        let me = scheduler::current_task().expect("wait_timeout() must be called inside a task");
        let _timer = TimerGuard(scheduler::add_timer(deadline, me));
        loop {
            if let Some(r) = &self.inner.borrow().result {
                return r.clone();
            }
            if Instant::now() >= deadline {
                self.cancel();
                return Err(Error::Timeout);
            }
            self.subscribe();
            if let Err(e) = scheduler::park() {
                self.cancel();
                return Err(e);
            }
        }
    }

    /// PHP: `->then(fn($v) => ...)`. Runs `f` with the value once fulfilled;
    /// a rejection skips `f` and flows through to the returned promise.
    pub fn then<U, F>(&self, f: F) -> Promise<U>
    where
        U: 'static,
        F: FnOnce(T) -> Result<U> + 'static,
    {
        let me = self.clone();
        spawn(move || f(me.wait()?))
    }

    /// PHP: `->catch(fn(Throwable $e) => ...)`. Recovers from a rejection.
    pub fn catch<F>(&self, f: F) -> Promise<T>
    where
        F: FnOnce(Error) -> Result<T> + 'static,
    {
        let me = self.clone();
        spawn(move || me.wait().or_else(f))
    }

    /// PHP: `->finally(fn() => ...)`. Runs `f` either way, passes result through.
    pub fn finally<F>(&self, f: F) -> Promise<T>
    where
        F: FnOnce() + 'static,
    {
        let me = self.clone();
        spawn(move || {
            let r = me.wait();
            f();
            r
        })
    }

    /// PHP: `->timeout(2.0)`. Rejects with `Error::Timeout` and cancels the work.
    pub fn timeout(&self, dur: Duration) -> Promise<T> {
        let me = self.clone();
        spawn(move || me.wait_timeout(dur))
    }

    /// PHP: `Promise::all([...])`. Fulfills with every value in order; rejects
    /// as soon as any promise rejects and cancels the rest.
    pub fn all(promises: Vec<Promise<T>>) -> Promise<Vec<T>> {
        spawn(move || {
            wait_until(&promises, |ps| {
                if let Some(e) = ps.iter().find_map(Promise::error) {
                    cancel_all(ps);
                    return Some(Err(e));
                }
                ps.iter().all(|p| p.is_fulfilled()).then(|| ps.iter().map(|p| p.wait()).collect())
            })
        })
    }

    /// PHP: `Promise::allSettled([...])`. Never rejects; each slot is Ok or Err.
    pub fn all_settled(promises: Vec<Promise<T>>) -> Promise<Vec<Result<T>>> {
        spawn(move || {
            wait_until(&promises, |ps| {
                ps.iter().all(|p| p.is_settled()).then(|| Ok(ps.iter().map(|p| p.wait()).collect()))
            })
        })
    }

    /// PHP: `Promise::any([...])`. First fulfillment wins, the rest are cancelled.
    pub fn any(promises: Vec<Promise<T>>) -> Promise<T> {
        spawn(move || {
            wait_until(&promises, |ps| {
                if let Some(ok) = ps.iter().find(|p| p.is_fulfilled()) {
                    let v = ok.wait();
                    cancel_all(ps);
                    return Some(v);
                }
                ps.iter().all(|p| p.is_settled()).then(|| {
                    Err(Error::AllRejected(ps.iter().filter_map(Promise::error).collect()))
                })
            })
        })
    }

    /// PHP: `Promise::race([...])`. First to settle wins, success or failure.
    pub fn race(promises: Vec<Promise<T>>) -> Promise<T> {
        spawn(move || {
            wait_until(&promises, |ps| {
                let first = ps.iter().find(|p| p.is_settled())?;
                let r = first.wait();
                cancel_all(ps);
                Some(r)
            })
        })
    }
}

fn cancel_all<T>(ps: &[Promise<T>]) {
    ps.iter().for_each(Promise::cancel);
}

/// Park until `check` returns `Some`, re-checking each time any promise settles.
fn wait_until<T, R>(ps: &[Promise<T>], check: impl Fn(&[Promise<T>]) -> Option<Result<R>>) -> Result<R> {
    loop {
        if let Some(r) = check(ps) {
            return r;
        }
        ps.iter().for_each(Promise::subscribe);
        if let Err(e) = scheduler::park() {
            cancel_all(ps);
            return Err(e);
        }
    }
}

pub(crate) struct TimerGuard(pub(crate) scheduler::TimerId);

impl Drop for TimerGuard {
    fn drop(&mut self) {
        scheduler::remove_timer(self.0);
    }
}
