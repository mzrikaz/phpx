//! ext/redis (phpredis 6): the Redis class on a native RESP client.
//! Each method is described by a spec (Redis command, which arguments are
//! keys for OPT_PREFIX, which are values for OPT_SERIALIZER, and how the
//! reply converts); the irregular ones have their own handlers. Replies
//! convert the way phpredis does (OK -> true, nil -> false, WITHSCORES ->
//! member => score, ...); MULTI and pipelines queue the conversions.

mod conn;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use conn::{Conn, Reply};

use crate::class::Class;
use crate::interp::{Ctrl, Frame, Interp, NativeMethod, R};
use crate::value::{Internal, Key, NativeData, Obj, PhpArray, Value};

const OPT_SERIALIZER: i64 = 1;
const OPT_PREFIX: i64 = 2;
const OPT_READ_TIMEOUT: i64 = 3;
const OPT_SCAN: i64 = 4;
const OPT_COMPRESSION: i64 = 7;
const OPT_REPLY_LITERAL: i64 = 8;
const OPT_COMPRESSION_LEVEL: i64 = 9;
const OPT_NULL_MULTIBULK_AS_NULL: i64 = 10;
const OPT_MAX_RETRIES: i64 = 11;
const OPT_BACKOFF_ALGORITHM: i64 = 12;
const OPT_BACKOFF_BASE: i64 = 13;
const OPT_BACKOFF_CAP: i64 = 14;
const OPT_TCP_KEEPALIVE: i64 = 6;

const SERIALIZER_NONE: i64 = 0;
const SERIALIZER_PHP: i64 = 1;
const SERIALIZER_JSON: i64 = 4;
const SCAN_RETRY: i64 = 1;

/// How a reply converts.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Out {
    /// +OK -> true; nil -> false.
    Ok,
    /// 1/0 -> true/false.
    Bool,
    Int,
    /// Bulk string (a stored value: unserialized); nil -> false.
    Val,
    /// Bulk string as is (not a stored value).
    Str,
    /// Bulk -> float; nil -> false.
    Float,
    /// Array of stored values (nil elements -> false).
    Vals,
    /// Array of plain strings.
    Strs,
    /// Flat pairs -> assoc, values unserialized (HGETALL).
    Map,
    /// Flat member/score pairs -> member => float.
    Scores,
    /// TYPE -> Redis::REDIS_* code.
    Type,
    /// Recursive, phpredis's raw conversion (EVAL, rawCommand).
    Raw,
    Ping,
    Info,
    /// BLPOP & co: nil -> [], else [key, value].
    Pop,
    /// HMGET: values keyed by the requested fields.
    Fields,
    /// Bulk -> int (TIME parts etc. stay strings): array of strings.
    Time,
}

#[derive(Clone, Copy, PartialEq)]
enum Keys {
    None,
    First,
    All,
    AllButLast,
    FirstTwo,
}

#[derive(Clone, Copy, PartialEq)]
enum Ser {
    None,
    /// The last argument is a value.
    Last,
    /// Every argument after the key.
    Rest,
    /// The third argument (SETEX key ttl value).
    Third,
}

struct Spec {
    cmd: &'static str,
    keys: Keys,
    ser: Ser,
    out: Out,
}

impl Spec {
    /// Array arguments expand into separate arguments (key lists like
    /// del(['a', 'b']), sAddArray()); elsewhere an array is a value.
    fn flattens(&self) -> bool {
        matches!(self.keys, Keys::All | Keys::AllButLast) || self.cmd == "SADD"
    }
}

enum Mode {
    Atomic,
    Multi(Vec<Pending>),
    Pipeline(Vec<(Vec<Vec<u8>>, Pending)>),
}

#[derive(Clone)]
struct Pending {
    out: Out,
    /// HMGET's requested fields.
    fields: Vec<Value>,
}

struct State {
    conn: Option<Conn>,
    host: String,
    port: i64,
    timeout: f64,
    read_timeout: f64,
    persistent_id: Option<String>,
    db: i64,
    auth: Value,
    prefix: Vec<u8>,
    serializer: i64,
    scan: i64,
    last_error: Option<String>,
    mode: Mode,
    options: Vec<(i64, Value)>,
}

struct RedisData(RefCell<State>);

impl NativeData for RedisData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

fn state<T>(it: &Interp, this: Option<&Rc<Obj>>, f: impl FnOnce(&mut State) -> T) -> R<T> {
    let o = this.ok_or_else(|| it.error("Error", "Non-static method called statically"))?;
    if !matches!(&*o.internal(), Internal::Data(d) if d.as_any().downcast_ref::<RedisData>().is_some()) {
        o.set_internal(Internal::Data(Box::new(RedisData(RefCell::new(State {
            conn: None,
            host: String::new(),
            port: 6379,
            timeout: 0.0,
            read_timeout: 0.0,
            persistent_id: None,
            db: 0,
            auth: Value::Null,
            prefix: vec![],
            serializer: SERIALIZER_NONE,
            scan: 0,
            last_error: None,
            mode: Mode::Atomic,
            options: vec![],
        })))));
    }
    let g = o.internal();
    let Internal::Data(d) = &*g else { unreachable!() };
    let st = d.as_any().downcast_ref::<RedisData>().unwrap();
    let r = f(&mut st.0.borrow_mut());
    Ok(r)
}

fn redis_exception(it: &Interp, msg: &str) -> Ctrl {
    it.error("RedisException", msg)
}

fn bytes(it: &Interp, v: &Value) -> R<Vec<u8>> {
    Ok(match v.deref() {
        Value::Float(f) => format!("{f}").into_bytes(),
        v => it.to_str(&v)?.to_vec(),
    })
}

fn serialize(it: &Interp, ser: i64, v: &Value) -> R<Vec<u8>> {
    match ser {
        SERIALIZER_PHP => crate::ext::serialize(it, v),
        SERIALIZER_JSON => {
            let mut f = Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
            let mut args = [v.clone()];
            Ok(it.to_str(&(it.builtins["json_encode"])(it, &mut f, &mut args)?)?.to_vec())
        }
        _ => bytes(it, v),
    }
}

fn unserialize(it: &Interp, ser: i64, b: Vec<u8>) -> R<Value> {
    match ser {
        SERIALIZER_PHP => {
            let silence = it.silence.get();
            it.silence.set(silence + 1);
            let r = crate::ext::unserialize(it, &b, None, 4096);
            it.silence.set(silence);
            match r {
                Ok(Value::Bool(false)) | Err(_) => Ok(Value::bytes(b)),
                Ok(v) => Ok(v),
            }
        }
        SERIALIZER_JSON => {
            let mut f = Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
            let mut args = [Value::bytes(b), Value::Bool(true)];
            (it.builtins["json_decode"])(it, &mut f, &mut args)
        }
        _ => Ok(Value::bytes(b)),
    }
}

