//! Runtime values with PHP 8 semantics: byte strings, copy-on-write ordered
//! arrays, reference-counted objects.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::class::Class;

/// PHP strings are byte strings, not UTF-8.
pub use crate::pstr::PStr;

pub fn pstr(s: &str) -> PStr {
    PStr::from_bytes(s.as_bytes())
}

pub fn pbytes(b: &[u8]) -> PStr {
    PStr::from_bytes(b)
}

/// A variable's storage cell. Two names (or an array element and a name)
/// that are references to each other share one cell.
pub type Slot = Rc<RefCell<Value>>;

/// Layout is fixed (`repr(C, u8)`): tag byte at offset 0, payload at 8,
/// 16 bytes total. Native code relies on it (see `tag` below).
#[derive(Clone, Default)]
#[repr(C, u8)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(PStr),
    Array(Rc<PhpArray>),
    Object(Rc<Obj>),
    /// A reference stored inside an array element or property (`$a[0] = &$x`,
    /// `foreach ($a as &$v)`). Never stored directly in a variable cell, and
    /// never returned from reads: array/property accessors dereference it.
    Ref(Slot),
    /// `resource` (streams, directory handles, stream contexts).
    Resource(Rc<Resource>),
}

/// A PHP resource: an id, a type name (`get_resource_type()`), and native
/// state. Closing sets the type to "Unknown" and drops the state.
pub struct Resource {
    pub id: i64,
    pub kind: Cell<&'static str>,
    pub data: RefCell<Option<Box<dyn std::any::Any>>>,
}

thread_local! {
    static NEXT_RES_ID: Cell<i64> = const { Cell::new(1) };
}

impl Resource {
    pub fn new(kind: &'static str, data: Box<dyn std::any::Any>) -> Value {
        // PHP's STDIN/STDOUT/STDERR are 1..3.
        let id = NEXT_RES_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        Value::Resource(Rc::new(Resource { id, kind: Cell::new(kind), data: RefCell::new(Some(data)) }))
    }

    pub fn close(&self) {
        self.kind.set("Unknown");
        self.data.borrow_mut().take();
    }

    pub fn is_open(&self) -> bool {
        self.data.borrow().is_some()
    }
}

/// Tag bytes of `Value` variants, for generated code.
pub mod tag {
    pub const NULL: u8 = 0;
    pub const BOOL: u8 = 1;
    pub const INT: u8 = 2;
    pub const FLOAT: u8 = 3;
    pub const STR: u8 = 4;
    pub const ARRAY: u8 = 5;
    pub const OBJECT: u8 = 6;
    pub const REF: u8 = 7;
    pub const RESOURCE: u8 = 8;
    /// Payload offset.
    pub const PAYLOAD: i32 = 8;
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Int(i64),
    Str(PStr),
}

impl Key {
    /// String keys that look like canonical decimal ints become int keys.
    pub fn from_bytes(b: &[u8]) -> Key {
        match canonical_int(b) {
            Some(n) => Key::Int(n),
            None => Key::Str(pbytes(b)),
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Key::Int(n) => Value::Int(*n),
            Key::Str(s) => Value::Str(s.clone()),
        }
    }
}

/// "123" and "-5" yes; "0123", "-0", "1.0", " 1" no.
fn canonical_int(b: &[u8]) -> Option<i64> {
    let digits = b.strip_prefix(b"-").unwrap_or(b);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    if (digits[0] == b'0' && digits.len() > 1) || b == b"-0" {
        return None;
    }
    std::str::from_utf8(b).ok()?.parse().ok()
}

/// Insertion-ordered map with a fast (non-cryptographic) hasher; PHP
/// arrays and property tables don't need DoS-resistant hashing per lookup.
pub type FxIndexMap<K, V> = IndexMap<K, V, std::hash::BuildHasherDefault<rustc_hash::FxHasher>>;

/// PHP array. Like PHP, a list-shaped array (keys exactly 0..len in
/// order) is stored "packed" as a plain vector: no hashing, 16 bytes per
/// element. Anything else is an insertion-ordered hash map. A packed array
/// converts to a hash the first time a key breaks the shape.
/// `#[repr(C)]`: generated code reads `pk` (offset 0) and `pk_len` (offset
/// 8) to index packed arrays without a call.
#[repr(C)]
pub struct PhpArray {
    /// Packed storage's buffer and length, mirrored after every change
    /// (null when the array is a hash).
    pk: *mut Value,
    pk_len: usize,
    /// Element count (packed or hash), also mirrored: `count()` inline.
    count: usize,
    store: Store,
    next_free: i64,
    /// Internal pointer (current()/next()/reset()), as an index.
    pub pos: Cell<usize>,
}

enum Store {
    Packed(Vec<Value>),
    Hash(FxIndexMap<Key, Value>),
}

/// One of two iterator types (packed or hash storage).
#[derive(Clone)]
pub enum Either<A, B> {
    L(A),
    R(B),
}

impl<T, A: Iterator<Item = T>, B: Iterator<Item = T>> Iterator for Either<A, B> {
    type Item = T;
    #[inline]
    fn next(&mut self) -> Option<T> {
        match self {
            Either::L(a) => a.next(),
            Either::R(b) => b.next(),
        }
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Either::L(a) => a.size_hint(),
            Either::R(b) => b.size_hint(),
        }
    }
}

impl<T, A: DoubleEndedIterator<Item = T>, B: DoubleEndedIterator<Item = T>> DoubleEndedIterator for Either<A, B> {
    fn next_back(&mut self) -> Option<T> {
        match self {
            Either::L(a) => a.next_back(),
            Either::R(b) => b.next_back(),
        }
    }
}

impl<T, A: ExactSizeIterator<Item = T>, B: ExactSizeIterator<Item = T>> ExactSizeIterator for Either<A, B> {}

/// Copy-on-write element copy: a reference only this array still holds
/// (refcount 1) becomes a plain value, like PHP.
fn copy_elem(v: &Value) -> Value {
    match v {
        Value::Ref(s) if Rc::strong_count(s) == 1 => s.borrow().clone(),
        other => other.clone(),
    }
}

