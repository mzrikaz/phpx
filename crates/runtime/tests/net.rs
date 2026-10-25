use std::time::{Duration, Instant};

use phpx_runtime::net::{TcpListener, TcpStream};
use phpx_runtime::{block_on, sleep, spawn, Error, Promise, Result};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Echo one message back, after `delay` (fake slow handler).
fn echo_once(mut s: TcpStream, delay: u64) -> Result<()> {
    let mut buf = vec![];
    s.read_to_end(&mut buf)?;
    sleep(ms(delay))?;
    s.write_all(&buf)
}

fn roundtrip(addr: std::net::SocketAddr, msg: String) -> Result<String> {
    let mut c = TcpStream::connect(addr)?;
    c.write_all(msg.as_bytes())?;
    c.shutdown_write()?;
    let mut out = vec![];
    c.read_to_end(&mut out)?;
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn echo_roundtrip() {
    let r = block_on(|| {
        let l = TcpListener::bind("127.0.0.1:0")?;
        let addr = l.local_addr()?;
        let server = spawn(move || {
            let (s, _) = l.accept()?;
            echo_once(s, 0)
        });
        let reply = roundtrip(addr, "hello".into())?;
        server.wait()?;
        Ok(reply)
    });
    assert_eq!(r.unwrap(), "hello");
}

#[test]
fn many_clients_overlap_on_one_thread() {
    const N: usize = 200;
    let start = Instant::now();
    let r = block_on(|| {
        let l = TcpListener::bind("127.0.0.1:0")?;
        let addr = l.local_addr()?;
        spawn(move || {
            for _ in 0..N {
                let (s, _) = l.accept()?;
                spawn(move || echo_once(s, 100)); // each handler sleeps 100ms
            }
            Ok(())
        });
        let clients = (0..N).map(|i| spawn(move || roundtrip(addr, format!("msg{i}")))).collect();
        Promise::all(clients).wait()
    });
    let replies = r.unwrap();
    assert_eq!(replies.len(), N);
    assert_eq!(replies[7], "msg7");
    // 200 x 100ms handlers overlapped, not 20s sequential.
    assert!(start.elapsed() < ms(1_000), "took {:?}", start.elapsed());
}

#[test]
fn accept_can_time_out() {
    let start = Instant::now();
    let r = block_on(|| {
        let l = TcpListener::bind("127.0.0.1:0")?;
        spawn(move || l.accept().map(|_| ())).timeout(ms(30)).wait()
    });
    assert_eq!(r, Err(Error::Timeout));
    // Cancelled accept dropped its fd waiter, so the runtime exited right away.
    assert!(start.elapsed() < ms(200));
}

#[test]
fn connect_refused_is_error() {
    let r = block_on(|| {
        // Bind then drop to get a port nobody listens on.
        let addr = TcpListener::bind("127.0.0.1:0")?.local_addr()?;
        TcpStream::connect(addr).map(|_| ())
    });
    assert!(matches!(r, Err(Error::Io(std::io::ErrorKind::ConnectionRefused, _))), "{r:?}");
}
