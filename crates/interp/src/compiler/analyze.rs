//! Which functions the native compiler accepts, and their local slots.
//!
//! v1 rejects anything that needs a local to be shared by reference or
//! looked up by name at runtime (those stay in the interpreter): by-ref
//! params, `use (&$x)`, `$a = &$b`, foreach by reference, `static` vars,
//! compact()/get_defined_vars()/func_get_args(), include, nested
//! declarations, try/catch, generators, and dynamic class references.

use std::collections::HashMap;

use phpx_syntax::ast::{self, ClassRef, Expr, ExprKind, MemberName, Stmt, StmtKind};

pub type Reject = &'static str;

/// Local variable layout: name -> slot index.
#[derive(Default)]
pub struct Locals {
    pub index: HashMap<&'static str, u32>,
    pub names: Vec<&'static str>,
}

impl Locals {
    pub fn slot(&mut self, n: &'static str) -> u32 {
        if let Some(i) = self.index.get(n) {
            return *i;
        }
        let i = self.names.len() as u32;
        self.names.push(n);
        self.index.insert(n, i);
        i
    }
}

pub struct Analysis {
    pub locals: Locals,
    pub uses_this: bool,
    /// Compiling a file's top-level code (declarations allowed).
    pub top_level: bool,
}

/// Scope-reading built-ins: they need named variables.
const SCOPE_FNS: &[&str] = &["compact", "eval", "get_defined_vars", "func_get_args", "func_num_args", "extract", "get_called_class"];

pub fn analyze_mode(params: &'static [ast::Param], captured: &[&'static str], body: Body, top_level: bool) -> Result<Analysis, Reject> {
    let mut a = Analysis { locals: Locals::default(), uses_this: false, top_level };
    if let (true, Body::Block(stmts)) = (top_level, body) {
        // Only declarations directly in the segment are hoisted/declared
        // in order; conditional ones (inside if/loops) must stay interpreted.
        if stmts.iter().any(|s| nested_decl(s, 0)) {
            return Err("conditional declaration");
        }
    }
    for p in params {
        if p.by_ref {
            return Err("by-reference parameter");
        }
        a.locals.slot(&p.name.name);
    }
    for c in captured {
        a.locals.slot(c);
    }
    match body {
        Body::Block(stmts) => a.stmts(stmts)?,
        Body::Expr(e) => a.expr(e)?,
    }
    Ok(a)
}

fn nested_decl(s: &Stmt, depth: u32) -> bool {
    use StmtKind::*;
    let inner = |b: &[Stmt]| b.iter().any(|s| nested_decl(s, depth + 1));
    match &s.kind {
        Function(_) | Class(_) | Const(_) => depth > 0,
        If { then, elseifs, else_, .. } => inner(then) || elseifs.iter().any(|(_, b)| inner(b)) || else_.as_deref().is_some_and(inner),
        While { body, .. } | DoWhile { body, .. } | For { body, .. } | Foreach { body, .. } | Block(body) => inner(body),
        Switch { cases, .. } => cases.iter().any(|c| inner(&c.body)),
        Try { body, catches, finally } => inner(body) || catches.iter().any(|c| inner(&c.body)) || finally.as_deref().is_some_and(inner),
        _ => false,
    }
}

