//! ext/hash's algorithms: the cryptographic digests (RustCrypto) and the
//! checksums and fast hashes PHP implements itself (crc32 variants,
//! adler32, FNV, joaat, MurmurHash3, xxHash), each producing the bytes
//! PHP's hash() returns.

use digest::DynDigest;

/// PHP's hash_algos(), in PHP's order (the ones phpx implements).
pub const ALGOS: &[&str] = &[
    "md2", "md4", "md5", "sha1", "sha224", "sha256", "sha384", "sha512/224", "sha512/256", "sha512", "sha3-224", "sha3-256", "sha3-384",
    "sha3-512", "ripemd128", "ripemd160", "ripemd256", "ripemd320", "whirlpool", "tiger128,3", "tiger160,3", "tiger192,3", "gost",
    "gost-crypto", "adler32", "crc32", "crc32b", "crc32c", "fnv132", "fnv1a32", "fnv164", "fnv1a64", "joaat", "murmur3a", "murmur3c",
    "murmur3f", "xxh32", "xxh64", "xxh3", "xxh128",
];

/// Algorithms usable with HMAC and PBKDF2 (the cryptographic ones).
pub fn is_crypto(algo: &str) -> bool {
    !matches!(
        algo,
        "adler32" | "crc32" | "crc32b" | "crc32c" | "fnv132" | "fnv1a32" | "fnv164" | "fnv1a64" | "joaat" | "murmur3a" | "murmur3c" | "murmur3f" | "xxh32" | "xxh64" | "xxh3" | "xxh128"
    )
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fast {
    Adler32,
    Crc32,
    Crc32b,
    Crc32c,
    Fnv132,
    Fnv1a32,
    Fnv164,
    Fnv1a64,
    Joaat,
    Murmur3a,
    Murmur3c,
    Murmur3f,
    Xxh32,
    Xxh64,
    Xxh3,
    Xxh128,
}

pub struct Hasher {
    imp: Imp,
    block: usize,
}

enum Imp {
    Digest(Box<dyn DynDigest>, usize),
    /// Buffered input for the non-streaming implementations.
    Fast(Fast, Vec<u8>, u64, Option<Vec<u8>>),
}

impl Clone for Hasher {
    fn clone(&self) -> Hasher {
        Hasher {
            imp: match &self.imp {
                Imp::Digest(d, n) => Imp::Digest(d.box_clone(), *n),
                Imp::Fast(f, b, s, k) => Imp::Fast(*f, b.clone(), *s, k.clone()),
            },
            block: self.block,
        }
    }
}

fn digest(d: Box<dyn DynDigest>, block: usize) -> Hasher {
    let n = d.output_size();
    Hasher { imp: Imp::Digest(d, n), block }
}

fn truncated(d: Box<dyn DynDigest>, block: usize, n: usize) -> Hasher {
    Hasher { imp: Imp::Digest(d, n), block }
}

impl Hasher {
    pub fn new(algo: &str) -> Option<Hasher> {
        Hasher::with_options(algo, 0, None)
    }

    /// `seed` / `secret`: hash()'s options for murmur and xxHash.
    pub fn with_options(algo: &str, seed: u64, secret: Option<Vec<u8>>) -> Option<Hasher> {
        use digest::Digest;
        let fast = |f: Fast| Some(Hasher { imp: Imp::Fast(f, vec![], seed, secret.clone()), block: 4 });
        Some(match algo.to_ascii_lowercase().as_str() {
            "md2" => digest(Box::new(md2::Md2::new()), 16),
            "md4" => digest(Box::new(md4::Md4::new()), 64),
            "md5" => digest(Box::new(md5::Md5::new()), 64),
            "sha1" => digest(Box::new(sha1::Sha1::new()), 64),
            "sha224" => digest(Box::new(sha2::Sha224::new()), 64),
            "sha256" => digest(Box::new(sha2::Sha256::new()), 64),
            "sha384" => digest(Box::new(sha2::Sha384::new()), 128),
            "sha512/224" => digest(Box::new(sha2::Sha512_224::new()), 128),
            "sha512/256" => digest(Box::new(sha2::Sha512_256::new()), 128),
            "sha512" => digest(Box::new(sha2::Sha512::new()), 128),
            "sha3-224" => digest(Box::new(sha3::Sha3_224::new()), 144),
            "sha3-256" => digest(Box::new(sha3::Sha3_256::new()), 136),
            "sha3-384" => digest(Box::new(sha3::Sha3_384::new()), 104),
            "sha3-512" => digest(Box::new(sha3::Sha3_512::new()), 72),
            "ripemd128" => digest(Box::new(ripemd::Ripemd128::new()), 64),
            "ripemd160" => digest(Box::new(ripemd::Ripemd160::new()), 64),
            "ripemd256" => digest(Box::new(ripemd::Ripemd256::new()), 64),
            "ripemd320" => digest(Box::new(ripemd::Ripemd320::new()), 64),
            "whirlpool" => digest(Box::new(whirlpool::Whirlpool::new()), 64),
            "tiger192,3" => digest(Box::new(tiger::Tiger::new()), 64),
            "tiger160,3" => truncated(Box::new(tiger::Tiger::new()), 64, 20),
            "tiger128,3" => truncated(Box::new(tiger::Tiger::new()), 64, 16),
            "gost" => digest(Box::new(gost94::Gost94Test::new()), 32),
            "gost-crypto" => digest(Box::new(gost94::Gost94CryptoPro::new()), 32),
            "adler32" => fast(Fast::Adler32)?,
            "crc32" => fast(Fast::Crc32)?,
            "crc32b" => fast(Fast::Crc32b)?,
            "crc32c" => fast(Fast::Crc32c)?,
            "fnv132" => fast(Fast::Fnv132)?,
            "fnv1a32" => fast(Fast::Fnv1a32)?,
            "fnv164" => fast(Fast::Fnv164)?,
            "fnv1a64" => fast(Fast::Fnv1a64)?,
            "joaat" => fast(Fast::Joaat)?,
            "murmur3a" => fast(Fast::Murmur3a)?,
            "murmur3c" => fast(Fast::Murmur3c)?,
            "murmur3f" => fast(Fast::Murmur3f)?,
            "xxh32" => fast(Fast::Xxh32)?,
            "xxh64" => fast(Fast::Xxh64)?,
            "xxh3" => fast(Fast::Xxh3)?,
            "xxh128" => fast(Fast::Xxh128)?,
            _ => return None,
        })
    }

    pub fn update(&mut self, d: &[u8]) {
        match &mut self.imp {
            Imp::Digest(h, _) => h.update(d),
            Imp::Fast(_, b, _, _) => b.extend_from_slice(d),
        }
    }

    pub fn finish(self) -> Vec<u8> {
        match self.imp {
            Imp::Digest(h, n) => {
                let mut out = h.finalize().to_vec();
                out.truncate(n);
                out
            }
            Imp::Fast(f, b, seed, secret) => fast_hash(f, &b, seed, secret.as_deref()),
        }
    }

    pub fn block_size(&self) -> usize {
        self.block
    }
}

fn crc_table(poly: u32, reflected: bool) -> [u32; 256] {
    let mut t = [0u32; 256];
    for (i, e) in t.iter_mut().enumerate() {
        let mut c = if reflected { i as u32 } else { (i as u32) << 24 };
        for _ in 0..8 {
            c = if reflected {
                if c & 1 != 0 { (c >> 1) ^ poly } else { c >> 1 }
            } else if c & 0x8000_0000 != 0 {
                (c << 1) ^ poly
            } else {
                c << 1
            };
        }
        *e = c;
    }
    t
}

fn murmur3a(d: &[u8], seed: u32) -> u32 {
    let (c1, c2) = (0xcc9e2d51u32, 0x1b873593u32);
    let mut h = seed;
    let chunks = d.chunks_exact(4);
    let tail = chunks.remainder();
    for c in chunks {
        let mut k = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
        k = k.wrapping_mul(c1).rotate_left(15).wrapping_mul(c2);
        h ^= k;
        h = h.rotate_left(13).wrapping_mul(5).wrapping_add(0xe6546b64);
    }
    let mut k = 0u32;
    for (i, &b) in tail.iter().enumerate() {
        k ^= (b as u32) << (8 * i);
    }
    if !tail.is_empty() {
        h ^= k.wrapping_mul(c1).rotate_left(15).wrapping_mul(c2);
    }
    h ^= d.len() as u32;
    fmix32(h)
}

fn fmix32(mut h: u32) -> u32 {
    h ^= h >> 16;
    h = h.wrapping_mul(0x85ebca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2ae35);
    h ^ (h >> 16)
}

fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51afd7ed558ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ceb9fe1a85ec53);
    k ^ (k >> 33)
}

