//! AST -> Cranelift IR for the native compiler.
//!
//! Values live in 16-byte stack slots (locals and temporaries) with the
//! `Value` layout. Expressions yield operands: unboxed int/float/bool in
//! registers, a borrowed local or constant, or an owned temporary. Int fast
//! paths are inline; everything else calls a helper in `rt.rs`.

use std::collections::HashMap;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{types, Block, FuncRef, InstBuilder, MemFlagsData, StackSlot, StackSlotData, StackSlotKind, Value as CValue};
use cranelift_frontend::FunctionBuilder;
use cranelift_jit::JITModule;
use cranelift_module::{FuncId, Module};
use phpx_syntax::ast::{self, BinOp, ClassRef, Expr, ExprKind, MemberName, Stmt, StmtKind, UnOp};

use super::analyze::Locals;
use super::rt::{binop_code, CallSite};
use crate::value::{tag, Value};

pub type Fail = &'static str;
type L<T> = Result<T, Fail>;

/// Root of a write target: a local's address, or a temp holding the object.
struct Root {
    addr: CValue,
    temp: Option<Op>,
}

#[derive(Clone, Copy)]
pub enum Op {
    Int(CValue),
    Float(CValue),
    /// i8 0/1.
    Bool(CValue),
    /// Borrowed local variable.
    Local(u32),
    /// Pointer to a Value: borrowed constant (None) or owned temp (Some).
    Ptr(CValue, Option<usize>),
}

pub struct Lower<'a> {
    pub b: FunctionBuilder<'a>,
    pub module: &'a mut JITModule,
    pub helpers: &'a HashMap<&'static str, FuncId>,
    pub frefs: HashMap<&'static str, FuncRef>,
    pub locals: &'a Locals,
    pub locals_slot: StackSlot,
    /// Top level: locals live in the frame's variable cells (so `global`
    /// and `$GLOBALS` in called functions see them); this slot holds a
    /// pointer to each cell's Value.
    pub top_ptrs: Option<StackSlot>,
    /// Top level: each local's cell (strong references), for handing the
    /// scope to the interpreter (`escapes`).
    pub top_rcs: Option<StackSlot>,
    /// Top level: one byte per local, set once it is assigned (a variable
    /// bound to no frame variable joins the frame only then).
    pub top_def: Option<StackSlot>,
    /// Top level: the scope epoch the cells were last checked against.
    pub top_epoch: Option<StackSlot>,
    /// Set while lowering the object of a chain element: where a `?->`
    /// below jumps when its object is null.
    chain_skip: Option<Block>,
    pub spec_addr: i64,
    /// Expressions and statements the interpreter runs (see Analysis).
    pub escapes: std::collections::HashSet<usize>,
    /// Every temporary location (slot, offset), for init and cleanup.
    pub temps: Vec<(StackSlot, i32)>,
    free: Vec<usize>,
    /// Iterator pointer slots (foreach), freed on every exit.
    pub iters: Vec<StackSlot>,
    /// (continue, break, handler depth) per enclosing loop / switch.
    loops: Vec<(Block, Block, usize)>,
    /// Enclosing try blocks, innermost last.
    handlers: Vec<Handler>,
    /// Stashed in-flight exceptions (one slot per finally), freed on exit.
    pub stashes: Vec<StackSlot>,
    pub exit_ok: Block,
    pub unwind: Block,
    pub cx: CValue,
    pub out: CValue,
    pub pos_addr: CValue,
    file: u32,
    terminated: bool,
    /// Constant Values (strings, null) referenced by address; kept alive by the caller.
    pub consts: &'a mut Vec<Box<Value>>,
    /// Call sites referenced by address; kept alive by the caller.
    pub sites: &'a mut Vec<Box<CallSite>>,
    pub msites: Vec<Box<super::rt::MethodSite>>,
    pub psites: Vec<Box<super::rt::PropSite>>,
    pub csites: Vec<Box<super::rt::ClassSite>>,
    /// Top-level code: `return` exits through this block (status 3).
    pub exit_ret: Option<Block>,
    /// Per pending call (innermost last): arguments to write back to
    /// their place if the callee took them by reference.
    write_backs: Vec<Vec<WriteBack>>,
    /// On-stack replacement: the loop statement to enter at, and its
    /// header block once lowered.
    pub osr_target: Option<usize>,
    pub osr_entry: Option<Block>,
    /// Inside a construct an OSR entry can't jump into (foreach holds an
    /// iterator, switch a subject, try a handler).
    no_osr: usize,
    /// Start of the statement being lowered.
    stmt_start: u32,
    /// Speculation (see `spec`): the plan of the loop being specialised,
    /// set while its specialised body is lowered.
    spec: Option<spec::Plan>,
    spec_on: bool,
    /// Generic code's entry block of each statement a fallback can enter.
    spec_entries: HashMap<usize, Block>,
    /// Statements already part of a speculated loop.
    spec_done: std::collections::HashSet<usize>,
    /// On-stack replacement into a speculated loop: (its entry, the block
    /// that continues at the generic entry once that exists).
    spec_osr: Option<(Block, Block)>,
    /// Generic code's block right after each statement, for speculative
    /// code that completed it itself.
    spec_after: HashMap<usize, Block>,
    /// Inside a speculated call statement: the reads made before the call
    /// (by node address) and the call's float result.
    pre: Option<(HashMap<usize, CValue>, CValue)>,
    /// Inside speculated code: object locals already checked in this run of
    /// statements, with their shape, slot storage and the calling scope
    /// (see `spec::ObjFact`).
    obj_facts: HashMap<u32, spec::ObjFact>,
    /// The interpreter compiling (for looking up functions to inline) and
    /// the namespace context of the code.
    pub interp: *const crate::interp::Interp,
    pub ctx: Option<std::rc::Rc<crate::interp::NsCtx>>,
}

mod spec;

/// A property or array-element argument (`sort($this->items)`): passed
/// through a temp the runtime binds by reference, then stored back.
struct WriteBack {
    pos: usize,
    target: &'static Expr,
    temp: usize,
}

/// What's known at compile time about which arguments a call may take by
/// reference.
#[derive(Clone, Copy)]
enum ByRef {
    /// None of them (a built-in without by-reference parameters).
    None,
    /// A built-in's by-reference parameter positions.
    Builtin(&'static [usize]),
    /// Unknown until run time (user functions and methods).
    Unknown,
}

impl ByRef {
    fn for_function(name: &str) -> ByRef {
        let l = name.trim_start_matches('\\').to_ascii_lowercase();
        let refs = crate::call::by_ref_params(&l);
        if !refs.is_empty() {
            ByRef::Builtin(refs)
        } else if crate::arginfo::spec(&l).is_some() {
            ByRef::None
        } else {
            ByRef::Unknown
        }
    }

    fn may_be_ref(self, i: usize) -> bool {
        match self {
            ByRef::None => false,
            ByRef::Builtin(refs) => refs.contains(&i),
            ByRef::Unknown => true,
        }
    }
}

/// A chain element (property, method call, element read) with a `?->`
/// somewhere in the chain below it.
fn nullsafe_below(e: &Expr) -> bool {
    fn has(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Prop { object, nullsafe, .. } | ExprKind::MethodCall { object, nullsafe, .. } => *nullsafe || has(object),
            ExprKind::Index { base, .. } => has(base),
            _ => false,
        }
    }
    match &e.kind {
        ExprKind::Prop { object, .. } | ExprKind::MethodCall { object, .. } => has(object),
        ExprKind::Index { base, index: Some(_) } => has(base),
        _ => false,
    }
}

/// A write target that can be evaluated again without side effects.
fn pure_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Var(_) => true,
        ExprKind::StaticProp { class: ClassRef::Name(_), name: MemberName::Ident(_) } => true,
        ExprKind::Prop { object, name: MemberName::Ident(_), nullsafe: false } => pure_operand(object),
        ExprKind::Index { base, index: Some(i) } => pure_place(base) && pure_operand(i),
        _ => false,
    }
}

fn pure_operand(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bytes(_) | ExprKind::Bool(_) | ExprKind::Null | ExprKind::Name(_) => true,
        _ => pure_place(e),
    }
}

/// An enclosing `try`: where its exceptions went before it, and the
/// `finally` that every exit through it must run.
#[derive(Clone, Copy)]
struct Handler {
    finally: Option<&'static [Stmt]>,
    outer_unwind: Block,
}

fn flags() -> MemFlagsData {
    MemFlagsData::trusted()
}

