//! Native compiler for whole PHP functions (Cranelift).
//!
//! A function is compiled once it is hot: after DEFAULT_THRESHOLD calls
//! (`PHPX_NATIVE_THRESHOLD=n`; `PHPX_NATIVE=all`: the first call; `=off`:
//! never), or mid-call when one of its loops runs long (on-stack
//! replacement: an entry that starts at that loop with the interpreter's
//! locals). Unsupported functions stay interpreted.
//! Compiled code runs with the interpreter's semantics: everything beyond
//! inline int fast paths goes through `rt` helpers that reuse interpreter
//! methods. `PHPX_NATIVE_LOG=1` reports what compiled and why not.

pub(crate) mod analyze;
pub mod fastbi;
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
/// Largest function (in IR instructions) worth compiling.
const MAX_INSTS: usize = 60_000;

pub type Entry = unsafe extern "C" fn(*mut Cx, *mut Value, u64, *mut Value) -> u64;

/// What the compiler needs to know about a function.
#[derive(Clone)]
pub struct Target {
    pub fn_id: usize,
    pub display: std::rc::Rc<str>,
    pub params: &'static [ast::Param],
    pub captured: Vec<&'static str>,
    pub body: Body,
    pub ctx: std::rc::Rc<crate::interp::NsCtx>,
    pub class: Option<std::rc::Rc<crate::class::Class>>,
    /// Declared return type: compiled code checks it before returning.
    pub ret: Option<&'static ast::Type>,
}

/// Calls before a function is compiled.
const DEFAULT_THRESHOLD: u32 = 1000;
/// Iterations of an interpreted loop (in one run) before it moves to
/// native code.
pub const OSR_ITERATIONS: u32 = 500;

