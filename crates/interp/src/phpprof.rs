//! PHP-level sampling profiler: PHPX_PROFILE_PHP=path samples the PHP
//! call stack every millisecond of CPU time (SIGPROF) and, when the script
//! ends, prints the hottest PHP functions (self and inclusive) and writes
//! the stacks to `path` in folded form (flamegraph input, leaf last).
//!
//! The signal handler only copies names out of the live call records into
//! a buffer allocated up front, so it neither allocates nor locks.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::trace::CallRec;

const BUF: usize = 128 << 20;
const MAX_DEPTH: usize = 48;
const MAX_NAME: usize = 120;

static mut DATA: *mut u8 = std::ptr::null_mut();
static USED: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicUsize = AtomicUsize::new(0);

fn put(at: &mut usize, b: &[u8]) -> bool {
    if *at + b.len() > BUF {
        return false;
    }
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), DATA.add(*at), b.len()) };
    *at += b.len();
    true
}

/// The name of one record's function, as up to two parts (class, name).
unsafe fn rec_name(r: &CallRec) -> (&[u8], &[u8]) {
    if !r.spec.is_null() {
        return (b"", (*r.spec).display.as_bytes());
    }
    let Some(f) = r.frame.as_ref() else { return (b"", b"?") };
    let class = f.class.as_ref().map_or(&b""[..], |c| c.name.as_bytes());
    let func = if f.func.is_empty() { &b"{main}"[..] } else { f.func.as_bytes() };
    (class, func)
}

extern "C" fn on_sample(_: libc::c_int) {
    let start = USED.load(Ordering::Relaxed);
    let mut at = start;
    // A sample: one depth byte, then per frame (innermost first) a length
    // byte and the name.
    let mut frames: [(&[u8], &[u8], bool); MAX_DEPTH] = [(b"", b"", false); MAX_DEPTH];
    let mut n = 0;
    let mut r = crate::trace::top();
    while let Some(rec) = unsafe { r.as_ref() } {
        if n == MAX_DEPTH {
            break;
        }
        let (class, func) = unsafe { rec_name(rec) };
        frames[n] = (class, func, rec.spec.is_null());
        n += 1;
        r = rec.prev;
    }
    if !put(&mut at, &[n as u8]) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    for (class, func, interp) in &frames[..n] {
        let sep: &[u8] = if class.is_empty() { b"" } else { b"::" };
        // Interpreted bodies are marked.
        let mark: &[u8] = if *interp { b" [i]" } else { b"" };
        let len = (class.len() + sep.len() + func.len() + mark.len()).min(MAX_NAME);
        let mut name = [0u8; MAX_NAME];
        let mut k = 0;
        for part in [*class, sep, *func, mark] {
            for &b in part {
                if k < len {
                    name[k] = b;
                    k += 1;
                }
            }
        }
        if !put(&mut at, &[len as u8]) || !put(&mut at, &name[..len]) {
            DROPPED.fetch_add(1, Ordering::Relaxed);
            return;
        }
    }
    USED.store(at, Ordering::Relaxed);
}

fn timer(usec: libc::suseconds_t) {
    let tv = libc::timeval { tv_sec: 0, tv_usec: usec };
    let it = libc::itimerval { it_interval: tv, it_value: tv };
    unsafe { libc::setitimer(libc::ITIMER_PROF, &it, std::ptr::null_mut()) };
}

/// Starts sampling when PHPX_PROFILE_PHP is set.
pub fn start() {
    if std::env::var_os("PHPX_PROFILE_PHP").is_none() {
        return;
    }
    unsafe {
        DATA = Box::leak(vec![0u8; BUF].into_boxed_slice()).as_mut_ptr();
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sample as extern "C" fn(libc::c_int) as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigaction(libc::SIGPROF, &sa, std::ptr::null_mut());
    }
    timer(1000);
}

/// Stops sampling and reports.
pub fn finish() {
    let Some(path) = std::env::var_os("PHPX_PROFILE_PHP") else { return };
    timer(0);
    let used = USED.load(Ordering::Relaxed);
    let data = unsafe { std::slice::from_raw_parts(DATA, used) };
    let mut folded: std::collections::HashMap<String, usize> = Default::default();
    let mut own: std::collections::HashMap<String, usize> = Default::default();
    let mut incl: std::collections::HashMap<String, usize> = Default::default();
    let (mut i, mut total) = (0, 0usize);
    while i < data.len() {
        let n = data[i] as usize;
        i += 1;
        let mut names = Vec::with_capacity(n);
        for _ in 0..n {
            let len = data[i] as usize;
            names.push(String::from_utf8_lossy(&data[i + 1..i + 1 + len]).into_owned());
            i += 1 + len;
        }
        total += 1;
        *own.entry(names.first().cloned().unwrap_or_else(|| "(none)".into())).or_default() += 1;
        let mut seen = std::collections::HashSet::new();
        for n in &names {
            if seen.insert(n) {
                *incl.entry(n.clone()).or_default() += 1;
            }
        }
        names.reverse();
        *folded.entry(names.join(";")).or_default() += 1;
    }
    let show = |title: &str, m: &std::collections::HashMap<String, usize>| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("--- PHP {title} ({total} samples, {} dropped)", DROPPED.load(Ordering::Relaxed));
        for (n, c) in v.into_iter().take(40) {
            eprintln!("{:5.1}%  {n}", *c as f64 * 100.0 / total.max(1) as f64);
        }
    };
    show("self", &own);
    show("inclusive", &incl);
    let out: String = folded.iter().map(|(k, v)| format!("{k} {v}\n")).collect();
    let _ = std::fs::write(path, out);
}