impl Clone for PhpArray {
    fn clone(&self) -> Self {
        let store = match &self.store {
            Store::Packed(v) => Store::Packed(v.iter().map(copy_elem).collect()),
            Store::Hash(m) => Store::Hash(m.iter().map(|(k, v)| (k.clone(), copy_elem(v))).collect()),
        };
        PhpArray::make(store, self.next_free, self.pos.get())
    }
}

impl Default for PhpArray {
    fn default() -> Self {
        PhpArray::make(Store::Packed(Vec::new()), i64::MIN, 0)
    }
}

impl PhpArray {
    pub fn new() -> Self {
        Self::default()
    }

    fn make(store: Store, next_free: i64, pos: usize) -> Self {
        let mut a = PhpArray { pk: std::ptr::null_mut(), pk_len: 0, count: 0, store, next_free, pos: Cell::new(pos) };
        a.sync();
        a
    }

    /// Refresh the packed mirror; called after anything that can change the
    /// storage's shape, length or buffer.
    #[inline]
    fn sync(&mut self) {
        match &mut self.store {
            Store::Packed(v) => {
                self.pk = v.as_mut_ptr();
                self.pk_len = v.len();
                self.count = v.len();
            }
            Store::Hash(m) => {
                self.pk = std::ptr::null_mut();
                self.pk_len = 0;
                self.count = m.len();
            }
        }
    }

    pub fn with_capacity(n: usize) -> Self {
        PhpArray::make(Store::Packed(Vec::with_capacity(n)), i64::MIN, 0)
    }

    pub fn from_list(vals: impl IntoIterator<Item = Value>) -> Self {
        let v: Vec<Value> = vals.into_iter().collect();
        let n = v.len() as i64;
        PhpArray::make(Store::Packed(v), if n == 0 { i64::MIN } else { n }, 0)
    }

    pub fn len(&self) -> usize {
        match &self.store {
            Store::Packed(v) => v.len(),
            Store::Hash(m) => m.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn to_hash(&mut self) -> &mut FxIndexMap<Key, Value> {
        if let Store::Packed(v) = &mut self.store {
            let m: FxIndexMap<Key, Value> = std::mem::take(v).into_iter().enumerate().map(|(i, x)| (Key::Int(i as i64), x)).collect();
            self.store = Store::Hash(m);
        }
        match &mut self.store {
            Store::Hash(m) => m,
            Store::Packed(_) => unreachable!(),
        }
    }

    #[inline]
    fn packed_index(len: usize, k: &Key) -> Option<usize> {
        match k {
            Key::Int(n) if *n >= 0 && (*n as usize) < len => Some(*n as usize),
            _ => None,
        }
    }

    /// Element value (references dereferenced).
    pub fn get(&self, k: &Key) -> Option<Value> {
        match &self.store {
            Store::Packed(v) => Self::packed_index(v.len(), k).map(|i| v[i].deref()),
            Store::Hash(m) => m.get(k).map(Value::deref),
        }
    }

    pub fn contains(&self, k: &Key) -> bool {
        match &self.store {
            Store::Packed(v) => Self::packed_index(v.len(), k).is_some(),
            Store::Hash(m) => m.contains_key(k),
        }
    }

    /// Raw element storage, for writes (may be a `Value::Ref`).
    pub fn get_mut(&mut self, k: &Key) -> Option<&mut Value> {
        match &mut self.store {
            Store::Packed(v) => Self::packed_index(v.len(), k).map(|i| &mut v[i]),
            Store::Hash(m) => m.get_mut(k),
        }
    }

    pub fn get_index(&self, i: usize) -> Option<(Key, Value)> {
        match &self.store {
            Store::Packed(v) => v.get(i).map(|x| (Key::Int(i as i64), x.deref())),
            Store::Hash(m) => m.get_index(i).map(|(k, v)| (k.clone(), v.deref())),
        }
    }

    pub fn get_index_raw_mut(&mut self, i: usize) -> Option<(Key, &mut Value)> {
        match &mut self.store {
            Store::Packed(v) => v.get_mut(i).map(|x| (Key::Int(i as i64), x)),
            Store::Hash(m) => m.get_index_mut(i).map(|(k, v)| (k.clone(), v)),
        }
    }

    pub fn keys(&self) -> impl DoubleEndedIterator<Item = Key> + ExactSizeIterator + '_ {
        match &self.store {
            Store::Packed(v) => Either::L((0..v.len()).map(|i| Key::Int(i as i64))),
            Store::Hash(m) => Either::R(m.keys().cloned()),
        }
    }

    /// array_pop(): like PHP, the next free index drops back if the popped
    /// key was the last one handed out.
    pub fn pop(&mut self) -> Option<Value> {
        let r = self.pop_inner();
        self.sync();
        r
    }

    fn pop_inner(&mut self) -> Option<Value> {
        let (k, v) = match &mut self.store {
            Store::Packed(v) => {
                let x = v.pop()?;
                (Key::Int(v.len() as i64), x)
            }
            Store::Hash(m) => m.pop()?,
        };
        if let Key::Int(n) = k {
            if self.next_free != i64::MIN && n == self.next_free - 1 {
                self.next_free = n;
            }
        }
        Some(v.into_deref())
    }

    /// Owned raw pairs: references stay references (sorting moves them).
    pub fn into_raw_pairs(self) -> Vec<(Key, Value)> {
        match self.store {
            Store::Packed(v) => v.into_iter().enumerate().map(|(i, x)| (Key::Int(i as i64), x)).collect(),
            Store::Hash(m) => m.into_iter().collect(),
        }
    }

    /// Owned (key, value) pairs, references dereferenced.
    pub fn into_pairs(self) -> Vec<(Key, Value)> {
        self.into_raw_pairs().into_iter().map(|(k, v)| (k, v.into_deref())).collect()
    }

    pub fn insert(&mut self, k: Key, v: Value) {
        self.insert_inner(k, v);
        self.sync();
    }

    fn insert_inner(&mut self, k: Key, v: Value) {
        if let Key::Int(n) = k {
            self.bump_next(n);
        }
        if let Store::Packed(vec) = &mut self.store {
            match k {
                Key::Int(n) if n >= 0 && (n as usize) < vec.len() => {
                    vec[n as usize] = v;
                    return;
                }
                Key::Int(n) if n >= 0 && n as usize == vec.len() => {
                    vec.push(v);
                    return;
                }
                _ => {}
            }
        }
        self.to_hash().insert(k, v);
    }

    fn bump_next(&mut self, n: i64) {
        if self.next_free == i64::MIN || n >= self.next_free {
            self.next_free = n.saturating_add(1);
        }
    }

    pub fn next_index(&self) -> i64 {
        if self.next_free == i64::MIN { 0 } else { self.next_free }
    }

    pub fn push(&mut self, v: Value) -> Key {
        let n = self.next_index();
        if let Store::Packed(vec) = &mut self.store {
            if n as usize == vec.len() {
                vec.push(v);
                self.next_free = n + 1;
                self.sync();
                return Key::Int(n);
            }
        }
        self.insert(Key::Int(n), v);
        Key::Int(n)
    }

    /// Entry for `$a[k]` writes, created as null if missing.
    pub fn entry(&mut self, k: Key) -> &mut Value {
        let p: *mut Value = self.entry_inner(k);
        self.sync();
        // SAFETY: sync() only reads the storage; the element didn't move.
        unsafe { &mut *p }
    }

    fn entry_inner(&mut self, k: Key) -> &mut Value {
        if let Key::Int(n) = k {
            self.bump_next(n);
        }
        if let Store::Packed(vec) = &mut self.store {
            match k {
                Key::Int(n) if n >= 0 && (n as usize) <= vec.len() => {
                    if n as usize == vec.len() {
                        vec.push(Value::Null);
                    }
                    // Re-borrow to satisfy the borrow checker.
                    let Store::Packed(vec) = &mut self.store else { unreachable!() };
                    return &mut vec[n as usize];
                }
                _ => {}
            }
        }
        self.to_hash().entry(k).or_default()
    }

    pub fn remove(&mut self, k: &Key) -> Option<Value> {
        let r = self.remove_inner(k);
        self.sync();
        r
    }

    fn remove_inner(&mut self, k: &Key) -> Option<Value> {
        if let Store::Packed(vec) = &mut self.store {
            match Self::packed_index(vec.len(), k) {
                None => return None,
                Some(i) if i + 1 == vec.len() => return vec.pop(),
                Some(_) => {}
            }
        }
        // TODO: O(n). Use tombstones + compaction like PHP's HashTable.
        self.to_hash().shift_remove(k)
    }

    /// (key, value) pairs in order, references dereferenced.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (Key, Value)> + ExactSizeIterator + '_ {
        match &self.store {
            Store::Packed(v) => Either::L(v.iter().enumerate().map(|(i, x)| (Key::Int(i as i64), x.deref()))),
            Store::Hash(m) => Either::R(m.iter().map(|(k, v)| (k.clone(), v.deref()))),
        }
    }

