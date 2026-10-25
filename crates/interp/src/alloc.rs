//! Memory accounting for `memory_limit`, `memory_get_usage()` and
//! `memory_get_peak_usage()`: a counting wrapper around the system
//! allocator. Parsed source (the AST, kept for the whole run like PHP's
//! opcache) is excluded from the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

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
static LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);
/// memory_limit as configured (for messages).
static CONFIGURED: AtomicUsize = AtomicUsize::new(usize::MAX);
static OVER: AtomicBool = AtomicBool::new(false);
/// Size of the allocation that crossed the limit.
static TRIED: AtomicUsize = AtomicUsize::new(0);

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
        if charged > LIMIT.load(Relaxed) && !OVER.load(Relaxed) {
            OVER.store(true, Relaxed);
            TRIED.store(size, Relaxed);
            // Reported at the next statement boundary.
            crate::value::DESTRUCT_PENDING.store(true, Relaxed);
        }
    }
}

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            charge(l.size());
        }
        p
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() {
            charge(l.size());
        }
        p
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        USED_C.with(|c| c.set(c.get() - l.size() as isize));
    }

    #[inline]
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let q = System.realloc(p, l, new);
        if !q.is_null() {
            if new >= l.size() {
                charge(new - l.size());
            } else {
                USED_C.with(|c| c.set(c.get() - (l.size() - new) as isize));
            }
        }
        q
    }
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

/// (configured, enforced); None = unlimited.
pub fn set_limit(limit: Option<(usize, usize)>) {
    CONFIGURED.store(limit.map_or(usize::MAX, |l| l.0), Relaxed);
    LIMIT.store(limit.map_or(usize::MAX, |l| l.1), Relaxed);
    if limit.is_none_or(|l| used() <= l.1) {
        OVER.store(false, Relaxed);
    }
}

/// The limit was crossed: Some(size of the allocation that crossed it).
/// Clears the flag and lifts the limit so shutdown code can run (PHP does
/// the same).
pub fn take_over() -> Option<(usize, usize)> {
    if !OVER.swap(false, Relaxed) {
        return None;
    }
    LIMIT.store(usize::MAX, Relaxed);
    Some((CONFIGURED.load(Relaxed), TRIED.load(Relaxed)))
}

/// Would allocating `bytes` more exceed the limit? Some(limit) if so.
/// Built-ins that allocate on behalf of the script (str_repeat, ...) check
/// first, so a huge request fails cleanly instead of exhausting the host.
pub fn would_exceed(bytes: usize) -> Option<usize> {
    let l = LIMIT.load(Relaxed);
    (l != usize::MAX && used().saturating_add(bytes) > l).then(|| CONFIGURED.load(Relaxed))
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
