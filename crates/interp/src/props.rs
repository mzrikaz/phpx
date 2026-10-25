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

/// Properties named with this prefix hold built-in classes' internal state
/// (iterator positions, flags, ...): they work like any property but are
/// left out wherever PHP code can enumerate properties (dumps, casts,
/// `foreach`, serialization, comparison), like PHP's C-level state.
pub const HIDDEN_PREFIX: &str = "__phpx_";

pub struct Shape {
    pub names: Vec<Rc<str>>,
    /// Some name has the hidden prefix (enumeration filters only then).
    hidden: bool,
    index: FxHashMap<Rc<str>, u32>,
    /// Shared shapes live as long as their root (a class, or the empty
    /// shape) and may be cached by address in generated code. Private
    /// ("dictionary mode") shapes belong to one object and are never cached.
    pub shared: bool,
    /// The first `declared` names are class-declared properties: unsetting
    /// one keeps its slot (it returns to its declared position when set
    /// again, as in PHP). Later names are dynamic.
    declared: usize,
    transitions: RefCell<FxHashMap<Rc<str>, Rc<Shape>>>,
    /// Declared slots emptied by `unset()` (private shapes only): unlike
    /// never-initialized typed properties, these reach __get/__set/__isset
    /// (PHP's IS_PROP_UNINIT flag, inverted).
    unset: Vec<u32>,
}

/// Beyond this many properties, or this many distinct successor shapes,
/// objects get private shapes (e.g. stdClass used as a dictionary).
const MAX_SHARED_PROPS: usize = 64;
const MAX_TRANSITIONS: usize = 64;

thread_local! {
    static EMPTY: Rc<Shape> = Rc::new(Shape::new(vec![], 0, true));
}

impl Shape {
    pub fn new(names: Vec<Rc<str>>, declared: usize, shared: bool) -> Shape {
        let index = names.iter().enumerate().map(|(i, n)| (n.clone(), i as u32)).collect();
        let hidden = names.iter().any(|n| n.starts_with(HIDDEN_PREFIX));
        Shape { names, index, hidden, declared, shared, transitions: RefCell::new(FxHashMap::default()), unset: vec![] }
    }

    /// Shape for a class's declared properties (lives with the class).
    pub fn declared(names: Vec<Rc<str>>) -> Rc<Shape> {
        let n = names.len();
        Rc::new(Shape::new(names, n, true))
    }

    #[inline]
    pub fn slot(&self, name: &str) -> Option<usize> {
        if self.names.len() <= 8 {
            // Small shapes: a linear scan beats hashing.
            return self.names.iter().position(|n| &**n == name);
        }
        self.index.get(name).map(|&i| i as usize)
    }

    /// Shared child shape with `name` appended, or None when the tree
    /// should stop growing (many properties or many different successors:
    /// the object switches to a private shape).
    fn child(self: &Rc<Shape>, name: &Rc<str>) -> Option<Rc<Shape>> {
        if let Some(s) = self.transitions.borrow().get(name) {
            return Some(s.clone());
        }
        if self.names.len() >= MAX_SHARED_PROPS || self.transitions.borrow().len() >= MAX_TRANSITIONS {
            return None;
        }
        let mut names = self.names.clone();
        names.push(name.clone());
        let s = Rc::new(Shape::new(names, self.declared, true));
        self.transitions.borrow_mut().insert(name.clone(), s.clone());
        Some(s)
    }

    fn push_private(&mut self, name: Rc<str>) {
        self.hidden |= name.starts_with(HIDDEN_PREFIX);
        self.index.insert(name.clone(), self.names.len() as u32);
        self.names.push(name);
    }

    fn private_copy(&self) -> Shape {
        let mut s = Shape::new(self.names.clone(), self.declared, false);
        s.unset = self.unset.clone();
        s
    }
}

/// Property slots: a vector in two words (pointer, then u32 length and
/// capacity) instead of `Vec`'s three, with the pointer first so generated
/// code can read it at a fixed offset.
#[repr(C)]
pub struct Slots {
    ptr: std::ptr::NonNull<Option<Value>>,
    len: u32,
    cap: u32,
}

impl Slots {
    const fn new() -> Slots {
        Slots { ptr: std::ptr::NonNull::dangling(), len: 0, cap: 0 }
    }

    fn layout(cap: u32) -> std::alloc::Layout {
        std::alloc::Layout::array::<Option<Value>>(cap as usize).expect("too many properties")
    }

    fn from_vec(v: Vec<Option<Value>>) -> Slots {
        if v.capacity() == 0 {
            return Slots::new();
        }
        // Same layout as ours (an array of `cap` slots): take the buffer.
        let mut v = std::mem::ManuallyDrop::new(v);
        let (len, cap) = (v.len() as u32, v.capacity() as u32);
        Slots { ptr: std::ptr::NonNull::new(v.as_mut_ptr()).unwrap(), len, cap }
    }