    /// Raw storage, references included (for var_dump's `&` marker).
    /// Stored values (references not dereferenced), for the cycle collector.
    pub fn raw_values(&self) -> Either<std::slice::Iter<'_, Value>, indexmap::map::Values<'_, Key, Value>> {
        match &self.store {
            Store::Packed(v) => Either::L(v.iter()),
            Store::Hash(m) => Either::R(m.values()),
        }
    }

    pub fn iter_raw(&self) -> impl DoubleEndedIterator<Item = (Key, &Value)> + ExactSizeIterator + '_ {
        match &self.store {
            Store::Packed(v) => Either::L(v.iter().enumerate().map(|(i, x)| (Key::Int(i as i64), x))),
            Store::Hash(m) => Either::R(m.iter().map(|(k, v)| (k.clone(), v))),
        }
    }

    pub fn values(&self) -> impl DoubleEndedIterator<Item = Value> + ExactSizeIterator + Clone + '_ {
        match &self.store {
            Store::Packed(v) => Either::L(v.iter().map(Value::deref)),
            Store::Hash(m) => Either::R(m.values().map(Value::deref)),
        }
    }

    pub fn is_list(&self) -> bool {
        match &self.store {
            Store::Packed(_) => true,
            Store::Hash(m) => m.keys().enumerate().all(|(i, k)| *k == Key::Int(i as i64)),
        }
    }

    /// Re-number int keys from 0 (used by array_values-like functions).
    pub fn reset_next(&mut self) {
        self.next_free = i64::MIN;
        let max = self.keys().filter_map(|k| if let Key::Int(n) = k { Some(n) } else { None }).max();
        if let Some(m) = max {
            self.bump_next(m);
        }
    }
}

// ---- objects ---------------------------------------------------------------


/// Objects whose last reference died and that have a `__destruct`: the
/// interpreter runs them at the next statement boundary (where an
/// exception from a destructor can propagate). DESTRUCT_PENDING is a plain
/// global so compiled code can test it with one load.
pub static DESTRUCT_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set once any namespaced user function has a built-in's name; compiled
/// code then stops inlining built-ins (`ns\strlen` could take precedence).
pub static BUILTIN_SHADOWED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

thread_local! {
    pub static DESTRUCT_QUEUE: RefCell<Vec<Rc<Obj>>> = const { RefCell::new(Vec::new()) };
}


pub struct Obj {
    pub id: u32,
    /// Slot in the cycle collector's registry (low 30 bits; all ones: not
    /// registered), "has native state" (bit 30) and "`__destruct` already
    /// ran (or was queued)" (bit 31).
    gc: Cell<u32>,
    pub class: Rc<Class>,
    /// Initialized properties in declaration order. Typed properties without
    /// a default are absent until assigned.
    pub props: RefCell<crate::props::Props>,
}

const GC_NONE: u32 = 0x3FFF_FFFF;
const HAS_INTERNAL: u32 = 1 << 30;
const DESTRUCTED: u32 = 1 << 31;

