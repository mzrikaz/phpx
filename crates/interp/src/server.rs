//! The built-in web server: `phpx -S host:port [-t docroot] [router.php]`,
//! like `php -S` (which `artisan serve` runs). Each connection gets its
//! own thread and a fresh interpreter, so requests share no PHP state;
//! output streams to the socket, headers going out before the first
//! byte. Logging matches PHP's so tools that parse it (Laravel) work.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{Arc, OnceLock};

use crate::interp::{Frame, Interp};
use crate::value::{pstr, Key, PhpArray, Value};

#[cfg(unix)]
#[path = "snapshot.rs"]
mod snapshot;

static SAPI: OnceLock<&'static str> = OnceLock::new();

/// php_sapi_name(): "cli", or "cli-server" under -S.
pub fn sapi() -> &'static str {
    SAPI.get().copied().unwrap_or("cli")
}

thread_local! {
    /// The request body (php://input).
    static BODY: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// Temporary files of this request's uploads.
    static UPLOADS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn set_body(body: Vec<u8>) {
    BODY.with(|b| *b.borrow_mut() = body);
}

/// Delete what's left of this request's uploads.
fn forget_uploads() {
    UPLOADS.with(|u| {
        for f in u.borrow_mut().drain() {
            let _ = std::fs::remove_file(f);
        }
    });
}

pub fn request_body() -> Vec<u8> {
    BODY.with(|b| b.borrow().clone())
}

pub fn is_uploaded(path: &str) -> bool {
    UPLOADS.with(|u| u.borrow().contains(path))
}

pub fn forget_upload(path: &str) {
    UPLOADS.with(|u| u.borrow_mut().remove(path));
}

struct Config {
    docroot: PathBuf,
    router: Option<PathBuf>,
    /// The router script as given on the command line: `SCRIPT_FILENAME`
    /// of requests for paths that name no file (PHP's server does this;
    /// Symfony's base URL detection relies on it).
    router_arg: Option<PathBuf>,
    addr: String,
}

/// `[Sun Oct 26 03:04:05 2026]` in local time, like PHP's server log.
fn log_time() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{} {} {:2} {:02}:{:02}:{:02} {}",
        DAYS[tm.tm_wday as usize % 7],
        MONTHS[tm.tm_mon as usize % 12],
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        tm.tm_year + 1900
    )
}

/// With several workers PHP prefixes each log line with the worker's
/// pid (Laravel's `serve` parses that format).
static PID_PREFIX: OnceLock<bool> = OnceLock::new();

fn log(msg: &str) {
    let mut e = std::io::stderr().lock();
    if PID_PREFIX.get().copied().unwrap_or(false) {
        let _ = writeln!(e, "[{}] [{}] {msg}", std::process::id(), log_time());
    } else {
        let _ = writeln!(e, "[{}] {msg}", log_time());
    }
}

fn http_date() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::gmtime_r(&now, &mut tm) };
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[tm.tm_wday as usize % 7],
        tm.tm_mday,
        MONTHS[tm.tm_mon as usize % 12],
        tm.tm_year + 1900,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

fn reason(code: i64) -> &'static str {
    match code {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        419 => "Page Expired",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

fn mime(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" | "map" => "application/json",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/vnd.microsoft.icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

struct Request {
    method: String,
    uri: String,
    version: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    remote: SocketAddr,
    local: SocketAddr,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn path(&self) -> &str {
        self.uri.split(['?', '#']).next().unwrap_or("/")
    }

    fn query(&self) -> &str {
        self.uri.split_once('?').map_or("", |(_, q)| q.split('#').next().unwrap_or(""))
    }
}

fn read_request(stream: &TcpStream) -> std::io::Result<Option<Request>> {
    let mut r = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.trim_end().splitn(3, ' ');
    let (method, uri, version) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("/").to_string(), parts.next().unwrap_or("HTTP/1.0").to_string());
    let mut headers = vec![];
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 {
            break;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    let mut req = Request { method, uri, version, headers, body: vec![], remote: stream.peer_addr()?, local: stream.local_addr()? };
    if req.header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            r.read_line(&mut size)?;
            let n = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16).unwrap_or(0);
            if n == 0 {
                let mut end = String::new();
                r.read_line(&mut end)?;
                break;
            }
            let mut chunk = vec![0; n];
            r.read_exact(&mut chunk)?;
            req.body.extend_from_slice(&chunk);
            let mut crlf = [0u8; 2];
            r.read_exact(&mut crlf)?;
        }
    } else if let Some(n) = req.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
        req.body = vec![0; n];
        r.read_exact(&mut req.body)?;
    }
    Ok(Some(req))
}

