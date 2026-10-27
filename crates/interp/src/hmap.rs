//! Hash storage of PHP arrays: entries in insertion order, plus a hash
//! index only once the array outgrows a few elements.
//!
//! Most PHP arrays with string keys are small (options, attributes, rows,
//! casts). For those, PHP's HashTable is one allocation and a lookup
//! compares a handful of keys; an `IndexMap` would allocate an entry
//! vector and a table and hash every key on insert. Here an array of up
//! to `SCAN_MAX` elements is just its entry vector, searched linearly;
//! beyond that a `hashbrown` table maps hashes to entry positions.

use std::hash::BuildHasher;

use hashbrown::HashTable;

use crate::value::{Key, KeyRef, Value};

/// Arrays up to this size have no index and are searched linearly.
pub const SCAN_MAX: usize = 8;

type Hasher = std::hash::BuildHasherDefault<rustc_hash::FxHasher>;

#[inline]
fn hash_of(k: KeyRef) -> u64 {
    Hasher::default().hash_one(k)
}

#[inline]
fn key_ref(k: &Key) -> KeyRef<'_> {
    match k {
        Key::Int(n) => KeyRef::Int(*n),
        Key::Str(s) => KeyRef::Str(s),
    }
}

#[inline]
fn matches(e: &Key, k: KeyRef) -> bool {
    match (e, k) {
        (Key::Int(a), KeyRef::Int(b)) => *a == b,
        (Key::Str(a), KeyRef::Str(b)) => bytes_eq(a, b),
        _ => false,
    }
}

/// Byte-string equality without a `memcmp` call for short strings (most
/// array keys and property names): two overlapping word loads.
#[inline(always)]
pub fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len();
    if n != b.len() {
        return false;
    }
    if std::ptr::eq(a.as_ptr(), b.as_ptr()) {
        return true;
    }
    let (pa, pb) = (a.as_ptr(), b.as_ptr());
    // SAFETY: every load stays within the n bytes of both slices.
    unsafe {
        match n {
            0 => true,
            1..=3 => *pa == *pb && *pa.add(n / 2) == *pb.add(n / 2) && *pa.add(n - 1) == *pb.add(n - 1),
            4..=7 => {
                let w = |p: *const u8, i: usize| (p.add(i) as *const u32).read_unaligned();
                w(pa, 0) == w(pb, 0) && w(pa, n - 4) == w(pb, n - 4)
            }
            8..=16 => {
                let w = |p: *const u8, i: usize| (p.add(i) as *const u64).read_unaligned();
                w(pa, 0) == w(pb, 0) && w(pa, n - 8) == w(pb, n - 8)
            }
            _ => a == b,
        }
    }
}

#[derive(Default)]
pub struct HMap {
    entries: Vec<(Key, Value)>,
    /// Entry positions by key hash (None while the map is small).
    index: Option<HashTable<u32>>,
}

impl Clone for HMap {
    /// A copy is almost always made to be written (copy-on-write): leave
    /// room for a few more entries so the first insert doesn't reallocate.
    fn clone(&self) -> HMap {
        self.copy(4).0
    }
}

impl HMap {
    /// A copy with room for `extra` more entries, and whether any value is
    /// a reference. Entries are copied as words after their counts are
    /// bumped (a typed copy of a (Key, Value) pair goes through
    /// overlapping stack stores).
    pub fn copy(&self, extra: usize) -> (HMap, bool) {
        let n = self.entries.len();
        let mut entries: Vec<(Key, Value)> = Vec::with_capacity(n + extra);
        let mut refs = false;
        let (src, dst) = (self.entries.as_ptr() as *const u64, entries.as_mut_ptr() as *mut u64);
        const W: usize = std::mem::size_of::<(Key, Value)>() / 8;
        for (i, (k, v)) in self.entries.iter().enumerate() {
            std::mem::forget(k.clone());
            std::mem::forget(v.clone());
            refs |= matches!(v, Value::Ref(_));
            // SAFETY: capacity reserved; the bits are a valid copy now that
            // the counts are bumped.
            unsafe {
                for w in 0..W {
                    dst.add(i * W + w).write(src.add(i * W + w).read());
                }
            }
        }
        // SAFETY: n entries written.
        unsafe { entries.set_len(n) };
        (HMap { entries, index: self.index.clone() }, refs)
    }
}

impl HMap {
    /// A copy with no spare room (for a value that is rarely written,
    /// like an array literal from its template).
    pub fn clone_exact(&self) -> HMap {
        self.copy(0).0
    }
}

