//! Runtime helpers called from native code.
//!
//! Conventions (native code relies on them):
//! - every `*mut Value` slot always holds a valid Value; empty = Null;
//! - `dst` slots are empty on entry and get the result;
//! - "consumed" arguments are taken and left Null;
//! - return 0 = ok, 1 = error (the exception is in `Interp::pending`).
//!
//! Helpers are thin wrappers over interpreter methods, so native code has
//! exactly the interpreter's semantics.

#![allow(clippy::missing_safety_doc)]

use std::cell::RefCell;
use std::ptr;
use std::rc::Rc;

use phpx_syntax::ast::{self, BinOp};

use crate::interp::{slot, Callable, Ctrl, Frame, Interp, R};
use crate::value::{Internal, Key, Obj, PhpArray, Value};

// Fast built-in entries are registered with the other helpers.
pub use crate::trace::{rt_call_pop, rt_call_push};
pub use super::fastbi::{
    rt_fast_chr, rt_fast_implode, rt_fast_in_array, rt_fast_key_exists, rt_fast_ord, rt_fast_str_contains, rt_fast_str_ends_with,
    rt_fast_str_replace, rt_fast_str_starts_with, rt_fast_strpos, rt_fast_strtolower, rt_fast_strtoupper, rt_fast_substr, rt_fast_trim,
};
pub use super::fastbi::{
    rt_fast_abs, rt_fast_array_key_first, rt_fast_array_key_last, rt_fast_array_keys, rt_fast_array_values, rt_fast_ceil, rt_fast_explode,
    rt_fast_floor, rt_fast_intval, rt_fast_lcfirst, rt_fast_max, rt_fast_min, rt_fast_str_repeat, rt_fast_ucfirst,
};

#[repr(C)]
pub struct Cx {
    pub it: *const Interp,
    pub frame: *mut Frame,
}

/// Per-function data for entry: local names in slot order (parameters
/// first), `$this`'s slot, and what binding/type errors need.
pub struct LocalsSpec {
    pub names: Vec<&'static str>,
    pub this_slot: Option<usize>,
    pub params: &'static [ast::Param],
    pub display: Rc<str>,
    pub ctx: Rc<crate::interp::NsCtx>,
    pub class: Option<Rc<crate::class::Class>>,
    /// All params untyped, by value, no defaults needed beyond the args:
    /// binding is a plain move.
    pub simple: bool,
    /// Per parameter: a check that accepts the common case of its declared
    /// type without the general coercion (which runs when it fails).
    pub quick: Vec<(Quick, bool)>,
    /// Closures: the local slot of each captured variable (in `use` order).
    pub captured_slots: Vec<usize>,
    pub ret: Option<&'static ast::Type>,
    pub ret_quick: (Quick, bool),
}

/// The declared return type, checked (and the value coerced) in the
/// callee before it returns.
pub unsafe extern "C" fn rt_check_ret(c: *mut Cx, out: *mut Value, spec: *const LocalsSpec) -> u64 {
    let (it, f) = cx(c);
    let spec = &*spec;
    if Quick::accepts(spec.ret_quick, &*out) {
        return 0;
    }
    let Some(ty) = spec.ret else { return 0 };
    let v = take(out);
    match it.check_return_type(ty, v, &spec.ctx, spec.class.as_ref(), f.static_class.as_ref(), &spec.display) {
        Ok(v) => {
            put(out, v);
            0
        }
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            1
        }
    }
}

/// A declared parameter type, reduced for a fast acceptance test.
#[derive(Clone, Copy, PartialEq)]
pub enum Quick {
    Untyped,
    Int,
    Float,
    Str,
    Bool,
    Array,
    Mixed,
    /// Anything else (classes, unions, ...): always the general path.
    Other,
}

impl Quick {
    /// (kind, nullable)
    pub fn of(ty: Option<&ast::Type>) -> (Quick, bool) {
        let Some(ty) = ty else { return (Quick::Untyped, false) };
        match ty {
            ast::Type::Nullable(t) => (Quick::of(Some(t)).0, true),
            ast::Type::Named(n) => (
                match n.to_ascii_lowercase().as_str() {
                    "int" => Quick::Int,
                    "float" => Quick::Float,
                    "string" => Quick::Str,
                    "bool" => Quick::Bool,
                    "array" => Quick::Array,
                    "mixed" => Quick::Mixed,
                    _ => Quick::Other,
                },
                false,
            ),
            _ => (Quick::Other, false),
        }
    }

    #[inline]
    fn accepts(q: (Quick, bool), v: &Value) -> bool {
        match (q.0, v) {
            (Quick::Untyped | Quick::Mixed, _)
            | (Quick::Int, Value::Int(_))
            | (Quick::Float, Value::Float(_))
            | (Quick::Str, Value::Str(_))
            | (Quick::Bool, Value::Bool(_))
            | (Quick::Array, Value::Array(_)) => true,
            (_, Value::Null) => q.1,
            _ => false,
        }
    }
}

unsafe fn cx<'a>(c: *mut Cx) -> (&'a Interp, &'a mut Frame) {
    (&*(*c).it, &mut *(*c).frame)
}

unsafe fn put(dst: *mut Value, v: Value) {
    ptr::write(dst, v);
}

unsafe fn take(p: *mut Value) -> Value {
    ptr::replace(p, Value::Null)
}

fn status<T>(it: &Interp, r: R<T>, ok: impl FnOnce(T)) -> u64 {
    match crate::errors::check(r) {
        Ok(v) => {
            ok(v);
            0
        }
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            1
        }
    }
}

pub fn binop_of(code: u64) -> BinOp {
    use BinOp::*;
    [Add, Sub, Mul, Div, Mod, Pow, Concat, BitAnd, BitOr, BitXor, Shl, Shr, And, Or, Xor, Eq, NotEq, Identical, NotIdentical, Lt, LtEq, Gt, GtEq, Spaceship, Coalesce][code as usize]
}

pub fn binop_code(op: BinOp) -> u64 {
    use BinOp::*;
    match op {
        Add => 0, Sub => 1, Mul => 2, Div => 3, Mod => 4, Pow => 5, Concat => 6, BitAnd => 7, BitOr => 8, BitXor => 9,
        Shl => 10, Shr => 11, And => 12, Or => 13, Xor => 14, Eq => 15, NotEq => 16, Identical => 17, NotIdentical => 18,
        Lt => 19, LtEq => 20, Gt => 21, GtEq => 22, Spaceship => 23, Coalesce => 24,
    }
}

// ---- entry / exit ------------------------------------------------------------

/// Move parameters (and closure captures, `$this`) from the interpreter frame
/// into native locals. Returns 2 ("decline": interpret this call instead)
/// when native code can't represent the frame, before touching anything.
pub unsafe extern "C" fn rt_load_locals(c: *mut Cx, locals: *mut Value, spec: *const LocalsSpec) -> u64 {
    let (_, f) = cx(c);
    let spec = &*spec;
    if spec.this_slot.is_some() && f.this.is_none() {
        return 2;
    }
    for name in &spec.names {
        if let Some(s) = f.vars.get(name) {
            // Shared cell (by-ref capture etc.): v1 locals can't alias.
            if Rc::strong_count(s) > 1 {
                return 2;
            }
        }
    }
    for (i, name) in spec.names.iter().enumerate() {
        if let Some(s) = f.vars.get(name) {
            put(locals.add(i), std::mem::take(&mut *s.borrow_mut()));
        }
    }
    if let (Some(i), Some(o)) = (spec.this_slot, &f.this) {
        ptr::drop_in_place(locals.add(i));
        put(locals.add(i), Value::Object(o.clone()));
    }
    0
}

/// Bind parameters from a caller's argument array (consumed) straight into
/// locals. Declines (2, nothing consumed) for cases the full interpreter
/// path should handle: by-ref/variadic/promoted params, missing required
/// args, `$this` missing.
pub unsafe extern "C" fn rt_bind_args(c: *mut Cx, locals: *mut Value, spec: *const LocalsSpec, args: *mut Value, n: u64) -> u64 {
    let (it, f) = cx(c);
    let spec = &*spec;
    let n = n as usize;
    if spec.this_slot.is_some() && f.this.is_none() {
        return 2;
    }
    if spec.simple && n >= spec.params.len() {
        for i in 0..spec.params.len() {
            put(locals.add(i), take(args.add(i)).into_deref());
        }
        for i in spec.params.len()..n {
            drop(take(args.add(i)));
        }
        if let (Some(i), Some(o)) = (spec.this_slot, &f.this) {
            put(locals.add(i), Value::Object(o.clone()));
        }
        return 0;
    }
    for (i, p) in spec.params.iter().enumerate() {
        if p.by_ref || p.variadic || (p.promote.is_some() && f.this.is_none()) || (i >= n && p.default.is_none()) {
            return 2;
        }
    }
    // Captured values come from the closure the call site put in the frame
    // (by value only: native locals can't share a by-reference capture).
    let closure = match &f.closure {
        _ if spec.captured_slots.is_empty() => None,
        Some(cd) if cd.captured.len() == spec.captured_slots.len() && cd.captured.iter().all(|c| !c.2) => Some(cd.clone()),
        _ => return 2,
    };
    let strict = || it.file_strict(it.pos.get().file as usize);
    for (i, p) in spec.params.iter().enumerate() {
        let v = if i < n {
            take(args.add(i)).into_deref()
        } else {
            match it.eval(f, p.default.as_ref().unwrap()) {
                Ok(v) => v,
                Err(e) => {
                    *it.pending.borrow_mut() = Some(e);
                    return 1;
                }
            }
        };
        let v = match &p.ty {
            _ if Quick::accepts(spec.quick[i], &v) => v,
            Some(ty) if !(v.is_null() && matches!(p.default.as_ref().map(|d| &d.kind), Some(ast::ExprKind::Null))) => {
                match it.coerce(v.clone(), ty, &spec.ctx, spec.class.as_ref(), f.static_class.as_ref(), strict()) {
                    Ok(Some(v)) => v,
                    Ok(None) => {
                        *it.pending.borrow_mut() = Some(it.param_type_error(&spec.display, i, p, ty, &v, spec.ctx.file));
                        return 1;
                    }
                    Err(e) => {
                        *it.pending.borrow_mut() = Some(e);
                        return 1;
                    }
                }
            }
            _ => v,
        };
        if p.promote.is_some() {
            if let Some(o) = &f.this {
                // A private promoted property shadowed by a subclass lives
                // under its mangled key.
                let key = it.prop_key(f, o, &p.name.name);
                o.set_prop(&key, v.clone());
            }
        }
        ptr::drop_in_place(locals.add(i));
        put(locals.add(i), v);
    }
    // Extra arguments are ignored (func_get_args() isn't compiled).
    for i in spec.params.len()..n {
        drop(take(args.add(i)));
    }
    if let Some(cd) = closure {
        for ((_, v, _), &i) in cd.captured.iter().zip(&spec.captured_slots) {
            ptr::drop_in_place(locals.add(i));
            put(locals.add(i), v.clone());
        }
    }
    if let (Some(i), Some(o)) = (spec.this_slot, &f.this) {
        ptr::drop_in_place(locals.add(i));
        put(locals.add(i), Value::Object(o.clone()));
    }
    0
}

/// Top-level code: move locals back into the frame's variables (later
/// segments, includes and catch blocks see them, like PHP).
/// Top level: point each local at its frame variable cell (created if
/// missing). Holds a strong count on each cell until rt_unbind_slots.
pub unsafe extern "C" fn rt_bind_slots(c: *mut Cx, ptrs: *mut *mut Value, rcs: *mut *const RefCell<Value>, spec: *const LocalsSpec) -> u64 {
    let (_, f) = cx(c);
    for (i, name) in (*spec).names.iter().enumerate() {
        let s = f.var_slot(name);
        *ptrs.add(i) = s.as_ptr();
        *rcs.add(i) = Rc::into_raw(s);
    }
    0
}

pub unsafe extern "C" fn rt_unbind_slots(rcs: *mut *const RefCell<Value>, n: u64) {
    for i in 0..n as usize {
        let p = *rcs.add(i);
        if !p.is_null() {
            drop(Rc::from_raw(p));
            *rcs.add(i) = ptr::null();
        }
    }
}