fn percent_decode(s: &str) -> String {
    crate::pstr::lossy(&percent_decode_bytes(s)).into_owned()
}

pub fn percent_decode_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// The script (or file) a URL path maps to: (vpath of the file, file,
/// PATH_INFO). Missing files fall back to an index in a parent directory.
fn resolve(docroot: &Path, path: &str) -> Option<(String, PathBuf, String)> {
    let decoded = percent_decode(path);
    let segs: Vec<&str> = decoded.split('/').filter(|s| !s.is_empty() && *s != "." && *s != "..").collect();
    for n in (0..=segs.len()).rev() {
        let vpath = format!("/{}", segs[..n].join("/"));
        let full = docroot.join(segs[..n].join("/"));
        let info = if n < segs.len() { format!("/{}", segs[n..].join("/")) } else { String::new() };
        if full.is_file() && n > 0 {
            return Some((vpath, full, info));
        }
        if full.is_dir() {
            for idx in ["index.php", "index.html"] {
                let f = full.join(idx);
                if f.is_file() {
                    let v = if vpath == "/" { format!("/{idx}") } else { format!("{vpath}/{idx}") };
                    // A missing file below a directory falls back to its index
                    // without PATH_INFO, as PHP's server does.
                    return Some((v, f, String::new()));
                }
            }
        }
    }
    None
}

/// Output sink: the status line and headers go out before the first byte.
struct HttpOut {
    stream: TcpStream,
    interp: Rc<RefCell<Weak<Interp>>>,
    host: String,
    started: Rc<RefCell<Option<i64>>>,
}

impl HttpOut {
    fn start(&mut self) -> std::io::Result<()> {
        if self.started.borrow().is_some() {
            return Ok(());
        }
        let (code, headers) = match self.interp.borrow().upgrade() {
            Some(it) => {
                let r = it.response.borrow();
                (if r.code == 0 { 200 } else { r.code }, r.headers.clone())
            }
            None => (200, vec![]),
        };
        *self.started.borrow_mut() = Some(code);
        let mut head = format!("HTTP/1.1 {code} {}\r\nHost: {}\r\nDate: {}\r\nConnection: close\r\n", reason(code), self.host, http_date());
        for h in &headers {
            head.push_str(h);
            head.push_str("\r\n");
        }
        if !headers.iter().any(|h| h.split(':').next().is_some_and(|n| n.trim().eq_ignore_ascii_case("content-type"))) {
            head.push_str("Content-type: text/html; charset=UTF-8\r\n");
        }
        head.push_str("\r\n");
        self.stream.write_all(head.as_bytes())
    }
}

