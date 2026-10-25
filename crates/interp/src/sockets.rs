//! Socket streams: fsockopen(), stream_socket_client/server/accept(),
//! stream_select(), TLS (tls://, ssl://, stream_socket_enable_crypto()).
//!
//! Sockets are non-blocking underneath: a read or write that would block
//! parks the green thread, so other tasks run while this one waits.

use std::collections::HashMap;
use std::os::fd::{AsFd, BorrowedFd};
use std::time::{Duration, Instant};

use phpx_runtime::net::{Listener, Stream as NetStream};
use phpx_runtime::tls::{self, TlsStream};

use crate::builtins::{arg, has, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::streams::{Backend, Stream};
use crate::value::{Key, PhpArray, Resource, Value};

pub enum Conn {
    Plain(NetStream),
    Tls(Box<TlsStream>),
    Server(Listener),
    /// Mid-upgrade (stream_socket_enable_crypto) or failed.
    Gone,
}

pub struct Sock {
    pub conn: Conn,
    /// Host the stream connected to (TLS name check and SNI).
    host: String,
    unix: bool,
    /// Bytes read so far (ftell).
    pub read_total: u64,
}

/// A read that waited past its deadline.
pub struct TimedOut;

impl Sock {
    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        match &self.conn {
            Conn::Plain(s) => Some(s.as_fd()),
            Conn::Tls(t) => Some(t.as_fd()),
            Conn::Server(l) => Some(l.as_fd()),
            Conn::Gone => None,
        }
    }

    /// Bytes decrypted but not yet read (ready without the socket).
    pub fn buffered(&mut self) -> usize {
        match &mut self.conn {
            Conn::Tls(t) => t.buffered(),
            _ => 0,
        }
    }

    /// One read. Ok(0) at end of stream.
    pub fn read(&mut self, buf: &mut [u8], deadline: Option<Instant>) -> Result<usize, TimedOut> {
        let r = match &mut self.conn {
            Conn::Plain(s) => s.read(buf, deadline),
            Conn::Tls(t) => t.read(buf, deadline),
            _ => return Ok(0),
        };
        match r {
            Ok(n) => {
                self.read_total += n as u64;
                Ok(n)
            }
            Err(phpx_runtime::Error::Io(std::io::ErrorKind::TimedOut, _)) => Err(TimedOut),
            Err(_) => Ok(0),
        }
    }

    pub fn write_all(&mut self, data: &[u8], deadline: Option<Instant>) -> Result<(), (i32, String)> {
        let r = match &mut self.conn {
            Conn::Plain(s) => s.write_all(data, deadline),
            Conn::Tls(t) => t.write_all(data, deadline),
            _ => return Err((32, "Broken pipe".into())),
        };
        r.map_err(|e| os_error(&e))
    }

    /// The `crypto` entry of stream_get_meta_data() for TLS streams.
    pub fn crypto_meta(&self) -> Option<Value> {
        let Conn::Tls(t) = &self.conn else { return None };
        let (proto, cipher, bits) = t.info()?;
        let mut m = PhpArray::new();
        for (k, v) in [("protocol", Value::str(&proto)), ("cipher_name", Value::str(&cipher)), ("cipher_bits", Value::Int(bits as i64)), ("cipher_version", Value::str(&proto))] {
            m.insert(Key::Str(crate::value::pstr(k)), v);
        }
        Some(Value::array(m))
    }

    pub fn stream_type(&self) -> &'static str {
        if self.unix {
            "unix_socket"
        } else {
            "tcp_socket/ssl"
        }
    }

    pub fn shutdown(&self, how: std::net::Shutdown) -> bool {
        match &self.conn {
            Conn::Plain(s) => s.shutdown(how).is_ok(),
            Conn::Tls(t) => t.socket().shutdown(how).is_ok(),
            _ => false,
        }
    }
}

impl Drop for Sock {
    fn drop(&mut self) {
        if let Conn::Tls(t) = &mut self.conn {
            t.close();
        }
    }
}

/// errno and message from a runtime error ("Connection refused (os error
/// 111)" style messages carry the errno).
fn os_error(e: &phpx_runtime::Error) -> (i32, String) {
    let phpx_runtime::Error::Io(kind, msg) = e else { return (0, e.to_string()) };
    if *kind == std::io::ErrorKind::TimedOut {
        return (110, "Connection timed out".into());
    }
    if let Some(i) = msg.rfind(" (os error ") {
        let code = msg[i + 11..].trim_end_matches(')').parse().unwrap_or(0);
        return (code, msg[..i].to_string());
    }
    (0, msg.clone())
}

