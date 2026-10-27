//! Built-in function signatures, taken from PHP's own Reflection (see
//! `tools/arginfo`): argument counts, parameter types with PHP's coercion
//! rules (TypeError, ArgumentCountError, the null deprecation), and named
//! arguments.
//!
//! Checks run before a built-in is called, so the built-ins themselves can
//! assume PHP has already validated their arguments' types.

use std::sync::OnceLock;

use rustc_hash::FxHashMap;

use crate::interp::{Frame, Interp, R};
use crate::value::{parse_numeric, Value};

const NULL: u16 = 1;
const FALSE: u16 = 1 << 1;
const TRUE: u16 = 1 << 2;
const INT: u16 = 1 << 3;
const FLOAT: u16 = 1 << 4;
const STRING: u16 = 1 << 5;
const ARRAY: u16 = 1 << 6;
const OBJECT: u16 = 1 << 7;
const CALLABLE: u16 = 1 << 8;
const ITERABLE: u16 = 1 << 9;
const MIXED: u16 = 1 << 10;
const BOOL: u16 = FALSE | TRUE;
const SCALAR: u16 = BOOL | INT | FLOAT | STRING;

/// " ($name)" after "Argument #n" in messages; nothing for a variadic
/// parameter (PHP doesn't name those).
fn arg_label(p: &Param) -> String {
    if p.variadic { String::new() } else { format!(" (${})", p.name) }
}

pub struct Param {
    pub name: &'static str,
    /// PHP's type string, for messages.
    pub ty: &'static str,
    mask: u16,
    /// Class and interface names in the type (lowercase).
    classes: Vec<String>,
    pub optional: bool,
    pub variadic: bool,
    /// The default value's PHP source ("" = not known).
    pub default: &'static str,
}

pub struct Spec {
    pub name: &'static str,
    pub params: Vec<Param>,
    /// Required argument count, and the most allowed (`usize::MAX` with a
    /// variadic parameter).
    pub min: usize,
    pub max: usize,
}

fn parse_type(ty: &str) -> (u16, Vec<String>) {
    if ty.is_empty() {
        return (MIXED, vec![]);
    }
    let (ty, nullable) = match ty.strip_prefix('?') {
        Some(t) => (t, true),
        None => (ty, false),
    };
    let mut mask = if nullable { NULL } else { 0 };
    let mut classes = vec![];
    for part in ty.split('|') {
        mask |= match part.to_ascii_lowercase().as_str() {
            "null" => NULL,
            "bool" => BOOL,
            "false" => FALSE,
            "true" => TRUE,
            "int" => INT,
            "float" => FLOAT,
            "string" => STRING,
            "array" => ARRAY,
            "object" => OBJECT,
            "callable" => CALLABLE,
            "iterable" => ITERABLE,
            "mixed" => MIXED,
            other => {
                classes.push(other.trim_start_matches('\\').to_string());
                0
            }
        };
    }
    (mask, classes)
}

fn build() -> FxHashMap<&'static str, Spec> {
    crate::arginfo_table::FUNCS
        .iter()
        .map(|(name, ps)| {
            let params: Vec<Param> = ps
                .iter()
                .map(|&(name, ty, flags, default)| {
                    let (mask, classes) = parse_type(ty);
                    Param { name, ty, mask, classes, optional: flags & 1 != 0, variadic: flags & 4 != 0, default }
                })
                .collect();
            let min = params.iter().filter(|p| !p.optional && !p.variadic).count();
            let max = if params.iter().any(|p| p.variadic) { usize::MAX } else { params.len() };
            (*name, Spec { name, params, min, max })
        })
        .collect()
}

/// The signature of built-in `name` (None: not a PHP function, e.g. a
/// phpx-only built-in).
pub fn spec(name: &str) -> Option<&'static Spec> {
    static SPECS: OnceLock<FxHashMap<&'static str, Spec>> = OnceLock::new();
    SPECS.get_or_init(build).get(name)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "argument" } else { "arguments" }
}

impl Spec {
    fn param(&self, i: usize) -> Option<&Param> {
        self.params.get(i).or_else(|| self.params.last().filter(|p| p.variadic))
    }

    /// Check (and where PHP converts, convert) `args` for a call.
    #[inline]
    /// `f` is the calling frame (callbacks are checked from its scope).
    pub fn check(&self, it: &Interp, f: &Frame, args: &mut [Value]) -> R<()> {
        let n = args.len();
        if n < self.min || n > self.max {
            return Err(self.count_error(it, n));
        }
        for (i, v) in args.iter_mut().enumerate() {
            let Some(p) = self.param(i) else { break };
            if p.mask & MIXED != 0 {
                continue;
            }
            if p.mask & CALLABLE != 0 && matches!(v, Value::Array(_) | Value::Object(_)) && p.mask & (ARRAY | OBJECT) == 0 {
                self.check_callable(it, f, i, p, v)?;
                continue;
            }
            if accepts(p, v) {
                continue;
            }
            self.coerce(it, f, i, p, v)?;
        }
        Ok(())
    }