/// Flatten arguments: arrays expand to their values.
fn flat(a: &[Value]) -> Vec<Value> {
    let mut out = vec![];
    for v in a {
        match v.deref() {
            Value::Array(arr) => out.extend(arr.values()),
            v => out.push(v),
        }
    }
    out
}

fn with_prefix(prefix: &[u8], k: Vec<u8>) -> Vec<u8> {
    if prefix.is_empty() {
        k
    } else {
        let mut p = prefix.to_vec();
        p.extend_from_slice(&k);
        p
    }
}

/// Build a command's argument list from PHP values per the spec.
fn build(it: &Interp, st: &State, cmd: &str, args: &[Value], keys: Keys, ser: Ser) -> R<Vec<Vec<u8>>> {
    let n = args.len();
    let mut out: Vec<Vec<u8>> = cmd.split(' ').map(|c| c.as_bytes().to_vec()).collect();
    for (i, v) in args.iter().enumerate() {
        let is_key = match keys {
            Keys::None => false,
            Keys::First => i == 0,
            Keys::All => true,
            Keys::AllButLast => i + 1 < n,
            Keys::FirstTwo => i < 2,
        };
        let is_val = match ser {
            Ser::None => false,
            Ser::Last => i + 1 == n && n > 1,
            Ser::Rest => i > 0,
            Ser::Third => i == 2,
        };
        let b = if is_val && !is_key { serialize(it, st.serializer, v)? } else { bytes(it, v)? };
        out.push(if is_key { with_prefix(&st.prefix, b) } else { b });
    }
    Ok(out)
}

/// Convert a reply. `Err` replies record the last error and give false.
fn convert(it: &Interp, st: &mut State, r: Reply, p: &Pending) -> R<Value> {
    if let Reply::Error(e) = &r {
        st.last_error = Some(e.clone());
        return Ok(Value::Bool(false));
    }
    let ser = st.serializer;
    let val = |b: Option<Vec<u8>>| -> R<Value> {
        match b {
            Some(b) => unserialize(it, ser, b),
            None => Ok(Value::Bool(false)),
        }
    };
    Ok(match (p.out, r) {
        (Out::Ok, Reply::Status(_)) => Value::Bool(true),
        (Out::Ok, Reply::Bulk(None)) | (Out::Ok, Reply::Array(None)) => Value::Bool(false),
        (Out::Ok, Reply::Int(n)) => Value::Bool(n != 0),
        (Out::Ok, Reply::Bulk(Some(_))) => Value::Bool(true),
        (Out::Bool, Reply::Int(n)) => Value::Bool(n != 0),
        (Out::Bool, Reply::Status(_)) => Value::Bool(true),
        (Out::Bool, Reply::Bulk(None)) => Value::Bool(false),
        (Out::Int, Reply::Int(n)) => Value::Int(n),
        (Out::Int, Reply::Bulk(None)) => Value::Bool(false),
        (Out::Val, Reply::Bulk(b)) => val(b)?,
        (Out::Str, Reply::Bulk(Some(b))) => Value::bytes(b),
        (Out::Str, Reply::Bulk(None)) => Value::Bool(false),
        (Out::Str, Reply::Status(s)) => Value::str(&s),
        (Out::Float, Reply::Bulk(Some(b))) => Value::Float(String::from_utf8_lossy(&b).parse().unwrap_or(0.0)),
        (Out::Float, Reply::Bulk(None)) => Value::Bool(false),
        (Out::Float, Reply::Int(n)) => Value::Float(n as f64),
        (Out::Vals, Reply::Array(items)) | (Out::Strs, Reply::Array(items)) => {
            let mut arr = PhpArray::new();
            for item in items.unwrap_or_default() {
                arr.push(match item {
                    Reply::Bulk(b) if p.out == Out::Vals => val(b)?,
                    Reply::Bulk(Some(b)) => Value::bytes(b),
                    Reply::Bulk(None) => Value::Bool(false),
                    Reply::Int(n) => Value::Int(n),
                    other => raw(it, st, other)?,
                });
            }
            Value::array(arr)
        }
        (Out::Map, Reply::Array(items)) | (Out::Scores, Reply::Array(items)) => {
            let items = items.unwrap_or_default();
            let mut arr = PhpArray::new();
            let mut it2 = items.into_iter();
            while let (Some(k), Some(v)) = (it2.next(), it2.next()) {
                let k = match k {
                    Reply::Bulk(Some(b)) => b,
                    Reply::Int(n) => n.to_string().into_bytes(),
                    _ => continue,
                };
                let v = match (p.out, v) {
                    (Out::Scores, Reply::Bulk(Some(b))) => Value::Float(String::from_utf8_lossy(&b).parse().unwrap_or(0.0)),
                    (Out::Map, Reply::Bulk(b)) => val(b)?,
                    (_, other) => raw(it, st, other)?,
                };
                arr.insert(Key::from_bytes(&k), v);
            }
            Value::array(arr)
        }
        (Out::Fields, Reply::Array(items)) => {
            let mut arr = PhpArray::new();
            for (f, item) in p.fields.iter().zip(items.unwrap_or_default()) {
                let v = match item {
                    Reply::Bulk(b) => val(b)?,
                    other => raw(it, st, other)?,
                };
                arr.insert(Key::from_bytes(&it.to_str(f)?), v);
            }
            Value::array(arr)
        }
        (Out::Type, Reply::Status(s)) => Value::Int(match s.as_str() {
            "string" => 1,
            "set" => 2,
            "list" => 3,
            "zset" => 4,
            "hash" => 5,
            "stream" => 6,
            _ => 0,
        }),
        (Out::Ping, Reply::Status(_)) => Value::Bool(true),
        (Out::Ping, Reply::Bulk(Some(b))) => Value::bytes(b),
        (Out::Info, Reply::Bulk(Some(b))) => {
            let mut arr = PhpArray::new();
            for line in String::from_utf8_lossy(&b).lines() {
                if line.starts_with('#') || line.is_empty() {
                    continue;
                }
                if let Some((k, v)) = line.split_once(':') {
                    let v = match v.parse::<i64>() {
                        Ok(n) if n.to_string() == v => Value::Int(n),
                        _ => match v.parse::<f64>() {
                            Ok(f) if v.contains('.') && !v.contains(',') => Value::Float(f),
                            _ => Value::str(v),
                        },
                    };
                    arr.insert(Key::from_bytes(k.as_bytes()), v);
                }
            }
            Value::array(arr)
        }
        (Out::Pop, Reply::Array(None)) => Value::array(PhpArray::new()),
        (Out::Pop, Reply::Array(Some(items))) => {
            let mut arr = PhpArray::new();
            for (i, item) in items.into_iter().enumerate() {
                arr.push(match item {
                    Reply::Bulk(b) if i > 0 => val(b)?,
                    Reply::Bulk(Some(b)) => Value::bytes(b),
                    other => raw(it, st, other)?,
                });
            }
            Value::array(arr)
        }
        (Out::Time, Reply::Array(items)) => {
            let mut arr = PhpArray::new();
            for item in items.unwrap_or_default() {
                if let Reply::Bulk(Some(b)) = item {
                    arr.push(Value::bytes(b));
                }
            }
            Value::array(arr)
        }
        (_, other) => raw(it, st, other)?,
    })
}

