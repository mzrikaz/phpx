//! SPL's file and directory classes, natively. The classes themselves are
//! declared in prelude_splfs.php (properties, constructors, rarely used
//! methods); the methods here replace the ones walking a directory, which
//! frameworks call for every file they discover (Symfony Finder, Laravel's
//! Filesystem, Composer, Blade Icons).
//!
//! A directory iterator reads its entries once, with their type from the
//! directory itself (like PHP: RecursiveDirectoryIterator::hasChildren()
//! needs no stat for plain files and directories).

use std::rc::Rc;

use crate::class::Class;
use crate::interp::{Frame, Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, Value};

const CURRENT_AS_PATHNAME: i64 = 32;
const CURRENT_AS_SELF: i64 = 16;
const KEY_AS_FILENAME: i64 = 256;
const FOLLOW_SYMLINKS: i64 = 16384;
const SKIP_DOTS: i64 = 4096;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Dir,
    File,
    Link,
    Other,
}

#[derive(Clone)]
struct Entry {
    name: Rc<str>,
    kind: Kind,
}

/// A directory iterator's entries and position.
#[derive(Clone)]
struct DirState {
    dir: Rc<str>,
    entries: Vec<Entry>,
    pos: usize,
}

impl NativeData for DirState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        Some(Box::new(self.clone()))
    }
}

impl DirState {
    fn current(&self) -> Option<&Entry> {
        self.entries.get(self.pos)
    }

    fn pathname(&self) -> String {
        match self.current() {
            Some(e) => format!("{}/{}", self.dir, e.name),
            None => String::new(),
        }
    }
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("splfileinfo", "__phpx_setpath", fi_set_path),
        ("splfileinfo", "getpath", fi_get_path),
        ("splfileinfo", "getfilename", fi_get_filename),
        ("splfileinfo", "getextension", fi_get_extension),
        ("splfileinfo", "getbasename", fi_get_basename),
        ("splfileinfo", "getpathname", fi_get_pathname),
        ("splfileinfo", "getrealpath", fi_get_realpath),
        ("splfileinfo", "isdir", fi_is_dir),
        ("splfileinfo", "isfile", fi_is_file),
        ("splfileinfo", "islink", fi_is_link),
        ("directoryiterator", "__phpx_open", di_open),
        ("directoryiterator", "__phpx_set_entries", di_set_entries),
        ("directoryiterator", "getfilename", fi_get_filename),
        ("directoryiterator", "getpath", fi_get_path),
        ("directoryiterator", "getpathname", fi_get_pathname),
        ("directoryiterator", "isdot", di_is_dot),
        ("directoryiterator", "current", di_current),
        ("directoryiterator", "key", di_key),
        ("directoryiterator", "next", di_next),
        ("directoryiterator", "rewind", di_rewind),
        ("directoryiterator", "valid", di_valid),
        ("directoryiterator", "seek", di_seek),
        ("globiterator", "count", di_count),
        ("directoryiterator", "__tostring", fi_get_filename),
        ("filesystemiterator", "current", fsi_current),
        ("filesystemiterator", "key", fsi_key),
        ("recursivedirectoryiterator", "haschildren", rdi_has_children),
        ("recursivedirectoryiterator", "key", fsi_key),
    ]
}

fn this(o: Option<&Rc<Obj>>) -> &Rc<Obj> {
    o.expect("SPL file method needs an object")
}

fn with_state<T>(o: &Obj, f: impl FnOnce(&mut DirState) -> T) -> Option<T> {
    let mut i = o.internal_mut()?;
    match &mut *i {
        Internal::Data(d) => d.as_any_mut().downcast_mut::<DirState>().map(f),
        _ => None,
    }
}

fn str_prop(o: &Obj, name: &str) -> String {
    match o.get_prop(name) {
        Some(Value::Str(s)) => crate::pstr::lossy(&s).into_owned(),
        _ => String::new(),
    }
}

