//! glob(): wildcards in any path component, {a,b} braces, and PHP's
//! GLOB_* flags.

// Values as PHP defines them (consts_table.rs).
const GLOB_MARK: i64 = 8;
const GLOB_NOSORT: i64 = 32;
const GLOB_NOCHECK: i64 = 16;
const GLOB_NOESCAPE: i64 = 4096;
const GLOB_BRACE: i64 = 128;
const GLOB_ONLYDIR: i64 = 1 << 30;

fn has_magic(s: &str, noescape: bool) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' if !noescape => i += 1,
            b'*' | b'?' | b'[' => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `a{b,c}d` -> [abd, acd] (nested braces too).
fn expand_braces(p: &str) -> Vec<String> {
    let b = p.as_bytes();
    let mut depth = 0;
    let mut open = None;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'{' => {
                if depth == 0 {
                    open = Some(i);
                }
                depth += 1;
            }
            b'}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    let o = open.unwrap();
                    // Split the alternatives at top-level commas.
                    let mut alts = vec![];
                    let (mut d, mut start) = (0, o + 1);
                    for j in o + 1..i {
                        match b[j] {
                            b'{' => d += 1,
                            b'}' => d -= 1,
                            b',' if d == 0 => {
                                alts.push(&p[start..j]);
                                start = j + 1;
                            }
                            _ => {}
                        }
                    }
                    alts.push(&p[start..i]);
                    let (pre, post) = (&p[..o], &p[i + 1..]);
                    return alts.iter().flat_map(|a| expand_braces(&format!("{pre}{a}{post}"))).collect();
                }
            }
            _ => {}
        }
    }
    vec![p.to_string()]
}

fn one(pattern: &str, flags: i64) -> Vec<String> {
    let noescape = flags & GLOB_NOESCAPE != 0;
    let absolute = pattern.starts_with('/');
    let comps: Vec<&str> = pattern.split('/').filter(|c| !c.is_empty()).collect();
    // Paths matched so far ("" = the current directory for relative patterns).
    let mut cur: Vec<String> = vec![if absolute { "/".to_string() } else { String::new() }];
    for (ci, comp) in comps.iter().enumerate() {
        let last = ci + 1 == comps.len();
        let mut next = vec![];
        for base in &cur {
            let join = |name: &str| if base.is_empty() { name.to_string() } else if base == "/" { format!("/{name}") } else { format!("{base}/{name}") };
            if !has_magic(comp, noescape) {
                let name = if noescape { comp.to_string() } else { unescape(comp) };
                let p = join(&name);
                if !last || std::fs::symlink_metadata(&p).is_ok() {
                    next.push(p);
                }
                continue;
            }
            let dir = if base.is_empty() { "." } else { base.as_str() };
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            if comp.starts_with('.') {
                // readdir() also lists . and ..
                names.push(".".into());
                names.push("..".into());
            } else {
                names.retain(|n| !n.starts_with('.'));
            }
            for n in names {
                let pat = if noescape { comp.replace('\\', "\\\\") } else { comp.to_string() };
                if crate::streams::fnmatch(pat.as_bytes(), n.as_bytes()) {
                    let p = join(&n);
                    if last || std::fs::metadata(&p).is_ok_and(|m| m.is_dir()) {
                        next.push(p);
                    }
                }
            }
        }
        cur = next;
        if cur.is_empty() {
            break;
        }
    }
    if comps.is_empty() {
        cur = if absolute { vec!["/".to_string()] } else { vec![] };
    }
    if flags & GLOB_ONLYDIR != 0 {
        cur.retain(|p| std::fs::metadata(p).is_ok_and(|m| m.is_dir()));
    }
    if flags & GLOB_NOSORT == 0 {
        cur.sort();
    }
    if flags & GLOB_MARK != 0 {
        for p in &mut cur {
            if !p.ends_with('/') && std::fs::metadata(&*p).is_ok_and(|m| m.is_dir()) {
                p.push('/');
            }
        }
    }
    cur
}

pub fn glob(pattern: &str, flags: i64) -> Vec<String> {
    let patterns = if flags & GLOB_BRACE != 0 { expand_braces(pattern) } else { vec![pattern.to_string()] };
    let mut out: Vec<String> = vec![];
    for p in patterns {
        for m in one(&p, flags) {
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    if out.is_empty() && flags & GLOB_NOCHECK != 0 {
        out.push(pattern.to_string());
    }
    out
}
