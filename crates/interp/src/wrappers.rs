//! User-space stream wrappers: stream_wrapper_register() and the
//! filesystem functions that dispatch `scheme://` paths to the wrapper
//! class's methods (stream_open, url_stat, unlink, mkdir, dir_opendir...),
//! as vfsStream and Flysystem's test doubles need.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

use crate::builtins::arg;
use crate::interp::{BuiltinFn, Frame, Interp, R};
use crate::value::{Key, Obj, PhpArray, Resource, Value};

thread_local! {
    /// scheme (lowercase) -> wrapper class name.
    static WRAPPERS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// Built-in schemes taken down with stream_wrapper_unregister().
    static DISABLED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

/// A new request: user wrappers registered by the last one are gone.
pub fn reset() {
    WRAPPERS.with(|w| w.borrow_mut().clear());
    DISABLED.with(|d| d.borrow_mut().clear());
}

const BUILTIN: &[&str] = &["https", "ftps", "compress.zlib", "php", "file", "glob", "data", "http", "ftp"];

fn scheme(path: &str) -> Option<String> {
    let (s, _) = path.split_once("://")?;
    (!s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))).then(|| s.to_ascii_lowercase())
}

/// The wrapper class for `path`, if a user wrapper handles it.
pub fn class_for(path: &str) -> Option<String> {
    WRAPPERS.with(|w| {
        let w = w.borrow();
        if w.is_empty() {
            return None;
        }
        // Plain paths belong to "file" (a user wrapper may replace it).
        let s = scheme(path).unwrap_or_else(|| "file".into());
        w.get(&s).cloned()
    })
}

/// Whether `path`'s built-in scheme was unregistered.
pub fn disabled(path: &str) -> bool {
    let s = scheme(path).unwrap_or_else(|| "file".into());
    DISABLED.with(|d| d.borrow().contains(&s))
}

fn instance(it: &Interp, class: &str) -> R<Rc<Obj>> {
    let cls = it.lookup_class(class)?;
    let f = Frame::new(it.file_ctx(it.pos.get().file as usize), 0);
    let o = it.instantiate(&f, &cls, vec![], vec![])?;
    if cls.props.iter().any(|p| &*p.name == "context") {
        o.set_prop("context", Value::Null);
    }
    Ok(o)
}

fn has_method(o: &Rc<Obj>, m: &str) -> bool {
    o.class.find_method(m).is_some()
}

/// Call `m` if the wrapper defines it, else warn like PHP and return None.
fn call(it: &Interp, o: &Rc<Obj>, m: &str, args: Vec<Value>, func: &str) -> R<Option<Value>> {
    if !has_method(o, m) {
        it.warn(&format!("{func}(): {}::{m} is not implemented!", o.class.name));
        return Ok(None);
    }
    it.call_method_dyn(o, m, args).map(Some)
}

// ---- streams ---------------------------------------------------------------------

/// fopen() on a user wrapper: Some(result) when `path` is one.
pub fn open(it: &Interp, path: &str, mode: &str) -> Option<Result<Value, String>> {
    let class = class_for(path)?;
    let r = (|| -> R<Result<Value, String>> {
        let o = instance(it, &class)?;
        let opened = crate::interp::slot(Value::Null);
        let ok = call(it, &o, "stream_open", vec![Value::str(path), Value::str(mode), Value::Int(0), Value::Ref(opened)], "fopen")?;
        if !ok.is_some_and(|v| v.truthy()) {
            return Ok(Err(format!("fopen({path}): Failed to open stream: \"{class}::stream_open\" call failed")));
        }
        let st = crate::streams::Stream::new(crate::streams::Backend::User(o), mode, path, "user-space");
        Ok(Ok(Resource::new("stream", Box::new(st))))
    })();
    Some(match r {
        Ok(x) => x,
        Err(e) => {
            crate::errors::defer(e);
            Err(String::new())
        }
    })
}