pub struct Native {
    module: JITModule,
    helpers: HashMap<&'static str, FuncId>,
    compiled: rustc_hash::FxHashMap<usize, Option<Entry>>,
    /// Calls so far, and whether the function was checked for loops (OSR).
    hot: rustc_hash::FxHashMap<usize, (u32, bool)>,
    threshold: u32,
    /// Functions seen running interpreted, for on-stack replacement.
    osr_info: HashMap<usize, Target>,
    /// OSR entries by loop statement.
    osr: HashMap<usize, Option<Entry>>,
    compile_ns: u64,
    consts: Vec<Box<Value>>,
    sites: Vec<Box<CallSite>>,
    msites: Vec<Box<rt::MethodSite>>,
    psites: Vec<Box<rt::PropSite>>,
    csites: Vec<Box<rt::ClassSite>>,
    specs: Vec<Box<LocalsSpec>>,
    log: bool,
    /// Debugging aids for bisecting miscompiles: compile at most this many
    /// functions (PHPX_NATIVE_MAX), and never ones whose name contains any
    /// of these (PHPX_NATIVE_SKIP, comma-separated).
    max: Option<usize>,
    skip: Vec<String>,
    count: usize,
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
    rt_check_ret: 3 => true,
    rt_site_reset: 1 => false,
    rt_destruct_now: 1 => true,
    rt_fast_ord: 3 => true,
    rt_fast_chr: 3 => true,
    rt_fast_key_exists: 3 => true,
    rt_fast_in_array: 3 => true,
    rt_fast_str_contains: 3 => true,
    rt_fast_str_starts_with: 3 => true,
    rt_fast_str_ends_with: 3 => true,
    rt_fast_strpos: 3 => true,
    rt_fast_substr: 3 => true,
    rt_fast_strtolower: 3 => true,
    rt_fast_strtoupper: 3 => true,
    rt_fast_trim: 3 => true,
    rt_fast_str_replace: 3 => true,
    rt_fast_implode: 3 => true,
    rt_fast_abs: 3 => true,
    rt_fast_min: 3 => true,
    rt_fast_max: 3 => true,
    rt_fast_floor: 3 => true,
    rt_fast_ceil: 3 => true,
    rt_fast_intval: 3 => true,
    rt_fast_explode: 3 => true,
    rt_fast_array_keys: 3 => true,
    rt_fast_array_values: 3 => true,
    rt_fast_array_key_first: 3 => true,
    rt_fast_array_key_last: 3 => true,
    rt_fast_ucfirst: 3 => true,
    rt_fast_lcfirst: 3 => true,
    rt_fast_str_repeat: 3 => true,
    rt_fast_get_class: 3 => true,
    rt_store_locals: 3 => false,
    rt_bind_slots: 5 => true,
    rt_unbind_slots: 4 => false,
    rt_eval_ast: 7 => true,
    rt_unset_local: 6 => false,
    rt_scope_refresh: 6 => false,
    rt_exec_ast: 7 => true,
    rt_call_push: 4 => false,
    rt_call_pop: 1 => false,
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
    rt_list_get: 5 => true,
    rt_quiet_index: 4 => true,
    rt_prop_get: 5 => true,
    rt_prop_read: 4 => true,
    rt_local_store: 5 => true,
    rt_quiet_prop: 5 => true,
    rt_isset_prop: 5 => true,
    rt_empty_prop: 5 => true,
    rt_const: 3 => true,
    rt_magic: 3 => true,
    rt_class_const: 6 => true,
    rt_static_prop_get: 6 => true,
    rt_called_class: 2 => true,
    rt_func_get_args: 4 => true,
    rt_func_num_args: 2 => true,
    rt_dyn_class: 2 => true,
    rt_class_name_of: 3 => true,
    rt_instanceof_dyn: 4 => true,
    rt_instanceof: 4 => true,
    rt_place_assign: 9 => true,
    rt_place_update: 9 => true,
    rt_unset_place: 6 => true,
    rt_call: 7 => true,
    rt_call_value: 8 => true,
    rt_method_call: 8 => true,
    rt_method_call_dyn: 8 => true,
    rt_prop_get_dyn: 4 => true,
    rt_static_call: 8 => true,
    rt_new: 8 => true,
    rt_new_object: 4 => true,
    rt_new_ctor: 8 => true,
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
    rt_silence_begin: 1 => true,
    rt_silence_end: 2 => false,
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
    /// `Frame::this` (an `Option<Rc<Obj>>`: the Rc's inner pointer or 0).
    pub frame_this: i32,
    /// `Frame::static_class` (raw Rc word or 0).
    pub frame_static: i32,
    /// `CallSite::direct`, its frame pointer and busy flag.
    pub site_direct: i32,
    pub site_frame: i32,
    pub site_busy: i32,
    /// Frame variable and argument counts (non-zero: the frame needs a
    /// reset after a direct call).
    pub frame_vars_len: i32,
    pub frame_args_len: i32,
    /// From an object's Rc pointer to its class pointer.
    pub obj_class: i32,
    /// `MethodSite::direct`, `direct_class`, `direct_scope`, frame, busy.
    pub msite_direct: i32,
    pub msite_class: i32,
    pub msite_scope: i32,
    pub msite_frame: i32,
    pub msite_busy: i32,
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
    if size_of::<Option<std::rc::Rc<crate::class::Class>>>() != 8
        || size_of::<Option<std::rc::Rc<crate::value::Obj>>>() != 8
        || size_of::<std::cell::UnsafeCell<Option<Box<crate::interp::Frame>>>>() != 8
        || size_of::<std::cell::Cell<bool>>() != 1
    {
        return None;
    }
    // `Frame::this` holds the same word an object Value's payload does.
    // (Probed with an array: Rc's layout doesn't depend on T, and creating
    // an object here would use up an object handle.)
    let v = Value::array(crate::value::PhpArray::new());
    let Value::Array(o) = &v else { return None };
    let this = Some(o.clone());
    let same = unsafe {
        let a = *(&this as *const _ as *const usize);
        let b = *((&v as *const Value as *const u8).add(tag::PAYLOAD as usize) as *const usize);
        a == b
    };
    drop(this);
    let none: Option<std::rc::Rc<crate::value::Obj>> = None;
    if !same || unsafe { *(&none as *const _ as *const usize) } != 0 {
        return None;
    }
    let (frame_vars_len, frame_args_len) = crate::interp::Frame::len_offsets()?;
    Some(Layout {
        obj_props: (2 * size_of::<usize>() + offset_of!(crate::value::Obj, props) + in_cell) as i32,
        none_tag,
        cx_frame: offset_of!(rt::Cx, frame) as i32,
        frame_class: offset_of!(crate::interp::Frame, class) as i32,
        frame_this: offset_of!(crate::interp::Frame, this) as i32,
        frame_static: offset_of!(crate::interp::Frame, static_class) as i32,
        site_direct: offset_of!(rt::CallSite, direct) as i32,
        site_frame: (offset_of!(rt::CallSite, frame) + rt::SiteFrame::FRAME) as i32,
        site_busy: (offset_of!(rt::CallSite, frame) + rt::SiteFrame::BUSY) as i32,
        frame_vars_len: frame_vars_len as i32,
        frame_args_len: frame_args_len as i32,
        obj_class: (2 * size_of::<usize>() + offset_of!(crate::value::Obj, class)) as i32,
        msite_direct: offset_of!(rt::MethodSite, direct) as i32,
        msite_class: offset_of!(rt::MethodSite, direct_class) as i32,
        msite_scope: offset_of!(rt::MethodSite, direct_scope) as i32,
        msite_frame: (offset_of!(rt::MethodSite, frame) + rt::SiteFrame::FRAME) as i32,
        msite_busy: (offset_of!(rt::MethodSite, frame) + rt::SiteFrame::BUSY) as i32,
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
    // Strings: our own header, strong count first.
    let s = Value::str("probe");
    let sp = unsafe { ((&s as *const Value as *const u8).add(tag::PAYLOAD as usize) as *const usize).read() };
    let s2 = s.clone();
    let ok = unsafe { (sp as *const usize).read() } == 2;
    drop(s2);
    if !ok || unsafe { (sp as *const usize).read() } != 1 {
        return false;
    }
    let v = Value::array(crate::value::PhpArray::new());
    let Value::Array(rc) = &v else { return false };
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

thread_local! {
    /// The thread's compiled code: kept from one request's interpreter to
    /// the next (functions are keyed by their syntax tree, which the
    /// thread's parse cache keeps), so a server worker compiles a hot
    /// function once, not once per request.
    static THREAD_NATIVE: std::rc::Rc<std::cell::RefCell<Option<Native>>> = std::rc::Rc::new(std::cell::RefCell::new(Native::from_env()));
}

/// The thread's native compiler, its inline caches emptied for a new
/// interpreter (they point at the previous one's classes and functions).
pub fn for_new_interp() -> std::rc::Rc<std::cell::RefCell<Option<Native>>> {
    THREAD_NATIVE.with(|n| {
        if let Ok(mut g) = n.try_borrow_mut() {
            if let Some(n) = g.as_mut() {
                n.reset_caches();
            }
        }
        n.clone()
    })
}

impl Native {
    fn reset_caches(&mut self) {
        self.sites.iter().for_each(|s| s.reset());
        self.msites.iter().for_each(|s| s.reset());
        self.psites.iter().for_each(|s| s.reset());
        self.csites.iter().for_each(|s| s.reset());
        rt::reset_dyn_sites();
    }

    pub fn from_env() -> Option<Native> {
        let mode = std::env::var("PHPX_NATIVE").unwrap_or_default();
        let threshold = match mode.as_str() {
            "off" | "0" => return None,
            "all" => 1,
            // Hot functions only: compiling costs about a millisecond, so
            // cold code (most of a framework's boot) stays interpreted.
            // Long loops switch over mid-call (on-stack replacement).
            _ => std::env::var("PHPX_NATIVE_THRESHOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_THRESHOLD),
        };
        Native::new(threshold)
    }

    fn new(threshold: u32) -> Option<Native> {
        let mut flags = settings::builder();
        flags.set("use_colocated_libcalls", "false").ok()?;
        flags.set("is_pic", "false").ok()?;
        // The IR verifier roughly doubles compile time; PHPX_NATIVE_VERIFY=1
        // turns it back on when debugging the lowering.
        flags.set("enable_verifier", if std::env::var("PHPX_NATIVE_VERIFY").is_ok() { "true" } else { "false" }).ok()?;
        flags.set("opt_level", "speed").ok()?;
        // The sampling profiler (PHPX_PROFILE) walks frame pointers through
        // compiled code.
        if std::env::var("PHPX_PROFILE").is_ok() {
            flags.set("preserve_frame_pointers", "true").ok()?;
        }
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
            compiled: Default::default(),
            hot: Default::default(),
            threshold,
            osr_info: HashMap::new(),
            osr: HashMap::new(),
            compile_ns: 0,
            consts: vec![],
            sites: vec![],
            msites: vec![],
            psites: vec![],
            csites: vec![],
            specs: vec![],
            log: std::env::var("PHPX_NATIVE_LOG").is_ok(),
            max: std::env::var("PHPX_NATIVE_MAX").ok().and_then(|v| v.parse().ok()),
            skip: std::env::var("PHPX_NATIVE_SKIP").map(|v| v.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect()).unwrap_or_default(),
            count: 0,
        })
    }

    /// Compiled entry for `fn_id`, compiling it (described by `mk`) when hot.
    /// None = interpret.
    fn entry_for(&mut self, it: &Interp, fn_id: usize, mk: impl FnOnce() -> Target) -> Option<Entry> {
        if let Some(e) = self.compiled.get(&fn_id) {
            return *e;
        }
        let (n, checked) = self.hot.entry(fn_id).or_default();
        *n += 1;
        if *n < self.threshold {
            // Remembered for on-stack replacement of its loops.
            if !*checked {
                *checked = true;
                let t = mk();
                if matches!(t.body, Body::Block(b) if has_loop(b)) {
                    self.osr_info.insert(fn_id, t);
                }
            }
            return None;
        }
        let t = &mk();
        // A compiler bug must never take the program down: treat a panic
        // as "not compiled" and keep interpreting.
        let r = self.timed(|n| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| n.compile(it, t))).unwrap_or(Err("internal compiler error")));
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
        self.compile_mode(it, t, false, None)
    }

