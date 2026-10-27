//! PHP byte strings: one reference-counted allocation holding a header and
//! the bytes inline (like PHP's `zend_string`), instead of `Rc<Vec<u8>>`'s
//! two allocations and extra indirection.
//!
//! Layout (`repr(C)`): the strong count is the first word, so generated
//! code can update it inline exactly as it does for `Rc` values.

use std::alloc::{alloc, dealloc, realloc, Layout};
use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::ptr::NonNull;

#[repr(C)]
struct Header {
    strong: usize,
    len: usize,
    cap: usize,
}

const HDR: usize = std::mem::size_of::<Header>();

pub struct PStr {
    ptr: NonNull<Header>,
}

fn layout(cap: usize) -> Layout {
    Layout::from_size_align(HDR + cap, std::mem::align_of::<Header>()).expect("string too large")
}

impl PStr {
    pub fn with_capacity(cap: usize) -> PStr {
        // SAFETY: non-zero size layout; header initialized before use.
        unsafe {
            let p = alloc(layout(cap)) as *mut Header;
            let Some(ptr) = NonNull::new(p) else { std::alloc::handle_alloc_error(layout(cap)) };
            ptr.as_ptr().write(Header { strong: 1, len: 0, cap });
            PStr { ptr }
        }
    }

    pub fn from_bytes(b: &[u8]) -> PStr {
        let mut s = PStr::with_capacity(b.len());
        // SAFETY: capacity reserved above.
        unsafe {
            std::ptr::copy_nonoverlapping(b.as_ptr(), s.data_mut(), b.len());
            (*s.ptr.as_ptr()).len = b.len();
        }
        s
    }

    #[inline]
    fn hdr(&self) -> &Header {
        // SAFETY: valid while self lives.
        unsafe { self.ptr.as_ref() }
    }

    #[inline]
    fn data(&self) -> *const u8 {
        // SAFETY: bytes follow the header in the same allocation.
        unsafe { (self.ptr.as_ptr() as *const u8).add(HDR) }
    }

    #[inline]
    fn data_mut(&mut self) -> *mut u8 {
        unsafe { (self.ptr.as_ptr() as *mut u8).add(HDR) }
    }

    /// Bytes allocated for the content (at least `len()`).
    pub fn capacity(&self) -> usize {
        self.hdr().cap
    }

    pub fn strong_count(s: &PStr) -> usize {
        s.hdr().strong
    }

    pub fn ptr_eq(a: &PStr, b: &PStr) -> bool {
        a.ptr == b.ptr
    }

    /// Unique access for in-place edits (`.=`): copies first if shared.
    fn make_unique(&mut self, extra: usize) {
        let len = self.len();
        if self.hdr().strong != 1 {
            let mut n = PStr::with_capacity((len + extra).max(len * 2).max(16));
            // SAFETY: capacity >= len.
            unsafe {
                std::ptr::copy_nonoverlapping(self.data(), n.data_mut(), len);
                (*n.ptr.as_ptr()).len = len;
            }
            *self = n;
            return;
        }
        let cap = self.hdr().cap;
        if len + extra > cap {
            let new_cap = (len + extra).max(cap * 2).max(16);
            // SAFETY: unique owner; realloc keeps the header and bytes.
            unsafe {
                let p = realloc(self.ptr.as_ptr() as *mut u8, layout(cap), HDR + new_cap) as *mut Header;
                let Some(ptr) = NonNull::new(p) else { std::alloc::handle_alloc_error(layout(new_cap)) };
                (*ptr.as_ptr()).cap = new_cap;
                self.ptr = ptr;
            }
        }
    }

    /// Append in place (amortized O(1) when unshared).
    pub fn push_bytes(&mut self, b: &[u8]) {
        self.make_unique(b.len());
        let len = self.len();
        // SAFETY: make_unique reserved room for b.
        unsafe {
            std::ptr::copy_nonoverlapping(b.as_ptr(), self.data_mut().add(len), b.len());
            (*self.ptr.as_ptr()).len = len + b.len();
        }
    }

    /// Mutable bytes (copy first if shared).
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        self.make_unique(0);
        let len = self.len();
        // SAFETY: unique owner.
        unsafe { std::slice::from_raw_parts_mut(self.data_mut(), len) }
    }

    /// Append an integer's decimal digits (no formatter machinery).
    pub fn push_int(&mut self, n: i64) {
        let mut buf = [0u8; 20];
        let s = fmt_int(n, &mut buf);
        self.push_bytes(s);
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: len bytes are initialized.
        unsafe { std::slice::from_raw_parts(self.data(), self.hdr().len) }
    }
}

