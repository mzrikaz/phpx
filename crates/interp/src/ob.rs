//! Output buffering with handlers: ob_start($callback, $chunk_size,
//! $flags) and the ob_* functions that flush, clean or inspect buffers.
//! The buffers themselves are `Interp::out` (level 0 is the unbuffered
//! output); this keeps each level's handler alongside.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::interp::{BuiltinFn, Callable, Interp, R};
use crate::value::{Key, PhpArray, Value};

const START: i64 = 1;
const CLEAN: i64 = 2;
const FLUSH: i64 = 4;
const FINAL: i64 = 8;
/// PHP_OUTPUT_HANDLER_STDFLAGS (cleanable | flushable | removable).
const STDFLAGS: i64 = 0x70;
const STARTED: i64 = 0x1000;

#[derive(Clone)]
struct Handler {
    callable: Option<Callable>,
    name: String,
    chunk_size: i64,
    flags: i64,
    started: bool,
}

thread_local! {
    /// Handlers of ob levels 1..; index = level - 1.
    static HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
}

fn levels(it: &Interp) -> usize {
    it.out.borrow().len() - 1
}

/// The handler of the top level (a default one if the level was pushed
/// without ob_start()).
fn top(it: &Interp) -> Handler {
    let n = levels(it);
    HANDLERS.with(|h| {
        let mut h = h.borrow_mut();
        h.truncate(n);
        while h.len() < n {
            h.push(Handler { callable: None, name: "default output handler".into(), chunk_size: 0, flags: STDFLAGS, started: false });
        }
        h.last().cloned().unwrap_or(Handler { callable: None, name: "default output handler".into(), chunk_size: 0, flags: STDFLAGS, started: false })
    })
}

fn callable_name(c: &Callable) -> String {
    match c {
        Callable::User(u) => u.name.to_string(),
        Callable::Builtin(n, _) => n.to_string(),
        Callable::Closure(c) => c.name.to_string(),
        Callable::Method { class, method, .. } => format!("{}::{}", class.name, method.name),
    }
}

/// Run the top level's handler over `buf` with `mode`; the text to pass on.
fn apply(it: &Interp, buf: Vec<u8>, mode: i64) -> R<Vec<u8>> {
    let h = top(it);
    let Some(c) = &h.callable else { return Ok(buf) };
    let mode = mode | if h.started { 0 } else { START };
    HANDLERS.with(|hs| {
        if let Some(x) = hs.borrow_mut().last_mut() {
            x.started = true;
        }
    });
    let r = it.call_callable(c, vec![Value::bytes(buf.clone()), Value::Int(mode)], vec![])?;
    Ok(match r {
        Value::Bool(false) => buf,
        v => it.to_str(&v)?.to_vec(),
    })
}

fn pop(it: &Interp) -> Vec<u8> {
    HANDLERS.with(|h| {
        let n = levels(it);
        h.borrow_mut().truncate(n.saturating_sub(1));
    });
    it.out.borrow_mut().pop().unwrap_or_default()
}

fn append_parent(it: &Interp, data: &[u8]) {
    if levels(it) == 0 {
        it.echo(data);
    } else {
        it.out.borrow_mut().last_mut().unwrap().extend_from_slice(data);
    }
}

fn no_buffer(it: &Interp, func: &str, what: &str) -> Value {
    let msg = match what {
        "delete" => "Failed to delete buffer. No buffer to delete",
        "flush" => "Failed to flush buffer. No buffer to flush",
        _ => "Failed to delete and flush buffer. No buffer to delete or flush",
    };
    it.notice(&format!("{func}(): {msg}"));
    Value::Bool(false)
}

/// ob_end_flush() for every open level (end of the script).
pub fn end_all(it: &Interp) -> R<()> {
    while levels(it) > 0 {
        top(it);
        let buf = std::mem::take(it.out.borrow_mut().last_mut().unwrap());
        let out = apply(it, buf, FINAL)?;
        pop(it);
        append_parent(it, &out);
    }
    Ok(())
}