pub unsafe extern "C" fn rt_store_locals(c: *mut Cx, locals: *mut Value, spec: *const LocalsSpec) {
    let (_, f) = cx(c);
    for (i, name) in (*spec).names.iter().enumerate() {
        let v = take(locals.add(i));
        match f.vars.get(name) {
            Some(s) => *s.borrow_mut() = v,
            None => {
                // Never-assigned locals stay undefined (isset() is false).
                if !v.is_null() {
                    f.vars.insert(name, slot(v));
                }
            }
        }
    }
}

pub unsafe extern "C" fn rt_declare_class(c: *mut Cx, class: *const ast::Class) -> u64 {
    let (it, f) = cx(c);
    let class: &'static ast::Class = &*class;
    if it.declared.borrow().contains(&(class as *const ast::Class as usize)) {
        return 0;
    }
    let ctx = f.ctx.clone();
    status(it, it.declare_class(class, &ctx), |_| {})
}

pub unsafe extern "C" fn rt_define_const(c: *mut Cx, name: *const String, v: *const Value) -> u64 {
    let (it, f) = cx(c);
    let full = crate::interp::qualify(&f.ctx.ns, &*name);
    it.constants.borrow_mut().insert(full, (*v).clone());
    0
}

pub unsafe extern "C" fn rt_release(v: *mut Value) {
    drop(take(v));
}

pub unsafe extern "C" fn rt_release_n(v: *mut Value, n: u64) {
    for i in 0..n as usize {
        drop(take(v.add(i)));
    }
}

/// dst (empty) := clone of src.
pub unsafe extern "C" fn rt_copy(dst: *mut Value, src: *const Value) {
    put(dst, (*src).deref());
}

// ---- operators -------------------------------------------------------------------

pub unsafe extern "C" fn rt_binop(c: *mut Cx, op: u64, a: *const Value, b: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    status(it, it.binop(binop_of(op), &*a, &*b), |v| put(dst, v))
}

/// Comparison result as 0/1 (comparisons don't fail).
pub unsafe extern "C" fn rt_compare(c: *mut Cx, op: u64, a: *const Value, b: *const Value) -> u64 {
    let (it, _) = cx(c);
    it.binop(binop_of(op), &*a, &*b).map(|v| v.truthy() as u64).unwrap_or(0)
}

pub unsafe extern "C" fn rt_truthy(v: *const Value) -> u64 {
    (*v).truthy() as u64
}

pub unsafe extern "C" fn rt_identical(a: *const Value, b: *const Value) -> u64 {
    crate::value::identical(&*a, &*b) as u64
}

pub unsafe extern "C" fn rt_loose_eq(a: *const Value, b: *const Value) -> u64 {
    crate::value::loose_eq(&*a, &*b) as u64
}

pub unsafe extern "C" fn rt_unary(c: *mut Cx, kind: u64, v: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let r = match kind {
        0 => it.binop(BinOp::Mul, &*v, &Value::Int(-1)),
        1 => it.binop(BinOp::Mul, &*v, &Value::Int(1)),
        _ => it.bitnot(&*v),
    };
    status(it, r, |v| put(dst, v))
}

/// `$local .= rhs` in place.
pub unsafe extern "C" fn rt_concat_assign(c: *mut Cx, local: *mut Value, rhs: *const Value) -> u64 {
    let (it, _) = cx(c);
    // Fast path: appending an int or string to a string we own.
    if let (Value::Str(s), Value::Int(_) | Value::Str(_)) = (&mut *local, &*rhs) {
        match &*rhs {
            Value::Int(n) => s.push_int(*n),
            Value::Str(r) => {
                let r = r.clone();
                s.push_bytes(&r);
            }
            _ => unreachable!(),
        }
        return 0;
    }
    let r = it.to_str(&*rhs).and_then(|b| {
        let cur = take(local);
        let v = match cur {
            Value::Str(mut s) => {
                s.push_bytes(&b);
                Value::Str(s)
            }
            other => {
                let mut s = match it.to_str(&other) {
                    Ok(s) => s.to_vec(),
                    Err(e) => {
                        put(local, other);
                        return Err(e);
                    }
                };
                s.extend_from_slice(&b);
                Value::bytes(s)
            }
        };
        put(local, v);
        Ok(())
    });
    status(it, r, |_| {})
}

pub unsafe extern "C" fn rt_echo(c: *mut Cx, v: *const Value) -> u64 {
    let (it, _) = cx(c);
    status(it, it.to_str(&*v), |b| it.echo(&b))
}

/// `++`/`--` on a local. flags: bit0 = increment, bit1 = prefix.
pub unsafe extern "C" fn rt_incdec(c: *mut Cx, local: *mut Value, flags: u64, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let cur = (*local).clone();
    status(it, it.incdec(&cur, flags & 1 != 0), |new| {
        if !dst.is_null() {
            put(dst, if flags & 2 != 0 { new.clone() } else { cur });
        }
        ptr::drop_in_place(local);
        put(local, new);
    })
}

pub unsafe extern "C" fn rt_cast(c: *mut Cx, kind: u64, v: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let ty = ["int", "float", "string", "bool", "array", "object", "unset"][kind as usize];
    status(it, it.cast(ty, (*v).clone()), |v| put(dst, v))
}

/// Append a value's string form to `buf` (ints formatted in place).
fn append_str(it: &Interp, buf: &mut crate::value::PStr, v: &Value) -> R<()> {
    match v {
        Value::Str(s) => buf.push_bytes(s),
        Value::Int(n) => buf.push_int(*n),
        other => buf.push_bytes(&it.to_str(other)?),
    }
    Ok(())
}

/// `"a $b c"`: build the string from `n` part pointers in one allocation.
pub unsafe extern "C" fn rt_interp(c: *mut Cx, parts: *const *const Value, n: u64, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let parts = std::slice::from_raw_parts(parts, n as usize);
    let cap: usize = parts.iter().map(|p| if let Value::Str(s) = &**p { s.len() } else { 20 }).sum();
    let mut buf = crate::value::PStr::with_capacity(cap);
    for p in parts {
        if let Err(e) = append_str(it, &mut buf, &**p) {
            *it.pending.borrow_mut() = Some(e);
            return 1;
        }
    }
    put(dst, Value::Str(buf));
    0
}

// ---- reads ------------------------------------------------------------------------

pub unsafe extern "C" fn rt_index_get(c: *mut Cx, base: *const Value, key: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    // Fast path: array with an int or plain string key that exists.
    if let Value::Array(a) = &*base {
        let found = match &*key {
            Value::Int(n) => a.get(&Key::Int(*n)),
            Value::Str(s) => a.get(&Key::from_pstr(s)),
            _ => None,
        };
        if let Some(v) = found {
            put(dst, v);
            return 0;
        }
    }
    status(it, it.read_index(&*base, &*key), |v| put(dst, v))
}

/// One element of a list() assignment (`first`: the first target).
pub unsafe extern "C" fn rt_list_get(c: *mut Cx, base: *const Value, key: *const Value, first: u64, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    if let (Value::Array(a), Value::Int(n)) = (&*base, &*key) {
        if let Some(v) = a.get(&Key::Int(*n)) {
            put(dst, v);
            return 0;
        }
    }
    let r = it.to_key(&*key).and_then(|k| it.list_elem(&*base, &k, first != 0));
    status(it, r, |v| put(dst, v))
}

/// Quiet `$base[$key]` for isset/??/empty: 1 = found (written to dst),
/// 0 = missing, 2 = error.
pub unsafe extern "C" fn rt_quiet_index(c: *mut Cx, base: *const Value, key: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let r: R<Option<Value>> = (|| {
        Ok(match &*base {
            Value::Array(a) => a.get(&it.to_key(&*key)?),
            Value::Str(s) => {
                let i = match &*key {
                    Value::Int(n) => Some(*n),
                    Value::Str(ks) => match crate::value::parse_numeric(ks) {
                        Some(crate::value::Num::Int(n)) => Some(n),
                        _ => None,
                    },
                    _ => None,
                };
                i.and_then(|i| {
                    let idx = if i < 0 { s.len() as i64 + i } else { i };
                    (idx >= 0 && idx < s.len() as i64).then(|| Value::bytes(vec![s[idx as usize]]))
                })
            }
            Value::Object(o) if o.class.is_a("arrayaccess") => {
                if it.call_method(o, "offsetExists", vec![(*key).clone()])?.truthy() {
                    Some(it.call_method(o, "offsetGet", vec![(*key).clone()])?)
                } else {
                    None
                }
            }
            _ => None,
        })
    })();
    match r {
        Ok(Some(v)) => {
            put(dst, v);
            1
        }
        Ok(None) => 0,
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            2
        }
    }
}

unsafe fn name_of(p: *const u8, len: u64) -> &'static str {
    std::str::from_utf8_unchecked(std::slice::from_raw_parts(p, len as usize))
}

pub unsafe extern "C" fn rt_prop_get(c: *mut Cx, obj: *const Value, name: *const u8, len: u64, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let name = name_of(name, len);
    match &*obj {
        Value::Object(o) => status(it, it.get_prop(f, o, name), |v| put(dst, v)),
        other => {
            it.warn(&format!("Attempt to read property \"{name}\" on {}", other.type_name()));
            0
        }
    }
}

/// Quiet `$obj->name`: 1 found, 0 missing, 2 error.
pub unsafe extern "C" fn rt_quiet_prop(c: *mut Cx, obj: *const Value, name: *const u8, len: u64, dst: *mut Value) -> u64 {
    quiet_prop(c, obj, name, len, dst, crate::expr::QUIET_READ)
}

/// `isset($obj->name)`: like rt_quiet_prop, but a magic property only
/// asks __isset.
pub unsafe extern "C" fn rt_isset_prop(c: *mut Cx, obj: *const Value, name: *const u8, len: u64, dst: *mut Value) -> u64 {
    quiet_prop(c, obj, name, len, dst, crate::expr::QUIET_ISSET)
}

/// `empty($obj->name)`: a magic property asks __isset, then __get.
pub unsafe extern "C" fn rt_empty_prop(c: *mut Cx, obj: *const Value, name: *const u8, len: u64, dst: *mut Value) -> u64 {
    quiet_prop(c, obj, name, len, dst, crate::expr::QUIET_EMPTY)
}

unsafe fn quiet_prop(c: *mut Cx, obj: *const Value, name: *const u8, len: u64, dst: *mut Value, mode: u8) -> u64 {
    let (it, f) = cx(c);
    let name = name_of(name, len);
    let Value::Object(o) = &*obj else { return 0 };
    let key = it.prop_key(f, o, name);
    let name = &*key;
    if it.prop_visible(f, o, name) {
        if let Some(v) = o.get_prop(name) {
            put(dst, v);
            return 1;
        }
        if it.prop_uninit(o, name) {
            return 0;
        }
    }
    match it.magic_isset(o, name, mode) {
        Ok(Some(v)) => {
            put(dst, v);
            1
        }
        Ok(None) => 0,
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            2
        }
    }
}

pub unsafe extern "C" fn rt_const(c: *mut Cx, name: *const String, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    status(it, it.constant(f, &*name), |v| put(dst, v))
}

pub unsafe extern "C" fn rt_magic(c: *mut Cx, e: *const ast::Expr, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    if let ast::ExprKind::MagicConst(m) = &(*e).kind {
        put(dst, it.magic_const(f, m, &*e));
    }
    0
}

pub unsafe extern "C" fn rt_class_const(c: *mut Cx, cref: *const ast::ClassRef, name: *const u8, len: u64, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    status(it, it.class_const(f, &*cref, name_of(name, len)), |v| put(dst, v))
}

pub unsafe extern "C" fn rt_static_prop_get(c: *mut Cx, cref: *const ast::ClassRef, name: *const u8, len: u64, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let r = it.find_static_prop(f, &*cref, name_of(name, len)).and_then(|sp| it.static_prop_slot(&sp));
    status(it, r, |v| put(dst, v))
}