/// `default_socket_timeout` (negative: no timeout).
pub fn default_timeout(it: &Interp) -> Option<Duration> {
    let t: f64 = it.ini_get("default_socket_timeout").and_then(|v| v.trim().parse().ok()).unwrap_or(60.0);
    timeout_from(t)
}

fn timeout_from(secs: f64) -> Option<Duration> {
    (secs >= 0.0 && secs.is_finite()).then(|| Duration::from_secs_f64(secs))
}

/// A parsed transport target.
struct Target {
    scheme: String,
    host: String,
    port: u16,
    path: String,
}

impl Target {
    fn parse(spec: &str) -> Option<Target> {
        let (scheme, rest) = match spec.find("://") {
            Some(i) => (spec[..i].to_ascii_lowercase(), &spec[i + 3..]),
            None => ("tcp".to_string(), spec),
        };
        if scheme == "unix" || scheme == "udg" {
            return Some(Target { scheme, host: String::new(), port: 0, path: rest.to_string() });
        }
        let (host, port) = if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']')?;
            (r[..end].to_string(), r[end + 1..].strip_prefix(':').unwrap_or(""))
        } else {
            let i = rest.rfind(':')?;
            (rest[..i].to_string(), &rest[i + 1..])
        };
        let port = port.split('/').next().unwrap_or("").parse().ok()?;
        Some(Target { scheme, host, port, path: String::new() })
    }

    fn tls(&self) -> bool {
        matches!(self.scheme.as_str(), "ssl" | "tls" | "tlsv1.0" | "tlsv1.1" | "tlsv1.2" | "tlsv1.3")
    }
}

/// TLS options from a stream context's `ssl` array.
fn ssl_options(it: &Interp, ctx: &Value) -> tls::Options {
    let mut o = tls::Options::default();
    let Value::Resource(r) = ctx else { return o };
    let opts = match r.data.borrow().as_ref().and_then(|b| b.downcast_ref::<std::cell::RefCell<Value>>()) {
        Some(c) => c.borrow().clone(),
        None => return o,
    };
    let Value::Array(all) = opts else { return o };
    let Some(Value::Array(ssl)) = all.get(&Key::Str(crate::value::pstr("ssl"))) else { return o };
    let get = |k: &str| ssl.get(&Key::Str(crate::value::pstr(k)));
    let flag = |k: &str, d: bool| get(k).map_or(d, |v| v.truthy());
    let text = |k: &str| get(k).filter(|v| !v.is_null()).and_then(|v| it.to_str(&v).ok()).map(|p| String::from_utf8_lossy(&p).into_owned());
    o.verify_peer = flag("verify_peer", true);
    o.verify_peer_name = flag("verify_peer_name", true);
    o.allow_self_signed = flag("allow_self_signed", false);
    o.sni = flag("SNI_enabled", true);
    o.cafile = text("cafile").map(Into::into);
    o.capath = text("capath").map(Into::into);
    o.peer_name = text("peer_name");
    if let Some(a) = text("alpn_protocols") {
        o.alpn = a.split(',').filter(|p| !p.is_empty()).map(|p| p.as_bytes().to_vec()).collect();
    }
    o
}

/// Connect to `t`; Err((errno, message)) as PHP reports it.
fn connect(t: &Target, deadline: Option<Instant>, ssl: &tls::Options) -> Result<Sock, (i32, String)> {
    if t.scheme == "unix" {
        let s = NetStream::connect_unix(&t.path, deadline).map_err(|e| os_error(&e))?;
        return Ok(Sock { conn: Conn::Plain(s), host: String::new(), unix: true, read_total: 0 });
    }
    if t.scheme == "udp" || t.scheme == "udg" {
        return Err((0, format!("Unable to find the socket transport \"{}\" - did you forget to enable it when you configured PHP?", t.scheme)));
    }
    let addrs = match phpx_runtime::net::resolve((t.host.as_str(), t.port)) {
        Ok(a) => a,
        Err(e) => {
            let msg = os_error(&e).1;
            let msg = msg.strip_prefix("failed to lookup address information: ").unwrap_or(&msg).to_string();
            return Err((0, format!("php_network_getaddresses: getaddrinfo for {} failed: {msg}", t.host)));
        }
    };
    let s = NetStream::connect_tcp(&addrs[..], deadline).map_err(|e| os_error(&e))?;
    let mut sock = Sock { conn: Conn::Plain(s), host: t.host.clone(), unix: false, read_total: 0 };
    if t.tls() {
        sock.enable_tls(ssl, deadline)?;
    }
    Ok(sock)
}

