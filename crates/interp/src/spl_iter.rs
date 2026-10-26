//! SPL's IteratorIterator, FilterIterator and RecursiveIteratorIterator,
//! natively. The classes are declared in prelude_spl.php (constructors,
//! hooks, rarely used methods); their state lives here and the stepping
//! methods, which run for every element a framework walks (Symfony
//! Finder wraps each directory scan in several of them), are native.
//!
//! Methods of the wrapped iterators, and the ones a subclass may
//! override (accept(), valid(), the RecursiveIteratorIterator hooks),
//! are called like PHP calls them.

use std::rc::Rc;

use crate::class::Class;
use crate::interp::{Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, Value};

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("iteratoriterator", "__phpx_init", ii_init),
        ("iteratoriterator", "getinneriterator", ii_inner),
        ("iteratoriterator", "__phpx_fetch", ii_fetch),
        ("iteratoriterator", "rewind", ii_rewind),
        ("iteratoriterator", "valid", ii_valid),
        ("iteratoriterator", "key", ii_key),
        ("iteratoriterator", "current", ii_current),
        ("iteratoriterator", "next", ii_next),
        ("filteriterator", "rewind", fi_rewind),
        ("filteriterator", "next", fi_next),
        ("recursiveiteratoriterator", "__phpx_init", rii_init),
        ("recursiveiteratoriterator", "getdepth", rii_depth),
        ("recursiveiteratoriterator", "getsubiterator", rii_sub),
        ("recursiveiteratoriterator", "getinneriterator", rii_inner),
        ("recursiveiteratoriterator", "setmaxdepth", rii_set_max),
        ("recursiveiteratoriterator", "getmaxdepth", rii_get_max),
        ("recursiveiteratoriterator", "callhaschildren", rii_call_has_children),
        ("recursiveiteratoriterator", "callgetchildren", rii_call_get_children),
        ("recursiveiteratoriterator", "rewind", rii_rewind),
        ("recursiveiteratoriterator", "valid", rii_valid),
        ("recursiveiteratoriterator", "key", rii_key),
        ("recursiveiteratoriterator", "current", rii_current),
        ("recursiveiteratoriterator", "next", rii_next),
    ]
}

fn this(o: Option<&Rc<Obj>>) -> &Rc<Obj> {
    o.expect("SPL iterator method needs an object")
}

fn obj_arg(a: &[Value], i: usize) -> Option<Rc<Obj>> {
    match a.get(i).map(Value::deref) {
        Some(Value::Object(o)) => Some(o),
        _ => None,
    }
}

/// Whether `o`'s method `lname` is still the one `class` declares (no
/// subclass overrides it).
fn own(o: &Obj, lname: &str, class: &str) -> bool {
    o.class.find_method(lname).is_some_and(|m| &*m.declaring == class)
}

// ---- IteratorIterator ------------------------------------------------------

#[derive(Default)]
struct IterState {
    inner: Option<Rc<Obj>>,
    valid: bool,
    key: Value,
    current: Value,
}

impl NativeData for IterState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    /// Uncloneable, as in PHP.
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

fn with_iter<T>(o: &Obj, f: impl FnOnce(&mut IterState) -> T) -> Option<T> {
    let mut i = o.internal_mut()?;
    match &mut *i {
        Internal::Data(d) => d.as_any_mut().downcast_mut::<IterState>().map(f),
        _ => None,
    }
}

fn inner_of(o: &Obj) -> Option<Rc<Obj>> {
    with_iter(o, |s| s.inner.clone()).flatten()
}

fn ii_init(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    this(o).set_internal(Internal::Data(Box::new(IterState { inner: obj_arg(&a, 0), ..Default::default() })));
    Ok(Value::Null)
}

fn ii_inner(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(inner_of(this(o)).map_or(Value::Null, Value::Object))
}

/// Refresh the cached element from the inner iterator.
fn fetch(it: &Interp, o: &Rc<Obj>) -> R<()> {
    let Some(inner) = inner_of(o) else { return Ok(()) };
    let valid = it.call_method(&inner, "valid", vec![])?.truthy();
    let (key, current) = if valid {
        let k = it.call_method(&inner, "key", vec![])?;
        (k, it.call_method(&inner, "current", vec![])?)
    } else {
        (Value::Null, Value::Null)
    };
    // (Dropped after the state is released: a destructor may run.)
    let old = with_iter(o, |s| {
        s.valid = valid;
        (std::mem::replace(&mut s.key, key), std::mem::replace(&mut s.current, current))
    });
    drop(old);
    Ok(())
}

fn ii_fetch(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    fetch(it, this(o))?;
    Ok(Value::Null)
}

fn rewind(it: &Interp, o: &Rc<Obj>) -> R<()> {
    if let Some(inner) = inner_of(o) {
        it.call_method(&inner, "rewind", vec![])?;
    }
    fetch(it, o)
}

fn next(it: &Interp, o: &Rc<Obj>) -> R<()> {
    if let Some(inner) = inner_of(o) {
        it.call_method(&inner, "next", vec![])?;
    }
    fetch(it, o)
}

