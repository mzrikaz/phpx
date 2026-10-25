//! Filesystem and path built-ins.
//! TODO: these block the OS thread; move to the runtime's blocking pool.

use std::collections::HashMap;
use std::path::Path;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Key, PhpArray, Value};

fn path(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&s(it, a, i)?).into_owned())
}

/// PHP dirname(): handles trailing slashes, "." for bare names, "/" for root.
pub fn dirname(p: &str, levels: usize) -> String {
    let mut cur = p.to_string();
    for _ in 0..levels {
        let trimmed = cur.trim_end_matches('/');
        if trimmed.is_empty() {
            return if cur.starts_with('/') { "/".into() } else { ".".into() };
        }
        cur = match trimmed.rfind('/') {
            Some(0) => "/".into(),
            Some(i) => trimmed[..i].trim_end_matches('/').to_string().chars().collect::<String>(),
            None => ".".into(),
        };
        if cur.is_empty() {
            cur = "/".into();
        }
    }
    cur
}

pub fn basename(p: &str, suffix: &str) -> String {
    let trimmed = p.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or("");
    match base.strip_suffix(suffix) {
        Some(b) if !suffix.is_empty() && !b.is_empty() => b.to_string(),
        _ => base.to_string(),
    }
}

thread_local! {
    /// realpath() results (PHP keeps a realpath cache too): resolving a
    /// path costs a readlink per component.
    static REALPATH: std::cell::RefCell<rustc_hash::FxHashMap<std::path::PathBuf, std::path::PathBuf>> = Default::default();
}

pub fn clear_realpath_cache() {
    REALPATH.with(|c| c.borrow_mut().clear());
}