impl Sock {
    fn enable_tls(&mut self, ssl: &tls::Options, deadline: Option<Instant>) -> Result<(), (i32, String)> {
        let Conn::Plain(s) = std::mem::replace(&mut self.conn, Conn::Gone) else { return Err((0, "not a plain socket".into())) };
        match TlsStream::connect(s, &self.host, ssl, deadline) {
            Ok(t) => {
                self.conn = Conn::Tls(Box::new(t));
                Ok(())
            }
            Err(e) => Err((0, os_error(&e).1)),
        }
    }
}

fn new_stream(sock: Sock, uri: &str, it: &Interp, ctx: Option<Value>) -> Value {
    let mut st = Stream::new(Backend::Socket(sock), "r+", uri, "");
    st.timeout = default_timeout(it);
    st.context = ctx;
    Resource::new("stream", Box::new(st))
}

fn float_opt(it: &Interp, a: &[Value], i: usize) -> R<Option<f64>> {
    Ok(if has(a, i) && !arg(a, i).is_null() { Some(crate::builtins::float(it, a, i)?) } else { None })
}

/// Shared tail of fsockopen / stream_socket_client.
fn client(it: &Interp, fname: &str, spec: &str, timeout: Option<f64>, ctx: &Value, a: &mut [Value], ei: usize) -> R<Value> {
    let set_err = |a: &mut [Value], code: i32, msg: &str| {
        if a.len() > ei {
            a[ei] = Value::Int(code as i64);
        }
        if a.len() > ei + 1 {
            a[ei + 1] = Value::str(msg);
        }
    };
    let Some(t) = Target::parse(spec) else {
        let msg = format!("Failed to parse address \"{spec}\"");
        it.warn(&format!("{fname}(): Unable to connect to {spec} ({msg})"));
        set_err(a, 0, &msg);
        return Ok(Value::Bool(false));
    };
    let wait = timeout.map(timeout_from).unwrap_or_else(|| default_timeout(it));
    let deadline = wait.map(|d| Instant::now() + d);
    match connect(&t, deadline, &ssl_options(it, ctx)) {
        Ok(sock) => {
            set_err(a, 0, "");
            Ok(new_stream(sock, spec, it, (!ctx.is_null()).then(|| ctx.clone())))
        }
        Err((code, msg)) => {
            if msg.starts_with("php_network_getaddresses") {
                it.warn(&format!("{fname}(): {msg}"));
            }
            it.warn(&format!("{fname}(): Unable to connect to {spec} ({msg})"));
            set_err(a, code, &msg);
            Ok(Value::Bool(false))
        }
    }
}

/// Run `g` on a socket stream; None if `v` isn't one.
fn with_sock<T>(v: &Value, g: impl FnOnce(&mut Stream, &mut Sock) -> T) -> Option<T> {
    let Value::Resource(r) = v else { return None };
    let mut d = r.data.borrow_mut();
    let st = d.as_mut()?.downcast_mut::<Stream>()?;
    let mut backend = std::mem::replace(&mut st.backend, Backend::Memory(vec![]));
    let out = match &mut backend {
        Backend::Socket(sk) => Some(g(st, sk)),
        _ => None,
    };
    st.backend = backend;
    out
}

