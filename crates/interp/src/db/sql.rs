//! PDO's placeholder parsing: `?` and `:name` outside quotes and comments.

#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(usize, usize),
    Positional,
    Named(String),
}

/// Split `sql` into text and placeholders.
pub fn scan(sql: &[u8]) -> Vec<Piece> {
    let mut out = vec![];
    let (mut i, mut start) = (0, 0);
    let flush = |out: &mut Vec<Piece>, a: usize, b: usize| {
        if b > a {
            out.push(Piece::Text(a, b));
        }
    };
    while i < sql.len() {
        let c = sql[i];
        match c {
            b'\'' | b'"' | b'`' => {
                // Quoted: skip to the matching quote (doubled or escaped).
                i += 1;
                while i < sql.len() {
                    if sql[i] == b'\\' && c != b'`' {
                        i += 2;
                        continue;
                    }
                    if sql[i] == c {
                        if sql.get(i + 1) == Some(&c) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' if sql.get(i + 1) == Some(&b'-') => {
                while i < sql.len() && sql[i] != b'\n' {
                    i += 1;
                }
            }
            b'#' => {
                while i < sql.len() && sql[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if sql.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < sql.len() && !(sql[i] == b'*' && sql[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            b'?' => {
                // `??` is an escaped literal question mark (PHP 8.4+).
                if sql.get(i + 1) == Some(&b'?') {
                    flush(&mut out, start, i + 1);
                    i += 2;
                    start = i;
                    continue;
                }
                flush(&mut out, start, i);
                out.push(Piece::Positional);
                i += 1;
                start = i;
            }
            b':' if sql.get(i + 1).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') && (i == 0 || sql[i - 1] != b':') => {
                let mut j = i + 1;
                while j < sql.len() && (sql[j].is_ascii_alphanumeric() || sql[j] == b'_') {
                    j += 1;
                }
                flush(&mut out, start, i);
                out.push(Piece::Named(crate::pstr::lossy(&sql[i..j]).into_owned()));
                i = j;
                start = i;
            }
            b':' if sql.get(i + 1) == Some(&b':') => i += 2,
            _ => i += 1,
        }
    }
    flush(&mut out, start, sql.len());
    out
}

/// MySQL's string escaping (mysql_real_escape_string) inside quotes.
pub fn mysql_quote(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 2);
    out.push(b'\'');
    for &c in s {
        match c {
            0 => out.extend_from_slice(b"\\0"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\'' => out.extend_from_slice(b"\\'"),
            b'"' => out.extend_from_slice(b"\\\""),
            0x1a => out.extend_from_slice(b"\\Z"),
            c => out.push(c),
        }
    }
    out.push(b'\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        let sql = b"select * from t where a = ? and b = :name and c = '?:x' -- ?\n and d::int = :d2";
        let p = scan(sql);
        let marks: Vec<&Piece> = p.iter().filter(|x| !matches!(x, Piece::Text(..))).collect();
        assert_eq!(marks, vec![&Piece::Positional, &Piece::Named(":name".into()), &Piece::Named(":d2".into())]);
    }
}
