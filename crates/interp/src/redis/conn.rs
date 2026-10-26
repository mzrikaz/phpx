//! A Redis connection: RESP2 over a runtime socket (TCP, TLS or Unix), so
//! a command parks only its green thread while it waits.

use std::time::{Duration, Instant};

use phpx_runtime::net::Stream;
use phpx_runtime::tls::{Options as TlsOptions, TlsStream};

#[derive(Debug, Clone)]
pub enum Reply {
    Status(String),
    Error(String),
    Int(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Reply>>),
}

enum Io {
    Plain(Stream),
    Tls(Box<TlsStream>),
}

pub struct Conn {
    io: Io,
    rbuf: Vec<u8>,
    rpos: usize,
    /// Per-read timeout (OPT_READ_TIMEOUT); None waits forever.
    pub read_timeout: Option<Duration>,
}

/// A connection-level failure (PHP's RedisException message).
pub struct IoError(pub String);

fn io_message(e: &phpx_runtime::Error) -> String {
    let msg = e.to_string();
    let msg = msg.trim_start_matches("io: ").to_string();
    if msg.contains("refused") {
        "Connection refused".into()
    } else if msg.contains("timed out") {
        "Connection timed out".into()
    } else if msg.contains("No such file") {
        "No such file or directory".into()
    } else if msg.contains("lookup") || msg.contains("Name or service") || msg.contains("resolve") || msg.contains("failed to lookup") {
        "php_network_getaddresses: getaddrinfo failed: Name or service not known".into()
    } else {
        msg
    }
}

impl Conn {
    /// `host`: a name or IP, `tls://host`, `unix:///path` or `/path`.
    pub fn connect(host: &str, port: u16, timeout: Option<Duration>) -> Result<Conn, IoError> {
        let deadline = timeout.map(|t| Instant::now() + t);
        let (tls, host) = match host.strip_prefix("tls://").or_else(|| host.strip_prefix("ssl://")) {
            Some(h) => (true, h),
            None => (false, host.strip_prefix("tcp://").unwrap_or(host)),
        };
        let unix = host.strip_prefix("unix://").or_else(|| host.starts_with('/').then_some(host));
        let stream = match unix {
            Some(path) => Stream::connect_unix(path, deadline),
            None => Stream::connect_tcp((host, port), deadline),
        }
        .map_err(|e| IoError(io_message(&e)))?;
        let io = if tls {
            let t = TlsStream::connect(stream, host, &TlsOptions::default(), deadline).map_err(|e| IoError(format!("SSL error: {e}")))?;
            Io::Tls(Box::new(t))
        } else {
            Io::Plain(stream)
        };
        Ok(Conn { io, rbuf: Vec::with_capacity(16384), rpos: 0, read_timeout: None })
    }

    pub fn send(&mut self, args: &[Vec<u8>]) -> Result<(), IoError> {
        let mut buf = Vec::with_capacity(32 + args.iter().map(|a| a.len() + 16).sum::<usize>());
        encode(&mut buf, args);
        self.write(&buf)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<(), IoError> {
        let r = match &mut self.io {
            Io::Plain(s) => s.write_all(buf, None),
            Io::Tls(t) => t.write_all(buf, None),
        };
        r.map_err(|_| IoError("Redis server went away".into()))
    }

    fn fill(&mut self) -> Result<(), IoError> {
        if self.rpos > 0 && self.rpos == self.rbuf.len() {
            self.rbuf.clear();
            self.rpos = 0;
        }
        let mut chunk = [0u8; 16384];
        let deadline = self.read_timeout.map(|t| Instant::now() + t);
        let n = match &mut self.io {
            Io::Plain(s) => s.read(&mut chunk, deadline),
            Io::Tls(t) => t.read(&mut chunk, deadline),
        }
        .map_err(|e| {
            if e.to_string().contains("timed out") {
                IoError("read error on connection".into())
            } else {
                IoError("Redis server went away".into())
            }
        })?;
        if n == 0 {
            return Err(IoError("Redis server went away".into()));
        }
        self.rbuf.extend_from_slice(&chunk[..n]);
        Ok(())
    }

    fn line(&mut self) -> Result<Vec<u8>, IoError> {
        loop {
            if let Some(p) = memchr::memmem::find(&self.rbuf[self.rpos..], b"\r\n") {
                let line = self.rbuf[self.rpos..self.rpos + p].to_vec();
                self.rpos += p + 2;
                return Ok(line);
            }
            self.fill()?;
        }
    }

    fn exact(&mut self, n: usize) -> Result<Vec<u8>, IoError> {
        while self.rbuf.len() - self.rpos < n + 2 {
            self.fill()?;
        }
        let v = self.rbuf[self.rpos..self.rpos + n].to_vec();
        self.rpos += n + 2;
        Ok(v)
    }

    pub fn read(&mut self) -> Result<Reply, IoError> {
        let line = self.line()?;
        let (kind, rest) = line.split_first().ok_or_else(|| IoError("protocol error".into()))?;
        let text = crate::pstr::lossy(rest).into_owned();
        let num = || text.parse::<i64>().map_err(|_| IoError("protocol error".into()));
        Ok(match kind {
            b'+' => Reply::Status(text),
            b'-' => Reply::Error(text),
            b':' => Reply::Int(num()?),
            b'$' => {
                let n = num()?;
                if n < 0 {
                    Reply::Bulk(None)
                } else {
                    Reply::Bulk(Some(self.exact(n as usize)?))
                }
            }
            b'*' => {
                let n = num()?;
                if n < 0 {
                    Reply::Array(None)
                } else {
                    let mut items = Vec::with_capacity(n as usize);
                    for _ in 0..n {
                        items.push(self.read()?);
                    }
                    Reply::Array(Some(items))
                }
            }
            _ => return Err(IoError("protocol error".into())),
        })
    }

    pub fn command(&mut self, args: &[Vec<u8>]) -> Result<Reply, IoError> {
        self.send(args)?;
        self.read()
    }
}

pub fn encode(buf: &mut Vec<u8>, args: &[Vec<u8>]) {
    buf.extend_from_slice(format!("*{}\r\n", args.len()).as_bytes());
    for a in args {
        buf.extend_from_slice(format!("${}\r\n", a.len()).as_bytes());
        buf.extend_from_slice(a);
        buf.extend_from_slice(b"\r\n");
    }
}
