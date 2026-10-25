//! Generators as green-thread tasks.
//!
//! A generator body runs in its own task. `next()`/`send()` hand it a
//! `Resume` and park the caller until it yields (`Event`). Because the body
//! is an ordinary task, it can do async I/O or sleep between yields.

use std::cell::RefCell;
use std::rc::Rc;

use phpx_runtime::raw;
use phpx_runtime::TaskId;

use crate::class::Class;
use crate::interp::{Ctrl, Interp, R};
use crate::value::{Internal, Obj, Value};

pub enum Resume {
    Next(Value),
    Throw(Rc<Obj>),
    Close,
}

pub enum Event {
    Yield(Value, Value),
    Done(Value),
    Failed(Ctrl),
}

#[derive(Default)]
pub struct GenState {
    to_gen: Option<Resume>,
    from_gen: Option<Event>,
    gen_task: Option<TaskId>,
    consumer: Option<TaskId>,
    started: bool,
    finished: bool,
    running: bool,
    /// Moved past the first yield (rewind() is then an error).
    advanced: bool,
    current: Option<(Value, Value)>,
    ret: Option<Value>,
    pub next_key: i64,
}

#[derive(Default)]
pub struct GenShared {
    pub st: RefCell<GenState>,
}

/// Owned by the Generator object. Dropping it closes the generator, which
/// runs the body's pending `finally` blocks.
pub struct GenHandle(pub Rc<GenShared>);

impl Drop for GenHandle {
    fn drop(&mut self) {
        close(&self.0);
    }
}

fn close(g: &GenShared) {
    let Ok(mut st) = g.st.try_borrow_mut() else { return };
    if st.finished {
        return;
    }
    st.to_gen = Some(Resume::Close);
    if let Some(t) = st.gen_task {
        raw::wake(t);
    }
}

impl Interp {
    /// Start a generator task for a body bound to `frame` (params already set).
    pub fn spawn_generator(&self, run: impl FnOnce(&Interp, Rc<GenShared>) -> R<Value> + 'static) -> Value {
        let shared = Rc::new(GenShared::default());
        let it = self.rc();
        let g = shared.clone();
        let task = raw::spawn_task(move || {
            let event = match it.gen_wait(&g) {
                Resume::Close => Event::Done(Value::Null),
                Resume::Throw(e) => Event::Failed(Ctrl::Throw(e)),
                Resume::Next(_) => match run(&it, g.clone()) {
                    Ok(v) => Event::Done(v),
                    Err(Ctrl::GenClose) => Event::Done(Value::Null),
                    Err(c) => Event::Failed(c),
                },
            };
            it.gen_emit(&g, event);
        });
        shared.st.borrow_mut().gen_task = Some(task);
        self.generators.borrow_mut().push(Rc::downgrade(&shared));
        let cls = self.classes.borrow().get("generator").cloned().expect("Generator class");
        Value::Object(Obj::new(cls, Internal::Generator(Rc::new(GenHandle(shared)))))
    }

    /// Generator side: wait for the consumer's next instruction.
    fn gen_wait(&self, g: &GenShared) -> Resume {
        loop {
            if let Some(r) = g.st.borrow_mut().to_gen.take() {
                return r;
            }
            if raw::park().is_err() {
                return Resume::Close;
            }
        }
    }

    fn gen_emit(&self, g: &GenShared, ev: Event) {
        let consumer = {
            let mut st = g.st.borrow_mut();
            if matches!(ev, Event::Done(_) | Event::Failed(_)) {
                st.finished = true;
            }
            st.from_gen = Some(ev);
            st.consumer
        };
        if let Some(c) = consumer {
            raw::wake(c);
        }
    }

    /// `yield $k => $v` inside a generator body. Returns the sent value.
    pub fn gen_yield(&self, g: &GenShared, key: Value, value: Value) -> R<Value> {
        self.gen_emit(g, Event::Yield(key, value));
        match self.gen_wait(g) {
            Resume::Next(v) => Ok(v),
            Resume::Throw(e) => Err(Ctrl::Throw(e)),
            Resume::Close => Err(Ctrl::GenClose),
        }
    }