    /// CPU time spent compiling, for PHPX_NATIVE_LOG.
    fn timed<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        if !self.log {
            return f(self);
        }
        let start = thread_cpu();
        let r = f(self);
        self.compile_ns += thread_cpu() - start;
        r
    }

    /// `top_level`: a file segment. Returns exit with status 3 (value in
    /// out); every exit writes locals back to the frame.
    /// `osr`: compile an entry that starts at that loop statement (locals
    /// come from the interpreter frame).
    fn compile_mode(&mut self, it: &Interp, t: &Target, top_level: bool, osr: Option<usize>) -> Result<Entry, &'static str> {
        // Compiling is the engine's work, not the script's (memory_limit).
        crate::alloc::uncharged(|| self.compile_mode_inner(it, t, top_level, osr))
    }

    fn compile_mode_inner(&mut self, _it: &Interp, t: &Target, top_level: bool, osr: Option<usize>) -> Result<Entry, &'static str> {
        if self.skip.iter().any(|s| t.display.contains(s.as_str())) || self.max.is_some_and(|m| self.count >= m) {
            return Err("excluded (PHPX_NATIVE_MAX / PHPX_NATIVE_SKIP)");
        }
        let mut a = analyze::analyze_mode(t.params, &t.captured, t.body, top_level)?;
        let escapes = std::mem::take(&mut a.escapes);
        if top_level && a.uses_this {
            // A file included from a method sees its `$this`; file code
            // binds frame variables, which `$this` is not.
            return Err("$this in included file code");
        }
        self.count += 1;
        let this_slot = if a.uses_this { Some(a.locals.slot("this") as usize) } else { None };
        let nlocals = a.locals.names.len();
        let spec = Box::new(LocalsSpec {
            names: a.locals.names.clone(),
            this_slot,
            params: t.params,
            display: t.display.clone(),
            ctx: t.ctx.clone(),
            class: t.class.clone(),
            simple: t.captured.is_empty() && !a.func_args && t.params.iter().all(|p| p.ty.is_none() && !p.by_ref && !p.variadic && p.promote.is_none()),
            func_args: a.func_args,
            captured_slots: t.captured.iter().map(|c| a.locals.index[c] as usize).collect(),
            ret: t.ret,
            ret_quick: rt::Quick::of(t.ret),
            quick: t.params.iter().map(|p| rt::Quick::of(p.ty.as_ref())).collect(),
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
            // The running interpreter's position (diagnostics): found
            // through cx, so the code serves every request's interpreter.
            let itp = b.ins().load(types::I64, cranelift_codegen::ir::MemFlagsData::trusted(), cxv, std::mem::offset_of!(rt::Cx, it) as i32);
            let pos_addr = b.ins().iadd_imm_s(itp, std::mem::offset_of!(Interp, pos) as i64);
            b.ins().jump(init, &[]);
            b.switch_to_block(start);
            let mut l = lower::Lower::new(
                b, &mut self.module, &self.helpers, &a.locals, locals_slot, exit_ok, unwind, cxv, outv, pos_addr, t.ctx.file as u32,
                &mut consts, &mut sites,
            );
            l.osr_target = osr;
            // The call-stack record (functions and methods, not files; an
            // OSR entry continues a call the interpreter already recorded).
            // (Pushed in `init`, before parameters bind; popped on every exit.)
            let call_rec = (!top_level && osr.is_none()).then(|| {
                l.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, std::mem::size_of::<crate::trace::CallRec>() as u32, 3))
            });
            let top_rcs = if top_level {
                l.exit_ret = Some(exit_ret);
                let n = (nlocals.max(1) * 8) as u32;
                l.top_ptrs = Some(l.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, n, 3)));
                Some(l.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, n, 3)))
            } else {
                None
            };
            l.spec_addr = spec_addr;
            l.top_rcs = top_rcs;
            l.top_def = top_level.then(|| l.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, nlocals.max(1) as u32, 0)));
            let top_def = l.top_def;
            l.top_epoch = top_level.then(|| l.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3)));
            let top_epoch = l.top_epoch;
            l.spec_addr = spec_addr;
            l.escapes = escapes;
            match t.body {
                Body::Block(stmts) => {
                    l.stmts(stmts)?;
                    if !l.finished() {
                        l.b.ins().jump(exit_ok, &[]);
                    }
                }
                Body::Expr(e) => {
                    // An arrow fn's body is one expression: diagnostics in
                    // it point at its line.
                    l.set_pos(e.span.start);
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
                if status == 0 && t.ret.is_some() {
                    // Declared return type (coerces the value in place).
                    use cranelift_codegen::ir::condcodes::IntCC;
                    let sp = l.b.ins().iconst(types::I64, spec_addr);
                    let st = l.call("rt_check_ret", &[cxv, outv, sp]).unwrap();
                    let bad = l.b.ins().icmp_imm_s(IntCC::NotEqual, st, 0);
                    let ok = l.b.create_block();
                    l.b.ins().brif(bad, unwind, &[], ok, &[]);
                    l.b.switch_to_block(ok);
                }
                if let Some(slot) = call_rec {
                    let rec = l.b.ins().stack_addr(types::I64, slot, 0);
                    l.call("rt_call_pop", &[rec]);
                }
                l.cleanup(nlocals);
                if let (Some(rcs), Some(def)) = (top_rcs, top_def) {
                    let ra = l.b.ins().stack_addr(types::I64, rcs, 0);
                    let da = l.b.ins().stack_addr(types::I64, def, 0);
                    let sp = l.b.ins().iconst(types::I64, spec_addr);
                    l.call("rt_unbind_slots", &[cxv, ra, da, sp]);
                }
                let s = l.b.ins().iconst(types::I64, status);
                l.b.ins().return_(&[s]);
            }
            l.b.switch_to_block(decline);
            if let Some(slot) = call_rec {
                let rec = l.b.ins().stack_addr(types::I64, slot, 0);
                l.call("rt_call_pop", &[rec]);
            }
            let two = l.b.ins().iconst(types::I64, 2);
            l.b.ins().return_(&[two]);
            // Init: empty every slot, then load parameters.
            l.b.switch_to_block(init);
            l.init_slots(nlocals);
            if let Some(slot) = call_rec {
                let rec = l.b.ins().stack_addr(types::I64, slot, 0);
                let sp = l.b.ins().iconst(types::I64, spec_addr);
                let locals = l.b.ins().stack_addr(types::I64, locals_slot, 0);
                l.call("rt_call_push", &[cxv, sp, locals, rec]);
            }
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
                    let da = l.b.ins().stack_addr(types::I64, top_def.unwrap(), 0);
                    let st = l.call("rt_bind_slots", &[cxv, pa, ra, da, sp]).unwrap();
                    let ep = l.b.ins().iconst(types::I64, crate::interp::scope_epoch_addr() as i64);
                    let now = l.b.ins().load(types::I64, cranelift_codegen::ir::MemFlagsData::trusted(), ep, 0);
                    l.b.ins().stack_store(types::I64, now, top_epoch.unwrap(), 0);
                    st
                }
                _ => {
                    let la = l.b.ins().stack_addr(types::I64, locals_slot, 0);
                    l.call("rt_load_locals", &[cxv, la, sp]).unwrap()
                }
            };
            l.b.ins().jump(bound, &[st.into()]);
            l.b.switch_to_block(from_args);
            let nparams = t.params.len();
            let lay = layout().filter(|_| inline_rc());
            if simple && (this_slot.is_none() || lay.is_some()) {
                // Exact arg count: move args into the parameter slots inline.
                let (inline_b, helper_b) = (l.b.create_block(), l.b.create_block());
                let exact = l.b.ins().icmp_imm_s(IntCC::Equal, nargsv, nparams as i64);
                l.b.ins().brif(exact, inline_b, &[], helper_b, &[]);
                l.b.switch_to_block(inline_b);
                let flags = cranelift_codegen::ir::MemFlagsData::trusted();
                if let (Some(ts), Some(lay)) = (this_slot, lay) {
                    // `$this` from the frame (decline without one), with its
                    // reference count bumped inline.
                    let frame = l.b.ins().load(types::I64, flags, cxv, lay.cx_frame);
                    let raw = l.b.ins().load(types::I64, flags, frame, lay.frame_this);
                    let have = l.b.create_block();
                    l.b.ins().brif(raw, have, &[], decline, &[]);
                    l.b.switch_to_block(have);
                    let rc = l.b.ins().load(types::I64, flags, raw, 0);
                    let rc = l.b.ins().iadd_imm_s(rc, 1);
                    l.b.ins().store(flags, rc, raw, 0);
                    let dst = l.b.ins().stack_addr(types::I64, locals_slot, (ts * 16) as i32);
                    let t = l.b.ins().iconst(types::I64, tag::OBJECT as i64);
                    l.b.ins().store(flags, t, dst, 0);
                    l.b.ins().store(flags, raw, dst, tag::PAYLOAD);
                }
                let null = l.b.ins().iconst(types::I64, tag::NULL as i64);
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
            match osr {
                Some(_) => {
                    let Some(head) = l.osr_entry else { return Err("loop not enterable (inside foreach, switch or try)") };
                    l.b.ins().jump(head, &[]);
                }
                None => {
                    l.b.ins().jump(start, &[]);
                }
            }
            l.b.seal_all_blocks();
            let msites = std::mem::take(&mut l.msites);
            let psites = std::mem::take(&mut l.psites);
            let csites = std::mem::take(&mut l.csites);
            l.b.finalize(cfg);
            self.msites.extend(msites);
            self.psites.extend(psites);
            self.csites.extend(csites);
        }
        // Register allocation grows faster than linearly: huge bodies (a
        // whole page template) stay interpreted.
        let insts = ctx.func.dfg.num_insts();
        if self.log {
            eprintln!("[native] {}: {insts} instructions", t.display);
        }
        if insts > MAX_INSTS {
            return Err("function too large");
        }
        // PHPX_NATIVE_DUMP=name: print the IR and machine code of the
        // functions whose display name contains `name`.
        let dump = std::env::var("PHPX_NATIVE_DUMP").is_ok_and(|d| !d.is_empty() && t.display.contains(d.as_str()));
        if dump {
            eprintln!("[native] IR of {}:\n{}", t.display, ctx.func.display());
            ctx.set_disasm(true);
        }
        self.module.define_function(id, &mut ctx).map_err(|e| {
            if self.log {
                eprintln!("[native] codegen error: {e:?}");
            }
            "codegen failed"
        })?;
        if let Some(vcode) = ctx.compiled_code().and_then(|c| c.vcode.as_ref()).filter(|_| dump) {
            eprintln!("[native] code of {}:\n{vcode}", t.display);
        }
        let size = ctx.compiled_code().map_or(0, |c| c.code_info().total_size);
        self.module.finalize_definitions().map_err(|_| "finalize failed")?;
        let ptr = self.module.get_finalized_function(id);
        perf_map(ptr, size, &t.display);
        self.consts.extend(consts);
        self.sites.extend(sites);
        self.specs.push(spec);
        let _ = &mut a;
        // SAFETY: generated with exactly the Entry signature.
        Ok(unsafe { std::mem::transmute::<*const u8, Entry>(ptr) })
    }
}