thread_local! {
    /// What objects without native state read (never borrowed mutably).
    static NO_INTERNAL: &'static RefCell<Internal> = Box::leak(Box::new(RefCell::new(Internal::None)));
    /// Native state (closures, generators, ...) by object handle: few
    /// objects have any, so it lives here rather than in every object.
    /// Boxed: entries keep their address while the map changes.
    static INTERNALS: RefCell<rustc_hash::FxHashMap<u32, Box<RefCell<Internal>>>> = RefCell::new(Default::default());
}

impl Obj {
    pub fn new(class: Rc<Class>, internal: Internal) -> Rc<Obj> {
        Obj::with_props(class, internal, Default::default())
    }

    /// New object with its initial property storage (avoids building an
    /// empty one first).
    pub fn with_props(class: Rc<Class>, internal: Internal, props: crate::props::Props) -> Rc<Obj> {
        crate::gc::track(|id| {
            let flags = match internal {
                Internal::None => 0,
                i => {
                    INTERNALS.with(|m| m.borrow_mut().insert(id, Box::new(RefCell::new(i))));
                    HAS_INTERNAL
                }
            };
            Rc::new(Obj { id, gc: Cell::new(GC_NONE | flags), class, props: RefCell::new(props) })
        })
    }

    fn internal_cell(&self) -> &RefCell<Internal> {
        if self.gc.get() & HAS_INTERNAL != 0 {
            let p = INTERNALS.with(|m| m.borrow().get(&self.id).map(|b| &**b as *const RefCell<Internal>));
            if let Some(p) = p {
                // SAFETY: the entry is boxed and only removed when this
                // object is dropped, so it outlives `&self`.
                return unsafe { &*p };
            }
        }
        NO_INTERNAL.with(|n| *n)
    }

    /// The object's native state (`Internal::None` for plain objects).
    pub fn internal(&self) -> std::cell::Ref<'_, Internal> {
        self.internal_cell().borrow()
    }

    /// The native state unless it's being changed right now (for the
    /// cycle collector, which can run at any statement boundary).
    pub fn try_internal(&self) -> Option<std::cell::Ref<'_, Internal>> {
        self.internal_cell().try_borrow().ok()
    }

    /// Mutable native state; None for objects without any (or while it's
    /// borrowed).
    pub fn internal_mut(&self) -> Option<std::cell::RefMut<'_, Internal>> {
        if self.gc.get() & HAS_INTERNAL == 0 {
            return None;
        }
        self.internal_cell().try_borrow_mut().ok()
    }

    pub fn gc_index(&self) -> u32 {
        match self.gc.get() & GC_NONE {
            GC_NONE => u32::MAX,
            i => i,
        }
    }

    pub fn set_gc_index(&self, i: u32) {
        let i = if i >= GC_NONE { GC_NONE } else { i };
        self.gc.set((self.gc.get() & !GC_NONE) | i);
    }

    /// `__destruct` already ran (or was queued).
    pub fn destructed(&self) -> bool {
        self.gc.get() & DESTRUCTED != 0
    }

    pub fn set_destructed(&self) {
        self.gc.set(self.gc.get() | DESTRUCTED);
    }

    /// A plain object of `class` with these property values in the class
    /// shape, written straight into its allocation (building it on the
    /// stack and copying it in costs more than the allocation itself).
    pub fn from_template(class: &Rc<Class>, template: &[Option<Value>]) -> Rc<Obj> {
        crate::gc::track(|id| {
            let mut rc = Rc::<Obj>::new_uninit();
            let p = Rc::get_mut(&mut rc).unwrap().as_mut_ptr();
            // SAFETY: every field is written before assume_init.
            unsafe {
                std::ptr::addr_of_mut!((*p).id).write(id);
                std::ptr::addr_of_mut!((*p).gc).write(Cell::new(GC_NONE));
                std::ptr::addr_of_mut!((*p).class).write(class.clone());
                // (Cloned here, not passed in: a Vec handed over by value
                // is reread right after being written, which stalls.)
                std::ptr::addr_of_mut!((*p).props).write(RefCell::new(crate::props::Props::from_template(class.shape.clone(), template)));
                rc.assume_init()
            }
        })
    }

    pub fn get_prop(&self, name: &str) -> Option<Value> {
        self.props.borrow().get(name).map(Value::deref)
    }

    /// Properties in order, references dereferenced.
    pub fn prop_list(&self) -> Vec<(Rc<str>, Value)> {
        self.props.borrow().iter().map(|(k, v)| (k.clone(), v.deref())).collect()
    }

    pub fn set_prop(&self, name: &str, v: Value) {
        let mut p = self.props.borrow_mut();
        // Declared properties already have their slot (declaration order,
        // from the class shape); new ones are appended.
        if let Err(v) = p.set_existing(name, v) {
            p.insert(Rc::from(name), v);
        }
    }
}

impl Obj {
    /// Free the handle and native state (the handle may be reused at once,
    /// so the state goes first).
    fn free(&mut self) {
        if self.gc.get() & HAS_INTERNAL != 0 {
            // Dropped outside the map borrow: it can free other objects.
            let st = INTERNALS.try_with(|m| m.borrow_mut().remove(&self.id)).ok().flatten();
            drop(st);
        }
        crate::gc::release(self, false);
    }
}

