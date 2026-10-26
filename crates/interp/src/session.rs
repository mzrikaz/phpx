//! ext/session: session_start() and friends with the files save handler
//! (sess_<id> files in session.save_path) or a user handler
//! (session_set_save_handler), the php and php_serialize formats, cookie
//! parameters, cache limiter headers, and writing at the end of the script.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::builtins::{arg, has, s};
use crate::interp::{BuiltinFn, Callable, Frame, Interp, Slot, R};
use crate::value::{Key, Obj, PhpArray, Value};
use std::rc::Rc;

const DISABLED: i64 = 0;
const NONE: i64 = 1;
const ACTIVE: i64 = 2;

enum Handler {
    Files,
    User(Rc<Obj>),
    Callables(Vec<Option<Callable>>),
}

struct State {
    status: i64,
    id: String,
    handler: Handler,
    /// $_SESSION's slot in the global scope.
    slot: Option<Slot>,
    /// What was read, for lazy_write.
    read: Vec<u8>,
    /// Cookie parameters set by session_set_cookie_params (None: from ini).
    cookie: Option<Cookie>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State { status: NONE, id: String::new(), handler: Handler::Files, slot: None, read: vec![], cookie: None });
}

/// A new request: no session yet, the default handler.
pub fn reset() {
    STATE.with(|s| *s.borrow_mut() = State { status: NONE, id: String::new(), handler: Handler::Files, slot: None, read: vec![], cookie: None });
}

fn ini(it: &Interp, k: &str) -> String {
    it.ini_get(k).unwrap_or_default()
}

fn ini_bool(it: &Interp, k: &str) -> bool {
    matches!(ini(it, k).to_ascii_lowercase().as_str(), "1" | "on" | "true" | "yes")
}

fn save_dir(it: &Interp) -> String {
    let p = ini(it, "session.save_path");
    // "N;MODE;/path": the directory is the last part.
    let p = p.rsplit(';').next().unwrap_or("").to_string();
    if p.is_empty() {
        std::env::temp_dir().to_string_lossy().into_owned()
    } else {
        p
    }
}

fn new_id() -> String {
    let mut b = [0u8; 16];
    unsafe { libc::getrandom(b.as_mut_ptr() as *mut libc::c_void, 16, 0) };
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 256 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b',' || c == b'-')
}

// ---- serialization ---------------------------------------------------------------------

/// End of one serialize() value starting at `i`.
fn ser_end(b: &[u8], i: usize) -> Option<usize> {
    let num_until = |from: usize, end: u8| -> Option<(usize, usize)> {
        let e = from + b.get(from..)?.iter().position(|&c| c == end)?;
        let n = std::str::from_utf8(&b[from..e]).ok()?.parse::<usize>().ok()?;
        Some((n, e + 1))
    };
    match *b.get(i)? {
        b'N' => Some(i + 2),
        b'b' | b'i' | b'd' => Some(i + b.get(i..)?.iter().position(|&c| c == b';')? + 1),
        b'r' | b'R' => Some(i + b.get(i..)?.iter().position(|&c| c == b';')? + 1),
        b's' | b'E' => {
            let (n, p) = num_until(i + 2, b':')?;
            Some(p + 1 + n + 2)
        }
        b'a' => {
            let (n, mut p) = num_until(i + 2, b':')?;
            p += 1;
            for _ in 0..n * 2 {
                p = ser_end(b, p)?;
            }
            Some(p + 1)
        }
        b'O' => {
            let (n, p) = num_until(i + 2, b':')?;
            let after_name = p + 1 + n + 2;
            let (cnt, mut p) = num_until(after_name, b':')?;
            p += 1;
            for _ in 0..cnt * 2 {
                p = ser_end(b, p)?;
            }
            Some(p + 1)
        }
        b'C' => {
            let (n, p) = num_until(i + 2, b':')?;
            let after_name = p + 1 + n + 2;
            let (dl, p) = num_until(after_name, b':')?;
            Some(p + 1 + dl + 1)
        }
        _ => None,
    }
}

