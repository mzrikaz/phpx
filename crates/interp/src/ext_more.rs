//! hash_pbkdf2, hash_hkdf, file/stream hashing and mhash; preg_filter;
//! filter_var_array/filter_input_array; the rest of SPL's autoload
//! functions and spl_classes().

use std::cell::RefCell;
use std::collections::HashMap;

use crate::builtins::{arg, has, int, s};
use crate::ext::{digest_out, hmac, unknown_algo, Hasher};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Key, PhpArray, Value};

fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

fn digest_len(algo: &str) -> Option<usize> {
    let mut h = Hasher::new(algo)?;
    h.update(b"");
    Some(h.finish().len())
}

fn crypto_algo(it: &Interp, func: &str, algo: &str) -> R<usize> {
    match digest_len(algo).filter(|_| !algo.eq_ignore_ascii_case("crc32b")) {
        Some(n) => Ok(n),
        None => Err(it.error("ValueError", &format!("{func}(): Argument #1 ($algo) must be a valid cryptographic hashing algorithm"))),
    }
}

pub(crate) fn pbkdf2(algo: &str, pass: &[u8], salt: &[u8], iters: u64, want: usize) -> Vec<u8> {
    let mut out = vec![];
    let mut block = 1u32;
    while out.len() < want {
        let mut msg = salt.to_vec();
        msg.extend_from_slice(&block.to_be_bytes());
        let mut u = hmac(algo, pass, &msg).unwrap_or_default();
        let mut t = u.clone();
        for _ in 1..iters {
            u = hmac(algo, pass, &u).unwrap_or_default();
            for (x, y) in t.iter_mut().zip(&u) {
                *x ^= y;
            }
        }
        out.extend_from_slice(&t);
        block += 1;
    }
    out.truncate(want);
    out
}

// ---- mhash -------------------------------------------------------------------------

const MHASH_NAMES: &[(i64, &str, &str)] = &[
    (0, "CRC32", "crc32"),
    (1, "MD5", "md5"),
    (2, "SHA1", "sha1"),
    (3, "HAVAL256", "haval256,3"),
    (5, "RIPEMD160", "ripemd160"),
    (7, "TIGER", "tiger192,3"),
    (8, "GOST", "gost"),
    (9, "CRC32B", "crc32b"),
    (10, "HAVAL224", "haval224,3"),
    (11, "HAVAL192", "haval192,3"),
    (12, "HAVAL160", "haval160,3"),
    (13, "HAVAL128", "haval128,3"),
    (14, "TIGER128", "tiger128,3"),
    (15, "TIGER160", "tiger160,3"),
    (16, "MD4", "md4"),
    (17, "SHA256", "sha256"),
    (18, "ADLER32", "adler32"),
    (19, "SHA224", "sha224"),
    (20, "SHA512", "sha512"),
    (21, "SHA384", "sha384"),
    (22, "WHIRLPOOL", "whirlpool"),
    (23, "RIPEMD128", "ripemd128"),
    (24, "RIPEMD256", "ripemd256"),
    (25, "RIPEMD320", "ripemd320"),
    (27, "SNEFRU256", "snefru256"),
    (28, "MD2", "md2"),
    (29, "FNV132", "fnv132"),
    (30, "FNV1A32", "fnv1a32"),
    (31, "FNV164", "fnv164"),
    (32, "FNV1A64", "fnv1a64"),
    (33, "JOAAT", "joaat"),
    (34, "CRC32C", "crc32c"),
    (35, "MURMUR3A", "murmur3a"),
    (36, "MURMUR3C", "murmur3c"),
    (37, "MURMUR3F", "murmur3f"),
    (38, "XXH32", "xxh32"),
    (39, "XXH64", "xxh64"),
    (40, "XXH3", "xxh3"),
    (41, "XXH128", "xxh128"),
];

fn mhash_entry(id: i64) -> Option<&'static (i64, &'static str, &'static str)> {
    MHASH_NAMES.iter().find(|e| e.0 == id)
}

// ---- filter arrays -------------------------------------------------------------------