impl Drop for Obj {
    fn drop(&mut self) {
        if !self.class.has_destruct || self.destructed() {
            self.free();
            return;
        }
        crate::gc::unregister(self);
        {
            // Resurrect the state into a fresh Rc (same handle, so the same
            // native state) for the destructor; it frees the handle when it
            // dies in turn.
            let o = Rc::new(Obj {
                id: self.id,
                gc: Cell::new(GC_NONE | DESTRUCTED | (self.gc.get() & HAS_INTERNAL)),
                class: self.class.clone(),
                props: RefCell::new(std::mem::take(self.props.get_mut())),
            });
            crate::gc::register(&o);
            if DESTRUCT_QUEUE.try_with(|q| q.borrow_mut().push(o)).is_ok() {
                DESTRUCT_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
            // Couldn't queue (thread ending): `o` drops as destructed and
            // frees the handle and state itself.
            return;
        }
    }
}

/// Native state carried by built-in objects.
#[derive(Default)]
pub enum Internal {
    #[default]
    None,
    Closure(crate::interp::Callable),
    Promise(phpx_runtime::Promise<crate::interp::Outcome>),
    Generator(Rc<crate::generator::GenHandle>),
    /// Native state of built-in classes (HashContext, ...).
    Data(Box<dyn std::any::Any>),
}

// ---- conversions -------------------------------------------------------------

impl Value {
    /// The value itself; for a reference, a copy of what it points to.
    pub fn deref(&self) -> Value {
        match self {
            Value::Ref(s) => s.borrow().clone(),
            v => v.clone(),
        }
    }

    pub fn into_deref(self) -> Value {
        match self {
            Value::Ref(s) => s.borrow().clone(),
            v => v,
        }
    }

    pub fn str(s: &str) -> Value {
        Value::Str(pstr(s))
    }

    pub fn bytes(b: Vec<u8>) -> Value {
        Value::Str(PStr::from(b))
    }

    pub fn array(a: PhpArray) -> Value {
        Value::Array(Rc::new(a))
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Ref(s) => s.borrow().type_name(),
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Resource(_) => "resource",
        }
    }

    /// `get_debug_type()`: class name for objects.
    pub fn debug_type(&self) -> String {
        match self {
            Value::Object(o) => o.class.name.to_string(),
            Value::Resource(r) if r.is_open() => format!("resource ({})", r.kind.get()),
            Value::Resource(_) => "resource (closed)".to_string(),
            v => v.type_name().to_string(),
        }
    }

    /// `gettype()` names.
    pub fn gettype(&self) -> &'static str {
        match self {
            Value::Ref(s) => s.borrow().gettype(),
            Value::Null => "NULL",
            Value::Bool(_) => "boolean",
            Value::Int(_) => "integer",
            Value::Float(_) => "double",
            Value::Str(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
            Value::Resource(r) if r.is_open() => "resource",
            Value::Resource(_) => "resource (closed)",
        }
    }

    pub fn is_null(&self) -> bool {
        match self {
            Value::Ref(s) => s.borrow().is_null(),
            v => matches!(v, Value::Null),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Value::Ref(s) => s.borrow().truthy(),
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(n) => *n != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !(s.is_empty() || &**s == b"0"),
            Value::Array(a) => !a.is_empty(),
            Value::Object(_) | Value::Resource(_) => true,
        }
    }

    /// String conversion for scalars. Arrays/objects are handled by the
    /// interpreter (warnings, `__toString`). Returns None for those.
    pub fn scalar_bytes(&self) -> Option<PStr> {
        Some(match self {
            Value::Null | Value::Bool(false) => crate::value::pbytes(b""),
            Value::Bool(true) => crate::value::pbytes(b"1"),
            Value::Int(n) => pstr(&n.to_string()),
            Value::Float(f) => pstr(&fmt_float_str(*f)),
            Value::Str(s) => s.clone(),
            Value::Array(_) | Value::Object(_) => return None,
            Value::Resource(r) => pstr(&format!("Resource id #{}", r.id)),
            Value::Ref(s) => return s.borrow().scalar_bytes(),
        })
    }

    pub fn as_array(&self) -> Option<&Rc<PhpArray>> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Rc<Obj>> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
}

/// Float for var_dump / var_export / json_encode (serialize_precision = -1):
/// shortest round-trip digits, scientific below 1e-4 and from 1e17.
pub fn fmt_float(f: f64) -> String {
    fmt_float_with(f, 17, true)
}

/// Float to string for echo / concatenation / (string) (precision = 14).
pub fn fmt_float_str(f: f64) -> String {
    fmt_float_with(f, 14, false)
}

fn fmt_float_with(f: f64, precision: i32, shortest: bool) -> String {
    if f.is_nan() {
        return "NAN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let sci = if shortest { format!("{:e}", f.abs()) } else { format!("{:.*e}", (precision - 1) as usize, f.abs()) };
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let sign = if f < 0.0 { "-" } else { "" };
    if exp < -4 || exp >= precision {
        let m = if digits.len() == 1 { format!("{digits}.0") } else { format!("{}.{}", &digits[..1], &digits[1..]) };
        return format!("{sign}{m}E{}{}", if exp < 0 { '-' } else { '+' }, exp.abs());
    }
    if exp >= 0 {
        let int_len = exp as usize + 1;
        if digits.len() <= int_len {
            format!("{sign}{digits}{}", "0".repeat(int_len - digits.len()))
        } else {
            format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
        }
    } else {
        format!("{sign}0.{}{digits}", "0".repeat((-exp - 1) as usize))
    }
}

/// Like `var_export`/`json_encode` float repr: always shows it's a float.
pub fn fmt_float_repr(f: f64) -> String {
    let s = fmt_float(f);
    if f.is_finite() && !s.contains(['.', 'E', 'N', 'I']) { format!("{s}.0") } else { s }
}

/// Result of parsing a numeric string.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

impl Num {
    pub fn to_value(self) -> Value {
        match self {
            Num::Int(n) => Value::Int(n),
            Num::Float(f) => Value::Float(f),
        }
    }

    pub fn as_f64(self) -> f64 {
        match self {
            Num::Int(n) => n as f64,
            Num::Float(f) => f,
        }
    }
}

/// PHP 8 numeric string: optional surrounding whitespace, sign, digits,
/// fraction, exponent. Returns None for non-numeric (and leading-numeric).
pub fn parse_numeric(b: &[u8]) -> Option<Num> {
    let (num, rest) = parse_numeric_prefix(b)?;
    if rest.iter().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0B' | b'\x0C')) {
        Some(num)
    } else {
        None
    }
}