fn encode(it: &Interp, data: &Value) -> R<Vec<u8>> {
    let Value::Array(a) = data.deref() else { return Ok(vec![]) };
    if ini(it, "session.serialize_handler") == "php_serialize" {
        return crate::ext::serialize(it, &Value::Array(a));
    }
    let mut out = vec![];
    for (k, v) in a.iter() {
        let name = match k {
            Key::Int(n) => n.to_string().into_bytes(),
            Key::Str(s) => s.to_vec(),
        };
        if name.contains(&b'|') {
            continue;
        }
        out.extend_from_slice(&name);
        out.push(b'|');
        out.extend_from_slice(&crate::ext::serialize(it, &v)?);
    }
    Ok(out)
}

fn decode(it: &Interp, data: &[u8]) -> R<Option<PhpArray>> {
    if data.is_empty() {
        return Ok(Some(PhpArray::new()));
    }
    if ini(it, "session.serialize_handler") == "php_serialize" {
        // Bad data is the session's failure, not unserialize()'s notice.
        it.silence.set(it.silence.get() + 1);
        let r = crate::ext::unserialize(it, data, None, 4096);
        it.silence.set(it.silence.get() - 1);
        return Ok(match r? {
            Value::Array(a) => Some((*a).clone()),
            _ => None,
        });
    }
    let mut out = PhpArray::new();
    let mut i = 0;
    while i < data.len() {
        let Some(bar) = data[i..].iter().position(|&c| c == b'|') else { return Ok(None) };
        let name = &data[i..i + bar];
        let start = i + bar + 1;
        let Some(end) = ser_end(data, start) else { return Ok(None) };
        let v = crate::ext::unserialize(it, &data[start..end], None, 4096)?;
        out.insert(Key::from_bytes(name), v);
        i = end;
    }
    Ok(Some(out))
}

// ---- storage -----------------------------------------------------------------------------

/// Call a user handler method / callable by index (open, close, read,
/// write, destroy, gc, create_sid).
fn user_call(it: &Interp, idx: usize, args: Vec<Value>) -> R<Option<Value>> {
    const NAMES: [&str; 7] = ["open", "close", "read", "write", "destroy", "gc", "create_sid"];
    let target = STATE.with(|s| match &s.borrow().handler {
        Handler::User(o) => Some((Some(o.clone()), None)),
        Handler::Callables(c) => Some((None, c.get(idx).cloned().flatten())),
        Handler::Files => None,
    });
    match target {
        Some((Some(o), _)) => {
            if o.class.find_method(NAMES[idx]).is_none() {
                return Ok(None);
            }
            it.call_method(&o, NAMES[idx], args).map(Some)
        }
        Some((None, Some(c))) => it.call_callable(&c, args, vec![]).map(Some),
        _ => Ok(None),
    }
}

fn is_files() -> bool {
    STATE.with(|s| matches!(s.borrow().handler, Handler::Files))
}

fn files_path(it: &Interp, id: &str) -> std::path::PathBuf {
    std::path::Path::new(&save_dir(it)).join(format!("sess_{id}"))
}

fn storage_read(it: &Interp, id: &str) -> R<Option<Vec<u8>>> {
    if is_files() {
        return Ok(Some(std::fs::read(files_path(it, id)).unwrap_or_default()));
    }
    let name = ini(it, "session.name");
    if !user_call(it, 0, vec![Value::str(&save_dir(it)), Value::str(&name)])?.is_none_or(|v| v.truthy()) {
        return Ok(None);
    }
    Ok(match user_call(it, 2, vec![Value::str(id)])? {
        Some(Value::Bool(false)) => None,
        Some(v) => Some(it.to_str(&v)?.to_vec()),
        None => Some(vec![]),
    })
}

fn storage_write(it: &Interp, id: &str, data: &[u8]) -> R<bool> {
    if is_files() {
        let p = files_path(it, id);
        return Ok(std::fs::write(&p, data).is_ok());
    }
    let ok = user_call(it, 3, vec![Value::str(id), Value::bytes(data.to_vec())])?.is_none_or(|v| v.truthy());
    user_call(it, 1, vec![])?;
    Ok(ok)
}

fn storage_destroy(it: &Interp, id: &str) -> R<bool> {
    if is_files() {
        let _ = std::fs::remove_file(files_path(it, id));
        return Ok(true);
    }
    let ok = user_call(it, 4, vec![Value::str(id)])?.is_none_or(|v| v.truthy());
    user_call(it, 1, vec![])?;
    Ok(ok)
}