    fn count_error(&self, it: &Interp, n: usize) -> crate::interp::Ctrl {
        let msg = if self.min == self.max {
            format!("{}() expects exactly {} {}, {n} given", self.name, self.min, plural(self.min))
        } else if n < self.min {
            format!("{}() expects at least {} {}, {n} given", self.name, self.min, plural(self.min))
        } else {
            format!("{}() expects at most {} {}, {n} given", self.name, self.max, plural(self.max))
        };
        it.error("ArgumentCountError", &msg)
    }

    fn type_error(&self, it: &Interp, i: usize, p: &Param, v: &Value) -> crate::interp::Ctrl {
        it.error(
            "TypeError",
            &format!("{}(): Argument #{}{} must be of type {}, {} given", self.name, i + 1, arg_label(p), p.ty, v.debug_type()),
        )
    }

    /// A value whose type isn't directly accepted: PHP's juggling for
    /// internal functions (scalars in coercive mode, int to float, objects
    /// with `__toString`), else a TypeError.
    #[cold]
    fn coerce(&self, it: &Interp, f: &Frame, i: usize, p: &Param, v: &mut Value) -> R<()> {
        let v0 = match &*v {
            Value::Ref(r) => r.borrow().clone(),
            other => other.clone(),
        };
        if let Value::Ref(_) = v {
            // By-reference argument: check its value only.
            if accepts(p, &v0) {
                return Ok(());
            }
        }
        // int -> float is allowed even in strict mode.
        if let (Value::Int(n), true) = (&v0, p.mask & FLOAT != 0) {
            if !matches!(v, Value::Ref(_)) {
                *v = Value::Float(*n as f64);
            }
            return Ok(());
        }
        if p.mask & CALLABLE != 0 {
            return self.check_callable(it, f, i, p, &v0);
        }
        let strict = it.file_strict(it.pos.get().file as usize);
        match &v0 {
            Value::Null if !strict && p.mask & SCALAR != 0 => {
                it.deprecated(&format!("{}(): Passing null to parameter #{} (${}) of type {} is deprecated", self.name, i + 1, p.name, p.ty));
                return Ok(());
            }
            Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_) if !strict && p.mask & SCALAR != 0 => {
                if let Some(c) = scalar_coerce(it, p.mask, &v0) {
                    if !matches!(v, Value::Ref(_)) {
                        *v = c;
                    }
                    return Ok(());
                }
            }
            Value::Object(o) if p.mask & STRING != 0 && o.class.find_method("__tostring").is_some() && !strict => {
                let s = it.to_str(&v0)?;
                if !matches!(v, Value::Ref(_)) {
                    *v = Value::Str(s);
                }
                return Ok(());
            }
            Value::Object(o) if p.mask & STRING != 0 && o.class.find_method("__tostring").is_some() => {
                // Strict mode still accepts Stringable for string.
                let s = it.to_str(&v0)?;
                if !matches!(v, Value::Ref(_)) {
                    *v = Value::Str(s);
                }
                return Ok(());
            }
            _ => {}
        }
        Err(self.type_error(it, i, p, &v0))
    }

    /// PHP's message for an invalid callback argument.
    fn check_callable(&self, it: &Interp, f: &Frame, i: usize, p: &Param, v: &Value) -> R<()> {
        let Some(why) = it.callable_error(f, v)? else { return Ok(()) };
        let or_null = if p.mask & NULL != 0 { " or null" } else { "" };
        Err(it.error("TypeError", &format!("{}(): Argument #{}{} must be a valid callback{or_null}, {why}", self.name, i + 1, arg_label(p))))
    }

    /// Positional arguments for a call with named ones: each name goes to
    /// its parameter; skipped optional parameters get PHP's default.
    pub fn bind_named(&self, it: &Interp, mut pos: Vec<Value>, named: Vec<(std::rc::Rc<str>, Value)>) -> R<Vec<Value>> {
        let mut slots: Vec<Option<Value>> = pos.drain(..).map(Some).collect();
        let variadic = self.params.last().filter(|p| p.variadic);
        for (name, v) in named {
            match self.params.iter().position(|p| *p.name == *name && !p.variadic) {
                Some(i) => {
                    if slots.len() <= i {
                        slots.resize_with(i + 1, || None);
                    }
                    if slots[i].is_some() {
                        return Err(it.error("Error", &format!("Named parameter ${name} overwrites previous argument")));
                    }
                    slots[i] = Some(v);
                }
                None if variadic.is_some() => {
                    return Err(it.error("ArgumentCountError", &format!("{}() does not accept unknown named parameters", self.name)));
                }
                None => return Err(it.error("Error", &format!("Unknown named parameter ${name}"))),
            }
        }
        let mut out = Vec::with_capacity(slots.len());
        for (i, s) in slots.into_iter().enumerate() {
            match s {
                Some(v) => out.push(v),
                None => {
                    let p = &self.params[i];
                    if !p.optional {
                        return Err(it.error(
                            "ArgumentCountError",
                            &format!("{}(): Argument #{} (${}) not passed", self.name, i + 1, p.name),
                        ));
                    }
                    out.push(default_value(it, p).ok_or_else(|| {
                        it.error(
                            "ArgumentCountError",
                            &format!("{}(): Argument #{} (${}) must be passed explicitly, because the default value is not known", self.name, i + 1, p.name),
                        )
                    })?);
                }
            }
        }
        Ok(out)
    }
}