/// Longest numeric prefix (after leading whitespace) and the remaining bytes.
pub fn parse_numeric_prefix(b: &[u8]) -> Option<(Num, &[u8])> {
    let start = b.iter().position(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0B' | b'\x0C')).unwrap_or(b.len());
    let s = &b[start..];
    let mut i = 0;
    if matches!(s.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let int_start = i;
    while s.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut digits = i - int_start;
    let mut is_float = false;
    if s.get(i) == Some(&b'.') {
        let f = i + 1;
        let mut j = f;
        while s.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if digits > 0 || j > f {
            digits += j - f;
            is_float = true;
            i = j;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if s.get(j).is_some_and(u8::is_ascii_digit) {
            while s.get(j).is_some_and(u8::is_ascii_digit) {
                j += 1;
            }
            is_float = true;
            i = j;
        }
    }
    let text = std::str::from_utf8(&s[..i]).ok()?;
    let num = if is_float {
        Num::Float(text.parse().ok()?)
    } else {
        match text.parse::<i64>() {
            Ok(n) => Num::Int(n),
            Err(_) => Num::Float(text.parse().ok()?),
        }
    };
    Some((num, &s[i..]))
}

// ---- comparison ----------------------------------------------------------------

/// `===`
pub fn identical(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Ref(x), _) => identical(&x.borrow(), b),
        (_, Value::Ref(y)) => identical(a, &y.borrow()),
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            Rc::ptr_eq(x, y)
                || (x.len() == y.len()
                    && x.iter().zip(y.iter()).all(|((ka, va), (kb, vb))| ka == kb && identical(&va, &vb)))
        }
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

/// `==`
pub fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Ref(x), _) => loose_eq(&x.borrow(), b),
        (_, Value::Ref(y)) => loose_eq(a, &y.borrow()),
        (Value::Object(x), Value::Object(y)) => {
            Rc::ptr_eq(x, y)
                || (Rc::ptr_eq(&x.class, &y.class) && {
                    let (px, py) = (x.prop_list(), y.prop_list());
                    px.len() == py.len() && px.iter().all(|(k, v)| py.iter().any(|(k2, w)| k == k2 && loose_eq(v, w)))
                })
        }
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(&k).is_some_and(|w| loose_eq(&v, &w)))
        }
        _ => compare(a, b) == Some(Ordering::Equal),
    }
}

/// `<=>` with PHP 8 rules. None = uncomparable (e.g. NAN), treated as "not equal".
pub fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    use Value::*;
    match (a, b) {
        (Ref(x), _) => compare(&x.borrow(), b),
        (_, Ref(y)) => compare(a, &y.borrow()),
        (Resource(x), _) => compare(&Int(x.id), b),
        (_, Resource(y)) => compare(a, &Int(y.id)),
        (Null, Null) => Some(Ordering::Equal),
        // null vs string compares as "" vs string; other null/bool cases as bools.
        (Null, Str(s)) => Some(b"".as_slice().cmp(s.as_slice())),
        (Str(s), Null) => Some(s.as_slice().cmp(b"".as_slice())),
        (Bool(_) | Null, _) | (_, Bool(_) | Null) => Some(a.truthy().cmp(&b.truthy())),
        (Int(x), Int(y)) => Some(x.cmp(y)),
        (Int(x), Float(y)) => (*x as f64).partial_cmp(y),
        (Float(x), Int(y)) => x.partial_cmp(&(*y as f64)),
        (Float(x), Float(y)) => x.partial_cmp(y),
        (Str(x), Str(y)) => match (parse_numeric(x), parse_numeric(y)) {
            (Some(nx), Some(ny)) => cmp_num(nx, ny),
            _ => Some(x.cmp(y).clamp(Ordering::Less, Ordering::Greater)),
        },
        (Int(_) | Float(_), Str(s)) => match parse_numeric(s) {
            Some(n) => cmp_num(num_of(a), n),
            None => Some((*a.scalar_bytes().unwrap()).cmp(s)),
        },
        (Str(s), Int(_) | Float(_)) => match parse_numeric(s) {
            Some(n) => cmp_num(n, num_of(b)),
            None => Some((**s).cmp(&*b.scalar_bytes().unwrap())),
        },
        (Array(x), Array(y)) => {
            if x.len() != y.len() {
                return Some(x.len().cmp(&y.len()));
            }
            for (k, v) in x.iter() {
                let w = y.get(&k)?;
                match compare(&v, &w)? {
                    Ordering::Equal => {}
                    o => return Some(o),
                }
            }
            Some(Ordering::Equal)
        }
        (Array(_), _) => Some(Ordering::Greater),
        (_, Array(_)) => Some(Ordering::Less),
        (Object(_), Object(_)) => if loose_eq(a, b) { Some(Ordering::Equal) } else { None },
        (Object(_), _) => Some(Ordering::Greater),
        (_, Object(_)) => Some(Ordering::Less),
    }
}

fn num_of(v: &Value) -> Num {
    match v {
        Value::Int(n) => Num::Int(*n),
        Value::Float(f) => Num::Float(*f),
        _ => Num::Int(0),
    }
}

fn cmp_num(a: Num, b: Num) -> Option<Ordering> {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => Some(x.cmp(&y)),
        _ => a.as_f64().partial_cmp(&b.as_f64()),
    }
}

// ---- dumping -------------------------------------------------------------------

fn bytes_lossy(b: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(b)
}

/// Raw output bytes. Dumps build `Vec<u8>` so non-UTF-8 strings survive.
pub struct Out(pub Vec<u8>);

impl Out {
    fn s(&mut self, s: &str) {
        self.0.extend_from_slice(s.as_bytes());
    }
    fn b(&mut self, b: &[u8]) {
        self.0.extend_from_slice(b);
    }
    fn pad(&mut self, n: usize) {
        self.0.extend(std::iter::repeat_n(b' ', n));
    }
}

/// Property name decoration for dumps: `"name":protected`, `"name":"Cls":private`.
pub fn prop_label(o: &Obj, name: &str) -> (String, crate::class::Vis, Rc<str>) {
    match o.class.find_prop(name) {
        Some(p) => (crate::call::plain_prop(name).to_string(), p.vis, p.declaring.clone()),
        None => (name.to_string(), crate::class::Vis::Public, o.class.name.clone()),
    }
}

