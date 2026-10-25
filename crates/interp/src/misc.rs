//! SAPI-level and assorted built-ins: headers, cookies, shutdown functions,
//! array pointers, HTML entities, zlib, CSV, password hashing, filters.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::rc::Rc;

use crate::builtins::{arg, callable, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{pstr, Key, PhpArray, Value};

fn bstr(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(String::from_utf8_lossy(&s(it, a, i)?).into_owned())
}

// ---- headers -----------------------------------------------------------------------

/// Response state of the running script (CLI: recorded, never sent).
#[derive(Default)]
pub struct Response {
    pub headers: Vec<String>,
    pub code: i64,
    pub shutdown: Vec<(crate::interp::Callable, Vec<Value>)>,
}

fn headers_already_sent(it: &Interp, fname: &str) -> bool {
    match it.output_started() {
        Some((file, line)) => {
            it.warn(&format!(
                "{fname}Cannot modify header information - headers already sent by (output started at {file}:{line})"
            ));
            true
        }
        None => false,
    }
}

fn set_header(it: &Interp, line: &str, replace: bool, code: i64) {
    let mut r = it.response.borrow_mut();
    if line.len() >= 5 && line[..5].eq_ignore_ascii_case("HTTP/") {
        if let Some(c) = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()) {
            r.code = c;
        }
        return;
    }
    let Some((name, _)) = line.split_once(':') else { return };
    if replace {
        r.headers.retain(|h| !h.split(':').next().is_some_and(|n| n.trim().eq_ignore_ascii_case(name.trim())));
    }
    r.headers.push(line.to_string());
    if name.trim().eq_ignore_ascii_case("location") && code == 0 && !(201..400).contains(&r.code) {
        r.code = 302;
    }
    if code > 0 {
        r.code = code;
    }
}

fn cookie_header(it: &Interp, a: &[Value], raw: bool) -> R<String> {
    let name = bstr(it, a, 0)?;
    let value = if has(a, 1) { s(it, a, 1)?.to_vec() } else { vec![] };
    let enc = if raw {
        String::from_utf8_lossy(&value).into_owned()
    } else {
        value.iter().map(|&c| if c.is_ascii_alphanumeric() || b"-_.~".contains(&c) { (c as char).to_string() } else { format!("%{c:02X}") }).collect()
    };
    let mut h = format!("Set-Cookie: {name}={}", if value.is_empty() { "deleted".into() } else { enc });
    let (mut expires, mut path, mut domain, mut secure, mut httponly, mut samesite) = (0i64, String::new(), String::new(), false, false, String::new());
    match arg(a, 2) {
        Value::Array(o) => {
            for (k, v) in o.iter() {
                let k = match k {
                    Key::Str(s) => String::from_utf8_lossy(&s).to_ascii_lowercase(),
                    Key::Int(_) => continue,
                };
                match k.as_str() {
                    "expires" => expires = it.to_int(&v)?,
                    "path" => path = it.to_string_lossy(&v)?,
                    "domain" => domain = it.to_string_lossy(&v)?,
                    "secure" => secure = v.truthy(),
                    "httponly" => httponly = v.truthy(),
                    "samesite" => samesite = it.to_string_lossy(&v)?,
                    _ => {}
                }
            }
        }
        v => {
            expires = if has(a, 2) { it.to_int(&v)? } else { 0 };
            path = if has(a, 3) { bstr(it, a, 3)? } else { String::new() };
            domain = if has(a, 4) { bstr(it, a, 4)? } else { String::new() };
            secure = arg(a, 5).truthy();
            httponly = arg(a, 6).truthy();
        }
    }
    if value.is_empty() {
        h.push_str("; expires=Thu, 01 Jan 1970 00:00:01 GMT; Max-Age=0");
    } else if expires > 0 {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        h.push_str(&format!("; expires={}; Max-Age={}", crate::builtins::format_date("D, d M Y H:i:s \\G\\M\\T", expires), (expires - now).max(0)));
    }
    if !path.is_empty() {
        h.push_str(&format!("; path={path}"));
    }
    if !domain.is_empty() {
        h.push_str(&format!("; domain={domain}"));
    }
    if secure {
        h.push_str("; secure");
    }
    if httponly {
        h.push_str("; HttpOnly");
    }
    if !samesite.is_empty() {
        h.push_str(&format!("; SameSite={samesite}"));
    }
    Ok(h)
}

// ---- HTML entities ---------------------------------------------------------------------

const ENTITIES: &[(&str, u32)] = &[
    ("nbsp", 160), ("iexcl", 161), ("cent", 162), ("pound", 163), ("curren", 164), ("yen", 165), ("brvbar", 166), ("sect", 167),
    ("uml", 168), ("copy", 169), ("ordf", 170), ("laquo", 171), ("not", 172), ("shy", 173), ("reg", 174), ("macr", 175),
    ("deg", 176), ("plusmn", 177), ("sup2", 178), ("sup3", 179), ("acute", 180), ("micro", 181), ("para", 182), ("middot", 183),
    ("cedil", 184), ("sup1", 185), ("ordm", 186), ("raquo", 187), ("frac14", 188), ("frac12", 189), ("frac34", 190), ("iquest", 191),
    ("Agrave", 192), ("Aacute", 193), ("Acirc", 194), ("Atilde", 195), ("Auml", 196), ("Aring", 197), ("AElig", 198), ("Ccedil", 199),
    ("Egrave", 200), ("Eacute", 201), ("Ecirc", 202), ("Euml", 203), ("Igrave", 204), ("Iacute", 205), ("Icirc", 206), ("Iuml", 207),
    ("ETH", 208), ("Ntilde", 209), ("Ograve", 210), ("Oacute", 211), ("Ocirc", 212), ("Otilde", 213), ("Ouml", 214), ("times", 215),
    ("Oslash", 216), ("Ugrave", 217), ("Uacute", 218), ("Ucirc", 219), ("Uuml", 220), ("Yacute", 221), ("THORN", 222), ("szlig", 223),
    ("agrave", 224), ("aacute", 225), ("acirc", 226), ("atilde", 227), ("auml", 228), ("aring", 229), ("aelig", 230), ("ccedil", 231),
    ("egrave", 232), ("eacute", 233), ("ecirc", 234), ("euml", 235), ("igrave", 236), ("iacute", 237), ("icirc", 238), ("iuml", 239),
    ("eth", 240), ("ntilde", 241), ("ograve", 242), ("oacute", 243), ("ocirc", 244), ("otilde", 245), ("ouml", 246), ("divide", 247),
    ("oslash", 248), ("ugrave", 249), ("uacute", 250), ("ucirc", 251), ("uuml", 252), ("yacute", 253), ("thorn", 254), ("yuml", 255),
    ("OElig", 338), ("oelig", 339), ("Scaron", 352), ("scaron", 353), ("Yuml", 376), ("fnof", 402), ("circ", 710), ("tilde", 732),
    ("Alpha", 913), ("Beta", 914), ("Gamma", 915), ("Delta", 916), ("Epsilon", 917), ("Zeta", 918), ("Eta", 919), ("Theta", 920),
    ("Iota", 921), ("Kappa", 922), ("Lambda", 923), ("Mu", 924), ("Nu", 925), ("Xi", 926), ("Omicron", 927), ("Pi", 928),
    ("Rho", 929), ("Sigma", 931), ("Tau", 932), ("Upsilon", 933), ("Phi", 934), ("Chi", 935), ("Psi", 936), ("Omega", 937),
    ("alpha", 945), ("beta", 946), ("gamma", 947), ("delta", 948), ("epsilon", 949), ("zeta", 950), ("eta", 951), ("theta", 952),
    ("iota", 953), ("kappa", 954), ("lambda", 955), ("mu", 956), ("nu", 957), ("xi", 958), ("omicron", 959), ("pi", 960),
    ("rho", 961), ("sigmaf", 962), ("sigma", 963), ("tau", 964), ("upsilon", 965), ("phi", 966), ("chi", 967), ("psi", 968),
    ("omega", 969), ("thetasym", 977), ("upsih", 978), ("piv", 982), ("ensp", 8194), ("emsp", 8195), ("thinsp", 8201), ("zwnj", 8204),
    ("zwj", 8205), ("lrm", 8206), ("rlm", 8207), ("ndash", 8211), ("mdash", 8212), ("lsquo", 8216), ("rsquo", 8217), ("sbquo", 8218),
    ("ldquo", 8220), ("rdquo", 8221), ("bdquo", 8222), ("dagger", 8224), ("Dagger", 8225), ("bull", 8226), ("hellip", 8230), ("permil", 8240),
    ("prime", 8242), ("Prime", 8243), ("lsaquo", 8249), ("rsaquo", 8250), ("oline", 8254), ("frasl", 8260), ("euro", 8364), ("image", 8465),
    ("weierp", 8472), ("real", 8476), ("trade", 8482), ("alefsym", 8501), ("larr", 8592), ("uarr", 8593), ("rarr", 8594), ("darr", 8595),
    ("harr", 8596), ("crarr", 8629), ("lArr", 8656), ("uArr", 8657), ("rArr", 8658), ("dArr", 8659), ("hArr", 8660), ("forall", 8704),
    ("part", 8706), ("exist", 8707), ("empty", 8709), ("nabla", 8711), ("isin", 8712), ("notin", 8713), ("ni", 8715), ("prod", 8719),
    ("sum", 8721), ("minus", 8722), ("lowast", 8727), ("radic", 8730), ("prop", 8733), ("infin", 8734), ("ang", 8736), ("and", 8743),
    ("or", 8744), ("cap", 8745), ("cup", 8746), ("int", 8747), ("there4", 8756), ("sim", 8764), ("cong", 8773), ("asymp", 8776),
    ("ne", 8800), ("equiv", 8801), ("le", 8804), ("ge", 8805), ("sub", 8834), ("sup", 8835), ("nsub", 8836), ("sube", 8838),
    ("supe", 8839), ("oplus", 8853), ("otimes", 8855), ("perp", 8869), ("sdot", 8901), ("lceil", 8968), ("rceil", 8969), ("lfloor", 8970),
    ("rfloor", 8971), ("lang", 9001), ("rang", 9002), ("loz", 9674), ("spades", 9824), ("clubs", 9827), ("hearts", 9829), ("diams", 9830),
];