/// `finally` bodies may not leave the block (return, or break/continue
/// past `depth` enclosing loops inside it): the compiler inlines them.
fn finally_ok(s: &[Stmt], depth: usize) -> bool {
    use StmtKind::*;
    s.iter().all(|s| match &s.kind {
        Return(_) => false,
        Break(l) | Continue(l) => {
            let n = match l.as_ref().map(|e| &e.kind) {
                None => 1,
                Some(ExprKind::Int(n)) => *n as usize,
                _ => return false,
            };
            n <= depth
        }
        If { then, elseifs, else_, .. } => {
            finally_ok(then, depth) && elseifs.iter().all(|(_, b)| finally_ok(b, depth)) && else_.as_deref().is_none_or(|b| finally_ok(b, depth))
        }
        While { body, .. } | DoWhile { body, .. } | For { body, .. } | Foreach { body, .. } => finally_ok(body, depth + 1),
        Switch { cases, .. } => cases.iter().all(|c| finally_ok(&c.body, depth + 1)),
        Block(b) => finally_ok(b, depth),
        Try { body, catches, finally } => {
            finally_ok(body, depth) && catches.iter().all(|c| finally_ok(&c.body, depth)) && finally.as_deref().is_none_or(|b| finally_ok(b, depth))
        }
        _ => true,
    })
}

