//! max_execution_time: a watcher thread measures the process CPU time (like
//! PHP on Linux, time spent sleeping or waiting on I/O doesn't count) and
//! raises a flag; the script stops at the next statement boundary with
//! PHP's fatal error.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::Duration;

/// Limit in milliseconds (0 = none) and the CPU time when it was set.
static LIMIT_MS: AtomicU64 = AtomicU64::new(0);
static START_NS: AtomicU64 = AtomicU64::new(0);
static EXPIRED: AtomicBool = AtomicBool::new(false);
/// Pid of the process whose watcher thread is running (a forked child has
/// none: fork keeps only the calling thread).
static WATCHER_PID: AtomicU32 = AtomicU32::new(0);

fn cpu_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: valid pointer to a timespec.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// (Re)start the timer: set_time_limit() semantics. 0 disables it.
pub fn set(seconds: i64) {
    EXPIRED.store(false, Relaxed);
    START_NS.store(cpu_ns(), Relaxed);
    LIMIT_MS.store(seconds.max(0) as u64 * 1000, Relaxed);
    if seconds > 0 {
        let pid = std::process::id();
        if WATCHER_PID.swap(pid, Relaxed) != pid {
            std::thread::Builder::new()
                .name("phpx-timeout".into())
                .spawn(|| loop {
                    std::thread::sleep(Duration::from_millis(50));
                    let lim = LIMIT_MS.load(Relaxed);
                    if lim > 0 && !EXPIRED.load(Relaxed) {
                        let used = cpu_ns().saturating_sub(START_NS.load(Relaxed)) / 1_000_000;
                        if used >= lim {
                            EXPIRED.store(true, Relaxed);
                            if let Ok(all) = crate::value::ALL_PENDING.lock() {
                                all.iter().for_each(|f| f.store(true, Relaxed));
                            }
                        }
                    }
                })
                .expect("spawn timeout watcher");
        }
    }
}

/// Held across fork(): the watcher thread can't be inside ALL_PENDING
/// then, so the child doesn't inherit a lock no thread will release.
pub fn fork_guard() -> std::sync::MutexGuard<'static, Vec<&'static AtomicBool>> {
    crate::value::ALL_PENDING.lock().unwrap_or_else(|e| e.into_inner())
}

/// In a forked child: the limit restarts against the child's own CPU
/// clock, with a watcher thread of its own.
pub fn after_fork() {
    let lim = LIMIT_MS.load(Relaxed);
    if lim > 0 {
        set((lim / 1000) as i64);
    } else {
        START_NS.store(cpu_ns(), Relaxed);
    }
}

/// Some(limit in seconds) once expired; disarms so shutdown code can run.
pub fn take_expired() -> Option<u64> {
    if !EXPIRED.swap(false, Relaxed) {
        return None;
    }
    let lim = LIMIT_MS.swap(0, Relaxed);
    Some(lim / 1000)
}
