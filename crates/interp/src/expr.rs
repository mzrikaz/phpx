//! Expression evaluation and assignment targets ("places").
//!
//! Writes like `$a['x'][] = 1` resolve to a root plus pre-evaluated keys, so
//! no user code (offsetSet, __set, __toString) ever runs while a RefCell is
//! borrowed.

use std::rc::Rc;

use phpx_syntax::ast::{self, BinOp, Expr, ExprKind, InterpPart, MemberName, UnOp};

use crate::class::{StaticProp, Vis};
use crate::interp::{slot, ClosureData, ClosureKind, Ctrl, Frame, Interp, Slot, R};
use crate::value::{identical, Internal, Key, Obj, PhpArray, Value};

#[derive(Clone)]
pub enum Root {
    Var(Slot),
    Prop(Rc<Obj>, Rc<str>),
    Static(Rc<StaticProp>),
}

#[derive(Clone)]
pub struct Place {
    pub root: Root,
    /// `None` = append (`$a[]`).
    pub path: Vec<Option<Key>>,
}

/// A write that reached an object (ArrayAccess) and must finish outside any borrow.
pub(crate) struct Pending {
    obj: Rc<Obj>,
    path: Vec<Option<Key>>,
    value: Value,
}

/// Int-int operators with PHP results, when no overflow/special case applies.
/// None = let the general path handle it.
#[inline]
fn int_binop(op: BinOp, x: i64, y: i64) -> Option<Value> {
    Some(match op {
        BinOp::Add => Value::Int(x.checked_add(y)?),
        BinOp::Sub => Value::Int(x.checked_sub(y)?),
        BinOp::Mul => Value::Int(x.checked_mul(y)?),
        BinOp::Lt => Value::Bool(x < y),
        BinOp::LtEq => Value::Bool(x <= y),
        BinOp::Gt => Value::Bool(x > y),
        BinOp::GtEq => Value::Bool(x >= y),
        BinOp::Eq | BinOp::Identical => Value::Bool(x == y),
        BinOp::NotEq | BinOp::NotIdentical => Value::Bool(x != y),
        BinOp::Mod if y != 0 && y != -1 => Value::Int(x % y),
        BinOp::BitAnd => Value::Int(x & y),
        BinOp::BitOr => Value::Int(x | y),
        BinOp::BitXor => Value::Int(x ^ y),
        _ => return None,
    })
}

fn is_chain(e: &Expr) -> bool {
    matches!(e.kind, ExprKind::Prop { .. } | ExprKind::MethodCall { .. } | ExprKind::Index { .. } | ExprKind::Call { .. })
}

