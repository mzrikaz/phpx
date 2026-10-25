//! Methods of built-in classes implemented in Rust: Closure, enums,
//! Promise, Concurrency.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use phpx_runtime::{Error as RtError, Promise};

use crate::class::Class;
use crate::interp::{Callable, ClosureData, ClosureKind, Ctrl, Interp, NativeMethod, Outcome, R};
use crate::value::{identical, Internal, Key, Obj, PhpArray, Value};

/// Keys: (lowercase class, lowercase method). `static ` prefix marks static
/// methods; class `*enum` applies to every enum.
pub fn table() -> HashMap<(&'static str, &'static str), NativeMethod> {
    let mut t: HashMap<(&'static str, &'static str), NativeMethod> = HashMap::new();
    t.insert(("closure", "static fromcallable"), closure_from_callable);
    t.insert(("closure", "call"), closure_call);
    t.insert(("closure", "bindto"), closure_bind_to);
    t.insert(("closure", "static bind"), closure_bind);
    t.insert(("closure", "__invoke"), closure_invoke);
    t.insert(("*enum", "static cases"), enum_cases);
    t.insert(("*enum", "static from"), enum_from);
    t.insert(("*enum", "static tryfrom"), enum_try_from);
    t.insert(("promise", "wait"), promise_wait);
    t.insert(("promise", "then"), promise_then);
    t.insert(("promise", "catch"), promise_catch);
    t.insert(("promise", "finally"), promise_finally);
    t.insert(("promise", "timeout"), promise_timeout);
    t.insert(("promise", "cancel"), promise_cancel);
    t.insert(("promise", "issettled"), promise_is_settled);
    t.insert(("promise", "static all"), promise_all);
    t.insert(("promise", "static allsettled"), promise_all_settled);
    t.insert(("promise", "static any"), promise_any);
    t.insert(("promise", "static race"), promise_race);
    for (name, f) in crate::generator::natives() {
        t.insert(("generator", name), f);
    }
    for (class, name, f) in crate::date::natives().into_iter().chain(crate::weak::natives()).chain(crate::pdo::natives()).chain(crate::fiber::natives()) {
        t.insert((class, name), f);
    }
    t.insert(("concurrency", "static run"), concurrency_run);
    t.insert(("concurrency", "static defer"), concurrency_defer);
    t
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or_default()
}

// ---- Closure ---------------------------------------------------------------

fn closure_of(o: Option<&Rc<Obj>>) -> Callable {
    match &*o.expect("closure method needs $this").internal() {
        Internal::Closure(c) => c.clone(),
        _ => unreachable!("Closure object without callable"),
    }
}

fn closure_from_callable(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let f = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    let c = it.to_callable(&f, &arg(&args, 0))?;
    Ok(it.closure_object(c))
}

fn closure_invoke(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    it.call_callable(&closure_of(this), args, vec![])
}

/// Rebind `$this` / scope. Only real closures can be rebound.
fn rebind(it: &Interp, c: &Callable, new_this: Value, scope: Option<Value>) -> R<Value> {
    let Callable::Closure(cd) = c else { return Ok(it.closure_object(c.clone())) };
    let this = match new_this {
        Value::Object(o) => Some(o),
        _ => None,
    };
    let scope_cls = match scope {
        Some(Value::Object(o)) => Some(o.class.clone()),
        Some(Value::Str(s)) if &*s == b"static" => cd.scope.clone(),
        Some(Value::Str(s)) => Some(it.lookup_class(&String::from_utf8_lossy(&s))?),
        _ => this.as_ref().map(|o| o.class.clone()).or_else(|| cd.scope.clone()),
    };
    let kind = match &cd.kind {
        ClosureKind::Closure(c) => ClosureKind::Closure(c),
        ClosureKind::Arrow(a) => ClosureKind::Arrow(a),
        ClosureKind::Wrap(w) => ClosureKind::Wrap(w.clone()),
    };
    let data = ClosureData {
        kind,
        name: cd.name.clone(),
        captured: cd.captured.clone(),
        static_class: this.as_ref().map(|o| o.class.clone()).or_else(|| scope_cls.clone()),
        this,
        scope: scope_cls,
        ctx: cd.ctx.clone(),
    };
    Ok(it.closure_object(Callable::Closure(Rc::new(data))))
}