fn gc(it: &Interp) -> R<Value> {
    let max = ini(it, "session.gc_maxlifetime").parse::<i64>().unwrap_or(1440);
    if !is_files() {
        return Ok(user_call(it, 5, vec![Value::Int(max)])?.unwrap_or(Value::Int(0)));
    }
    let now = std::time::SystemTime::now();
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(save_dir(it)) {
        for e in rd.flatten() {
            if !e.file_name().to_string_lossy().starts_with("sess_") {
                continue;
            }
            let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| now.duration_since(t).ok()).is_some_and(|d| d.as_secs() as i64 > max);
            if old && std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
    }
    Ok(Value::Int(n))
}

// ---- cookies and headers ---------------------------------------------------------------

#[derive(Clone)]
struct Cookie {
    lifetime: i64,
    path: String,
    domain: String,
    secure: bool,
    partitioned: bool,
    httponly: bool,
    samesite: String,
}

fn cookie_params(it: &Interp) -> Cookie {
    STATE.with(|s| s.borrow().cookie.clone()).unwrap_or_else(|| Cookie {
        lifetime: ini(it, "session.cookie_lifetime").parse().unwrap_or(0),
        path: ini(it, "session.cookie_path"),
        domain: ini(it, "session.cookie_domain"),
        secure: ini_bool(it, "session.cookie_secure"),
        partitioned: ini_bool(it, "session.cookie_partitioned"),
        httponly: ini_bool(it, "session.cookie_httponly"),
        samesite: ini(it, "session.cookie_samesite"),
    })
}

/// Settings can't change once output started: PHP's warning, then true.
fn headers_sent(it: &Interp, func: &str, what: &str) -> bool {
    match it.output_started() {
        Some((file, line)) => {
            it.warn(&format!("{func}(): {what} cannot be changed after headers have already been sent (sent from {file} on line {line})"));
            true
        }
        None => false,
    }
}

fn send_cookie(it: &Interp, f: &mut Frame, id: &str) -> R<()> {
    if !ini_bool(it, "session.use_cookies") {
        return Ok(());
    }
    let Cookie { lifetime: life, path, domain, secure, httponly, samesite, .. } = cookie_params(it);
    let mut opts = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        opts.insert(Key::from_bytes(k.as_bytes()), v);
    };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    put("expires", Value::Int(if life > 0 { now + life } else { 0 }));
    put("path", Value::str(&path));
    put("domain", Value::str(&domain));
    put("secure", Value::Bool(secure));
    put("httponly", Value::Bool(httponly));
    if !samesite.is_empty() {
        put("samesite", Value::str(&samesite));
    }
    let name = ini(it, "session.name");
    let mut args = [Value::str(&name), Value::str(id), Value::array(opts)];
    (it.builtins["setcookie"])(it, f, &mut args)?;
    Ok(())
}

fn cache_headers(it: &Interp, f: &mut Frame) -> R<()> {
    let expire = ini(it, "session.cache_expire").parse::<i64>().unwrap_or(180);
    let lines: Vec<String> = match ini(it, "session.cache_limiter").as_str() {
        "nocache" => vec!["Expires: Thu, 19 Nov 1981 08:52:00 GMT".into(), "Cache-Control: no-store, no-cache, must-revalidate".into(), "Pragma: no-cache".into()],
        "private" => vec!["Expires: Thu, 19 Nov 1981 08:52:00 GMT".into(), format!("Cache-Control: private, max-age={}", expire * 60)],
        "private_no_expire" => vec![format!("Cache-Control: private, max-age={}", expire * 60)],
        "public" => vec![format!("Cache-Control: public, max-age={}", expire * 60)],
        _ => vec![],
    };
    for l in lines {
        (it.builtins["header"])(it, f, &mut [Value::str(&l)])?;
    }
    Ok(())
}

// ---- lifecycle -----------------------------------------------------------------------------

fn session_slot(it: &Interp, f: &mut Frame) -> Slot {
    it.global_slot(f, "_SESSION")
}

