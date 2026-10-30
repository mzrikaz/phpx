//! Leaf methods: a method whose whole body is `return <expr>;` over its
//! integer, float and bool parameters, typed properties of `$this` and
//! literals (`return $this->w * $this->h;`, `return $this->n > 0;`) is
//! compiled a second time as a frameless function that call sites call
//! straight away (see `rt::Accessor`).
//!
//! A leaf never fails: anything it can't compute exactly as the method
//! would (another value type, an uninitialized property, integer overflow,
//! division by zero) makes it decline before anything happened, and the
//! site makes the real call, which raises whatever PHP raises. Nothing in
//! it can run PHP code, so the missing frame is never observable.
//!
//! The function is compiled once per method body; what depends on the
//! object's class (its shape and the properties' slots) comes from the
//! call site's cache, as `slots` (`[shape, slot of each property...]`).

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, Block, InstBuilder, MemFlagsData, Value as CValue};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::Module;
use phpx_syntax::ast::{self, BinOp, Expr, ExprKind, MemberName, StmtKind, UnOp};

use crate::value::tag;

/// Leaf ABI: (object Rc pointer, arguments, out slot, class data) -> 0
/// (answered in `out`) or 1 (declined: make the call).
pub type LeafFn = unsafe extern "C" fn(*const u8, *const crate::value::Value, *mut crate::value::Value, *const usize) -> u64;

/// Most properties a leaf reads (the site keeps their slots).
pub const MAX_PROPS: usize = 4;

#[derive(Clone, Copy, PartialEq, Debug)]
enum T {
    I,
    F,
    B,
}

/// A leaf's plan: the properties it reads (in `slots` order) and the
/// types it computes with.
pub struct Plan {
    pub props: Vec<&'static str>,
    prop_ty: Vec<T>,
    params: Vec<T>,
    body: &'static Expr,
    ret: Option<T>,
}

fn named(t: Option<&ast::Type>) -> Option<Option<T>> {
    let Some(t) = t else { return Some(None) };
    let ast::Type::Named(n) = t else { return None };
    Some(Some(match n.to_ascii_lowercase().as_str() {
        "int" => T::I,
        "float" => T::F,
        "bool" => T::B,
        _ => return None,
    }))
}

/// The plan for method body `func`, if it is a leaf; `prop_ty` gives the
/// declared type of a property of `$this` by name.
pub fn plan(func: &'static ast::Function, prop_ty: impl Fn(&str) -> Option<&'static ast::Type>) -> Option<Plan> {
    if func.by_ref || func.is_generator || func.params.len() > 3 {
        return None;
    }
    let [st] = func.body.as_deref()? else { return None };
    let StmtKind::Return(Some(body)) = &st.kind else { return None };
    let mut params = vec![];
    for p in &func.params {
        if p.by_ref || p.variadic || p.default.is_some() || p.promote.is_some() {
            return None;
        }
        params.push(named(p.ty.as_ref())??);
    }
    let ret = named(func.ret.as_ref())?;
    let mut plan = Plan { props: vec![], prop_ty: vec![], params, body, ret };
    let t = plan.ty(body, func, &prop_ty)?;
    match (ret, t) {
        (None, _) | (Some(T::F), T::I) => {}
        (Some(r), t) if r == t => {}
        _ => return None,
    }
    Some(plan)
}