fn filter_array(it: &Interp, data: &PhpArray, def: &Value, add_empty: bool) -> R<Value> {
    let mut out = PhpArray::new();
    match def {
        Value::Array(d) => {
            for (k, spec) in d.iter() {
                let v = data.get(&k);
                let Some(v) = v else {
                    if add_empty {
                        out.insert(k, Value::Null);
                    }
                    continue;
                };
                let (filter, opts) = match &spec {
                    Value::Array(sa) => {
                        let f = sa.get(&Key::from_bytes(b"filter")).map(|x| it.to_int(&x)).transpose()?.unwrap_or(516);
                        (f, spec.clone())
                    }
                    other => (it.to_int(other)?, Value::Null),
                };
                out.insert(k, crate::misc::filter_value(it, &v, filter, &opts)?);
            }
        }
        other => {
            let filter = if other.is_null() { 516 } else { it.to_int(other)? };
            for (k, v) in data.iter() {
                out.insert(k, crate::misc::filter_value(it, &v, filter, &Value::Null)?);
            }
        }
    }
    Ok(Value::array(out))
}

thread_local! {
    static AUTOLOAD_EXTS: RefCell<String> = RefCell::new(".inc,.php".into());
}

pub fn reset() {
    AUTOLOAD_EXTS.with(|x| *x.borrow_mut() = ".inc,.php".to_string());
}