/// include/require of a user-wrapper path: the whole stream's bytes, or
/// PHP's failure message. None: not a user-wrapper path.
pub fn read_for_include(it: &Interp, path: &str, func: &str) -> Option<R<Result<Vec<u8>, String>>> {
    let class = class_for(path)?;
    Some((|| {
        let o = instance(it, &class)?;
        let opened = crate::interp::slot(Value::Null);
        let ok = call(it, &o, "stream_open", vec![Value::str(path), Value::str("rb"), Value::Int(0), Value::Ref(opened)], func)?;
        if !ok.is_some_and(|v| v.truthy()) {
            return Ok(Err(format!("{func}({path}): Failed to open stream: \"{class}::stream_open\" call failed")));
        }
        let mut src = vec![];
        loop {
            let (chunk, eof) = read(it, &o, 8192);
            if let Some(e) = crate::errors::take() {
                return Err(e);
            }
            src.extend_from_slice(&chunk);
            if eof || chunk.is_empty() {
                break;
            }
        }
        if has_method(&o, "stream_close") {
            it.call_method(&o, "stream_close", vec![])?;
        }
        Ok(Ok(src))
    })())
}

pub fn read(it: &Interp, o: &Rc<Obj>, n: usize) -> (Vec<u8>, bool) {
    let data = match it.call_method(o, "stream_read", vec![Value::Int(n as i64)]) {
        Ok(v @ Value::Str(_)) => v.scalar_bytes().unwrap_or_default().to_vec(),
        Ok(_) => vec![],
        Err(e) => {
            crate::errors::defer(e);
            vec![]
        }
    };
    let eof = has_method(o, "stream_eof") && it.call_method(o, "stream_eof", vec![]).map(|v| v.truthy()).unwrap_or(true);
    (data, eof)
}

pub fn write(it: &Interp, o: &Rc<Obj>, data: &[u8]) -> usize {
    match it.call_method(o, "stream_write", vec![Value::bytes(data.to_vec())]) {
        Ok(v) => it.to_int(&v).unwrap_or(0).clamp(0, data.len() as i64) as usize,
        Err(e) => {
            crate::errors::defer(e);
            0
        }
    }
}

/// A user stream method: its result, or None when undefined or throwing.
pub fn method(it: &Interp, o: &Rc<Obj>, m: &str, args: Vec<Value>) -> Option<Value> {
    if !has_method(o, m) {
        return None;
    }
    match it.call_method_dyn(o, m, args) {
        Ok(v) => Some(v),
        Err(e) => {
            crate::errors::defer(e);
            None
        }
    }
}

// ---- path functions ---------------------------------------------------------------

static ORIGINALS: OnceLock<HashMap<&'static str, BuiltinFn>> = OnceLock::new();

fn original(name: &str) -> BuiltinFn {
    ORIGINALS.get().and_then(|m| m.get(name)).copied().expect("wrapped built-in")
}

fn path_of(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&it.to_str(&arg(a, i))?).into_owned())
}

/// url_stat(): the stat array (numeric and named keys) or None.
fn url_stat(it: &Interp, path: &str, flags: i64) -> R<Option<PhpArray>> {
    let Some(class) = class_for(path) else { return Ok(None) };
    let o = instance(it, &class)?;
    if !has_method(&o, "url_stat") {
        return Ok(None);
    }
    Ok(match it.call_method(&o, "url_stat", vec![Value::str(path), Value::Int(flags)])? {
        Value::Array(a) => Some(normalize_stat(&a)),
        _ => None,
    })
}

/// A stat array with both index and name keys, like PHP builds it.
pub fn normalize_stat(a: &PhpArray) -> PhpArray {
    const NAMES: [&str; 13] = ["dev", "ino", "mode", "nlink", "uid", "gid", "rdev", "size", "atime", "mtime", "ctime", "blksize", "blocks"];
    let get = |i: usize| -> Value {
        a.get(&Key::from_bytes(NAMES[i].as_bytes())).or_else(|| a.get(&Key::Int(i as i64))).map(|v| v.deref()).unwrap_or(Value::Int(if i >= 11 { -1 } else { 0 }))
    };
    let mut out = PhpArray::new();
    for i in 0..13 {
        out.insert(Key::Int(i as i64), get(i));
    }
    for (i, n) in NAMES.iter().enumerate() {
        out.insert(Key::from_bytes(n.as_bytes()), get(i));
    }
    out
}

