//! mb_convert_case() and friends the way PHP's mbstring does them: full
//! and simple mappings, titlecase (a letter is titled unless it follows a
//! cased one, case-ignorable characters in between), final sigma, and
//! case folding. Property tables come from mbcase_table.rs.

use crate::mbcase_table::{CASED, CASE_IGNORABLE, SIMPLE_FOLD, SIMPLE_UPPER, TITLE};

fn in_ranges(t: &[(u32, u32)], c: char) -> bool {
    let c = c as u32;
    t.binary_search_by(|&(a, b)| {
        if c < a {
            std::cmp::Ordering::Greater
        } else if c > b {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

fn lookup(t: &[(u32, u32)], c: char) -> Option<char> {
    t.binary_search_by_key(&(c as u32), |e| e.0).ok().and_then(|i| char::from_u32(t[i].1))
}

pub fn is_cased(c: char) -> bool {
    in_ranges(CASED, c)
}

pub fn is_case_ignorable(c: char) -> bool {
    in_ranges(CASE_IGNORABLE, c)
}

fn single(mut it: impl Iterator<Item = char>, c: char) -> char {
    match (it.next(), it.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

pub fn upper_simple(c: char) -> char {
    lookup(SIMPLE_UPPER, c).unwrap_or_else(|| single(c.to_uppercase(), c))
}

pub fn lower_simple(c: char) -> char {
    // İ's full lowercase is i + combining dot; its simple one is i.
    c.to_lowercase().next().unwrap_or(c)
}

pub fn title_simple(c: char) -> char {
    lookup(TITLE, c).unwrap_or_else(|| upper_simple(c))
}

fn title_full(c: char, out: &mut String) {
    if let Some(t) = lookup(TITLE, c) {
        out.push(t);
        return;
    }
    // Multi-character uppercase (ß, ligatures, ŉ): titled up to its first
    // cased letter, lower after (SpecialCasing's title column).
    let mut titled = false;
    for u in c.to_uppercase() {
        if titled {
            out.extend(u.to_lowercase());
        } else {
            out.push(u);
            titled = is_cased(u);
        }
    }
}

fn fold_simple(c: char) -> char {
    lookup(SIMPLE_FOLD, c).unwrap_or_else(|| lower_simple(c))
}

fn fold_full(c: char, out: &mut String) {
    // Full folding expands what uppercases to several letters (ß -> ss,
    // also via its simple fold: ẞ -> ß -> ss).
    let f = if c == '\u{130}' { c } else { fold_simple(c) };
    if f.to_uppercase().count() > 1 || f == '\u{130}' {
        for u in f.to_uppercase() {
            out.extend(u.to_lowercase());
        }
        return;
    }
    out.push(f);
}

/// Σ at the end of a word lowercases to ς: a cased letter before it and
/// none after (case-ignorable characters skipped).
fn final_sigma(chars: &[char], i: usize) -> bool {
    let before = chars[..i].iter().rev().find(|c| !is_case_ignorable(**c)).is_some_and(|c| is_cased(*c));
    let after = chars[i + 1..].iter().find(|c| !is_case_ignorable(**c)).is_some_and(|c| is_cased(*c));
    before && !after
}

fn lower_full_into(chars: &[char], i: usize, out: &mut String) {
    let c = chars[i];
    if c == 'Σ' && final_sigma(chars, i) {
        out.push('ς');
    } else {
        out.extend(c.to_lowercase());
    }
}

/// mb_convert_case() on text (modes as MB_CASE_*).
pub fn convert(s: &str, mode: i64) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    match mode {
        0 => out.extend(chars.iter().flat_map(|c| c.to_uppercase())),
        1 => {
            for i in 0..chars.len() {
                lower_full_into(&chars, i, &mut out);
            }
        }
        3 => {
            for &c in &chars {
                fold_full(c, &mut out);
            }
        }
        4 => out.extend(chars.iter().map(|&c| upper_simple(c))),
        5 => out.extend(chars.iter().map(|&c| lower_simple(c))),
        7 => out.extend(chars.iter().map(|&c| fold_simple(c))),
        2 | 6 => {
            let full = mode == 2;
            let mut title_mode = true;
            for (i, &c) in chars.iter().enumerate() {
                if title_mode {
                    if full {
                        title_full(c, &mut out);
                    } else {
                        out.push(title_simple(c));
                    }
                } else if full {
                    lower_full_into(&chars, i, &mut out);
                } else {
                    out.push(lower_simple(c));
                }
                if !is_case_ignorable(c) {
                    title_mode = !is_cased(c);
                }
            }
        }
        _ => out.push_str(s),
    }
    out
}