/// phpredis's raw reply conversion.
fn raw(it: &Interp, st: &mut State, r: Reply) -> R<Value> {
    Ok(match r {
        Reply::Status(_) => Value::Bool(true),
        Reply::Error(e) => {
            st.last_error = Some(e);
            Value::Bool(false)
        }
        Reply::Int(n) => Value::Int(n),
        Reply::Bulk(Some(b)) => Value::bytes(b),
        Reply::Bulk(None) => Value::Bool(false),
        Reply::Array(None) => Value::Bool(false),
        Reply::Array(Some(items)) => {
            let mut arr = PhpArray::new();
            for i in items {
                arr.push(raw(it, st, i)?);
            }
            Value::array(arr)
        }
    })
}

/// Send a command (or queue it in MULTI/pipeline mode) and convert.
fn exec(it: &Interp, this: Option<&Rc<Obj>>, cmd: Vec<Vec<u8>>, p: Pending) -> R<Value> {
    let o = this.unwrap().clone();
    let r = state(it, this, |st| -> Result<Option<Reply>, String> {
        match &mut st.mode {
            Mode::Pipeline(q) => {
                q.push((cmd, p.clone()));
                Ok(None)
            }
            Mode::Multi(q) => {
                q.push(p.clone());
                let conn = st.conn.as_mut().ok_or_else(|| "Redis server went away".to_string())?;
                match conn.command(&cmd) {
                    Ok(Reply::Error(e)) => {
                        st.last_error = Some(e);
                        Ok(None)
                    }
                    Ok(_) => Ok(None),
                    Err(e) => Err(e.0),
                }
            }
            Mode::Atomic => {
                let conn = st.conn.as_mut().ok_or_else(|| "Redis server went away".to_string())?;
                conn.command(&cmd).map(Some).map_err(|e| e.0)
            }
        }
    })?;
    match r {
        Err(msg) => {
            state(it, this, |st| st.conn = None)?;
            Err(redis_exception(it, &msg))
        }
        Ok(None) => Ok(Value::Object(o)),
        Ok(Some(reply)) => state(it, this, |st| convert(it, st, reply, &p))?,
    }
}

fn pending(out: Out) -> Pending {
    Pending { out, fields: vec![] }
}

fn run(it: &Interp, this: Option<&Rc<Obj>>, a: &[Value], s: &Spec) -> R<Value> {
    let args = if s.flattens() { flat(a) } else { a.iter().map(|v| v.deref()).collect() };
    let cmd = state(it, this, |st| build(it, st, s.cmd, &args, s.keys, s.ser))??;
    exec(it, this, cmd, pending(s.out))
}

macro_rules! spec {
    ($cmd:literal, $keys:ident, $ser:ident, $out:ident) => {{
        const S: Spec = Spec { cmd: $cmd, keys: Keys::$keys, ser: Ser::$ser, out: Out::$out };
        (|it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>| -> R<Value> { run(it, this, &a, &S) }) as NativeMethod
    }};
}

fn arg(a: &[Value], i: usize) -> Value {
    a.get(i).cloned().unwrap_or_default().into_deref()
}

fn float_arg(it: &Interp, a: &[Value], i: usize) -> R<f64> {
    Ok(match a.get(i) {
        Some(v) if !v.is_null() => it.to_float(v)?,
        _ => 0.0,
    })
}

// ---- connection ---------------------------------------------------------------------

fn connect(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, persistent: bool) -> R<Value> {
    let host = String::from_utf8_lossy(&it.to_str(&arg(&a, 0))?).into_owned();
    let unix = host.starts_with('/') || host.starts_with("unix://");
    let port = match a.get(1) {
        Some(v) if !v.is_null() && !unix => it.to_int(v)?,
        _ => 6379,
    };
    let port = if port <= 0 { 6379 } else { port };
    let timeout = float_arg(it, &a, 2)?;
    let persistent_id = if persistent { a.get(3).filter(|v| !v.is_null()).map(|v| it.to_string_lossy(v)).transpose()? } else { None };
    let read_timeout = float_arg(it, &a, if persistent { 5 } else { 5 })?;
    let to = (timeout > 0.0).then(|| Duration::from_secs_f64(timeout));
    match Conn::connect(&host, port as u16, to) {
        Ok(mut c) => {
            if read_timeout > 0.0 {
                c.read_timeout = Some(Duration::from_secs_f64(read_timeout));
            }
            state(it, this, |st| {
                st.conn = Some(c);
                st.host = host;
                st.port = port;
                st.timeout = timeout;
                st.read_timeout = read_timeout;
                st.persistent_id = persistent_id;
                st.db = 0;
                st.mode = Mode::Atomic;
            })?;
            Ok(Value::Bool(true))
        }
        Err(e) => Err(redis_exception(it, &e.0)),
    }
}

fn simple_cmd(it: &Interp, this: Option<&Rc<Obj>>, args: Vec<Vec<u8>>) -> R<Reply> {
    let r = state(it, this, |st| match st.conn.as_mut() {
        Some(c) => c.command(&args).map_err(|e| e.0),
        None => Err("Redis server went away".to_string()),
    })?;
    r.map_err(|m| redis_exception(it, &m))
}

fn auth(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>) -> R<Value> {
    let cred = arg(&a, 0);
    let mut args = vec![b"AUTH".to_vec()];
    match &cred {
        Value::Array(arr) => {
            let vals: Vec<Value> = arr.values().collect();
            for v in vals.iter().filter(|v| !v.is_null()) {
                args.push(it.to_str(v)?.to_vec());
            }
        }
        v => args.push(it.to_str(v)?.to_vec()),
    }
    match simple_cmd(it, this, args)? {
        Reply::Error(e) => {
            state(it, this, |st| st.last_error = Some(e.clone()))?;
            Err(redis_exception(it, &e))
        }
        _ => {
            state(it, this, |st| st.auth = cred)?;
            Ok(Value::Bool(true))
        }
    }
}

fn select(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>) -> R<Value> {
    let db = it.to_int(&arg(&a, 0))?;
    match simple_cmd(it, this, vec![b"SELECT".to_vec(), db.to_string().into_bytes()])? {
        Reply::Error(e) => {
            state(it, this, |st| st.last_error = Some(e))?;
            Ok(Value::Bool(false))
        }
        _ => {
            state(it, this, |st| st.db = db)?;
            Ok(Value::Bool(true))
        }
    }
}

