//! Runtime values with PHP 8 semantics: byte strings, copy-on-write ordered
//! arrays, reference-counted objects.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
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
/// 16 bytes total. Native code relies on it (see `tag` below). (The tag
/// stays a byte: a wider one would cost `Option<Value>` its niche, which
/// property slots rely on.)
#[derive(Default)]
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

impl Clone for Value {
    /// A refcount bump and a 16-byte copy. (The derived clone copies the
    /// payload through overlapping stack stores and reloads them, which
    /// stalls store-to-load forwarding on every clone.)
    #[inline]
    fn clone(&self) -> Value {
        match self {
            Value::Str(s) => std::mem::forget(s.clone()),
            Value::Array(a) => std::mem::forget(a.clone()),
            Value::Object(o) => std::mem::forget(o.clone()),
            Value::Ref(r) => std::mem::forget(r.clone()),
            Value::Resource(r) => std::mem::forget(r.clone()),
            Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) => {}
        }
        // SAFETY: every payload is plain data or a counted pointer whose
        // clone (counted above) has the same bits.
        unsafe { std::ptr::read(self) }
    }
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Int(i64),
    Str(PStr),
}

// Hashed by hand so a borrowed `KeyRef` hashes the same (lookups by
// borrowed bytes then don't build an owned key).
impl std::hash::Hash for Key {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match self {
            Key::Int(n) => KeyRef::Int(*n).hash(h),
            Key::Str(s) => KeyRef::Str(s).hash(h),
        }
    }
}

/// A key to look up by, borrowing a string key's bytes.
#[derive(Clone, Copy)]
pub enum KeyRef<'a> {
    Int(i64),
    Str(&'a [u8]),
}

impl KeyRef<'_> {
    /// Like `Key::from_bytes`, borrowing.
    #[inline]
    pub fn from_bytes(b: &[u8]) -> KeyRef<'_> {
        match canonical_int(b) {
            Some(n) => KeyRef::Int(n),
            None => KeyRef::Str(b),
        }
    }
}

impl std::hash::Hash for KeyRef<'_> {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        match self {
            KeyRef::Int(n) => {
                h.write_u8(0);
                h.write_i64(*n);
            }
            KeyRef::Str(b) => {
                h.write_u8(1);
                h.write(b);
            }
        }
    }
}

impl indexmap::Equivalent<Key> for KeyRef<'_> {
    #[inline]
    fn equivalent(&self, k: &Key) -> bool {
        match (self, k) {
            (KeyRef::Int(a), Key::Int(b)) => a == b,
            (KeyRef::Str(a), Key::Str(b)) => *a == &**b,
            _ => false,
        }
    }
}

impl Key {
    /// String keys that look like canonical decimal ints become int keys.
    pub fn from_bytes(b: &[u8]) -> Key {
        match canonical_int(b) {
            Some(n) => Key::Int(n),
            None => Key::Str(pbytes(b)),
        }
    }

