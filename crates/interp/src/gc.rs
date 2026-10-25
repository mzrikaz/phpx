//! Cycle collector. Reference counting (`Rc`) frees acyclic garbage at once;
//! objects that reference each other (directly or through arrays, reference
//! cells and closures) need this pass, like PHP's `gc_collect_cycles()`.
//!
//! Every live object is in a registry. A collection counts, for each
//! object, the references to it held by other registered objects; an object
//! whose strong count exceeds that is referenced from outside (a variable,
//! the stack, native code) and is live, along with everything it reaches.
//! The rest is cyclic garbage: its destructors run first, and on the next
//! pass the cycles are broken by clearing the objects' properties.
//!
//! Containers that are shared (an array or reference cell with more than one
//! owner) are not looked into: references inside them stay uncounted, which
//! keeps their targets alive. The pass may miss some garbage; it never frees
//! a reachable object.

use std::cell::{Cell, RefCell};
use std::mem::ManuallyDrop;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::interp::Callable;
use crate::value::{Internal, Obj, Value, DESTRUCT_PENDING, DESTRUCT_QUEUE};

const MIN_THRESHOLD: usize = 10_000;

thread_local! {
    static REGISTRY: RefCell<Vec<*const Obj>> = const { RefCell::new(Vec::new()) };
    /// Objects registered since the last collection.
    static SINCE: Cell<usize> = const { Cell::new(0) };
    static THRESHOLD: Cell<usize> = const { Cell::new(MIN_THRESHOLD) };
    static DUE: Cell<bool> = const { Cell::new(false) };
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    static RUNS: Cell<u64> = const { Cell::new(0) };
    static COLLECTED: Cell<u64> = const { Cell::new(0) };
}

