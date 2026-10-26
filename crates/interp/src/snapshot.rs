//! Snapshot mode for the built-in server (unix): the app boots once in a
//! template process, which forks a pool of children from that state. Each
//! child serves one request on its own copy-on-write copy of the booted app
//! and exits, so requests share no PHP state, as under PHP-FPM, without
//! paying for the boot.
//!
//! What "boot" is comes from a profile recognised from the project's files
//! (Laravel: autoloader, `bootstrap/app.php`, the HTTP kernel's bootstrappers);
//! the project's code is not changed. A boot that leaves state a child must
//! not share (output, headers, open connections or files) disables snapshots
//! and the server runs every request from scratch as usual.

use std::cell::RefCell;
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use super::{log, Config, HttpOut, Request};
use crate::interp::{Frame, Interp};
use crate::value::{pstr, Key, PhpArray, Value};

thread_local! {
    /// Files the template had parsed when this process was forked from it.
    static KNOWN: RefCell<std::collections::HashSet<PathBuf>> = RefCell::new(Default::default());
    /// Template pipe (write end) and the directory children report into.
    static REPORT: RefCell<Option<(i32, PathBuf)>> = const { RefCell::new(None) };
}

/// Pipe messages, child to template: (kind, pid) as two native-endian u32s
/// (8 bytes: atomic on a pipe).
const BUSY: u32 = 1;
const MANIFEST: u32 = 2;

fn send(fd: i32, kind: u32) {
    let mut m = [0u8; 8];
    m[..4].copy_from_slice(&kind.to_ne_bytes());
    m[4..].copy_from_slice(&std::process::id().to_ne_bytes());
    // SAFETY: writing 8 bytes from a valid buffer.
    unsafe { libc::write(fd, m.as_ptr().cast(), 8) };
}

/// How to boot an app ahead of its requests.
struct Profile {
    name: &'static str,
    /// PHP run in the template; returns the value `provides` hands back.
    boot: String,
    /// The include whose result the boot stands in for.
    provides: PathBuf,
}

/// Recognise the framework behind a docroot.
fn detect(cfg: &Config) -> Option<Profile> {
    let root = cfg.docroot.parent()?;
    let app = root.join("bootstrap/app.php");
    if app.is_file() && root.join("vendor/laravel/framework").is_dir() && root.join("vendor/autoload.php").is_file() {
        let q = |p: &Path| p.to_string_lossy().replace('\\', "\\\\").replace('\'', "\\'");
        // Every bootstrapper but BootProviders runs here: providers may read
        // the request while booting (Telescope does), so they boot in the
        // child, which the kernel is left to do.
        let boot = format!(
            r#"<?php
return (static function () {{
    require '{autoload}';
    $app = require '{app}';
    $kernel = $app->make(\Illuminate\Contracts\Http\Kernel::class);
    $boot = \Illuminate\Foundation\Bootstrap\BootProviders::class;
    $all = (fn () => $this->bootstrappers())->call($kernel);
    if (!in_array($boot, $all, true)) {{
        throw new \RuntimeException('the HTTP kernel does not boot providers with BootProviders');
    }}
    foreach ($all as $b) {{
        if ($b === $boot) {{
            continue;
        }}
        $app['events']->dispatch('bootstrapping: '.$b, [$app]);
        $app->make($b)->bootstrap($app);
        $app['events']->dispatch('bootstrapped: '.$b, [$app]);
    }}
    (fn () => $this->bootstrappers = [$boot])->call($kernel);
    if ((fn () => $this->bootstrappers())->call($kernel) !== [$boot]) {{
        throw new \RuntimeException('the HTTP kernel overrides bootstrappers()');
    }}
    return $app;
}})();
"#,
            autoload = q(&root.join("vendor/autoload.php")),
            app = q(&app)
        );
        let provides = app.canonicalize().unwrap_or(app);
        return Some(Profile { name: "Laravel", boot, provides });
    }
    None
}

/// What a child does once its request is known.
enum Child {
    /// Run the main script on the booted state.
    Snapshot { stream: TcpStream, req: Request, peer: String, started: Rc<RefCell<Option<i64>>> },
    /// Serve it the ordinary way (a static file, another script).
    Plain { stream: TcpStream, req: Request, peer: String },
}