impl Interp {
    /// Hot cases only. This frame stays on the native stack through nested
    /// PHP calls, so the rest lives in `eval_rest` (only on the stack while
    /// it runs).
    pub fn eval(&self, f: &mut Frame, e: &'static Expr) -> R<Value> {
        match &e.kind {
            ExprKind::Int(n) => Ok(Value::Int(*n)),
            ExprKind::Float(x) => Ok(Value::Float(*x)),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Null => Ok(Value::Null),
            ExprKind::Var(n) => match f.vars.get_hinted(n, &e.hint) {
                Some(s) => Ok(s.borrow().clone()),
                None => self.read_var(f, n),
            },
            ExprKind::Binary { op, left, right } if !matches!(op, BinOp::And | BinOp::Or | BinOp::Coalesce) => {
                let a = self.eval(f, left)?;
                let b = self.eval(f, right)?;
                if let (Value::Int(x), Value::Int(y)) = (&a, &b) {
                    if let Some(v) = int_binop(*op, *x, *y) {
                        return Ok(v);
                    }
                }
                self.binop(*op, &a, &b)
            }
            // `$i++` / `$x = expr` / `$x += expr` on locals: no place machinery.
            ExprKind::IncDec { prefix, inc, expr } if matches!(expr.kind, ExprKind::Var(_)) => {
                let ExprKind::Var(n) = &expr.kind else { unreachable!() };
                if let Some(s) = f.vars.get_hinted(n, &expr.hint) {
                    let mut cell = s.borrow_mut();
                    if let Value::Int(old) = *cell {
                        if let Some(new) = if *inc { old.checked_add(1) } else { old.checked_sub(1) } {
                            *cell = Value::Int(new);
                            return Ok(Value::Int(if *prefix { new } else { old }));
                        }
                    }
                }
                self.eval_rest(f, e)
            }
            ExprKind::Assign { target, value } if matches!(&target.kind, ExprKind::Var(n) if n != "this") => {
                let ExprKind::Var(n) = &target.kind else { unreachable!() };
                let v = self.eval(f, value)?;
                match f.vars.get_hinted(n, &target.hint) {
                    Some(s) => *s.borrow_mut() = v.clone(),
                    None => {
                        f.vars.insert(n, slot(v.clone()));
                    }
                }
                Ok(v)
            }
            ExprKind::AssignOp { op, target, value }
                if matches!(target.kind, ExprKind::Var(_)) && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul) =>
            {
                let ExprKind::Var(n) = &target.kind else { unreachable!() };
                let rhs = self.eval(f, value)?;
                if let (Some(s), Value::Int(y)) = (f.vars.get_hinted(n, &target.hint), &rhs) {
                    let mut cell = s.borrow_mut();
                    if let Value::Int(x) = *cell {
                        if let Some(v) = int_binop(*op, x, *y) {
                            *cell = v.clone();
                            return Ok(v);
                        }
                    }
                }
                self.assign_op_slow(f, *op, target, rhs)
            }
            ExprKind::Ternary { cond, then: Some(t), else_ } => {
                if self.eval(f, cond)?.truthy() { self.eval(f, t) } else { self.eval(f, else_) }
            }
            ExprKind::Prop { .. } | ExprKind::MethodCall { .. } | ExprKind::Index { .. } | ExprKind::Call { .. } => {
                Ok(self.eval_chain(f, e)?.unwrap_or(Value::Null))
            }
            _ => self.eval_rest(f, e),
        }
    }

    /// `$x op= rhs` once rhs is already evaluated (general path).
    #[inline(never)]
    fn assign_op_slow(&self, f: &mut Frame, op: BinOp, target: &'static Expr, rhs: Value) -> R<Value> {
        let place = self.place(f, target)?;
        let cur = match self.place_get(f, &place)? {
            Some(v) => v,
            None => {
                self.warn_undefined_place(target);
                Value::Null
            }
        };
        let v = self.binop(op, &cur, &rhs)?;
        self.place_set(f, &place, v.clone())?;
        Ok(v)
    }

    #[inline(never)]
    fn eval_rest(&self, f: &mut Frame, e: &'static Expr) -> R<Value> {
        use ExprKind::*;
        Ok(match &e.kind {
            Int(n) => Value::Int(*n),
            Float(x) => Value::Float(*x),
            Str(s) => Value::str(s),
            Bytes(b) => Value::bytes(b.clone()),
            Bool(b) => Value::Bool(*b),
            Null => Value::Null,
            Interp(parts) => {
                let mut buf = vec![];
                for p in parts {
                    match p {
                        InterpPart::Lit(s) => buf.extend_from_slice(s),
                        InterpPart::Expr(e) => {
                            let v = self.eval(f, e)?;
                            buf.extend_from_slice(&self.to_str(&v)?);
                        }
                    }
                }
                Value::bytes(buf)
            }
            Var(n) => self.read_var(f, n)?,
            Name(n) => self.constant(f, n)?,
            MagicConst(m) => self.magic_const(f, m, e),
            Array(items) => match self.const_array_cached(e, items) {
                Some(v) => v,
                None => self.eval_array(f, items)?,
            },
            Assign { target, value } => {
                if let Array(items) = &target.kind {
                    let v = self.eval(f, value)?;
                    self.destructure(f, items, v.clone())?;
                    return Ok(v);
                }
                if let Var(n) = &target.kind {
                    if &**n != "this" {
                        let v = self.eval(f, value)?;
                        *f.var_slot(n).borrow_mut() = v.clone();
                        return Ok(v);
                    }
                }
                let place = self.place(f, target)?;
                let v = self.eval(f, value)?;
                self.place_set(f, &place, v.clone())?;
                v
            }
            AssignRef { target, value } => {
                let s = self.make_ref(f, value)?;
                let v = s.borrow().clone();
                self.bind_ref(f, target, s)?;
                v
            }
            AssignOp { op: BinOp::Concat, target, value } if self.concat_in_place_target(f, target) => {
                let rhs = self.eval(f, value)?;
                let rhs = self.to_str(&rhs)?;
                self.concat_in_place(f, target, &rhs)?
            }
            AssignOp { op, target, value } => {
                let place = self.place(f, target)?;
                if *op == BinOp::Coalesce {
                    if let Some(cur) = self.place_get(f, &place)? {
                        if !cur.is_null() {
                            return Ok(cur);
                        }
                    }
                    let v = self.eval(f, value)?;
                    self.place_set(f, &place, v.clone())?;
                    return Ok(v);
                }
                let cur = match self.place_get(f, &place)? {
                    Some(v) => v,
                    None => {
                        self.warn_undefined_place(target);
                        Value::Null
                    }
                };
                let rhs = self.eval(f, value)?;
                let v = self.binop(*op, &cur, &rhs)?;
                self.place_set(f, &place, v.clone())?;
                v
            }
            Binary { op: BinOp::And, left, right } => Value::Bool(self.eval(f, left)?.truthy() && self.eval(f, right)?.truthy()),
            Binary { op: BinOp::Or, left, right } => Value::Bool(self.eval(f, left)?.truthy() || self.eval(f, right)?.truthy()),
            Binary { op: BinOp::Coalesce, left, right } => match self.eval_quiet(f, left)? {
                Some(v) if !v.is_null() => v,
                _ => self.eval(f, right)?,
            },
            Binary { op, left, right } => {
                let a = self.eval(f, left)?;
                let b = self.eval(f, right)?;
                self.binop(*op, &a, &b)?
            }
            Unary { op: UnOp::Silence, expr } => {
                self.silence.set(self.silence.get() + 1);
                let r = self.eval(f, expr);
                self.silence.set(self.silence.get() - 1);
                r?
            }
            VarVar(n) => {
                let name = self.var_name(f, n)?;
                self.read_var(f, name)?
            }
            GlobalVar(n) => {
                let name = self.var_name(f, n)?;
                match self.with_globals(f, |v| v.get(name).map(|s| s.borrow().clone())) {
                    Some(v) => v,
                    None => {
                        self.warn(&format!("Undefined global variable ${name}"));
                        Value::Null
                    }
                }
            }
            Unary { op, expr } => {
                let v = self.eval(f, expr)?;
                match op {
                    UnOp::Not => Value::Bool(!v.truthy()),
                    UnOp::Neg => self.binop(BinOp::Mul, &v, &Value::Int(-1))?,
                    UnOp::Plus => self.binop(BinOp::Mul, &v, &Value::Int(1))?,
                    UnOp::BitNot => self.bitnot(&v)?,
                    UnOp::Silence => v,
                }
            }
            IncDec { prefix, inc, expr } => {
                let place = self.place(f, expr)?;
                let cur = match self.place_get(f, &place)? {
                    Some(v) => v,
                    None => {
                        self.warn_undefined_place(expr);
                        Value::Null
                    }
                };
                let new = self.incdec(&cur, *inc)?;
                self.place_set(f, &place, new.clone())?;
                if *prefix { new } else { cur }
            }
            Ternary { cond, then, else_ } => {
                let c = self.eval(f, cond)?;
                if c.truthy() {
                    match then {
                        Some(t) => self.eval(f, t)?,
                        None => c,
                    }
                } else {
                    self.eval(f, else_)?
                }
            }
            Prop { .. } | MethodCall { .. } | Index { .. } | Call { .. } => self.eval_chain(f, e)?.unwrap_or(Value::Null),
            CallableRef(inner) => self.callable_ref(f, inner)?,
            StaticProp { class, name } => {
                let sp = self.find_static_prop(f, class, &name.name)?;
                self.static_prop_slot(&sp)?
            }
            StaticCall { class, name, args } => {
                let cls = self.class_ref(f, class)?;
                let name = self.member_name(f, name)?;
                let params = cls.find_method(&name.to_ascii_lowercase()).and_then(|m| crate::call::method_params(m));
                let (pos, named) = self.eval_args_for(f, args, params)?;
                self.static_call(f, class, cls, &name, pos, named)?
            }
            ClassConst { class, name } => self.class_const(f, class, &name.name)?,
            New { class, args } => {
                let cls = match class {
                    ast::ClassRef::Anonymous(c) => self.declare_anon_class(f, c)?,
                    other => self.class_ref(f, other)?,
                };
                let params = cls.find_method("__construct").and_then(|m| crate::call::method_params(m));
                let (pos, named) = self.eval_args_for(f, args, params)?;
                Value::Object(self.instantiate(f, &cls, pos, named)?)
            }
            Clone(x) => {
                let v = self.eval(f, x)?;
                let Value::Object(o) = v else { return Err(self.error("Error", "__clone method called on non-object")) };
                Value::Object(self.clone_object(&o)?)
            }
            Closure(c) => {
                let mut captured = vec![];
                for u in &c.uses {
                    let name: &'static str = &u.var.name;
                    if u.by_ref {
                        captured.push((name, Value::Ref(f.var_slot(name)), true));
                    } else {
                        let v = match f.vars.get(name) {
                            Some(s) => s.borrow().clone(),
                            None => {
                                self.warn(&format!("Undefined variable ${name}"));
                                Value::Null
                            }
                        };
                        captured.push((name, v, false));
                    }
                }
                self.make_closure(f, ClosureKind::Closure(c), captured, c.is_static)
            }
            ArrowFn(a) => {
                let names = self.arrow_captures(a);
                let captured = names
                    .iter()
                    .filter_map(|n| f.vars.get(n).map(|s| (*n, s.borrow().clone(), false)))
                    .collect();
                self.make_closure(f, ClosureKind::Arrow(a), captured, a.is_static)
            }
            Match { subject, arms } => {
                let v = self.eval(f, subject)?;
                let mut default = None;
                for arm in arms {
                    match &arm.conds {
                        None => default = Some(&arm.body),
                        Some(conds) => {
                            for c in conds {
                                if identical(&v, &self.eval(f, c)?) {
                                    return self.eval(f, &arm.body);
                                }
                            }
                        }
                    }
                }
                match default {
                    Some(b) => self.eval(f, b)?,
                    None => {
                        let repr = match &v {
                            Value::Str(s) => {
                                let mut o = crate::value::Out(vec![]);
                                crate::value::var_export(&Value::Str(s.clone()), 0, &mut o);
                                String::from_utf8_lossy(&o.0).into_owned()
                            }
                            Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::Null => {
                                let mut o = crate::value::Out(vec![]);
                                crate::value::var_export(&v, 0, &mut o);
                                String::from_utf8_lossy(&o.0).into_owned()
                            }
                            other => format!("of type {}", other.debug_type()),
                        };
                        return Err(self.error("UnhandledMatchError", &format!("Unhandled match case {repr}")));
                    }
                }
            }
            Instanceof { expr, class } => {
                let v = self.eval(f, expr)?;
                let Value::Object(o) = v else { return Ok(Value::Bool(false)) };
                let lname = match class {
                    ast::ClassRef::Name(n) => match &*n.to_ascii_lowercase() {
                        "self" | "static" | "parent" => self.class_ref(f, class)?.lname.clone(),
                        _ => self.lower.get(&self.resolve_class_name(&f.ctx, n)),
                    },
                    ast::ClassRef::Dynamic(d) => match self.eval(f, d)? {
                        Value::Object(c) => c.class.lname.clone(),
                        Value::Str(s) => self.lower.get(String::from_utf8_lossy(&s).trim_start_matches('\\')),
                        _ => return Err(self.error("Error", "Class name must be a valid object or a string")),
                    },
                    ast::ClassRef::Anonymous(_) => return Ok(Value::Bool(false)),
                };
                Value::Bool(o.class.is_a(&lname))
            }
            Cast { ty, expr } => {
                let v = self.eval(f, expr)?;
                self.cast(ty, v)?
            }
            Isset(es) => {
                for x in es {
                    match self.eval_quiet_mode(f, x, true)? {
                        Some(v) if !v.is_null() => {}
                        _ => return Ok(Value::Bool(false)),
                    }
                }
                Value::Bool(true)
            }
            Empty(x) => Value::Bool(!self.eval_quiet(f, x)?.is_some_and(|v| v.truthy())),
            Throw(x) => {
                let v = self.eval(f, x)?;
                match v {
                    Value::Object(o) if o.class.is_a("throwable") => return Err(Ctrl::Throw(o)),
                    _ => return Err(self.error("Error", "Can only throw objects")),
                }
            }
            Print(x) => {
                let v = self.eval(f, x)?;
                let b = self.to_str(&v)?;
                self.echo(&b);
                Value::Int(1)
            }
            Exit(arg) => {
                let code = match arg {
                    None => 0,
                    Some(a) => match self.eval(f, a)? {
                        Value::Int(n) => n as i32,
                        other => {
                            let b = self.to_str(&other)?;
                            self.echo(&b);
                            0
                        }
                    },
                };
                return Err(Ctrl::Exit(code));
            }
            Include { kind, path } => {
                let p = self.eval(f, path)?;
                let p = self.to_str(&p)?;
                self.include(f, *kind, &String::from_utf8_lossy(&p))?
            }
            Yield { key, value } => {
                let Some(g) = f.gen.clone() else { return Err(self.error("Error", "Cannot yield outside a generator")) };
                let v = match value {
                    Some(v) => self.eval(f, v)?,
                    None => Value::Null,
                };
                let k = match key {
                    Some(k) => {
                        let k = self.eval(f, k)?;
                        if let Value::Int(n) = k {
                            let mut st = g.st.borrow_mut();
                            if n >= st.next_key {
                                st.next_key = n + 1;
                            }
                        }
                        k
                    }
                    None => {
                        let mut st = g.st.borrow_mut();
                        st.next_key += 1;
                        Value::Int(st.next_key - 1)
                    }
                };
                self.gen_yield(&g, k, v)?
            }
            YieldFrom(src) => {
                let Some(g) = f.gen.clone() else { return Err(self.error("Error", "Cannot use \"yield from\" outside a generator")) };
                match self.eval(f, src)? {
                    Value::Array(a) => {
                        for (k, v) in a.iter() {
                            self.gen_yield(&g, k.to_value(), v)?;
                        }
                        Value::Null
                    }
                    Value::Object(o) if matches!(&*o.internal(), Internal::Generator(_)) => {
                        // Delegate: forward values out and sent values in.
                        self.call_method(&o, "rewind", vec![])?;
                        while self.call_method(&o, "valid", vec![])?.truthy() {
                            let k = self.call_method(&o, "key", vec![])?;
                            let v = self.call_method(&o, "current", vec![])?;
                            let sent = self.gen_yield(&g, k, v)?;
                            if sent.is_null() {
                                self.call_method(&o, "next", vec![])?;
                            } else {
                                self.call_method(&o, "send", vec![sent])?;
                            }
                        }
                        self.call_method(&o, "getReturn", vec![])?
                    }
                    Value::Object(o) if o.class.is_a("traversable") => {
                        for (k, v) in self.iterate(&o)? {
                            self.gen_yield(&g, k, v)?;
                        }
                        Value::Null
                    }
                    _ => return Err(self.error("Error", "Can use \"yield from\" only with arrays and Traversables")),
                }
            }
            Error => unreachable!("parser error nodes never reach the interpreter"),
        })
    }

    /// `$s .= x` and `$this->p .= x` append in place (amortized O(1)) when
    /// the string isn't shared, instead of copying the whole string.
    fn concat_in_place_target(&self, f: &Frame, target: &Expr) -> bool {
        match &target.kind {
            ExprKind::Var(n) => &**n != "this",
            ExprKind::Prop { object, name: MemberName::Ident(name), nullsafe: false } => {
                matches!(&object.kind, ExprKind::Var(v) if &**v == "this")
                    && f.this.as_ref().is_some_and(|o| o.class.shadowed.is_empty())
                    && f.this.as_ref().is_some_and(|o| {
                        o.props.borrow().get(&*name.name).is_some_and(|v| !matches!(v, Value::Ref(_)))
                            && o.class.find_prop(&name.name).is_none_or(|d| !d.readonly && d.ty.is_none())
                            && self.prop_visible(f, o, &name.name)
                    })
            }
            _ => false,
        }
    }

    fn concat_in_place(&self, f: &mut Frame, target: &'static Expr, rhs: &[u8]) -> R<Value> {
        let append = |it: &Interp, cur: Value| -> R<Value> {
            Ok(match cur {
                Value::Str(mut s) => {
                    s.push_bytes(rhs);
                    Value::Str(s)
                }
                other => {
                    let mut b = it.to_str(&other)?.to_vec();
                    b.extend_from_slice(rhs);
                    Value::bytes(b)
                }
            })
        };
        match &target.kind {
            ExprKind::Var(n) => {
                if !f.vars.contains_key(&**n) {
                    self.warn(&format!("Undefined variable ${n}"));
                }
                let s = f.var_slot(n);
                let cur = std::mem::take(&mut *s.borrow_mut());
                let v = append(self, cur)?;
                *s.borrow_mut() = v.clone();
                Ok(v)
            }
            ExprKind::Prop { name: MemberName::Ident(name), .. } => {
                let o = f.this.clone().unwrap();
                let cur = o.props.borrow_mut().get_mut(&*name.name).map(std::mem::take).unwrap_or_default();
                let v = append(self, cur)?;
                o.set_prop(&name.name, v.clone());
                Ok(v)
            }
            _ => unreachable!(),
        }
    }

    pub fn read_var(&self, f: &mut Frame, n: &str) -> R<Value> {
        if n == "this" {
            return match &f.this {
                Some(o) => Ok(Value::Object(o.clone())),
                None => Err(self.error("Error", "Using $this when not in object context")),
            };
        }
        match f.vars.get(n) {
            Some(s) => Ok(s.borrow().clone()),
            None if n == "GLOBALS" => Ok(self.globals_array(f)),
            None => {
                self.warn(&format!("Undefined variable ${n}"));
                Ok(Value::Null)
            }
        }
    }

    pub(crate) fn warn_undefined_place(&self, e: &Expr) {
        if let ExprKind::Var(n) = &e.kind {
            self.warn(&format!("Undefined variable ${n}"));
        }
    }

    pub(crate) fn magic_const(&self, f: &Frame, m: &str, e: &Expr) -> Value {
        match m {
            "__LINE__" => Value::Int(self.line_of(f.ctx.file, e.span.start) as i64),
            "__FILE__" => Value::str(&self.file_path(f.ctx.file)),
            "__DIR__" => {
                let p = self.file_path(f.ctx.file);
                let dir = std::path::Path::new(&p).parent().map(|d| d.display().to_string()).unwrap_or_default();
                Value::str(&dir)
            }
            "__CLASS__" => Value::str(f.class.as_ref().map(|c| &*c.name).unwrap_or("")),
            "__FUNCTION__" => Value::str(&f.func),
            "__METHOD__" => match &f.class {
                Some(c) if !f.func.is_empty() && !f.func.starts_with("{closure") => Value::str(&format!("{}::{}", c.name, f.func)),
                _ => Value::str(&f.func),
            },
            "__NAMESPACE__" => Value::str(&f.ctx.ns),
            _ => Value::str(""),
        }
    }

    /// A literal array of constants is built once and shared (copy on write),
    /// like PHP's immutable arrays.
    fn const_array_cached(&self, e: &'static Expr, items: &'static [Option<ast::ArrayItem>]) -> Option<Value> {
        let key = e as *const Expr as usize;
        if let Some(v) = self.const_arrays.borrow().get(&key) {
            return v.clone();
        }
        let v = const_array(items);
        self.const_arrays.borrow_mut().insert(key, v.clone());
        v
    }

    fn eval_array(&self, f: &mut Frame, items: &'static [Option<ast::ArrayItem>]) -> R<Value> {
        let mut a = PhpArray::new();
        for it in items.iter().flatten() {
            let v = if it.by_ref { Value::Ref(self.make_ref(f, &it.value)?) } else { self.eval(f, &it.value)? };
            if it.spread {
                match v {
                    Value::Array(src) => {
                        for (k, v) in src.iter() {
                            match k {
                                Key::Int(_) => {
                                    a.push(v.clone());
                                }
                                Key::Str(_) => a.insert(k.clone(), v.clone()),
                            }
                        }
                    }
                    Value::Object(o) if o.class.is_a("traversable") => {
                        for (k, v) in self.iterate(&o)? {
                            match self.to_key(&k)? {
                                Key::Int(_) => {
                                    a.push(v);
                                }
                                key => a.insert(key, v),
                            }
                        }
                    }
                    _ => return Err(self.error("Error", "Only arrays and Traversables can be unpacked")),
                }
                continue;
            }
            match &it.key {
                Some(k) => {
                    let k = self.eval(f, k)?;
                    let k = self.to_key(&k)?;
                    a.insert(k, v);
                }
                None => {
                    a.push(v);
                }
            }
        }
        Ok(Value::array(a))
    }

    /// Collect (key, value) pairs from an Iterator / IteratorAggregate.
    pub fn iterate(&self, o: &Rc<Obj>) -> R<Vec<(Value, Value)>> {
        let it = if o.class.is_a("iteratoraggregate") {
            match self.call_method(o, "getIterator", vec![])? {
                Value::Object(i) => i,
                _ => return Err(self.error("TypeError", "getIterator() must return a Traversable")),
            }
        } else {
            o.clone()
        };
        let mut out = vec![];
        self.call_method(&it, "rewind", vec![])?;
        while self.call_method(&it, "valid", vec![])?.truthy() {
            let v = self.call_method(&it, "current", vec![])?;
            let k = self.call_method(&it, "key", vec![])?;
            out.push((k, v));
            self.call_method(&it, "next", vec![])?;
        }
        Ok(out)
    }

    pub(crate) fn arrow_captures(&self, a: &'static ast::ArrowFn) -> Rc<Vec<&'static str>> {
        let key = a as *const _ as usize;
        if let Some(v) = self.arrow_caps.borrow().get(&key) {
            return v.clone();
        }
        let mut names: Vec<&'static str> = vec![];
        let params: Vec<&str> = a.params.iter().map(|p| &*p.name.name).collect();
        phpx_syntax::checks::walk_expr(
            &a.body,
            &mut |e| {
                if let ExprKind::Var(n) = &e.kind {
                    let n: &'static str = n;
                    if n != "this" && !params.contains(&n) && !names.contains(&n) {
                        names.push(n);
                    }
                }
            },
            &mut |_| {},
        );
        let v = Rc::new(names);
        self.arrow_caps.borrow_mut().insert(key, v.clone());
        v
    }

    pub fn make_closure(&self, f: &Frame, kind: ClosureKind, captured: Vec<(&'static str, Value, bool)>, is_static: bool) -> Value {
        let name = self.closure_name(f, &kind);
        let data = ClosureData {
            name,
            kind,
            captured,
            this: if is_static { None } else { f.this.clone() },
            scope: f.class.clone(),
            static_class: f.static_class.clone(),
            ctx: f.ctx.clone(),
        };
        self.closure_object(crate::interp::Callable::Closure(Rc::new(data)))
    }

    /// PHP 8.4+ closure name: `{closure:<where>:<line>}`, where <where> is
    /// the enclosing function (`N\foo()`, `C::m()`, an outer closure) or,
    /// at top level, the file.
    fn closure_name(&self, f: &Frame, kind: &ClosureKind) -> Rc<str> {
        let (node, off) = match kind {
            ClosureKind::Closure(c) => (*c as *const ast::Closure as usize, c.span.start),
            ClosureKind::Arrow(a) => (*a as *const ast::ArrowFn as usize, a.span.start),
            ClosureKind::Wrap(_) => return Rc::from("{closure}"),
        };
        let outer_key = (Rc::as_ptr(&f.func) as *const u8 as usize, f.class.as_ref().map_or(0, |c| Rc::as_ptr(c) as usize));
        if let Some((k, n)) = self.closure_names.borrow().get(&node) {
            if *k == outer_key {
                return n.clone();
            }
        }
        let outer = if f.func.is_empty() {
            self.file_path(f.ctx.file)
        } else if f.func.starts_with("{closure") {
            f.func.to_string()
        } else if let Some(c) = &f.class {
            format!("{}::{}()", c.name, f.func)
        } else {
            format!("{}()", f.func)
        };
        let name: Rc<str> = Rc::from(format!("{{closure:{outer}:{}}}", self.line_of(f.ctx.file, off as u32)));
        self.closure_names.borrow_mut().insert(node, (outer_key, name.clone()));
        name
    }

    pub fn closure_object(&self, c: crate::interp::Callable) -> Value {
        let cls = self.classes.borrow().get("closure").cloned().expect("Closure class");
        Value::Object(Obj::new(cls, Internal::Closure(c)))
    }

    // ---- chains: ->, ?->, [], () -----------------------------------------

    /// Evaluate the base of a chain. `None` means a `?->` short-circuited,
    /// which skips the rest of the chain (PHP semantics).
    fn chain_base(&self, f: &mut Frame, e: &'static Expr) -> R<Option<Value>> {
        if is_chain(e) { self.eval_chain(f, e) } else { self.eval(f, e).map(Some) }
    }

    /// Each chain kind is its own non-inlined function so only the active
    /// one's locals are on the stack during nested calls.
    fn eval_chain(&self, f: &mut Frame, e: &'static Expr) -> R<Option<Value>> {
        match &e.kind {
            ExprKind::Prop { object, name, nullsafe } => self.chain_prop(f, object, name, *nullsafe),
            ExprKind::MethodCall { object, name, args, nullsafe } => self.chain_method(f, object, name, args, *nullsafe),
            ExprKind::Index { base, index } => self.chain_index(f, base, index.as_deref()),
            ExprKind::Call { callee, args } => match &callee.kind {
                ExprKind::Name(n) => self.call_named(f, n, args).map(Some),
                _ => self.chain_call(f, callee, args),
            },
            _ => self.eval(f, e).map(Some),
        }
    }

    #[inline(never)]
    fn chain_prop(&self, f: &mut Frame, object: &'static Expr, name: &'static MemberName, nullsafe: bool) -> R<Option<Value>> {
        let Some(base) = self.chain_base(f, object)? else { return Ok(None) };
        if nullsafe && base.is_null() {
            return Ok(None);
        }
        let name = self.member_name(f, name)?;
        match base {
            Value::Object(o) => self.get_prop(f, &o, &name).map(Some),
            other => {
                self.warn(&format!("Attempt to read property \"{name}\" on {}", other.type_name()));
                Ok(Some(Value::Null))
            }
        }
    }

    #[inline(never)]
    fn chain_method(
        &self,
        f: &mut Frame,
        object: &'static Expr,
        name: &'static MemberName,
        args: &'static [ast::Arg],
        nullsafe: bool,
    ) -> R<Option<Value>> {
        let Some(base) = self.chain_base(f, object)? else { return Ok(None) };
        if nullsafe && base.is_null() {
            return Ok(None);
        }
        let name = self.member_name(f, name)?;
        let params = match &base {
            Value::Object(o) => o.class.find_method(&name.to_ascii_lowercase()).and_then(|m| crate::call::method_params(m)),
            _ => None,
        };
        let (pos, named) = self.eval_args_for(f, args, params)?;
        match base {
            Value::Object(o) => self.method_call(f, &o, &name, pos, named).map(Some),
            other => Err(self.error("Error", &format!("Call to a member function {name}() on {}", other.type_name()))),
        }
    }

    #[inline(never)]
    fn chain_index(&self, f: &mut Frame, base: &'static Expr, index: Option<&'static Expr>) -> R<Option<Value>> {
        let Some(b) = self.chain_base(f, base)? else { return Ok(None) };
        let Some(index) = index else { return Err(self.error("Error", "Cannot use [] for reading")) };
        let k = self.eval(f, index)?;
        self.read_index(&b, &k).map(Some)
    }

    #[inline(never)]
    fn chain_call(&self, f: &mut Frame, callee: &'static Expr, args: &'static [ast::Arg]) -> R<Option<Value>> {
        let Some(c) = self.chain_base(f, callee)? else { return Ok(None) };
        let callable = self.to_callable(f, &c)?;
        let (pos, named) = self.eval_args_for(f, args, crate::call::callable_params(&callable))?;
        self.call_callable(&callable, pos, named).map(Some)
    }

    pub fn member_name(&self, f: &mut Frame, n: &'static MemberName) -> R<Rc<str>> {
        match n {
            MemberName::Ident(i) => Ok(Rc::from(&*i.name)),
            MemberName::Dynamic(e) => {
                let v = self.eval(f, e)?;
                let b = self.to_str(&v)?;
                Ok(Rc::from(String::from_utf8_lossy(&b).as_ref()))
            }
        }
    }

    pub fn read_index(&self, base: &Value, k: &Value) -> R<Value> {
        match base {
            Value::Array(a) => {
                let key = self.to_key(k)?;
                match a.get(&key) {
                    Some(v) => Ok(v.clone()),
                    None => {
                        self.warn(&format!("Undefined array key {}", key_repr(&key)));
                        Ok(Value::Null)
                    }
                }
            }
            Value::Str(s) => {
                let i = match k {
                    Value::Int(n) => *n,
                    Value::Str(ks) => match crate::value::parse_numeric(ks) {
                        Some(crate::value::Num::Int(n)) => n,
                        _ => {
                            return Err(self.error("TypeError", &format!("Cannot access offset of type {} on string", k.type_name())))
                        }
                    },
                    Value::Bool(b) => *b as i64,
                    Value::Float(x) => *x as i64,
                    _ => return Err(self.error("TypeError", &format!("Cannot access offset of type {} on string", k.type_name()))),
                };
                let idx = if i < 0 { s.len() as i64 + i } else { i };
                if idx < 0 || idx >= s.len() as i64 {
                    self.warn(&format!("Uninitialized string offset {i}"));
                    return Ok(Value::str(""));
                }
                Ok(Value::bytes(vec![s[idx as usize]]))
            }
            Value::Object(o) if o.class.is_a("arrayaccess") => self.call_method(o, "offsetGet", vec![k.clone()]),
            Value::Object(o) => Err(self.error("Error", &format!("Cannot use object of type {} as array", o.class.name))),
            other => {
                self.warn(&format!("Trying to access array offset on {}", other.type_name()));
                Ok(Value::Null)
            }
        }
    }

    /// For empty/??: like eval, but missing things are `None`, silently.
    pub fn eval_quiet(&self, f: &mut Frame, e: &'static Expr) -> R<Option<Value>> {
        self.eval_quiet_mode(f, e, false)
    }

    /// `isset_only`: a magic property answers from `__isset()` alone, without
    /// calling `__get()` (PHP's isset() semantics).
    fn eval_quiet_mode(&self, f: &mut Frame, e: &'static Expr, isset_only: bool) -> R<Option<Value>> {
        match &e.kind {
            ExprKind::Var(n) => {
                if n == "this" {
                    return Ok(f.this.clone().map(Value::Object));
                }
                match f.vars.get(&**n) {
                    Some(s) => Ok(Some(s.borrow().clone())),
                    None if n == "GLOBALS" => Ok(Some(self.globals_array(f))),
                    None => Ok(None),
                }
            }
            ExprKind::VarVar(n) => {
                let name = self.var_name(f, n)?;
                if name == "this" {
                    return Ok(f.this.clone().map(Value::Object));
                }
                Ok(f.vars.get(name).map(|s| s.borrow().clone()))
            }
            ExprKind::GlobalVar(n) => {
                let name = self.var_name(f, n)?;
                Ok(self.with_globals(f, |v| v.get(name).map(|s| s.borrow().clone())))
            }
            ExprKind::Index { base, index: Some(index) } => {
                let Some(b) = self.eval_quiet(f, base)? else { return Ok(None) };
                let k = self.eval(f, index)?;
                match &b {
                    Value::Array(a) => Ok(a.get(&self.to_key(&k)?)),
                    Value::Str(s) => {
                        let i = match &k {
                            Value::Int(n) => *n,
                            Value::Str(ks) => match crate::value::parse_numeric(ks) {
                                Some(crate::value::Num::Int(n)) => n,
                                _ => return Ok(None),
                            },
                            _ => return Ok(None),
                        };
                        let idx = if i < 0 { s.len() as i64 + i } else { i };
                        Ok((idx >= 0 && idx < s.len() as i64).then(|| Value::bytes(vec![s[idx as usize]])))
                    }
                    Value::Object(o) if o.class.is_a("arrayaccess") => {
                        if self.call_method(o, "offsetExists", vec![k.clone()])?.truthy() {
                            Ok(Some(self.call_method(o, "offsetGet", vec![k])?))
                        } else {
                            Ok(None)
                        }
                    }
                    _ => Ok(None),
                }
            }
            ExprKind::Prop { object, name, .. } => {
                let Some(b) = self.eval_quiet(f, object)? else { return Ok(None) };
                let Value::Object(o) = b else { return Ok(None) };
                let name = self.member_name(f, name)?;
                let name = self.prop_key_rc(f, &o, name);
                if self.prop_visible(f, &o, &name) {
                    if let Some(v) = o.get_prop(&name) {
                        return Ok(Some(v));
                    }
                }
                if o.class.find_method("__isset").is_some() {
                    if self.call_method(&o, "__isset", vec![Value::str(&name)])?.truthy() {
                        if isset_only {
                            return Ok(Some(Value::Bool(true)));
                        }
                        return Ok(Some(self.call_method(&o, "__get", vec![Value::str(&name)])?));
                    }
                }
                Ok(None)
            }
            ExprKind::StaticProp { class, name } => {
                let cls = self.class_ref(f, class)?;
                match cls.static_props.get(&*name.name) {
                    Some(sp) => self.static_prop_slot(&sp.clone()).map(Some),
                    None => Ok(None),
                }
            }
            ExprKind::ClassConst { .. } | ExprKind::Name(_) => self.eval(f, e).map(Some),
            _ => self.eval(f, e).map(Some),
        }
    }

    // ---- places ------------------------------------------------------------

    pub fn place(&self, f: &mut Frame, e: &'static Expr) -> R<Place> {
        match &e.kind {
            ExprKind::Var(n) => {
                if &**n == "this" {
                    return Err(self.error("Error", "Cannot re-assign $this"));
                }
                Ok(Place { root: Root::Var(f.var_slot(n)), path: vec![] })
            }
            ExprKind::VarVar(n) => {
                let name = self.var_name(f, n)?;
                if name == "this" {
                    return Err(self.error("Error", "Cannot re-assign $this"));
                }
                Ok(Place { root: Root::Var(f.var_slot(name)), path: vec![] })
            }
            ExprKind::GlobalVar(n) => {
                let name = self.var_name(f, n)?;
                Ok(Place { root: Root::Var(self.global_slot(f, name)), path: vec![] })
            }
            ExprKind::Index { base, index } => {
                let mut p = self.place(f, base)?;
                let k = match index {
                    Some(i) => {
                        let v = self.eval(f, i)?;
                        Some(self.to_key_for_write(&v)?)
                    }
                    None => None,
                };
                p.path.push(k);
                Ok(p)
            }
            ExprKind::Prop { object, name, nullsafe } => {
                if *nullsafe {
                    return Err(self.error("Error", "Can't use nullsafe operator in write context"));
                }
                let name = self.member_name(f, name)?;
                let obj = match &object.kind {
                    ExprKind::Var(n) if &**n == "this" => f.this.clone().map(Value::Object).unwrap_or(Value::Null),
                    _ => self.eval(f, object)?,
                };
                match obj {
                    Value::Object(o) => {
                        let name = self.prop_key_rc(f, &o, name);
                        Ok(Place { root: Root::Prop(o, name), path: vec![] })
                    }
                    other => Err(self.error("Error", &format!("Attempt to assign property \"{name}\" on {}", other.type_name()))),
                }
            }
            ExprKind::StaticProp { class, name } => {
                let sp = self.find_static_prop(f, class, &name.name)?;
                self.static_prop_slot(&sp)?; // make sure it's initialized
                Ok(Place { root: Root::Static(sp), path: vec![] })
            }
            ExprKind::Call { .. } | ExprKind::MethodCall { .. } | ExprKind::StaticCall { .. } => {
                Err(self.error("Error", "Can't use function return value in write context"))
            }
            _ => Err(self.error("Error", "Cannot assign to this expression")),
        }
    }

    /// Array keys used for writes: same as reads.
    fn to_key_for_write(&self, v: &Value) -> R<Key> {
        self.to_key(v)
    }

    pub fn place_get(&self, f: &mut Frame, p: &Place) -> R<Option<Value>> {
        let mut cur = match &p.root {
            Root::Var(s) => s.borrow().clone(),
            Root::Prop(o, name) => {
                if self.prop_visible(f, o, name) {
                    match o.get_prop(name) {
                        Some(v) => v,
                        None if p.path.is_empty() && o.class.find_method("__get").is_none() => return Ok(None),
                        None => self.get_prop(f, o, name)?,
                    }
                } else {
                    self.get_prop(f, o, name)?
                }
            }
            Root::Static(sp) => self.static_prop_slot(sp)?,
        };
        for k in &p.path {
            let Some(k) = k else { return Ok(None) };
            cur = match &cur {
                Value::Array(a) => match a.get(k) {
                    Some(v) => v.clone(),
                    None => return Ok(None),
                },
                Value::Object(o) if o.class.is_a("arrayaccess") => {
                    let kv = k.to_value();
                    if !self.call_method(o, "offsetExists", vec![kv.clone()])?.truthy() {
                        return Ok(None);
                    }
                    self.call_method(o, "offsetGet", vec![kv])?
                }
                Value::Str(_) => self.read_index(&cur, &k.to_value())?,
                _ => return Ok(None),
            };
        }
        Ok(Some(cur))
    }

    pub fn place_set(&self, f: &mut Frame, p: &Place, v: Value) -> R<()> {
        if p.path.is_empty() {
            match &p.root {
                Root::Var(s) => *s.borrow_mut() = v,
                Root::Prop(o, name) => self.set_prop(f, o, name, v)?,
                Root::Static(sp) => *sp.value.borrow_mut() = Some(v),
            }
            return Ok(());
        }
        // Take the root value out, write into it, put it back. Taking it
        // keeps the array uniquely owned so Rc::make_mut doesn't copy.
        let pending = match &p.root {
            Root::Var(s) => {
                let mut cur = std::mem::take(&mut *s.borrow_mut());
                let r = self.write_path(&mut cur, &p.path, v);
                *s.borrow_mut() = cur;
                r?
            }
            Root::Prop(o, name) => {
                let decl_ro = o.class.find_prop(name).is_some_and(|d| d.readonly) || o.class.is_enum();
                let present = o.props.borrow().contains_key(&**name);
                if !present && o.class.find_method("__get").is_some() {
                    // Indirect modification through __get works only for objects.
                    match self.get_prop(f, o, name)? {
                        Value::Object(inner) => Some(Pending { obj: inner, path: p.path.clone(), value: v }),
                        _ => {
                            self.warn(&format!("Indirect modification of overloaded property {}::${name} has no effect", o.class.name));
                            None
                        }
                    }
                } else {
                    if !self.prop_visible(f, o, name) {
                        return Err(self.error("Error", &format!("Cannot modify {} property {}::${name}", vis_word(o, name), o.class.name)));
                    }
                    let mut cur = o.props.borrow_mut().get_mut(&**name).map(std::mem::take).unwrap_or_default();
                    if decl_ro && present && !matches!(cur, Value::Object(_)) {
                        o.props.borrow_mut().insert(name.clone(), cur);
                        return Err(self.error("Error", &format!("Cannot modify readonly property {}::${name}", o.class.name)));
                    }
                    let r = self.write_path(&mut cur, &p.path, v);
                    o.props.borrow_mut().insert(name.clone(), cur);
                    r?
                }
            }
            Root::Static(sp) => {
                let mut cur = sp.value.borrow_mut().take().unwrap_or_default();
                let r = self.write_path(&mut cur, &p.path, v);
                *sp.value.borrow_mut() = Some(cur);
                r?
            }
        };
        if let Some(p) = pending {
            self.finish_pending(p)?;
        }
        Ok(())
    }

    pub(crate) fn write_path(&self, cur: &mut Value, path: &[Option<Key>], v: Value) -> R<Option<Pending>> {
        self.write_path_mode(cur, path, v, false)
    }

    /// `rebind`: replace a reference stored at the target instead of writing
    /// through it (for `$a[0] = &$x`).
    fn write_path_mode(&self, cur: &mut Value, path: &[Option<Key>], v: Value, rebind: bool) -> R<Option<Pending>> {
        let (first, rest) = path.split_first().expect("non-empty path");
        if let Value::Ref(s) = cur {
            let s = s.clone();
            return self.write_path_mode(&mut s.borrow_mut(), path, v, rebind);
        }
        match cur {
            Value::Null => *cur = Value::array(PhpArray::new()),
            Value::Bool(false) => {
                self.deprecated("Automatic conversion of false to array is deprecated");
                *cur = Value::array(PhpArray::new());
            }
            _ => {}
        }
        match cur {
            Value::Array(a) => {
                let a = Rc::make_mut(a);
                let slot = match first {
                    None => {
                        let k = a.push(Value::Null);
                        a.get_mut(&k).unwrap()
                    }
                    Some(k) => a.entry(k.clone()),
                };
                if !rest.is_empty() {
                    return self.write_path_mode(slot, rest, v, rebind);
                }
                match slot {
                    Value::Ref(s) if !rebind => *s.borrow_mut() = v,
                    _ => *slot = v,
                }
                Ok(None)
            }
            Value::Object(o) => Ok(Some(Pending { obj: o.clone(), path: path.to_vec(), value: v })),
            Value::Str(s) if rest.is_empty() => {
                let Some(k) = first else { return Err(self.error("Error", "[] operator not supported for strings")) };
                let Key::Int(i) = k else {
                    return Err(self.error("TypeError", "Cannot access offset of type string on string"));
                };
                let mut bytes = s.to_vec();
                let idx = if *i < 0 { bytes.len() as i64 + i } else { *i };
                if idx < 0 {
                    self.warn(&format!("Illegal string offset {i}"));
                    return Ok(None);
                }
                let idx = idx as usize;
                if idx >= bytes.len() {
                    bytes.resize(idx + 1, b' ');
                }
                let vb = self.to_str(&v)?;
                match vb.first() {
                    Some(c) => bytes[idx] = *c,
                    None => return Err(self.error("Error", "Cannot assign an empty string to a string offset")),
                }
                *cur = Value::bytes(bytes);
                Ok(None)
            }
            Value::Str(_) => Err(self.error("Error", "Cannot use string offset as an array")),
            _ => Err(self.error("Error", "Cannot use a scalar value as an array")),
        }
    }

    // ---- references ------------------------------------------------------

    /// The shared cell behind an assignable expression, for `&`. Array
    /// elements and properties become `Value::Ref` on first use.
    pub fn make_ref(&self, f: &mut Frame, e: &'static Expr) -> R<Slot> {
        match &e.kind {
            ExprKind::Var(n) if &**n != "this" => Ok(f.var_slot(n)),
            ExprKind::Index { .. } | ExprKind::Prop { .. } | ExprKind::StaticProp { .. } | ExprKind::VarVar(_) | ExprKind::GlobalVar(_) => {
                let p = self.place(f, e)?;
                self.place_ref(&p)
            }
            _ => {
                // `$a = &f()`: PHP notices and binds to a copy.
                let v = self.eval(f, e)?;
                Ok(slot(v))
            }
        }
    }

    pub fn place_ref(&self, p: &Place) -> R<Slot> {
        match &p.root {
            Root::Var(s) if p.path.is_empty() => Ok(s.clone()),
            Root::Var(s) => {
                let mut cur = std::mem::take(&mut *s.borrow_mut());
                let r = self.ref_path(&mut cur, &p.path);
                *s.borrow_mut() = cur;
                r
            }
            Root::Prop(o, name) => {
                let mut props = o.props.borrow_mut();
                let raw = props.slot_or_default(name.clone());
                if p.path.is_empty() {
                    return Ok(cell_of(raw));
                }
                let mut cur = std::mem::take(raw);
                drop(props);
                let r = self.ref_path(&mut cur, &p.path);
                o.props.borrow_mut().insert(name.clone(), cur);
                r
            }
            Root::Static(sp) => {
                let mut v = sp.value.borrow_mut();
                let raw = v.get_or_insert(Value::Null);
                if p.path.is_empty() {
                    return Ok(cell_of(raw));
                }
                let mut cur = std::mem::take(raw);
                drop(v);
                let r = self.ref_path(&mut cur, &p.path);
                *sp.value.borrow_mut() = Some(cur);
                r
            }
        }
    }

    fn ref_path(&self, cur: &mut Value, path: &[Option<Key>]) -> R<Slot> {
        if let Value::Ref(s) = cur {
            let s = s.clone();
            return self.ref_path(&mut s.borrow_mut(), path);
        }
        if matches!(cur, Value::Null) {
            *cur = Value::array(PhpArray::new());
        }
        let (first, rest) = path.split_first().expect("non-empty path");
        match cur {
            Value::Array(a) => {
                let a = Rc::make_mut(a);
                let slot = match first {
                    None => {
                        let k = a.push(Value::Null);
                        a.get_mut(&k).unwrap()
                    }
                    Some(k) => a.entry(k.clone()),
                };
                if rest.is_empty() { Ok(cell_of(slot)) } else { self.ref_path(slot, rest) }
            }
            Value::Object(o) if o.class.is_a("arrayaccess") => {
                self.warn(&format!("Indirect modification of overloaded element of {} has no effect", o.class.name));
                Ok(slot(Value::Null))
            }
            _ => Err(self.error("Error", "Cannot create references to elements of a non-array")),
        }
    }

    /// Make `target` an alias of the cell `s` (`$target = &...`).
    pub fn bind_ref(&self, f: &mut Frame, target: &'static Expr, s: Slot) -> R<()> {
        match &target.kind {
            ExprKind::Var(n) if &**n != "this" => {
                f.vars.insert(n, s);
                Ok(())
            }
            ExprKind::VarVar(n) => {
                let name = self.var_name(f, n)?;
                f.vars.insert(name, s);
                Ok(())
            }
            ExprKind::GlobalVar(n) => {
                let name = self.var_name(f, n)?;
                self.with_globals(f, |v| v.insert(name, s));
                Ok(())
            }
            _ => {
                let p = self.place(f, target)?;
                if p.path.is_empty() {
                    match &p.root {
                        Root::Prop(o, name) => {
                            o.props.borrow_mut().insert(name.clone(), Value::Ref(s));
                        }
                        Root::Static(sp) => *sp.value.borrow_mut() = Some(Value::Ref(s)),
                        Root::Var(_) => unreachable!(),
                    }
                    return Ok(());
                }
                let pending = match &p.root {
                    Root::Var(v) => {
                        let mut cur = std::mem::take(&mut *v.borrow_mut());
                        let r = self.write_path_mode(&mut cur, &p.path, Value::Ref(s), true);
                        *v.borrow_mut() = cur;
                        r?
                    }
                    Root::Prop(o, name) => {
                        let mut cur = o.props.borrow_mut().get_mut(&**name).map(std::mem::take).unwrap_or_default();
                        let r = self.write_path_mode(&mut cur, &p.path, Value::Ref(s), true);
                        o.props.borrow_mut().insert(name.clone(), cur);
                        r?
                    }
                    Root::Static(sp) => {
                        let mut cur = sp.value.borrow_mut().take().unwrap_or_default();
                        let r = self.write_path_mode(&mut cur, &p.path, Value::Ref(s), true);
                        *sp.value.borrow_mut() = Some(cur);
                        r?
                    }
                };
                if pending.is_some() {
                    return Err(self.error("Error", "Cannot assign by reference to an ArrayAccess element"));
                }
                Ok(())
            }
        }
    }

    pub(crate) fn finish_pending(&self, p: Pending) -> R<()> {
        let o = p.obj;
        if !o.class.is_a("arrayaccess") {
            return Err(self.error("Error", &format!("Cannot use object of type {} as array", o.class.name)));
        }
        let key = p.path[0].as_ref().map(Key::to_value).unwrap_or(Value::Null);
        if p.path.len() == 1 {
            self.call_method(&o, "offsetSet", vec![key, p.value])?;
            return Ok(());
        }
        match self.call_method(&o, "offsetGet", vec![key])? {
            Value::Object(inner) => {
                let mut tmp = Value::Object(inner);
                if let Some(next) = self.write_path(&mut tmp, &p.path[1..], p.value)? {
                    self.finish_pending(next)?;
                }
                Ok(())
            }
            _ => {
                self.warn(&format!("Indirect modification of overloaded element of {} has no effect", o.class.name));
                Ok(())
            }
        }
    }

    pub fn assign_to(&self, f: &mut Frame, target: &'static Expr, v: Value) -> R<()> {
        match &target.kind {
            ExprKind::Array(items) => self.destructure(f, items, v),
            ExprKind::Var(n) if &**n != "this" => {
                *f.var_slot(n).borrow_mut() = v;
                Ok(())
            }
            _ => {
                let p = self.place(f, target)?;
                self.place_set(f, &p, v)
            }
        }
    }

    fn destructure(&self, f: &mut Frame, items: &'static [Option<ast::ArrayItem>], v: Value) -> R<()> {
        let mut pos = 0i64;
        for it in items {
            let Some(it) = it else {
                pos += 1;
                continue;
            };
            let key = match &it.key {
                Some(k) => {
                    let kv = self.eval(f, k)?;
                    self.to_key(&kv)?
                }
                None => {
                    pos += 1;
                    Key::Int(pos - 1)
                }
            };
            let elem = match &v {
                Value::Array(a) => match a.get(&key) {
                    Some(x) => x.clone(),
                    None => {
                        self.warn(&format!("Undefined array key {}", key_repr(&key)));
                        Value::Null
                    }
                },
                Value::Object(o) if o.class.is_a("arrayaccess") => self.call_method(o, "offsetGet", vec![key.to_value()])?,
                _ => Value::Null,
            };
            self.assign_to(f, &it.value, elem)?;
        }
        Ok(())
    }

    pub fn unset(&self, f: &mut Frame, e: &'static Expr) -> R<()> {
        match &e.kind {
            ExprKind::Var(n) => {
                f.vars.remove(&**n);
                Ok(())
            }
            ExprKind::VarVar(n) => {
                let name = self.var_name(f, n)?;
                f.vars.remove(name);
                Ok(())
            }
            ExprKind::GlobalVar(n) => {
                let name = self.var_name(f, n)?;
                self.with_globals(f, |v| v.remove(name));
                Ok(())
            }
            ExprKind::Index { base, index: Some(index) } => {
                let kv = self.eval(f, index)?;
                // Object containers: offsetUnset.
                if let Some(Value::Object(o)) = self.eval_quiet(f, base)? {
                    if o.class.is_a("arrayaccess") {
                        self.call_method(&o, "offsetUnset", vec![kv])?;
                    }
                    return Ok(());
                }
                let key = self.to_key(&kv)?;
                let p = self.place(f, base)?;
                fn remove_at(cur: &mut Value, path: &[Option<Key>], key: &Key) {
                    match cur {
                        Value::Ref(s) => {
                            let s = s.clone();
                            remove_at(&mut s.borrow_mut(), path, key);
                        }
                        Value::Array(a) => match path.split_first() {
                            None => {
                                Rc::make_mut(a).remove(key);
                            }
                            Some((Some(k), rest)) => {
                                if let Some(next) = Rc::make_mut(a).get_mut(k) {
                                    remove_at(next, rest, key);
                                }
                            }
                            Some((None, _)) => {}
                        },
                        _ => {}
                    }
                }
                let remove = |cur: &mut Value| remove_at(cur, &p.path, &key);
                match &p.root {
                    Root::Var(s) => remove(&mut s.borrow_mut()),
                    Root::Prop(o, name) => {
                        let mut cur = o.props.borrow_mut().get_mut(&**name).map(std::mem::take);
                        if let Some(c) = cur.as_mut() {
                            remove(c);
                            o.props.borrow_mut().insert(name.clone(), cur.unwrap());
                        }
                    }
                    Root::Static(sp) => {
                        if let Some(c) = sp.value.borrow_mut().as_mut() {
                            remove(c);
                        }
                    }
                }
                Ok(())
            }
            ExprKind::Prop { object, name, .. } => {
                let name = self.member_name(f, name)?;
                if let Value::Object(o) = self.eval(f, object)? {
                    let name = self.prop_key_rc(f, &o, name);
                    if o.props.borrow().contains_key(&*name) {
                        if o.class.find_prop(&name).is_some_and(|d| d.readonly) {
                            return Err(self.error("Error", &format!("Cannot unset readonly property {}::${name}", o.class.name)));
                        }
                        o.props.borrow_mut().shift_remove(&*name);
                    } else if o.class.find_method("__unset").is_some() {
                        self.call_method(&o, "__unset", vec![Value::str(&name)])?;
                    }
                }
                Ok(())
            }
            ExprKind::StaticProp { .. } => Err(self.error("Error", "Attempt to unset static property")),
            _ => Ok(()),
        }
    }

    // ---- properties ----------------------------------------------------------

    /// Storage key of `$o->name` seen from the running scope: the plain name,
    /// or the mangled key of an ancestor's private property that a subclass
    /// redeclared (only when the scope is that ancestor).
    pub fn prop_key<'a>(&self, f: &Frame, o: &Obj, name: &'a str) -> std::borrow::Cow<'a, str> {
        if !o.class.shadowed.is_empty() {
            if let Some(scope) = &f.class {
                for (decl, plain, mangled) in &o.class.shadowed {
                    if &**plain == name && **decl == *scope.lname {
                        return std::borrow::Cow::Owned(mangled.to_string());
                    }
                }
            }
        }
        std::borrow::Cow::Borrowed(name)
    }

    pub fn prop_key_rc(&self, f: &Frame, o: &Obj, name: Rc<str>) -> Rc<str> {
        match self.prop_key(f, o, &name) {
            std::borrow::Cow::Owned(k) => Rc::from(k.as_str()),
            std::borrow::Cow::Borrowed(_) => name,
        }
    }

    /// Is the property stored under `key` (already mapped with `prop_key`)
    /// visible from the running scope?
    pub fn prop_visible(&self, f: &Frame, o: &Obj, key: &str) -> bool {
        let Some(d) = o.class.find_prop(key) else { return true };
        self.member_visible(f.class.as_deref(), d.vis, &d.declaring)
    }

    pub fn member_visible(&self, scope: Option<&crate::class::Class>, vis: Vis, declaring: &str) -> bool {
        match vis {
            Vis::Public => true,
            Vis::Private => scope.is_some_and(|c| c.name.eq_ignore_ascii_case(declaring) || *c.lname == *declaring),
            Vis::Protected => scope.is_some_and(|c| {
                let d = self.lower.get(declaring);
                c.is_a(&d) || self.classes.borrow().get(&*d).is_some_and(|dc| dc.is_a(&c.lname))
            }),
        }
    }

    pub fn get_prop(&self, f: &Frame, o: &Rc<Obj>, name: &str) -> R<Value> {
        let key = self.prop_key(f, o, name);
        let name = &*key;
        let visible = self.prop_visible(f, o, name);
        if visible {
            if let Some(v) = o.get_prop(name) {
                return Ok(v);
            }
        }
        if o.class.find_method("__get").is_some() {
            return self.call_method(o, "__get", vec![Value::str(name)]);
        }
        if !visible {
            return Err(self.error("Error", &format!("Cannot access {} property {}::${name}", vis_word(o, name), o.class.name)));
        }
        if let Some(d) = o.class.find_prop(name) {
            if matches!(d.default, crate::class::PropDefault::Uninit) {
                return Err(self.error(
                    "Error",
                    &format!("Typed property {}::${name} must not be accessed before initialization", d.declaring),
                ));
            }
        }
        self.warn(&format!("Undefined property: {}::${name}", o.class.name));
        Ok(Value::Null)
    }

    pub fn set_prop(&self, f: &Frame, o: &Rc<Obj>, name: &str, v: Value) -> R<()> {
        let key = self.prop_key(f, o, name);
        let name = &*key;
        if o.class.is_enum() {
            return Err(self.error("Error", &format!("Cannot modify readonly property {}::${name}", o.class.name)));
        }
        match o.class.find_prop(name).cloned() {
            Some(d) => {
                if !self.member_visible(f.class.as_deref(), d.vis, &d.declaring) {
                    if o.class.find_method("__set").is_some() {
                        self.call_method(o, "__set", vec![Value::str(name), v])?;
                        return Ok(());
                    }
                    return Err(self.error("Error", &format!("Cannot modify {} property {}::${name}", vis_word(o, name), o.class.name)));
                }
                if d.readonly {
                    if o.props.borrow().contains_key(name) {
                        return Err(self.error("Error", &format!("Cannot modify readonly property {}::${name}", o.class.name)));
                    }
                    let in_scope = f.class.as_ref().is_some_and(|c| c.name.eq_ignore_ascii_case(&d.declaring));
                    if !in_scope {
                        let scope = f.class.as_ref().map(|c| format!("scope {}", c.name)).unwrap_or_else(|| "global scope".into());
                        return Err(self.error("Error", &format!("Cannot initialize readonly property {}::${name} from {scope}", o.class.name)));
                    }
                }
                let v = match d.ty {
                    Some(ty) => {
                        let decl_class = self.classes.borrow().get(&*self.lower.get(&d.declaring)).cloned();
                        match self.coerce(v.clone(), ty, &d.ctx, decl_class.as_ref(), Some(&o.class), f.ctx.strict)? {
                            Some(v) => v,
                            None => {
                                return Err(self.error(
                                    "TypeError",
                                    &format!(
                                        "Cannot assign {} to property {}::${name} of type {}",
                                        v.debug_type(),
                                        o.class.name,
                                        crate::call::type_str(ty)
                                    ),
                                ))
                            }
                        }
                    }
                    None => v,
                };
                o.set_prop(name, v);
            }
            None => {
                if o.props.borrow().contains_key(name) {
                    o.set_prop(name, v);
                } else if o.class.find_method("__set").is_some() {
                    self.call_method(o, "__set", vec![Value::str(name), v])?;
                } else {
                    if &*o.class.lname != "stdclass" {
                        self.deprecated(&format!("Creation of dynamic property {}::${name} is deprecated", o.class.name));
                    }
                    o.set_prop(name, v);
                }
            }
        }
        Ok(())
    }
}

