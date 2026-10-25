//! Non-blocking TCP that reads like blocking code.
//!
//! Every socket is in non-blocking mode. When an operation would block, the
//! task parks on the reactor until the fd is ready, and other tasks run.

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, ToSocketAddrs};
use std::os::fd::AsFd;

use socket2::{Domain, Protocol, Socket, Type};

use crate::error::{Error, Result};
use crate::scheduler;

/// Retry `op` until it stops returning `WouldBlock`, parking in between.
fn retry<S: AsFd, R>(sock: &S, readable: bool, mut op: impl FnMut() -> io::Result<R>) -> Result<R> {
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                scheduler::wait_io(sock.as_fd(), readable)?
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
}

fn register<S: AsFd>(sock: &S) -> Result<()> {
    scheduler::register_io(sock.as_fd()).map_err(Error::from)
}

fn resolve(addr: impl ToSocketAddrs) -> Result<SocketAddr> {
    // TODO: DNS resolution blocks the thread; move to a resolver pool.
    addr.to_socket_addrs()?
        .next()
        .ok_or_else(|| Error::Io(io::ErrorKind::InvalidInput, "address resolved to nothing".into()))
}

pub struct TcpListener {
    inner: std::net::TcpListener,
}

impl TcpListener {
    pub fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        let addr = resolve(addr)?;
        let sock = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
        sock.set_reuse_address(true)?;
        sock.set_nonblocking(true)?;
        sock.bind(&addr.into())?;
        sock.listen(1024)?;
        let inner: std::net::TcpListener = sock.into();
        register(&inner)?;
        Ok(Self { inner })
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.inner.local_addr()?)
    }

    /// Park until a client connects.
    pub fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (s, peer) = retry(&self.inner, true, || self.inner.accept())?;
        Ok((TcpStream::from_std(s)?, peer))
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        scheduler::deregister_io(self.inner.as_fd());
    }
}

pub struct TcpStream {
    inner: std::net::TcpStream,
}

impl TcpStream {
    fn from_std(inner: std::net::TcpStream) -> Result<Self> {
        inner.set_nonblocking(true)?;
        inner.set_nodelay(true)?;
        register(&inner)?;
        Ok(Self { inner })
    }

    /// Park until the TCP handshake finishes.
    pub fn connect(addr: impl ToSocketAddrs) -> Result<Self> {
        let addr = resolve(addr)?;
        let sock = Socket::new(Domain::for_address(addr), Type::STREAM, Some(Protocol::TCP))?;
        sock.set_nonblocking(true)?;
        match sock.connect(&addr.into()) {
            Ok(()) => {}
            Err(e) if e.raw_os_error() == Some(libc_einprogress()) => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e.into()),
        }
        let stream = Self::from_std(sock.into())?;
        scheduler::wait_io(stream.inner.as_fd(), false)?;
        if let Some(e) = stream.inner.take_error()? {
            return Err(e.into());
        }
        Ok(stream)
    }

    pub fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.inner.peer_addr()?)
    }

    /// Read some bytes. `Ok(0)` means the peer closed the connection.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        let s = &self.inner;
        retry(s, true, || (&*s).read(buf))
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let s = &self.inner;
        retry(s, false, || (&*s).write(buf))
    }

    pub fn write_all(&mut self, mut buf: &[u8]) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf)? {
                0 => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
                n => buf = &buf[n..],
            }
        }
        Ok(())
    }

    /// Read until the peer closes its side.
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize> {
        let start = out.len();
        let mut buf = [0u8; 8192];
        loop {
            match self.read(&mut buf)? {
                0 => return Ok(out.len() - start),
                n => out.extend_from_slice(&buf[..n]),
            }
        }
    }

    /// Half-close: tell the peer we're done writing.
    pub fn shutdown_write(&self) -> Result<()> {
        Ok(self.inner.shutdown(Shutdown::Write)?)
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        scheduler::deregister_io(self.inner.as_fd());
    }
}

/// EINPROGRESS: same value on Linux (115) differs on BSD/macOS (36).
const fn libc_einprogress() -> i32 {
    if cfg!(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")) {
        36
    } else {
        115
    }
}
