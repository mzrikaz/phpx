//! The PHP call stack: debug_backtrace(), exception traces and the "Stack
//! trace:" of uncaught exceptions.
//!
//! Every running function body (interpreted or compiled) links a record
//! on its own native stack into a list whose head is the green thread's
//! task word, so each task has its own stack and nothing is allocated.

use std::rc::Rc;

use phpx_syntax::ast;

use crate::compiler::rt::LocalsSpec;
use crate::interp::{Frame, Interp, Pos, PRELUDE_FILE};
use crate::value::{pstr, Key, PhpArray, Value};

#[repr(C)]
pub struct CallRec {
    pub prev: *const CallRec,
    pub frame: *const Frame,
    /// Compiled bodies: their locals (parameters first) and spec.
    pub spec: *const LocalsSpec,
    pub locals: *const Value,
    /// Interpreted bodies: the parameters (values are in the frame).
    pub params: *const ast::Param,
    pub nparams: usize,
    /// Where the call happened.
    pub pos: Pos,
    /// The callee is a built-in class/function written in PHP (prelude).
    pub internal: bool,
    /// The interpreter's current position: restored to `pos` on return.
    pub cur: *const std::cell::Cell<Pos>,
}

impl CallRec {
    pub const fn empty() -> CallRec {
        CallRec {
            prev: std::ptr::null(),
            frame: std::ptr::null(),
            spec: std::ptr::null(),
            locals: std::ptr::null(),
            params: std::ptr::null(),
            nparams: 0,
            pos: Pos { file: 0, off: 0 },
            internal: false,
            cur: std::ptr::null(),
        }
    }
}

#[inline(always)]
pub fn top() -> *const CallRec {
    phpx_runtime::task_word() as *const CallRec
}

/// Link `rec` (which must stay put until `pop`) as the innermost call.
#[inline(always)]
pub fn push(rec: &mut CallRec) {
    rec.prev = top();
    phpx_runtime::set_task_word(rec as *const CallRec as usize);
}

#[inline(always)]
pub fn pop(rec: &CallRec) {
    phpx_runtime::set_task_word(rec.prev as usize);
    // Back in the caller: diagnostics point at its statement again.
    if let Some(cur) = unsafe { rec.cur.as_ref() } {
        cur.set(rec.pos);
    }
}

/// Pops on drop (interpreted calls, including the error path).
pub struct Guard(pub Box<CallRec>);

impl Drop for Guard {
    fn drop(&mut self) {
        pop(&self.0);
    }
}

/// An interpreted call: push a record for `f` called from `pos`.
pub fn enter(it: &Interp, f: &Frame, params: &'static [ast::Param], pos: Pos) -> Guard {
    let mut rec = Box::new(CallRec {
        frame: f as *const Frame,
        params: params.as_ptr(),
        nparams: params.len(),
        pos,
        internal: f.ctx.file == PRELUDE_FILE,
        cur: &it.pos,
        ..CallRec::empty()
    });
    push(&mut rec);
    Guard(rec)
}

/// Compiled prologue: record the call (the frame is the callee's Cx frame).
pub unsafe extern "C" fn rt_call_push(c: *mut crate::compiler::rt::Cx, spec: *const LocalsSpec, locals: *const Value, rec: *mut CallRec) {
    let it = &*(*c).it;
    let rec = &mut *rec;
    *rec = CallRec {
        frame: (*c).frame,
        spec,
        locals,
        pos: it.pos.get(),
        internal: (&*spec).ctx.file == PRELUDE_FILE,
        cur: &it.pos,
        ..CallRec::empty()
    };
    push(rec);
}

pub unsafe extern "C" fn rt_call_pop(rec: *const CallRec) {
    pop(&*rec);
}

/// Arguments of a record (current parameter values, like PHP).
unsafe fn args_of(r: &CallRec) -> Vec<Value> {
    if !r.spec.is_null() {
        let spec = &*r.spec;
        return (0..spec.params.len()).map(|i| (*r.locals.add(i)).deref()).collect();
    }
    if r.params.is_null() || r.frame.is_null() {
        return vec![];
    }
    let f = &*r.frame;
    let params = std::slice::from_raw_parts(r.params, r.nparams);
    let mut out = vec![];
    for p in params {
        match f.vars.get(&p.name.name) {
            Some(v) if p.variadic => {
                if let Value::Array(a) = v.borrow().deref() {
                    out.extend(a.iter().map(|(_, v)| v.clone()));
                }
            }
            Some(v) => out.push(v.borrow().deref()),
            None => {}
        }
    }
    out
}