/// The class reference for a runtime class value (0 = error pending).
pub unsafe extern "C" fn rt_dyn_class(c: *mut Cx, v: *const Value) -> u64 {
    let (it, _) = cx(c);
    match it.class_of_value(&(*v).deref()) {
        Ok(cls) => it.dyn_class_ref(&cls) as *const ast::ClassRef as u64,
        Err(e) => {
            status(it, Err::<(), _>(e), |_| ());
            0
        }
    }
}

/// `$x::class`.
pub unsafe extern "C" fn rt_class_name_of(c: *mut Cx, v: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    status(it, it.class_name_of_value(&(*v).deref()), |v| put(dst, v))
}

/// `$v instanceof $class` (an object or a class name; unknown names are
/// false without autoloading).
pub unsafe extern "C" fn rt_instanceof_dyn(c: *mut Cx, v: *const Value, class: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let r = (|| -> R<Value> {
        let lname = match (*class).deref() {
            Value::Object(c) => c.class.lname.clone(),
            Value::Str(s) => it.lower.get(String::from_utf8_lossy(&s).trim_start_matches('\\')),
            _ => return Err(it.error("Error", "Class name must be a valid object or a string")),
        };
        Ok(Value::Bool(matches!((*v).deref(), Value::Object(o) if o.class.is_a(&lname))))
    })();
    status(it, r, |v| put(dst, v))
}

/// get_called_class().
pub unsafe extern "C" fn rt_called_class(c: *mut Cx, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let r = match &f.static_class {
        Some(c) => Ok(Value::str(&c.name)),
        None => Err(it.error("Error", "get_called_class() must be called from within a class")),
    };
    status(it, r, |v| put(dst, v))
}

pub unsafe extern "C" fn rt_instanceof(c: *mut Cx, v: *const Value, cref: *const ast::ClassRef) -> u64 {
    let (it, f) = cx(c);
    let Value::Object(o) = &*v else { return 0 };
    let ast::ClassRef::Name(n) = &*cref else { return 0 };
    let lname = match &*n.to_ascii_lowercase() {
        "self" | "static" | "parent" => match it.class_ref(f, &*cref) {
            Ok(c) => c.lname.clone(),
            Err(_) => return 0,
        },
        _ => it.lower.get(&it.resolve_class_name(&f.ctx, n)),
    };
    o.class.is_a(&lname) as u64
}

// ---- writes -----------------------------------------------------------------------

/// Where a write goes: a native local, or a property of an object.
unsafe fn keys_of(it: &Interp, keys: *const Value, n: u64, append: u64) -> R<Vec<Option<crate::expr::Off>>> {
    let mut path = Vec::with_capacity(n as usize + 1);
    for i in 0..n as usize {
        path.push(Some(it.to_key_for_write(&*keys.add(i))?));
    }
    if append != 0 {
        path.push(None);
    }
    Ok(path)
}

/// `root[keys...] = val` (or `->name[keys...]` when `name` is set; root is
/// then the object). `val` is consumed. Writes the assigned value to `dst`
/// when non-null.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn rt_place_assign(
    c: *mut Cx,
    root: *mut Value,
    name: *const u8,
    len: u64,
    keys: *const Value,
    n: u64,
    append: u64,
    val: *mut Value,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    let v = take(val);
    let r = (|| -> R<()> {
        let path = keys_of(it, keys, n, append)?;
        if name.is_null() {
            let mut cur = take(root);
            let pending = it.write_path(&mut cur, &path, v.clone());
            put(root, cur);
            if let Some(p) = pending? {
                it.finish_pending(p)?;
            }
        } else {
            let place = root_place(it, f, root, name, len, path)?;
            it.place_set(f, &place, v.clone())?;
        }
        Ok(())
    })();
    status(it, r, |_| {
        if !dst.is_null() {
            put(dst, v);
        }
    })
}

/// `$local[] = val` / `$local[key] = val` (`key` may be null for append).
/// `val` is consumed; the assigned value goes to `dst` if non-null.
pub unsafe extern "C" fn rt_local_store(c: *mut Cx, local: *mut Value, key: *const Value, val: *mut Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    if matches!(&*local, Value::Null) {
        put(local, Value::array(PhpArray::new()));
    }
    if let Value::Array(a) = &mut *local {
        let k = if key.is_null() {
            None
        } else {
            match it.to_key(&*key) {
                Ok(k) => Some(k),
                Err(e) => {
                    *it.pending.borrow_mut() = Some(e);
                    return 1;
                }
            }
        };
        let v = take(val);
        if !dst.is_null() {
            put(dst, v.clone());
        }
        let arr = Rc::make_mut(a);
        match k {
            None => {
                arr.push(v);
            }
            Some(k) => {
                let slot = arr.entry(k);
                match slot {
                    // An element that is a reference: write through it.
                    Value::Ref(s) => *s.borrow_mut() = v,
                    _ => *slot = v,
                }
            }
        }
        return 0;
    }
    // Strings, ArrayAccess, scalars (errors): the general path.
    let append = key.is_null() as u64;
    let n = (!key.is_null()) as u64;
    rt_place_assign(c, local, ptr::null(), 0, key, n, append, val, dst)
}

/// A `$obj->name` site. Generated code checks (object shape, caller
/// scope) against the cached key and then reads/writes the slot directly;
/// these helpers are the miss path and fill the cache. `#[repr(C)]`: field
/// offsets are used by generated code.
#[repr(C)]
pub struct PropSite {
    /// Read cache: shape key (0 = empty), slot, scope key.
    pub shape: std::cell::Cell<usize>,
    pub slot: std::cell::Cell<usize>,
    pub scope: std::cell::Cell<usize>,
    /// Write cache (only plain, untyped, non-readonly, visible properties).
    pub wshape: std::cell::Cell<usize>,
    pub wslot: std::cell::Cell<usize>,
    pub wscope: std::cell::Cell<usize>,
    pub name: &'static str,
}

impl PropSite {
    pub fn new(name: &'static str) -> PropSite {
        PropSite { shape: Default::default(), slot: Default::default(), scope: Default::default(), wshape: Default::default(), wslot: Default::default(), wscope: Default::default(), name }
    }
}

pub const PROPSITE_SHAPE: i32 = 0;
pub const PROPSITE_SLOT: i32 = 8;
pub const PROPSITE_SCOPE: i32 = 16;
pub const PROPSITE_WSHAPE: i32 = 24;
pub const PROPSITE_WSLOT: i32 = 32;
pub const PROPSITE_WSCOPE: i32 = 40;

/// The words generated code compares: the object's shape pointer (first
/// word of its `Props`), and the frame's `class` field.
unsafe fn shape_key(o: &Obj) -> Option<usize> {
    let p = o.props.as_ptr();
    (*p).shape().shared.then(|| *(p as *const usize))
}

unsafe fn scope_key(f: &Frame) -> usize {
    *(std::ptr::addr_of!(f.class) as *const usize)
}

pub unsafe extern "C" fn rt_prop_read(c: *mut Cx, obj: *const Value, site: *const PropSite, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let site = &*site;
    if let Value::Object(o) = &*obj {
        // Cache only plain visible, present properties (no __get path).
        let key = it.prop_key(f, o, site.name);
        if it.prop_visible(f, o, &key) {
            let props = o.props.borrow();
            if let Some((i, _, v)) = props.get_full(&key) {
                if !matches!(v, Value::Ref(_)) {
                    if let Some(k) = shape_key(o) {
                        site.shape.set(k);
                        site.slot.set(i);
                        site.scope.set(scope_key(f));
                    }
                }
                put(dst, v.deref());
                return 0;
            }
        }
    }
    let name = site.name;
    rt_prop_get(c, obj, name.as_ptr(), name.len() as u64, dst)
}

/// `$obj->name = val` miss path. `val` is consumed; a copy goes to `dst`.
pub unsafe extern "C" fn rt_prop_write(c: *mut Cx, obj: *const Value, site: *const PropSite, val: *mut Value, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let site = &*site;
    let v = take(val);
    let Value::Object(o) = &*obj else {
        let e = it.error("Error", &format!("Attempt to assign property \"{}\" on {}", site.name, (*obj).type_name()));
        *it.pending.borrow_mut() = Some(e);
        return 1;
    };
    let r = it.set_prop(f, o, site.name, v.clone());
    if r.is_ok() {
        let plain = match o.class.find_prop(&it.prop_key(f, o, site.name)) {
            Some(d) => d.ty.is_none() && !d.readonly && it.member_visible(f.class.as_deref(), d.vis, &d.declaring),
            None => true,
        };
        if plain {
            let key = it.prop_key(f, o, site.name);
            let props = o.props.borrow();
            if let Some((i, _, cur)) = props.get_full(&key) {
                if !matches!(cur, Value::Ref(_)) {
                    if let Some(k) = shape_key(o) {
                        site.wshape.set(k);
                        site.wslot.set(i);
                        site.wscope.set(scope_key(f));
                    }
                }
            }
        }
    }
    status(it, r, |_| put(dst, v))
}

/// Flag on a place's name length: the root is a class reference and the
/// name a static property of that class.
pub const STATIC_ROOT: u64 = 1 << 63;

/// The non-local root of a write: property `name` of the object at `root`,
/// or (with STATIC_ROOT) static property `name` of the class at `root`.
unsafe fn root_place(it: &Interp, f: &mut Frame, root: *mut Value, name: *const u8, len: u64, path: Vec<Option<crate::expr::Off>>) -> R<crate::expr::Place> {
    if len & STATIC_ROOT != 0 {
        let sp = it.find_static_prop(f, &*(root as *const ast::ClassRef), name_of(name, len & !STATIC_ROOT))?;
        it.static_prop_slot(&sp)?;
        return Ok(crate::expr::Place { root: crate::expr::Root::Static(sp), path });
    }
    match &*root {
        Value::Object(o) => Ok(place_for(it, f, Some(o.clone()), name_of(name, len), path)),
        other => Err(it.error("Error", &format!("Attempt to assign property \"{}\" on {}", name_of(name, len), other.type_name()))),
    }
}

fn place_for(it: &Interp, f: &Frame, root_obj: Option<Rc<Obj>>, name: &'static str, path: Vec<Option<crate::expr::Off>>) -> crate::expr::Place {
    let o = root_obj.unwrap();
    let key = it.prop_key_rc(f, &o, Rc::from(name));
    crate::expr::Place { root: crate::expr::Root::Prop(o, key), path }
}

/// Read-modify-write on a place. mode: 0..=24 = BinOp code (`op=`),
/// 100 = `++`, 101 = `--` (bit 8 = prefix for inc/dec).
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn rt_place_update(
    c: *mut Cx,
    root: *mut Value,
    name: *const u8,
    len: u64,
    keys: *const Value,
    n: u64,
    mode: u64,
    rhs: *const Value,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    let r = (|| -> R<Value> {
        let path = keys_of(it, keys, n, 0)?;
        let obj = if name.is_null() { None } else { Some(root_place(it, f, root, name, len, path.clone())?) };
        let read = |it: &Interp, f: &mut Frame| -> R<Option<Value>> {
            match &obj {
                None => {
                    let mut cur = (*root).clone();
                    for k in &path {
                        cur = match &cur {
                            Value::Array(a) => match a.get(&k.as_ref().unwrap().key(it)?) {
                                Some(v) => v,
                                None => return Ok(None),
                            },
                            Value::Object(o) if o.class.is_a("arrayaccess") => {
                                it.call_method(o, "offsetGet", vec![k.as_ref().unwrap().to_value()])?
                            }
                            _ => return Ok(None),
                        };
                    }
                    Ok(Some(cur))
                }
                Some(p) => it.place_get(f, p),
            }
        };
        let cur = read(it, f)?;
        let (new, result) = match mode {
            100 | 101 | 356 | 357 => {
                let cur = cur.unwrap_or(Value::Null);
                let new = it.incdec(&cur, mode & 0xFF == 100)?;
                let res = if mode & 256 != 0 { new.clone() } else { cur };
                (new, res)
            }
            m if binop_of(m) == BinOp::Coalesce => {
                if let Some(v) = cur.filter(|v| !v.is_null()) {
                    return Ok(v);
                }
                // Without a value: only the check (null = assign next).
                if rhs.is_null() {
                    return Ok(Value::Null);
                }
                ((*rhs).clone(), (*rhs).clone())
            }
            m => {
                let cur = cur.unwrap_or(Value::Null);
                let new = it.binop(binop_of(m), &cur, &*rhs)?;
                (new.clone(), new)
            }
        };
        match &obj {
            None => {
                let mut v = take(root);
                let pending = if path.is_empty() {
                    v = new;
                    Ok(None)
                } else {
                    it.write_path(&mut v, &path, new)
                };
                put(root, v);
                if let Some(p) = pending? {
                    it.finish_pending(p)?;
                }
            }
            Some(p) => it.place_set(f, p, new)?,
        }
        Ok(result)
    })();
    status(it, r, |v| {
        if !dst.is_null() {
            put(dst, v);
        }
    })
}

