//! PHP's error reporting: set_error_handler() / set_exception_handler()
//! stacks, error_reporting() levels, `@`, error_get_last(), and the
//! default "PHP Warning:  ..." output.
//!
//! A handler that throws does so the way PHP does: the error site (often
//! inside a built-in) carries on, and the exception surfaces at the next
//! check (built-in return, compiled helper status, statement end).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::interp::{BuiltinFn, Callable, Ctrl, Frame, Interp, R};
use crate::value::{Obj, PhpArray, Value};
use std::rc::Rc;

pub const E_ERROR: i64 = 1;
pub const E_WARNING: i64 = 2;
pub const E_NOTICE: i64 = 8;
pub const E_USER_ERROR: i64 = 256;
pub const E_USER_WARNING: i64 = 512;
pub const E_USER_NOTICE: i64 = 1024;
pub const E_RECOVERABLE_ERROR: i64 = 4096;
pub const E_DEPRECATED: i64 = 8192;
pub const E_USER_DEPRECATED: i64 = 16384;
pub const E_ALL: i64 = 30719;
/// What error_reporting() reports inside `@`.
const SILENCED_MASK: i64 = 4437;

struct Handler {
    value: Value,
    callable: Option<Callable>,
    levels: i64,
}

/// Threads with a `RAISED` exception: `take()` runs after nearly every
/// operation, and a plain load is cheaper than a thread-local access
/// (a function call on macOS).
static ANY_RAISED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ERROR_HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
    static EXCEPTION_HANDLERS: RefCell<Vec<(Value, Option<Callable>)>> = const { RefCell::new(Vec::new()) };
    static LAST: RefCell<Option<(i64, String, String, i64)>> = const { RefCell::new(None) };
    /// An exception a handler threw, waiting to surface.
    static RAISED: RefCell<Option<Ctrl>> = const { RefCell::new(None) };
    static IN_HANDLER: Cell<bool> = const { Cell::new(false) };
}

fn label(level: i64) -> &'static str {
    match level {
        E_WARNING | E_USER_WARNING => "Warning",
        E_NOTICE | E_USER_NOTICE => "Notice",
        E_DEPRECATED | E_USER_DEPRECATED => "Deprecated",
        E_USER_ERROR | E_ERROR => "Fatal error",
        E_RECOVERABLE_ERROR => "Recoverable fatal error",
        _ => "Warning",
    }
}

/// Forget one request's handlers and last error (server workers).
pub fn reset() {
    ERROR_HANDLERS.with(|h| h.borrow_mut().clear());
    EXCEPTION_HANDLERS.with(|h| h.borrow_mut().clear());
    LAST.with(|l| *l.borrow_mut() = None);
    if RAISED.with(|r| r.borrow_mut().take()).is_some() {
        ANY_RAISED.fetch_sub(1, Ordering::Relaxed);
    }
    IN_HANDLER.with(|h| h.set(false));
}

/// error_reporting(), as seen from PHP code (masked inside `@`).
pub fn reporting(it: &Interp) -> i64 {
    let level = it.ini_get("error_reporting").and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(E_ALL);
    if it.silence.get() > 0 {
        level & SILENCED_MASK
    } else {
        level
    }
}

/// Raise an error of `level`. Returns true when a user handler took it.
pub fn raise(it: &Interp, level: i64, msg: &str) -> bool {
    let (file, line) = it.here();
    raise_at(it, level, msg, &file, line as i64)
}

pub fn raise_at(it: &Interp, level: i64, msg: &str, file: &str, line: i64) -> bool {
    LAST.with(|l| *l.borrow_mut() = Some((level, msg.to_string(), file.to_string(), line)));
    if !IN_HANDLER.with(|h| h.get()) {
        let handler = ERROR_HANDLERS.with(|h| h.borrow().last().filter(|h| h.levels & level != 0).and_then(|h| h.callable.clone()));
        if let Some(c) = handler {
            IN_HANDLER.with(|h| h.set(true));
            let r = it.call_callable(&c, vec![Value::Int(level), Value::str(msg), Value::str(file), Value::Int(line)], vec![]);
            IN_HANDLER.with(|h| h.set(false));
            match r {
                Ok(Value::Bool(false)) => {}
                Ok(_) => return true,
                Err(e) => {
                    defer(e);
                    return true;
                }
            }
        }
    }
    if it.silence.get() > 0 || reporting(it) & level == 0 {
        return false;
    }
    it.flush();
    eprintln!("PHP {}:  {msg} in {file} on line {line}", label(level));
    false
}

/// Keep a handler's exception for the next check (the first one wins).
pub fn defer(e: Ctrl) {
    RAISED.with(|r| {
        let mut r = r.borrow_mut();
        if r.is_none() {
            *r = Some(e);
            ANY_RAISED.fetch_add(1, Ordering::Relaxed);
        }
    });
}

/// The exception an error handler threw, if any.
#[inline]
pub fn take() -> Option<Ctrl> {
    if ANY_RAISED.load(Ordering::Relaxed) == 0 {
        return None;
    }
    let e = RAISED.with(|r| r.borrow_mut().take());
    if e.is_some() {
        ANY_RAISED.fetch_sub(1, Ordering::Relaxed);
    }
    e
}

