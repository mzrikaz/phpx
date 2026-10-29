//! Callbacks that built-ins call over and over (array_map, usort,
//! array_filter, ...): the callable is resolved once to its compiled entry
//! and runs in one reused frame, with the arguments passed in place.
//! Anything that can't run that way (built-ins, generators, by-reference
//! parameters, code that doesn't compile) takes the general call path.


use crate::compiler::Entry;
use crate::interp::{Callable, ClosureKind, Frame, Interp, R};
use crate::value::Value;

pub struct Callback<'a> {
    it: &'a Interp,
    callable: Callable,
    prepared: Option<Prepared>,
    /// Preparation attempts left (code compiles after a few calls).
    tries: u8,
}

/// The compiled entry and its frame, set up once for the callable (scope,
/// `$this`, the closure); calls only reset what a call leaves behind.
struct Prepared {
    entry: Entry,
    frame: Frame,
}

impl Interp {
    /// A callable prepared for repeated calls from a built-in.
    pub fn callback(&self, callable: Callable) -> Callback<'_> {
        Callback { it: self, callable, prepared: None, tries: 3 }
    }
}

impl Callback<'_> {
    pub fn callable(&self) -> &Callable {
        &self.callable
    }

    /// Call with these arguments (consumed).
    pub fn call(&mut self, args: &mut [Value]) -> R<Value> {
        if self.prepared.is_none() && self.tries > 0 {
            self.tries -= 1;
            self.prepared = self.prepare();
        }
        if let Some(p) = &mut self.prepared {
            let it = self.it;
            // By-value parameters: references are passed as their values.
            for a in args.iter_mut() {
                if let Value::Ref(_) = a {
                    *a = std::mem::take(a).into_deref();
                }
            }
            let mut out = Value::Null;
            let (ptr, n) = (args.as_mut_ptr(), args.len() as u64);
            // SAFETY: the entry follows the Entry ABI; `args` and `out`
            // outlive the call; a declined call leaves the arguments as they
            // were.
            let st = unsafe { crate::compiler::rt::enter(it, p.entry, &mut p.frame, ptr, n, &mut out) };
            p.frame.reset_vars();
            match st {
                Some(0) => return Ok(out),
                Some(_) => return Err(it.pending.borrow_mut().take().expect("native error without pending exception")),
                None => {}
            }
        }
        let pos: Vec<Value> = args.iter_mut().map(std::mem::take).collect();
        self.it.call_callable(&self.callable, pos, vec![])
    }

    fn prepare(&self) -> Option<Prepared> {
        let it = self.it;
        let plain = |ps: &[phpx_syntax::ast::Param]| ps.iter().all(|p| !p.by_ref && !p.variadic);
        match &self.callable {
            Callable::Closure(cd) => {
                let (fn_id, params, body, ret) = match &cd.kind {
                    ClosureKind::Closure(c) if !c.is_generator => {
                        (*c as *const _ as usize, &c.params[..], crate::compiler::Body::Block(&c.body), c.ret.as_ref())
                    }
                    ClosureKind::Arrow(a) if !a.is_generator => (*a as *const _ as usize, &a.params[..], crate::compiler::Body::Expr(&a.body), a.ret.as_ref()),
                    _ => return None,
                };
                if !plain(params) || cd.captured.iter().any(|c| c.2) {
                    return None;
                }
                let entry = it.native_entry(fn_id, || crate::compiler::Target {
                    fn_id,
                    display: cd.name.clone(),
                    params,
                    captured: cd.captured.iter().map(|(n, _, _)| *n).collect(),
                    body,
                    ctx: cd.ctx.clone(),
                    class: cd.scope.clone(),
                    ret,
                })?;
                let mut frame = Frame::for_call(cd.ctx.clone(), cd.name.clone(), fn_id);
                frame.this = cd.this.clone();
                frame.class = cd.scope.clone();
                frame.static_class = cd.static_class.clone();
                frame.closure = Some(cd.clone());
                Some(Prepared { entry, frame })
            }
            Callable::User(u) => {
                if !plain(&u.decl.params) || u.decl.is_generator {
                    return None;
                }
                let fn_id = u.decl as *const _ as usize;
                let body = u.decl.body.as_deref()?;
                let entry = it.native_entry(fn_id, || crate::compiler::Target {
                    fn_id,
                    display: u.name.clone(),
                    params: &u.decl.params,
                    captured: vec![],
                    body: crate::compiler::Body::Block(body),
                    ctx: u.ctx.clone(),
                    class: None,
                    ret: u.decl.ret.as_ref(),
                })?;
                Some(Prepared { entry, frame: Frame::for_call(u.ctx.clone(), u.name.clone(), fn_id) })
            }
            Callable::Method { this, method, static_class, .. } => {
                let crate::class::MethodBody::User(func) = &method.body else { return None };
                if !plain(&func.params) || func.is_generator {
                    return None;
                }
                // SAFETY: plain lookup of the compiled method.
                let (entry, decl_class) = unsafe { crate::compiler::rt::method_entry(it, method)? };
                let mut frame = Frame::for_call(method.ctx.clone(), method.name.clone(), *func as *const _ as usize);
                frame.class = decl_class;
                frame.static_class = Some(static_class.clone());
                frame.this = if method.is_static { None } else { this.clone() };
                Some(Prepared { entry, frame })
            }
            Callable::Builtin(..) | Callable::Magic { .. } => None,
        }
    }
}
