//! Type inference for the JIT's numeric subset.
//!
//! Every local gets one type for the whole function (int, float or bool).
//! Anything outside the subset, or a variable whose type would change
//! (int in one branch, float in another), rejects the function: it then
//! simply runs in the interpreter. Reads must be of definitely-assigned
//! variables, so compiled code never has PHP's "undefined variable" path.

use std::collections::{HashMap, HashSet};

use phpx_syntax::ast::{BinOp, Expr, ExprKind, Stmt, StmtKind, UnOp};

use crate::Ty;

/// Inference lattice: unknown yet (recursive call in progress) or a type.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum T {
    Bot,
    Ty(Ty),
}

pub type Bail = &'static str;

pub fn join(a: T, b: T) -> Result<T, Bail> {
    match (a, b) {
        (T::Bot, x) | (x, T::Bot) => Ok(x),
        (T::Ty(x), T::Ty(y)) if x == y => Ok(a),
        _ => Err("variable changes type"),
    }
}

/// What the inference needs from the outside world.
pub trait Callees {
    /// Return type of user function `name` called with `args`, compiling it
    /// if needed. `Err` if it can't be JIT-compiled.
    fn user_call(&mut self, name: &str, args: &[Ty]) -> Result<T, Bail>;
}

/// Built-ins the JIT implements inline.
pub fn builtin_type(name: &str, args: &[T]) -> Option<Result<T, Bail>> {
    let lname = name.trim_start_matches('\\').to_ascii_lowercase();
    let numeric = |t: &T| matches!(t, T::Bot | T::Ty(Ty::Int) | T::Ty(Ty::Float));
    Some(match (lname.as_str(), args) {
        ("abs", [a]) if numeric(a) => Ok(*a),
        ("min" | "max", [a, b]) if numeric(a) && numeric(b) => join(*a, *b),
        ("intdiv", [a, b]) if matches!(a, T::Bot | T::Ty(Ty::Int)) && matches!(b, T::Bot | T::Ty(Ty::Int)) => Ok(T::Ty(Ty::Int)),
        ("sqrt" | "floor" | "ceil", [a]) if numeric(a) => Ok(T::Ty(Ty::Float)),
        _ => return None,
    })
}

pub struct Infer<'c> {
    pub vars: HashMap<&'static str, T>,
    pub ret: T,
    callees: &'c mut dyn Callees,
    changed: bool,
}

