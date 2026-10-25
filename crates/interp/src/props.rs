//! Object property storage: a shared, immutable *shape* (property names in
//! order, like V8 hidden classes / PHP's property tables) plus one flat
//! vector of values per object.
//!
//! All instances of a class start from the class's shape, so declared
//! properties cost 16 bytes each and lookups can be cached by
//! (shape, slot). Adding a dynamic property moves the object to a child
//! shape (shared between objects that add the same names in the same order).
//! An empty slot (`None`) is an unset or uninitialized (typed, no default)
//! property: present in the shape, absent to PHP code.

use std::cell::RefCell;
use std::rc::Rc;

use rustc_hash::FxHashMap;

use crate::value::Value;

pub struct Shape {
    pub names: Vec<Rc<str>>,
    index: FxHashMap<Rc<str>, u32>,
    /// The first `declared` names are class-declared properties: unsetting
    /// one keeps its slot (it returns to its declared position when set
    /// again, as in PHP). Later names are dynamic.
    declared: usize,
    transitions: RefCell<FxHashMap<Rc<str>, Rc<Shape>>>,
}

thread_local! {
    static EMPTY: Rc<Shape> = Rc::new(Shape::new(vec![], 0));
}

impl Shape {
    pub fn new(names: Vec<Rc<str>>, declared: usize) -> Shape {
        let index = names.iter().enumerate().map(|(i, n)| (n.clone(), i as u32)).collect();
        Shape { names, index, declared, transitions: RefCell::new(FxHashMap::default()) }
    }

    /// Shape for a class's declared properties.
    pub fn declared(names: Vec<Rc<str>>) -> Rc<Shape> {
        let n = names.len();
        Rc::new(Shape::new(names, n))
    }

    #[inline]
    pub fn slot(&self, name: &str) -> Option<usize> {
        if self.names.len() <= 8 {
            // Small shapes: a linear scan beats hashing.
            return self.names.iter().position(|n| &**n == name);
        }
        self.index.get(name).map(|&i| i as usize)
    }

    fn child(self: &Rc<Shape>, name: Rc<str>) -> Rc<Shape> {
        if let Some(s) = self.transitions.borrow().get(&name) {
            return s.clone();
        }
        let mut names = self.names.clone();
        names.push(name.clone());
        let s = Rc::new(Shape::new(names, self.declared));
        self.transitions.borrow_mut().insert(name, s.clone());
        s
    }
}

#[derive(Clone)]
pub struct Props {
    shape: Rc<Shape>,
    vals: Vec<Option<Value>>,
}

impl Default for Props {
    fn default() -> Self {
        Props { shape: EMPTY.with(Rc::clone), vals: vec![] }
    }
}

impl Props {
    /// Instance storage for `shape` with these initial values (None =
    /// uninitialized).
    pub fn with_shape(shape: Rc<Shape>, vals: Vec<Option<Value>>) -> Props {
        debug_assert_eq!(shape.names.len(), vals.len());
        Props { shape, vals }
    }

    pub fn shape(&self) -> &Rc<Shape> {
        &self.shape
    }

    #[inline]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.shape.slot(name).and_then(|i| self.vals[i].as_ref())
    }

    #[inline]
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Value> {
        let i = self.shape.slot(name)?;
        self.vals[i].as_mut()
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Value in slot `i` with its name (None if the slot is empty).
    #[inline]
    pub fn get_index(&self, i: usize) -> Option<(&Rc<str>, &Value)> {
        let v = self.vals.get(i)?.as_ref()?;
        Some((&self.shape.names[i], v))
    }

    #[inline]
    pub fn get_index_mut(&mut self, i: usize) -> Option<&mut Value> {
        self.vals.get_mut(i)?.as_mut()
    }

    pub fn get_full(&self, name: &str) -> Option<(usize, &Rc<str>, &Value)> {
        let i = self.shape.slot(name)?;
        let v = self.vals[i].as_ref()?;
        Some((i, &self.shape.names[i], v))
    }

    /// Set a property, adding it (at the end) if the shape lacks it.
    pub fn insert(&mut self, name: Rc<str>, v: Value) -> Option<Value> {
        match self.shape.slot(&name) {
            Some(i) => self.vals[i].replace(v),
            None => {
                self.shape = self.shape.child(name);
                self.vals.push(Some(v));
                None
            }
        }
    }

    /// The property's value, created as null if absent (for references).
    pub fn slot_or_default(&mut self, name: Rc<str>) -> &mut Value {
        let i = match self.shape.slot(&name) {
            Some(i) => i,
            None => {
                self.insert(name, Value::Null);
                self.vals.len() - 1
            }
        };
        self.vals[i].get_or_insert(Value::Null)
    }

    /// `unset($o->p)`.
    pub fn shift_remove(&mut self, name: &str) -> Option<Value> {
        let i = self.shape.slot(name)?;
        let old = self.vals[i].take();
        old.as_ref()?;
        if i >= self.shape.declared {
            // Dynamic property: drop it from this object's shape so a later
            // assignment appends it at the end, like PHP. The object gets a
            // private shape (rare path).
            let mut names = self.shape.names.clone();
            names.remove(i);
            self.vals.remove(i);
            self.shape = Rc::new(Shape::new(names, self.shape.declared));
        }
        old
    }

    pub fn len(&self) -> usize {
        self.vals.iter().filter(|v| v.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.vals.iter().all(Option::is_none)
    }

    /// Present properties in order.
    pub fn iter(&self) -> impl Iterator<Item = (&Rc<str>, &Value)> + '_ {
        self.shape.names.iter().zip(self.vals.iter()).filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Rc<str>, &mut Value)> + '_ {
        self.shape.names.iter().zip(self.vals.iter_mut()).filter_map(|(k, v)| v.as_mut().map(|v| (k, v)))
    }

    pub fn keys(&self) -> impl Iterator<Item = &Rc<str>> + '_ {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> + '_ {
        self.vals.iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compact_sizes() {
        // Guard against layout regressions: objects must stay small.
        assert!(std::mem::size_of::<crate::value::Obj>() <= 96);
        assert_eq!(std::mem::size_of::<Option<crate::value::Value>>(), 16);
    }
}
