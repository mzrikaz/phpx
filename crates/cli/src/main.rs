//! `phpx <file>` (or `phpx run <file>`): run a PHP file, with php's
//! command-line options (-r, -S, -d, -n, ...). `phpx check <paths...>`:
//! parse files and report syntax errors and unsupported PHP features.
//! `phpx ast <file>`: dump the syntax tree.

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
        Some("run") => php_cli(&args[1..]),
        Some("optrees") => optrees::run(),
        Some("help") => {
            usage();
            ExitCode::SUCCESS
        }
        _ => php_cli(&args),
    }
}

fn usage() {
    eprintln!(
        "usage:
  phpx [options] [-f] <file.php> [--] [args...]
  phpx [options] -r <code> [--] [args...]
  phpx [options] -S <addr>:<port> [-t docroot] [router.php]
  phpx run <file.php> [args...]          (same as without `run`)
  phpx check [--summary] <file-or-dir>...
  phpx ast <file>

options: -n (no php.ini)  -d key[=value]  -c <path>  -v  -m  -l <file>"
    );
}

/// php's command line: ini options, then a script, -r code or -S.
fn php_cli(args: &[String]) -> ExitCode {
    let (mut no_ini, mut overrides) = (false, vec![]);
    let (mut code, mut server, mut docroot, mut script) = (None, None, None, None);
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let next = |i: &mut usize| -> Option<String> {
            *i += 1;
            args.get(*i).cloned()
        };
        match a {
            "-n" => no_ini = true,
            "-d" => {
                let Some(d) = next(&mut i) else { return missing(a) };
                let (k, v) = d.split_once('=').unwrap_or((&d, "1"));
                overrides.push((k.to_string(), v.to_string()));
            }
            d if d.starts_with("-d") => {
                let (k, v) = d[2..].split_once('=').unwrap_or((&d[2..], "1"));
                overrides.push((k.to_string(), v.to_string()));
            }
            "-c" => {
                next(&mut i);
            }
            "-q" | "-e" | "-H" => {}
            "-r" => match next(&mut i) {
                Some(c) => {
                    code = Some(c);
                    i += 1;
                    break;
                }
                None => return missing(a),
            },
            "-S" => match next(&mut i) {
                Some(addr) => server = Some(addr),
                None => return missing(a),
            },
            "-t" => match next(&mut i) {
                Some(d) => docroot = Some(d),
                None => return missing(a),
            },
            "-f" => match next(&mut i) {
                Some(f) => {
                    script = Some(f);
                    i += 1;
                    break;
                }
                None => return missing(a),
            },
            "-v" | "--version" => {
                println!("PHP 8.5.10 (cli) (phpx {}) (NTS)\nCopyright (c) The PHP Group\nZend Engine v4.5.10, Copyright (c) Zend Technologies (phpx runtime)", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-m" => {
                println!("[PHP Modules]");
                for m in phpx_interp::extensions() {
                    println!("{m}");
                }
                println!("\n[Zend Modules]\n");
                return ExitCode::SUCCESS;
            }
            "-l" => {
                let Some(f) = next(&mut i) else { return missing(a) };
                return lint(&f);
            }
            "-h" | "--help" | "-?" => {
                usage();
                return ExitCode::SUCCESS;
            }
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with('-') && s.len() > 1 && script.is_none() && server.is_none() => {
                eprintln!("phpx: unknown option {s}");
                usage();
                return ExitCode::from(1);
            }
            s => {
                if server.is_none() {
                    script = Some(s.to_string());
                    i += 1;
                    break;
                }
                // With -S, the next plain argument is the router script.
                script = Some(s.to_string());
            }
        }
        i += 1;
    }
    phpx_interp::set_ini_options(no_ini, overrides);
    if let Some(addr) = server {
        #[cfg(feature = "profile")]
        let guard = std::env::var("PHPX_PROFILE").ok().map(|_| pprof::ProfilerGuard::new(997).expect("profiler"));
        let code = phpx_interp::server::serve(&addr, docroot.as_deref().map(std::path::Path::new), script.as_deref().map(std::path::Path::new));
        #[cfg(feature = "profile")]
        if let Some(g) = guard {
            report(g);
        }
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    let rest: Vec<String> = args.get(i..).map(|r| r.to_vec()).unwrap_or_default();
    let rest: Vec<String> = match rest.first().map(String::as_str) {
        Some("--") => rest[1..].to_vec(),
        _ => rest,
    };
    if let Some(c) = code {
        let out = Box::new(std::io::BufWriter::new(std::io::stdout()));
        let mut argv = vec!["Standard input code".to_string()];
        argv.extend(rest);
        let src = format!("<?php {c}");
        let code = phpx_interp::run_source_args(std::path::Path::new("Command line code"), src, argv, out);
        return ExitCode::from(code.clamp(0, 255) as u8);
    }
    match script {
        Some(s) => run(&s, &rest),
        None => {
            // No script: read it from stdin, like php.
            let mut src = String::new();
            if std::io::Read::read_to_string(&mut std::io::stdin(), &mut src).is_err() {
                return ExitCode::from(1);
            }
            let out = Box::new(std::io::BufWriter::new(std::io::stdout()));
            let mut argv = vec!["Standard input code".to_string()];
            argv.extend(rest);
            let code = phpx_interp::run_source_args(std::path::Path::new("Standard input code"), src, argv, out);
            ExitCode::from(code.clamp(0, 255) as u8)
        }
    }
}

fn missing(opt: &str) -> ExitCode {
    eprintln!("phpx: option {opt} needs an argument");
    ExitCode::from(1)
}

fn lint(path: &str) -> ExitCode {
    let src = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(_) => {
            eprintln!("Could not open input file: {path}");
            return ExitCode::from(1);
        }
    };
    let (_, diags) = phpx_syntax::check(&src);
    let errors: Vec<_> = diags.iter().filter(|d| d.severity == Severity::Error).collect();
    if errors.is_empty() {
        println!("No syntax errors detected in {path}");
        ExitCode::SUCCESS
    } else {
        for d in errors {
            eprint!("{}", d.render(path, &src));
        }
        println!("Errors parsing {path}");
        ExitCode::from(255)
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
    // PHPX_PROFILE_FOLDED=path: the stacks in folded form (flamegraph input),
    // leaf last.
    if let Ok(path) = std::env::var("PHPX_PROFILE_FOLDED") {
        let mut out = String::new();
        for (frames, count) in &r.data {
            let names: Vec<String> = frames.frames.iter().rev().flat_map(|f| f.iter().rev().map(|s| s.name())).collect();
            out.push_str(&format!("{} {count}\n", names.join(";")));
        }
        let _ = std::fs::write(path, out);
    }
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