fn close(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    state(it, this, |st| st.conn = None)?;
    Ok(Value::Bool(true))
}

// ---- options ------------------------------------------------------------------------

fn set_option(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let opt = it.to_int(&arg(&a, 0))?;
    let v = arg(&a, 1);
    let ok = match opt {
        OPT_PREFIX => {
            let p = if v.is_null() { vec![] } else { it.to_str(&v)?.to_vec() };
            state(it, this, |st| st.prefix = p)?;
            true
        }
        OPT_SERIALIZER => {
            let s = it.to_int(&v)?;
            if matches!(s, SERIALIZER_NONE | SERIALIZER_PHP | SERIALIZER_JSON) {
                state(it, this, |st| st.serializer = s)?;
                true
            } else {
                false
            }
        }
        OPT_READ_TIMEOUT => {
            let t = it.to_float(&v)?;
            state(it, this, |st| {
                st.read_timeout = t;
                if let Some(c) = st.conn.as_mut() {
                    c.read_timeout = (t > 0.0).then(|| Duration::from_secs_f64(t));
                }
            })?;
            true
        }
        OPT_SCAN => {
            let s = it.to_int(&v)?;
            state(it, this, |st| st.scan = s)?;
            true
        }
        OPT_COMPRESSION => it.to_int(&v)? == 0,
        OPT_REPLY_LITERAL | OPT_COMPRESSION_LEVEL | OPT_NULL_MULTIBULK_AS_NULL | OPT_MAX_RETRIES | OPT_BACKOFF_ALGORITHM | OPT_BACKOFF_BASE | OPT_BACKOFF_CAP | OPT_TCP_KEEPALIVE => {
            state(it, this, |st| st.options.push((opt, v)))?;
            true
        }
        _ => false,
    };
    Ok(Value::Bool(ok))
}

fn get_option(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let opt = it.to_int(&arg(&a, 0))?;
    state(it, this, |st| match opt {
        OPT_PREFIX => {
            if st.prefix.is_empty() {
                Value::Null
            } else {
                Value::bytes(st.prefix.clone())
            }
        }
        OPT_SERIALIZER => Value::Int(st.serializer),
        OPT_READ_TIMEOUT => Value::Float(st.read_timeout),
        OPT_SCAN => Value::Int(st.scan),
        OPT_COMPRESSION => Value::Int(0),
        _ => st.options.iter().rev().find(|(k, _)| *k == opt).map(|(_, v)| v.clone()).unwrap_or(Value::Bool(false)),
    })
}

// ---- irregular commands ----------------------------------------------------------------

fn set(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let (key, val, opts) = (arg(&a, 0), arg(&a, 1), arg(&a, 2));
    let mut cmd = state(it, this, |st| -> R<Vec<Vec<u8>>> {
        Ok(vec![b"SET".to_vec(), with_prefix(&st.prefix, bytes(it, &key)?), serialize(it, st.serializer, &val)?])
    })??;
    match &opts {
        Value::Int(_) | Value::Float(_) => {
            cmd.push(b"EX".to_vec());
            cmd.push(it.to_int(&opts)?.to_string().into_bytes());
        }
        Value::Array(o) => {
            for (k, v) in o.iter() {
                match k {
                    Key::Int(_) => cmd.push(it.to_str(&v)?.to_ascii_uppercase()),
                    Key::Str(s) => {
                        cmd.push(s.to_ascii_uppercase());
                        cmd.push(it.to_str(&v)?.to_vec());
                    }
                }
            }
        }
        _ => {}
    }
    exec(it, this, cmd, pending(Out::Ok))
}

fn zadd(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let key = arg(&a, 0);
    let mut cmd = state(it, this, |st| -> R<Vec<Vec<u8>>> { Ok(vec![b"ZADD".to_vec(), with_prefix(&st.prefix, bytes(it, &key)?)]) })??;
    let mut i = 1;
    if let Value::Array(o) = arg(&a, 1) {
        for v in o.values() {
            cmd.push(it.to_str(&v)?.to_ascii_uppercase());
        }
        i = 2;
    }
    let ser = state(it, this, |st| st.serializer)?;
    let mut incr = false;
    while i + 1 < a.len() {
        cmd.push(bytes(it, &arg(&a, i))?);
        cmd.push(serialize(it, ser, &arg(&a, i + 1))?);
        i += 2;
    }
    if cmd.iter().any(|c| c == b"INCR") {
        incr = true;
    }
    exec(it, this, cmd, pending(if incr { Out::Float } else { Out::Int }))
}

/// zRange(key, start, end, withscores|options)
fn zrange(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, rev: bool) -> R<Value> {
    let key = arg(&a, 0);
    let mut cmd = state(it, this, |st| -> R<Vec<Vec<u8>>> {
        Ok(vec![if rev { b"ZREVRANGE".to_vec() } else { b"ZRANGE".to_vec() }, with_prefix(&st.prefix, bytes(it, &key)?), bytes(it, &arg(&a, 1))?, bytes(it, &arg(&a, 2))?])
    })??;
    let ws = match arg(&a, 3) {
        Value::Array(o) => {
            let mut ws = false;
            for (k, v) in o.iter() {
                match k {
                    Key::Str(s) if s.eq_ignore_ascii_case(b"withscores") => ws = v.truthy(),
                    Key::Int(_) if it.to_str(&v)?.eq_ignore_ascii_case(b"withscores") => ws = true,
                    Key::Str(s) if s.eq_ignore_ascii_case(b"rev") => {
                        if v.truthy() {
                            cmd.push(b"REV".to_vec())
                        }
                    }
                    Key::Int(_) => cmd.push(it.to_str(&v)?.to_ascii_uppercase()),
                    _ => {}
                }
            }
            ws
        }
        v => v.truthy(),
    };
    if ws {
        cmd.push(b"WITHSCORES".to_vec());
    }
    exec(it, this, cmd, Pending { out: if ws { Out::Scores } else { Out::Vals }, fields: vec![] })
}

/// zRangeByScore(key, min, max, [withscores, limit => [off, count]])
fn zrange_by_score(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, cmdname: &str) -> R<Value> {
    let key = arg(&a, 0);
    let mut cmd = state(it, this, |st| -> R<Vec<Vec<u8>>> {
        Ok(vec![cmdname.as_bytes().to_vec(), with_prefix(&st.prefix, bytes(it, &key)?), bytes(it, &arg(&a, 1))?, bytes(it, &arg(&a, 2))?])
    })??;
    let mut ws = false;
    if let Value::Array(o) = arg(&a, 3) {
        for (k, v) in o.iter() {
            let Key::Str(s) = k else { continue };
            if s.eq_ignore_ascii_case(b"withscores") && v.truthy() {
                ws = true;
            } else if s.eq_ignore_ascii_case(b"limit") {
                if let Value::Array(l) = v.deref() {
                    let l: Vec<Value> = l.values().collect();
                    if l.len() == 2 {
                        cmd.push(b"LIMIT".to_vec());
                        cmd.push(bytes(it, &l[0])?);
                        cmd.push(bytes(it, &l[1])?);
                    }
                }
            }
        }
    }
    if ws {
        cmd.push(b"WITHSCORES".to_vec());
    }
    exec(it, this, cmd, Pending { out: if ws { Out::Scores } else { Out::Vals }, fields: vec![] })
}