fn decode_entities(st: &[u8], flags: i64) -> Vec<u8> {
    let mut out = Vec::with_capacity(st.len());
    let mut i = 0;
    while i < st.len() {
        if st[i] == b'&' {
            if let Some(end) = st[i + 1..].iter().take(32).position(|&c| c == b';') {
                let name = &st[i + 1..i + 1 + end];
                let cp: Option<u32> = if let Some(num) = name.strip_prefix(b"#") {
                    let txt = String::from_utf8_lossy(num);
                    let v = if let Some(h) = txt.strip_prefix('x').or_else(|| txt.strip_prefix('X')) {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        txt.parse().ok()
                    };
                    v.filter(|&c| c != 0 && !(0xD800..0xE000).contains(&c) && c <= 0x10FFFF)
                } else {
                    match name {
                        b"amp" => Some(b'&' as u32),
                        b"lt" => Some(b'<' as u32),
                        b"gt" => Some(b'>' as u32),
                        b"quot" if flags & 2 != 0 => Some(b'"' as u32),
                        b"apos" | b"#039" if flags & 3 == 3 => Some(b'\'' as u32),
                        n => ENTITIES.iter().find(|(e, _)| e.as_bytes() == n).map(|(_, c)| *c),
                    }
                };
                // Numeric quotes follow ENT_ flags too.
                let cp = match cp {
                    Some(34) if flags & 2 == 0 && name.starts_with(b"#") => None,
                    Some(39) if flags & 3 != 3 && name.starts_with(b"#") => None,
                    c => c,
                };
                if let Some(c) = cp.and_then(char::from_u32) {
                    out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
                    i += end + 2;
                    continue;
                }
            }
        }
        out.push(st[i]);
        i += 1;
    }
    out
}

fn encode_entities(st: &[u8], flags: i64, double: bool) -> Vec<u8> {
    let text = String::from_utf8_lossy(st);
    let mut out = String::with_capacity(st.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '&' => {
                let rest: String = chars[i..].iter().take(33).collect();
                if !double && is_entity_ref(rest.as_bytes()) {
                    out.push('&');
                } else {
                    out.push_str("&amp;");
                }
            }
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if flags & 2 != 0 => out.push_str("&quot;"),
            '\'' if flags & 3 == 3 => out.push_str("&#039;"),
            c if (c as u32) >= 160 => match ENTITIES.iter().find(|(_, cp)| *cp == c as u32) {
                Some((name, _)) => {
                    out.push('&');
                    out.push_str(name);
                    out.push(';');
                }
                None => out.push(c),
            },
            c => out.push(c),
        }
        i += 1;
    }
    out.into_bytes()
}

fn is_entity_ref(s: &[u8]) -> bool {
    match s.iter().take(32).position(|&c| c == b';') {
        Some(e) if e > 1 => {
            let n = &s[1..e];
            n.iter().all(|c| c.is_ascii_alphanumeric()) || (n[0] == b'#' && n[1..].iter().all(|c| c.is_ascii_alphanumeric()))
        }
        _ => false,
    }
}

// ---- CSV ----------------------------------------------------------------------------------

pub fn csv_line(fields: &[Vec<u8>], sep: u8, enc: u8, esc: Option<u8>, eol: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.push(sep);
        }
        let needs = f.iter().any(|&c| c == sep || c == enc || c == b'\n' || c == b'\r' || c == b'\t' || c == b' ' || Some(c) == esc);
        if needs {
            out.push(enc);
            let mut escaped = false;
            for &c in f {
                if esc.is_some_and(|e| e == c) {
                    escaped = true;
                } else if !escaped && c == enc {
                    out.push(enc);
                } else {
                    escaped = false;
                }
                out.push(c);
            }
            out.push(enc);
        } else {
            out.extend_from_slice(f);
        }
    }
    out.extend_from_slice(eol);
    out
}

