//! Memory accounting for `memory_limit`, `memory_get_usage()` and
//! `memory_get_peak_usage()`: a counting wrapper around the system
//! allocator. Parsed source (the AST, kept for the whole run like PHP's
//! opcache) is excluded from the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::Ordering::Relaxed;

pub struct Counting;

// Per-thread counters (no atomic read-modify-write on every allocation):
// the limit applies to the interpreter's thread, like PHP's per-request
// accounting. Const-initialized cells have no destructor, so they're safe
// to touch from the allocator at any point of a thread's life.
// Signed: a thread can free memory another thread allocated (it then goes
// negative, which counts as zero).
thread_local! {
    static USED_C: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static PEAK_C: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    /// Bytes not charged to the script (parsed code).
    static EXCLUDED_C: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}
// The limit, like the counters, is per thread (each server worker's
// request has its own memory_limit).
thread_local! {
    static LIMIT: Cell<usize> = const { Cell::new(usize::MAX) };
    /// memory_limit as configured (for messages).
    static CONFIGURED: Cell<usize> = const { Cell::new(usize::MAX) };
    static OVER: Cell<bool> = const { Cell::new(false) };
    /// Size of the allocation that crossed the limit.
    static TRIED: Cell<usize> = const { Cell::new(0) };
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[inline]
fn charge(size: usize) {
    let u = USED_C.with(|c| {
        let u = c.get() + size as isize;
        c.set(u);
        u
    });
    if u > PEAK_C.with(|p| p.get()) {
        PEAK_C.with(|p| p.set(u));
        let charged = (u - EXCLUDED_C.with(|e| e.get())).max(0) as usize;
        if charged > LIMIT.with(|l| l.get()) && !OVER.with(|o| o.get()) {
            OVER.with(|o| o.set(true));
            TRIED.with(|t| t.set(size));
            // Reported at the next statement boundary.
            crate::value::destruct_pending().store(true, Relaxed);
        }
    }
}

// Small blocks (most values: strings, arrays, objects) come from per-thread
// free lists of 16-byte size classes, carved from larger chunks, like PHP's
// zend_mm bins: no per-block header, no rounding beyond 16 bytes, and a
// push/pop instead of a general-purpose malloc. Freed blocks go to the
// freeing thread's list (memory is interchangeable between threads); chunks
// are kept for reuse, as PHP keeps its heap for the request.
const CLASSES: usize = 16;
const MAX_SMALL: usize = CLASSES * 16;
const CHUNK: usize = 32 * 1024;

thread_local! {
    /// Free-list heads, one per size class (one cell each: an alloc or free
    /// touches only its own).
    static FREE: [std::cell::Cell<*mut u8>; CLASSES] = const { [const { std::cell::Cell::new(std::ptr::null_mut()) }; CLASSES] };
}

#[inline]
fn class_of(l: &Layout) -> Option<usize> {
    // AddressSanitizer builds (RUSTFLAGS="--cfg phpx_asan -Zsanitizer=address")
    // hand every block to the system allocator, which ASan instruments.
    if cfg!(phpx_asan) {
        return None;
    }
    (l.size() <= MAX_SMALL && l.align() <= 16).then(|| (l.size().max(1) + 15) / 16 - 1)
}

#[inline]
unsafe fn small_alloc(c: usize) -> *mut u8 {
    FREE.with(|f| {
        let head = &f[c];
        let mut p = head.get();
        if p.is_null() {
            p = refill(c);
            if p.is_null() {
                return p;
            }
        }
        head.set(*(p as *mut *mut u8));
        p
    })
}

/// Carve a new chunk into blocks of class `c`; returns the list head.
#[cold]
unsafe fn refill(c: usize) -> *mut u8 {
    let size = (c + 1) * 16;
    let base = System.alloc(Layout::from_size_align_unchecked(CHUNK, 16));
    if base.is_null() {
        return base;
    }
    let n = CHUNK / size;
    for i in 0..n {
        let b = base.add(i * size);
        let next = if i + 1 < n { base.add((i + 1) * size) } else { std::ptr::null_mut() };
        *(b as *mut *mut u8) = next;
    }
    base
}

#[inline]
unsafe fn small_free(p: *mut u8, c: usize) {
    FREE.with(|f| {
        let head = &f[c];
        *(p as *mut *mut u8) = head.get();
        head.set(p);
    })
}

/// Bytes a block really takes (small blocks: their size class).
#[inline]
fn block_size(l: &Layout) -> usize {
    match class_of(l) {
        Some(c) => (c + 1) * 16,
        None => l.size(),
    }
}

#[inline]
fn uncharge(size: usize) {
    USED_C.with(|c| c.set(c.get() - size as isize));
}

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = match class_of(&l) {
            Some(c) => small_alloc(c),
            None => System.alloc(l),
        };
        if !p.is_null() {
            charge(block_size(&l));
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = match class_of(&l) {
            Some(c) => {
                let p = small_alloc(c);
                if !p.is_null() {
                    std::ptr::write_bytes(p, 0, l.size());
                }
                p
            }
            None => System.alloc_zeroed(l),
        };
        if !p.is_null() {
            charge(block_size(&l));
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        match class_of(&l) {
            Some(c) => small_free(p, c),
            None => System.dealloc(p, l),
        }
        uncharge(block_size(&l));
    }

    #[inline]
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let nl = Layout::from_size_align_unchecked(new, l.align());
        match (class_of(&l), class_of(&nl)) {
            // Still fits its block.
            (Some(a), Some(b)) if a == b => p,
            (None, None) => {
                let q = System.realloc(p, l, new);
                if !q.is_null() {
                    if new >= l.size() {
                        charge(new - l.size());
                    } else {
                        uncharge(l.size() - new);
                    }
                }
                q
            }
            _ => {
                // Moving between a size class and the system allocator
                // (alloc and dealloc do the accounting).
                let q = self.alloc(nl);
                if !q.is_null() {
                    std::ptr::copy_nonoverlapping(p, q, l.size().min(new));
                    self.dealloc(p, l);
                }
                q
            }
        }
    }
}