fn hmget(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let key = arg(&a, 0);
    let fields: Vec<Value> = match arg(&a, 1) {
        Value::Array(f) => f.values().collect(),
        _ => vec![],
    };
    let mut cmd = state(it, this, |st| -> R<Vec<Vec<u8>>> { Ok(vec![b"HMGET".to_vec(), with_prefix(&st.prefix, bytes(it, &key)?)]) })??;
    for f in &fields {
        cmd.push(bytes(it, f)?);
    }
    exec(it, this, cmd, Pending { out: Out::Fields, fields })
}

/// mSet / mSetNx / hMSet: an associative array of pairs.
fn pairs(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, cmdname: &str, hash: bool, out: Out) -> R<Value> {
    let (head, map) = if hash { (Some(arg(&a, 0)), arg(&a, 1)) } else { (None, arg(&a, 0)) };
    let mut cmd = vec![cmdname.as_bytes().to_vec()];
    state(it, this, |st| -> R<()> {
        if let Some(k) = &head {
            cmd.push(with_prefix(&st.prefix, bytes(it, k)?));
        }
        if let Value::Array(m) = &map {
            for (k, v) in m.iter() {
                let kb = match k {
                    Key::Int(n) => n.to_string().into_bytes(),
                    Key::Str(s) => s.to_vec(),
                };
                cmd.push(if hash { kb } else { with_prefix(&st.prefix, kb) });
                cmd.push(serialize(it, st.serializer, &v)?);
            }
        }
        Ok(())
    })??;
    exec(it, this, cmd, pending(out))
}

/// blPop(keys..., timeout) / brPop
fn bpop(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, cmdname: &str) -> R<Value> {
    let args = flat(&a);
    let cmd = state(it, this, |st| build(it, st, cmdname, &args, Keys::AllButLast, Ser::None))??;
    // The server blocks up to the timeout: never time the read out first.
    let saved = state(it, this, |st| st.conn.as_mut().map(|c| c.read_timeout.take()))?;
    let r = exec(it, this, cmd, pending(Out::Pop));
    state(it, this, |st| {
        if let (Some(c), Some(t)) = (st.conn.as_mut(), saved) {
            c.read_timeout = t;
        }
    })?;
    r
}

/// eval(script, args, num_keys) / evalSha
fn eval(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, cmdname: &str) -> R<Value> {
    let script = bytes(it, &arg(&a, 0))?;
    let args: Vec<Value> = match arg(&a, 1) {
        Value::Array(x) => x.values().collect(),
        _ => vec![],
    };
    let nkeys = if a.len() > 2 { it.to_int(&arg(&a, 2))?.max(0) as usize } else { 0 };
    let mut cmd = vec![cmdname.as_bytes().to_vec(), script, nkeys.to_string().into_bytes()];
    state(it, this, |st| -> R<()> {
        for (i, v) in args.iter().enumerate() {
            let b = bytes(it, v)?;
            cmd.push(if i < nkeys { with_prefix(&st.prefix, b) } else { b });
        }
        Ok(())
    })??;
    exec(it, this, cmd, pending(Out::Raw))
}

/// Subcommand-style methods: script/client/config/object/info/...
fn sub(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, cmdname: &str, out: Out) -> R<Value> {
    let mut cmd = vec![cmdname.as_bytes().to_vec()];
    for v in flat(&a) {
        cmd.push(bytes(it, &v)?);
    }
    // CLIENT GETNAME / SCRIPT LOAD answer with strings.
    let out = match (cmdname, a.first().map(|v| it.to_string_lossy(v).unwrap_or_default().to_ascii_lowercase())) {
        ("CLIENT", Some(s)) if s == "getname" => Out::Str,
        ("CLIENT", Some(s)) if s == "list" || s == "info" => Out::Str,
        ("SCRIPT", Some(s)) if s == "load" => Out::Str,
        ("CONFIG", Some(s)) if s == "get" => Out::Map,
        _ => out,
    };
    exec(it, this, cmd, pending(out))
}

fn raw_command(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let mut cmd = vec![];
    for v in flat(&a) {
        cmd.push(bytes(it, &v)?);
    }
    exec(it, this, cmd, pending(Out::Raw))
}

/// __phpx_scan(cmd, key, cursor, pattern, count): [new cursor, items or
/// false]. The prelude's scan()/hScan()/sScan()/zScan() keep the
/// by-reference iterator.
fn scan(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>) -> R<Value> {
    let cmdname = String::from_utf8_lossy(&it.to_str(&arg(&a, 0))?).into_owned();
    let keyed = cmdname != "SCAN";
    let key = keyed.then(|| arg(&a, 1));
    let cur = arg(&a, 2);
    let done_result = |c: Value, v: Value| Value::array(PhpArray::from_list([c, v]));
    if matches!(cur, Value::Int(0)) || matches!(&cur, Value::Str(s) if &s[..] == b"0") {
        return Ok(done_result(Value::Int(0), Value::Bool(false)));
    }
    let mut cursor = if cur.is_null() { b"0".to_vec() } else { it.to_str(&cur)?.to_vec() };
    let pattern = Some(arg(&a, 3)).filter(|v| !v.is_null());
    let count = Some(arg(&a, 4)).filter(|v| !v.is_null() && !matches!(v, Value::Int(0)));
    let retry = state(it, this, |st| st.scan & SCAN_RETRY != 0)?;
    loop {
        let mut cmd = vec![cmdname.as_bytes().to_vec()];
        state(it, this, |st| -> R<()> {
            if let Some(k) = &key {
                cmd.push(with_prefix(&st.prefix, bytes(it, k)?));
            }
            cmd.push(cursor.clone());
            if let Some(p) = &pattern {
                cmd.push(b"MATCH".to_vec());
                cmd.push(with_prefix(if keyed { b"" } else { &st.prefix }, bytes(it, p)?));
            }
            if let Some(c) = &count {
                cmd.push(b"COUNT".to_vec());
                cmd.push(bytes(it, c)?);
            }
            Ok(())
        })??;
        let reply = simple_cmd(it, this, cmd)?;
        let Reply::Array(Some(mut parts)) = reply else {
            if let Reply::Error(e) = reply {
                state(it, this, |st| st.last_error = Some(e))?;
            }
            return Ok(done_result(Value::Int(0), Value::Bool(false)));
        };
        let items = parts.pop().unwrap_or(Reply::Array(None));
        cursor = match parts.pop() {
            Some(Reply::Bulk(Some(b))) => b,
            _ => b"0".to_vec(),
        };
        let out = match cmdname.as_str() {
            "HSCAN" => Out::Map,
            "ZSCAN" => Out::Scores,
            "SSCAN" => Out::Vals,
            _ => Out::Strs,
        };
        let v = state(it, this, |st| convert(it, st, items, &pending(out)))??;
        let done = cursor == b"0";
        let c = match String::from_utf8_lossy(&cursor).parse::<i64>() {
            Ok(n) => Value::Int(n),
            Err(_) => Value::bytes(cursor.clone()),
        };
        let empty = matches!(&v, Value::Array(x) if x.is_empty());
        if empty && retry && !done {
            continue;
        }
        if empty && done {
            return Ok(done_result(c, Value::Bool(false)));
        }
        return Ok(done_result(c, v));
    }
}