fn vis_word(o: &Obj, name: &str) -> &'static str {
    match o.class.find_prop(name).map(|d| d.vis) {
        Some(Vis::Private) => "private",
        Some(Vis::Protected) => "protected",
        _ => "public",
    }
}

/// The cell for a stored value: an existing reference, or the value moved
/// into a new cell that the storage now references.
fn cell_of(raw: &mut Value) -> Slot {
    if let Value::Ref(s) = raw {
        return s.clone();
    }
    let s = slot(std::mem::take(raw));
    *raw = Value::Ref(s.clone());
    s
}

pub fn key_repr(k: &Key) -> String {
    match k {
        Key::Int(n) => n.to_string(),
        Key::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s)),
    }
}

/// The value of an array literal made only of constants (scalars, strings,
/// nested constant arrays), or None.
pub fn const_array(items: &[Option<ast::ArrayItem>]) -> Option<Value> {
    fn scalar(e: &Expr) -> Option<Value> {
        Some(match &e.kind {
            ExprKind::Int(n) => Value::Int(*n),
            ExprKind::Float(x) => Value::Float(*x),
            ExprKind::Str(s) => Value::str(s),
            ExprKind::Bytes(b) => Value::bytes(b.clone()),
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Null => Value::Null,
            ExprKind::Unary { op: ast::UnOp::Neg, expr } => match &expr.kind {
                ExprKind::Int(n) => Value::Int(n.checked_neg()?),
                ExprKind::Float(x) => Value::Float(-x),
                _ => return None,
            },
            ExprKind::Array(inner) => const_array(inner)?,
            _ => return None,
        })
    }
    let mut a = PhpArray::new();
    for it in items {
        let it = it.as_ref()?;
        if it.spread || it.by_ref {
            return None;
        }
        let v = scalar(&it.value)?;
        match &it.key {
            None => {
                a.push(v);
            }
            Some(k) => {
                let key = match scalar(k)? {
                    Value::Int(n) => Key::Int(n),
                    Value::Str(s) => Key::from_bytes(&s),
                    Value::Bool(b) => Key::Int(b as i64),
                    Value::Null => Key::Str(crate::value::pstr("")),
                    _ => return None,
                };
                a.insert(key, v);
            }
        }
    }
    Some(Value::array(a))
}