fn start(it: &Interp, f: &mut Frame, a: &[Value]) -> R<Value> {
    if STATE.with(|s| s.borrow().status) == ACTIVE {
        it.notice("session_start(): Ignoring session_start() because a session is already active");
        return Ok(Value::Bool(true));
    }
    if let Some((file, line)) = it.output_started() {
        it.warn(&format!("session_start(): Session cannot be started after headers have already been sent (sent from {file} on line {line})"));
        return Ok(Value::Bool(false));
    }
    let mut read_and_close = false;
    if let Value::Array(opts) = arg(a, 0) {
        for (k, v) in opts.iter() {
            let k = crate::pstr::lossy(&k.to_value().scalar_bytes().unwrap_or_default()).into_owned();
            if k == "read_and_close" {
                read_and_close = v.truthy();
                continue;
            }
            let val = it.to_string_lossy(&v)?;
            it.ini.borrow_mut().set(&format!("session.{k}"), &val);
        }
    }
    let name = ini(it, "session.name");
    let mut id = STATE.with(|s| s.borrow().id.clone());
    if id.is_empty() {
        let from_cookie = it.with_globals(f, |v| v.get("_COOKIE").map(|s| s.borrow().clone()));
        if let Some(Value::Array(c)) = from_cookie {
            if let Some(v) = c.get(&Key::from_bytes(name.as_bytes())) {
                let cand = it.to_string_lossy(&v)?;
                if valid_id(&cand) {
                    id = cand;
                }
            }
        }
    }
    let fresh = id.is_empty();
    if fresh {
        id = match user_call(it, 6, vec![])? {
            Some(v) if !matches!(v, Value::Bool(false)) => it.to_string_lossy(&v)?,
            _ => new_id(),
        };
    }
    let Some(raw) = storage_read(it, &id)? else {
        it.warn(&format!("session_start(): Failed to read session data: {} (path: {})", ini(it, "session.save_handler"), save_dir(it)));
        return Ok(Value::Bool(false));
    };
    let data = match decode(it, &raw)? {
        Some(d) => d,
        None => {
            it.warn("session_start(): Failed to decode session object. Session has been destroyed");
            storage_destroy(it, &id)?;
            *session_slot(it, f).borrow_mut() = Value::array(PhpArray::new());
            return Ok(Value::Bool(false));
        }
    };
    let slot = session_slot(it, f);
    *slot.borrow_mut() = Value::array(data);
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.status = ACTIVE;
        s.id = id.clone();
        s.slot = Some(slot);
        s.read = raw;
    });
    // The CLI sends no headers.
    if crate::server::sapi() != "cli" {
        send_cookie(it, f, &id)?;
        cache_headers(it, f)?;
    }
    if read_and_close {
        STATE.with(|s| s.borrow_mut().status = NONE);
    }
    Ok(Value::Bool(true))
}

/// Write the session (session_write_close and the end of the script).
pub fn write_close(it: &Interp) -> R<bool> {
    let (status, id, slot, read) = STATE.with(|s| {
        let s = s.borrow();
        (s.status, s.id.clone(), s.slot.clone(), s.read.clone())
    });
    if status != ACTIVE {
        return Ok(false);
    }
    let data = slot.map(|s| s.borrow().clone()).unwrap_or_default();
    let enc = encode(it, &data)?;
    let lazy = ini_bool(it, "session.lazy_write") || it.ini_get("session.lazy_write").is_none();
    let ok = if lazy && enc == read && !is_files() {
        user_call(it, 1, vec![])?;
        true
    } else {
        storage_write(it, &id, &enc)?
    };
    STATE.with(|s| s.borrow_mut().status = NONE);
    if !ok {
        it.warn(&format!("session_write_close(): Failed to write session data using user defined save handler. (session.save_path: {}, handler: write)", ini(it, "session.save_path")));
    }
    Ok(ok)
}

/// End of the script: an active session is written.
pub fn finish(it: &Interp) -> R<()> {
    write_close(it).map(|_| ())
}