/// `unset(root[keys...])` / `unset($obj->name)`.
pub unsafe extern "C" fn rt_unset_place(c: *mut Cx, root: *mut Value, name: *const u8, len: u64, keys: *const Value, n: u64) -> u64 {
    let (it, f) = cx(c);
    let r = (|| -> R<()> {
        let path = keys_of(it, keys, n, 0)?;
        if len & STATIC_ROOT != 0 {
            let place = root_place(it, f, root, name, len, vec![])?;
            let crate::expr::Root::Static(sp) = &place.root else { unreachable!() };
            if path.is_empty() {
                return Err(it.error("Error", "Attempt to unset static property"));
            }
            let mut cur = sp.value.borrow_mut().take().unwrap_or_default();
            let r = remove_path(it, &mut cur, &path);
            *sp.value.borrow_mut() = Some(cur);
            return r;
        }
        if !name.is_null() {
            let Value::Object(o) = &*root else { return Ok(()) };
            let pkey = it.prop_key(f, o, name_of(name, len));
            let pname = &*pkey;
            if path.is_empty() {
                if o.props.borrow().contains_key(pname) {
                    if o.class.find_prop(pname).is_some_and(|d| d.readonly) {
                        return Err(it.error("Error", &format!("Cannot unset readonly property {}::${pname}", o.class.name)));
                    }
                    o.props.borrow_mut().shift_remove(pname);
                } else if it.prop_visible(f, o, pname) && it.prop_uninit(o, pname) {
                    o.props.borrow_mut().mark_unset(pname);
                } else if it.has_magic(o, crate::expr::MAGIC_UNSET, pname) {
                    it.call_magic(o, crate::expr::MAGIC_UNSET, pname, None)?;
                }
                return Ok(());
            }
            // unset($obj->p[k]): edit the property's array in place.
            let mut cur = o.props.borrow_mut().get_mut(pname).map(std::mem::take).unwrap_or_default();
            remove_path(it, &mut cur, &path)?;
            o.props.borrow_mut().insert(Rc::from(pname), cur);
            return Ok(());
        }
        let mut cur = take(root);
        let r = remove_path(it, &mut cur, &path);
        put(root, cur);
        r
    })();
    status(it, r, |_| {})
}

fn remove_path(it: &Interp, cur: &mut Value, path: &[Option<crate::expr::Off>]) -> R<()> {
    let Some((last, rest)) = path.split_last() else { return Ok(()) };
    let mut c = cur;
    for k in rest {
        match c {
            Value::Array(a) => match k.as_ref().and_then(|k| k.key_opt()).and_then(|k| Rc::make_mut(a).get_mut(k)) {
                Some(next) => c = next,
                None => return Ok(()),
            },
            Value::Ref(s) => {
                let s = s.clone();
                return remove_path(it, &mut s.borrow_mut(), path);
            }
            _ => return Ok(()),
        }
    }
    match c {
        Value::Array(a) => {
            let k = last.as_ref().unwrap().key(it)?;
            Rc::make_mut(a).remove(&k);
        }
        Value::Object(o) if o.class.is_a("arrayaccess") => {
            it.call_method(o, "offsetUnset", vec![last.as_ref().unwrap().to_value()])?;
        }
        _ => {}
    }
    Ok(())
}

// ---- calls ------------------------------------------------------------------------

/// Gather consumed args; for by-reference params whose argument is a local,
/// pass a cell and copy the result back afterwards (copy-in/copy-out).
unsafe fn take_args(args: *mut Value, n: u64) -> Vec<Value> {
    (0..n as usize).map(|i| take(args.add(i))).collect()
}

/// Generated code puts one more word after the `locals` pointers: a bit
/// per argument the runtime bound by reference, so it can store
/// property/element arguments back.
unsafe fn mark_by_ref(locals: *const *mut Value, n: usize, i: usize) {
    if i < 64 {
        let mask = locals.add(n) as *mut u64;
        *mask |= 1 << i;
    }
}

unsafe fn with_refs(
    params: Option<&'static [ast::Param]>,
    pos: Vec<Value>,
    locals: *const *mut Value,
    call: impl FnOnce(Vec<Value>) -> R<Value>,
) -> R<Value> {
    let n = pos.len();
    with_refs_masked(params, pos, locals, n, call)
}

/// `with_refs` where the generated code passed `n` arguments in all (the
/// by-reference mask follows the `n` local pointers).
unsafe fn with_refs_masked(
    params: Option<&'static [ast::Param]>,
    mut pos: Vec<Value>,
    locals: *const *mut Value,
    n: usize,
    call: impl FnOnce(Vec<Value>) -> R<Value>,
) -> R<Value> {
    let mut cells = vec![];
    if let Some(ps) = params {
        for i in 0..pos.len() {
            let by_ref = match ps.get(i) {
                Some(p) => p.by_ref,
                None => ps.last().is_some_and(|p| p.variadic && p.by_ref),
            };
            let local = *locals.add(i);
            if by_ref && !local.is_null() {
                let cell = slot(take(local));
                pos[i] = Value::Ref(cell.clone());
                cells.push((local, cell));
                mark_by_ref(locals, n, i);
            }
        }
    }
    let r = call(pos);
    for (local, cell) in cells {
        put(local, cell.borrow().clone());
    }
    r
}

/// A `foo(...)` call site. After the first native call it remembers the
/// target (functions can't be redeclared), skipping lookups.
/// `#[repr(C)]`: generated code reads `direct` and `frame` to call the
/// target without going through `rt_call`.
#[repr(C)]
pub struct CallSite {
    /// The cached native entry once compiled code may call it directly
    /// (0 until then).
    pub direct: std::cell::Cell<usize>,
    /// A frame reused by calls from this site (rebuilding one per call is
    /// a large part of call overhead); recursion falls back to a fresh one.
    pub frame: SiteFrame,
    pub name: &'static str,
    /// (entry, function). Plain pointer: functions are never freed.
    pub cached: std::cell::Cell<Option<(crate::compiler::Entry, *const crate::interp::UserFn)>>,
    /// Resolved plain built-in (no by-ref params, no caller-scope access),
    /// with its PHP signature.
    pub builtin: std::cell::Cell<Option<crate::interp::BuiltinFn>>,
    pub bspec: std::cell::Cell<Option<&'static crate::arginfo::Spec>>,
    /// Resolved built-in with by-reference parameters (preg_match, sort,
    /// ...): the function and those parameters' positions.
    pub builtin_ref: std::cell::Cell<Option<(crate::interp::BuiltinFn, &'static [usize])>>,
    /// The numeric JIT's entry for the argument types last seen here.
    pub jit: std::cell::Cell<Option<JitSite>>,
}

#[derive(Clone, Copy)]
pub struct JitSite {
    /// Argument types (`phpx_jit::Ty`), one per argument.
    tys: [phpx_jit::Ty; 4],
    n: u8,
    entry: phpx_jit::Entry,
    ret: phpx_jit::Ty,
}

/// Call the cached numeric-JIT entry if the arguments have the cached
/// types. None = not taken (arguments untouched).
unsafe fn jit_site_call(site: &CallSite, args: *mut Value, n: u64, dst: *mut Value) -> Option<u64> {
    use phpx_jit::Ty;
    let js = site.jit.get()?;
    if n != js.n as u64 {
        return None;
    }
    let mut bits = [0i64; 4];
    for i in 0..n as usize {
        bits[i] = match (&*args.add(i), js.tys[i]) {
            (Value::Int(v), Ty::Int) => *v,
            (Value::Float(v), Ty::Float) => v.to_bits() as i64,
            (Value::Bool(v), Ty::Bool) => *v as i64,
            _ => return None,
        };
    }
    let mut out = 0i64;
    if (js.entry)(bits.as_ptr(), &mut out) != 0 {
        // Deoptimized or interrupted: the general path reruns it (the
        // code is side-effect free) and keeps the JIT's statistics.
        site.jit.set(None);
        return None;
    }
    // Numeric arguments: nothing to drop.
    put(
        dst,
        match js.ret {
            Ty::Int => Value::Int(out),
            Ty::Float => Value::Float(f64::from_bits(out as u64)),
            Ty::Bool => Value::Bool(out != 0),
        },
    );
    Some(0)
}

/// A reusable callee frame, lent out while not busy. `#[repr(C)]`: the
/// frame pointer (0 = none yet) and busy flag are read by generated code.
#[derive(Default)]
#[repr(C)]
pub struct SiteFrame {
    frame: std::cell::UnsafeCell<Option<Box<Frame>>>,
    busy: std::cell::Cell<bool>,
    /// Frames replaced while in use (the cached callee changed during a
    /// recursive call): kept alive until that call has returned.
    retired: std::cell::UnsafeCell<Vec<Box<Frame>>>,
}

/// After a direct call from generated code: ready the site frame for the
/// next call.
pub unsafe extern "C" fn rt_site_reset(frame: *mut Frame) {
    (*frame).reset_after_call();
}

impl SiteFrame {
    /// Run `g` with this site's frame (built by `make` on first use) or,
    /// when it's in use (recursion), with a fresh one.
    unsafe fn with<T>(&self, make: impl FnOnce() -> Frame, g: impl FnOnce(&mut Frame) -> T) -> T {
        if self.busy.get() {
            let mut f = make();
            return g(&mut f);
        }
        self.busy.set(true);
        (*self.retired.get()).clear();
        let slot = &mut *self.frame.get();
        // The frame itself (on the heap), not the slot: the slot may be
        // emptied while the call runs (see `invalidate`).
        let f: &mut Frame = &mut **slot.get_or_insert_with(|| Box::new(make()));
        let r = g(f);
        f.reset_after_call();
        self.busy.set(false);
        r
    }