/// Address ranges of compiled code (start, end, PHP name), recorded while
/// the in-process profiler runs (PHPX_PROFILE), which can't symbolize them.
static JIT_SYMBOLS: std::sync::Mutex<Vec<(usize, usize, String)>> = std::sync::Mutex::new(Vec::new());

/// The PHP function whose compiled code contains `ip`, with the offset.
pub fn jit_symbol(ip: usize) -> Option<(String, usize)> {
    let syms = JIT_SYMBOLS.lock().ok()?;
    syms.iter().find(|(s, e, _)| (*s..*e).contains(&ip)).map(|(s, _, n)| (n.clone(), ip - s))
}

/// PHPX_PERF_MAP=1: append each compiled function to /tmp/perf-PID.map,
/// so `perf report` names generated code by its PHP function.
fn perf_map(ptr: *const u8, size: u32, name: &str) {
    use std::io::Write;
    thread_local! {
        static PROFILING: bool = std::env::var("PHPX_PROFILE").is_ok();
    }
    if PROFILING.with(|p| *p) {
        if let Ok(mut syms) = JIT_SYMBOLS.lock() {
            syms.push((ptr as usize, ptr as usize + size as usize, name.to_string()));
        }
    }
    thread_local! {
        static MAP: Option<std::cell::RefCell<std::fs::File>> = std::env::var("PHPX_PERF_MAP").is_ok().then(|| {
            let path = format!("/tmp/perf-{}.map", std::process::id());
            std::cell::RefCell::new(std::fs::OpenOptions::new().create(true).append(true).open(path).expect("perf map"))
        });
    }
    MAP.with(|m| {
        if let Some(f) = m {
            let _ = writeln!(f.borrow_mut(), "{:x} {:x} php::{name}", ptr as usize, size);
        }
    });
}