fn set_save_handler(it: &Interp, f: &mut Frame, a: &[Value]) -> R<Value> {
    if STATE.with(|s| s.borrow().status) == ACTIVE {
        it.warn("session_set_save_handler(): Session save handler cannot be changed when a session is active");
        return Ok(Value::Bool(false));
    }
    if headers_sent(it, "session_set_save_handler", "Session save handler") {
        return Ok(Value::Bool(false));
    }
    match arg(a, 0) {
        Value::Object(o) if !o.class.is_a("closure") => {
            if !o.class.is_a("sessionhandlerinterface") {
                return Err(it.error("TypeError", "session_set_save_handler(): Argument #1 ($open) must be of type SessionHandlerInterface"));
            }
            STATE.with(|s| s.borrow_mut().handler = Handler::User(o));
            it.ini.borrow_mut().set("session.save_handler", "user");
            if !has(a, 1) || a[1].truthy() {
                (it.builtins["session_register_shutdown"])(it, f, &mut [])?;
            }
        }
        _ => {
            let mut cbs = vec![];
            for i in 0..7 {
                cbs.push(match a.get(i) {
                    Some(v) if !v.is_null() => Some(it.to_callable(f, v)?),
                    _ => None,
                });
            }
            STATE.with(|s| s.borrow_mut().handler = Handler::Callables(cbs));
            it.ini.borrow_mut().set("session.save_handler", "user");
        }
    }
    Ok(Value::Bool(true))
}