impl<'c> Infer<'c> {
    /// Infer variable and return types; iterate until stable.
    pub fn run(
        params: &[(&'static str, Ty)],
        body: &'static [Stmt],
        assumed_ret: T,
        callees: &'c mut dyn Callees,
    ) -> Result<(HashMap<&'static str, T>, T), Bail> {
        let mut inf = Infer { vars: HashMap::new(), ret: assumed_ret, callees, changed: false };
        for (n, t) in params {
            inf.vars.insert(n, T::Ty(*t));
        }
        for _ in 0..8 {
            inf.changed = false;
            let mut assigned: HashSet<&'static str> = params.iter().map(|(n, _)| *n).collect();
            let diverges = inf.stmts(body, &mut assigned)?;
            if !diverges {
                return Err("function can return null");
            }
            if !inf.changed {
                return Ok((inf.vars, inf.ret));
            }
        }
        Err("types did not converge")
    }

    fn set_var(&mut self, n: &'static str, t: T) -> Result<(), Bail> {
        let old = self.vars.get(n).copied().unwrap_or(T::Bot);
        let new = join(old, t)?;
        if new != old {
            self.vars.insert(n, new);
            self.changed = true;
        }
        Ok(())
    }

    /// Returns whether the statement list always ends in `return`.
    fn stmts(&mut self, stmts: &'static [Stmt], assigned: &mut HashSet<&'static str>) -> Result<bool, Bail> {
        for s in stmts {
            if self.stmt(s, assigned)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn stmt(&mut self, s: &'static Stmt, assigned: &mut HashSet<&'static str>) -> Result<bool, Bail> {
        match &s.kind {
            StmtKind::Expr(e) => {
                self.expr(e, assigned)?;
                Ok(false)
            }
            StmtKind::Return(Some(e)) => {
                let t = self.expr(e, assigned)?;
                let r = join(self.ret, t)?;
                if r != self.ret {
                    self.ret = r;
                    self.changed = true;
                }
                Ok(true)
            }
            StmtKind::If { cond, then, elseifs, else_ } => {
                self.expr(cond, assigned)?;
                let mut outs = vec![];
                let mut branch = |inf: &mut Self, body: &'static [Stmt], start: &HashSet<&'static str>| -> Result<(), Bail> {
                    let mut a = start.clone();
                    let div = inf.stmts(body, &mut a)?;
                    outs.push((a, div));
                    Ok(())
                };
                let base = assigned.clone();
                branch(self, then, &base)?;
                let mut cur = base.clone();
                for (c, b) in elseifs {
                    self.expr(c, &mut cur)?;
                    branch(self, b, &cur)?;
                }
                match else_ {
                    Some(b) => branch(self, b, &cur)?,
                    None => outs.push((cur, false)),
                }
                let live: Vec<_> = outs.iter().filter(|(_, d)| !d).map(|(a, _)| a).collect();
                if live.is_empty() {
                    return Ok(true);
                }
                *assigned = live[0].iter().filter(|v| live.iter().all(|a| a.contains(*v))).copied().collect();
                Ok(false)
            }
            StmtKind::While { cond, body } => {
                self.expr(cond, assigned)?;
                let mut a = assigned.clone();
                self.stmts(body, &mut a)?;
                Ok(false)
            }
            StmtKind::DoWhile { body, cond } => {
                let div = self.stmts(body, assigned)?;
                if div {
                    return Ok(true);
                }
                self.expr(cond, assigned)?;
                Ok(false)
            }
            StmtKind::For { init, cond, step, body } => {
                for e in init {
                    self.expr(e, assigned)?;
                }
                for e in cond {
                    self.expr(e, assigned)?;
                }
                let mut a = assigned.clone();
                self.stmts(body, &mut a)?;
                for e in step {
                    self.expr(e, &mut a)?;
                }
                Ok(false)
            }
            StmtKind::Block(b) => self.stmts(b, assigned),
            StmtKind::Break(None) | StmtKind::Continue(None) => Ok(false),
            StmtKind::Break(Some(e)) | StmtKind::Continue(Some(e)) if matches!(e.kind, ExprKind::Int(_)) => Ok(false),
            StmtKind::Nop => Ok(false),
            _ => Err("statement not supported"),
        }
    }

    pub fn expr(&mut self, e: &'static Expr, assigned: &mut HashSet<&'static str>) -> Result<T, Bail> {
        use ExprKind::*;
        Ok(match &e.kind {
            Int(_) => T::Ty(Ty::Int),
            Float(_) => T::Ty(Ty::Float),
            Bool(_) => T::Ty(Ty::Bool),
            Var(n) => {
                if !assigned.contains(n.as_str()) {
                    return Err("variable may be undefined");
                }
                self.vars.get(n.as_str()).copied().unwrap_or(T::Bot)
            }
            Assign { target, value } => {
                let ExprKind::Var(n) = &target.kind else { return Err("assignment target not a local") };
                let t = self.expr(value, assigned)?;
                self.set_var(n, t)?;
                assigned.insert(n);
                t
            }
            AssignOp { op, target, value } => {
                let ExprKind::Var(n) = &target.kind else { return Err("assignment target not a local") };
                let cur = self.expr(target, assigned)?;
                let rhs = self.expr(value, assigned)?;
                let t = binop_type(*op, cur, rhs)?;
                self.set_var(n, t)?;
                t
            }
            IncDec { expr, .. } => {
                if !matches!(expr.kind, Var(_)) {
                    return Err("++/-- target not a local");
                }
                match self.expr(expr, assigned)? {
                    t @ (T::Bot | T::Ty(Ty::Int) | T::Ty(Ty::Float)) => t,
                    _ => return Err("++/-- on bool"),
                }
            }
            Binary { op, left, right } => {
                let a = self.expr(left, assigned)?;
                // Right side of && / || may not run: its assignments don't count.
                let b = if matches!(op, BinOp::And | BinOp::Or) {
                    let mut tmp = assigned.clone();
                    self.expr(right, &mut tmp)?
                } else {
                    self.expr(right, assigned)?
                };
                binop_type(*op, a, b)?
            }
            Unary { op, expr } => {
                let t = self.expr(expr, assigned)?;
                match op {
                    UnOp::Not => T::Ty(Ty::Bool),
                    UnOp::Neg | UnOp::Plus => match t {
                        T::Ty(Ty::Bool) => return Err("unary minus on bool"),
                        t => t,
                    },
                    UnOp::BitNot => match t {
                        T::Bot | T::Ty(Ty::Int) => T::Ty(Ty::Int),
                        _ => return Err("~ on non-int"),
                    },
                    UnOp::Silence => return Err("@ not supported"),
                }
            }
            Ternary { cond, then, else_ } => {
                let c = self.expr(cond, assigned)?;
                let mut a1 = assigned.clone();
                let mut a2 = assigned.clone();
                let t1 = match then {
                    Some(t) => self.expr(t, &mut a1)?,
                    None => c,
                };
                let t2 = self.expr(else_, &mut a2)?;
                *assigned = a1.intersection(&a2).copied().collect();
                join(t1, t2)?
            }
            Cast { ty, expr } => {
                self.expr(expr, assigned)?;
                match *ty {
                    "int" => T::Ty(Ty::Int),
                    "float" => T::Ty(Ty::Float),
                    "bool" => T::Ty(Ty::Bool),
                    _ => return Err("cast not supported"),
                }
            }
            Call { callee, args } => {
                let ExprKind::Name(name) = &callee.kind else { return Err("dynamic call") };
                if args.iter().any(|a| a.spread || a.name.is_some()) {
                    return Err("spread/named args");
                }
                let mut tys = vec![];
                for a in args {
                    tys.push(self.expr(&a.value, assigned)?);
                }
                if let Some(t) = builtin_type(name, &tys) {
                    return t;
                }
                let concrete: Option<Vec<Ty>> = tys.iter().map(|t| if let T::Ty(t) = t { Some(*t) } else { None }).collect();
                let Some(concrete) = concrete else { return Ok(T::Bot) };
                self.callees.user_call(name, &concrete)?
            }
            _ => return Err("expression not supported"),
        })
    }
}

pub fn binop_type(op: BinOp, a: T, b: T) -> Result<T, Bail> {
    use BinOp::*;
    use Ty::*;
    if matches!(op, And | Or | Xor) {
        return Ok(T::Ty(Bool));
    }
    let (T::Ty(x), T::Ty(y)) = (a, b) else {
        // Unknown operand (recursive call in progress): result unknown too,
        // except for operators whose result type doesn't depend on it.
        return Ok(match op {
            Eq | NotEq | Identical | NotIdentical | Lt | LtEq | Gt | GtEq => T::Ty(Bool),
            Spaceship | Mod | BitAnd | BitOr | BitXor | Shl | Shr => T::Ty(Int),
            _ => match (a, b) {
                (T::Ty(Float), _) | (_, T::Ty(Float)) if matches!(op, Add | Sub | Mul | Div) => T::Ty(Float),
                _ => T::Bot,
            },
        });
    };
    Ok(T::Ty(match op {
        Add | Sub | Mul => match (x, y) {
            (Int, Int) => Int,
            (Bool, _) | (_, Bool) => return Err("arithmetic on bool"),
            _ => Float,
        },
        Div => match (x, y) {
            (Bool, _) | (_, Bool) => return Err("arithmetic on bool"),
            // int / int is int or float depending on the values.
            (Int, Int) => return Err("int / int has a dynamic type"),
            _ => Float,
        },
        Mod | BitAnd | BitOr | BitXor | Shl | Shr => match (x, y) {
            (Int, Int) => Int,
            _ => return Err("integer operator on non-int"),
        },
        Eq | NotEq | Identical | NotIdentical | Lt | LtEq | Gt | GtEq => Bool,
        Spaceship => match (x, y) {
            (Float, _) | (_, Float) => return Err("<=> on floats"),
            _ => Int,
        },
        Pow => return Err("** not supported"),
        Concat => return Err("string operator"),
        Coalesce => return Err("?? not supported"),
        And | Or | Xor => Bool,
    }))
}
