//! php.ini settings: defaults, php.ini files, `-d` overrides, and the
//! ini_* built-ins.
//!
//! Lookup (first found wins): `PHPRC` (a file, or a directory holding
//! php.ini), then `php.ini` in the script's directory or an ancestor up to
//! the project root (the nearest directory with composer.json). `-n` skips
//! files; `-d key=value` overrides them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::builtins::{arg, has, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{pstr, Key, PhpArray, Value};


#[derive(Default)]
pub struct Ini {
    /// What ini_set() changed this request.
    changed: HashMap<String, String>,
    /// Values at startup (after files and -d), for ini_restore(); shared
    /// by a thread's requests.
    startup: Rc<HashMap<String, String>>,
    pub loaded_file: Option<PathBuf>,
}

type Startup = (Rc<HashMap<String, String>>, Option<PathBuf>);

thread_local! {
    /// The built-in defaults, and what loading files gave per (script
    /// directory, -n, -d overrides): like PHP, php.ini is read once.
    static DEFAULTS: Rc<HashMap<String, String>> =
        Rc::new(crate::phpinfo_data::INI.iter().map(|d| (d.name.to_string(), d.value.unwrap_or("").to_string())).collect());
    static LOADED: std::cell::RefCell<HashMap<(Option<PathBuf>, bool, Vec<(String, String)>), Startup>> = Default::default();
}

/// Parse INI text into (section, key, value) triples, PHP style: `;`
/// comments, `[section]`, quoted values, bare booleans.
pub fn parse(text: &str) -> Vec<(Option<String>, String, String)> {
    let mut out = vec![];
    let mut section = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = Some(line[1..line.len() - 1].trim().to_string());
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let k = k.trim().to_string();
        let mut v = v.trim();
        let val = if (v.starts_with('"') && v.len() >= 2 && v[1..].contains('"')) || (v.starts_with('\'') && v.len() >= 2) {
            let q = v.chars().next().unwrap();
            let end = v[1..].find(q).map(|e| e + 1).unwrap_or(v.len());
            v[1..end].to_string()
        } else {
            if let Some(c) = v.find(" ;") {
                v = v[..c].trim();
            }
            match v.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" => "1".to_string(),
                "off" | "false" | "no" | "none" | "null" => String::new(),
                _ => v.to_string(),
            }
        };
        out.push((section.clone(), k, val));
    }
    out
}

/// phpx values are larger than PHP's (objects ~1.6x, see
/// docs/security-and-memory.md), so the limit is enforced at this multiple
/// of memory_limit; messages report the configured value. Goes back to 1 once
/// compact object layout lands.
pub const MEMORY_OVERHEAD: usize = 2;

