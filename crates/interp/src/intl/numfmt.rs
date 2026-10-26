//! NumberFormatter: ICU's DecimalFormat patterns (#,##0.00;(#,##0.00),
//! ¤, %, ‰, E0, @@#) applied to exact decimal digits, with each locale's
//! symbols, patterns, numbering system and currency data from
//! locales_table.rs (generated from PHP's own ICU).

use super::locales_table::{CURRENCY_DIGITS, CURRENCY_SYMBOLS, LOCALES};

pub struct LocaleData {
    pub name: &'static str,
    /// DECIMAL, PERCENT, CURRENCY, CURRENCY_ACCOUNTING, SCIENTIFIC.
    pub patterns: [&'static str; 5],
    /// Decimal, grouping, minus, plus, percent, permill, exponential,
    /// infinity, NaN, monetary decimal, monetary grouping, zero digit.
    pub symbols: [&'static str; 12],
    /// Default currency ("XXX": none).
    pub currency: &'static str,
    /// ICU's minimum grouping digits.
    pub min_grouping: u8,
}

pub const SYM_DECIMAL: usize = 0;
pub const SYM_GROUP: usize = 1;
pub const SYM_MINUS: usize = 2;
pub const SYM_PLUS: usize = 3;
pub const SYM_PERCENT: usize = 4;
pub const SYM_PERMILL: usize = 5;
pub const SYM_EXP: usize = 6;
pub const SYM_INF: usize = 7;
pub const SYM_NAN: usize = 8;
pub const SYM_MONEY_DECIMAL: usize = 9;
pub const SYM_MONEY_GROUP: usize = 10;
pub const SYM_ZERO: usize = 11;

fn find(name: &str) -> Option<&'static LocaleData> {
    LOCALES.binary_search_by(|l| l.name.cmp(name)).ok().map(|i| &LOCALES[i])
}

/// ICU-style canonical form: `en-us@currency=EUR` -> (`en_US`, keywords).
pub fn canonicalize(locale: &str) -> (String, String) {
    let (base, kw) = match locale.split_once('@') {
        Some((b, k)) => (b, k.to_string()),
        None => (locale, String::new()),
    };
    let base = base.split('.').next().unwrap_or("");
    let mut out: Vec<String> = vec![];
    for (i, part) in base.split(['_', '-']).filter(|p| !p.is_empty()).enumerate() {
        out.push(if i == 0 {
            part.to_ascii_lowercase()
        } else if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut s = part.to_ascii_lowercase();
            s[..1].make_ascii_uppercase();
            s
        } else {
            part.to_ascii_uppercase()
        });
    }
    (out.join("_"), kw)
}

/// The locale data for `locale` (falling back subtag by subtag), and the
/// name it resolved to. None: ICU has no data for its language.
pub fn resolve(locale: &str) -> Option<&'static LocaleData> {
    let (mut name, _) = canonicalize(locale);
    loop {
        if let Some(d) = find(&name) {
            return Some(d);
        }
        name.truncate(name.rfind('_')?);
    }
}

/// The `currency=XXX` keyword of a locale, if any.
pub fn keyword_currency(locale: &str) -> Option<String> {
    let (_, kw) = canonicalize(locale);
    kw.split(';').find_map(|k| k.split_once('=').filter(|(n, _)| n.trim().eq_ignore_ascii_case("currency")).map(|(_, v)| v.trim().to_ascii_uppercase()))
}

/// The symbol `locale` shows for `currency`.
pub fn currency_symbol(locale: &str, currency: &str) -> String {
    let mut name = locale.to_string();
    loop {
        if let Ok(i) = CURRENCY_SYMBOLS.binary_search_by(|e| (e.0, e.1).cmp(&(name.as_str(), currency))) {
            return CURRENCY_SYMBOLS[i].2.to_string();
        }
        match name.rfind('_') {
            Some(i) => name.truncate(i),
            None => return currency.to_string(),
        }
    }
}

pub fn currency_digits(currency: &str) -> u32 {
    CURRENCY_DIGITS.iter().find(|c| c.0 == currency).map_or(2, |c| c.1 as u32)
}

