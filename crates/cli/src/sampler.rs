//! In-process sampling profiler (PHPX_PROFILE): a SIGPROF timer whose
//! handler records the interrupted instruction and the frame-pointer chain
//! above it. (pprof's frame-pointer walk starts at the caller's return
//! address, so every sample lost its leaf function.) Builds with frame
//! pointers (`-C force-frame-pointers=yes`) walk Rust and generated code
//! alike.

use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::Relaxed};

/// Frames kept per sample (leaf first).
const DEPTH: usize = 96;
/// Samples kept: about 4 minutes of CPU time at 1 kHz.
const MAX_SAMPLES: usize = 250_000;
const STRIDE: usize = DEPTH + 1;

static BUF: AtomicPtr<usize> = AtomicPtr::new(std::ptr::null_mut());
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// Start sampling every millisecond of CPU time (any thread).
pub fn start() {
    // Zeroed pages are only touched as samples arrive.
    let buf: &'static mut [usize] = vec![0usize; MAX_SAMPLES * STRIDE].leak();
    BUF.store(buf.as_mut_ptr(), Relaxed);
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigprof as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGPROF, &sa, std::ptr::null_mut());
        let tick = libc::timeval { tv_sec: 0, tv_usec: 1000 };
        let t = libc::itimerval { it_interval: tick, it_value: tick };
        libc::setitimer(libc::ITIMER_PROF, &t, std::ptr::null_mut());
    }
}

/// Stop sampling; the stacks recorded (leaf first: the interrupted
/// instruction, then return addresses).
pub fn stop() -> Vec<&'static [usize]> {
    unsafe {
        let zero = libc::timeval { tv_sec: 0, tv_usec: 0 };
        let t = libc::itimerval { it_interval: zero, it_value: zero };
        libc::setitimer(libc::ITIMER_PROF, &t, std::ptr::null_mut());
        libc::signal(libc::SIGPROF, libc::SIG_IGN);
    }
    let buf = BUF.load(Relaxed);
    if buf.is_null() {
        return vec![];
    }
    let n = NEXT.load(Relaxed).min(MAX_SAMPLES);
    (0..n)
        .map(|i| unsafe {
            let s = buf.add(i * STRIDE);
            std::slice::from_raw_parts(s.add(1), *s)
        })
        .collect()
}

/// Whether `addr..addr+16` can be read (a frame record).
fn readable(addr: usize) -> bool {
    addr % 8 == 0 && pprof::validate(addr as *const libc::c_void) && ((addr & 4095) <= 4080 || pprof::validate((addr + 8) as *const libc::c_void))
}

extern "C" fn on_sigprof(_: libc::c_int, _: *mut libc::siginfo_t, uc: *mut libc::c_void) {
    let buf = BUF.load(Relaxed);
    if buf.is_null() || uc.is_null() {
        return;
    }
    let i = NEXT.fetch_add(1, Relaxed);
    if i >= MAX_SAMPLES {
        return;
    }
    unsafe {
        let uc = &*(uc as *const libc::ucontext_t);
        let rip = uc.uc_mcontext.gregs[libc::REG_RIP as usize] as usize;
        let mut fp = uc.uc_mcontext.gregs[libc::REG_RBP as usize] as usize;
        let s = buf.add(i * STRIDE);
        *s.add(1) = rip;
        let mut n = 1;
        while n < DEPTH && fp != 0 && readable(fp) {
            let next = *(fp as *const usize);
            let ret = *((fp + 8) as *const usize);
            if ret == 0 {
                break;
            }
            *s.add(1 + n) = ret;
            n += 1;
            // Stacks grow down: callers' records are at higher addresses.
            if next <= fp {
                break;
            }
            fp = next;
        }
        *s = n;
    }
}