    /// Forget the frame (the cached callee changed). A frame in use is
    /// retired rather than dropped: the running call still holds it, and it
    /// must not be reused for the new callee (its scope would be stale).
    fn invalidate(&self) {
        unsafe {
            let slot = &mut *self.frame.get();
            if self.busy.get() {
                if let Some(f) = slot.take() {
                    (*self.retired.get()).push(f);
                }
            } else {
                *slot = None;
            }
        }
    }
}

/// A `$obj->m(...)` call site with a monomorphic inline cache (also used
/// by static calls, `new` and `$f(...)`). `#[repr(C)]`: generated code
/// reads the `direct*` fields and the frame for direct method calls.
#[repr(C)]
pub struct MethodSite {
    /// Compiled entry of the cached method when it may be called directly
    /// (0: not), for objects of class `direct_class` called from scope
    /// `direct_scope`.
    pub direct: std::cell::Cell<usize>,
    pub direct_class: std::cell::Cell<usize>,
    pub direct_scope: std::cell::Cell<usize>,
    pub frame: SiteFrame,
    pub name: &'static str,
    pub lname: &'static str,
    pub cached: std::cell::Cell<Option<MethodCache>>,
    /// The method last found here while it isn't compiled: (class, scope,
    /// method), called through the interpreter without looking it up again.
    pub interp: std::cell::RefCell<Option<(usize, usize, Rc<crate::class::Method>)>>,
}

impl MethodSite {
    pub fn new(name: &'static str, lname: &'static str) -> MethodSite {
        MethodSite {
            direct: Default::default(),
            direct_class: Default::default(),
            direct_scope: Default::default(),
            frame: Default::default(),
            name,
            lname,
            cached: Default::default(),
            interp: Default::default(),
        }
    }
}

/// Plain pointers: classes and methods are never freed while running.
#[derive(Clone, Copy)]
pub struct MethodCache {
    class: *const crate::class::Class,
    /// Calling scope the visibility check passed for.
    scope: *const crate::class::Class,
    entry: crate::compiler::Entry,
    method: *const crate::class::Method,
    decl_class: *const crate::class::Class,
    /// Static call sites (`A::m()`): the frame's `static` class, and the
    /// caller's `static` and `$this` classes the entry is valid for.
    sc: *const crate::class::Class,
    static_cls: *const crate::class::Class,
    this_cls: *const crate::class::Class,
}

/// Re-materialize an Rc from a cached pointer (taking a new reference).
unsafe fn rc_from(p: *const crate::class::Class) -> Option<Rc<crate::class::Class>> {
    if p.is_null() {
        return None;
    }
    Rc::increment_strong_count(p);
    Some(Rc::from_raw(p))
}

/// Native entry for a user method (None: abstract, native, or doesn't compile).
pub(crate) unsafe fn method_entry(it: &Interp, m: &Rc<crate::class::Method>) -> Option<(crate::compiler::Entry, Option<Rc<crate::class::Class>>)> {
    let crate::class::MethodBody::User(func) = &m.body else { return None };
    let body = func.body.as_deref()?;
    let fn_id = *func as *const ast::Function as usize;
    let (decl_class, display) = m.declared(&it.classes.borrow());
    let entry = it.native_entry(fn_id, || crate::compiler::Target {
        fn_id,
        display,
        params: &func.params,
        captured: vec![],
        body: crate::compiler::Body::Block(body),
        ctx: m.ctx.clone(),
        class: decl_class.clone(),
        ret: func.ret.as_ref(),
    })?;
    Some((entry, decl_class))
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn rt_call(
    c: *mut Cx,
    site: *const CallSite,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    if !meta.is_null() {
        // Spread / named arguments: the general path.
        let r = it.resolve_call(f, (*site).name).and_then(|callable| {
            let params = crate::call::callable_params(&callable);
            call_expanded(it, take_args(args, n), meta, params, locals, |pos, named| it.call_callable(&callable, pos, named))
        });
        return status(it, r, |v| put(dst, v));
    }
    if let Some(fun) = (*site).builtin.get() {
        // The generated code laid the arguments out contiguously: lend them
        // to the built-in in place, then drop them (leaving null slots).
        let vals = std::slice::from_raw_parts_mut(args, n as usize);
        for v in vals.iter_mut() {
            if let Value::Ref(_) = v {
                *v = std::mem::take(v).into_deref();
            }
        }
        let r = crate::builtins::invoke(it, (*site).name, (*site).bspec.get(), fun, f, vals);
        for v in vals.iter_mut() {
            if !matches!(v, Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_)) {
                drop(std::mem::take(v));
            }
        }
        return status(it, r, |v| put(dst, v));
    }
    if let Some((fun, refs)) = (*site).builtin_ref.get() {
        // By-reference built-in: lend the variables' values, write them back.
        let vals = std::slice::from_raw_parts_mut(args, n as usize);
        for (i, v) in vals.iter_mut().enumerate() {
            if refs.contains(&i) && !locals.is_null() {
                let local = *locals.add(i);
                if !local.is_null() {
                    *v = take(local);
                    mark_by_ref(locals, n as usize, i);
                    continue;
                }
            }
            if let Value::Ref(_) = v {
                *v = std::mem::take(v).into_deref();
            }
        }
        let r = crate::builtins::invoke(it, (*site).name, (*site).bspec.get(), fun, f, vals);
        for (i, v) in vals.iter_mut().enumerate() {
            if refs.contains(&i) && !locals.is_null() {
                let local = *locals.add(i);
                if !local.is_null() {
                    put(local, std::mem::take(v));
                    continue;
                }
            }
            drop(std::mem::take(v));
        }
        return status(it, r, |v| put(dst, v));
    }
    if let Some(st) = jit_site_call(&*site, args, n, dst) {
        return st;
    }
    if let Some((entry, u)) = (*site).cached.get() {
        let u = &*u;
        let make = || Frame::for_call(u.ctx.clone(), u.name.clone(), u.decl as *const ast::Function as usize);
        if let Some(st) = (*site).frame.with(make, |frame| enter(it, entry, frame, args, n, dst)) {
            return st;
        }
    }
    let callable = match it.resolve_call(f, (*site).name) {
        Ok(c) => c,
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            return 1;
        }
    };
    if let Callable::User(u) = &callable {
        if let Some(st) = fast_user_call(it, u, args, n, dst, &*site) {
            return st;
        }
    }
    let pos = take_args(args, n);
    let r = (|| -> R<Value> {
        if let Callable::Builtin(bname, fun) = &callable {
            // get_class()/get_parent_class() without args read the caller's scope.
            if pos.is_empty() && matches!(*bname, "get_class" | "get_parent_class") {
                let cls = if *bname == "get_class" { f.class.clone() } else { f.class.as_ref().and_then(|c| c.parent.clone()) };
                return Ok(cls.map(|c| Value::str(&c.name)).unwrap_or(Value::Bool(false)));
            }
            let refs = crate::call::by_ref_params(bname);
            if !refs.is_empty() {
                (*site).builtin_ref.set(Some((*fun, refs)));
                (*site).bspec.set(crate::arginfo::spec(bname));
                let mut vals = pos;
                let n = vals.len();
                let idxs: Vec<usize> = refs.iter().copied().filter(|i| *i < vals.len()).collect();
                for &idx in &idxs {
                    let local = *locals.add(idx);
                    if !local.is_null() {
                        vals[idx] = take(local);
                        mark_by_ref(locals, n, idx);
                    }
                }
                let r = crate::builtins::invoke(it, bname, crate::arginfo::spec(bname), *fun, f, &mut vals);
                for &idx in &idxs {
                    let local = *locals.add(idx);
                    if !local.is_null() {
                        put(local, std::mem::take(&mut vals[idx]));
                    }
                }
                return r;
            }
            // Plain built-in: call it directly with the caller's frame, and
            // remember it at this call site.
            let spec = crate::arginfo::spec(bname);
            if !matches!(*bname, "get_class" | "get_parent_class") {
                (*site).builtin.set(Some(*fun));
                (*site).bspec.set(spec);
            }
            let mut vals: Vec<Value> = pos.into_iter().map(Value::into_deref).collect();
            return crate::builtins::invoke(it, bname, spec, *fun, f, &mut vals);
        }
        let params = crate::call::callable_params(&callable);
        with_refs(params, pos, locals, |pos| it.call_callable(&callable, pos, vec![]))
    })();
    status(it, r, |v| put(dst, v))
}

/// Call a native-compiled user function directly with the argument array
/// (numeric JIT first). None = take the general path (args untouched).
unsafe fn fast_user_call(it: &Interp, u: &Rc<crate::interp::UserFn>, args: *mut Value, n: u64, dst: *mut Value, site: &CallSite) -> Option<u64> {
    if let Some(v) = it.try_jit(u, std::slice::from_raw_parts(args, n as usize)) {
        remember_jit(it, u, args, n, site);
        for i in 0..n as usize {
            drop(take(args.add(i)));
        }
        put(dst, v);
        return Some(0);
    }
    let fn_id = u.decl as *const ast::Function as usize;
    let entry = it.native_entry(fn_id, || crate::compiler::Target {
        fn_id,
        display: u.name.clone(),
        params: &u.decl.params,
        captured: vec![],
        body: crate::compiler::Body::Block(u.decl.body.as_deref().unwrap_or(&[])),
        ctx: u.ctx.clone(),
        class: None,
        ret: u.decl.ret.as_ref(),
    })?;
    // Remember the native target at this call site, unless the numeric JIT
    // may still take this function later (all-numeric args, not rejected):
    // then keep asking it.
    let args_numeric = std::slice::from_raw_parts(args, n as usize)
        .iter()
        .all(|v| matches!(v, Value::Int(_) | Value::Float(_) | Value::Bool(_)));
    let jit_may_take = args_numeric && it.jit.borrow().as_ref().is_some_and(|j| !j.rejected(u.decl));
    if !jit_may_take {
        site.cached.set(Some((entry, Rc::as_ptr(u))));
        site.frame.invalidate();
        // Direct calls from compiled code, unless binding would always
        // decline (by-reference or variadic parameters).
        if u.decl.params.iter().all(|p| !p.by_ref && !p.variadic) {
            site.direct.set(entry as usize);
        }
    }
    let mut frame = Frame::for_call(u.ctx.clone(), u.name.clone(), fn_id);
    enter(it, entry, &mut frame, args, n, dst)
}

/// The JIT ran this call: cache its entry at the site for these argument
/// types (not for `float` parameters, where ints are widened first).
unsafe fn remember_jit(it: &Interp, u: &crate::interp::UserFn, args: *mut Value, n: u64, site: &CallSite) {
    use phpx_jit::Ty;
    if n > 4 || u.decl.params.iter().any(|p| p.ty.is_some()) {
        return;
    }
    let mut tys = [Ty::Int; 4];
    for i in 0..n as usize {
        tys[i] = match &*args.add(i) {
            Value::Int(_) => Ty::Int,
            Value::Float(_) => Ty::Float,
            Value::Bool(_) => Ty::Bool,
            _ => return,
        };
    }
    let Ok(jit) = it.jit.try_borrow() else { return };
    if let Some((entry, ret)) = jit.as_ref().and_then(|j| j.ready_entry(u.decl, &tys[..n as usize])) {
        site.jit.set(Some(JitSite { tys, n: n as u8, entry, ret }));
    }
}

/// Run a native entry with a fresh frame; applies the declared return type.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn enter(it: &Interp, entry: crate::compiler::Entry, frame: &mut Frame, args: *mut Value, n: u64, dst: *mut Value) -> Option<u64> {
    if phpx_runtime::stack_remaining().is_some_and(|r| r < 256 * 1024) {
        return None; // the general path raises the proper error
    }
    let saved = it.pos.get();
    let mut cx2 = Cx { it, frame };
    // (The callee checks its declared return type itself.)
    let st = entry(&mut cx2, args, n, dst);
    it.pos.set(saved);
    match st {
        2 => None,
        st => Some(st),
    }
}