const SPL_CLASSES: &[&str] = &[
    "AppendIterator", "ArrayIterator", "ArrayObject", "BadFunctionCallException", "BadMethodCallException", "CachingIterator", "CallbackFilterIterator", "DirectoryIterator", "DomainException",
    "EmptyIterator", "FilesystemIterator", "FilterIterator", "GlobIterator", "InfiniteIterator", "InvalidArgumentException", "IteratorIterator", "LengthException", "LimitIterator", "LogicException",
    "MultipleIterator", "NoRewindIterator", "OuterIterator", "OutOfBoundsException", "OutOfRangeException", "OverflowException", "ParentIterator", "RangeException", "RecursiveArrayIterator",
    "RecursiveCachingIterator", "RecursiveCallbackFilterIterator", "RecursiveDirectoryIterator", "RecursiveFilterIterator", "RecursiveIterator", "RecursiveIteratorIterator", "RecursiveRegexIterator",
    "RecursiveTreeIterator", "RegexIterator", "RuntimeException", "SeekableIterator", "SplDoublyLinkedList", "SplFileInfo", "SplFileObject", "SplFixedArray", "SplHeap", "SplMinHeap", "SplMaxHeap",
    "SplObjectStorage", "SplObserver", "SplPriorityQueue", "SplQueue", "SplStack", "SplSubject", "SplTempFileObject", "UnderflowException", "UnexpectedValueException",
];

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("hash_pbkdf2", |it, _, a| {
        let algo = text(it, a, 0)?;
        let size = crypto_algo(it, "hash_pbkdf2", &algo)?;
        let (pass, salt) = (s(it, a, 1)?, s(it, a, 2)?);
        let iters = int(it, a, 3)?;
        let length = if has(a, 4) { int(it, a, 4)? } else { 0 };
        let binary = has(a, 5) && a[5].truthy();
        if iters <= 0 {
            return Err(it.error("ValueError", "hash_pbkdf2(): Argument #4 ($iterations) must be greater than 0"));
        }
        if length < 0 {
            return Err(it.error("ValueError", "hash_pbkdf2(): Argument #5 ($length) must be greater than or equal to 0"));
        }
        let length = length as usize;
        let want = match (length, binary) {
            (0, _) => size,
            (n, true) => n,
            (n, false) => n.div_ceil(2),
        };
        let raw = pbkdf2(&algo, &pass, &salt, iters as u64, want);
        if binary {
            return Ok(Value::bytes(raw));
        }
        let mut hex = crate::ext::hex(&raw);
        hex.truncate(if length == 0 { size * 2 } else { length });
        Ok(Value::str(&hex))
    });
    t.insert("hash_hkdf", |it, _, a| {
        let algo = text(it, a, 0)?;
        let size = crypto_algo(it, "hash_hkdf", &algo)?;
        let key = s(it, a, 1)?;
        let length = if has(a, 2) { int(it, a, 2)? } else { 0 };
        let info = if has(a, 3) { s(it, a, 3)?.to_vec() } else { vec![] };
        let salt = if has(a, 4) { s(it, a, 4)?.to_vec() } else { vec![] };
        if key.is_empty() {
            return Err(it.error("ValueError", "hash_hkdf(): Argument #2 ($key) must not be empty"));
        }
        if length < 0 {
            return Err(it.error("ValueError", "hash_hkdf(): Argument #3 ($length) must be greater than or equal to 0"));
        }
        if length as usize > size * 255 {
            return Err(it.error("ValueError", &format!("hash_hkdf(): Argument #3 ($length) must be less than or equal to {}", size * 255)));
        }
        let length = if length == 0 { size } else { length as usize };
        let salt = if salt.is_empty() { vec![0u8; size] } else { salt };
        let prk = hmac(&algo, &salt, &key).unwrap_or_default();
        let mut out = vec![];
        let mut prev: Vec<u8> = vec![];
        let mut i = 1u8;
        while out.len() < length {
            let mut msg = prev.clone();
            msg.extend_from_slice(&info);
            msg.push(i);
            prev = hmac(&algo, &prk, &msg).unwrap_or_default();
            out.extend_from_slice(&prev);
            i = i.wrapping_add(1);
        }
        out.truncate(length);
        Ok(Value::bytes(out))
    });
    t.insert("hash_hmac_file", |it, _, a| {
        let algo = text(it, a, 0)?;
        crypto_algo(it, "hash_hmac_file", &algo)?;
        let path = text(it, a, 1)?;
        let key = s(it, a, 2)?;
        match std::fs::read(&path) {
            Ok(d) => Ok(digest_out(hmac(&algo, &key, &d).unwrap_or_default(), has(a, 3) && a[3].truthy())),
            Err(e) => {
                it.warn(&format!("hash_hmac_file({path}): Failed to open stream: {}", crate::streams::io_msg(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("hash_update_stream", |it, f, a| {
        let limit = if has(a, 2) { int(it, a, 2)? } else { -1 };
        let mut total = 0i64;
        loop {
            let want = if limit < 0 { 8192 } else { (limit - total).min(8192) };
            if want <= 0 {
                break;
            }
            let chunk = (it.builtins["fread"])(it, f, &mut [arg(a, 1), Value::Int(want)])?;
            let b = match chunk {
                Value::Str(b) if !b.is_empty() => b,
                _ => break,
            };
            total += b.len() as i64;
            crate::ext::with_hash(it, &arg(a, 0), |st| st.h.update(&b))?;
        }
        Ok(Value::Int(total))
    });
    t.insert("hash_update_file", |it, _, a| {
        let path = text(it, a, 1)?;
        match std::fs::read(&path) {
            Ok(d) => {
                crate::ext::with_hash(it, &arg(a, 0), |st| st.h.update(&d))?;
                Ok(Value::Bool(true))
            }
            Err(e) => {
                it.warn(&format!("hash_update_file({path}): Failed to open stream: {}", crate::streams::io_msg(&e)));
                Ok(Value::Bool(false))
            }
        }
    });
    t.insert("mhash", |it, _, a| {
        it.deprecated("Function mhash() is deprecated since 8.1");
        let id = int(it, a, 0)?;
        let Some(e) = mhash_entry(id) else { return Ok(Value::Bool(false)) };
        let data = s(it, a, 1)?;
        if has(a, 2) && !a[2].is_null() {
            let key = s(it, a, 2)?;
            return Ok(hmac(e.2, &key, &data).map(Value::bytes).unwrap_or(Value::Bool(false)));
        }
        let Some(mut h) = Hasher::new(e.2) else { return Err(unknown_algo(it, "mhash", e.2)) };
        h.update(&data);
        Ok(Value::bytes(h.finish()))
    });
    t.insert("mhash_get_hash_name", |it, _, a| {
        it.deprecated("Function mhash_get_hash_name() is deprecated since 8.1");
        Ok(mhash_entry(int(it, a, 0)?).map(|e| Value::str(e.1)).unwrap_or(Value::Bool(false)))
    });
    t.insert("mhash_get_block_size", |it, _, a| {
        it.deprecated("Function mhash_get_block_size() is deprecated since 8.1");
        Ok(mhash_entry(int(it, a, 0)?).and_then(|e| digest_len(e.2)).map(|n| Value::Int(n as i64)).unwrap_or(Value::Bool(false)))
    });
    t.insert("mhash_count", |it, _, _| {
        it.deprecated("Function mhash_count() is deprecated since 8.1");
        Ok(Value::Int(41))
    });
    t.insert("mhash_keygen_s2k", |it, _, a| {
        it.deprecated("Function mhash_keygen_s2k() is deprecated since 8.1");
        let Some(e) = mhash_entry(int(it, a, 0)?) else { return Ok(Value::Bool(false)) };
        let (pass, salt) = (s(it, a, 1)?, s(it, a, 2)?);
        let len = int(it, a, 3)?.max(0) as usize;
        let mut salt8 = salt.to_vec();
        salt8.resize(8, 0);
        let mut out = vec![];
        let mut times = 0;
        while out.len() < len {
            let Some(mut h) = Hasher::new(e.2) else { return Ok(Value::Bool(false)) };
            h.update(&vec![0u8; times]);
            h.update(&salt8);
            h.update(&pass);
            out.extend_from_slice(&h.finish());
            times += 1;
        }
        out.truncate(len);
        Ok(Value::bytes(out))
    });
    t.insert("preg_filter", |it, f, a| {
        let subject = arg(a, 2);
        let limit = if has(a, 3) { arg(a, 3) } else { Value::Int(-1) };
        let mut total = 0i64;
        let mut one = |v: Value| -> R<Option<Value>> {
            let mut args = [arg(a, 0), arg(a, 1), v, limit.clone(), Value::Null];
            let r = (it.builtins["preg_replace"])(it, f, &mut args)?;
            let n = it.to_int(&args[4])?;
            total += n;
            Ok((n > 0).then_some(r))
        };
        let r = match subject {
            Value::Array(x) => {
                let mut out = PhpArray::new();
                for (k, v) in x.iter() {
                    if let Some(r) = one(v)? {
                        out.insert(k, r);
                    }
                }
                Value::array(out)
            }
            v => one(v)?.unwrap_or(Value::Null),
        };
        if a.len() > 4 {
            a[4] = Value::Int(total);
        }
        Ok(r)
    });
    t.insert("filter_var_array", |it, _, a| {
        let Value::Array(data) = arg(a, 0) else {
            return Err(it.error("TypeError", "filter_var_array(): Argument #1 ($array) must be of type array"));
        };
        let add_empty = !has(a, 2) || a[2].truthy();
        filter_array(it, &data, &arg(a, 1), add_empty)
    });
    t.insert("filter_input_array", |_, _, _| Ok(Value::Null));
    t.insert("spl_autoload_extensions", |it, _, a| {
        if let Some(v) = a.first().filter(|v| !v.is_null()) {
            let e = it.to_string_lossy(v)?;
            AUTOLOAD_EXTS.with(|x| *x.borrow_mut() = e);
        }
        Ok(AUTOLOAD_EXTS.with(|x| Value::str(&x.borrow())))
    });
    t.insert("spl_autoload", |it, f, a| {
        let class = text(it, a, 0)?;
        let exts = match a.get(1).filter(|v| !v.is_null()) {
            Some(v) => it.to_string_lossy(v)?,
            None => AUTOLOAD_EXTS.with(|x| x.borrow().clone()),
        };
        let base = class.to_ascii_lowercase().replace('\\', "/");
        let paths: Vec<String> = it.ini_get("include_path").unwrap_or_else(|| ".".into()).split(':').map(str::to_string).collect();
        for ext in exts.split(',') {
            for dir in &paths {
                let p = std::path::Path::new(dir).join(format!("{base}{ext}"));
                if p.is_file() {
                    let path = p.to_string_lossy().into_owned();
                    it.include(f, phpx_syntax::ast::IncludeKind::RequireOnce, &path)?;
                    return Ok(Value::Null);
                }
            }
        }
        Ok(Value::Null)
    });
    t.insert("spl_autoload_call", |it, _, a| {
        let class = text(it, a, 0)?;
        it.find_class(&class)?;
        Ok(Value::Null)
    });
    t.insert("spl_classes", |_, _, _| {
        let mut out = PhpArray::new();
        for c in SPL_CLASSES {
            out.insert(Key::from_bytes(c.as_bytes()), Value::str(c));
        }
        Ok(Value::array(out))
    });
}