pub fn csv_parse(line: &[u8], sep: u8, enc: u8, esc: Option<u8>) -> Vec<Value> {
    let mut fields = vec![];
    let mut cur = vec![];
    let mut i = 0;
    let n = line.len();
    let line = if line.ends_with(b"\r\n") {
        &line[..n - 2]
    } else if line.ends_with(b"\n") {
        &line[..n - 1]
    } else {
        line
    };
    if line.is_empty() {
        return vec![Value::Null];
    }
    loop {
        // skip leading whitespace before an enclosure
        let mut j = i;
        while j < line.len() && matches!(line[j], b' ' | b'\t') && line[j] != sep {
            j += 1;
        }
        if j < line.len() && line[j] == enc {
            i = j + 1;
            loop {
                if i >= line.len() {
                    break;
                }
                let c = line[i];
                if esc.is_some_and(|e| e == c) && esc != Some(enc) && i + 1 < line.len() {
                    cur.push(c);
                    cur.push(line[i + 1]);
                    i += 2;
                    continue;
                }
                if c == enc {
                    if line.get(i + 1) == Some(&enc) {
                        cur.push(enc);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                cur.push(c);
                i += 1;
            }
            // text after the closing enclosure up to the separator
            while i < line.len() && line[i] != sep {
                cur.push(line[i]);
                i += 1;
            }
        } else {
            while i < line.len() && line[i] != sep {
                cur.push(line[i]);
                i += 1;
            }
        }
        fields.push(Value::bytes(std::mem::take(&mut cur)));
        if i >= line.len() {
            break;
        }
        i += 1; // separator
        if i >= line.len() {
            fields.push(Value::str(""));
            break;
        }
    }
    fields
}

fn csv_opts(it: &Interp, a: &[Value], from: usize, name: &str) -> R<(u8, u8, Option<u8>)> {
    if !has(a, from + 2) {
        it.deprecated(&format!("{name}(): the $escape parameter must be provided as its default value will change"));
    }
    let ch = |i: usize, d: u8| -> R<Option<u8>> {
        if has(a, i) {
            let v = s(it, a, i)?;
            Ok(v.first().copied())
        } else {
            Ok(Some(d))
        }
    };
    Ok((ch(from, b',')?.unwrap_or(b','), ch(from + 1, b'"')?.unwrap_or(b'"'), ch(from + 2, b'\\')?))
}

// ---- filter_var -----------------------------------------------------------------------------

pub const FILTER_CONSTS: &[(&str, i64)] = &[
    ("PASSWORD_BCRYPT_DEFAULT_COST", 12),
    ("FILTER_VALIDATE_INT", 257),
    ("FILTER_VALIDATE_BOOLEAN", 258),
    ("FILTER_VALIDATE_BOOL", 258),
    ("FILTER_VALIDATE_FLOAT", 259),
    ("FILTER_VALIDATE_REGEXP", 272),
    ("FILTER_VALIDATE_DOMAIN", 277),
    ("FILTER_VALIDATE_URL", 273),
    ("FILTER_VALIDATE_EMAIL", 274),
    ("FILTER_VALIDATE_IP", 275),
    ("FILTER_VALIDATE_MAC", 276),
    ("FILTER_DEFAULT", 516),
    ("FILTER_UNSAFE_RAW", 516),
    ("FILTER_SANITIZE_STRING", 513),
    ("FILTER_SANITIZE_NUMBER_INT", 519),
    ("FILTER_SANITIZE_NUMBER_FLOAT", 520),
    ("FILTER_SANITIZE_SPECIAL_CHARS", 515),
    ("FILTER_SANITIZE_FULL_SPECIAL_CHARS", 522),
    ("FILTER_SANITIZE_EMAIL", 517),
    ("FILTER_SANITIZE_URL", 518),
    ("FILTER_SANITIZE_ADD_SLASHES", 523),
    ("FILTER_CALLBACK", 1024),
    ("FILTER_FLAG_ALLOW_OCTAL", 1),
    ("FILTER_FLAG_ALLOW_HEX", 2),
    ("FILTER_FLAG_ALLOW_FRACTION", 4096),
    ("FILTER_FLAG_ALLOW_THOUSAND", 8192),
    ("FILTER_FLAG_IPV4", 1048576),
    ("FILTER_FLAG_IPV6", 2097152),
    ("FILTER_FLAG_NO_PRIV_RANGE", 8388608),
    ("FILTER_FLAG_NO_RES_RANGE", 4194304),
    ("FILTER_FLAG_GLOBAL_RANGE", 268435456),
    ("FILTER_NULL_ON_FAILURE", 134217728),
    ("FILTER_REQUIRE_ARRAY", 16777216),
    ("FILTER_FORCE_ARRAY", 67108864),
    ("FILTER_REQUIRE_SCALAR", 33554432),
    ("FILTER_FLAG_PATH_REQUIRED", 262144),
    ("FILTER_FLAG_QUERY_REQUIRED", 524288),
    ("FILTER_FLAG_HOSTNAME", 1048576),
    ("FILTER_FLAG_EMAIL_UNICODE", 1048576),
    ("FILTER_FLAG_STRIP_LOW", 4),
    ("FILTER_FLAG_STRIP_HIGH", 8),
    ("FILTER_FLAG_ENCODE_LOW", 16),
    ("FILTER_FLAG_ENCODE_HIGH", 32),
    ("FILTER_FLAG_ENCODE_AMP", 64),
    ("FILTER_FLAG_NO_ENCODE_QUOTES", 128),
    ("INPUT_POST", 0),
    ("INPUT_GET", 1),
    ("INPUT_COOKIE", 2),
    ("INPUT_ENV", 4),
    ("INPUT_SERVER", 5),
];

fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.rsplit_once('@') else { return false };
    if local.is_empty() || local.len() > 64 || domain.is_empty() || e.len() > 320 {
        return false;
    }
    let local_ok = local.split('.').all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_alphanumeric() || b"!#$%&'*+/=?^_`{|}~-".contains(&c)));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        });
    local_ok && domain_ok
}

fn valid_url(u: &str, flags: i64) -> bool {
    let Some(parts) = crate::ext::parse_url(u) else { return false };
    let get = |k: &str| parts.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
    let Some(Value::Str(scheme)) = get("scheme") else { return false };
    let scheme = String::from_utf8_lossy(&scheme).to_ascii_lowercase();
    let host = get("host");
    if host.is_none() && !matches!(scheme.as_str(), "mailto" | "news" | "file") {
        return false;
    }
    if let Some(Value::Str(h)) = &host {
        let h = String::from_utf8_lossy(h);
        if !(h.starts_with('[') || h.split('.').all(|l| !l.is_empty() && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))) {
            return false;
        }
    }
    if flags & 262144 != 0 && get("path").is_none() {
        return false;
    }
    if flags & 524288 != 0 && get("query").is_none() {
        return false;
    }
    u.bytes().all(|c| c > 32 && c < 127)
}