/// MurmurHash3_x86_128.
fn murmur3c(d: &[u8], seed: u32) -> [u32; 4] {
    let (c1, c2, c3, c4) = (0x239b961bu32, 0xab0e9789u32, 0x38b34ae5u32, 0xa1e38b93u32);
    let (mut h1, mut h2, mut h3, mut h4) = (seed, seed, seed, seed);
    let chunks = d.chunks_exact(16);
    let tail = chunks.remainder();
    for c in chunks {
        let w = |i: usize| u32::from_le_bytes([c[i], c[i + 1], c[i + 2], c[i + 3]]);
        let (mut k1, mut k2, mut k3, mut k4) = (w(0), w(4), w(8), w(12));
        k1 = k1.wrapping_mul(c1).rotate_left(15).wrapping_mul(c2);
        h1 ^= k1;
        h1 = h1.rotate_left(19).wrapping_add(h2).wrapping_mul(5).wrapping_add(0x561ccd1b);
        k2 = k2.wrapping_mul(c2).rotate_left(16).wrapping_mul(c3);
        h2 ^= k2;
        h2 = h2.rotate_left(17).wrapping_add(h3).wrapping_mul(5).wrapping_add(0x0bcaa747);
        k3 = k3.wrapping_mul(c3).rotate_left(17).wrapping_mul(c4);
        h3 ^= k3;
        h3 = h3.rotate_left(15).wrapping_add(h4).wrapping_mul(5).wrapping_add(0x96cd1c35);
        k4 = k4.wrapping_mul(c4).rotate_left(18).wrapping_mul(c1);
        h4 ^= k4;
        h4 = h4.rotate_left(13).wrapping_add(h1).wrapping_mul(5).wrapping_add(0x32ac3b17);
    }
    let mut k = [0u32; 4];
    for (i, &b) in tail.iter().enumerate() {
        k[i / 4] ^= (b as u32) << (8 * (i % 4));
    }
    if tail.len() > 12 {
        h4 ^= k[3].wrapping_mul(c4).rotate_left(18).wrapping_mul(c1);
    }
    if tail.len() > 8 {
        h3 ^= k[2].wrapping_mul(c3).rotate_left(17).wrapping_mul(c4);
    }
    if tail.len() > 4 {
        h2 ^= k[1].wrapping_mul(c2).rotate_left(16).wrapping_mul(c3);
    }
    if !tail.is_empty() {
        h1 ^= k[0].wrapping_mul(c1).rotate_left(15).wrapping_mul(c2);
    }
    let len = d.len() as u32;
    h1 ^= len;
    h2 ^= len;
    h3 ^= len;
    h4 ^= len;
    h1 = h1.wrapping_add(h2).wrapping_add(h3).wrapping_add(h4);
    h2 = h2.wrapping_add(h1);
    h3 = h3.wrapping_add(h1);
    h4 = h4.wrapping_add(h1);
    h1 = fmix32(h1);
    h2 = fmix32(h2);
    h3 = fmix32(h3);
    h4 = fmix32(h4);
    h1 = h1.wrapping_add(h2).wrapping_add(h3).wrapping_add(h4);
    h2 = h2.wrapping_add(h1);
    h3 = h3.wrapping_add(h1);
    h4 = h4.wrapping_add(h1);
    [h1, h2, h3, h4]
}