// ---- patterns ---------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Affix {
    Lit(String),
    /// ¤ (1), ¤¤ (2: ISO code), ¤¤¤ (3: plural name), ...
    Currency(usize),
    Percent,
    Permill,
    Minus,
    Plus,
}

#[derive(Clone, Debug)]
pub struct Pattern {
    pub pos_prefix: Vec<Affix>,
    pub pos_suffix: Vec<Affix>,
    pub neg: Option<(Vec<Affix>, Vec<Affix>)>,
    pub min_int: u32,
    pub max_int: u32,
    pub min_frac: u32,
    pub max_frac: u32,
    /// Significant digits (`@`): (min, max).
    pub sig: Option<(u32, u32)>,
    pub grouping: u32,
    pub grouping2: u32,
    pub grouping_used: bool,
    pub multiplier: i64,
    /// Scientific: minimum exponent digits, and whether `+` shows.
    pub exp: Option<(u32, bool)>,
    pub decimal_always: bool,
    /// The scientific pattern has no fraction part: show every digit.
    pub sci_all_frac: bool,
}

fn parse_affix(chars: &[char], i: &mut usize, stop_at_number: bool) -> Result<Vec<Affix>, ()> {
    let mut out: Vec<Affix> = vec![];
    let mut lit = String::new();
    let flush = |lit: &mut String, out: &mut Vec<Affix>| {
        if !lit.is_empty() {
            out.push(Affix::Lit(std::mem::take(lit)));
        }
    };
    while *i < chars.len() {
        let c = chars[*i];
        if c == ';' || (stop_at_number && "#0123456789@.,".contains(c)) {
            break;
        }
        match c {
            '\'' => {
                *i += 1;
                if *i < chars.len() && chars[*i] == '\'' {
                    lit.push('\'');
                    *i += 1;
                    continue;
                }
                while *i < chars.len() {
                    if chars[*i] == '\'' {
                        if *i + 1 < chars.len() && chars[*i + 1] == '\'' {
                            lit.push('\'');
                            *i += 2;
                            continue;
                        }
                        break;
                    }
                    lit.push(chars[*i]);
                    *i += 1;
                }
                *i += 1;
                continue;
            }
            '¤' => {
                flush(&mut lit, &mut out);
                let mut n = 0;
                while *i < chars.len() && chars[*i] == '¤' {
                    n += 1;
                    *i += 1;
                }
                out.push(Affix::Currency(n));
                continue;
            }
            '%' => {
                flush(&mut lit, &mut out);
                out.push(Affix::Percent);
            }
            '‰' => {
                flush(&mut lit, &mut out);
                out.push(Affix::Permill);
            }
            '-' => {
                flush(&mut lit, &mut out);
                out.push(Affix::Minus);
            }
            '+' => {
                flush(&mut lit, &mut out);
                out.push(Affix::Plus);
            }
            '*' => {
                // Padding: skip the pad character.
                *i += 1;
            }
            c => lit.push(c),
        }
        *i += 1;
    }
    flush(&mut lit, &mut out);
    Ok(out)
}