fn closure_bind_to(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    rebind(it, &closure_of(this), arg(&args, 0), args.get(1).cloned())
}

fn closure_bind(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let Value::Object(c) = arg(&args, 0) else { return Err(it.error("TypeError", "Closure::bind(): Argument #1 must be a Closure")) };
    rebind(it, &closure_of(Some(&c)), arg(&args, 1), args.get(2).cloned())
}

fn closure_call(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, mut args: Vec<Value>) -> R<Value> {
    if args.is_empty() {
        return Err(it.error("ArgumentCountError", "Closure::call() expects at least 1 argument, 0 given"));
    }
    let new_this = args.remove(0);
    let bound = rebind(it, &closure_of(this), new_this.clone(), Some(new_this))?;
    let Value::Object(b) = bound else { unreachable!() };
    it.call_callable(&closure_of(Some(&b)), args, vec![])
}

// ---- enums -------------------------------------------------------------------

fn enum_cases(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let mut out = PhpArray::new();
    for c in &cls.cases {
        out.push(it.get_class_const(cls, c)?);
    }
    Ok(Value::array(out))
}

fn enum_lookup(it: &Interp, cls: &Rc<Class>, v: &Value) -> R<Option<Value>> {
    for c in &cls.cases {
        let Value::Object(o) = it.get_class_const(cls, c)? else { continue };
        let backing = o.get_prop("value").unwrap_or_default();
        // Coercive mode: "1" finds the case backed by 1.
        let loose_int = matches!((&backing, v), (Value::Int(b), Value::Str(s)) if crate::value::parse_numeric(s) == Some(crate::value::Num::Int(*b)));
        if identical(&backing, v) || loose_int {
            return Ok(Some(Value::Object(o)));
        }
    }
    Ok(None)
}

fn enum_from(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let v = arg(&args, 0);
    match enum_lookup(it, cls, &v)? {
        Some(c) => Ok(c),
        None => {
            let shown = match &v {
                Value::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s)),
                other => it.to_string_lossy(other)?,
            };
            Err(it.error("ValueError", &format!("{shown} is not a valid backing value for enum {}", cls.name)))
        }
    }
}

fn enum_try_from(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    Ok(enum_lookup(it, cls, &arg(&args, 0))?.unwrap_or(Value::Null))
}

// ---- Promise -----------------------------------------------------------------

pub fn promise_object(it: &Interp, p: Promise<Outcome>) -> Value {
    let cls = it.classes.borrow().get("promise").cloned().expect("Promise class");
    Value::Object(Obj::new(cls, Internal::Promise(p)))
}

fn promise_of(it: &Interp, v: &Value) -> R<Promise<Outcome>> {
    if let Value::Object(o) = v {
        if let Internal::Promise(p) = &*o.internal() {
            return Ok(p.clone());
        }
    }
    Err(it.error("TypeError", &format!("Expected Promise, {} given", v.debug_type())))
}

/// Runtime-level failures become PHP exceptions.
pub fn rt_error(it: &Interp, e: RtError) -> Ctrl {
    match e {
        RtError::Cancelled => it.error("CancelledException", "Promise was cancelled"),
        RtError::Timeout => it.error("TimeoutException", "Promise timed out"),
        RtError::Panic(m) => it.error("Error", &format!("phpx internal error: {m}")),
        RtError::Exception(m) => it.error("Exception", &m),
        RtError::Io(_, m) => it.error("RuntimeException", &m),
        RtError::AllRejected(_) => it.error("AggregateException", "All promises were rejected"),
    }
}

fn settle(it: &Interp, r: phpx_runtime::Result<Outcome>) -> Outcome {
    match r {
        Ok(o) => o,
        Err(e) => Err(rt_error(it, e)),
    }
}

