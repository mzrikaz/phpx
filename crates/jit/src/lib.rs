//! Cranelift JIT for type-specialized numeric functions.
//!
//! A hot user function whose body stays within a pure numeric subset (int,
//! float and bool locals, arithmetic, comparisons, control flow, calls to
//! other such functions) is compiled to native code for the argument types
//! it is called with. Anything the native code can't handle exactly like PHP
//! (integer overflow to float, division by zero, deep recursion) makes it
//! return "deopt"; the interpreter then reruns the call. Rerunning is safe
//! because compiled code has no side effects besides local variables.

mod infer;

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, AbiParam, Block, InstBuilder, MemFlagsData, Value as CValue};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};
use phpx_syntax::ast::{BinOp, Expr, ExprKind, Function, Stmt, StmtKind, Type, UnOp};

use infer::{Bail, Callees, Infer, T};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Ty {
    Int,
    Float,
    Bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum JitVal {
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl JitVal {
    pub fn ty(self) -> Ty {
        match self {
            JitVal::Int(_) => Ty::Int,
            JitVal::Float(_) => Ty::Float,
            JitVal::Bool(_) => Ty::Bool,
        }
    }

    fn bits(self) -> i64 {
        match self {
            JitVal::Int(n) => n,
            JitVal::Float(f) => f.to_bits() as i64,
            JitVal::Bool(b) => b as i64,
        }
    }
}

pub enum Outcome {
    Done(JitVal),
    /// Native code bailed out: run the call in the interpreter.
    Deopt,
    /// Not compiled (cold, or outside the subset).
    Skip,
}

/// How the JIT finds the function a name refers to (namespace-aware, so it
/// is the interpreter's job).
pub trait Resolver {
    fn resolve(&self, from: &'static Function, name: &str) -> Option<&'static Function>;
}

type Entry = unsafe extern "C" fn(*const i64, *mut i64) -> i64;

enum Spec {
    InProgress { ret: T, id: Option<FuncId> },
    Ready { id: FuncId, entry: Option<Entry>, ret: Ty },
    Failed,
}

/// Recursion depth past which compiled code defers to the interpreter
/// (which raises PHP's stack-size error properly).
const MAX_DEPTH: i64 = 20_000;
/// Calls before a function is compiled.
const HOT: u32 = 50;
/// Deopts before a specialization is abandoned.
const MAX_DEOPTS: u32 = 20;

pub struct Jit {
    module: JITModule,
    specs: HashMap<(usize, u64), Spec>,
    hot: HashMap<usize, u32>,
    deopts: HashMap<(usize, u64), u32>,
    /// Unique suffix for internal symbol names (one per specialization).
    next_sym: u32,
    /// Functions that failed to compile: skipped from then on, cheaply.
    rejected: std::collections::HashSet<usize>,
    /// Why functions were rejected (for `PHPX_JIT_LOG`).
    log: bool,
}

/// Cache key: function address + argument types packed into a u64 (no
/// allocation per call). Up to 28 parameters.
fn key(f: &'static Function, tys: &[Ty]) -> (usize, u64) {
    let mut packed = tys.len() as u64;
    for (i, t) in tys.iter().take(28).enumerate() {
        let code = match t {
            Ty::Int => 1,
            Ty::Float => 2,
            Ty::Bool => 3,
        };
        packed |= code << (8 + 2 * i);
    }
    (f as *const Function as usize, packed)
}

fn clty(t: Ty) -> types::Type {
    match t {
        Ty::Float => types::F64,
        Ty::Int | Ty::Bool => types::I64,
    }
}

impl Jit {
    pub fn new() -> Option<Jit> {
        let mut flags = settings::builder();
        flags.set("use_colocated_libcalls", "false").ok()?;
        flags.set("is_pic", "false").ok()?;
        flags.set("opt_level", "speed").ok()?;
        let isa = cranelift_native::builder().ok()?.finish(settings::Flags::new(flags)).ok()?;
        let module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));
        Some(Jit {
            module,
            specs: HashMap::new(),
            hot: HashMap::new(),
            deopts: HashMap::new(),
            rejected: Default::default(),
            next_sym: 0,
            log: std::env::var("PHPX_JIT_LOG").is_ok(),
        })
    }

    /// Known not to compile: callers can skip preparing arguments.
    pub fn rejected(&self, f: &'static Function) -> bool {
        self.rejected.contains(&(f as *const Function as usize))
    }

    /// Run `f(args)` natively if possible.
    pub fn call(&mut self, f: &'static Function, args: &[JitVal], r: &dyn Resolver) -> Outcome {
        if args.len() > 28 {
            return Outcome::Skip;
        }
        let mut tys_buf = [Ty::Int; 28];
        for (i, a) in args.iter().enumerate() {
            tys_buf[i] = a.ty();
        }
        let tys = &tys_buf[..args.len()];
        let k = key(f, &tys);
        let (entry, ret) = match self.specs.get(&k) {
            Some(Spec::Ready { entry: Some(e), ret, .. }) => (*e, *ret),
            Some(Spec::Failed) | Some(Spec::InProgress { .. }) => return Outcome::Skip,
            Some(Spec::Ready { entry: None, id, ret }) => {
                // Compiled as a callee; build its entry wrapper now.
                let (id, ret) = (*id, *ret);
                match self.entry_for(id, tys, ret) {
                    Ok(e) => {
                        self.specs.insert(k.clone(), Spec::Ready { id, entry: Some(e), ret });
                        (e, ret)
                    }
                    Err(_) => return Outcome::Skip,
                }
            }
            None => {
                // A function with a loop can be hot in a single call.
                let threshold = if f.body.as_deref().is_some_and(has_loop) { 1 } else { HOT };
                let n = self.hot.entry(k.0).or_default();
                *n += 1;
                if *n < threshold {
                    return Outcome::Skip;
                }
                match self.compile(f, tys, r).and_then(|(id, ret)| Ok((self.entry_for(id, tys, ret)?, ret, id))) {
                    Ok((e, ret, id)) => {
                        if self.log {
                            eprintln!("[jit] {}{:?} compiled -> {ret:?}", f.name.name, tys);
                        }
                        self.specs.insert(k.clone(), Spec::Ready { id, entry: Some(e), ret });
                        (e, ret)
                    }
                    Err(why) => {
                        if self.log {
                            eprintln!("[jit] {}: not compiled: {why}", f.name.name);
                        }
                        // Skip this function cheaply from now on, unless another
                        // type specialization of it did compile.
                        if !self.specs.iter().any(|(sk, s)| sk.0 == k.0 && matches!(s, Spec::Ready { .. })) {
                            self.rejected.insert(k.0);
                        }
                        return Outcome::Skip;
                    }
                }
            }
        };
        let mut bits = [0i64; 28];
        for (i, a) in args.iter().enumerate() {
            bits[i] = a.bits();
        }
        let mut out = 0i64;
        // SAFETY: `entry` was generated for exactly these argument types; it
        // reads `args.len()` slots and writes one result slot.
        let status = unsafe { entry(bits.as_ptr(), &mut out) };
        if status != 0 {
            let d = self.deopts.entry(k.clone()).or_default();
            *d += 1;
            if *d > MAX_DEOPTS {
                self.specs.insert(k, Spec::Failed);
            }
            return Outcome::Deopt;
        }
        Outcome::Done(match ret {
            Ty::Int => JitVal::Int(out),
            Ty::Float => JitVal::Float(f64::from_bits(out as u64)),
            Ty::Bool => JitVal::Bool(out != 0),
        })
    }

    /// Compile `f` for `tys` (and, during inference, any callees). Returns
    /// the internal function id and return type.
    fn compile(&mut self, f: &'static Function, tys: &[Ty], r: &dyn Resolver) -> Result<(FuncId, Ty), Bail> {
        let k = key(f, tys);
        match self.specs.get(&k) {
            Some(Spec::Ready { id, ret, .. }) => return Ok((*id, *ret)),
            Some(Spec::Failed) => return Err("previously rejected"),
            Some(Spec::InProgress { .. }) => return Err("mutual recursion"),
            None => {}
        }
        let res = self.compile_inner(f, tys, r);
        match &res {
            Ok((id, ret)) => {
                self.specs.insert(k, Spec::Ready { id: *id, entry: None, ret: *ret });
            }
            Err(_) => {
                // A declared-but-failed function still needs a body.
                if let Some(Spec::InProgress { id: Some(id), .. }) = self.specs.get(&k) {
                    let id = *id;
                    let _ = self.define_stub(id, tys);
                }
                self.specs.insert(k, Spec::Failed);
            }
        }
        res
    }

    fn compile_inner(&mut self, f: &'static Function, tys: &[Ty], r: &dyn Resolver) -> Result<(FuncId, Ty), Bail> {
        let body = f.body.as_deref().ok_or("no body")?;
        if f.is_generator || f.by_ref || tys.len() != f.params.len() {
            return Err("generator / by-ref / arity");
        }
        let mut params = vec![];
        for (p, t) in f.params.iter().zip(tys) {
            if p.by_ref || p.variadic || p.promote.is_some() {
                return Err("parameter kind");
            }
            if let Some(Type::Named(n)) = &p.ty {
                let ok = matches!((n.to_ascii_lowercase().as_str(), t), ("int", Ty::Int) | ("float", Ty::Float) | ("bool", Ty::Bool));
                if !ok {
                    return Err("parameter type");
                }
            } else if p.ty.is_some() {
                return Err("parameter type");
            }
            params.push((&*p.name.name, *t));
        }
        let k = key(f, tys);
        self.specs.insert(k.clone(), Spec::InProgress { ret: T::Bot, id: None });

        // Infer until the (possibly self-recursive) return type is stable.
        let mut assumed = T::Bot;
        let (vars, ret) = loop {
            let mut cb = CalleeCx { jit: self, r, from: f, self_key: k.clone() };
            let (vars, ret) = Infer::run(&params, body, assumed, &mut cb)?;
            if ret == assumed {
                break (vars, ret);
            }
            assumed = ret;
            self.specs.insert(k.clone(), Spec::InProgress { ret, id: None });
        };
        let T::Ty(ret) = ret else { return Err("return type unknown") };
        if let Some(Type::Named(n)) = &f.ret {
            let ok = matches!((n.to_ascii_lowercase().as_str(), ret), ("int", Ty::Int) | ("float", Ty::Float) | ("bool", Ty::Bool));
            if !ok {
                return Err("declared return type");
            }
        } else if f.ret.is_some() {
            return Err("declared return type");
        }
        let mut var_tys = HashMap::new();
        for (n, t) in vars {
            match t {
                T::Ty(t) => {
                    var_tys.insert(n, t);
                }
                T::Bot => return Err("variable type unknown"),
            }
        }

        let sig = self.internal_sig(tys, ret);
        self.next_sym += 1;
        let id = self
            .module
            .declare_function(&format!("php_{}_{}", f.name.name, self.next_sym), Linkage::Local, &sig)
            .map_err(|_| "declare failed")?;
        self.specs.insert(k.clone(), Spec::InProgress { ret: T::Ty(ret), id: Some(id) });

        let cfg = self.module.target_config();
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fctx = FunctionBuilderContext::new();
        {
            let b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let mut g = Gen {
                b,
                vars: HashMap::new(),
                ret,
                deopt: None,
                loops: vec![],
                depth: None,
                jit: self,
                r,
                from: f,
                terminated: false,
            };
            g.function(&params, &var_tys, body)?;
            let Gen { b, .. } = g;
            b.finalize(cfg);
        }
        self.module.define_function(id, &mut ctx).map_err(|e| {
            if self.log {
                eprintln!("[jit] define failed: {e:?}");
            }
            "codegen failed"
        })?;
        Ok((id, ret))
    }

    fn internal_sig(&self, tys: &[Ty], _ret: Ty) -> cranelift_codegen::ir::Signature {
        let mut sig = self.module.make_signature();
        for t in tys {
            sig.params.push(AbiParam::new(clty(*t)));
        }
        sig.params.push(AbiParam::new(types::I64)); // depth
        sig.returns.push(AbiParam::new(types::I64)); // value bits
        sig.returns.push(AbiParam::new(types::I64)); // status: 0 ok, 1 deopt
        sig
    }

    fn define_stub(&mut self, id: FuncId, tys: &[Ty]) -> Result<(), Bail> {
        let mut ctx = self.module.make_context();
        ctx.func.signature = self.internal_sig(tys, Ty::Int);
        let mut fctx = FunctionBuilderContext::new();
        let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
        let blk = b.create_block();
        b.append_block_params_for_function_params(blk);
        b.switch_to_block(blk);
        let zero = b.ins().iconst(types::I64, 0);
        let one = b.ins().iconst(types::I64, 1);
        b.ins().return_(&[zero, one]);
        b.seal_all_blocks();
        b.finalize(self.module.target_config());
        self.module.define_function(id, &mut ctx).map_err(|_| "stub failed")
    }

    /// `extern "C" fn(args: *const i64, out: *mut i64) -> status` around an
    /// internal function, so Rust can call any arity.
    fn entry_for(&mut self, id: FuncId, tys: &[Ty], _ret: Ty) -> Result<Entry, Bail> {
        let mut sig = self.module.make_signature();
        sig.params.push(AbiParam::new(types::I64));
        sig.params.push(AbiParam::new(types::I64));
        sig.returns.push(AbiParam::new(types::I64));
        let wid = self.module.declare_anonymous_function(&sig).map_err(|_| "declare failed")?;
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fctx = FunctionBuilderContext::new();
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let blk = b.create_block();
            b.append_block_params_for_function_params(blk);
            b.switch_to_block(blk);
            let (args, out) = (b.block_params(blk)[0], b.block_params(blk)[1]);
            let trusted = MemFlagsData::trusted();
            let mut call_args = vec![];
            for (i, t) in tys.iter().enumerate() {
                call_args.push(b.ins().load(clty(*t), trusted, args, (i * 8) as i32));
            }
            call_args.push(b.ins().iconst(types::I64, 0));
            let callee = self.module.declare_func_in_func(id, b.func);
            let call = b.ins().call(callee, &call_args);
            let (v, st) = (b.inst_results(call)[0], b.inst_results(call)[1]);
            b.ins().store(trusted, v, out, 0);
            b.ins().return_(&[st]);
            b.seal_all_blocks();
            b.finalize(self.module.target_config());
        }
        self.module.define_function(wid, &mut ctx).map_err(|_| "entry codegen failed")?;
        self.module.finalize_definitions().map_err(|_| "finalize failed")?;
        let ptr = self.module.get_finalized_function(wid);
        // SAFETY: the wrapper above has exactly this signature.
        Ok(unsafe { std::mem::transmute::<*const u8, Entry>(ptr) })
    }

    /// Callee info for codegen: id, return type.
    fn callee(&self, f: &'static Function, tys: &[Ty]) -> Option<(FuncId, Ty)> {
        match self.specs.get(&key(f, tys))? {
            Spec::Ready { id, ret, .. } => Some((*id, *ret)),
            Spec::InProgress { id: Some(id), ret: T::Ty(ret) } => Some((*id, *ret)),
            _ => None,
        }
    }
}

