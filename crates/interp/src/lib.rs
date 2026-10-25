//! Tree-walking interpreter for phpx.
//!
//! Runs parsed `.php` files directly on the green-thread runtime. It is the
//! fast dev mode, and the semantic reference the compiler will be tested
//! against: every behavior here is checked against real PHP output.

mod arrays;
mod builtins;
mod call;
mod class;
mod compiler;
mod composer;
mod expr;
mod ext;
mod fs;
mod gc;
mod generator;
pub mod interp;
mod json;
mod misc;
mod native;
mod ops;
mod preg;
mod streams;
mod strings;
pub mod value;

use std::io::Write;
use std::path::Path;
use std::rc::Rc;

pub use interp::Interp;
use interp::{Ctrl, Frame};

/// Stack per green thread for interpreted code (tree-walking recursion is deep).
const INTERP_STACK: usize = 8 * 1024 * 1024;
/// The main request task gets more, so deep recursion works like in PHP
/// (pages are only committed when touched).
const MAIN_STACK: usize = 256 * 1024 * 1024;

/// Run a PHP file to completion, writing output to `out`. Returns the exit code.
pub fn run_file(path: &Path, out: Box<dyn Write>) -> i32 {
    run_file_args(path, &[], out)
}

/// Run a script with `$argv` = [path, args...].
pub fn run_file_args(path: &Path, args: &[String], out: Box<dyn Write>) -> i32 {
    let src = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => {
            eprintln!("Could not open input file: {} ({e})", path.display());
            return 1;
        }
    };
    let abs = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut argv = vec![path.display().to_string()];
    argv.extend(args.iter().cloned());
    run_source_args(&abs, src, argv, out)
}

pub fn run_source(path: &Path, src: String, out: Box<dyn Write>) -> i32 {
    run_source_args(path, src, vec![path.display().to_string()], out)
}

/// CLI superglobals: `$_SERVER` (environment + script info), `$argv`,
/// `$argc`, empty request arrays.
fn cli_globals(frame: &mut Frame, path: &Path, argv: &[String]) {
    use value::{pstr, Key, PhpArray, Value};
    let list = Value::array(PhpArray::from_list(argv.iter().map(|a| Value::str(a.as_str())).collect::<Vec<_>>()));
    let mut server = PhpArray::new();
    let mut put = |k: &str, v: Value| {
        server.insert(Key::Str(pstr(k)), v);
    };
    for (k, v) in std::env::vars() {
        put(&k, Value::str(&v));
    }
    let script = argv.first().cloned().unwrap_or_else(|| path.display().to_string());
    put("PHP_SELF", Value::str(&script));
    put("SCRIPT_NAME", Value::str(&script));
    put("SCRIPT_FILENAME", Value::str(&script));
    put("PATH_TRANSLATED", Value::str(&script));
    put("DOCUMENT_ROOT", Value::str(""));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    put("REQUEST_TIME_FLOAT", Value::Float(now.as_secs_f64()));
    put("REQUEST_TIME", Value::Int(now.as_secs() as i64));
    put("argv", list.clone());
    put("argc", Value::Int(argv.len() as i64));
    let set = |f: &mut Frame, n: &'static str, v: Value| *f.var_slot(n).borrow_mut() = v;
    set(frame, "_SERVER", Value::array(server));
    for n in ["_GET", "_POST", "_COOKIE", "_FILES", "_REQUEST", "_ENV"] {
        set(frame, n, Value::array(PhpArray::new()));
    }
    set(frame, "argv", list);
    set(frame, "argc", Value::Int(argv.len() as i64));
}

pub fn run_source_args(path: &Path, src: String, argv: Vec<String>, out: Box<dyn Write>) -> i32 {
    phpx_runtime::set_stack_size(MAIN_STACK);
    let it = Interp::new(out);
    let (id, file) = match it.parse_file(path, src) {
        Ok(x) => x,
        Err(msg) => {
            eprint!("{msg}");
            return 255;
        }
    };
    it.register_strict(id, file);
    it.included.borrow_mut().insert(path.to_path_buf());
    let main_it = it.clone();
    let script = path.to_path_buf();
    let code = phpx_runtime::block_on(move || {
        // Only the root task gets the big stack; async tasks get the default.
        phpx_runtime::set_stack_size(INTERP_STACK);
        let ctx = main_it.top_ctx(id, false);
        let mut frame = Frame::new(ctx, 0);
        cli_globals(&mut frame, &script, &argv);
        main_it.globals.set(&mut frame);
        let code = match main_it.run_file_in(&mut frame, file, id) {
            Ok(_) | Err(Ctrl::GenClose) => 0,
            Err(Ctrl::Exit(c)) => c,
            Err(Ctrl::Throw(e)) => {
                main_it.flush();
                eprintln!("{}", main_it.uncaught_message(&e));
                255
            }
        };
        main_it.globals.set(std::ptr::null_mut());
        drop(frame);
        main_it.close_all_generators();
        // Shutdown functions, then destruct what the globals held.
        let code = match main_it.run_shutdown() {
            Ok(Some(c)) => c,
            Err(Ctrl::Throw(e)) if code == 0 => {
                main_it.flush();
                eprintln!("{}", main_it.uncaught_message(&e));
                255
            }
            _ => code,
        };
        let code = match main_it.run_destructors() {
            Err(Ctrl::Throw(e)) if code == 0 => {
                main_it.flush();
                eprintln!("{}", main_it.uncaught_message(&e));
                255
            }
            _ => code,
        };
        Ok(code)
    });
    it.flush();
    match code {
        Ok(c) => c,
        Err(e) => {
            eprintln!("phpx: runtime error: {e}");
            255
        }
    }
}

/// Run PHP source and capture its output (for tests).
pub fn run_capture(src: &str) -> (String, i32) {
    let buf = Rc::new(std::cell::RefCell::new(Vec::new()));
    struct W(Rc<std::cell::RefCell<Vec<u8>>>);
    impl Write for W {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let code = run_source(Path::new("/test.php"), src.to_string(), Box::new(W(buf.clone())));
    let out = String::from_utf8_lossy(&buf.borrow()).into_owned();
    (out, code)
}
