//! Native compiler for whole PHP functions (Cranelift).
//!
//! A function is compiled the second time it is called (`PHPX_NATIVE=all`:
//! the first time; `=off`: never). Unsupported functions stay interpreted.
//! Compiled code runs with the interpreter's semantics: everything beyond
//! inline int fast paths goes through `rt` helpers that reuse interpreter
//! methods. `PHPX_NATIVE_LOG=1` reports what compiled and why not.

pub(crate) mod analyze;
mod lower;
pub mod rt;

use std::collections::HashMap;

use cranelift_codegen::ir::{types, AbiParam, InstBuilder, StackSlotData, StackSlotKind};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module};
use phpx_syntax::ast;

use crate::interp::{Interp, R};
use crate::value::{tag, Value};
pub use analyze::Body;
use rt::{CallSite, Cx, LocalsSpec};

/// `entry(cx, args, nargs, out)`. With `args` null, parameters come from
/// the interpreter frame (`invoke` already bound them); otherwise they are
/// bound natively from the argument array (fast calls between compiled
/// functions). Returns 0 ok, 1 error (`pending`), 2 declined.
pub type Entry = unsafe extern "C" fn(*mut Cx, *mut Value, u64, *mut Value) -> u64;

/// What the compiler needs to know about a function.
pub struct Target {
    pub fn_id: usize,
    pub display: std::rc::Rc<str>,
    pub params: &'static [ast::Param],
    pub captured: Vec<&'static str>,
    pub body: Body,
    pub ctx: std::rc::Rc<crate::interp::NsCtx>,
    pub class: Option<std::rc::Rc<crate::class::Class>>,
}

pub struct Native {
    module: JITModule,
    helpers: HashMap<&'static str, FuncId>,
    compiled: HashMap<usize, Option<Entry>>,
    hot: HashMap<usize, u32>,
    threshold: u32,
    consts: Vec<Box<Value>>,
    sites: Vec<Box<CallSite>>,
    msites: Vec<Box<rt::MethodSite>>,
    psites: Vec<Box<rt::PropSite>>,
    specs: Vec<Box<LocalsSpec>>,
    log: bool,
}

macro_rules! helpers {
    ($($name:ident : $n:literal => $ret:literal),* $(,)?) => {
        &[$((stringify!($name), rt::$name as *const u8, $n, $ret)),*]
    };
}

/// (name, address, parameter count, returns a value). All params are i64.
const HELPERS: &[(&str, *const u8, usize, bool)] = helpers![
    rt_load_locals: 3 => true,
    rt_bind_args: 5 => true,
    rt_store_locals: 3 => false,
    rt_bind_slots: 4 => true,
    rt_unbind_slots: 2 => false,
    rt_declare_class: 2 => true,
    rt_define_const: 3 => true,
    rt_release: 1 => false,
    rt_release_n: 2 => false,
    rt_copy: 2 => false,
    rt_binop: 5 => true,
    rt_compare: 4 => true,
    rt_truthy: 1 => true,
    rt_identical: 2 => true,
    rt_loose_eq: 2 => true,
    rt_unary: 4 => true,
    rt_concat_assign: 3 => true,
    rt_interp: 4 => true,
    rt_echo: 2 => true,
    rt_incdec: 4 => true,
    rt_cast: 4 => true,
    rt_index_get: 4 => true,
    rt_quiet_index: 4 => true,
    rt_prop_get: 5 => true,
    rt_prop_read: 4 => true,
    rt_local_store: 5 => true,
    rt_quiet_prop: 5 => true,
    rt_const: 3 => true,
    rt_magic: 3 => true,
    rt_class_const: 5 => true,
    rt_static_prop_get: 5 => true,
    rt_instanceof: 3 => true,
    rt_place_assign: 9 => true,
    rt_place_update: 9 => true,
    rt_unset_place: 6 => true,
    rt_call: 7 => true,
    rt_call_value: 7 => true,
    rt_method_call: 8 => true,
    rt_method_call_dyn: 8 => true,
    rt_prop_get_dyn: 4 => true,
    rt_static_call: 9 => true,
    rt_new: 7 => true,
    rt_make_closure: 6 => true,
    rt_clone: 3 => true,
    rt_array_new: 1 => false,
    rt_array_add: 5 => true,
    rt_iter_new: 3 => true,
    rt_iter_next: 4 => true,
    rt_iter_free: 1 => false,
    rt_throw: 2 => true,
    rt_prop_write: 5 => true,
    rt_destruct: 1 => true,
    rt_catch: 3 => true,
    rt_pending_save: 2 => true,
    rt_pending_restore: 2 => false,
    rt_stash_free: 1 => false,
    rt_exit: 2 => true,
    rt_unhandled_match: 2 => true,
];