fn int_prop(o: &Obj, name: &str) -> i64 {
    match o.get_prop(name) {
        Some(Value::Int(n)) => n,
        _ => 0,
    }
}

/// `$o->getPathname()`, without a call unless a subclass overrides it.
fn pathname(it: &Interp, o: &Rc<Obj>) -> R<String> {
    if let Some(m) = o.class.find_method("getpathname") {
        if !matches!(&*m.declaring, "splfileinfo" | "directoryiterator") {
            let v = it.call_method(o, "getPathname", vec![])?;
            return Ok(crate::pstr::lossy(&it.to_str(&v)?).into_owned());
        }
    }
    Ok(with_state(o, |s| s.pathname()).unwrap_or_else(|| str_prop(o, "pathName")))
}

/// Path and file name as SplFileInfo stores them (trailing slashes go).
fn split_path(path: &str) -> (&str, &str) {
    let mut p = path;
    while p.len() > 1 && p.ends_with('/') {
        p = &p[..p.len() - 1];
    }
    let name = match p.rfind('/') {
        Some(i) => &p[i + 1..],
        None => p,
    };
    (p, name)
}

fn set_path(o: &Obj, path: &str) {
    let (p, name) = split_path(path);
    o.set_prop("pathName", Value::str(p));
    o.set_prop("fileName", Value::str(name));
}

fn fi_set_path(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let p = it.to_str(&a.first().cloned().unwrap_or_default())?;
    set_path(this(o), &crate::pstr::lossy(&p));
    Ok(Value::Null)
}

fn fi_get_path(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    if let Some(d) = with_state(o, |s| s.dir.clone()) {
        return Ok(Value::str(&d));
    }
    let p = pathname(it, o)?;
    Ok(Value::str(p.rfind('/').map_or("", |i| &p[..i])))
}

fn filename(it: &Interp, o: &Rc<Obj>) -> R<String> {
    if let Some(m) = o.class.find_method("getfilename") {
        if !matches!(&*m.declaring, "splfileinfo" | "directoryiterator") {
            let v = it.call_method(o, "getFilename", vec![])?;
            return Ok(crate::pstr::lossy(&it.to_str(&v)?).into_owned());
        }
    }
    Ok(match with_state(o, |s| s.current().map(|e| e.name.to_string()).unwrap_or_default()) {
        Some(n) => n,
        None => str_prop(o, "fileName"),
    })
}

fn fi_get_filename(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    Ok(match with_state(o, |s| s.current().map(|e| Value::str(&e.name)).unwrap_or_else(|| Value::str(""))) {
        Some(v) => v,
        None => o.get_prop("fileName").unwrap_or_else(|| Value::str("")),
    })
}

fn fi_get_extension(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let f = filename(it, this(o))?;
    Ok(Value::str(f.rfind('.').map_or("", |i| &f[i + 1..])))
}

fn fi_get_basename(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let f = filename(it, this(o))?;
    let mut args = vec![Value::str(&f)];
    args.extend(a.into_iter().take(1));
    let fun = it.builtins["basename"];
    crate::builtins::invoke(it, "basename", None, fun, &mut Frame::new(it.file_ctx(0), 0), &mut args)
}

fn fi_get_pathname(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    Ok(match with_state(o, |s| s.pathname()) {
        Some(p) => Value::str(&p),
        None => o.get_prop("pathName").map_or_else(|| Ok(Value::str("")), |v| Ok::<_, crate::interp::Ctrl>(Value::Str(it.to_str(&v)?)))?,
    })
}

fn fi_get_realpath(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let p = pathname(it, this(o))?;
    let p = if p.is_empty() { ".".to_string() } else { p };
    Ok(match crate::fs::realpath_cached(std::path::Path::new(&p)) {
        Some(r) => Value::str(&r.to_string_lossy()),
        None => Value::Bool(false),
    })
}

/// The current entry's type when the directory said (no stat needed).
fn known_kind(o: &Obj) -> Option<Kind> {
    with_state(o, |s| s.current().map(|e| e.kind)).flatten()
}