impl<'a> Lower<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        b: FunctionBuilder<'a>,
        module: &'a mut JITModule,
        helpers: &'a HashMap<&'static str, FuncId>,
        locals: &'a Locals,
        locals_slot: StackSlot,
        exit_ok: Block,
        unwind: Block,
        cx: CValue,
        out: CValue,
        pos_addr: CValue,
        file: u32,
        consts: &'a mut Vec<Box<Value>>,
        sites: &'a mut Vec<Box<CallSite>>,
    ) -> Self {
        Lower {
            b,
            module,
            helpers,
            frefs: HashMap::new(),
            locals,
            locals_slot,
            top_ptrs: None,
            top_rcs: None,
            top_def: None,
            top_epoch: None,
            chain_skip: None,
            spec_addr: 0,
            escapes: Default::default(),
            temps: vec![],
            free: vec![],
            iters: vec![],
            loops: vec![],
            handlers: vec![],
            stashes: vec![],
            exit_ok,
            unwind,
            cx,
            out,
            pos_addr,
            file,
            terminated: false,
            consts,
            sites,
            msites: vec![],
            psites: vec![],
            csites: vec![],
            exit_ret: None,
            write_backs: vec![],
            osr_target: None,
            osr_entry: None,
            no_osr: 0,
            stmt_start: u32::MAX,
            spec: None,
            spec_on: false,
            obj_facts: HashMap::new(),
            spec_entries: HashMap::new(),
            spec_done: Default::default(),
            spec_osr: None,
            spec_after: HashMap::new(),
            pre: None,
            interp: std::ptr::null(),
            ctx: None,
        }
    }

    /// `head` is where an OSR entry into loop `s` lands.
    fn osr_mark(&mut self, s: &'static Stmt, head: Block) {
        if self.no_osr == 0 && self.osr_target == Some(s as *const Stmt as usize) {
            self.osr_entry = Some(head);
        }
    }

    // ---- plumbing -------------------------------------------------------------

    pub fn call(&mut self, name: &'static str, args: &[CValue]) -> Option<CValue> {
        let fref = match self.frefs.get(name) {
            Some(f) => *f,
            None => {
                let id = self.helpers[name];
                let f = self.module.declare_func_in_func(id, self.b.func);
                self.frefs.insert(name, f);
                f
            }
        };
        let inst = self.b.ins().call(fref, args);
        self.b.inst_results(inst).first().copied()
    }

    /// Call a status-returning helper; nonzero status unwinds.
    fn call_checked(&mut self, name: &'static str, args: &[CValue]) {
        let st = self.call(name, args).unwrap();
        let bad = self.b.ins().icmp_imm_s(IntCC::NotEqual, st, 0);
        let cont = self.b.create_block();
        self.b.ins().brif(bad, self.unwind, &[], cont, &[]);
        self.b.switch_to_block(cont);
    }

    fn iconst(&mut self, v: i64) -> CValue {
        self.b.ins().iconst(types::I64, v)
    }

    fn local_addr(&mut self, i: u32) -> CValue {
        match self.top_ptrs {
            Some(p) => self.b.ins().stack_load(types::I64, types::I64, p, (i * 8) as i32),
            None => self.b.ins().stack_addr(types::I64, self.locals_slot, (i * 16) as i32),
        }
    }

    /// Top level: local `i` now holds an assigned value.
    fn mark_defined(&mut self, i: u32) {
        if let Some(def) = self.top_def {
            let one = self.b.ins().iconst(types::I8, 1);
            self.b.ins().stack_store(types::I64, one, def, i as i32);
        }
    }

    fn alloc_temp(&mut self) -> usize {
        if let Some(t) = self.free.pop() {
            debug_assert!(!self.free.contains(&t), "temp {t} on the free list twice");
            return t;
        }
        let slot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 4));
        self.temps.push((slot, 0));
        self.temps.len() - 1
    }

    /// `n` contiguous temps (for argument / key arrays).
    fn alloc_array(&mut self, n: usize) -> (StackSlot, Vec<usize>) {
        let slot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (n.max(1) * 16) as u32, 4));
        let ids = (0..n)
            .map(|i| {
                self.temps.push((slot, (i * 16) as i32));
                self.temps.len() - 1
            })
            .collect();
        (slot, ids)
    }

    fn temp_addr(&mut self, t: usize) -> CValue {
        let (slot, off) = self.temps[t];
        self.b.ins().stack_addr(types::I64, slot, off)
    }

    fn const_value(&mut self, v: Value) -> CValue {
        let b = Box::new(v);
        let addr = &*b as *const Value as i64;
        self.consts.push(b);
        self.iconst(addr)
    }

    fn addr(&mut self, op: Op) -> Option<CValue> {
        match op {
            Op::Local(i) => Some(self.local_addr(i)),
            Op::Ptr(p, _) => Some(p),
            _ => None,
        }
    }

    fn load_tag(&mut self, addr: CValue) -> CValue {
        self.b.ins().load(types::I8, flags(), addr, 0)
    }

    fn store_tag(&mut self, addr: CValue, t: u8) {
        // The tag and its padding as one word: later 8-byte copies of the
        // Value then read it straight from the store buffer.
        let v = self.b.ins().iconst(types::I64, t as i64);
        self.b.ins().store(flags(), v, addr, 0);
    }

    pub fn clear_slot(&mut self, addr: CValue) {
        self.clear(addr);
    }

    /// Drop whatever heap value is in `addr`, leaving it empty. With inline
    /// refcounting, only the last reference calls into Rust to free.
    fn clear(&mut self, addr: CValue) {
        let t = self.load_tag(addr);
        let heap = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, t, tag::STR as i64);
        let (drop_b, cont) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(heap, drop_b, &[], cont, &[]);
        self.b.switch_to_block(drop_b);
        if super::inline_rc() {
            let rc = self.b.ins().load(types::I64, flags(), addr, tag::PAYLOAD);
            let cnt = self.b.ins().load(types::I64, flags(), rc, 0);
            let last = self.b.ins().icmp_imm_s(IntCC::Equal, cnt, 1);
            let (free_b, dec_b) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(last, free_b, &[], dec_b, &[]);
            self.b.switch_to_block(dec_b);
            let n = self.b.ins().iadd_imm_s(cnt, -1);
            self.b.ins().store(flags(), n, rc, 0);
            self.store_tag(addr, tag::NULL);
            self.b.ins().jump(cont, &[]);
            self.b.switch_to_block(free_b);
        }
        self.call("rt_release", &[addr]);
        self.b.ins().jump(cont, &[]);
        self.b.switch_to_block(cont);
    }

    /// Release an operand after use (owned temps only).
    fn release(&mut self, op: Op) {
        if let Op::Ptr(p, Some(t)) = op {
            self.clear(p);
            debug_assert!(!self.free.contains(&t), "temp {t} released twice");
            self.free.push(t);
        }
    }

    /// Operand as a pointer to a Value (scalars go to a fresh temp).
    fn materialize(&mut self, op: Op) -> Op {
        match op {
            Op::Int(_) | Op::Float(_) | Op::Bool(_) => {
                let t = self.alloc_temp();
                let a = self.temp_addr(t);
                self.store_into(a, op);
                Op::Ptr(a, Some(t))
            }
            other => other,
        }
    }

    /// Write `op` into the empty slot at `dst` (copying borrowed values,
    /// moving owned temps).
    fn store_into(&mut self, dst: CValue, op: Op) {
        match op {
            Op::Int(v) => {
                self.store_tag(dst, tag::INT);
                self.b.ins().store(flags(), v, dst, tag::PAYLOAD);
            }
            Op::Float(v) => {
                self.store_tag(dst, tag::FLOAT);
                self.b.ins().store(flags(), v, dst, tag::PAYLOAD);
            }
            Op::Bool(v) => {
                self.store_tag(dst, tag::BOOL);
                self.b.ins().store(flags(), v, dst, tag::PAYLOAD);
            }
            Op::Local(_) | Op::Ptr(_, None) => {
                let src = self.addr(op).unwrap();
                let t = self.load_tag(src);
                let heap = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, t, tag::STR as i64);
                let (slow, fast, cont) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().brif(heap, slow, &[], fast, &[]);
                self.b.switch_to_block(fast);
                self.copy16(dst, src);
                self.b.ins().jump(cont, &[]);
                self.b.switch_to_block(slow);
                if super::inline_rc() && !matches!(op, Op::Local(_)) {
                    // Constants hold no Refs: bump the count and copy.
                    self.rc_inc(src);
                    self.copy16(dst, src);
                } else if super::inline_rc() {
                    // A local can't hold a Ref in compiled code either (v1
                    // rejects anything that would create one).
                    self.rc_inc(src);
                    self.copy16(dst, src);
                } else {
                    self.call("rt_copy", &[dst, src]);
                }
                self.b.ins().jump(cont, &[]);
                self.b.switch_to_block(cont);
            }
            Op::Ptr(src, Some(t)) => {
                self.copy16(dst, src);
                self.store_tag(src, tag::NULL);
                self.free.push(t);
            }
        }
    }

    /// Increment the strong count of the Rc in the heap value at `src`.
    fn rc_inc(&mut self, src: CValue) {
        let rc = self.b.ins().load(types::I64, flags(), src, tag::PAYLOAD);
        let cnt = self.b.ins().load(types::I64, flags(), rc, 0);
        let n = self.b.ins().iadd_imm_s(cnt, 1);
        self.b.ins().store(flags(), n, rc, 0);
    }

    fn copy16(&mut self, dst: CValue, src: CValue) {
        let a = self.b.ins().load(types::I64, flags(), src, 0);
        let c = self.b.ins().load(types::I64, flags(), src, 8);
        self.b.ins().store(flags(), a, dst, 0);
        self.b.ins().store(flags(), c, dst, 8);
    }

    /// Move the value in slot `src` into local `i` (src left empty, but
    /// not returned to the temp pool).
    fn move_slot_to_local(&mut self, i: u32, src: CValue) {
        self.mark_defined(i);
        let dst = self.local_addr(i);
        self.clear(dst);
        self.copy16(dst, src);
        self.store_tag(src, tag::NULL);
    }

    fn assign_local(&mut self, i: u32, op: Op) {
        if let Op::Local(j) = op {
            if i == j {
                return;
            }
        }
        self.mark_defined(i);
        let dst = self.local_addr(i);
        self.clear(dst);
        self.store_into(dst, op);
    }

    /// Owned temp holding a copy of `op` (for results that must outlive
    /// later writes to the source local).
    fn own(&mut self, op: Op) -> Op {
        match op {
            Op::Local(_) | Op::Ptr(_, None) => {
                let t = self.alloc_temp();
                let a = self.temp_addr(t);
                self.store_into(a, op);
                Op::Ptr(a, Some(t))
            }
            other => other,
        }
    }

    // ---- truthiness / ints --------------------------------------------------------

    fn truthy(&mut self, op: Op) -> CValue {
        match op {
            Op::Int(v) => self.b.ins().icmp_imm_s(IntCC::NotEqual, v, 0),
            Op::Float(v) => {
                let z = self.b.ins().f64const(0.0);
                self.b.ins().fcmp(FloatCC::NotEqual, v, z)
            }
            Op::Bool(v) => v,
            _ => {
                let a = self.addr(op).unwrap();
                let t = self.load_tag(a);
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I8);
                let (is_int_b, not_int, is_bool_b, slow) =
                    (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
                let is_int = self.b.ins().icmp_imm_u(IntCC::Equal, t, tag::INT as i64);
                self.b.ins().brif(is_int, is_int_b, &[], not_int, &[]);
                self.b.switch_to_block(is_int_b);
                let p = self.b.ins().load(types::I64, flags(), a, tag::PAYLOAD);
                let r = self.b.ins().icmp_imm_s(IntCC::NotEqual, p, 0);
                self.b.ins().jump(merge, &[r.into()]);
                self.b.switch_to_block(not_int);
                let is_bool = self.b.ins().icmp_imm_u(IntCC::Equal, t, tag::BOOL as i64);
                self.b.ins().brif(is_bool, is_bool_b, &[], slow, &[]);
                self.b.switch_to_block(is_bool_b);
                let p = self.b.ins().load(types::I8, flags(), a, tag::PAYLOAD);
                self.b.ins().jump(merge, &[p.into()]);
                self.b.switch_to_block(slow);
                let r = self.call("rt_truthy", &[a]).unwrap();
                let r = self.b.ins().ireduce(types::I8, r);
                self.b.ins().jump(merge, &[r.into()]);
                self.b.switch_to_block(merge);
                let r = self.b.block_params(merge)[0];
                self.release(op);
                r
            }
        }
    }

    /// (is_int i8, payload loader) for the int fast path.
    fn int_check(&mut self, op: Op) -> (CValue, Option<CValue>) {
        match op {
            Op::Int(v) => (self.b.ins().iconst(types::I8, 1), Some(v)),
            Op::Float(_) | Op::Bool(_) => (self.b.ins().iconst(types::I8, 0), None),
            _ => {
                let a = self.addr(op).unwrap();
                let t = self.load_tag(a);
                (self.b.ins().icmp_imm_u(IntCC::Equal, t, tag::INT as i64), None)
            }
        }
    }

    fn int_payload(&mut self, op: Op, known: Option<CValue>) -> CValue {
        match (known, self.addr(op)) {
            (Some(v), _) => v,
            (None, Some(a)) => self.b.ins().load(types::I64, flags(), a, tag::PAYLOAD),
            // Float/bool operand: the int path is never taken at runtime.
            (None, None) => self.iconst(0),
        }
    }

    // ---- statements ---------------------------------------------------------------

    pub fn stmts(&mut self, stmts: &'static [Stmt]) -> L<()> {
        for s in stmts {
            if self.terminated {
                break;
            }
            self.stmt(s)?;
        }
        Ok(())
    }

    pub fn set_pos(&mut self, off: u32) {
        if self.file as usize == crate::interp::PRELUDE_FILE {
            // Built-in classes: positions stay at the caller.
            return;
        }
        let p = self.pos_addr;
        let file = self.b.ins().iconst(types::I32, self.file as i64);
        let off = self.b.ins().iconst(types::I32, off as i64);
        self.b.ins().store(flags(), file, p, 0);
        self.b.ins().store(flags(), off, p, 4);
    }

    /// Arrow fn result: write into the caller's out slot.
    pub fn store_result(&mut self, v: Op) {
        let out = self.out;
        self.store_into(out, v);
    }

    pub fn finished(&self) -> bool {
        self.terminated
    }

    pub fn stmt(&mut self, s: &'static Stmt) -> L<()> {
        let key = s as *const Stmt as usize;
        if !self.spec_on {
            // A fallback from speculative code continues here.
            if let Some(b) = self.spec_entries.remove(&key) {
                self.b.ins().jump(b, &[]);
                self.b.switch_to_block(b);
            }
        }
        self.boundary_check();
        self.set_pos(s.span.start);
        self.stmt_start = s.span.start;
        self.epoch_check()?;
        if self.escapes.contains(&(s as *const Stmt as usize)) {
            return self.escape_stmt(s);
        }
        if !self.spec_on && matches!(s.kind, StmtKind::For { .. } | StmtKind::While { .. }) && !self.spec_done.contains(&key) {
            if let Some(plan) = self.spec_plan(s) {
                self.spec_done.extend(spec::plan_stmts(s));
                return self.spec_loop_stmt(s, plan);
            }
        }
        self.stmt_body(s)?;
        if !self.spec_on {
            // Speculative code that finished this statement itself (after a
            // call) continues in the generic code from here.
            if let Some(b) = self.spec_after.remove(&key) {
                if !self.terminated {
                    self.b.ins().jump(b, &[]);
                }
                self.b.switch_to_block(b);
            }
        }
        Ok(())
    }

    /// File code: if variables were unbound or rebound since the cells
    /// were last checked (by a function, an include, ...), follow them.
    fn epoch_check(&mut self) -> L<()> {
        let Some(es) = self.top_epoch else { return Ok(()) };
        let addr = self.iconst(crate::interp::scope_epoch_addr() as i64);
        let now = self.b.ins().load(types::I64, flags(), addr, 0);
        let seen = self.b.ins().stack_load(types::I64, types::I64, es, 0);
        let same = self.b.ins().icmp(IntCC::Equal, now, seen);
        let (refresh, cont) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(same, cont, &[], refresh, &[]);
        self.b.set_cold_block(refresh);
        self.b.switch_to_block(refresh);
        let [pa, ra, da, sp] = self.scope_args()?;
        let ea = self.b.ins().stack_addr(types::I64, es, 0);
        self.call("rt_scope_refresh", &[self.cx, pa, ra, da, sp, ea]);
        self.b.ins().jump(cont, &[]);
        self.b.switch_to_block(cont);
        Ok(())
    }

    /// The scope arguments of the escape helpers: (ptrs, cells, spec).
    fn scope_args(&mut self) -> L<[CValue; 4]> {
        let (Some(ptrs), Some(rcs), Some(def)) = (self.top_ptrs, self.top_rcs, self.top_def) else { return Err("escape outside file code") };
        let pa = self.b.ins().stack_addr(types::I64, ptrs, 0);
        let ra = self.b.ins().stack_addr(types::I64, rcs, 0);
        let da = self.b.ins().stack_addr(types::I64, def, 0);
        let sp = self.iconst(self.spec_addr);
        Ok([pa, ra, da, sp])
    }

    /// An expression the interpreter evaluates on the frame.
    fn escape_expr(&mut self, e: &'static Expr) -> L<Op> {
        let [pa, ra, da, sp] = self.scope_args()?;
        let ep = self.iconst(e as *const Expr as i64);
        self.helper_value("rt_eval_ast", |l| vec![l.cx, ep, pa, ra, da, sp])
    }

    /// A statement the interpreter runs on the frame (a `return` in it
    /// returns from the file).
    fn escape_stmt(&mut self, s: &'static Stmt) -> L<()> {
        let [pa, ra, da, sp] = self.scope_args()?;
        let stp = self.iconst(s as *const Stmt as i64);
        let out = self.out;
        let st = self.call("rt_exec_ast", &[self.cx, stp, pa, ra, da, sp, out]).unwrap();
        let bad = self.b.ins().icmp_imm_s(IntCC::Equal, st, 1);
        let (ok, ret_chk) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(bad, self.unwind, &[], ret_chk, &[]);
        self.b.switch_to_block(ret_chk);
        let ret = self.b.ins().icmp_imm_s(IntCC::Equal, st, 3);
        let target = self.exit_ret.unwrap_or(self.exit_ok);
        self.b.ins().brif(ret, target, &[], ok, &[]);
        self.b.switch_to_block(ok);
        Ok(())
    }

    /// Before a call: point diagnostics and the call's stack frame at
    /// `off` (PHP's line for it), unless the statement's own position
    /// already says so and no nested call can have moved it.
    fn mark_call(&mut self, off: u32, args: &[ast::Arg]) {
        let simple = |e: &Expr| matches!(e.kind, ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Null | ExprKind::Var(_));
        if off != self.stmt_start || !args.iter().all(|a| simple(&a.value)) {
            self.set_pos(off);
        }
    }

    /// Queued destructors only (after releasing a temporary object).
    fn destruct_check(&mut self) {
        let flag = self.iconst(crate::value::destruct_pending() as *const _ as i64);
        let pending = self.b.ins().load(types::I8, flags(), flag, 0);
        let (run, cont) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(pending, run, &[], cont, &[]);
        self.b.set_cold_block(run);
        self.b.switch_to_block(run);
        self.call_checked("rt_destruct_now", &[self.cx]);
        self.b.ins().jump(cont, &[]);
        self.b.switch_to_block(cont);
    }

    /// Statement boundary: queued destructors, cycle collection, memory and
    /// time limits (one load + branch when nothing is pending).
    fn boundary_check(&mut self) {
        let flag = self.iconst(crate::value::destruct_pending() as *const _ as i64);
        let pending = self.b.ins().load(types::I8, flags(), flag, 0);
        let (run, cont) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(pending, run, &[], cont, &[]);
        self.b.set_cold_block(run);
        self.b.switch_to_block(run);
        self.call_checked("rt_destruct", &[self.cx]);
        self.b.ins().jump(cont, &[]);
        self.b.switch_to_block(cont);
    }

    fn stmt_body(&mut self, s: &'static Stmt) -> L<()> {
        match &s.kind {
            StmtKind::Expr(e) => self.discard(e)?,
            StmtKind::Echo(es) => {
                for e in es {
                    let op = self.expr(e)?;
                    let op = self.materialize(op);
                    let a = self.addr(op).unwrap();
                    self.call_checked("rt_echo", &[self.cx, a]);
                    self.release(op);
                }
            }
            StmtKind::InlineHtml(h) => {
                let c = self.const_value(Value::str(h));
                self.call_checked("rt_echo", &[self.cx, c]);
            }
            StmtKind::Return(e) => {
                match e {
                    Some(e) => {
                        let op = self.expr(e)?;
                        let out = self.out;
                        let finally = self.handlers.iter().any(|h| h.finally.is_some());
                        match op {
                            // Locals are dropped on exit anyway: move, don't
                            // copy (unless a finally may still read them).
                            Op::Local(i) if !finally && self.top_ptrs.is_none() => {
                                let la = self.local_addr(i);
                                self.copy16(out, la);
                                self.store_tag(la, tag::NULL);
                            }
                            op => self.store_into(out, op),
                        }
                    }
                    None => {}
                }
                self.run_finallys(0)?;
                if !self.terminated {
                    let target = self.exit_ret.unwrap_or(self.exit_ok);
                    self.b.ins().jump(target, &[]);
                }
                self.terminated = true;
            }
            // Top level only (the analysis rejects them elsewhere).
            StmtKind::Function(_) | StmtKind::Use { .. } | StmtKind::Declare { body: None, .. } => {}
            StmtKind::Class(c) => {
                let p = self.iconst(&**c as *const ast::Class as i64);
                self.call_checked("rt_declare_class", &[self.cx, p]);
            }
            StmtKind::Const(items) => {
                for (name, e) in items {
                    let v = self.expr(e)?;
                    let v = self.materialize(v);
                    let va = self.addr(v).unwrap();
                    let np = self.iconst(&name.name as *const String as i64);
                    self.call_checked("rt_define_const", &[self.cx, np, va]);
                    self.release(v);
                }
            }
            StmtKind::If { cond, then, elseifs, else_ } => {
                let merge = self.b.create_block();
                let mut falls = false;
                let mut arms: Vec<(&'static Expr, &'static [Stmt])> = vec![(cond, then)];
                arms.extend(elseifs.iter().map(|(c, b)| (c, &b[..])));
                for (c, body) in arms {
                    let op = self.expr(c)?;
                    let t = self.truthy(op);
                    let (yes, no) = (self.b.create_block(), self.b.create_block());
                    self.b.ins().brif(t, yes, &[], no, &[]);
                    self.b.switch_to_block(yes);
                    self.terminated = false;
                    self.stmts(body)?;
                    if !self.terminated {
                        self.b.ins().jump(merge, &[]);
                        falls = true;
                    }
                    self.b.switch_to_block(no);
                    self.terminated = false;
                }
                if let Some(b) = else_ {
                    self.stmts(b)?;
                }
                if !self.terminated {
                    self.b.ins().jump(merge, &[]);
                    falls = true;
                }
                self.b.switch_to_block(merge);
                self.terminated = !falls;
                if !falls {
                    self.b.ins().trap(cranelift_codegen::ir::TrapCode::user(1).unwrap());
                }
            }
            StmtKind::While { cond, body } => {
                let (head, bodyb, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.osr_mark(s, head);
                self.b.ins().jump(head, &[]);
                self.b.switch_to_block(head);
                let op = self.expr(cond)?;
                let t = self.truthy(op);
                self.b.ins().brif(t, bodyb, &[], exit, &[]);
                self.b.switch_to_block(bodyb);
                self.loop_body(body, head, exit)?;
                self.b.switch_to_block(exit);
            }
            StmtKind::DoWhile { body, cond } => {
                let (bodyb, test, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.osr_mark(s, test);
                self.b.ins().jump(bodyb, &[]);
                self.b.switch_to_block(bodyb);
                self.loop_body(body, test, exit)?;
                self.b.switch_to_block(test);
                let op = self.expr(cond)?;
                let t = self.truthy(op);
                self.b.ins().brif(t, bodyb, &[], exit, &[]);
                self.b.switch_to_block(exit);
            }
            StmtKind::For { init, cond, step, body } => {
                for e in init {
                    self.discard(e)?;
                }
                let (head, bodyb, stepb, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.osr_mark(s, head);
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
                self.loop_body(body, stepb, exit)?;
                self.b.switch_to_block(stepb);
                for e in step {
                    self.discard(e)?;
                }
                self.b.ins().jump(head, &[]);
                self.b.switch_to_block(exit);
            }
            StmtKind::Foreach { subject, key, value, body, .. } => {
                // Entered before it starts (the interpreter hasn't evaluated
                // the subject yet): the entry runs the whole statement.
                if self.no_osr == 0 && self.osr_target == Some(s as *const Stmt as usize) {
                    let head = self.b.create_block();
                    self.osr_mark(s, head);
                    self.b.ins().jump(head, &[]);
                    self.b.switch_to_block(head);
                }
                self.no_osr += 1;
                let r = self.foreach(subject, key.as_ref(), value, body);
                self.no_osr -= 1;
                r?
            }
            StmtKind::Switch { subject, cases } => {
                self.no_osr += 1;
                let r = self.switch(subject, cases);
                self.no_osr -= 1;
                r?
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
                let (cont, brk, depth) = self.loops[self.loops.len() - n];
                let target = if matches!(s.kind, StmtKind::Break(_)) { brk } else { cont };
                self.run_finallys(depth)?;
                if !self.terminated {
                    self.b.ins().jump(target, &[]);
                }
                self.terminated = true;
            }
            StmtKind::Block(b) => self.stmts(b)?,
            StmtKind::Try { body, catches, finally } => {
                self.no_osr += 1;
                let r = self.try_stmt(body, catches, finally.as_deref());
                self.no_osr -= 1;
                r?
            }
            StmtKind::Nop => {}
            StmtKind::Unset(es) => {
                for e in es {
                    match &e.kind {
                        ExprKind::Var(n) if self.top_ptrs.is_some() => {
                            // A frame variable: unbound, not just emptied.
                            let i = self.locals.index[n.as_str()];
                            let [pa, ra, da, sp] = self.scope_args()?;
                            let iv = self.iconst(i as i64);
                            self.call("rt_unset_local", &[self.cx, pa, ra, da, sp, iv]);
                        }
                        ExprKind::Var(n) => {
                            let i = self.locals.index[n.as_str()];
                            let a = self.local_addr(i);
                            self.clear(a);
                            self.store_tag(a, tag::NULL);
                        }
                        ExprKind::GlobalVar(n) => {
                            let (np, nl) = self.global_name(n)?;
                            self.call_checked("rt_global_unset", &[self.cx, np, nl]);
                        }
                        _ => {
                            let (root, name, keys) = self.place(e, true)?;
                            let (np, nl) = name;
                            let (kslot, kn, kids) = keys;
                            let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
                            let knv = self.iconst(kn as i64);
                            self.call_checked("rt_unset_place", &[self.cx, root.addr, np, nl, ka, knv]);
                            self.release_ids(&kids);
                            self.release_root(root);
                        }
                    }
                }
            }
            _ => return Err("statement"),
        }
        Ok(())
    }

    /// Inline the `finally` blocks of handlers `from..` (innermost first)
    /// on a return / break path. Each runs outside its own try.
    fn run_finallys(&mut self, from: usize) -> L<()> {
        let saved = (self.handlers.clone(), self.unwind);
        for h in (from..saved.0.len()).rev() {
            let Some(fin) = saved.0[h].finally else { continue };
            self.handlers.truncate(h);
            self.unwind = saved.0[h].outer_unwind;
            self.stmts(fin)?;
            if self.terminated {
                break;
            }
        }
        (self.handlers, self.unwind) = saved;
        Ok(())
    }

    fn try_stmt(&mut self, body: &'static [Stmt], catches: &'static [ast::Catch], finally: Option<&'static [Stmt]>) -> L<()> {
        let outer = self.unwind;
        let live: Vec<bool> = (0..self.temps.len()).map(|i| !self.free.contains(&i)).collect();
        let iters0 = self.iters.len();
        let (landing, after) = (self.b.create_block(), self.b.create_block());
        let fin_exc = finally.map(|_| self.b.create_block());
        let mut falls = false;
        let handler = Handler { finally, outer_unwind: outer };

        // Body (and each catch) runs with this try's handler active; the
        // fall-through path runs finally inline, outside the handler.
        let guarded = |l: &mut Self, stmts: &'static [Stmt], unwind: Block, falls: &mut bool| -> L<()> {
            l.handlers.push(handler);
            l.unwind = unwind;
            l.terminated = false;
            l.stmts(stmts)?;
            l.handlers.pop();
            l.unwind = outer;
            if !l.terminated {
                if let Some(fin) = finally {
                    l.stmts(fin)?;
                }
                if !l.terminated {
                    l.b.ins().jump(after, &[]);
                    *falls = true;
                }
            }
            Ok(())
        };
        guarded(self, body, landing, &mut falls)?;

        // Landing: drop whatever the body held when it threw.
        self.b.switch_to_block(landing);
        for t in 0..self.temps.len() {
            if !live.get(t).copied().unwrap_or(false) {
                let a = self.temp_addr(t);
                self.clear(a);
                self.store_tag(a, tag::NULL);
            }
        }
        for s in self.iters[iters0..].to_vec() {
            let a = self.b.ins().stack_addr(types::I64, s, 0);
            self.call("rt_iter_free", &[a]);
        }
        let rethrow = fin_exc.unwrap_or(outer);
        for c in catches {
            let dst = match &c.var {
                Some(v) => {
                    let i = self.locals.index[v.name.as_str()];
                    self.mark_defined(i);
                    self.local_addr(i)
                }
                None => self.iconst(0),
            };
            let cp = self.iconst(c as *const ast::Catch as i64);
            let hit = self.call("rt_catch", &[self.cx, cp, dst]).unwrap();
            let (yes, no) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(hit, yes, &[], no, &[]);
            self.b.switch_to_block(yes);
            guarded(self, &c.body, rethrow, &mut falls)?;
            self.b.switch_to_block(no);
        }
        self.b.ins().jump(rethrow, &[]);

        // Exception path through finally: stash it, run finally, rethrow.
        // exit() skips finally.
        if let (Some(fin), Some(blk)) = (finally, fin_exc) {
            self.b.switch_to_block(blk);
            let stash = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
            self.stashes.push(stash);
            let sa = self.b.ins().stack_addr(types::I64, stash, 0);
            let exit = self.call("rt_pending_save", &[self.cx, sa]).unwrap();
            let run = self.b.create_block();
            self.b.ins().brif(exit, outer, &[], run, &[]);
            self.b.switch_to_block(run);
            self.terminated = false;
            self.stmts(fin)?;
            if !self.terminated {
                let sa = self.b.ins().stack_addr(types::I64, stash, 0);
                self.call("rt_pending_restore", &[self.cx, sa]);
                self.b.ins().jump(outer, &[]);
            }
        }

        self.b.switch_to_block(after);
        self.terminated = !falls;
        if !falls {
            self.b.ins().trap(cranelift_codegen::ir::TrapCode::user(1).unwrap());
        }
        Ok(())
    }

    fn loop_body(&mut self, body: &'static [Stmt], cont: Block, exit: Block) -> L<()> {
        self.loops.push((cont, exit, self.handlers.len()));
        self.terminated = false;
        if body.is_empty() {
            // `while (...) {}` must still notice time limits.
            self.boundary_check();
        }
        self.stmts(body)?;
        if !self.terminated {
            self.b.ins().jump(cont, &[]);
        }
        self.terminated = false;
        self.loops.pop();
        Ok(())
    }

    fn foreach(&mut self, subject: &'static Expr, key: Option<&'static Expr>, value: &'static Expr, body: &'static [Stmt]) -> L<()> {
        let op = self.expr(subject)?;
        let op = self.materialize(op);
        let sa = self.addr(op).unwrap();
        let islot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        self.iters.push(islot);
        // Packed arrays iterate inline over a held copy (a snapshot: writes
        // to the variable separate, as foreach-by-value requires); idx < 0
        // means "use the iterator helper".
        let fast = super::layout().is_some();
        let arr_t = self.alloc_temp();
        let idx_slot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 8, 3));
        let (packed_setup, iter_setup, after_setup) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        if fast {
            let tg = self.load_tag(sa);
            let is_arr = self.b.ins().icmp_imm_s(IntCC::Equal, tg, tag::ARRAY as i64);
            let chk = self.b.create_block();
            self.b.ins().brif(is_arr, chk, &[], iter_setup, &[]);
            self.b.switch_to_block(chk);
            let rc = self.b.ins().load(types::I64, flags(), sa, tag::PAYLOAD);
            let pk = self.b.ins().load(types::I64, flags(), rc, 16);
            let packed = self.b.ins().icmp_imm_s(IntCC::NotEqual, pk, 0);
            self.b.ins().brif(packed, packed_setup, &[], iter_setup, &[]);
            self.b.switch_to_block(packed_setup);
            let arr_a = self.temp_addr(arr_t);
            self.store_into(arr_a, Op::Ptr(sa, None));
            let zero = self.iconst(0);
            self.b.ins().stack_store(types::I64, zero, idx_slot, 0);
            self.b.ins().jump(after_setup, &[]);
        } else {
            self.b.ins().jump(iter_setup, &[]);
            self.b.switch_to_block(packed_setup);
            self.b.ins().jump(after_setup, &[]);
        }
        self.b.switch_to_block(iter_setup);
        let ia = self.b.ins().stack_addr(types::I64, islot, 0);
        self.call_checked("rt_iter_new", &[self.cx, sa, ia]);
        let neg = self.iconst(-1);
        self.b.ins().stack_store(types::I64, neg, idx_slot, 0);
        self.b.ins().jump(after_setup, &[]);
        self.b.switch_to_block(after_setup);
        self.release(op);
        let (head, bodyb, exit) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().jump(head, &[]);
        self.b.switch_to_block(head);
        let kt = key.map(|_| self.alloc_temp());
        let vt = self.alloc_temp();
        let (packed_next, iter_next, more) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        let idx = self.b.ins().stack_load(types::I64, types::I64, idx_slot, 0);
        let iter_mode = self.b.ins().icmp_imm_s(IntCC::SignedLessThan, idx, 0);
        self.b.ins().brif(iter_mode, iter_next, &[], packed_next, &[]);
        // Packed: element idx of the held array, or done.
        self.b.switch_to_block(packed_next);
        {
            let arr_a = self.temp_addr(arr_t);
            let rc = self.b.ins().load(types::I64, flags(), arr_a, tag::PAYLOAD);
            let len = self.b.ins().load(types::I64, flags(), rc, 24);
            let more_p = self.b.ins().icmp(IntCC::SignedLessThan, idx, len);
            let (take, deref, plain) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
            self.b.ins().brif(more_p, take, &[], exit, &[]);
            self.b.switch_to_block(take);
            let pk = self.b.ins().load(types::I64, flags(), rc, 16);
            let off = self.b.ins().ishl_imm_u(idx, 4);
            let p = self.b.ins().iadd(pk, off);
            let va = self.temp_addr(vt);
            let et = self.load_tag(p);
            let is_ref = self.b.ins().icmp_imm_s(IntCC::Equal, et, tag::REF as i64);
            let after = self.b.create_block();
            self.b.ins().brif(is_ref, deref, &[], plain, &[]);
            self.b.switch_to_block(deref);
            self.call("rt_copy", &[va, p]);
            self.b.ins().jump(after, &[]);
            self.b.switch_to_block(plain);
            self.store_into(va, Op::Ptr(p, None));
            self.b.ins().jump(after, &[]);
            self.b.switch_to_block(after);
            if let Some(t) = kt {
                let ka = self.temp_addr(t);
                self.store_tag(ka, tag::INT);
                self.b.ins().store(flags(), idx, ka, tag::PAYLOAD);
            }
            let n = self.b.ins().iadd_imm_s(idx, 1);
            self.b.ins().stack_store(types::I64, n, idx_slot, 0);
            self.b.ins().jump(more, &[]);
        }
        self.b.switch_to_block(iter_next);
        let iter = self.b.ins().stack_load(types::I64, types::I64, islot, 0);
        let ka = match kt {
            Some(t) => self.temp_addr(t),
            None => self.iconst(0),
        };
        let va = self.temp_addr(vt);
        let r = self.call("rt_iter_next", &[self.cx, iter, ka, va]).unwrap();
        let err = self.b.ins().icmp_imm_s(IntCC::Equal, r, 2);
        let ok = self.b.create_block();
        self.b.ins().brif(err, self.unwind, &[], ok, &[]);
        self.b.switch_to_block(ok);
        let has = self.b.ins().icmp_imm_s(IntCC::Equal, r, 1);
        self.b.ins().brif(has, more, &[], exit, &[]);
        self.b.switch_to_block(more);
        // kt/vt stay reserved for this loop: rt_iter_next writes them each
        // iteration, so move out of them without returning them to the pool.
        if let (Some(k), Some(t)) = (key, kt) {
            let ExprKind::Var(n) = &k.kind else { return Err("foreach key") };
            let i = self.locals.index[n.as_str()];
            let ta = self.temp_addr(t);
            self.move_slot_to_local(i, ta);
        }
        let ExprKind::Var(n) = &value.kind else { return Err("foreach value") };
        let i = self.locals.index[n.as_str()];
        let va = self.temp_addr(vt);
        self.move_slot_to_local(i, va);
        self.b.ins().jump(bodyb, &[]);
        self.b.switch_to_block(bodyb);
        self.loop_body(body, head, exit)?;
        self.b.switch_to_block(exit);
        let ia = self.b.ins().stack_addr(types::I64, islot, 0);
        self.call("rt_iter_free", &[ia]);
        let arr_a = self.temp_addr(arr_t);
        self.release(Op::Ptr(arr_a, Some(arr_t)));
        Ok(())
    }

    fn switch(&mut self, subject: &'static Expr, cases: &'static [ast::Case]) -> L<()> {
        let op = self.expr(subject)?;
        let op = self.own(op);
        let op = self.materialize(op);
        let sa = self.addr(op).unwrap();
        let end = self.b.create_block();
        let bodies: Vec<Block> = cases.iter().map(|_| self.b.create_block()).collect();
        // Tests in order; no match jumps to default (or end).
        let default = cases.iter().position(|c| c.test.is_none());
        for (i, c) in cases.iter().enumerate() {
            let Some(t) = &c.test else { continue };
            let top = self.expr(t)?;
            let top = self.materialize(top);
            let ta = self.addr(top).unwrap();
            let eq = self.call("rt_loose_eq", &[sa, ta]).unwrap();
            self.release(top);
            let next = self.b.create_block();
            self.b.ins().brif(eq, bodies[i], &[], next, &[]);
            self.b.switch_to_block(next);
        }
        let fallback = default.map(|d| bodies[d]).unwrap_or(end);
        self.b.ins().jump(fallback, &[]);
        // Bodies fall through to the next one, like PHP.
        self.loops.push((end, end, self.handlers.len()));
        for (i, c) in cases.iter().enumerate() {
            self.b.switch_to_block(bodies[i]);
            self.terminated = false;
            self.stmts(&c.body)?;
            if !self.terminated {
                let next = bodies.get(i + 1).copied().unwrap_or(end);
                self.b.ins().jump(next, &[]);
            }
        }
        self.loops.pop();
        self.terminated = false;
        self.b.switch_to_block(end);
        self.release(op);
        Ok(())
    }

    // ---- expressions ---------------------------------------------------------------

    /// Evaluate for side effects only.
    pub fn discard(&mut self, e: &'static Expr) -> L<()> {
        match &e.kind {
            ExprKind::Assign { target, value } if self.simple_index_target(target).is_some() => {
                self.local_store(target, value, false)?;
            }
            ExprKind::Assign { target, value } if matches!(&target.kind, ExprKind::Index { base, index: Some(_) } if matches!(&base.kind, ExprKind::Prop { name: MemberName::Ident(_), nullsafe: false, .. })) => {
                let op = self.assign_place(target, value, false)?;
                self.release(op);
            }
            ExprKind::IncDec { expr, inc, .. } if matches!(expr.kind, ExprKind::Var(_)) => {
                self.incdec_local(expr, *inc, false, false)?;
            }
            _ => {
                let op = self.expr(e)?;
                self.release(op);
            }
        }
        Ok(())
    }

    fn local_of(&self, e: &Expr) -> Option<u32> {
        match &e.kind {
            ExprKind::Var(n) => self.locals.index.get(n.as_str()).copied(),
            _ => None,
        }
    }

    fn null(&mut self) -> Op {
        let c = self.const_value(Value::Null);
        Op::Ptr(c, None)
    }

    fn name_parts(&mut self, s: &'static str) -> (CValue, CValue) {
        let p = self.iconst(s.as_ptr() as i64);
        let l = self.iconst(s.len() as i64);
        (p, l)
    }

    /// The literal name of `$_SERVER` / `$GLOBALS['name']`, as (ptr, len).
    fn global_name(&mut self, n: &'static Expr) -> L<(CValue, CValue)> {
        match &n.kind {
            ExprKind::Str(s) => Ok(self.name_parts(s)),
            _ => Err("dynamic $GLOBALS name"),
        }
    }

    fn member(&self, m: &'static MemberName) -> L<&'static str> {
        match m {
            MemberName::Ident(i) => Ok(&i.name),
            MemberName::Dynamic(_) => Err("dynamic member name"),
        }
    }

    pub fn expr(&mut self, e: &'static Expr) -> L<Op> {
        if crate::value::stack_low() {
            return Err("expression nested too deeply");
        }
        if !self.escapes.is_empty() && self.escapes.contains(&(e as *const Expr as usize)) {
            return self.escape_expr(e);
        }
        // A `?->` on null skips the rest of its chain: the outermost element
        // of a chain with one below it owns the block they jump to.
        let inherited = self.chain_skip.take();
        if inherited.is_none() && nullsafe_below(e) {
            let t = self.alloc_temp();
            let (skip, merge) = (self.b.create_block(), self.b.create_block());
            let r = self.expr_kind(e, Some(skip))?;
            let ta = self.temp_addr(t);
            self.store_into(ta, r);
            self.b.ins().jump(merge, &[]);
            self.b.switch_to_block(skip);
            let ta = self.temp_addr(t);
            self.store_tag(ta, tag::NULL);
            self.b.ins().jump(merge, &[]);
            self.b.switch_to_block(merge);
            let ta = self.temp_addr(t);
            return Ok(Op::Ptr(ta, Some(t)));
        }
        self.expr_kind(e, inherited)
    }

    /// The object (or base) of a chain element, inside the chain `chain`.
    fn chain_expr(&mut self, e: &'static Expr, chain: Option<Block>) -> L<Op> {
        self.chain_skip = chain;
        let r = self.expr(e);
        self.chain_skip = None;
        r
    }

    fn expr_kind(&mut self, e: &'static Expr, chain: Option<Block>) -> L<Op> {
        use ExprKind::*;
        Ok(match &e.kind {
            Int(n) => Op::Int(self.iconst(*n)),
            Float(f) => Op::Float(self.b.ins().f64const(*f)),
            Bool(b) => Op::Bool(self.b.ins().iconst(types::I8, *b as i64)),
            Null => self.null(),
            Bytes(b) => {
                let c = self.const_value(Value::bytes(b.clone()));
                Op::Ptr(c, None)
            }
            Str(s) => {
                let c = self.const_value(Value::str(s));
                Op::Ptr(c, None)
            }
            Var(n) => Op::Local(*self.locals.index.get(n.as_str()).ok_or("unknown local")?),
            GlobalVar(n) => {
                let (np, nl) = self.global_name(n)?;
                let quiet = self.iconst(0);
                self.helper_value("rt_global_get", |l| vec![l.cx, np, nl, quiet])?
            }
            Interp(parts) => {
                // Evaluate parts (left to right), then build the string once.
                let mut ops = vec![];
                for p in parts {
                    let op = match p {
                        ast::InterpPart::Lit(s) => {
                            let c = self.const_value(Value::bytes(s.clone()));
                            Op::Ptr(c, None)
                        }
                        ast::InterpPart::Expr(e) => {
                            let v = self.expr(e)?;
                            // A later part may modify a borrowed local: own it.
                            let v = self.own(v);
                            self.materialize(v)
                        }
                    };
                    ops.push(op);
                }
                self.join_strings(ops)?
            }
            Name(n) => self.helper_value("rt_const", |l| {
                let p = l.iconst(n as *const String as i64);
                vec![l.cx, p]
            })?,
            MagicConst(_) => self.helper_value("rt_magic", |l| {
                let p = l.iconst(e as *const Expr as i64);
                vec![l.cx, p]
            })?,
            Array(items) if crate::expr::const_array(items).is_some() => {
                let c = self.const_value(crate::expr::const_array(items).unwrap());
                Op::Ptr(c, None)
            }
            Array(items) if literal_template(items).is_some() => {
                // Constant keys: the values into a row of temps, then one
                // copy of the key layout with the values moved in.
                let n = items.len();
                let (slot, ids) = self.alloc_array(n);
                for (i, it) in items.iter().flatten().enumerate() {
                    let v = self.expr(&it.value)?;
                    let v = self.own(v);
                    let a = self.b.ins().stack_addr(types::I64, slot, (i * 16) as i32);
                    self.store_into(a, v);
                }
                let tp = self.const_value(literal_template(items).unwrap());
                let base = self.b.ins().stack_addr(types::I64, slot, 0);
                let nv = self.iconst(n as i64);
                let r = self.helper_value("rt_array_lit", |_| vec![tp, base, nv])?;
                self.release_ids(&ids);
                r
            }
            Array(items) => {
                let t = self.alloc_temp();
                let ta = self.temp_addr(t);
                // String keys (or int keys out of order): a hash from the start.
                let hash = items.iter().flatten().any(|it| it.key.as_ref().is_some_and(|k| !matches!(k.kind, Int(_))));
                let n = self.iconst(items.len() as i64);
                let h = self.iconst(hash as i64);
                self.call("rt_array_new", &[ta, n, h]);
                for it in items.iter().flatten() {
                    let key = match &it.key {
                        Some(k) => {
                            let k = self.expr(k)?;
                            Some(self.materialize(k))
                        }
                        None => None,
                    };
                    let v = self.expr(&it.value)?;
                    let v = self.own(v);
                    let v = self.materialize(v);
                    let ka = match key {
                        Some(k) => self.addr(k).unwrap(),
                        None => self.iconst(0),
                    };
                    let va = self.addr(v).unwrap();
                    let ta = self.temp_addr(t);
                    let sp = self.iconst(it.spread as i64);
                    self.call_checked("rt_array_add", &[self.cx, ta, ka, va, sp]);
                    self.release(v);
                    if let Some(k) = key {
                        self.release(k);
                    }
                }
                let ta = self.temp_addr(t);
                Op::Ptr(ta, Some(t))
            }
            Assign { target, value } => self.assign(target, value)?,
            AssignOp { op, target, value } => self.assign_op(*op, target, value)?,
            IncDec { prefix, inc, expr } => match &expr.kind {
                Var(_) => self.incdec_local(expr, *inc, *prefix, true)?,
                Prop { object, name: MemberName::Ident(name), nullsafe: false }
                    if super::layout().is_some() && matches!(&object.kind, Var(n) if self.locals.index.contains_key(n.as_str())) =>
                {
                    self.incdec_prop(expr, object, &name.name, *inc, *prefix)?
                }
                _ => {
                    let mode = (if *inc { 100 } else { 101 }) | if *prefix { 256 } else { 0 };
                    self.place_update(expr, mode, None)?
                }
            },
            Binary { op: BinOp::And | BinOp::Or, left, right } => {
                let is_and = matches!(e.kind, Binary { op: BinOp::And, .. });
                let l = self.expr(left)?;
                let a = self.truthy(l);
                let (rhs, merge) = (self.b.create_block(), self.b.create_block());
                self.b.append_block_param(merge, types::I8);
                let short = self.b.ins().iconst(types::I8, if is_and { 0 } else { 1 });
                if is_and {
                    self.b.ins().brif(a, rhs, &[], merge, &[short.into()]);
                } else {
                    self.b.ins().brif(a, merge, &[short.into()], rhs, &[]);
                }
                self.b.switch_to_block(rhs);
                let r = self.expr(right)?;
                let c = self.truthy(r);
                self.b.ins().jump(merge, &[c.into()]);
                self.b.switch_to_block(merge);
                Op::Bool(self.b.block_params(merge)[0])
            }
            Binary { op: BinOp::Coalesce, left, right } => {
                let t = self.alloc_temp();
                let found = self.quiet(left, t)?;
                let (yes, no, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().brif(found, yes, &[], no, &[]);
                self.b.switch_to_block(yes);
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(no);
                let ta = self.temp_addr(t);
                self.clear(ta);
                let r = self.expr(right)?;
                let ta = self.temp_addr(t);
                self.store_into(ta, r);
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(merge);
                let ta = self.temp_addr(t);
                Op::Ptr(ta, Some(t))
            }
            Binary { op: BinOp::Concat, .. } => {
                // `a . b . c`: evaluate the operands, then build the string
                // once. The chain continues only past literal operands, so
                // the result converts operands in PHP's order: PHP also
                // evaluates both sides of a `.` before converting them,
                // and literals have nothing to evaluate.
                let mut parts = vec![];
                let mut cur = e;
                while let Binary { op: BinOp::Concat, left, right } = &cur.kind {
                    parts.push(&**right);
                    let simple = |x: &Expr| matches!(x.kind, Str(_) | Bytes(_) | Int(_));
                    if !simple(right) || !matches!(&left.kind, Binary { op: BinOp::Concat, .. }) {
                        cur = left;
                        break;
                    }
                    cur = left;
                }
                parts.push(cur);
                parts.reverse();
                let mut ops = vec![];
                for p in parts {
                    let v = self.expr(p)?;
                    let v = self.own(v);
                    ops.push(self.materialize(v));
                }
                self.join_strings(ops)?
            }
            // `$x === null` / `$x !== null`: a tag compare (references
            // take the general path).
            Binary { op: op @ (BinOp::Identical | BinOp::NotIdentical), left, right }
                if matches!(left.kind, Null) || matches!(right.kind, Null) =>
            {
                let other = if matches!(left.kind, Null) { right } else { left };
                let v = self.expr(other)?;
                let Some(a) = self.addr(v) else {
                    // An int, float or bool is never null.
                    self.release(v);
                    return Ok(Op::Bool(self.b.ins().iconst(types::I8, (*op == BinOp::NotIdentical) as i64)));
                };
                let tg = self.load_tag(a);
                let is_ref = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::REF as i64);
                let (fast, slow, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.append_block_param(merge, types::I8);
                self.b.ins().brif(is_ref, slow, &[], fast, &[]);
                self.b.switch_to_block(fast);
                let cc = if *op == BinOp::Identical { IntCC::Equal } else { IntCC::NotEqual };
                let r = self.b.ins().icmp_imm_u(cc, tg, tag::NULL as i64);
                self.b.ins().jump(merge, &[r.into()]);
                self.b.set_cold_block(slow);
                self.b.switch_to_block(slow);
                let n = self.const_value(Value::Null);
                let code = self.iconst(binop_code(*op) as i64);
                let r = self.call("rt_compare", &[self.cx, code, a, n]).unwrap();
                let r = self.b.ins().ireduce(types::I8, r);
                self.b.ins().jump(merge, &[r.into()]);
                self.b.switch_to_block(merge);
                self.release(v);
                Op::Bool(self.b.block_params(merge)[0])
            }
            Binary { op, left, right } => {
                let a = self.expr(left)?;
                let b = self.expr(right)?;
                self.binary(*op, a, b)?
            }
            Unary { op: UnOp::Silence, expr } => {
                // An exception out of the operand restores the level too.
                let old = self.call("rt_silence_begin", &[self.cx]).unwrap();
                let (outer, restore) = (self.unwind, self.b.create_block());
                self.unwind = restore;
                let v = self.expr(expr);
                self.unwind = outer;
                let v = v?;
                self.call("rt_silence_end", &[self.cx, old]);
                let cont = self.b.create_block();
                self.b.ins().jump(cont, &[]);
                self.b.switch_to_block(restore);
                self.call("rt_silence_end", &[self.cx, old]);
                self.b.ins().jump(outer, &[]);
                self.b.switch_to_block(cont);
                v
            }
            // Negative number literals (-1, -0.5): constants, not a negation.
            Unary { op: UnOp::Neg, expr } if matches!(expr.kind, Int(n) if n != i64::MIN) => {
                let Int(n) = expr.kind else { unreachable!() };
                Op::Int(self.iconst(-n))
            }
            Unary { op: UnOp::Neg, expr } if matches!(expr.kind, Float(_)) => {
                let Float(x) = expr.kind else { unreachable!() };
                Op::Float(self.b.ins().f64const(-x))
            }
            Unary { op, expr } => {
                let v = self.expr(expr)?;
                if let (UnOp::Neg, Op::Float(x)) = (op, &v) {
                    return Ok(Op::Float(self.b.ins().fneg(*x)));
                }
                match op {
                    UnOp::Not => {
                        let t = self.truthy(v);
                        Op::Bool(self.b.ins().bxor_imm_u(t, 1))
                    }
                    UnOp::Silence => v,
                    UnOp::Neg | UnOp::Plus | UnOp::BitNot => {
                        let kind = match op {
                            UnOp::Neg => 0,
                            UnOp::Plus => 1,
                            _ => 2,
                        };
                        let v = self.materialize(v);
                        let va = self.addr(v).unwrap();
                        let k = self.iconst(kind);
                        let r = self.helper_value("rt_unary", |l| vec![l.cx, k, va])?;
                        self.release(v);
                        r
                    }
                }
            }
            Ternary { cond, then, else_ } => {
                let c = self.expr(cond)?;
                let t = self.alloc_temp();
                let (yes, no, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                match then {
                    Some(th) => {
                        let ct = self.truthy(c);
                        self.b.ins().brif(ct, yes, &[], no, &[]);
                        self.b.switch_to_block(yes);
                        let v = self.expr(th)?;
                        let ta = self.temp_addr(t);
                        self.store_into(ta, v);
                        self.b.ins().jump(merge, &[]);
                    }
                    None => {
                        // `a ?: b`: keep a's value.
                        let c = self.own(c);
                        let c = self.materialize(c);
                        let ca = self.addr(c).unwrap();
                        let ct = self.truthy(Op::Ptr(ca, None));
                        self.b.ins().brif(ct, yes, &[], no, &[]);
                        self.b.switch_to_block(yes);
                        // Move a's value (its temp is freed once, below).
                        let ta = self.temp_addr(t);
                        self.copy16(ta, ca);
                        self.store_tag(ca, tag::NULL);
                        self.b.ins().jump(merge, &[]);
                        self.b.switch_to_block(no);
                        self.release(c);
                        let v = self.expr(else_)?;
                        let ta = self.temp_addr(t);
                        self.store_into(ta, v);
                        self.b.ins().jump(merge, &[]);
                        self.b.switch_to_block(merge);
                        let ta = self.temp_addr(t);
                        return Ok(Op::Ptr(ta, Some(t)));
                    }
                }
                self.b.switch_to_block(no);
                let v = self.expr(else_)?;
                let ta = self.temp_addr(t);
                self.store_into(ta, v);
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(merge);
                let ta = self.temp_addr(t);
                Op::Ptr(ta, Some(t))
            }
            Cast { ty, expr } => {
                let kind = match *ty {
                    "int" => 0,
                    "float" => 1,
                    "string" => 2,
                    "bool" => 3,
                    "array" => 4,
                    "object" => 5,
                    _ => 6,
                };
                let v = self.expr(expr)?;
                if kind == 3 {
                    return Ok(Op::Bool(self.truthy(v)));
                }
                match (kind, &v) {
                    (0, Op::Int(_)) | (1, Op::Float(_)) => return Ok(v),
                    _ => {}
                }
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                let k = self.iconst(kind);
                let want = match kind {
                    0 => Some(tag::INT),
                    1 => Some(tag::FLOAT),
                    2 => Some(tag::STR),
                    4 => Some(tag::ARRAY),
                    _ => None,
                };
                let Some(want) = want else {
                    let r = self.helper_value("rt_cast", |l| vec![l.cx, k, va])?;
                    self.release(v);
                    return Ok(r);
                };
                // Already of that type: a copy (no helper call).
                let t = self.alloc_temp();
                let (same, other, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                let tg = self.load_tag(va);
                let eq = self.b.ins().icmp_imm_u(IntCC::Equal, tg, want as i64);
                self.b.ins().brif(eq, same, &[], other, &[]);
                self.b.switch_to_block(same);
                let ta = self.temp_addr(t);
                self.store_into(ta, Op::Ptr(va, None));
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(other);
                let ta = self.temp_addr(t);
                self.call_checked("rt_cast", &[self.cx, k, va, ta]);
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(merge);
                self.release(v);
                let ta = self.temp_addr(t);
                Op::Ptr(ta, Some(t))
            }
            Print(x) => {
                let v = self.expr(x)?;
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                self.call_checked("rt_echo", &[self.cx, va]);
                self.release(v);
                Op::Int(self.iconst(1))
            }
            Throw(x) => {
                let v = self.expr(x)?;
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                self.call("rt_throw", &[self.cx, va]);
                self.b.ins().jump(self.unwind, &[]);
                let dead = self.b.create_block();
                self.b.switch_to_block(dead);
                self.release(v);
                self.null()
            }
            Exit(x) => {
                let va = match x {
                    Some(x) => {
                        let v = self.expr(x)?;
                        let v = self.materialize(v);
                        self.addr(v).unwrap()
                    }
                    None => self.iconst(0),
                };
                self.call("rt_exit", &[self.cx, va]);
                self.b.ins().jump(self.unwind, &[]);
                let dead = self.b.create_block();
                self.b.switch_to_block(dead);
                self.null()
            }
            Clone(x) => {
                let v = self.expr(x)?;
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                let r = self.helper_value("rt_clone", |l| vec![l.cx, va])?;
                self.release(v);
                r
            }
            Isset(es) => {
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I8);
                for x in es {
                    let t = self.alloc_temp();
                    let found = self.quiet_mode(x, t, crate::expr::QUIET_ISSET)?;
                    let ta = self.temp_addr(t);
                    self.clear(ta);
                    self.free.push(t);
                    let next = self.b.create_block();
                    let zero = self.b.ins().iconst(types::I8, 0);
                    self.b.ins().brif(found, next, &[], merge, &[zero.into()]);
                    self.b.switch_to_block(next);
                }
                let one = self.b.ins().iconst(types::I8, 1);
                self.b.ins().jump(merge, &[one.into()]);
                self.b.switch_to_block(merge);
                Op::Bool(self.b.block_params(merge)[0])
            }
            Empty(x) => {
                let t = self.alloc_temp();
                let found = self.quiet_mode(x, t, crate::expr::QUIET_EMPTY)?;
                let (yes, merge) = (self.b.create_block(), self.b.create_block());
                self.b.append_block_param(merge, types::I8);
                let one = self.b.ins().iconst(types::I8, 1);
                self.b.ins().brif(found, yes, &[], merge, &[one.into()]);
                self.b.switch_to_block(yes);
                let ta = self.temp_addr(t);
                let tv = self.truthy(Op::Ptr(ta, None));
                let r = self.b.ins().bxor_imm_u(tv, 1);
                self.b.ins().jump(merge, &[r.into()]);
                self.b.switch_to_block(merge);
                let ta = self.temp_addr(t);
                self.clear(ta);
                self.free.push(t);
                Op::Bool(self.b.block_params(merge)[0])
            }
            Instanceof { expr, class: ClassRef::Dynamic(ce) } => {
                let v = self.expr(expr)?;
                let v = self.materialize(v);
                let cv = self.expr(ce)?;
                let cv = self.materialize(cv);
                let (va, ca) = (self.addr(v).unwrap(), self.addr(cv).unwrap());
                let r = self.helper_value("rt_instanceof_dyn", |l| vec![l.cx, va, ca])?;
                self.release(cv);
                self.release(v);
                r
            }
            Instanceof { expr, class } => {
                let v = self.expr(expr)?;
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                let cr = self.iconst(class as *const ClassRef as i64);
                let site = self.class_site();
                let r = self.call("rt_instanceof", &[self.cx, va, cr, site]).unwrap();
                self.release(v);
                Op::Bool(self.b.ins().ireduce(types::I8, r))
            }
            Match { subject, arms } => self.match_expr(subject, arms)?,
            Prop { object, name: MemberName::Dynamic(ne), nullsafe: false } => {
                let o = self.chain_expr(object, chain)?;
                let o = self.own(o);
                let o = self.materialize(o);
                let nv = self.expr(ne)?;
                let nv = self.materialize(nv);
                let (oa, na) = (self.addr(o).unwrap(), self.addr(nv).unwrap());
                let r = self.helper_value("rt_prop_get_dyn", |l| vec![l.cx, oa, na])?;
                self.release(nv);
                self.release(o);
                r
            }
            Prop { object, name, nullsafe } => {
                let name = self.member(name)?;
                let o = self.chain_expr(object, chain)?;
                let o = self.materialize(o);
                let oa = self.addr(o).unwrap();
                let (np, nl) = self.name_parts(name);
                let site = Box::new(super::rt::PropSite::new(name));
                let sp = &*site as *const super::rt::PropSite as i64;
                self.psites.push(site);
                let _ = (np, nl);
                let r = if *nullsafe { self.nullsafe(oa, chain, |l| l.prop_read(oa, sp))? } else { self.prop_read(oa, sp)? };
                self.release(o);
                r
            }
            Index { base, index } => {
                let b = self.chain_expr(base, chain)?;
                let b = self.materialize(b);
                let k = self.expr(index.as_deref().ok_or("[] read")?)?;
                let k = self.materialize(k);
                let (ba, ka) = (self.addr(b).unwrap(), self.addr(k).unwrap());
                let r = self.index_read(ba, ka)?;
                self.release(k);
                self.release(b);
                r
            }
            ClassConst { class, name } => {
                if e.span.start != self.stmt_start {
                    self.set_pos(e.span.start);
                }
                match class {
                    ClassRef::Dynamic(ce) if name.name.eq_ignore_ascii_case("class") => {
                        let v = self.expr(ce)?;
                        let v = self.materialize(v);
                        let va = self.addr(v).unwrap();
                        let r = self.helper_value("rt_class_name_of", |l| vec![l.cx, va])?;
                        self.release(v);
                        r
                    }
                    _ => {
                        let cr = self.class_ref_ptr(class)?;
                        let (np, nl) = self.name_parts(&name.name);
                        let site = self.class_site();
                        self.helper_value("rt_class_const", |l| vec![l.cx, cr, np, nl, site])?
                    }
                }
            }
            StaticProp { class, name } => {
                let name = self.member(name)?;
                let cr = self.class_ref_ptr(class)?;
                let (np, nl) = self.name_parts(name);
                let site = self.class_site();
                self.helper_value("rt_static_prop_get", |l| vec![l.cx, cr, np, nl, site])?
            }
            Call { callee, args } if !matches!(callee.kind, ExprKind::Name(_)) => {
                let cv = self.expr(callee)?;
                let cv = self.own(cv);
                let cv = self.materialize(cv);
                let ca = self.addr(cv).unwrap();
                let site = Box::new(super::rt::MethodSite::new("", ""));
                let sp = self.iconst(&*site as *const super::rt::MethodSite as i64);
                self.msites.push(site);
                let (aa, la, n_) = self.args(args, ByRef::Unknown)?;
                let meta = self.arg_meta(args);
                let r = self.helper_value("rt_call_value", |l| vec![l.cx, ca, sp, aa, n_, la, meta])?;
                self.write_back(la, args.len())?;
                self.release(cv);
                r
            }
            Call { callee, args } if args.is_empty() && self.top_ptrs.is_none() && matches!(&callee.kind, ExprKind::Name(n) if n.trim_start_matches('\\').eq_ignore_ascii_case("func_get_args")) => {
                let la = self.b.ins().stack_addr(types::I64, self.locals_slot, 0);
                let sp = self.iconst(self.spec_addr);
                self.helper_value("rt_func_get_args", |l| vec![l.cx, la, sp])?
            }
            Call { callee, args } if args.is_empty() && self.top_ptrs.is_none() && matches!(&callee.kind, ExprKind::Name(n) if n.trim_start_matches('\\').eq_ignore_ascii_case("func_num_args")) => {
                self.helper_value("rt_func_num_args", |l| vec![l.cx])?
            }
            Call { callee, args } if args.is_empty() && matches!(&callee.kind, ExprKind::Name(n) if n.trim_start_matches('\\').eq_ignore_ascii_case("get_called_class")) => {
                self.helper_value("rt_called_class", |l| vec![l.cx])?
            }
            Call { callee, args } if is_call_user_func(callee, args) => {
                let ExprKind::Name(n) = &callee.kind else { unreachable!() };
                let site = Box::new(CallSite::new(n));
                let sp = &*site as *const CallSite as i64;
                self.sites.push(site);
                let msite = Box::new(super::rt::MethodSite::new("", ""));
                let mp = &*msite as *const super::rt::MethodSite as i64;
                self.msites.push(msite);
                let (aa, _, n_) = self.args(args, ByRef::None)?;
                self.mark_call(callee.span.start, args);
                let (sp, mp) = (self.iconst(sp), self.iconst(mp));
                self.helper_value("rt_call_user_func", |l| vec![l.cx, sp, mp, aa, n_])?
            }
            Call { callee, args } if is_call_user_func_array(callee, args) => {
                let ExprKind::Name(n) = &callee.kind else { unreachable!() };
                let site = Box::new(CallSite::new(n));
                let sp = &*site as *const CallSite as i64;
                self.sites.push(site);
                let msite = Box::new(super::rt::MethodSite::new("", ""));
                let mp = &*msite as *const super::rt::MethodSite as i64;
                self.msites.push(msite);
                // A literal list: its elements laid out like call_user_func's
                // arguments.
                let list = match &args[1].value.kind {
                    ExprKind::Array(items) if items.len() <= 16 && items.iter().all(|i| i.as_ref().is_some_and(|i| i.key.is_none() && !i.spread && !i.by_ref)) => Some(items),
                    _ => None,
                };
                let (aa, n_) = match list {
                    Some(items) => {
                        let n = items.len() + 1;
                        let (slot, ids) = self.alloc_array(n);
                        for i in 0..n {
                            let a = self.b.ins().stack_addr(types::I64, slot, (i * 16) as i32);
                            self.store_tag(a, tag::NULL);
                        }
                        let vals = std::iter::once(&args[0].value).chain(items.iter().flatten().map(|i| &i.value));
                        for (i, e) in vals.enumerate() {
                            let v = self.expr(e)?;
                            let dst = self.b.ins().stack_addr(types::I64, slot, (i * 16) as i32);
                            self.store_into(dst, v);
                        }
                        self.free.extend(ids);
                        (self.b.ins().stack_addr(types::I64, slot, 0), self.iconst(n as i64))
                    }
                    None => {
                        let (aa, _, n_) = self.args(args, ByRef::None)?;
                        (aa, n_)
                    }
                };
                self.mark_call(callee.span.start, args);
                let (sp, mp, lf) = (self.iconst(sp), self.iconst(mp), self.iconst(list.is_some() as i64));
                self.helper_value("rt_call_user_func_array", |l| vec![l.cx, sp, mp, aa, n_, lf])?
            }
            Call { callee, args } if intrinsic_kind(callee, args).is_some() => {
                let ExprKind::Name(n) = &callee.kind else { unreachable!() };
                self.intrinsic(intrinsic_kind(callee, args).unwrap(), n, args)?
            }
            Call { callee, args } => {
                let ExprKind::Name(n) = &callee.kind else { unreachable!() };
                let site = Box::new(CallSite { direct: Default::default(), frame: Default::default(), name: n, cached: Default::default(), builtin: Default::default(), bspec: Default::default(), builtin_ref: Default::default(), jit: Default::default() });
                let sp = &*site as *const CallSite as i64;
                self.sites.push(site);
                let (aa, la, n_) = self.args(args, ByRef::for_function(n))?;
                self.mark_call(callee.span.start, args);
                let sp = self.iconst(sp);
                let meta = self.arg_meta(args);
                let plain = args.iter().all(|a| !a.spread && a.name.is_none());
                // A common built-in: its fast entry first (unless a
                // namespaced function shadows a built-in name).
                let unqualified = n.trim_start_matches('\\');
                let fast = if plain && !unqualified.contains('\\') { super::fastbi::lookup(&unqualified.to_ascii_lowercase(), args.len()) } else { None };
                let (t, merge) = match fast {
                    Some(h) => {
                        let t = self.alloc_temp();
                        let (try_b, generic, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                        self.shadow_check(n, try_b, generic);
                        self.b.switch_to_block(try_b);
                        let ta = self.temp_addr(t);
                        let st = self.call(h, &[aa, n_, ta]).unwrap();
                        self.b.ins().brif(st, generic, &[], merge, &[]);
                        self.b.set_cold_block(generic);
                        self.b.switch_to_block(generic);
                        (Some(t), Some(merge))
                    }
                    None => (None, None),
                };
                let r = match super::layout().filter(|_| plain) {
                    Some(lay) => self.direct_call(lay, sp, aa, n_, la, meta),
                    None => self.helper_value("rt_call", |l| vec![l.cx, sp, aa, n_, la, meta])?,
                };
                let r = match (t, merge) {
                    (Some(t), Some(merge)) => {
                        let ta = self.temp_addr(t);
                        self.store_into(ta, r);
                        self.b.ins().jump(merge, &[]);
                        self.b.switch_to_block(merge);
                        Op::Ptr(self.temp_addr(t), Some(t))
                    }
                    _ => r,
                };
                self.write_back(la, args.len())?;
                r
            }
            MethodCall { object, name: MemberName::Dynamic(ne), args, nullsafe: false } => {
                let o = self.chain_expr(object, chain)?;
                let o = self.own(o);
                let o = self.materialize(o);
                let nv = self.expr(ne)?;
                let nv = self.own(nv);
                let nv = self.materialize(nv);
                let (oa, na) = (self.addr(o).unwrap(), self.addr(nv).unwrap());
                let (aa, la, n_) = self.args(args, ByRef::Unknown)?;
                let meta = self.arg_meta(args);
                let r = self.helper_value("rt_method_call_dyn", |l| vec![l.cx, oa, na, aa, n_, la, meta])?;
                self.write_back(la, args.len())?;
                self.release(nv);
                self.release(o);
                r
            }
            MethodCall { object, name: mname, args, nullsafe } => {
                let name = self.member(mname)?;
                let name_off = match mname {
                    MemberName::Ident(i) => i.span.start,
                    MemberName::Dynamic(d) => d.span.start,
                };
                let o = self.chain_expr(object, chain)?;
                // A local receiver is read in place when the arguments run
                // no PHP code that could change it (compiled locals hold no
                // references, and the call itself holds the object as
                // `$this`).
                let keep = matches!(o, Op::Local(_))
                    && args.iter().all(|a| match (&a.value.kind, &object.kind) {
                        (ExprKind::Var(x), ExprKind::Var(y)) => x != y,
                        (k, _) => !matches!(k, ExprKind::Prop { .. } | ExprKind::Index { .. } | ExprKind::StaticProp { .. }) && pure_operand(&a.value),
                    });
                let o = if keep { o } else { self.own(o) };
                let o = self.materialize(o);
                let oa = self.addr(o).unwrap();
                // Method lookup is by lowercase name: precompute it.
                let lname: &'static str = Box::leak(name.to_ascii_lowercase().into_boxed_str());
                let site = Box::new(super::rt::MethodSite::new(name, lname));
                let sp = &*site as *const super::rt::MethodSite as i64;
                self.msites.push(site);
                let r = if *nullsafe {
                    self.nullsafe(oa, chain, |l| {
                        let (aa, la, n_) = l.args(args, ByRef::Unknown)?;
                        l.mark_call(name_off, args);
                        let sp = l.iconst(sp);
                        let meta = l.arg_meta(args);
                        let r = l.helper_value("rt_method_call", |l| vec![l.cx, oa, sp, aa, n_, la, meta])?;
                        l.write_back(la, args.len())?;
                        Ok(r)
                    })?
                } else {
                    let (aa, la, n_) = self.args(args, ByRef::Unknown)?;
                    self.mark_call(name_off, args);
                    let sp = self.iconst(sp);
                    let meta = self.arg_meta(args);
                    let r = match super::layout().filter(|_| super::inline_rc() && args.iter().all(|a| !a.spread && a.name.is_none())) {
                        Some(lay) => self.direct_method_call(lay, oa, sp, aa, n_, la, meta, args.len()),
                        None => self.helper_value("rt_method_call", |l| vec![l.cx, oa, sp, aa, n_, la, meta])?,
                    };
                    self.write_back(la, args.len())?;
                    r
                };
                if matches!(o, Op::Ptr(_, Some(_))) {
                    // A temporary (`(new C)->m()`) dies right after the call.
                    self.release(o);
                    self.destruct_check();
                } else {
                    self.release(o);
                }
                r
            }
            StaticCall { class, name: sname, args } => {
                let name = self.member(sname)?;
                let name_off = match sname {
                    MemberName::Ident(i) => i.span.start,
                    MemberName::Dynamic(d) => d.span.start,
                };
                let cr = self.class_ref_ptr(class)?;
                let lname: &'static str = Box::leak(name.to_ascii_lowercase().into_boxed_str());
                let site = Box::new(super::rt::MethodSite::new(name, lname));
                let sp = self.iconst(&*site as *const super::rt::MethodSite as i64);
                self.msites.push(site);
                let (aa, la, n_) = self.args(args, ByRef::Unknown)?;
                self.mark_call(name_off, args);
                let meta = self.arg_meta(args);
                let r = match super::layout().filter(|_| args.iter().all(|a| !a.spread && a.name.is_none()) && matches!(class, ClassRef::Name(_))) {
                    Some(lay) => self.direct_static_call(lay, cr, sp, aa, n_, la, meta),
                    None => self.helper_value("rt_static_call", |l| vec![l.cx, cr, sp, aa, n_, la, meta])?,
                };
                self.write_back(la, args.len())?;
                r
            }
            New { class, args } => {
                // `new` reports at its own line (the object exists before
                // its arguments are evaluated).
                if e.span.start != self.stmt_start {
                    self.set_pos(e.span.start);
                }
                let cr = self.class_ref_ptr(class)?;
                let site = Box::new(super::rt::MethodSite::new("__construct", "__construct"));
                let sp = self.iconst(&*site as *const super::rt::MethodSite as i64);
                self.msites.push(site);
                let plain = matches!(class, ClassRef::Name(_)) && args.iter().all(|a| !a.spread && a.name.is_none());
                match super::layout().filter(|_| plain && super::inline_rc()) {
                    Some(lay) => self.new_native(lay, cr, sp, args, e.span.start)?,
                    None => {
                        let (aa, la, n_) = self.args(args, ByRef::Unknown)?;
                        if !args.is_empty() {
                            self.set_pos(e.span.start);
                        }
                        let meta = self.arg_meta(args);
                        let r = self.helper_value("rt_new", |l| vec![l.cx, cr, sp, aa, n_, la, meta])?;
                        self.write_back(la, args.len())?;
                        r
                    }
                }
            }
            Closure(c) => {
                let vals: Vec<Option<u32>> = c.uses.iter().map(|u| self.locals.index.get(u.var.name.as_str()).copied()).collect();
                self.make_closure(0, &**c as *const ast::Closure as i64, vals)?
            }
            ArrowFn(a) => {
                // Same capture list the interpreter computes, values from our locals.
                let names = arrow_names(a);
                let vals = names.iter().map(|n| self.locals.index.get(n).copied()).collect();
                self.make_closure(1, &**a as *const ast::ArrowFn as i64, vals)?
            }
            _ => return Err("expression"),
        })
    }

    /// Call `helper(args..., dst)` with a fresh temp as dst.
    /// `foo(...)`: once the site has cached a compiled target, call its entry
    /// directly (with the site's frame) instead of going through `rt_call`;
    /// `rt_call` handles the first calls, calls deeper than the site keeps frames for and
    /// declined bindings (arguments untouched).
    fn direct_call(&mut self, lay: super::Layout, sp: CValue, aa: CValue, n: CValue, la: CValue, meta: CValue) -> Op {
        self.emit_direct(
            lay,
            sp,
            lay.site_frame,
            aa,
            n,
            |l, slow, _| {
                let entry = l.b.ins().load(types::I64, flags(), sp, lay.site_direct);
                let c1 = l.b.create_block();
                l.b.ins().brif(entry, c1, &[], slow, &[]);
                l.b.switch_to_block(c1);
                (entry, None, None)
            },
            |l, ta| l.call_checked("rt_call", &[l.cx, sp, aa, n, la, meta, ta]),
        )
    }

    /// `$o->m(...)`: like `direct_call`, for an object of the cached class
    /// called from the cached scope; `$this` is lent to the site's frame.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn direct_method_call(&mut self, lay: super::Layout, oa: CValue, sp: CValue, aa: CValue, n: CValue, la: CValue, meta: CValue, nargs: usize) -> Op {
        self.emit_direct(
            lay,
            sp,
            lay.msite_frame,
            aa,
            n,
            |l, slow, out_t| {
                use super::rt::{MethodSite, SecondEntry};
                let second = std::mem::offset_of!(MethodSite, second) as i32;
                let tg = l.load_tag(oa);
                let is_obj = l.b.ins().icmp_imm_s(IntCC::Equal, tg, tag::OBJECT as i64);
                let (c1, first, try2, out) = (l.b.create_block(), l.b.create_block(), l.b.create_block(), l.b.create_block());
                l.b.append_block_param(out, types::I64);
                l.b.append_block_param(out, types::I64);
                l.b.ins().brif(is_obj, c1, &[], slow, &[]);
                l.b.switch_to_block(c1);
                let rc = l.b.ins().load(types::I64, flags(), oa, tag::PAYLOAD);
                let cls = l.b.ins().load(types::I64, flags(), rc, lay.obj_class);
                let caller = l.b.ins().load(types::I64, flags(), l.cx, lay.cx_frame);
                let scope = l.b.ins().load(types::I64, flags(), caller, lay.frame_class);
                // The cached (class, scope), or the second one beside it
                // (code that sees two classes, like an interface's).
                let entry_for = |l: &mut Self, class_off: i32, scope_off: i32, entry_off: i32, frames_off: i32, miss: Block| {
                    let want = l.b.ins().load(types::I64, flags(), sp, class_off);
                    let same = l.b.ins().icmp(IntCC::Equal, cls, want);
                    let want = l.b.ins().load(types::I64, flags(), sp, scope_off);
                    let same2 = l.b.ins().icmp(IntCC::Equal, scope, want);
                    let both = l.b.ins().band(same, same2);
                    let hit = l.b.create_block();
                    l.b.ins().brif(both, hit, &[], miss, &[]);
                    l.b.switch_to_block(hit);
                    let entry = l.b.ins().load(types::I64, flags(), sp, entry_off);
                    let frames = l.b.ins().iadd_imm_s(sp, frames_off as i64);
                    let go = l.b.create_block();
                    l.b.ins().brif(entry, go, &[], miss, &[]);
                    l.b.switch_to_block(go);
                    l.b.ins().jump(out, &[entry.into(), frames.into()]);
                };
                if nargs <= 3 {
                    l.accessor(lay, sp, rc, cls, scope, aa, nargs, out_t, first);
                } else {
                    l.b.ins().jump(first, &[]);
                }
                l.b.switch_to_block(first);
                entry_for(l, lay.msite_class, lay.msite_scope, lay.msite_direct, lay.msite_frame, try2);
                l.b.switch_to_block(try2);
                entry_for(
                    l,
                    second + std::mem::offset_of!(SecondEntry, class) as i32,
                    second + std::mem::offset_of!(SecondEntry, scope) as i32,
                    second + std::mem::offset_of!(SecondEntry, entry) as i32,
                    second + std::mem::offset_of!(SecondEntry, frame) as i32,
                    slow,
                );
                l.b.switch_to_block(out);
                let (entry, frames) = (l.b.block_params(out)[0], l.b.block_params(out)[1]);
                (entry, Some(rc), Some(frames))
            },
            |l, ta| l.call_checked("rt_method_call", &[l.cx, oa, sp, aa, n, la, meta, ta]),
        )
    }

    /// A cached trivial getter or setter (`rt::Accessor`) of the object's
    /// class answered without calling it: the result goes to temp `t` and
    /// on to `done`; anything else continues at `miss`.
    #[allow(clippy::too_many_arguments)]
    fn accessor(&mut self, lay: super::Layout, sp: CValue, rc: CValue, cls: CValue, scope: CValue, aa: CValue, nargs: usize, (t, done): (usize, Block), miss: Block) {
        use super::rt::{Accessor, MethodSite, ACC_CONST, ACC_LEAF, ACC_SET, ACC_SET_THIS, ACC_SLOT};
        // Either entry of the site's (for this class and calling scope);
        // `base` then addresses it.
        let acc = std::mem::offset_of!(MethodSite, acc) as i64;
        let (hit, try2) = (self.b.create_block(), self.b.create_block());
        self.b.append_block_param(hit, types::I64);
        for (i, next) in [(0, try2), (1, miss)] {
            let base = self.b.ins().iadd_imm_s(sp, acc + (i * std::mem::size_of::<Accessor>()) as i64);
            let want = self.b.ins().load(types::I64, flags(), base, std::mem::offset_of!(Accessor, class) as i32);
            let same = self.b.ins().icmp(IntCC::Equal, cls, want);
            let want = self.b.ins().load(types::I64, flags(), base, std::mem::offset_of!(Accessor, scope) as i32);
            let same2 = self.b.ins().icmp(IntCC::Equal, scope, want);
            let both = self.b.ins().band(same, same2);
            self.b.ins().brif(both, hit, &[base.into()], next, &[]);
            if i == 0 {
                self.b.switch_to_block(next);
            }
        }
        self.b.switch_to_block(hit);
        let base = self.b.block_params(hit)[0];
        let off = |f: usize| f as i32;
        let sp = base;
        let kind = self.b.ins().load(types::I64, flags(), sp, off(std::mem::offset_of!(Accessor, kind)));
        // A leaf method: called without a frame; it may decline (then the
        // real call). Its arguments are scalars: nothing to release.
        let (leaf, not_leaf) = (self.b.create_block(), self.b.create_block());
        let is_leaf = self.b.ins().icmp_imm_u(IntCC::Equal, kind, ACC_LEAF as i64);
        self.b.ins().brif(is_leaf, leaf, &[], not_leaf, &[]);
        self.b.switch_to_block(leaf);
        let f = self.b.ins().load(types::I64, flags(), sp, off(std::mem::offset_of!(Accessor, leaf)));
        let slots = self.b.ins().iadd_imm_s(sp, off(std::mem::offset_of!(Accessor, slots)) as i64);
        let mut sig = self.module.make_signature();
        for _ in 0..4 {
            sig.params.push(cranelift_codegen::ir::AbiParam::new(types::I64));
        }
        sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
        let sigref = self.b.import_signature(sig);
        let ta = self.temp_addr(t);
        let call = self.b.ins().call_indirect(sigref, f, &[rc, aa, ta, slots]);
        let declined = self.b.inst_results(call)[0];
        self.b.ins().brif(declined, miss, &[], done, &[]);
        self.b.switch_to_block(not_leaf);
        if nargs > 1 {
            self.b.ins().jump(miss, &[]);
            return;
        }
        // The slot of objects of the cached shape, and its tag.
        let slot_of = |l: &mut Self| {
            let props = l.b.ins().iadd_imm_s(rc, lay.obj_props as i64);
            let shape = l.b.ins().load(types::I64, flags(), props, 0);
            let want = l.b.ins().load(types::I64, flags(), sp, off(std::mem::offset_of!(Accessor, shape)));
            let same = l.b.ins().icmp(IntCC::Equal, shape, want);
            let ok = l.b.create_block();
            l.b.ins().brif(same, ok, &[], miss, &[]);
            l.b.switch_to_block(ok);
            let data = l.b.ins().load(types::I64, flags(), props, 8);
            let slot = l.b.ins().load(types::I64, flags(), sp, off(std::mem::offset_of!(Accessor, slot)));
            let o = l.b.ins().ishl_imm_u(slot, 4);
            l.b.ins().iadd(data, o)
        };
        // Is tag `vt` one the cached mask accepts? (The empty slot's tag
        // is above every value's, so no bit of the mask.)
        let accepts = |l: &mut Self, vt: CValue| {
            let mask = l.b.ins().load(types::I64, flags(), sp, off(std::mem::offset_of!(Accessor, mask)));
            let w = l.b.ins().uextend(types::I64, vt);
            let bits = l.b.ins().ushr(mask, w);
            l.b.ins().band_imm_u(bits, 1)
        };
        if nargs == 0 {
            let (cnst, other, slotb) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
            let is_const = self.b.ins().icmp_imm_u(IntCC::Equal, kind, ACC_CONST as i64);
            self.b.ins().brif(is_const, cnst, &[], other, &[]);
            self.b.switch_to_block(cnst);
            let va = self.b.ins().iadd_imm_s(sp, off(std::mem::offset_of!(Accessor, value)) as i64);
            let ta = self.temp_addr(t);
            self.store_into(ta, Op::Ptr(va, None));
            self.b.ins().jump(done, &[]);
            self.b.switch_to_block(other);
            let is_slot = self.b.ins().icmp_imm_u(IntCC::Equal, kind, ACC_SLOT as i64);
            self.b.ins().brif(is_slot, slotb, &[], miss, &[]);
            self.b.switch_to_block(slotb);
            let p = slot_of(self);
            let vt = self.load_tag(p);
            let ok = accepts(self, vt);
            let read = self.b.create_block();
            self.b.ins().brif(ok, read, &[], miss, &[]);
            self.b.switch_to_block(read);
            let ta = self.temp_addr(t);
            self.store_into(ta, Op::Ptr(p, None));
            self.b.ins().jump(done, &[]);
            return;
        }
        let (setb, chk, store) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        let is_set = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, kind, ACC_SET as i64);
        self.b.ins().brif(is_set, setb, &[], miss, &[]);
        self.b.switch_to_block(setb);
        let p = slot_of(self);
        let vt = self.load_tag(p);
        let is_ref = self.b.ins().icmp_imm_u(IntCC::Equal, vt, tag::REF as i64);
        self.b.ins().brif(is_ref, miss, &[], chk, &[]);
        self.b.switch_to_block(chk);
        let at = self.load_tag(aa);
        let ok = accepts(self, at);
        self.b.ins().brif(ok, store, &[], miss, &[]);
        self.b.switch_to_block(store);
        // The old value out (none if never initialized), the argument
        // moved in; the old one is released last (its destructor may run).
        let old_t = self.alloc_temp();
        let olda = self.temp_addr(old_t);
        self.copy16(olda, p);
        let empty = self.b.ins().icmp_imm_u(IntCC::Equal, vt, lay.none_tag as i64);
        let null = self.b.ins().iconst(types::I8, tag::NULL as i64);
        let ot = self.b.ins().select(empty, null, vt);
        self.b.ins().store(flags(), ot, olda, 0);
        self.copy16(p, aa);
        self.store_tag(aa, tag::NULL);
        let ta = self.temp_addr(t);
        let (this_b, null_b, fin) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        let ret_this = self.b.ins().icmp_imm_u(IntCC::Equal, kind, ACC_SET_THIS as i64);
        self.b.ins().brif(ret_this, this_b, &[], null_b, &[]);
        self.b.switch_to_block(this_b);
        let cnt = self.b.ins().load(types::I64, flags(), rc, 0);
        let cnt = self.b.ins().iadd_imm_s(cnt, 1);
        self.b.ins().store(flags(), cnt, rc, 0);
        self.store_tag(ta, tag::OBJECT);
        self.b.ins().store(flags(), rc, ta, tag::PAYLOAD);
        self.b.ins().jump(fin, &[]);
        self.b.switch_to_block(null_b);
        self.store_tag(ta, tag::NULL);
        self.b.ins().jump(fin, &[]);
        self.b.switch_to_block(fin);
        let olda = self.temp_addr(old_t);
        self.release(Op::Ptr(olda, Some(old_t)));
        self.b.ins().jump(done, &[]);
    }

    /// `A::m(...)` on a static method: like `direct_call`, for the cached
    /// calling scope and `static` class.
    #[allow(clippy::too_many_arguments)]
    fn direct_static_call(&mut self, lay: super::Layout, cr: CValue, sp: CValue, aa: CValue, n: CValue, la: CValue, meta: CValue) -> Op {
        self.emit_direct(
            lay,
            sp,
            lay.msite_frame,
            aa,
            n,
            |l, slow, _| {
                let caller = l.b.ins().load(types::I64, flags(), l.cx, lay.cx_frame);
                let scope = l.b.ins().load(types::I64, flags(), caller, lay.frame_class);
                let want = l.b.ins().load(types::I64, flags(), sp, lay.msite_scope);
                let same = l.b.ins().icmp(IntCC::Equal, scope, want);
                let (c1, c2, c3) = (l.b.create_block(), l.b.create_block(), l.b.create_block());
                l.b.ins().brif(same, c1, &[], slow, &[]);
                l.b.switch_to_block(c1);
                let st = l.b.ins().load(types::I64, flags(), caller, lay.frame_static);
                let want = l.b.ins().load(types::I64, flags(), sp, lay.msite_class);
                let same = l.b.ins().icmp(IntCC::Equal, st, want);
                l.b.ins().brif(same, c2, &[], slow, &[]);
                l.b.switch_to_block(c2);
                let entry = l.b.ins().load(types::I64, flags(), sp, lay.msite_direct);
                l.b.ins().brif(entry, c3, &[], slow, &[]);
                l.b.switch_to_block(c3);
                // An instance method (`parent::m()`): the caller's `$this`,
                // of the class cached, is lent to it.
                let want = l.b.ins().load(types::I64, flags(), sp, lay.msite_this);
                let (inst, has_this, out) = (l.b.create_block(), l.b.create_block(), l.b.create_block());
                l.b.append_block_param(out, types::I64);
                let none = l.b.ins().iconst(types::I64, 0);
                l.b.ins().brif(want, inst, &[], out, &[none.into()]);
                l.b.switch_to_block(inst);
                let th = l.b.ins().load(types::I64, flags(), caller, lay.frame_this);
                l.b.ins().brif(th, has_this, &[], slow, &[]);
                l.b.switch_to_block(has_this);
                let cls = l.b.ins().load(types::I64, flags(), th, lay.obj_class);
                let same = l.b.ins().icmp(IntCC::Equal, cls, want);
                l.b.ins().brif(same, out, &[th.into()], slow, &[]);
                l.b.switch_to_block(out);
                let this = l.b.block_params(out)[0];
                (entry, Some(this), None)
            },
            |l, ta| l.call_checked("rt_static_call", &[l.cx, cr, sp, aa, n, la, meta, ta]),
        )
    }

    /// `new C(...)`: the object first (PHP evaluates the arguments after
    /// creating it), then the constructor called directly like a method;
    /// `rt_new` handles what the site can't cache.
    fn new_native(&mut self, lay: super::Layout, cr: CValue, sp: CValue, args: &'static [ast::Arg], off: u32) -> L<Op> {
        let t = self.alloc_temp();
        let ta = self.temp_addr(t);
        let st = self.call("rt_new_object", &[self.cx, cr, sp, ta]).unwrap();
        let failed = self.b.ins().icmp_imm_s(IntCC::Equal, st, 1);
        let cont = self.b.create_block();
        self.b.ins().brif(failed, self.unwind, &[], cont, &[]);
        self.b.switch_to_block(cont);
        let (aa, la, n) = self.args(args, ByRef::Unknown)?;
        let (ctor_b, chk3, noctor_b, slow_b, done) =
            (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(slow_b);
        let is0 = self.b.ins().icmp_imm_s(IntCC::Equal, st, 0);
        self.b.ins().brif(is0, ctor_b, &[], chk3, &[]);
        self.b.switch_to_block(chk3);
        let is3 = self.b.ins().icmp_imm_s(IntCC::Equal, st, 3);
        self.b.ins().brif(is3, noctor_b, &[], slow_b, &[]);

        self.b.switch_to_block(ctor_b);
        let meta = self.iconst(0);
        let r = self.emit_direct(
            lay,
            sp,
            lay.msite_frame,
            aa,
            n,
            |l, slow, _| {
                let entry = l.b.ins().load(types::I64, flags(), sp, lay.msite_direct);
                let c1 = l.b.create_block();
                l.b.ins().brif(entry, c1, &[], slow, &[]);
                l.b.switch_to_block(c1);
                let rc = l.b.ins().load(types::I64, flags(), ta, tag::PAYLOAD);
                (entry, Some(rc), None)
            },
            |l, out| l.call_checked("rt_new_ctor", &[l.cx, ta, sp, aa, n, la, meta, out]),
        );
        // The constructor's own return value is discarded.
        self.release(r);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(noctor_b);
        // No constructor: the arguments were evaluated for their effects.
        self.call("rt_release_n", &[aa, n]);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(slow_b);
        // Created here, after the arguments: back at the `new`'s line.
        if off != self.stmt_start || !args.is_empty() {
            self.set_pos(off);
        }
        let meta = self.iconst(0);
        self.call_checked("rt_new", &[self.cx, cr, sp, aa, n, la, meta, ta]);
        self.b.ins().jump(done, &[]);

        self.b.switch_to_block(done);
        self.write_back(la, args.len())?;
        Ok(Op::Ptr(self.temp_addr(t), Some(t)))
    }

    /// The shared shape of a direct call. `guard` branches to `slow` unless
    /// the cached target applies, returning (entry, `$this`'s Rc pointer
    /// for methods); the callee frame is the site's for the current call
    /// depth (`sf_off`: the site's `SiteFrame`); `slow_call` runs the
    /// helper path, writing the result to the given address.
    #[allow(clippy::too_many_arguments)]
    fn emit_direct(
        &mut self,
        lay: super::Layout,
        sp: CValue,
        sf_off: i32,
        aa: CValue,
        n: CValue,
        guard: impl FnOnce(&mut Self, Block, (usize, Block)) -> (CValue, Option<CValue>, Option<CValue>),
        slow_call: impl FnOnce(&mut Self, CValue),
    ) -> Op {
        use super::rt::SiteFrame;
        let t = self.alloc_temp();
        let (slow, done) = (self.b.create_block(), self.b.create_block());
        // (The guard may pick the site's second entry, with its own frames,
        // or answer without a call: then it stores the result in `t` and
        // jumps to `done`.)
        let (entry, this, frames) = guard(self, slow, (t, done));
        let (sp, sf_off) = match frames {
            Some(a) => (a, 0),
            None => (sp, sf_off),
        };
        let (c2, fast, chk) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        let depth_off = sf_off + SiteFrame::DEPTH as i32;
        let d = self.b.ins().load(types::I32, flags(), sp, depth_off);
        let len = self.b.ins().load(types::I32, flags(), sp, sf_off + SiteFrame::LEN as i32);
        let have = self.b.ins().icmp(IntCC::UnsignedLessThan, d, len);
        self.b.ins().brif(have, c2, &[], slow, &[]);
        self.b.switch_to_block(c2);
        // Room on the native stack (the general path raises PHP's error
        // for runaway recursion): at least 256 KiB above the task's limit
        // (no limit outside tasks: the difference is then the address).
        let spv = self.b.ins().get_stack_pointer(types::I64);
        let la = self.iconst(phpx_runtime::stack_limit_addr() as i64);
        let lim = self.b.ins().load(types::I64, flags(), la, 0);
        let room = self.b.ins().isub(spv, lim);
        let low = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, room, 256 * 1024);
        self.b.ins().brif(low, slow, &[], fast, &[]);

        self.b.switch_to_block(fast);
        let buf = self.b.ins().load(types::I64, flags(), sp, sf_off + SiteFrame::BUF as i32);
        let dw = self.b.ins().uextend(types::I64, d);
        let off = self.b.ins().ishl_imm_u(dw, 3);
        let fp = self.b.ins().iadd(buf, off);
        let frame = self.b.ins().load(types::I64, flags(), fp, 0);
        // (Lent: to a call suspended in another task.)
        let lent = self.b.ins().load(types::I8, flags(), frame, lay.frame_lent);
        let take = self.b.create_block();
        self.b.ins().brif(lent, slow, &[], take, &[]);
        self.b.switch_to_block(take);
        let one = self.b.ins().iconst(types::I8, 1);
        self.b.ins().store(flags(), one, frame, lay.frame_lent);
        let d1 = self.b.ins().iadd_imm_s(d, 1);
        self.b.ins().store(flags(), d1, sp, depth_off);
        if let Some(rc) = this {
            // Lend `$this` to the frame (one more reference); 0: none (a
            // static method through `A::m()`).
            let (lend, lent_b) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(rc, lend, &[], lent_b, &[]);
            self.b.switch_to_block(lend);
            let cnt = self.b.ins().load(types::I64, flags(), rc, 0);
            let cnt = self.b.ins().iadd_imm_s(cnt, 1);
            self.b.ins().store(flags(), cnt, rc, 0);
            self.b.ins().store(flags(), rc, frame, lay.frame_this);
            self.b.ins().jump(lent_b, &[]);
            self.b.switch_to_block(lent_b);
        }
        // The callee's Cx: same interpreter, the site's frame.
        let it = self.b.ins().load(types::I64, flags(), self.cx, 0);
        let cx2 = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));
        self.b.ins().stack_store(types::I64, it, cx2, 0);
        self.b.ins().stack_store(types::I64, frame, cx2, lay.cx_frame);
        let cx2a = self.b.ins().stack_addr(types::I64, cx2, 0);
        let pa = self.pos_addr;
        let saved = self.b.ins().load(types::I64, flags(), pa, 0);
        let mut sig = self.module.make_signature();
        for _ in 0..4 {
            sig.params.push(cranelift_codegen::ir::AbiParam::new(types::I64));
        }
        sig.returns.push(cranelift_codegen::ir::AbiParam::new(types::I64));
        let sigref = self.b.import_signature(sig);
        let ta = self.temp_addr(t);
        let inst = self.b.ins().call_indirect(sigref, entry, &[cx2a, aa, n, ta]);
        let st = self.b.inst_results(inst)[0];
        self.b.ins().store(flags(), saved, pa, 0);
        let zero = self.b.ins().iconst(types::I8, 0);
        self.b.ins().store(flags(), zero, frame, lay.frame_lent);
        self.b.ins().store(flags(), d, sp, depth_off);
        if let Some(rc) = this {
            // Take `$this` back and drop the frame's reference.
            let (back, back_done) = (self.b.create_block(), self.b.create_block());
            self.b.ins().brif(rc, back, &[], back_done, &[]);
            self.b.switch_to_block(back);
            let null = self.iconst(0);
            self.b.ins().store(flags(), null, frame, lay.frame_this);
            let tt = self.alloc_temp();
            let tta = self.temp_addr(tt);
            self.store_tag(tta, tag::OBJECT);
            self.b.ins().store(flags(), rc, tta, tag::PAYLOAD);
            self.clear(tta);
            self.free.push(tt);
            self.b.ins().jump(back_done, &[]);
            self.b.switch_to_block(back_done);
        }
        // Plain functions leave the frame untouched unless they ran
        // something that needed interpreter variables.
        let vl = self.b.ins().load(types::I64, flags(), frame, lay.frame_vars_len);
        let al = self.b.ins().load(types::I64, flags(), frame, lay.frame_args_len);
        let used = self.b.ins().bor(vl, al);
        let (reset, after) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(used, reset, &[], after, &[]);
        self.b.set_cold_block(reset);
        self.b.switch_to_block(reset);
        self.call("rt_site_reset", &[frame]);
        self.b.ins().jump(after, &[]);
        self.b.switch_to_block(after);
        self.b.ins().brif(st, chk, &[], done, &[]);
        self.b.switch_to_block(chk);
        // 2: declined before binding (try the general path); else an error.
        let declined = self.b.ins().icmp_imm_s(IntCC::Equal, st, 2);
        self.b.ins().brif(declined, slow, &[], self.unwind, &[]);

        self.b.switch_to_block(slow);
        let ta = self.temp_addr(t);
        slow_call(self, ta);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
        let ta = self.temp_addr(t);
        Op::Ptr(ta, Some(t))
    }

    fn helper_value(&mut self, name: &'static str, args: impl FnOnce(&mut Self) -> Vec<CValue>) -> L<Op> {
        let mut a = args(self);
        let t = self.alloc_temp();
        let ta = self.temp_addr(t);
        a.push(ta);
        self.call_checked(name, &a);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    /// Inline-cache guard for a property site: branches to `slow` unless the
    /// value at `oa` is an object whose shape and the caller's scope match
    /// the cache at `site + off_shape/off_scope`. Returns (props pointer)
    /// in the fall-through block.
    fn prop_guard(&mut self, lay: super::Layout, oa: CValue, site: CValue, off_shape: i32, off_scope: i32, slow: Block) -> CValue {
        let tg = self.load_tag(oa);
        let is_obj = self.b.ins().icmp_imm_s(IntCC::Equal, tg, tag::OBJECT as i64);
        let (c1, c2, ok) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_obj, c1, &[], slow, &[]);
        self.b.switch_to_block(c1);
        let rc = self.b.ins().load(types::I64, flags(), oa, tag::PAYLOAD);
        let props = self.b.ins().iadd_imm_s(rc, lay.obj_props as i64);
        let shape = self.b.ins().load(types::I64, flags(), props, 0);
        let cached = self.b.ins().load(types::I64, flags(), site, off_shape);
        let same = self.b.ins().icmp(IntCC::Equal, shape, cached);
        self.b.ins().brif(same, c2, &[], slow, &[]);
        self.b.switch_to_block(c2);
        let frame = self.b.ins().load(types::I64, flags(), self.cx, lay.cx_frame);
        let scope = self.b.ins().load(types::I64, flags(), frame, lay.frame_class);
        let cs = self.b.ins().load(types::I64, flags(), site, off_scope);
        let same = self.b.ins().icmp(IntCC::Equal, scope, cs);
        self.b.ins().brif(same, ok, &[], slow, &[]);
        self.b.switch_to_block(ok);
        props
    }

    /// Address of the cached slot; branches to `slow` if it's empty or a
    /// reference.
    fn prop_slot(&mut self, lay: super::Layout, props: CValue, site: CValue, off_slot: i32, slow: Block) -> CValue {
        let data = self.b.ins().load(types::I64, flags(), props, 8);
        let slot = self.b.ins().load(types::I64, flags(), site, off_slot);
        let off = self.b.ins().ishl_imm_u(slot, 4);
        let p = self.b.ins().iadd(data, off);
        let vt = self.load_tag(p);
        let empty = self.b.ins().icmp_imm_s(IntCC::Equal, vt, lay.none_tag as i8 as i64);
        let is_ref = self.b.ins().icmp_imm_s(IntCC::Equal, vt, tag::REF as i64);
        let bad = self.b.ins().bor(empty, is_ref);
        let ok = self.b.create_block();
        self.b.ins().brif(bad, slow, &[], ok, &[]);
        self.b.switch_to_block(ok);
        p
    }

    /// Built-in inlined into generated code (guarded: a namespaced user
    /// function with the same name disables it at run time).
    /// Branch to `builtin` when a call by `name` means the built-in, else
    /// to `other`: decided by the site's first run in each request (see
    /// `shadow_flag`, `crate::value::site_mark`).
    pub(super) fn shadow_check(&mut self, name: &str, builtin: Block, other: Block) {
        let Some(flag) = self.shadow_flag(name) else {
            self.b.ins().jump(builtin, &[]);
            return;
        };
        let mark = crate::value::site_mark();
        let ma = self.iconst(mark as *const _ as i64);
        let marked = self.b.ins().load(types::I8, flags(), ma, 0);
        let decide = self.b.create_block();
        self.b.ins().brif(marked, builtin, &[], decide, &[]);
        self.b.set_cold_block(decide);
        self.b.switch_to_block(decide);
        let fa = self.iconst(flag as *const _ as i64);
        let shadowed = self.b.ins().load(types::I8, flags(), fa, 0);
        let set = self.b.create_block();
        self.b.ins().brif(shadowed, other, &[], set, &[]);
        self.b.set_cold_block(set);
        self.b.switch_to_block(set);
        let one = self.b.ins().iconst(types::I8, 1);
        self.b.ins().store(flags(), one, ma, 0);
        self.b.ins().jump(builtin, &[]);
    }

    /// The flag compiled code checks before taking a built-in's inline
    /// path for a call by `name` (as written): set while a function of
    /// that name in this code's namespace is declared, which the call
    /// would mean instead. None: no check (a fully qualified name, or
    /// global code, where no function can share a built-in's name).
    pub(super) fn shadow_flag(&self, name: &str) -> Option<&'static std::sync::atomic::AtomicBool> {
        if name.starts_with('\\') {
            return None;
        }
        let lname = name.to_ascii_lowercase();
        let Some(ctx) = &self.ctx else { return Some(crate::value::builtin_shadowed()) };
        if ctx.use_fns.contains_key(&lname) {
            // `use function`: never the built-in.
            static ALWAYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            return Some(&ALWAYS);
        }
        if ctx.ns.is_empty() {
            return None;
        }
        Some(crate::value::shadow_flag(&format!("{}\\{lname}", ctx.ns.to_ascii_lowercase())))
    }

    fn intrinsic(&mut self, kind: Intrinsic, name: &'static str, args: &'static [ast::Arg]) -> L<Op> {
        let t = self.alloc_temp();
        let (fast, slow, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(slow);
        self.shadow_check(name, fast, slow);
        // Generic call: also the fallback for the cases the fast code skips.
        let generic = |l: &mut Self| -> L<()> {
            let site = Box::new(CallSite { direct: Default::default(), frame: Default::default(), name, cached: Default::default(), builtin: Default::default(), bspec: Default::default(), builtin_ref: Default::default(), jit: Default::default() });
            let sp = &*site as *const CallSite as i64;
            l.sites.push(site);
            let (aa, la, n_) = l.args(args, ByRef::None)?;
            let sp = l.iconst(sp);
            let meta = l.iconst(0);
            let ta = l.temp_addr(t);
            l.call_checked("rt_call", &[l.cx, sp, aa, n_, la, meta, ta]);
            l.write_back(la, args.len())?;
            l.b.ins().jump(merge, &[]);
            Ok(())
        };
        self.b.switch_to_block(fast);
        let v = self.expr(&args[0].value)?;
        let v = self.materialize(v);
        let va = self.addr(v).unwrap();
        let tg = self.load_tag(va);
        let ta = self.temp_addr(t);
        match kind {
            Intrinsic::IsTag(tags) => {
                let mut hit = self.b.ins().iconst(types::I8, 0);
                for &tg_want in tags {
                    let e = self.b.ins().icmp_imm_s(IntCC::Equal, tg, tg_want as i64);
                    hit = self.b.ins().bor(hit, e);
                }
                self.store_into(ta, Op::Bool(hit));
                self.release(v);
                self.b.ins().jump(merge, &[]);
            }
            Intrinsic::Strlen | Intrinsic::Count => {
                // strlen: string header len (offset 8). count: array's mirrored count.
                let (want, off) = match kind {
                    Intrinsic::Strlen => (tag::STR, 8),
                    _ => (tag::ARRAY, 16 + 16),
                };
                let ok = self.b.ins().icmp_imm_s(IntCC::Equal, tg, want as i64);
                let (yes, no) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(ok, yes, &[], no, &[]);
                self.b.switch_to_block(yes);
                let p = self.b.ins().load(types::I64, flags(), va, tag::PAYLOAD);
                let n = self.b.ins().load(types::I64, flags(), p, off);
                self.store_into(ta, Op::Int(n));
                // Both branches drop the operand at run time; its temp goes
                // back to the free list once (below), not once per branch.
                if let Op::Ptr(p, Some(_)) = v {
                    self.clear(p);
                }
                self.b.ins().jump(merge, &[]);
                // Other types: the real built-in (errors, Countable, ...)
                // with the value already evaluated.
                self.b.switch_to_block(no);
                let site = Box::new(CallSite { direct: Default::default(), frame: Default::default(), name, cached: Default::default(), builtin: Default::default(), bspec: Default::default(), builtin_ref: Default::default(), jit: Default::default() });
                let sp = &*site as *const CallSite as i64;
                self.sites.push(site);
                let (slot, ids) = self.alloc_array(1);
                let aa = self.b.ins().stack_addr(types::I64, slot, 0);
                self.store_into(aa, Op::Ptr(va, None));
                self.release(v);
                // One argument pointer, then the by-reference mask word.
                let lslot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, 16, 3));
                let zero = self.iconst(0);
                self.b.ins().stack_store(types::I64, zero, lslot, 0);
                self.b.ins().stack_store(types::I64, zero, lslot, 8);
                let la = self.b.ins().stack_addr(types::I64, lslot, 0);
                let (sp, one) = (self.iconst(sp), self.iconst(1));
                let ta = self.temp_addr(t);
                self.call_checked("rt_call", &[self.cx, sp, aa, one, la, zero, ta]);
                self.free.extend(ids);
                self.b.ins().jump(merge, &[]);
            }
        }
        self.b.switch_to_block(slow);
        generic(self)?;
        self.b.switch_to_block(merge);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    /// `$arr[$k]`: packed array + in-range int key read inline; everything
    /// else (hash arrays, strings, ArrayAccess, warnings) via rt_index_get.
    fn index_read(&mut self, ba: CValue, ka: CValue) -> L<Op> {
        if super::layout().is_none() {
            return self.helper_value("rt_index_get", |l| vec![l.cx, ba, ka]);
        }
        let t = self.alloc_temp();
        let ta = self.temp_addr(t);
        let (slow, merge) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(slow);
        let p = self.packed_elem(ba, ka, slow);
        self.store_into(ta, Op::Ptr(p, None));
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(slow);
        let ta2 = self.temp_addr(t);
        self.call_checked("rt_index_get", &[self.cx, ba, ka, ta2]);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    /// Address of `$arr[$k]` when the value at `ba` is a packed array, the
    /// key at `ka` an in-range int and the element not a reference;
    /// otherwise branches to `slow`.
    fn packed_elem(&mut self, ba: CValue, ka: CValue, slow: Block) -> CValue {
        let tg = self.load_tag(ba);
        let is_arr = self.b.ins().icmp_imm_s(IntCC::Equal, tg, tag::ARRAY as i64);
        let (c1, c2, c3, ok) = (self.b.create_block(), self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_arr, c1, &[], slow, &[]);
        self.b.switch_to_block(c1);
        let kt = self.load_tag(ka);
        let is_int = self.b.ins().icmp_imm_s(IntCC::Equal, kt, tag::INT as i64);
        self.b.ins().brif(is_int, c2, &[], slow, &[]);
        self.b.switch_to_block(c2);
        let rc = self.b.ins().load(types::I64, flags(), ba, tag::PAYLOAD);
        let arr = self.b.ins().iadd_imm_s(rc, 16);
        let pk = self.b.ins().load(types::I64, flags(), arr, 0);
        let len = self.b.ins().load(types::I64, flags(), arr, 8);
        let k = self.b.ins().load(types::I64, flags(), ka, tag::PAYLOAD);
        // Unsigned compare: negative keys fail too. A hash array has len 0.
        let inb = self.b.ins().icmp(IntCC::UnsignedLessThan, k, len);
        self.b.ins().brif(inb, c3, &[], slow, &[]);
        self.b.switch_to_block(c3);
        let off = self.b.ins().ishl_imm_u(k, 4);
        let p = self.b.ins().iadd(pk, off);
        let vt = self.load_tag(p);
        let is_ref = self.b.ins().icmp_imm_s(IntCC::Equal, vt, tag::REF as i64);
        self.b.ins().brif(is_ref, slow, &[], ok, &[]);
        self.b.switch_to_block(ok);
        p
    }

    /// `$obj->name` read with an inline cache; the miss path fills it.
    fn prop_read(&mut self, oa: CValue, sp: i64) -> L<Op> {
        let Some(lay) = super::layout() else {
            let sp = self.iconst(sp);
            return self.helper_value("rt_prop_read", |l| vec![l.cx, oa, sp]);
        };
        let t = self.alloc_temp();
        let ta = self.temp_addr(t);
        let site = self.iconst(sp);
        let (slow, merge) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(slow);
        let props = self.prop_guard(lay, oa, site, super::rt::PROPSITE_SHAPE, super::rt::PROPSITE_SCOPE, slow);
        let p = self.prop_slot(lay, props, site, super::rt::PROPSITE_SLOT, slow);
        self.store_into(ta, Op::Ptr(p, None));
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(slow);
        let site = self.iconst(sp);
        let ta2 = self.temp_addr(t);
        self.call_checked("rt_prop_read", &[self.cx, oa, site, ta2]);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    /// `$obj->name = value` with an inline cache (plain properties only).
    fn prop_write(&mut self, object: &'static Expr, name: &'static str, value: &'static Expr) -> L<Op> {
        let o = self.expr(object)?;
        // (`$this` never changes: read in place.)
        let o = if matches!(&object.kind, ExprKind::Var(n) if n == "this") { o } else { self.own(o) };
        let o = self.materialize(o);
        let oa = self.addr(o).unwrap();
        let v = self.expr(value)?;
        let r = self.prop_store(oa, name, v);
        self.release(o);
        r
    }

    /// The store of `prop_write`, for an object at `oa` and a computed value.
    fn prop_store(&mut self, oa: CValue, name: &'static str, v: Op) -> L<Op> {
        let v = self.own(v);
        let v = self.materialize(v);
        let va = self.addr(v).unwrap();
        let site = Box::new(super::rt::PropSite::new(name));
        let sp = &*site as *const super::rt::PropSite as i64;
        self.psites.push(site);
        let Some(lay) = super::layout() else {
            let s = self.iconst(sp);
            self.call_checked("rt_prop_write", &[self.cx, oa, s, va, va]);
            return Ok(v);
        };
        let old_t = self.alloc_temp();
        let site_v = self.iconst(sp);
        let (typed, slow, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(slow);
        let props = self.prop_guard(lay, oa, site_v, super::rt::PROPSITE_WSHAPE, super::rt::PROPSITE_WSCOPE, typed);
        let p = self.prop_slot(lay, props, site_v, super::rt::PROPSITE_WSLOT, typed);
        // Old value out, a copy of the new value in; the old one is
        // released after the merge (its destructor may run PHP code).
        let olda = self.temp_addr(old_t);
        self.copy16(olda, p);
        self.store_into(p, Op::Ptr(va, None));
        self.b.ins().jump(merge, &[]);
        // A typed property, for a value of exactly its type (no coercion).
        self.b.switch_to_block(typed);
        let site_v = self.iconst(sp);
        let want = self.b.ins().load(types::I8, flags(), site_v, super::rt::PROPSITE_TTAG);
        let have = self.load_tag(va);
        let same = self.b.ins().icmp(IntCC::Equal, want, have);
        let set = self.b.ins().icmp_imm_u(IntCC::NotEqual, want, 0);
        let fits = self.b.ins().band(same, set);
        // Null into a nullable type; an object of the accepted class.
        let nullable = self.b.ins().load(types::I8, flags(), site_v, super::rt::PROPSITE_TNULL);
        let is_null = self.b.ins().icmp_imm_u(IntCC::Equal, have, tag::NULL as i64);
        let null_ok = self.b.ins().band(is_null, nullable);
        let fits = self.b.ins().bor(fits, null_ok);
        let (go, obj_chk, obj_cls) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(fits, go, &[], obj_chk, &[]);
        self.b.switch_to_block(obj_chk);
        let is_obj = self.b.ins().icmp_imm_u(IntCC::Equal, have, tag::OBJECT as i64);
        self.b.ins().brif(is_obj, obj_cls, &[], slow, &[]);
        self.b.switch_to_block(obj_cls);
        let rc = self.b.ins().load(types::I64, flags(), va, tag::PAYLOAD);
        let cls = self.b.ins().load(types::I64, flags(), rc, lay.obj_class);
        let accepted = self.b.ins().load(types::I64, flags(), site_v, super::rt::PROPSITE_TCLASSW);
        let ok = self.b.ins().icmp(IntCC::Equal, cls, accepted);
        self.b.ins().brif(ok, go, &[], slow, &[]);
        self.b.switch_to_block(go);
        // The object's shape and the calling scope: the first entry's, or
        // the second's (same type).
        let tg = self.load_tag(oa);
        let is_obj = self.b.ins().icmp_imm_s(IntCC::Equal, tg, tag::OBJECT as i64);
        let (c1, second, found) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.append_block_param(found, types::I64);
        self.b.ins().brif(is_obj, c1, &[], slow, &[]);
        self.b.switch_to_block(c1);
        let rc = self.b.ins().load(types::I64, flags(), oa, tag::PAYLOAD);
        let props = self.b.ins().iadd_imm_s(rc, lay.obj_props as i64);
        let shape = self.b.ins().load(types::I64, flags(), props, 0);
        let frame = self.b.ins().load(types::I64, flags(), self.cx, lay.cx_frame);
        let scope = self.b.ins().load(types::I64, flags(), frame, lay.frame_class);
        for (i, (shape_off, scope_off, slot_off, next)) in [
            (super::rt::PROPSITE_TSHAPE, super::rt::PROPSITE_TSCOPE, super::rt::PROPSITE_TSLOT, second),
            (super::rt::PROPSITE_TSHAPE2, super::rt::PROPSITE_TSCOPE2, super::rt::PROPSITE_TSLOT2, slow),
        ]
        .into_iter()
        .enumerate()
        {
            let want = self.b.ins().load(types::I64, flags(), site_v, shape_off);
            let same = self.b.ins().icmp(IntCC::Equal, shape, want);
            let want = self.b.ins().load(types::I64, flags(), site_v, scope_off);
            let same2 = self.b.ins().icmp(IntCC::Equal, scope, want);
            let both = self.b.ins().band(same, same2);
            let hit = self.b.create_block();
            self.b.ins().brif(both, hit, &[], next, &[]);
            self.b.switch_to_block(hit);
            let slot = self.b.ins().load(types::I64, flags(), site_v, slot_off);
            self.b.ins().jump(found, &[slot.into()]);
            if i == 0 {
                self.b.switch_to_block(second);
            }
        }
        self.b.switch_to_block(found);
        let slot = self.b.block_params(found)[0];
        // (A typed property is uninitialized until its first write, as
        // in a constructor: then there is no old value.)
        let data = self.b.ins().load(types::I64, flags(), props, 8);
        let off = self.b.ins().ishl_imm_u(slot, 4);
        let p = self.b.ins().iadd(data, off);
        let vt = self.load_tag(p);
        let is_ref = self.b.ins().icmp_imm_s(IntCC::Equal, vt, tag::REF as i64);
        let (not_ref, fresh, replace) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_ref, slow, &[], not_ref, &[]);
        self.b.switch_to_block(not_ref);
        let empty = self.b.ins().icmp_imm_s(IntCC::Equal, vt, lay.none_tag as i8 as i64);
        self.b.ins().brif(empty, fresh, &[], replace, &[]);
        self.b.switch_to_block(fresh);
        let olda = self.temp_addr(old_t);
        self.store_tag(olda, tag::NULL);
        self.store_into(p, Op::Ptr(va, None));
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(replace);
        let olda = self.temp_addr(old_t);
        self.copy16(olda, p);
        self.store_into(p, Op::Ptr(va, None));
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(slow);
        let s = self.iconst(sp);
        self.call_checked("rt_prop_write", &[self.cx, oa, s, va, va]);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        let olda = self.temp_addr(old_t);
        self.release(Op::Ptr(olda, Some(old_t)));
        Ok(v)
    }

    /// `?->`: if the object is null the whole call/fetch is null (args unevaluated).
    /// `?->`: run `f` unless the object at `oa` is null; inside a chain
    /// (`chain`), null skips to the chain's end.
    fn nullsafe(&mut self, oa: CValue, chain: Option<Block>, f: impl FnOnce(&mut Self) -> L<Op>) -> L<Op> {
        if let Some(skip) = chain {
            let tg = self.load_tag(oa);
            let is_null = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::NULL as i64);
            let run = self.b.create_block();
            self.b.ins().brif(is_null, skip, &[], run, &[]);
            self.b.switch_to_block(run);
            return f(self);
        }
        let t = self.alloc_temp();
        let tg = self.load_tag(oa);
        let is_null = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::NULL as i64);
        let (skip, run, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_null, skip, &[], run, &[]);
        self.b.switch_to_block(run);
        let r = f(self)?;
        let ta = self.temp_addr(t);
        self.store_into(ta, r);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(skip);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    fn make_closure(&mut self, kind: i64, node: i64, vals: Vec<Option<u32>>) -> L<Op> {
        let (slot, ids) = self.alloc_array(vals.len());
        for (i, v) in vals.iter().enumerate() {
            let a = self.b.ins().stack_addr(types::I64, slot, (i * 16) as i32);
            match v {
                Some(l) => self.store_into(a, Op::Local(*l)),
                None => self.store_tag(a, tag::NULL),
            }
        }
        let va = self.b.ins().stack_addr(types::I64, slot, 0);
        let k = self.iconst(kind);
        let nd = self.iconst(node);
        let n = self.iconst(vals.len() as i64);
        let r = self.helper_value("rt_make_closure", |l| vec![l.cx, k, nd, va, n])?;
        self.free.extend(ids);
        Ok(r)
    }

    /// Evaluate call arguments into a contiguous temp array (moved in) plus a
    /// parallel array of local addresses (for by-reference parameters).
    /// Spread/named-argument descriptor for helpers: the AST arg slice
    /// (null when all arguments are plain positional).
    fn arg_meta(&mut self, args: &'static [ast::Arg]) -> CValue {
        if args.iter().any(|a| a.spread || a.name.is_some()) {
            self.iconst(args.as_ptr() as i64)
        } else {
            self.iconst(0)
        }
    }

    /// Lay out the arguments for a call: values in a contiguous array, and
    /// for each argument a pointer the runtime may bind by reference (a
    /// local's slot, or a write-back temp for a property/element), plus a
    /// final word where the runtime reports which ones it bound. Call
    /// `write_back` after the call.
    fn args(&mut self, args: &'static [ast::Arg], by_ref: ByRef) -> L<(CValue, CValue, CValue)> {
        let n = args.len();
        let (slot, ids) = self.alloc_array(n);
        let lslot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, ((n + 1) * 8) as u32, 3));
        for i in 0..n {
            let a = self.b.ins().stack_addr(types::I64, slot, (i * 16) as i32);
            self.store_tag(a, tag::NULL);
        }
        let mut wbs = vec![];
        for (i, a) in args.iter().enumerate() {
            let place = !a.spread && a.name.is_none() && matches!(a.value.kind, ExprKind::Prop { .. } | ExprKind::Index { .. } | ExprKind::StaticProp { .. });
            let wb = if place && by_ref.may_be_ref(i) && i < 64 {
                if !pure_place(&a.value) {
                    if let ByRef::Builtin(_) = by_ref {
                        return Err("by-reference argument with side effects");
                    }
                    None
                } else {
                    Some(self.alloc_temp())
                }
            } else {
                None
            };
            let la = match (self.local_of(&a.value), wb) {
                (Some(l), _) => {
                    // (Passing by reference defines the variable.)
                    self.mark_defined(l);
                    self.local_addr(l)
                }
                (None, Some(t)) => self.temp_addr(t),
                (None, None) => self.iconst(0),
            };
            self.b.ins().stack_store(types::I64, la, lslot, (i * 8) as i32);
            let v = self.expr(&a.value)?;
            let dst = self.b.ins().stack_addr(types::I64, slot, (i * 16) as i32);
            self.store_into(dst, v);
            if let Some(t) = wb {
                // The temp starts as a copy of the argument.
                let ta = self.temp_addr(t);
                let dst = self.b.ins().stack_addr(types::I64, slot, (i * 16) as i32);
                self.store_into(ta, Op::Ptr(dst, None));
                wbs.push(WriteBack { pos: i, target: &a.value, temp: t });
            }
        }
        let zero = self.iconst(0);
        self.b.ins().stack_store(types::I64, zero, lslot, (n * 8) as i32);
        self.free.extend(ids);
        self.write_backs.push(wbs);
        let aa = self.b.ins().stack_addr(types::I64, slot, 0);
        let la = self.b.ins().stack_addr(types::I64, lslot, 0);
        Ok((aa, la, self.iconst(n as i64)))
    }

    /// After a call: store arguments the callee bound by reference back to
    /// their property or element.
    fn write_back(&mut self, la: CValue, n: usize) -> L<()> {
        let wbs = self.write_backs.pop().unwrap_or_default();
        if wbs.is_empty() {
            return Ok(());
        }
        let mask = self.b.ins().load(types::I64, flags(), la, (n * 8) as i32);
        for wb in wbs {
            let (store, skip, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
            let bit = self.b.ins().band_imm_s(mask, 1i64 << wb.pos);
            self.b.ins().brif(bit, store, &[], skip, &[]);
            self.b.switch_to_block(store);
            let (root, name, keys) = self.place(wb.target, false)?;
            let (np, nl) = name;
            let (kslot, kn, kids) = keys;
            let va = self.temp_addr(wb.temp);
            let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
            let knv = self.iconst(kn as i64);
            let ap = self.iconst(0);
            let ra = root.addr;
            let r = self.helper_value("rt_place_assign", |l| vec![l.cx, ra, np, nl, ka, knv, ap, va])?;
            self.release(r);
            self.release_ids(&kids);
            self.release_root(root);
            self.b.ins().jump(done, &[]);
            self.b.switch_to_block(skip);
            self.b.ins().jump(done, &[]);
            self.b.switch_to_block(done);
            let ta = self.temp_addr(wb.temp);
            self.clear(ta);
            self.free.push(wb.temp);
        }
        Ok(())
    }

    fn match_expr(&mut self, subject: &'static Expr, arms: &'static [ast::MatchArm]) -> L<Op> {
        let s = self.expr(subject)?;
        let s = self.own(s);
        let s = self.materialize(s);
        let sa = self.addr(s).unwrap();
        let t = self.alloc_temp();
        let merge = self.b.create_block();
        let mut default = None;
        for arm in arms {
            let Some(conds) = &arm.conds else {
                default = Some(&arm.body);
                continue;
            };
            let body = self.b.create_block();
            for c in conds {
                let cv = self.expr(c)?;
                let cv = self.materialize(cv);
                let ca = self.addr(cv).unwrap();
                let eq = self.call("rt_identical", &[sa, ca]).unwrap();
                self.release(cv);
                let next = self.b.create_block();
                self.b.ins().brif(eq, body, &[], next, &[]);
                self.b.switch_to_block(next);
            }
            let after = self.b.current_block().unwrap();
            self.b.switch_to_block(body);
            let v = self.expr(&arm.body)?;
            let ta = self.temp_addr(t);
            self.store_into(ta, v);
            self.b.ins().jump(merge, &[]);
            self.b.switch_to_block(after);
        }
        match default {
            Some(d) => {
                let v = self.expr(d)?;
                let ta = self.temp_addr(t);
                self.store_into(ta, v);
                self.b.ins().jump(merge, &[]);
            }
            None => {
                self.call("rt_unhandled_match", &[self.cx, sa]);
                self.b.ins().jump(self.unwind, &[]);
            }
        }
        self.b.switch_to_block(merge);
        self.release(s);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    /// isset/??/empty operand: writes the value (if any) into temp `t` and
    /// returns found (i8).
    /// `isset($base[$k])` with the base at `base(self)`: rt_isset_index, or
    /// rt_quiet_index for strings and the rest. Returns the i8 answer.
    fn isset_index(&mut self, base: impl Fn(&mut Self) -> CValue, ka: CValue, t: usize) -> CValue {
        let ba = base(self);
        let r = self.call("rt_isset_index", &[self.cx, ba, ka]).unwrap();
        let (general, decided, done) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.append_block_param(done, types::I8);
        let other = self.b.ins().icmp_imm_u(IntCC::Equal, r, 3);
        self.b.ins().brif(other, general, &[], decided, &[]);
        self.b.switch_to_block(decided);
        let err = self.b.ins().icmp_imm_u(IntCC::Equal, r, 2);
        let ok = self.b.create_block();
        self.b.ins().brif(err, self.unwind, &[], ok, &[]);
        self.b.switch_to_block(ok);
        let found = self.b.ins().ireduce(types::I8, r);
        self.b.ins().jump(done, &[found.into()]);
        self.b.switch_to_block(general);
        let ba = base(self);
        let ta = self.temp_addr(t);
        let r = self.call("rt_quiet_index", &[self.cx, ba, ka, ta]).unwrap();
        let err = self.b.ins().icmp_imm_s(IntCC::Equal, r, 2);
        let ok = self.b.create_block();
        self.b.ins().brif(err, self.unwind, &[], ok, &[]);
        self.b.switch_to_block(ok);
        let present = self.b.ins().icmp_imm_s(IntCC::Equal, r, 1);
        let ta = self.temp_addr(t);
        let tg = self.load_tag(ta);
        let nn = self.b.ins().icmp_imm_u(IntCC::NotEqual, tg, tag::NULL as i64);
        let f = self.b.ins().band(present, nn);
        self.b.ins().jump(done, &[f.into()]);
        self.b.switch_to_block(done);
        self.b.block_params(done)[0]
    }

    fn quiet(&mut self, e: &'static Expr, t: usize) -> L<CValue> {
        self.quiet_mode(e, t, crate::expr::QUIET_READ)
    }

    /// `quiet`; `mode` (QUIET_*) for isset()'s and empty()'s own operand,
    /// where a magic property answers differently (see magic_isset).
    fn quiet_mode(&mut self, e: &'static Expr, t: usize, mode: u8) -> L<CValue> {
        match &e.kind {
            ExprKind::GlobalVar(n) => {
                let (np, nl) = self.global_name(n)?;
                let quiet = self.iconst(1);
                let ta = self.temp_addr(t);
                self.call_checked("rt_global_get", &[self.cx, np, nl, quiet, ta]);
                let ta = self.temp_addr(t);
                let tg = self.load_tag(ta);
                Ok(self.b.ins().icmp_imm_u(IntCC::NotEqual, tg, tag::NULL as i64))
            }
            ExprKind::Var(n) => {
                let i = *self.locals.index.get(n.as_str()).ok_or("unknown local")?;
                let la = self.local_addr(i);
                let tg = self.load_tag(la);
                let found = self.b.ins().icmp_imm_u(IntCC::NotEqual, tg, tag::NULL as i64);
                let ta = self.temp_addr(t);
                self.store_into(ta, Op::Local(i));
                Ok(found)
            }
            ExprKind::Index { base, index: Some(index) } if matches!(&base.kind, ExprKind::Var(n) if self.locals.index.contains_key(n.as_str())) => {
                // `$local[k]`: read in place (no copy of the array).
                let ExprKind::Var(n) = &base.kind else { unreachable!() };
                let i = self.locals.index[n.as_str()];
                let k = self.expr(index)?;
                let k = self.materialize(k);
                let ka = self.addr(k).unwrap();
                let ba = self.local_addr(i);
                if mode == crate::expr::QUIET_ISSET {
                    let found = self.isset_index(|l| l.local_addr(i), ka, t);
                    self.release(k);
                    return Ok(found);
                }
                let ta = self.temp_addr(t);
                let r = self.call("rt_quiet_index", &[self.cx, ba, ka, ta]).unwrap();
                self.release(k);
                let err = self.b.ins().icmp_imm_s(IntCC::Equal, r, 2);
                let ok = self.b.create_block();
                self.b.ins().brif(err, self.unwind, &[], ok, &[]);
                self.b.switch_to_block(ok);
                let present = self.b.ins().icmp_imm_s(IntCC::Equal, r, 1);
                let ta = self.temp_addr(t);
                let tg = self.load_tag(ta);
                let nn = self.b.ins().icmp_imm_u(IntCC::NotEqual, tg, tag::NULL as i64);
                Ok(self.b.ins().band(present, nn))
            }
            ExprKind::Index { base, index: Some(_) } | ExprKind::Prop { object: base, .. } => {
                let bt = self.alloc_temp();
                let bfound = self.quiet(base, bt)?;
                let merge = self.b.create_block();
                self.b.append_block_param(merge, types::I8);
                let go = self.b.create_block();
                let zero = self.b.ins().iconst(types::I8, 0);
                self.b.ins().brif(bfound, go, &[], merge, &[zero.into()]);
                self.b.switch_to_block(go);
                let ba = self.temp_addr(bt);
                let ta = self.temp_addr(t);
                let r = match &e.kind {
                    ExprKind::Index { index: Some(index), .. } if mode == crate::expr::QUIET_ISSET => {
                        let k = self.expr(index)?;
                        let k = self.materialize(k);
                        let ka = self.addr(k).unwrap();
                        let found = self.isset_index(|l| l.temp_addr(bt), ka, t);
                        self.release(k);
                        self.b.ins().jump(merge, &[found.into()]);
                        self.b.switch_to_block(merge);
                        let ba = self.temp_addr(bt);
                        self.clear(ba);
                        self.free.push(bt);
                        return Ok(self.b.block_params(merge)[0]);
                    }
                    ExprKind::Index { index: Some(index), .. } => {
                        let k = self.expr(index)?;
                        let k = self.materialize(k);
                        let ka = self.addr(k).unwrap();
                        let r = self.call("rt_quiet_index", &[self.cx, ba, ka, ta]).unwrap();
                        self.release(k);
                        r
                    }
                    ExprKind::Prop { name, .. } => {
                        let name = self.member(name)?;
                        let site = Box::new(super::rt::PropSite::new(name));
                        let sp = self.iconst(&*site as *const super::rt::PropSite as i64);
                        self.psites.push(site);
                        let m = self.iconst(mode as i64);
                        self.call("rt_quiet_prop_site", &[self.cx, ba, sp, m, ta]).unwrap()
                    }
                    _ => unreachable!(),
                };
                let err = self.b.ins().icmp_imm_s(IntCC::Equal, r, 2);
                let ok = self.b.create_block();
                self.b.ins().brif(err, self.unwind, &[], ok, &[]);
                self.b.switch_to_block(ok);
                // Found only if present and not null.
                let present = self.b.ins().icmp_imm_s(IntCC::Equal, r, 1);
                let ta = self.temp_addr(t);
                let tg = self.load_tag(ta);
                let nn = self.b.ins().icmp_imm_u(IntCC::NotEqual, tg, tag::NULL as i64);
                let f = self.b.ins().band(present, nn);
                self.b.ins().jump(merge, &[f.into()]);
                self.b.switch_to_block(merge);
                let ba = self.temp_addr(bt);
                self.clear(ba);
                self.free.push(bt);
                Ok(self.b.block_params(merge)[0])
            }
            _ => {
                let v = self.expr(e)?;
                let ta = self.temp_addr(t);
                self.store_into(ta, v);
                let ta = self.temp_addr(t);
                let tg = self.load_tag(ta);
                Ok(self.b.ins().icmp_imm_u(IntCC::NotEqual, tg, tag::NULL as i64))
            }
        }
    }

    // ---- assignment ----------------------------------------------------------------

    /// `$local[]` / `$local[k]` (one level on a local): the fast store path.
    fn simple_index_target(&self, target: &Expr) -> Option<u32> {
        match &target.kind {
            ExprKind::Index { base, .. } => self.local_of(base),
            _ => None,
        }
    }

    fn local_store(&mut self, target: &'static Expr, value: &'static Expr, want: bool) -> L<Op> {
        let ExprKind::Index { base, index } = &target.kind else { unreachable!() };
        let i = self.local_of(base).unwrap();
        let key = match index {
            Some(k) => {
                let k = self.expr(k)?;
                Some(self.materialize(k))
            }
            None => None,
        };
        let v = self.expr(value)?;
        let v = self.own(v);
        let v = self.materialize(v);
        let va = self.addr(v).unwrap();
        let ka = match key {
            Some(k) => self.addr(k).unwrap(),
            None => self.iconst(0),
        };
        self.mark_defined(i);
        let la = self.local_addr(i);
        let r = if want {
            self.helper_value("rt_local_store", |l| vec![l.cx, la, ka, va])?
        } else if key.is_some() {
            let (slow, merge) = (self.b.create_block(), self.b.create_block());
            self.b.set_cold_block(slow);
            self.elem_store_fast(la, ka, va, slow, merge);
            self.b.switch_to_block(slow);
            let la = self.local_addr(i);
            let z = self.iconst(0);
            self.call_checked("rt_local_store", &[self.cx, la, ka, va, z]);
            self.b.ins().jump(merge, &[]);
            self.b.switch_to_block(merge);
            Op::Int(self.iconst(0))
        } else {
            let z = self.iconst(0);
            self.call_checked("rt_local_store", &[self.cx, la, ka, va, z]);
            Op::Int(self.iconst(0))
        };
        self.release(v);
        if let Some(k) = key {
            self.release(k);
        }
        Ok(r)
    }

    fn assign(&mut self, target: &'static Expr, value: &'static Expr) -> L<Op> {
        if self.simple_index_target(target).is_some() {
            return self.local_store(target, value, true);
        }
        match &target.kind {
            ExprKind::Var(n) => {
                let i = self.locals.index[n.as_str()];
                let v = self.expr(value)?;
                self.assign_local(i, v);
                Ok(Op::Local(i))
            }
            ExprKind::Prop { object, name: MemberName::Ident(name), nullsafe: false } => self.prop_write(object, &name.name, value),
            ExprKind::GlobalVar(n) => {
                let (np, nl) = self.global_name(n)?;
                let v = self.expr(value)?;
                let v = self.own(v);
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                self.call_checked("rt_global_set", &[self.cx, np, nl, va, va]);
                Ok(v)
            }
            ExprKind::Array(items) => {
                // [$a, $b] = expr / ['k' => $a] = expr
                let v = self.expr(value)?;
                let v = self.own(v);
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                let mut pos = 0i64;
                let mut first = true;
                for it in items {
                    let Some(it) = it else {
                        pos += 1;
                        continue;
                    };
                    let key = match &it.key {
                        Some(k) => self.expr(k)?,
                        None => {
                            pos += 1;
                            Op::Int(self.iconst(pos - 1))
                        }
                    };
                    let key = self.materialize(key);
                    let ka = self.addr(key).unwrap();
                    let fl = self.iconst(first as i64);
                    first = false;
                    let r = self.helper_value("rt_list_get", |l| vec![l.cx, va, ka, fl])?;
                    self.release(key);
                    let ExprKind::Var(n) = &it.value.kind else { return Err("list target") };
                    let i = self.locals.index[n.as_str()];
                    self.assign_local(i, r);
                }
                Ok(v)
            }
            _ => self.assign_place(target, value, true),
        }
    }

    /// Assignment to an element or property path (`$a[k][j] = v`,
    /// `$o->p[k] = v`, ...); `want`: the assigned value is used.
    fn assign_place(&mut self, target: &'static Expr, value: &'static Expr, want: bool) -> L<Op> {
        let (root, name, keys) = self.place(target, false)?;
        let (np, nl) = name;
        let (kslot, kn, kids) = keys;
        let append = matches!(&target.kind, ExprKind::Index { index: None, .. });
        let v = self.expr(value)?;
        let v = self.own(v);
        let v = self.materialize(v);
        let va = self.addr(v).unwrap();
        let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
        let knv = self.iconst(kn as i64);
        let ap = self.iconst(append as i64);
        let ra = root.addr;
        // `$o->name[k] = v` / `$o->name[] = v`: cached slot.
        let r = match &target.kind {
            ExprKind::Index { base, .. } if matches!(&base.kind, ExprKind::Prop { name: MemberName::Ident(_), nullsafe: false, .. }) => {
                let ExprKind::Prop { name: MemberName::Ident(name), .. } = &base.kind else { unreachable!() };
                let site = Box::new(super::rt::PropSite::new(&name.name));
                let sp = &*site as *const super::rt::PropSite as i64;
                self.psites.push(site);
                match super::layout() {
                    Some(lay) if !want && !append && kn == 1 => {
                        // Cached slot checked inline; the element stored
                        // without the general helper when possible.
                        let (slow, merge) = (self.b.create_block(), self.b.create_block());
                        self.b.set_cold_block(slow);
                        let site_v = self.iconst(sp);
                        let props = self.prop_guard(lay, ra, site_v, super::rt::PROPSITE_ESHAPE, super::rt::PROPSITE_ESCOPE, slow);
                        let p = self.prop_slot(lay, props, site_v, super::rt::PROPSITE_ESLOT, slow);
                        let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
                        self.elem_store_fast(p, ka, va, slow, merge);
                        self.b.switch_to_block(slow);
                        let (sp, ka, z) = (self.iconst(sp), self.b.ins().stack_addr(types::I64, kslot, 0), self.iconst(0));
                        self.call_checked("rt_prop_elem_assign", &[self.cx, ra, sp, ka, ap, va, z]);
                        self.b.ins().jump(merge, &[]);
                        self.b.switch_to_block(merge);
                        Op::Int(self.iconst(0))
                    }
                    _ => {
                        let sp = self.iconst(sp);
                        self.helper_value("rt_prop_elem_assign", |l| vec![l.cx, ra, sp, ka, ap, va])?
                    }
                }
            }
            _ => self.helper_value("rt_place_assign", |l| vec![l.cx, ra, np, nl, ka, knv, ap, va])?,
        };
        self.release(v);
        self.release_ids(&kids);
        self.release_root(root);
        Ok(r)
    }

    /// `[k] = v` into the array value at `slot` (moving the value at `va`
    /// in): an int key overwriting a scalar of an unshared packed array
    /// inline, other int/string keys of an unshared array by
    /// rt_arr_set_fast; anything else branches to `slow` with nothing
    /// changed. Done: jumps to `merge`.
    fn elem_store_fast(&mut self, slot: CValue, ka: CValue, va: CValue, slow: Block, merge: Block) {
        let helper = self.b.create_block();
        if super::inline_rc() {
            let p = self.packed_elem(slot, ka, helper);
            let (c1, c2, ok) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
            let vt = self.load_tag(p);
            let scalar = self.b.ins().icmp_imm_u(IntCC::UnsignedLessThan, vt, tag::STR as i64);
            self.b.ins().brif(scalar, c1, &[], helper, &[]);
            self.b.switch_to_block(c1);
            let rc = self.b.ins().load(types::I64, flags(), slot, tag::PAYLOAD);
            let strong = self.b.ins().load(types::I64, flags(), rc, 0);
            let one = self.b.ins().icmp_imm_s(IntCC::Equal, strong, 1);
            self.b.ins().brif(one, c2, &[], helper, &[]);
            self.b.switch_to_block(c2);
            let weak = self.b.ins().load(types::I64, flags(), rc, 8);
            let one = self.b.ins().icmp_imm_s(IntCC::Equal, weak, 1);
            self.b.ins().brif(one, ok, &[], helper, &[]);
            self.b.switch_to_block(ok);
            self.copy16(p, va);
            self.store_tag(va, tag::NULL);
            self.b.ins().jump(merge, &[]);
        } else {
            self.b.ins().jump(helper, &[]);
        }
        self.b.switch_to_block(helper);
        let r = self.call("rt_arr_set_fast", &[slot, ka, va]).unwrap();
        self.b.ins().brif(r, slow, &[], merge, &[]);
    }

    fn class_site(&mut self) -> CValue {
        let site = Box::<super::rt::ClassSite>::default();
        let sp = &*site as *const super::rt::ClassSite as i64;
        self.csites.push(site);
        self.iconst(sp)
    }

    /// A class reference operand: the AST node for a name, or for `$x::`
    /// the interned reference of the class `$x` names (evaluated now).
    fn class_ref_ptr(&mut self, class: &'static ClassRef) -> L<CValue> {
        match class {
            ClassRef::Name(_) => Ok(self.iconst(class as *const ClassRef as i64)),
            ClassRef::Dynamic(e) => {
                let v = self.expr(e)?;
                let v = self.materialize(v);
                let va = self.addr(v).unwrap();
                let r = self.call("rt_dyn_class", &[self.cx, va]).unwrap();
                self.release(v);
                let bad = self.b.ins().icmp_imm_s(IntCC::Equal, r, 0);
                let cont = self.b.create_block();
                self.b.ins().brif(bad, self.unwind, &[], cont, &[]);
                self.b.switch_to_block(cont);
                Ok(r)
            }
            ClassRef::Anonymous(_) => Err("anonymous class"),
        }
    }

    /// Resolve a write target to (root pointer, (name ptr, len) or (0, 0),
    /// key array). The root is a local's address, or a temp holding the
    /// object for property targets.
    #[allow(clippy::type_complexity)]
    fn place(&mut self, target: &'static Expr, for_unset: bool) -> L<(Root, (CValue, CValue), (StackSlot, usize, Vec<usize>))> {
        let mut keys: Vec<&'static Expr> = vec![];
        let mut cur = target;
        let mut skip_last_append = false;
        loop {
            match &cur.kind {
                ExprKind::Index { base, index } => {
                    match index {
                        Some(i) => keys.push(i),
                        None if cur as *const Expr == target as *const Expr => skip_last_append = true,
                        None => return Err("[] in the middle of a path"),
                    }
                    cur = base;
                }
                _ => break,
            }
        }
        let _ = (skip_last_append, for_unset);
        keys.reverse();
        let (root, name) = match &cur.kind {
            ExprKind::Var(n) => {
                let i = self.locals.index[n.as_str()];
                if !for_unset {
                    self.mark_defined(i);
                }
                (Root { addr: self.local_addr(i), temp: None }, (self.iconst(0), self.iconst(0)))
            }
            ExprKind::Prop { object, name, nullsafe: false } => {
                let name = self.member(name)?;
                let o = self.expr(object)?;
                let o = self.own(o);
                let o = self.materialize(o);
                let oa = self.addr(o).unwrap();
                let np = self.name_parts(name);
                (Root { addr: oa, temp: Some(o) }, np)
            }
            ExprKind::StaticProp { class, name } => {
                let name = self.member(name)?;
                let cr = self.class_ref_ptr(class)?;
                let (np, nl) = self.name_parts(name);
                let nl = self.b.ins().bor_imm_s(nl, super::rt::STATIC_ROOT as i64);
                (Root { addr: cr, temp: None }, (np, nl))
            }
            // A global: a temp holding a reference to its slot, written
            // through like a by-reference local.
            ExprKind::GlobalVar(n) => {
                let (np, nl) = self.global_name(n)?;
                let t = self.alloc_temp();
                let ta = self.temp_addr(t);
                self.call("rt_global_ref", &[self.cx, np, nl, ta]);
                let ta = self.temp_addr(t);
                (Root { addr: ta, temp: Some(Op::Ptr(ta, Some(t))) }, (self.iconst(0), self.iconst(0)))
            }
            _ => return Err("write target"),
        };
        let (kslot, kids) = self.alloc_array(keys.len());
        for (i, k) in keys.iter().enumerate() {
            let a = self.b.ins().stack_addr(types::I64, kslot, (i * 16) as i32);
            self.store_tag(a, tag::NULL);
            let v = self.expr(k)?;
            let a = self.b.ins().stack_addr(types::I64, kslot, (i * 16) as i32);
            self.store_into(a, v);
        }
        Ok((root, name, (kslot, keys.len(), kids)))
    }

    fn release_ids(&mut self, ids: &[usize]) {
        for &t in ids {
            let a = self.temp_addr(t);
            self.clear(a);
            self.free.push(t);
        }
    }

    fn release_root(&mut self, root: Root) {
        if let Some(o) = root.temp {
            self.release(o);
        }
    }

    fn assign_op(&mut self, op: BinOp, target: &'static Expr, value: &'static Expr) -> L<Op> {
        if let ExprKind::Var(n) = &target.kind {
            let i = self.locals.index[n.as_str()];
            self.mark_defined(i);
            match op {
                BinOp::Concat => {
                    let v = self.expr(value)?;
                    let v = self.materialize(v);
                    let va = self.addr(v).unwrap();
                    let la = self.local_addr(i);
                    self.call_checked("rt_concat_assign", &[self.cx, la, va]);
                    self.release(v);
                }
                BinOp::Coalesce => {
                    let la = self.local_addr(i);
                    let tg = self.load_tag(la);
                    let is_null = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::NULL as i64);
                    let (set, done) = (self.b.create_block(), self.b.create_block());
                    self.b.ins().brif(is_null, set, &[], done, &[]);
                    self.b.switch_to_block(set);
                    let v = self.expr(value)?;
                    self.assign_local(i, v);
                    self.b.ins().jump(done, &[]);
                    self.b.switch_to_block(done);
                }
                _ => {
                    let v = self.expr(value)?;
                    let r = self.binary(op, Op::Local(i), v)?;
                    self.assign_local(i, r);
                }
            }
            return Ok(Op::Local(i));
        }
        // `$local->name op= v` (not ??=): read and write through the
        // property caches, the arithmetic inline. PHP's order: the object,
        // then the value, then the property is read.
        if let ExprKind::Prop { object, name: MemberName::Ident(name), nullsafe: false } = &target.kind {
            if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div)
                && matches!(&object.kind, ExprKind::Var(n) if self.locals.index.contains_key(n.as_str()))
                && super::layout().is_some()
            {
                return self.prop_arith(op, target, object, &name.name, value);
            }
            if !matches!(op, BinOp::Coalesce) && matches!(&object.kind, ExprKind::Var(n) if self.locals.index.contains_key(n.as_str())) {
                let o = self.expr(object)?;
                let o = self.own(o);
                let o = self.materialize(o);
                let v = self.expr(value)?;
                let v = self.own(v);
                let v = self.materialize(v);
                let t = self.alloc_temp();
                let (fast, general, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                let oa = self.addr(o).unwrap();
                let tg = self.load_tag(oa);
                let is_obj = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::OBJECT as i64);
                self.b.ins().brif(is_obj, fast, &[], general, &[]);
                self.b.switch_to_block(fast);
                let site = Box::new(super::rt::PropSite::new(&name.name));
                let sp = &*site as *const super::rt::PropSite as i64;
                self.psites.push(site);
                let oa = self.addr(o).unwrap();
                let cur = self.prop_read(oa, sp)?;
                let va = self.addr(v).unwrap();
                let r = self.binary(op, cur, Op::Ptr(va, None))?;
                let oa = self.addr(o).unwrap();
                let r = self.prop_store(oa, &name.name, r)?;
                let ta = self.temp_addr(t);
                self.store_into(ta, r);
                self.b.ins().jump(merge, &[]);
                // Not an object: the general path (its errors and warnings).
                self.b.switch_to_block(general);
                let va = self.addr(v).unwrap();
                let r = self.place_update_ra(target, binop_code(op) as i64, va)?;
                let ta = self.temp_addr(t);
                self.store_into(ta, r);
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(merge);
                self.release(v);
                self.release(o);
                let ta = self.temp_addr(t);
                return Ok(Op::Ptr(ta, Some(t)));
            }
        }
        self.place_update(target, binop_code(op) as i64, Some(value))
    }

    /// `$local->name op= v` (`+ - * /`) on an int or float property: the
    /// slot found through the write cache and updated in place, an int
    /// staying an int (no overflow) and a float a float; anything else
    /// takes the general path, which fills the cache.
    fn prop_arith(&mut self, op: BinOp, target: &'static Expr, object: &'static Expr, name: &'static str, value: &'static Expr) -> L<Op> {
        let lay = super::layout().unwrap();
        let site = Box::new(super::rt::PropSite::new(name));
        let sp = &*site as *const super::rt::PropSite as i64;
        self.psites.push(site);
        // (`$this` never changes; another local is read first, as PHP
        // does, unless the value can't change it.)
        let o = self.expr(object)?;
        let keep = matches!(&object.kind, ExprKind::Var(n) if n == "this") || pure_operand(value);
        let o = if keep { o } else { self.own(o) };
        let o = self.materialize(o);
        let v = self.expr(value)?;
        let v = self.own(v);
        let t = self.alloc_temp();
        let (general, merge) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(general);
        let oa = self.addr(o).unwrap();
        let site_v = self.iconst(sp);
        let props = self.prop_guard(lay, oa, site_v, super::rt::PROPSITE_WSHAPE, super::rt::PROPSITE_WSCOPE, general);
        let p = self.prop_slot(lay, props, site_v, super::rt::PROPSITE_WSLOT, general);
        let ct = self.load_tag(p);
        // The value's tag (known for a register operand).
        let vt = match v {
            Op::Int(_) => self.b.ins().iconst(types::I8, tag::INT as i64),
            Op::Float(_) => self.b.ins().iconst(types::I8, tag::FLOAT as i64),
            Op::Bool(_) => self.b.ins().iconst(types::I8, tag::BOOL as i64),
            _ => {
                let va = self.addr(v).unwrap();
                self.load_tag(va)
            }
        };
        let payload = |l: &mut Self, ty| match v {
            Op::Int(x) | Op::Float(x) => x,
            _ => {
                let va = l.addr(v).unwrap();
                l.b.ins().load(ty, flags(), va, tag::PAYLOAD)
            }
        };
        let (int_b, flt_chk, flt_b) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        let ct_int = self.b.ins().icmp_imm_u(IntCC::Equal, ct, tag::INT as i64);
        let vt_int = self.b.ins().icmp_imm_u(IntCC::Equal, vt, tag::INT as i64);
        let both = self.b.ins().band(ct_int, vt_int);
        if op == BinOp::Div {
            self.b.ins().jump(flt_chk, &[]);
        } else {
            self.b.ins().brif(both, int_b, &[], flt_chk, &[]);
        }
        // int op int: an int unless it overflows.
        self.b.switch_to_block(int_b);
        if op != BinOp::Div {
            let x = self.b.ins().load(types::I64, flags(), p, tag::PAYLOAD);
            let y = if matches!(v, Op::Float(_)) { self.iconst(0) } else { payload(self, types::I64) };
            let (r, of) = match op {
                BinOp::Add => self.b.ins().sadd_overflow(x, y),
                BinOp::Sub => self.b.ins().ssub_overflow(x, y),
                _ => self.b.ins().smul_overflow(x, y),
            };
            let ok = self.b.create_block();
            self.b.ins().brif(of, general, &[], ok, &[]);
            self.b.switch_to_block(ok);
            self.b.ins().store(flags(), r, p, tag::PAYLOAD);
            let ta = self.temp_addr(t);
            self.store_into(ta, Op::Int(r));
            self.b.ins().jump(merge, &[]);
        } else {
            self.b.ins().jump(general, &[]);
        }
        // float op (float or int): a float.
        self.b.switch_to_block(flt_chk);
        let ct_flt = self.b.ins().icmp_imm_u(IntCC::Equal, ct, tag::FLOAT as i64);
        let vt_flt = self.b.ins().icmp_imm_u(IntCC::Equal, vt, tag::FLOAT as i64);
        let num = self.b.ins().bor(vt_flt, vt_int);
        let go = self.b.ins().band(ct_flt, num);
        self.b.ins().brif(go, flt_b, &[], general, &[]);
        self.b.switch_to_block(flt_b);
        let x = self.b.ins().load(types::F64, flags(), p, tag::PAYLOAD);
        let y = match v {
            Op::Float(y) => y,
            Op::Int(n) => self.b.ins().fcvt_from_sint(types::F64, n),
            _ => {
                let (fb, ib, j) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.append_block_param(j, types::F64);
                self.b.ins().brif(vt_flt, fb, &[], ib, &[]);
                self.b.switch_to_block(fb);
                let y = payload(self, types::F64);
                self.b.ins().jump(j, &[y.into()]);
                self.b.switch_to_block(ib);
                let n = payload(self, types::I64);
                let y = self.b.ins().fcvt_from_sint(types::F64, n);
                self.b.ins().jump(j, &[y.into()]);
                self.b.switch_to_block(j);
                self.b.block_params(j)[0]
            }
        };
        let r = match op {
            BinOp::Add => self.b.ins().fadd(x, y),
            BinOp::Sub => self.b.ins().fsub(x, y),
            BinOp::Mul => self.b.ins().fmul(x, y),
            _ => {
                let zero = self.b.ins().f64const(0.0);
                let z = self.b.ins().fcmp(FloatCC::Equal, y, zero);
                let ok = self.b.create_block();
                self.b.ins().brif(z, general, &[], ok, &[]);
                self.b.switch_to_block(ok);
                self.b.ins().fdiv(x, y)
            }
        };
        self.b.ins().store(flags(), r, p, tag::PAYLOAD);
        let ta = self.temp_addr(t);
        self.store_into(ta, Op::Float(r));
        self.b.ins().jump(merge, &[]);
        // The general path (its errors, warnings and coercions), then the
        // cache for next time.
        self.b.switch_to_block(general);
        let vm = self.materialize(v);
        let va = self.addr(vm).unwrap();
        let r = self.place_update_ra(target, binop_code(op) as i64, va)?;
        let ta = self.temp_addr(t);
        self.store_into(ta, r);
        let oa = self.addr(o).unwrap();
        let site_v = self.iconst(sp);
        self.call("rt_arith_fill", &[self.cx, oa, site_v]);
        if !matches!(v, Op::Ptr(..)) {
            self.release(vm);
        }
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        self.release(v);
        self.release(o);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    fn place_update(&mut self, target: &'static Expr, mode: i64, value: Option<&'static Expr>) -> L<Op> {
        let (root, name, keys) = self.place(target, false)?;
        let (np, nl) = name;
        let (kslot, kn, kids) = keys;
        if mode == binop_code(BinOp::Coalesce) as i64 {
            if let Some(value) = value {
                return self.place_coalesce(root, (np, nl), (kslot, kn, kids), value);
            }
        }
        let rhs = match value {
            Some(v) => {
                let v = self.expr(v)?;
                Some(self.materialize(v))
            }
            None => None,
        };
        let ra = match rhs {
            Some(r) => self.addr(r).unwrap(),
            None => self.iconst(0),
        };
        let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
        let knv = self.iconst(kn as i64);
        let m = self.iconst(mode);
        let rootp = root.addr;
        let r = self.helper_value("rt_place_update", |l| vec![l.cx, rootp, np, nl, ka, knv, m, ra])?;
        if let Some(rh) = rhs {
            self.release(rh);
        }
        self.release_ids(&kids);
        self.release_root(root);
        Ok(r)
    }

    /// `place_update` with the right-hand side already evaluated (at `ra`,
    /// borrowed).
    fn place_update_ra(&mut self, target: &'static Expr, mode: i64, ra: CValue) -> L<Op> {
        let (root, name, keys) = self.place(target, false)?;
        let (np, nl) = name;
        let (kslot, kn, kids) = keys;
        let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
        let knv = self.iconst(kn as i64);
        let m = self.iconst(mode);
        let rootp = root.addr;
        let r = self.helper_value("rt_place_update", |l| vec![l.cx, rootp, np, nl, ka, knv, m, ra])?;
        self.release_ids(&kids);
        self.release_root(root);
        Ok(r)
    }

    /// `place ??= value`: the value is only evaluated (and assigned) when
    /// the place is unset or null.
    fn place_coalesce(&mut self, root: Root, name: (CValue, CValue), keys: (StackSlot, usize, Vec<usize>), value: &'static Expr) -> L<Op> {
        let (np, nl) = name;
        let (kslot, kn, kids) = keys;
        let mode = binop_code(BinOp::Coalesce) as i64;
        let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
        let knv = self.iconst(kn as i64);
        let m = self.iconst(mode);
        let none = self.iconst(0);
        let rootp = root.addr;
        let t = self.alloc_temp();
        let ta = self.temp_addr(t);
        self.call_checked("rt_place_update", &[self.cx, rootp, np, nl, ka, knv, m, none, ta]);
        let tg = self.load_tag(ta);
        let is_null = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::NULL as i64);
        let (set, done) = (self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_null, set, &[], done, &[]);
        self.b.switch_to_block(set);
        let v = self.expr(value)?;
        let v = self.materialize(v);
        let va = self.addr(v).unwrap();
        let ka = self.b.ins().stack_addr(types::I64, kslot, 0);
        let knv = self.iconst(kn as i64);
        let m = self.iconst(mode);
        let ta = self.temp_addr(t);
        self.clear(ta);
        let ta = self.temp_addr(t);
        self.call_checked("rt_place_update", &[self.cx, rootp, np, nl, ka, knv, m, va, ta]);
        self.release(v);
        self.b.ins().jump(done, &[]);
        self.b.switch_to_block(done);
        self.release_ids(&kids);
        self.release_root(root);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    /// `$local->name++` (and --, prefix or postfix): an int in a slot the
    /// site's own cache knows is updated in place; anything else (a miss,
    /// another type, overflow) takes the general path, which then fills
    /// the cache for next time.
    fn incdec_prop(&mut self, expr: &'static Expr, object: &'static Expr, name: &'static str, inc: bool, prefix: bool) -> L<Op> {
        let lay = super::layout().unwrap();
        let site = Box::new(super::rt::PropSite::new(name));
        let sp = &*site as *const super::rt::PropSite as i64;
        self.psites.push(site);
        let o = self.expr(object)?;
        // (`$this` never changes: read in place.)
        let o = if matches!(&object.kind, ExprKind::Var(n) if n == "this") { o } else { self.own(o) };
        let o = self.materialize(o);
        let t = self.alloc_temp();
        let (general, merge) = (self.b.create_block(), self.b.create_block());
        self.b.set_cold_block(general);
        let oa = self.addr(o).unwrap();
        let site_v = self.iconst(sp);
        let props = self.prop_guard(lay, oa, site_v, super::rt::PROPSITE_WSHAPE, super::rt::PROPSITE_WSCOPE, general);
        let p = self.prop_slot(lay, props, site_v, super::rt::PROPSITE_WSLOT, general);
        let tg = self.load_tag(p);
        let is_int = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::INT as i64);
        let go = self.b.create_block();
        self.b.ins().brif(is_int, go, &[], general, &[]);
        self.b.switch_to_block(go);
        let old = self.b.ins().load(types::I64, flags(), p, tag::PAYLOAD);
        let one = self.iconst(1);
        let (new, of) = if inc { self.b.ins().sadd_overflow(old, one) } else { self.b.ins().ssub_overflow(old, one) };
        let ok = self.b.create_block();
        self.b.ins().brif(of, general, &[], ok, &[]);
        self.b.switch_to_block(ok);
        self.b.ins().store(flags(), new, p, tag::PAYLOAD);
        let ta = self.temp_addr(t);
        self.store_into(ta, Op::Int(if prefix { new } else { old }));
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(general);
        let mode = (if inc { 100 } else { 101 }) | if prefix { 256 } else { 0 };
        let r = self.place_update(expr, mode, None)?;
        let ta = self.temp_addr(t);
        self.store_into(ta, r);
        let oa = self.addr(o).unwrap();
        let site_v = self.iconst(sp);
        self.call("rt_incdec_fill", &[self.cx, oa, site_v]);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        self.release(o);
        let ta = self.temp_addr(t);
        Ok(Op::Ptr(ta, Some(t)))
    }

    fn incdec_local(&mut self, expr: &'static Expr, inc: bool, prefix: bool, want: bool) -> L<Op> {
        let ExprKind::Var(n) = &expr.kind else { unreachable!() };
        let i = self.locals.index[n.as_str()];
        self.mark_defined(i);
        let la = self.local_addr(i);
        let tg = self.load_tag(la);
        let is_int = self.b.ins().icmp_imm_u(IntCC::Equal, tg, tag::INT as i64);
        let t = if want { Some(self.alloc_temp()) } else { None };
        let (fast, slow, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
        self.b.ins().brif(is_int, fast, &[], slow, &[]);
        self.b.switch_to_block(fast);
        let old = self.b.ins().load(types::I64, flags(), la, tag::PAYLOAD);
        let one = self.iconst(1);
        let (new, of) = if inc { self.b.ins().sadd_overflow(old, one) } else { self.b.ins().ssub_overflow(old, one) };
        let ok = self.b.create_block();
        self.b.ins().brif(of, slow, &[], ok, &[]);
        self.b.switch_to_block(ok);
        self.b.ins().store(flags(), new, la, tag::PAYLOAD);
        if let Some(t) = t {
            let ta = self.temp_addr(t);
            self.store_into(ta, Op::Int(if prefix { new } else { old }));
        }
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(slow);
        let flags_v = self.iconst((inc as i64) | ((prefix as i64) << 1));
        let dst = match t {
            Some(t) => self.temp_addr(t),
            None => self.iconst(0),
        };
        let la = self.local_addr(i);
        self.call_checked("rt_incdec", &[self.cx, la, flags_v, dst]);
        self.b.ins().jump(merge, &[]);
        self.b.switch_to_block(merge);
        Ok(match t {
            Some(t) => {
                let ta = self.temp_addr(t);
                Op::Ptr(ta, Some(t))
            }
            None => Op::Int(self.iconst(0)),
        })
    }

    // ---- binary operators -------------------------------------------------------------

    /// The string made of `ops` (materialized values) in one allocation.
    fn join_strings(&mut self, ops: Vec<Op>) -> L<Op> {
        let pslot = self.b.create_sized_stack_slot(StackSlotData::new(StackSlotKind::ExplicitSlot, (ops.len().max(1) * 8) as u32, 3));
        for (i, op) in ops.iter().enumerate() {
            let a = self.addr(*op).unwrap();
            self.b.ins().stack_store(types::I64, a, pslot, (i * 8) as i32);
        }
        let pa = self.b.ins().stack_addr(types::I64, pslot, 0);
        let n = self.iconst(ops.len() as i64);
        let r = self.helper_value("rt_interp", |l| vec![l.cx, pa, n])?;
        for op in ops {
            self.release(op);
        }
        Ok(r)
    }

    fn binary(&mut self, op: BinOp, a: Op, b: Op) -> L<Op> {
        use BinOp::*;
        match op {
            Lt | LtEq | Gt | GtEq | Eq | NotEq | Identical | NotIdentical => {
                let (ai, ak) = self.int_check(a);
                let (bi, bk) = self.int_check(b);
                let both = self.b.ins().band(ai, bi);
                let (fast, slow, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.append_block_param(merge, types::I8);
                self.b.ins().brif(both, fast, &[], slow, &[]);
                self.b.switch_to_block(fast);
                let (x, y) = (self.int_payload(a, ak), self.int_payload(b, bk));
                let cc = match op {
                    Lt => IntCC::SignedLessThan,
                    LtEq => IntCC::SignedLessThanOrEqual,
                    Gt => IntCC::SignedGreaterThan,
                    GtEq => IntCC::SignedGreaterThanOrEqual,
                    Eq | Identical => IntCC::Equal,
                    _ => IntCC::NotEqual,
                };
                let r = self.b.ins().icmp(cc, x, y);
                self.b.ins().jump(merge, &[r.into()]);
                self.b.switch_to_block(slow);
                let am = self.materialize(a);
                let bm = self.materialize(b);
                let (aa, ba) = (self.addr(am).unwrap(), self.addr(bm).unwrap());
                let r = if matches!(op, Identical | NotIdentical) {
                    // (Never fails or runs PHP code: no interpreter needed.)
                    let r = self.call("rt_identical", &[aa, ba]).unwrap();
                    let r = self.b.ins().ireduce(types::I8, r);
                    if op == NotIdentical { self.b.ins().bxor_imm_u(r, 1) } else { r }
                } else {
                    let code = self.iconst(binop_code(op) as i64);
                    let r = self.call("rt_compare", &[self.cx, code, aa, ba]).unwrap();
                    self.b.ins().ireduce(types::I8, r)
                };
                // Release only temps created by materializing scalars here;
                // owned operands are released after the merge.
                if matches!(a, Op::Int(_) | Op::Float(_) | Op::Bool(_)) {
                    self.release(am);
                }
                if matches!(b, Op::Int(_) | Op::Float(_) | Op::Bool(_)) {
                    self.release(bm);
                }
                self.b.ins().jump(merge, &[r.into()]);
                self.b.switch_to_block(merge);
                // Operands materialized only on the slow path; release
                // owned operand temps (no-op if they were scalars).
                self.release(a);
                self.release(b);
                Ok(Op::Bool(self.b.block_params(merge)[0]))
            }
            Add | Sub | Mul | Mod | BitAnd | BitOr | BitXor | Shl | Shr | Spaceship => {
                let (ai, ak) = self.int_check(a);
                let (bi, bk) = self.int_check(b);
                let both = self.b.ins().band(ai, bi);
                let t = self.alloc_temp();
                let (fast, slow, merge) = (self.b.create_block(), self.b.create_block(), self.b.create_block());
                self.b.ins().brif(both, fast, &[], slow, &[]);
                self.b.switch_to_block(fast);
                let (x, y) = (self.int_payload(a, ak), self.int_payload(b, bk));
                let (r, of) = match op {
                    Add => self.b.ins().sadd_overflow(x, y),
                    Sub => self.b.ins().ssub_overflow(x, y),
                    Mul => self.b.ins().smul_overflow(x, y),
                    BitAnd => (self.b.ins().band(x, y), self.b.ins().iconst(types::I8, 0)),
                    BitOr => (self.b.ins().bor(x, y), self.b.ins().iconst(types::I8, 0)),
                    BitXor => (self.b.ins().bxor(x, y), self.b.ins().iconst(types::I8, 0)),
                    Spaceship => {
                        let gt = self.b.ins().icmp(IntCC::SignedGreaterThan, x, y);
                        let lt = self.b.ins().icmp(IntCC::SignedLessThan, x, y);
                        let (gt, lt) = (self.b.ins().uextend(types::I64, gt), self.b.ins().uextend(types::I64, lt));
                        (self.b.ins().isub(gt, lt), self.b.ins().iconst(types::I8, 0))
                    }
                    // Counts outside 0..64 (negative: an error) take the slow path.
                    Shl | Shr => {
                        let bad = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThanOrEqual, y, 64);
                        let r = if matches!(op, Shl) { self.b.ins().ishl(x, y) } else { self.b.ins().sshr(x, y) };
                        (r, bad)
                    }
                    _ => {
                        // `%`: divisors 0 (error) and -1 (INT_MIN edge) take the slow path.
                        let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                        let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                        let bad = self.b.ins().bor(z, m1);
                        let one = self.iconst(1);
                        let safe = self.b.ins().select(bad, one, y);
                        (self.b.ins().srem(x, safe), bad)
                    }
                };
                let ok = self.b.create_block();
                self.b.ins().brif(of, slow, &[], ok, &[]);
                self.b.switch_to_block(ok);
                let ta = self.temp_addr(t);
                self.store_into(ta, Op::Int(r));
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(slow);
                if matches!(op, Add | Sub | Mul) {
                    // Ints and floats (int overflow lands here too, which
                    // PHP also computes as float): in registers.
                    let generic = self.b.create_block();
                    let (x, y) = self.float_operands(a, b, generic);
                    let r = match op {
                        Add => self.b.ins().fadd(x, y),
                        Sub => self.b.ins().fsub(x, y),
                        _ => self.b.ins().fmul(x, y),
                    };
                    let ta = self.temp_addr(t);
                    self.store_into(ta, Op::Float(r));
                    self.b.ins().jump(merge, &[]);
                    self.b.switch_to_block(generic);
                }
                let am = self.materialize(a);
                let bm = self.materialize(b);
                let (aa, ba) = (self.addr(am).unwrap(), self.addr(bm).unwrap());
                let code = self.iconst(binop_code(op) as i64);
                let ta = self.temp_addr(t);
                self.call_checked("rt_binop", &[self.cx, code, aa, ba, ta]);
                if matches!(a, Op::Int(_) | Op::Float(_) | Op::Bool(_)) {
                    self.release(am);
                }
                if matches!(b, Op::Int(_) | Op::Float(_) | Op::Bool(_)) {
                    self.release(bm);
                }
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(merge);
                self.release(a);
                self.release(b);
                let ta = self.temp_addr(t);
                Ok(Op::Ptr(ta, Some(t)))
            }
            Div => {
                // A float on either side and a non-zero divisor: divided in
                // registers (int / int keeps PHP's int-or-float result rules
                // in the helper).
                let t = self.alloc_temp();
                let (slow, merge) = (self.b.create_block(), self.b.create_block());
                // int / int: an int when exact, else the float quotient;
                // zero and PHP_INT_MIN / -1 take the helper.
                let (ai, ak) = self.int_check(a);
                let (bi, bk) = self.int_check(b);
                let both = self.b.ins().band(ai, bi);
                let (ints, not_ints) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(both, ints, &[], not_ints, &[]);
                self.b.switch_to_block(ints);
                let (x, y) = (self.int_payload(a, ak), self.int_payload(b, bk));
                let z = self.b.ins().icmp_imm_s(IntCC::Equal, y, 0);
                let m1 = self.b.ins().icmp_imm_s(IntCC::Equal, y, -1);
                let min = self.b.ins().icmp_imm_s(IntCC::Equal, x, i64::MIN);
                let edge = self.b.ins().band(m1, min);
                let bad = self.b.ins().bor(z, edge);
                let go = self.b.create_block();
                self.b.ins().brif(bad, slow, &[], go, &[]);
                self.b.switch_to_block(go);
                let rem = self.b.ins().srem(x, y);
                let exact = self.b.ins().icmp_imm_s(IntCC::Equal, rem, 0);
                let (q_int, q_flt) = (self.b.create_block(), self.b.create_block());
                self.b.ins().brif(exact, q_int, &[], q_flt, &[]);
                self.b.switch_to_block(q_int);
                let q = self.b.ins().sdiv(x, y);
                let ta = self.temp_addr(t);
                self.store_into(ta, Op::Int(q));
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(q_flt);
                let (fx, fy) = (self.b.ins().fcvt_from_sint(types::F64, x), self.b.ins().fcvt_from_sint(types::F64, y));
                let q = self.b.ins().fdiv(fx, fy);
                let ta = self.temp_addr(t);
                self.store_into(ta, Op::Float(q));
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(not_ints);
                let (af, bf) = (self.float_tag(a), self.float_tag(b));
                let any = self.b.ins().bor(af, bf);
                let try_f = self.b.create_block();
                self.b.ins().brif(any, try_f, &[], slow, &[]);
                self.b.switch_to_block(try_f);
                let (x, y) = self.float_operands(a, b, slow);
                let z = self.b.ins().f64const(0.0);
                let nz = self.b.ins().fcmp(FloatCC::NotEqual, y, z);
                let ok = self.b.create_block();
                self.b.ins().brif(nz, ok, &[], slow, &[]);
                self.b.switch_to_block(ok);
                let r = self.b.ins().fdiv(x, y);
                let ta = self.temp_addr(t);
                self.store_into(ta, Op::Float(r));
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(slow);
                let am = self.materialize(a);
                let bm = self.materialize(b);
                let (aa, ba) = (self.addr(am).unwrap(), self.addr(bm).unwrap());
                let code = self.iconst(binop_code(op) as i64);
                let ta = self.temp_addr(t);
                self.call_checked("rt_binop", &[self.cx, code, aa, ba, ta]);
                if matches!(a, Op::Int(_) | Op::Float(_) | Op::Bool(_)) {
                    self.release(am);
                }
                if matches!(b, Op::Int(_) | Op::Float(_) | Op::Bool(_)) {
                    self.release(bm);
                }
                self.b.ins().jump(merge, &[]);
                self.b.switch_to_block(merge);
                self.release(a);
                self.release(b);
                let ta = self.temp_addr(t);
                Ok(Op::Ptr(ta, Some(t)))
            }
            _ => {
                let am = self.materialize(a);
                let bm = self.materialize(b);
                let (aa, ba) = (self.addr(am).unwrap(), self.addr(bm).unwrap());
                let code = self.iconst(binop_code(op) as i64);
                let r = self.helper_value("rt_binop", |l| vec![l.cx, code, aa, ba])?;
                self.release(am);
                self.release(bm);
                Ok(r)
            }
        }
    }

    /// i8: whether `op` holds a float.
    fn float_tag(&mut self, op: Op) -> CValue {
        match op {
            Op::Float(_) => self.b.ins().iconst(types::I8, 1),
            Op::Int(_) | Op::Bool(_) => self.b.ins().iconst(types::I8, 0),
            _ => {
                let a = self.addr(op).unwrap();
                let t = self.load_tag(a);
                self.b.ins().icmp_imm_u(IntCC::Equal, t, tag::FLOAT as i64)
            }
        }
    }

    /// Both operands as f64 when each is an int or a float; branches to
    /// `other` otherwise (bools, strings, null, arrays...).
    fn float_operands(&mut self, a: Op, b: Op, other: Block) -> (CValue, CValue) {
        let x = self.as_f64(a, other);
        let y = self.as_f64(b, other);
        (x, y)
    }

    fn as_f64(&mut self, op: Op, other: Block) -> CValue {
        match op {
            Op::Float(v) => v,
            Op::Int(v) => self.b.ins().fcvt_from_sint(types::F64, v),
            Op::Bool(_) => {
                self.b.ins().jump(other, &[]);
                let dead = self.b.create_block();
                self.b.switch_to_block(dead);
                self.b.ins().f64const(0.0)
            }
            _ => {
                let a = self.addr(op).unwrap();
                let t = self.load_tag(a);
                let is_f = self.b.ins().icmp_imm_u(IntCC::Equal, t, tag::FLOAT as i64);
                let is_i = self.b.ins().icmp_imm_u(IntCC::Equal, t, tag::INT as i64);
                let num = self.b.ins().bor(is_f, is_i);
                let go = self.b.create_block();
                self.b.ins().brif(num, go, &[], other, &[]);
                self.b.switch_to_block(go);
                let raw = self.b.ins().load(types::I64, flags(), a, tag::PAYLOAD);
                let fv = self.b.ins().load(types::F64, flags(), a, tag::PAYLOAD);
                let iv = self.b.ins().fcvt_from_sint(types::F64, raw);
                self.b.ins().select(is_f, fv, iv)
            }
        }
    }
}