thread_local! {
    static INLINE_RC: bool = rc_layout_ok();
    static LAYOUT: Option<Layout> = probe_layout();
}

/// Offsets generated code uses for inline property access.
#[derive(Clone, Copy)]
pub struct Layout {
    /// From the Rc pointer of an object to its `Props`.
    pub obj_props: i32,
    /// Tag byte of `Option::<Value>::None` (an empty property slot).
    pub none_tag: u8,
    /// `Cx::frame`, and `Frame::class` within the frame.
    pub cx_frame: i32,
    pub frame_class: i32,
}

pub fn layout() -> Option<Layout> {
    LAYOUT.with(|l| *l)
}

fn probe_layout() -> Option<Layout> {
    use std::mem::{offset_of, size_of};
    if !rc_layout_ok() {
        return None;
    }
    // An empty slot must be told apart from every Value by its tag byte, and
    // a full slot must look exactly like a Value.
    let none: Option<Value> = None;
    let none_tag = unsafe { *(&none as *const Option<Value> as *const u8) };
    if none_tag <= tag::RESOURCE {
        return None;
    }
    let some: Option<Value> = Some(Value::Int(7));
    let (t, p) = unsafe {
        let b = &some as *const Option<Value> as *const u8;
        (*b, *(b.add(tag::PAYLOAD as usize) as *const i64))
    };
    if t != tag::INT || p != 7 {
        return None;
    }
    // RefCell<Props>: where the value sits inside the cell.
    let cell = std::cell::RefCell::new(crate::props::Props::default());
    let in_cell = cell.as_ptr() as usize - &cell as *const _ as usize;
    if size_of::<Option<std::rc::Rc<crate::class::Class>>>() != 8 {
        return None;
    }
    Some(Layout {
        obj_props: (2 * size_of::<usize>() + offset_of!(crate::value::Obj, props) + in_cell) as i32,
        none_tag,
        cx_frame: offset_of!(rt::Cx, frame) as i32,
        frame_class: offset_of!(crate::interp::Frame, class) as i32,
    })
}

/// Generated code inlines Rc strong-count updates: the count is the first
/// word of the allocation the Rc points to (std's `RcInner` is
/// `repr(C)` with `strong` first). Verified at runtime; if it ever doesn't
/// hold, codegen falls back to helper calls.
pub fn inline_rc() -> bool {
    INLINE_RC.with(|b| *b)
}

fn rc_layout_ok() -> bool {
    let v = Value::str("probe");
    let Value::Str(rc) = &v else { return false };
    let raw = std::rc::Rc::as_ptr(rc) as *const u8;
    // Payload word of the Value is the pointer to the Rc's inner block.
    let payload = unsafe { ((&v as *const Value as *const u8).add(tag::PAYLOAD as usize) as *const usize).read() };
    let count = |rc_ptr: usize| unsafe { (rc_ptr as *const usize).read() };
    let before = count(payload);
    let c2 = v.clone();
    let after = count(payload);
    drop(c2);
    // The data pointer sits after strong+weak (two words) in RcInner.
    before == 1 && after == 2 && raw as usize == payload + 2 * std::mem::size_of::<usize>()
}

impl Native {
    pub fn from_env() -> Option<Native> {
        let mode = std::env::var("PHPX_NATIVE").unwrap_or_default();
        let threshold = match mode.as_str() {
            "off" | "0" => return None,
            "all" => 1,
            _ => 2,
        };
        Native::new(threshold)
    }