pub fn filter_value(it: &Interp, v: &Value, filter: i64, opts: &Value) -> R<Value> {
    let (flags, options) = match opts {
        Value::Array(o) => (
            o.get(&Key::Str(pstr("flags"))).map(|f| it.to_int(&f)).transpose()?.unwrap_or(0),
            o.get(&Key::Str(pstr("options"))),
        ),
        Value::Null => (0, None),
        other => (it.to_int(other)?, None),
    };
    let fail = || if flags & 134217728 != 0 { Value::Null } else { Value::Bool(false) };
    let default = || -> Option<Value> {
        match &options {
            Some(Value::Array(o)) => o.get(&Key::Str(pstr("default"))),
            _ => None,
        }
    };
    let opt = |k: &str| -> Option<Value> {
        match &options {
            Some(Value::Array(o)) => o.get(&Key::Str(pstr(k))),
            _ => None,
        }
    };
    if matches!(v, Value::Array(_) | Value::Object(_)) && filter != 1024 {
        return Ok(default().unwrap_or_else(fail));
    }
    let text = match v {
        Value::Bool(false) | Value::Null if filter != 258 => String::new(),
        _ => it.to_string_lossy(v)?,
    };
    let t = text.trim();
    let r = match filter {
        257 => {
            let parsed = if flags & 2 != 0 && (t.starts_with("0x") || t.starts_with("0X")) {
                i64::from_str_radix(&t[2..], 16).ok()
            } else if flags & 1 != 0 && t.len() > 1 && t.starts_with('0') {
                i64::from_str_radix(&t[1..], 8).ok()
            } else {
                let ok = !t.is_empty()
                    && (t == "0" || {
                        let d = t.strip_prefix(['-', '+']).unwrap_or(t);
                        !d.is_empty() && !d.starts_with('0') && d.bytes().all(|c| c.is_ascii_digit())
                    });
                if ok { t.parse::<i64>().ok() } else { None }
            };
            match parsed {
                Some(n) => {
                    let min = opt("min_range").map(|x| it.to_int(&x)).transpose()?;
                    let max = opt("max_range").map(|x| it.to_int(&x)).transpose()?;
                    if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                        None
                    } else {
                        Some(Value::Int(n))
                    }
                }
                None => None,
            }
        }
        258 => match v {
            Value::Bool(b) => Some(Value::Bool(*b)),
            _ => match t.to_ascii_lowercase().as_str() {
                "1" | "true" | "on" | "yes" => Some(Value::Bool(true)),
                "0" | "false" | "off" | "no" | "" => {
                    if t.is_empty() && flags & 134217728 != 0 && !matches!(v, Value::Str(_)) {
                        None
                    } else {
                        Some(Value::Bool(false))
                    }
                }
                _ => None,
            },
        },
        259 => {
            let clean = if flags & 8192 != 0 { t.replace(',', "") } else { t.to_string() };
            match crate::value::parse_numeric(clean.as_bytes()) {
                Some(n) => Some(Value::Float(n.as_f64())),
                None => None,
            }
        }
        272 => {
            let Some(re) = opt("regexp") else {
                return Err(it.error("ValueError", "filter_var(): \"regexp\" option missing"));
            };
            let f = &mut crate::interp::Frame::new(it.top_ctx(0, false), 0);
            let mut args = vec![re, Value::str(&text)];
            let m = (it.builtins["preg_match"])(it, f, &mut args)?;
            matches!(m, Value::Int(1)).then(|| Value::str(&text))
        }
        273 => valid_url(&text, flags).then(|| Value::str(&text)),
        274 => valid_email(&text).then(|| Value::str(&text)),
        275 => {
            let ok = match text.parse::<std::net::IpAddr>() {
                Ok(std::net::IpAddr::V4(ip)) => {
                    (flags & 2097152 == 0 || flags & 1048576 != 0)
                        && !(flags & 8388608 != 0 && ip.is_private())
                        && !(flags & 4194304 != 0 && (ip.is_loopback() || ip.is_unspecified() || ip.is_link_local() || ip.is_broadcast() || ip.octets()[0] >= 240))
                }
                Ok(std::net::IpAddr::V6(ip)) => {
                    (flags & 1048576 == 0 || flags & 2097152 != 0) && !(flags & 4194304 != 0 && (ip.is_loopback() || ip.is_unspecified()))
                }
                Err(_) => false,
            };
            ok.then(|| Value::str(&text))
        }
        277 => (!text.is_empty() && text.len() <= 253).then(|| Value::str(&text)),
        516 | 513 => Some(match v {
            Value::Str(_) => Value::str(&text),
            other => Value::str(&it.to_string_lossy(other)?),
        }),
        519 => Some(Value::str(&text.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == '+').collect::<String>())),
        520 => Some(Value::str(
            &text
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '-' || *c == '+' || (flags & 4096 != 0 && *c == '.') || (flags & 8192 != 0 && *c == ',') || (flags & 16384 != 0 && (*c == 'e' || *c == 'E')))
                .collect::<String>(),
        )),
        515 | 522 => Some(Value::bytes(encode_entities(text.as_bytes(), 3, true))),
        517 => Some(Value::str(&text.chars().filter(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-=?^_`{|}~@.[]".contains(*c)).collect::<String>())),
        518 => Some(Value::str(&text.chars().filter(|c| c.is_ascii_graphic()).collect::<String>())),
        523 => Some(Value::str(&text.replace('\\', "\\\\").replace('\'', "\\'").replace('"', "\\\"").replace('\0', "\\0"))),
        1024 => {
            let Some(cb) = options.clone() else {
                it.warn("filter_var(): First argument is expected to be a valid callback");
                return Ok(Value::Bool(false));
            };
            let f = &mut crate::interp::Frame::new(it.top_ctx(0, false), 0);
            let c = it.to_callable(f, &cb)?;
            return it.call_callable(&c, vec![v.clone()], vec![]);
        }
        _ => Some(Value::str(&text)),
    };
    Ok(r.or_else(default).unwrap_or_else(fail))
}

// ---- gz -------------------------------------------------------------------------------------

fn inflate_with(data: &[u8], kind: u8) -> Option<Vec<u8>> {
    let mut out = vec![];
    let ok = match kind {
        0 => flate2::read::DeflateDecoder::new(data).read_to_end(&mut out).is_ok(),
        1 => flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).is_ok(),
        _ => flate2::read::MultiGzDecoder::new(data).read_to_end(&mut out).is_ok(),
    };
    ok.then_some(out)
}

fn deflate_with(data: &[u8], level: i64, kind: u8) -> Vec<u8> {
    let lvl = if (0..=9).contains(&level) { flate2::Compression::new(level as u32) } else { flate2::Compression::default() };
    match kind {
        0 => {
            let mut e = flate2::write::DeflateEncoder::new(vec![], lvl);
            let _ = e.write_all(data);
            e.finish().unwrap_or_default()
        }
        1 => {
            let mut e = flate2::write::ZlibEncoder::new(vec![], lvl);
            let _ = e.write_all(data);
            e.finish().unwrap_or_default()
        }
        _ => {
            let mut e = flate2::GzBuilder::new().operating_system(3).write(vec![], lvl);
            let _ = e.write_all(data);
            e.finish().unwrap_or_default()
        }
    }
}

// ---- levenshtein / similar_text / soundex ----------------------------------------------------

fn levenshtein(a: &[u8], b: &[u8], ins: i64, rep: i64, del: i64) -> i64 {
    let mut prev: Vec<i64> = (0..=b.len() as i64).map(|i| i * ins).collect();
    for (i, &ca) in a.iter().enumerate() {
        let mut cur = vec![(i as i64 + 1) * del];
        for (j, &cb) in b.iter().enumerate() {
            let c = (prev[j] + if ca == cb { 0 } else { rep }).min(prev[j + 1] + del).min(cur[j] + ins);
            cur.push(c);
        }
        prev = cur;
    }
    prev[b.len()]
}

fn similar(a: &[u8], b: &[u8]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let (mut max, mut pa, mut pb) = (0, 0, 0);
    for i in 0..a.len() {
        for j in 0..b.len() {
            let mut k = 0;
            while i + k < a.len() && j + k < b.len() && a[i + k] == b[j + k] {
                k += 1;
            }
            if k > max {
                max = k;
                pa = i;
                pb = j;
            }
        }
    }
    if max == 0 {
        return 0;
    }
    max + similar(&a[..pa], &b[..pb]) + similar(&a[pa + max..], &b[pb + max..])
}

fn soundex(s: &[u8]) -> String {
    let code = |c: u8| match c.to_ascii_uppercase() {
        b'B' | b'F' | b'P' | b'V' => b'1',
        b'C' | b'G' | b'J' | b'K' | b'Q' | b'S' | b'X' | b'Z' => b'2',
        b'D' | b'T' => b'3',
        b'L' => b'4',
        b'M' | b'N' => b'5',
        b'R' => b'6',
        _ => 0,
    };
    let letters: Vec<u8> = s.iter().copied().filter(|c| c.is_ascii_alphabetic()).collect();
    let Some(&first) = letters.first() else { return String::new() };
    let mut out = vec![first.to_ascii_uppercase()];
    let mut last = code(first);
    for &c in &letters[1..] {
        let d = code(c);
        let upper = c.to_ascii_uppercase();
        if d != 0 && d != last {
            out.push(d);
            if out.len() == 4 {
                break;
            }
        }
        if upper != b'H' && upper != b'W' {
            last = d;
        }
    }
    while out.len() < 4 {
        out.push(b'0');
    }
    String::from_utf8(out).unwrap()
}

// ---- sscanf ---------------------------------------------------------------------------------------

fn sscanf(input: &[u8], fmt: &[u8]) -> Option<Vec<Value>> {
    let mut out = vec![];
    let (mut i, mut f) = (0, 0);
    let mut failed = false;
    while f < fmt.len() {
        let c = fmt[f];
        if c.is_ascii_whitespace() {
            while i < input.len() && input[i].is_ascii_whitespace() {
                i += 1;
            }
            f += 1;
            continue;
        }
        if c != b'%' {
            if !failed && input.get(i) == Some(&c) {
                i += 1;
            } else {
                failed = true;
            }
            f += 1;
            continue;
        }
        f += 1;
        if fmt.get(f) == Some(&b'%') {
            if input.get(i) == Some(&b'%') {
                i += 1;
            } else {
                failed = true;
            }
            f += 1;
            continue;
        }
        let suppress = fmt.get(f) == Some(&b'*');
        if suppress {
            f += 1;
        }
        let st = f;
        while f < fmt.len() && fmt[f].is_ascii_digit() {
            f += 1;
        }
        let width: Option<usize> = std::str::from_utf8(&fmt[st..f]).ok().and_then(|w| w.parse().ok());
        while f < fmt.len() && matches!(fmt[f], b'l' | b'h' | b'L') {
            f += 1;
        }
        let Some(&conv) = fmt.get(f) else { break };
        f += 1;
        if failed || i >= input.len() && conv != b'n' {
            failed = true;
            if !suppress && conv != b'n' {
                out.push(Value::Null);
            }
            continue;
        }
        if !matches!(conv, b'c' | b'[' | b'n') {
            while i < input.len() && input[i].is_ascii_whitespace() {
                i += 1;
            }
        }
        let max = width.unwrap_or(usize::MAX);
        let take = |i: &mut usize, ok: &dyn Fn(u8, usize) -> bool| -> Vec<u8> {
            let st = *i;
            while *i < input.len() && *i - st < max && ok(input[*i], *i - st) {
                *i += 1;
            }
            input[st..*i].to_vec()
        };
        let v = match conv {
            b'd' | b'i' | b'u' => {
                let t = take(&mut i, &|c, k| c.is_ascii_digit() || (k == 0 && (c == b'-' || c == b'+')));
                match String::from_utf8_lossy(&t).parse::<i64>() {
                    Ok(n) => Value::Int(n),
                    Err(_) => {
                        failed = true;
                        Value::Null
                    }
                }
            }
            b'x' | b'X' => {
                let t = take(&mut i, &|c, _| c.is_ascii_hexdigit());
                match i64::from_str_radix(&String::from_utf8_lossy(&t), 16) {
                    Ok(n) => Value::Int(n),
                    Err(_) => {
                        failed = true;
                        Value::Null
                    }
                }
            }
            b'o' => {
                let t = take(&mut i, &|c, _| (b'0'..=b'7').contains(&c));
                Value::Int(i64::from_str_radix(&String::from_utf8_lossy(&t), 8).unwrap_or(0))
            }
            b'f' | b'e' | b'g' | b'E' => {
                let t = take(&mut i, &|c, k| c.is_ascii_digit() || c == b'.' || c == b'e' || c == b'E' || (k == 0 && (c == b'-' || c == b'+')));
                Value::Float(String::from_utf8_lossy(&t).parse().unwrap_or(0.0))
            }
            b's' => Value::bytes(take(&mut i, &|c, _| !c.is_ascii_whitespace())),
            b'c' => {
                let n = width.unwrap_or(1).min(input.len() - i);
                let v = input[i..i + n].to_vec();
                i += n;
                Value::bytes(v)
            }
            b'n' => Value::Int(i as i64),
            _ => Value::Null,
        };
        if !suppress {
            out.push(v);
        }
    }
    let _ = failed;
    Some(out)
}

// ---- registration --------------------------------------------------------------------------

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("header", |it, _, a| {
        if headers_already_sent(it, "") {
            return Ok(Value::Null);
        }
        let line = bstr(it, a, 0)?;
        let replace = !has(a, 1) || arg(a, 1).truthy();
        let code = if has(a, 2) { int(it, a, 2)? } else { 0 };
        set_header(it, &line, replace, code);
        Ok(Value::Null)
    });
    t.insert("header_remove", |it, _, a| {
        let mut r = it.response.borrow_mut();
        if has(a, 0) && !arg(a, 0).is_null() {
            let n = it.to_string_lossy(&arg(a, 0))?;
            r.headers.retain(|h| !h.split(':').next().is_some_and(|x| x.trim().eq_ignore_ascii_case(n.trim())));
        } else {
            r.headers.clear();
        }
        Ok(Value::Null)
    });
    t.insert("headers_list", |it, _, _| {
        Ok(Value::array(PhpArray::from_list(it.response.borrow().headers.iter().map(|h| Value::str(h)).collect::<Vec<_>>())))
    });
    t.insert("headers_sent", |it, _, a| {
        let started = it.output_started();
        if let Some((file, line)) = &started {
            if !a.is_empty() {
                a[0] = Value::str(file);
            }
            if a.len() > 1 {
                a[1] = Value::Int(*line as i64);
            }
        }
        Ok(Value::Bool(started.is_some()))
    });
    t.insert("http_response_code", |it, _, a| {
        let mut r = it.response.borrow_mut();
        let old = r.code;
        if has(a, 0) && !arg(a, 0).is_null() {
            r.code = it.to_int(&arg(a, 0))?;
            return Ok(if old == 0 { Value::Bool(true) } else { Value::Int(old) });
        }
        Ok(if old == 0 { Value::Bool(false) } else { Value::Int(old) })
    });
    t.insert("setcookie", |it, _, a| {
        if headers_already_sent(it, "") {
            return Ok(Value::Bool(false));
        }
        let h = cookie_header(it, a, false)?;
        it.response.borrow_mut().headers.push(h);
        Ok(Value::Bool(true))
    });
    t.insert("setrawcookie", |it, _, a| {
        let h = cookie_header(it, a, true)?;
        it.response.borrow_mut().headers.push(h);
        Ok(Value::Bool(true))
    });
    t.insert("header_register_callback", |_, _, _| Ok(Value::Bool(true)));
    t.insert("register_shutdown_function", |it, f, a| {
        let c = callable(it, f, a, 0)?;
        let rest = a.iter().skip(1).cloned().collect();
        it.response.borrow_mut().shutdown.push((c, rest));
        Ok(Value::Null)
    });
    t.insert("assert", |it, _, a| {
        if arg(a, 0).truthy() {
            return Ok(Value::Bool(true));
        }
        match arg(a, 1) {
            Value::Object(o) if o.class.is_a("throwable") => Err(crate::interp::Ctrl::Throw(o)),
            Value::Null => Err(it.error("AssertionError", "assert(false)")),
            m => Err(it.error("AssertionError", &it.to_string_lossy(&m)?)),
        }
    });
    t.insert("ignore_user_abort", |_, _, _| Ok(Value::Int(0)));
    t.insert("connection_status", |_, _, _| Ok(Value::Int(0)));
    t.insert("connection_aborted", |_, _, _| Ok(Value::Int(0)));
    t.insert("error_get_last", |_, _, _| Ok(Value::Null));
    t.insert("error_clear_last", |_, _, _| Ok(Value::Null));
    t.insert("restore_error_handler", |_, _, _| Ok(Value::Bool(true)));
    t.insert("restore_exception_handler", |_, _, _| Ok(Value::Bool(true)));
    t.insert("class_alias", |it, _, a| {
        let orig = bstr(it, a, 0)?;
        let alias = bstr(it, a, 1)?;
        let autoload = !has(a, 2) || arg(a, 2).truthy();
        let cls = if autoload { it.find_class(&orig)? } else { it.classes.borrow().get(&*it.lower.get(orig.trim_start_matches('\\'))).cloned() };
        let Some(cls) = cls else {
            it.warn(&format!("Class \"{orig}\" not found"));
            return Ok(Value::Bool(false));
        };
        let key = it.lower.get(alias.trim_start_matches('\\'));
        if it.classes.borrow().contains_key(&*key) {
            it.warn(&format!("Cannot declare class {alias}, because the name is already in use"));
            return Ok(Value::Bool(false));
        }
        it.classes.borrow_mut().insert(key, cls);
        Ok(Value::Bool(true))
    });
    t.insert("func_get_arg", |it, f, a| {
        let n = int(it, a, 0)?;
        match f.args.get(n.max(0) as usize) {
            Some(v) if n >= 0 => Ok(v.clone()),
            _ => Err(it.error("ValueError", "func_get_arg(): Argument #1 ($position) must be less than the number of the arguments passed to the currently executed function")),
        }
    });
    t.insert("debug_backtrace", |_, _, _| Ok(Value::array(PhpArray::new())));
    t.insert("debug_print_backtrace", |_, _, _| Ok(Value::Null));
    t.insert("php_uname", |it, _, a| {
        let mode = if has(a, 0) { bstr(it, a, 0)? } else { "a".into() };
        let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim().to_string();
        let rel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim().to_string();
        let ver = std::fs::read_to_string("/proc/sys/kernel/version").unwrap_or_default().trim().to_string();
        Ok(Value::str(&match mode.as_str() {
            "s" => "Linux".to_string(),
            "n" => host,
            "r" => rel,
            "v" => ver,
            "m" => std::env::consts::ARCH.to_string(),
            _ => format!("Linux {host} {rel} {ver} {}", std::env::consts::ARCH),
        }))
    });
    t.insert("getmypid", |_, _, _| Ok(Value::Int(std::process::id() as i64)));
    t.insert("getmyuid", |_, _, _| Ok(Value::Int(1000)));
    t.insert("getmygid", |_, _, _| Ok(Value::Int(1000)));
    t.insert("getmyinode", |_, _, _| Ok(Value::Int(0)));
    t.insert("get_current_user", |_, _, _| Ok(Value::str(&std::env::var("USER").unwrap_or_default())));
    t.insert("gethostname", |_, _, _| Ok(Value::str(std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim())));
    t.insert("gethostbyname", |it, _, a| {
        let h = bstr(it, a, 0)?;
        use std::net::ToSocketAddrs;
        Ok(Value::str(
            &(h.as_str(), 0)
                .to_socket_addrs()
                .ok()
                .and_then(|mut i| i.find(|a| a.is_ipv4()))
                .map(|a| a.ip().to_string())
                .unwrap_or(h),
        ))
    });
    t.insert("gethostbynamel", |it, _, a| {
        let h = bstr(it, a, 0)?;
        use std::net::ToSocketAddrs;
        Ok(match (h.as_str(), 0).to_socket_addrs() {
            Ok(i) => Value::array(PhpArray::from_list(i.filter(|a| a.is_ipv4()).map(|a| Value::str(&a.ip().to_string())).collect::<Vec<_>>())),
            Err(_) => Value::Bool(false),
        })
    });
    t.insert("setlocale", |it, _, a| {
        let l = if has(a, 1) {
            match arg(a, 1) {
                Value::Array(x) => x.values().next().map(|v| it.to_string_lossy(&v)).transpose()?.unwrap_or_default(),
                v => it.to_string_lossy(&v)?,
            }
        } else {
            String::new()
        };
        Ok(match l.as_str() {
            "" | "0" | "C" | "POSIX" => Value::str("C"),
            l if l.starts_with("C.") || l.eq_ignore_ascii_case("en_US.UTF-8") || l.eq_ignore_ascii_case("C.UTF-8") => Value::str(l),
            _ => Value::Bool(false),
        })
    });
    t.insert("localeconv", |_, _, _| {
        let mut m = PhpArray::new();
        for (k, v) in [("decimal_point", "."), ("thousands_sep", ""), ("int_curr_symbol", ""), ("currency_symbol", ""), ("mon_decimal_point", ""), ("mon_thousands_sep", ""), ("positive_sign", ""), ("negative_sign", "")] {
            m.insert(Key::Str(pstr(k)), Value::str(v));
        }
        Ok(Value::array(m))
    });
    t.insert("mail", |_, _, _| Ok(Value::Bool(false)));
    t.insert("escapeshellarg", |it, _, a| Ok(Value::str(&format!("'{}'", bstr(it, a, 0)?.replace('\'', "'\\''")))));
    t.insert("escapeshellcmd", |it, _, a| {
        let st = bstr(it, a, 0)?;
        let mut out = String::new();
        for c in st.chars() {
            if "#&;`|*?~<>^()[]{}$\\,\n\u{FF}'\"".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        Ok(Value::str(&out))
    });
    t.insert("putenv", |it, _, a| {
        let st = bstr(it, a, 0)?;
        // SAFETY: phpx runs PHP on one OS thread; no concurrent env readers.
        match st.split_once('=') {
            Some((k, v)) => unsafe { std::env::set_var(k, v) },
            None => unsafe { std::env::remove_var(&st) },
        }
        Ok(Value::Bool(true))
    });

    // array internal pointer
    t.insert("next", |it, _, a| ptr_move(it, a, 0));
    t.insert("prev", |it, _, a| ptr_move(it, a, 1));
    t.insert("reset", |it, _, a| ptr_move(it, a, 2));
    t.insert("end", |it, _, a| ptr_move(it, a, 3));
    t.insert("current", |_, _, a| Ok(match arg(a, 0) {
        Value::Array(x) => x.get_index(x.pos.get()).map(|(_, v)| v).unwrap_or(Value::Bool(false)),
        _ => Value::Bool(false),
    }));
    t.insert("pos", |_, _, a| Ok(match arg(a, 0) {
        Value::Array(x) => x.get_index(x.pos.get()).map(|(_, v)| v).unwrap_or(Value::Bool(false)),
        _ => Value::Bool(false),
    }));
    t.insert("key", |_, _, a| Ok(match arg(a, 0) {
        Value::Array(x) => x.get_index(x.pos.get()).map(|(k, _)| k.to_value()).unwrap_or(Value::Null),
        _ => Value::Null,
    }));

    t.insert("array_replace_recursive", |it, _, a| {
        fn merge(base: &PhpArray, over: &PhpArray) -> PhpArray {
            let mut out = base.clone();
            for (k, v) in over.iter() {
                match (out.get(&k), &v) {
                    (Some(Value::Array(b)), Value::Array(o)) => {
                        let m = merge(&b, o);
                        out.insert(k, Value::array(m));
                    }
                    _ => out.insert(k, v),
                }
            }
            out
        }
        let mut acc = (*crate::builtins::arr(it, a, 0, "array_replace_recursive")?).clone();
        for i in 1..a.len() {
            let o = crate::builtins::arr(it, a, i, "array_replace_recursive")?;
            acc = merge(&acc, &o);
        }
        Ok(Value::array(acc))
    });
    t.insert("array_walk_recursive", |it, f, a| {
        let c = callable(it, f, a, 1)?;
        let extra = if has(a, 2) { vec![arg(a, 2)] } else { vec![] };
        fn walk(it: &Interp, c: &crate::interp::Callable, v: &Value, extra: &[Value]) -> R<()> {
            if let Value::Array(x) = v {
                for (k, v) in x.iter() {
                    if matches!(v, Value::Array(_)) {
                        walk(it, c, &v, extra)?;
                    } else {
                        let mut args = vec![v, k.to_value()];
                        args.extend(extra.iter().cloned());
                        it.call_callable(c, args, vec![])?;
                    }
                }
            }
            Ok(())
        }
        walk(it, &c, &arg(a, 0), &extra)?;
        Ok(Value::Bool(true))
    });

    // HTML entities
    t.insert("html_entity_decode", |it, _, a| {
        let flags = if has(a, 1) { int(it, a, 1)? } else { 3 | 8 };
        Ok(Value::bytes(decode_entities(&s(it, a, 0)?, flags)))
    });
    t.insert("htmlentities", |it, _, a| {
        let flags = if has(a, 1) { int(it, a, 1)? } else { 3 | 8 };
        let double = !has(a, 3) || arg(a, 3).truthy();
        Ok(Value::bytes(encode_entities(&s(it, a, 0)?, flags, double)))
    });
    t.insert("get_html_translation_table", |it, _, a| {
        let table = if has(a, 0) { int(it, a, 0)? } else { 0 };
        let flags = if has(a, 1) { int(it, a, 1)? } else { 3 | 8 };
        let mut m = PhpArray::new();
        let mut put = |c: &str, e: &str| {
            m.insert(Key::Str(pstr(c)), Value::str(e));
        };
        if flags & 2 != 0 {
            put("\"", "&quot;");
        }
        put("&", "&amp;");
        if flags & 3 == 3 {
            put("'", "&#039;");
        }
        put("<", "&lt;");
        put(">", "&gt;");
        if table == 1 {
            for (name, cp) in ENTITIES {
                put(&char::from_u32(*cp).unwrap().to_string(), &format!("&{name};"));
            }
        }
        Ok(Value::array(m))
    });

    // class introspection
    t.insert("get_class_methods", |it, f, a| {
        let cls = match arg(a, 0) {
            Value::Object(o) => Some(o.class.clone()),
            v => it.find_class(&it.to_string_lossy(&v)?)?,
        };
        let Some(cls) = cls else {
            return Err(it.error("TypeError", "get_class_methods(): Argument #1 ($object_or_class) must be an object or a valid class name"));
        };
        let mut names: Vec<(usize, Rc<str>)> = vec![];
        for (i, m) in cls.method_order().iter().enumerate() {
            if it.member_visible(f.class.as_deref(), m.vis, &m.declaring) {
                names.push((i, m.name.clone()));
            }
        }
        Ok(Value::array(PhpArray::from_list(names.into_iter().map(|(_, n)| Value::str(&n)).collect::<Vec<_>>())))
    });
    t.insert("get_class_vars", |it, f, a| {
        let n = bstr(it, a, 0)?;
        let Some(cls) = it.find_class(&n)? else { return Ok(Value::Bool(false)) };
        let mut m = PhpArray::new();
        for p in &cls.props {
            if it.member_visible(f.class.as_deref(), p.vis, &p.declaring) {
                let v = it.prop_default(&cls, p)?;
                m.insert(Key::Str(pstr(&p.name)), v.unwrap_or(Value::Null));
            }
        }
        for (name, sp) in &cls.static_props {
            if it.member_visible(f.class.as_deref(), sp.vis, &sp.declaring) {
                m.insert(Key::Str(pstr(name)), it.static_prop_slot(sp)?);
            }
        }
        Ok(Value::array(m))
    });
    t.insert("class_parents", |it, _, a| {
        let cls = match arg(a, 0) {
            Value::Object(o) => Some(o.class.clone()),
            v => it.find_class(&it.to_string_lossy(&v)?)?,
        };
        let Some(cls) = cls else { return Ok(Value::Bool(false)) };
        let mut m = PhpArray::new();
        let mut p = cls.parent.clone();
        while let Some(c) = p {
            m.insert(Key::Str(pstr(&c.name)), Value::str(&c.name));
            p = c.parent.clone();
        }
        Ok(Value::array(m))
    });
    t.insert("class_uses", |it, _, a| {
        let cls = match arg(a, 0) {
            Value::Object(o) => Some(o.class.clone()),
            v => it.find_class(&it.to_string_lossy(&v)?)?,
        };
        let Some(cls) = cls else { return Ok(Value::Bool(false)) };
        let mut m = PhpArray::new();
        for t in it.class_traits(&cls) {
            m.insert(Key::Str(pstr(&t)), Value::str(&t));
        }
        Ok(Value::array(m))
    });

    // printf family
    t.insert("vprintf", |it, _, a| {
        let mut args = vec![arg(a, 0)];
        if let Value::Array(x) = arg(a, 1) {
            args.extend(x.values());
        }
        let out = crate::strings::sprintf(it, &args)?;
        it.echo(&out);
        Ok(Value::Int(out.len() as i64))
    });
    t.insert("fprintf", |it, f, a| {
        let out = crate::strings::sprintf(it, &a[1..])?;
        let mut w = vec![arg(a, 0), Value::bytes(out.clone())];
        (it.builtins["fwrite"])(it, f, &mut w)?;
        Ok(Value::Int(out.len() as i64))
    });
    t.insert("vfprintf", |it, f, a| {
        let mut args = vec![arg(a, 1)];
        if let Value::Array(x) = arg(a, 2) {
            args.extend(x.values());
        }
        let out = crate::strings::sprintf(it, &args)?;
        let mut w = vec![arg(a, 0), Value::bytes(out.clone())];
        (it.builtins["fwrite"])(it, f, &mut w)?;
        Ok(Value::Int(out.len() as i64))
    });
    t.insert("sscanf", |it, _, a| {
        let vals = sscanf(&s(it, a, 0)?, &s(it, a, 1)?).unwrap_or_default();
        if a.len() > 2 {
            let n = vals.iter().filter(|v| !v.is_null()).count();
            for (i, v) in vals.into_iter().enumerate() {
                if 2 + i < a.len() {
                    a[2 + i] = v;
                }
            }
            return Ok(Value::Int(n as i64));
        }
        if vals.is_empty() {
            return Ok(Value::Int(-1));
        }
        Ok(Value::array(PhpArray::from_list(vals)))
    });
    t.insert("similar_text", |it, _, a| {
        let (x, y) = (s(it, a, 0)?, s(it, a, 1)?);
        let sim = similar(&x, &y);
        if a.len() > 2 {
            let total = x.len() + y.len();
            a[2] = Value::Float(if total == 0 { 0.0 } else { sim as f64 * 2.0 * 100.0 / total as f64 });
        }
        Ok(Value::Int(sim as i64))
    });
    t.insert("levenshtein", |it, _, a| {
        let (x, y) = (s(it, a, 0)?, s(it, a, 1)?);
        let (ins, rep, del) = if has(a, 2) { (int(it, a, 2)?, int(it, a, 3)?, int(it, a, 4)?) } else { (1, 1, 1) };
        Ok(Value::Int(levenshtein(&x, &y, ins, rep, del)))
    });
    t.insert("soundex", |it, _, a| Ok(Value::str(&soundex(&s(it, a, 0)?))));
    t.insert("srand", |it, _, a| {
        let _ = (it, a);
        Ok(Value::Null)
    });
    t.insert("openssl_random_pseudo_bytes", |it, _, a| {
        let n = int(it, a, 0)?.max(0) as usize;
        if a.len() > 1 {
            a[1] = Value::Bool(true);
        }
        Ok(Value::bytes(crate::builtins::secure_bytes(n)))
    });

    // passwords
    t.insert("password_hash", |it, _, a| {
        let pw = s(it, a, 0)?;
        let mut cost = 12;
        if let Value::Array(o) = arg(a, 2) {
            if let Some(c) = o.get(&Key::Str(pstr("cost"))) {
                cost = it.to_int(&c)? as u32;
            }
        }
        if !(4..=31).contains(&cost) {
            return Err(it.error("ValueError", &format!("password_hash(): Argument #3 ($options) must contain a \"cost\" value between 4 and 31")));
        }
        match bcrypt::hash(&pw[..pw.len().min(72)], cost) {
            Ok(h) => Ok(Value::str(&h.replacen("$2b$", "$2y$", 1))),
            Err(_) => Ok(Value::Bool(false)),
        }
    });
    t.insert("password_verify", |it, _, a| {
        let pw = s(it, a, 0)?;
        let h = bstr(it, a, 1)?;
        if !h.starts_with("$2") {
            return Ok(Value::Bool(false));
        }
        Ok(Value::Bool(bcrypt::verify(&pw[..pw.len().min(72)], &h.replacen("$2y$", "$2b$", 1)).unwrap_or(false)))
    });
    t.insert("password_needs_rehash", |it, _, a| {
        let h = bstr(it, a, 0)?;
        let mut cost = 12;
        if let Value::Array(o) = arg(a, 2) {
            if let Some(c) = o.get(&Key::Str(pstr("cost"))) {
                cost = it.to_int(&c)?;
            }
        }
        Ok(Value::Bool(!(h.starts_with("$2y$") && h.get(4..6).and_then(|c| c.parse::<i64>().ok()) == Some(cost))))
    });
    t.insert("password_get_info", |it, _, a| {
        let h = bstr(it, a, 0)?;
        let mut m = PhpArray::new();
        let bc = h.starts_with("$2y$") && h.len() == 60;
        m.insert(Key::Str(pstr("algo")), if bc { Value::str("2y") } else { Value::Null });
        m.insert(Key::Str(pstr("algoName")), Value::str(if bc { "bcrypt" } else { "unknown" }));
        let mut o = PhpArray::new();
        if bc {
            o.insert(Key::Str(pstr("cost")), Value::Int(h[4..6].parse().unwrap_or(10)));
        }
        m.insert(Key::Str(pstr("options")), Value::array(o));
        Ok(Value::array(m))
    });
    t.insert("crypt", |it, _, a| {
        let pw = s(it, a, 0)?;
        let salt = bstr(it, a, 1)?;
        if salt.starts_with("$2") && salt.len() >= 29 {
            let cost: u32 = salt[4..6].parse().unwrap_or(10);
            let raw = &salt[7..29];
            if let Ok(parts) = bcrypt::hash_with_salt(&pw[..pw.len().min(72)], cost, decode_bcrypt_salt(raw)) {
                let prefix = &salt[..4];
                return Ok(Value::str(&format!("{prefix}{}", &parts.format_for_version(bcrypt::Version::TwoB)[4..])));
            }
        }
        Ok(Value::str("*0"))
    });

    // zlib
    t.insert("gzinflate", |it, _, a| Ok(inflate_with(&s(it, a, 0)?, 0).map(Value::bytes).unwrap_or_else(|| {
        it.warn("gzinflate(): data error");
        Value::Bool(false)
    })));
    t.insert("gzuncompress", |it, _, a| Ok(inflate_with(&s(it, a, 0)?, 1).map(Value::bytes).unwrap_or_else(|| {
        it.warn("gzuncompress(): data error");
        Value::Bool(false)
    })));
    t.insert("gzdecode", |it, _, a| Ok(inflate_with(&s(it, a, 0)?, 2).map(Value::bytes).unwrap_or_else(|| {
        it.warn("gzdecode(): data error");
        Value::Bool(false)
    })));
    t.insert("zlib_decode", |it, _, a| {
        let d = s(it, a, 0)?;
        let kind = if d.starts_with(&[0x1f, 0x8b]) { 2 } else if d.first().is_some_and(|b| b & 0x0f == 8) { 1 } else { 0 };
        Ok(inflate_with(&d, kind).map(Value::bytes).unwrap_or(Value::Bool(false)))
    });
    t.insert("gzdeflate", |it, _, a| Ok(Value::bytes(deflate_with(&s(it, a, 0)?, if has(a, 1) { int(it, a, 1)? } else { -1 }, 0))));
    t.insert("gzcompress", |it, _, a| Ok(Value::bytes(deflate_with(&s(it, a, 0)?, if has(a, 1) { int(it, a, 1)? } else { -1 }, 1))));
    t.insert("gzencode", |it, _, a| Ok(Value::bytes(deflate_with(&s(it, a, 0)?, if has(a, 1) { int(it, a, 1)? } else { -1 }, 2))));
    t.insert("zlib_encode", |it, _, a| {
        let enc = int(it, a, 1)?;
        let kind = match enc {
            -15 => 0,
            31 => 2,
            _ => 1,
        };
        Ok(Value::bytes(deflate_with(&s(it, a, 0)?, if has(a, 2) { int(it, a, 2)? } else { -1 }, kind)))
    });

    // CSV
    t.insert("str_getcsv", |it, _, a| {
        let (sep, enc, esc) = csv_opts(it, a, 1, "str_getcsv")?;
        Ok(Value::array(PhpArray::from_list(csv_parse(&s(it, a, 0)?, sep, enc, esc))))
    });
    t.insert("fputcsv", |it, f, a| {
        let fields: Vec<Vec<u8>> = match arg(a, 1) {
            Value::Array(x) => x.values().map(|v| it.to_str(&v).map(|b| b.to_vec())).collect::<R<_>>()?,
            _ => vec![],
        };
        let (sep, enc, esc) = csv_opts(it, a, 2, "fputcsv")?;
        let eol = if has(a, 5) { s(it, a, 5)?.to_vec() } else { b"\n".to_vec() };
        let line = csv_line(&fields, sep, enc, esc, &eol);
        let n = line.len();
        let mut w = vec![arg(a, 0), Value::bytes(line)];
        (it.builtins["fwrite"])(it, f, &mut w)?;
        Ok(Value::Int(n as i64))
    });
    t.insert("fgetcsv", |it, f, a| {
        let (sep, enc, esc) = csv_opts(it, a, 2, "fgetcsv")?;
        // Read a logical line: keep reading while inside an open enclosure.
        let mut line: Vec<u8> = vec![];
        loop {
            let mut r = vec![arg(a, 0)];
            match (it.builtins["fgets"])(it, f, &mut r)? {
                Value::Str(b) => line.extend_from_slice(&b),
                _ => break,
            }
            let quotes = line.iter().filter(|&&c| c == enc).count();
            if quotes % 2 == 0 {
                break;
            }
        }
        if line.is_empty() {
            return Ok(Value::Bool(false));
        }
        Ok(Value::array(PhpArray::from_list(csv_parse(&line, sep, enc, esc))))
    });

    t.insert("filter_var", |it, _, a| {
        let filter = if has(a, 1) { int(it, a, 1)? } else { 516 };
        filter_value(it, &arg(a, 0), filter, &arg(a, 2))
    });
    t.insert("filter_input", |_, _, _| Ok(Value::Null));
    t.insert("filter_has_var", |_, _, _| Ok(Value::Bool(false)));
    t.insert("filter_list", |_, _, _| {
        Ok(Value::array(PhpArray::from_list(["int", "boolean", "bool", "float", "validate_regexp", "validate_url", "validate_email", "validate_ip", "string", "unsafe_raw", "email", "url", "number_int", "number_float", "callback"].map(Value::str))))
    });
    t.insert("filter_id", |_, _, _| Ok(Value::Bool(false)));

    t.insert("is_writeable", |it, _, a| {
        let p = bstr(it, a, 0)?;
        Ok(Value::Bool(std::fs::metadata(&p).is_ok_and(|m| !m.permissions().readonly())))
    });
    t.insert("time_nanosleep", |it, _, a| {
        let d = std::time::Duration::from_secs(int(it, a, 0)?.max(0) as u64) + std::time::Duration::from_nanos(int(it, a, 1)?.max(0) as u64);
        phpx_runtime::sleep(d).map_err(|e| crate::native::rt_error(it, e))?;
        Ok(Value::Bool(true))
    });
    t.insert("get_include_path", |it, _, _| Ok(Value::str(&it.include_path.borrow())));
    t.insert("set_include_path", |it, _, a| {
        let old = it.include_path.borrow().clone();
        let p = bstr(it, a, 0)?;
        it.ini.borrow_mut().set("include_path", &p);
        *it.include_path.borrow_mut() = p;
        Ok(Value::str(&old))
    });
    t.insert("stream_resolve_include_path", |it, _, a| {
        let p = bstr(it, a, 0)?;
        if std::path::Path::new(&p).exists() {
            return Ok(Value::str(&std::fs::canonicalize(&p).map(|x| x.display().to_string()).unwrap_or(p)));
        }
        for dir in it.include_path.borrow().split(':') {
            let cand = std::path::Path::new(dir).join(&p);
            if cand.exists() {
                return Ok(Value::str(&cand.display().to_string()));
            }
        }
        Ok(Value::Bool(false))
    });
    t.insert("getrusage", |_, _, _| {
        let mut m = PhpArray::new();
        for k in ["ru_utime.tv_sec", "ru_utime.tv_usec", "ru_stime.tv_sec", "ru_stime.tv_usec", "ru_maxrss"] {
            m.insert(Key::Str(pstr(k)), Value::Int(0));
        }
        Ok(Value::array(m))
    });
    t.insert("sys_getloadavg", |_, _, _| {
        let s = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
        let v: Vec<Value> = s.split_whitespace().take(3).map(|x| Value::Float(x.parse().unwrap_or(0.0))).collect();
        Ok(Value::array(PhpArray::from_list(v)))
    });

    // user-callback set operations
    t.insert("array_udiff", |it, f, a| set_op(it, f, a, false, true));
    t.insert("array_uintersect", |it, f, a| set_op(it, f, a, true, true));
    t.insert("array_diff_ukey", |it, f, a| set_op(it, f, a, false, false));
    t.insert("array_intersect_ukey", |it, f, a| set_op(it, f, a, true, false));
}