impl Write for HttpOut {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.start()?;
        self.stream.write_all(b)?;
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

/// Fill $_GET / $_POST / ... by running parse_str() on url-encoded text.
/// Over max_input_vars, PHP keeps the first ones and warns; the nesting
/// limit drops a too-deep name. (Both bound the work a request's input
/// can cause.)
fn parse_into(it: &Interp, _f: &mut Frame, text: &[u8]) -> Value {
    let (max_vars, max_nesting) = crate::ext::input_limits(it);
    let (vars, cut) = crate::ext::parse_query(text, max_vars, max_nesting);
    if cut {
        it.warn(&format!("Unknown: Input variables exceeded {max_vars}. To increase the limit change max_input_vars in php.ini."));
    }
    Value::array(vars)
}

fn urlencode(b: &[u8]) -> String {
    let mut s = String::with_capacity(b.len());
    for &c in b {
        if c.is_ascii_alphanumeric() || b"-_.".contains(&c) {
            s.push(c as char);
        } else {
            s.push_str(&format!("%{c:02X}"));
        }
    }
    s
}

/// Name with brackets kept literal for parse_str (`a[b][]`).
fn encode_name(name: &str) -> String {
    name.split_inclusive(['[', ']']).map(|p| {
        let (body, tail) = match p.chars().last() {
            Some(c @ ('[' | ']')) => (&p[..p.len() - 1], c.to_string()),
            _ => (p, String::new()),
        };
        format!("{}{tail}", urlencode(body.as_bytes()))
    }).collect()
}

/// multipart/form-data: (url-encoded fields, url-encoded $_FILES).
fn multipart(body: &[u8], boundary: &str) -> (String, String) {
    let delim = format!("--{boundary}");
    let mut fields = String::new();
    let mut files = String::new();
    let add = |s: &mut String, part: String| {
        if !s.is_empty() {
            s.push('&');
        }
        s.push_str(&part);
    };
    let mut rest = body;
    while let Some(p) = memchr::memmem::find(rest, delim.as_bytes()) {
        rest = &rest[p + delim.len()..];
        if rest.starts_with(b"--") {
            break;
        }
        let rest_part = rest.strip_prefix(b"\r\n").unwrap_or(rest);
        let Some(he) = memchr::memmem::find(rest_part, b"\r\n\r\n") else { break };
        let head = crate::pstr::lossy(&rest_part[..he]).into_owned();
        let data_start = &rest_part[he + 4..];
        let end = memchr::memmem::find(data_start, format!("\r\n{delim}").as_bytes()).unwrap_or(data_start.len());
        let data = &data_start[..end];
        let mut name = None;
        let mut filename = None;
        let mut ctype = String::new();
        for line in head.lines() {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-disposition:") {
                for attr in line.split(';').skip(1) {
                    if let Some((k, v)) = attr.trim().split_once('=') {
                        let v = v.trim().trim_matches('"').to_string();
                        match k.trim().to_ascii_lowercase().as_str() {
                            "name" => name = Some(v),
                            "filename" => filename = Some(v),
                            _ => {}
                        }
                    }
                }
            } else if lower.starts_with("content-type:") {
                ctype = line[13..].trim().to_string();
            }
        }
        let Some(name) = name else { continue };
        match filename {
            None => add(&mut fields, format!("{}={}", encode_name(&name), urlencode(data))),
            Some(fname) => {
                let (base, sub) = match name.find('[') {
                    Some(i) => (&name[..i], &name[i..]),
                    None => (name.as_str(), ""),
                };
                let (tmp, error) = if fname.is_empty() {
                    (String::new(), 4)
                } else {
                    let tmp = std::env::temp_dir().join(format!("php{:x}{:x}", std::process::id(), rand_u64()));
                    match std::fs::write(&tmp, data) {
                        Ok(()) => {
                            let t = tmp.to_string_lossy().into_owned();
                            UPLOADS.with(|u| u.borrow_mut().insert(t.clone()));
                            (t, 0)
                        }
                        Err(_) => (String::new(), 7),
                    }
                };
                let size = if error == 0 { data.len() } else { 0 };
                let b = encode_name(base);
                let s = encode_name(sub);
                for (k, v) in [("name", fname.clone()), ("full_path", fname.clone()), ("type", ctype.clone()), ("tmp_name", tmp), ("error", error.to_string()), ("size", size.to_string())] {
                    add(&mut files, format!("{b}[{k}]{s}={}", urlencode(v.as_bytes())));
                }
            }
        }
        rest = &data_start[end..];
    }
    (fields, files)
}

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom_fill(&mut b);
    u64::from_le_bytes(b)
}

fn getrandom_fill(b: &mut [u8]) -> bool {
    // getentropy is capped at 256 bytes per call; callers ask for far less.
    unsafe { libc::getentropy(b.as_mut_ptr() as *mut libc::c_void, b.len()) == 0 }
}

