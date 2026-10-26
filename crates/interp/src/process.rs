//! Running programs: proc_open() and friends, exec(), shell_exec(),
//! system(), passthru(), popen()/pclose(). Pipes are non-blocking on the
//! reactor, so waiting for a child's output parks only the green thread.

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

use phpx_runtime::net::Pipe;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::sockets::{pipe_stream, Sock};
use crate::streams::{Backend, Stream};
use crate::value::{pstr, Key, PhpArray, Resource, Value};

struct Proc {
    /// Taken by proc_close().
    child: Option<Child>,
    pid: u32,
    command: String,
    /// Exit status once seen (proc_get_status() keeps reporting it).
    status: Option<ExitStatus>,
}

fn shell(cmd: &str) -> Command {
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(cmd);
    c
}

fn exit_code(st: &ExitStatus) -> i64 {
    use std::os::unix::process::ExitStatusExt;
    match (st.code(), st.signal()) {
        (Some(c), _) => c as i64,
        (None, Some(sig)) => 128 + sig as i64,
        _ => -1,
    }
}

/// Wait for a child without blocking the thread.
fn wait(it: &Interp, child: &mut Child) -> R<ExitStatus> {
    let mut nap = Duration::from_micros(200);
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Ok(st),
            Ok(None) => {
                phpx_runtime::sleep(nap).map_err(|e| crate::native::rt_error(it, e))?;
                nap = (nap * 2).min(Duration::from_millis(20));
            }
            Err(e) => return Err(it.error("Error", &format!("waitpid failed: {e}"))),
        }
    }
}

fn pipe_of(fd: impl Into<OwnedFd>) -> Option<Pipe> {
    Pipe::from_fd(fd.into()).ok()
}

/// Read a pipe to the end (parking between chunks).
fn read_all(p: &mut Pipe, mut each: impl FnMut(&[u8])) {
    let mut buf = [0u8; 16384];
    while let Ok(n) = p.read(&mut buf, None) {
        if n == 0 {
            break;
        }
        each(&buf[..n]);
    }
}

/// Run `cmd` with the shell, stdout captured: (output, exit code).
fn run(it: &Interp, cmd: &str, mut each: impl FnMut(&[u8])) -> R<Option<i64>> {
    it.flush();
    let mut c = shell(cmd);
    c.stdout(Stdio::piped());
    let Ok(mut child) = c.spawn() else { return Ok(None) };
    if let Some(mut p) = child.stdout.take().and_then(pipe_of) {
        read_all(&mut p, &mut each);
    }
    let st = wait(it, &mut child)?;
    Ok(Some(exit_code(&st)))
}

fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

fn check_cmd(it: &Interp, fname: &str, cmd: &str) -> R<()> {
    if cmd.is_empty() {
        return Err(it.error("ValueError", &format!("{fname}(): Argument #1 ($command) cannot be empty")));
    }
    if cmd.contains('\0') {
        return Err(it.error("ValueError", &format!("{fname}(): Argument #1 ($command) must not contain any null bytes")));
    }
    Ok(())
}

/// Lines of command output as exec() stores them (trailing whitespace off).
fn lines(out: &[u8]) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = out.split(|&b| b == b'\n').map(|l| {
        let mut l = l.to_vec();
        while l.last().is_some_and(|c| c.is_ascii_whitespace()) {
            l.pop();
        }
        l
    }).collect();
    if out.ends_with(b"\n") || out.is_empty() {
        v.pop();
    }
    v
}

fn proc_of<T>(it: &Interp, v: &Value, fname: &str, g: impl FnOnce(&mut Proc) -> T) -> R<T> {
    if let Value::Resource(r) = v {
        let mut d = r.data.borrow_mut();
        if let Some(p) = d.as_mut().and_then(|b| b.downcast_mut::<Proc>()) {
            return Ok(g(p));
        }
    }
    Err(it.error("TypeError", &format!("{fname}(): supplied resource is not a valid process resource")))
}