/// next / prev / reset / end (by-ref array; moves its internal pointer).
fn ptr_move(it: &Interp, a: &mut [Value], kind: u8) -> R<Value> {
    let Some(Value::Array(mut x)) = a.first().cloned() else {
        return match a.first() {
            Some(Value::Object(_)) => Ok(Value::Bool(false)),
            other => Err(it.error(
                "TypeError",
                &format!("Argument #1 ($array) must be of type array, {} given", other.map(|v| v.debug_type()).unwrap_or("null".into())),
            )),
        };
    };
    let len = x.len();
    let p = x.pos.get();
    let np = match kind {
        0 => if p >= len { p } else { p + 1 },
        1 => if p == 0 || p >= len { len } else { p - 1 },
        2 => 0,
        _ => len.saturating_sub(1),
    };
    if np != p {
        Rc::make_mut(&mut x).pos.set(np);
    }
    let v = x.get_index(np).map(|(_, v)| v).unwrap_or(Value::Bool(false));
    a[0] = Value::Array(x);
    Ok(v)
}

/// array_udiff / array_uintersect (compare values) and the *_ukey variants
/// (compare keys) with a user comparator (last argument).
fn set_op(it: &Interp, f: &mut crate::interp::Frame, a: &mut Vec<Value>, keep_common: bool, by_value: bool) -> R<Value> {
    let c = callable(it, f, a, a.len() - 1)?;
    let base = crate::builtins::arr(it, a, 0, "array_udiff")?;
    let others: Vec<Rc<PhpArray>> = (1..a.len() - 1).map(|i| crate::builtins::arr(it, a, i, "array_udiff")).collect::<R<_>>()?;
    let mut out = PhpArray::new();
    for (k, v) in base.iter() {
        let probe = if by_value { v.clone() } else { k.to_value() };
        let mut in_all = true;
        let mut in_any = false;
        for o in &others {
            let mut found = false;
            for (k2, v2) in o.iter() {
                let other = if by_value { v2 } else { k2.to_value() };
                let r = it.call_callable(&c, vec![probe.clone(), other], vec![])?;
                if it.to_int(&r)? == 0 {
                    found = true;
                    break;
                }
            }
            in_all &= found;
            in_any |= found;
        }
        if (keep_common && in_all) || (!keep_common && !in_any) {
            out.insert(k, v);
        }
    }
    Ok(Value::array(out))
}

fn decode_bcrypt_salt(s: &str) -> [u8; 16] {
    const ALPHA: &[u8] = b"./ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut bits: u32 = 0;
    let mut n = 0;
    let mut out = vec![];
    for c in s.bytes() {
        let v = ALPHA.iter().position(|&x| x == c).unwrap_or(0) as u32;
        bits = bits << 6 | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    let mut salt = [0u8; 16];
    for (i, b) in out.into_iter().take(16).enumerate() {
        salt[i] = b;
    }
    salt
}