fn fi_is_dir(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    match known_kind(o) {
        Some(Kind::Dir) => Ok(Value::Bool(true)),
        Some(Kind::File) => Ok(Value::Bool(false)),
        _ => Ok(Value::Bool(std::fs::metadata(pathname(it, o)?).is_ok_and(|m| m.is_dir()))),
    }
}

fn fi_is_file(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    match known_kind(o) {
        Some(Kind::File) => Ok(Value::Bool(true)),
        Some(Kind::Dir) => Ok(Value::Bool(false)),
        _ => Ok(Value::Bool(std::fs::metadata(pathname(it, o)?).is_ok_and(|m| m.is_file()))),
    }
}

fn fi_is_link(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    match known_kind(o) {
        Some(Kind::Link) => Ok(Value::Bool(true)),
        Some(Kind::Dir | Kind::File) => Ok(Value::Bool(false)),
        _ => Ok(Value::Bool(std::fs::symlink_metadata(pathname(it, o)?).is_ok_and(|m| m.file_type().is_symlink()))),
    }
}

fn kind_of(t: std::io::Result<std::fs::FileType>) -> Kind {
    match t {
        Ok(t) if t.is_symlink() => Kind::Link,
        Ok(t) if t.is_dir() => Kind::Dir,
        Ok(t) if t.is_file() => Kind::File,
        _ => Kind::Other,
    }
}

/// Open `directory` (DirectoryIterator and subclasses' constructors).
fn di_open(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this(o);
    let directory = crate::pstr::lossy(&it.to_str(&a.first().cloned().unwrap_or_default())?).into_owned();
    let dir = {
        let mut d = directory.as_str();
        while d.len() > 1 && d.ends_with('/') {
            d = &d[..d.len() - 1];
        }
        d.to_string()
    };
    let rd = match std::fs::read_dir(&directory) {
        Ok(rd) => rd,
        Err(_) => {
            let why = if std::path::Path::new(&directory).exists() { "Not a directory" } else { "No such file or directory" };
            return Err(it.error("UnexpectedValueException", &format!("{}::__construct({directory}): Failed to open directory: {why}", o.class.name)));
        }
    };
    let skip_dots = o.class.is_a("filesystemiterator") && int_prop(o, "__phpx_flags") & SKIP_DOTS != 0;
    let mut entries = vec![];
    if !skip_dots {
        entries.push(Entry { name: Rc::from("."), kind: Kind::Dir });
        entries.push(Entry { name: Rc::from(".."), kind: Kind::Dir });
    }
    entries.extend(rd.flatten().map(|e| Entry { name: Rc::from(e.file_name().to_string_lossy().as_ref()), kind: kind_of(e.file_type()) }));
    start(o, DirState { dir: Rc::from(dir.as_str()), entries, pos: 0 });
    Ok(Value::Null)
}

/// GlobIterator: the matches of a pattern, in `dir`.
fn di_set_entries(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this(o);
    let dir = crate::pstr::lossy(&it.to_str(&a.first().cloned().unwrap_or_default())?).into_owned();
    let mut entries = vec![];
    if let Some(Value::Array(names)) = a.get(1).map(Value::deref) {
        for (_, n) in names.iter() {
            let name = crate::pstr::lossy(&it.to_str(&n)?).into_owned();
            let kind = kind_of(std::fs::symlink_metadata(format!("{dir}/{name}")).map(|m| m.file_type()));
            entries.push(Entry { name: Rc::from(name.as_str()), kind });
        }
    }
    start(o, DirState { dir: Rc::from(dir.as_str()), entries, pos: 0 });
    Ok(Value::Null)
}

fn start(o: &Obj, st: DirState) {
    let path = st.pathname();
    o.set_internal(Internal::Data(Box::new(st)));
    sync(o, &path);
}

