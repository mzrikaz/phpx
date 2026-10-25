//! ext/random: the engines (Mt19937, PcgOneseq128XslRr64,
//! Xoshiro256StarStar, Secure, user engines), Random\Randomizer, and the
//! functions on the default Mt19937 (mt_rand, rand, mt_srand, shuffle,
//! str_shuffle, array_rand, lcg_value). Ported from PHP's ext/random so
//! seeded sequences match PHP number for number.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, PhpArray, Value};

const ATTEMPTS: u32 = 50;
const MT_N: usize = 624;
const MT_M: usize = 397;
const MT_RAND_MAX: u64 = 0x7FFF_FFFF;

// ---- engines ----------------------------------------------------------------

#[derive(Clone)]
pub struct Mt {
    state: [u32; MT_N],
    count: usize,
    /// MT_RAND_PHP: PHP 5's incorrect twist (and modulus scaling).
    php_mode: bool,
}

impl Mt {
    fn seeded(seed: u32, php_mode: bool) -> Mt {
        let mut mt = Mt { state: [0; MT_N], count: 0, php_mode };
        mt.seed(seed);
        mt
    }

    fn seed(&mut self, seed: u32) {
        self.state[0] = seed;
        for i in 1..MT_N {
            let p = self.state[i - 1];
            self.state[i] = 1812433253u32.wrapping_mul(p ^ (p >> 30)).wrapping_add(i as u32);
        }
        self.reload();
    }

    fn reload(&mut self) {
        let twist = |m: u32, u: u32, v: u32, php: bool| -> u32 {
            let mix = (u & 0x8000_0000) | (v & 0x7FFF_FFFF);
            let low = if php { u } else { v } & 1;
            m ^ (mix >> 1) ^ (0u32.wrapping_sub(low) & 0x9908_b0df)
        };
        let s = &mut self.state;
        for i in 0..MT_N - MT_M {
            s[i] = twist(s[i + MT_M], s[i], s[i + 1], self.php_mode);
        }
        for i in MT_N - MT_M..MT_N - 1 {
            s[i] = twist(s[i + MT_M - MT_N], s[i], s[i + 1], self.php_mode);
        }
        s[MT_N - 1] = twist(s[MT_M - 1], s[MT_N - 1], s[0], self.php_mode);
        self.count = 0;
    }

    fn next(&mut self) -> u32 {
        if self.count >= MT_N {
            self.reload();
        }
        let mut s1 = self.state[self.count];
        self.count += 1;
        s1 ^= s1 >> 11;
        s1 ^= (s1 << 7) & 0x9d2c_5680;
        s1 ^= (s1 << 15) & 0xefc6_0000;
        s1 ^ (s1 >> 18)
    }
}

const PCG_MULT: u128 = (2549297995355413924u128 << 64) | 4865540595714422341;
const PCG_INC: u128 = (6364136223846793005u128 << 64) | 1442695040888963407;

fn pcg_seed(seed: u128) -> u128 {
    let s = 0u128.wrapping_mul(PCG_MULT).wrapping_add(PCG_INC);
    s.wrapping_add(seed).wrapping_mul(PCG_MULT).wrapping_add(PCG_INC)
}

fn pcg_next(s: &mut u128) -> u64 {
    *s = s.wrapping_mul(PCG_MULT).wrapping_add(PCG_INC);
    let (hi, lo) = ((*s >> 64) as u64, *s as u64);
    (hi ^ lo).rotate_right((hi >> 58) as u32)
}

fn pcg_advance(s: &mut u128, mut advance: u64) {
    let (mut cur_mult, mut cur_plus, mut acc_mult, mut acc_plus) = (PCG_MULT, PCG_INC, 1u128, 0u128);
    while advance > 0 {
        if advance & 1 != 0 {
            acc_mult = acc_mult.wrapping_mul(cur_mult);
            acc_plus = acc_plus.wrapping_mul(cur_mult).wrapping_add(cur_plus);
        }
        cur_plus = cur_mult.wrapping_add(1).wrapping_mul(cur_plus);
        cur_mult = cur_mult.wrapping_mul(cur_mult);
        advance /= 2;
    }
    *s = acc_mult.wrapping_mul(*s).wrapping_add(acc_plus);
}