pub type Values<'a> = std::iter::Map<std::slice::Iter<'a, (Key, Value)>, fn(&'a (Key, Value)) -> &'a Value>;

impl HMap {
    pub fn with_capacity(n: usize) -> HMap {
        HMap { entries: Vec::with_capacity(n), index: None }
    }

    /// Room for `n` more entries.
    pub fn reserve(&mut self, n: usize) {
        self.entries.reserve(n);
        if let Some(t) = &mut self.index {
            let entries = &self.entries;
            t.reserve(n, |&j| hash_of(key_ref(&entries[j as usize].0)));
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Position of a key.
    #[inline(always)]
    pub fn find(&self, k: KeyRef) -> Option<usize> {
        match &self.index {
            None => self.entries.iter().position(|(e, _)| matches(e, k)),
            Some(t) => {
                let entries = &self.entries;
                t.find(hash_of(k), |&i| matches(&entries[i as usize].0, k)).map(|&i| i as usize)
            }
        }
    }

    #[inline]
    pub fn get(&self, k: &Key) -> Option<&Value> {
        self.find(key_ref(k)).map(|i| &self.entries[i].1)
    }

    #[inline]
    pub fn get_mut(&mut self, k: &Key) -> Option<&mut Value> {
        self.find(key_ref(k)).map(|i| &mut self.entries[i].1)
    }

    #[inline]
    pub fn contains_key(&self, k: &Key) -> bool {
        self.find(key_ref(k)).is_some()
    }

    #[inline]
    pub fn get_index(&self, i: usize) -> Option<(&Key, &Value)> {
        self.entries.get(i).map(|(k, v)| (k, v))
    }

    #[inline]
    pub fn get_index_mut(&mut self, i: usize) -> Option<(&Key, &mut Value)> {
        self.entries.get_mut(i).map(|(k, v)| (&*k, v))
    }

    /// Append a key known to be absent.
    pub fn insert_absent(&mut self, k: Key, v: Value) {
        self.push_new(k, v);
    }

    /// Append a key known to be absent.
    fn push_new(&mut self, k: Key, v: Value) -> usize {
        let i = self.entries.len();
        match &mut self.index {
            Some(t) => {
                let entries = &self.entries;
                t.insert_unique(hash_of(key_ref(&k)), i as u32, |&j| hash_of(key_ref(&entries[j as usize].0)));
            }
            None if i >= SCAN_MAX => {
                self.push_entry(k, v);
                self.build_index();
                return i;
            }
            None => {}
        }
        self.push_entry(k, v);
        i
    }

    /// `entries.push((k, v))` without building the pair on the stack: that
    /// copy reloads it with loads spanning separate stores, which stall
    /// store-to-load forwarding (a third of an insert's time on Apple M).
    #[inline(always)]
    fn push_entry(&mut self, k: Key, v: Value) {
        let i = self.entries.len();
        if i == self.entries.capacity() {
            self.entries.reserve(1);
        }
        // SAFETY: capacity > i; both fields are written before the length
        // covers the slot.
        unsafe {
            let p = self.entries.as_mut_ptr().add(i);
            std::ptr::addr_of_mut!((*p).0).write(k);
            std::ptr::addr_of_mut!((*p).1).write(v);
            self.entries.set_len(i + 1);
        }
    }

    fn build_index(&mut self) {
        let mut t = HashTable::with_capacity(self.entries.len().max(SCAN_MAX * 2));
        for (i, (k, _)) in self.entries.iter().enumerate() {
            let entries = &self.entries;
            t.insert_unique(hash_of(key_ref(k)), i as u32, |&j| hash_of(key_ref(&entries[j as usize].0)));
        }
        self.index = Some(t);
    }

    /// Set a key's value (appending it when new).
    #[inline]
    pub fn insert(&mut self, k: Key, v: Value) {
        match self.find(key_ref(&k)) {
            Some(i) => self.entries[i].1 = v,
            None => {
                self.push_new(k, v);
            }
        }
    }

    /// The value of a key, appended as null when missing.
    pub fn entry_or_default(&mut self, k: Key) -> &mut Value {
        let i = match self.find(key_ref(&k)) {
            Some(i) => i,
            None => self.push_new(k, Value::Null),
        };
        &mut self.entries[i].1
    }

    /// Remove a key, keeping the order of the rest.
    pub fn shift_remove(&mut self, k: &Key) -> Option<Value> {
        let i = self.find(key_ref(k))?;
        if let Some(t) = &mut self.index {
            let h = hash_of(key_ref(k));
            if let Ok(e) = t.find_entry(h, |&j| j as usize == i) {
                e.remove();
            }
            for j in t.iter_mut() {
                if *j as usize > i {
                    *j -= 1;
                }
            }
        }
        Some(self.entries.remove(i).1)
    }

    pub fn pop(&mut self) -> Option<(Key, Value)> {
        let (k, v) = self.entries.pop()?;
        if let Some(t) = &mut self.index {
            let last = self.entries.len();
            if let Ok(e) = t.find_entry(hash_of(key_ref(&k)), |&j| j as usize == last) {
                e.remove();
            }
        }
        Some((k, v))
    }

    pub fn keys(&self) -> impl DoubleEndedIterator<Item = &Key> + ExactSizeIterator {
        self.entries.iter().map(|(k, _)| k)
    }

    pub fn values(&self) -> Values<'_> {
        self.entries.iter().map(|(_, v)| v)
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut Value> {
        self.entries.iter_mut().map(|(_, v)| v)
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = (&Key, &Value)> + ExactSizeIterator {
        self.entries.iter().map(|(k, v)| (k, v))
    }
}

impl std::ops::Index<usize> for HMap {
    type Output = Value;
    #[inline]
    fn index(&self, i: usize) -> &Value {
        &self.entries[i].1
    }
}

impl IntoIterator for HMap {
    type Item = (Key, Value);
    type IntoIter = std::vec::IntoIter<(Key, Value)>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl FromIterator<(Key, Value)> for HMap {
    fn from_iter<I: IntoIterator<Item = (Key, Value)>>(it: I) -> HMap {
        let it = it.into_iter();
        let mut m = HMap::with_capacity(it.size_hint().0);
        for (k, v) in it {
            m.insert(k, v);
        }
        m
    }
}

impl HMap {
    /// From entries whose keys are known to be distinct (e.g. a packed
    /// array's 0..n).
    pub fn from_unique(entries: Vec<(Key, Value)>) -> HMap {
        let mut m = HMap { entries, index: None };
        if m.entries.len() > SCAN_MAX {
            m.build_index();
        }
        m
    }
}
