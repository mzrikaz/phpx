//! Single-threaded green-thread scheduler.
//!
//! Each task is a stackful coroutine. Code inside a task calls `park()` to give
//! up the CPU until something calls `wake()` on it. Nothing in user code ever
//! blocks the OS thread: `sleep`, `Promise::wait` and (later) I/O all park.
//!
//! M:N across cores comes later; the public API (`spawn`/`park`/`wake`) stays
//! the same, only `Rc`/`RefCell` turn into `Arc`/locks.

use std::cell::{Cell, RefCell};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::ptr;
use std::time::Instant;

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, CoroutineResult, Yielder};
use polling::{Event, Events, Poller};

use crate::error::{Error, Result};

pub type TaskId = u64;
pub(crate) type TimerId = u64;

type Co = Coroutine<(), (), (), DefaultStack>;

/// Default virtual stack size per task. Pages are committed lazily by the OS,
/// so the real cost of a task is only what its stack actually touches.
const DEFAULT_STACK_SIZE: usize = 128 * 1024;

/// Finished tasks return their stacks here so `spawn` skips mmap/munmap.
const STACK_POOL_MAX: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Ready,
    Running,
    Parked,
}

struct Task {
    co: Option<Co>,
    state: State,
    cancelled: bool,
    /// `wake()` arrived while the task was running; next `park()` returns at once.
    notified: bool,
    /// Lowest usable address of the task's stack.
    stack_limit: usize,
}

#[derive(Default)]
struct Scheduler {
    tasks: HashMap<TaskId, Task>,
    ready: VecDeque<TaskId>,
    timers: BinaryHeap<Reverse<(Instant, TimerId, TaskId)>>,
    live_timers: HashSet<TimerId>,
    next_task: TaskId,
    next_timer: TimerId,
    current: Option<TaskId>,
    deferred: Vec<Box<dyn FnOnce()>>,
    /// fd -> task parked until that fd is ready. Removed when the event fires.
    io_waiters: HashMap<usize, TaskId>,
    /// fd -> (readable, writable) of the event that woke its waiter.
    io_fired: HashMap<usize, (bool, bool)>,
    stack_pool: Vec<DefaultStack>,
}

thread_local! {
    static SCHED: RefCell<Scheduler> = RefCell::new(Scheduler::default());
    static YIELDER: Cell<*const Yielder<(), ()>> = const { Cell::new(ptr::null()) };
    /// epoll (Linux) / kqueue (BSD, macOS). TODO: io_uring backend.
    static POLLER: Poller = Poller::new().expect("failed to create poller");
    static STACK_SIZE: Cell<usize> = const { Cell::new(DEFAULT_STACK_SIZE) };
    /// Stack limit of the running task, 0 outside tasks.
    static STACK_LIMIT: Cell<usize> = const { Cell::new(0) };
}

/// Bytes of stack left in the running task (None outside a task). Lets
/// deeply recursive code (an interpreter) fail cleanly instead of crashing.
#[inline(never)]
pub fn stack_remaining() -> Option<usize> {
    let limit = STACK_LIMIT.get();
    if limit == 0 {
        return None;
    }
    let marker = 0u8;
    Some((std::ptr::addr_of!(marker) as usize).saturating_sub(limit))
}

/// Set the stack size for tasks spawned from now on (e.g. a tree-walking
/// interpreter needs far more than compiled code). Must be called while idle.
pub fn set_stack_size(bytes: usize) {
    STACK_SIZE.set(bytes.max(16 * 1024));
    // Pooled stacks have the old size.
    with(|s| s.stack_pool.clear());
}

pub fn stack_size() -> usize {
    STACK_SIZE.get()
}

/// Borrow the scheduler. Never call user code or switch tasks inside `f`.
fn with<R>(f: impl FnOnce(&mut Scheduler) -> R) -> R {
    SCHED.with(|s| f(&mut s.borrow_mut()))
}

/// Spawn a raw task. Higher layers (`Promise`) wrap this to carry results.
pub fn spawn_task(f: impl FnOnce() + 'static) -> TaskId {
    let stack = match with(|s| s.stack_pool.pop()) {
        Some(st) => st,
        None => DefaultStack::new(STACK_SIZE.get()).expect("failed to allocate task stack"),
    };
    use corosensei::stack::Stack;
    let stack_limit = stack.limit().get();
    let co = Coroutine::with_stack(stack, move |y: &Yielder<(), ()>, ()| {
        YIELDER.set(y as *const _);
        f();
    });
    with(|s| {
        let id = s.next_task;
        s.next_task += 1;
        s.tasks.insert(
            id,
            Task { co: Some(co), state: State::Ready, cancelled: false, notified: false, stack_limit },
        );
        s.ready.push_back(id);
        id
    })
}

pub fn current_task() -> Option<TaskId> {
    with(|s| s.current)
}