impl Pattern {
    pub fn parse(p: &str) -> Option<Pattern> {
        let chars: Vec<char> = p.chars().collect();
        let mut i = 0;
        let pos_prefix = parse_affix(&chars, &mut i, true).ok()?;
        // The number.
        let (mut int_part, mut frac_part, mut exp_part) = (String::new(), String::new(), String::new());
        let mut seen_dot = false;
        let mut decimal_always = false;
        while i < chars.len() {
            let c = chars[i];
            match c {
                '#' | '0'..='9' | '@' | ',' if !seen_dot && exp_part.is_empty() => int_part.push(c),
                '.' if !seen_dot && exp_part.is_empty() => seen_dot = true,
                '#' | '0'..='9' | '@' if seen_dot && exp_part.is_empty() => frac_part.push(c),
                'E' if exp_part.is_empty() => exp_part.push('E'),
                '+' | '0' if !exp_part.is_empty() => exp_part.push(c),
                _ => break,
            }
            i += 1;
        }
        if seen_dot && frac_part.is_empty() {
            decimal_always = true;
        }
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        let pos_suffix = parse_affix(&chars, &mut i, false).ok()?;
        let neg = if i < chars.len() && chars[i] == ';' {
            i += 1;
            let np = parse_affix(&chars, &mut i, true).ok()?;
            while i < chars.len() && "#0123456789@.,E+".contains(chars[i]) {
                i += 1;
            }
            let ns = parse_affix(&chars, &mut i, false).ok()?;
            Some((np, ns))
        } else {
            None
        };
        let groups: Vec<&str> = int_part.split(',').collect();
        let (grouping, grouping2) = match groups.len() {
            0 | 1 => (0, 0),
            2 => (groups[1].len() as u32, groups[1].len() as u32),
            n => (groups[n - 1].len() as u32, groups[n - 2].len() as u32),
        };
        let digits: String = int_part.chars().filter(|c| *c != ',').collect();
        let sig = if digits.contains('@') {
            let min = digits.chars().filter(|c| *c == '@').count() as u32;
            let max = min + digits.chars().skip_while(|c| *c != '@').filter(|c| *c == '#').count() as u32;
            Some((min, max))
        } else {
            None
        };
        let min_int = digits.chars().filter(|c| c.is_ascii_digit()).count() as u32;
        let exp = if exp_part.is_empty() {
            None
        } else {
            Some((exp_part.chars().filter(|c| *c == '0').count() as u32, exp_part.contains('+')))
        };
        let max_int = if exp.is_some() { digits.chars().count() as u32 } else { u32::MAX };
        let min_frac = frac_part.chars().filter(|c| c.is_ascii_digit()).count() as u32;
        let max_frac = frac_part.chars().count() as u32;
        let multiplier = if pos_prefix.iter().chain(&pos_suffix).any(|a| *a == Affix::Percent) {
            100
        } else if pos_prefix.iter().chain(&pos_suffix).any(|a| *a == Affix::Permill) {
            1000
        } else {
            1
        };
        Some(Pattern {
            pos_prefix,
            pos_suffix,
            neg,
            min_int,
            max_int,
            min_frac,
            max_frac,
            sig,
            grouping,
            grouping2,
            grouping_used: grouping > 0,
            multiplier,
            sci_all_frac: exp.is_some() && frac_part.is_empty(),
            exp,
            decimal_always,
        })
    }

    pub fn is_currency(&self) -> bool {
        self.pos_prefix.iter().chain(&self.pos_suffix).any(|a| matches!(a, Affix::Currency(_)))
    }
}

// ---- decimal arithmetic --------------------------------------------------------

/// A non-negative decimal: digits (no leading zeros unless the value is 0)
/// with the decimal point after `point` of them (may be negative or past
/// the end).
#[derive(Clone, Debug)]
pub struct Dec {
    pub digits: Vec<u8>,
    pub point: i32,
}

impl Dec {
    pub fn from_f64(x: f64) -> Dec {
        let s = format!("{:e}", x.abs());
        let (mant, exp) = s.split_once('e').unwrap();
        let exp: i32 = exp.parse().unwrap();
        let digits: Vec<u8> = mant.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
        Dec { digits, point: exp + 1 }.trimmed()
    }

    pub fn from_u128(n: u128) -> Dec {
        Dec { digits: n.to_string().bytes().map(|b| b - b'0').collect(), point: n.to_string().len() as i32 }.trimmed()
    }

    fn trimmed(mut self) -> Dec {
        while self.digits.len() > 1 && self.digits[0] == 0 {
            self.digits.remove(0);
            self.point -= 1;
        }
        while self.digits.len() > 1 && *self.digits.last().unwrap() == 0 {
            self.digits.pop();
        }
        if self.digits.iter().all(|d| *d == 0) {
            self.digits = vec![0];
            self.point = 1;
        }
        self
    }

    pub fn is_zero(&self) -> bool {
        self.digits.iter().all(|d| *d == 0)
    }

