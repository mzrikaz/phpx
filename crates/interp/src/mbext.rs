//! The rest of mbstring: case-insensitive/last-occurrence searches,
//! mb_strcut, MIME headers, numeric entities, settings, and the mb_ereg
//! family (Oniguruma syntax mapped onto PCRE2 in UTF mode).

use std::cell::RefCell;
use std::collections::HashMap;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{pstr, Key, PhpArray, Value};

/// mbstring settings of one request (server workers).
pub fn reset() {
    SUBST.with(|s| *s.borrow_mut() = Value::Int(63));
    LANG.with(|l| *l.borrow_mut() = "neutral".into());
    SEARCH.with(|s| *s.borrow_mut() = Default::default());
}

fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

fn lower(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

/// Char offset of byte index `b` in `s`.
fn char_index(s: &str, b: usize) -> i64 {
    s[..b].chars().count() as i64
}

/// Byte index of char offset `c` (negative counts from the end).
fn byte_index(s: &str, c: i64) -> Option<usize> {
    let n = s.chars().count() as i64;
    let c = if c < 0 { n + c } else { c };
    if c < 0 || c > n {
        return None;
    }
    Some(s.char_indices().nth(c as usize).map_or(s.len(), |(i, _)| i))
}

/// Last occurrence of `needle` in `hay` at or after char `offset`
/// (negative: search stops that many chars from the end).
fn rfind_chars(hay: &str, needle: &str, offset: i64) -> Option<usize> {
    let n = hay.chars().count() as i64;
    let (from, to) = if offset >= 0 { (byte_index(hay, offset)?, hay.len()) } else { (0, byte_index(hay, (n + offset + needle.chars().count() as i64).min(n))?) };
    hay[from..to].rfind(needle).map(|i| i + from)
}

thread_local! {
    static SUBST: RefCell<Value> = RefCell::new(Value::Int(63));
    static LANG: RefCell<String> = RefCell::new("neutral".into());
    static SEARCH: RefCell<(String, String, usize, Vec<Value>)> = RefCell::new(Default::default());
}

/// `/pattern/u` (plus `i` / `x` / `m` options) for the ereg family.
fn pcre(pattern: &str, opts: &str) -> Vec<u8> {
    let mut p = Vec::with_capacity(pattern.len() + 4);
    p.push(1u8);
    p.extend_from_slice(pattern.as_bytes());
    p.push(1);
    p.push(b'u');
    for c in opts.chars() {
        match c {
            'i' => p.push(b'i'),
            'x' => p.push(b'x'),
            'm' => p.push(b's'),
            _ => {}
        }
    }
    p
}

fn call(it: &Interp, f: &mut crate::interp::Frame, name: &str, args: &mut [Value]) -> R<Value> {
    (it.builtins[name])(it, f, args)
}

fn ereg(it: &Interp, f: &mut crate::interp::Frame, a: &mut [Value], opts: &str, fname: &str) -> R<Value> {
    let pat = text(it, a, 0)?;
    if pat.is_empty() {
        return Err(it.error("ValueError", &format!("{fname}(): Argument #1 ($pattern) must not be empty")));
    }
    let mut args = [Value::bytes(pcre(&pat, opts)), arg(a, 1), Value::Null];
    let r = call(it, f, "preg_match", &mut args)?;
    let found = matches!(r, Value::Int(1));
    if a.len() > 2 {
        a[2] = if found {
            // Unmatched groups are false, as Oniguruma reports them.
            let Value::Array(m) = std::mem::take(&mut args[2]) else { unreachable!() };
            let mut out = PhpArray::new();
            for (k, v) in m.iter() {
                if let Key::Int(_) = k {
                    out.insert(k, if matches!(&v, Value::Str(s) if s.is_empty()) { Value::Bool(false) } else { v });
                }
            }
            Value::array(out)
        } else {
            Value::array(PhpArray::new())
        };
    }
    Ok(Value::Bool(found))
}

fn ereg_replace(it: &Interp, f: &mut crate::interp::Frame, a: &mut [Value], icase: bool) -> R<Value> {
    let pat = text(it, a, 0)?;
    let opts = if has(a, 3) && !arg(a, 3).is_null() { text(it, a, 3)? } else { String::new() };
    let opts = if icase { format!("{opts}i") } else { opts };
    let mut args = [Value::bytes(pcre(&pat, &opts)), arg(a, 1), arg(a, 2)];
    let r = call(it, f, "preg_replace", &mut args)?;
    Ok(if r.is_null() { Value::Bool(false) } else { r })
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("mb_language", |it, _, a| {
        if !has(a, 0) || arg(a, 0).is_null() {
            return Ok(Value::str(&LANG.with(|l| l.borrow().clone())));
        }
        let l = text(it, a, 0)?;
        let canon = match l.to_ascii_lowercase().as_str() {
            "neutral" | "uni" | "universal" => "neutral",
            "en" | "english" => "English",
            "ja" | "japanese" => "Japanese",
            "de" | "german" => "German",
            _ => return Err(it.error("ValueError", &format!("mb_language(): Argument #1 ($language) must be a valid language, \"{l}\" given"))),
        };
        LANG.with(|x| *x.borrow_mut() = canon.into());
        Ok(Value::Bool(true))
    });
    t.insert("mb_http_input", |_, _, _| Ok(Value::Bool(false)));
    t.insert("mb_http_output", |it, _, a| Ok(if has(a, 0) && !arg(a, 0).is_null() { let _ = text(it, a, 0)?; Value::Bool(true) } else { Value::str("UTF-8") }));
    t.insert("mb_detect_order", |_, _, a| Ok(if has(a, 0) && !arg(a, 0).is_null() { Value::Bool(true) } else { Value::array(PhpArray::from_list([Value::str("ASCII"), Value::str("UTF-8")])) }));
    t.insert("mb_substitute_character", |_, _, a| {
        if !has(a, 0) || arg(a, 0).is_null() {
            return Ok(SUBST.with(|x| x.borrow().clone()));
        }
        SUBST.with(|x| *x.borrow_mut() = arg(a, 0));
        Ok(Value::Bool(true))
    });
    t.insert("mb_preferred_mime_name", |it, _, a| {
        let e = text(it, a, 0)?;
        Ok(Value::str(match e.to_ascii_lowercase().as_str() {
            "utf8" | "utf-8" => "UTF-8",
            "sjis" | "shift_jis" => "Shift_JIS",
            "eucjp" | "euc-jp" => "EUC-JP",
            "iso-8859-1" | "latin1" => "ISO-8859-1",
            "ascii" => "US-ASCII",
            _ => return Ok(Value::Bool(false)),
        }))
    });
    t.insert("mb_parse_str", |it, f, a| {
        let mut args = [arg(a, 0), Value::Null];
        (it.builtins["parse_str"])(it, f, &mut args)?;
        if a.len() > 1 {
            a[1] = std::mem::take(&mut args[1]);
        }
        Ok(Value::Bool(true))
    });
    t.insert("mb_output_handler", |_, _, a| Ok(arg(a, 0)));
    t.insert("mb_strripos", |it, _, a| {
        let (h, n) = (lower(&text(it, a, 0)?), lower(&text(it, a, 1)?));
        let off = if has(a, 2) { int(it, a, 2)? } else { 0 };
        Ok(rfind_chars(&h, &n, off).map(|b| Value::Int(char_index(&h, b))).unwrap_or(Value::Bool(false)))
    });
    for (name, last, icase) in [("mb_strrchr", true, false), ("mb_stristr", false, true), ("mb_strrichr", true, true)] {
        let f: BuiltinFn = match (last, icase) {
            (true, false) => |it, _, a| part(it, a, true, false),
            (false, true) => |it, _, a| part(it, a, false, true),
            _ => |it, _, a| part(it, a, true, true),
        };
        t.insert(name, f);
    }
    t.insert("mb_strcut", |it, _, a| {
        let b = s(it, a, 0)?;
        let len = b.len() as i64;
        let mut start = int(it, a, 1)?;
        if start < 0 {
            start = (len + start).max(0);
        }
        if start > len {
            return Ok(Value::str(""));
        }
        // The start moves back to a character boundary, then the length
        // counts from there; the end also moves back to a boundary.
        let mut st = start as usize;
        while st > 0 && st < b.len() && (b[st] & 0xC0) == 0x80 {
            st -= 1;
        }
        let end = if has(a, 2) && !arg(a, 2).is_null() {
            let l = int(it, a, 2)?;
            if l < 0 { (len + l).max(st as i64) } else { (st as i64 + l).min(len) }
        } else {
            len
        };
        let mut en = end as usize;
        while en > st && en < b.len() && (b[en] & 0xC0) == 0x80 {
            en -= 1;
        }
        Ok(Value::bytes(b[st..en].to_vec()))
    });
    t.insert("mb_list_encodings", |_, _, _| {
        let list = ["BASE64", "UUENCODE", "HTML-ENTITIES", "Quoted-Printable", "7bit", "8bit", "UCS-4", "UCS-4BE", "UCS-4LE", "UCS-2", "UCS-2BE", "UCS-2LE", "UTF-32", "UTF-32BE", "UTF-32LE", "UTF-16", "UTF-16BE", "UTF-16LE", "UTF-8", "UTF-7", "UTF7-IMAP", "ASCII", "EUC-JP", "SJIS", "eucJP-win", "EUC-JP-2004", "SJIS-win", "JIS", "ISO-2022-JP", "Windows-1252", "Windows-1254", "ISO-8859-1", "ISO-8859-2", "ISO-8859-3", "ISO-8859-4", "ISO-8859-5", "ISO-8859-6", "ISO-8859-7", "ISO-8859-8", "ISO-8859-9", "ISO-8859-10", "ISO-8859-13", "ISO-8859-14", "ISO-8859-15", "ISO-8859-16", "EUC-CN", "CP936", "GB18030", "HZ", "EUC-TW", "BIG-5", "CP950", "EUC-KR", "UHC", "ISO-2022-KR", "Windows-1251", "CP866", "KOI8-R", "KOI8-U", "ArmSCII-8", "CP850"];
        Ok(Value::array(PhpArray::from_list(list.iter().map(|e| Value::str(e)))))
    });
    t.insert("mb_encoding_aliases", |it, _, a| {
        let e = text(it, a, 0)?;
        let aliases: &[&str] = match e.to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" => &["utf8"],
            "ascii" => &["ANSI_X3.4-1968", "iso-ir-6", "ANSI_X3.4-1986", "ISO_646.irv:1991", "US-ASCII", "ISO646-US", "us", "IBM367", "IBM-367", "cp367", "csASCII"],
            "iso-8859-1" => &["ISO_8859-1", "latin1"],
            _ => return Err(it.error("ValueError", &format!("mb_encoding_aliases(): Argument #1 ($encoding) must be a valid encoding, \"{e}\" given"))),
        };
        Ok(Value::array(PhpArray::from_list(aliases.iter().map(|x| Value::str(x)))))
    });
    t.insert("mb_encode_mimeheader", |it, _, a| {
        let st = text(it, a, 0)?;
        if st.is_ascii() {
            return Ok(Value::str(&st));
        }
        let charset = if has(a, 1) && !arg(a, 1).is_null() { text(it, a, 1)? } else { "UTF-8".into() };
        let q = has(a, 2) && !arg(a, 2).is_null() && text(it, a, 2)?.eq_ignore_ascii_case("q");
        let nl = if has(a, 3) && !arg(a, 3).is_null() { text(it, a, 3)? } else { "\r\n".into() };
        // Encode in chunks that keep each encoded word under 75 chars.
        let mut words = vec![];
        let mut chunk = String::new();
        for ch in st.chars() {
            chunk.push(ch);
            if chunk.len() >= 30 {
                words.push(std::mem::take(&mut chunk));
            }
        }
        if !chunk.is_empty() {
            words.push(chunk);
        }
        let enc = |w: &str| {
            if q {
                let mut o = String::new();
                for b in w.bytes() {
                    if b.is_ascii_alphanumeric() {
                        o.push(b as char);
                    } else {
                        o.push_str(&format!("={b:02X}"));
                    }
                }
                format!("=?{charset}?Q?{o}?=")
            } else {
                format!("=?{charset}?B?{}?=", crate::strings::base64_encode(w.as_bytes()))
            }
        };
        Ok(Value::str(&words.iter().map(|w| enc(w)).collect::<Vec<_>>().join(&format!("{nl} "))))
    });
    t.insert("mb_decode_mimeheader", |it, _, a| {
        let st = text(it, a, 0)?;
        let mut out = String::new();
        let mut rest = st.as_str();
        let mut after_word = false;
        while let Some(i) = rest.find("=?") {
            let (pre, tail) = rest.split_at(i);
            let parts: Vec<&str> = tail[2..].splitn(3, '?').collect();
            if parts.len() == 3 {
                if let Some(end) = parts[2].find("?=") {
                    let data = &parts[2][..end];
                    let decoded = if parts[1].eq_ignore_ascii_case("b") {
                        crate::strings::base64_decode(data.as_bytes()).unwrap_or_default()
                    } else {
                        let mut v = vec![];
                        let b = data.as_bytes();
                        let mut j = 0;
                        while j < b.len() {
                            match b[j] {
                                b'_' => v.push(b' '),
                                b'=' if j + 2 < b.len() => {
                                    if let Ok(x) = u8::from_str_radix(&data[j + 1..(j + 3).min(data.len())], 16) {
                                        v.push(x);
                                        j += 2;
                                    }
                                }
                                c => v.push(c),
                            }
                            j += 1;
                        }
                        v
                    };
                    // Whitespace between adjacent encoded words is dropped.
                    if !(after_word && pre.trim().is_empty()) {
                        out.push_str(pre);
                    }
                    out.push_str(&crate::pstr::lossy(&decoded));
                    rest = &parts[2][end + 2..];
                    after_word = true;
                    continue;
                }
            }
            out.push_str(&rest[..i + 2]);
            rest = &rest[i + 2..];
            after_word = false;
        }
        out.push_str(rest);
        Ok(Value::str(&out))
    });
    t.insert("mb_convert_kana", |_, _, a| Ok(arg(a, 0)));
    t.insert("mb_convert_variables", |it, _, a| {
        let _ = text(it, a, 0)?;
        Ok(Value::str("UTF-8"))
    });
    t.insert("mb_encode_numericentity", |it, _, a| {
        let st = text(it, a, 0)?;
        let Value::Array(map) = arg(a, 1) else { return Ok(Value::str(&st)) };
        let m: Vec<i64> = map.values().map(|v| it.to_int(&v)).collect::<R<_>>()?;
        let hex = has(a, 3) && arg(a, 3).truthy();
        let mut out = String::new();
        'chars: for ch in st.chars() {
            let c = ch as i64;
            for q in m.chunks(4) {
                if q.len() == 4 && c >= q[0] && c <= q[1] {
                    let v = (c + q[2]) & q[3];
                    out.push_str(&if hex { format!("&#x{v:X};") } else { format!("&#{v};") });
                    continue 'chars;
                }
            }
            out.push(ch);
        }
        Ok(Value::str(&out))
    });
    t.insert("mb_decode_numericentity", |it, _, a| {
        let st = text(it, a, 0)?;
        let Value::Array(map) = arg(a, 1) else { return Ok(Value::str(&st)) };
        let m: Vec<i64> = map.values().map(|v| it.to_int(&v)).collect::<R<_>>()?;
        let mut out = String::new();
        let mut rest = st.as_str();
        while let Some(i) = rest.find("&#") {
            out.push_str(&rest[..i]);
            let tail = &rest[i + 2..];
            let (hex, body) = match tail.strip_prefix(['x', 'X']) {
                Some(b) => (true, b),
                None => (false, tail),
            };
            let digits: String = body.chars().take_while(|c| if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() }).collect();
            let semi = body[digits.len()..].starts_with(';');
            let val = if hex { i64::from_str_radix(&digits, 16).ok() } else { digits.parse().ok() };
            let decoded = val.and_then(|v| m.chunks(4).find_map(|q| {
                let c = v - q.get(2).copied().unwrap_or(0);
                (q.len() == 4 && c >= q[0] && c <= q[1]).then(|| char::from_u32(c as u32)).flatten()
            }));
            match decoded {
                Some(ch) if !digits.is_empty() => {
                    out.push(ch);
                    rest = &body[digits.len() + semi as usize..];
                }
                _ => {
                    out.push_str("&#");
                    rest = tail;
                }
            }
        }
        out.push_str(rest);
        Ok(Value::str(&out))
    });
    t.insert("mb_send_mail", |_, _, _| Ok(Value::Bool(false)));
    t.insert("mb_get_info", |_, _, a| {
        let mut m = PhpArray::new();
        for (k, v) in [
            ("internal_encoding", "UTF-8"),
            ("http_input", ""),
            ("http_output", "UTF-8"),
            ("http_output_conv_mimetypes", "^(text/|application/xhtml\\+xml)"),
            ("mail_charset", "UTF-8"),
            ("mail_header_encoding", "BASE64"),
            ("mail_body_encoding", "BASE64"),
            ("illegal_chars", "0"),
            ("encoding_translation", "Off"),
            ("language", "neutral"),
            ("substitute_character", "63"),
            ("strict_detection", "Off"),
        ] {
            m.insert(Key::Str(pstr(k)), Value::str(v));
        }
        if has(a, 0) {
            let k = arg(a, 0);
            return Ok(match &k {
                Value::Str(s) => m.get(&Key::Str(s.clone())).unwrap_or(Value::Bool(false)),
                _ => Value::array(m),
            });
        }
        Ok(Value::array(m))
    });
    t.insert("mb_regex_encoding", |_, _, a| Ok(if has(a, 0) && !arg(a, 0).is_null() { Value::Bool(true) } else { Value::str("UTF-8") }));
    t.insert("mb_regex_set_options", |_, _, _| Ok(Value::str("pr")));
    t.insert("mb_ereg", |it, f, a| ereg(it, f, a, "", "mb_ereg"));
    t.insert("mb_eregi", |it, f, a| ereg(it, f, a, "i", "mb_eregi"));
    t.insert("mb_ereg_replace", |it, f, a| ereg_replace(it, f, a, false));
    t.insert("mb_eregi_replace", |it, f, a| ereg_replace(it, f, a, true));
    t.insert("mb_ereg_replace_callback", |it, f, a| {
        let pat = text(it, a, 0)?;
        let opts = if has(a, 3) && !arg(a, 3).is_null() { text(it, a, 3)? } else { String::new() };
        let mut args = [Value::bytes(pcre(&pat, &opts)), arg(a, 1), arg(a, 2)];
        let r = call(it, f, "preg_replace_callback", &mut args)?;
        Ok(if r.is_null() { Value::Bool(false) } else { r })
    });
    t.insert("mb_split", |it, f, a| {
        let pat = text(it, a, 0)?;
        if pat.is_empty() {
            return Ok(Value::array(PhpArray::from_list([arg(a, 1)])));
        }
        let limit = if has(a, 2) { arg(a, 2) } else { Value::Int(-1) };
        let mut args = [Value::bytes(pcre(&pat, "")), arg(a, 1), limit];
        call(it, f, "preg_split", &mut args)
    });
    t.insert("mb_ereg_match", |it, f, a| {
        let pat = text(it, a, 0)?;
        let opts = if has(a, 2) && !arg(a, 2).is_null() { text(it, a, 2)? } else { String::new() };
        let mut args = [Value::bytes(pcre(&format!("\\A(?:{pat})"), &opts)), arg(a, 1)];
        Ok(Value::Bool(matches!(call(it, f, "preg_match", &mut args)?, Value::Int(1))))
    });
    t.insert("mb_ereg_search_init", |it, _, a| {
        let st = text(it, a, 0)?;
        let pat = if has(a, 1) && !arg(a, 1).is_null() { text(it, a, 1)? } else { String::new() };
        SEARCH.with(|x| *x.borrow_mut() = (st, pat, 0, vec![]));
        Ok(Value::Bool(true))
    });
    t.insert("mb_ereg_search_setpos", |it, _, a| {
        let p = int(it, a, 0)?.max(0) as usize;
        SEARCH.with(|x| x.borrow_mut().2 = p);
        Ok(Value::Bool(true))
    });
    t.insert("mb_ereg_search_getpos", |_, _, _| Ok(Value::Int(SEARCH.with(|x| x.borrow().2 as i64))));
    t.insert("mb_ereg_search_getregs", |_, _, _| {
        Ok(SEARCH.with(|x| {
            let r = &x.borrow().3;
            if r.is_empty() { Value::Bool(false) } else { Value::array(PhpArray::from_list(r.iter().cloned())) }
        }))
    });
    for name in ["mb_ereg_search", "mb_ereg_search_pos", "mb_ereg_search_regs"] {
        let f: BuiltinFn = match name {
            "mb_ereg_search" => |it, f, a| search(it, f, a, 0),
            "mb_ereg_search_pos" => |it, f, a| search(it, f, a, 1),
            _ => |it, f, a| search(it, f, a, 2),
        };
        t.insert(name, f);
    }
}