/// Decimal digits of `n` written into `buf` (fast path for int-to-string).
pub fn fmt_int(n: i64, buf: &mut [u8; 20]) -> &[u8] {
    let mut i = buf.len();
    let neg = n < 0;
    let mut u = n.unsigned_abs();
    loop {
        i -= 1;
        buf[i] = b'0' + (u % 10) as u8;
        u /= 10;
        if u == 0 {
            break;
        }
    }
    if neg {
        i -= 1;
        buf[i] = b'-';
    }
    &buf[i..]
}

/// Strong count of an immortal string: never reaches zero (clones and
/// drops move it by one), so it is never freed or changed in place.
const IMMORTAL: usize = usize::MAX / 2;

impl PStr {
    /// The literal cached in `cache` (a string literal's slot in the syntax
    /// tree), made immortal on first use.
    #[inline]
    pub fn literal(cache: &std::cell::Cell<usize>, s: &str) -> PStr {
        let p = cache.get();
        if p != 0 {
            // SAFETY: an immortal header leaked below; the count is bumped.
            unsafe {
                let h = p as *mut Header;
                (*h).strong += 1;
                return PStr { ptr: NonNull::new_unchecked(h) };
            }
        }
        let v = PStr::from_bytes(s.as_bytes());
        // SAFETY: we own the only reference.
        unsafe { (*v.ptr.as_ptr()).strong = IMMORTAL };
        cache.set(v.ptr.as_ptr() as usize);
        v
    }
}

impl Drop for PStr {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: we own one strong reference.
        unsafe {
            let h = self.ptr.as_ptr();
            (*h).strong -= 1;
            if (*h).strong == 0 {
                dealloc(h as *mut u8, layout((*h).cap));
            }
        }
    }
}

impl Clone for PStr {
    #[inline]
    fn clone(&self) -> PStr {
        // SAFETY: single-threaded refcount.
        unsafe { (*self.ptr.as_ptr()).strong += 1 };
        PStr { ptr: self.ptr }
    }
}

impl Deref for PStr {
    type Target = [u8];
    #[inline]
    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for PStr {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Borrow<[u8]> for PStr {
    fn borrow(&self) -> &[u8] {
        self.as_slice()
    }
}

impl PartialEq for PStr {
    fn eq(&self, o: &PStr) -> bool {
        self.ptr == o.ptr || self.as_slice() == o.as_slice()
    }
}

impl Eq for PStr {}

impl PartialOrd for PStr {
    fn partial_cmp(&self, o: &PStr) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for PStr {
    fn cmp(&self, o: &PStr) -> Ordering {
        self.as_slice().cmp(o.as_slice())
    }
}

impl Hash for PStr {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.as_slice().hash(h)
    }
}

impl fmt::Debug for PStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", crate::pstr::lossy(self))
    }
}

impl From<Vec<u8>> for PStr {
    fn from(v: Vec<u8>) -> PStr {
        PStr::from_bytes(&v)
    }
}

impl From<&[u8]> for PStr {
    fn from(v: &[u8]) -> PStr {
        PStr::from_bytes(v)
    }
}

impl Default for PStr {
    fn default() -> PStr {
        PStr::with_capacity(0)
    }
}

#[cfg(test)]
mod tests {
    use super::PStr;

    #[test]
    fn grow_share_drop() {
        let mut a = PStr::from_bytes(b"ab");
        let b = a.clone();
        assert_eq!(PStr::strong_count(&a), 2);
        a.push_bytes(b"cd");
        assert_eq!(&*a, b"abcd");
        assert_eq!(&*b, b"ab");
        for _ in 0..100 {
            a.push_bytes(b"x");
        }
        assert_eq!(a.len(), 104);
        a.bytes_mut()[0] = b'Z';
        assert_eq!(a[0], b'Z');
    }
}

/// `String::from_utf8_lossy` with a fast check first: valid UTF-8 (nearly
/// every string) is validated word at a time instead of chunk by chunk.
#[inline]
pub fn lossy(b: &[u8]) -> std::borrow::Cow<'_, str> {
    match std::str::from_utf8(b) {
        Ok(s) => std::borrow::Cow::Borrowed(s),
        Err(_) => String::from_utf8_lossy(b),
    }
}
