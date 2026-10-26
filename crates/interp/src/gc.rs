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

use std::cell::{Cell, UnsafeCell};
use std::mem::ManuallyDrop;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::interp::Callable;
use crate::value::{destruct_pending, Internal, Obj, Value, DESTRUCT_QUEUE};

const MIN_THRESHOLD: usize = 10_000;

/// All collector state in one thread-local (one lookup per object).
///
/// The registry is the object store, like PHP's: an object's handle
/// (`spl_object_id`) is its index. A slot holds the object's address, 0
/// while its object is being destroyed (handle still taken), or, when free,
/// `next_free << 1 | 1` (a free list, most recently freed first, as PHP
/// reuses handles).
struct State {
    store: UnsafeCell<Vec<usize>>,
    free_head: Cell<usize>,
    live: Cell<usize>,
    /// Objects registered since the last collection.
    since: Cell<usize>,
    threshold: Cell<usize>,
    due: Cell<bool>,
    enabled: Cell<bool>,
    runs: Cell<u64>,
    collected: Cell<u64>,
    /// Registration counter (orders destructors of collected cycles).
    seq: Cell<u32>,
}

thread_local! {
    static ST: State = const {
        State {
            // Handle 0 is never used.
            store: UnsafeCell::new(Vec::new()),
            free_head: Cell::new(0),
            live: Cell::new(0),
            since: Cell::new(0),
            threshold: Cell::new(MIN_THRESHOLD),
            due: Cell::new(false),
            enabled: Cell::new(true),
            runs: Cell::new(0),
            collected: Cell::new(0),
            seq: Cell::new(0),
        }
    };
}

impl State {
    /// SAFETY: one thread; no caller holds a reference across a call that
    /// can re-enter the collector (these functions never run PHP code).
    #[allow(clippy::mut_from_ref)]
    fn store(&self) -> &mut Vec<usize> {
        unsafe { &mut *self.store.get() }
    }

