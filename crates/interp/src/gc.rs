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

/// All collector state in one thread-local (one lookup per object).
struct State {
    registry: RefCell<Vec<*const Obj>>,
    /// Objects registered since the last collection.
    since: Cell<usize>,
    threshold: Cell<usize>,
    due: Cell<bool>,
    enabled: Cell<bool>,
    runs: Cell<u64>,
    collected: Cell<u64>,
    /// Object handles (spl_object_id): PHP reuses freed ones, most recently
    /// freed first.
    next_id: Cell<u32>,
    free_ids: RefCell<Vec<u32>>,
}

thread_local! {
    static ST: State = const {
        State {
            registry: RefCell::new(Vec::new()),
            since: Cell::new(0),
            threshold: Cell::new(MIN_THRESHOLD),
            due: Cell::new(false),
            enabled: Cell::new(true),
            runs: Cell::new(0),
            collected: Cell::new(0),
            next_id: Cell::new(1),
            free_ids: RefCell::new(Vec::new()),
        }
    };
}

/// Allocate an object: its handle, then (with the handle) the object, then
/// its registry slot, with one thread-local lookup.
pub fn track(make: impl FnOnce(u32) -> Rc<Obj>) -> Rc<Obj> {
    ST.with(|st| {
        let id = st.free_ids.borrow_mut().pop().unwrap_or_else(|| {
            let id = st.next_id.get();
            st.next_id.set(id + 1);
            id
        });
        let o = make(id);
        register_in(st, &o);
        o
    })
}

fn register_in(st: &State, o: &Rc<Obj>) {
    let p = Rc::as_ptr(o);
    let mut r = st.registry.borrow_mut();
    o.gc_index.set(r.len() as u32);
    r.push(p);
    let n = st.since.get() + 1;
    st.since.set(n);
    if n >= st.threshold.get() && st.enabled.get() && !st.due.get() {
        st.due.set(true);
        DESTRUCT_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Object freed: give back its registry slot and (unless `keep_id`) its
/// handle.
pub fn release(o: &Obj, keep_id: bool) {
    let _ = ST.try_with(|st| {
        unregister_in(st, o);
        if !keep_id {
            st.free_ids.borrow_mut().push(o.id);
        }
    });
}

pub fn register(o: &Rc<Obj>) {
    let p = Rc::as_ptr(o);
    let _ = ST.try_with(|st| {
        let mut r = st.registry.borrow_mut();
        o.gc_index.set(r.len() as u32);
        r.push(p);
        let n = st.since.get() + 1;
        st.since.set(n);
        if n >= st.threshold.get() && st.enabled.get() && !st.due.get() {
            st.due.set(true);
            // Runs at the next statement boundary, with the destructors.
            DESTRUCT_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
}

pub fn unregister(o: &Obj) {
    let _ = ST.try_with(|st| unregister_in(st, o));
}

fn unregister_in(st: &State, o: &Obj) {
    let i = o.gc_index.get();
    if i == u32::MAX {
        return;
    }
    o.gc_index.set(u32::MAX);
    {
        let mut r = st.registry.borrow_mut();
        let i = i as usize;
        if i < r.len() {
            r.swap_remove(i);
            if i < r.len() {
                // SAFETY: registry entries are live objects (removed on drop).
                unsafe { (*r[i]).gc_index.set(i as u32) };
            }
        }
    }
}

/// A collection is due (checked at statement boundaries).
pub fn take_due() -> bool {
    ST.with(|st| st.due.replace(false))
}

pub fn set_enabled(on: bool) {
    ST.with(|st| st.enabled.set(on));
}

pub fn enabled() -> bool {
    ST.with(|st| st.enabled.get())
}

/// (runs, collected, threshold, roots) for gc_status().
pub fn status() -> (u64, u64, usize, usize) {
    ST.with(|st| (st.runs.get(), st.collected.get(), st.threshold.get(), st.registry.borrow().len()))
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
    let ptrs: Vec<*const Obj> = ST.with(|st| st.registry.borrow().clone());
    let n = ptrs.len();
    ST.with(|st| st.runs.set(st.runs.get() + 1));
    ST.with(|st| st.since.set(0));
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
        ST.with(|st| st.due.set(true));
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
    ST.with(|st| st.collected.set(st.collected.get() + count as u64));
    adapt(n, count);
    count
}

/// Next run after max(10k, live objects) new ones: amortized O(1) per
/// allocation, like PHP's growing root buffer.
fn adapt(scanned: usize, freed: usize) {
    let live = scanned - freed;
    ST.with(|st| st.threshold.set(live.max(MIN_THRESHOLD)));
}