/// Canonical path of an existing file, cached per thread.
pub fn realpath_cached(p: &Path) -> Option<std::path::PathBuf> {
    if let Some(r) = REALPATH.with(|c| c.borrow().get(p).cloned()) {
        return Some(r);
    }
    let r = std::fs::canonicalize(p).ok()?;
    REALPATH.with(|c| c.borrow_mut().insert(p.to_path_buf(), r.clone()));
    Some(r)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("dirname", |it, _, a| {
        let levels = if has(a, 1) { int(it, a, 1)?.max(1) as usize } else { 1 };
        Ok(Value::str(&dirname(&path(it, a, 0)?, levels)))
    });
    t.insert("basename", |it, _, a| {
        let suffix = if has(a, 1) { path(it, a, 1)? } else { String::new() };
        Ok(Value::str(&basename(&path(it, a, 0)?, &suffix)))
    });
    t.insert("pathinfo", |it, _, a| {
        let p = path(it, a, 0)?;
        let base = basename(&p, "");
        let mut out = PhpArray::new();
        let set = |out: &mut PhpArray, k: &str, v: &str| out.insert(Key::Str(crate::value::pstr(k)), Value::str(v));
        if p.contains('/') {
            set(&mut out, "dirname", &dirname(&p, 1));
        } else {
            set(&mut out, "dirname", ".");
        }
        set(&mut out, "basename", &base);
        if let Some(i) = base.rfind('.') {
            set(&mut out, "extension", &base[i + 1..]);
            set(&mut out, "filename", &base[..i]);
        } else {
            set(&mut out, "filename", &base);
        }
        if has(a, 1) {
            let flag = int(it, a, 1)?;
            let key = match flag {
                1 => "dirname",
                2 => "basename",
                4 => "extension",
                _ => "filename",
            };
            return Ok(out.get(&Key::Str(crate::value::pstr(key))).unwrap_or(Value::str("")));
        }
        Ok(Value::array(out))
    });
    t.insert("realpath", |it, _, a| {
        Ok(realpath_cached(std::path::Path::new(&path(it, a, 0)?)).map(|p| Value::str(&p.display().to_string())).unwrap_or(Value::Bool(false)))
    });
    t.insert("file_exists", |it, _, a| Ok(Value::Bool(Path::new(&path(it, a, 0)?).exists())));
    t.insert("is_file", |it, _, a| Ok(Value::Bool(Path::new(&path(it, a, 0)?).is_file())));
    t.insert("is_dir", |it, _, a| Ok(Value::Bool(Path::new(&path(it, a, 0)?).is_dir())));
    t.insert("is_readable", |it, _, a| Ok(Value::Bool(Path::new(&path(it, a, 0)?).exists())));
    t.insert("is_writable", |it, _, a| {
        let p = path(it, a, 0)?;
        Ok(Value::Bool(std::fs::metadata(&p).is_ok_and(|m| !m.permissions().readonly())))
    });
    t.insert("is_link", |it, _, a| Ok(Value::Bool(std::fs::symlink_metadata(path(it, a, 0)?).is_ok_and(|m| m.file_type().is_symlink()))));
    t.insert("filesize", |it, _, a| {
        Ok(std::fs::metadata(path(it, a, 0)?).map(|m| Value::Int(m.len() as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("filemtime", |it, _, a| {
        let m = std::fs::metadata(path(it, a, 0)?).and_then(|m| m.modified());
        Ok(m.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| Value::Int(d.as_secs() as i64))
            .unwrap_or(Value::Bool(false)))
    });
    t.insert("file_get_contents", |it, _, a| {
        let p = path(it, a, 0)?;
        let offset = if has(a, 3) { int(it, a, 3)? } else { 0 };
        let length = if has(a, 4) && !arg(a, 4).is_null() { Some(int(it, a, 4)?) } else { None };
        if length.is_some_and(|l| l < 0) {
            return Err(it.error("ValueError", "file_get_contents(): Argument #5 ($length) must be greater than or equal to 0"));
        }
        if p.starts_with("php://") || p.starts_with("data:") || (p.contains("://") && !p.starts_with("file://")) || crate::wrappers::class_for(&p).is_some() || crate::wrappers::disabled(&p) {
            let res = match crate::streams::open(it, &p, "rb") {
                Ok(r) => r,
                Err(msg) => {
                    it.warn(&msg.replacen("fopen(", "file_get_contents(", 1));
                    return Ok(Value::Bool(false));
                }
            };
            let data = crate::streams::with_stream(it, &res, "file_get_contents", |st| {
                let mut out = vec![];
                loop {
                    let chunk = st.read(65536, it);
                    if chunk.is_empty() {
                        break;
                    }
                    out.extend_from_slice(&chunk);
                }
                out
            })?;
            crate::streams::close_stream(it, &res);
            let start = (offset.max(0) as usize).min(data.len());
            let end = length.map_or(data.len(), |l| (start + l as usize).min(data.len()));
            return Ok(Value::bytes(data[start..end].to_vec()));
        }
        let p = p.strip_prefix("file://").map(str::to_string).unwrap_or(p);
        let read = if offset != 0 || length.is_some() {
            use std::io::{Read, Seek};
            std::fs::File::open(&p).and_then(|mut f| {
                let len = f.metadata()?.len() as i64;
                let start = if offset < 0 { (len + offset).max(0) } else { offset };
                f.seek(std::io::SeekFrom::Start(start as u64))?;
                let mut b = vec![];
                match length {
                    Some(l) => f.take(l as u64).read_to_end(&mut b)?,
                    None => f.read_to_end(&mut b)?,
                };
                Ok(b)
            })
        } else {
            std::fs::read(&p)
        };
        match read {
            Ok(b) => Ok(Value::bytes(b)),
            Err(e) => {
                it.warn(&format!("file_get_contents({p}): Failed to open stream: {}", io_reason(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("file_put_contents", |it, _, a| {
        let p = path(it, a, 0)?;
        let data = match arg(a, 1) {
            Value::Array(x) => {
                let mut b = vec![];
                for v in x.values() {
                    b.extend_from_slice(&it.to_str(&v)?);
                }
                b
            }
            v => it.to_str(&v)?.to_vec(),
        };
        let append = int(it, a, 2)? & 8 != 0;
        if p.starts_with("php://") || (p.contains("://") && !p.starts_with("file://")) || crate::wrappers::class_for(&p).is_some() || crate::wrappers::disabled(&p) {
            let res = match crate::streams::open(it, &p, if append { "ab" } else { "wb" }) {
                Ok(r) => r,
                Err(msg) => {
                    it.warn(&msg.replacen("fopen(", "file_put_contents(", 1));
                    return Ok(Value::Bool(false));
                }
            };
            let n = crate::streams::with_stream(it, &res, "file_put_contents", |st| st.write(&data, it))?;
            crate::streams::close_stream(it, &res);
            return Ok(Value::Int(n as i64));
        }
        let p = p.strip_prefix("file://").map(str::to_string).unwrap_or(p);
        let r = if append {
            use std::io::Write;
            std::fs::OpenOptions::new().append(true).create(true).open(&p).and_then(|mut f| f.write_all(&data))
        } else {
            std::fs::write(&p, &data)
        };
        match r {
            Ok(()) => Ok(Value::Int(data.len() as i64)),
            Err(e) => {
                it.warn(&format!("file_put_contents({p}): Failed to open stream: {}", io_reason(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("file", |it, _, a| {
        let p = path(it, a, 0)?;
        let flags = int(it, a, 1)?;
        let Ok(b) = std::fs::read(&p) else { return Ok(Value::Bool(false)) };
        let mut lines = vec![];
        let mut start = 0;
        for (i, c) in b.iter().enumerate() {
            if *c == b'\n' {
                let end = if flags & 2 != 0 { i } else { i + 1 };
                lines.push(b[start..end].to_vec());
                start = i + 1;
            }
        }
        if start < b.len() {
            lines.push(b[start..].to_vec());
        }
        if flags & 4 != 0 {
            lines.retain(|l| !l.is_empty());
        }
        Ok(Value::array(PhpArray::from_list(lines.into_iter().map(Value::bytes))))
    });
    t.insert("unlink", |it, _, a| Ok(Value::Bool(std::fs::remove_file(path(it, a, 0)?).is_ok())));
    t.insert("mkdir", |it, _, a| {
        let p = path(it, a, 0)?;
        let r = if arg(a, 2).truthy() { std::fs::create_dir_all(&p) } else { std::fs::create_dir(&p) };
        Ok(Value::Bool(r.is_ok()))
    });
    t.insert("rmdir", |it, _, a| Ok(Value::Bool(std::fs::remove_dir(path(it, a, 0)?).is_ok())));
    t.insert("rename", |it, _, a| Ok(Value::Bool(std::fs::rename(path(it, a, 0)?, path(it, a, 1)?).is_ok())));
    t.insert("copy", |it, _, a| Ok(Value::Bool(std::fs::copy(path(it, a, 0)?, path(it, a, 1)?).is_ok())));
    t.insert("touch", |it, _, a| {
        let p = path(it, a, 0)?;
        Ok(Value::Bool(std::fs::OpenOptions::new().create(true).append(true).open(p).is_ok()))
    });
    t.insert("scandir", |it, _, a| {
        let p = path(it, a, 0)?;
        let Ok(rd) = std::fs::read_dir(&p) else { return Ok(Value::Bool(false)) };
        let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.push(".".into());
        names.push("..".into());
        names.sort();
        if int(it, a, 1)? == 1 {
            names.reverse();
        }
        Ok(Value::array(PhpArray::from_list(names.iter().map(|n| Value::str(n)))))
    });
    t.insert("glob", |it, _, a| {
        let pat = path(it, a, 0)?;
        let flags = if a.len() > 1 { it.to_int(&a[1])? } else { 0 };
        Ok(Value::array(PhpArray::from_list(crate::glob::glob(&pat, flags).iter().map(|n| Value::str(n)))))
    });
    t.insert("getcwd", |_, _, _| Ok(std::env::current_dir().map(|d| Value::str(&d.display().to_string())).unwrap_or(Value::Bool(false))));
    t.insert("chdir", |it, _, a| Ok(Value::Bool(std::env::set_current_dir(path(it, a, 0)?).is_ok())));
    t.insert("sys_get_temp_dir", |_, _, _| Ok(Value::str(&std::env::temp_dir().display().to_string())));
    t.insert("tempnam", |it, _, a| {
        let dir = path(it, a, 0)?;
        let prefix = path(it, a, 1)?;
        let name = format!("{dir}/{prefix}{:x}", crate::builtins::random_u64());
        std::fs::write(&name, b"").ok();
        Ok(Value::str(&name))
    });
    t.insert("fopen", |it, _, _| Err(it.error("Error", "phpx: stream functions (fopen & co) are not supported yet")));
    t.insert("clearstatcache", |_, _, a| {
        if a.first().is_some_and(|v| v.truthy()) {
            crate::fs::clear_realpath_cache();
        }
        Ok(Value::Null)
    });
}

fn io_reason(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory",
        std::io::ErrorKind::PermissionDenied => "Permission denied",
        _ => "I/O error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_like_php() {
        assert_eq!(dirname("/a/b/c.php", 1), "/a/b");
        assert_eq!(dirname("/a/b/", 1), "/a");
        assert_eq!(dirname("file", 1), ".");
        assert_eq!(dirname("/file", 1), "/");
        assert_eq!(dirname("/a/b/c", 2), "/a");
        assert_eq!(basename("/a/b/c.php", ".php"), "c");
        assert_eq!(basename("/a/b/", ""), "b");
        assert!(crate::streams::fnmatch(b"*.php", b"index.php"));
        assert!(!crate::streams::fnmatch(b"*.php", b"index.phpx"));
    }
}
