//! Stream filters: stream_filter_append/prepend/remove, the built-in
//! filters (string.*, convert.*, zlib.*, dechunk, consumed) and user
//! filters (php_user_filter subclasses with bucket brigades).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::arg;
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Obj, PhpArray, Resource, Value};

pub enum Kind {
    Rot13,
    Upper,
    Lower,
    B64Enc(Vec<u8>),
    B64Dec(Vec<u8>),
    QpEnc,
    QpDec(Vec<u8>),
    /// Raw deflate; with gzip framing: (crc, header written, level).
    Deflate(flate2::Compress, Option<(flate2::Crc, bool, u32)>),
    /// Raw inflate; with gzip framing: the header bytes still to skip.
    Inflate(flate2::Decompress, Option<Vec<u8>>),
    /// dechunk: (bytes left in the current chunk, pending header text, done)
    Dechunk(usize, Vec<u8>, bool),
    Iconv(String, String, Vec<u8>),
    Consumed(i64),
    User(Rc<Obj>),
}

pub struct Filter {
    pub kind: Kind,
}

pub type FilterRef = Rc<RefCell<Filter>>;

thread_local! {
    /// stream_filter_register(): name pattern -> class.
    static USER: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
}

/// A new request: user filters registered by the last one are gone.
pub fn reset() {
    USER.with(|u| u.borrow_mut().clear());
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 3 * 4 + 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        out.push(B64[(b[0] >> 2) as usize]);
        out.push(B64[(((b[0] & 3) << 4) | (b[1] >> 4)) as usize]);
        out.push(if c.len() > 1 { B64[(((b[1] & 15) << 2) | (b[2] >> 6)) as usize] } else { b'=' });
        out.push(if c.len() > 2 { B64[(b[2] & 63) as usize] } else { b'=' });
    }
    out
}

fn iconv(from: &str, to: &str, data: &[u8]) -> Option<Vec<u8>> {
    crate::iconv_conv::convert(from, to, data).ok()
}