fn stat_field(st: &PhpArray, name: &str) -> i64 {
    match st.get(&Key::from_bytes(name.as_bytes())).map(|v| v.deref()) {
        Some(Value::Int(n)) => n,
        Some(Value::Float(f)) => f as i64,
        _ => 0,
    }
}

const S_IFMT: i64 = 0o170000;
const S_IFDIR: i64 = 0o040000;
const S_IFREG: i64 = 0o100000;
const S_IFLNK: i64 = 0o120000;
const QUIET: i64 = 2;
const LINK: i64 = 1;

fn intercept(name: &'static str) -> BuiltinFn {
    match name {
        "file_exists" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET)?.is_some())),
            None => original("file_exists")(it, f, a),
        },
        "is_file" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET)?.is_some_and(|s| stat_field(&s, "mode") & S_IFMT == S_IFREG))),
            None => original("is_file")(it, f, a),
        },
        "is_dir" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET)?.is_some_and(|s| stat_field(&s, "mode") & S_IFMT == S_IFDIR))),
            None => original("is_dir")(it, f, a),
        },
        "is_link" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET | LINK)?.is_some_and(|s| stat_field(&s, "mode") & S_IFMT == S_IFLNK))),
            None => original("is_link")(it, f, a),
        },
        "is_readable" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET)?.is_some_and(|s| stat_field(&s, "mode") & 0o444 != 0))),
            None => original("is_readable")(it, f, a),
        },
        "is_writable" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET)?.is_some_and(|s| stat_field(&s, "mode") & 0o222 != 0))),
            None => original("is_writable")(it, f, a),
        },
        "is_writeable" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET)?.is_some_and(|s| stat_field(&s, "mode") & 0o222 != 0))),
            None => original("is_writeable")(it, f, a),
        },
        "is_executable" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(url_stat(it, &path_of(it, a, 0)?, QUIET)?.is_some_and(|s| stat_field(&s, "mode") & 0o111 != 0))),
            None => original("is_executable")(it, f, a),
        },
        "filesize" => |it, f, a| stat_int(it, f, a, "filesize", "size"),
        "filemtime" => |it, f, a| stat_int(it, f, a, "filemtime", "mtime"),
        "fileatime" => |it, f, a| stat_int(it, f, a, "fileatime", "atime"),
        "filectime" => |it, f, a| stat_int(it, f, a, "filectime", "ctime"),
        "fileperms" => |it, f, a| stat_int(it, f, a, "fileperms", "mode"),
        "fileowner" => |it, f, a| stat_int(it, f, a, "fileowner", "uid"),
        "filegroup" => |it, f, a| stat_int(it, f, a, "filegroup", "gid"),
        "fileinode" => |it, f, a| stat_int(it, f, a, "fileinode", "ino"),
        "filetype" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            if class_for(&p).is_none() {
                return original("filetype")(it, f, a);
            }
            Ok(match url_stat(it, &p, 0)? {
                Some(s) => Value::str(match stat_field(&s, "mode") & S_IFMT {
                    S_IFDIR => "dir",
                    S_IFLNK => "link",
                    S_IFREG => "file",
                    0o010000 => "fifo",
                    0o020000 => "char",
                    0o060000 => "block",
                    0o140000 => "socket",
                    _ => "unknown",
                }),
                None => {
                    it.warn(&format!("filetype(): Lstat failed for {p}"));
                    Value::Bool(false)
                }
            })
        },
        "stat" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            if class_for(&p).is_none() {
                return original("stat")(it, f, a);
            }
            Ok(match url_stat(it, &p, 0)? {
                Some(s) => Value::array(s),
                None => {
                    it.warn(&format!("stat(): stat failed for {p}"));
                    Value::Bool(false)
                }
            })
        },
        "lstat" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            if class_for(&p).is_none() {
                return original("lstat")(it, f, a);
            }
            Ok(match url_stat(it, &p, LINK)? {
                Some(s) => Value::array(s),
                None => {
                    it.warn(&format!("lstat(): Lstat failed for {p}"));
                    Value::Bool(false)
                }
            })
        },
        "unlink" => |it, f, a| path_call(it, f, a, "unlink", "unlink", 1),
        "rmdir" => |it, f, a| path_call(it, f, a, "rmdir", "rmdir", 1),
        "mkdir" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            let Some(class) = class_for(&p) else { return original("mkdir")(it, f, a) };
            let o = instance(it, &class)?;
            let mode = a.get(1).map(|v| it.to_int(v)).transpose()?.unwrap_or(0o777);
            let rec = a.get(2).is_some_and(|v| v.truthy());
            let r = call(it, &o, "mkdir", vec![Value::str(&p), Value::Int(mode), Value::Int(if rec { 1 } else { 0 } | 8)], "mkdir")?;
            Ok(Value::Bool(r.is_some_and(|v| v.truthy())))
        },
        "rename" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            let Some(class) = class_for(&p) else { return original("rename")(it, f, a) };
            let to = path_of(it, a, 1)?;
            let o = instance(it, &class)?;
            let r = call(it, &o, "rename", vec![Value::str(&p), Value::str(&to)], "rename")?;
            Ok(Value::Bool(r.is_some_and(|v| v.truthy())))
        },
        "touch" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            let Some(class) = class_for(&p) else { return original("touch")(it, f, a) };
            let o = instance(it, &class)?;
            let mtime = a.get(1).filter(|v| !v.is_null()).map(|v| it.to_int(v)).transpose()?;
            let atime = a.get(2).filter(|v| !v.is_null()).map(|v| it.to_int(v)).transpose()?.or(mtime);
            let arg = match mtime {
                Some(m) => Value::array(PhpArray::from_list([Value::Int(m), Value::Int(atime.unwrap_or(m))])),
                None => Value::array(PhpArray::new()),
            };
            let r = call(it, &o, "stream_metadata", vec![Value::str(&p), Value::Int(1), arg], "touch")?;
            Ok(Value::Bool(r.is_some_and(|v| v.truthy())))
        },
        "chmod" => |it, f, a| metadata(it, f, a, "chmod", 6),
        "chown" => |it, f, a| metadata(it, f, a, "chown", if matches!(arg(a, 1), Value::Int(_)) { 3 } else { 2 }),
        "chgrp" => |it, f, a| metadata(it, f, a, "chgrp", if matches!(arg(a, 1), Value::Int(_)) { 5 } else { 4 }),
        "opendir" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            let Some(class) = class_for(&p) else { return original("opendir")(it, f, a) };
            match dir_entries(it, &class, &p)? {
                Some(entries) => Ok(crate::streams::dir_resource(entries)),
                None => {
                    it.warn(&format!("opendir({p}): Failed to open directory: \"{class}::dir_opendir\" call failed"));
                    Ok(Value::Bool(false))
                }
            }
        },
        "scandir" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            let Some(class) = class_for(&p) else { return original("scandir")(it, f, a) };
            match dir_entries(it, &class, &p)? {
                Some(mut entries) => {
                    let desc = a.get(1).map(|v| it.to_int(v)).transpose()?.unwrap_or(0) == 1;
                    if a.get(1).map(|v| it.to_int(v)).transpose()?.unwrap_or(0) != 2 {
                        entries.sort();
                        if desc {
                            entries.reverse();
                        }
                    }
                    Ok(Value::array(PhpArray::from_list(entries.iter().map(|e| Value::str(e)).collect::<Vec<_>>())))
                }
                None => {
                    it.warn(&format!("scandir({p}): Failed to open directory: \"{class}::dir_opendir\" call failed"));
                    it.warn("scandir(): (errno 0): Success");
                    Ok(Value::Bool(false))
                }
            }
        },
        "file" => |it, f, a| {
            let p = path_of(it, a, 0)?;
            if class_for(&p).is_none() {
                return original("file")(it, f, a);
            }
            let data = (it.builtins["file_get_contents"])(it, f, &mut [Value::str(&p)])?;
            let Value::Str(b) = data else { return Ok(Value::Bool(false)) };
            let flags = a.get(1).map(|v| it.to_int(v)).transpose()?.unwrap_or(0);
            let mut lines = vec![];
            let mut start = 0;
            for (i, &c) in b.iter().enumerate() {
                if c == b'\n' {
                    lines.push(b[start..=i].to_vec());
                    start = i + 1;
                }
            }
            if start < b.len() {
                lines.push(b[start..].to_vec());
            }
            let lines: Vec<Value> = lines
                .into_iter()
                .map(|mut l| {
                    if flags & 2 != 0 {
                        while l.last().is_some_and(|&c| c == b'\n' || c == b'\r') {
                            l.pop();
                        }
                    }
                    l
                })
                .filter(|l| flags & 4 == 0 || !l.is_empty())
                .map(Value::bytes)
                .collect();
            Ok(Value::array(PhpArray::from_list(lines)))
        },
        "copy" => |it, f, a| {
            let (from, to) = (path_of(it, a, 0)?, path_of(it, a, 1)?);
            if class_for(&from).is_none() && class_for(&to).is_none() {
                return original("copy")(it, f, a);
            }
            let data = (it.builtins["file_get_contents"])(it, f, &mut [Value::str(&from)])?;
            if matches!(data, Value::Bool(false)) {
                return Ok(Value::Bool(false));
            }
            let r = (it.builtins["file_put_contents"])(it, f, &mut [Value::str(&to), data])?;
            Ok(Value::Bool(!matches!(r, Value::Bool(false))))
        },
        "realpath" => |it, f, a| match class_for(&path_of(it, a, 0)?) {
            Some(_) => Ok(Value::Bool(false)),
            None => original("realpath")(it, f, a),
        },
        _ => unreachable!(),
    }
}

