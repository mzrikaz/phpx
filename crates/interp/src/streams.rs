//! Stream resources: fopen() and friends, php:// streams, directory
//! handles, stat().
//! TODO: file I/O blocks the OS thread; move to the runtime's blocking pool.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Key, PhpArray, Resource, Value};

pub(crate) enum Backend {
    File(File),
    Socket(crate::sockets::Sock),
    Memory(Vec<u8>),
    Stdin,
    Stdout,
    Stderr,
    Output,
}

/// Read-ahead for file streams: a buffer and a cursor into it (consuming
/// from the front doesn't move the rest).
#[derive(Default)]
pub(crate) struct ReadBuf {
    buf: Vec<u8>,
    at: usize,
}

impl ReadBuf {
    fn len(&self) -> usize {
        self.buf.len() - self.at
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn clear(&mut self) {
        self.buf.clear();
        self.at = 0;
    }

    fn bytes(&self) -> &[u8] {
        &self.buf[self.at..]
    }

    fn extend_from_slice(&mut self, b: &[u8]) {
        if self.at > 0 && self.at == self.buf.len() {
            self.clear();
        }
        self.buf.extend_from_slice(b);
    }

    /// Take the next `n` buffered bytes.
    fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.len());
        let out = self.buf[self.at..self.at + n].to_vec();
        self.at += n;
        if self.at == self.buf.len() {
            self.clear();
        }
        out
    }
}

pub struct Stream {
    pub(crate) backend: Backend,
    /// Position for memory streams.
    pos: usize,
    /// Read-ahead buffer for files and sockets (fgets reads lines).
    pub(crate) rbuf: ReadBuf,
    pub(crate) eof: bool,
    /// Sockets: read/write timeout (None: wait forever), whether the last
    /// read timed out, blocking mode, the stream context.
    pub(crate) timeout: Option<std::time::Duration>,
    timed_out: bool,
    pub(crate) blocking: bool,
    pub(crate) context: Option<Value>,
    mode: String,
    uri: String,
    wrapper: &'static str,
    append: bool,
}

pub struct Dir {
    entries: Vec<String>,
    at: usize,
}

fn path_arg(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&s(it, a, i)?).into_owned())
}

/// Run `g` on an open stream resource; TypeError for anything else.
fn with_stream<T>(it: &Interp, v: &Value, fname: &str, g: impl FnOnce(&mut Stream) -> T) -> R<T> {
    if let Value::Resource(r) = v {
        let mut d = r.data.borrow_mut();
        if let Some(st) = d.as_mut().and_then(|b| b.downcast_mut::<Stream>()) {
            return Ok(g(st));
        }
        if d.is_none() {
            return Err(it.error("TypeError", &format!("{fname}(): supplied resource is not a valid stream resource")));
        }
    }
    Err(it.error(
        "TypeError",
        &format!("{fname}(): Argument #1 ($stream) must be of type resource, {} given", v.debug_type()),
    ))
}

impl Stream {
    pub(crate) fn new(backend: Backend, mode: &str, uri: &str, wrapper: &'static str) -> Stream {
        Stream { backend, pos: 0, rbuf: ReadBuf::default(), eof: false, timeout: None, timed_out: false, blocking: true, context: None, mode: mode.to_string(), uri: uri.to_string(), wrapper, append: mode.starts_with('a') }
    }