    fn new(threshold: u32) -> Option<Native> {
        let mut flags = settings::builder();
        flags.set("use_colocated_libcalls", "false").ok()?;
        flags.set("is_pic", "false").ok()?;
        flags.set("opt_level", "speed").ok()?;
        let isa = cranelift_native::builder().ok()?.finish(settings::Flags::new(flags)).ok()?;
        let mut jb = JITBuilder::with_isa(isa, default_libcall_names());
        for (name, ptr, _, _) in HELPERS {
            jb.symbol(*name, *ptr);
        }
        let mut module = JITModule::new(jb);
        let mut helpers = HashMap::new();
        for (name, _, n, ret) in HELPERS {
            let mut sig = module.make_signature();
            for _ in 0..*n {
                sig.params.push(AbiParam::new(types::I64));
            }
            if *ret {
                sig.returns.push(AbiParam::new(types::I64));
            }
            let id = module.declare_function(name, Linkage::Import, &sig).ok()?;
            helpers.insert(*name, id);
        }
        Some(Native {
            module,
            helpers,
            compiled: HashMap::new(),
            hot: HashMap::new(),
            threshold,
            consts: vec![],
            sites: vec![],
            msites: vec![],
            psites: vec![],
            specs: vec![],
            log: std::env::var("PHPX_NATIVE_LOG").is_ok(),
        })
    }