    fn note_new(&self) {
        self.live.set(self.live.get() + 1);
        let n = self.since.get() + 1;
        self.since.set(n);
        if n >= self.threshold.get() && self.enabled.get() && !self.due.get() {
            self.due.set(true);
            // Runs at the next statement boundary, with the destructors.
            destruct_pending().store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Allocate an object: its handle, then (with the handle) the object, then
/// its registry slot, with one thread-local lookup.
#[inline]
pub fn track(make: impl FnOnce(u32) -> Rc<Obj>) -> Rc<Obj> {
    ST.with(|st| {
        let id = match st.free_head.get() {
            0 => {
                let store = st.store();
                if store.is_empty() {
                    store.push(0);
                }
                store.push(0);
                store.len() - 1
            }
            id => {
                st.free_head.set(st.store()[id] >> 1);
                id
            }
        };
        assert!(id <= u32::MAX as usize, "too many objects");
        let o = make(id as u32);
        let seq = st.seq.get().wrapping_add(1);
        st.seq.set(seq);
        o.set_seq(seq);
        st.store()[id] = Rc::as_ptr(&o) as usize;
        st.note_new();
        o
    })
}

/// A new request's interpreter: handles start over from 1, like PHP's
/// object store at request start. (Free handles are relinked lowest first
/// when objects of an earlier request still live.)
pub fn new_request() {
    free_leftovers();
    let _ = ST.try_with(|st| {
        let store = st.store();
        if st.live.get() == 0 {
            store.clear();
            st.free_head.set(0);
        } else {
            let mut head = 0;
            for id in (1..store.len()).rev() {
                if store[id] & 1 == 1 {
                    store[id] = head << 1 | 1;
                    head = id;
                }
            }
            st.free_head.set(head);
        }
        st.since.set(0);
        st.threshold.set(MIN_THRESHOLD);
        st.due.set(false);
        st.enabled.set(true);
        st.runs.set(0);
        st.collected.set(0);
    });
}

/// Objects an earlier request's interpreter left alive (reference cycles:
/// PHP frees its whole heap at request end, phpx frees by reference
/// count). Their destructors ran at that request's shutdown; what they,
/// their classes' static properties and constants hold is released so the
/// cycles fall apart.
fn free_leftovers() {
    let live: Vec<Rc<Obj>> = ST
        .try_with(|st| {
            if st.live.get() == 0 {
                return vec![];
            }
            st.store()
                .iter()
                .filter(|&&p| p != 0 && p & 1 == 0)
                .map(|&p| unsafe {
                    Rc::increment_strong_count(p as *const Obj);
                    Rc::from_raw(p as *const Obj)
                })
                .collect()
        })
        .unwrap_or_default();
    if live.is_empty() {
        return;
    }
    let mut held: Vec<Box<dyn std::any::Any>> = vec![];
    for o in &live {
        o.set_destructed();
        held.push(Box::new(std::mem::take(&mut *o.props.borrow_mut())));
        held.push(Box::new(o.take_internal()));
        let mut c = Some(o.class.clone());
        while let Some(cls) = c {
            for sp in cls.static_props.values() {
                held.push(Box::new(sp.value.borrow_mut().take()));
            }
            held.push(Box::new(std::mem::take(&mut *cls.consts.borrow_mut())));
            c = cls.parent.clone();
        }
    }
    drop(held);
    drop(live);
}

/// Object freed: give back its registry slot and (unless `keep_id`) its
/// handle.
pub fn release(o: &Obj, keep_id: bool) {
    let _ = ST.try_with(|st| {
        let id = o.id as usize;
        let store = st.store();
        if id < store.len() {
            if store[id] != 0 && store[id] & 1 == 0 {
                st.live.set(st.live.get() - 1);
            }
            if keep_id {
                store[id] = 0;
            } else {
                store[id] = st.free_head.get() << 1 | 1;
                st.free_head.set(id);
            }
        }
    });
}

/// A destructed object brought back for its destructor (same handle).
pub fn register(o: &Rc<Obj>) {
    let _ = ST.try_with(|st| {
        st.store()[o.id as usize] = Rc::as_ptr(o) as usize;
        st.note_new();
    });
}

/// Out of the registry, keeping the handle (an object about to be
/// resurrected for its destructor).
pub fn unregister(o: &Obj) {
    release(o, true);
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
    ST.with(|st| (st.runs.get(), st.collected.get(), st.threshold.get(), st.live.get()))
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
    for v in props.all_values() {
        if !visit_value(v, f) {
            return false;
        }
    }
    drop(props);
    let Some(internal) = o.try_internal() else { return false };
    if let Internal::Closure(c) = &*internal {
        {
            match c {
                Callable::Closure(cd) if Rc::strong_count(cd) == 1 => {
                    if let Some(t) = &cd.this {
                        f(Rc::as_ptr(t));
                    }
                    for (_, v, _) in &cd.captured {
                        if !visit_value(v, f) {
                            return false;
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
    let ptrs: Vec<*const Obj> = ST.with(|st| st.store().iter().filter(|&&e| e != 0 && e & 1 == 0).map(|&e| e as *const Obj).collect());
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
    let mut pending: Vec<&Rc<Obj>> = garbage.iter().filter(|o| o.class.has_destruct && !o.destructed()).collect();
    // In the order the objects were created (PHP: the order they became
    // possible roots), not by handle.
    pending.sort_by_key(|o| o.seq());
    if !pending.is_empty() {
        for o in pending {
            o.set_destructed();
            let o = (*o).clone();
            let _ = DESTRUCT_QUEUE.try_with(|q| q.borrow_mut().push(o));
        }
        destruct_pending().store(true, std::sync::atomic::Ordering::Relaxed);
        ST.with(|st| st.due.set(true));
        return 0;
    }
    let count = garbage.len();
    // Break the cycles: move the contents out, then drop everything.
    let mut doomed: Vec<(crate::props::Props, Internal)> = Vec::with_capacity(count);
    for o in &garbage {
        let props = o.props.try_borrow_mut().map(|mut p| std::mem::take(&mut *p)).unwrap_or_default();
        let internal = o.internal_mut().map(|mut i| std::mem::take(&mut *i)).unwrap_or_default();
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