/// mb_ereg_search*: next match from the saved position (0: bool, 1:
/// [pos, len], 2: groups).
fn search(it: &Interp, f: &mut crate::interp::Frame, a: &mut [Value], kind: u8) -> R<Value> {
    let (st, mut pat, pos, _) = SEARCH.with(|x| x.borrow().clone());
    if has(a, 0) && !arg(a, 0).is_null() {
        pat = text(it, a, 0)?;
        SEARCH.with(|x| x.borrow_mut().1 = pat.clone());
    }
    let mut args = [Value::bytes(pcre(&pat, "")), Value::str(&st), Value::Null, Value::Int(256), Value::Int(pos as i64)];
    let found = matches!(call(it, f, "preg_match", &mut args)?, Value::Int(1));
    if !found {
        SEARCH.with(|x| x.borrow_mut().3.clear());
        return Ok(Value::Bool(false));
    }
    let Value::Array(m) = &args[2] else { unreachable!() };
    let mut regs = vec![];
    let (mut start, mut end) = (0usize, 0usize);
    for (k, v) in m.iter() {
        if let (Key::Int(i), Value::Array(pair)) = (k, v) {
            let s = pair.get(&Key::Int(0)).unwrap_or_default();
            let off = pair.get(&Key::Int(1)).map(|o| it.to_int(&o)).transpose()?.unwrap_or(-1);
            if i == 0 {
                start = off.max(0) as usize;
                end = start + it.to_str(&s)?.len();
            }
            regs.push(if off < 0 { Value::Bool(false) } else { s });
        }
    }
    SEARCH.with(|x| {
        let mut x = x.borrow_mut();
        x.2 = if end == start { end + 1 } else { end };
        x.3 = regs.clone();
    });
    Ok(match kind {
        0 => Value::Bool(true),
        1 => Value::array(PhpArray::from_list([Value::Int(start as i64), Value::Int((end - start) as i64)])),
        _ => Value::array(PhpArray::from_list(regs)),
    })
}

/// mb_strrchr / mb_stristr / mb_strrichr.
fn part(it: &Interp, a: &[Value], last: bool, icase: bool) -> R<Value> {
    let (h, n) = (text(it, a, 0)?, text(it, a, 1)?);
    let before = has(a, 2) && arg(a, 2).truthy();
    let (hs, ns) = if icase { (lower(&h), lower(&n)) } else { (h.clone(), n.clone()) };
    let found = if last { hs.rfind(&ns) } else { hs.find(&ns) };
    let Some(bi) = found else { return Ok(Value::Bool(false)) };
    // Map the byte offset in the (possibly lowered) copy to `h` by chars.
    let ci = char_index(&hs, bi);
    let b = byte_index(&h, ci).unwrap_or(h.len());
    Ok(Value::str(if before { &h[..b] } else { &h[b..] }))
}