fn stat_int(it: &Interp, f: &mut Frame, a: &mut [Value], func: &'static str, field: &str) -> R<Value> {
    let p = path_of(it, a, 0)?;
    if class_for(&p).is_none() {
        return original(func)(it, f, a);
    }
    Ok(match url_stat(it, &p, 0)? {
        Some(s) => Value::Int(stat_field(&s, field)),
        None => {
            it.warn(&format!("{func}(): stat failed for {p}"));
            Value::Bool(false)
        }
    })
}

fn path_call(it: &Interp, f: &mut Frame, a: &mut [Value], func: &'static str, m: &str, nargs: usize) -> R<Value> {
    let p = path_of(it, a, 0)?;
    let Some(class) = class_for(&p) else { return original(func)(it, f, a) };
    let o = instance(it, &class)?;
    let mut args = vec![Value::str(&p)];
    if nargs > 1 {
        args.push(Value::Int(0));
    }
    if m == "rmdir" {
        args.push(Value::Int(8));
    }
    let r = call(it, &o, m, args, func)?;
    Ok(Value::Bool(r.is_some_and(|v| v.truthy())))
}

fn metadata(it: &Interp, f: &mut Frame, a: &mut [Value], func: &'static str, option: i64) -> R<Value> {
    let p = path_of(it, a, 0)?;
    let Some(class) = class_for(&p) else { return original(func)(it, f, a) };
    let o = instance(it, &class)?;
    let r = call(it, &o, "stream_metadata", vec![Value::str(&p), Value::Int(option), arg(a, 1)], func)?;
    Ok(Value::Bool(r.is_some_and(|v| v.truthy())))
}