/// Start `f` as a new task; its PHP result (or throw) becomes the promise.
pub fn spawn_php(it: &Interp, f: impl FnOnce(&Interp) -> Outcome + 'static) -> Promise<Outcome> {
    let rc = it.rc();
    phpx_runtime::spawn(move || Ok(f(&rc)))
}

pub fn async_call(it: &Interp, c: Callable) -> Value {
    let p = spawn_php(it, move |it| it.call_callable(&c, vec![], vec![]));
    promise_object(it, p)
}

fn promise_wait(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let p = promise_of(it, &Value::Object(this.unwrap().clone()))?;
    settle(it, p.wait())
}

fn chain(it: &Interp, this: Option<&Rc<Obj>>, args: Vec<Value>, mode: u8) -> R<Value> {
    let p = promise_of(it, &Value::Object(this.unwrap().clone()))?;
    let f = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    let cb = it.to_callable(&f, &arg(&args, 0))?;
    let np = spawn_php(it, move |it| {
        let r = settle(it, p.wait());
        match (mode, r) {
            (0, Ok(v)) => it.call_callable(&cb, vec![v], vec![]),
            (1, Err(Ctrl::Throw(e))) => it.call_callable(&cb, vec![Value::Object(e)], vec![]),
            (2, r) => {
                it.call_callable(&cb, vec![], vec![])?;
                r
            }
            (_, r) => r,
        }
    });
    Ok(promise_object(it, np))
}

fn promise_then(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    chain(it, this, args, 0)
}

fn promise_catch(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    chain(it, this, args, 1)
}

fn promise_finally(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    chain(it, this, args, 2)
}

fn promise_timeout(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let p = promise_of(it, &Value::Object(this.unwrap().clone()))?;
    let secs = it.to_float(&arg(&args, 0))?;
    Ok(promise_object(it, p.timeout(Duration::from_secs_f64(secs.max(0.0)))))
}

fn promise_cancel(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    promise_of(it, &Value::Object(this.unwrap().clone()))?.cancel();
    Ok(Value::Null)
}

fn promise_is_settled(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Bool(promise_of(it, &Value::Object(this.unwrap().clone()))?.is_settled()))
}

/// (key, promise) pairs from a PHP array of promises.
fn promise_list(it: &Interp, v: &Value) -> R<Vec<(Key, Promise<Outcome>)>> {
    let Value::Array(a) = v else { return Err(it.error("TypeError", "Expected an array of promises")) };
    a.iter().map(|(k, v)| Ok((k.clone(), promise_of(it, &v)?))).collect()
}

/// Turn PHP throws into runtime errors so runtime combinators fail fast,
/// keeping the original exception in `first`.
fn lift(p: &Promise<Outcome>, first: &Rc<RefCell<Option<Ctrl>>>) -> Promise<Value> {
    let first = first.clone();
    p.then(move |o| match o {
        Ok(v) => Ok(v),
        Err(c) => {
            first.borrow_mut().get_or_insert(c);
            Err(RtError::exception("__php_throw__"))
        }
    })
}

fn all_impl(it: &Interp, list: Vec<(Key, Promise<Outcome>)>) -> Outcome {
    let first = Rc::new(RefCell::new(None));
    let lifted: Vec<_> = list.iter().map(|(_, p)| lift(p, &first)).collect();
    match Promise::all(lifted).wait() {
        Ok(vals) => {
            let mut out = PhpArray::new();
            for ((k, _), v) in list.into_iter().zip(vals) {
                out.insert(k, v);
            }
            Ok(Value::array(out))
        }
        Err(RtError::Exception(m)) if m == "__php_throw__" => Err(first.borrow_mut().take().unwrap()),
        Err(e) => Err(rt_error(it, e)),
    }
}

fn promise_all(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let list = promise_list(it, &arg(&args, 0))?;
    Ok(promise_object(it, spawn_php(it, move |it| all_impl(it, list))))
}

