//! WeakReference and WeakMap.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::class::Class;
use crate::interp::{BuiltinFn, Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, PhpArray, Value};

struct WeakRef(Weak<Obj>);

impl NativeData for WeakRef {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

/// Entries keyed by the key object's address; dead keys are dropped on
/// the next access.
#[derive(Clone, Default)]
struct Map(Vec<(Weak<Obj>, Value)>);

impl Map {
    fn purge(&mut self) {
        self.0.retain(|(k, _)| k.strong_count() > 0);
    }

    fn find(&self, o: &Rc<Obj>) -> Option<usize> {
        let p = Rc::as_ptr(o);
        self.0.iter().position(|(k, _)| k.as_ptr() == p && k.strong_count() > 0)
    }
}

impl NativeData for Map {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        Some(Box::new(self.clone()))
    }
}

thread_local! {
    /// WeakReference::create() returns the same object while it lives.
    static REFS: RefCell<HashMap<usize, (Weak<Obj>, Weak<Obj>)>> = RefCell::new(HashMap::new());
}

fn weakref_create(it: &Interp, _: Option<&Rc<Obj>>, cls: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let Some(Value::Object(target)) = a.first().map(Value::deref) else {
        return Err(it.error("TypeError", "WeakReference::create(): Argument #1 ($object) must be of type object"));
    };
    let key = Rc::as_ptr(&target) as usize;
    let cached = REFS.with(|r| {
        let r = r.borrow();
        let (t, w) = r.get(&key)?;
        (t.as_ptr() == Rc::as_ptr(&target) && t.strong_count() > 0).then(|| w.upgrade()).flatten()
    });
    if let Some(w) = cached {
        return Ok(Value::Object(w));
    }
    let o = it.new_instance(cls)?;
    o.set_internal(Internal::Data(Box::new(WeakRef(Rc::downgrade(&target)))));
    REFS.with(|r| {
        let mut r = r.borrow_mut();
        r.retain(|_, (t, w)| t.strong_count() > 0 && w.strong_count() > 0);
        r.insert(key, (Rc::downgrade(&target), Rc::downgrade(&o)));
    });
    Ok(Value::Object(o))
}

fn weakref_get(_: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let Some(o) = this else { return Ok(Value::Null) };
    let target = match &*o.internal() {
        Internal::Data(d) => d.as_any().downcast_ref::<WeakRef>().and_then(|w| w.0.upgrade()),
        _ => None,
    };
    Ok(target.map(Value::Object).unwrap_or(Value::Null))
}

fn with_map<T>(o: &Rc<Obj>, g: impl FnOnce(&mut Map) -> T) -> T {
    let has = matches!(&*o.internal(), Internal::Data(d) if d.as_any().is::<Map>());
    if !has {
        o.set_internal(Internal::Data(Box::new(Map::default())));
    }
    let mut i = o.internal_mut().expect("WeakMap state");
    let Internal::Data(d) = &mut *i else { unreachable!() };
    g(d.as_any_mut().downcast_mut::<Map>().unwrap())
}

fn key(it: &Interp, a: &[Value]) -> R<Rc<Obj>> {
    match a.first().map(Value::deref) {
        Some(Value::Object(o)) => Ok(o),
        _ => Err(it.error("TypeError", "WeakMap key must be an object")),
    }
}

fn map_get(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.expect("WeakMap method");
    let k = key(it, &a)?;
    // `$r = &$map[$k]` and `$map[$k][] = ...` share the element's cell.
    let want_ref = it.want_ref.get() == Some(crate::interp::WANT_REF_ANY);
    let found = with_map(o, |m| {
        m.find(&k).map(|i| {
            let v = &mut m.0[i].1;
            if want_ref && !matches!(v, Value::Ref(_)) {
                *v = Value::Ref(crate::interp::slot(std::mem::take(v)));
            }
            if want_ref { v.clone() } else { v.deref() }
        })
    });
    match found {
        Some(v) => Ok(v),
        None => Err(it.error("Error", &format!("Object {}#{} not contained in WeakMap", k.class.name, k.id))),
    }
}

fn map_set(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.expect("WeakMap method");
    let k = key(it, &a)?;
    let v = a.get(1).map(Value::deref).unwrap_or_default();
    with_map(o, |m| {
        m.purge();
        match m.find(&k) {
            Some(i) => match &m.0[i].1 {
                Value::Ref(cell) => *cell.borrow_mut() = v,
                _ => m.0[i].1 = v,
            },
            None => m.0.push((Rc::downgrade(&k), v)),
        }
    });
    Ok(Value::Null)
}

fn map_exists(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.expect("WeakMap method");
    let k = key(it, &a)?;
    Ok(Value::Bool(with_map(o, |m| m.find(&k).is_some_and(|i| !m.0[i].1.deref().is_null()))))
}

fn map_unset(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this.expect("WeakMap method");
    let k = key(it, &a)?;
    let removed = with_map(o, |m| m.find(&k).map(|i| m.0.remove(i)));
    drop(removed);
    Ok(Value::Null)
}

fn map_count(_: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this.expect("WeakMap method");
    Ok(Value::Int(with_map(o, |m| {
        m.purge();
        m.0.len() as i64
    })))
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("weakreference", "static create", weakref_create as NativeMethod),
        ("weakreference", "get", weakref_get),
        ("weakmap", "offsetget", map_get),
        ("weakmap", "offsetset", map_set),
        ("weakmap", "offsetexists", map_exists),
        ("weakmap", "offsetunset", map_unset),
        ("weakmap", "count", map_count),
    ]
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    // [[key, value], ...] for WeakMap::getIterator().
    t.insert("__phpx_weakmap_entries", |_, _, a| {
        let Some(Value::Object(o)) = a.first().map(Value::deref) else { return Ok(Value::array(PhpArray::new())) };
        let items = with_map(&o, |m| {
            m.purge();
            m.0.iter().filter_map(|(k, v)| Some(Value::array(PhpArray::from_list([Value::Object(k.upgrade()?), v.deref()])))).collect::<Vec<_>>()
        });
        Ok(Value::array(PhpArray::from_list(items)))
    });
}