impl Plan {
    fn ty(&mut self, e: &Expr, func: &ast::Function, prop_ty: &impl Fn(&str) -> Option<&'static ast::Type>) -> Option<T> {
        Some(match &e.kind {
            ExprKind::Int(_) => T::I,
            ExprKind::Float(_) => T::F,
            ExprKind::Bool(_) => T::B,
            ExprKind::Var(v) => {
                let i = func.params.iter().position(|p| *p.name.name == **v)?;
                self.params[i]
            }
            ExprKind::Prop { object, name: MemberName::Ident(name), nullsafe: false } => {
                if !matches!(&object.kind, ExprKind::Var(v) if &**v == "this") {
                    return None;
                }
                if let Some(i) = self.props.iter().position(|p| *p == name.name) {
                    return Some(self.prop_ty[i]);
                }
                let t = named(Some(prop_ty(&name.name)?))??;
                if self.props.len() == MAX_PROPS {
                    return None;
                }
                // SAFETY: the AST lives for the whole run.
                self.props.push(unsafe { std::mem::transmute::<&str, &'static str>(&name.name) });
                self.prop_ty.push(t);
                t
            }
            ExprKind::Unary { op: UnOp::Neg, expr } => match self.ty(expr, func, prop_ty)? {
                T::B => return None,
                t => t,
            },
            ExprKind::Unary { op: UnOp::Not, expr } => match self.ty(expr, func, prop_ty)? {
                T::B => T::B,
                _ => return None,
            },
            ExprKind::Binary { op, left, right } => {
                let (a, b) = (self.ty(left, func, prop_ty)?, self.ty(right, func, prop_ty)?);
                use BinOp::*;
                match op {
                    Add | Sub | Mul if a != T::B && b != T::B => {
                        if a == T::I && b == T::I {
                            T::I
                        } else {
                            T::F
                        }
                    }
                    // (int / int may be either: not a leaf.)
                    Div if a != T::B && b != T::B && (a == T::F || b == T::F) => T::F,
                    Mod if a == T::I && b == T::I => T::I,
                    Lt | LtEq | Gt | GtEq | Eq | NotEq if a != T::B && b != T::B => T::B,
                    Identical | NotIdentical | Eq | NotEq if a == b => T::B,
                    And | Or if a == T::B && b == T::B => T::B,
                    _ => return None,
                }
            }
            ExprKind::Ternary { cond, then: Some(a), else_: b } => {
                if self.ty(cond, func, prop_ty)? != T::B {
                    return None;
                }
                let (a, b) = (self.ty(a, func, prop_ty)?, self.ty(b, func, prop_ty)?);
                if a != b {
                    return None;
                }
                a
            }
            _ => return None,
        })
    }
}

/// A computed value: an i64, an f64, or an i8 bool.
#[derive(Clone, Copy)]
enum V {
    I(CValue),
    F(CValue),
    B(CValue),
}

struct Gen<'a, 'b> {
    b: &'a mut FunctionBuilder<'b>,
    decline: Block,
    params: HashMap<&'static str, V>,
    props: Vec<V>,
    plan: &'a Plan,
}

fn mf() -> MemFlagsData {
    MemFlagsData::trusted()
}

impl Gen<'_, '_> {
    fn ok_unless(&mut self, bad: CValue) {
        let ok = self.b.create_block();
        self.b.ins().brif(bad, self.decline, &[], ok, &[]);
        self.b.switch_to_block(ok);
    }

    fn to_f(&mut self, v: V) -> CValue {
        match v {
            V::F(x) => x,
            V::I(x) => self.b.ins().fcvt_from_sint(types::F64, x),
            V::B(_) => unreachable!(),
        }
    }

    fn expr(&mut self, e: &'static Expr) -> V {
        match &e.kind {
            ExprKind::Int(n) => V::I(self.b.ins().iconst(types::I64, *n)),
            ExprKind::Float(x) => V::F(self.b.ins().f64const(*x)),
            ExprKind::Bool(v) => V::B(self.b.ins().iconst(types::I8, *v as i64)),
            ExprKind::Var(v) => self.params[v.as_str()],
            ExprKind::Prop { name: MemberName::Ident(name), .. } => {
                let i = self.plan.props.iter().position(|p| *p == name.name).unwrap();
                self.props[i]
            }
            ExprKind::Unary { op: UnOp::Neg, expr } => match self.expr(expr) {
                V::F(x) => V::F(self.b.ins().fneg(x)),
                V::I(x) => {
                    let z = self.b.ins().iconst(types::I64, 0);
                    let (r, of) = self.b.ins().ssub_overflow(z, x);
                    self.ok_unless(of);
                    V::I(r)
                }
                V::B(_) => unreachable!(),
            },
            ExprKind::Unary { op: UnOp::Not, expr } => {
                let V::B(x) = self.expr(expr) else { unreachable!() };
                V::B(self.b.ins().bxor_imm_u(x, 1))
            }
            ExprKind::Binary { op, left, right } => {
                let (a, b) = (self.expr(left), self.expr(right));
                self.binary(*op, a, b)
            }
            ExprKind::Ternary { cond, then: Some(a), else_: b } => {
                // (Both sides are computed: neither has side effects, and
                // one that declines makes the real call compute it.)
                let V::B(c) = self.expr(cond) else { unreachable!() };
                match (self.expr(a), self.expr(b)) {
                    (V::I(x), V::I(y)) => V::I(self.b.ins().select(c, x, y)),
                    (V::F(x), V::F(y)) => V::F(self.b.ins().select(c, x, y)),
                    (V::B(x), V::B(y)) => V::B(self.b.ins().select(c, x, y)),
                    _ => unreachable!(),
                }
            }
            _ => unreachable!("not a leaf"),
        }
    }

    fn binary(&mut self, op: BinOp, a: V, b: V) -> V {
        use BinOp::*;
        let cmp_i = |op: BinOp| match op {
            Lt => IntCC::SignedLessThan,
            LtEq => IntCC::SignedLessThanOrEqual,
            Gt => IntCC::SignedGreaterThan,
            GtEq => IntCC::SignedGreaterThanOrEqual,
            Eq | Identical => IntCC::Equal,
            _ => IntCC::NotEqual,
        };
        let cmp_f = |op: BinOp| match op {
            Lt => FloatCC::LessThan,
            LtEq => FloatCC::LessThanOrEqual,
            Gt => FloatCC::GreaterThan,
            GtEq => FloatCC::GreaterThanOrEqual,
            Eq | Identical => FloatCC::Equal,
            _ => FloatCC::NotEqual,
        };
        match (op, a, b) {
            (And | Or, V::B(x), V::B(y)) => V::B(if op == And { self.b.ins().band(x, y) } else { self.b.ins().bor(x, y) }),
            (Eq | NotEq | Identical | NotIdentical, V::B(x), V::B(y)) => V::B(self.b.ins().icmp(cmp_i(op), x, y)),
            (Add | Sub | Mul, V::I(x), V::I(y)) => {
                let (r, of) = match op {
                    Add => self.b.ins().sadd_overflow(x, y),
                    Sub => self.b.ins().ssub_overflow(x, y),
                    _ => self.b.ins().smul_overflow(x, y),
                };
                self.ok_unless(of);
                V::I(r)
            }
            (Mod, V::I(x), V::I(y)) => {
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                let bad = self.b.ins().bor(z, m1);
                self.ok_unless(bad);
                V::I(self.b.ins().srem(x, y))
            }
            (Lt | LtEq | Gt | GtEq | Eq | NotEq | Identical | NotIdentical, V::I(x), V::I(y)) => V::B(self.b.ins().icmp(cmp_i(op), x, y)),
            (Add | Sub | Mul | Div, a, b) => {
                let (x, y) = (self.to_f(a), self.to_f(b));
                V::F(match op {
                    Add => self.b.ins().fadd(x, y),
                    Sub => self.b.ins().fsub(x, y),
                    Mul => self.b.ins().fmul(x, y),
                    _ => {
                        let zero = self.b.ins().f64const(0.0);
                        let bad = self.b.ins().fcmp(FloatCC::Equal, y, zero);
                        self.ok_unless(bad);
                        self.b.ins().fdiv(x, y)
                    }
                })
            }
            // (Mixed int/float comparisons compare as floats, like PHP's
            // for the values a leaf sees: exact below 2^53.)
            (Lt | LtEq | Gt | GtEq | Eq | NotEq | Identical | NotIdentical, a, b) => {
                let (x, y) = (self.to_f(a), self.to_f(b));
                V::B(self.b.ins().fcmp(cmp_f(op), x, y))
            }
            _ => unreachable!("not a leaf"),
        }
    }
}

impl super::Native {
    /// The leaf function of a method body (compiled on first use).
    pub fn leaf(&mut self, func: &'static ast::Function, plan: &Plan) -> Option<LeafFn> {
        let key = func as *const ast::Function as usize;
        if let Some(l) = self.leaves.get(&key) {
            return *l;
        }
        let r = crate::alloc::uncharged(|| self.compile_leaf(func, plan));
        self.leaves.insert(key, r);
        r
    }

    fn compile_leaf(&mut self, func: &'static ast::Function, plan: &Plan) -> Option<LeafFn> {
        let lay = super::layout()?;
        let mut sig = self.module.make_signature();
        for _ in 0..4 {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = self.module.declare_anonymous_function(&sig).ok()?;
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fctx = FunctionBuilderContext::new();
        let cfg = self.module.target_config();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            b.switch_to_block(entry);
            let ps = b.block_params(entry).to_vec();
            let (rc, args, out, slots) = (ps[0], ps[1], ps[2], ps[3]);
            let decline = b.create_block();
            b.set_cold_block(decline);
            let mut g = Gen { b: &mut b, decline, params: HashMap::new(), props: vec![], plan };
            // A value of type `t` at `a`, checked (an int for a float
            // parameter is converted, as binding would).
            let load = |g: &mut Gen, a: CValue, t: T, int_ok: bool| -> V {
                let tg = g.b.ins().load(types::I8, mf(), a, 0);
                let want = match t {
                    T::I => tag::INT,
                    T::F => tag::FLOAT,
                    T::B => tag::BOOL,
                };
                if t == T::F && int_ok {
                    let (fl, int, join) = (g.b.create_block(), g.b.create_block(), g.b.create_block());
                    g.b.append_block_param(join, types::F64);
                    let is_f = g.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::FLOAT as i64);
                    g.b.ins().brif(is_f, fl, &[], int, &[]);
                    g.b.switch_to_block(fl);
                    let x = g.b.ins().load(types::F64, mf(), a, tag::PAYLOAD);
                    g.b.ins().jump(join, &[x.into()]);
                    g.b.switch_to_block(int);
                    let is_i = g.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::INT as i64);
                    let conv = g.b.create_block();
                    g.b.ins().brif(is_i, conv, &[], g.decline, &[]);
                    g.b.switch_to_block(conv);
                    let n = g.b.ins().load(types::I64, mf(), a, tag::PAYLOAD);
                    let x = g.b.ins().fcvt_from_sint(types::F64, n);
                    g.b.ins().jump(join, &[x.into()]);
                    g.b.switch_to_block(join);
                    return V::F(g.b.block_params(join)[0]);
                }
                let bad = g.b.ins().icmp_imm_u(IntCC::NotEqual, tg, want as i64);
                g.ok_unless(bad);
                match t {
                    T::I => V::I(g.b.ins().load(types::I64, mf(), a, tag::PAYLOAD)),
                    T::F => V::F(g.b.ins().load(types::F64, mf(), a, tag::PAYLOAD)),
                    T::B => V::B(g.b.ins().load(types::I8, mf(), a, tag::PAYLOAD)),
                }
            };
            for (i, (p, t)) in func.params.iter().zip(&plan.params).enumerate() {
                let a = g.b.ins().iadd_imm_s(args, (i * 16) as i64);
                let v = load(&mut g, a, *t, true);
                g.params.insert(p.name.name.as_str(), v);
            }
            if !plan.props.is_empty() {
                // The object still has the shape the site's slots are for.
                let props = g.b.ins().iadd_imm_s(rc, lay.obj_props as i64);
                let shape = g.b.ins().load(types::I64, mf(), props, 0);
                let want = g.b.ins().load(types::I64, mf(), slots, 0);
                let bad = g.b.ins().icmp(IntCC::NotEqual, shape, want);
                g.ok_unless(bad);
                let data = g.b.ins().load(types::I64, mf(), props, 8);
                for (i, t) in plan.prop_ty.iter().enumerate() {
                    let slot = g.b.ins().load(types::I64, mf(), slots, (8 + i * 8) as i32);
                    let off = g.b.ins().ishl_imm_u(slot, 4);
                    let a = g.b.ins().iadd(data, off);
                    // (Typed: never a reference; empty until initialized,
                    // which the tag check declines.)
                    let v = load(&mut g, a, *t, false);
                    g.props.push(v);
                }
            }
            let mut v = g.expr(plan.body);
            if let (Some(T::F), V::I(x)) = (plan.ret, v) {
                v = V::F(g.b.ins().fcvt_from_sint(types::F64, x));
            }
            let (t, x) = match v {
                V::I(x) => (tag::INT, x),
                V::F(x) => (tag::FLOAT, x),
                V::B(x) => (tag::BOOL, x),
            };
            let tv = g.b.ins().iconst(types::I8, t as i64);
            g.b.ins().store(mf(), tv, out, 0);
            g.b.ins().store(mf(), x, out, tag::PAYLOAD);
            let zero = g.b.ins().iconst(types::I64, 0);
            g.b.ins().return_(&[zero]);
            b.switch_to_block(decline);
            let one = b.ins().iconst(types::I64, 1);
            b.ins().return_(&[one]);
            b.seal_all_blocks();
            b.finalize(cfg);
        }
        self.module.define_function(id, &mut ctx).ok()?;
        let size = ctx.compiled_code().map_or(0, |c| c.code_info().total_size);
        self.module.finalize_definitions().ok()?;
        let ptr = self.module.get_finalized_function(id);
        super::perf_map(ptr, size, "leaf");
        // SAFETY: generated with exactly the LeafFn signature.
        Some(unsafe { std::mem::transmute::<*const u8, LeafFn>(ptr) })
    }
}
