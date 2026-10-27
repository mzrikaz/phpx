//! Thread-locals touched on every call, with a copy outside thread-local
//! storage for the main thread.
//!
//! On macOS every thread-local access is a call to `_tlv_get_addr`, a large
//! part of a short PHP call. The first thread to claim it (the main thread,
//! which runs the interpreter: the allocator claims it on its first
//! allocation) uses plain statics, found by comparing the thread pointer
//! (one instruction); other threads use the thread-local. Elsewhere,
//! thread-locals are cheap and always used.

use std::thread::LocalKey;

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
static OWNER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[inline(always)]
fn thread_pointer() -> usize {
    let tp: usize;
    // SAFETY: reads the thread's TSD base (low bits may hold a CPU number
    // on some kernels: masked off).
    unsafe { std::arch::asm!("mrs {}, tpidrro_el0", out(reg) tp, options(nomem, nostack, preserves_flags)) };
    tp & !7
}

/// Whether this is the main thread, claiming that role for this thread if
/// no thread has it yet.
#[inline(always)]
pub fn claim_main() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        use std::sync::atomic::Ordering::Relaxed;
        let me = thread_pointer();
        let owner = OWNER.load(Relaxed);
        owner == me || (owner == 0 && OWNER.compare_exchange(0, me, Relaxed, Relaxed).is_ok())
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    false
}

/// Whether this is the thread that claimed the main-thread statics.
#[inline(always)]
pub fn is_main() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        OWNER.load(std::sync::atomic::Ordering::Relaxed) == thread_pointer()
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    false
}

/// A thread-local value with a main-thread copy (see the module docs).
pub struct FastLocal<T: 'static> {
    main: T,
    key: &'static LocalKey<T>,
}

// SAFETY: `main` is only touched by the main thread (`is_main`).
unsafe impl<T> Sync for FastLocal<T> {}

impl<T: 'static> FastLocal<T> {
    /// `main` is the main thread's initial value; `key` the thread-local
    /// other threads use (const-initialized the same way).
    pub const fn new(main: T, key: &'static LocalKey<T>) -> FastLocal<T> {
        FastLocal { main, key }
    }

    #[inline(always)]
    pub fn with<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        if is_main() {
            return f(&self.main);
        }
        self.key.with(f)
    }

    /// Like `LocalKey::try_with` (fails only for another thread's copy,
    /// during or after its destruction).
    #[inline(always)]
    pub fn try_with<R>(&'static self, f: impl FnOnce(&T) -> R) -> Result<R, std::thread::AccessError> {
        if is_main() {
            return Ok(f(&self.main));
        }
        self.key.try_with(f)
    }
}

/// `static NAME: FastLocal<T> = init;` with its thread-local twin.
#[macro_export]
macro_rules! fast_local {
    ($(#[$m:meta])* $vis:vis static $name:ident: $t:ty = $init:expr;) => {
        $(#[$m])* $vis static $name: $crate::fastlocal::FastLocal<$t> = {
            ::std::thread_local! { static TLS: $t = const { $init }; }
            $crate::fastlocal::FastLocal::new($init, &TLS)
        };
    };
}