/// The SplFileInfo properties follow the current entry (var_dump & co).
fn sync(o: &Obj, path: &str) {
    if path.is_empty() {
        o.set_prop("pathName", Value::str(""));
        o.set_prop("fileName", Value::str(""));
    } else {
        set_path(o, path);
    }
}

fn di_is_dot(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Bool(with_state(this(o), |s| s.current().is_some_and(|e| &*e.name == "." || &*e.name == "..")).unwrap_or(false)))
}

fn di_current(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Object(this(o).clone()))
}

fn di_key(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Int(with_state(this(o), |s| s.pos as i64).unwrap_or(0)))
}

fn di_next(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    if let Some(p) = with_state(o, |s| {
        s.pos += 1;
        s.pathname()
    }) {
        sync(o, &p);
    }
    Ok(Value::Null)
}

fn di_rewind(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    if let Some(p) = with_state(o, |s| {
        s.pos = 0;
        s.pathname()
    }) {
        sync(o, &p);
    }
    Ok(Value::Null)
}

fn di_valid(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Bool(with_state(this(o), |s| s.pos < s.entries.len()).unwrap_or(false)))
}

fn di_count(_: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    Ok(Value::Int(with_state(this(o), |s| s.entries.len() as i64).unwrap_or(0)))
}

fn di_seek(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this(o);
    let off = it.to_int(&a.first().cloned().unwrap_or_default())?;
    let len = with_state(o, |s| s.entries.len()).unwrap_or(0) as i64;
    if off < 0 || off >= len {
        return Err(it.error("OutOfBoundsException", &format!("Seek position {off} is out of range")));
    }
    if let Some(p) = with_state(o, |s| {
        s.pos = off as usize;
        s.pathname()
    }) {
        sync(o, &p);
    }
    Ok(Value::Null)
}

fn fsi_current(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    let flags = int_prop(o, "__phpx_flags");
    if flags & CURRENT_AS_PATHNAME != 0 {
        return Ok(Value::str(&pathname(it, o)?));
    }
    if flags & CURRENT_AS_SELF != 0 {
        return Ok(Value::Object(o.clone()));
    }
    // getFileInfo(): a new info object for the current path.
    let path = pathname(it, o)?;
    let class = str_prop(o, "__phpx_infoClass");
    let cls = if class.is_empty() { it.lookup_class("SplFileInfo")? } else { it.lookup_class(&class)? };
    if &*cls.lname == "splfileinfo" {
        let info = it.new_instance(&cls)?;
        set_path(&info, &path);
        return Ok(Value::Object(info));
    }
    let f = Frame::new(it.file_ctx(0), 0);
    Ok(Value::Object(it.instantiate(&f, &cls, vec![Value::str(&path)], vec![])?))
}

fn fsi_key(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, _: Vec<Value>) -> R<Value> {
    let o = this(o);
    if int_prop(o, "__phpx_flags") & KEY_AS_FILENAME != 0 {
        return Ok(Value::str(&filename(it, o)?));
    }
    Ok(Value::str(&pathname(it, o)?))
}

fn rdi_has_children(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = this(o);
    let allow_links = a.first().is_some_and(|v| v.truthy());
    let Some(Some(e)) = with_state(o, |s| s.current().cloned()) else { return Ok(Value::Bool(false)) };
    if &*e.name == "." || &*e.name == ".." {
        return Ok(Value::Bool(false));
    }
    match e.kind {
        Kind::Dir => return Ok(Value::Bool(true)),
        Kind::File => return Ok(Value::Bool(false)),
        _ => {}
    }
    let p = pathname(it, o)?;
    let Ok(m) = std::fs::symlink_metadata(&p) else { return Ok(Value::Bool(false)) };
    if !m.file_type().is_symlink() {
        return Ok(Value::Bool(m.is_dir()));
    }
    if !allow_links && int_prop(o, "__phpx_flags") & FOLLOW_SYMLINKS == 0 {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(std::fs::metadata(&p).is_ok_and(|m| m.is_dir())))
}