fn promise_all_settled(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let list = promise_list(it, &arg(&args, 0))?;
    let np = spawn_php(it, move |it| {
        let mut out = PhpArray::new();
        for (k, p) in list {
            let mut entry = PhpArray::new();
            match settle(it, p.wait()) {
                Ok(v) => {
                    entry.insert(Key::Str(crate::value::pbytes(b"state")), Value::str("fulfilled"));
                    entry.insert(Key::Str(crate::value::pbytes(b"value")), v);
                }
                Err(Ctrl::Throw(e)) => {
                    entry.insert(Key::Str(crate::value::pbytes(b"state")), Value::str("rejected"));
                    entry.insert(Key::Str(crate::value::pbytes(b"reason")), Value::Object(e));
                }
                Err(exit) => return Err(exit),
            }
            out.insert(k, Value::array(entry));
        }
        Ok(Value::array(out))
    });
    Ok(promise_object(it, np))
}

fn promise_any(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let list = promise_list(it, &arg(&args, 0))?;
    let np = spawn_php(it, move |it| {
        let errors: Rc<RefCell<Vec<Value>>> = Rc::new(RefCell::new(vec![]));
        let lifted: Vec<Promise<Value>> = list
            .iter()
            .map(|(_, p)| {
                let errors = errors.clone();
                p.then(move |o| match o {
                    Ok(v) => Ok(v),
                    Err(Ctrl::Throw(e)) => {
                        errors.borrow_mut().push(Value::Object(e));
                        Err(RtError::exception("__php_throw__"))
                    }
                    Err(Ctrl::Exit(_) | Ctrl::GenClose) => Err(RtError::exception("__php_exit__")),
                })
            })
            .collect();
        match Promise::any(lifted).wait() {
            Ok(v) => Ok(v),
            Err(RtError::AllRejected(_)) => {
                let Ctrl::Throw(agg) = it.error("AggregateException", "All promises were rejected") else { unreachable!() };
                agg.set_prop("errors", Value::array(PhpArray::from_list(errors.borrow().clone())));
                Err(Ctrl::Throw(agg))
            }
            Err(e) => Err(rt_error(it, e)),
        }
    });
    Ok(promise_object(it, np))
}

fn promise_race(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let list = promise_list(it, &arg(&args, 0))?;
    let np = spawn_php(it, move |it| {
        let first = Rc::new(RefCell::new(None));
        let lifted: Vec<_> = list.iter().map(|(_, p)| lift(p, &first)).collect();
        match Promise::race(lifted).wait() {
            Ok(v) => Ok(v),
            Err(RtError::Exception(m)) if m == "__php_throw__" => Err(first.borrow_mut().take().unwrap()),
            Err(e) => Err(rt_error(it, e)),
        }
    });
    Ok(promise_object(it, np))
}

// ---- Concurrency ---------------------------------------------------------------

fn callables(it: &Interp, v: &Value) -> R<Vec<(Key, Callable)>> {
    let Value::Array(a) = v else { return Err(it.error("TypeError", "Concurrency expects an array of closures")) };
    let f = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    a.iter().map(|(k, v)| Ok((k.clone(), it.to_callable(&f, &v)?))).collect()
}

fn concurrency_run(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    let tasks = callables(it, &arg(&args, 0))?;
    let list: Vec<(Key, Promise<Outcome>)> = tasks
        .into_iter()
        .map(|(k, c)| (k, spawn_php(it, move |it| it.call_callable(&c, vec![], vec![]))))
        .collect();
    all_impl(it, list)
}

fn concurrency_defer(it: &Interp, _: Option<&Rc<Obj>>, _: &Rc<Class>, args: Vec<Value>) -> R<Value> {
    for (_, c) in callables(it, &arg(&args, 0))? {
        defer_call(it, c);
    }
    Ok(Value::Null)
}

pub fn defer_call(it: &Interp, c: Callable) {
    let rc = it.rc();
    phpx_runtime::defer(move || {
        if let Err(Ctrl::Throw(e)) = rc.call_callable(&c, vec![], vec![]) {
            eprintln!("{}", rc.uncaught_message(&e));
        }
    });
}