    /// Compiled entry for `fn_id`, compiling it (described by `mk`) when hot.
    /// None = interpret.
    fn entry_for(&mut self, it: &Interp, fn_id: usize, mk: impl FnOnce() -> Target) -> Option<Entry> {
        if let Some(e) = self.compiled.get(&fn_id) {
            return *e;
        }
        let t = &mk();
        // A function with a loop can be hot within a single call.
        let has_loop = matches!(t.body, Body::Block(b) if has_loop(b));
        let n = self.hot.entry(t.fn_id).or_default();
        *n += 1;
        if *n < self.threshold && !has_loop {
            return None;
        }
        // A compiler bug must never take the program down: treat a panic
        // as "not compiled" and keep interpreting.
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.compile(it, t)))
            .unwrap_or(Err("internal compiler error"));
        match &r {
            Ok(_) if self.log => eprintln!("[native] {} compiled", t.display),
            Err(why) if self.log => eprintln!("[native] {}: not compiled: {why}", t.display),
            _ => {}
        }
        let e = r.ok();
        self.compiled.insert(t.fn_id, e);
        e
    }

    fn compile(&mut self, it: &Interp, t: &Target) -> Result<Entry, &'static str> {
        self.compile_mode(it, t, false)
    }

    /// `top_level`: a file segment. Returns exit with status 3 (value in
    /// out); every exit writes locals back to the frame.
    fn compile_mode(&mut self, it: &Interp, t: &Target, top_level: bool) -> Result<Entry, &'static str> {
        let mut a = analyze::analyze_mode(t.params, &t.captured, t.body, top_level)?;
        let this_slot = if a.uses_this { Some(a.locals.slot("this") as usize) } else { None };
        let nlocals = a.locals.names.len();
        let spec = Box::new(LocalsSpec {
            names: a.locals.names.clone(),
            this_slot,
            params: t.params,
            display: t.display.clone(),
            ctx: t.ctx.clone(),
            class: t.class.clone(),
            simple: t.params.iter().all(|p| p.ty.is_none() && !p.by_ref && !p.variadic && p.promote.is_none()),
        });
        let simple = spec.simple;
        let spec_addr = &*spec as *const LocalsSpec as i64;

        let mut sig = self.module.make_signature();
        for _ in 0..4 {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = self.module.declare_anonymous_function(&sig).map_err(|_| "declare failed")?;
        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;
        let mut fctx = FunctionBuilderContext::new();
        let cfg = self.module.target_config();
        let pos_addr = &it.pos as *const _ as i64;
        let mut consts = vec![];
        let mut sites = vec![];
        {
            let mut b = FunctionBuilder::new(&mut ctx.func, &mut fctx);
            let entry = b.create_block();
            b.append_block_params_for_function_params(entry);
            let ps = b.block_params(entry).to_vec();
            let (cxv, argsv, nargsv, outv) = (ps[0], ps[1], ps[2], ps[3]);
            let start = b.create_block();
            let exit_ok = b.create_block();
            let exit_ret = b.create_block();
            let unwind = b.create_block();
            let decline = b.create_block();
            let locals_slot = b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (nlocals.max(1) * 16) as u32, 4));
            // The entry block only jumps to `init`, which is filled last
            // (once all temp slots are known); emitting the jump now puts
            // the entry block first in the layout, as Cranelift requires.
            let init = b.create_block();
            b.switch_to_block(entry);
            b.ins().jump(init, &[]);
            b.switch_to_block(start);
            let mut l = lower::Lower::new(
                b, &mut self.module, &self.helpers, &a.locals, locals_slot, exit_ok, unwind, cxv, outv, pos_addr, t.ctx.file as u32,
                &mut consts, &mut sites,
            );
            let top_rcs = if top_level {
                l.exit_ret = Some(exit_ret);
                let n = (nlocals.max(1) * 8) as u32;
                l.top_ptrs = Some(l.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, n, 3)));
                Some(l.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, n, 3)))
            } else {
                None
            };
            match t.body {
                Body::Block(stmts) => {
                    l.stmts(stmts)?;
                    if !l.finished() {
                        l.b.ins().jump(exit_ok, &[]);
                    }
                }
                Body::Expr(e) => {
                    let v = l.expr(e)?;
                    // Arrow fn: its value is the return value.
                    l.store_result(v);
                    l.b.ins().jump(exit_ok, &[]);
                }
            }
            // Exits: drop every local, temp and iterator (top level: hand
            // locals back to the frame first).
            for (blk, status) in [(exit_ok, 0i64), (unwind, 1), (exit_ret, 3)] {
                l.b.switch_to_block(blk);
                l.cleanup(nlocals);
                if let Some(rcs) = top_rcs {
                    let ra = l.b.ins().stack_addr(types::I64, rcs, 0);
                    let n = l.b.ins().iconst(types::I64, nlocals as i64);
                    l.call("rt_unbind_slots", &[ra, n]);
                }
                let s = l.b.ins().iconst(types::I64, status);
                l.b.ins().return_(&[s]);
            }
            l.b.switch_to_block(decline);
            let two = l.b.ins().iconst(types::I64, 2);
            l.b.ins().return_(&[two]);
            // Init: empty every slot, then load parameters.
            l.b.switch_to_block(init);
            l.init_slots(nlocals);
            if let Some(rcs) = top_rcs {
                let zero = l.b.ins().iconst(types::I64, 0);
                for i in 0..nlocals {
                    l.b.ins().stack_store(types::I64, zero, rcs, (i * 8) as i32);
                }
            }
            // Parameters: from the interpreter frame, or from the args array.
            use cranelift_codegen::ir::condcodes::IntCC;
            let (from_frame, from_args, bound) = (l.b.create_block(), l.b.create_block(), l.b.create_block());
            l.b.append_block_param(bound, types::I64);
            let no_args = l.b.ins().icmp_imm_s(IntCC::Equal, argsv, 0);
            l.b.ins().brif(no_args, from_frame, &[], from_args, &[]);
            l.b.switch_to_block(from_frame);
            let sp = l.b.ins().iconst(types::I64, spec_addr);
            let st = match (l.top_ptrs, top_rcs) {
                (Some(ptrs), Some(rcs)) => {
                    let pa = l.b.ins().stack_addr(types::I64, ptrs, 0);
                    let ra = l.b.ins().stack_addr(types::I64, rcs, 0);
                    l.call("rt_bind_slots", &[cxv, pa, ra, sp]).unwrap()
                }
                _ => {
                    let la = l.b.ins().stack_addr(types::I64, locals_slot, 0);
                    l.call("rt_load_locals", &[cxv, la, sp]).unwrap()
                }
            };
            l.b.ins().jump(bound, &[st.into()]);
            l.b.switch_to_block(from_args);
            let nparams = t.params.len();
            if simple && this_slot.is_none() {
                // Exact arg count: move args into the parameter slots inline.
                let (inline_b, helper_b) = (l.b.create_block(), l.b.create_block());
                let exact = l.b.ins().icmp_imm_s(IntCC::Equal, nargsv, nparams as i64);
                l.b.ins().brif(exact, inline_b, &[], helper_b, &[]);
                l.b.switch_to_block(inline_b);
                let flags = cranelift_codegen::ir::MemFlagsData::trusted();
                let null = l.b.ins().iconst(types::I8, tag::NULL as i64);
                for i in 0..nparams {
                    let src = l.b.ins().iadd_imm_s(argsv, (i * 16) as i64);
                    let dst = l.b.ins().stack_addr(types::I64, locals_slot, (i * 16) as i32);
                    let a = l.b.ins().load(types::I64, flags, src, 0);
                    let b2 = l.b.ins().load(types::I64, flags, src, 8);
                    l.b.ins().store(flags, a, dst, 0);
                    l.b.ins().store(flags, b2, dst, 8);
                    l.b.ins().store(flags, null, src, 0);
                }
                let zero = l.b.ins().iconst(types::I64, 0);
                l.b.ins().jump(bound, &[zero.into()]);
                l.b.switch_to_block(helper_b);
            }
            let la = l.b.ins().stack_addr(types::I64, locals_slot, 0);
            let sp = l.b.ins().iconst(types::I64, spec_addr);
            let st = l.call("rt_bind_args", &[cxv, la, sp, argsv, nargsv]).unwrap();
            l.b.ins().jump(bound, &[st.into()]);
            l.b.switch_to_block(bound);
            let st = l.b.block_params(bound)[0];
            let (check_err, ok_b) = (l.b.create_block(), l.b.create_block());
            let declined = l.b.ins().icmp_imm_s(IntCC::Equal, st, 2);
            l.b.ins().brif(declined, decline, &[], check_err, &[]);
            l.b.switch_to_block(check_err);
            let failed = l.b.ins().icmp_imm_s(IntCC::Equal, st, 1);
            l.b.ins().brif(failed, unwind, &[], ok_b, &[]);
            l.b.switch_to_block(ok_b);
            l.b.ins().jump(start, &[]);
            l.b.seal_all_blocks();
            let msites = std::mem::take(&mut l.msites);
            let psites = std::mem::take(&mut l.psites);
            l.b.finalize(cfg);
            self.msites.extend(msites);
            self.psites.extend(psites);
        }
        self.module.define_function(id, &mut ctx).map_err(|e| {
            if self.log {
                eprintln!("[native] codegen error: {e:?}");
            }
            "codegen failed"
        })?;
        self.module.finalize_definitions().map_err(|_| "finalize failed")?;
        let ptr = self.module.get_finalized_function(id);
        self.consts.extend(consts);
        self.sites.extend(sites);
        self.specs.push(spec);
        let _ = &mut a;
        // SAFETY: generated with exactly the Entry signature.
        Ok(unsafe { std::mem::transmute::<*const u8, Entry>(ptr) })
    }
}