/// MurmurHash3_x64_128.
fn murmur3f(d: &[u8], seed: u64) -> [u64; 2] {
    let (c1, c2) = (0x87c37b91114253d5u64, 0x4cf5ad432745937fu64);
    let (mut h1, mut h2) = (seed, seed);
    let chunks = d.chunks_exact(16);
    let tail = chunks.remainder();
    for c in chunks {
        let mut k1 = u64::from_le_bytes(c[0..8].try_into().unwrap());
        let mut k2 = u64::from_le_bytes(c[8..16].try_into().unwrap());
        k1 = k1.wrapping_mul(c1).rotate_left(31).wrapping_mul(c2);
        h1 ^= k1;
        h1 = h1.rotate_left(27).wrapping_add(h2).wrapping_mul(5).wrapping_add(0x52dce729);
        k2 = k2.wrapping_mul(c2).rotate_left(33).wrapping_mul(c1);
        h2 ^= k2;
        h2 = h2.rotate_left(31).wrapping_add(h1).wrapping_mul(5).wrapping_add(0x38495ab5);
    }
    let (mut k1, mut k2) = (0u64, 0u64);
    for (i, &b) in tail.iter().enumerate() {
        if i < 8 {
            k1 ^= (b as u64) << (8 * i);
        } else {
            k2 ^= (b as u64) << (8 * (i - 8));
        }
    }
    if tail.len() > 8 {
        h2 ^= k2.wrapping_mul(c2).rotate_left(33).wrapping_mul(c1);
    }
    if !tail.is_empty() {
        h1 ^= k1.wrapping_mul(c1).rotate_left(31).wrapping_mul(c2);
    }
    let len = d.len() as u64;
    h1 ^= len;
    h2 ^= len;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix64(h1);
    h2 = fmix64(h2);
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    [h1, h2]
}