/// One stack entry as PHP describes it.
pub struct Entry {
    pub file: Option<String>,
    pub line: usize,
    pub function: Rc<str>,
    pub class: Option<Rc<str>>,
    pub object: Option<Value>,
    pub args: Vec<Value>,
}

/// The call stack, innermost first (built-in code nested in built-in code
/// is hidden, as PHP's internal frames are).
pub fn entries(it: &Interp, want_args: bool, limit: usize) -> Vec<Entry> {
    let mut out = vec![];
    let mut r = top();
    unsafe {
        while let Some(rec) = r.as_ref() {
            let caller_internal = rec.prev.as_ref().is_some_and(|p| p.internal);
            if !(rec.internal && caller_internal) && !rec.frame.is_null() {
                let f = &*rec.frame;
                let file = if rec.pos.file as usize == PRELUDE_FILE { None } else { Some(it.file_path(rec.pos.file as usize)) };
                let line = if file.is_some() { it.line_of(rec.pos.file as usize, rec.pos.off) } else { 0 };
                let class = match &f.class {
                    Some(c) => Some(c.name.clone()),
                    None if !rec.spec.is_null() => (*rec.spec).class.as_ref().map(|c| c.name.clone()),
                    None => None,
                };
                out.push(Entry {
                    file,
                    line,
                    function: f.func.clone(),
                    class,
                    object: f.this.clone().map(Value::Object),
                    args: if want_args { args_of(rec) } else { vec![] },
                });
                if limit > 0 && out.len() >= limit {
                    break;
                }
            }
            r = rec.prev;
        }
    }
    out
}

fn put(a: &mut PhpArray, k: &str, v: Value) {
    a.insert(Key::Str(pstr(k)), v);
}

/// debug_backtrace() / Exception::getTrace() arrays.
pub fn as_array(entries: &[Entry], with_object: bool, with_args: bool) -> Value {
    let mut list = PhpArray::new();
    for e in entries {
        let mut a = PhpArray::new();
        if let Some(f) = &e.file {
            put(&mut a, "file", Value::str(f));
            put(&mut a, "line", Value::Int(e.line as i64));
        }
        put(&mut a, "function", Value::str(&e.function));
        if let Some(c) = &e.class {
            put(&mut a, "class", Value::str(c));
            if with_object {
                if let Some(o) = &e.object {
                    put(&mut a, "object", o.clone());
                }
            }
            put(&mut a, "type", Value::str(if e.object.is_some() { "->" } else { "::" }));
        }
        if with_args {
            put(&mut a, "args", Value::array(PhpArray::from_list(e.args.iter().cloned())));
        }
        list.push(Value::array(a));
    }
    Value::array(list)
}

/// An argument in a trace string: `'abc'`, `1`, `Array`, `Object(Foo)`.
fn arg_str(v: &Value, max: usize) -> String {
    match v.deref() {
        Value::Null => "NULL".into(),
        Value::Bool(b) => if b { "true".into() } else { "false".into() },
        Value::Int(n) => n.to_string(),
        Value::Float(f) => crate::value::fmt_float_repr(f),
        Value::Str(s) => {
            let t = String::from_utf8_lossy(&s);
            if s.len() > max {
                format!("'{}...'", String::from_utf8_lossy(&s[..max]))
            } else {
                format!("'{t}'")
            }
        }
        Value::Array(_) => "Array".into(),
        Value::Object(o) => format!("Object({})", o.class.name),
        Value::Resource(r) => format!("Resource id #{}", r.id),
        Value::Ref(_) => unreachable!(),
    }
}

/// getTraceAsString() of a trace array.
pub fn trace_string(trace: &Value) -> String {
    let mut out = String::new();
    let mut n = 0;
    if let Value::Array(a) = trace {
        for (_, e) in a.iter() {
            let Value::Array(e) = e else { continue };
            let get = |k: &str| e.get(&Key::Str(pstr(k)));
            let s = |k: &str| get(k).and_then(|v| v.scalar_bytes()).map(|b| String::from_utf8_lossy(&b).into_owned());
            let place = match (s("file"), s("line")) {
                (Some(f), Some(l)) => format!("{f}({l})"),
                _ => "[internal function]".into(),
            };
            let func = format!("{}{}{}", s("class").unwrap_or_default(), s("type").unwrap_or_default(), s("function").unwrap_or_default());
            let args = match get("args") {
                Some(Value::Array(a)) => a.iter().map(|(_, v)| arg_str(&v, 15)).collect::<Vec<_>>().join(", "),
                _ => String::new(),
            };
            out.push_str(&format!("#{n} {place}: {func}({args})\n"));
            n += 1;
        }
    }
    out.push_str(&format!("#{n} {{main}}"));
    out
}
