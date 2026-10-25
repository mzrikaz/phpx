//! `phpx run <file>`: run a PHP file. `phpx check <paths...>`: parse files and
//! report syntax errors and unsupported PHP features. `phpx ast <file>`: dump
//! the syntax tree.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use phpx_syntax::Severity;

mod optrees;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check") if args.len() > 1 => check(&args[1..]),
        Some("ast") if args.len() == 2 => ast(&args[1]),
        Some("run") if args.len() >= 2 => run(&args[1], &args[2..]),
        Some("optrees") => optrees::run(),
        _ => {
            eprintln!("usage:\n  phpx run <file.php> [args...]\n  phpx check [--summary] <file-or-dir>...\n  phpx ast <file>");
            ExitCode::from(2)
        }
    }
}

fn run(path: &str, args: &[String]) -> ExitCode {
    #[cfg(feature = "profile")]
    let guard = std::env::var("PHPX_PROFILE").ok().map(|_| pprof::ProfilerGuard::new(997).expect("profiler"));
    let out = Box::new(std::io::BufWriter::new(std::io::stdout()));
    let code = phpx_interp::run_file_args(Path::new(path), args, out);
    #[cfg(feature = "profile")]
    if let Some(g) = guard {
        report(g);
    }
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// Print the hottest functions (self time and inclusive) from the samples.
#[cfg(feature = "profile")]
fn report(g: pprof::ProfilerGuard) {
    let Ok(r) = g.report().build() else { return };
    let mut own: std::collections::HashMap<String, isize> = Default::default();
    let mut incl: std::collections::HashMap<String, isize> = Default::default();
    let mut total = 0;
    for (frames, count) in &r.data {
        total += count;
        let names: Vec<String> = frames.frames.iter().flat_map(|f| f.iter().map(|s| s.name())).collect();
        if let Some(top) = names.first() {
            *own.entry(top.clone()).or_default() += count;
        }
        let mut seen = std::collections::HashSet::new();
        for n in names {
            if seen.insert(n.clone()) {
                *incl.entry(n).or_default() += count;
            }
        }
    }
    let show = |title: &str, m: &std::collections::HashMap<String, isize>| {
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!("--- {title} ({total} samples)");
        for (n, c) in v.into_iter().take(25) {
            let short: String = n.chars().take(110).collect();
            eprintln!("{:5.1}%  {short}", *c as f64 * 100.0 / total.max(1) as f64);
        }
    };
    show("self", &own);
    show("inclusive", &incl);
}

fn ast(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (file, diags) = phpx_syntax::check(&src);
    println!("{file:#?}");
    for d in &diags {
        eprint!("{}", d.render(path, &src));
    }
    if diags.iter().any(|d| d.is_error()) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(rd) = std::fs::read_dir(path) else { return };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            collect(&p, out);
        }
    } else if path.extension().is_some_and(|e| e == "php") {
        out.push(path.to_path_buf());
    }
}

fn check(args: &[String]) -> ExitCode {
    let summary_only = args.iter().any(|a| a == "--summary");
    let mut files = vec![];
    for a in args.iter().filter(|a| !a.starts_with("--")) {
        collect(Path::new(a), &mut files);
    }

    let start = Instant::now();
    let (mut ok, mut unsupported, mut syntax, mut bytes) = (0, 0, 0, 0);
    let mut by_code: std::collections::BTreeMap<&str, usize> = Default::default();
    for f in &files {
        // Non-UTF-8 files are read lossily; spans still point at the right lines.
        let raw = match std::fs::read(f) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("{}: {e}", f.display());
                continue;
            }
        };
        bytes += raw.len();
        let src = String::from_utf8_lossy(&raw);
        let (_, diags) = phpx_syntax::check(&src);
        let errors: Vec<_> = diags.iter().filter(|d| d.severity == Severity::Error).collect();
        if errors.is_empty() {
            ok += 1;
        } else if errors.iter().all(|d| d.is_unsupported()) {
            unsupported += 1;
        } else {
            syntax += 1;
        }
        for d in &diags {
            *by_code.entry(d.code).or_default() += 1;
            if !summary_only {
                eprint!("{}", d.render(&f.display().to_string(), &src));
            }
        }
    }
    let secs = start.elapsed().as_secs_f64();
    println!(
        "{} files ({:.1} MB) in {:.2}s: {ok} ok, {unsupported} use unsupported features, {syntax} with syntax errors",
        files.len(),
        bytes as f64 / 1e6,
        secs
    );
    if !by_code.is_empty() {
        let codes: Vec<String> = by_code.iter().map(|(c, n)| format!("{c}×{n}")).collect();
        println!("diagnostics: {}", codes.join(" "));
    }
    if unsupported + syntax > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS }
}