    /// Consumer side: hand the generator an instruction and wait for its event.
    fn gen_resume(&self, g: &GenShared, r: Resume) -> R<()> {
        let task = {
            let mut st = g.st.borrow_mut();
            if st.finished {
                return Ok(());
            }
            if st.running {
                drop(st);
                return Err(self.error("Error", "Cannot resume an already running generator"));
            }
            st.running = true;
            st.to_gen = Some(r);
            st.consumer = phpx_runtime::current_task();
            st.gen_task
        };
        if let Some(t) = task {
            raw::wake(t);
        }
        let ev = loop {
            if let Some(ev) = g.st.borrow_mut().from_gen.take() {
                break ev;
            }
            if let Err(e) = raw::park() {
                g.st.borrow_mut().running = false;
                return Err(crate::native::rt_error(self, e));
            }
        };
        let mut st = g.st.borrow_mut();
        st.running = false;
        match ev {
            Event::Yield(k, v) => st.current = Some((k, v)),
            Event::Done(v) => {
                st.current = None;
                st.ret = Some(v);
            }
            Event::Failed(c) => {
                st.current = None;
                return Err(c);
            }
        }
        Ok(())
    }

    fn gen_start(&self, g: &GenShared) -> R<()> {
        let start = {
            let mut st = g.st.borrow_mut();
            !std::mem::replace(&mut st.started, true)
        };
        if start {
            self.gen_resume(g, Resume::Next(Value::Null))?;
        }
        Ok(())
    }

    /// Close every generator still alive at the end of the request, so their
    /// tasks finish (abandoned generators would otherwise park forever).
    pub fn close_all_generators(&self) {
        for g in self.generators.borrow_mut().drain(..) {
            if let Some(g) = g.upgrade() {
                close(&g);
            }
        }
    }
}

fn current_value(g: &GenShared) -> Value {
    let st = g.st.borrow();
    st.current.as_ref().map(|(_, v)| v.clone()).unwrap_or_default()
}

fn current_key(g: &GenShared) -> Value {
    let st = g.st.borrow();
    st.current.as_ref().map(|(k, _)| k.clone()).unwrap_or_default()
}

fn gen_of(this: Option<&Rc<Obj>>) -> Rc<GenShared> {
    match &*this.expect("Generator method needs $this").internal.borrow() {
        Internal::Generator(h) => h.0.clone(),
        _ => unreachable!("Generator object without state"),
    }
}

type M = fn(&Interp, Option<&Rc<Obj>>, &Rc<Class>, Vec<Value>) -> R<Value>;

pub fn natives() -> Vec<(&'static str, M)> {
    vec![
        ("current", |it, this, _, _| {
            let g = gen_of(this);
            it.gen_start(&g)?;
            Ok(current_value(&g))
        }),
        ("key", |it, this, _, _| {
            let g = gen_of(this);
            it.gen_start(&g)?;
            Ok(current_key(&g))
        }),
        ("next", |it, this, _, _| {
            let g = gen_of(this);
            if !g.st.borrow().started {
                it.gen_start(&g)?;
            }
            g.st.borrow_mut().advanced = true;
            it.gen_resume(&g, Resume::Next(Value::Null))?;
            Ok(Value::Null)
        }),
        ("send", |it, this, _, a| {
            let g = gen_of(this);
            // send() on a fresh generator first runs it to its first yield.
            it.gen_start(&g)?;
            g.st.borrow_mut().advanced = true;
            it.gen_resume(&g, Resume::Next(a.into_iter().next().unwrap_or_default()))?;
            Ok(current_value(&g))
        }),
        ("throw", |it, this, _, a| {
            let g = gen_of(this);
            let Some(Value::Object(e)) = a.into_iter().next() else {
                return Err(it.error("TypeError", "Generator::throw(): Argument #1 ($exception) must be of type Throwable"));
            };
            if !g.st.borrow().started {
                // Throwing into an unstarted generator throws at its start.
                g.st.borrow_mut().started = true;
            }
            it.gen_resume(&g, Resume::Throw(e))?;
            Ok(current_value(&g))
        }),
        ("valid", |it, this, _, _| {
            let g = gen_of(this);
            it.gen_start(&g)?;
            let valid = g.st.borrow().current.is_some();
            Ok(Value::Bool(valid))
        }),
        ("rewind", |it, this, _, _| {
            let g = gen_of(this);
            it.gen_start(&g)?;
            if g.st.borrow().advanced {
                return Err(it.error("Exception", "Cannot rewind a generator that was already run"));
            }
            Ok(Value::Null)
        }),
        ("getreturn", |it, this, _, _| {
            let g = gen_of(this);
            let st = g.st.borrow();
            match (&st.ret, st.finished) {
                (Some(v), true) => Ok(v.clone()),
                _ => {
                    drop(st);
                    Err(it.error("Exception", "Cannot get return value of a generator that hasn't returned"))
                }
            }
        }),
    ]
}
