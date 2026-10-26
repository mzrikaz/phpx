//! Core functions about the running script: declared classes, included
//! files, mangled object properties, forward_static_call().

use std::collections::HashMap;
use std::rc::Rc;

use phpx_syntax::ast::ClassKind;

use crate::builtins::{arg, callable};
use crate::class::Class;
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Key, PhpArray, Value};

/// Declared classes of `kinds`, in declaration order (built-ins first).
fn declared(it: &Interp, kinds: &[ClassKind]) -> Value {
    let mut seen: Vec<Rc<Class>> = vec![];
    for c in it.classes.borrow().values() {
        if kinds.contains(&c.kind) && !c.name.starts_with("class@anonymous") && !seen.iter().any(|s| Rc::ptr_eq(s, c)) {
            seen.push(c.clone());
        }
    }
    seen.sort_by_key(|c| (c.decl_ctx.file, c.decl.span.start));
    Value::array(PhpArray::from_list(seen.iter().map(|c| Value::str(c.php_name())).collect::<Vec<_>>()))
}

fn included_files(it: &Interp) -> Value {
    let included = it.included.borrow();
    Value::array(PhpArray::from_list(included.iter().map(|p| Value::str(&p.to_string_lossy())).collect::<Vec<_>>()))
}

fn mangled_vars(it: &Interp, a: &[Value]) -> R<Value> {
    let Value::Object(o) = arg(a, 0) else {
        return Err(it.error("TypeError", &format!("get_mangled_object_vars(): Argument #1 ($object) must be of type object, {} given", arg(a, 0).debug_type())));
    };
    let mut out = PhpArray::new();
    for (k, v) in o.props.borrow().iter() {
        out.insert(Key::from_bytes(&crate::ext::mangled(&o, k)), v.clone());
    }
    Ok(Value::array(out))
}

/// Function-name prefixes of extensions (get_extension_funcs()).
fn ext_prefixes(ext: &str) -> Option<&'static [&'static str]> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "json" => &["json_"],
        "pcre" => &["preg_"],
        "ctype" => &["ctype_"],
        "mbstring" => &["mb_"],
        "hash" => &["hash", "mhash"],
        "date" => &["date", "time", "mktime", "gmmktime", "checkdate", "strtotime", "gmdate", "idate", "getdate", "localtime", "strftime", "gmstrftime"],
        "spl" => &["spl_", "iterator_", "class_implements", "class_parents", "class_uses"],
        "filter" => &["filter_"],
        "random" => &["random_", "mt_", "rand", "srand", "lcg_value", "getrandmax"],
        "pdo" => &["pdo_"],
        "zlib" => &["gz", "zlib_", "deflate_", "inflate_", "ob_gzhandler", "readgzfile"],
        "tokenizer" => &["token_"],
        "libxml" => &["libxml_"],
        "dom" => &["dom_"],
        "reflection" => &[],
        _ => return None,
    })
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("get_declared_classes", |it, _, _| Ok(declared(it, &[ClassKind::Class, ClassKind::Enum])));
    t.insert("get_declared_interfaces", |it, _, _| Ok(declared(it, &[ClassKind::Interface])));
    t.insert("get_declared_traits", |it, _, _| Ok(declared(it, &[ClassKind::Trait])));
    t.insert("get_included_files", |it, _, _| Ok(included_files(it)));
    t.insert("get_required_files", |it, _, _| Ok(included_files(it)));
    t.insert("get_mangled_object_vars", |it, _, a| mangled_vars(it, a));
    t.insert("zend_version", |_, _, _| Ok(Value::str("4.5.10")));
    t.insert("get_resources", |it, _, a| {
        let want = match a.first() {
            Some(v) if !v.is_null() => Some(it.to_string_lossy(v)?),
            _ => None,
        };
        let mut out = PhpArray::new();
        for name in ["STDIN", "STDOUT", "STDERR"] {
            if let Some(Value::Resource(r)) = it.constants.borrow().get(name).cloned() {
                if want.as_deref().is_none_or(|w| w == r.kind.get()) {
                    out.insert(Key::Int(r.id), Value::Resource(r));
                }
            }
        }
        Ok(Value::array(out))
    });
    t.insert("get_extension_funcs", |it, _, a| {
        let ext = it.to_string_lossy(&arg(a, 0))?;
        let loaded = crate::builtins::EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(&ext));
        let Some(prefixes) = ext_prefixes(&ext).filter(|_| loaded) else { return Ok(Value::Bool(false)) };
        let mut names: Vec<&str> = it.builtins.keys().copied().filter(|n| !n.starts_with("__phpx") && prefixes.iter().any(|p| n.starts_with(p))).collect();
        names.sort_unstable();
        if names.is_empty() {
            return Ok(Value::Bool(false));
        }
        Ok(Value::array(PhpArray::from_list(names.into_iter().map(Value::str).collect::<Vec<_>>())))
    });
    // PHP 8.4+ exposes exit/die and (8.5) clone as functions too.
    t.insert("clone", |it, _, a| {
        let Value::Object(o) = arg(a, 0) else {
            return Err(it.error("TypeError", &format!("clone(): Argument #1 ($object) must be of type object, {} given", arg(a, 0).debug_type())));
        };
        let c = it.clone_object(&o)?;
        if let Value::Array(with) = arg(a, 1) {
            for (k, v) in with.iter() {
                let name = String::from_utf8_lossy(&k.to_value().scalar_bytes().unwrap_or_default()).into_owned();
                c.set_prop(&name, v);
            }
        }
        Ok(Value::Object(c))
    });
    let exit: BuiltinFn = |it, _, a| {
        match a.first() {
            Some(Value::Int(n)) => Err(crate::interp::Ctrl::Exit(*n as i32)),
            Some(v) if !v.is_null() => {
                let msg = it.to_str(v)?;
                it.echo(&msg);
                Err(crate::interp::Ctrl::Exit(0))
            }
            _ => Err(crate::interp::Ctrl::Exit(0)),
        }
    };
    t.insert("exit", exit);
    t.insert("die", exit);
    t.insert("forward_static_call", |it, f, a| {
        let c = callable(it, f, a, 0)?;
        it.call_callable(&c, a[1..].to_vec(), vec![])
    });
    t.insert("forward_static_call_array", |it, f, a| {
        let c = callable(it, f, a, 0)?;
        let args: Vec<Value> = match arg(a, 1) {
            Value::Array(x) => x.values().collect(),
            _ => vec![],
        };
        it.call_callable(&c, args, vec![])
    });
}