pub unsafe extern "C" fn rt_method_call(
    c: *mut Cx,
    obj: *const Value,
    site: *const MethodSite,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    let site = &*site;
    let name = site.name;
    if !meta.is_null() {
        return method_call_general(it, f, obj, name, args, n, locals, meta, dst);
    }
    if let Value::Object(o) = &*obj {
        let scope = f.class.as_ref().map_or(ptr::null(), Rc::as_ptr);
        let hit = match site.cached.get() {
            Some(mc) if mc.class == Rc::as_ptr(&o.class) && mc.scope == scope => Some(mc),
            _ => None,
        };
        let key = (Rc::as_ptr(&o.class) as usize, scope as usize);
        let hit = hit.or_else(|| {
            // Miss: look the method up (or take the interpreted one found
            // before) and, if it compiles, cache it.
            let known = site.interp.borrow().as_ref().filter(|e| (e.0, e.1) == key).map(|e| e.2.clone());
            let m = match known {
                Some(m) => m,
                None => {
                    let m = o.class.find_method_from(site.lname, f.class.as_deref())?.clone();
                    if !it.member_visible(f.class.as_deref(), m.vis, &m.declaring) {
                        return None;
                    }
                    *site.interp.borrow_mut() = Some((key.0, key.1, m.clone()));
                    m
                }
            };
            let (entry, decl_class) = method_entry(it, &m)?;
            let mc = MethodCache {
                class: Rc::as_ptr(&o.class),
                scope,
                entry,
                method: Rc::as_ptr(&m),
                decl_class: decl_class.as_ref().map_or(ptr::null(), Rc::as_ptr),
                sc: ptr::null(),
                static_cls: ptr::null(),
                this_cls: ptr::null(),
            };
            site.cached.set(Some(mc));
            site.frame.invalidate();
            // Direct calls from compiled code: instance methods whose
            // binding won't always decline.
            let direct = !m.is_static && crate::call::method_params(&m).is_some_and(|ps| ps.iter().all(|p| !p.by_ref && !p.variadic));
            site.direct.set(if direct { entry as usize } else { 0 });
            // Keyed the way generated code reads them: the raw Rc words.
            site.direct_class.set(*(std::ptr::addr_of!(o.class) as *const usize));
            site.direct_scope.set(scope_key(f));
            Some(mc)
        });
        if let Some(mc) = hit {
            let m = &*mc.method;
            let crate::class::MethodBody::User(func) = &m.body else { unreachable!() };
            let make = || {
                let decl_class = rc_from(mc.decl_class);
                let mut frame = Frame::for_call(m.ctx.clone(), m.name.clone(), *func as *const ast::Function as usize);
                frame.static_class = Some(o.class.clone());
                frame.class = decl_class;
                frame
            };
            let r = site.frame.with(make, |frame| {
                if !m.is_static {
                    frame.this = Some(o.clone());
                }
                enter(it, mc.entry, frame, args, n, dst)
            });
            if let Some(st) = r {
                return st;
            }
        } else {
            // Not compiled (yet): the interpreter, without another lookup.
            let known = site.interp.borrow().as_ref().filter(|e| (e.0, e.1) == key).map(|e| e.2.clone());
            if let Some(m) = known.filter(|m| matches!(m.body, crate::class::MethodBody::User(f) if f.body.is_some())) {
                let vals = take_args(args, n);
                let o = o.clone();
                let r = with_refs(crate::call::method_params(&m), vals, locals, |pos| it.invoke_method(Some(o.clone()), &m, o.class.clone(), pos, vec![]));
                return status(it, r, |v| put(dst, v));
            }
        }
    }
    method_call_general(it, f, obj, name, args, n, locals, ptr::null(), dst)
}

/// `$obj->name(...)` without the inline cache (also dynamic names, and
/// spread/named arguments when `meta` is set).
#[allow(clippy::too_many_arguments)]
unsafe fn method_call_general(
    it: &Interp,
    f: &mut Frame,
    obj: *const Value,
    name: &str,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let vals = take_args(args, n);
    let r = match &*obj {
        Value::Object(o) => {
            let o = o.clone();
            if meta.is_null() {
                let params = o.class.find_method_from(&name.to_ascii_lowercase(), f.class.as_deref()).and_then(|m| crate::call::method_params(m));
                with_refs(params, vals, locals, |pos| it.method_call(f, &o, name, pos, vec![]))
            } else {
                let params = o.class.find_method_from(&name.to_ascii_lowercase(), f.class.as_deref()).and_then(|m| crate::call::method_params(m));
                call_expanded(it, vals, meta, params, locals, |pos, named| it.method_call(f, &o, name, pos, named))
            }
        }
        other => Err(it.error("Error", &format!("Call to a member function {name}() on {}", other.type_name()))),
    };
    status(it, r, |v| put(dst, v))
}

/// A call with named (or spread) arguments. Positional arguments still
/// bind by reference where the callee takes them so (they come first, so
/// their indices match the generated code's local pointers).
unsafe fn call_expanded(
    it: &Interp,
    vals: Vec<Value>,
    meta: *const ast::Arg,
    params: Option<&'static [ast::Param]>,
    locals: *const *mut Value,
    call: impl FnOnce(Vec<Value>, Vec<(Rc<str>, Value)>) -> R<Value>,
) -> R<Value> {
    let ast_args = std::slice::from_raw_parts(meta, vals.len());
    if locals.is_null() || ast_args.iter().any(|a| a.spread) {
        let (pos, named) = expand_args(it, vals, meta)?;
        return call(pos, named);
    }
    let n = ast_args.len();
    let (pos, named) = expand_args(it, vals, meta)?;
    with_refs_masked(params, pos, locals, n, |pos| call(pos, named))
}

/// Spread and named arguments, expanded like the interpreter's eval_args.
unsafe fn expand_args(it: &Interp, vals: Vec<Value>, meta: *const ast::Arg) -> R<(Vec<Value>, Vec<(Rc<str>, Value)>)> {
    let ast_args = std::slice::from_raw_parts(meta, vals.len());
    let mut pos = vec![];
    let mut named = vec![];
    for (v, a) in vals.into_iter().zip(ast_args) {
        if a.spread {
            let pairs: Vec<(Value, Value)> = match v {
                Value::Array(arr) => arr.iter().map(|(k, v)| (k.to_value(), v)).collect(),
                Value::Object(o) if o.class.is_a("traversable") => it.iterate(&o)?,
                _ => return Err(it.error("Error", "Only arrays and Traversables can be unpacked")),
            };
            for (k, v) in pairs {
                match k {
                    Value::Str(s) => named.push((Rc::from(String::from_utf8_lossy(&s).as_ref()), v)),
                    _ => pos.push(v),
                }
            }
        } else if let Some(n) = &a.name {
            named.push((Rc::from(&*n.name), v));
        } else {
            if !named.is_empty() {
                return Err(it.error("Error", "Cannot use positional argument after named argument"));
            }
            pos.push(v);
        }
    }
    Ok((pos, named))
}

/// `$f(...)`: call any callable value.
pub unsafe extern "C" fn rt_call_value(
    c: *mut Cx,
    callee: *const Value,
    site: *const MethodSite,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    if meta.is_null() {
        if let Value::Object(o) = &*callee {
            if let Some(st) = closure_call(it, o, &*site, args, n, dst) {
                return st;
            }
        }
    }
    let vals = take_args(args, n);
    let r = it.to_callable(f, &*callee).and_then(|callable| {
        if meta.is_null() {
            let params = crate::call::callable_params(&callable);
            with_refs(params, vals, locals, |pos| it.call_callable(&callable, pos, vec![]))
        } else {
            let (pos, named) = expand_args(it, vals, meta)?;
            it.call_callable(&callable, pos, named)
        }
    });
    status(it, r, |v| put(dst, v))
}

/// `$f(...)` on a closure: its compiled body, entered directly with the
/// arguments and captured values (no interpreter frame variables). None =
/// not taken (arguments untouched).
unsafe fn closure_call(it: &Interp, o: &Rc<crate::value::Obj>, site: &MethodSite, args: *mut Value, n: u64, dst: *mut Value) -> Option<u64> {
    let cd = match &*o.internal() {
        crate::value::Internal::Closure(c) => match c {
            Callable::Closure(cd) => cd.clone(),
            _ => return None,
        },
        _ => return None,
    };
    let (node, params, body, ret) = match &cd.kind {
        crate::interp::ClosureKind::Closure(c) if !c.is_generator => {
            (*c as *const ast::Closure as usize, &c.params, crate::compiler::Body::Block(&c.body), c.ret.as_ref())
        }
        crate::interp::ClosureKind::Arrow(a) if !a.is_generator => (*a as *const ast::ArrowFn as usize, &a.params, crate::compiler::Body::Expr(&a.body), a.ret.as_ref()),
        _ => return None,
    };
    let entry = match site.cached.get() {
        Some(mc) if mc.class as usize == node => mc.entry,
        _ => {
            let entry = it.native_entry(node, || crate::compiler::Target {
                fn_id: node,
                display: cd.name.clone(),
                params,
                captured: cd.captured.iter().map(|(n, _, _)| *n).collect(),
                body,
                ctx: cd.ctx.clone(),
                class: cd.scope.clone(),
                ret,
            })?;
            // (The cache's class field holds the closure's AST node here.)
            site.cached.set(Some(MethodCache {
                class: node as *const crate::class::Class,
                scope: ptr::null(),
                entry,
                method: ptr::null(),
                decl_class: ptr::null(),
                sc: ptr::null(),
                static_cls: ptr::null(),
                this_cls: ptr::null(),
            }));
            site.frame.invalidate();
            entry
        }
    };
    let make = || Frame::for_call(cd.ctx.clone(), cd.name.clone(), node);
    site.frame.with(make, |frame| enter_closure(it, &cd, entry, frame, args, n, dst))
}

/// Run a closure's native entry in `frame` (set up here for this closure).
#[allow(clippy::too_many_arguments)]
pub unsafe fn enter_closure(
    it: &Interp,
    cd: &Rc<crate::interp::ClosureData>,
    entry: crate::compiler::Entry,
    frame: &mut Frame,
    args: *mut Value,
    n: u64,
    dst: *mut Value,
) -> Option<u64> {
    frame.this = cd.this.clone();
    frame.class = cd.scope.clone();
    frame.static_class = cd.static_class.clone();
    frame.func = cd.name.clone();
    frame.closure = Some(cd.clone());
    enter(it, entry, frame, args, n, dst)
}

/// `$obj->$name(...)`.
pub unsafe extern "C" fn rt_method_call_dyn(
    c: *mut Cx,
    obj: *const Value,
    name: *const Value,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    match it.to_string_lossy(&*name) {
        Ok(nm) => method_call_general(it, f, obj, &nm, args, n, locals, meta, dst),
        Err(e) => {
            drop(take_args(args, n));
            *it.pending.borrow_mut() = Some(e);
            1
        }
    }
}

