//! Minimal HTTP/1.1 server: one green thread per connection, one OS thread total.
//!
//! What the compiler will emit for routes like:
//!
//! ```php
//! Route::get('/', fn() => 'hello');
//! Route::get('/slow', function () {
//!     $row = DB::query('select ...');          // ~50ms, parks this request only
//!     return 'done';
//! });
//! Route::get('/profile', function () {
//!     [$posts, $friends] = Concurrency::run([  // 50ms + 50ms overlapped
//!         fn() => Http::get('/posts'),
//!         fn() => DB::friends(),
//!     ]);
//!     return 'profile';
//! });
//! ```
//!
//! Run:   cargo run --release --example http
//! Load:  wrk -t4 -c1000 -d5s http://127.0.0.1:8080/slow

use std::time::Duration;

use phpx_runtime::net::{TcpListener, TcpStream};
use phpx_runtime::{block_on, concurrency, sleep, spawn, Result};

fn fake_io(ms: u64) -> Result<()> {
    sleep(Duration::from_millis(ms))
}

fn route(path: &str) -> Result<(u16, String)> {
    Ok(match path {
        "/" => (200, "hello\n".into()),
        "/slow" => {
            fake_io(50)?;
            (200, "done\n".into())
        }
        "/profile" => {
            concurrency::run(vec![Box::new(|| fake_io(50)), Box::new(|| fake_io(50))])?;
            (200, "profile\n".into())
        }
        _ => (404, "not found\n".into()),
    })
}

/// Serve requests on one keep-alive connection until the client closes it.
fn handle(mut conn: TcpStream) -> Result<()> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        // Read until a full header block is buffered.
        let end = loop {
            if let Some(i) = find(&buf, b"\r\n\r\n") {
                break i + 4;
            }
            match conn.read(&mut chunk)? {
                0 => return Ok(()),
                n => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
        let mut body_len = 0;
        let mut close = false;
        for line in head.lines().skip(1) {
            if let Some((k, v)) = line.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "content-length" => body_len = v.trim().parse().unwrap_or(0),
                    "connection" => close = v.trim().eq_ignore_ascii_case("close"),
                    _ => {}
                }
            }
        }
        // Skip the request body (this demo ignores it).
        while buf.len() < end + body_len {
            match conn.read(&mut chunk)? {
                0 => return Ok(()),
                n => buf.extend_from_slice(&chunk[..n]),
            }
        }
        buf.drain(..end + body_len);

        let path = head.split_whitespace().nth(1).unwrap_or("/");
        let (status, body) = route(path).unwrap_or_else(|e| (500, format!("{e}\n")));
        let reason = match status {
            200 => "OK",
            404 => "Not Found",
            _ => "Internal Server Error",
        };
        let resp = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n{}\r\n{body}",
            body.len(),
            if close { "Connection: close\r\n" } else { "" },
        );
        conn.write_all(resp.as_bytes())?;
        if close {
            return Ok(());
        }
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn main() {
    let addr = std::env::var("ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let r = block_on::<(), _>(move || {
        let listener = TcpListener::bind(addr.as_str())?;
        println!("listening on http://{}", listener.local_addr()?);
        loop {
            let (conn, _) = listener.accept()?;
            // A broken connection only ends its own green thread.
            spawn(move || handle(conn));
        }
    });
    if let Err(e) = r {
        eprintln!("server error: {e}");
    }
}