/// $_FILES error and size are ints.
fn int_leaves(v: Value, under: bool) -> Value {
    match v {
        Value::Array(a) => {
            let mut out = PhpArray::new();
            for (k, x) in a.iter() {
                let here = under || matches!(&k, Key::Str(s) if &s[..] == b"error" || &s[..] == b"size");
                out.insert(k.clone(), int_leaves(x.clone(), here));
            }
            Value::array(out)
        }
        x if under => match x.scalar_bytes().and_then(|b| std::str::from_utf8(&b).ok().and_then(|s| s.parse::<i64>().ok())) {
            Some(n) => Value::Int(n),
            None => x,
        },
        x => x,
    }
}

/// A request's superglobals. `inherit` (snapshot children): $_SERVER keeps
/// what the booted template put there (e.g. .env values) under the request's
/// keys, and $_ENV is left as boot left it.
fn request_globals(it: &Rc<Interp>, frame: &mut Frame, req: &Request, cfg: &Config, script: &(String, PathBuf, String), inherit: bool) {
    let mut server = match frame.var_slot("_SERVER").borrow().clone() {
        Value::Array(a) if inherit => (*a).clone(),
        _ => PhpArray::new(),
    };
    let mut put = |k: &str, v: Value| {
        server.insert(Key::Str(pstr(k)), v);
    };
    if !inherit {
        for (k, v) in std::env::vars() {
            put(&k, Value::str(&v));
        }
        put("DOCUMENT_ROOT", Value::str(&cfg.docroot.to_string_lossy()));
    }
    put("REMOTE_ADDR", Value::str(&req.remote.ip().to_string()));
    put("REMOTE_PORT", Value::str(&req.remote.port().to_string()));
    put("SERVER_SOFTWARE", Value::str("PHP 8.5.10 Development Server"));
    put("SERVER_PROTOCOL", Value::str(&req.version));
    let (host, port) = cfg.addr.rsplit_once(':').unwrap_or((&cfg.addr, "80"));
    put("SERVER_NAME", Value::str(host));
    put("SERVER_PORT", Value::str(port));
    put("SERVER_ADDR", Value::str(&req.local.ip().to_string()));
    put("REQUEST_URI", Value::str(&req.uri));
    put("REQUEST_METHOD", Value::str(&req.method));
    put("SCRIPT_NAME", Value::str(&script.0));
    put("SCRIPT_FILENAME", Value::str(&script.1.to_string_lossy()));
    if !script.2.is_empty() {
        put("PATH_INFO", Value::str(&script.2));
    }
    put("PHP_SELF", Value::str(&format!("{}{}", script.0, script.2)));
    if !req.query().is_empty() {
        put("QUERY_STRING", Value::str(req.query()));
    }
    for (k, v) in &req.headers {
        let name = k.to_ascii_uppercase().replace('-', "_");
        match name.as_str() {
            "CONTENT_TYPE" | "CONTENT_LENGTH" => put(&name, Value::str(v)),
            _ => put(&format!("HTTP_{name}"), Value::str(v)),
        }
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    put("REQUEST_TIME_FLOAT", Value::Float(now.as_secs_f64()));
    put("REQUEST_TIME", Value::Int(now.as_secs() as i64));
    let get = parse_into(it, frame, req.query().as_bytes());
    let ctype = req.header("content-type").unwrap_or("").to_ascii_lowercase();
    let (post, files) = if req.method.eq_ignore_ascii_case("POST") && ctype.starts_with("application/x-www-form-urlencoded") {
        (parse_into(it, frame, &req.body), Value::array(PhpArray::new()))
    } else if req.method.eq_ignore_ascii_case("POST") && ctype.starts_with("multipart/form-data") {
        let raw = req.header("content-type").unwrap_or("");
        let boundary = raw.split(';').find_map(|p| p.trim().strip_prefix("boundary=").map(|b| b.trim_matches('"').to_string())).unwrap_or_default();
        let (fields, files) = multipart(&req.body, &boundary);
        (parse_into(it, frame, fields.as_bytes()), parse_into(it, frame, files.as_bytes()))
    } else {
        (Value::array(PhpArray::new()), Value::array(PhpArray::new()))
    };
    let files = int_leaves(files, false);
    let cookie = req.header("cookie").map(|c| c.split(';').map(str::trim).collect::<Vec<_>>().join("&")).unwrap_or_default();
    let cookies = parse_into(it, frame, cookie.as_bytes());
    let mut request = match &get {
        Value::Array(a) => (**a).clone(),
        _ => PhpArray::new(),
    };
    if let Value::Array(p) = &post {
        for (k, v) in p.iter() {
            request.insert(k.clone(), v.clone());
        }
    }
    let set = |f: &mut Frame, n: &'static str, v: Value| *f.var_slot(n).borrow_mut() = v;
    set(frame, "_SERVER", Value::array(server));
    set(frame, "_GET", get);
    set(frame, "_POST", post);
    set(frame, "_COOKIE", cookies);
    set(frame, "_FILES", files);
    set(frame, "_REQUEST", Value::array(request));
    if !inherit {
        set(frame, "_ENV", Value::array(PhpArray::new()));
    }
    it.response.borrow_mut().headers.push("X-Powered-By: PHP/8.5.10".into());
}

/// $_SERVER for a snapshot template: what every request shares.
fn server_base(put: &mut impl FnMut(&str, Value), cfg: &Config) {
    for (k, v) in std::env::vars() {
        put(&k, Value::str(&v));
    }
    put("DOCUMENT_ROOT", Value::str(&cfg.docroot.to_string_lossy()));
    put("SERVER_SOFTWARE", Value::str("PHP 8.5.10 Development Server"));
    let (host, port) = cfg.addr.rsplit_once(':').unwrap_or((&cfg.addr, "80"));
    put("SERVER_NAME", Value::str(host));
    put("SERVER_PORT", Value::str(port));
}

fn send_simple(stream: &mut TcpStream, code: i64, ctype: &str, body: &[u8], host: &str) {
    let head = format!(
        "HTTP/1.1 {code} {}\r\nHost: {host}\r\nDate: {}\r\nConnection: close\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\r\n",
        reason(code),
        http_date(),
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn not_found_page(path: &str) -> Vec<u8> {
    format!(
        "<!doctype html><html><head><title>404 Not Found</title><style>AAA{{}}</style></head><body><h1>Not Found</h1><p>The requested resource <code class=\"url\">{}</code> was not found on this server.</p></body></html>",
        path.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    )
    .replace("<style>AAA{}</style>", "")
    .into_bytes()
}

/// Run `script` for the request; returns (status sent, returned false).
fn run_php(stream: &TcpStream, req: &Request, cfg: &Arc<Config>, script_file: &Path, target: (String, PathBuf, String)) -> (Option<i64>, bool) {
    let src = match std::fs::read(script_file) {
        Ok(b) => crate::pstr::lossy(&b).into_owned(),
        Err(_) => return (None, false),
    };
    let interp = Rc::new(RefCell::new(Weak::new()));
    let started = Rc::new(RefCell::new(None));
    let host = req.header("host").unwrap_or("localhost").to_string();
    let Ok(sock) = stream.try_clone() else { return (None, false) };
    let out = HttpOut { stream: sock, interp: interp.clone(), host, started: started.clone() };
    // PHP consumes multipart bodies into $_POST/$_FILES; php://input is empty.
    let multipart = req.header("content-type").is_some_and(|c| c.to_ascii_lowercase().starts_with("multipart/form-data"));
    set_body(if multipart { vec![] } else { req.body.clone() });
    let cfg2 = cfg.clone();
    let req_ptr: *const Request = req;
    let interp2 = interp.clone();
    let setup: crate::Setup = Box::new(move |it, frame| {
        *interp2.borrow_mut() = Rc::downgrade(it);
        // The request outlives this call (run_main returns before it drops).
        let req = unsafe { &*req_ptr };
        request_globals(it, frame, req, &cfg2, &target, false);
        true
    });
    let path = script_file.canonicalize().unwrap_or_else(|_| script_file.to_path_buf());
    let (_, returned_false) = crate::run_main(&path, src, Box::new(out), setup);
    let sent = *started.borrow();
    if sent.is_none() && !returned_false {
        // No output: the headers go out alone.
        let mut out = HttpOut { stream: stream.try_clone().unwrap(), interp, host: req.header("host").unwrap_or("localhost").to_string(), started: started.clone() };
        let _ = out.start();
    }
    forget_uploads();
    let code = *started.borrow();
    (code, returned_false)
}

fn serve_static(stream: &mut TcpStream, req: &Request, file: &Path) -> i64 {
    let host = req.header("host").unwrap_or("localhost").to_string();
    match std::fs::read(file) {
        Ok(body) => {
            let m = mime(file);
            let ctype = if m.starts_with("text/") || m == "application/json" || m == "image/svg+xml" { format!("{m}; charset=UTF-8") } else { m.to_string() };
            if req.method.eq_ignore_ascii_case("HEAD") {
                send_simple(stream, 200, &ctype, &[], &host);
            } else {
                send_simple(stream, 200, &ctype, &body, &host);
            }
            200
        }
        Err(_) => {
            send_simple(stream, 404, "text/html; charset=UTF-8", &not_found_page(req.path()), &host);
            404
        }
    }
}

fn handle(stream: TcpStream, cfg: Arc<Config>) {
    let Some((req, peer)) = accept_request(&stream) else { return };
    respond(stream, &req, &cfg, &peer);
}

/// Read a connection's request, logging like PHP's server.
fn accept_request(stream: &TcpStream) -> Option<(Request, String)> {
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    log(&format!("{peer} Accepted"));
    match read_request(stream) {
        Ok(Some(r)) => Some((r, peer)),
        _ => {
            log(&format!("{peer} Closed without sending a request; it was probably just an unused speculative preconnection"));
            log(&format!("{peer} Closing"));
            None
        }
    }
}

/// Where a request goes without a router: (script name, file, path info).
/// A path naming no file keeps the router as its file.
fn router_target(req: &Request, cfg: &Config) -> (String, PathBuf, String) {
    resolve(&cfg.docroot, req.path()).unwrap_or_else(|| {
        let file = cfg.router_arg.clone().unwrap_or_else(|| cfg.docroot.join(req.path().trim_start_matches('/')));
        (req.path().to_string(), file, String::new())
    })
}

fn respond(stream: TcpStream, req: &Request, cfg: &Arc<Config>, peer: &str) {
    if let Some(router) = &cfg.router {
        let (sent, returned_false) = run_php(&stream, req, cfg, router, router_target(req, cfg));
        if !returned_false {
            router_done(req, sent, peer);
            return;
        }
    }
    serve_target(stream, req, cfg, peer);
}

/// The router handled the request (it didn't return false).
fn router_done(req: &Request, sent: Option<i64>, peer: &str) {
    if let Some(code) = sent {
        if code != 200 {
            log(&format!("{peer} [{code}]: {} {}", req.method, req.uri));
        }
    }
    log(&format!("{peer} Closing"));
}

/// No router, or it returned false: the PHP script or static file the path
/// names.
fn serve_target(mut stream: TcpStream, req: &Request, cfg: &Arc<Config>, peer: &str) {
    let status_log = |code: i64, note: &str| log(&format!("{peer} [{code}]: {} {}{note}", req.method, req.uri));
    match resolve(&cfg.docroot, req.path()) {
        Some(t) if t.1.extension().is_some_and(|e| e == "php") => {
            let file = t.1.clone();
            let (sent, _) = run_php(&stream, req, cfg, &file, t);
            if let Some(code) = sent {
                status_log(code, "");
            }
        }
        Some(t) => {
            let code = serve_static(&mut stream, req, &t.1);
            status_log(code, "");
        }
        None => {
            let host = req.header("host").unwrap_or("localhost").to_string();
            send_simple(&mut stream, 404, "text/html; charset=UTF-8", &not_found_page(req.path()), &host);
            status_log(404, " - No such file or directory");
        }
    }
    log(&format!("{peer} Closing"));
}

/// `-S addr`: serve until killed. Returns the exit code.
pub fn serve(addr: &str, docroot: Option<&Path>, router: Option<&Path>) -> i32 {
    let _ = SAPI.set("cli-server");
    let docroot = docroot.map(Path::to_path_buf).unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let docroot = docroot.canonicalize().unwrap_or(docroot);
    if !docroot.is_dir() {
        eprintln!("Directory {} does not exist.", docroot.display());
        return 1;
    }
    let router_arg = router.map(Path::to_path_buf);
    let router = router.map(|r| r.canonicalize().unwrap_or_else(|_| r.to_path_buf()));
    if let Some(r) = &router {
        if !r.is_file() {
            eprintln!("Could not open input file: {}", r.display());
            return 1;
        }
    }
    let listener = match TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            let why = match e.kind() {
                std::io::ErrorKind::AddrInUse => "Address already in use".to_string(),
                _ => e.to_string(),
            };
            log(&format!("Failed to listen on {addr} (reason: {why})"));
            return 1;
        }
    };
    let cfg = Arc::new(Config { docroot, router, router_arg, addr: addr.to_string() });
    // Worker threads run requests one at a time each (PHP's server has one
    // unless PHP_CLI_SERVER_WORKERS says more); a worker keeps parsed
    // files between requests, while each request gets a fresh interpreter.
    // Log lines carry the pid with more than one (artisan serve parses them).
    let workers = std::env::var("PHP_CLI_SERVER_WORKERS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 1).unwrap_or(1);
    let _ = PID_PREFIX.set(workers > 1);
    log(&format!("PHP 8.5.10 Development Server (http://{addr}) started"));
    // PHPX_SNAPSHOT=1: boot the app once and fork each request from it.
    #[cfg(unix)]
    if std::env::var_os("PHPX_SNAPSHOT").is_some_and(|v| v == "1") {
        let Err(why) = snapshot::serve(&listener, &cfg);
        log(&format!("Snapshot: off ({why}); serving every request from scratch"));
    }
    let (tx, rx) = std::sync::mpsc::channel::<TcpStream>();
    let rx = Arc::new(std::sync::Mutex::new(rx));
    let mut threads = vec![];
    for _ in 0..workers {
        let (rx, cfg) = (rx.clone(), cfg.clone());
        // A single worker takes over the main thread's thread-local fast
        // path (the main thread only accepts connections from then on).
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<usize>();
        let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
        let spawned = std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(move || {
            let _ = ready_tx.send(phpx_runtime::fastlocal::thread_id());
            if go_rx.recv() == Ok(()) && phpx_runtime::fastlocal::is_main() {
                crate::alloc::adopt_main_heap();
            }
            loop {
                let next = rx.lock().ok().and_then(|r| r.recv().ok());
                let Some(stream) = next else { return };
                handle(stream, cfg.clone());
                crate::reset_request_state();
            }
        });
        match spawned {
            Ok(t) => threads.push(t),
            Err(_) => {
                eprintln!("phpx: could not start a server worker thread");
                return 1;
            }
        }
        if let Ok(tid) = ready_rx.recv() {
            if workers == 1 {
                phpx_runtime::fastlocal::hand_main_to(tid);
            }
        }
        let _ = go_tx.send(());
    }
    // PHPX_SERVE_MAX_REQUESTS=n: stop after n connections (benchmarks and
    // profiling, which report when the process ends).
    let max = std::env::var("PHPX_SERVE_MAX_REQUESTS").ok().and_then(|v| v.parse::<usize>().ok());
    for (n, conn) in listener.incoming().enumerate() {
        let Ok(stream) = conn else { continue };
        if tx.send(stream).is_err() {
            return 1;
        }
        if max.is_some_and(|m| n + 1 >= m) {
            break;
        }
    }
    drop(tx);
    for t in threads {
        let _ = t.join();
    }
    crate::phpprof::finish();
    0
}
