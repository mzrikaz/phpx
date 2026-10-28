//! Speculative loops (docs/engine-v2.md, "First version"): a `for` or
//! `while` loop in a function whose float locals can live unboxed gets a
//! second, specialised body. Those locals (the candidates) are Cranelift
//! variables holding f64s; everything else they are computed from is read
//! without side effects and checked to be a number. A failed check writes
//! the candidates back to their slots and continues in the generic code of
//! the same statement, which runs it again from the start: every check of
//! a statement comes before its only side effect (the final store).
//!
//! First version: candidates are assigned only from `+ - * /` over float
//! and int literals, other candidates, locals and `$local->name` reads, and
//! `sqrt()`; the loop holds nothing but such assignments, property stores
//! of such values, other statements that don't mention a candidate, and
//! nested `for`/`while` loops (no break, continue or return).

use std::collections::{HashMap, HashSet};

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, Block, InstBuilder, Value as CValue};
use cranelift_frontend::Variable;
use phpx_syntax::ast::{BinOp, Expr, ExprKind, MemberName, Stmt, StmtKind, UnOp};

use super::{flags, Lower, L, Op};
use crate::value::tag;

/// What a speculated expression produces.
#[derive(Clone, Copy, PartialEq)]
enum Ty {
    /// Certainly a float (given its checks).
    F,
    /// An int or a float (checked; converted where a float is needed).
    N,
    /// Not speculated.
    X,
}

/// One speculated loop.
pub(super) struct Plan {
    /// Candidate local -> (its f64 variable, its "assigned" flag).
    vars: HashMap<u32, (Variable, Variable)>,
    /// Candidates the loop may read before assigning them: loaded (and
    /// checked to be floats) when the loop starts.
    entry: Vec<u32>,
    /// Statements of the loop (itself included): where their generic code
    /// starts, for the fallback.
    entries: HashMap<usize, Block>,
}