    /// Multiply by a positive integer (percent, permill, MULTIPLIER).
    pub fn mul(self, m: u64) -> Dec {
        if m == 1 {
            return self;
        }
        let mut out = vec![0u64; self.digits.len() + 20];
        let n = out.len();
        for (i, &d) in self.digits.iter().rev().enumerate() {
            out[n - 1 - i] += d as u64 * m;
        }
        let mut carry = 0;
        for x in out.iter_mut().rev() {
            let v = *x + carry;
            *x = v % 10;
            carry = v / 10;
        }
        let digits: Vec<u8> = out.iter().map(|&d| d as u8).collect();
        Dec { digits, point: self.point + 20 }.trimmed()
    }

    /// Round to `places` digits after the point (negative: tens, ...).
    pub fn round(mut self, places: i32, mode: i64, negative: bool) -> Dec {
        let keep = self.point + places;
        if keep >= self.digits.len() as i32 {
            return self;
        }
        if keep < 0 {
            // Every digit drops; the result is 0 or one unit up.
            let unit_up = round_up(mode, negative, 0, &self.digits, true);
            self.digits = vec![if unit_up { 1 } else { 0 }];
            self.point = if unit_up { 1 - places } else { 1 };
            return self.trimmed();
        }
        let keep = keep as usize;
        let last = if keep == 0 { 0 } else { self.digits[keep - 1] };
        let up = round_up(mode, negative, last, &self.digits[keep..], keep == 0);
        self.digits.truncate(keep);
        if self.digits.is_empty() {
            self.digits = vec![0];
            self.point += 1;
        }
        if up {
            let mut i = self.digits.len();
            loop {
                if i == 0 {
                    self.digits.insert(0, 1);
                    self.point += 1;
                    break;
                }
                i -= 1;
                if self.digits[i] == 9 {
                    self.digits[i] = 0;
                } else {
                    self.digits[i] += 1;
                    break;
                }
            }
        }
        self.trimmed()
    }

    /// Round to `n` significant digits.
    pub fn round_sig(self, n: u32, mode: i64, negative: bool) -> Dec {
        if self.is_zero() {
            return self;
        }
        let places = n as i32 - self.point;
        self.round(places, mode, negative)
    }
}

/// Whether dropping `dropped` (after the kept digit `last`) rounds up.
fn round_up(mode: i64, negative: bool, last: u8, dropped: &[u8], below_all: bool) -> bool {
    let nonzero = dropped.iter().any(|d| *d != 0);
    if !nonzero {
        return false;
    }
    // With every digit below the rounding position, the first dropped
    // digit is not the half digit unless it sits right at it.
    let first = if below_all && dropped.is_empty() { 0 } else { dropped[0] };
    let rest_nonzero = dropped.iter().skip(1).any(|d| *d != 0);
    match mode {
        0 => !negative,       // CEILING
        1 => negative,        // FLOOR
        2 => false,           // DOWN
        3 => true,            // UP
        5 => first > 5 || (first == 5 && rest_nonzero), // HALFDOWN
        6 => first >= 5,      // HALFUP
        8 => first > 5 || (first == 5 && (rest_nonzero || last % 2 == 0)), // HALFODD
        _ => first > 5 || (first == 5 && (rest_nonzero || last % 2 == 1)), // HALFEVEN
    }
}

// ---- formatting ---------------------------------------------------------------

/// What a formatter needs at format time.
pub struct Ctx<'a> {
    pub loc: &'a LocaleData,
    pub locale: &'a str,
    pub pattern: &'a Pattern,
    pub symbols: &'a [String; 12],
    pub currency: &'a str,
    pub rounding_mode: i64,
    /// Overrides of the pattern's affixes (setTextAttribute).
    pub affixes: &'a [Option<String>; 4],
}

fn render_affix(ctx: &Ctx, parts: &[Affix], currency_sym: &str) -> String {
    let mut s = String::new();
    for a in parts {
        match a {
            Affix::Lit(l) => s.push_str(l),
            Affix::Currency(1) | Affix::Currency(5) => s.push_str(currency_sym),
            Affix::Currency(_) => s.push_str(ctx.currency),
            Affix::Percent => s.push_str(&ctx.symbols[SYM_PERCENT]),
            Affix::Permill => s.push_str(&ctx.symbols[SYM_PERMILL]),
            Affix::Minus => s.push_str(&ctx.symbols[SYM_MINUS]),
            Affix::Plus => s.push_str(&ctx.symbols[SYM_PLUS]),
        }
    }
    s
}