/// `$obj->$name` read.
pub unsafe extern "C" fn rt_prop_get_dyn(c: *mut Cx, obj: *const Value, name: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    match it.to_string_lossy(&*name) {
        Ok(nm) => rt_prop_get(c, obj, nm.as_ptr(), nm.len() as u64, dst),
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            1
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn rt_static_call(
    c: *mut Cx,
    cref: *const ast::ClassRef,
    site: *const MethodSite,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    let site = &*site;
    let cref = &*cref;
    if !meta.is_null() || !matches!(cref, ast::ClassRef::Name(n) if !n.starts_with('\x01')) {
        let cls = match it.class_ref(f, cref) {
            Ok(c) => c,
            Err(e) => return status(it, Err::<(), _>(e), |_| ()),
        };
        return static_call_general(it, f, cref, cls, site.name, args, n, locals, meta, dst);
    }
    // A named class (or self/parent/static) resolves the same way for the
    // same calling scope, `static` and `$this` class: cache per those.
    let ptr_of = |c: Option<&Rc<crate::class::Class>>| c.map_or(ptr::null(), Rc::as_ptr);
    let scope = ptr_of(f.class.as_ref());
    let stc = ptr_of(f.static_class.as_ref());
    let thc = ptr_of(f.this.as_ref().map(|o| &o.class));
    let mut hit = match site.cached.get() {
        Some(mc) if mc.scope == scope && mc.static_cls == stc && mc.this_cls == thc => Some(mc),
        _ => None,
    };
    let mut resolved = None;
    if hit.is_none() {
        let cls = match it.class_ref(f, cref) {
            Ok(c) => c,
            Err(e) => return status(it, Err::<(), _>(e), |_| ()),
        };
        hit = static_lookup(it, f, cref, &cls, site, scope, stc, thc);
        resolved = Some(cls);
    }
    if let Some(mc) = hit {
        let m = &*mc.method;
        let crate::class::MethodBody::User(func) = &m.body else { unreachable!() };
        let make = || {
            let mut frame = Frame::for_call(m.ctx.clone(), m.name.clone(), *func as *const ast::Function as usize);
            frame.static_class = rc_from(mc.sc);
            frame.class = rc_from(mc.decl_class);
            frame
        };
        let this = if m.is_static { None } else { f.this.clone() };
        let r = site.frame.with(make, |frame| {
            frame.this = this;
            enter(it, mc.entry, frame, args, n, dst)
        });
        if let Some(st) = r {
            return st;
        }
    }
    let cls = match resolved {
        Some(c) => c,
        None => match it.class_ref(f, cref) {
            Ok(c) => c,
            Err(e) => return status(it, Err::<(), _>(e), |_| ()),
        },
    };
    static_call_general(it, f, cref, cls, site.name, args, n, locals, ptr::null(), dst)
}

/// Resolve `A::m()` for the inline cache: a visible, compiled user method,
/// static or (with a fitting `$this`) an instance method.
#[allow(clippy::too_many_arguments)]
unsafe fn static_lookup(
    it: &Interp,
    f: &Frame,
    cref: &ast::ClassRef,
    cls: &Rc<crate::class::Class>,
    site: &MethodSite,
    scope: *const crate::class::Class,
    stc: *const crate::class::Class,
    thc: *const crate::class::Class,
) -> Option<MethodCache> {
    let m = cls.find_method(site.lname)?.clone();
    if !it.member_visible(f.class.as_deref(), m.vis, &m.declaring) {
        return None;
    }
    let sc = if m.is_static {
        let forwarding = matches!(cref, ast::ClassRef::Name(n) if matches!(&*n.to_ascii_lowercase(), "self" | "parent" | "static"));
        if forwarding { f.static_class.clone().unwrap_or_else(|| cls.clone()) } else { cls.clone() }
    } else {
        // `parent::m()` / `A::m()` from an instance of A: keeps `$this`.
        let this = f.this.as_ref().filter(|o| o.class.is_subclass_of(cls))?;
        this.class.clone()
    };
    let (entry, decl_class) = method_entry(it, &m)?;
    // Classes live for the whole run: the cache holds plain pointers.
    let mc = MethodCache {
        class: Rc::as_ptr(cls),
        scope,
        entry,
        method: Rc::as_ptr(&m),
        decl_class: decl_class.as_ref().map_or(ptr::null(), Rc::as_ptr),
        sc: Rc::as_ptr(&sc),
        static_cls: stc,
        this_cls: thc,
    };
    site.cached.set(Some(mc));
    site.frame.invalidate();
    // Direct calls from compiled code (static methods only: no `$this` to
    // lend), keyed by the caller's raw scope and `static` class words.
    let direct = m.is_static && crate::call::method_params(&m).is_some_and(|ps| ps.iter().all(|p| !p.by_ref && !p.variadic));
    site.direct.set(if direct { entry as usize } else { 0 });
    site.direct_class.set(*(std::ptr::addr_of!(f.static_class) as *const usize));
    site.direct_scope.set(scope_key(f));
    Some(mc)
}

#[allow(clippy::too_many_arguments)]
unsafe fn static_call_general(
    it: &Interp,
    f: &mut Frame,
    cref: &'static ast::ClassRef,
    cls: Rc<crate::class::Class>,
    name: &str,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let pos = take_args(args, n);
    let r = (|| {
        if !meta.is_null() {
            let params = cls.find_method(&name.to_ascii_lowercase()).and_then(|m| crate::call::method_params(m));
            return call_expanded(it, pos, meta, params, locals, |pos, named| it.static_call(f, cref, cls.clone(), name, pos, named));
        }
        let params = cls.find_method(&name.to_ascii_lowercase()).and_then(|m| crate::call::method_params(m));
        with_refs(params, pos, locals, |pos| it.static_call(f, cref, cls.clone(), name, pos, vec![]))
    })();
    status(it, r, |v| put(dst, v))
}

pub unsafe extern "C" fn rt_new(
    c: *mut Cx,
    cref: *const ast::ClassRef,
    site: *const MethodSite,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    let site = &*site;
    let cref = &*cref;
    if meta.is_null() && matches!(cref, ast::ClassRef::Name(n) if !n.starts_with('\x01')) {
        if let Some(st) = new_cached(it, f, cref, site, args, n, dst) {
            return st;
        }
    }
    let pos = take_args(args, n);
    let r = it.class_ref(f, cref).and_then(|cls| {
        // `new C` of a plain class without a constructor: no call machinery.
        if cls.plain_new && cls.ctor.is_none() && pos.is_empty() {
            return Ok(Value::Object(it.new_instance(&cls)?));
        }
        if !meta.is_null() {
            let params = cls.find_method("__construct").and_then(|m| crate::call::method_params(m));
            return call_expanded(it, pos, meta, params, locals, |pos, named| it.instantiate(f, &cls, pos, named).map(Value::Object));
        }
        let params = cls.find_method("__construct").and_then(|m| crate::call::method_params(m));
        with_refs(params, pos, locals, |pos| it.instantiate(f, &cls, pos, vec![]).map(Value::Object))
    });
    status(it, r, |v| put(dst, v))
}

/// `new C(...)` through the site cache: the class (resolved per calling
/// scope and `static`) and its compiled constructor. None = not cached
/// (nothing consumed); the general path resolves and reports errors.
unsafe fn new_cached(it: &Interp, f: &mut Frame, cref: &'static ast::ClassRef, site: &MethodSite, args: *mut Value, n: u64, dst: *mut Value) -> Option<u64> {
    let mc = match new_cache(it, f, cref, site) {
        Ok(Some(mc)) => mc,
        Ok(None) => return None,
        Err(e) => {
            // Arguments are dropped with the error.
            drop(take_args(args, n));
            return Some(status(it, Err::<(), _>(e), |_| ()));
        }
    };
    if mc.method.is_null() && n != 0 {
        return None;
    }
    let o = match new_object(it, &mc) {
        Ok(o) => o,
        Err(e) => return Some(status(it, Err::<(), _>(e), |_| ())),
    };
    if mc.method.is_null() {
        put(dst, Value::Object(o));
        return Some(0);
    }
    let mut out = Value::Null;
    let st = run_ctor(it, site, &mc, &o, args, n, ptr::null(), &mut out);
    if st == 0 {
        put(dst, Value::Object(o));
    }
    Some(st)
}

/// The class (resolved for this calling scope and `static`) and compiled
/// constructor `new` at this site uses, filling the cache on a miss. None:
/// not cacheable (the general path resolves and reports errors).
unsafe fn new_cache(it: &Interp, f: &Frame, cref: &'static ast::ClassRef, site: &MethodSite) -> R<Option<MethodCache>> {
    let ptr_of = |c: Option<&Rc<crate::class::Class>>| c.map_or(ptr::null(), Rc::as_ptr);
    let scope = ptr_of(f.class.as_ref());
    let stc = ptr_of(f.static_class.as_ref());
    if let Some(mc) = site.cached.get() {
        if mc.scope == scope && mc.static_cls == stc {
            return Ok(Some(mc));
        }
    }
    // Resolve (autoloading if needed) here: a class that can't be found is
    // an error before the arguments are evaluated, as in PHP.
    let cls = match cref {
        ast::ClassRef::Name(_) => {
            let mut f2 = Frame::for_call(f.ctx.clone(), f.func.clone(), 0);
            f2.class = f.class.clone();
            f2.static_class = f.static_class.clone();
            f2.this = f.this.clone();
            match it.class_ref(&mut f2, cref) {
                Ok(c) => c,
                Err(e) => return Err(e),
            }
        }
        _ => return Ok(None),
    };
    if cls.kind != ast::ClassKind::Class || cls.is_abstract || cls.is_a("throwable") {
        return Ok(None);
    }
    let mut pending_compile = false;
    let (entry, decl_class, method, direct) = match &cls.ctor {
        Some(ctor) => {
            if !it.member_visible(f.class.as_deref(), ctor.vis, &ctor.declaring) {
                return Ok(None);
            }
            let plain = crate::call::method_params(ctor).is_some_and(|ps| ps.iter().all(|p| !p.by_ref && !p.variadic));
            match method_entry(it, ctor) {
                Some((entry, decl_class)) => (Some(entry), decl_class, Rc::as_ptr(ctor), plain),
                // Not compiled (yet): the object is still created here; the
                // constructor runs interpreted until it compiles.
                None => {
                    pending_compile = true;
                    (None, None, Rc::as_ptr(ctor), false)
                }
            }
        }
        None if cls.plain_new => (None, None, ptr::null(), false),
        None => return Ok(None),
    };
    let mc = MethodCache {
        class: Rc::as_ptr(&cls),
        scope,
        // No constructor: a null entry.
        entry: entry.unwrap_or(no_ctor),
        method,
        decl_class: decl_class.as_ref().map_or(ptr::null(), Rc::as_ptr),
        sc: Rc::as_ptr(&cls),
        static_cls: stc,
        // (Unused by `new` sites otherwise.)
        this_cls: if pending_compile { CTOR_NOT_COMPILED } else { ptr::null() },
    };
    site.cached.set(Some(mc));
    site.frame.invalidate();
    // Compiled code may call the constructor directly.
    site.direct.set(match entry {
        Some(e) if direct => e as usize,
        _ => 0,
    });
    Ok(Some(mc))
}

/// A new instance of the cached class (no constructor run).
unsafe fn new_object(it: &Interp, mc: &MethodCache) -> R<Rc<crate::value::Obj>> {
    // Classes live for the whole run: borrow the cached pointer as an Rc
    // without touching its count.
    let cls = std::mem::ManuallyDrop::new(Rc::from_raw(mc.class));
    it.new_instance(&cls)
}

/// Run the cached constructor on `o` (its result goes to `out`); an
/// interpreted run when the compiled one declines.
#[allow(clippy::too_many_arguments)]
unsafe fn run_ctor(it: &Interp, site: &MethodSite, mc: &MethodCache, o: &Rc<crate::value::Obj>, args: *mut Value, n: u64, locals: *const *mut Value, out: *mut Value) -> u64 {
    let mut mc = *mc;
    if mc.this_cls == CTOR_NOT_COMPILED {
        // The constructor wasn't compiled when the site was cached: maybe it
        // is now (it compiles after a few calls).
        let m = std::mem::ManuallyDrop::new(Rc::from_raw(mc.method));
        if let Some((entry, decl_class)) = method_entry(it, &m) {
            mc.entry = entry;
            mc.this_cls = ptr::null();
            mc.decl_class = decl_class.as_ref().map_or(ptr::null(), Rc::as_ptr);
            site.cached.set(Some(mc));
            site.frame.invalidate();
            let plain = crate::call::method_params(&m).is_some_and(|ps| ps.iter().all(|p| !p.by_ref && !p.variadic));
            site.direct.set(if plain { entry as usize } else { 0 });
        }
    }
    let mc = &mc;
    let m = &*mc.method;
    let crate::class::MethodBody::User(func) = &m.body else {
        // A built-in class's native constructor (DateTime, ...).
        let pos = take_args(args, n);
        let cls = std::mem::ManuallyDrop::new(Rc::from_raw(mc.class));
        let ctor = cls.ctor.clone().unwrap();
        let r = it.invoke_method(Some(o.clone()), &ctor, (*cls).clone(), pos, vec![]);
        return status(it, r, |v| put(out, v));
    };
    let make = || {
        let mut frame = Frame::for_call(m.ctx.clone(), m.name.clone(), *func as *const ast::Function as usize);
        frame.static_class = rc_from(mc.class);
        frame.class = rc_from(mc.decl_class);
        frame
    };
    let r = site.frame.with(make, |frame| {
        frame.this = Some(o.clone());
        enter(it, mc.entry, frame, args, n, out)
    });
    match r {
        Some(st) => st,
        None => {
            // Declined before binding (arguments untouched): run the
            // constructor through the interpreter on this object.
            let pos = take_args(args, n);
            let cls = std::mem::ManuallyDrop::new(Rc::from_raw(mc.class));
            let ctor = cls.ctor.clone().unwrap();
            let params = crate::call::method_params(&ctor);
            let r = if locals.is_null() {
                it.invoke_method(Some(o.clone()), &ctor, (*cls).clone(), pos, vec![])
            } else {
                with_refs(params, pos, locals, |pos| it.invoke_method(Some(o.clone()), &ctor, (*cls).clone(), pos, vec![]))
            };
            status(it, r, |v| put(out, v))
        }
    }
}

/// `new C(...)` in compiled code, first step: the object, before the
/// arguments are evaluated (PHP's order). 0: created in `dst`, the
/// constructor comes next (`rt_new_ctor` or a direct call); 3: created, no
/// constructor; 2: not handled here (nothing done; use `rt_new`).
pub unsafe extern "C" fn rt_new_object(c: *mut Cx, cref: *const ast::ClassRef, site: *const MethodSite, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let mc = match new_cache(it, f, &*cref, &*site) {
        Ok(Some(mc)) => mc,
        Ok(None) => return 2,
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            return 1;
        }
    };
    match new_object(it, &mc) {
        Ok(o) => {
            put(dst, Value::Object(o));
            if mc.method.is_null() { 3 } else { 0 }
        }
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            1
        }
    }
}

