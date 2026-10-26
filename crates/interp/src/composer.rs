//! Native Composer support (opt-in: PHPX_NATIVE_COMPOSER=1). `require
//! 'vendor/autoload.php'` loads Composer's generated maps (plain PHP files
//! returning arrays) and registers a built-in autoloader instead of running
//! Composer's bootstrap; there is no ClassLoader object then.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::interp::{Callable, Frame, Interp, R};
use crate::value::{Key, Value};

#[derive(Default)]
pub struct Composer {
    classmap: HashMap<String, PathBuf>,
    /// PSR-4 prefixes, longest first: ("App\\", [dirs]).
    psr4: Vec<(String, Vec<PathBuf>)>,
    /// PSR-0 prefixes.
    psr0: Vec<(String, Vec<PathBuf>)>,
}

impl Interp {
    /// Handle `require .../vendor/autoload.php`.
    pub fn composer_boot(&self, f: &mut Frame, autoload_php: &Path) -> R<Value> {
        let dir = autoload_php.parent().unwrap().join("composer");
        let load = |it: &Interp, f: &mut Frame, name: &str| -> R<Option<Value>> {
            let p = dir.join(name);
            if !p.is_file() {
                return Ok(None);
            }
            it.include(f, phpx_syntax::ast::IncludeKind::Require, &p.display().to_string()).map(Some)
        };
        let mut c = Composer::default();
        if let Some(Value::Array(m)) = load(self, f, "autoload_classmap.php")? {
            for (k, v) in m.iter() {
                c.classmap.insert(key_string(&k).to_ascii_lowercase(), PathBuf::from(self.to_string_lossy(&v)?));
            }
        }
        for (file, target) in [("autoload_psr4.php", &mut c.psr4), ("autoload_namespaces.php", &mut c.psr0)] {
            if let Some(Value::Array(m)) = load(self, f, file)? {
                for (k, v) in m.iter() {
                    let dirs = match v {
                        Value::Array(d) => d.values().map(|x| self.to_string_lossy(&x).map(PathBuf::from)).collect::<R<Vec<_>>>()?,
                        other => vec![PathBuf::from(self.to_string_lossy(&other)?)],
                    };
                    target.push((key_string(&k), dirs));
                }
                target.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
            }
        }
        *self.composer.borrow_mut() = Some(c);
        let loader = Callable::Builtin("__phpx_composer_autoload", composer_autoload);
        self.autoloaders.borrow_mut().push(loader);
        // Eagerly included files (helpers.php & co), in Composer's order.
        if let Some(Value::Array(files)) = load(self, f, "autoload_files.php")? {
            for v in files.values() {
                let p = self.to_string_lossy(&v)?;
                self.include(f, phpx_syntax::ast::IncludeKind::RequireOnce, &p)?;
            }
        }
        Ok(Value::Bool(true))
    }

    fn composer_find(&self, class: &str) -> Option<PathBuf> {
        let c = self.composer.borrow();
        let c = c.as_ref()?;
        if let Some(p) = c.classmap.get(&class.to_ascii_lowercase()) {
            return Some(p.clone());
        }
        for (prefix, dirs) in &c.psr4 {
            if let Some(rest) = class.strip_prefix(prefix.as_str()) {
                let rel = format!("{}.php", rest.replace('\\', "/"));
                if let Some(p) = dirs.iter().map(|d| d.join(&rel)).find(|p| p.is_file()) {
                    return Some(p);
                }
            }
        }
        for (prefix, dirs) in &c.psr0 {
            if class.starts_with(prefix.as_str()) {
                let rel = format!("{}.php", class.replace(['\\', '_'], "/"));
                if let Some(p) = dirs.iter().map(|d| d.join(&rel)).find(|p| p.is_file()) {
                    return Some(p);
                }
            }
        }
        None
    }
}

fn key_string(k: &Key) -> String {
    match k {
        Key::Int(n) => n.to_string(),
        Key::Str(s) => String::from_utf8_lossy(s).into_owned(),
    }
}

fn composer_autoload(it: &Interp, f: &mut Frame, a: &mut [Value]) -> R<Value> {
    let class = it.to_string_lossy(&a.first().cloned().unwrap_or_default())?;
    if let Some(path) = it.composer_find(class.trim_start_matches('\\')) {
        it.include(f, phpx_syntax::ast::IncludeKind::RequireOnce, &path.display().to_string())?;
        return Ok(Value::Bool(true));
    }
    Ok(Value::Bool(false))
}