struct CalleeCx<'a> {
    jit: &'a mut Jit,
    r: &'a dyn Resolver,
    from: &'static Function,
    self_key: (usize, u64),
}

impl Callees for CalleeCx<'_> {
    fn user_call(&mut self, name: &str, args: &[Ty]) -> Result<T, Bail> {
        let f = self.r.resolve(self.from, name).ok_or("unknown function")?;
        let k = key(f, args);
        if k == self.self_key {
            if let Some(Spec::InProgress { ret, .. }) = self.jit.specs.get(&k) {
                return Ok(*ret);
            }
        }
        let (_, ret) = self.jit.compile(f, args, self.r)?;
        Ok(T::Ty(ret))
    }
}

// ---- code generation -----------------------------------------------------

struct Gen<'a> {
    b: FunctionBuilder<'a>,
    vars: HashMap<&'static str, (Variable, Ty)>,
    ret: Ty,
    deopt: Option<Block>,
    /// (continue target, break target) per enclosing loop.
    loops: Vec<(Block, Block)>,
    depth: Option<CValue>,
    jit: &'a mut Jit,
    r: &'a dyn Resolver,
    from: &'static Function,
    /// Current block already ended (return/break/continue): skip the rest.
    terminated: bool,
}

type G<T> = Result<T, Bail>;