    #[inline(always)]
    fn from_slice(s: &[Option<Value>]) -> Slots {
        if s.is_empty() {
            return Slots::new();
        }
        let mut n = Slots::with_capacity(s.len() as u32);
        for (i, v) in s.iter().enumerate() {
            // SAFETY: capacity reserved; len updated as each slot is written.
            unsafe { n.ptr.as_ptr().add(i).write(v.clone()) };
            n.len += 1;
        }
        n
    }

    #[inline(always)]
    fn with_capacity(cap: u32) -> Slots {
        // SAFETY: non-zero size.
        let p = unsafe { std::alloc::alloc(Slots::layout(cap)) } as *mut Option<Value>;
        let Some(ptr) = std::ptr::NonNull::new(p) else { std::alloc::handle_alloc_error(Slots::layout(cap)) };
        Slots { ptr, len: 0, cap }
    }

    fn push(&mut self, v: Option<Value>) {
        if self.len == self.cap {
            let cap = (self.cap * 2).max(4);
            // SAFETY: grow the buffer we own (or allocate the first one).
            let p = unsafe {
                if self.cap == 0 {
                    std::alloc::alloc(Slots::layout(cap))
                } else {
                    std::alloc::realloc(self.ptr.as_ptr() as *mut u8, Slots::layout(self.cap), Slots::layout(cap).size())
                }
            } as *mut Option<Value>;
            self.ptr = std::ptr::NonNull::new(p).unwrap_or_else(|| std::alloc::handle_alloc_error(Slots::layout(cap)));
            self.cap = cap;
        }
        // SAFETY: len < cap.
        unsafe { self.ptr.as_ptr().add(self.len as usize).write(v) };
        self.len += 1;
    }

    fn remove(&mut self, i: usize) -> Option<Value> {
        let len = self.len as usize;
        assert!(i < len);
        // SAFETY: i in bounds; the tail shifts down over the removed slot.
        unsafe {
            let p = self.ptr.as_ptr().add(i);
            let v = p.read();
            std::ptr::copy(p.add(1), p, len - i - 1);
            self.len -= 1;
            v
        }
    }
}

impl std::ops::Deref for Slots {
    type Target = [Option<Value>];
    #[inline]
    fn deref(&self) -> &[Option<Value>] {
        // SAFETY: len slots are initialized.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len as usize) }
    }
}

impl std::ops::DerefMut for Slots {
    #[inline]
    fn deref_mut(&mut self) -> &mut [Option<Value>] {
        // SAFETY: as above, uniquely borrowed.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len as usize) }
    }
}

impl Drop for Slots {
    fn drop(&mut self) {
        // SAFETY: drop the initialized slots, then the buffer we own.
        unsafe {
            std::ptr::drop_in_place(&mut **self as *mut [Option<Value>]);
            if self.cap > 0 {
                std::alloc::dealloc(self.ptr.as_ptr() as *mut u8, Slots::layout(self.cap));
            }
        }
    }
}

/// `#[repr(C)]`: generated code reads `shape` (offset 0) and the slot
/// pointer (offset 8) directly for inline-cached property access.
#[repr(C)]
pub struct Props {
    shape: Rc<Shape>,
    vals: Slots,
}

impl Clone for Props {
    fn clone(&self) -> Self {
        Props { shape: self.shape.clone(), vals: Slots::from_slice(&self.vals) }
    }
}

impl Default for Props {
    fn default() -> Self {
        Props { shape: EMPTY.with(Rc::clone), vals: Slots::new() }
    }
}

impl Props {
    /// Instance storage for `shape` with these initial values (None =
    /// uninitialized).
    pub fn with_shape(shape: Rc<Shape>, vals: Vec<Option<Value>>) -> Props {
        // Generated code indexes the slots by shape position.
        assert_eq!(shape.names.len(), vals.len());
        Props { shape, vals: Slots::from_vec(vals) }
    }