/// A descriptor spec entry: what the child gets for that fd, plus the
/// parent's end when it's a pipe (mode as the parent sees it).
enum Spec {
    Pipe(&'static str),
    Stdio(Stdio),
    Inherit,
}

fn spec(it: &Interp, v: &Value, fd: usize) -> R<Spec> {
    match v {
        Value::Array(a) => {
            let kind = a.get(&Key::Int(0)).map(|k| it.to_str(&k)).transpose()?.map(|k| crate::pstr::lossy(&k).into_owned()).unwrap_or_default();
            let s1 = a.get(&Key::Int(1)).map(|k| it.to_str(&k)).transpose()?.map(|k| crate::pstr::lossy(&k).into_owned()).unwrap_or_default();
            match kind.as_str() {
                "pipe" => Ok(Spec::Pipe(if s1.starts_with('r') { "w" } else { "r" })),
                "file" => {
                    let mode = a.get(&Key::Int(2)).map(|k| it.to_str(&k)).transpose()?.map(|k| crate::pstr::lossy(&k).into_owned()).unwrap_or_else(|| "r".into());
                    let mut o = std::fs::OpenOptions::new();
                    match mode.chars().next().unwrap_or('r') {
                        'w' => o.write(true).create(true).truncate(true),
                        'a' => o.append(true).create(true),
                        'x' => o.write(true).create_new(true),
                        'c' => o.write(true).create(true),
                        _ => o.read(true),
                    };
                    if mode.contains('+') {
                        o.read(true).write(true);
                    }
                    match o.open(&s1) {
                        Ok(f) => Ok(Spec::Stdio(Stdio::from(f))),
                        Err(e) => Err(it.error("ValueError", &format!("proc_open(): Failed to open {s1}: {e}"))),
                    }
                }
                "null" => Ok(Spec::Stdio(Stdio::null())),
                "redirect" => {
                    let to = a.get(&Key::Int(1)).map(|k| it.to_int(&k)).transpose()?.unwrap_or(-1);
                    let dup = unsafe { libc::dup(to as i32) };
                    if dup < 0 {
                        return Err(it.error("ValueError", &format!("proc_open(): Argument #2 ($descriptor_spec) redirection target {to} not found")));
                    }
                    // SAFETY: a fresh fd from dup().
                    Ok(Spec::Stdio(unsafe { <Stdio as std::os::fd::FromRawFd>::from_raw_fd(dup) }))
                }
                _ => Err(it.error("ValueError", &format!("proc_open(): Argument #2 ($descriptor_spec) must contain only valid descriptor types for {fd}"))),
            }
        }
        Value::Resource(r) => {
            let d = r.data.borrow();
            match d.as_ref().and_then(|b| b.downcast_ref::<Stream>()).map(|s| &s.backend) {
                Some(Backend::File(f)) => Ok(Spec::Stdio(f.try_clone().map(Stdio::from).map_err(|e| it.error("Error", &e.to_string()))?)),
                _ => Ok(Spec::Inherit),
            }
        }
        _ => Err(it.error("ValueError", "proc_open(): Argument #2 ($descriptor_spec) must only contain arrays and streams")),
    }
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("proc_open", |it, _, a| {
        let (mut c, command) = match arg(a, 0) {
            Value::Array(parts) => {
                let parts: Vec<String> = parts.values().map(|v| it.to_str(&v).map(|s| crate::pstr::lossy(&s).into_owned())).collect::<R<_>>()?;
                if parts.is_empty() {
                    return Err(it.error("ValueError", "proc_open(): Argument #1 ($command) must have at least one element"));
                }
                let mut c = Command::new(&parts[0]);
                c.args(&parts[1..]);
                (c, parts.join(" "))
            }
            v => {
                let cmd = crate::pstr::lossy(&it.to_str(&v)?).into_owned();
                check_cmd(it, "proc_open", &cmd)?;
                (shell(&cmd), cmd)
            }
        };
        let mut pipes_wanted = vec![];
        if let Value::Array(desc) = arg(a, 1) {
            for (k, v) in desc.iter() {
                let Key::Int(fd) = k else { continue };
                let sp = spec(it, &v, fd as usize)?;
                match (fd, sp) {
                    (0..=2, Spec::Pipe(mode)) => {
                        let st = Stdio::piped();
                        match fd {
                            0 => c.stdin(st),
                            1 => c.stdout(st),
                            _ => c.stderr(st),
                        };
                        pipes_wanted.push((fd, mode));
                    }
                    (0..=2, Spec::Stdio(st)) => {
                        match fd {
                            0 => c.stdin(st),
                            1 => c.stdout(st),
                            _ => c.stderr(st),
                        };
                    }
                    _ => {}
                }
            }
        }
        if has(a, 3) && !arg(a, 3).is_null() {
            c.current_dir(text(it, a, 3)?);
        }
        if let Value::Array(env) = arg(a, 4) {
            c.env_clear();
            for (k, v) in env.iter() {
                let val = crate::pstr::lossy(&it.to_str(&v)?).into_owned();
                match k {
                    Key::Str(s) => {
                        c.env(crate::pstr::lossy(&s).into_owned(), val);
                    }
                    // A list entry is a whole "NAME=value" (Symfony Process
                    // passes its environment like this).
                    Key::Int(_) => {
                        if let Some((name, value)) = val.split_once('=') {
                            c.env(name, value);
                        }
                    }
                }
            }
        }
        it.flush();
        let mut child = match c.spawn() {
            Ok(ch) => ch,
            Err(e) => {
                it.warn(&format!("proc_open(): Exec failed: {e}"));
                return Ok(Value::Bool(false));
            }
        };
        let mut pipes = PhpArray::new();
        for (fd, mode) in pipes_wanted {
            let p = match fd {
                0 => child.stdin.take().and_then(pipe_of),
                1 => child.stdout.take().and_then(pipe_of),
                _ => child.stderr.take().and_then(pipe_of),
            };
            if let Some(p) = p {
                pipes.insert(Key::Int(fd), pipe_stream(Sock::pipe(p, None), mode));
            }
        }
        if a.len() > 2 {
            a[2] = Value::array(pipes);
        }
        let pid = child.id();
        Ok(Resource::new("process", Box::new(Proc { child: Some(child), pid, command, status: None })))
    });
    t.insert("proc_close", |it, _, a| {
        let v = arg(a, 0);
        // Take the child out so the wait doesn't hold the resource borrowed.
        let (status, child) = proc_of(it, &v, "proc_close", |p| (p.status, p.child.take()))?;
        let st = match (status, child) {
            (Some(st), _) => Some(st),
            (None, Some(mut ch)) => Some(wait(it, &mut ch)?),
            (None, None) => None,
        };
        if let Value::Resource(r) = &v {
            r.close();
        }
        Ok(Value::Int(st.map_or(-1, |st| exit_code(&st))))
    });
    t.insert("proc_get_status", |it, _, a| {
        proc_of(it, &arg(a, 0), "proc_get_status", |p| {
            if p.status.is_none() {
                p.status = p.child.as_mut().and_then(|c| c.try_wait().ok().flatten());
            }
            use std::os::unix::process::ExitStatusExt;
            let (running, code, sig, signaled) = match &p.status {
                None => (true, -1, 0, false),
                Some(st) => (false, st.code().map_or(-1, |c| c as i64), st.signal().unwrap_or(0) as i64, st.signal().is_some()),
            };
            let mut m = PhpArray::new();
            for (k, v) in [
                ("command", Value::str(&p.command)),
                ("pid", Value::Int(p.pid as i64)),
                ("cached", Value::Bool(p.status.is_some())),
                ("running", Value::Bool(running)),
                ("signaled", Value::Bool(signaled)),
                ("stopped", Value::Bool(false)),
                ("exitcode", Value::Int(code)),
                ("termsig", Value::Int(sig)),
                ("stopsig", Value::Int(0)),
            ] {
                m.insert(Key::Str(pstr(k)), v);
            }
            Value::array(m)
        })
    });
    t.insert("proc_terminate", |it, _, a| {
        let sig = if has(a, 1) { int(it, a, 1)? } else { 15 };
        proc_of(it, &arg(a, 0), "proc_terminate", |p| {
            // SAFETY: plain kill(2).
            Value::Bool(unsafe { libc::kill(p.pid as i32, sig as i32) } == 0)
        })
    });
    t.insert("proc_nice", |it, _, a| {
        let n = int(it, a, 0)?;
        // SAFETY: plain nice(3).
        Ok(Value::Bool(unsafe { libc::nice(n as i32) } != -1))
    });
    t.insert("exec", |it, _, a| {
        let cmd = text(it, a, 0)?;
        check_cmd(it, "exec", &cmd)?;
        let mut out = vec![];
        let Some(code) = run(it, &cmd, |b| out.extend_from_slice(b))? else { return Ok(Value::Bool(false)) };
        let ls = lines(&out);
        if a.len() > 1 {
            let mut arr = match std::mem::take(&mut a[1]).into_deref() {
                Value::Array(x) => (*x).clone(),
                _ => PhpArray::new(),
            };
            for l in &ls {
                arr.push(Value::bytes(l.clone()));
            }
            a[1] = Value::array(arr);
        }
        if a.len() > 2 {
            a[2] = Value::Int(code);
        }
        Ok(Value::bytes(ls.last().cloned().unwrap_or_default()))
    });
    t.insert("shell_exec", |it, _, a| {
        let cmd = text(it, a, 0)?;
        check_cmd(it, "shell_exec", &cmd)?;
        let mut out = vec![];
        match run(it, &cmd, |b| out.extend_from_slice(b))? {
            Some(_) if !out.is_empty() => Ok(Value::bytes(out)),
            Some(_) => Ok(Value::Null),
            None => Ok(Value::Bool(false)),
        }
    });
    // `cmd`: shell_exec(), with PHP 8.5's deprecation (once per place).
    t.insert("__phpx_backtick", |it, f, a| {
        thread_local! {
            static SEEN: std::cell::RefCell<std::collections::HashSet<(u32, u32)>> = Default::default();
        }
        let p = it.pos.get();
        if SEEN.with(|s| s.borrow_mut().insert((p.file, p.off))) {
            it.deprecated("The backtick (`) operator is deprecated, use shell_exec() instead");
        }
        (it.builtins["shell_exec"])(it, f, a)
    });
    t.insert("system", |it, _, a| {
        let cmd = text(it, a, 0)?;
        check_cmd(it, "system", &cmd)?;
        let mut all = vec![];
        let r = run(it, &cmd, |b| {
            it.echo(b);
            all.extend_from_slice(b);
        })?;
        let Some(code) = r else { return Ok(Value::Bool(false)) };
        if a.len() > 1 {
            a[1] = Value::Int(code);
        }
        Ok(Value::bytes(lines(&all).last().cloned().unwrap_or_default()))
    });
    t.insert("passthru", |it, _, a| {
        let cmd = text(it, a, 0)?;
        check_cmd(it, "passthru", &cmd)?;
        let Some(code) = run(it, &cmd, |b| it.echo(b))? else { return Ok(Value::Bool(false)) };
        if a.len() > 1 {
            a[1] = Value::Int(code);
        }
        Ok(Value::Null)
    });
    t.insert("popen", |it, _, a| {
        let cmd = text(it, a, 0)?;
        let mode = text(it, a, 1)?;
        let mut c = shell(&cmd);
        let reading = mode.starts_with('r');
        if reading {
            c.stdout(Stdio::piped());
        } else {
            c.stdin(Stdio::piped());
        }
        it.flush();
        let Ok(mut child) = c.spawn() else {
            it.warn(&format!("popen({cmd},{mode}): No such file or directory"));
            return Ok(Value::Bool(false));
        };
        let p = if reading { child.stdout.take().and_then(pipe_of) } else { child.stdin.take().and_then(pipe_of) };
        match p {
            Some(p) => Ok(pipe_stream(Sock::pipe(p, Some(child)), &mode)),
            None => Ok(Value::Bool(false)),
        }
    });
    t.insert("pclose", |it, _, a| {
        let v = arg(a, 0);
        let Value::Resource(r) = &v else { return Ok(Value::Int(-1)) };
        let child = {
            let mut d = r.data.borrow_mut();
            d.as_mut().and_then(|b| b.downcast_mut::<Stream>()).and_then(|s| match &mut s.backend {
                Backend::Socket(sk) => sk.child.take(),
                _ => None,
            })
        };
        // Close our end first so the child sees EOF.
        r.close();
        match child {
            Some(mut ch) => Ok(Value::Int(exit_code(&wait(it, &mut ch)?))),
            None => Ok(Value::Int(-1)),
        }
    });
}