// ---- transactions ---------------------------------------------------------------------

fn multi(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let pipeline = a.first().is_some_and(|v| matches!(v, Value::Int(2)));
    if pipeline {
        state(it, this, |st| st.mode = Mode::Pipeline(vec![]))?;
    } else {
        let in_pipeline = state(it, this, |st| matches!(st.mode, Mode::Pipeline(_)))?;
        if in_pipeline {
            // MULTI inside a pipeline is queued like any command.
            return exec(it, this, vec![b"MULTI".to_vec()], pending(Out::Ok));
        }
        match simple_cmd(it, this, vec![b"MULTI".to_vec()])? {
            Reply::Error(e) => {
                state(it, this, |st| st.last_error = Some(e))?;
                return Ok(Value::Bool(false));
            }
            _ => state(it, this, |st| st.mode = Mode::Multi(vec![]))?,
        }
    }
    Ok(Value::Object(this.unwrap().clone()))
}

fn pipeline(it: &Interp, this: Option<&Rc<Obj>>, c: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    multi(it, this, c, vec![Value::Int(2)])
}

fn exec_tx(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let mode = state(it, this, |st| std::mem::replace(&mut st.mode, Mode::Atomic))?;
    match mode {
        Mode::Atomic => Ok(Value::Bool(false)),
        Mode::Multi(q) => {
            let reply = simple_cmd(it, this, vec![b"EXEC".to_vec()])?;
            let Reply::Array(Some(items)) = reply else {
                if let Reply::Error(e) = reply {
                    state(it, this, |st| st.last_error = Some(e))?;
                }
                return Ok(Value::Bool(false));
            };
            let mut arr = PhpArray::new();
            for (r, p) in items.into_iter().zip(q.iter()) {
                arr.push(state(it, this, |st| convert(it, st, r, p))??);
            }
            Ok(Value::array(arr))
        }
        Mode::Pipeline(q) => {
            let mut buf = vec![];
            for (cmd, _) in &q {
                conn::encode(&mut buf, cmd);
            }
            let replies = state(it, this, |st| -> Result<Vec<Reply>, String> {
                let c = st.conn.as_mut().ok_or_else(|| "Redis server went away".to_string())?;
                c.write(&buf).map_err(|e| e.0)?;
                let mut out = vec![];
                for _ in 0..q.len() {
                    out.push(c.read().map_err(|e| e.0)?);
                }
                Ok(out)
            })?
            .map_err(|m| redis_exception(it, &m))?;
            let mut arr = PhpArray::new();
            for (r, (_, p)) in replies.into_iter().zip(q.iter()) {
                arr.push(state(it, this, |st| convert(it, st, r, p))??);
            }
            Ok(Value::array(arr))
        }
    }
}

fn discard(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let mode = state(it, this, |st| std::mem::replace(&mut st.mode, Mode::Atomic))?;
    if let Mode::Multi(_) = mode {
        simple_cmd(it, this, vec![b"DISCARD".to_vec()])?;
    }
    Ok(Value::Bool(true))
}

// ---- pub/sub --------------------------------------------------------------------------

fn subscribe(it: &Interp, this: Option<&Rc<Obj>>, a: Vec<Value>, pattern: bool) -> R<Value> {
    let chans: Vec<Value> = match arg(&a, 0) {
        Value::Array(c) => c.values().collect(),
        v => vec![v],
    };
    let cb = arg(&a, 1);
    let f = Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    let callable = it.to_callable(&f, &cb)?;
    let mut cmd = vec![if pattern { b"PSUBSCRIBE".to_vec() } else { b"SUBSCRIBE".to_vec() }];
    for c in &chans {
        cmd.push(bytes(it, c)?);
    }
    state(it, this, |st| st.conn.as_mut().map(|c| c.send(&cmd).map_err(|e| e.0)))?
        .ok_or_else(|| redis_exception(it, "Redis server went away"))?
        .map_err(|m| redis_exception(it, &m))?;
    let me = Value::Object(this.unwrap().clone());
    let mut active = chans.len();
    while active > 0 {
        let r = state(it, this, |st| st.conn.as_mut().map(|c| c.read().map_err(|e| e.0)))?
            .ok_or_else(|| redis_exception(it, "Redis server went away"))?
            .map_err(|m| redis_exception(it, &m))?;
        let Reply::Array(Some(parts)) = r else { continue };
        let strs: Vec<Vec<u8>> = parts
            .into_iter()
            .map(|p| match p {
                Reply::Bulk(Some(b)) => b,
                Reply::Int(n) => n.to_string().into_bytes(),
                _ => vec![],
            })
            .collect();
        match strs.first().map(|s| s.as_slice()) {
            Some(b"message") if strs.len() == 3 => {
                it.call_callable(&callable, vec![me.clone(), Value::bytes(strs[1].clone()), Value::bytes(strs[2].clone())], vec![])?;
            }
            Some(b"pmessage") if strs.len() == 4 => {
                it.call_callable(&callable, vec![me.clone(), Value::bytes(strs[1].clone()), Value::bytes(strs[2].clone()), Value::bytes(strs[3].clone())], vec![])?;
            }
            Some(b"unsubscribe") | Some(b"punsubscribe") => {
                active = strs.get(2).and_then(|n| String::from_utf8_lossy(n).parse().ok()).unwrap_or(0);
            }
            _ => {}
        }
    }
    Ok(Value::Bool(true))
}

// ---- accessors ------------------------------------------------------------------------