    /// Like `with_shape`, cloning the values from a template.
    #[inline(always)]
    pub fn from_template(shape: Rc<Shape>, vals: &[Option<Value>]) -> Props {
        debug_assert_eq!(shape.names.len(), vals.len());
        Props { shape, vals: Slots::from_slice(vals) }
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
                if self.shape.shared {
                    self.shape = match self.shape.child(&name) {
                        Some(c) => c,
                        None => {
                            let mut p = self.shape.private_copy();
                            p.push_private(name);
                            Rc::new(p)
                        }
                    };
                } else {
                    match Rc::get_mut(&mut self.shape) {
                        Some(sh) => sh.push_private(name),
                        None => {
                            let mut p = self.shape.private_copy();
                            p.push_private(name);
                            self.shape = Rc::new(p);
                        }
                    }
                }
                self.vals.push(Some(v));
                None
            }
        }
    }

    /// Set a property the shape already has a slot for (writing through a
    /// reference); gives the value back if there's none.
    #[inline]
    pub fn set_existing(&mut self, name: &str, v: Value) -> Result<(), Value> {
        let Some(i) = self.shape.slot(name) else { return Err(v) };
        match &mut self.vals[i] {
            Some(Value::Ref(r)) => *r.borrow_mut() = v,
            s => *s = Some(v),
        }
        Ok(())
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

    /// A declared property that was never initialized (typed, no default)
    /// and not unset since: reads fail and magic methods are bypassed.
    pub fn is_uninit(&self, name: &str) -> bool {
        self.shape.slot(name).is_some_and(|i| {
            i < self.shape.declared && self.vals[i].is_none() && !self.shape.unset.contains(&(i as u32))
        })
    }

    /// `unset()` of an uninitialized declared property: from now on it is
    /// unset (magic methods apply).
    pub fn mark_unset(&mut self, name: &str) {
        if let Some(i) = self.shape.slot(name).filter(|&i| i < self.shape.declared) {
            self.note_unset(i);
        }
    }

    /// Record declared slot `i` as unset: the object moves to a private
    /// shape carrying the mark (unset() of declared properties is rare).
    fn note_unset(&mut self, i: usize) {
        if self.shape.unset.contains(&(i as u32)) {
            return;
        }
        let mut p = self.shape.private_copy();
        p.unset.push(i as u32);
        self.shape = Rc::new(p);
    }

    /// `unset($o->p)`.
    pub fn shift_remove(&mut self, name: &str) -> Option<Value> {
        let i = self.shape.slot(name)?;
        let old = self.vals[i].take();
        old.as_ref()?;
        if i < self.shape.declared {
            self.note_unset(i);
        }
        if i >= self.shape.declared {
            // Dynamic property: drop it from this object's shape so a later
            // assignment appends it at the end, like PHP. The object gets a
            // private shape (rare path).
            let mut names = self.shape.names.clone();
            names.remove(i);
            self.vals.remove(i);
            let mut sh = Shape::new(names, self.shape.declared, false);
            sh.unset = self.shape.unset.clone();
            self.shape = Rc::new(sh);
        }
        old
    }

    pub fn len(&self) -> usize {
        self.iter().count()
    }

    pub fn is_empty(&self) -> bool {
        self.iter().next().is_none()
    }

    /// Present properties in order (without hidden ones).
    pub fn iter(&self) -> impl Iterator<Item = (&Rc<str>, &Value)> + '_ {
        let hidden = self.shape.hidden;
        self.shape
            .names
            .iter()
            .zip(self.vals.iter())
            .filter_map(move |(k, v)| v.as_ref().filter(|_| !hidden || !k.starts_with(HIDDEN_PREFIX)).map(|v| (k, v)))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Rc<str>, &mut Value)> + '_ {
        let hidden = self.shape.hidden;
        self.shape
            .names
            .iter()
            .zip(self.vals.iter_mut())
            .filter_map(move |(k, v)| v.as_mut().filter(|_| !hidden || !k.starts_with(HIDDEN_PREFIX)).map(|v| (k, v)))
    }

    pub fn keys(&self) -> impl Iterator<Item = &Rc<str>> + '_ {
        self.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Value> + '_ {
        self.iter().map(|(_, v)| v)
    }

    /// Drop every value now (the slots stay, empty).
    pub fn drop_values(&mut self) {
        for v in self.vals.iter_mut() {
            drop(v.take());
        }
    }

    /// Every value, hidden ones included (for the cycle collector).
    pub fn all_values(&self) -> impl Iterator<Item = &Value> + '_ {
        self.vals.iter().flatten()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compact_sizes() {
        // Guard against layout regressions: objects must stay small.
        assert!(std::mem::size_of::<crate::value::Obj>() <= 48);
        assert_eq!(std::mem::size_of::<Option<crate::value::Value>>(), 16);
        let mut p = super::Props::default();
        p.insert(std::rc::Rc::from("a"), crate::value::Value::Int(1));
        p.insert(std::rc::Rc::from("b"), crate::value::Value::Int(2));
        let q = p.clone();
        assert_eq!(q.len(), 2);
        assert_eq!(std::mem::size_of::<super::Props>(), 24);
        p.shift_remove("a");
        assert_eq!(p.keys().map(|k| &**k).collect::<Vec<_>>(), ["b"]);
    }
}