pub fn register(o: &Rc<Obj>) {
    let p = Rc::as_ptr(o);
    let _ = REGISTRY.try_with(|r| {
        let mut r = r.borrow_mut();
        o.gc_index.set(r.len() as u32);
        r.push(p);
    });
    let n = SINCE.with(|s| {
        s.set(s.get() + 1);
        s.get()
    });
    if n >= THRESHOLD.with(Cell::get) && ENABLED.with(Cell::get) && !DUE.with(Cell::get) {
        DUE.with(|d| d.set(true));
        // Runs at the next statement boundary, with the destructors.
        DESTRUCT_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub fn unregister(o: &Obj) {
    let i = o.gc_index.get();
    if i == u32::MAX {
        return;
    }
    o.gc_index.set(u32::MAX);
    let _ = REGISTRY.try_with(|r| {
        let mut r = r.borrow_mut();
        let i = i as usize;
        if i < r.len() {
            r.swap_remove(i);
            if i < r.len() {
                // SAFETY: registry entries are live objects (removed on drop).
                unsafe { (*r[i]).gc_index.set(i as u32) };
            }
        }
    });
}

/// A collection is due (checked at statement boundaries).
pub fn take_due() -> bool {
    DUE.with(|d| d.replace(false))
}

pub fn set_enabled(on: bool) {
    ENABLED.with(|e| e.set(on));
}

pub fn enabled() -> bool {
    ENABLED.with(Cell::get)
}

/// (runs, collected, threshold, roots) for gc_status().
pub fn status() -> (u64, u64, usize, usize) {
    (RUNS.with(Cell::get), COLLECTED.with(Cell::get), THRESHOLD.with(Cell::get), REGISTRY.with(|r| r.borrow().len()))
}

/// Visit the objects `v` holds a counted reference to.
fn visit_value(v: &Value, f: &mut dyn FnMut(*const Obj)) -> bool {
    match v {
        Value::Object(o) => f(Rc::as_ptr(o)),
        Value::Array(a) if Rc::strong_count(a) == 1 => {
            for x in a.raw_values() {
                if !visit_value(x, f) {
                    return false;
                }
            }
        }
        Value::Ref(s) if Rc::strong_count(s) == 1 => match s.try_borrow() {
            Ok(x) => return visit_value(&x, f),
            Err(_) => return false,
        },
        _ => {}
    }
    true
}

/// Visit an object's outgoing references. False: it couldn't be inspected
/// (borrowed right now), so treat it as externally referenced.
fn visit_obj(o: &Obj, f: &mut dyn FnMut(*const Obj)) -> bool {
    let Ok(props) = o.props.try_borrow() else { return false };
    for v in props.values() {
        if !visit_value(v, f) {
            return false;
        }
    }
    drop(props);
    let Ok(internal) = o.internal.try_borrow() else { return false };
    if let Internal::Closure(c) = &*internal {
        if Rc::strong_count(c) == 1 {
            match &**c {
                Callable::Closure(cd) if Rc::strong_count(cd) == 1 => {
                    if let Some(t) = &cd.this {
                        f(Rc::as_ptr(t));
                    }
                    for (_, slot, _) in &cd.captured {
                        if Rc::strong_count(slot) == 1 {
                            match slot.try_borrow() {
                                Ok(x) => {
                                    if !visit_value(&x, f) {
                                        return false;
                                    }
                                }
                                Err(_) => return false,
                            }
                        }
                    }
                }
                Callable::Method { this: Some(t), .. } => f(Rc::as_ptr(t)),
                _ => {}
            }
        }
    }
    true
}

/// Run a collection. Returns the number of objects freed. Must be called at
/// a safe point (no property or array borrows held by the caller).
pub fn collect() -> usize {
    let ptrs: Vec<*const Obj> = REGISTRY.with(|r| r.borrow().clone());
    let n = ptrs.len();
    RUNS.with(|r| r.set(r.get() + 1));
    SINCE.with(|s| s.set(0));
    if n == 0 {
        return 0;
    }
    let index: FxHashMap<usize, usize> = ptrs.iter().enumerate().map(|(i, p)| (*p as usize, i)).collect();
    let mut internal = vec![0usize; n];
    let mut edges: Vec<Vec<u32>> = vec![vec![]; n];
    let mut opaque = vec![false; n];
    for (i, &p) in ptrs.iter().enumerate() {
        // SAFETY: registry entries are live; nothing is dropped during the scan.
        let o = unsafe { &*p };
        let ok = visit_obj(o, &mut |c| {
            if let Some(&j) = index.get(&(c as usize)) {
                internal[j] += 1;
                edges[i].push(j as u32);
            }
        });
        if !ok {
            opaque[i] = true;
        }
    }
    // Externally referenced objects are roots; mark what they reach.
    let mut live = vec![false; n];
    let mut stack: Vec<usize> = vec![];
    for (i, &p) in ptrs.iter().enumerate() {
        // SAFETY: as above; ManuallyDrop keeps the count unchanged.
        let rc = ManuallyDrop::new(unsafe { Rc::from_raw(p) });
        if opaque[i] || Rc::strong_count(&rc) > internal[i] {
            live[i] = true;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        for &j in &edges[i] {
            let j = j as usize;
            if !live[j] {
                live[j] = true;
                stack.push(j);
            }
        }
    }
    let garbage: Vec<Rc<Obj>> = (0..n)
        .filter(|&i| !live[i])
        .map(|i| {
            // SAFETY: take a new strong reference to keep it alive while we work.
            unsafe {
                Rc::increment_strong_count(ptrs[i]);
                Rc::from_raw(ptrs[i])
            }
        })
        .collect();
    if garbage.is_empty() {
        adapt(n, 0);
        return 0;
    }
    // Destructors first (PHP semantics); the cycles are broken next time.
    let pending: Vec<&Rc<Obj>> = garbage.iter().filter(|o| o.class.has_destruct && !o.destructed.get()).collect();
    if !pending.is_empty() {
        for o in pending {
            o.destructed.set(true);
            let o = (*o).clone();
            let _ = DESTRUCT_QUEUE.try_with(|q| q.borrow_mut().push(o));
        }
        DESTRUCT_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
        DUE.with(|d| d.set(true));
        return 0;
    }
    let count = garbage.len();
    // Break the cycles: move the contents out, then drop everything.
    let mut doomed: Vec<(crate::props::Props, Internal)> = Vec::with_capacity(count);
    for o in &garbage {
        let props = o.props.try_borrow_mut().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        let internal = o.internal.try_borrow_mut().map(|mut i| std::mem::take(&mut *i)).unwrap_or_default();
        doomed.push((props, internal));
    }
    drop(doomed);
    drop(garbage);
    COLLECTED.with(|c| c.set(c.get() + count as u64));
    adapt(n, count);
    count
}

/// Next run after max(10k, live objects) new ones: amortized O(1) per
/// allocation, like PHP's growing root buffer.
fn adapt(scanned: usize, freed: usize) {
    let live = scanned - freed;
    THRESHOLD.with(|t| t.set(live.max(MIN_THRESHOLD)));
}