fn natives_table() -> Vec<(&'static str, NativeMethod)> {
    vec![
        ("__construct", |it, this, _, _| {
            state(it, this, |_| ())?;
            Ok(Value::Null)
        }),
        ("connect", |it, this, _, a| connect(it, this, a, false)),
        ("open", |it, this, _, a| connect(it, this, a, false)),
        ("pconnect", |it, this, _, a| connect(it, this, a, true)),
        ("popen", |it, this, _, a| connect(it, this, a, true)),
        ("auth", |it, this, _, a| auth(it, this, a)),
        ("select", |it, this, _, a| select(it, this, a)),
        ("close", close),
        ("setoption", set_option),
        ("getoption", get_option),
        ("isconnected", |it, this, _, _| Ok(Value::Bool(state(it, this, |st| st.conn.is_some())?))),
        ("gethost", |it, this, _, _| Ok(Value::str(&state(it, this, |st| st.host.clone())?))),
        ("getport", |it, this, _, _| Ok(Value::Int(state(it, this, |st| st.port)?))),
        ("getdbnum", |it, this, _, _| Ok(Value::Int(state(it, this, |st| st.db)?))),
        ("gettimeout", |it, this, _, _| Ok(Value::Float(state(it, this, |st| st.timeout)?))),
        ("getreadtimeout", |it, this, _, _| Ok(Value::Float(state(it, this, |st| st.read_timeout)?))),
        ("getpersistentid", |it, this, _, _| Ok(state(it, this, |st| st.persistent_id.as_deref().map_or(Value::Null, Value::str))?)),
        ("getauth", |it, this, _, _| state(it, this, |st| st.auth.clone())),
        ("getlasterror", |it, this, _, _| Ok(state(it, this, |st| st.last_error.as_deref().map_or(Value::Null, Value::str))?)),
        ("clearlasterror", |it, this, _, _| {
            state(it, this, |st| st.last_error = None)?;
            Ok(Value::Bool(true))
        }),
        ("getmode", |it, this, _, _| {
            Ok(Value::Int(state(it, this, |st| match st.mode {
                Mode::Atomic => 0,
                Mode::Multi(_) => 1,
                Mode::Pipeline(_) => 2,
            })?))
        }),
        ("multi", multi),
        ("pipeline", pipeline),
        ("exec", exec_tx),
        ("discard", discard),
        ("rawcommand", raw_command),
        ("set", set),
        ("zadd", zadd),
        ("zrange", |it, this, _, a| zrange(it, this, a, false)),
        ("zrevrange", |it, this, _, a| zrange(it, this, a, true)),
        ("zrangebyscore", |it, this, _, a| zrange_by_score(it, this, a, "ZRANGEBYSCORE")),
        ("zrevrangebyscore", |it, this, _, a| zrange_by_score(it, this, a, "ZREVRANGEBYSCORE")),
        ("hmget", hmget),
        ("mset", |it, this, _, a| pairs(it, this, a, "MSET", false, Out::Ok)),
        ("msetnx", |it, this, _, a| pairs(it, this, a, "MSETNX", false, Out::Bool)),
        ("hmset", |it, this, _, a| pairs(it, this, a, "HMSET", true, Out::Ok)),
        ("blpop", |it, this, _, a| bpop(it, this, a, "BLPOP")),
        ("brpop", |it, this, _, a| bpop(it, this, a, "BRPOP")),
        ("bzpopmin", |it, this, _, a| bpop(it, this, a, "BZPOPMIN")),
        ("bzpopmax", |it, this, _, a| bpop(it, this, a, "BZPOPMAX")),
        ("eval", |it, this, _, a| eval(it, this, a, "EVAL")),
        ("evalsha", |it, this, _, a| eval(it, this, a, "EVALSHA")),
        ("eval_ro", |it, this, _, a| eval(it, this, a, "EVAL_RO")),
        ("evalsha_ro", |it, this, _, a| eval(it, this, a, "EVALSHA_RO")),
        ("script", |it, this, _, a| sub(it, this, a, "SCRIPT", Out::Raw)),
        ("client", |it, this, _, a| sub(it, this, a, "CLIENT", Out::Raw)),
        ("config", |it, this, _, a| sub(it, this, a, "CONFIG", Out::Raw)),
        ("object", |it, this, _, a| sub(it, this, a, "OBJECT", Out::Raw)),
        ("slowlog", |it, this, _, a| sub(it, this, a, "SLOWLOG", Out::Raw)),
        ("info", |it, this, _, a| sub(it, this, a, "INFO", Out::Info)),
        ("__phpx_scan", |it, this, _, a| scan(it, this, a)),
        ("subscribe", |it, this, _, a| subscribe(it, this, a, false)),
        ("psubscribe", |it, this, _, a| subscribe(it, this, a, true)),
        // Regular commands: (Redis command, keys, values, reply).
        ("ping", spec!("PING", None, None, Ping)),
        ("echo", spec!("ECHO", None, None, Str)),
        ("get", spec!("GET", First, None, Val)),
        ("getset", spec!("GETSET", First, Last, Val)),
        ("getdel", spec!("GETDEL", First, None, Val)),
        ("getex", spec!("GETEX", First, None, Val)),
        ("setex", spec!("SETEX", First, Third, Ok)),
        ("psetex", spec!("PSETEX", First, Third, Ok)),
        ("setnx", spec!("SETNX", First, Last, Bool)),
        ("mget", spec!("MGET", All, None, Vals)),
        ("getmultiple", spec!("MGET", All, None, Vals)),
        ("exists", spec!("EXISTS", All, None, Int)),
        ("del", spec!("DEL", All, None, Int)),
        ("delete", spec!("DEL", All, None, Int)),
        ("unlink", spec!("UNLINK", All, None, Int)),
        ("touch", spec!("TOUCH", All, None, Int)),
        ("incr", spec!("INCR", First, None, Int)),
        ("incrby", spec!("INCRBY", First, None, Int)),
        ("incrbyfloat", spec!("INCRBYFLOAT", First, None, Float)),
        ("decr", spec!("DECR", First, None, Int)),
        ("decrby", spec!("DECRBY", First, None, Int)),
        ("expire", spec!("EXPIRE", First, None, Bool)),
        ("pexpire", spec!("PEXPIRE", First, None, Bool)),
        ("expireat", spec!("EXPIREAT", First, None, Bool)),
        ("pexpireat", spec!("PEXPIREAT", First, None, Bool)),
        ("settimeout", spec!("EXPIRE", First, None, Bool)),
        ("ttl", spec!("TTL", First, None, Int)),
        ("pttl", spec!("PTTL", First, None, Int)),
        ("persist", spec!("PERSIST", First, None, Bool)),
        ("type", spec!("TYPE", First, None, Type)),
        ("append", spec!("APPEND", First, Last, Int)),
        ("strlen", spec!("STRLEN", First, None, Int)),
        ("getrange", spec!("GETRANGE", First, None, Str)),
        ("setrange", spec!("SETRANGE", First, None, Int)),
        ("getbit", spec!("GETBIT", First, None, Int)),
        ("setbit", spec!("SETBIT", First, None, Int)),
        ("bitcount", spec!("BITCOUNT", First, None, Int)),
        ("keys", spec!("KEYS", None, None, Strs)),
        ("dbsize", spec!("DBSIZE", None, None, Int)),
        ("randomkey", spec!("RANDOMKEY", None, None, Str)),
        ("rename", spec!("RENAME", FirstTwo, None, Ok)),
        ("renamekey", spec!("RENAME", FirstTwo, None, Ok)),
        ("renamenx", spec!("RENAMENX", FirstTwo, None, Bool)),
        ("move", spec!("MOVE", First, None, Bool)),
        ("dump", spec!("DUMP", First, None, Str)),
        ("flushdb", spec!("FLUSHDB", None, None, Ok)),
        ("flushall", spec!("FLUSHALL", None, None, Ok)),
        ("save", spec!("SAVE", None, None, Ok)),
        ("bgsave", spec!("BGSAVE", None, None, Ok)),
        ("lastsave", spec!("LASTSAVE", None, None, Int)),
        ("time", spec!("TIME", None, None, Time)),
        ("publish", spec!("PUBLISH", None, None, Int)),
        ("hset", spec!("HSET", First, Last, Int)),
        ("hsetnx", spec!("HSETNX", First, Last, Bool)),
        ("hget", spec!("HGET", First, None, Val)),
        ("hgetall", spec!("HGETALL", First, None, Map)),
        ("hkeys", spec!("HKEYS", First, None, Strs)),
        ("hvals", spec!("HVALS", First, None, Vals)),
        ("hlen", spec!("HLEN", First, None, Int)),
        ("hexists", spec!("HEXISTS", First, None, Bool)),
        ("hdel", spec!("HDEL", First, None, Int)),
        ("hincrby", spec!("HINCRBY", First, None, Int)),
        ("hincrbyfloat", spec!("HINCRBYFLOAT", First, None, Float)),
        ("hstrlen", spec!("HSTRLEN", First, None, Int)),
        ("lpush", spec!("LPUSH", First, Rest, Int)),
        ("rpush", spec!("RPUSH", First, Rest, Int)),
        ("lpushx", spec!("LPUSHX", First, Rest, Int)),
        ("rpushx", spec!("RPUSHX", First, Rest, Int)),
        ("lpop", spec!("LPOP", First, None, Val)),
        ("rpop", spec!("RPOP", First, None, Val)),
        ("lrange", spec!("LRANGE", First, None, Vals)),
        ("lgetrange", spec!("LRANGE", First, None, Vals)),
        ("llen", spec!("LLEN", First, None, Int)),
        ("lsize", spec!("LLEN", First, None, Int)),
        ("lindex", spec!("LINDEX", First, None, Val)),
        ("lget", spec!("LINDEX", First, None, Val)),
        ("lset", spec!("LSET", First, Last, Ok)),
        ("lrem", |it, this, _, a| {
            // lRem(key, value, count) -> LREM key count value
            let (key, value) = (arg(&a, 0), arg(&a, 1));
            let count = if a.len() > 2 { it.to_int(&arg(&a, 2))? } else { 0 };
            let cmd = state(it, this, |st| -> R<Vec<Vec<u8>>> {
                Ok(vec![b"LREM".to_vec(), with_prefix(&st.prefix, bytes(it, &key)?), count.to_string().into_bytes(), serialize(it, st.serializer, &value)?])
            })??;
            exec(it, this, cmd, pending(Out::Int))
        }),
        ("ltrim", spec!("LTRIM", First, None, Ok)),
        ("rpoplpush", spec!("RPOPLPUSH", FirstTwo, None, Val)),
        ("lmove", spec!("LMOVE", FirstTwo, None, Val)),
        ("sadd", spec!("SADD", First, Rest, Int)),
        ("saddarray", spec!("SADD", First, Rest, Int)),
        ("srem", spec!("SREM", First, Rest, Int)),
        ("scard", spec!("SCARD", First, None, Int)),
        ("ssize", spec!("SCARD", First, None, Int)),
        ("sismember", spec!("SISMEMBER", First, Last, Bool)),
        ("scontains", spec!("SISMEMBER", First, Last, Bool)),
        ("smembers", spec!("SMEMBERS", First, None, Vals)),
        ("sgetmembers", spec!("SMEMBERS", First, None, Vals)),
        ("spop", spec!("SPOP", First, None, Val)),
        ("srandmember", spec!("SRANDMEMBER", First, None, Val)),
        ("smove", spec!("SMOVE", FirstTwo, Last, Bool)),
        ("sinter", spec!("SINTER", All, None, Vals)),
        ("sunion", spec!("SUNION", All, None, Vals)),
        ("sdiff", spec!("SDIFF", All, None, Vals)),
        ("sinterstore", spec!("SINTERSTORE", All, None, Int)),
        ("sunionstore", spec!("SUNIONSTORE", All, None, Int)),
        ("sdiffstore", spec!("SDIFFSTORE", All, None, Int)),
        ("zscore", spec!("ZSCORE", First, Last, Float)),
        ("zcard", spec!("ZCARD", First, None, Int)),
        ("zsize", spec!("ZCARD", First, None, Int)),
        ("zcount", spec!("ZCOUNT", First, None, Int)),
        ("zincrby", spec!("ZINCRBY", First, Last, Float)),
        ("zrank", spec!("ZRANK", First, Last, Int)),
        ("zrevrank", spec!("ZREVRANK", First, Last, Int)),
        ("zrem", spec!("ZREM", First, Rest, Int)),
        ("zdelete", spec!("ZREM", First, Rest, Int)),
        ("zremrangebyscore", spec!("ZREMRANGEBYSCORE", First, None, Int)),
        ("zremrangebyrank", spec!("ZREMRANGEBYRANK", First, None, Int)),
        ("zpopmin", spec!("ZPOPMIN", First, None, Scores)),
        ("zpopmax", spec!("ZPOPMAX", First, None, Scores)),
        ("pfadd", spec!("PFADD", First, None, Bool)),
        ("pfcount", spec!("PFCOUNT", All, None, Int)),
        ("pfmerge", spec!("PFMERGE", All, None, Ok)),
        ("watch", spec!("WATCH", All, None, Ok)),
        ("unwatch", spec!("UNWATCH", None, None, Ok)),
        ("xadd", spec!("XADD", First, None, Str)),
        ("xlen", spec!("XLEN", First, None, Int)),
        ("xdel", spec!("XDEL", First, None, Int)),
        ("geoadd", spec!("GEOADD", First, None, Int)),
        ("swapdb", spec!("SWAPDB", None, None, Ok)),
        ("wait", spec!("WAIT", None, None, Int)),
        ("role", spec!("ROLE", None, None, Raw)),
        ("lastsave", spec!("LASTSAVE", None, None, Int)),
    ]
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    natives_table().into_iter().map(|(n, f)| ("redis", n, f)).collect()
}