    fn fill(&mut self, want: usize) -> usize {
        if let Backend::Socket(sk) = &mut self.backend {
            // Sockets: one read of whatever has arrived.
            if self.rbuf.len() < want && !self.eof {
                let mut chunk = [0u8; 16384];
                let now = std::time::Instant::now();
                let deadline = if self.blocking { self.timeout.map(|t| now + t) } else { Some(now) };
                self.timed_out = false;
                match sk.read(&mut chunk, deadline) {
                    Ok(0) => self.eof = true,
                    Ok(n) => self.rbuf.extend_from_slice(&chunk[..n]),
                    Err(crate::sockets::TimedOut) => self.timed_out = self.blocking,
                }
            }
            return self.rbuf.len();
        }
        if let Backend::File(f) = &mut self.backend {
            while self.rbuf.len() < want {
                let mut chunk = vec![0u8; (want - self.rbuf.len()).max(8192)];
                match f.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => self.rbuf.extend_from_slice(&chunk[..n]),
                }
            }
        }
        self.rbuf.len()
    }

    fn read(&mut self, n: usize, it: &Interp) -> Vec<u8> {
        let out = match &mut self.backend {
            Backend::Memory(m) => {
                let end = (self.pos + n).min(m.len());
                let out = m[self.pos.min(end)..end].to_vec();
                self.pos = end;
                out
            }
            Backend::File(_) => {
                self.fill(n);
                self.rbuf.take(n)
            }
            Backend::Socket(_) => {
                if self.rbuf.is_empty() {
                    self.fill(1);
                }
                return self.rbuf.take(n);
            }
            Backend::Stdin => {
                it.flush();
                let mut buf = vec![0u8; n];
                let k = std::io::stdin().read(&mut buf).unwrap_or(0);
                buf.truncate(k);
                buf
            }
            _ => vec![],
        };
        if out.len() < n {
            self.eof = true;
        }
        out
    }

    fn read_line(&mut self, max: Option<usize>, it: &Interp) -> Option<Vec<u8>> {
        let mut out = vec![];
        if let Backend::File(_) | Backend::Socket(_) = self.backend {
            // Files and sockets: find the newline in the read-ahead buffer, a chunk at a
            // time.
            loop {
                let room = max.map_or(usize::MAX, |m| m.saturating_sub(out.len()));
                if room == 0 {
                    break;
                }
                if self.rbuf.is_empty() && self.fill(1) == 0 {
                    // A read hit the end: now feof() is true (not before).
                    if matches!(self.backend, Backend::File(_)) {
                        self.eof = true;
                    }
                    break;
                }
                if self.rbuf.is_empty() {
                    break;
                }
                let avail = &self.rbuf.bytes()[..self.rbuf.len().min(room)];
                match memchr::memchr(b'\n', avail) {
                    Some(i) => {
                        out.extend(self.rbuf.take(i + 1));
                        break;
                    }
                    None => {
                        let n = avail.len();
                        out.extend(self.rbuf.take(n));
                    }
                }
            }
            return (!out.is_empty()).then_some(out);
        }
        loop {
            if max.is_some_and(|m| out.len() >= m) {
                break;
            }
            let b = self.read(1, it);
            match b.first() {
                None => break,
                Some(&c) => {
                    out.push(c);
                    if c == b'\n' {
                        break;
                    }
                }
            }
        }
        // PHP reports EOF once a read hits the end.
        if matches!(self.backend, Backend::File(_)) && self.fill(1) == 0 {
            self.eof = true;
        }
        if let Backend::Memory(m) = &self.backend {
            if self.pos >= m.len() {
                self.eof = true;
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn write(&mut self, data: &[u8], it: &Interp) -> usize {
        match &mut self.backend {
            Backend::Memory(m) => {
                if self.append {
                    self.pos = m.len();
                }
                let end = self.pos + data.len();
                if end > m.len() {
                    m.resize(end, 0);
                }
                m[self.pos..end].copy_from_slice(data);
                self.pos = end;
                data.len()
            }
            Backend::File(f) => {
                // Drop read-ahead: the OS position is past it.
                if !self.rbuf.is_empty() {
                    let back = self.rbuf.len() as i64;
                    let _ = f.seek(SeekFrom::Current(-back));
                    self.rbuf.clear();
                }
                f.write_all(data).map(|_| data.len()).unwrap_or(0)
            }
            Backend::Stdout | Backend::Output => {
                it.echo(data);
                data.len()
            }
            Backend::Stderr => {
                it.flush();
                let _ = std::io::stderr().write_all(data);
                data.len()
            }
            Backend::Socket(sk) => {
                let deadline = self.timeout.map(|t| std::time::Instant::now() + t);
                match sk.write_all(data, deadline) {
                    Ok(()) => data.len(),
                    Err((code, msg)) => {
                        it.notice(&format!("fwrite(): Send of {} bytes failed with errno={code} {msg}", data.len()));
                        0
                    }
                }
            }
            Backend::Stdin => 0,
        }
    }

    fn tell(&mut self) -> i64 {
        match &mut self.backend {
            Backend::Memory(_) => self.pos as i64,
            Backend::Socket(sk) => sk.read_total as i64 - self.rbuf.len() as i64,
            Backend::File(f) => f.stream_position().map(|p| p as i64 - self.rbuf.len() as i64).unwrap_or(-1),
            _ => 0,
        }
    }

    fn seek(&mut self, off: i64, whence: i64) -> bool {
        self.eof = false;
        match &mut self.backend {
            Backend::Memory(m) => {
                let base = match whence {
                    1 => self.pos as i64,
                    2 => m.len() as i64,
                    _ => 0,
                };
                let p = base + off;
                if p < 0 {
                    return false;
                }
                self.pos = p as usize;
                true
            }
            Backend::File(f) => {
                let target = match whence {
                    1 => SeekFrom::Current(off - self.rbuf.len() as i64),
                    2 => SeekFrom::End(off),
                    _ => {
                        if off < 0 {
                            return false;
                        }
                        SeekFrom::Start(off as u64)
                    }
                };
                self.rbuf.clear();
                f.seek(target).is_ok()
            }
            _ => false,
        }
    }

    fn at_eof(&mut self) -> bool {
        match &self.backend {
            Backend::Memory(m) => self.eof || self.pos >= m.len() && self.eof,
            _ => self.eof,
        }
    }

    fn size(&self) -> Option<u64> {
        match &self.backend {
            Backend::Memory(m) => Some(m.len() as u64),
            Backend::File(f) => f.metadata().ok().map(|m| m.len()),
            _ => None,
        }
    }
}

/// fopen(): returns the stream or a PHP warning message.
pub fn open(it: &Interp, path: &str, mode: &str) -> Result<Value, String> {
    let m = mode.replace(['b', 't'], "");
    let lower = path.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("php://") {
        let backend = match rest {
            "stdin" => Backend::Stdin,
            "stdout" => Backend::Stdout,
            "stderr" => Backend::Stderr,
            "output" => Backend::Output,
            "input" => Backend::Memory(vec![]),
            r if r == "memory" || r.starts_with("temp") => Backend::Memory(vec![]),
            _ => return Err(format!("fopen(): Invalid php:// URL specified")),
        };
        let wrapper = if rest.starts_with("temp") { "TEMP" } else if rest == "memory" { "MEMORY" } else { "PHP" };
        return Ok(Resource::new("stream", Box::new(Stream::new(backend, mode, path, wrapper))));
    }
    let local = path.strip_prefix("file://").unwrap_or(path);
    if path.contains("://") && !path.starts_with("file://") {
        return Err(format!("fopen(): Unable to find the wrapper \"{}\"", path.split("://").next().unwrap_or("")));
    }
    let mut o = OpenOptions::new();
    let plus = m.contains('+');
    match m.chars().next().unwrap_or('r') {
        'r' => o.read(true).write(plus),
        'w' => o.write(true).create(true).truncate(true).read(plus),
        'a' => o.append(true).create(true).read(plus),
        'x' => o.write(true).create_new(true).read(plus),
        'c' => o.write(true).create(true).read(plus),
        _ => return Err(format!("fopen({path}): Failed to open stream: invalid mode")),
    };
    let _ = it;
    match o.open(local) {
        Ok(f) => {
            if std::fs::metadata(local).is_ok_and(|md| md.is_dir()) && !plus && m.starts_with('r') {
                // PHP opens directories for reading but every read fails; keep it simple.
            }
            Ok(Resource::new("stream", Box::new(Stream::new(Backend::File(f), mode, path, "plainfile"))))
        }
        Err(e) => Err(format!("fopen({path}): Failed to open stream: {}", io_msg(&e))),
    }
}

pub fn io_msg(e: &std::io::Error) -> &'static str {
    match e.kind() {
        std::io::ErrorKind::NotFound => "No such file or directory",
        std::io::ErrorKind::PermissionDenied => "Permission denied",
        std::io::ErrorKind::AlreadyExists => "File exists",
        _ if e.raw_os_error() == Some(21) => "Is a directory",
        _ => "Operation failed",
    }
}

/// STDIN / STDOUT / STDERR (resources 1..3).
pub fn std_constants(add: &mut dyn FnMut(&str, Value)) {
    add("STDIN", Resource::new("stream", Box::new(Stream::new(Backend::Stdin, "r", "php://stdin", "PHP"))));
    add("STDOUT", Resource::new("stream", Box::new(Stream::new(Backend::Stdout, "w", "php://stdout", "PHP"))));
    add("STDERR", Resource::new("stream", Box::new(Stream::new(Backend::Stderr, "w", "php://stderr", "PHP"))));
    for (k, v) in [
        ("SEEK_SET", 0),
        ("SEEK_CUR", 1),
        ("SEEK_END", 2),
        ("LOCK_SH", 1),
        ("LOCK_EX", 2),
        ("LOCK_UN", 3),
        ("LOCK_NB", 4),
        ("FILE_USE_INCLUDE_PATH", 1),
        ("FILE_IGNORE_NEW_LINES", 2),
        ("FILE_SKIP_EMPTY_LINES", 4),
        ("FILE_APPEND", 8),
        ("FILE_NO_DEFAULT_CONTEXT", 16),
        ("SCANDIR_SORT_ASCENDING", 0),
        ("SCANDIR_SORT_DESCENDING", 1),
        ("SCANDIR_SORT_NONE", 2),
        ("FNM_NOESCAPE", 2),
        ("FNM_PATHNAME", 1),
        ("FNM_PERIOD", 4),
        ("FNM_CASEFOLD", 16),
    ] {
        add(k, Value::Int(v));
    }
}

fn stat_array(md: &std::fs::Metadata) -> Value {
    let vals = [
        ("dev", md.dev() as i64),
        ("ino", md.ino() as i64),
        ("mode", md.mode() as i64),
        ("nlink", md.nlink() as i64),
        ("uid", md.uid() as i64),
        ("gid", md.gid() as i64),
        ("rdev", md.rdev() as i64),
        ("size", md.size() as i64),
        ("atime", md.atime()),
        ("mtime", md.mtime()),
        ("ctime", md.ctime()),
        ("blksize", md.blksize() as i64),
        ("blocks", md.blocks() as i64),
    ];
    let mut a = PhpArray::new();
    for (_, v) in vals {
        a.push(Value::Int(v));
    }
    for (k, v) in vals {
        a.insert(Key::Str(crate::value::pstr(k)), Value::Int(v));
    }
    Value::array(a)
}

fn stat_fail(it: &Interp, fname: &str, p: &str) -> Value {
    it.warn(&format!("{fname}(): {} failed for {p}", if fname == "lstat" { "Lstat" } else { "stat" }));
    Value::Bool(false)
}

/// `fnmatch()` (no FNM_ flags beyond the default).
pub fn fnmatch(pat: &[u8], s: &[u8]) -> bool {
    let (mut p, mut i) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while i < s.len() {
        if p < pat.len() {
            match pat[p] {
                b'*' => {
                    star = Some(p);
                    mark = i;
                    p += 1;
                    continue;
                }
                b'?' => {
                    p += 1;
                    i += 1;
                    continue;
                }
                b'[' => {
                    if let Some(end) = pat[p + 1..].iter().position(|&c| c == b']') {
                        let set = &pat[p + 1..p + 1 + end];
                        let (neg, set) = match set.first() {
                            Some(b'!' | b'^') => (true, &set[1..]),
                            _ => (false, set),
                        };
                        let mut hit = false;
                        let mut k = 0;
                        while k < set.len() {
                            if k + 2 < set.len() && set[k + 1] == b'-' {
                                hit |= set[k] <= s[i] && s[i] <= set[k + 2];
                                k += 3;
                            } else {
                                hit |= set[k] == s[i];
                                k += 1;
                            }
                        }
                        if hit != neg {
                            p += end + 2;
                            i += 1;
                            continue;
                        }
                    }
                }
                b'\\' if p + 1 < pat.len() && pat[p + 1] == s[i] => {
                    p += 2;
                    i += 1;
                    continue;
                }
                c if c == s[i] => {
                    p += 1;
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some(sp) => {
                p = sp + 1;
                mark += 1;
                i = mark;
            }
            None => return false,
        }
    }
    pat[p..].iter().all(|&c| c == b'*')
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("fopen", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        let mode = path_arg(it, a, 1)?;
        match open(it, &p, &mode) {
            Ok(v) => Ok(v),
            Err(msg) => {
                it.warn(&msg);
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("tmpfile", |it, _, _| {
        let _ = it;
        Ok(Resource::new("stream", Box::new(Stream::new(Backend::Memory(vec![]), "r+b", "php://temp", "TEMP"))))
    });
    t.insert("fclose", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "fclose", |_| ())?;
        if let Value::Resource(r) = v {
            r.close();
        }
        Ok(Value::Bool(true))
    });
    t.insert("fread", |it, _, a| {
        let n = int(it, a, 1)?;
        if n <= 0 {
            return Err(it.error("ValueError", "fread(): Argument #2 ($length) must be greater than 0"));
        }
        let v = arg(a, 0);
        with_stream(it, &v, "fread", |st| {
            let b = st.read(n as usize, it);
            // A socket read that timed out gives false.
            if b.is_empty() && st.timed_out {
                Value::Bool(false)
            } else {
                Value::bytes(b)
            }
        })
    });
    for name in ["fwrite", "fputs"] {
        t.insert(name, |it, _, a| {
            let mut data = s(it, a, 1)?.to_vec();
            if has(a, 2) && !arg(a, 2).is_null() {
                data.truncate(int(it, a, 2)?.max(0) as usize);
            }
            let v = arg(a, 0);
            with_stream(it, &v, "fwrite", |st| Value::Int(st.write(&data, it) as i64))
        });
    }
    t.insert("fgets", |it, _, a| {
        let max = if has(a, 1) && !arg(a, 1).is_null() { Some((int(it, a, 1)? - 1).max(0) as usize) } else { None };
        let v = arg(a, 0);
        with_stream(it, &v, "fgets", |st| st.read_line(max, it).map(Value::bytes).unwrap_or(Value::Bool(false)))
    });
    t.insert("fgetc", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "fgetc", |st| {
            let b = st.read(1, it);
            if b.is_empty() {
                Value::Bool(false)
            } else {
                Value::bytes(b)
            }
        })
    });
    t.insert("feof", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "feof", |st| Value::Bool(st.at_eof()))
    });
    t.insert("ftell", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "ftell", |st| Value::Int(st.tell()))
    });
    t.insert("fseek", |it, _, a| {
        let off = int(it, a, 1)?;
        let whence = if has(a, 2) { int(it, a, 2)? } else { 0 };
        let v = arg(a, 0);
        with_stream(it, &v, "fseek", |st| Value::Int(if st.seek(off, whence) { 0 } else { -1 }))
    });
    t.insert("rewind", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "rewind", |st| Value::Bool(st.seek(0, 0)))
    });
    t.insert("fflush", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "fflush", |st| {
            if let Backend::File(f) = &mut st.backend {
                let _ = f.flush();
            }
            Value::Bool(true)
        })
    });
    t.insert("ftruncate", |it, _, a| {
        let size = int(it, a, 1)?.max(0) as u64;
        let v = arg(a, 0);
        with_stream(it, &v, "ftruncate", |st| {
            Value::Bool(match &mut st.backend {
                Backend::Memory(m) => {
                    m.resize(size as usize, 0);
                    true
                }
                Backend::File(f) => {
                    st.rbuf.clear();
                    f.set_len(size).is_ok()
                }
                _ => false,
            })
        })
    });
    t.insert("flock", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "flock", |_| Value::Bool(true))
    });
    t.insert("fstat", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "fstat", |st| match &st.backend {
            Backend::File(f) => f.metadata().map(|m| stat_array(&m)).unwrap_or(Value::Bool(false)),
            _ => {
                let mut arr = PhpArray::new();
                arr.insert(Key::Str(crate::value::pstr("size")), Value::Int(st.size().unwrap_or(0) as i64));
                Value::array(arr)
            }
        })
    });
    t.insert("stream_get_contents", |it, _, a| {
        let max = if has(a, 1) && !arg(a, 1).is_null() { int(it, a, 1)? } else { -1 };
        let off = if has(a, 2) { int(it, a, 2)? } else { -1 };
        let v = arg(a, 0);
        with_stream(it, &v, "stream_get_contents", |st| {
            if off >= 0 {
                st.seek(off, 0);
            }
            let mut out = vec![];
            loop {
                let want = if max >= 0 { (max as usize).saturating_sub(out.len()) } else { 65536 };
                if want == 0 {
                    break;
                }
                let chunk = st.read(want, it);
                if chunk.is_empty() {
                    break;
                }
                out.extend_from_slice(&chunk);
            }
            Value::bytes(out)
        })
    });
    t.insert("fpassthru", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "fpassthru", |st| {
            let mut n = 0;
            loop {
                let chunk = st.read(65536, it);
                if chunk.is_empty() {
                    break;
                }
                n += chunk.len();
                it.echo(&chunk);
            }
            Value::Int(n as i64)
        })
    });
    t.insert("readfile", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        match std::fs::read(&p) {
            Ok(b) => {
                it.echo(&b);
                Ok(Value::Int(b.len() as i64))
            }
            Err(e) => {
                it.warn(&format!("readfile({p}): Failed to open stream: {}", io_msg(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("stream_get_meta_data", |it, _, a| {
        let v = arg(a, 0);
        with_stream(it, &v, "stream_get_meta_data", |st| {
            let mut m = PhpArray::new();
            let mut put = |k: &str, v: Value| {
                m.insert(Key::Str(crate::value::pstr(k)), v);
            };
            if let Backend::Socket(sk) = &st.backend {
                if let Some(c) = sk.crypto_meta() {
                    put("crypto", c);
                }
            }
            put("timed_out", Value::Bool(st.timed_out));
            put("blocked", Value::Bool(st.blocking));
            put("eof", Value::Bool(st.eof));
            if let Backend::Socket(sk) = &st.backend {
                put("stream_type", Value::str(sk.stream_type()));
                put("mode", Value::str(&st.mode));
                put("unread_bytes", Value::Int(st.rbuf.len() as i64));
                put("seekable", Value::Bool(false));
                if !st.uri.is_empty() {
                    put("uri", Value::str(&st.uri));
                }
                return Value::array(m);
            }
            put("wrapper_type", Value::str(if st.wrapper == "plainfile" { "plainfile" } else { "PHP" }));
            put("stream_type", Value::str(match st.wrapper {
                "plainfile" => "STDIO",
                "TEMP" => "TEMP",
                "MEMORY" => "MEMORY",
                _ => "STDIO",
            }));
            put("mode", Value::str(&st.mode));
            put("unread_bytes", Value::Int(0));
            put("seekable", Value::Bool(matches!(st.backend, Backend::File(_) | Backend::Memory(_))));
            put("uri", Value::str(&st.uri));
            Value::array(m)
        })
    });
    t.insert("is_resource", |_, _, a| Ok(Value::Bool(matches!(arg(a, 0), Value::Resource(r) if r.is_open()))));
    t.insert("get_resource_type", |it, _, a| match arg(a, 0) {
        Value::Resource(r) => Ok(Value::str(r.kind.get())),
        other => Err(it.error(
            "TypeError",
            &format!("get_resource_type(): Argument #1 ($resource) must be of type resource, {} given", other.debug_type()),
        )),
    });
    t.insert("get_resource_id", |_, _, a| Ok(match arg(a, 0) {
        Value::Resource(r) => Value::Int(r.id),
        _ => Value::Int(0),
    }));
    t.insert("stream_context_create", |_, _, a| {
        let opts = match arg(a, 0) {
            Value::Array(x) => Value::Array(x),
            _ => Value::array(PhpArray::new()),
        };
        Ok(Resource::new("stream-context", Box::new(std::cell::RefCell::new(opts))))
    });
    t.insert("stream_context_get_options", |_, _, a| Ok(match arg(a, 0) {
        Value::Resource(r) => r
            .data
            .borrow()
            .as_ref()
            .and_then(|b| b.downcast_ref::<std::cell::RefCell<Value>>())
            .map(|c| c.borrow().clone())
            .unwrap_or_else(|| Value::array(PhpArray::new())),
        _ => Value::array(PhpArray::new()),
    }));
    t.insert("stream_context_set_option", |_, _, _| Ok(Value::Bool(true)));
    t.insert("stream_context_get_default", |_, _, _| {
        Ok(Resource::new("stream-context", Box::new(std::cell::RefCell::new(Value::array(PhpArray::new())))))
    });
    t.insert("stream_set_write_buffer", |_, _, _| Ok(Value::Int(0)));
    t.insert("stream_is_local", |_, _, _| Ok(Value::Bool(true)));
    t.insert("stream_isatty", |_, _, _| Ok(Value::Bool(false)));
    t.insert("posix_isatty", |_, _, _| Ok(Value::Bool(false)));
    t.insert("stream_get_wrappers", |_, _, _| Ok(Value::array(PhpArray::from_list(["php", "file"].map(Value::str)))));

    // ---- directories -------------------------------------------------------------
    t.insert("opendir", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        match std::fs::read_dir(&p) {
            Ok(rd) => {
                let mut entries = vec![".".to_string(), "..".to_string()];
                entries.extend(rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()));
                Ok(Resource::new("stream", Box::new(Dir { entries, at: 0 })))
            }
            Err(e) => {
                it.warn(&format!("opendir({p}): Failed to open directory: {}", io_msg(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("readdir", |_, _, a| Ok(with_dir(&arg(a, 0), |d| {
        let r = d.entries.get(d.at).map(|e| Value::str(e)).unwrap_or(Value::Bool(false));
        d.at += 1;
        r
    })));
    t.insert("rewinddir", |_, _, a| Ok(with_dir(&arg(a, 0), |d| {
        d.at = 0;
        Value::Null
    })));
    t.insert("closedir", |_, _, a| {
        if let Value::Resource(r) = arg(a, 0) {
            r.close();
        }
        Ok(Value::Null)
    });

    // ---- stat & permissions --------------------------------------------------------
    t.insert("stat", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        Ok(std::fs::metadata(&p).map(|m| stat_array(&m)).unwrap_or_else(|_| stat_fail(it, "stat", &p)))
    });
    t.insert("lstat", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        Ok(std::fs::symlink_metadata(&p).map(|m| stat_array(&m)).unwrap_or_else(|_| stat_fail(it, "lstat", &p)))
    });
    fn meta_int(it: &Interp, a: &[Value], fname: &str, g: fn(&std::fs::Metadata) -> i64) -> R<Value> {
        let p = path_arg(it, a, 0)?;
        Ok(match std::fs::metadata(&p) {
            Ok(m) => Value::Int(g(&m)),
            Err(_) => {
                it.warn(&format!("{fname}(): stat failed for {p}"));
                Value::Bool(false)
            }
        })
    }
    t.insert("fileperms", |it, _, a| meta_int(it, a, "fileperms", |m| m.mode() as i64));
    t.insert("fileowner", |it, _, a| meta_int(it, a, "fileowner", |m| m.uid() as i64));
    t.insert("filegroup", |it, _, a| meta_int(it, a, "filegroup", |m| m.gid() as i64));
    t.insert("fileinode", |it, _, a| meta_int(it, a, "fileinode", |m| m.ino() as i64));
    t.insert("fileatime", |it, _, a| meta_int(it, a, "fileatime", |m| m.atime()));
    t.insert("filectime", |it, _, a| meta_int(it, a, "filectime", |m| m.ctime()));
    t.insert("filetype", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        Ok(match std::fs::symlink_metadata(&p) {
            Ok(m) if m.is_symlink() => Value::str("link"),
            Ok(m) if m.is_dir() => Value::str("dir"),
            Ok(_) => Value::str("file"),
            Err(_) => Value::Bool(false),
        })
    });
    t.insert("is_executable", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        Ok(Value::Bool(std::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.mode() & 0o111 != 0)))
    });
    t.insert("chmod", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        let mode = int(it, a, 1)? as u32;
        Ok(Value::Bool(std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).is_ok()))
    });
    for name in ["chown", "chgrp", "lchown", "lchgrp"] {
        t.insert(name, |_, _, _| Ok(Value::Bool(true)));
    }
    t.insert("umask", |_, _, _| Ok(Value::Int(0o022)));
    t.insert("disk_free_space", |_, _, _| Ok(Value::Float(1e12)));
    t.insert("disk_total_space", |_, _, _| Ok(Value::Float(2e12)));
    t.insert("is_uploaded_file", |_, _, _| Ok(Value::Bool(false)));
    t.insert("move_uploaded_file", |_, _, _| Ok(Value::Bool(false)));
    t.insert("link", |it, _, a| Ok(Value::Bool(std::fs::hard_link(path_arg(it, a, 0)?, path_arg(it, a, 1)?).is_ok())));
    t.insert("symlink", |it, _, a| Ok(Value::Bool(std::os::unix::fs::symlink(path_arg(it, a, 0)?, path_arg(it, a, 1)?).is_ok())));
    t.insert("readlink", |it, _, a| {
        let p = path_arg(it, a, 0)?;
        Ok(std::fs::read_link(&p).map(|t| Value::str(&t.to_string_lossy())).unwrap_or(Value::Bool(false)))
    });
    t.insert("fnmatch", |it, _, a| Ok(Value::Bool(fnmatch(&s(it, a, 0)?, &s(it, a, 1)?))));
    t.insert("clearstatcache", |_, _, _| Ok(Value::Null));
    t.insert("realpath_cache_size", |_, _, _| Ok(Value::Int(0)));
}

fn with_dir(v: &Value, g: impl FnOnce(&mut Dir) -> Value) -> Value {
    if let Value::Resource(r) = v {
        if let Some(d) = r.data.borrow_mut().as_mut().and_then(|b| b.downcast_mut::<Dir>()) {
            return g(d);
        }
    }
    Value::Bool(false)
}