/// Second step: run the constructor on the object at `obj` (the cache was
/// just validated by `rt_new_object`). The constructor's result goes to
/// `dst`.
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn rt_new_ctor(c: *mut Cx, obj: *const Value, site: *const MethodSite, args: *mut Value, n: u64, locals: *const *mut Value, meta: *const ast::Arg, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let site = &*site;
    let (Some(mc), Value::Object(o)) = (site.cached.get(), &*obj) else { unreachable!("rt_new_ctor without rt_new_object") };
    if !meta.is_null() {
        let r = expand_args(it, take_args(args, n), meta).and_then(|(pos, named)| {
            let cls = std::mem::ManuallyDrop::new(Rc::from_raw(mc.class));
            it.invoke_method(Some(o.clone()), cls.ctor.as_ref().unwrap(), (*cls).clone(), pos, named)
        });
        return status(it, r, |v| put(dst, v));
    }
    run_ctor(it, site, &mc, o, args, n, locals, dst)
}

/// A `new` site's cache: the constructor had no compiled entry yet.
const CTOR_NOT_COMPILED: *const crate::class::Class = 1 as *const _;

/// Placeholder entry for classes without a constructor (never called).
unsafe extern "C" fn no_ctor(_: *mut Cx, _: *mut Value, _: u64, _: *mut Value) -> u64 {
    2
}

/// kind 0: closure (`vals` = its `use` list, in order); kind 1: arrow fn
/// (`vals` = values for `arrow_captures` names). `vals` are consumed.
pub unsafe extern "C" fn rt_make_closure(c: *mut Cx, kind: u64, node: *const (), vals: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let vals = take_args(vals, n);
    let v = if kind == 0 {
        let cl = &*(node as *const ast::Closure);
        let captured = cl.uses.iter().zip(vals).map(|(u, v)| (&*u.var.name, v, false)).collect();
        it.make_closure(f, crate::interp::ClosureKind::Closure(cl), captured, cl.is_static)
    } else {
        let a = &*(node as *const ast::ArrowFn);
        let names = it.arrow_captures(a);
        let captured = names.iter().zip(vals).map(|(n, v)| (*n, v, false)).collect();
        it.make_closure(f, crate::interp::ClosureKind::Arrow(a), captured, a.is_static)
    };
    put(dst, v);
    0
}

pub unsafe extern "C" fn rt_clone(c: *mut Cx, v: *const Value, dst: *mut Value) -> u64 {
    let (it, _) = cx(c);
    let r = match &*v {
        Value::Object(o) => it.clone_object(o).map(Value::Object),
        _ => Err(it.error("Error", "__clone method called on non-object")),
    };
    status(it, r, |v| put(dst, v))
}

// ---- arrays & iteration ----------------------------------------------------------

pub unsafe extern "C" fn rt_array_new(dst: *mut Value) {
    put(dst, Value::array(PhpArray::new()));
}

/// Append an item to an array literal. `key` may be null; `val` is consumed.
pub unsafe extern "C" fn rt_array_add(c: *mut Cx, arr: *mut Value, key: *const Value, val: *mut Value, spread: u64) -> u64 {
    let (it, _) = cx(c);
    let v = take(val);
    let Value::Array(a) = &mut *arr else { return 0 };
    let a = Rc::make_mut(a);
    let r = (|| -> R<()> {
        if spread != 0 {
            match v {
                Value::Array(src) => {
                    for (k, v) in src.iter() {
                        match k {
                            Key::Int(_) => {
                                a.push(v);
                            }
                            Key::Str(_) => a.insert(k.clone(), v),
                        }
                    }
                }
                Value::Object(o) if o.class.is_a("traversable") => {
                    for (k, v) in it.iterate(&o)? {
                        match it.to_key(&k)? {
                            Key::Int(_) => {
                                a.push(v);
                            }
                            key => a.insert(key, v),
                        }
                    }
                }
                _ => return Err(it.error("Error", "Only arrays and Traversables can be unpacked")),
            }
            return Ok(());
        }
        if key.is_null() {
            a.push(v);
        } else {
            a.insert(it.to_key(&*key)?, v);
        }
        Ok(())
    })();
    status(it, r, |_| {})
}

pub enum Iter {
    Array { arr: Rc<PhpArray>, i: usize },
    Iterator { obj: Rc<Obj>, started: bool },
    Items { items: Vec<(Value, Value)>, i: usize },
}

/// Start `foreach` over `subject`. Writes a boxed iterator pointer to `out`
/// (null for "nothing to iterate").
pub unsafe extern "C" fn rt_iter_new(c: *mut Cx, subject: *const Value, out: *mut *mut Iter) -> u64 {
    let (it, f) = cx(c);
    if !(*out).is_null() {
        drop(Box::from_raw(*out));
        *out = ptr::null_mut();
    }
    let r: R<Option<Iter>> = (|| {
        Ok(Some(match &*subject {
            Value::Array(a) => Iter::Array { arr: a.clone(), i: 0 },
            Value::Object(o) if matches!(&*o.internal(), Internal::Generator(_)) || o.class.is_a("iterator") => {
                Iter::Iterator { obj: o.clone(), started: false }
            }
            Value::Object(o) if o.class.is_a("iteratoraggregate") => match it.call_method(o, "getIterator", vec![])? {
                Value::Object(inner) => Iter::Iterator { obj: inner, started: false },
                _ => return Err(it.error("TypeError", "getIterator() must return a Traversable")),
            },
            Value::Object(o) => {
                let items = o
                    .prop_list()
                    .into_iter()
                    .filter(|(n, _)| it.prop_visible(f, o, n))
                    .map(|(n, v)| (Value::str(crate::call::plain_prop(&n)), v))
                    .collect();
                Iter::Items { items, i: 0 }
            }
            other => {
                it.warn(&format!("foreach() argument must be of type array|object, {} given", other.type_name()));
                return Ok(None);
            }
        }))
    })();
    status(it, r, |i| {
        if let Some(i) = i {
            *out = Box::into_raw(Box::new(i));
        }
    })
}

/// Next item: 1 = item written (key to `key` if non-null, value to `val`),
/// 0 = done, 2 = error.
pub unsafe extern "C" fn rt_iter_next(c: *mut Cx, iter: *mut Iter, key: *mut Value, val: *mut Value) -> u64 {
    let (it, _) = cx(c);
    if iter.is_null() {
        return 0;
    }
    let r: R<Option<(Value, Value)>> = (|| {
        Ok(match &mut *iter {
            Iter::Array { arr, i } => {
                let item = arr.get_index(*i).map(|(k, v)| (k.to_value(), v));
                *i += 1;
                item
            }
            Iter::Items { items, i } => {
                let item = items.get(*i).cloned();
                *i += 1;
                item
            }
            Iter::Iterator { obj, started } => {
                if *started {
                    it.call_method(obj, "next", vec![])?;
                } else {
                    it.call_method(obj, "rewind", vec![])?;
                    *started = true;
                }
                if !it.call_method(obj, "valid", vec![])?.truthy() {
                    None
                } else {
                    let v = it.call_method(obj, "current", vec![])?;
                    let k = if key.is_null() { Value::Null } else { it.call_method(obj, "key", vec![])? };
                    Some((k, v))
                }
            }
        })
    })();
    match r {
        Ok(Some((k, v))) => {
            if !key.is_null() {
                put(key, k);
            }
            put(val, v);
            1
        }
        Ok(None) => 0,
        Err(e) => {
            *it.pending.borrow_mut() = Some(e);
            2
        }
    }
}

pub unsafe extern "C" fn rt_iter_free(slot: *mut *mut Iter) {
    if !(*slot).is_null() {
        drop(Box::from_raw(*slot));
        *slot = ptr::null_mut();
    }
}

// ---- control ----------------------------------------------------------------------

pub unsafe extern "C" fn rt_throw(c: *mut Cx, v: *const Value) -> u64 {
    let (it, _) = cx(c);
    let e = match &*v {
        Value::Object(o) if o.class.is_a("throwable") => Ctrl::Throw(o.clone()),
        _ => it.error("Error", "Can only throw objects"),
    };
    *it.pending.borrow_mut() = Some(e);
    1
}

/// `catch (T $e)`: 1 = the pending exception matched (taken, stored in
/// `dst` when non-null), 0 = no match (still pending).
pub unsafe extern "C" fn rt_catch(c: *mut Cx, catch: *const ast::Catch, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let exc = match &*it.pending.borrow() {
        Some(Ctrl::Throw(o)) => o.clone(),
        _ => return 0,
    };
    for t in &(*catch).types {
        let name = it.resolve_class_name(&f.ctx, t);
        if exc.class.is_a(&it.lower.get(&name)) {
            *it.pending.borrow_mut() = None;
            if !dst.is_null() {
                *dst = Value::Object(exc);
            }
            return 1;
        }
    }
    0
}

/// `@expr`: raise the silence level; returns the old one for
/// rt_silence_end (on both the normal and the exception path).
pub unsafe extern "C" fn rt_silence_begin(c: *mut Cx) -> u64 {
    let (it, _) = cx(c);
    let old = it.silence.get();
    it.silence.set(old + 1);
    old as u64
}

pub unsafe extern "C" fn rt_silence_end(c: *mut Cx, old: u64) {
    let (it, _) = cx(c);
    it.silence.set(old as u32);
}

/// Move the pending exception into `slot` before a finally runs. 1 = it
/// was exit() (left pending: finally is skipped).
pub unsafe extern "C" fn rt_pending_save(c: *mut Cx, slot: *mut *mut Ctrl) -> u64 {
    let (it, _) = cx(c);
    let mut p = it.pending.borrow_mut();
    if matches!(*p, Some(Ctrl::Exit(_)) | None) {
        return 1;
    }
    rt_stash_free(slot);
    *slot = Box::into_raw(Box::new(p.take().unwrap()));
    0
}

pub unsafe extern "C" fn rt_pending_restore(c: *mut Cx, slot: *mut *mut Ctrl) {
    let (it, _) = cx(c);
    if !(*slot).is_null() {
        *it.pending.borrow_mut() = Some(*Box::from_raw(*slot));
        *slot = ptr::null_mut();
    }
}

pub unsafe extern "C" fn rt_stash_free(slot: *mut *mut Ctrl) {
    if !(*slot).is_null() {
        drop(Box::from_raw(*slot));
        *slot = ptr::null_mut();
    }
}

pub unsafe extern "C" fn rt_destruct_now(c: *mut Cx) -> u64 {
    let (it, _) = cx(c);
    status(it, it.destruct_now(), |_| ())
}

pub unsafe extern "C" fn rt_destruct(c: *mut Cx) -> u64 {
    let (it, _) = cx(c);
    status(it, it.run_destructors(), |_| ())
}

pub unsafe extern "C" fn rt_exit(c: *mut Cx, v: *const Value) -> u64 {
    let (it, _) = cx(c);
    let code = if v.is_null() {
        0
    } else {
        match &*v {
            Value::Int(n) => *n as i32,
            other => {
                match it.to_str(other) {
                    Ok(b) => it.echo(&b),
                    Err(e) => {
                        *it.pending.borrow_mut() = Some(e);
                        return 1;
                    }
                }
                0
            }
        }
    };
    *it.pending.borrow_mut() = Some(Ctrl::Exit(code));
    1
}

pub unsafe extern "C" fn rt_unhandled_match(c: *mut Cx, v: *const Value) -> u64 {
    let (it, _) = cx(c);
    let v = &*v;
    let repr = match v {
        Value::Str(_) | Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null => {
            let mut o = crate::value::Out(vec![]);
            crate::value::var_export(v, 0, &mut o);
            String::from_utf8_lossy(&o.0).into_owned()
        }
        other => format!("of type {}", other.debug_type()),
    };
    *it.pending.borrow_mut() = Some(it.error("UnhandledMatchError", &format!("Unhandled match case {repr}")));
    1
}

impl SiteFrame {
    pub const FRAME: usize = std::mem::offset_of!(SiteFrame, frame);
    pub const BUSY: usize = std::mem::offset_of!(SiteFrame, busy);
}