fn expect_current() -> TaskId {
    current_task().expect("must be called from inside a task (use block_on)")
}

pub(crate) fn is_cancelled(id: TaskId) -> bool {
    with(|s| s.tasks.get(&id).is_some_and(|t| t.cancelled))
}

/// Suspend the current task until `wake()` is called on it.
/// Returns `Err(Cancelled)` if the task was cancelled before or while parked.
/// Callers must re-check their condition after return: wakeups can be spurious.
pub fn park() -> Result<()> {
    let id = expect_current();
    let must_suspend = with(|s| {
        let t = s.tasks.get_mut(&id).unwrap();
        if t.cancelled {
            return Err(Error::Cancelled);
        }
        if t.notified {
            t.notified = false;
            return Ok(false);
        }
        t.state = State::Parked;
        Ok(true)
    })?;
    if must_suspend {
        suspend_current();
    }
    if is_cancelled(id) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

/// Let other ready tasks run, then continue. Later the compiler inserts this at
/// loop back-edges so CPU-heavy code can't starve the thread.
pub fn yield_now() -> Result<()> {
    let id = expect_current();
    with(|s| {
        s.tasks.get_mut(&id).unwrap().state = State::Ready;
        s.ready.push_back(id);
    });
    suspend_current();
    if is_cancelled(id) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

fn suspend_current() {
    let y = YIELDER.get();
    assert!(!y.is_null(), "no active task to suspend");
    // SAFETY: `y` was set by this task's coroutine body and the coroutine is
    // still alive (we're running on its stack).
    unsafe { (*y).suspend(()) };
    // Other tasks overwrote YIELDER while we were away.
    YIELDER.set(y);
}

pub fn wake(id: TaskId) {
    with(|s| {
        let Some(t) = s.tasks.get_mut(&id) else { return };
        match t.state {
            State::Parked => {
                t.state = State::Ready;
                s.ready.push_back(id);
            }
            State::Running => t.notified = true,
            State::Ready => {}
        }
    })
}

/// Mark a task cancelled and wake it so its pending `park()` returns `Err(Cancelled)`.
pub fn cancel(id: TaskId) {
    let exists = with(|s| match s.tasks.get_mut(&id) {
        Some(t) => {
            t.cancelled = true;
            true
        }
        None => false,
    });
    if exists {
        wake(id);
    }
}

pub(crate) fn add_timer(at: Instant, task: TaskId) -> TimerId {
    with(|s| {
        let tid = s.next_timer;
        s.next_timer += 1;
        s.timers.push(Reverse((at, tid, task)));
        s.live_timers.insert(tid);
        tid
    })
}

pub(crate) fn remove_timer(tid: TimerId) {
    with(|s| {
        s.live_timers.remove(&tid);
    })
}

/// Queue a closure to run after the request's main task returns its result.
pub fn defer(f: impl FnOnce() + 'static) {
    with(|s| s.deferred.push(Box::new(f)));
}

pub(crate) fn take_deferred() -> Vec<Box<dyn FnOnce()>> {
    with(|s| std::mem::take(&mut s.deferred))
}

pub(crate) fn assert_idle() {
    with(|s| {
        assert!(s.current.is_none(), "block_on cannot be nested inside a task");
        assert!(s.tasks.is_empty(), "runtime already has live tasks");
    })
}

/// Start tracking a non-blocking fd. Interest is armed per wait (oneshot).
pub(crate) fn register_io(fd: BorrowedFd<'_>) -> io::Result<()> {
    // SAFETY: callers (net.rs) call `deregister_io` before the fd is closed.
    POLLER.with(|p| unsafe { p.add(&fd, Event::none(fd.as_raw_fd() as usize)) })
}

pub(crate) fn deregister_io(fd: BorrowedFd<'_>) {
    let _ = POLLER.with(|p| p.delete(fd));
}

/// Park until one of `fds` (each with its read / write interest) is ready
/// or `deadline` passes. Returns (readable, writable) per fd; all false on
/// timeout. Every fd must be registered (`register_io`). One waiter per fd.
pub(crate) fn wait_fds(fds: &[(BorrowedFd<'_>, bool, bool)], deadline: Option<Instant>) -> Result<Vec<(bool, bool)>> {
    let me = expect_current();
    for &(fd, r, w) in fds {
        let key = fd.as_raw_fd() as usize;
        let ev = Event::new(key, r, w);
        POLLER.with(|p| p.modify(fd, ev))?;
        with(|s| {
            let prev = s.io_waiters.insert(key, me);
            assert!(prev.is_none_or(|t| t == me), "two tasks waiting on fd {key}");
        });
    }

    // Disarm whatever didn't fire, also on cancel, so a late event doesn't
    // wake a stranger.
    struct Guard<'a, 'b>(&'a [(BorrowedFd<'b>, bool, bool)], Option<TimerId>);
    impl Drop for Guard<'_, '_> {
        fn drop(&mut self) {
            for &(fd, ..) in self.0 {
                let key = fd.as_raw_fd() as usize;
                if with(|s| s.io_waiters.remove(&key)).is_some() {
                    let _ = POLLER.with(|p| p.modify(fd, Event::none(key)));
                }
                with(|s| s.io_fired.remove(&key));
            }
            if let Some(t) = self.1 {
                remove_timer(t);
            }
        }
    }
    let _guard = Guard(fds, deadline.map(|at| add_timer(at, me)));
    let keys: Vec<usize> = fds.iter().map(|(fd, ..)| fd.as_raw_fd() as usize).collect();
    loop {
        let fired: Option<Vec<(bool, bool)>> = with(|s| {
            keys.iter().any(|k| s.io_fired.contains_key(k)).then(|| keys.iter().map(|k| s.io_fired.get(k).copied().unwrap_or_default()).collect())
        });
        if let Some(f) = fired {
            return Ok(f);
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            return Ok(vec![(false, false); keys.len()]);
        }
        park()?;
    }
}

/// Block the OS thread until an fd is ready or `deadline` passes, then wake
/// the waiting tasks. Only called when no task is ready to run.
fn poll_io(events: &mut Events, deadline: Option<Instant>) {
    let timeout = deadline.map(|at| at.saturating_duration_since(Instant::now()));
    events.clear();
    match POLLER.with(|p| p.wait(events, timeout)) {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::Interrupted => return,
        Err(e) => panic!("poller failed: {e}"),
    }
    let woken: Vec<TaskId> = with(|s| {
        events
            .iter()
            .filter_map(|ev| {
                let t = s.io_waiters.remove(&ev.key)?;
                s.io_fired.insert(ev.key, (ev.readable, ev.writable));
                Some(t)
            })
            .collect()
    });
    woken.into_iter().for_each(wake);
}

/// Fire due timers. Returns how many tasks were woken.
fn fire_timers(now: Instant) -> usize {
    let mut due = Vec::new();
    with(|s| {
        while let Some(Reverse((at, tid, task))) = s.timers.peek().copied() {
            if !s.live_timers.contains(&tid) {
                s.timers.pop();
                continue;
            }
            if at > now {
                break;
            }
            s.timers.pop();
            s.live_timers.remove(&tid);
            due.push(task);
        }
    });
    let n = due.len();
    due.into_iter().for_each(wake);
    n
}

/// Earliest live timer deadline, discarding removed timers on the way.
fn next_deadline() -> Option<Instant> {
    with(|s| {
        while let Some(Reverse((at, tid, _))) = s.timers.peek().copied() {
            if s.live_timers.contains(&tid) {
                return Some(at);
            }
            s.timers.pop();
        }
        None
    })
}

/// Drive tasks until none are left.
pub(crate) fn run() {
    let mut events = Events::new();
    loop {
        if let Some(id) = with(|s| s.ready.pop_front()) {
            run_task(id);
            continue;
        }
        if fire_timers(Instant::now()) > 0 {
            continue;
        }
        let deadline = next_deadline();
        let (parked, waiting_io) = with(|s| (s.tasks.len(), !s.io_waiters.is_empty()));
        if parked == 0 {
            return;
        }
        if deadline.is_none() && !waiting_io {
            leak_all_tasks();
            panic!("deadlock: {parked} task(s) parked with nothing left to wake them");
        }
        poll_io(&mut events, deadline);
    }
}

/// Forget every remaining task without unwinding its stack. Used before
/// panicking so thread-local teardown doesn't resume half-run coroutines.
fn leak_all_tasks() {
    let tasks = with(|s| std::mem::take(&mut *s));
    for (_, t) in tasks.tasks {
        std::mem::forget(t.co);
    }
}

fn run_task(id: TaskId) {
    let co = with(|s| {
        let t = s.tasks.get_mut(&id)?;
        t.state = State::Running;
        s.current = Some(id);
        Some((t.co.take()?, t.stack_limit))
    });
    let Some((mut co, limit)) = co else { return };

    STACK_LIMIT.set(limit);
    let res = co.resume(());
    STACK_LIMIT.set(0);
    YIELDER.set(ptr::null());

    let finished = with(|s| {
        s.current = None;
        match res {
            CoroutineResult::Yield(()) => {
                let t = s.tasks.get_mut(&id).unwrap();
                t.co = Some(co);
                if t.state == State::Running {
                    t.state = State::Ready;
                    s.ready.push_back(id);
                }
                None
            }
            CoroutineResult::Return(()) => {
                s.tasks.remove(&id);
                Some(co)
            }
        }
    });
    if let Some(co) = finished {
        let stack = co.into_stack();
        with(|s| {
            if s.stack_pool.len() < STACK_POOL_MAX {
                s.stack_pool.push(stack);
            }
        });
    }
}