fn splitmix64(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut r = *seed;
    r = (r ^ (r >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    r = (r ^ (r >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    r ^ (r >> 31)
}

fn xoshiro_next(s: &mut [u64; 4]) -> u64 {
    let r = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
    let t = s[1] << 17;
    s[2] ^= s[0];
    s[3] ^= s[1];
    s[1] ^= s[2];
    s[0] ^= s[3];
    s[2] ^= t;
    s[3] = s[3].rotate_left(45);
    r
}

fn xoshiro_jump(s: &mut [u64; 4], jmp: &[u64; 4]) {
    let mut acc = [0u64; 4];
    for &word in jmp {
        for j in 0..64 {
            if word & (1u64 << j) != 0 {
                for k in 0..4 {
                    acc[k] ^= s[k];
                }
            }
            xoshiro_next(s);
        }
    }
    *s = acc;
}

#[derive(Clone)]
enum State {
    Mt(Box<Mt>),
    Pcg(u128),
    Xoshiro([u64; 4]),
    Secure,
}

impl State {
    /// (result, size in bytes).
    fn generate(&mut self) -> (u64, usize) {
        match self {
            State::Mt(mt) => (mt.next() as u64, 4),
            State::Pcg(s) => (pcg_next(s), 8),
            State::Xoshiro(s) => (xoshiro_next(s), 8),
            State::Secure => (u64::from_le_bytes(crate::builtins::secure_bytes(8).try_into().unwrap()), 8),
        }
    }
}

struct EngineData(RefCell<State>);

impl NativeData for EngineData {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        Some(Box::new(EngineData(RefCell::new(self.0.borrow().clone()))))
    }
}

thread_local! {
    /// The default engine behind mt_rand() & co (seeded on first use).
    static DEFAULT: RefCell<Option<Mt>> = const { RefCell::new(None) };
    static LCG: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

/// A new request starts unseeded (PHP's RINIT).
pub(crate) fn reset() {
    DEFAULT.with(|d| *d.borrow_mut() = None);
    LCG.with(|l| l.set(None));
}

fn random_seed() -> u32 {
    u32::from_le_bytes(crate::builtins::secure_bytes(4).try_into().unwrap())
}

fn with_default<T>(f: impl FnOnce(&mut Mt) -> T) -> T {
    DEFAULT.with(|d| f(d.borrow_mut().get_or_insert_with(|| Mt::seeded(random_seed(), false))))
}

/// Where random results come from.
#[derive(Clone, Copy)]
enum Src<'a> {
    Default,
    /// A built-in engine object (its native state).
    Native(&'a Rc<Obj>),
    /// A user class implementing Random\Engine.
    User(&'a Rc<Obj>),
}

fn engine_src(o: &Rc<Obj>) -> Src<'_> {
    if matches!(&*o.internal(), Internal::Data(d) if d.as_any().is::<EngineData>()) {
        Src::Native(o)
    } else {
        Src::User(o)
    }
}

fn with_state<T>(o: &Obj, f: impl FnOnce(&mut State) -> T) -> Option<T> {
    let g = o.internal();
    let Internal::Data(d) = &*g else { return None };
    let e = d.as_any().downcast_ref::<EngineData>()?;
    let r = f(&mut e.0.borrow_mut());
    Some(r)
}

fn broken(it: &Interp, msg: &str) -> crate::interp::Ctrl {
    it.error("Random\\BrokenRandomEngineError", msg)
}

fn generate(it: &Interp, src: Src) -> R<(u64, usize)> {
    match src {
        Src::Default => Ok(with_default(|mt| (mt.next() as u64, 4))),
        Src::Native(o) => with_state(o, |s| s.generate()).ok_or_else(|| it.error("Error", "Random engine is not initialized")),
        Src::User(o) => {
            let v = it.call_method(o, "generate", vec![])?;
            let b = it.to_str(&v)?;
            if b.is_empty() {
                return Err(broken(it, "A random engine must return a non-empty string"));
            }
            let n = b.len().min(8);
            Ok((b[..n].iter().enumerate().fold(0u64, |acc, (i, &c)| acc | (c as u64) << (8 * i)), n))
        }
    }
}

/// At least `bytes` bytes of output, little-endian.
fn gather(it: &Interp, src: Src, bytes: usize) -> R<u64> {
    let (mut result, mut total) = (0u64, 0usize);
    while total < bytes {
        let (r, n) = generate(it, src)?;
        if total < 8 {
            result |= r << (total * 8);
        }
        total += n;
    }
    Ok(result)
}

fn range32(it: &Interp, src: Src, umax: u32) -> R<u32> {
    let mut result = gather(it, src, 4)? as u32;
    if umax == u32::MAX {
        return Ok(result);
    }
    let umax = umax + 1;
    if umax & (umax - 1) == 0 {
        return Ok(result & (umax - 1));
    }
    let limit = u32::MAX - (u32::MAX % umax) - 1;
    let mut count = 0;
    while result > limit {
        count += 1;
        if count > ATTEMPTS {
            return Err(broken(it, "Failed to generate an acceptable random number in 50 attempts"));
        }
        result = gather(it, src, 4)? as u32;
    }
    Ok(result % umax)
}

fn range64(it: &Interp, src: Src, umax: u64) -> R<u64> {
    let mut result = gather(it, src, 8)?;
    if umax == u64::MAX {
        return Ok(result);
    }
    let umax = umax + 1;
    if umax & (umax - 1) == 0 {
        return Ok(result & (umax - 1));
    }
    let limit = u64::MAX - (u64::MAX % umax) - 1;
    let mut count = 0;
    while result > limit {
        count += 1;
        if count > ATTEMPTS {
            return Err(broken(it, "Failed to generate an acceptable random number in 50 attempts"));
        }
        result = gather(it, src, 8)?;
    }
    Ok(result % umax)
}

/// php_random_range(): uniform in [min, max].
fn range(it: &Interp, src: Src, min: i64, max: i64) -> R<i64> {
    let umax = (max as u64).wrapping_sub(min as u64);
    if umax > u32::MAX as u64 {
        return Ok(range64(it, src, umax)?.wrapping_add(min as u64) as i64);
    }
    Ok((range32(it, src, umax as u32)? as u64).wrapping_add(min as u64) as i64)
}

/// MT_RAND_PHP's scaling (RAND_RANGE_BADSCALING).
fn bad_scaling(r: u64, min: i64, max: i64) -> i64 {
    let offset = ((max as f64 - min as f64 + 1.0) * (r as f64 / (MT_RAND_MAX as f64 + 1.0))) as u64;
    offset.wrapping_add(min as u64) as i64
}

/// mt_rand()/rand() with bounds on the default engine.
fn mt_rand_common(it: &Interp, min: i64, max: i64) -> R<i64> {
    if with_default(|mt| mt.php_mode) {
        let r = with_default(|mt| mt.next()) as u64 >> 1;
        return Ok(bad_scaling(r, min, max));
    }
    range(it, Src::Default, min, max)
}

pub(crate) fn shuffle_values(it: &Interp, vals: &mut [Value]) -> R<()> {
    shuffle_in(it, Src::Default, vals)
}

fn shuffle_in<T>(it: &Interp, src: Src, v: &mut [T]) -> R<()> {
    let mut n_left = v.len() as i64;
    if n_left < 2 {
        return Ok(());
    }
    loop {
        n_left -= 1;
        if n_left == 0 {
            return Ok(());
        }
        let j = range(it, src, 0, n_left)?;
        if j != n_left {
            v.swap(n_left as usize, j as usize);
        }
    }
}

/// php_array_pick_keys(): one key, or a list of `num` keys in array order.
fn pick_keys(it: &Interp, src: Src, x: &PhpArray, num: i64, func: &str) -> R<Value> {
    let n = x.len();
    if n == 0 {
        return Err(it.error("ValueError", &format!("{func}(): Argument #1 ($array) must not be empty")));
    }
    if num == 1 {
        let i = range(it, src, 0, n as i64 - 1)? as usize;
        return Ok(x.get_index(i).unwrap().0.to_value());
    }
    if num <= 0 || num as usize > n {
        return Err(it.error("ValueError", &format!("{func}(): Argument #2 ($num) must be between 1 and the number of elements in argument #1 ($array)")));
    }
    let (negative, want) = if num as usize > n >> 1 { (true, n - num as usize) } else { (false, num as usize) };
    let mut set = vec![false; n];
    let (mut left, mut failures) = (want, 0);
    while left > 0 {
        let r = range(it, src, 0, n as i64 - 1)? as usize;
        if set[r] {
            failures += 1;
            if failures > ATTEMPTS {
                return Err(broken(it, "Failed to generate an acceptable random number in 50 attempts"));
            }
        } else {
            set[r] = true;
            left -= 1;
            failures = 0;
        }
    }
    let keys = x.iter().enumerate().filter(|(i, _)| set[*i] ^ negative).map(|(_, (k, _))| k.to_value());
    Ok(Value::array(PhpArray::from_list(keys.collect::<Vec<_>>())))
}

/// Combined LCG (lcg_value()).
fn combined_lcg() -> f64 {
    let (mut s0, mut s1) = LCG.with(|l| {
        l.get().unwrap_or_else(|| {
            let seed = u64::from_le_bytes(crate::builtins::secure_bytes(8).try_into().unwrap());
            (seed as u32 as i32, (seed >> 32) as u32 as i32)
        })
    });
    let modmult = |a: i32, b: i32, c: i32, m: i32, s: &mut i32| {
        let q = *s / a;
        *s = b.wrapping_mul(*s - a * q).wrapping_sub(c.wrapping_mul(q));
        if *s < 0 {
            *s += m;
        }
    };
    modmult(53668, 40014, 12211, 2147483563, &mut s0);
    modmult(52774, 40692, 3791, 2147483399, &mut s1);
    LCG.with(|l| l.set(Some((s0, s1))));
    let mut z = s0.wrapping_sub(s1);
    if z < 1 {
        z += 2147483562;
    }
    z as u64 as f64 * 4.656613e-10
}

// ---- getFloat (gammasection) ---------------------------------------------

fn gamma_max(x: f64, y: f64) -> f64 {
    if x.abs() > y.abs() { x.next_up() - x } else { y - y.next_down() }
}

fn ceilint(a: f64, b: f64, g: f64) -> u64 {
    let s = b / g - a / g;
    let e = if a.abs() <= b.abs() { -a / g - (s - b / g) } else { b / g - (s + a / g) };
    let si = s.ceil();
    if s != si { si as u64 } else { si as u64 + (e > 0.0) as u64 }
}

fn from_max(max: f64, k: u64, g: f64) -> f64 {
    let (hi, lo) = ((k >> 2) as f64, (k & 3) as f64);
    4.0 * (max * 0.25 - hi * g) - lo * g
}

fn from_min(min: f64, k: u64, g: f64) -> f64 {
    let (hi, lo) = ((k >> 2) as f64, (k & 3) as f64);
    4.0 * (min * 0.25 + hi * g) + lo * g
}

/// Random\Randomizer::getFloat() for each IntervalBoundary.
fn gamma_section(it: &Interp, src: Src, min: f64, max: f64, kind: &str) -> R<f64> {
    let g = gamma_max(min, max);
    let hi = ceilint(min, max, g);
    let near_min = min.abs() <= max.abs();
    Ok(match kind {
        "ClosedOpen" => {
            if max <= min || hi < 1 {
                return Ok(f64::NAN);
            }
            let k = 1 + range64(it, src, hi - 1)?;
            match (near_min, k == hi) {
                (true, true) => min,
                (true, false) => from_max(max, k, g),
                (false, _) => from_min(min, k - 1, g),
            }
        }
        "ClosedClosed" => {
            if max < min {
                return Ok(f64::NAN);
            }
            let k = range64(it, src, hi)?;
            match (near_min, k == hi) {
                (true, true) => min,
                (true, false) => from_max(max, k, g),
                (false, true) => max,
                (false, false) => from_min(min, k, g),
            }
        }
        "OpenClosed" => {
            if max <= min || hi < 1 {
                return Ok(f64::NAN);
            }
            let k = range64(it, src, hi - 1)?;
            match (near_min, k == hi - 1) {
                (true, _) => from_max(max, k, g),
                (false, true) => max,
                (false, false) => from_min(min, k + 1, g),
            }
        }
        _ => {
            if max <= min || hi < 2 {
                return Ok(f64::NAN);
            }
            let k = 1 + range64(it, src, hi - 2)?;
            if near_min { from_max(max, k, g) } else { from_min(min, k, g) }
        }
    })
}

// ---- functions ----------------------------------------------------------------

fn seed_default(it: &Interp, a: &[Value]) -> R<Value> {
    let mode = if has(a, 1) { int(it, a, 1)? } else { 0 };
    let php_mode = mode == 1;
    if php_mode {
        it.deprecated("The MT_RAND_PHP variant of Mt19937 is deprecated");
    }
    let seed = if arg(a, 0).is_null() { random_seed() } else { int(it, a, 0)? as u32 };
    DEFAULT.with(|d| *d.borrow_mut() = Some(Mt::seeded(seed, php_mode)));
    Ok(Value::Null)
}

fn bytes_string(r: u64, n: usize) -> Value {
    Value::bytes((0..n).map(|i| (r >> (i * 8)) as u8).collect())
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("mt_srand", |it, _, a| seed_default(it, a));
    t.insert("srand", |it, _, a| seed_default(it, a));
    t.insert("mt_rand", |it, _, a| {
        if a.is_empty() {
            return Ok(Value::Int((with_default(|mt| mt.next()) >> 1) as i64));
        }
        let (min, max) = (int(it, a, 0)?, int(it, a, 1)?);
        if max < min {
            return Err(it.error("ValueError", "mt_rand(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)"));
        }
        Ok(Value::Int(mt_rand_common(it, min, max)?))
    });
    t.insert("rand", |it, _, a| {
        if a.is_empty() {
            return Ok(Value::Int((with_default(|mt| mt.next()) >> 1) as i64));
        }
        let (min, max) = (int(it, a, 0)?, int(it, a, 1)?);
        Ok(Value::Int(if max < min { mt_rand_common(it, max, min)? } else { mt_rand_common(it, min, max)? }))
    });
    t.insert("lcg_value", |it, _, _| {
        it.deprecated("Function lcg_value() is deprecated since 8.4, use \\Random\\Randomizer::getFloat() instead");
        Ok(Value::Float(combined_lcg()))
    });
    t.insert("array_rand", |it, _, a| {
        let x = crate::builtins::arr(it, a, 0, "array_rand")?;
        let num = if has(a, 1) { int(it, a, 1)? } else { 1 };
        pick_keys(it, Src::Default, &x, num, "array_rand")
    });
    t.insert("str_shuffle", |it, _, a| {
        let mut b = s(it, a, 0)?.to_vec();
        shuffle_in(it, Src::Default, &mut b)?;
        Ok(Value::bytes(b))
    });
}

// ---- classes -----------------------------------------------------------------

fn this(o: Option<&Rc<Obj>>) -> &Rc<Obj> {
    o.expect("Random method")
}

fn set_state(o: &Rc<Obj>, s: State) {
    o.set_internal(Internal::Data(Box::new(EngineData(RefCell::new(s)))));
}

/// A 16- or 32-byte seed string as little-endian u64 words.
fn seed_words(b: &[u8]) -> Vec<u64> {
    b.chunks(8).map(|c| c.iter().enumerate().fold(0u64, |acc, (j, &x)| acc | (x as u64) << (j * 8))).collect()
}

fn engine_generate(it: &Interp, this: Option<&Rc<Obj>>, _: &Rc<crate::class::Class>, _: Vec<Value>) -> R<Value> {
    let (r, n) = generate(it, Src::Native(this.unwrap()))?;
    Ok(bytes_string(r, n))
}

fn randomizer_src(it: &Interp, o: &Rc<Obj>) -> R<Rc<Obj>> {
    match o.get_prop("engine") {
        Some(Value::Object(e)) => Ok(e),
        _ => Err(it.error("Error", "Typed property Random\\Randomizer::$engine must not be accessed before initialization")),
    }
}

/// Run `f` with the Randomizer's engine as a source.
fn with_engine<T>(it: &Interp, o: Option<&Rc<Obj>>, f: impl FnOnce(Src) -> R<T>) -> R<T> {
    let e = randomizer_src(it, this(o))?;
    f(engine_src(&e))
}

const RZ: &str = "Random\\Randomizer";

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("random\\engine\\mt19937", "__construct", |it, o, _, a| {
            let mode = if has(&a, 1) { int(it, &a, 1)? } else { 0 };
            let php_mode = match mode {
                0 => false,
                1 => {
                    it.deprecated("The MT_RAND_PHP variant of Mt19937 is deprecated");
                    true
                }
                _ => return Err(it.error("ValueError", "Random\\Engine\\Mt19937::__construct(): Argument #2 ($mode) must be either MT_RAND_MT19937 or MT_RAND_PHP")),
            };
            let seed = if arg(&a, 0).is_null() { random_seed() } else { int(it, &a, 0)? as u32 };
            set_state(this(o), State::Mt(Box::new(Mt::seeded(seed, php_mode))));
            Ok(Value::Null)
        }),
        ("random\\engine\\mt19937", "generate", engine_generate),
        ("random\\engine\\pcgoneseq128xslrr64", "__construct", |it, o, _, a| {
            let seed: u128 = match arg(&a, 0) {
                Value::Null => u128::from_le_bytes(crate::builtins::secure_bytes(16).try_into().unwrap()),
                Value::Str(b) => {
                    if b.len() != 16 {
                        return Err(it.error("ValueError", "Random\\Engine\\PcgOneseq128XslRr64::__construct(): Argument #1 ($seed) must be a 16 byte (128 bit) string"));
                    }
                    let w = seed_words(&b);
                    (w[0] as u128) << 64 | w[1] as u128
                }
                v => it.to_int(&v)? as u64 as u128,
            };
            set_state(this(o), State::Pcg(pcg_seed(seed)));
            Ok(Value::Null)
        }),
        ("random\\engine\\pcgoneseq128xslrr64", "generate", engine_generate),
        ("random\\engine\\pcgoneseq128xslrr64", "jump", |it, o, _, a| {
            let n = int(it, &a, 0)?;
            if n < 0 {
                return Err(it.error("ValueError", "Random\\Engine\\PcgOneseq128XslRr64::jump(): Argument #1 ($advance) must be greater than or equal to 0"));
            }
            with_state(this(o), |s| {
                if let State::Pcg(p) = s {
                    pcg_advance(p, n as u64);
                }
            });
            Ok(Value::Null)
        }),
        ("random\\engine\\xoshiro256starstar", "__construct", |it, o, _, a| {
            let st: [u64; 4] = match arg(&a, 0) {
                Value::Null => loop {
                    let w = seed_words(&crate::builtins::secure_bytes(32));
                    if w.iter().any(|&x| x != 0) {
                        break [w[0], w[1], w[2], w[3]];
                    }
                },
                Value::Str(b) => {
                    if b.len() != 32 {
                        return Err(it.error("ValueError", "Random\\Engine\\Xoshiro256StarStar::__construct(): Argument #1 ($seed) must be a 32 byte (256 bit) string"));
                    }
                    let w = seed_words(&b);
                    if w.iter().all(|&x| x == 0) {
                        return Err(it.error("ValueError", "Random\\Engine\\Xoshiro256StarStar::__construct(): Argument #1 ($seed) must not consist entirely of NUL bytes"));
                    }
                    [w[0], w[1], w[2], w[3]]
                }
                v => {
                    let mut seed = it.to_int(&v)? as u64;
                    [splitmix64(&mut seed), splitmix64(&mut seed), splitmix64(&mut seed), splitmix64(&mut seed)]
                }
            };
            set_state(this(o), State::Xoshiro(st));
            Ok(Value::Null)
        }),
        ("random\\engine\\xoshiro256starstar", "generate", engine_generate),
        ("random\\engine\\xoshiro256starstar", "jump", |_, o, _, _| {
            with_state(this(o), |s| {
                if let State::Xoshiro(x) = s {
                    xoshiro_jump(x, &[0x180ec6d33cfd0aba, 0xd5a61266f0c9392c, 0xa9582618e03fc9aa, 0x39abdc4529b1661c]);
                }
            });
            Ok(Value::Null)
        }),
        ("random\\engine\\xoshiro256starstar", "jumplong", |_, o, _, _| {
            with_state(this(o), |s| {
                if let State::Xoshiro(x) = s {
                    xoshiro_jump(x, &[0x76e15d3efefdcbbf, 0xc5004e441c522fb3, 0x77710069854ee241, 0x39109bb02acbe635]);
                }
            });
            Ok(Value::Null)
        }),
        ("random\\engine\\secure", "__construct", |_, o, _, _| {
            set_state(this(o), State::Secure);
            Ok(Value::Null)
        }),
        ("random\\engine\\secure", "generate", |it, o, _, _| {
            let o = this(o);
            if with_state(o, |_| ()).is_none() {
                set_state(o, State::Secure);
            }
            engine_generate(it, Some(o), &o.class, vec![])
        }),
        ("random\\randomizer", "nextint", |it, o, _, _| {
            with_engine(it, o, |src| {
                let (r, n) = generate(it, src)?;
                if n > 8 {
                    return Err(it.error("Random\\RandomException", "Generated value exceeds size of int"));
                }
                Ok(Value::Int((r >> 1) as i64))
            })
        }),
        ("random\\randomizer", "getint", |it, o, _, a| {
            let (min, max) = (int(it, &a, 0)?, int(it, &a, 1)?);
            if max < min {
                return Err(it.error("ValueError", &format!("{RZ}::getInt(): Argument #2 ($max) must be greater than or equal to argument #1 ($min)")));
            }
            with_engine(it, o, |src| {
                if let Src::Native(e) = src {
                    if with_state(e, |s| matches!(s, State::Mt(mt) if mt.php_mode)) == Some(true) {
                        let r = generate(it, src)?.0 >> 1;
                        return Ok(Value::Int(bad_scaling(r, min, max)));
                    }
                }
                Ok(Value::Int(range(it, src, min, max)?))
            })
        }),
        ("random\\randomizer", "nextfloat", |it, o, _, _| {
            with_engine(it, o, |src| Ok(Value::Float((gather(it, src, 8)? >> 11) as f64 / (1u64 << 53) as f64)))
        }),
        ("random\\randomizer", "getfloat", |it, o, _, a| {
            let (min, max) = (it.to_float(&arg(&a, 0))?, it.to_float(&arg(&a, 1))?);
            if !min.is_finite() {
                return Err(it.error("ValueError", &format!("{RZ}::getFloat(): Argument #1 ($min) must be finite")));
            }
            if !max.is_finite() {
                return Err(it.error("ValueError", &format!("{RZ}::getFloat(): Argument #2 ($max) must be finite")));
            }
            let kind = match arg(&a, 2) {
                Value::Object(b) => match b.get_prop("name") {
                    Some(Value::Str(n)) => String::from_utf8_lossy(&n).into_owned(),
                    _ => "ClosedOpen".into(),
                },
                _ => "ClosedOpen".into(),
            };
            let closed = kind == "ClosedClosed";
            if (closed && max < min) || (!closed && max <= min) {
                let rel = if closed { "greater than or equal to" } else { "greater than" };
                return Err(it.error("ValueError", &format!("{RZ}::getFloat(): Argument #2 ($max) must be {rel} argument #1 ($min)")));
            }
            let r = with_engine(it, o, |src| gamma_section(it, src, min, max, &kind))?;
            if r.is_nan() && kind == "OpenOpen" {
                return Err(it.error("ValueError", "The given interval is empty, there are no floats between argument #1 ($min) and argument #2 ($max)"));
            }
            Ok(Value::Float(r))
        }),
        ("random\\randomizer", "getbytes", |it, o, _, a| {
            let len = int(it, &a, 0)?;
            if len < 1 {
                return Err(it.error("ValueError", &format!("{RZ}::getBytes(): Argument #1 ($length) must be greater than 0")));
            }
            with_engine(it, o, |src| {
                let mut out = Vec::with_capacity(len as usize);
                while out.len() < len as usize {
                    let (mut r, n) = generate(it, src)?;
                    for _ in 0..n {
                        out.push(r as u8);
                        r >>= 8;
                        if out.len() >= len as usize {
                            break;
                        }
                    }
                }
                Ok(Value::bytes(out))
            })
        }),
        ("random\\randomizer", "getbytesfromstring", |it, o, _, a| {
            let source = s(it, &a, 0)?.to_vec();
            let len = int(it, &a, 1)?;
            if source.is_empty() {
                return Err(it.error("ValueError", &format!("{RZ}::getBytesFromString(): Argument #1 ($string) must not be empty")));
            }
            if len < 1 {
                return Err(it.error("ValueError", &format!("{RZ}::getBytesFromString(): Argument #2 ($length) must be greater than 0")));
            }
            let (len, max_offset) = (len as usize, source.len() as u64 - 1);
            with_engine(it, o, |src| {
                let mut out = Vec::with_capacity(len);
                if max_offset > 0xff {
                    while out.len() < len {
                        out.push(source[range(it, src, 0, max_offset as i64)? as usize]);
                    }
                } else {
                    let mut mask = max_offset;
                    mask |= mask >> 1;
                    mask |= mask >> 2;
                    mask |= mask >> 4;
                    let mask = mask.wrapping_mul(0x0101_0101_0101_0101);
                    let mut failures = 0;
                    while out.len() < len {
                        let (r, n) = generate(it, src)?;
                        let mut offsets = r & mask;
                        for _ in 0..n {
                            let off = offsets & 0xff;
                            offsets >>= 8;
                            if off > max_offset {
                                failures += 1;
                                if failures > ATTEMPTS {
                                    return Err(broken(it, "Failed to generate an acceptable random number in 50 attempts"));
                                }
                                continue;
                            }
                            failures = 0;
                            out.push(source[off as usize]);
                            if out.len() >= len {
                                break;
                            }
                        }
                    }
                }
                Ok(Value::bytes(out))
            })
        }),
        ("random\\randomizer", "shufflearray", |it, o, _, a| {
            let x = match arg(&a, 0) {
                Value::Array(x) => x,
                v => return Err(it.error("TypeError", &format!("{RZ}::shuffleArray(): Argument #1 ($array) must be of type array, {} given", v.type_name()))),
            };
            let mut vals: Vec<Value> = x.values().collect();
            with_engine(it, o, |src| shuffle_in(it, src, &mut vals))?;
            Ok(Value::array(PhpArray::from_list(vals)))
        }),
        ("random\\randomizer", "shufflebytes", |it, o, _, a| {
            let mut b = s(it, &a, 0)?.to_vec();
            with_engine(it, o, |src| shuffle_in(it, src, &mut b))?;
            Ok(Value::bytes(b))
        }),
        ("random\\randomizer", "pickarraykeys", |it, o, _, a| {
            let x = match arg(&a, 0) {
                Value::Array(x) => x,
                v => return Err(it.error("TypeError", &format!("{RZ}::pickArrayKeys(): Argument #1 ($array) must be of type array, {} given", v.type_name()))),
            };
            let num = int(it, &a, 1)?;
            let r = with_engine(it, o, |src| pick_keys(it, src, &x, num, &format!("{RZ}::pickArrayKeys")))?;
            Ok(match r {
                Value::Array(_) => r,
                k => Value::array(PhpArray::from_list(vec![k])),
            })
        }),
    ]
}