#[derive(Clone, Copy)]
pub enum Body {
    Block(&'static [Stmt]),
    Expr(&'static Expr),
}

impl Analysis {
    fn stmts(&mut self, s: &'static [Stmt]) -> Result<(), Reject> {
        s.iter().try_for_each(|s| self.stmt(s))
    }

    fn stmt(&mut self, s: &'static Stmt) -> Result<(), Reject> {
        use StmtKind::*;
        match &s.kind {
            Expr(e) | Return(Some(e)) => self.expr(e),
            Return(None) | Nop | InlineHtml(_) => Ok(()),
            Global(_) => Err("global"),
            Echo(es) => es.iter().try_for_each(|e| self.expr(e)),
            If { cond, then, elseifs, else_ } => {
                self.expr(cond)?;
                self.stmts(then)?;
                for (c, b) in elseifs {
                    self.expr(c)?;
                    self.stmts(b)?;
                }
                else_.as_deref().map_or(Ok(()), |b| self.stmts(b))
            }
            While { cond, body } | DoWhile { body, cond } => {
                self.expr(cond)?;
                self.stmts(body)
            }
            For { init, cond, step, body } => {
                init.iter().chain(cond).chain(step).try_for_each(|e| self.expr(e))?;
                self.stmts(body)
            }
            Foreach { subject, key, value, by_ref, body } => {
                if *by_ref {
                    return Err("foreach by reference");
                }
                self.expr(subject)?;
                if let Some(k) = key {
                    self.target(k)?;
                }
                self.target(value)?;
                self.stmts(body)
            }
            Switch { subject, cases } => {
                self.expr(subject)?;
                for c in cases {
                    if let Some(t) = &c.test {
                        self.expr(t)?;
                    }
                    self.stmts(&c.body)?;
                }
                Ok(())
            }
            Break(l) | Continue(l) => match l.as_ref().map(|e| &e.kind) {
                None => Ok(()),
                Some(ExprKind::Int(n)) if *n >= 1 => Ok(()),
                _ => Err("dynamic break level"),
            },
            Block(b) => self.stmts(b),
            Unset(es) => es.iter().try_for_each(|e| match &e.kind {
                ExprKind::Var(n) if n != "this" => {
                    self.locals.slot(n);
                    Ok(())
                }
                ExprKind::Index { base, index: Some(i) } => {
                    self.place_base(base)?;
                    self.expr(i)
                }
                ExprKind::Prop { object, name, .. } => {
                    self.expr(object)?;
                    self.member(name)
                }
                _ => Err("unset target"),
            }),
            Try { body, catches, finally } => {
                self.stmts(body)?;
                for c in catches {
                    if let Some(v) = &c.var {
                        self.locals.slot(&v.name);
                    }
                    self.stmts(&c.body)?;
                }
                if let Some(fin) = finally {
                    if !finally_ok(fin, 0) {
                        return Err("return / break out of finally");
                    }
                    self.stmts(fin)?;
                }
                Ok(())
            }
            StaticVar(_) => Err("static variable"),
            // At file top level: functions are hoisted (no-op), classes and
            // constants are declared through helpers.
            Function(_) | Class(_) if self.top_level => Ok(()),
            Const(items) if self.top_level => items.iter().try_for_each(|(_, e)| self.expr(e)),
            Use { .. } | Declare { body: None, .. } if self.top_level => Ok(()),
            Function(_) | Class(_) | Const(_) => Err("nested declaration"),
            Namespace { .. } | Use { .. } | Declare { .. } => Err("top-level statement"),
            Label(_) | Goto(_) => Err("goto"),
        }
    }

    /// Assignment target in foreach / list().
    fn target(&mut self, e: &'static Expr) -> Result<(), Reject> {
        match &e.kind {
            ExprKind::Var(n) if n != "this" => {
                self.locals.slot(n);
                Ok(())
            }
            _ => Err("complex foreach target"),
        }
    }

    /// Base of an index write: must bottom out in a local or a property.
    fn place_base(&mut self, e: &'static Expr) -> Result<(), Reject> {
        match &e.kind {
            ExprKind::Var(n) if n != "this" => {
                self.locals.slot(n);
                Ok(())
            }
            ExprKind::Index { base, index } => {
                self.place_base(base)?;
                index.as_deref().map_or(Ok(()), |i| self.expr(i))
            }
            ExprKind::Prop { object, name, nullsafe: false } => {
                self.expr(object)?;
                self.member(name)
            }
            _ => Err("write target"),
        }
    }

    fn member(&mut self, m: &'static MemberName) -> Result<(), Reject> {
        match m {
            MemberName::Ident(_) => Ok(()),
            MemberName::Dynamic(e) => self.expr(e),
        }
    }

    fn class_ref(&mut self, c: &'static ClassRef) -> Result<(), Reject> {
        match c {
            ClassRef::Name(_) => Ok(()),
            _ => Err("dynamic class reference"),
        }
    }

    fn args(&mut self, args: &'static [ast::Arg]) -> Result<(), Reject> {
        args.iter().try_for_each(|a| self.expr(&a.value))
    }

    fn expr(&mut self, e: &'static Expr) -> Result<(), Reject> {
        use ExprKind::*;
        if crate::value::stack_low() {
            return Err("expression nested too deeply");
        }
        match &e.kind {
            Int(_) | Float(_) | Str(_) | Bytes(_) | Bool(_) | Null | Name(_) | MagicConst(_) => Ok(()),
            Var(n) if n == "this" => {
                self.uses_this = true;
                Ok(())
            }
            Var(n) if n == "GLOBALS" => Err("$GLOBALS"),
            VarVar(_) | GlobalVar(_) => Err("variable variable / $GLOBALS"),
            Unary { op: ast::UnOp::Silence, .. } => Err("@ operator"),
            Var(n) => {
                self.locals.slot(n);
                Ok(())
            }
            Interp(parts) => parts.iter().try_for_each(|p| match p {
                ast::InterpPart::Lit(_) => Ok(()),
                ast::InterpPart::Expr(e) => self.expr(e),
            }),
            Array(items) => {
                for it in items {
                    let Some(it) = it else { return Err("array hole") };
                    if it.by_ref {
                        return Err("array item by reference");
                    }
                    if let Some(k) = &it.key {
                        self.expr(k)?;
                    }
                    self.expr(&it.value)?;
                }
                Ok(())
            }
            Assign { target, value } => {
                match &target.kind {
                    Var(n) if n != "this" => {
                        self.locals.slot(n);
                    }
                    Index { .. } | Prop { nullsafe: false, .. } => self.place_base(target)?,
                    Array(items) => {
                        // List destructuring into locals.
                        for it in items.iter().flatten() {
                            if it.by_ref || it.spread {
                                return Err("list() by reference");
                            }
                            if let Some(k) = &it.key {
                                self.expr(k)?;
                            }
                            self.target(&it.value)?;
                        }
                    }
                    StaticProp { .. } => return Err("static property write"),
                    _ => return Err("assignment target"),
                }
                self.expr(value)
            }
            AssignRef { .. } => Err("assignment by reference"),
            AssignOp { target, value, .. } => {
                match &target.kind {
                    Var(n) if n != "this" => {
                        self.locals.slot(n);
                    }
                    Index { .. } | Prop { nullsafe: false, .. } => self.place_base(target)?,
                    _ => return Err("compound assignment target"),
                }
                self.expr(value)
            }
            IncDec { expr, .. } => match &expr.kind {
                Var(n) if n != "this" => {
                    self.locals.slot(n);
                    Ok(())
                }
                Index { .. } | Prop { nullsafe: false, .. } => self.place_base(expr),
                _ => Err("++/-- target"),
            },
            Binary { left, right, .. } => {
                self.expr(left)?;
                self.expr(right)
            }
            Unary { expr, .. } | Cast { expr, .. } | Print(expr) | Clone(expr) | Throw(expr) | Empty(expr) => self.expr(expr),
            Ternary { cond, then, else_ } => {
                self.expr(cond)?;
                if let Some(t) = then {
                    self.expr(t)?;
                }
                self.expr(else_)
            }
            Call { callee, args } => {
                match &callee.kind {
                    Name(n) => {
                        let l = n.trim_start_matches('\\').to_ascii_lowercase();
                        if SCOPE_FNS.contains(&l.as_str()) {
                            return Err("scope-reading function");
                        }
                    }
                    _ => self.expr(callee)?,
                }
                self.args(args)
            }
            MethodCall { object, name, args, .. } => {
                self.expr(object)?;
                self.member(name)?;
                self.args(args)
            }
            StaticCall { class, name, args } => {
                self.class_ref(class)?;
                self.member(name)?;
                self.args(args)
            }
            Prop { object, name, .. } => {
                self.expr(object)?;
                self.member(name)
            }
            Index { base, index } => {
                self.expr(base)?;
                index.as_deref().map_or(Err("[] read"), |i| self.expr(i))
            }
            ClassConst { class, .. } | StaticProp { class, .. } => self.class_ref(class),
            New { class, args } => {
                match class {
                    ClassRef::Name(_) => {}
                    _ => return Err("dynamic/anonymous new"),
                }
                self.args(args)
            }
            Closure(c) => {
                if c.uses.iter().any(|u| u.by_ref) {
                    return Err("closure captures by reference");
                }
                for u in &c.uses {
                    self.locals.slot(&u.var.name);
                }
                Ok(())
            }
            ArrowFn(_) => Ok(()),
            Match { subject, arms } => {
                self.expr(subject)?;
                for a in arms {
                    for c in a.conds.iter().flatten() {
                        self.expr(c)?;
                    }
                    self.expr(&a.body)?;
                }
                Ok(())
            }
            Instanceof { expr, class } => {
                self.expr(expr)?;
                self.class_ref(class)
            }
            Isset(es) => es.iter().try_for_each(|e| self.quiet(e)),
            Exit(e) => e.as_deref().map_or(Ok(()), |e| self.expr(e)),
            CallableRef(_) => Err("first-class callable"),
            Include { .. } => Err("include"),
            Yield { .. } | YieldFrom(_) => Err("generator"),
            Error => Err("parse error node"),
        }
    }

    /// Operand of isset()/??: locals, index/prop chains.
    fn quiet(&mut self, e: &'static Expr) -> Result<(), Reject> {
        match &e.kind {
            ExprKind::Var(n) if n == "this" => {
                self.uses_this = true;
                Ok(())
            }
            ExprKind::Var(n) => {
                self.locals.slot(n);
                Ok(())
            }
            ExprKind::Index { base, index: Some(i) } => {
                self.quiet(base)?;
                self.expr(i)
            }
            ExprKind::Prop { object, name, .. } => {
                self.quiet(object)?;
                self.member(name)
            }
            _ => self.expr(e),
        }
    }
}