fn fast_hash(f: Fast, d: &[u8], seed: u64, secret: Option<&[u8]>) -> Vec<u8> {
    match f {
        Fast::Adler32 => {
            let (mut a, mut b) = (1u32, 0u32);
            for &c in d {
                a = (a + c as u32) % 65521;
                b = (b + a) % 65521;
            }
            ((b << 16) | a).to_be_bytes().to_vec()
        }
        Fast::Crc32 => {
            // The bzip2 CRC: MSB-first, written least significant byte first.
            let t = crc_table(0x04C11DB7, false);
            let mut c = !0u32;
            for &b in d {
                c = (c << 8) ^ t[((c >> 24) ^ b as u32) as usize];
            }
            (!c).to_le_bytes().to_vec()
        }
        Fast::Crc32b | Fast::Crc32c => {
            let t = crc_table(if f == Fast::Crc32b { 0xEDB88320 } else { 0x82F63B78 }, true);
            let mut c = !0u32;
            for &b in d {
                c = (c >> 8) ^ t[((c ^ b as u32) & 0xff) as usize];
            }
            (!c).to_be_bytes().to_vec()
        }
        Fast::Fnv132 | Fast::Fnv1a32 => {
            let mut h = 0x811c9dc5u32;
            for &b in d {
                if f == Fast::Fnv132 {
                    h = h.wrapping_mul(0x01000193) ^ b as u32;
                } else {
                    h = (h ^ b as u32).wrapping_mul(0x01000193);
                }
            }
            h.to_be_bytes().to_vec()
        }
        Fast::Fnv164 | Fast::Fnv1a64 => {
            let mut h = 0xcbf29ce484222325u64;
            for &b in d {
                if f == Fast::Fnv164 {
                    h = h.wrapping_mul(0x100000001b3) ^ b as u64;
                } else {
                    h = (h ^ b as u64).wrapping_mul(0x100000001b3);
                }
            }
            h.to_be_bytes().to_vec()
        }
        Fast::Joaat => {
            let mut h = 0u32;
            for &b in d {
                h = h.wrapping_add(b as u32);
                h = h.wrapping_add(h << 10);
                h ^= h >> 6;
            }
            h = h.wrapping_add(h << 3);
            h ^= h >> 11;
            h = h.wrapping_add(h << 15);
            h.to_be_bytes().to_vec()
        }
        Fast::Murmur3a => murmur3a(d, seed as u32).to_be_bytes().to_vec(),
        Fast::Murmur3c => murmur3c(d, seed as u32).iter().flat_map(|h| h.to_be_bytes()).collect(),
        Fast::Murmur3f => murmur3f(d, seed).iter().flat_map(|h| h.to_be_bytes()).collect(),
        Fast::Xxh32 => xxhash_rust::xxh32::xxh32(d, seed as u32).to_be_bytes().to_vec(),
        Fast::Xxh64 => xxhash_rust::xxh64::xxh64(d, seed).to_be_bytes().to_vec(),
        Fast::Xxh3 => match secret {
            Some(s) => xxhash_rust::xxh3::xxh3_64_with_secret(d, s),
            None => xxhash_rust::xxh3::xxh3_64_with_seed(d, seed),
        }
        .to_be_bytes()
        .to_vec(),
        Fast::Xxh128 => match secret {
            Some(s) => xxhash_rust::xxh3::xxh3_128_with_secret(d, s),
            None => xxhash_rust::xxh3::xxh3_128_with_seed(d, seed),
        }
        .to_be_bytes()
        .to_vec(),
    }
}