pub fn var_dump(v: &Value, indent: usize, out: &mut Out, seen: &mut Vec<*const Obj>) {
    var_dump_item(&v.deref(), false, indent, out, seen)
}

/// True for a reference that something besides this storage still holds;
/// var_dump marks those with `&`.
fn is_shared_ref(raw: &Value) -> bool {
    matches!(raw, Value::Ref(s) if Rc::strong_count(s) > 1)
}

fn var_dump_item(v: &Value, amp: bool, indent: usize, out: &mut Out, seen: &mut Vec<*const Obj>) {
    out.pad(indent);
    if amp {
        out.0.push(b'&');
    }
    match v {
        Value::Ref(_) => unreachable!(),
        Value::Resource(r) if r.is_open() => out.s(&format!("resource({}) of type ({})\n", r.id, r.kind.get())),
        Value::Resource(r) => out.s(&format!("resource({}) of type (Unknown)\n", r.id)),
        Value::Null => out.s("NULL\n"),
        Value::Bool(b) => out.s(&format!("bool({b})\n")),
        Value::Int(n) => out.s(&format!("int({n})\n")),
        Value::Float(f) => out.s(&format!("float({})\n", fmt_float(*f))),
        Value::Str(s) => {
            out.s(&format!("string({}) \"", s.len()));
            out.b(s);
            out.s("\"\n");
        }
        Value::Array(a) => {
            out.s(&format!("array({}) {{\n", a.len()));
            for (k, raw) in a.iter_raw() {
                let (amp, v) = (is_shared_ref(raw), raw.deref());
                out.pad(indent + 2);
                match k {
                    Key::Int(n) => out.s(&format!("[{n}]=>\n")),
                    Key::Str(s) => {
                        out.s("[\"");
                        out.b(&s);
                        out.s("\"]=>\n");
                    }
                }
                var_dump_item(&v, amp, indent + 2, out, seen);
            }
            out.pad(indent);
            out.s("}\n");
        }
        Value::Object(o) => {
            if o.class.is_enum() {
                let case = o.get_prop("name").and_then(|v| v.scalar_bytes()).unwrap_or_else(|| pstr(""));
                out.s(&format!("enum({}::{})\n", o.class.name, bytes_lossy(&case)));
                return;
            }
            let ptr = Rc::as_ptr(o);
            if seen.contains(&ptr) {
                out.s("*RECURSION*\n");
                return;
            }
            let props: Vec<(Rc<str>, bool, Value)> =
                o.props.borrow().iter().map(|(k, v)| (k.clone(), is_shared_ref(v), v.deref())).collect();
            out.s(&format!("object({})#{} ({}) {{\n", o.class.name, o.id, props.len()));
            seen.push(ptr);
            for (name, amp, v) in props.iter() {
                out.pad(indent + 2);
                let (n, vis, decl) = prop_label(o, name);
                match vis {
                    crate::class::Vis::Public => out.s(&format!("[\"{n}\"]=>\n")),
                    crate::class::Vis::Protected => out.s(&format!("[\"{n}\":protected]=>\n")),
                    crate::class::Vis::Private => out.s(&format!("[\"{n}\":\"{decl}\":private]=>\n")),
                }
                var_dump_item(v, *amp, indent + 2, out, seen);
            }
            seen.pop();
            out.pad(indent);
            out.s("}\n");
        }
    }
}

pub fn print_r(v: &Value, indent: usize, out: &mut Out, seen: &mut Vec<*const Obj>) {
    match v {
        Value::Ref(s) => print_r(&s.borrow().clone(), indent, out, seen),
        Value::Array(a) => {
            out.s("Array\n");
            print_r_body(a.iter().map(|(k, v)| (key_label(&k), v)).collect(), indent, out, seen);
        }
        Value::Object(o) if !o.class.is_enum() => {
            let ptr = Rc::as_ptr(o);
            out.s(&format!("{} Object\n", o.class.name));
            if seen.contains(&ptr) {
                out.pad(indent + 4);
                out.s(" *RECURSION*");
                return;
            }
            seen.push(ptr);
            let props = o.prop_list();
            let items = props
                .iter()
                .map(|(n, v)| {
                    let (n, vis, decl) = prop_label(o, n);
                    let label = match vis {
                        crate::class::Vis::Public => n,
                        crate::class::Vis::Protected => format!("{n}:protected"),
                        crate::class::Vis::Private => format!("{n}:{decl}:private"),
                    };
                    (label.into_bytes(), v.clone())
                })
                .collect();
            print_r_body(items, indent, out, seen);
            seen.pop();
        }
        Value::Object(o) => {
            let case = o.get_prop("name").and_then(|v| v.scalar_bytes()).unwrap_or_else(|| pstr(""));
            out.s(&format!("{} Enum ( [name] => {} )", o.class.name, bytes_lossy(&case)));
        }
        other => out.b(&other.scalar_bytes().unwrap()),
    }
}

fn key_label(k: &Key) -> Vec<u8> {
    match k {
        Key::Int(n) => n.to_string().into_bytes(),
        Key::Str(s) => s.to_vec(),
    }
}

fn print_r_body(items: Vec<(Vec<u8>, Value)>, indent: usize, out: &mut Out, seen: &mut Vec<*const Obj>) {
    out.pad(indent);
    out.s("(\n");
    for (k, v) in items {
        out.pad(indent + 4);
        out.s("[");
        out.b(&k);
        out.s("] => ");
        print_r(&v, indent + 8, out, seen);
        out.s("\n");
    }
    out.pad(indent);
    out.s(")\n");
}