impl<'a> Lower<'a> {
    /// A plan for loop `s`, or None when it isn't worth or able to speculate.
    pub(super) fn spec_plan(&mut self, s: &'static Stmt) -> Option<Plan> {
        // (Inside a try a catch block would read the slots while the values
        // are still in registers.)
        if self.top_ptrs.is_some() || self.spec_on || !self.handlers.is_empty() || std::env::var_os("PHPX_NO_SPECULATE").is_some() {
            return None;
        }
        let body = loop_body(s)?;
        if !region_ok(body) {
            return None;
        }
        // Candidates: locals assigned in the loop, all assignments of them
        // speculable as floats (to a fixpoint: dropping one can drop others).
        let mut assigns: HashMap<&'static str, Vec<(Option<BinOp>, &'static Expr)>> = HashMap::new();
        collect_assigns(body, &mut assigns);
        // Start from locals only ever given arithmetic, never used as an
        // object (so removing one can't make another speculable again).
        let dbg = format!("{body:?}");
        let mut cands: HashSet<&'static str> = assigns
            .iter()
            .filter(|(n, rhs)| {
                **n != "this"
                    && self.locals.index.contains_key(*n)
                    && !dbg.contains(&format!("object: Expr {{ kind: Var(\"{n}\")"))
                    && rhs.iter().all(|(_, r)| arith_shape(r))
            })
            .map(|(n, _)| *n)
            .collect();
        loop {
            let before = cands.len();
            let snapshot = cands.clone();
            cands.retain(|n| {
                assigns[n].iter().all(|(op, rhs)| {
                    let r = self.ty(rhs, &snapshot);
                    match op {
                        None => r == Ty::F,
                        Some(_) => r != Ty::X,
                    }
                })
            });
            // Mentioned anywhere the specialised code doesn't handle them.
            let mut bad = HashSet::new();
            self.uses_outside(body, &cands, &mut bad);
            if let Some(StmtKind::For { init, cond, step, .. }) = Some(&s.kind) {
                for e in init.iter().chain(cond).chain(step) {
                    mentions(e, &cands, &mut bad);
                }
            }
            if let StmtKind::While { cond, .. } = &s.kind {
                mentions(cond, &cands, &mut bad);
            }
            cands.retain(|n| !bad.contains(n));
            if cands.len() == before {
                break;
            }
        }
        if cands.is_empty() {
            return None;
        }
        let mut entry = HashSet::new();
        let mut assigned = HashSet::new();
        read_before_assign(body, &cands, &mut assigned, &mut entry);
        let mut vars = HashMap::new();
        for n in &cands {
            let i = self.locals.index[n];
            let v = self.b.declare_var(types::F64);
            let d = self.b.declare_var(types::I8);
            vars.insert(i, (v, d));
        }
        let mut entries = HashMap::new();
        let mut all = vec![];
        region_stmts(s, &mut all);
        for st in all {
            entries.insert(st as *const Stmt as usize, self.b.create_block());
        }
        if std::env::var_os("PHPX_SPEC_LOG").is_some() {
            eprintln!("[spec] loop at {}: candidates {:?}, loaded at entry {:?}", s.span.start, cands, entry);
        }
        let entry = entry.iter().map(|n| self.locals.index[n]).collect();
        Some(Plan { vars, entry, entries })
    }

    /// Candidates mentioned where the specialised code can't keep them:
    /// statements it lowers generically (those read the slots), and any
    /// expression shape `ty` doesn't speculate.
    fn uses_outside(&self, body: &[Stmt], cands: &HashSet<&'static str>, bad: &mut HashSet<&'static str>) {
        for s in body {
            match &s.kind {
                StmtKind::Expr(e) => match &e.kind {
                    ExprKind::Assign { target, value } | ExprKind::AssignOp { target, value, .. } => {
                        let handled_op = !matches!(&e.kind, ExprKind::AssignOp { op, .. } if !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div));
                        let r = self.ty(value, cands);
                        match &target.kind {
                            ExprKind::Var(n) if cands.contains(n.as_str()) && handled_op && r != Ty::X => leaf_uses(value, cands, bad),
                            // A property store the specialised code makes (a float value).
                            ExprKind::Prop { object, name: MemberName::Ident(_), nullsafe: false }
                                if handled_op && r == Ty::F && matches!(&object.kind, ExprKind::Var(o) if !cands.contains(o.as_str()) && self.locals.index.contains_key(o.as_str())) =>
                            {
                                leaf_uses(value, cands, bad)
                            }
                            // Generic code that only reads candidates (they are
                            // written back first): `$r[$i] = $s`.
                            _ if !matches!(&target.kind, ExprKind::Var(n) if cands.contains(n.as_str())) && arith_shape(value) => {
                                mentions(target, cands, bad);
                                leaf_uses(value, cands, bad);
                            }
                            _ => mentions(e, cands, bad),
                        }
                    }
                    _ => mentions(e, cands, bad),
                },
                StmtKind::For { body, .. } | StmtKind::While { body, .. } => self.uses_outside(body, cands, bad),
                _ => {}
            }
        }
    }

    /// A local that isn't a candidate (read as a Value from its slot).
    fn plain_local(&self, n: &str, cands: &HashSet<&'static str>) -> bool {
        n != "this" && !cands.contains(n) && self.locals.index.contains_key(n)
    }

    /// The static type of a speculated expression, given the candidates.
    fn ty(&self, e: &Expr, cands: &HashSet<&'static str>) -> Ty {
        match &e.kind {
            ExprKind::Float(_) => Ty::F,
            ExprKind::Int(_) => Ty::N,
            ExprKind::Var(n) if cands.contains(n.as_str()) => Ty::F,
            ExprKind::Var(n) if self.locals.index.contains_key(n.as_str()) => Ty::N,
            // Speculated to hold a float (checked strictly when read).
            ExprKind::Prop { object, name: MemberName::Ident(_), nullsafe: false } => match &object.kind {
                ExprKind::Var(o) if !cands.contains(o.as_str()) && self.locals.index.contains_key(o.as_str()) => Ty::F,
                _ => Ty::X,
            },
            // `$local[$k]` (a literal or local key): speculated to be a float
            // in a packed array.
            ExprKind::Index { base, index: Some(k) } => match (&base.kind, &k.kind) {
                (ExprKind::Var(b), ExprKind::Int(_)) if self.plain_local(b, cands) => Ty::F,
                (ExprKind::Var(b), ExprKind::Var(kv)) if self.plain_local(b, cands) && self.plain_local(kv, cands) => Ty::F,
                _ => Ty::X,
            },
            ExprKind::Unary { op: UnOp::Neg, expr } => self.ty(expr, cands),
            ExprKind::Binary { op: BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, left, right } => {
                match (self.ty(left, cands), self.ty(right, cands)) {
                    (Ty::X, _) | (_, Ty::X) => Ty::X,
                    (Ty::F, _) | (_, Ty::F) => Ty::F,
                    // int op int may stay an int: not speculated.
                    _ => Ty::X,
                }
            }
            ExprKind::Call { callee, args } if is_sqrt(callee, args) => match self.ty(&args[0].value, cands) {
                Ty::X => Ty::X,
                _ => Ty::F,
            },
            _ => Ty::X,
        }
    }

    /// Lower loop `s` with its specialised version in front: checked entry,
    /// the specialised loop, then the generic loop the fallbacks lead into.
    pub(super) fn spec_loop_stmt(&mut self, s: &'static Stmt, plan: Plan) -> L<()> {
        let after = self.b.create_block();
        let generic = plan.entries[&(s as *const Stmt as usize)];
        // Entry: the candidates read before assignment must hold floats.
        for &i in &plan.entry {
            let a = self.local_addr(i);
            let tg = self.load_tag(a);
            let ok = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::FLOAT as i64);
            let next = self.b.create_block();
            self.b.ins().brif(ok, next, &[], generic, &[]);
            self.b.switch_to_block(next);
        }
        let zero = self.b.ins().f64const(0.0);
        let no = self.b.ins().iconst(types::I8, 0);
        let yes = self.b.ins().iconst(types::I8, 1);
        for (&i, &(v, d)) in &plan.vars {
            if plan.entry.contains(&i) {
                let a = self.local_addr(i);
                let x = self.b.ins().load(types::F64, flags(), a, tag::PAYLOAD);
                self.b.def_var(v, x);
                self.b.def_var(d, yes);
            } else {
                self.b.def_var(v, zero);
                self.b.def_var(d, no);
            }
        }
        let saved = std::mem::replace(&mut self.spec, Some(plan));
        self.spec_on = true;
        let r = self.spec_loop(s);
        self.spec_on = false;
        let plan = std::mem::replace(&mut self.spec, saved).expect("plan");
        r?;
        // Normal exit: candidates back to their slots, then on after the loop.
        self.spill(&plan);
        self.b.ins().jump(after, &[]);
        // The generic loop (fallbacks enter its statements).
        self.spec_entries.extend(plan.entries);
        self.b.switch_to_block(generic);
        self.stmt_body(s)?;
        if !self.terminated {
            self.b.ins().jump(after, &[]);
        }
        self.terminated = false;
        // Entered mid-run (on-stack replacement) at one of these loops: the
        // specialised entry, which falls back to the generic one.
        if let Some((entry, gen_entry)) = self.spec_osr.take() {
            if let Some(g) = self.osr_entry {
                self.b.switch_to_block(gen_entry);
                self.b.ins().jump(g, &[]);
                self.osr_entry = Some(entry);
            }
        }
        self.b.switch_to_block(after);
        Ok(())
    }

    /// The on-stack replacement entry of speculated loop `s` (its head
    /// comes next): candidates the loop reads must hold floats, the others
    /// are loaded when they do; else the generic entry.
    fn spec_osr_entry(&mut self, s: &'static Stmt, head: Block) {
        if self.no_osr != 0 || self.osr_target != Some(s as *const Stmt as usize) {
            return;
        }
        let cur = self.b.current_block().unwrap();
        let (entry, gen_entry) = (self.b.create_block(), self.b.create_block());
        self.b.switch_to_block(entry);
        let cands = self.cand_names();
        let mut read = HashSet::new();
        if let Some(body) = loop_body(s) {
            for st in body {
                if let StmtKind::Expr(e) = &st.kind {
                    mentions(e, &cands, &mut read);
                }
            }
        }
        let mut vars: Vec<(u32, (Variable, Variable))> = self.spec.as_ref().unwrap().vars.iter().map(|(&i, &vd)| (i, vd)).collect();
        vars.sort_by_key(|e| e.0);
        let names: HashMap<u32, &'static str> = self.locals.index.iter().map(|(n, i)| (*i, *n)).collect();
        for (i, (v, d)) in vars {
            let a = self.local_addr(i);
            let tg = self.load_tag(a);
            let is_f = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::FLOAT as i64);
            let x = self.b.ins().load(types::F64, flags(), a, tag::PAYLOAD);
            let zero = self.b.ins().f64const(0.0);
            let x = self.b.ins().select(is_f, x, zero);
            self.b.def_var(v, x);
            self.b.def_var(d, is_f);
            if names.get(&i).is_some_and(|n| read.contains(n)) {
                let next = self.b.create_block();
                self.b.ins().brif(is_f, next, &[], gen_entry, &[]);
                self.b.switch_to_block(next);
            }
        }
        self.b.ins().jump(head, &[]);
        self.b.switch_to_block(cur);
        self.spec_osr = Some((entry, gen_entry));
    }

    /// Candidates assigned so far back into their slots (as floats).
    fn spill(&mut self, plan: &Plan) {
        let mut vars: Vec<_> = plan.vars.iter().map(|(&i, &vd)| (i, vd)).collect();
        vars.sort_by_key(|e| e.0);
        for (i, (v, d)) in vars {
            let set = self.b.use_var(d);
            let (store, next) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(set, store, &[], next, &[]);
            self.b.switch_to_block(store);
            let x = self.b.use_var(v);
            self.assign_local(i, Op::Float(x));
            self.b.ins().jump(next, &[]);
            self.b.switch_to_block(next);
        }
    }

    /// The specialised version of loop `s`.
    fn spec_loop(&mut self, s: &'static Stmt) -> L<()> {
        let (head, bodyb, stepb, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
        match &s.kind {
            StmtKind::For { init, cond, step, body } => {
                for e in init {
                    self.discard(e)?;
                }
                self.spec_osr_entry(s, head);
                self.b.ins().jump(head, &[]);
                self.b.switch_to_block(head);
                if cond.is_empty() {
                    self.b.ins().jump(bodyb, &[]);
                } else {
                    for e in &cond[..cond.len() - 1] {
                        self.discard(e)?;
                    }
                    let op = self.expr(cond.last().unwrap())?;
                    let t = self.truthy(op);
                    self.b.ins().brif(t, bodyb, &[], exit, &[]);
                }
                self.b.switch_to_block(bodyb);
                self.spec_body(body, stepb, exit)?;
                self.b.switch_to_block(stepb);
                for e in step {
                    self.discard(e)?;
                }
                self.b.ins().jump(head, &[]);
            }
            StmtKind::While { cond, body } => {
                self.spec_osr_entry(s, head);
                self.b.ins().jump(head, &[]);
                self.b.switch_to_block(head);
                let op = self.expr(cond)?;
                let t = self.truthy(op);
                self.b.ins().brif(t, bodyb, &[], exit, &[]);
                self.b.switch_to_block(bodyb);
                self.spec_body(body, head, exit)?;
                self.b.switch_to_block(stepb);
                self.b.ins().jump(head, &[]);
            }
            _ => unreachable!(),
        }
        self.b.switch_to_block(exit);
        Ok(())
    }

    fn spec_body(&mut self, body: &'static [Stmt], cont: Block, exit: Block) -> L<()> {
        self.loops.push((cont, exit, self.handlers.len()));
        self.terminated = false;
        self.boundary_check();
        for st in body {
            self.spec_stmt(st)?;
        }
        self.b.ins().jump(cont, &[]);
        self.terminated = false;
        self.loops.pop();
        Ok(())
    }

    fn spec_stmt(&mut self, s: &'static Stmt) -> L<()> {
        self.set_pos(s.span.start);
        self.stmt_start = s.span.start;
        let fb = self.spec.as_ref().expect("plan").entries[&(s as *const Stmt as usize)];
        match &s.kind {
            StmtKind::For { .. } | StmtKind::While { .. } => return self.spec_loop(s),
            StmtKind::Expr(e) => match &e.kind {
                ExprKind::Assign { target, value } | ExprKind::AssignOp { target, value, .. } => {
                    let op = match &e.kind {
                        ExprKind::AssignOp { op, .. } => Some(*op),
                        _ => None,
                    };
                    if let ExprKind::Var(n) = &target.kind {
                        if let Some(&i) = self.locals.index.get(n.as_str()) {
                            if let Some(&(v, d)) = self.spec.as_ref().unwrap().vars.get(&i) {
                                let fail = self.fail_block(fb);
                                let mut r = self.fexpr(value, fail)?;
                                if let Some(op) = op {
                                    let cur = self.b.use_var(v);
                                    r = self.fbin(op, cur, r, fail);
                                }
                                self.b.def_var(v, r);
                                let yes = self.b.ins().iconst(types::I8, 1);
                                self.b.def_var(d, yes);
                                return Ok(());
                            }
                        }
                    }
                    if let ExprKind::Prop { object, name: MemberName::Ident(name), nullsafe: false } = &target.kind {
                        if let ExprKind::Var(o) = &object.kind {
                            let cands = self.cand_names();
                            if self.locals.index.contains_key(o.as_str()) && !cands.contains(o.as_str()) && self.ty(value, &cands) == Ty::F {
                                let fail = self.fail_block(fb);
                                let mut r = self.fexpr(value, fail)?;
                                let oi = self.locals.index[o.as_str()];
                                if let Some(op) = op {
                                    let oa = self.local_addr(oi);
                                    let cur = self.peek_prop(oa, &name.name, fail, true)?;
                                    r = self.fbin(op, cur, r, fail);
                                }
                                let oa = self.local_addr(oi);
                                let res = self.prop_store(oa, &name.name, Op::Float(r))?;
                                self.release(res);
                                return Ok(());
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
        // Anything else only reads candidates, if it mentions them at all:
        // its generic code, in place, after writing them back.
        let cands = self.cand_names();
        let mut seen = HashSet::new();
        if let StmtKind::Expr(e) = &s.kind {
            mentions(e, &cands, &mut seen);
        }
        if !seen.is_empty() {
            let plan = self.spec.take().unwrap();
            self.spill(&plan);
            self.spec = Some(plan);
        }
        self.stmt(s)
    }

    fn cand_names(&self) -> HashSet<&'static str> {
        let plan = self.spec.as_ref().unwrap();
        self.locals.index.iter().filter(|(_, i)| plan.vars.contains_key(i)).map(|(n, _)| *n).collect()
    }

    /// A cold block that writes the candidates back and continues in the
    /// generic code at `fb`.
    fn fail_block(&mut self, fb: Block) -> Block {
        let cur = self.b.current_block().unwrap();
        let fail = self.b.create_block();
        self.b.set_cold_block(fail);
        self.b.switch_to_block(fail);
        let plan = self.spec.take().unwrap();
        self.spill(&plan);
        self.spec = Some(plan);
        self.b.ins().jump(fb, &[]);
        self.b.switch_to_block(cur);
        fail
    }

    /// Evaluate a speculated expression (`ty` not X) as an f64; checks
    /// branch to `fail`.
    fn fexpr(&mut self, e: &'static Expr, fail: Block) -> L<CValue> {
        Ok(match &e.kind {
            ExprKind::Float(x) => self.b.ins().f64const(*x),
            ExprKind::Int(n) => self.b.ins().f64const(*n as f64),
            ExprKind::Var(n) => {
                let i = self.locals.index[n.as_str()];
                if let Some(&(v, _)) = self.spec.as_ref().unwrap().vars.get(&i) {
                    self.b.use_var(v)
                } else {
                    let a = self.local_addr(i);
                    self.num_at(a, fail, false)
                }
            }
            ExprKind::Prop { object, name: MemberName::Ident(name), .. } => {
                let ExprKind::Var(o) = &object.kind else { unreachable!() };
                let oa = self.local_addr(self.locals.index[o.as_str()]);
                self.peek_prop(oa, &name.name, fail, true)?
            }
            ExprKind::Index { base, index: Some(k) } => {
                let ExprKind::Var(b) = &base.kind else { unreachable!() };
                let ba = self.local_addr(self.locals.index[b.as_str()]);
                let ka = match &k.kind {
                    ExprKind::Var(kv) => self.local_addr(self.locals.index[kv.as_str()]),
                    ExprKind::Int(n) => {
                        let c = self.const_value(crate::value::Value::Int(*n));
                        c
                    }
                    _ => unreachable!(),
                };
                let p = self.packed_elem(ba, ka, fail);
                self.num_at(p, fail, true)
            }
            ExprKind::Unary { op: UnOp::Neg, expr } => {
                let x = self.fexpr(expr, fail)?;
                self.b.ins().fneg(x)
            }
            ExprKind::Binary { op, left, right } => {
                let x = self.fexpr(left, fail)?;
                let y = self.fexpr(right, fail)?;
                self.fbin(*op, x, y, fail)
            }
            ExprKind::Call { args, .. } => {
                // sqrt(), unless a namespaced function shadows the built-in.
                let flag = self.iconst(crate::value::builtin_shadowed() as *const _ as i64);
                let shadowed = self.b.ins().load(types::I8, flags(), flag, 0);
                let ok = self.b.create_block();
                self.b.ins().brif(shadowed, fail, &[], ok, &[]);
                self.b.switch_to_block(ok);
                let x = self.fexpr(&args[0].value, fail)?;
                self.b.ins().sqrt(x)
            }
            _ => unreachable!("not speculated"),
        })
    }

    fn fbin(&mut self, op: BinOp, x: CValue, y: CValue, fail: Block) -> CValue {
        match op {
            BinOp::Add => self.b.ins().fadd(x, y),
            BinOp::Sub => self.b.ins().fsub(x, y),
            BinOp::Mul => self.b.ins().fmul(x, y),
            _ => {
                // Division by zero throws: the generic code does that.
                let z = self.b.ins().f64const(0.0);
                let nz = self.b.ins().fcmp(FloatCC::NotEqual, y, z);
                let ok = self.b.create_block();
                self.b.ins().brif(nz, ok, &[], fail, &[]);
                self.b.switch_to_block(ok);
                self.b.ins().fdiv(x, y)
            }
        }
    }

    /// The number in the Value at `a` as an f64 (an int converted; only a
    /// float with `float_only`); anything else branches to `fail`.
    fn num_at(&mut self, a: CValue, fail: Block, float_only: bool) -> CValue {
        let tg = self.load_tag(a);
        let is_f = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::FLOAT as i64);
        let ok = if float_only {
            is_f
        } else {
            let is_i = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::INT as i64);
            self.b.ins().bor(is_f, is_i)
        };
        let go = self.b.create_block();
        self.b.ins().brif(ok, go, &[], fail, &[]);
        self.b.switch_to_block(go);
        let fv = self.b.ins().load(types::F64, flags(), a, tag::PAYLOAD);
        if float_only {
            return fv;
        }
        let raw = self.b.ins().load(types::I64, flags(), a, tag::PAYLOAD);
        let iv = self.b.ins().fcvt_from_sint(types::F64, raw);
        self.b.ins().select(is_f, fv, iv)
    }

    /// `$obj->name` read without side effects: the inline cache, else the
    /// peek helper; neither found (or not a number) means `fail`.
    fn peek_prop(&mut self, oa: CValue, name: &'static str, fail: Block, float_only: bool) -> L<CValue> {
        let site = Box::new(crate::compiler::rt::PropSite::new(name));
        let sp = &*site as *const crate::compiler::rt::PropSite as i64;
        self.psites.push(site);
        let (miss, merge) = (self.b.create_block(), self.b.create_block());
        self.b.append_block_param(merge, types::F64);
        self.b.set_cold_block(miss);
        match crate::compiler::layout() {
            Some(lay) => {
                // Hit: the number straight from the property's slot.
                let site_v = self.iconst(sp);
                let props = self.prop_guard(lay, oa, site_v, crate::compiler::rt::PROPSITE_SHAPE, crate::compiler::rt::PROPSITE_SCOPE, miss);
                let p = self.prop_slot(lay, props, site_v, crate::compiler::rt::PROPSITE_SLOT, miss);
                let x = self.num_at(p, fail, float_only);
                self.b.ins().jump(merge, &[x.into()]);
            }
            None => {
                self.b.ins().jump(miss, &[]);
            }
        }
        // Miss: the helper's copy (owned) in a temp; released unless a number.
        self.b.switch_to_block(miss);
        let t = self.alloc_temp();
        let site_v = self.iconst(sp);
        let ta = self.temp_addr(t);
        let found = self.call("rt_prop_peek", &[self.cx, oa, site_v, ta]).unwrap();
        let (got, drop_fail) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(found, got, &[], fail, &[]);
        self.b.switch_to_block(drop_fail);
        let ta = self.temp_addr(t);
        self.clear(ta);
        self.b.ins().jump(fail, &[]);
        self.b.switch_to_block(got);
        let ta = self.temp_addr(t);
        let x = self.num_at(ta, drop_fail, float_only);
        // (A number holds no reference: the temp is just marked empty.)
        let ta = self.temp_addr(t);
        self.store_tag(ta, tag::NULL);
        self.b.ins().jump(merge, &[x.into()]);
        self.free.push(t);
        self.b.switch_to_block(merge);
        Ok(self.b.block_params(merge)[0])
    }
}

/// The body of a `for`/`while` statement.
fn loop_body(s: &Stmt) -> Option<&'static [Stmt]> {
    match &s.kind {
        StmtKind::For { body, .. } | StmtKind::While { body, .. } => Some(unsafe { std::mem::transmute::<&[Stmt], &'static [Stmt]>(body.as_slice()) }),
        _ => None,
    }
}

/// Only expression statements and nested loops (no break, continue,
/// return, echo or anything with a block of its own).
fn region_ok(body: &[Stmt]) -> bool {
    body.iter().all(|s| match &s.kind {
        // (Dynamic scope could reach a candidate without naming it.)
        StmtKind::Expr(e) => {
            let d = format!("{e:?}");
            !["VarVar(", "Include {", "Eval(", "\"compact\"", "\"extract\"", "\"get_defined_vars\"", "\"parse_str\""].iter().any(|k| d.contains(k))
        }
        StmtKind::For { body, .. } | StmtKind::While { body, .. } => region_ok(body),
        _ => false,
    })
}

fn region_stmts(s: &'static Stmt, out: &mut Vec<&'static Stmt>) {
    out.push(s);
    if let Some(body) = loop_body(s) {
        for st in body {
            region_stmts(st, out);
        }
    }
}

fn collect_assigns(body: &'static [Stmt], out: &mut HashMap<&'static str, Vec<(Option<BinOp>, &'static Expr)>>) {
    for s in body {
        match &s.kind {
            StmtKind::Expr(e) => match &e.kind {
                ExprKind::Assign { target, value } => {
                    if let ExprKind::Var(n) = &target.kind {
                        out.entry(n.as_str()).or_default().push((None, value));
                    }
                }
                ExprKind::AssignOp { op: op @ (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div), target, value } => {
                    if let ExprKind::Var(n) = &target.kind {
                        out.entry(n.as_str()).or_default().push((Some(*op), value));
                    }
                }
                _ => {}
            },
            StmtKind::For { body, .. } | StmtKind::While { body, .. } => collect_assigns(body, out),
            _ => {}
        }
    }
}

/// In a speculated right-hand side: candidates are fine where `ty` reads
/// them; anything mentioning one elsewhere is not.
fn leaf_uses(e: &Expr, cands: &HashSet<&'static str>, bad: &mut HashSet<&'static str>) {
    match &e.kind {
        ExprKind::Var(_) | ExprKind::Float(_) | ExprKind::Int(_) => {}
        ExprKind::Unary { op: UnOp::Neg, expr } => leaf_uses(expr, cands, bad),
        ExprKind::Binary { op: BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, left, right } => {
            leaf_uses(left, cands, bad);
            leaf_uses(right, cands, bad);
        }
        ExprKind::Call { callee, args } if is_sqrt(callee, args) => leaf_uses(&args[0].value, cands, bad),
        ExprKind::Prop { object, .. } if matches!(&object.kind, ExprKind::Var(o) if !cands.contains(o.as_str())) => {}
        ExprKind::Index { base, index: Some(k) } => {
            mentions(base, cands, bad);
            mentions(k, cands, bad);
        }
        _ => mentions(e, cands, bad),
    }
}

/// Every candidate named anywhere in `e` (closures included).
fn mentions(e: &Expr, cands: &HashSet<&'static str>, bad: &mut HashSet<&'static str>) {
    let dbg = format!("{e:?}");
    for c in cands {
        if dbg.contains(&format!("Var(\"{c}\")")) || dbg.contains(&format!("name: \"{c}\"")) {
            bad.insert(c);
        }
    }
}

/// Candidates a loop iteration may read before assigning them.
fn read_before_assign(body: &[Stmt], cands: &HashSet<&'static str>, assigned: &mut HashSet<&'static str>, entry: &mut HashSet<&'static str>) {
    for s in body {
        match &s.kind {
            StmtKind::Expr(e) => {
                let (target, value, compound) = match &e.kind {
                    ExprKind::Assign { target, value } => (Some(target), Some(value), false),
                    ExprKind::AssignOp { target, value, .. } => (Some(target), Some(value), true),
                    _ => (None, None, false),
                };
                if let Some(v) = value {
                    let mut read = HashSet::new();
                    mentions(v, cands, &mut read);
                    if compound {
                        if let Some(ExprKind::Var(n)) = target.map(|t| &t.kind) {
                            if cands.contains(n.as_str()) {
                                read.insert(cands.get(n.as_str()).unwrap());
                            }
                        }
                    }
                    for r in read {
                        if !assigned.contains(r) {
                            entry.insert(r);
                        }
                    }
                }
                if let Some(ExprKind::Var(n)) = target.map(|t| &t.kind) {
                    if let Some(c) = cands.get(n.as_str()) {
                        assigned.insert(c);
                    }
                }
            }
            StmtKind::For { body, .. } | StmtKind::While { body, .. } => {
                // May run zero times: what it assigns doesn't count after it.
                let mut inner = assigned.clone();
                read_before_assign(body, cands, &mut inner, entry);
            }
            _ => {}
        }
    }
}

fn is_sqrt(callee: &Expr, args: &[phpx_syntax::ast::Arg]) -> bool {
    matches!(&callee.kind, ExprKind::Name(n) if n.strip_prefix('\\').unwrap_or(n).eq_ignore_ascii_case("sqrt"))
        && args.len() == 1
        && !args[0].spread
        && args[0].name.is_none()
}

/// The statements of loop `s` (itself included), by address.
pub(super) fn plan_stmts(s: &'static Stmt) -> Vec<usize> {
    let mut all = vec![];
    region_stmts(s, &mut all);
    all.into_iter().map(|s| s as *const Stmt as usize).collect()
}

/// An expression `ty` might speculate (literals, variables, property reads,
/// arithmetic, `sqrt()`), before knowing the candidates.
fn arith_shape(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Float(_) | ExprKind::Int(_) | ExprKind::Var(_) => true,
        ExprKind::Prop { name: MemberName::Ident(_), nullsafe: false, .. } => true,
        ExprKind::Index { base, index: Some(k) } => matches!(base.kind, ExprKind::Var(_)) && matches!(k.kind, ExprKind::Var(_) | ExprKind::Int(_)),
        ExprKind::Unary { op: UnOp::Neg, expr } => arith_shape(expr),
        ExprKind::Binary { op: BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div, left, right } => arith_shape(left) && arith_shape(right),
        ExprKind::Call { callee, args } => is_sqrt(callee, args) && arith_shape(&args[0].value),
        _ => false,
    }
}