fn files_native(it: &Interp, a: &[Value], op: &str) -> R<Value> {
    let saved = STATE.with(|s| std::mem::replace(&mut s.borrow_mut().handler, Handler::Files));
    let r = (|| -> R<Value> {
        Ok(match op {
            "open" | "close" => Value::Bool(true),
            "read" => match storage_read(it, &it.to_string_lossy(&arg(a, 0))?)? {
                Some(b) => Value::bytes(b),
                None => Value::Bool(false),
            },
            "write" => Value::Bool(storage_write(it, &it.to_string_lossy(&arg(a, 0))?, &it.to_str(&arg(a, 1))?)?),
            "destroy" => Value::Bool(storage_destroy(it, &it.to_string_lossy(&arg(a, 0))?)?),
            "gc" => gc(it)?,
            _ => Value::str(&new_id()),
        })
    })();
    STATE.with(|s| s.borrow_mut().handler = saved);
    r
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("session_start", |it, f, a| start(it, f, a));
    t.insert("session_status", |_, _, _| Ok(Value::Int(STATE.with(|s| s.borrow().status))));
    t.insert("session_id", |it, _, a| {
        let old = STATE.with(|s| s.borrow().id.clone());
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            if STATE.with(|s| s.borrow().status) == ACTIVE {
                it.warn("session_id(): Session ID cannot be changed when a session is active");
                return Ok(Value::Bool(false));
            }
            if headers_sent(it, "session_id", "Session ID") {
                return Ok(Value::Bool(false));
            }
            let id = it.to_string_lossy(v)?;
            STATE.with(|s| s.borrow_mut().id = id);
        }
        Ok(Value::str(&old))
    });
    t.insert("session_name", |it, _, a| {
        let old = ini(it, "session.name");
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            if STATE.with(|s| s.borrow().status) == ACTIVE {
                it.warn("session_name(): Session name cannot be changed when a session is active");
                return Ok(Value::Bool(false));
            }
            if headers_sent(it, "session_name", "Session name") {
                return Ok(Value::Bool(false));
            }
            let n = it.to_string_lossy(v)?;
            if n.is_empty() {
                return Err(it.error("ValueError", "session_name(): Argument #1 ($name) cannot be empty"));
            }
            it.ini.borrow_mut().set("session.name", &n);
        }
        Ok(Value::str(&old))
    });
    t.insert("session_save_path", |it, _, a| {
        let old = ini(it, "session.save_path");
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            if headers_sent(it, "session_save_path", "Session save path") {
                return Ok(Value::Bool(false));
            }
            let p = it.to_string_lossy(v)?;
            it.ini.borrow_mut().set("session.save_path", &p);
        }
        Ok(Value::str(&old))
    });
    t.insert("session_module_name", |it, _, a| {
        let old = ini(it, "session.save_handler");
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            if headers_sent(it, "session_module_name", "Session save handler module") {
                return Ok(Value::Bool(false));
            }
            let m = it.to_string_lossy(v)?;
            if m == "user" {
                return Err(it.error("ValueError", "session_module_name(): Argument #1 ($module) cannot be \"user\""));
            }
            if m != "files" {
                it.warn(&format!("session_module_name(): Session handler module \"{m}\" cannot be found"));
                return Ok(Value::Bool(false));
            }
            STATE.with(|s| s.borrow_mut().handler = Handler::Files);
            it.ini.borrow_mut().set("session.save_handler", "files");
        }
        Ok(Value::str(&old))
    });
    t.insert("session_create_id", |it, _, a| {
        let prefix = if has(a, 0) { crate::pstr::lossy(&s(it, a, 0)?).into_owned() } else { String::new() };
        if !prefix.is_empty() && !valid_id(&prefix) {
            it.warn("session_create_id(): Prefix cannot contain special characters. Only the A-Z, a-z, 0-9, \"-\", and \",\" characters are allowed");
            return Ok(Value::Bool(false));
        }
        Ok(Value::str(&format!("{prefix}{}", new_id())))
    });
    t.insert("session_regenerate_id", |it, f, a| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            it.warn("session_regenerate_id(): Session ID cannot be regenerated when there is no active session");
            return Ok(Value::Bool(false));
        }
        let delete_old = a.first().is_some_and(|v| v.truthy());
        let old = STATE.with(|s| s.borrow().id.clone());
        if delete_old {
            storage_destroy(it, &old)?;
        } else {
            let data = STATE.with(|s| s.borrow().slot.clone()).map(|s| s.borrow().clone()).unwrap_or_default();
            let enc = encode(it, &data)?;
            storage_write(it, &old, &enc)?;
        }
        let id = new_id();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.id = id.clone();
            // Force the next write: the new id has no data yet.
            s.read.clear();
            s.read.push(0);
        });
        if crate::server::sapi() != "cli" {
            send_cookie(it, f, &id)?;
        }
        Ok(Value::Bool(true))
    });
    t.insert("session_encode", |it, f, _| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            it.warn("session_encode(): Cannot encode non-existent session");
            return Ok(Value::Bool(false));
        }
        let slot = session_slot(it, f);
        let v = slot.borrow().clone();
        Ok(Value::bytes(encode(it, &v)?))
    });
    t.insert("session_decode", |it, f, a| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            it.warn("session_decode(): Session data cannot be decoded when there is no active session");
            return Ok(Value::Bool(false));
        }
        let data = s(it, a, 0)?;
        match decode(it, &data)? {
            Some(d) => {
                let slot = session_slot(it, f);
                let mut cur = match slot.borrow().deref() {
                    Value::Array(x) => (*x).clone(),
                    _ => PhpArray::new(),
                };
                for (k, v) in d.iter() {
                    cur.insert(k, v);
                }
                *slot.borrow_mut() = Value::array(cur);
                Ok(Value::Bool(true))
            }
            None => {
                it.warn("session_decode(): Failed to decode session object. Session has been destroyed");
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("session_destroy", |it, _, _| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            it.warn("session_destroy(): Trying to destroy uninitialized session");
            return Ok(Value::Bool(false));
        }
        let id = STATE.with(|s| s.borrow().id.clone());
        let ok = storage_destroy(it, &id)?;
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.status = NONE;
            s.id.clear();
        });
        Ok(Value::Bool(ok))
    });
    t.insert("session_unset", |it, f, _| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            return Ok(Value::Bool(false));
        }
        *session_slot(it, f).borrow_mut() = Value::array(PhpArray::new());
        Ok(Value::Bool(true))
    });
    t.insert("session_gc", |it, _, _| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            it.warn("session_gc(): Session cannot be garbage collected when there is no active session");
            return Ok(Value::Bool(false));
        }
        gc(it)
    });
    let close: BuiltinFn = |it, _, _| Ok(Value::Bool(write_close(it)?));
    t.insert("session_write_close", close);
    t.insert("session_commit", close);
    t.insert("session_abort", |it, _, _| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            return Ok(Value::Bool(false));
        }
        if !is_files() {
            user_call(it, 1, vec![])?;
        }
        STATE.with(|s| s.borrow_mut().status = NONE);
        Ok(Value::Bool(true))
    });
    t.insert("session_reset", |it, f, _| {
        if STATE.with(|s| s.borrow().status) != ACTIVE {
            return Ok(Value::Bool(false));
        }
        let read = STATE.with(|s| s.borrow().read.clone());
        let data = decode(it, &read)?.unwrap_or_default();
        *session_slot(it, f).borrow_mut() = Value::array(data);
        Ok(Value::Bool(true))
    });
    t.insert("session_register_shutdown", |it, f, _| {
        let cb = it.to_callable(f, &Value::str("session_write_close"))?;
        it.response.borrow_mut().shutdown.push((cb, vec![]));
        Ok(Value::Null)
    });
    t.insert("session_set_save_handler", |it, f, a| set_save_handler(it, f, a));
    t.insert("session_get_cookie_params", |it, _, _| {
        let c = cookie_params(it);
        let mut out = PhpArray::new();
        let mut put = |k: &str, v: Value| {
            out.insert(Key::from_bytes(k.as_bytes()), v);
        };
        put("lifetime", Value::Int(c.lifetime));
        put("path", Value::str(&c.path));
        put("domain", Value::str(&c.domain));
        put("secure", Value::Bool(c.secure));
        put("partitioned", Value::Bool(c.partitioned));
        put("httponly", Value::Bool(c.httponly));
        put("samesite", Value::str(&c.samesite));
        Ok(Value::array(out))
    });
    t.insert("session_set_cookie_params", |it, _, a| {
        if STATE.with(|s| s.borrow().status) == ACTIVE {
            it.warn("session_set_cookie_params(): Session cookie parameters cannot be changed when a session is active");
            return Ok(Value::Bool(false));
        }
        if headers_sent(it, "session_set_cookie_params", "Session cookie parameters") {
            return Ok(Value::Bool(false));
        }
        let mut p = cookie_params(it);
        match arg(a, 0) {
            Value::Array(o) => {
                for (k, v) in o.iter() {
                    let k = crate::pstr::lossy(&k.to_value().scalar_bytes().unwrap_or_default()).to_ascii_lowercase();
                    match k.as_str() {
                        "lifetime" => p.lifetime = it.to_int(&v)?,
                        "path" => p.path = it.to_string_lossy(&v)?,
                        "domain" => p.domain = it.to_string_lossy(&v)?,
                        "secure" => p.secure = v.truthy(),
                        "partitioned" => p.partitioned = v.truthy(),
                        "httponly" => p.httponly = v.truthy(),
                        "samesite" => p.samesite = it.to_string_lossy(&v)?,
                        _ => {
                            it.warn(&format!("session_set_cookie_params(): Argument #1 ($lifetime_or_options) contains an unrecognized key \"{k}\""));
                            return Ok(Value::Bool(false));
                        }
                    }
                }
            }
            v => {
                p.lifetime = it.to_int(&v)?;
                if let Some(x) = a.get(1).filter(|v| !v.is_null()) {
                    p.path = it.to_string_lossy(x)?;
                }
                if let Some(x) = a.get(2).filter(|v| !v.is_null()) {
                    p.domain = it.to_string_lossy(x)?;
                }
                if let Some(x) = a.get(3).filter(|v| !v.is_null()) {
                    p.secure = x.truthy();
                }
                if let Some(x) = a.get(4).filter(|v| !v.is_null()) {
                    p.httponly = x.truthy();
                }
            }
        }
        STATE.with(|s| s.borrow_mut().cookie = Some(p));
        Ok(Value::Bool(true))
    });
    t.insert("session_cache_limiter", |it, _, a| {
        let old = ini(it, "session.cache_limiter");
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            if headers_sent(it, "session_cache_limiter", "Session cache limiter") {
                return Ok(Value::Bool(false));
            }
            let l = it.to_string_lossy(v)?;
            it.ini.borrow_mut().set("session.cache_limiter", &l);
        }
        Ok(Value::str(&old))
    });
    t.insert("session_cache_expire", |it, _, a| {
        let old = ini(it, "session.cache_expire").parse::<i64>().unwrap_or(180);
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            if headers_sent(it, "session_cache_expire", "Session cache expiration") {
                return Ok(Value::Bool(false));
            }
            let n = it.to_int(v)?;
            it.ini.borrow_mut().set("session.cache_expire", &n.to_string());
        }
        Ok(Value::Int(old))
    });
    t.insert("__phpx_session_files", |it, _, a| {
        let op = it.to_string_lossy(&arg(a, 0))?;
        files_native(it, &a[1..], &op)
    });
    let _ = DISABLED;
}
