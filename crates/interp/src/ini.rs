//! php.ini settings: defaults, php.ini files, `-d` overrides, and the
//! ini_* built-ins.
//!
//! Lookup (first found wins): `PHPRC` (a file, or a directory holding
//! php.ini), then `php.ini` in the script's directory or an ancestor up to
//! the project root (the nearest directory with composer.json). `-n` skips
//! files; `-d key=value` overrides them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::builtins::{arg, has, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{pstr, Key, PhpArray, Value};

/// PHP 8.5 CLI defaults without a php.ini.
const DEFAULTS: &[(&str, &str)] = &[
    ("allow_url_fopen", "1"),
    ("arg_separator.input", "&"),
    ("arg_separator.output", "&"),
    ("assert.active", "1"),
    ("assert.exception", "1"),
    ("auto_detect_line_endings", "0"),
    ("date.timezone", "UTC"),
    ("default_charset", "UTF-8"),
    ("default_mimetype", "text/html"),
    ("default_socket_timeout", "60"),
    ("disable_classes", ""),
    ("disable_functions", ""),
    ("display_errors", "1"),
    ("display_startup_errors", "1"),
    ("error_log", ""),
    ("error_reporting", "30719"),
    ("expose_php", "1"),
    ("file_uploads", "1"),
    ("html_errors", "0"),
    ("ignore_user_abort", "0"),
    ("implicit_flush", "1"),
    ("include_path", ".:/usr/share/php"),
    ("log_errors", "0"),
    ("max_execution_time", "0"),
    ("max_file_uploads", "20"),
    ("max_input_nesting_level", "64"),
    ("max_input_time", "-1"),
    ("max_input_vars", "1000"),
    ("mbstring.language", "neutral"),
    ("memory_limit", "128M"),
    ("open_basedir", ""),
    ("output_buffering", "0"),
    ("output_handler", ""),
    ("pcre.backtrack_limit", "1000000"),
    ("pcre.jit", "1"),
    ("pcre.recursion_limit", "100000"),
    ("post_max_size", "8M"),
    ("precision", "14"),
    ("register_argc_argv", "1"),
    ("request_order", ""),
    ("serialize_precision", "-1"),
    ("session.auto_start", "0"),
    ("session.cache_expire", "180"),
    ("session.cache_limiter", "nocache"),
    ("session.cookie_domain", ""),
    ("session.cookie_httponly", "0"),
    ("session.cookie_lifetime", "0"),
    ("session.cookie_path", "/"),
    ("session.cookie_samesite", ""),
    ("session.cookie_secure", "0"),
    ("session.gc_maxlifetime", "1440"),
    ("session.name", "PHPSESSID"),
    ("session.save_handler", "files"),
    ("session.save_path", ""),
    ("session.serialize_handler", "php"),
    ("session.use_cookies", "1"),
    ("session.use_only_cookies", "1"),
    ("session.use_strict_mode", "0"),
    ("short_open_tag", "1"),
    ("sys_temp_dir", ""),
    ("unserialize_callback_func", ""),
    ("unserialize_max_depth", "4096"),
    ("upload_max_filesize", "2M"),
    ("upload_tmp_dir", ""),
    ("user_agent", ""),
    ("variables_order", "EGPCS"),
    ("zend.assertions", "1"),
    ("zend.enable_gc", "1"),
    ("zend.exception_ignore_args", "0"),
    ("zlib.output_compression", "0"),
];

#[derive(Default)]
pub struct Ini {
    values: HashMap<String, String>,
    /// Values at startup (after files and -d), for ini_restore().
    startup: HashMap<String, String>,
    pub loaded_file: Option<PathBuf>,
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
    pub fn new() -> Ini {
        let values: HashMap<String, String> = DEFAULTS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Ini { startup: values.clone(), values, loaded_file: None }
    }

    /// Load php.ini for `script` (unless `no_files`) and apply overrides.
    pub fn load(&mut self, script: &Path, no_files: bool, overrides: &[(String, String)]) {
        if !no_files {
            if let Some(f) = find_ini(script) {
                if let Ok(text) = std::fs::read_to_string(&f) {
                    for (_, k, v) in parse(&text) {
                        self.values.insert(k, v);
                    }
                    self.loaded_file = Some(f);
                }
            }
        }
        for (k, v) in overrides {
            self.values.insert(k.clone(), v.clone());
        }
        self.startup = self.values.clone();
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.values.get(k).map(String::as_str)
    }

    /// The value at startup (phpinfo()'s "Master Value").
    pub fn startup_value(&self, k: &str) -> Option<&str> {
        self.startup.get(k).map(String::as_str)
    }

    /// ini_set(): old value, or None for unknown keys (PHP returns false).
    pub fn set(&mut self, k: &str, v: &str) -> Option<String> {
        let old = self.values.get(k)?.clone();
        self.values.insert(k.to_string(), v.to_string());
        Some(old)
    }

    pub fn restore(&mut self, k: &str) {
        if let Some(v) = self.startup.get(k).cloned() {
            self.values.insert(k.to_string(), v);
        }
    }

    pub fn all(&self) -> Vec<(String, String)> {
        let mut v: Vec<_> = self.values.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        v.sort();
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
        let prefix = if has(a, 0) && !arg(a, 0).is_null() { Some(format!("{}.", bstr(it, a, 0)?.to_ascii_lowercase())) } else { None };
        let details = !has(a, 1) || arg(a, 1).truthy();
        let mut out = PhpArray::new();
        let ini = it.ini.borrow();
        for (k, v) in ini.all() {
            if prefix.as_ref().is_some_and(|p| !k.starts_with(p.as_str())) {
                continue;
            }
            let val = if details {
                let mut d = PhpArray::new();
                d.insert(Key::Str(pstr("global_value")), Value::str(ini.startup.get(&k).map(String::as_str).unwrap_or("")));
                d.insert(Key::Str(pstr("local_value")), Value::str(&v));
                d.insert(Key::Str(pstr("access")), Value::Int(7));
                Value::array(d)
            } else {
                Value::str(&v)
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