fn status(it: &Interp, level: usize, h: &Handler) -> Value {
    let used = it.out.borrow().get(level + 1).map_or(0, Vec::len);
    let mut a = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        a.insert(Key::from_bytes(k.as_bytes()), v);
    };
    put("name", Value::str(&h.name));
    put("type", Value::Int(if h.callable.is_some() { 1 } else { 0 }));
    put("flags", Value::Int(h.flags | h.callable.is_some() as i64 | if h.started { STARTED } else { 0 }));
    put("level", Value::Int(level as i64));
    put("chunk_size", Value::Int(h.chunk_size));
    put("buffer_size", Value::Int(if h.chunk_size > 1 { (h.chunk_size / 4096 + 1) * 4096 } else { 16384 }));
    put("buffer_used", Value::Int(used as i64));
    Value::array(a)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("ob_start", |it, f, a| {
        let callable = match a.first().map(|v| v.deref()) {
            None | Some(Value::Null) => None,
            Some(v) => match it.to_callable(f, &v) {
                Ok(c) => Some(c),
                Err(_) => {
                    it.warn("ob_start(): no array or string given");
                    it.notice("ob_start(): Failed to create buffer");
                    return Ok(Value::Bool(false));
                }
            },
        };
        let chunk_size = if a.len() > 1 { it.to_int(&a[1])?.max(0) } else { 0 };
        let flags = if a.len() > 2 { it.to_int(&a[2])? & STDFLAGS } else { STDFLAGS };
        let name = callable.as_ref().map_or_else(|| "default output handler".to_string(), callable_name);
        top(it);
        it.out.borrow_mut().push(vec![]);
        HANDLERS.with(|h| h.borrow_mut().push(Handler { callable, name, chunk_size, flags, started: false }));
        Ok(Value::Bool(true))
    });
    t.insert("ob_get_clean", |it, _, _| {
        if levels(it) == 0 {
            return Ok(Value::Bool(false));
        }
        let buf = std::mem::take(it.out.borrow_mut().last_mut().unwrap());
        apply(it, buf.clone(), CLEAN | FINAL)?;
        pop(it);
        Ok(Value::bytes(buf))
    });
    t.insert("ob_get_flush", |it, _, _| {
        if levels(it) == 0 {
            return Ok(no_buffer(it, "ob_get_flush", "delete and flush"));
        }
        let buf = std::mem::take(it.out.borrow_mut().last_mut().unwrap());
        let out = apply(it, buf.clone(), FINAL)?;
        pop(it);
        append_parent(it, &out);
        Ok(Value::bytes(buf))
    });
    t.insert("ob_end_clean", |it, _, _| {
        if levels(it) == 0 {
            return Ok(no_buffer(it, "ob_end_clean", "delete"));
        }
        let buf = std::mem::take(it.out.borrow_mut().last_mut().unwrap());
        apply(it, buf, CLEAN | FINAL)?;
        pop(it);
        Ok(Value::Bool(true))
    });
    t.insert("ob_end_flush", |it, _, _| {
        if levels(it) == 0 {
            return Ok(no_buffer(it, "ob_end_flush", "delete and flush"));
        }
        let buf = std::mem::take(it.out.borrow_mut().last_mut().unwrap());
        let out = apply(it, buf, FINAL)?;
        pop(it);
        append_parent(it, &out);
        Ok(Value::Bool(true))
    });
    t.insert("ob_flush", |it, _, _| {
        if levels(it) == 0 {
            return Ok(no_buffer(it, "ob_flush", "flush"));
        }
        let buf = std::mem::take(it.out.borrow_mut().last_mut().unwrap());
        let out = apply(it, buf, FLUSH)?;
        // The level stays; its output goes to the one below.
        let n = levels(it);
        if n == 1 {
            it.out.borrow_mut()[0].extend_from_slice(&out);
        } else {
            it.out.borrow_mut()[n - 1].extend_from_slice(&out);
        }
        Ok(Value::Bool(true))
    });
    t.insert("ob_clean", |it, _, _| {
        if levels(it) == 0 {
            return Ok(no_buffer(it, "ob_clean", "delete"));
        }
        let buf = std::mem::take(it.out.borrow_mut().last_mut().unwrap());
        apply(it, buf, CLEAN)?;
        Ok(Value::Bool(true))
    });
    t.insert("ob_get_length", |it, _, _| {
        if levels(it) == 0 {
            return Ok(Value::Bool(false));
        }
        Ok(Value::Int(it.out.borrow().last().map_or(0, Vec::len) as i64))
    });
    t.insert("ob_list_handlers", |it, _, _| {
        top_sync(it);
        Ok(HANDLERS.with(|h| Value::array(PhpArray::from_list(h.borrow().iter().map(|x| Value::str(&x.name)).collect::<Vec<_>>()))))
    });
    t.insert("ob_get_status", |it, _, a| {
        top_sync(it);
        let full = a.first().is_some_and(|v| v.truthy());
        let hs = HANDLERS.with(|h| h.borrow().clone());
        if full {
            return Ok(Value::array(PhpArray::from_list(hs.iter().enumerate().map(|(i, h)| status(it, i, h)).collect::<Vec<_>>())));
        }
        Ok(match hs.last() {
            Some(h) => status(it, hs.len() - 1, h),
            None => Value::array(PhpArray::new()),
        })
    });
    t.insert("ob_implicit_flush", |_, _, _| Ok(Value::Null));
    t.insert("output_add_rewrite_var", |_, _, _| Ok(Value::Bool(true)));
    t.insert("output_reset_rewrite_vars", |_, _, _| Ok(Value::Bool(true)));
}

fn top_sync(it: &Interp) {
    if levels(it) > 0 {
        top(it);
    } else {
        HANDLERS.with(|h| h.borrow_mut().clear());
    }
}