/// Run `data` through one filter.
fn run(it: &Interp, f: &mut Filter, data: Vec<u8>, closing: bool) -> R<Vec<u8>> {
    Ok(match &mut f.kind {
        Kind::Rot13 => data
            .into_iter()
            .map(|c| match c {
                b'a'..=b'z' => (c - b'a' + 13) % 26 + b'a',
                b'A'..=b'Z' => (c - b'A' + 13) % 26 + b'A',
                c => c,
            })
            .collect(),
        Kind::Upper => data.to_ascii_uppercase(),
        Kind::Lower => data.to_ascii_lowercase(),
        Kind::B64Enc(rem) => {
            rem.extend_from_slice(&data);
            let take = if closing { rem.len() } else { rem.len() / 3 * 3 };
            let chunk: Vec<u8> = rem.drain(..take).collect();
            b64_encode(&chunk)
        }
        Kind::B64Dec(rem) => {
            rem.extend(data.into_iter().filter(|c| !c.is_ascii_whitespace()));
            let take = if closing { rem.len() } else { rem.len() / 4 * 4 };
            let chunk: Vec<u8> = rem.drain(..take).collect();
            crate::strings::base64_decode(&chunk).unwrap_or_default()
        }
        Kind::QpEnc => {
            let mut a = [Value::bytes(data)];
            let mut fr = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
            it.to_str(&(it.builtins["quoted_printable_encode"])(it, &mut fr, &mut a)?)?.to_vec()
        }
        Kind::QpDec(rem) => {
            rem.extend_from_slice(&data);
            // Keep a trailing partial escape for the next chunk.
            let keep = if closing {
                0
            } else {
                match rem.iter().rposition(|&c| c == b'=') {
                    Some(p) if rem.len() - p < 3 => rem.len() - p,
                    _ => 0,
                }
            };
            let n = rem.len() - keep;
            let chunk: Vec<u8> = rem.drain(..n).collect();
            let mut a = [Value::bytes(chunk)];
            let mut fr = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
            it.to_str(&(it.builtins["quoted_printable_decode"])(it, &mut fr, &mut a)?)?.to_vec()
        }
        Kind::Deflate(c, gz) => {
            let mut out = vec![];
            if let Some((crc, header, level)) = gz {
                crc.update(&data);
                if !*header {
                    let xfl = if *level == 9 { 2 } else if *level < 2 { 4 } else { 0 };
                    out.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, xfl, 3]);
                    *header = true;
                }
            }
            let flush = if closing { flate2::FlushCompress::Finish } else { flate2::FlushCompress::None };
            let mut input = &data[..];
            loop {
                out.reserve(64 * 1024);
                let (bi, bo) = (c.total_in(), c.total_out());
                let Ok(st) = c.compress_vec(input, &mut out, flush) else { break };
                input = &input[(c.total_in() - bi) as usize..];
                let produced = c.total_out() - bo;
                if st == flate2::Status::StreamEnd || (!closing && input.is_empty()) || (produced == 0 && input.is_empty() && st == flate2::Status::BufError) {
                    break;
                }
            }
            if let (true, Some((crc, _, _))) = (closing, gz) {
                out.extend_from_slice(&crc.sum().to_le_bytes());
                out.extend_from_slice(&crc.amount().to_le_bytes());
            }
            out
        }
        Kind::Inflate(d, gz) => {
            let mut out = vec![];
            let mut data = data;
            if let Some(pending) = gz {
                pending.extend_from_slice(&data);
                match gzip_header_len(pending) {
                    Some(n) => {
                        data = pending.split_off(n);
                        *gz = None;
                    }
                    None => return Ok(vec![]),
                }
            }
            let mut input = &data[..];
            loop {
                out.reserve(64 * 1024);
                let (bi, bo) = (d.total_in(), d.total_out());
                let Ok(st) = d.decompress_vec(input, &mut out, flate2::FlushDecompress::None) else { break };
                input = &input[(d.total_in() - bi) as usize..];
                let produced = d.total_out() - bo;
                if st == flate2::Status::StreamEnd || (input.is_empty() && produced == 0) || (input.is_empty() && out.len() < out.capacity()) {
                    break;
                }
            }
            out
        }
        Kind::Dechunk(left, head, done) => {
            let mut out = vec![];
            let mut i = 0;
            while i < data.len() && !*done {
                if *left > 0 {
                    let n = (*left).min(data.len() - i);
                    out.extend_from_slice(&data[i..i + n]);
                    *left -= n;
                    i += n;
                    continue;
                }
                head.push(data[i]);
                i += 1;
                if head.ends_with(b"\n") {
                    let line = String::from_utf8_lossy(head).trim().to_string();
                    head.clear();
                    if line.is_empty() {
                        continue;
                    }
                    let size = usize::from_str_radix(line.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
                    if size == 0 {
                        *done = true;
                    }
                    *left = size;
                }
            }
            out
        }
        Kind::Iconv(from, to, rem) => {
            rem.extend_from_slice(&data);
            match iconv(from, to, rem) {
                Some(out) => {
                    rem.clear();
                    out
                }
                None if closing => {
                    rem.clear();
                    vec![]
                }
                None => vec![],
            }
        }
        Kind::Consumed(n) => {
            *n += data.len() as i64;
            data
        }
        Kind::User(o) => {
            let o = o.clone();
            return user_run(it, &o, data, closing);
        }
    })
}

/// Length of a complete gzip header at the start of `b`.
pub fn gzip_header_len(b: &[u8]) -> Option<usize> {
    if b.len() < 10 {
        return None;
    }
    let flags = b[3];
    let mut i = 10;
    if flags & 4 != 0 {
        let x = u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize;
        i += 2 + x;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            i += b.get(i..)?.iter().position(|&c| c == 0)? + 1;
        }
    }
    if flags & 2 != 0 {
        i += 2;
    }
    (i <= b.len()).then_some(i)
}

fn brigade(buckets: Vec<Value>) -> Value {
    Resource::new("userfilter.bucket brigade", Box::new(buckets))
}

fn bucket(it: &Interp, data: Vec<u8>) -> R<Value> {
    let cls = it.lookup_class("StreamBucket")?;
    let o = it.new_instance(&cls)?;
    let len = data.len() as i64;
    o.set_prop("bucket", Resource::new("userfilter.bucket", Box::new(())));
    o.set_prop("data", Value::bytes(data));
    o.set_prop("datalen", Value::Int(len));
    Ok(Value::Object(o))
}

fn with_brigade<T>(v: &Value, g: impl FnOnce(&mut Vec<Value>) -> T) -> Option<T> {
    let Value::Resource(r) = v.deref() else { return None };
    let mut d = r.data.borrow_mut();
    d.as_mut().and_then(|b| b.downcast_mut::<Vec<Value>>()).map(g)
}