fn has_loop(stmts: &[ast::Stmt]) -> bool {
    use ast::StmtKind::*;
    stmts.iter().any(|s| match &s.kind {
        While { .. } | DoWhile { .. } | For { .. } | Foreach { .. } => true,
        If { then, elseifs, else_, .. } => has_loop(then) || elseifs.iter().any(|(_, b)| has_loop(b)) || else_.as_deref().is_some_and(has_loop),
        Block(b) => has_loop(b),
        Switch { cases, .. } => cases.iter().any(|c| has_loop(&c.body)),
        _ => false,
    })
}

impl lower::Lower<'_> {
    fn init_slots(&mut self, nlocals: usize) {
        let null = self.b.ins().iconst(types::I8, tag::NULL as i64);
        let nlocals = if self.top_ptrs.is_some() { 0 } else { nlocals };
        for i in 0..nlocals {
            let a = self.b.ins().stack_addr(types::I64, self.locals_slot, (i * 16) as i32);
            self.b.ins().store(cranelift_codegen::ir::MemFlagsData::trusted(), null, a, 0);
        }
        for &(slot, off) in &self.temps {
            let a = self.b.ins().stack_addr(types::I64, slot, off);
            self.b.ins().store(cranelift_codegen::ir::MemFlagsData::trusted(), null, a, 0);
        }
        let zero = self.b.ins().iconst(types::I64, 0);
        for &s in self.iters.iter().chain(&self.stashes) {
            self.b.ins().stack_store(types::I64, zero, s, 0);
        }
    }

    /// Drop every slot. Inline tag checks: scalars need no call.
    fn cleanup(&mut self, nlocals: usize) {
        // Top level: the locals belong to the frame.
        let nlocals = if self.top_ptrs.is_some() { 0 } else { nlocals };
        for i in 0..nlocals {
            let a = self.b.ins().stack_addr(types::I64, self.locals_slot, (i * 16) as i32);
            self.clear_slot(a);
        }
        let temps = self.temps.clone();
        for (slot, off) in temps {
            let a = self.b.ins().stack_addr(types::I64, slot, off);
            self.clear_slot(a);
        }
        let iters = self.iters.clone();
        for s in iters {
            let a = self.b.ins().stack_addr(types::I64, s, 0);
            self.call("rt_iter_free", &[a]);
        }
        for s in self.stashes.clone() {
            let a = self.b.ins().stack_addr(types::I64, s, 0);
            self.call("rt_stash_free", &[a]);
        }
    }
}