/// `r`, unless an error handler threw meanwhile.
#[inline]
pub fn check<T>(r: R<T>) -> R<T> {
    match take() {
        Some(e) if r.is_ok() => Err(e),
        _ => r,
    }
}

/// An uncaught exception: the exception handler if one is set, else the
/// fatal error message. Returns the exit code.
pub fn uncaught(it: &Interp, e: Rc<Obj>) -> i32 {
    let handler = EXCEPTION_HANDLERS.with(|h| h.borrow_mut().pop()).and_then(|h| h.1);
    if let Some(c) = handler {
        EXCEPTION_HANDLERS.with(|h| h.borrow_mut().clear());
        match it.call_callable(&c, vec![Value::Object(e)], vec![]) {
            Ok(_) => return 0,
            Err(Ctrl::Exit(code)) => return code,
            Err(Ctrl::Throw(e2)) => {
                it.flush();
                eprintln!("{}", it.uncaught_message(&e2));
                return 255;
            }
            Err(_) => return 255,
        }
    }
    it.flush();
    eprintln!("{}", it.uncaught_message(&e));
    255
}

fn callable_arg(it: &Interp, f: &Frame, v: &Value, func: &str) -> R<Option<Callable>> {
    if v.is_null() {
        return Ok(None);
    }
    match it.to_callable(f, v) {
        Ok(c) => Ok(Some(c)),
        Err(_) => Err(it.error("TypeError", &format!("{func}(): Argument #1 ($callback) must be a valid callback or null, no array or string given"))),
    }
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("set_error_handler", |it, f, a| {
        let v = a.first().cloned().unwrap_or_default().into_deref();
        let callable = callable_arg(it, f, &v, "set_error_handler")?;
        let levels = match a.get(1) {
            Some(l) => it.to_int(l)?,
            None => E_ALL,
        };
        Ok(ERROR_HANDLERS.with(|h| {
            let mut h = h.borrow_mut();
            let prev = h.last().map(|x| x.value.clone()).unwrap_or_default();
            h.push(Handler { value: v, callable, levels });
            prev
        }))
    });
    t.insert("restore_error_handler", |_, _, _| {
        ERROR_HANDLERS.with(|h| h.borrow_mut().pop());
        Ok(Value::Bool(true))
    });
    t.insert("get_error_handler", |_, _, _| Ok(ERROR_HANDLERS.with(|h| h.borrow().last().map(|x| x.value.clone()).unwrap_or_default())));
    t.insert("set_exception_handler", |it, f, a| {
        let v = a.first().cloned().unwrap_or_default().into_deref();
        let callable = callable_arg(it, f, &v, "set_exception_handler")?;
        Ok(EXCEPTION_HANDLERS.with(|h| {
            let mut h = h.borrow_mut();
            let prev = h.last().map(|x| x.0.clone()).unwrap_or_default();
            h.push((v, callable));
            prev
        }))
    });
    t.insert("restore_exception_handler", |_, _, _| {
        EXCEPTION_HANDLERS.with(|h| h.borrow_mut().pop());
        Ok(Value::Bool(true))
    });
    t.insert("get_exception_handler", |_, _, _| Ok(EXCEPTION_HANDLERS.with(|h| h.borrow().last().map(|x| x.0.clone()).unwrap_or_default())));
    t.insert("error_get_last", |_, _, _| {
        Ok(LAST.with(|l| match &*l.borrow() {
            Some((ty, msg, file, line)) => {
                let mut a = PhpArray::new();
                a.insert(crate::value::Key::from_bytes(b"type"), Value::Int(*ty));
                a.insert(crate::value::Key::from_bytes(b"message"), Value::str(msg));
                a.insert(crate::value::Key::from_bytes(b"file"), Value::str(file));
                a.insert(crate::value::Key::from_bytes(b"line"), Value::Int(*line));
                Value::array(a)
            }
            None => Value::Null,
        }))
    });
    t.insert("error_clear_last", |_, _, _| {
        LAST.with(|l| *l.borrow_mut() = None);
        Ok(Value::Null)
    });
    t.insert("error_reporting", |it, _, a| {
        let old = reporting(it);
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            let v = it.to_int(v)?;
            it.ini.borrow_mut().set("error_reporting", &v.to_string());
        }
        Ok(Value::Int(old))
    });
    let trigger: BuiltinFn = |it, _, a| {
        let msg = crate::pstr::lossy(&crate::builtins::s(it, a, 0)?).into_owned();
        let level = if a.len() > 1 { it.to_int(&a[1])? } else { E_USER_NOTICE };
        if !matches!(level, E_USER_ERROR | E_USER_WARNING | E_USER_NOTICE | E_USER_DEPRECATED) {
            return Err(it.error("ValueError", "trigger_error(): Argument #2 ($error_level) must be one of E_USER_ERROR, E_USER_WARNING, E_USER_NOTICE, or E_USER_DEPRECATED"));
        }
        if level == E_USER_ERROR {
            raise(it, E_DEPRECATED, "Passing E_USER_ERROR to trigger_error() is deprecated since 8.4, throw an exception or call exit with a string message instead");
            if !raise(it, level, &msg) {
                return Err(Ctrl::Exit(255));
            }
            return Ok(Value::Bool(true));
        }
        raise(it, level, &msg);
        Ok(Value::Bool(true))
    };
    t.insert("trigger_error", trigger);
    t.insert("user_error", trigger);
}