fn user_run(it: &Interp, o: &Rc<Obj>, data: Vec<u8>, closing: bool) -> R<Vec<u8>> {
    let input = brigade(if data.is_empty() { vec![] } else { vec![bucket(it, data)?] });
    let output = brigade(vec![]);
    let consumed = crate::interp::slot(Value::Int(0));
    let st = it.call_method(o, "filter", vec![input, output.clone(), Value::Ref(consumed), Value::Bool(closing)])?;
    if it.to_int(&st)? != 2 {
        return Ok(vec![]);
    }
    let buckets = with_brigade(&output, std::mem::take).unwrap_or_default();
    let mut out = vec![];
    for b in buckets {
        if let Value::Object(bo) = b {
            if let Some(d) = bo.get_prop("data") {
                out.extend_from_slice(&it.to_str(&d)?);
            }
        }
    }
    Ok(out)
}

/// Run `data` through a chain of filters.
pub fn apply(it: &Interp, chain: &[FilterRef], mut data: Vec<u8>, closing: bool) -> Vec<u8> {
    for f in chain {
        match run(it, &mut f.borrow_mut(), data, closing) {
            Ok(d) => data = d,
            Err(e) => {
                crate::errors::defer(e);
                return vec![];
            }
        }
    }
    data
}

/// onClose() of user filters.
pub fn closed(it: &Interp, f: &FilterRef) {
    if let Kind::User(o) = &f.borrow().kind {
        if let Err(e) = it.call_method(o, "onClose", vec![]) {
            crate::errors::defer(e);
        }
    }
}

/// Make a filter by name (None: unknown).
pub fn create(it: &Interp, name: &str, params: &Value, stream: &Value) -> R<Option<Filter>> {
    let lower = name.to_ascii_lowercase();
    let param_int = |k: &str| -> Option<i64> {
        match params {
            Value::Array(a) => a.get(&crate::value::Key::from_bytes(k.as_bytes())).and_then(|v| it.to_int(&v).ok()),
            Value::Int(n) if k == "level" => Some(*n),
            _ => None,
        }
    };
    let kind = match lower.as_str() {
        "string.rot13" => Kind::Rot13,
        "string.toupper" => Kind::Upper,
        "string.tolower" => Kind::Lower,
        "convert.base64-encode" => Kind::B64Enc(vec![]),
        "convert.base64-decode" => Kind::B64Dec(vec![]),
        "convert.quoted-printable-encode" => Kind::QpEnc,
        "convert.quoted-printable-decode" => Kind::QpDec(vec![]),
        "zlib.deflate" => {
            let level = param_int("level").filter(|l| (0..=9).contains(l)).unwrap_or(6) as u32;
            let window = param_int("window").unwrap_or(-15);
            let c = flate2::Compress::new(flate2::Compression::new(level), (8..=15).contains(&window));
            Kind::Deflate(c, (window > 15).then(|| (flate2::Crc::new(), false, level)))
        }
        "zlib.inflate" => {
            let window = param_int("window").unwrap_or(-15);
            let d = flate2::Decompress::new((8..=15).contains(&window));
            Kind::Inflate(d, (window > 15).then(Vec::new))
        }
        "dechunk" => Kind::Dechunk(0, vec![], false),
        "consumed" => Kind::Consumed(0),
        n if n.starts_with("convert.iconv.") => {
            let spec = &name["convert.iconv.".len()..];
            let (from, to) = spec.split_once('/').or_else(|| spec.split_once('.')).unwrap_or((spec, "UTF-8"));
            Kind::Iconv(from.to_string(), to.to_string(), vec![])
        }
        _ => {
            let class = USER.with(|u| {
                u.borrow().iter().find(|(pat, _)| pat.eq_ignore_ascii_case(name) || (pat.ends_with(".*") && lower.starts_with(&pat[..pat.len() - 1].to_ascii_lowercase()))).map(|(_, c)| c.clone())
            });
            let Some(class) = class else { return Ok(None) };
            let cls = it.lookup_class(&class)?;
            let o = it.new_instance(&cls)?;
            o.set_prop("filtername", Value::str(name));
            o.set_prop("params", params.clone());
            o.set_prop("stream", stream.clone());
            if !it.call_method(&o, "onCreate", vec![])?.truthy() {
                return Ok(None);
            }
            Kind::User(o)
        }
    };
    Ok(Some(Filter { kind }))
}