/// Is `v` directly of an accepted type (no conversion needed)?
#[inline]
fn accepts(p: &Param, v: &Value) -> bool {
    let m = p.mask;
    match v {
        Value::Null => m & NULL != 0,
        Value::Bool(true) => m & TRUE != 0,
        Value::Bool(false) => m & FALSE != 0,
        Value::Int(_) => m & INT != 0,
        Value::Float(_) => m & FLOAT != 0,
        // A string for a callable parameter must name a function (checked
        // in `coerce`); arrays and objects are validated by the built-in.
        Value::Str(_) => m & STRING != 0,
        Value::Array(_) => m & (ARRAY | ITERABLE | CALLABLE) != 0,
        Value::Object(o) => {
            m & (OBJECT | CALLABLE) != 0
                || (m & ITERABLE != 0 && o.class.is_a("traversable"))
                || p.classes.iter().any(|c| o.class.is_a(c))
        }
        // Resources and references: not described by declared types.
        Value::Resource(_) | Value::Ref(_) => true,
    }
}

/// Coercive mode: the scalar converted to one of the parameter's scalar
/// types, PHP's preference order (int, float, string, bool), or None.
fn scalar_coerce(it: &Interp, mask: u16, v: &Value) -> Option<Value> {
    let num = |n: crate::value::Num| -> Option<Value> {
        use crate::value::Num;
        match n {
            Num::Int(i) if mask & INT != 0 => Some(Value::Int(i)),
            Num::Int(i) => Some(Value::Float(i as f64)),
            Num::Float(f) if mask & FLOAT != 0 => Some(Value::Float(f)),
            Num::Float(f) => {
                if !f.is_finite() || f < i64::MIN as f64 || f >= i64::MAX as f64 {
                    return None;
                }
                if f.fract() != 0.0 {
                    it.deprecated(&format!("Implicit conversion from float {} to int loses precision", crate::value::fmt_float(f)));
                }
                Some(Value::Int(f as i64))
            }
        }
    };
    if mask & (INT | FLOAT) != 0 {
        match v {
            Value::Bool(b) => return num(crate::value::Num::Int(*b as i64)),
            Value::Float(f) => return num(crate::value::Num::Float(*f)).or_else(|| (mask & (STRING | BOOL) != 0).then(|| v.clone())),
            Value::Str(s) => {
                if let Some(n) = parse_numeric(s) {
                    if let crate::value::Num::Float(f) = n {
                        if mask & FLOAT == 0 && f.fract() != 0.0 && mask & STRING == 0 {
                            it.deprecated(&format!("Implicit conversion from float-string \"{}\" to int loses precision", crate::pstr::lossy(s)));
                            return Some(Value::Int(f as i64));
                        }
                    }
                    if mask & STRING == 0 {
                        return num(n);
                    }
                }
            }
            _ => {}
        }
    }
    if mask & STRING != 0 {
        // The built-in converts to string itself (same result).
        return Some(v.clone());
    }
    if mask & BOOL != 0 {
        return Some(Value::Bool(v.truthy()));
    }
    None
}

/// A parameter's default from its PHP source (cached per parameter).
fn default_value(it: &Interp, p: &Param) -> Option<Value> {
    if p.default.is_empty() {
        return None;
    }
    thread_local! {
        static CACHE: std::cell::RefCell<FxHashMap<usize, Value>> = Default::default();
    }
    let key = p as *const Param as usize;
    if let Some(v) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(v);
    }
    let v = it.eval_const_source(p.default)?;
    CACHE.with(|c| c.borrow_mut().insert(key, v.clone()));
    Some(v)
}