impl Gen<'_> {
    fn function(&mut self, params: &[(&'static str, Ty)], var_tys: &HashMap<&'static str, Ty>, body: &'static [Stmt]) -> G<()> {
        let entry = self.b.create_block();
        self.b.append_block_params_for_function_params(entry);
        self.b.switch_to_block(entry);
        let bparams = self.b.block_params(entry).to_vec();
        for (n, t) in var_tys {
            let v = self.b.declare_var(clty(*t));
            self.vars.insert(n, (v, *t));
        }
        // Inference proved reads are definitely assigned; zero-init the rest
        // so every path has a definition for the SSA builder.
        for (n, (v, t)) in &self.vars {
            let init = match params.iter().position(|(p, _)| p == n) {
                Some(i) => bparams[i],
                None if *t == Ty::Float => self.b.ins().f64const(0.0),
                None => self.b.ins().iconst(types::I64, 0),
            };
            self.b.def_var(*v, init);
        }
        let depth = bparams[params.len()];
        let next = self.b.ins().iadd_imm_s(depth, 1);
        self.depth = Some(next);
        let too_deep = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThan, depth, MAX_DEPTH);
        self.deopt_if(too_deep);
        self.stmts(body)?;
        if !self.terminated {
            return Err("function can fall through");
        }
        let deopt = self.deopt_block();
        self.b.switch_to_block(deopt);
        let zero = self.b.ins().iconst(types::I64, 0);
        let one = self.b.ins().iconst(types::I64, 1);
        self.b.ins().return_(&[zero, one]);
        self.b.seal_all_blocks();
        Ok(())
    }

    fn plain(&mut self) -> MemFlagsData {
        MemFlagsData::new()
    }

    fn deopt_block(&mut self) -> Block {
        if let Some(d) = self.deopt {
            return d;
        }
        let d = self.b.create_block();
        self.deopt = Some(d);
        d
    }

    /// Branch to the deopt exit when `cond` (i8) is true.
    fn deopt_if(&mut self, cond: CValue) {
        let d = self.deopt_block();
        let cont = self.b.create_block();
        self.b.ins().brif(cond, d, &[], cont, &[]);
        self.b.switch_to_block(cont);
    }

    /// Start a fresh (unreachable) block after a terminator.
    fn dead(&mut self) {
        self.terminated = true;
    }

    fn stmts(&mut self, stmts: &'static [Stmt]) -> G<()> {
        for s in stmts {
            if self.terminated {
                break;
            }
            self.stmt(s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, s: &'static Stmt) -> G<()> {
        match &s.kind {
            StmtKind::Expr(e) => {
                self.expr(e)?;
            }
            StmtKind::Return(Some(e)) => {
                let (v, t) = self.expr(e)?;
                let v = self.convert(v, t, self.ret)?;
                let bits = if self.ret == Ty::Float { { let mf = self.plain(); self.b.ins().bitcast(types::I64, mf, v) } } else { v };
                let ok = self.b.ins().iconst(types::I64, 0);
                self.b.ins().return_(&[bits, ok]);
                self.dead();
            }
            StmtKind::If { cond, then, elseifs, else_ } => {
                let merge = self.b.create_block();
                let mut any_falls = false;
                let mut conds: Vec<(&'static Expr, &'static [Stmt])> = vec![(cond, then)];
                conds.extend(elseifs.iter().map(|(c, b)| (c, &b[..])));
                for (c, body) in conds {
                    let t = self.truthy_expr(c)?;
                    let yes = self.b.create_block();
                    let no = self.b.create_block();
                    self.b.ins().brif(t, yes, &[], no, &[]);
                    self.b.switch_to_block(yes);
                    self.terminated = false;
                    self.stmts(body)?;
                    if !self.terminated {
                        self.b.ins().jump(merge, &[]);
                        any_falls = true;
                    }
                    self.b.switch_to_block(no);
                    self.terminated = false;
                }
                if let Some(b) = else_ {
                    self.stmts(b)?;
                }
                if !self.terminated {
                    self.b.ins().jump(merge, &[]);
                    any_falls = true;
                }
                self.b.switch_to_block(merge);
                self.terminated = !any_falls;
                if self.terminated {
                    // Unreachable merge block still needs a terminator.
                    self.b.ins().trap(cranelift_codegen::ir::TrapCode::user(1).unwrap());
                }
            }
            StmtKind::While { cond, body } => {
                let (head, bodyb, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().jump(head, &[]);
                self.b.switch_to_block(head);
                let t = self.truthy_expr(cond)?;
                self.b.ins().brif(t, bodyb, &[], exit, &[]);
                self.b.switch_to_block(bodyb);
                self.loop_body(body, head, exit)?;
                self.b.switch_to_block(exit);
            }
            StmtKind::DoWhile { body, cond } => {
                let (bodyb, test, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().jump(bodyb, &[]);
                self.b.switch_to_block(bodyb);
                self.loop_body(body, test, exit)?;
                self.b.switch_to_block(test);
                let t = self.truthy_expr(cond)?;
                self.b.ins().brif(t, bodyb, &[], exit, &[]);
                self.b.switch_to_block(exit);
            }
            StmtKind::For { init, cond, step, body } => {
                for e in init {
                    self.expr(e)?;
                }
                let (head, bodyb, stepb, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().jump(head, &[]);
                self.b.switch_to_block(head);
                if cond.is_empty() {
                    self.b.ins().jump(bodyb, &[]);
                } else {
                    for e in &cond[..cond.len() - 1] {
                        self.expr(e)?;
                    }
                    let t = self.truthy_expr(cond.last().unwrap())?;
                    self.b.ins().brif(t, bodyb, &[], exit, &[]);
                }
                self.b.switch_to_block(bodyb);
                self.loop_body(body, stepb, exit)?;
                self.b.switch_to_block(stepb);
                for e in step {
                    self.expr(e)?;
                }
                self.b.ins().jump(head, &[]);
                self.b.switch_to_block(exit);
            }
            StmtKind::Break(lvl) | StmtKind::Continue(lvl) => {
                let n = match lvl {
                    None => 1,
                    Some(Expr { kind: ExprKind::Int(n), .. }) => *n as usize,
                    _ => return Err("dynamic break level"),
                };
                if n == 0 || n > self.loops.len() {
                    return Err("break level");
                }
                let (cont, brk) = self.loops[self.loops.len() - n];
                let target = if matches!(s.kind, StmtKind::Break(_)) { brk } else { cont };
                self.b.ins().jump(target, &[]);
                self.dead();
            }
            StmtKind::Block(b) => self.stmts(b)?,
            StmtKind::Nop => {}
            _ => return Err("statement not supported"),
        }
        Ok(())
    }

    /// Body of a loop; falls through to `cont`.
    fn loop_body(&mut self, body: &'static [Stmt], cont: Block, exit: Block) -> G<()> {
        self.loops.push((cont, exit));
        self.terminated = false;
        self.stmts(body)?;
        if !self.terminated {
            self.b.ins().jump(cont, &[]);
        }
        self.terminated = false;
        self.loops.pop();
        Ok(())
    }

    fn convert(&mut self, v: CValue, from: Ty, to: Ty) -> G<CValue> {
        Ok(match (from, to) {
            (a, b) if a == b => v,
            (Ty::Int, Ty::Float) => self.b.ins().fcvt_from_sint(types::F64, v),
            _ => return Err("type conversion"),
        })
    }

    fn to_f64(&mut self, v: CValue, t: Ty) -> G<CValue> {
        match t {
            Ty::Float => Ok(v),
            Ty::Int => Ok(self.b.ins().fcvt_from_sint(types::F64, v)),
            Ty::Bool => Err("float op on bool"),
        }
    }

    /// PHP truthiness as an i8 condition.
    fn truthy(&mut self, v: CValue, t: Ty) -> CValue {
        match t {
            Ty::Int | Ty::Bool => self.b.ins().icmp_imm_s(IntCC::NotEqual, v, 0),
            Ty::Float => {
                let z = self.b.ins().f64const(0.0);
                // NAN is truthy: "unordered or not equal".
                self.b.ins().fcmp(FloatCC::NotEqual, v, z)
            }
        }
    }

    fn truthy_expr(&mut self, e: &'static Expr) -> G<CValue> {
        let (v, t) = self.expr(e)?;
        Ok(self.truthy(v, t))
    }

    fn bool_val(&mut self, c: CValue) -> (CValue, Ty) {
        (self.b.ins().uextend(types::I64, c), Ty::Bool)
    }

    fn var(&self, n: &str) -> G<(Variable, Ty)> {
        self.vars.get(n).copied().ok_or("unknown variable")
    }

    fn expr(&mut self, e: &'static Expr) -> G<(CValue, Ty)> {
        use ExprKind::*;
        Ok(match &e.kind {
            Int(n) => (self.b.ins().iconst(types::I64, *n), Ty::Int),
            Float(f) => (self.b.ins().f64const(*f), Ty::Float),
            Bool(b) => (self.b.ins().iconst(types::I64, *b as i64), Ty::Bool),
            Var(n) => {
                let (v, t) = self.var(n)?;
                (self.b.use_var(v), t)
            }
            Assign { target, value } => {
                let ExprKind::Var(n) = &target.kind else { return Err("assign target") };
                let (v, t) = self.expr(value)?;
                let (var, vt) = self.var(n)?;
                let v = self.convert(v, t, vt)?;
                self.b.def_var(var, v);
                (v, vt)
            }
            AssignOp { op, target, value } => {
                let ExprKind::Var(n) = &target.kind else { return Err("assign target") };
                let (var, vt) = self.var(n)?;
                let cur = self.b.use_var(var);
                let (rhs, rt) = self.expr(value)?;
                let (v, t) = self.binop(*op, cur, vt, rhs, rt)?;
                let v = self.convert(v, t, vt)?;
                self.b.def_var(var, v);
                (v, vt)
            }
            IncDec { prefix, inc, expr } => {
                let ExprKind::Var(n) = &expr.kind else { return Err("++ target") };
                let (var, t) = self.var(n)?;
                let old = self.b.use_var(var);
                let new = match t {
                    Ty::Int => {
                        let one = self.b.ins().iconst(types::I64, 1);
                        let (r, of) = if *inc { self.b.ins().sadd_overflow(old, one) } else { self.b.ins().ssub_overflow(old, one) };
                        self.deopt_if(of);
                        r
                    }
                    Ty::Float => {
                        let one = self.b.ins().f64const(if *inc { 1.0 } else { -1.0 });
                        self.b.ins().fadd(old, one)
                    }
                    Ty::Bool => return Err("++ on bool"),
                };
                self.b.def_var(var, new);
                (if *prefix { new } else { old }, t)
            }
            Binary { op: BinOp::And | BinOp::Or, left, right } => {
                let is_and = matches!(e.kind, Binary { op: BinOp::And, .. });
                let a = self.truthy_expr(left)?;
                let rhs = self.b.create_block();
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I64);
                let short = self.b.ins().iconst(types::I64, if is_and { 0 } else { 1 });
                if is_and {
                    self.b.ins().brif(a, rhs, &[], merge, &[short.into()]);
                } else {
                    self.b.ins().brif(a, merge, &[short.into()], rhs, &[]);
                }
                self.b.switch_to_block(rhs);
                let c = self.truthy_expr(right)?;
                let (bv, _) = self.bool_val(c);
                self.b.ins().jump(merge, &[bv.into()]);
                self.b.switch_to_block(merge);
                (self.b.block_params(merge)[0], Ty::Bool)
            }
            Binary { op, left, right } => {
                let (a, at) = self.expr(left)?;
                let (b, bt) = self.expr(right)?;
                self.binop(*op, a, at, b, bt)?
            }
            Unary { op, expr } => {
                let (v, t) = self.expr(expr)?;
                match (op, t) {
                    (UnOp::Not, _) => {
                        let c = self.truthy(v, t);
                        let n = self.b.ins().bxor_imm_s(c, 1);
                        self.bool_val(n)
                    }
                    (UnOp::Neg, Ty::Int) => {
                        let z = self.b.ins().iconst(types::I64, 0);
                        let (r, of) = self.b.ins().ssub_overflow(z, v);
                        self.deopt_if(of);
                        (r, Ty::Int)
                    }
                    (UnOp::Neg, Ty::Float) => (self.b.ins().fneg(v), Ty::Float),
                    (UnOp::Plus, Ty::Int | Ty::Float) => (v, t),
                    (UnOp::BitNot, Ty::Int) => (self.b.ins().bnot(v), Ty::Int),
                    _ => return Err("unary op"),
                }
            }
            Ternary { cond, then, else_ } => {
                let (cv, ct) = self.expr(cond)?;
                let c = self.truthy(cv, ct);
                let (yes, no, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().brif(c, yes, &[], no, &[]);
                self.b.switch_to_block(yes);
                let (tv, tt) = match then {
                    Some(t) => self.expr(t)?,
                    None => (cv, ct),
                };
                self.b.append_block_param(merge, clty(tt));
                self.b.ins().jump(merge, &[tv.into()]);
                self.b.switch_to_block(no);
                let (ev, et) = self.expr(else_)?;
                if et != tt {
                    return Err("ternary branch types differ");
                }
                self.b.ins().jump(merge, &[ev.into()]);
                self.b.switch_to_block(merge);
                (self.b.block_params(merge)[0], tt)
            }
            Cast { ty, expr } => {
                let (v, t) = self.expr(expr)?;
                match (*ty, t) {
                    ("int", Ty::Int | Ty::Bool) => (v, Ty::Int),
                    ("int", Ty::Float) => {
                        // PHP wraps out-of-range floats; let the interpreter do it.
                        let lo = self.b.ins().f64const(-9.223372036854775808e18);
                        let hi = self.b.ins().f64const(9.223372036854775808e18);
                        let ge = self.b.ins().fcmp(FloatCC::GreaterThanOrEqual, v, lo);
                        let lt = self.b.ins().fcmp(FloatCC::LessThan, v, hi);
                        let ok = self.b.ins().band(ge, lt);
                        let bad = self.b.ins().bxor_imm_s(ok, 1);
                        self.deopt_if(bad);
                        (self.b.ins().fcvt_to_sint(types::I64, v), Ty::Int)
                    }
                    ("float", Ty::Float) => (v, Ty::Float),
                    ("float", Ty::Int | Ty::Bool) => (self.b.ins().fcvt_from_sint(types::F64, v), Ty::Float),
                    ("bool", _) => {
                        let c = self.truthy(v, t);
                        self.bool_val(c)
                    }
                    _ => return Err("cast"),
                }
            }
            Call { callee, args } => {
                let ExprKind::Name(name) = &callee.kind else { return Err("dynamic call") };
                let mut vals = vec![];
                for a in args {
                    vals.push(self.expr(&a.value)?);
                }
                if let Some(r) = self.builtin(name, &vals)? {
                    return Ok(r);
                }
                let f = self.r.resolve(self.from, name).ok_or("unknown function")?;
                let tys: Vec<Ty> = vals.iter().map(|v| v.1).collect();
                let (id, ret) = self.jit.callee(f, &tys).ok_or("callee not compiled")?;
                let fref = self.jit.module.declare_func_in_func(id, self.b.func);
                let mut cargs: Vec<CValue> = vals.iter().map(|v| v.0).collect();
                cargs.push(self.depth.unwrap());
                let call = self.b.ins().call(fref, &cargs);
                let (v, st) = (self.b.inst_results(call)[0], self.b.inst_results(call)[1]);
                let failed = self.b.ins().icmp_imm_s(IntCC::NotEqual, st, 0);
                self.deopt_if(failed);
                let v = if ret == Ty::Float { { let mf = self.plain(); self.b.ins().bitcast(types::F64, mf, v) } } else { v };
                (v, ret)
            }
            _ => return Err("expression not supported"),
        })
    }

    fn builtin(&mut self, name: &str, vals: &[(CValue, Ty)]) -> G<Option<(CValue, Ty)>> {
        let lname = name.trim_start_matches('\\').to_ascii_lowercase();
        Ok(Some(match (lname.as_str(), vals) {
            ("abs", [(v, Ty::Int)]) => {
                let z = self.b.ins().iconst(types::I64, 0);
                let (neg, of) = self.b.ins().ssub_overflow(z, *v);
                self.deopt_if(of);
                let isneg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, *v, 0);
                (self.b.ins().select(isneg, neg, *v), Ty::Int)
            }
            ("abs", [(v, Ty::Float)]) => (self.b.ins().fabs(*v), Ty::Float),
            ("max" | "min", [(a, Ty::Int), (b, Ty::Int)]) => {
                let cc = if lname == "max" { IntCC::SignedGreaterThan } else { IntCC::SignedLessThan };
                let take_b = self.b.ins().icmp(cc, *b, *a);
                (self.b.ins().select(take_b, *b, *a), Ty::Int)
            }
            ("max" | "min", [(a, Ty::Float), (b, Ty::Float)]) => {
                let cc = if lname == "max" { FloatCC::GreaterThan } else { FloatCC::LessThan };
                let take_b = self.b.ins().fcmp(cc, *b, *a);
                (self.b.ins().select(take_b, *b, *a), Ty::Float)
            }
            ("intdiv", [(a, Ty::Int), (b, Ty::Int)]) => {
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, *b, 0);
                self.deopt_if(zero);
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, *b, -1);
                let min = self.b.ins().icmp_imm_s(IntCC::Equal, *a, i64::MIN);
                let both = self.b.ins().band(m1, min);
                self.deopt_if(both);
                (self.b.ins().sdiv(*a, *b), Ty::Int)
            }
            ("sqrt", [(v, t)]) => {
                let f = self.to_f64(*v, *t)?;
                (self.b.ins().sqrt(f), Ty::Float)
            }
            ("floor", [(v, t)]) => {
                let f = self.to_f64(*v, *t)?;
                (self.b.ins().floor(f), Ty::Float)
            }
            ("ceil", [(v, t)]) => {
                let f = self.to_f64(*v, *t)?;
                (self.b.ins().ceil(f), Ty::Float)
            }
            _ => return Ok(None),
        }))
    }

    fn binop(&mut self, op: BinOp, a: CValue, at: Ty, b: CValue, bt: Ty) -> G<(CValue, Ty)> {
        use BinOp::*;
        let ints = at == Ty::Int && bt == Ty::Int;
        Ok(match op {
            Add | Sub | Mul if ints => {
                let (r, of) = match op {
                    Add => self.b.ins().sadd_overflow(a, b),
                    Sub => self.b.ins().ssub_overflow(a, b),
                    _ => self.b.ins().smul_overflow(a, b),
                };
                self.deopt_if(of);
                (r, Ty::Int)
            }
            Add | Sub | Mul | Div => {
                let (x, y) = (self.to_f64(a, at)?, self.to_f64(b, bt)?);
                (
                    match op {
                        Add => self.b.ins().fadd(x, y),
                        Sub => self.b.ins().fsub(x, y),
                        Mul => self.b.ins().fmul(x, y),
                        _ => {
                            if ints {
                                return Err("int / int");
                            }
                            // PHP 8 throws DivisionByZeroError even for floats.
                            let z = self.b.ins().f64const(0.0);
                            let is_zero = self.b.ins().fcmp(FloatCC::Equal, y, z);
                            self.deopt_if(is_zero);
                            self.b.ins().fdiv(x, y)
                        }
                    },
                    Ty::Float,
                )
            }
            Mod if ints => {
                let zero = self.b.ins().icmp_imm_s(IntCC::Equal, b, 0);
                self.deopt_if(zero);
                // x % -1 is 0 in PHP; avoid the INT_MIN / -1 trap.
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, b, -1);
                let one = self.b.ins().iconst(types::I64, 1);
                let safe = self.b.ins().select(m1, one, b);
                let r = self.b.ins().srem(a, safe);
                let z = self.b.ins().iconst(types::I64, 0);
                (self.b.ins().select(m1, z, r), Ty::Int)
            }
            BitAnd if ints => (self.b.ins().band(a, b), Ty::Int),
            BitOr if ints => (self.b.ins().bor(a, b), Ty::Int),
            BitXor if ints => (self.b.ins().bxor(a, b), Ty::Int),
            Shl | Shr if ints => {
                let neg = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, b, 0);
                self.deopt_if(neg);
                let big = self.b.ins().icmp_imm_s(IntCC::SignedGreaterThanOrEqual, b, 64);
                let (shifted, overflow) = if op == Shl {
                    (self.b.ins().ishl(a, b), self.b.ins().iconst(types::I64, 0))
                } else {
                    (self.b.ins().sshr(a, b), self.b.ins().sshr_imm_s(a, 63))
                };
                (self.b.ins().select(big, overflow, shifted), Ty::Int)
            }
            Eq | NotEq | Lt | LtEq | Gt | GtEq | Identical | NotIdentical => {
                if matches!(op, Identical | NotIdentical) && at != bt {
                    return Ok((self.b.ins().iconst(types::I64, (op == NotIdentical) as i64), Ty::Bool));
                }
                let c = if at == Ty::Bool || bt == Ty::Bool {
                    let (x, y) = (self.truthy(a, at), self.truthy(b, bt));
                    let cc = int_cc(op);
                    self.b.ins().icmp(cc, x, y)
                } else if ints {
                    self.b.ins().icmp(int_cc(op), a, b)
                } else {
                    let (x, y) = (self.to_f64(a, at)?, self.to_f64(b, bt)?);
                    self.b.ins().fcmp(float_cc(op), x, y)
                };
                self.bool_val(c)
            }
            Spaceship if ints => {
                let gt = self.b.ins().icmp(IntCC::SignedGreaterThan, a, b);
                let lt = self.b.ins().icmp(IntCC::SignedLessThan, a, b);
                let (g, _) = self.bool_val(gt);
                let (l, _) = self.bool_val(lt);
                (self.b.ins().isub(g, l), Ty::Int)
            }
            Xor => {
                let (x, y) = (self.truthy(a, at), self.truthy(b, bt));
                let r = self.b.ins().bxor(x, y);
                self.bool_val(r)
            }
            _ => return Err("binary op"),
        })
    }
}