/// A stream filter resource: the filter and its stream.
pub struct Handle {
    pub filter: FilterRef,
    pub stream: Value,
}

fn attach(it: &Interp, a: &[Value], prepend: bool, func: &str) -> R<Value> {
    let stream = arg(a, 0);
    let name = it.to_string_lossy(&arg(a, 1))?;
    let mode = match a.get(2) {
        Some(v) if !v.is_null() => it.to_int(v)?,
        _ => 0,
    };
    let params = a.get(3).cloned().unwrap_or_default();
    let (readable, writable) = crate::streams::with_stream(it, &stream, func, |st| st.rw())?;
    let mode = if mode == 0 { (readable as i64) | ((writable as i64) << 1) } else { mode };
    let mut handle = Value::Bool(false);
    for (bit, read) in [(1, true), (2, false)] {
        if mode & bit == 0 {
            continue;
        }
        let Some(f) = create(it, &name, &params, &stream)? else {
            it.warn(&format!("{func}(): Unable to create or locate filter \"{name}\""));
            return Ok(Value::Bool(false));
        };
        let f = Rc::new(RefCell::new(f));
        crate::streams::with_stream(it, &stream, func, |st| st.add_filter(f.clone(), read, prepend))?;
        handle = Resource::new("stream filter", Box::new(Handle { filter: f, stream: stream.clone() }));
    }
    Ok(handle)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("stream_filter_append", |it, _, a| attach(it, a, false, "stream_filter_append"));
    t.insert("stream_filter_prepend", |it, _, a| attach(it, a, true, "stream_filter_prepend"));
    t.insert("stream_filter_remove", |it, _, a| {
        let Value::Resource(r) = arg(a, 0) else {
            return Err(it.error("TypeError", "stream_filter_remove(): Argument #1 ($stream_filter) must be of type resource"));
        };
        let handle = r.data.borrow_mut().take();
        let Some(h) = handle.and_then(|b| b.downcast::<Handle>().ok()) else {
            it.warn("stream_filter_remove(): Invalid resource given, not a stream filter");
            return Ok(Value::Bool(false));
        };
        crate::streams::with_stream(it, &h.stream, "stream_filter_remove", |st| st.remove_filter(&h.filter, it))?;
        closed(it, &h.filter);
        r.close();
        Ok(Value::Bool(true))
    });
    t.insert("stream_filter_register", |it, _, a| {
        let name = it.to_string_lossy(&arg(a, 0))?;
        let class = it.to_string_lossy(&arg(a, 1))?;
        if name.is_empty() {
            return Err(it.error("ValueError", "stream_filter_register(): Argument #1 ($filter_name) must be a non-empty string"));
        }
        if class.is_empty() {
            return Err(it.error("ValueError", "stream_filter_register(): Argument #2 ($class) must be a non-empty string"));
        }
        let exists = USER.with(|u| u.borrow().iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)));
        if exists {
            return Ok(Value::Bool(false));
        }
        USER.with(|u| u.borrow_mut().push((name, class)));
        Ok(Value::Bool(true))
    });
    t.insert("stream_get_filters", |_, _, _| {
        let mut out: Vec<String> = ["zlib.*", "convert.iconv.*", "string.rot13", "string.toupper", "string.tolower", "convert.*", "consumed", "dechunk"].iter().map(|s| s.to_string()).collect();
        USER.with(|u| out.extend(u.borrow().iter().map(|(n, _)| n.clone())));
        Ok(Value::array(PhpArray::from_list(out.iter().map(|s| Value::str(s)).collect::<Vec<_>>())))
    });
    t.insert("stream_bucket_make_writeable", |_, _, a| Ok(with_brigade(&arg(a, 0), |b| if b.is_empty() { Value::Null } else { b.remove(0) }).unwrap_or(Value::Null)));
    t.insert("stream_bucket_append", |_, _, a| {
        let b = arg(a, 1);
        with_brigade(&arg(a, 0), |v| v.push(b));
        Ok(Value::Null)
    });
    t.insert("stream_bucket_prepend", |_, _, a| {
        let b = arg(a, 1);
        with_brigade(&arg(a, 0), |v| v.insert(0, b));
        Ok(Value::Null)
    });
    t.insert("stream_bucket_new", |it, _, a| {
        let data = it.to_str(&arg(a, 1))?.to_vec();
        bucket(it, data)
    });
}