/// Names an arrow fn captures, in the interpreter's order (see `arrow_captures`).
fn arrow_names(a: &'static ast::ArrowFn) -> Vec<&'static str> {
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
    names
}

#[derive(Clone, Copy)]
enum Intrinsic {
    /// `is_*`: true when the argument's tag is one of these.
    IsTag(&'static [u8]),
    Strlen,
    Count,
}

/// `call_user_func($cb, ...)` with plain arguments, by a name that can
/// only mean the built-in (the helper checks no namespaced function
/// shadows it).
/// The key layout of an array literal whose keys are all constant (or
/// absent) and distinct, and whose values are plain (no spread, no `&`):
/// an array with those keys in order and null values. None otherwise, or
/// for a literal with fewer than two items.
fn literal_template(items: &[Option<ast::ArrayItem>]) -> Option<Value> {
    if items.len() < 2 || items.len() > 64 {
        return None;
    }
    let mut a = crate::value::PhpArray::new();
    for it in items {
        let it = it.as_ref()?;
        if it.spread || it.by_ref {
            return None;
        }
        match it.key.as_ref().map(|k| &k.kind) {
            None => {
                if a.next_index() == i64::MAX && a.len() > 0 {
                    return None;
                }
                a.push(Value::Null);
            }
            Some(ExprKind::Int(n)) => a.insert(crate::value::Key::Int(*n), Value::Null),
            Some(ExprKind::Str(s)) => a.insert(crate::value::Key::from_bytes(s.as_bytes()), Value::Null),
            Some(ExprKind::Bytes(b)) => a.insert(crate::value::Key::from_bytes(b), Value::Null),
            Some(_) => return None,
        }
    }
    (a.len() == items.len()).then(|| Value::array(a))
}

fn is_call_user_func(callee: &Expr, args: &[ast::Arg]) -> bool {
    let ExprKind::Name(n) = &callee.kind else { return false };
    let name = n.strip_prefix('\\').unwrap_or(n);
    name.eq_ignore_ascii_case("call_user_func") && !args.is_empty() && args.iter().all(|a| !a.spread && a.name.is_none())
}

fn is_call_user_func_array(callee: &Expr, args: &[ast::Arg]) -> bool {
    let ExprKind::Name(n) = &callee.kind else { return false };
    let name = n.strip_prefix('\\').unwrap_or(n);
    name.eq_ignore_ascii_case("call_user_func_array") && args.len() == 2 && args.iter().all(|a| !a.spread && a.name.is_none())
}

/// Built-ins compiled inline: one plain positional argument, called by a
/// name that can only mean the built-in (or `\name`).
fn intrinsic_kind(callee: &Expr, args: &[ast::Arg]) -> Option<Intrinsic> {
    let ExprKind::Name(n) = &callee.kind else { return None };
    if args.len() != 1 || args[0].spread || args[0].name.is_some() {
        return None;
    }
    let name = n.strip_prefix('\\').unwrap_or(n);
    if name.contains('\\') {
        return None;
    }
    Some(match name.to_ascii_lowercase().as_str() {
        "strlen" => Intrinsic::Strlen,
        "count" | "sizeof" => Intrinsic::Count,
        "is_array" => Intrinsic::IsTag(&[tag::ARRAY]),
        "is_string" => Intrinsic::IsTag(&[tag::STR]),
        "is_int" | "is_integer" | "is_long" => Intrinsic::IsTag(&[tag::INT]),
        "is_float" | "is_double" => Intrinsic::IsTag(&[tag::FLOAT]),
        "is_bool" => Intrinsic::IsTag(&[tag::BOOL]),
        "is_null" => Intrinsic::IsTag(&[tag::NULL]),
        "is_object" => Intrinsic::IsTag(&[tag::OBJECT]),
        "is_scalar" => Intrinsic::IsTag(&[tag::INT, tag::FLOAT, tag::STR, tag::BOOL]),
        _ => return None,
    })
}