fn find_ini(script: &Path) -> Option<PathBuf> {
    if let Ok(rc) = std::env::var("PHPRC") {
        let p = PathBuf::from(rc);
        let f = if p.is_dir() { p.join("php.ini") } else { p };
        return f.is_file().then_some(f);
    }
    let mut dir = script.parent().map(Path::to_path_buf);
    while let Some(d) = dir {
        let f = d.join("php.ini");
        if f.is_file() {
            return Some(f);
        }
        if d.join("composer.json").is_file() {
            return None;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

impl Ini {
    /// PHP 8.5's built-in defaults (`php -n`) for every directive of the
    /// modules phpx implements.
    pub fn new() -> Ini {
        Ini { changed: HashMap::new(), startup: DEFAULTS.with(|d| d.clone()), loaded_file: None }
    }

    /// Load php.ini for `script` (unless `no_files`) and apply overrides.
    pub fn load(&mut self, script: &Path, no_files: bool, overrides: &[(String, String)]) {
        let key = (script.parent().map(Path::to_path_buf), no_files, overrides.to_vec());
        if let Some((startup, file)) = LOADED.with(|l| l.borrow().get(&key).cloned()) {
            self.startup = startup;
            self.loaded_file = file;
            self.changed.clear();
            return;
        }
        let mut values = (*self.startup).clone();
        if !no_files {
            if let Some(f) = find_ini(script) {
                if let Ok(text) = std::fs::read_to_string(&f) {
                    for (_, k, v) in parse(&text) {
                        values.insert(k, v);
                    }
                    self.loaded_file = Some(f);
                }
            }
        }
        for (k, v) in overrides {
            values.insert(k.clone(), v.clone());
        }
        self.startup = Rc::new(values);
        self.changed.clear();
        LOADED.with(|l| l.borrow_mut().insert(key, (self.startup.clone(), self.loaded_file.clone())));
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.changed.get(k).or_else(|| self.startup.get(k)).map(String::as_str)
    }

    /// The value at startup (phpinfo()'s "Master Value").
    pub fn startup_value(&self, k: &str) -> Option<&str> {
        self.startup.get(k).map(String::as_str)
    }

    /// ini_set(): old value, or None for unknown keys (PHP returns false).
    pub fn set(&mut self, k: &str, v: &str) -> Option<String> {
        let old = self.get(k)?.to_string();
        self.changed.insert(k.to_string(), v.to_string());
        Some(old)
    }

    pub fn restore(&mut self, k: &str) {
        self.changed.remove(k);
    }

    pub fn all(&self) -> Vec<(String, String)> {
        let mut v: Vec<_> = self.startup.iter().filter(|(k, _)| !self.changed.contains_key(*k)).chain(&self.changed).map(|(k, v)| (k.clone(), v.clone())).collect();
        // zend_ini_sort_entries(): case-insensitive.
        v.sort_by_key(|(k, _)| k.to_ascii_lowercase());
        v
    }
}

impl Interp {
    pub fn ini_get(&self, k: &str) -> Option<String> {
        self.ini.borrow().get(k).map(str::to_string)
    }

    /// Apply side effects of a setting (startup or ini_set).
    pub fn ini_apply(&self, k: &str, v: &str) {
        match k {
            "memory_limit" => {
                if let Some(l) = crate::alloc::parse_quantity(v) {
                    crate::alloc::set_limit(l.map(|l| (l, l.saturating_mul(MEMORY_OVERHEAD))));
                }
            }
            "include_path" => *self.include_path.borrow_mut() = v.to_string(),
            "zend.enable_gc" => crate::gc::set_enabled(v == "1"),
            "max_execution_time" => crate::timeout::set(v.trim().parse().unwrap_or(0)),
            _ => {}
        }
    }

    /// After loading files: apply every setting with a side effect.
    pub fn ini_apply_all(&self) {
        for k in ["memory_limit", "include_path", "zend.enable_gc", "max_execution_time"] {
            if let Some(v) = self.ini_get(k) {
                self.ini_apply(k, &v);
            }
        }
    }
}

fn bstr(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

fn ini_value(v: &Value, it: &Interp) -> R<String> {
    Ok(match v {
        Value::Bool(true) => "1".into(),
        Value::Bool(false) | Value::Null => String::new(),
        v => it.to_string_lossy(v)?,
    })
}

fn parsed_array(text: &str, sections: bool) -> Value {
    let mut out = PhpArray::new();
    let put = |arr: &mut PhpArray, k: &str, v: &str| {
        let val = Value::str(v);
        // `key[] = v` and `key[sub] = v` build arrays, like PHP.
        if let Some(base) = k.strip_suffix("[]") {
            let key = Key::from_bytes(base.as_bytes());
            let mut inner = match arr.get(&key) {
                Some(Value::Array(a)) => (*a).clone(),
                _ => PhpArray::new(),
            };
            inner.push(val);
            arr.insert(key, Value::array(inner));
        } else if let (Some(open), true) = (k.find('['), k.ends_with(']')) {
            let key = Key::from_bytes(k[..open].as_bytes());
            let sub = Key::from_bytes(k[open + 1..k.len() - 1].as_bytes());
            let mut inner = match arr.get(&key) {
                Some(Value::Array(a)) => (*a).clone(),
                _ => PhpArray::new(),
            };
            inner.insert(sub, val);
            arr.insert(key, Value::array(inner));
        } else {
            arr.insert(Key::from_bytes(k.as_bytes()), val);
        }
    };
    for (sec, k, v) in parse(text) {
        match (sections, sec) {
            (true, Some(sname)) => {
                let skey = Key::Str(pstr(&sname));
                let mut inner = match out.get(&skey) {
                    Some(Value::Array(a)) => (*a).clone(),
                    _ => PhpArray::new(),
                };
                put(&mut inner, &k, &v);
                out.insert(skey, Value::array(inner));
            }
            _ => put(&mut out, &k, &v),
        }
    }
    Value::array(out)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("ini_get", |it, _, a| {
        let k = bstr(it, a, 0)?;
        Ok(it.ini_get(&k).map(|v| Value::str(&v)).unwrap_or(Value::Bool(false)))
    });
    t.insert("ini_set", |it, _, a| {
        let k = bstr(it, a, 0)?;
        let v = ini_value(&arg(a, 1), it)?;
        // Only PHP_INI_USER directives can change at run time.
        if crate::phpinfo::directive(&k).is_some_and(|d| d.access & 1 == 0) {
            return Ok(Value::Bool(false));
        }
        let old = it.ini.borrow_mut().set(&k, &v);
        Ok(match old {
            Some(o) => {
                it.ini_apply(&k, &v);
                Value::str(&o)
            }
            None => Value::Bool(false),
        })
    });
    t.insert("ini_alter", |it, f, a| (it.builtins["ini_set"])(it, f, a));
    t.insert("ini_restore", |it, _, a| {
        let k = bstr(it, a, 0)?;
        it.ini.borrow_mut().restore(&k);
        if let Some(v) = it.ini_get(&k) {
            it.ini_apply(&k, &v);
        }
        Ok(Value::Null)
    });
    t.insert("ini_get_all", |it, _, a| {
        // By extension, like PHP (not by name prefix).
        let module = if has(a, 0) && !arg(a, 0).is_null() {
            let ext = bstr(it, a, 0)?;
            match crate::phpinfo::module_name(&ext) {
                Some(m) => Some(m),
                None => {
                    it.warn(&format!("ini_get_all(): Extension \"{ext}\" cannot be found"));
                    return Ok(Value::Bool(false));
                }
            }
        } else {
            None
        };
        let details = !has(a, 1) || arg(a, 1).truthy();
        let mut out = PhpArray::new();
        let ini = it.ini.borrow();
        for (k, v) in ini.all() {
            let def = crate::phpinfo::directive(&k);
            if module.is_some_and(|m| def.is_none_or(|d| d.module != m)) {
                continue;
            }
            // A NULL default not set since shows as null.
            let shown = |s: &str| if s.is_empty() && def.is_some_and(|d| d.value.is_none()) { Value::Null } else { Value::str(s) };
            let val = if details {
                let mut d = PhpArray::new();
                d.insert(Key::Str(pstr("global_value")), shown(ini.startup.get(&k).map(String::as_str).unwrap_or("")));
                d.insert(Key::Str(pstr("local_value")), shown(&v));
                d.insert(Key::Str(pstr("access")), Value::Int(def.map_or(7, |d| d.access as i64)));
                Value::array(d)
            } else {
                shown(&v)
            };
            out.insert(Key::Str(pstr(&k)), val);
        }
        Ok(Value::array(out))
    });
    t.insert("get_cfg_var", |it, _, a| {
        let k = bstr(it, a, 0)?;
        if k == "cfg_file_path" {
            return Ok(Value::str(&it.ini.borrow().loaded_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default()));
        }
        Ok(it.ini.borrow().startup.get(&k).map(|v| Value::str(v)).unwrap_or(Value::Bool(false)))
    });
    t.insert("php_ini_loaded_file", |it, _, _| {
        Ok(it.ini.borrow().loaded_file.as_ref().map(|p| Value::str(&p.display().to_string())).unwrap_or(Value::Bool(false)))
    });
    t.insert("php_ini_scanned_files", |_, _, _| Ok(Value::Bool(false)));
    t.insert("set_time_limit", |it, _, a| {
        let v = it.to_int(&arg(a, 0))?;
        it.ini.borrow_mut().set("max_execution_time", &v.to_string());
        crate::timeout::set(v);
        Ok(Value::Bool(true))
    });
    t.insert("memory_get_usage", |_, _, _| Ok(Value::Int(crate::alloc::used() as i64)));
    t.insert("memory_get_peak_usage", |_, _, _| Ok(Value::Int(crate::alloc::peak() as i64)));
    t.insert("memory_reset_peak_usage", |_, _, _| Ok(Value::Null));
    t.insert("parse_ini_string", |it, _, a| {
        let text = bstr(it, a, 0)?;
        Ok(parsed_array(&text, arg(a, 1).truthy()))
    });
    t.insert("parse_ini_file", |it, _, a| {
        let p = bstr(it, a, 0)?;
        match std::fs::read_to_string(&p) {
            Ok(text) => Ok(parsed_array(&text, arg(a, 1).truthy())),
            Err(_) => Ok(Value::Bool(false)),
        }
    });
}

