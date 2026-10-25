//! Fiber: a green-thread task that runs only while resumed. start() and
//! resume() hand it control and park the caller until it suspends or
//! finishes; Fiber::suspend() inside it does the reverse.

use std::cell::RefCell;
use std::rc::Rc;

use phpx_runtime::raw;
use phpx_runtime::TaskId;

use crate::class::Class;
use crate::interp::{Callable, Ctrl, Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, Value};

enum ToFiber {
    Start(Vec<Value>),
    Value(Value),
    Throw(Rc<Obj>),
}

enum FromFiber {
    Suspend(Value),
    Done(Value),
    Failed(Ctrl),
}

#[derive(PartialEq, Clone, Copy)]
enum State {
    Init,
    Running,
    Suspended,
    Terminated,
}

struct Shared {
    callable: Callable,
    state: State,
    to: Option<ToFiber>,
    from: Option<FromFiber>,
    task: Option<TaskId>,
    resumer: Option<TaskId>,
    ret: Option<Value>,
    /// The Fiber object (getCurrent()); weak to avoid a cycle.
    obj: std::rc::Weak<Obj>,
}

struct FiberData(Rc<RefCell<Shared>>);

impl NativeData for FiberData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

impl Drop for FiberData {
    fn drop(&mut self) {
        // A suspended fiber that's dropped is destroyed: unwind its task.
        if let Ok(s) = self.0.try_borrow() {
            if s.state == State::Suspended {
                if let Some(t) = s.task {
                    raw::cancel(t);
                }
            }
        }
    }
}

thread_local! {
    /// Fibers currently running, innermost last.
    static RUNNING: RefCell<Vec<Rc<RefCell<Shared>>>> = const { RefCell::new(Vec::new()) };
}

fn fiber_error(it: &Interp, msg: &str) -> Ctrl {
    it.error("FiberError", msg)
}

fn state_of(it: &Interp, this: Option<&Rc<Obj>>) -> R<Rc<RefCell<Shared>>> {
    let o = this.ok_or_else(|| it.error("Error", "Fiber method called statically"))?;
    match &*o.internal() {
        Internal::Data(d) => d.as_any().downcast_ref::<FiberData>().map(|f| f.0.clone()),
        _ => None,
    }
    .ok_or_else(|| it.error("Error", "Fiber is not constructed"))
}

/// Run the fiber until it suspends or ends; the value it suspended with.
fn transfer(it: &Interp, sh: &Rc<RefCell<Shared>>, msg: ToFiber) -> R<Value> {
    let task = {
        let mut s = sh.borrow_mut();
        s.state = State::Running;
        s.to = Some(msg);
        s.resumer = phpx_runtime::current_task();
        s.task
    };
    RUNNING.with(|r| r.borrow_mut().push(sh.clone()));
    if let Some(t) = task {
        raw::wake(t);
    }
    let ev = loop {
        if let Some(ev) = sh.borrow_mut().from.take() {
            break ev;
        }
        if let Err(e) = raw::park() {
            RUNNING.with(|r| r.borrow_mut().pop());
            return Err(crate::native::rt_error(it, e));
        }
    };
    RUNNING.with(|r| r.borrow_mut().pop());
    let mut s = sh.borrow_mut();
    match ev {
        FromFiber::Suspend(v) => {
            s.state = State::Suspended;
            Ok(v)
        }
        FromFiber::Done(v) => {
            s.state = State::Terminated;
            s.ret = Some(v);
            Ok(Value::Null)
        }
        FromFiber::Failed(c) => {
            s.state = State::Terminated;
            Err(c)
        }
    }
}

/// Inside the fiber: wait for the next instruction.
fn wait(sh: &Rc<RefCell<Shared>>) -> Option<ToFiber> {
    loop {
        if let Some(m) = sh.borrow_mut().to.take() {
            return Some(m);
        }
        if raw::park().is_err() {
            return None;
        }
    }
}

fn emit(sh: &Rc<RefCell<Shared>>, ev: FromFiber) {
    let resumer = {
        let mut s = sh.borrow_mut();
        s.from = Some(ev);
        s.resumer
    };
    if let Some(t) = resumer {
        raw::wake(t);
    }
}