pub fn var_export(v: &Value, indent: usize, out: &mut Out) {
    match v {
        Value::Ref(s) => var_export(&s.borrow().clone(), indent, out),
        Value::Null | Value::Resource(_) => out.s("NULL"),
        Value::Bool(b) => out.s(if *b { "true" } else { "false" }),
        Value::Int(n) => out.s(&n.to_string()),
        Value::Float(f) => out.s(&fmt_float_repr(*f)),
        Value::Str(s) => {
            out.s("'");
            for &c in s.iter() {
                match c {
                    b'\'' => out.s("\\'"),
                    b'\\' => out.s("\\\\"),
                    _ => out.0.push(c),
                }
            }
            out.s("'");
        }
        Value::Array(a) => {
            out.s("array (\n");
            for (k, v) in a.iter() {
                out.pad(indent + 2);
                match k {
                    Key::Int(n) => out.s(&n.to_string()),
                    Key::Str(s) => var_export(&Value::Str(s.clone()), 0, out),
                }
                out.s(" => ");
                if matches!(v, Value::Array(_) | Value::Object(_)) {
                    out.s("\n");
                    out.pad(indent + 2);
                }
                var_export(&v, indent + 2, out);
                out.s(",\n");
            }
            out.pad(indent);
            out.s(")");
        }
        Value::Object(o) if o.class.is_enum() => {
            let case = o.get_prop("name").and_then(|v| v.scalar_bytes()).unwrap_or_else(|| pstr(""));
            out.s(&format!("\\{}::{}", o.class.name, bytes_lossy(&case)));
        }
        Value::Object(o) => {
            out.s(&format!("\\{}::__set_state(array(\n", o.class.name));
            for (k, v) in o.prop_list() {
                out.pad(indent + 3);
                out.s(&format!("'{k}' => "));
                var_export(&v, indent + 2, out);
                out.s(",\n");
            }
            out.pad(indent);
            out.s("))");
        }
    }
}

/// Small helper for building strings in builtins.
pub fn join_display(parts: &[String], sep: &str) -> String {
    let mut s = String::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            s.push_str(sep);
        }
        let _ = write!(s, "{p}");
    }
    s
}

/// Interned lowercase names, to avoid re-lowercasing on every lookup.
#[derive(Default)]
pub struct Lower(RefCell<HashMap<String, Rc<str>>>);

impl Lower {
    pub fn get(&self, s: &str) -> Rc<str> {
        if let Some(v) = self.0.borrow().get(s) {
            return v.clone();
        }
        let l: Rc<str> = Rc::from(s.to_ascii_lowercase());
        self.0.borrow_mut().insert(s.to_string(), l.clone());
        l
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Native code writes values directly; check the layout it assumes.
    #[test]
    fn value_layout_is_stable() {
        assert_eq!(std::mem::size_of::<Value>(), 16);
        let read = |v: &Value| -> (u8, u64) {
            let p = v as *const Value as *const u8;
            unsafe { (*p, (p.add(tag::PAYLOAD as usize) as *const u64).read()) }
        };
        assert_eq!(read(&Value::Null).0, tag::NULL);
        assert_eq!(read(&Value::Bool(true)), (tag::BOOL, read(&Value::Bool(true)).1));
        assert_eq!(read(&Value::Bool(true)).1 & 0xFF, 1);
        assert_eq!(read(&Value::Int(-5)), (tag::INT, -5i64 as u64));
        assert_eq!(read(&Value::Float(1.5)), (tag::FLOAT, 1.5f64.to_bits()));
        assert_eq!(read(&Value::str("x")).0, tag::STR);
        assert_eq!(read(&Value::array(PhpArray::new())).0, tag::ARRAY);
    }

    #[test]
    fn float_format_matches_php() {
        assert_eq!(fmt_float(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(fmt_float(1.0), "1");
        assert_eq!(fmt_float(1e15), "1000000000000000");
        assert_eq!(fmt_float(1e17), "1.0E+17");
        assert_eq!(fmt_float_str(1e14), "1.0E+14");
        assert_eq!(fmt_float_str(0.1 + 0.2), "0.3");
        assert_eq!(fmt_float_str(123456789012345678.0), "1.2345678901235E+17");
        assert_eq!(fmt_float_str(-1.5), "-1.5");
        assert_eq!(fmt_float(0.0001), "0.0001");
        assert_eq!(fmt_float(0.00001), "1.0E-5");
        assert_eq!(fmt_float(-1.5e-7), "-1.5E-7");
        assert_eq!(fmt_float(123456789012345680.0), "1.2345678901234568E+17");
        assert_eq!(fmt_float(-0.0), "-0");
    }

    #[test]
    fn numeric_strings() {
        assert_eq!(parse_numeric(b" 12 "), Some(Num::Int(12)));
        assert_eq!(parse_numeric(b"1e3"), Some(Num::Float(1000.0)));
        assert_eq!(parse_numeric(b".5"), Some(Num::Float(0.5)));
        assert_eq!(parse_numeric(b"12abc"), None);
        assert_eq!(parse_numeric(b"abc"), None);
        assert_eq!(parse_numeric(b"0x1A"), None);
        assert_eq!(parse_numeric(b"."), None);
    }

    #[test]
    fn loose_comparison_php8() {
        let s = Value::str;
        assert!(loose_eq(&Value::Int(0), &s("0")));
        assert!(!loose_eq(&Value::Int(0), &s("a"))); // PHP 8 change
        assert!(loose_eq(&s("1e3"), &s("1000")));
        assert!(loose_eq(&Value::Null, &s("")));
        assert!(loose_eq(&Value::Null, &Value::Bool(false)));
        assert!(!loose_eq(&s("abc"), &s("ABC")));
        assert!(loose_eq(&Value::Int(1), &Value::Float(1.0)));
    }

    #[test]
    fn array_keys_and_next_index() {
        let mut a = PhpArray::new();
        a.insert(Key::from_bytes(b"5"), Value::Int(1));
        assert_eq!(a.push(Value::Int(2)), Key::Int(6));
        assert_eq!(Key::from_bytes(b"05"), Key::Str(pstr("05")));
        let mut b = PhpArray::new();
        b.insert(Key::Int(-5), Value::Null);
        assert_eq!(b.push(Value::Null), Key::Int(-4)); // PHP 8.3
    }
}