fn ii_rewind(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    rewind(it, this(o))?;
    Ok(Value::Null)
}

fn ii_next(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    next(it, this(o))?;
    Ok(Value::Null)
}

fn ii_valid(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Bool(with_iter(this(o), |s| s.valid).unwrap_or(false)))
}

fn ii_key(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(with_iter(this(o), |s| s.key.clone()).unwrap_or_default())
}

fn ii_current(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(with_iter(this(o), |s| s.current.clone()).unwrap_or_default())
}

// ---- FilterIterator --------------------------------------------------------

/// Step past the elements accept() rejects.
fn skip(it: &Interp, o: &Rc<Obj>) -> R<()> {
    let plain_valid = own(o, "valid", "iteratoriterator");
    loop {
        let valid = if plain_valid { with_iter(o, |s| s.valid).unwrap_or(false) } else { it.call_method(o, "valid", vec![])?.truthy() };
        if !valid || it.call_method(o, "accept", vec![])?.truthy() {
            return Ok(());
        }
        next(it, o)?;
    }
}

fn fi_rewind(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    rewind(it, o)?;
    skip(it, o)?;
    Ok(Value::Null)
}

fn fi_next(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    next(it, o)?;
    skip(it, o)?;
    Ok(Value::Null)
}

// ---- RecursiveIteratorIterator ---------------------------------------------

const SELF_FIRST: i64 = 1;
const CHILD_FIRST: i64 = 2;

/// One depth of the walk: the iterator, whether its element was already
/// yielded before its children (SELF_FIRST), and whether its children
/// are done (CHILD_FIRST).
struct Level {
    it: Rc<Obj>,
    yielded: bool,
    done: bool,
}

struct RiiState {
    root: Option<Rc<Obj>>,
    mode: i64,
    max: i64,
    stack: Vec<Level>,
}

impl NativeData for RiiState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    /// Uncloneable, as in PHP.
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

fn with_rii<T>(o: &Obj, f: impl FnOnce(&mut RiiState) -> T) -> Option<T> {
    let mut i = o.internal_mut()?;
    match &mut *i {
        Internal::Data(d) => d.as_any_mut().downcast_mut::<RiiState>().map(f),
        _ => None,
    }
}

fn top(o: &Obj) -> Option<Rc<Obj>> {
    with_rii(o, |s| s.stack.last().map(|l| l.it.clone())).flatten()
}

fn depth(o: &Obj) -> usize {
    with_rii(o, |s| s.stack.len().saturating_sub(1)).unwrap_or(0)
}

/// A hook (beginChildren() & co) runs only when a subclass defines it.
fn hook(it: &Interp, o: &Rc<Obj>, lname: &str, name: &'static str) -> R<()> {
    if !own(o, lname, "recursiveiteratoriterator") {
        it.call_method(o, name, vec![])?;
    }
    Ok(())
}

fn rii_init(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let mode = match a.get(1).map(Value::deref) {
        Some(Value::Int(n)) => n,
        _ => 0,
    };
    this(o).set_internal(Internal::Data(Box::new(RiiState { root: obj_arg(&a, 0), mode, max: -1, stack: vec![] })));
    Ok(Value::Null)
}

fn rii_depth(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Int(depth(this(o)) as i64))
}

fn rii_sub(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let level = match a.first().map(Value::deref) {
        Some(Value::Int(n)) => Some(n),
        _ => None,
    };
    let sub = with_rii(this(o), |s| {
        let at = match level {
            Some(n) if n < 0 => return None,
            Some(n) => n as usize,
            None => s.stack.len().checked_sub(1)?,
        };
        s.stack.get(at).map(|l| l.it.clone())
    });
    Ok(sub.flatten().map_or(Value::Null, Value::Object))
}

fn rii_inner(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    let inner = top(o).or_else(|| with_rii(o, |s| s.root.clone()).flatten());
    Ok(inner.map_or(Value::Null, Value::Object))
}

fn rii_set_max(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let max = match a.first().map(Value::deref) {
        Some(Value::Int(n)) => n,
        _ => -1,
    };
    with_rii(this(o), |s| s.max = max);
    Ok(Value::Null)
}

fn rii_get_max(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let max = with_rii(this(o), |s| s.max).unwrap_or(-1);
    Ok(if max < 0 { Value::Bool(false) } else { Value::Int(max) })
}

/// The current inner iterator (the root before rewind()).
fn current_inner(it: &Interp, o: &Rc<Obj>) -> R<Rc<Obj>> {
    match top(o).or_else(|| with_rii(o, |s| s.root.clone()).flatten()) {
        Some(i) => Ok(i),
        None => Err(it.error("Error", "The object is in an invalid state as the parent constructor was not called")),
    }
}

fn rii_call_has_children(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let inner = current_inner(it, this(o))?;
    Ok(Value::Bool(it.call_method(&inner, "hasChildren", vec![])?.truthy()))
}

fn rii_call_get_children(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let inner = current_inner(it, this(o))?;
    it.call_method(&inner, "getChildren", vec![])
}