fn thread_cpu() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

impl Drop for Native {
    fn drop(&mut self) {
        if self.log {
            eprintln!("[native] {} entries compiled in {:.1} ms CPU", self.count, self.compile_ns as f64 / 1e6);
        }
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
        let null = self.b.ins().iconst(types::I64, tag::NULL as i64);
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
                        ret: None,
                    };
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| n.compile_mode(self, &t, true, None)))
                        .unwrap_or(Err("internal compiler error"));
                    if n.log {
                        match &r {
                            Ok(_) => eprintln!("[native] top-level of {} compiled", self.file_path(f.ctx.file)),
                            Err(why) => eprintln!("[native] top-level of {}: not compiled: {why}", self.file_path(f.ctx.file)),
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

    /// On-stack replacement: an interpreted loop that ran long continues
    /// (with the rest of its function) in native code. Some(result of the
    /// function) when it did; None = keep interpreting.
    pub fn try_osr(&self, f: &mut crate::interp::Frame, loop_stmt: &'static ast::Stmt) -> Option<R<Value>> {
        if f.fn_id == 0 || f.gen.is_some() {
            return None;
        }
        let key = loop_stmt as *const ast::Stmt as usize;
        let entry = {
            let mut guard = self.native.try_borrow_mut().ok()?;
            let n = guard.as_mut()?;
            match n.osr.get(&key) {
                Some(e) => (*e)?,
                None => {
                    let t = n.osr_info.get(&f.fn_id)?.clone();
                    let r = n.timed(|n| std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| n.compile_mode(self, &t, false, Some(key)))).unwrap_or(Err("internal compiler error")));
                    if n.log {
                        match &r {
                            Ok(_) => eprintln!("[native] {} compiled (loop entry)", t.display),
                            Err(why) => eprintln!("[native] {} loop entry: not compiled: {why}", t.display),
                        }
                    }
                    let e = r.ok();
                    n.osr.insert(key, e);
                    e?
                }
            }
        };
        let mut cx = Cx { it: self as *const Interp, frame: f as *mut _ };
        let mut out = Value::Null;
        // SAFETY: entry follows the Entry ABI (no args: locals from the frame).
        let st = unsafe { entry(&mut cx, std::ptr::null_mut(), 0, &mut out) };
        match st {
            0 => Some(Ok(out)),
            1 => Some(Err(self.pending.borrow_mut().take().expect("native error without pending exception"))),
            _ => {
                // Declined (shared variables): never retry this loop.
                if let Ok(mut g) = self.native.try_borrow_mut() {
                    if let Some(n) = g.as_mut() {
                        n.osr.insert(key, None);
                    }
                }
                None
            }
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
        self.run_native_entry(entry, f)
    }

    /// Run a compiled entry whose parameters are already bound in `f`.
    pub fn run_native_entry(&self, entry: Entry, f: &mut crate::interp::Frame) -> Option<R<Value>> {
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
