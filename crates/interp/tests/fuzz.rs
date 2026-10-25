//! Randomized robustness tests (ignored by default; they take a while):
//!
//!     cargo test --release -p phpx-interp --test fuzz -- --ignored
//!
//! PHPX_FUZZ_SEED and PHPX_FUZZ_ITERS change the seed and the amount of
//! work. A failure prints the input that triggered it.
//!
//! - `parser`: mutated test sources must never make the front end panic.
//! - `builtins`: parsers of untrusted data (unserialize, json_decode,
//!   unpack, sprintf, regexes, ...) fed random bytes must never panic
//!   (a panic inside a built-in surfaces as "phpx internal error") or
//!   crash the process.

use std::path::Path;

/// xorshift64*: small, deterministic, good enough for fuzzing.
struct Rng(u64);

impl Rng {
    fn new() -> Rng {
        let seed = std::env::var("PHPX_FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

fn iters(default: usize) -> usize {
    std::env::var("PHPX_FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn corpus() -> Vec<Vec<u8>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/php");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "php")).collect();
    files.sort();
    files.iter().map(|p| std::fs::read(p).unwrap()).collect()
}

const TOKENS: &[&str] = &[
    "(", ")", "[", "]", "{", "}", ";", ",", "=>", "->", "?->", "::", "$", "$x", "\"", "'", "`", "<?php", "?>", "<<<EOT\n",
    "\nEOT;\n", "fn", "function", "class", "match", "yield", "...", "&", "#[", "/*", "*/", "//", "\\", "\0", "\u{ff}", "{$", "${",
    "1e999", "0x", "0b", "PHP_INT_MAX", "static", "new", "?", ":", "??=", "**", "<<", "|>", "enum", "readonly", "abstract",
];

fn mutate(rng: &mut Rng, src: &[u8], corpus: &[Vec<u8>]) -> Vec<u8> {
    let mut s = src.to_vec();
    for _ in 0..1 + rng.below(8) {
        let at = rng.below(s.len() + 1);
        match rng.below(7) {
            0 if !s.is_empty() => {
                let i = rng.below(s.len());
                s[i] ^= 1 << rng.below(8);
            }
            1 => {
                let t = rng.pick(TOKENS).as_bytes();
                s.splice(at..at, t.iter().copied());
            }
            2 if !s.is_empty() => {
                let end = (at + rng.below(64)).min(s.len());
                s.drain(at.min(end)..end);
            }
            3 => s.truncate(at),
            4 => {
                // Splice in a slice of another file.
                let other = rng.pick(corpus);
                if !other.is_empty() {
                    let a = rng.below(other.len());
                    let b = (a + rng.below(200)).min(other.len());
                    s.splice(at..at, other[a..b].iter().copied());
                }
            }
            5 => {
                // Deep nesting.
                let n = 1 + rng.below(3000);
                let (open, close) = *rng.pick(&[("(", ")"), ("[", "]"), ("{", "}"), ("f(", ")")]);
                let mut ins = open.repeat(n).into_bytes();
                ins.extend(close.repeat(n).into_bytes());
                s.splice(at..at, ins);
            }
            _ => {
                let len = s.len();
                if len > 0 {
                    let a = rng.below(len);
                    let b = (a + rng.below(100)).min(len);
                    let dup = s[a..b].to_vec();
                    s.splice(at.min(len)..at.min(len), dup);
                }
            }
        }
    }
    s
}

#[test]
#[ignore]
fn parser() {
    let corpus = corpus();
    let mut rng = Rng::new();
    let n = iters(20_000);
    // A thread with a known stack, like the CLI's main thread.
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            for i in 0..n {
                let base = rng.pick(&corpus).clone();
                let input = mutate(&mut rng, &base, &corpus);
                let src = String::from_utf8_lossy(&input).into_owned();
                let r = std::panic::catch_unwind(|| {
                    let (file, _diags) = phpx_syntax::check(&src);
                    // The tree is dropped here, as the CLI's `check` does.
                    drop(file);
                });
                if r.is_err() {
                    let path = std::env::temp_dir().join(format!("phpx-fuzz-parser-{i}.php"));
                    std::fs::write(&path, &input).unwrap();
                    panic!("front end panicked on input #{i}, saved to {}", path.display());
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Random bytes, biased towards the syntax the target functions parse.
fn data(rng: &mut Rng, alphabet: &[u8]) -> Vec<u8> {
    let len = rng.below(64);
    (0..len)
        .map(|_| if rng.below(4) == 0 { rng.next() as u8 } else { alphabet[rng.below(alphabet.len())] })
        .collect()
}

fn php_str(b: &[u8]) -> String {
    let mut s = String::from("\"");
    for &c in b {
        s.push_str(&format!("\\x{c:02x}"));
    }
    s.push('"');
    s
}

#[test]
#[ignore]
fn builtins() {
    let mut rng = Rng::new();
    let n = iters(300);
    let ser = b"aiOsbdNC:;{}0123456789\"-.EeINF";
    let json = b"{}[]\":,0123456789-+.eEtruefalsnl\\u ";
    let fmt = b"%sdfxXbcoeEgGu'0-+ 123456789.$";
    let re = b"/()[]{}*+?|^$.\\dswbQEuiumsxU#~0123456789-abc";
    for round in 0..n {
        let mut script = String::from("<?php\n$bad = 0;\nfunction t($f) { global $bad; try { $f(); } catch (Throwable $e) { if (str_contains($e->getMessage(), 'phpx internal error')) { echo 'INTERNAL: ', $e->getMessage(), \"\\n\"; $bad++; } } }\n");
        for _ in 0..50 {
            let call = match rng.below(9) {
                0 => format!("@unserialize({})", php_str(&data(&mut rng, ser))),
                1 => format!("@json_decode({}, true, {})", php_str(&data(&mut rng, json)), 1 + rng.below(600)),
                2 => format!("@unpack({}, {})", php_str(&data(&mut rng, b"aAhHcCsSnvlLNVqQJPfgdeExXZ@*0123456789/")), php_str(&data(&mut rng, b"\0\xff\x01abc"))),
                3 => format!("@sprintf({}, 1, 'a', 2.5, -3)", php_str(&data(&mut rng, fmt))),
                4 => format!("@preg_match({}, {})", php_str(&data(&mut rng, re)), php_str(&data(&mut rng, b"abc123 \n"))),
                5 => format!("@preg_replace({}, 'x', {})", php_str(&data(&mut rng, re)), php_str(&data(&mut rng, b"abc123 \n"))),
                6 => format!("@pack({}, 1, 2, 3)", php_str(&data(&mut rng, b"aAhHcCsSnvlLNVqQJPfgdeExXZ@*0123456789"))),
                7 => format!("@str_getcsv({}, {})", php_str(&data(&mut rng, b",;\"'\n\\ab")), php_str(&data(&mut rng, b",;").get(..1).unwrap_or(b","))),
                _ => format!("@parse_url({})", php_str(&data(&mut rng, b"htps:/@?#[]:80abc.%"))),
            };
            script.push_str(&format!("t(fn() => {call});\n"));
        }
        script.push_str("echo \"done $bad\\n\";\n");
        let src = script.clone();
        let r = std::thread::Builder::new().stack_size(64 << 20).spawn(move || phpx_interp::run_capture(&src)).unwrap().join();
        let ok = matches!(&r, Ok((out, _)) if out.contains("done 0"));
        if !ok {
            let path = std::env::temp_dir().join(format!("phpx-fuzz-builtins-{round}.php"));
            std::fs::write(&path, &script).unwrap();
            let detail = match r {
                Ok((out, code)) => format!("exit {code}, output:\n{}", out.lines().filter(|l| l.contains("INTERNAL")).take(5).collect::<Vec<_>>().join("\n")),
                Err(_) => "the interpreter panicked".to_string(),
            };
            panic!("round {round} failed ({detail}); script saved to {}", path.display());
        }
    }
}

/// Every built-in with random hostile arguments (tools/fuzz/all_builtins.php).
#[test]
#[ignore]
fn all_builtins() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fuzz/all_builtins.php");
    let src = std::fs::read_to_string(&script).unwrap();
    let seed = std::env::var("PHPX_FUZZ_SEED").unwrap_or_else(|_| "1".into());
    let per = std::env::var("PHPX_FUZZ_ITERS").unwrap_or_else(|_| "30".into());
    // The script reads its seed and call count from $argv.
    let src = src.replacen("$seed = (int)($argv[1] ?? 1);", &format!("$seed = {seed};"), 1).replacen("$per = (int)($argv[2] ?? 30);", &format!("$per = {per};"), 1);
    let (out, code) = std::thread::Builder::new().stack_size(64 << 20).spawn(move || phpx_interp::run_capture(&src)).unwrap().join().expect("interpreter panicked");
    let internal: Vec<&str> = out.lines().filter(|l| l.starts_with("INTERNAL")).collect();
    assert!(internal.is_empty() && out.contains("done:"), "exit {code}:\n{}", internal.join("\n"));
}