impl Interp {
    /// Run a file's top-level segment natively: compiled when it has a loop
    /// (or `PHPX_NATIVE=all`). None = interpret it.
    pub fn native_top_level(&self, f: &mut crate::interp::Frame, body: &'static [ast::Stmt]) -> Option<R<crate::interp::Flow>> {
        let key = body.as_ptr() as usize;
        let entry = {
            let mut guard = self.native.try_borrow_mut().ok()?;
            let n = guard.as_mut()?;
            match n.compiled.get(&key) {
                Some(e) => (*e)?,
                None => {
                    if n.threshold > 1 && !has_loop(body) {
                        return None;
                    }
                    let t = Target {
                        fn_id: key,
                        display: std::rc::Rc::from("{main}"),
                        params: &[],
                        captured: vec![],
                        body: Body::Block(body),
                        ctx: f.ctx.clone(),
                        class: None,
                    };
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| n.compile_mode(self, &t, true)))
                        .unwrap_or(Err("internal compiler error"));
                    if n.log {
                        match &r {
                            Ok(_) => eprintln!("[native] top-level compiled"),
                            Err(why) => eprintln!("[native] top-level: not compiled: {why}"),
                        }
                    }
                    let e = r.ok();
                    n.compiled.insert(key, e);
                    e?
                }
            }
        };
        let mut cx = Cx { it: self as *const Interp, frame: f as *mut _ };
        let mut out = Value::Null;
        // SAFETY: entry follows the Entry ABI.
        let st = unsafe { entry(&mut cx, std::ptr::null_mut(), 0, &mut out) };
        match st {
            0 => Some(Ok(crate::interp::Flow::Normal)),
            3 => Some(Ok(crate::interp::Flow::Return(out))),
            1 => Some(Err(self.pending.borrow_mut().take().expect("native error without pending exception"))),
            _ => None,
        }
    }

    /// Native entry for a function, compiling it when hot (`mk` describes it
    /// and is only called if needed). None = interpret.
    pub fn native_entry(&self, fn_id: usize, mk: impl FnOnce() -> Target) -> Option<Entry> {
        let mut guard = self.native.try_borrow_mut().ok()?;
        guard.as_mut()?.entry_for(self, fn_id, mk)
    }

    /// Run a user function natively (parameters already bound in `f`).
    /// None = run it in the interpreter (cold, unsupported, or declined).
    pub fn try_native(&self, fn_id: usize, mk: impl FnOnce() -> Target, f: &mut crate::interp::Frame) -> Option<R<Value>> {
        // The native borrow ends here: native code may call back into PHP code.
        let entry = self.native_entry(fn_id, mk)?;
        let mut cx = Cx { it: self as *const Interp, frame: f as *mut _ };
        let mut out = Value::Null;
        // SAFETY: entry follows the Entry ABI; cx and out outlive the call.
        let st = unsafe { entry(&mut cx, std::ptr::null_mut(), 0, &mut out) };
        match st {
            0 => Some(Ok(out)),
            1 => Some(Err(self.pending.borrow_mut().take().expect("native error without pending exception"))),
            _ => None,
        }
    }
}