/// Unicode general category S* (symbols), approximately.
fn is_symbol(c: char) -> bool {
    matches!(c, '$' | '+' | '<' | '=' | '>' | '^' | '`' | '|' | '~' | '¢'..='¦' | '¨' | '©' | '¬' | '®'..='±' | '´' | '¸' | '×' | '÷')
        || ('\u{20a0}'..='\u{20cf}').contains(&c)
        || matches!(c, '\u{058f}' | '\u{060b}' | '\u{09f2}' | '\u{09f3}' | '\u{09fb}' | '\u{0af1}' | '\u{0bf9}' | '\u{0e3f}' | '\u{17db}' | '\u{fdfc}' | '\u{fe69}' | '\u{ff04}' | '\u{ffe0}' | '\u{ffe1}' | '\u{ffe5}' | '\u{ffe6}')
        || ('\u{2100}'..='\u{214f}').contains(&c) && !c.is_alphabetic()
        || ('\u{2190}'..='\u{23ff}').contains(&c)
}

/// Unicode general category Z* (separators).
fn is_separator(c: char) -> bool {
    c == ' ' || c == '\u{a0}' || c == '\u{1680}' || ('\u{2000}'..='\u{200a}').contains(&c) || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

fn digit_chars(zero: &str) -> [char; 10] {
    let z = zero.chars().next().unwrap_or('0') as u32;
    std::array::from_fn(|i| char::from_u32(z + i as u32).unwrap_or('0'))
}

/// Format a number: `negative` and its magnitude as decimal digits (None
/// for NaN, `inf` for ±infinity).
pub fn format(ctx: &Ctx, negative: bool, value: Option<Dec>, inf: bool) -> String {
    let p = ctx.pattern;
    let currency_style = p.is_currency();
    let currency_sym = if currency_style { currency_symbol(ctx.locale, ctx.currency) } else { String::new() };
    let (prefix, suffix) = if negative {
        match &p.neg {
            Some((np, ns)) => (ctx.affixes[2].clone().unwrap_or_else(|| render_affix(ctx, np, &currency_sym)), ctx.affixes[3].clone().unwrap_or_else(|| render_affix(ctx, ns, &currency_sym))),
            None => (
                ctx.affixes[2].clone().unwrap_or_else(|| format!("{}{}", ctx.symbols[SYM_MINUS], render_affix(ctx, &p.pos_prefix, &currency_sym))),
                ctx.affixes[3].clone().unwrap_or_else(|| render_affix(ctx, &p.pos_suffix, &currency_sym)),
            ),
        }
    } else {
        (ctx.affixes[0].clone().unwrap_or_else(|| render_affix(ctx, &p.pos_prefix, &currency_sym)), ctx.affixes[1].clone().unwrap_or_else(|| render_affix(ctx, &p.pos_suffix, &currency_sym)))
    };
    let Some(value) = value else {
        // NaN has no sign: the positive affixes.
        let pre = ctx.affixes[0].clone().unwrap_or_else(|| render_affix(ctx, &p.pos_prefix, &currency_sym));
        let suf = ctx.affixes[1].clone().unwrap_or_else(|| render_affix(ctx, &p.pos_suffix, &currency_sym));
        return format!("{pre}{}{suf}", ctx.symbols[SYM_NAN]);
    };
    let number = if inf { ctx.symbols[SYM_INF].clone() } else { format_number(ctx, value, negative, currency_style) };
    let mut out = prefix.clone();
    // ICU's currency spacing: a letter-like symbol touching a digit gets a
    // no-break space.
    let pre_currency = matches!(if negative { p.neg.as_ref().map_or(p.pos_prefix.last(), |n| n.0.last()) } else { p.pos_prefix.last() }, Some(Affix::Currency(_)));
    let post_currency = matches!(if negative { p.neg.as_ref().map_or(p.pos_suffix.first(), |n| n.1.first()) } else { p.pos_suffix.first() }, Some(Affix::Currency(_)));
    let digits = digit_chars(&ctx.symbols[SYM_ZERO]);
    let is_digit = |c: Option<char>| c.is_some_and(|c| digits.contains(&c));
    if pre_currency && is_digit(number.chars().next()) && prefix.chars().last().is_some_and(|c| !is_symbol(c) && !is_separator(c)) {
        out.push('\u{a0}');
    }
    out.push_str(&number);
    if post_currency && is_digit(number.chars().last()) && suffix.chars().next().is_some_and(|c| !is_symbol(c) && !is_separator(c)) {
        out.push('\u{a0}');
    }
    out.push_str(&suffix);
    out
}

fn format_number(ctx: &Ctx, value: Dec, negative: bool, currency_style: bool) -> String {
    let p = ctx.pattern;
    let digits = digit_chars(&ctx.symbols[SYM_ZERO]);
    let value = value.mul(p.multiplier.unsigned_abs());
    let (dec_sym, group_sym) = if currency_style { (&ctx.symbols[SYM_MONEY_DECIMAL], &ctx.symbols[SYM_MONEY_GROUP]) } else { (&ctx.symbols[SYM_DECIMAL], &ctx.symbols[SYM_GROUP]) };
    if let Some((min_exp, plus)) = p.exp {
        return format_scientific(ctx, value, negative, &digits, dec_sym, min_exp, plus);
    }
    let value = match p.sig {
        Some((_, max)) => value.round_sig(max, ctx.rounding_mode, negative),
        None => value.round(p.max_frac as i32, ctx.rounding_mode, negative),
    };
    // Integer digits.
    let mut int: Vec<u8> = if value.point > 0 {
        (0..value.point as usize).map(|i| value.digits.get(i).copied().unwrap_or(0)).collect()
    } else {
        vec![]
    };
    while int.len() > 1 && int[0] == 0 {
        int.remove(0);
    }
    if int == [0] {
        int.clear();
    }
    let min_int = if p.sig.is_some() { 1 } else { p.min_int };
    while (int.len() as u32) < min_int {
        int.insert(0, 0);
    }
    if int.len() as u32 > p.max_int {
        int.drain(..int.len() - p.max_int as usize);
    }
    // Fraction digits.
    let mut frac: Vec<u8> = if value.point < value.digits.len() as i32 {
        let start = value.point.max(0) as usize;
        let mut f: Vec<u8> = vec![0; (-value.point).max(0) as usize];
        f.extend(&value.digits[start..]);
        f
    } else {
        vec![]
    };
    let min_frac = match p.sig {
        Some((min, _)) => {
            let have = int.iter().skip_while(|d| **d == 0).count() as u32;
            min.saturating_sub(have.max(if int.is_empty() || int.iter().all(|d| *d == 0) { 0 } else { have }))
        }
        None => p.min_frac,
    };
    while (frac.len() as u32) < min_frac {
        frac.push(0);
    }
    let mut out = String::new();
    let n = int.len();
    let grouped = p.grouping_used && p.grouping > 0 && n as u32 >= p.grouping + ctx.loc.min_grouping as u32;
    for (i, d) in int.iter().enumerate() {
        if grouped && i > 0 {
            let from_right = (n - i) as u32;
            let at_group = if from_right == p.grouping {
                true
            } else if from_right > p.grouping && p.grouping2 > 0 {
                (from_right - p.grouping) % p.grouping2 == 0
            } else {
                false
            };
            if at_group {
                out.push_str(group_sym);
            }
        }
        out.push(digits[*d as usize]);
    }
    if !frac.is_empty() || p.decimal_always {
        out.push_str(dec_sym);
        for d in &frac {
            out.push(digits[*d as usize]);
        }
    }
    out
}

fn format_scientific(ctx: &Ctx, value: Dec, negative: bool, digits: &[char; 10], dec_sym: &str, min_exp: u32, plus: bool) -> String {
    let p = ctx.pattern;
    let mut value = value;
    if !p.sci_all_frac {
        // Mantissa digits: integer digits + fraction digits of the pattern.
        let sig = p.max_int.max(1) + p.max_frac;
        value = value.round_sig(sig, ctx.rounding_mode, negative);
    }
    let (exp, mant) = if value.is_zero() { (0, value.digits.clone()) } else { (value.point - 1, value.digits.clone()) };
    let mut out = String::new();
    out.push(digits[mant[0] as usize]);
    let mut frac: Vec<u8> = mant[1..].to_vec();
    while (frac.len() as u32) < p.min_frac {
        frac.push(0);
    }
    if !frac.is_empty() {
        out.push_str(dec_sym);
        for d in frac {
            out.push(digits[d as usize]);
        }
    }
    out.push_str(&ctx.symbols[SYM_EXP]);
    if exp < 0 {
        out.push_str(&ctx.symbols[SYM_MINUS]);
    } else if plus {
        out.push_str(&ctx.symbols[SYM_PLUS]);
    }
    let e = exp.unsigned_abs().to_string();
    for _ in e.len() as u32..min_exp.max(1) {
        out.push(digits[0]);
    }
    for b in e.bytes() {
        out.push(digits[(b - b'0') as usize]);
    }
    out
}

// ---- parsing --------------------------------------------------------------------

/// Parse a number at byte offset `start`: (value, end offset). Grouping
/// separators are skipped; parsing stops at the first character that
/// can't continue the number.
pub fn parse(ctx: &Ctx, s: &str, start: usize) -> Option<(f64, usize)> {
    let p = ctx.pattern;
    let currency_sym = currency_symbol(ctx.locale, ctx.currency);
    let pos_prefix = render_affix(ctx, &p.pos_prefix, &currency_sym);
    let pos_suffix = render_affix(ctx, &p.pos_suffix, &currency_sym);
    let mut rest = &s[start..];
    let mut negative = false;
    let minus = ctx.symbols[SYM_MINUS].as_str();
    if let Some(r) = rest.strip_prefix(minus).or_else(|| rest.strip_prefix('-')) {
        negative = true;
        rest = r;
    }
    if !pos_prefix.is_empty() {
        if let Some(r) = rest.strip_prefix(pos_prefix.as_str()) {
            rest = r.trim_start_matches('\u{a0}');
        }
    }
    if !negative {
        if let Some(r) = rest.strip_prefix(minus).or_else(|| rest.strip_prefix('-')) {
            negative = true;
            rest = r;
        }
    }
    let digits = digit_chars(&ctx.symbols[SYM_ZERO]);
    let currency_style = p.is_currency();
    let (dec_sym, group_sym) = if currency_style { (&ctx.symbols[SYM_MONEY_DECIMAL], &ctx.symbols[SYM_MONEY_GROUP]) } else { (&ctx.symbols[SYM_DECIMAL], &ctx.symbols[SYM_GROUP]) };
    let mut num = String::new();
    let mut seen_dot = false;
    let mut any = false;
    let mut idx = 0;
    let bytes_before = s.len() - rest.len();
    while idx < rest.len() {
        let tail = &rest[idx..];
        let c = tail.chars().next().unwrap();
        if let Some(d) = digits.iter().position(|x| *x == c).or_else(|| c.to_digit(10).map(|d| d as usize)) {
            num.push((b'0' + d as u8) as char);
            any = true;
            idx += c.len_utf8();
        } else if !seen_dot && tail.starts_with(dec_sym.as_str()) {
            seen_dot = true;
            num.push('.');
            idx += dec_sym.len();
        } else if !seen_dot && !group_sym.is_empty() && any && tail.starts_with(group_sym.as_str()) {
            idx += group_sym.len();
        } else {
            break;
        }
    }
    if !any {
        return None;
    }
    let mut end = bytes_before + idx;
    let mut v: f64 = num.parse().ok()?;
    let after = &s[end..];
    let after_trim = after.trim_start_matches('\u{a0}');
    if !pos_suffix.is_empty() && after_trim.starts_with(pos_suffix.as_str()) {
        end += after.len() - after_trim.len() + pos_suffix.len();
    }
    if p.multiplier != 1 {
        v /= p.multiplier as f64;
    }
    Some((if negative { -v } else { v }, end))
}