fn dir_entries(it: &Interp, class: &str, p: &str) -> R<Option<Vec<String>>> {
    let o = instance(it, class)?;
    if !call(it, &o, "dir_opendir", vec![Value::str(p), Value::Int(0)], "opendir")?.is_some_and(|v| v.truthy()) {
        return Ok(None);
    }
    let mut entries = vec![];
    loop {
        match it.call_method(&o, "dir_readdir", vec![])? {
            Value::Bool(false) | Value::Null => break,
            v => entries.push(it.to_string_lossy(&v)?),
        }
        if entries.len() > 1_000_000 {
            break;
        }
    }
    if has_method(&o, "dir_closedir") {
        it.call_method(&o, "dir_closedir", vec![])?;
    }
    Ok(Some(entries))
}

const INTERCEPTED: &[&str] = &[
    "file_exists", "is_file", "is_dir", "is_link", "is_readable", "is_writable", "is_writeable", "is_executable", "filesize", "filemtime", "fileatime", "filectime", "fileperms", "fileowner",
    "filegroup", "fileinode", "filetype", "stat", "lstat", "unlink", "rmdir", "mkdir", "rename", "touch", "chmod", "chown", "chgrp", "opendir", "scandir", "file", "copy", "realpath",
];

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    let _ = ORIGINALS.set(INTERCEPTED.iter().filter_map(|n| t.get(n).map(|f| (*n, *f))).collect());
    for n in INTERCEPTED {
        if t.contains_key(n) {
            t.insert(n, intercept(n));
        }
    }
    t.insert("stream_wrapper_register", |it, _, a| {
        let proto = it.to_string_lossy(&arg(a, 0))?;
        let class = it.to_string_lossy(&arg(a, 1))?;
        let lower = proto.to_ascii_lowercase();
        let exists = WRAPPERS.with(|w| w.borrow().contains_key(&lower)) || (BUILTIN.contains(&lower.as_str()) && !DISABLED.with(|d| d.borrow().contains(&lower)));
        if exists {
            it.warn(&format!("stream_wrapper_register(): Protocol {proto}:// is already defined"));
            return Ok(Value::Bool(false));
        }
        let Some(cls) = it.find_class(&class)? else {
            return Err(it.error("TypeError", &format!("stream_wrapper_register(): Argument #2 ($class) must be a valid class name, {class} given")));
        };
        WRAPPERS.with(|w| w.borrow_mut().insert(lower, cls.name.to_string()));
        Ok(Value::Bool(true))
    });
    t.insert("stream_register_wrapper", |it, f, a| (it.builtins["stream_wrapper_register"])(it, f, a));
    t.insert("stream_wrapper_unregister", |it, _, a| {
        let proto = it.to_string_lossy(&arg(a, 0))?;
        let lower = proto.to_ascii_lowercase();
        if WRAPPERS.with(|w| w.borrow_mut().remove(&lower)).is_some() {
            return Ok(Value::Bool(true));
        }
        if BUILTIN.contains(&lower.as_str()) && DISABLED.with(|d| d.borrow_mut().insert(lower)) {
            return Ok(Value::Bool(true));
        }
        it.warn(&format!("stream_wrapper_unregister(): Unable to unregister protocol {proto}://"));
        Ok(Value::Bool(false))
    });
    t.insert("stream_wrapper_restore", |it, _, a| {
        let proto = it.to_string_lossy(&arg(a, 0))?;
        let lower = proto.to_ascii_lowercase();
        if !BUILTIN.contains(&lower.as_str()) {
            it.warn(&format!("stream_wrapper_restore(): {proto}:// never existed, nothing to restore"));
            return Ok(Value::Bool(false));
        }
        let was_user = WRAPPERS.with(|w| w.borrow_mut().remove(&lower)).is_some();
        let was_off = DISABLED.with(|d| d.borrow_mut().remove(&lower));
        if !was_user && !was_off {
            it.notice(&format!("stream_wrapper_restore(): {proto}:// was never changed, nothing to restore"));
        }
        Ok(Value::Bool(true))
    });
    t.insert("stream_get_wrappers", |_, _, _| {
        let mut out: Vec<String> = BUILTIN.iter().filter(|b| !DISABLED.with(|d| d.borrow().contains(**b))).map(|s| s.to_string()).collect();
        let mut user: Vec<String> = WRAPPERS.with(|w| w.borrow().keys().cloned().collect());
        user.sort();
        out.extend(user);
        Ok(Value::array(PhpArray::from_list(out.iter().map(|s| Value::str(s)).collect::<Vec<_>>())))
    });
}