/// stream_select(): which of the listed streams are ready.
fn select(it: &Interp, a: &mut [Value]) -> R<Value> {
    let secs = if has(a, 3) && !arg(a, 3).is_null() { Some(crate::builtins::int(it, a, 3)?) } else { None };
    let usec = if has(a, 4) && !arg(a, 4).is_null() { crate::builtins::int(it, a, 4)? } else { 0 };
    let deadline = secs.map(|s| Instant::now() + Duration::from_secs(s.max(0) as u64) + Duration::from_micros(usec.max(0) as u64));
    let lists: Vec<Option<PhpArray>> = (0..3)
        .map(|i| match arg(a, i) {
            Value::Array(x) => Some((*x).clone()),
            _ => None,
        })
        .collect();
    // Per fd: (read, write) interest; streams ready without waiting.
    let mut interest: HashMap<i32, (bool, bool)> = HashMap::new();
    let mut ready: Vec<Vec<bool>> = vec![];
    for (li, l) in lists.iter().enumerate() {
        let mut row = vec![];
        if let Some(l) = l {
            for (_, v) in l.iter() {
                let mut now = false;
                let fd = match &v {
                    Value::Resource(r) => {
                        let mut d = r.data.borrow_mut();
                        match d.as_mut().and_then(|b| b.downcast_mut::<Stream>()) {
                            Some(st) => match &mut st.backend {
                                Backend::Socket(sk) => {
                                    if li == 0 && (!st.rbuf.is_empty() || st.eof || sk.buffered() > 0) {
                                        now = true;
                                    }
                                    sk.fd().map(|f| std::os::fd::AsRawFd::as_raw_fd(&f))
                                }
                                // Files and memory never block.
                                _ => {
                                    now = li < 2;
                                    None
                                }
                            },
                            None => None,
                        }
                    }
                    _ => None,
                };
                if let (Some(fd), false, true) = (fd, now, li < 2) {
                    let e = interest.entry(fd).or_default();
                    if li == 0 {
                        e.0 = true;
                    } else {
                        e.1 = true;
                    }
                }
                row.push(now);
            }
        }
        ready.push(row);
    }
    let any_now = ready.iter().flatten().any(|&x| x);
    let fds: Vec<(i32, bool, bool)> = interest.iter().map(|(&fd, &(r, w))| (fd, r, w)).collect();
    let mut fired: HashMap<i32, (bool, bool)> = HashMap::new();
    if !fds.is_empty() {
        // SAFETY: the streams in the argument arrays own these fds and stay
        // alive for this call.
        let borrowed: Vec<(BorrowedFd<'_>, bool, bool)> = fds.iter().map(|&(fd, r, w)| (unsafe { BorrowedFd::borrow_raw(fd) }, r, w)).collect();
        let dl = if any_now { Some(Instant::now()) } else { deadline };
        let res = phpx_runtime::net::select(&borrowed, dl).map_err(|e| crate::native::rt_error(it, e))?;
        for (i, &(fd, ..)) in fds.iter().enumerate() {
            fired.insert(fd, res[i]);
        }
    } else if !any_now {
        if let Some(d) = deadline {
            phpx_runtime::sleep(d.saturating_duration_since(Instant::now())).map_err(|e| crate::native::rt_error(it, e))?;
        }
    }
    let mut count = 0;
    for (li, l) in lists.into_iter().enumerate() {
        let Some(l) = l else { continue };
        let mut out = PhpArray::new();
        for (j, (k, v)) in l.iter().enumerate() {
            let fd = with_sock(&v, |_, sk| sk.fd().map(|f| std::os::fd::AsRawFd::as_raw_fd(&f))).flatten();
            let hit = ready[li][j]
                || fd.and_then(|fd| fired.get(&fd)).is_some_and(|&(r, w)| if li == 0 { r } else if li == 1 { w } else { false });
            if hit {
                out.insert(k.clone(), v.clone());
                count += 1;
            }
        }
        a[li] = Value::array(out);
    }
    Ok(Value::Int(count))
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    for name in ["fsockopen", "pfsockopen"] {
        t.insert(name, |it, _, a| {
            let host = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
            let port = if has(a, 1) { crate::builtins::int(it, a, 1)? } else { -1 };
            let spec = if host.starts_with("unix://") || port < 0 { host } else { format!("{host}:{port}") };
            let timeout = float_opt(it, a, 4)?;
            client(it, "fsockopen", &spec, timeout, &Value::Null, a, 2)
        });
    }
    t.insert("stream_socket_client", |it, _, a| {
        let spec = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        let timeout = float_opt(it, a, 3)?;
        let ctx = arg(a, 5);
        client(it, "stream_socket_client", &spec, timeout, &ctx, a, 1)
    });
    t.insert("stream_socket_server", |it, _, a| {
        let spec = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        let fail = |a: &mut [Value], code: i32, msg: &str| {
            it.warn(&format!("stream_socket_server(): Unable to connect to {spec} ({msg})"));
            if a.len() > 1 {
                a[1] = Value::Int(code as i64);
            }
            if a.len() > 2 {
                a[2] = Value::str(msg);
            }
            Ok(Value::Bool(false))
        };
        let Some(t) = Target::parse(&spec) else { return fail(a, 0, &format!("Failed to parse address \"{spec}\"")) };
        let l = if t.scheme == "unix" { Listener::bind_unix(&t.path) } else { Listener::bind_tcp((t.host.as_str(), t.port)) };
        match l {
            Ok(l) => {
                let sock = Sock { conn: Conn::Server(l), host: t.host, unix: t.scheme == "unix", read_total: 0 };
                Ok(new_stream(sock, &spec, it, None))
            }
            Err(e) => {
                let (code, msg) = os_error(&e);
                fail(a, code, &msg)
            }
        }
    });
    t.insert("stream_socket_accept", |it, _, a| {
        let wait = match float_opt(it, a, 1)? {
            Some(t) => timeout_from(t),
            None => default_timeout(it),
        };
        let deadline = wait.map(|d| Instant::now() + d);
        let res = with_sock(&arg(a, 0), |_, sk| match &sk.conn {
            Conn::Server(l) => Some(l.accept(deadline).map(|(s, peer)| (Sock { conn: Conn::Plain(s), host: String::new(), unix: sk.unix, read_total: 0 }, peer))),
            _ => None,
        })
        .flatten();
        match res {
            Some(Ok((sock, peer))) => {
                if a.len() > 2 {
                    a[2] = peer.map(|p| Value::str(&p)).unwrap_or(Value::Null);
                }
                Ok(new_stream(sock, "", it, None))
            }
            Some(Err(e)) => {
                it.warn(&format!("stream_socket_accept(): Accept failed: {}", os_error(&e).1));
                Ok(Value::Bool(false))
            }
            None => Ok(Value::Bool(false)),
        }
    });
    t.insert("stream_socket_get_name", |it, _, a| {
        let remote = has(a, 1) && arg(a, 1).truthy();
        let _ = it;
        Ok(with_sock(&arg(a, 0), |_, sk| {
            let name = match &sk.conn {
                Conn::Plain(s) => if remote { s.peer_addr() } else { s.local_addr() },
                Conn::Tls(t) => if remote { t.socket().peer_addr() } else { t.socket().local_addr() },
                Conn::Server(l) => if remote { None } else { l.local_addr() },
                Conn::Gone => None,
            };
            name.map(|n| Value::str(&n)).unwrap_or(Value::Bool(false))
        })
        .unwrap_or(Value::Bool(false)))
    });
    t.insert("stream_select", |it, _, a| select(it, a));
    t.insert("stream_set_timeout", |it, _, a| {
        let secs = crate::builtins::int(it, a, 1)?;
        let usec = if has(a, 2) { crate::builtins::int(it, a, 2)? } else { 0 };
        let d = Duration::from_secs(secs.max(0) as u64) + Duration::from_micros(usec.max(0) as u64);
        Ok(Value::Bool(with_sock(&arg(a, 0), |st, _| st.timeout = Some(d)).is_some()))
    });
    t.insert("stream_set_blocking", |it, _, a| {
        let on = arg(a, 1).truthy();
        let _ = it;
        with_sock(&arg(a, 0), |st, _| st.blocking = on);
        Ok(Value::Bool(true))
    });
    t.insert("stream_socket_enable_crypto", |it, _, a| {
        let on = arg(a, 1).truthy();
        let ctx = arg(a, 0);
        let ssl = with_sock(&ctx, |st, _| st.context.clone()).flatten().map(|c| ssl_options(it, &c)).unwrap_or_default();
        let deadline = default_timeout(it).map(|d| Instant::now() + d);
        let r = with_sock(&ctx, |_, sk| match (&sk.conn, on) {
            (Conn::Plain(_), true) => sk.enable_tls(&ssl, deadline).map_err(|e| e.1),
            (Conn::Tls(_), true) => Ok(()),
            (_, false) => Ok(()),
            _ => Err("not a socket".to_string()),
        });
        match r {
            Some(Ok(())) => Ok(Value::Bool(true)),
            Some(Err(msg)) => {
                it.warn(&format!("stream_socket_enable_crypto(): SSL operation failed with code 1. OpenSSL Error messages:\n{msg}"));
                Ok(Value::Bool(false))
            }
            None => Ok(Value::Bool(false)),
        }
    });
    t.insert("stream_socket_shutdown", |it, _, a| {
        let how = crate::builtins::int(it, a, 1)?;
        let how = match how {
            0 => std::net::Shutdown::Read,
            1 => std::net::Shutdown::Write,
            _ => std::net::Shutdown::Both,
        };
        Ok(Value::Bool(with_sock(&arg(a, 0), |_, sk| sk.shutdown(how)).unwrap_or(false)))
    });
    t.insert("stream_get_transports", |_, _, _| {
        Ok(Value::array(PhpArray::from_list(["tcp", "udp", "unix", "udg", "ssl", "tls", "tlsv1.0", "tlsv1.1", "tlsv1.2", "tlsv1.3"].map(Value::str))))
    });
}

pub fn constants(add: &mut dyn FnMut(&str, Value)) {
    for (k, v) in [
        ("STREAM_CLIENT_PERSISTENT", 1),
        ("STREAM_CLIENT_ASYNC_CONNECT", 2),
        ("STREAM_CLIENT_CONNECT", 4),
        ("STREAM_SERVER_BIND", 4),
        ("STREAM_SERVER_LISTEN", 8),
        ("STREAM_SHUT_RD", 0),
        ("STREAM_SHUT_WR", 1),
        ("STREAM_SHUT_RDWR", 2),
        ("STREAM_CRYPTO_METHOD_ANY_CLIENT", 127),
        ("STREAM_CRYPTO_METHOD_SSLv2_CLIENT", 3),
        ("STREAM_CRYPTO_METHOD_SSLv3_CLIENT", 5),
        ("STREAM_CRYPTO_METHOD_SSLv23_CLIENT", 57),
        ("STREAM_CRYPTO_METHOD_TLS_CLIENT", 121),
        ("STREAM_CRYPTO_METHOD_TLSv1_0_CLIENT", 9),
        ("STREAM_CRYPTO_METHOD_TLSv1_1_CLIENT", 17),
        ("STREAM_CRYPTO_METHOD_TLSv1_2_CLIENT", 33),
        ("STREAM_CRYPTO_METHOD_TLSv1_3_CLIENT", 65),
        ("STREAM_CRYPTO_METHOD_ANY_SERVER", 126),
        ("STREAM_CRYPTO_METHOD_SSLv2_SERVER", 2),
        ("STREAM_CRYPTO_METHOD_SSLv3_SERVER", 4),
        ("STREAM_CRYPTO_METHOD_SSLv23_SERVER", 120),
        ("STREAM_CRYPTO_METHOD_TLS_SERVER", 120),
        ("STREAM_CRYPTO_METHOD_TLSv1_0_SERVER", 8),
        ("STREAM_CRYPTO_METHOD_TLSv1_1_SERVER", 16),
        ("STREAM_CRYPTO_METHOD_TLSv1_2_SERVER", 32),
        ("STREAM_CRYPTO_METHOD_TLSv1_3_SERVER", 64),
        ("STREAM_CRYPTO_PROTO_SSLv3", 4),
        ("STREAM_CRYPTO_PROTO_TLSv1_0", 8),
        ("STREAM_CRYPTO_PROTO_TLSv1_1", 16),
        ("STREAM_CRYPTO_PROTO_TLSv1_2", 32),
        ("STREAM_CRYPTO_PROTO_TLSv1_3", 64),
        ("STREAM_PF_INET", 2),
        ("STREAM_PF_INET6", 10),
        ("STREAM_PF_UNIX", 1),
        ("STREAM_IPPROTO_IP", 0),
        ("STREAM_IPPROTO_TCP", 6),
        ("STREAM_IPPROTO_UDP", 17),
        ("STREAM_IPPROTO_ICMP", 1),
        ("STREAM_IPPROTO_RAW", 255),
        ("STREAM_SOCK_STREAM", 1),
        ("STREAM_SOCK_DGRAM", 2),
        ("STREAM_SOCK_RAW", 3),
        ("STREAM_SOCK_SEQPACKET", 5),
        ("STREAM_SOCK_RDM", 4),
        ("STREAM_PEEK", 2),
        ("STREAM_OOB", 1),
    ] {
        add(k, Value::Int(v));
    }
}