fn has_loop(stmts: &[Stmt]) -> bool {
    stmts.iter().any(|s| match &s.kind {
        StmtKind::While { .. } | StmtKind::DoWhile { .. } | StmtKind::For { .. } | StmtKind::Foreach { .. } => true,
        StmtKind::If { then, elseifs, else_, .. } => {
            has_loop(then) || elseifs.iter().any(|(_, b)| has_loop(b)) || else_.as_deref().is_some_and(has_loop)
        }
        StmtKind::Block(b) => has_loop(b),
        _ => false,
    })
}

fn int_cc(op: BinOp) -> IntCC {
    match op {
        BinOp::Eq | BinOp::Identical => IntCC::Equal,
        BinOp::NotEq | BinOp::NotIdentical => IntCC::NotEqual,
        BinOp::Lt => IntCC::SignedLessThan,
        BinOp::LtEq => IntCC::SignedLessThanOrEqual,
        BinOp::Gt => IntCC::SignedGreaterThan,
        _ => IntCC::SignedGreaterThanOrEqual,
    }
}

/// Ordered comparisons, so NAN compares false like PHP (except `!=`).
fn float_cc(op: BinOp) -> FloatCC {
    match op {
        BinOp::Eq | BinOp::Identical => FloatCC::Equal,
        BinOp::NotEq | BinOp::NotIdentical => FloatCC::NotEqual,
        BinOp::Lt => FloatCC::LessThan,
        BinOp::LtEq => FloatCC::LessThanOrEqual,
        BinOp::Gt => FloatCC::GreaterThan,
        _ => FloatCC::GreaterThanOrEqual,
    }
}
