//! The rest of ext/zlib: gzopen() streams (and compress.zlib://), the
//! gz* stream functions, incremental deflate_init/deflate_add and
//! inflate_init/inflate_add, gzfile/readgzfile and ob_gzhandler.

use std::collections::HashMap;
use std::io::Read;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Internal, Key, NativeData, Obj, PhpArray, Value};

fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

/// Decompressed contents of a gzip (or plain) file.
fn read_gz(path: &str) -> std::io::Result<Vec<u8>> {
    let raw = std::fs::read(path)?;
    if raw.starts_with(&[0x1f, 0x8b]) {
        let mut out = vec![];
        flate2::read::MultiGzDecoder::new(&raw[..]).read_to_end(&mut out)?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

/// gzopen()/compress.zlib://: Ok(stream) or Err(warning).
pub fn open(it: &Interp, path: &str, mode: &str, func: &str) -> Result<Value, String> {
    let path = path.strip_prefix("compress.zlib://").unwrap_or(path);
    let level = mode.chars().find(|c| c.is_ascii_digit()).and_then(|c| c.to_digit(10)).map_or(-1, |d| d as i64);
    if mode.starts_with('r') {
        return match read_gz(path) {
            Ok(data) => Ok(crate::streams::memory_stream(data, mode, path, "ZLIB")),
            Err(e) => Err(format!("{func}({path}): Failed to open stream: {}", crate::streams::io_msg(&e))),
        };
    }
    let plain: String = mode.chars().filter(|c| !c.is_ascii_digit() && !matches!(c, 'f' | 'h' | 'R' | 'F')).collect();
    let res = crate::streams::open(it, path, &plain).map_err(|m| m.replacen("fopen(", &format!("{func}("), 1))?;
    let mut params = PhpArray::new();
    params.insert(Key::from_bytes(b"window"), Value::Int(31));
    if level >= 0 {
        params.insert(Key::from_bytes(b"level"), Value::Int(level));
    }
    if let Ok(Some(f)) = crate::filters::create(it, "zlib.deflate", &Value::array(params), &res) {
        let f = std::rc::Rc::new(std::cell::RefCell::new(f));
        let _ = crate::streams::with_stream(it, &res, func, |st| st.add_filter(f, false, false));
    }
    Ok(res)
}

// ---- incremental contexts -------------------------------------------------------------

struct DeflateState {
    c: flate2::Compress,
    gzip: Option<(flate2::Crc, bool, u32)>,
    done: bool,
}

struct InflateState {
    d: flate2::Decompress,
    gzip_pending: Option<Vec<u8>>,
    status: i64,
    read: u64,
}

macro_rules! native_data {
    ($t:ty) => {
        impl NativeData for $t {
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
    };
}
native_data!(DeflateState);
native_data!(InflateState);

fn opt_int(it: &Interp, opts: &Value, k: &str) -> R<Option<i64>> {
    match opts {
        Value::Array(a) => a.get(&Key::from_bytes(k.as_bytes())).map(|v| it.to_int(&v)).transpose(),
        _ => Ok(None),
    }
}

fn encoding_check(it: &Interp, func: &str, enc: i64) -> R<()> {
    if matches!(enc, -15 | 15 | 31) {
        Ok(())
    } else {
        Err(it.error("ValueError", &format!("{func}(): Argument #1 ($encoding) must be one of ZLIB_ENCODING_RAW, ZLIB_ENCODING_GZIP, or ZLIB_ENCODING_DEFLATE")))
    }
}

fn deflate_init(it: &Interp, a: &[Value]) -> R<Value> {
    let enc = int(it, a, 0)?;
    encoding_check(it, "deflate_init", enc)?;
    let opts = arg(a, 1);
    let level = opt_int(it, &opts, "level")?.unwrap_or(-1);
    if !(-1..=9).contains(&level) {
        return Err(it.error("ValueError", "deflate_init(): \"level\" option must be between -1 and 9"));
    }
    let level = if level < 0 { 6 } else { level as u32 };
    let c = flate2::Compress::new(flate2::Compression::new(level), enc == 15);
    let cls = it.lookup_class("DeflateContext")?;
    let o = Obj::new(cls, Internal::Data(Box::new(DeflateState { c, gzip: (enc == 31).then(|| (flate2::Crc::new(), false, level)), done: false })));
    Ok(Value::Object(o))
}

fn deflate_add(it: &Interp, a: &[Value]) -> R<Value> {
    let data = s(it, a, 1)?.to_vec();
    let flush = if has(a, 2) { int(it, a, 2)? } else { 2 };
    let Value::Object(o) = arg(a, 0) else {
        return Err(it.error("TypeError", "deflate_add(): Argument #1 ($context) must be of type DeflateContext"));
    };
    let mut guard = o.internal_mut();
    let Some(Internal::Data(d)) = guard.as_deref_mut() else { return Ok(Value::Bool(false)) };
    let Some(st) = d.as_any_mut().downcast_mut::<DeflateState>() else { return Ok(Value::Bool(false)) };
    let mode = match flush {
        0 => flate2::FlushCompress::None,
        1 => flate2::FlushCompress::Partial,
        2 => flate2::FlushCompress::Sync,
        3 => flate2::FlushCompress::Full,
        4 => flate2::FlushCompress::Finish,
        5 => flate2::FlushCompress::Partial,
        _ => return Err(it.error("ValueError", "deflate_add(): Argument #3 ($flush_mode) must be one of ZLIB_NO_FLUSH, ZLIB_PARTIAL_FLUSH, ZLIB_SYNC_FLUSH, ZLIB_FULL_FLUSH, ZLIB_BLOCK, or ZLIB_FINISH")),
    };
    let mut out = vec![];
    if st.done {
        // A finished context starts a new stream.
        st.c.reset();
        if let Some(g) = &mut st.gzip {
            *g = (flate2::Crc::new(), false, g.2);
        }
        st.done = false;
    }
    if let Some((crc, header, level)) = &mut st.gzip {
        crc.update(&data);
        if !*header {
            let xfl = if *level == 9 { 2 } else if *level < 2 { 4 } else { 0 };
            out.extend_from_slice(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, xfl, 3]);
            *header = true;
        }
    }
    let mut input = &data[..];
    loop {
        out.reserve(64 * 1024);
        let (bi, bo) = (st.c.total_in(), st.c.total_out());
        let Ok(r) = st.c.compress_vec(input, &mut out, mode) else { break };
        input = &input[(st.c.total_in() - bi) as usize..];
        let produced = st.c.total_out() - bo;
        if r == flate2::Status::StreamEnd {
            break;
        }
        if input.is_empty() && out.len() < out.capacity() && (flush != 4 || produced == 0) {
            break;
        }
    }
    if flush == 4 {
        if let Some((crc, _, _)) = &st.gzip {
            out.extend_from_slice(&crc.sum().to_le_bytes());
            out.extend_from_slice(&crc.amount().to_le_bytes());
        }
        st.done = true;
    }
    Ok(Value::bytes(out))
}

fn inflate_init(it: &Interp, a: &[Value]) -> R<Value> {
    let enc = int(it, a, 0)?;
    encoding_check(it, "inflate_init", enc)?;
    let d = flate2::Decompress::new(enc == 15);
    let cls = it.lookup_class("InflateContext")?;
    let o = Obj::new(cls, Internal::Data(Box::new(InflateState { d, gzip_pending: (enc == 31).then(Vec::new), status: 0, read: 0 })));
    Ok(Value::Object(o))
}

fn with_inflate<T>(it: &Interp, v: &Value, func: &str, g: impl FnOnce(&mut InflateState) -> T) -> R<T> {
    if let Value::Object(o) = v {
        if let Some(Internal::Data(d)) = o.internal_mut().as_deref_mut() {
            if let Some(st) = d.as_any_mut().downcast_mut::<InflateState>() {
                return Ok(g(st));
            }
        }
    }
    Err(it.error("TypeError", &format!("{func}(): Argument #1 ($context) must be of type InflateContext")))
}

fn inflate_add(it: &Interp, a: &[Value]) -> R<Value> {
    let data = s(it, a, 1)?.to_vec();
    let r = with_inflate(it, &arg(a, 0), "inflate_add", |st| -> Option<Vec<u8>> {
        let mut data = data;
        st.read += data.len() as u64;
        if let Some(p) = &mut st.gzip_pending {
            p.extend_from_slice(&data);
            match crate::filters::gzip_header_len(p) {
                Some(n) => {
                    data = p.split_off(n);
                    st.gzip_pending = None;
                }
                None => return Some(vec![]),
            }
        }
        let mut out = vec![];
        let mut input = &data[..];
        loop {
            out.reserve(64 * 1024);
            let (bi, bo) = (st.d.total_in(), st.d.total_out());
            let r = st.d.decompress_vec(input, &mut out, flate2::FlushDecompress::None).ok()?;
            input = &input[(st.d.total_in() - bi) as usize..];
            let produced = st.d.total_out() - bo;
            if r == flate2::Status::StreamEnd {
                st.status = 1;
                break;
            }
            if input.is_empty() && (produced == 0 || out.len() < out.capacity()) {
                break;
            }
        }
        Some(out)
    })?;
    match r {
        Some(b) => Ok(Value::bytes(b)),
        None => {
            it.warn("inflate_add(): data error");
            Ok(Value::Bool(false))
        }
    }
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("gzopen", |it, _, a| {
        let path = text(it, a, 0)?;
        let mode = text(it, a, 1)?;
        match open(it, &path, &mode, "gzopen") {
            Ok(v) => Ok(v),
            Err(msg) => {
                it.warn(&msg);
                Ok(Value::Bool(false))
            }
        }
    });
    // The gz* stream functions are the f* ones on a gzopen() stream.
    t.insert("gzread", |it, f, a| (it.builtins["fread"])(it, f, a));
    t.insert("gzwrite", |it, f, a| (it.builtins["fwrite"])(it, f, a));
    t.insert("gzputs", |it, f, a| (it.builtins["fwrite"])(it, f, a));
    t.insert("gzclose", |it, f, a| (it.builtins["fclose"])(it, f, a));
    t.insert("gzgets", |it, f, a| (it.builtins["fgets"])(it, f, a));
    t.insert("gzgetc", |it, f, a| (it.builtins["fgetc"])(it, f, a));
    t.insert("gzeof", |it, f, a| (it.builtins["feof"])(it, f, a));
    t.insert("gztell", |it, f, a| (it.builtins["ftell"])(it, f, a));
    t.insert("gzrewind", |it, f, a| (it.builtins["rewind"])(it, f, a));
    t.insert("gzpassthru", |it, f, a| (it.builtins["fpassthru"])(it, f, a));
    t.insert("gzseek", |it, f, a| {
        if has(a, 2) && int(it, a, 2)? == 2 {
            return Err(it.error("ValueError", "gzseek(): SEEK_END is not supported"));
        }
        (it.builtins["fseek"])(it, f, a)
    });
    t.insert("gzfile", |it, _, a| {
        let path = text(it, a, 0)?;
        match read_gz(&path) {
            Ok(data) => {
                let mut lines = vec![];
                let mut start = 0;
                for (i, &c) in data.iter().enumerate() {
                    if c == b'\n' {
                        lines.push(Value::bytes(data[start..=i].to_vec()));
                        start = i + 1;
                    }
                }
                if start < data.len() {
                    lines.push(Value::bytes(data[start..].to_vec()));
                }
                Ok(Value::array(PhpArray::from_list(lines)))
            }
            Err(e) => {
                it.warn(&format!("gzfile({path}): Failed to open stream: {}", crate::streams::io_msg(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("readgzfile", |it, _, a| {
        let path = text(it, a, 0)?;
        match read_gz(&path) {
            Ok(data) => {
                it.echo(&data);
                Ok(Value::Int(data.len() as i64))
            }
            Err(e) => {
                it.warn(&format!("readgzfile({path}): Failed to open stream: {}", crate::streams::io_msg(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("deflate_init", |it, _, a| deflate_init(it, a));
    t.insert("deflate_add", |it, _, a| deflate_add(it, a));
    t.insert("inflate_init", |it, _, a| inflate_init(it, a));
    t.insert("inflate_add", |it, _, a| inflate_add(it, a));
    t.insert("inflate_get_status", |it, _, a| Ok(Value::Int(with_inflate(it, &arg(a, 0), "inflate_get_status", |st| st.status)?)));
    t.insert("inflate_get_read_len", |it, _, a| Ok(Value::Int(with_inflate(it, &arg(a, 0), "inflate_get_read_len", |st| st.read as i64)?)));
    t.insert("zlib_get_coding_type", |_, _, _| Ok(Value::Bool(false)));
    t.insert("ob_gzhandler", |it, f, a| {
        let accept = it.with_globals(f, |v| v.get("_SERVER"));
        let accept = match accept {
            Some(Value::Array(srv)) => srv.get(&Key::from_bytes(b"HTTP_ACCEPT_ENCODING")).map(|v| it.to_string_lossy(&v)).transpose()?,
            _ => None,
        };
        let Some(acc) = accept else { return Ok(Value::Bool(false)) };
        let data = s(it, a, 0)?.to_vec();
        let mut fr = crate::interp::Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
        let (func, enc) = if acc.contains("gzip") {
            ("gzencode", "gzip")
        } else if acc.contains("deflate") {
            ("gzcompress", "deflate")
        } else {
            return Ok(Value::Bool(false));
        };
        let out = (it.builtins[func])(it, &mut fr, &mut [Value::bytes(data)])?;
        (it.builtins["header"])(it, &mut fr, &mut [Value::str(&format!("Content-Encoding: {enc}"))])?;
        (it.builtins["header"])(it, &mut fr, &mut [Value::str("Vary: Accept-Encoding")])?;
        Ok(out)
    });
}