/// The limit was crossed and not yet reported (code that produces a lot
/// of data in one go, like var_dump, stops early).
pub fn over() -> bool {
    OVER.with(|o| o.get())
}

/// Bytes charged to the script now.
fn raw(c: &'static std::thread::LocalKey<std::cell::Cell<isize>>) -> isize {
    c.with(|c| c.get())
}

pub fn used() -> usize {
    (raw(&USED_C) - raw(&EXCLUDED_C)).max(0) as usize
}

pub fn peak() -> usize {
    (raw(&PEAK_C) - raw(&EXCLUDED_C)).max(0) as usize
}

/// Raw allocator total on this thread (for measuring what to exclude).
pub fn raw_used() -> usize {
    raw(&USED_C).max(0) as usize
}

pub fn exclude(bytes: usize) {
    EXCLUDED_C.with(|e| e.set(e.get() + bytes as isize));
}

/// Run `f` (the native compiler) outside the script's accounting: what
/// it allocates, keeps or frees is excluded, and its peak doesn't count.
pub fn uncharged<T>(f: impl FnOnce() -> T) -> T {
    let (used, peak) = (raw(&USED_C), raw(&PEAK_C));
    // Excluding everything up front keeps the limit from tripping meanwhile.
    let headroom = isize::MAX / 4;
    EXCLUDED_C.with(|e| e.set(e.get() + headroom));
    let r = f();
    let grown = raw(&USED_C) - used;
    EXCLUDED_C.with(|e| e.set(e.get() - headroom + grown));
    PEAK_C.with(|p| p.set(peak.max(raw(&USED_C))));
    r
}

/// (configured, enforced); None = unlimited.
pub fn set_limit(limit: Option<(usize, usize)>) {
    CONFIGURED.with(|c| c.set(limit.map_or(usize::MAX, |l| l.0)));
    LIMIT.with(|c| c.set(limit.map_or(usize::MAX, |l| l.1)));
    if limit.is_none_or(|l| used() <= l.1) {
        OVER.with(|o| o.set(false));
    }
}

/// The limit was crossed: Some(size of the allocation that crossed it).
/// Clears the flag and lifts the limit so shutdown code can run (PHP does
/// the same).
pub fn take_over() -> Option<(usize, usize)> {
    if !OVER.with(|o| o.replace(false)) {
        return None;
    }
    LIMIT.with(|l| l.set(usize::MAX));
    Some((CONFIGURED.with(|c| c.get()), TRIED.with(|t| t.get())))
}

/// Would allocating `bytes` more exceed the limit? Some(limit) if so.
/// Built-ins that allocate on behalf of the script (str_repeat, ...) check
/// first, so a huge request fails cleanly instead of exhausting the host.
pub fn would_exceed(bytes: usize) -> Option<usize> {
    let l = LIMIT.with(|l| l.get());
    (l != usize::MAX && used().saturating_add(bytes) > l).then(|| CONFIGURED.with(|c| c.get()))
}

/// "128M" / "1G" / "512K" / "-1" / bytes, like PHP's ini quantity parsing.
pub fn parse_quantity(s: &str) -> Option<Option<usize>> {
    let s = s.trim();
    if s == "-1" {
        return Some(None);
    }
    let (num, mul) = match s.chars().last()? {
        'k' | 'K' => (&s[..s.len() - 1], 1024usize),
        'm' | 'M' => (&s[..s.len() - 1], 1024 * 1024),
        'g' | 'G' => (&s[..s.len() - 1], 1024 * 1024 * 1024),
        _ => (s, 1),
    };
    let n: i64 = num.trim().parse().ok()?;
    if n < 0 {
        return Some(None);
    }
    Some(Some((n as usize).saturating_mul(mul)))
}
