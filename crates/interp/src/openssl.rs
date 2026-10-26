//! ext/openssl's symmetric side: openssl_encrypt/openssl_decrypt with the
//! AES modes (ECB, CBC, CTR, CFB, CFB8, CFB1, OFB, GCM), the cipher and
//! digest queries, openssl_digest, openssl_pbkdf2 and openssl_error_string.

use std::cell::RefCell;
use std::collections::HashMap;

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use ghash::universal_hash::UniversalHash;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{PhpArray, Value};

const RAW_DATA: i64 = 1;
const ZERO_PADDING: i64 = 2;

thread_local! {
    /// OpenSSL's error queue, drained by openssl_error_string().
    static ERRORS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn push_error(e: &str) {
    ERRORS.with(|q| q.borrow_mut().push(e.to_string()));
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Ecb,
    Cbc,
    Ctr,
    Cfb,
    Cfb8,
    Cfb1,
    Ofb,
    Gcm,
}

const MODES: &[(&str, Mode)] = &[
    ("cbc", Mode::Cbc),
    ("cfb", Mode::Cfb),
    ("cfb1", Mode::Cfb1),
    ("cfb8", Mode::Cfb8),
    ("ctr", Mode::Ctr),
    ("ecb", Mode::Ecb),
    ("gcm", Mode::Gcm),
    ("ofb", Mode::Ofb),
];

#[derive(Clone, Copy)]
struct Cipher {
    key_len: usize,
    mode: Mode,
}

impl Cipher {
    fn lookup(name: &str) -> Option<Cipher> {
        let name = name.to_ascii_lowercase();
        let name = name.strip_prefix("id-").unwrap_or(&name);
        let rest = name.strip_prefix("aes-").or_else(|| name.strip_prefix("aes"))?;
        let (bits, mode) = rest.split_once('-').unwrap_or((rest, "cbc"));
        let key_len = match bits {
            "128" => 16,
            "192" => 24,
            "256" => 32,
            _ => return None,
        };
        let mode = MODES.iter().find(|(m, _)| *m == mode)?.1;
        Some(Cipher { key_len, mode })
    }

    fn iv_len(self) -> usize {
        match self.mode {
            Mode::Ecb => 0,
            Mode::Gcm => 12,
            _ => 16,
        }
    }
}

enum Aes {
    A128(aes::Aes128),
    A192(aes::Aes192),
    A256(aes::Aes256),
}

impl Aes {
    fn new(key: &[u8]) -> Aes {
        match key.len() {
            16 => Aes::A128(aes::Aes128::new(GenericArray::from_slice(key))),
            24 => Aes::A192(aes::Aes192::new(GenericArray::from_slice(key))),
            _ => Aes::A256(aes::Aes256::new(GenericArray::from_slice(key))),
        }
    }

    fn enc(&self, b: &mut [u8; 16]) {
        let g = GenericArray::from_mut_slice(b);
        match self {
            Aes::A128(c) => c.encrypt_block(g),
            Aes::A192(c) => c.encrypt_block(g),
            Aes::A256(c) => c.encrypt_block(g),
        }
    }

    fn dec(&self, b: &mut [u8; 16]) {
        let g = GenericArray::from_mut_slice(b);
        match self {
            Aes::A128(c) => c.decrypt_block(g),
            Aes::A192(c) => c.decrypt_block(g),
            Aes::A256(c) => c.decrypt_block(g),
        }
    }

    fn keystream(&self, b: &[u8; 16]) -> [u8; 16] {
        let mut k = *b;
        self.enc(&mut k);
        k
    }
}

fn block(b: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out[..b.len()].copy_from_slice(b);
    out
}

fn xor(out: &mut [u8], ks: &[u8]) {
    for (o, k) in out.iter_mut().zip(ks) {
        *o ^= k;
    }
}

/// The stream modes (CTR, OFB, CFB*): the same keystream both ways, only
/// CFB feeds back the ciphertext.
fn stream(aes: &Aes, mode: Mode, iv: &[u8], data: &[u8], encrypt: bool) -> Vec<u8> {
    let mut out = data.to_vec();
    let mut reg = block(iv);
    match mode {
        Mode::Ctr => {
            for chunk in out.chunks_mut(16) {
                xor(chunk, &aes.keystream(&reg));
                reg = u128::from_be_bytes(reg).wrapping_add(1).to_be_bytes();
            }
        }
        Mode::Ofb => {
            for chunk in out.chunks_mut(16) {
                aes.enc(&mut reg);
                xor(chunk, &reg);
            }
        }
        Mode::Cfb => {
            for chunk in out.chunks_mut(16) {
                let ks = aes.keystream(&reg);
                let input = block(chunk);
                xor(chunk, &ks);
                reg = if encrypt { block(chunk) } else { input };
            }
        }
        Mode::Cfb8 => {
            for byte in out.iter_mut() {
                let ks = aes.keystream(&reg)[0];
                let input = *byte;
                *byte ^= ks;
                let fed = if encrypt { *byte } else { input };
                reg.copy_within(1.., 0);
                reg[15] = fed;
            }
        }
        Mode::Cfb1 => {
            for byte in out.iter_mut() {
                let mut res = 0u8;
                for bit in (0..8).rev() {
                    let ks = aes.keystream(&reg)[0] >> 7;
                    let input = (*byte >> bit) & 1;
                    let o = input ^ ks;
                    res |= o << bit;
                    let fed = if encrypt { o } else { input };
                    let r = u128::from_be_bytes(reg) << 1 | fed as u128;
                    reg = r.to_be_bytes();
                }
                *byte = res;
            }
        }
        _ => unreachable!(),
    }
    out
}

/// ECB/CBC; None where OpenSSL's final step fails.
fn blocks(aes: &Aes, mode: Mode, iv: &[u8], data: &[u8], encrypt: bool, pad: bool) -> Option<Vec<u8>> {
    let mut buf = data.to_vec();
    if encrypt && pad {
        let n = 16 - buf.len() % 16;
        buf.resize(buf.len() + n, n as u8);
    }
    if buf.len() % 16 != 0 {
        push_error("error:1C80006B:Provider routines::wrong final block length");
        return None;
    }
    let mut prev = block(iv);
    for chunk in buf.chunks_mut(16) {
        let mut b = block(chunk);
        if encrypt {
            if mode == Mode::Cbc {
                xor(&mut b, &prev);
            }
            aes.enc(&mut b);
            prev = b;
        } else {
            let input = b;
            aes.dec(&mut b);
            if mode == Mode::Cbc {
                xor(&mut b, &prev);
            }
            prev = input;
        }
        chunk.copy_from_slice(&b);
    }
    if !encrypt && pad {
        let n = buf.last().copied().unwrap_or(0) as usize;
        if n == 0 || n > 16 || n > buf.len() || !buf[buf.len() - n..].iter().all(|&b| b as usize == n) {
            push_error("error:1C800064:Provider routines::bad decrypt");
            return None;
        }
        buf.truncate(buf.len() - n);
    }
    Some(buf)
}

fn ghash(h: &[u8; 16], aad: &[u8], data: &[u8]) -> [u8; 16] {
    let mut g = ghash::GHash::new(GenericArray::from_slice(h));
    g.update_padded(aad);
    g.update_padded(data);
    let mut lens = [0u8; 16];
    lens[..8].copy_from_slice(&((aad.len() as u64) * 8).to_be_bytes());
    lens[8..].copy_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    g.update(&[lens.into()]);
    g.finalize().into()
}

/// AES-GCM: (output, full 16-byte tag).
fn gcm(aes: &Aes, iv: &[u8], aad: &[u8], data: &[u8], encrypt: bool) -> (Vec<u8>, [u8; 16]) {
    let h = aes.keystream(&[0u8; 16]);
    let j0 = if iv.len() == 12 {
        let mut j = block(iv);
        j[15] = 1;
        j
    } else {
        ghash(&h, &[], iv)
    };
    let mut out = data.to_vec();
    let mut ctr = j0;
    for chunk in out.chunks_mut(16) {
        let c = u32::from_be_bytes(ctr[12..].try_into().unwrap()).wrapping_add(1);
        ctr[12..].copy_from_slice(&c.to_be_bytes());
        xor(chunk, &aes.keystream(&ctr));
    }
    let mut tag = ghash(&h, aad, if encrypt { &out } else { data });
    xor(&mut tag, &aes.keystream(&j0));
    (out, tag)
}

/// The key padded with NULs (or cut) to the cipher's length.
fn fit_key(key: &[u8], len: usize) -> Vec<u8> {
    let mut k = key[..key.len().min(len)].to_vec();
    k.resize(len, 0);
    k
}

/// php_openssl_validate_iv(): Some(iv of the right length), with PHP's
/// warnings; None when an AEAD IV length can't be set.
fn fit_iv(it: &Interp, func: &str, c: Cipher, iv: &[u8]) -> Option<Vec<u8>> {
    let want = c.iv_len();
    if iv.len() == want {
        return Some(iv.to_vec());
    }
    if c.mode == Mode::Gcm {
        if iv.is_empty() {
            it.warn(&format!("{func}(): Setting of IV length for AEAD mode failed"));
            return None;
        }
        return Some(iv.to_vec());
    }
    if iv.is_empty() {
        return Some(vec![0; want]);
    }
    if iv.len() < want {
        it.warn(&format!("{func}(): IV passed is only {} bytes long, cipher expects an IV of precisely {want} bytes, padding with \\0", iv.len()));
    } else {
        it.warn(&format!("{func}(): IV passed is {} bytes long which is longer than the {want} expected by selected cipher, truncating", iv.len()));
    }
    Some(fit_key(iv, want))
}

fn cipher_arg(it: &Interp, func: &str, a: &[Value], i: usize) -> R<Option<Cipher>> {
    let name = s(it, a, i)?;
    let c = Cipher::lookup(&crate::pstr::lossy(&name));
    if c.is_none() {
        it.warn(&format!("{func}(): Unknown cipher algorithm"));
    }
    Ok(c)
}

/// OpenSSL's digest names in PHP's hash spelling, for the digests both have.
fn md_name(name: &str) -> Option<String> {
    let mut n = name.to_ascii_lowercase();
    if let Some(r) = n.strip_prefix("sha2-") {
        n = format!("sha{r}");
    } else if let Some(r) = n.strip_prefix("sha-") {
        n = format!("sha{r}");
    } else if n == "ripemd" || n == "rmd160" || n == "ripemd-160" {
        n = "ripemd160".into();
    }
    n = n.replace("sha512-", "sha512/");
    MD_METHODS.contains(&n.as_str()).then_some(n)
}

const MD_METHODS: &[&str] = &[
    "md4", "md5", "ripemd160", "sha1", "sha224", "sha256", "sha3-224", "sha3-256", "sha3-384", "sha3-512", "sha384", "sha512", "sha512/224", "sha512/256", "whirlpool",
];

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("openssl_cipher_iv_length", |it, _, a| {
        Ok(match cipher_arg(it, "openssl_cipher_iv_length", a, 0)? {
            Some(c) => Value::Int(c.iv_len() as i64),
            None => Value::Bool(false),
        })
    });
    t.insert("openssl_cipher_key_length", |it, _, a| {
        Ok(match cipher_arg(it, "openssl_cipher_key_length", a, 0)? {
            Some(c) => Value::Int(c.key_len as i64),
            None => Value::Bool(false),
        })
    });
    t.insert("openssl_get_cipher_methods", |_, _, a| {
        let aliases = arg(a, 0).truthy();
        let mut names = vec![];
        for bits in ["128", "192", "256"] {
            for (m, _) in MODES {
                names.push(format!("aes-{bits}-{m}"));
                if aliases {
                    names.push(format!("AES-{bits}-{}", m.to_ascii_uppercase()));
                }
            }
            if aliases {
                names.push(format!("aes{bits}"));
            }
        }
        names.sort();
        Ok(Value::array(PhpArray::from_list(names.iter().map(|n| Value::str(n)))))
    });
    t.insert("openssl_get_md_methods", |_, _, a| {
        let aliases = arg(a, 0).truthy();
        let mut names: Vec<String> = MD_METHODS.iter().map(|m| m.replace('/', "-")).collect();
        if aliases {
            names.extend(MD_METHODS.iter().map(|m| m.to_ascii_uppercase().replace('/', "-")));
        }
        names.sort();
        Ok(Value::array(PhpArray::from_list(names.iter().map(|n| Value::str(n)))))
    });
    t.insert("openssl_encrypt", |it, _, a| {
        const F: &str = "openssl_encrypt";
        let data = s(it, a, 0)?;
        let Some(c) = cipher_arg(it, F, a, 1)? else { return Ok(Value::Bool(false)) };
        let key = fit_key(&s(it, a, 2)?, c.key_len);
        let opts = if has(a, 3) { int(it, a, 3)? } else { 0 };
        let iv = if has(a, 4) { s(it, a, 4)?.to_vec() } else { vec![] };
        let aad = if has(a, 6) { s(it, a, 6)?.to_vec() } else { vec![] };
        let tag_len = if has(a, 7) { int(it, a, 7)? } else { 16 };
        if iv.is_empty() && c.iv_len() > 0 && c.mode != Mode::Gcm {
            it.warn(&format!("{F}(): Using an empty Initialization Vector (iv) is potentially insecure and not recommended"));
        }
        let Some(iv) = fit_iv(it, F, c, &iv) else { return Ok(Value::Bool(false)) };
        let aes = Aes::new(&key);
        let out = match c.mode {
            Mode::Ecb | Mode::Cbc => match blocks(&aes, c.mode, &iv, &data, true, opts & ZERO_PADDING == 0) {
                Some(o) => o,
                None => return Ok(Value::Bool(false)),
            },
            Mode::Gcm => {
                if !(1..=16).contains(&tag_len) {
                    it.warn(&format!("{F}(): Retrieving verification tag failed"));
                    return Ok(Value::Bool(false));
                }
                let (o, tag) = gcm(&aes, &iv, &aad, &data, true);
                if a.len() > 5 {
                    a[5] = Value::bytes(tag[..tag_len as usize].to_vec());
                }
                o
            }
            m => stream(&aes, m, &iv, &data, true),
        };
        Ok(if opts & RAW_DATA != 0 { Value::bytes(out) } else { Value::str(&crate::strings::base64_encode(&out)) })
    });
    t.insert("openssl_decrypt", |it, _, a| {
        const F: &str = "openssl_decrypt";
        let input = s(it, a, 0)?;
        let Some(c) = cipher_arg(it, F, a, 1)? else { return Ok(Value::Bool(false)) };
        let key = fit_key(&s(it, a, 2)?, c.key_len);
        let opts = if has(a, 3) { int(it, a, 3)? } else { 0 };
        let iv = if has(a, 4) { s(it, a, 4)?.to_vec() } else { vec![] };
        let tag = arg(a, 5);
        let aad = if has(a, 6) { s(it, a, 6)?.to_vec() } else { vec![] };
        let data = if opts & RAW_DATA != 0 {
            input.to_vec()
        } else {
            match crate::strings::base64_decode(&input) {
                Some(d) => d,
                None => {
                    it.warn(&format!("{F}(): Failed to base64 decode the input"));
                    return Ok(Value::Bool(false));
                }
            }
        };
        let Some(iv) = fit_iv(it, F, c, &iv) else { return Ok(Value::Bool(false)) };
        let aes = Aes::new(&key);
        let out = match c.mode {
            Mode::Ecb | Mode::Cbc => blocks(&aes, c.mode, &iv, &data, false, opts & ZERO_PADDING == 0),
            Mode::Gcm => {
                let want = if tag.is_null() { vec![] } else { it.to_str(&tag)?.to_vec() };
                let (o, full) = gcm(&aes, &iv, &aad, &data, false);
                (!want.is_empty() && want.len() <= 16 && full[..want.len()] == want[..]).then_some(o)
            }
            m => Some(stream(&aes, m, &iv, &data, false)),
        };
        Ok(out.map_or(Value::Bool(false), Value::bytes))
    });
    t.insert("openssl_digest", |it, _, a| {
        let data = s(it, a, 0)?;
        let algo = crate::pstr::lossy(&s(it, a, 1)?).into_owned();
        let binary = arg(a, 2).truthy();
        let Some(mut h) = md_name(&algo).and_then(|n| crate::ext::Hasher::new(&n)) else {
            it.warn("openssl_digest(): Unknown digest algorithm");
            return Ok(Value::Bool(false));
        };
        h.update(&data);
        let raw = h.finish();
        Ok(if binary { Value::bytes(raw) } else { Value::str(&crate::ext::hex(&raw)) })
    });
    t.insert("openssl_pbkdf2", |it, _, a| {
        let (pass, salt) = (s(it, a, 0)?, s(it, a, 1)?);
        let len = int(it, a, 2)?;
        let iters = int(it, a, 3)?;
        let algo = if has(a, 4) { crate::pstr::lossy(&s(it, a, 4)?).into_owned() } else { "sha1".into() };
        let Some(algo) = md_name(&algo) else {
            it.warn("openssl_pbkdf2(): Unknown digest algorithm");
            return Ok(Value::Bool(false));
        };
        if len <= 0 {
            return Err(it.error("ValueError", "openssl_pbkdf2(): Argument #3 ($key_length) must be greater than 0"));
        }
        if iters <= 0 {
            return Err(it.error("ValueError", "openssl_pbkdf2(): Argument #4 ($iterations) must be greater than 0"));
        }
        Ok(Value::bytes(crate::ext_more::pbkdf2(&algo, &pass, &salt, iters as u64, len as usize)))
    });
    t.insert("openssl_error_string", |_, _, _| {
        Ok(ERRORS.with(|q| {
            let mut q = q.borrow_mut();
            if q.is_empty() {
                Value::Bool(false)
            } else {
                Value::str(&q.remove(0))
            }
        }))
    });
}