fn construct(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.ok_or_else(|| it.error("Error", "Fiber::__construct called statically"))?;
    let f = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    let callable = it.to_callable(&f, a.first().unwrap_or(&Value::Null))?;
    let sh = Rc::new(RefCell::new(Shared {
        callable,
        state: State::Init,
        to: None,
        from: None,
        task: None,
        resumer: None,
        ret: None,
        obj: Rc::downgrade(o),
    }));
    o.set_internal(Internal::Data(Box::new(FiberData(sh))));
    Ok(Value::Null)
}

fn start(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let sh = state_of(it, this)?;
    if sh.borrow().state != State::Init {
        return Err(fiber_error(it, "Cannot start a fiber that has already been started"));
    }
    let it2 = it.rc();
    let g = sh.clone();
    let task = raw::spawn_task(move || {
        let ev = match wait(&g) {
            Some(ToFiber::Start(args)) => {
                let c = g.borrow().callable.clone();
                match it2.call_callable(&c, args, vec![]) {
                    Ok(v) => FromFiber::Done(v),
                    Err(c) => FromFiber::Failed(c),
                }
            }
            Some(ToFiber::Throw(e)) => FromFiber::Failed(Ctrl::Throw(e)),
            _ => FromFiber::Done(Value::Null),
        };
        emit(&g, ev);
    });
    sh.borrow_mut().task = Some(task);
    transfer(it, &sh, ToFiber::Start(a))
}

fn resume(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let sh = state_of(it, this)?;
    if sh.borrow().state != State::Suspended {
        return Err(fiber_error(it, "Cannot resume a fiber that is not suspended"));
    }
    transfer(it, &sh, ToFiber::Value(a.into_iter().next().unwrap_or_default()))
}

fn throw(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let sh = state_of(it, this)?;
    if sh.borrow().state != State::Suspended {
        return Err(fiber_error(it, "Cannot resume a fiber that is not suspended"));
    }
    let Some(Value::Object(e)) = a.into_iter().next() else {
        return Err(it.error("TypeError", "Fiber::throw(): Argument #1 ($exception) must be of type Throwable"));
    };
    transfer(it, &sh, ToFiber::Throw(e))
}

fn suspend(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let Some(sh) = RUNNING.with(|r| r.borrow().last().cloned()) else {
        return Err(fiber_error(it, "Cannot suspend outside of a fiber"));
    };
    if sh.borrow().task != phpx_runtime::current_task() {
        return Err(fiber_error(it, "Cannot suspend outside of a fiber"));
    }
    emit(&sh, FromFiber::Suspend(a.into_iter().next().unwrap_or_default()));
    match wait(&sh) {
        Some(ToFiber::Value(v)) => Ok(v),
        Some(ToFiber::Throw(e)) => Err(Ctrl::Throw(e)),
        // Destroyed while suspended: unwind without running more code.
        _ => Err(Ctrl::GenClose),
    }
}

fn get_current(_: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(RUNNING
        .with(|r| r.borrow().last().cloned())
        .filter(|sh| sh.borrow().task == phpx_runtime::current_task())
        .and_then(|sh| sh.borrow().obj.upgrade())
        .map(Value::Object)
        .unwrap_or(Value::Null))
}

fn get_return(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let sh = state_of(it, this)?;
    let s = sh.borrow();
    match (s.state, &s.ret) {
        (State::Terminated, Some(v)) => Ok(v.clone()),
        (State::Terminated, None) => Err(fiber_error(it, "Cannot get fiber return value: The fiber threw an exception")),
        (State::Init, _) => Err(fiber_error(it, "Cannot get fiber return value: The fiber has not been started")),
        _ => Err(fiber_error(it, "Cannot get fiber return value: The fiber has not returned")),
    }
}

fn is(it: &Interp, this: Option<&Rc<Obj>>, f: fn(State) -> bool) -> R<Value> {
    let sh = state_of(it, this)?;
    let st = sh.borrow().state;
    Ok(Value::Bool(f(st)))
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("fiber", "__construct", construct as NativeMethod),
        ("fiber", "start", start),
        ("fiber", "resume", resume),
        ("fiber", "throw", throw),
        ("fiber", "getreturn", get_return),
        ("fiber", "isstarted", |it, this, _, _| is(it, this, |s| s != State::Init)),
        ("fiber", "issuspended", |it, this, _, _| is(it, this, |s| s == State::Suspended)),
        ("fiber", "isrunning", |it, this, _, _| is(it, this, |s| s == State::Running)),
        ("fiber", "isterminated", |it, this, _, _| is(it, this, |s| s == State::Terminated)),
        ("fiber", "static suspend", suspend),
        ("fiber", "static getcurrent", get_current),
    ]
}
