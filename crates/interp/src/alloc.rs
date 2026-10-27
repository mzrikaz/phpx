//! Memory accounting for `memory_limit`, `memory_get_usage()` and
//! `memory_get_peak_usage()`: a counting wrapper around the system
//! allocator. Parsed source (the AST, kept for the whole run like PHP's
//! opcache) is excluded from the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::Ordering::Relaxed;

pub struct Counting;

// Small blocks (most values: strings, arrays, objects) come from per-thread
// free lists of 16-byte size classes, carved from larger chunks, like PHP's
// zend_mm bins: no per-block header, no rounding beyond 16 bytes, and a
// push/pop instead of a general-purpose malloc. Freed blocks go to the
// freeing thread's list (memory is interchangeable between threads); chunks
// are kept for reuse, as PHP keeps its heap for the request.
const CLASSES: usize = 16;
const MAX_SMALL: usize = CLASSES * 16;
const CHUNK: usize = 32 * 1024;

/// A thread's allocator state, in one thread-local so an allocation looks
/// it up once (on macOS each thread-local access is a call).
///
/// The counters are per thread (no atomic read-modify-write on every
/// allocation), and so is the limit: it applies to the interpreter's
/// thread, like PHP's per-request accounting (each server worker's request
/// has its own memory_limit). Signed: a thread can free memory another
/// thread allocated (it then goes negative, which counts as zero).
/// Const-initialized, with no destructor, so it's safe to touch from the
/// allocator at any point of a thread's life.
struct Heap {
    /// Free-list heads, one per size class.
    free: [Cell<*mut u8>; CLASSES],
    used: Cell<isize>,
    peak: Cell<isize>,
    /// Bytes not charged to the script (parsed code).
    excluded: Cell<isize>,
    limit: Cell<usize>,
    /// memory_limit as configured (for messages).
    configured: Cell<usize>,
    over: Cell<bool>,
    /// Size of the allocation that crossed the limit.
    tried: Cell<usize>,
}

thread_local! {
    static HEAP: Heap = const {
        Heap {
            free: [const { Cell::new(std::ptr::null_mut()) }; CLASSES],
            used: Cell::new(0),
            peak: Cell::new(0),
            excluded: Cell::new(0),
            limit: Cell::new(usize::MAX),
            configured: Cell::new(usize::MAX),
            over: Cell::new(false),
            tried: Cell::new(0),
        }
    };
}

/// The heap of the first thread that allocates (the main thread, which runs
/// the interpreter), outside thread-local storage: on macOS every
/// thread-local access is a call to `_tlv_get_addr`, a large part of an
/// allocation. That thread is recognized by its thread pointer (one
/// instruction); other threads use `HEAP`.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
struct MainHeap(Heap);

// SAFETY: only the main thread (phpx_runtime::fastlocal) touches it.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
unsafe impl Sync for MainHeap {}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
static MAIN_HEAP: MainHeap = MainHeap(Heap {
    free: [const { Cell::new(std::ptr::null_mut()) }; CLASSES],
    used: Cell::new(0),
    peak: Cell::new(0),
    excluded: Cell::new(0),
    limit: Cell::new(usize::MAX),
    configured: Cell::new(usize::MAX),
    over: Cell::new(false),
    tried: Cell::new(0),
});