/// Whether the walk goes into the children of `inner`'s current element.
fn can_descend(it: &Interp, o: &Rc<Obj>, inner: &Rc<Obj>) -> R<bool> {
    let max = with_rii(o, |s| s.max).unwrap_or(-1);
    if max >= 0 && depth(o) as i64 >= max {
        return Ok(false);
    }
    let v = if own(o, "callhaschildren", "recursiveiteratoriterator") {
        it.call_method(inner, "hasChildren", vec![])?
    } else {
        it.call_method(o, "callHasChildren", vec![])?
    };
    Ok(v.truthy())
}

fn descend(it: &Interp, o: &Rc<Obj>, inner: &Rc<Obj>) -> R<()> {
    let child = if own(o, "callgetchildren", "recursiveiteratoriterator") {
        it.call_method(inner, "getChildren", vec![])?
    } else {
        it.call_method(o, "callGetChildren", vec![])?
    };
    let Value::Object(child) = child.deref() else {
        return Err(it.error("UnexpectedValueException", "Objects returned by RecursiveIterator::getChildren() must implement RecursiveIterator"));
    };
    it.call_method(&child, "rewind", vec![])?;
    with_rii(o, |s| s.stack.push(Level { it: child, yielded: false, done: false }));
    hook(it, o, "beginchildren", "beginChildren")
}

/// Advance the iterator at the top of the stack to its next element.
fn advance(it: &Interp, o: &Rc<Obj>) -> R<()> {
    let Some(inner) = top(o) else { return Ok(()) };
    it.call_method(&inner, "next", vec![])?;
    with_rii(o, |s| {
        if let Some(l) = s.stack.last_mut() {
            l.yielded = false;
            l.done = false;
        }
    });
    Ok(())
}

/// Move to the next element to yield (or the end).
fn settle(it: &Interp, o: &Rc<Obj>) -> R<()> {
    let mode = with_rii(o, |s| s.mode).unwrap_or(0);
    loop {
        let Some((inner, d, yielded, done)) = with_rii(o, |s| s.stack.last().map(|l| (l.it.clone(), s.stack.len() - 1, l.yielded, l.done))).flatten() else {
            return Ok(());
        };
        if !it.call_method(&inner, "valid", vec![])?.truthy() {
            if d == 0 {
                return hook(it, o, "enditeration", "endIteration");
            }
            let popped = with_rii(o, |s| s.stack.pop());
            drop(popped);
            hook(it, o, "endchildren", "endChildren")?;
            if mode == CHILD_FIRST {
                with_rii(o, |s| {
                    if let Some(l) = s.stack.last_mut() {
                        l.done = true;
                    }
                });
                return hook(it, o, "nextelement", "nextElement");
            }
            advance(it, o)?;
            continue;
        }
        if !done && can_descend(it, o, &inner)? {
            if mode == SELF_FIRST && !yielded {
                with_rii(o, |s| {
                    if let Some(l) = s.stack.last_mut() {
                        l.yielded = true;
                    }
                });
                return hook(it, o, "nextelement", "nextElement");
            }
            descend(it, o, &inner)?;
            continue;
        }
        return hook(it, o, "nextelement", "nextElement");
    }
}

fn rii_rewind(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    let root = current_root(it, o)?;
    it.call_method(&root, "rewind", vec![])?;
    let old = with_rii(o, |s| std::mem::replace(&mut s.stack, vec![Level { it: root, yielded: false, done: false }]));
    drop(old);
    hook(it, o, "beginiteration", "beginIteration")?;
    settle(it, o)?;
    Ok(Value::Null)
}

fn current_root(it: &Interp, o: &Rc<Obj>) -> R<Rc<Obj>> {
    match with_rii(o, |s| s.root.clone()).flatten() {
        Some(r) => Ok(r),
        None => Err(it.error("Error", "The object is in an invalid state as the parent constructor was not called")),
    }
}

fn rii_valid(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    match top(this(o)) {
        Some(inner) => Ok(Value::Bool(it.call_method(&inner, "valid", vec![])?.truthy())),
        None => Ok(Value::Bool(false)),
    }
}

fn rii_key(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    match top(this(o)) {
        Some(inner) => it.call_method(&inner, "key", vec![]),
        None => Ok(Value::Null),
    }
}

fn rii_current(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    match top(this(o)) {
        Some(inner) => it.call_method(&inner, "current", vec![]),
        None => Ok(Value::Null),
    }
}

fn rii_next(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    let Some((inner, yielded, done)) = with_rii(o, |s| s.stack.last().map(|l| (l.it.clone(), l.yielded, l.done))).flatten() else {
        return Ok(Value::Null);
    };
    if yielded && !done && can_descend(it, o, &inner)? {
        // SELF_FIRST: the parent was yielded; its children come next.
        with_rii(o, |s| {
            if let Some(l) = s.stack.last_mut() {
                l.done = true;
            }
        });
        descend(it, o, &inner)?;
    } else {
        advance(it, o)?;
    }
    settle(it, o)?;
    Ok(Value::Null)
}