    /// Like `from_bytes` for a string value: shares it instead of copying
    /// (an array lookup with a string key then allocates nothing).
    /// (A string with spare capacity, e.g. built by interpolation or `.=`,
    /// is copied to its exact size instead: a key lives as long as its
    /// array.)
    #[inline]
    pub fn from_pstr(s: &PStr) -> Key {
        match canonical_int(s) {
            Some(n) => Key::Int(n),
            None if s.capacity() > s.len() => Key::Str(pbytes(s)),
            None => Key::Str(s.clone()),
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
#[inline(always)]
fn canonical_int(b: &[u8]) -> Option<i64> {
    // Fast exit for the usual string key.
    match b.first() {
        Some(c) if c.is_ascii_digit() || *c == b'-' => canonical_int_slow(b),
        _ => None,
    }
}

#[inline(never)]
fn canonical_int_slow(b: &[u8]) -> Option<i64> {
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
    Hash(crate::hmap::HMap),
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
/// An element as an array copy holds it: a reference only the source
/// holds becomes its value.
pub fn copy_elem(v: &Value) -> Value {
    match v {
        Value::Ref(s) if Rc::strong_count(s) == 1 => s.borrow().clone(),
        other => other.clone(),
    }
}

impl Clone for PhpArray {
    fn clone(&self) -> Self {
        let store = match &self.store {
            Store::Packed(v) => Store::Packed(v.iter().map(copy_elem).collect()),
            Store::Hash(m) => {
                // Copies the hash table as is (no rehashing), then drops
                // references nothing else shares (the source held the only
                // other count).
                let (mut c, refs) = m.copy(4);
                for v in c.values_mut().filter(|_| refs) {
                    if let Value::Ref(s) = v {
                        if Rc::strong_count(s) == 2 {
                            let inner = s.borrow().clone();
                            *v = inner;
                        }
                    }
                }
                Store::Hash(c)
            }
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

    fn to_hash(&mut self) -> &mut crate::hmap::HMap {
        if let Store::Packed(v) = &mut self.store {
            let m = crate::hmap::HMap::from_unique(std::mem::take(v).into_iter().enumerate().map(|(i, x)| (Key::Int(i as i64), x)).collect());
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

    /// The element at a borrowed key, borrowed (a reference stays one).
    #[inline]
    pub fn peek(&self, k: KeyRef) -> Option<&Value> {
        match &self.store {
            Store::Packed(v) => match k {
                KeyRef::Int(n) if n >= 0 => v.get(n as usize),
                _ => None,
            },
            Store::Hash(m) => m.find(k).map(|i| &m[i]),
        }
    }

    /// Element value by a borrowed key (references dereferenced).
    #[inline]
    pub fn get_ref(&self, k: KeyRef) -> Option<Value> {
        match &self.store {
            Store::Packed(v) => match k {
                KeyRef::Int(n) if n >= 0 && (n as usize) < v.len() => Some(v[n as usize].deref()),
                _ => None,
            },
            Store::Hash(m) => m.find(k).map(|i| m[i].deref()),
        }
    }

    /// Whether a borrowed key is present.
    #[inline]
    pub fn contains_ref(&self, k: KeyRef) -> bool {
        match &self.store {
            Store::Packed(v) => matches!(k, KeyRef::Int(n) if n >= 0 && (n as usize) < v.len()),
            Store::Hash(m) => m.find(k).is_some(),
        }
    }

    pub fn contains(&self, k: &Key) -> bool {
        match &self.store {
            Store::Packed(v) => Self::packed_index(v.len(), k).is_some(),
            Store::Hash(m) => m.contains_key(k),
        }
    }

    /// A copy without the spare room `clone` leaves for writes (template
    /// arrays have no references to drop).
    pub fn clone_exact(&self) -> PhpArray {
        let store = match &self.store {
            Store::Packed(v) => Store::Packed(v.clone()),
            Store::Hash(m) => Store::Hash(m.clone_exact()),
        };
        PhpArray::make(store, self.next_free, 0)
    }

    /// The value of the `i`th element (in order), for writes.
    pub fn value_at_mut(&mut self, i: usize) -> &mut Value {
        match &mut self.store {
            Store::Packed(v) => &mut v[i],
            Store::Hash(m) => m.get_index_mut(i).expect("element index").1,
        }
    }

    /// Raw element storage by a borrowed key, for writes (may be a
    /// `Value::Ref`).
    #[inline]
    pub fn get_mut_ref(&mut self, k: KeyRef) -> Option<&mut Value> {
        match &mut self.store {
            Store::Packed(v) => match k {
                KeyRef::Int(n) if n >= 0 && (n as usize) < v.len() => Some(&mut v[n as usize]),
                _ => None,
            },
            Store::Hash(m) => m.find(k).map(|i| m.get_index_mut(i).unwrap().1),
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

    /// Insert a key known to be absent (no lookup).
    pub fn insert_absent(&mut self, k: Key, v: Value) {
        if let Key::Int(n) = k {
            self.bump_next(n);
            if let Store::Packed(vec) = &mut self.store {
                if n >= 0 && n as usize == vec.len() {
                    vec.push(v);
                    self.sync();
                    return;
                }
            }
        }
        self.to_hash().insert_absent(k, v);
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
        self.to_hash().entry_or_default(k)
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
    pub fn raw_values(&self) -> Either<std::slice::Iter<'_, Value>, crate::hmap::Values<'_>> {
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

    /// Stored as a plain list (keys 0..len).
    pub fn is_packed(&self) -> bool {
        matches!(self.store, Store::Packed(_))
    }

    pub fn is_list(&self) -> bool {
        match &self.store {
            Store::Packed(_) => true,
            Store::Hash(m) => m.keys().enumerate().all(|(i, k)| *k == Key::Int(i as i64)),
        }
    }

    /// Empty hash-stored array with room for `n` elements.
    pub fn with_hash_capacity(n: usize) -> Self {
        PhpArray::make(Store::Hash(crate::hmap::HMap::with_capacity(n)), i64::MIN, 0)
    }

    /// Room for `n` more elements.
    pub fn reserve(&mut self, n: usize) {
        match &mut self.store {
            Store::Packed(v) => v.reserve(n),
            Store::Hash(m) => m.reserve(n),
        }
        self.sync();
    }

    /// Whether any key is an int.
    pub fn has_int_keys(&self) -> bool {
        match &self.store {
            Store::Packed(v) => !v.is_empty(),
            Store::Hash(m) => m.keys().any(|k| matches!(k, Key::Int(_))),
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


/// Work waiting for the next statement boundary on this thread: queued
/// destructors, a cycle collection, an exceeded memory or time limit
/// (where an exception or fatal error can surface). One flag per thread,
/// at a fixed address so compiled code (also per thread) tests it with
/// one load.
#[inline]
pub fn destruct_pending() -> &'static std::sync::atomic::AtomicBool {
    if phpx_runtime::fastlocal::is_main() {
        return &MAIN_FLAGS.0;
    }
    thread_flags().0
}

/// Set once a namespaced user function of this request has a built-in's
/// name; compiled code then stops inlining built-ins (`ns\strlen` could
/// take precedence).
#[inline]
pub fn builtin_shadowed() -> &'static std::sync::atomic::AtomicBool {
    if phpx_runtime::fastlocal::is_main() {
        return &MAIN_FLAGS.1;
    }
    thread_flags().1
}

/// The main thread's flags, outside thread-local storage (see
/// `phpx_runtime::fastlocal`); the timeout watcher raises them too.
pub static MAIN_FLAGS: (std::sync::atomic::AtomicBool, std::sync::atomic::AtomicBool) =
    (std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false));

/// Other threads' flags. Out of line: inlined, the thread-local lookup (a
/// call on macOS) could be hoisted above the main-thread check.
#[cold]
#[inline(never)]
fn thread_flags() -> (&'static std::sync::atomic::AtomicBool, &'static std::sync::atomic::AtomicBool) {
    THREAD_FLAGS.with(|f| *f)
}

/// Every thread's pending flag (the timeout watcher raises them all).
pub static ALL_PENDING: std::sync::Mutex<Vec<&'static std::sync::atomic::AtomicBool>> = std::sync::Mutex::new(Vec::new());

thread_local! {
    static THREAD_FLAGS: (&'static std::sync::atomic::AtomicBool, &'static std::sync::atomic::AtomicBool) = {
        let pending: &'static std::sync::atomic::AtomicBool = Box::leak(Box::new(std::sync::atomic::AtomicBool::new(false)));
        if let Ok(mut all) = ALL_PENDING.lock() {
            all.push(pending);
        }
        (pending, Box::leak(Box::new(std::sync::atomic::AtomicBool::new(false))))
    };
}

thread_local! {
    pub static DESTRUCT_QUEUE: RefCell<Vec<Rc<Obj>>> = const { RefCell::new(Vec::new()) };
}


pub struct Obj {
    pub id: u32,
    /// Flags: "has native state" (bit 30), "`__destruct` already ran (or
    /// was queued)" (bit 31), and the registration order (low 30 bits). (The handle `id` is also the object's
    /// slot in the collector's registry.)
    gc: Cell<u32>,
    pub class: Rc<Class>,
    /// Initialized properties in declaration order. Typed properties without
    /// a default are absent until assigned.
    pub props: RefCell<crate::props::Props>,
    /// Native state (closures, generators, ...), boxed so a borrow of it
    /// stays valid while it is replaced; set only with HAS_INTERNAL.
    internal: std::cell::UnsafeCell<Option<Box<RefCell<Internal>>>>,
}

const HAS_INTERNAL: u32 = 1 << 30;
const SEQ_MASK: u32 = HAS_INTERNAL - 1;
const DESTRUCTED: u32 = 1 << 31;

thread_local! {
    /// What objects without native state read (never borrowed mutably).
    static NO_INTERNAL: &'static RefCell<Internal> = Box::leak(Box::new(RefCell::new(Internal::None)));
}

impl Obj {
    pub fn new(class: Rc<Class>, internal: Internal) -> Rc<Obj> {
        Obj::with_props(class, internal, Default::default())
    }

    /// New object with its initial property storage (avoids building an
    /// empty one first).
    pub fn with_props(class: Rc<Class>, internal: Internal, props: crate::props::Props) -> Rc<Obj> {
        crate::gc::track(|id| {
            let (flags, internal) = match internal {
                Internal::None => (0, None),
                i => (HAS_INTERNAL, Some(Box::new(RefCell::new(i)))),
            };
            Rc::new(Obj { id, gc: Cell::new(flags), class, props: RefCell::new(props), internal: std::cell::UnsafeCell::new(internal) })
        })
    }

    fn internal_cell(&self) -> &RefCell<Internal> {
        if self.gc.get() & HAS_INTERNAL != 0 {
            // SAFETY: the box is only taken out by take_internal (the cycle
            // collector, with no borrow of it live) or when the object drops.
            if let Some(b) = unsafe { (*self.internal.get()).as_deref() } {
                return b;
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

    /// Registration order (low 30 bits of the flags word, wrapping).
    pub fn seq(&self) -> u32 {
        self.gc.get() & SEQ_MASK
    }

    pub fn set_seq(&self, seq: u32) {
        self.gc.set((self.gc.get() & !SEQ_MASK) | (seq & SEQ_MASK));
    }

    /// `__destruct` already ran (or was queued).
    pub fn destructed(&self) -> bool {
        self.gc.get() & DESTRUCTED != 0
    }

    /// Remove the native state (closures, generators, ...), for freeing an
    /// object's cycles.
    pub fn take_internal(&self) -> Option<Box<RefCell<Internal>>> {
        if self.gc.get() & HAS_INTERNAL == 0 {
            return None;
        }
        // SAFETY: see internal_cell.
        unsafe { (*self.internal.get()).take() }
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
                std::ptr::addr_of_mut!((*p).gc).write(Cell::new(0));
                std::ptr::addr_of_mut!((*p).class).write(class.clone());
                // (Cloned here, not passed in: a Vec handed over by value
                // is reread right after being written, which stalls.)
                std::ptr::addr_of_mut!((*p).props).write(RefCell::new(crate::props::Props::from_template(class.shape.clone(), template)));
                std::ptr::addr_of_mut!((*p).internal).write(std::cell::UnsafeCell::new(None));
                rc.assume_init()
            }
        })
    }

    pub fn get_prop(&self, name: &str) -> Option<Value> {
        self.props.borrow().get(name).map(Value::deref)
    }

    /// Virtual properties of native state (see `NativeData::view_props`).
    pub fn view_props(&self) -> Vec<(Rc<str>, Value)> {
        if self.gc.get() & HAS_INTERNAL == 0 {
            return vec![];
        }
        match self.try_internal().as_deref() {
            Some(Internal::Data(d)) => d.view_props(),
            _ => vec![],
        }
    }

    /// Native comparison of two objects' states, if their class has one.
    /// `==` between Closure objects (None: not both closures). As in PHP,
    /// distinct closures differ unless both come from the same callable
    /// (`strlen(...)`, `$o->m(...)`, Closure::fromCallable()).
    pub fn closure_eq(&self, other: &Obj) -> Option<bool> {
        if self.gc.get() & other.gc.get() & HAS_INTERNAL == 0 {
            return None;
        }
        let (a, b) = (self.try_internal()?, other.try_internal()?);
        match (&*a, &*b) {
            (Internal::Closure(x), Internal::Closure(y)) => Some(std::ptr::eq(self, other) || same_callable(x, y)),
            _ => None,
        }
    }

    pub fn native_compare(&self, other: &Obj) -> Option<Ordering> {
        if self.gc.get() & other.gc.get() & HAS_INTERNAL == 0 {
            return None;
        }
        let (a, b) = (self.try_internal()?, other.try_internal()?);
        match (&*a, &*b) {
            (Internal::Data(x), Internal::Data(y)) => x.compare(&**y),
            _ => None,
        }
    }

    /// Give an existing object native state (a constructor of a built-in
    /// class running for a user subclass).
    pub fn set_internal(&self, i: Internal) {
        if self.gc.get() & HAS_INTERNAL != 0 {
            *self.internal_cell().borrow_mut() = i;
            return;
        }
        // SAFETY: no state yet, so nothing borrows it.
        unsafe { *self.internal.get() = Some(Box::new(RefCell::new(i))) };
        self.gc.set(self.gc.get() | HAS_INTERNAL);
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
        // Like PHP: what the object holds is released before its own handle
        // goes back to the free list (so the handle is the next one reused).
        self.props.get_mut().drop_values();
        if self.gc.get() & HAS_INTERNAL != 0 {
            // Taken out first: dropping it can free other objects.
            let st = self.internal.get_mut().take();
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
                gc: Cell::new(DESTRUCTED | (self.gc.get() & (HAS_INTERNAL | SEQ_MASK))),
                class: self.class.clone(),
                props: RefCell::new(std::mem::take(self.props.get_mut())),
                internal: std::cell::UnsafeCell::new(self.internal.get_mut().take()),
            });
            crate::gc::register(&o);
            if DESTRUCT_QUEUE.try_with(|q| q.borrow_mut().push(o)).is_ok() {
                destruct_pending().store(true, std::sync::atomic::Ordering::Relaxed);
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
    /// Native state of built-in classes (HashContext, DateTime, ...).
    Data(Box<dyn NativeData>),
}

/// Native state of a built-in class's object.
pub trait NativeData: std::any::Any {
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    /// State for `clone` (None: the class is uncloneable).
    fn clone_data(&self) -> Option<Box<dyn NativeData>>;
    /// Properties shown by var_dump, print_r, var_export, json_encode and
    /// array casts but not real (DateTime's `date`, `timezone`, ...).
    fn view_props(&self) -> Vec<(Rc<str>, Value)> {
        vec![]
    }
    /// Comparison with another object's state (DateTime compares instants).
    fn compare(&self, _other: &dyn NativeData) -> Option<Ordering> {
        None
    }
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
        (Value::Resource(x), Value::Resource(y)) => x.id == y.id,
        _ => false,
    }
}

/// `==`
/// Less than 256 KiB of task stack left: recursive walks over values stop
/// (recursive structures built with references can be arbitrarily deep).
pub fn stack_low() -> bool {
    phpx_runtime::stack_remaining().is_some_and(|r| r < 256 * 1024)
}

/// Whether two first-class-callable closures wrap the same callable.
fn same_callable(x: &crate::interp::Callable, y: &crate::interp::Callable) -> bool {
    use crate::interp::{Callable, ClosureKind};
    let unwrap = |c: &Callable| -> Option<Callable> {
        match c {
            Callable::Closure(cd) => match &cd.kind {
                ClosureKind::Wrap(inner) => Some(inner.clone()),
                _ => None,
            },
            c => Some(c.clone()),
        }
    };
    match (unwrap(x), unwrap(y)) {
        (Some(Callable::User(p)), Some(Callable::User(q))) => std::ptr::eq(p.decl, q.decl),
        (Some(Callable::Builtin(p, _)), Some(Callable::Builtin(q, _))) => p == q,
        (Some(Callable::Method { this: t1, method: m1, .. }), Some(Callable::Method { this: t2, method: m2, .. })) => {
            Rc::ptr_eq(&m1, &m2)
                && match (t1, t2) {
                    (Some(a), Some(b)) => Rc::ptr_eq(&a, &b),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

pub fn loose_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Ref(x), _) => loose_eq(&x.borrow(), b),
        (_, Value::Ref(y)) => loose_eq(a, &y.borrow()),
        (Value::Object(x), Value::Object(y)) => {
            if let Some(eq) = x.closure_eq(y) {
                return eq;
            }
            if let Some(o) = x.native_compare(y) {
                return o == Ordering::Equal;
            }
            Rc::ptr_eq(x, y)
                || (Rc::ptr_eq(&x.class, &y.class) && {
                    let (px, py) = (x.prop_list(), y.prop_list());
                    px.len() == py.len() && px.iter().all(|(k, v)| py.iter().any(|(k2, w)| k == k2 && loose_eq(v, w)))
                })
        }
        (Value::Array(x), Value::Array(y)) => {
            // The same array (also how a self-referencing array compares
            // equal to itself without recursing forever).
            Rc::ptr_eq(x, y)
                || (x.len() == y.len() && !stack_low() && x.iter().all(|(k, v)| y.get(&k).is_some_and(|w| loose_eq(&v, &w))))
        }
        // A string starting above '9' is never numeric (PHP's
        // zend_fast_equal_strings): compare the bytes.
        (Value::Str(x), Value::Str(y)) if x.as_slice().first().is_some_and(|&c| c > b'9') || y.as_slice().first().is_some_and(|&c| c > b'9') => x.as_slice() == y.as_slice(),
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
            if Rc::ptr_eq(x, y) {
                return Some(Ordering::Equal);
            }
            if stack_low() {
                // Nested beyond what the stack can take (a recursive
                // structure through references): uncomparable.
                return None;
            }
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
        (Object(x), Object(y)) => match x.native_compare(y) {
            Some(o) => Some(o),
            None => if loose_eq(a, b) { Some(Ordering::Equal) } else { None },
        },
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
    crate::pstr::lossy(b)
}

/// Raw output bytes. Dumps build `Vec<u8>` so non-UTF-8 strings survive.
pub struct Out(pub Vec<u8>);

impl Out {
    // Past memory_limit, stop growing: the fatal error is reported at the
    // next statement boundary (a dump can be quadratic in nesting depth).
    fn s(&mut self, s: &str) {
        self.b(s.as_bytes());
    }
    fn b(&mut self, b: &[u8]) {
        if !crate::alloc::over() {
            self.0.extend_from_slice(b);
        }
    }
    fn pad(&mut self, n: usize) {
        if !crate::alloc::over() {
            self.0.extend(std::iter::repeat_n(b' ', n));
        }
    }
}

/// Property name decoration for dumps: `"name":protected`, `"name":"Cls":private`.
pub fn prop_label(o: &Obj, name: &str) -> (String, crate::class::Vis, Rc<str>) {
    match o.class.find_prop(name) {
        Some(p) => (crate::call::plain_prop(name).to_string(), p.vis, Rc::from(crate::class::display_name(&p.declaring))),
        None => (name.to_string(), crate::class::Vis::Public, o.class.name.clone()),
    }
}

/// An object's `__debugInfo()` entries, for var_dump and print_r (None:
/// dump its properties).
pub type DebugInfo<'a> = &'a dyn Fn(&Rc<Obj>) -> Option<Vec<(Key, Value)>>;

/// Dump labels for `__debugInfo` keys: mangled names (`\0Class\0p`,
/// `\0*\0p`) show as private/protected, like properties.
enum DebugLabel {
    Int(i64),
    Public(String),
    Protected(String),
    Private(String, String),
}

fn debug_label(k: &Key) -> DebugLabel {
    match k {
        Key::Int(n) => DebugLabel::Int(*n),
        Key::Str(s) => {
            let s = bytes_lossy(s);
            if let Some(rest) = s.strip_prefix('\0') {
                if let Some((cls, name)) = rest.split_once('\0') {
                    return if cls == "*" { DebugLabel::Protected(name.to_string()) } else { DebugLabel::Private(name.to_string(), cls.to_string()) };
                }
            }
            DebugLabel::Public(s.into_owned())
        }
    }
}

pub fn var_dump(v: &Value, indent: usize, out: &mut Out, seen: &mut Vec<usize>, dbg: DebugInfo) {
    var_dump_item(&v.deref(), false, indent, out, seen, dbg)
}

/// True for a reference that something besides this storage still holds;
/// var_dump marks those with `&`.
fn is_shared_ref(raw: &Value) -> bool {
    matches!(raw, Value::Ref(s) if Rc::strong_count(s) > 1)
}

fn var_dump_item(v: &Value, amp: bool, indent: usize, out: &mut Out, seen: &mut Vec<usize>, dbg: DebugInfo) {
    out.pad(indent);
    // A reference back to an array on the path prints as plain *RECURSION*.
    let amp = amp && !matches!(v, Value::Array(a) if seen.contains(&(Rc::as_ptr(a) as usize)));
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
            // An array on the current path again (through a reference):
            // recursion, as PHP reports it.
            let ptr = Rc::as_ptr(a) as usize;
            if seen.contains(&ptr) || stack_low() {
                out.s("*RECURSION*\n");
                return;
            }
            seen.push(ptr);
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
                var_dump_item(&v, amp, indent + 2, out, seen, dbg);
            }
            seen.pop();
            out.pad(indent);
            out.s("}\n");
        }
        Value::Object(o) => {
            if o.class.is_enum() {
                let case = o.get_prop("name").and_then(|v| v.scalar_bytes()).unwrap_or_else(|| pstr(""));
                out.s(&format!("enum({}::{})\n", o.class.name, bytes_lossy(&case)));
                return;
            }
            let ptr = Rc::as_ptr(o) as usize;
            if seen.contains(&ptr) {
                out.s("*RECURSION*\n");
                return;
            }
            if let Some(items) = dbg(o) {
                out.s(&format!("object({})#{} ({}) {{\n", o.class.name, o.id, items.len()));
                seen.push(ptr);
                for (k, raw) in &items {
                    out.pad(indent + 2);
                    match debug_label(k) {
                        DebugLabel::Int(n) => out.s(&format!("[{n}]=>\n")),
                        DebugLabel::Public(n) => out.s(&format!("[\"{n}\"]=>\n")),
                        DebugLabel::Protected(n) => out.s(&format!("[\"{n}\":protected]=>\n")),
                        DebugLabel::Private(n, c) => out.s(&format!("[\"{n}\":\"{c}\":private]=>\n")),
                    }
                    var_dump_item(&raw.deref(), is_shared_ref(raw), indent + 2, out, seen, dbg);
                }
                seen.pop();
                out.pad(indent);
                out.s("}\n");
                return;
            }
            // Empty slots of typed properties show as uninitialized(T) but
            // don't count.
            let mut props: Vec<(Rc<str>, bool, Option<Value>)> = o
                .props
                .borrow()
                .slots()
                .filter_map(|(k, v)| match v {
                    Some(v) => Some((k.clone(), is_shared_ref(v), Some(v.deref()))),
                    None => o.class.find_prop(k).filter(|d| d.ty.is_some()).map(|_| (k.clone(), false, None)),
                })
                .collect();
            props.extend(o.view_props().into_iter().map(|(k, v)| (k, false, Some(v))));
            let count = props.iter().filter(|p| p.2.is_some()).count();
            out.s(&format!("object({})#{} ({}) {{\n", o.class.name, o.id, count));
            seen.push(ptr);
            for (name, amp, v) in props.iter() {
                out.pad(indent + 2);
                let (n, vis, decl) = prop_label(o, name);
                match vis {
                    crate::class::Vis::Public => out.s(&format!("[\"{n}\"]=>\n")),
                    crate::class::Vis::Protected => out.s(&format!("[\"{n}\":protected]=>\n")),
                    crate::class::Vis::Private => out.s(&format!("[\"{n}\":\"{decl}\":private]=>\n")),
                }
                match v {
                    Some(v) => var_dump_item(v, *amp, indent + 2, out, seen, dbg),
                    None => {
                        let ty = o.class.find_prop(name).and_then(|d| d.ty).map(crate::call::type_str).unwrap_or_default();
                        out.pad(indent + 2);
                        out.s(&format!("uninitialized({ty})\n"));
                    }
                }
            }
            seen.pop();
            out.pad(indent);
            out.s("}\n");
        }
    }
}

pub fn print_r(v: &Value, indent: usize, out: &mut Out, seen: &mut Vec<usize>, dbg: DebugInfo) {
    match v {
        Value::Ref(s) => print_r(&s.borrow().clone(), indent, out, seen, dbg),
        Value::Array(a) => {
            out.s("Array\n");
            let ptr = Rc::as_ptr(a) as usize;
            if seen.contains(&ptr) || stack_low() {
                out.s(" *RECURSION*");
                return;
            }
            seen.push(ptr);
            print_r_body(a.iter().map(|(k, v)| (key_label(&k), v)).collect(), indent, out, seen, dbg);
            seen.pop();
        }
        Value::Object(o) if !o.class.is_enum() => {
            let ptr = Rc::as_ptr(o) as usize;
            out.s(&format!("{} Object\n", o.class.name));
            if seen.contains(&ptr) {
                out.s(" *RECURSION*");
                return;
            }
            seen.push(ptr);
            if let Some(items) = dbg(o) {
                let items = items
                    .into_iter()
                    .map(|(k, v)| {
                        let label = match debug_label(&k) {
                            DebugLabel::Int(n) => n.to_string(),
                            DebugLabel::Public(n) => n,
                            DebugLabel::Protected(n) => format!("{n}:protected"),
                            DebugLabel::Private(n, c) => format!("{n}:{c}:private"),
                        };
                        (label.into_bytes(), v.deref())
                    })
                    .collect();
                print_r_body(items, indent, out, seen, dbg);
                seen.pop();
                return;
            }
            let mut props = o.prop_list();
            props.extend(o.view_props());
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
            print_r_body(items, indent, out, seen, dbg);
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

fn print_r_body(items: Vec<(Vec<u8>, Value)>, indent: usize, out: &mut Out, seen: &mut Vec<usize>, dbg: DebugInfo) {
    out.pad(indent);
    out.s("(\n");
    for (k, v) in items {
        out.pad(indent + 4);
        out.s("[");
        out.b(&k);
        out.s("] => ");
        print_r(&v, indent + 8, out, seen, dbg);
        out.s("\n");
    }
    out.pad(indent);
    out.s(")\n");
}

pub fn var_export(v: &Value, indent: usize, out: &mut Out) {
    var_export_checked(v, indent, out);
}

/// var_export; true if it met a circular reference (written as NULL; the
/// caller warns, like PHP).
pub fn var_export_checked(v: &Value, indent: usize, out: &mut Out) -> bool {
    let mut circular = false;
    export(v, indent, out, &mut vec![], &mut circular);
    circular
}

fn export(v: &Value, indent: usize, out: &mut Out, seen: &mut Vec<usize>, circular: &mut bool) {
    let var_export = |v: &Value, indent: usize, out: &mut Out, seen: &mut Vec<usize>, circular: &mut bool| export(v, indent, out, seen, circular);
    match v {
        Value::Ref(s) => var_export(&s.borrow().clone(), indent, out, seen, circular),
        Value::Null | Value::Resource(_) => out.s("NULL"),
        Value::Bool(b) => out.s(if *b { "true" } else { "false" }),
        // PHP_INT_MIN has no literal: PHP writes it as an expression.
        Value::Int(i64::MIN) => out.s("-9223372036854775807-1"),
        Value::Int(n) => out.s(&n.to_string()),
        Value::Float(f) => out.s(&fmt_float_repr(*f)),
        Value::Str(s) => {
            // NUL bytes can't appear in a single-quoted literal: PHP splits
            // the string around them ('' . "\0" . '').
            out.s("'");
            for &c in s.iter() {
                match c {
                    b'\'' => out.s("\\'"),
                    b'\\' => out.s("\\\\"),
                    0 => out.s("' . \"\\0\" . '"),
                    _ => out.b(&[c]),
                }
            }
            out.s("'");
        }
        Value::Array(a) => {
            let ptr = Rc::as_ptr(a) as usize;
            if seen.contains(&ptr) || stack_low() {
                *circular = true;
                out.s("NULL");
                return;
            }
            seen.push(ptr);
            // Nested: on its own line (after the recursion check, which
            // writes a plain NULL), as PHP lays it out.
            if indent > 0 {
                out.s("\n");
                out.pad(indent);
            }
            out.s("array (\n");
            for (k, v) in a.iter() {
                out.pad(indent + 2);
                match k {
                    Key::Int(n) => out.s(&n.to_string()),
                    Key::Str(s) => var_export(&Value::Str(s.clone()), 0, out, seen, circular),
                }
                out.s(" => ");
                var_export(&v, indent + 2, out, seen, circular);
                out.s(",\n");
            }
            seen.pop();
            out.pad(indent);
            out.s(")");
        }
        Value::Object(o) if o.class.is_enum() => {
            if indent > 0 {
                out.s("\n");
                out.pad(indent);
            }
            let case = o.get_prop("name").and_then(|v| v.scalar_bytes()).unwrap_or_else(|| pstr(""));
            out.s(&format!("\\{}::{}", o.class.name, bytes_lossy(&case)));
        }
        Value::Object(o) => {
            let ptr = Rc::as_ptr(o) as usize;
            if seen.contains(&ptr) || stack_low() {
                *circular = true;
                out.s("NULL");
                return;
            }
            seen.push(ptr);
            if indent > 0 {
                out.s("\n");
                out.pad(indent);
            }
            // stdClass exports as a cast (PHP 7.3+).
            let std = o.class.lname.as_ref() == "stdclass";
            if std {
                out.s("(object) array(\n");
            } else {
                out.s(&format!("\\{}::__set_state(array(\n", o.class.name));
            }
            for (k, v) in o.prop_list().into_iter().chain(o.view_props()) {
                out.pad(indent + 3);
                out.s(&format!("'{k}' => "));
                var_export(&v, indent + 2, out, seen, circular);
                out.s(",\n");
            }
            seen.pop();
            out.pad(indent);
            out.s(if std { ")" } else { "))" });
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
pub struct Lower(RefCell<rustc_hash::FxHashMap<String, Rc<str>>>);

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