/// The calling thread's heap.
#[inline(always)]
fn with_heap<T>(f: impl FnOnce(&Heap) -> T) -> T {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if phpx_runtime::fastlocal::claim_main() {
        return f(&MAIN_HEAP.0);
    }
    HEAP.with(f)
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[inline]
fn charge(h: &Heap, size: usize) {
    let u = h.used.get() + size as isize;
    h.used.set(u);
    if u > h.peak.get() {
        h.peak.set(u);
        let charged = (u - h.excluded.get()).max(0) as usize;
        if charged > h.limit.get() && !h.over.get() {
            h.over.set(true);
            h.tried.set(size);
            // Reported at the next statement boundary.
            crate::value::destruct_pending().store(true, Relaxed);
        }
    }
}

#[inline]
fn uncharge(h: &Heap, size: usize) {
    h.used.set(h.used.get() - size as isize);
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
unsafe fn small_alloc(h: &Heap, c: usize) -> *mut u8 {
    let head = &h.free[c];
    let mut p = head.get();
    if p.is_null() {
        p = refill(c);
        if p.is_null() {
            return p;
        }
    }
    head.set(*(p as *mut *mut u8));
    p
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
unsafe fn small_free(h: &Heap, p: *mut u8, c: usize) {
    let head = &h.free[c];
    *(p as *mut *mut u8) = head.get();
    head.set(p);
}

/// Bytes a block really takes (small blocks: their size class).
#[inline]
fn block_size(l: &Layout) -> usize {
    match class_of(l) {
        Some(c) => (c + 1) * 16,
        None => l.size(),
    }
}

unsafe impl GlobalAlloc for Counting {
    #[inline]
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        with_heap(|h| {
            let p = match class_of(&l) {
                Some(c) => small_alloc(h, c),
                None => System.alloc(l),
            };
            if !p.is_null() {
                charge(h, block_size(&l));
            }
            p
        })
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        with_heap(|h| {
            let p = match class_of(&l) {
                Some(c) => {
                    let p = small_alloc(h, c);
                    if !p.is_null() {
                        std::ptr::write_bytes(p, 0, l.size());
                    }
                    p
                }
                None => System.alloc_zeroed(l),
            };
            if !p.is_null() {
                charge(h, block_size(&l));
            }
            p
        })
    }

    #[inline]
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        with_heap(|h| {
            match class_of(&l) {
                Some(c) => small_free(h, p, c),
                None => System.dealloc(p, l),
            }
            uncharge(h, block_size(&l));
        })
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
                    with_heap(|h| {
                        if new >= l.size() {
                            charge(h, new - l.size());
                        } else {
                            uncharge(h, l.size() - new);
                        }
                    });
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
    with_heap(|h| h.over.get())
}

/// Bytes charged to the script now.
pub fn used() -> usize {
    with_heap(|h| (h.used.get() - h.excluded.get()).max(0) as usize)
}

pub fn peak() -> usize {
    with_heap(|h| (h.peak.get() - h.excluded.get()).max(0) as usize)
}

/// Raw allocator total on this thread (for measuring what to exclude).
pub fn raw_used() -> usize {
    with_heap(|h| h.used.get().max(0) as usize)
}

pub fn exclude(bytes: usize) {
    with_heap(|h| h.excluded.set(h.excluded.get() + bytes as isize));
}

/// Run `f` (the native compiler) outside the script's accounting: what
/// it allocates, keeps or frees is excluded, and its peak doesn't count.
pub fn uncharged<T>(f: impl FnOnce() -> T) -> T {
    let (used, peak) = with_heap(|h| (h.used.get(), h.peak.get()));
    // Excluding everything up front keeps the limit from tripping meanwhile.
    let headroom = isize::MAX / 4;
    with_heap(|h| h.excluded.set(h.excluded.get() + headroom));
    let r = f();
    with_heap(|h| {
        let grown = h.used.get() - used;
        h.excluded.set(h.excluded.get() - headroom + grown);
        h.peak.set(peak.max(h.used.get()));
    });
    r
}

/// (configured, enforced); None = unlimited.
pub fn set_limit(limit: Option<(usize, usize)>) {
    with_heap(|h| {
        h.configured.set(limit.map_or(usize::MAX, |l| l.0));
        h.limit.set(limit.map_or(usize::MAX, |l| l.1));
    });
    if limit.is_none_or(|l| used() <= l.1) {
        with_heap(|h| h.over.set(false));
    }
}

/// The limit was crossed: Some(size of the allocation that crossed it).
/// Clears the flag and lifts the limit so shutdown code can run (PHP does
/// the same).
pub fn take_over() -> Option<(usize, usize)> {
    with_heap(|h| {
        if !h.over.replace(false) {
            return None;
        }
        h.limit.set(usize::MAX);
        Some((h.configured.get(), h.tried.get()))
    })
}

/// Would allocating `bytes` more exceed the limit? Some(limit) if so.
/// Built-ins that allocate on behalf of the script (str_repeat, ...) check
/// first, so a huge request fails cleanly instead of exhausting the host.
pub fn would_exceed(bytes: usize) -> Option<usize> {
    let (l, configured) = with_heap(|h| (h.limit.get(), h.configured.get()));
    (l != usize::MAX && used().saturating_add(bytes) > l).then_some(configured)
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