/// Serve with snapshots. Returns only if the app can't be snapshotted (the
/// reason); children never return.
pub(super) fn serve(listener: &TcpListener, cfg: &Arc<Config>) -> Result<std::convert::Infallible, String> {
    let profile = detect(cfg).ok_or("no known framework")?;
    let main = match &cfg.router {
        Some(r) => r.clone(),
        None => cfg.docroot.join("index.php"),
    };
    let main = main.canonicalize().map_err(|e| format!("{}: {e}", main.display()))?;
    let src = std::fs::read(&main).map_err(|e| format!("{}: {e}", main.display()))?;
    let src = crate::pstr::lossy(&src).into_owned();
    let workers = std::env::var("PHPX_SNAPSHOT_WORKERS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0);
    let workers = workers.unwrap_or_else(|| std::thread::available_parallelism().map_or(4, |n| n.get()));

    let failure: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let child: Rc<RefCell<Option<Child>>> = Rc::new(RefCell::new(None));
    let interp: Rc<RefCell<std::rc::Weak<Interp>>> = Rc::new(RefCell::new(std::rc::Weak::new()));
    let (failure2, child2, interp2, cfg2, main2) = (failure.clone(), child.clone(), interp.clone(), cfg.clone(), main.clone());
    let listen_fd = listener.as_raw_fd();
    let setup: crate::Setup = Box::new(move |it, frame| {
        *interp2.borrow_mut() = Rc::downgrade(it);
        match template(it, frame, &profile, &cfg2) {
            Ok(()) => log(&format!("Snapshot: {} booted once; forking {workers} workers from it", profile.name)),
            Err(why) => {
                *failure2.borrow_mut() = Some(why);
                return false;
            }
        }
        let c = pool(it, listen_fd, workers, &cfg2, &main2);
        let run = matches!(c, Child::Snapshot { .. });
        if let Child::Snapshot { stream, req, started, .. } = &c {
            enter_request(it, frame, stream, req, &cfg2, &interp2, started);
        }
        *child2.borrow_mut() = Some(c);
        run
    });
    let (_, returned_false) = crate::run_main(&main, src, Box::new(std::io::sink()), setup);
    if let Some(why) = failure.borrow_mut().take() {
        return Err(why);
    }
    // Only a child gets here.
    let c = child.borrow_mut().take();
    let sock = match &c {
        Some(Child::Snapshot { stream, .. } | Child::Plain { stream, .. }) => stream.try_clone().ok(),
        None => None,
    };
    match c {
        Some(Child::Snapshot { stream, req, peer, started }) => {
            let sent = *started.borrow();
            if sent.is_none() && !returned_false {
                let host = req.header("host").unwrap_or("localhost").to_string();
                if let Ok(s) = stream.try_clone() {
                    let mut out = HttpOut { stream: s, interp: interp.clone(), host, started: started.clone() };
                    let _ = out.start();
                }
            }
            super::forget_uploads();
            if cfg.router.is_some() && returned_false {
                super::serve_target(stream, &req, cfg, &peer);
            } else if cfg.router.is_some() {
                super::router_done(&req, *started.borrow(), &peer);
            } else {
                if let Some(code) = *started.borrow() {
                    log(&format!("{peer} [{code}]: {} {}", req.method, req.uri));
                }
                log(&format!("{peer} Closing"));
            }
        }
        Some(Child::Plain { stream, req, peer }) => super::respond(stream, &req, cfg, &peer),
        None => {}
    }
    // The client has its response; what follows doesn't keep it waiting.
    if let Some(s) = sock {
        let _ = s.shutdown(std::net::Shutdown::Both);
    }
    report();
    // SAFETY: ends this child without running the template's exit handlers.
    unsafe { libc::_exit(0) }
}

/// Boot the app in the template and check it left nothing a child can't
/// inherit.
fn template(it: &Rc<Interp>, frame: &mut Frame, profile: &Profile, cfg: &Config) -> Result<(), String> {
    let mut server = PhpArray::new();
    super::server_base(&mut |k: &str, v: Value| server.insert(Key::Str(pstr(k)), v), cfg);
    *frame.var_slot("_SERVER").borrow_mut() = Value::array(server);
    for n in ["_GET", "_POST", "_COOKIE", "_FILES", "_REQUEST", "_ENV"] {
        *frame.var_slot(n).borrow_mut() = Value::array(PhpArray::new());
    }
    it.globals.set(frame);
    let fds = open_fds();
    let boot_path = profile.provides.with_file_name("phpx-snapshot-boot.php");
    let (id, file) = it.parse_file(&boot_path, profile.boot.clone())?;
    let app = match it.run_file_in(frame, file, id) {
        Ok(v) => v,
        Err(crate::interp::Ctrl::Throw(e)) => return Err(format!("boot threw: {}", it.uncaught_message(&e))),
        Err(_) => return Err("boot exited".into()),
    };
    if it.out.borrow().iter().any(|b| !b.is_empty()) {
        return Err("boot printed output".into());
    }
    if !it.response.borrow().headers.is_empty() || it.response.borrow().code != 0 {
        return Err("boot set response headers".into());
    }
    let opened: Vec<i32> = open_fds().into_iter().filter(|fd| !fds.contains(fd)).collect();
    if !opened.is_empty() {
        return Err(format!("boot left files or connections open (fds {opened:?})"));
    }
    it.preloaded.borrow_mut().insert(profile.provides.clone(), app);
    Ok(())
}

/// Template: keep `workers` children waiting for a connection, forever.
/// Returns in a child, with its request.
fn pool(it: &Rc<Interp>, listen_fd: i32, workers: usize, cfg: &Arc<Config>, main: &Path) -> Child {
    let (busy_rd, busy_wr) = new_pipe();
    let dir = std::env::temp_dir().join(format!("phpx-snapshot-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    REPORT.with(|r| *r.borrow_mut() = Some((busy_wr, dir.clone())));
    KNOWN.with(|k| k.borrow_mut().extend(crate::interp::parsed_paths()));
    // Children poll the listener without blocking in accept(), so a newer
    // generation can replace them.
    // SAFETY: setting a flag on a valid fd.
    unsafe { libc::fcntl(listen_fd, libc::F_SETFL, libc::fcntl(listen_fd, libc::F_GETFL) | libc::O_NONBLOCK) };
    let mut idle: std::collections::HashSet<i32> = Default::default();
    let mut pending: Vec<u8> = vec![];
    let mut warm: std::collections::VecDeque<PathBuf> = Default::default();
    // Idle children watch this pipe's read end: closing the write end
    // (when the template has warmed up since forking them) retires those
    // that haven't taken a connection yet.
    let mut gen = new_pipe();
    let mut warmed = false;
    let mut last_gen = std::time::Instant::now();
    loop {
        if warmed && warm.is_empty() && last_gen.elapsed() >= std::time::Duration::from_millis(250) {
            // SAFETY: closing this generation's pipe; children hold their own copies.
            unsafe {
                libc::close(gen.0);
                libc::close(gen.1);
            }
            gen = new_pipe();
            idle.clear();
            warmed = false;
            last_gen = std::time::Instant::now();
        }
        while idle.len() < workers {
            let guard = crate::timeout::fork_guard();
            // SAFETY: the template is single-threaded apart from the timeout
            // watcher, whose only lock `guard` holds.
            let pid = unsafe { libc::fork() };
            drop(guard);
            match pid {
                0 => {
                    // SAFETY: closing the ends this child doesn't use.
                    unsafe {
                        libc::close(busy_rd);
                        libc::close(gen.1);
                    }
                    return child(listen_fd, gen.0, busy_wr, cfg, main);
                }
                -1 => {
                    log(&format!("Snapshot: fork failed: {}", std::io::Error::last_os_error()));
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    break;
                }
                pid => {
                    idle.insert(pid);
                }
            }
        }
        // Children report taking a connection and, after it, the files
        // they parsed that this template hadn't.
        let mut pfd = libc::pollfd { fd: busy_rd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        if unsafe { libc::poll(&mut pfd, 1, if warm.is_empty() { 200 } else { 0 }) } > 0 {
            let mut b = [0u8; 512];
            // SAFETY: reading into a buffer of that size.
            let n = unsafe { libc::read(busy_rd, b.as_mut_ptr().cast(), b.len()) };
            if n > 0 {
                pending.extend_from_slice(&b[..n as usize]);
            }
            while pending.len() >= 8 {
                let m: Vec<u8> = pending.drain(..8).collect();
                let kind = u32::from_ne_bytes(m[..4].try_into().unwrap());
                let pid = u32::from_ne_bytes(m[4..].try_into().unwrap());
                match kind {
                    BUSY => {
                        idle.remove(&(pid as i32));
                    }
                    MANIFEST => {
                        let f = dir.join(pid.to_string());
                        let list = std::fs::read_to_string(&f).unwrap_or_default();
                        let _ = std::fs::remove_file(&f);
                        KNOWN.with(|k| {
                            let mut k = k.borrow_mut();
                            for p in list.lines().map(PathBuf::from) {
                                if k.insert(p.clone()) {
                                    warm.push_back(p);
                                }
                            }
                        });
                    }
                    _ => {}
                }
            }
        }
        // Parse what children needed, a few ms at a time: children forked
        // from here on find the syntax trees ready.
        let until = std::time::Instant::now() + std::time::Duration::from_millis(20);
        while std::time::Instant::now() < until {
            let Some(p) = warm.pop_front() else { break };
            let _ = it.load_file(&p, &p);
            warmed = true;
        }
        loop {
            let mut status = 0;
            // SAFETY: reaping any exited child.
            let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
            if pid <= 0 {
                break;
            }
            idle.remove(&pid);
        }
    }
}

/// A fresh child: wait for a connection, tell the template, read the request.
fn child(listen_fd: i32, gen_rd: i32, busy_wr: i32, cfg: &Arc<Config>, main: &Path) -> Child {
    phpx_runtime::after_fork();
    crate::timeout::after_fork();
    crate::random::reset();
    let stream = loop {
        let mut pfd = [libc::pollfd { fd: listen_fd, events: libc::POLLIN, revents: 0 }, libc::pollfd { fd: gen_rd, events: libc::POLLIN, revents: 0 }];
        // SAFETY: two valid pollfds.
        unsafe { libc::poll(pfd.as_mut_ptr(), 2, -1) };
        if pfd[1].revents != 0 {
            // Retired: a newer generation, forked from a warmer template, takes over.
            // SAFETY: ends an idle child.
            unsafe { libc::_exit(0) }
        }
        // SAFETY: accept on the listening socket inherited from the server
        // (non-blocking: another child may have taken the connection).
        let fd = unsafe { libc::accept(listen_fd, std::ptr::null_mut(), std::ptr::null_mut()) };
        if fd >= 0 {
            // SAFETY: a socket accept just returned, owned from here on.
            let s = unsafe { <TcpStream as std::os::fd::FromRawFd>::from_raw_fd(fd) };
            // (BSD sockets inherit O_NONBLOCK from the listener.)
            let _ = s.set_nonblocking(false);
            break s;
        }
    };
    send(busy_wr, BUSY);
    // SAFETY: the child is done with the listener and the generation pipe.
    unsafe {
        libc::close(listen_fd);
        libc::close(gen_rd);
    }
    let Some((req, peer)) = super::accept_request(&stream) else {
        // SAFETY: nothing to serve; end the child.
        unsafe { libc::_exit(0) }
    };
    let on_main = cfg.router.is_some() || super::resolve(&cfg.docroot, req.path()).is_some_and(|t| t.1.canonicalize().is_ok_and(|p| p == main));
    if on_main {
        Child::Snapshot { stream, req, peer, started: Rc::new(RefCell::new(None)) }
    } else {
        Child::Plain { stream, req, peer }
    }
}

fn new_pipe() -> (i32, i32) {
    let mut fds = [0i32; 2];
    // SAFETY: valid array for pipe().
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        panic!("phpx: snapshot pipe: {}", std::io::Error::last_os_error());
    }
    (fds[0], fds[1])
}

/// Tell the template which files this child parsed that it hadn't.
fn report() {
    let Some((fd, dir)) = REPORT.with(|r| r.borrow().clone()) else { return };
    let new: Vec<String> = KNOWN.with(|k| {
        let k = k.borrow();
        crate::interp::parsed_paths().into_iter().filter(|p| !k.contains(p)).map(|p| p.to_string_lossy().into_owned()).collect()
    });
    if new.is_empty() {
        return;
    }
    let f = dir.join(std::process::id().to_string());
    if std::fs::write(&f, new.join("\n")).is_ok() {
        send(fd, MANIFEST);
    }
}

/// Point the booted interpreter at this child's request.
fn enter_request(it: &Rc<Interp>, frame: &mut Frame, stream: &TcpStream, req: &Request, cfg: &Arc<Config>, interp: &Rc<RefCell<std::rc::Weak<Interp>>>, started: &Rc<RefCell<Option<i64>>>) {
    let host = req.header("host").unwrap_or("localhost").to_string();
    if let Ok(s) = stream.try_clone() {
        *it.writer.borrow_mut() = Box::new(HttpOut { stream: s, interp: interp.clone(), host, started: started.clone() });
    }
    let multipart = req.header("content-type").is_some_and(|c| c.to_ascii_lowercase().starts_with("multipart/form-data"));
    super::set_body(if multipart { vec![] } else { req.body.clone() });
    let target = match &cfg.router {
        Some(_) => super::router_target(req, cfg),
        None => super::resolve(&cfg.docroot, req.path()).unwrap_or_default(),
    };
    super::request_globals(it, frame, req, cfg, &target, true);
}

/// The process's open file descriptors.
fn open_fds() -> Vec<i32> {
    // SAFETY: F_GETFD only queries flags.
    (0..1024).filter(|&fd| unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1).collect()
}
