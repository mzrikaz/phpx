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
    match r {
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
        if p.by_ref || p.variadic || p.promote.is_some() || (i >= n && p.default.is_none()) {
            return 2;
        }
    }
    let strict = it.file_strict(it.pos.get().file as usize);
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
            Some(ty) if !(v.is_null() && matches!(p.default.as_ref().map(|d| &d.kind), Some(ast::ExprKind::Null))) => {
                match it.coerce(v.clone(), ty, &spec.ctx, spec.class.as_ref(), f.static_class.as_ref(), strict) {
                    Ok(Some(v)) => v,
                    Ok(None) => {
                        *it.pending.borrow_mut() = Some(it.error(
                            "TypeError",
                            &format!(
                                "{}(): Argument #{} (${}) must be of type {}, {} given",
                                spec.display,
                                i + 1,
                                p.name.name,
                                crate::call::type_str(ty),
                                v.debug_type()
                            ),
                        ));
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
        ptr::drop_in_place(locals.add(i));
        put(locals.add(i), v);
    }
    // Extra arguments are ignored (func_get_args() isn't compiled).
    for i in spec.params.len()..n {
        drop(take(args.add(i)));
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
            Value::Str(s) => a.get(&Key::from_bytes(s)),
            _ => None,
        };
        if let Some(v) = found {
            put(dst, v);
            return 0;
        }
    }
    status(it, it.read_index(&*base, &*key), |v| put(dst, v))
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
    }
    if o.class.find_method("__isset").is_some() {
        let r = it.call_method(o, "__isset", vec![Value::str(name)]).and_then(|set| {
            if set.truthy() { it.call_method(o, "__get", vec![Value::str(name)]).map(Some) } else { Ok(None) }
        });
        return match r {
            Ok(Some(v)) => {
                put(dst, v);
                1
            }
            Ok(None) => 0,
            Err(e) => {
                *it.pending.borrow_mut() = Some(e);
                2
            }
        };
    }
    0
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
unsafe fn keys_of(it: &Interp, keys: *const Value, n: u64, append: u64) -> R<Vec<Option<Key>>> {
    let mut path = Vec::with_capacity(n as usize + 1);
    for i in 0..n as usize {
        path.push(Some(it.to_key(&*keys.add(i))?));
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
            let Value::Object(o) = &*root else {
                return Err(it.error("Error", &format!("Attempt to assign property \"{}\" on {}", name_of(name, len), (*root).type_name())));
            };
            let place = crate::expr::Place { root: crate::expr::Root::Prop(o.clone(), it.prop_key_rc(f, o, Rc::from(name_of(name, len)))), path };
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

fn place_for(it: &Interp, f: &Frame, root_obj: Option<Rc<Obj>>, name: &'static str, path: Vec<Option<Key>>) -> crate::expr::Place {
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
        let obj = if name.is_null() {
            None
        } else {
            match &*root {
                Value::Object(o) => Some(o.clone()),
                other => return Err(it.error("Error", &format!("Attempt to assign property \"{}\" on {}", name_of(name, len), other.type_name()))),
            }
        };
        let read = |it: &Interp, f: &mut Frame| -> R<Option<Value>> {
            match &obj {
                None => {
                    let mut cur = (*root).clone();
                    for k in &path {
                        cur = match &cur {
                            Value::Array(a) => match a.get(k.as_ref().unwrap()) {
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
                Some(o) => it.place_get(f, &place_for(it, f, Some(o.clone()), name_of(name, len), path.clone())),
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
            Some(o) => it.place_set(f, &place_for(it, f, Some(o.clone()), name_of(name, len), path), new)?,
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
                } else if o.class.find_method("__unset").is_some() {
                    it.call_method(o, "__unset", vec![Value::str(pname)])?;
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

fn remove_path(it: &Interp, cur: &mut Value, path: &[Option<Key>]) -> R<()> {
    let Some((last, rest)) = path.split_last() else { return Ok(()) };
    let mut c = cur;
    for k in rest {
        match c {
            Value::Array(a) => match Rc::make_mut(a).get_mut(k.as_ref().unwrap()) {
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
            Rc::make_mut(a).remove(last.as_ref().unwrap());
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

unsafe fn with_refs(
    params: Option<&'static [ast::Param]>,
    mut pos: Vec<Value>,
    locals: *const *mut Value,
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
pub struct CallSite {
    pub name: &'static str,
    /// (entry, function). Plain pointer: functions are never freed.
    pub cached: std::cell::Cell<Option<(crate::compiler::Entry, *const crate::interp::UserFn)>>,
    /// Resolved plain built-in (no by-ref params, no caller-scope access).
    pub builtin: std::cell::Cell<Option<crate::interp::BuiltinFn>>,
    /// A frame reused by calls from this site (rebuilding one per call is
    /// a large part of call overhead); recursion falls back to a fresh one.
    pub frame: SiteFrame,
}

/// A reusable callee frame, lent out while not busy.
#[derive(Default)]
pub struct SiteFrame {
    frame: std::cell::UnsafeCell<Option<Box<Frame>>>,
    busy: std::cell::Cell<bool>,
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
        let slot = &mut *self.frame.get();
        let f = slot.get_or_insert_with(|| Box::new(make()));
        let r = g(f);
        f.reset_after_call();
        self.busy.set(false);
        r
    }

    /// Forget the frame (the cached callee changed).
    fn invalidate(&self) {
        if !self.busy.get() {
            unsafe { *self.frame.get() = None };
        }
    }
}

/// A `$obj->m(...)` call site with a monomorphic inline cache.
pub struct MethodSite {
    pub name: &'static str,
    pub lname: &'static str,
    pub cached: std::cell::Cell<Option<MethodCache>>,
    pub frame: SiteFrame,
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
    /// Leaked once per site: only used in error messages.
    display: *const str,
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
            let (pos, named) = expand_args(it, take_args(args, n), meta)?;
            it.call_callable(&callable, pos, named)
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
        let r = crate::builtins::guarded(it, "built-in", || fun(it, f, vals));
        for v in vals.iter_mut() {
            if !matches!(v, Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_)) {
                drop(std::mem::take(v));
            }
        }
        return status(it, r, |v| put(dst, v));
    }
    if let Some((entry, u)) = (*site).cached.get() {
        let u = &*u;
        let make = || Frame::for_call(u.ctx.clone(), u.name.clone(), u.decl as *const ast::Function as usize);
        if let Some(st) = (*site).frame.with(make, |frame| enter(it, entry, frame, args, n, dst, u.decl.ret.as_ref(), None, &u.name)) {
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
            if crate::call::BY_REF.iter().any(|(nm, _)| nm == bname) {
                let mut vals = pos;
                let idxs: Vec<usize> = crate::call::BY_REF.iter().filter(|(nm, _)| nm == bname).map(|(_, i)| *i).filter(|i| *i < vals.len()).collect();
                for &idx in &idxs {
                    let local = *locals.add(idx);
                    if !local.is_null() {
                        vals[idx] = take(local);
                    }
                }
                let r = crate::builtins::guarded(it, bname, || fun(it, f, &mut vals));
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
            if !matches!(*bname, "get_class" | "get_parent_class") {
                (*site).builtin.set(Some(*fun));
            }
            let mut vals: Vec<Value> = pos.into_iter().map(Value::into_deref).collect();
            return crate::builtins::guarded(it, bname, || fun(it, f, &mut vals));
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
    }
    let mut frame = Frame::for_call(u.ctx.clone(), u.name.clone(), fn_id);
    enter(it, entry, &mut frame, args, n, dst, u.decl.ret.as_ref(), None, &u.name)
}

/// Run a native entry with a fresh frame; applies the declared return type.
#[allow(clippy::too_many_arguments)]
unsafe fn enter(
    it: &Interp,
    entry: crate::compiler::Entry,
    frame: &mut Frame,
    args: *mut Value,
    n: u64,
    dst: *mut Value,
    ret: Option<&ast::Type>,
    class: Option<&Rc<crate::class::Class>>,
    display: &str,
) -> Option<u64> {
    if phpx_runtime::stack_remaining().is_some_and(|r| r < 256 * 1024) {
        return None; // the general path raises the proper error
    }
    let saved = it.pos.get();
    let mut cx2 = Cx { it, frame };
    let st = entry(&mut cx2, args, n, dst);
    it.pos.set(saved);
    match (st, ret) {
        (2, _) => None,
        (0, Some(ty)) => {
            let v = take(dst);
            match it.check_return_type(ty, v, &frame.ctx, class, frame.static_class.as_ref(), display) {
                Ok(v) => {
                    put(dst, v);
                    Some(0)
                }
                Err(e) => {
                    *it.pending.borrow_mut() = Some(e);
                    Some(1)
                }
            }
        }
        (st, _) => Some(st),
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
        let hit = hit.or_else(|| {
            // Miss: look the method up and (if it compiles) cache it.
            let m = o.class.find_method(site.lname)?.clone();
            let crate::class::MethodBody::User(func) = &m.body else { return None };
            if func.body.is_none() || !it.member_visible(f.class.as_deref(), m.vis, &m.declaring) {
                return None;
            }
            let fn_id = *func as *const ast::Function as usize;
            let decl_class = it.classes.borrow().get(&*m.declaring).cloned();
            let display: Rc<str> = Rc::from(format!("{}::{}", decl_class.as_ref().map(|c| &*c.name).unwrap_or(""), m.name));
            let entry = it.native_entry(fn_id, || crate::compiler::Target {
                fn_id,
                display: display.clone(),
                params: &func.params,
                captured: vec![],
                body: crate::compiler::Body::Block(func.body.as_deref().unwrap()),
                ctx: m.ctx.clone(),
                class: decl_class.clone(),
            })?;
            let mc = MethodCache {
                class: Rc::as_ptr(&o.class),
                scope,
                entry,
                method: Rc::as_ptr(&m),
                decl_class: decl_class.as_ref().map_or(ptr::null(), Rc::as_ptr),
                display: Box::leak(display.to_string().into_boxed_str()),
            };
            site.cached.set(Some(mc));
            site.frame.invalidate();
            Some(mc)
        });
        if let Some(mc) = hit {
            let m = &*mc.method;
            let crate::class::MethodBody::User(func) = &m.body else { unreachable!() };
            let make = || {
                let decl_class = if mc.decl_class.is_null() {
                    None
                } else {
                    // Re-materialize the Rc without changing the count net.
                    Rc::increment_strong_count(mc.decl_class);
                    Some(Rc::from_raw(mc.decl_class))
                };
                let mut frame = Frame::for_call(m.ctx.clone(), m.name.clone(), *func as *const ast::Function as usize);
                frame.static_class = Some(o.class.clone());
                frame.class = decl_class;
                frame
            };
            let r = site.frame.with(make, |frame| {
                if !m.is_static {
                    frame.this = Some(o.clone());
                }
                // Only needed to check a declared return type.
                let ret_class = if func.ret.is_some() { frame.class.clone() } else { None };
                enter(it, mc.entry, frame, args, n, dst, func.ret.as_ref(), ret_class.as_ref(), &*mc.display)
            });
            if let Some(st) = r {
                return st;
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
                let params = o.class.find_method(&name.to_ascii_lowercase()).and_then(|m| crate::call::method_params(m));
                with_refs(params, vals, locals, |pos| it.method_call(f, &o, name, pos, vec![]))
            } else {
                expand_args(it, vals, meta).and_then(|(pos, named)| it.method_call(f, &o, name, pos, named))
            }
        }
        other => Err(it.error("Error", &format!("Call to a member function {name}() on {}", other.type_name()))),
    };
    status(it, r, |v| put(dst, v))
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
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
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
    name: *const u8,
    len: u64,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    let name = name_of(name, len);
    let pos = take_args(args, n);
    let r = it.class_ref(f, &*cref).and_then(|cls| {
        if !meta.is_null() {
            let (pos, named) = expand_args(it, pos, meta)?;
            return it.static_call(f, &*cref, cls.clone(), name, pos, named);
        }
        let params = cls.find_method(&name.to_ascii_lowercase()).and_then(|m| crate::call::method_params(m));
        with_refs(params, pos, locals, |pos| it.static_call(f, &*cref, cls.clone(), name, pos, vec![]))
    });
    status(it, r, |v| put(dst, v))
}

pub unsafe extern "C" fn rt_new(
    c: *mut Cx,
    cref: *const ast::ClassRef,
    args: *mut Value,
    n: u64,
    locals: *const *mut Value,
    meta: *const ast::Arg,
    dst: *mut Value,
) -> u64 {
    let (it, f) = cx(c);
    let pos = take_args(args, n);
    let r = it.class_ref(f, &*cref).and_then(|cls| {
        // `new C` of a plain class without a constructor: no call machinery.
        if cls.plain_new && cls.ctor.is_none() && pos.is_empty() {
            return Ok(Value::Object(it.new_instance(&cls)?));
        }
        if !meta.is_null() {
            let (pos, named) = expand_args(it, pos, meta)?;
            return it.instantiate(f, &cls, pos, named).map(Value::Object);
        }
        let params = cls.find_method("__construct").and_then(|m| crate::call::method_params(m));
        with_refs(params, pos, locals, |pos| it.instantiate(f, &cls, pos, vec![]).map(Value::Object))
    });
    status(it, r, |v| put(dst, v))
}

/// kind 0: closure (`vals` = its `use` list, in order); kind 1: arrow fn
/// (`vals` = values for `arrow_captures` names). `vals` are consumed.
pub unsafe extern "C" fn rt_make_closure(c: *mut Cx, kind: u64, node: *const (), vals: *mut Value, n: u64, dst: *mut Value) -> u64 {
    let (it, f) = cx(c);
    let vals = take_args(vals, n);
    let v = if kind == 0 {
        let cl = &*(node as *const ast::Closure);
        let captured = cl.uses.iter().zip(vals).map(|(u, v)| (&*u.var.name, slot(v), false)).collect();
        it.make_closure(f, crate::interp::ClosureKind::Closure(cl), captured, cl.is_static)
    } else {
        let a = &*(node as *const ast::ArrowFn);
        let names = it.arrow_captures(a);
        let captured = names.iter().zip(vals).map(|(n, v)| (*n, slot(v), false)).collect();
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
            Value::Object(o) if matches!(&*o.internal.borrow(), Internal::Generator(_)) || o.class.is_a("iterator") => {
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
