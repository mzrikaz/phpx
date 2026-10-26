//! Non-blocking sockets that read like blocking code.
//!
//! Every socket is in non-blocking mode. When an operation would block, the
//! task parks on the reactor until the fd is ready, and other tasks run.
//! Operations take an optional deadline (PHP's socket timeouts); outside a
//! task (e.g. destructors at shutdown) they fall back to poll(2).

use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, ToSocketAddrs};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};
use std::path::Path;
use std::time::Instant;

use socket2::{Protocol, SockAddr, Socket, Type};

use crate::error::{Error, Result};
use crate::scheduler;

fn timed_out() -> Error {
    Error::Io(io::ErrorKind::TimedOut, "timed out".into())
}

/// Wait until any of `fds` is ready or `deadline` passes: (readable,
/// writable) per fd, all false on timeout. Each fd must belong to a socket
/// from this module.
pub fn select(fds: &[(BorrowedFd<'_>, bool, bool)], deadline: Option<Instant>) -> Result<Vec<(bool, bool)>> {
    if scheduler::current_task().is_some() {
        // Ready already (or a zero timeout): answer without parking.
        let now = poll_fds(fds, Some(Instant::now()))?;
        if now.iter().any(|&(r, w)| r || w) || deadline.is_some_and(|d| Instant::now() >= d) {
            return Ok(now);
        }
        return scheduler::wait_fds(fds, deadline);
    }
    poll_fds(fds, deadline)
}

/// poll(2) until `deadline` (None: forever).
fn poll_fds(fds: &[(BorrowedFd<'_>, bool, bool)], deadline: Option<Instant>) -> Result<Vec<(bool, bool)>> {
    let mut p: Vec<libc::pollfd> = fds
        .iter()
        .map(|&(fd, r, w)| libc::pollfd {
            fd: fd.as_raw_fd(),
            events: if r { libc::POLLIN } else { 0 } | if w { libc::POLLOUT } else { 0 },
            revents: 0,
        })
        .collect();
    loop {
        let ms = match deadline {
            Some(d) => d.saturating_duration_since(Instant::now()).as_millis().min(i32::MAX as u128) as i32,
            None => -1,
        };
        // SAFETY: `p` is a valid array of pollfd for its length.
        let n = unsafe { libc::poll(p.as_mut_ptr(), p.len() as libc::nfds_t, ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.into());
        }
        let err = libc::POLLERR | libc::POLLHUP;
        return Ok(p.iter().map(|x| (x.revents & (libc::POLLIN | err) != 0, x.revents & (libc::POLLOUT | err) != 0)).collect());
    }
}

/// Retry `op` until it stops returning `WouldBlock`, parking in between.
fn retry<R>(fd: BorrowedFd<'_>, readable: bool, deadline: Option<Instant>, mut op: impl FnMut() -> io::Result<R>) -> Result<R> {
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Err(timed_out());
                }
                select(&[(fd, readable, !readable)], deadline)?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
}

fn register<S: AsFd>(sock: &S) -> Result<()> {
    scheduler::register_io(sock.as_fd()).map_err(Error::from)
}

/// Resolve a host name. IP literals skip the resolver.
pub fn resolve(addr: impl ToSocketAddrs) -> Result<Vec<SocketAddr>> {
    // TODO: getaddrinfo blocks the thread; move to a resolver pool.
    let v: Vec<SocketAddr> = addr.to_socket_addrs()?.collect();
    if v.is_empty() {
        return Err(Error::Io(io::ErrorKind::InvalidInput, "address resolved to nothing".into()));
    }
    Ok(v)
}

/// A connected stream socket (TCP or Unix domain).
pub struct Stream {
    sock: Socket,
}

impl Stream {
    fn new(sock: Socket) -> Result<Self> {
        sock.set_nonblocking(true)?;
        // Socket::domain() needs SO_DOMAIN, which macOS lacks.
        if sock.local_addr().is_ok_and(|a| !a.is_unix()) {
            let _ = sock.set_nodelay(true);
        }
        register(&sock)?;
        Ok(Self { sock })
    }

    fn connect_addr(addr: SockAddr, deadline: Option<Instant>) -> Result<Self> {
        let proto = if addr.is_unix() { None } else { Some(Protocol::TCP) };
        let sock = Socket::new(addr.domain(), Type::STREAM, proto)?;
        sock.set_nonblocking(true)?;
        let started = match sock.connect(&addr) {
            Ok(()) => false,
            Err(e) if e.raw_os_error() == Some(libc::EINPROGRESS) || e.kind() == io::ErrorKind::WouldBlock => true,
            Err(e) => return Err(e.into()),
        };
        let stream = Self::new(sock)?;
        if started {
            let ready = select(&[(stream.sock.as_fd(), false, true)], deadline)?;
            if !ready[0].1 {
                return Err(timed_out());
            }
            if let Some(e) = stream.sock.take_error()? {
                return Err(e.into());
            }
        }
        Ok(stream)
    }

    /// Connect over TCP, trying each resolved address in turn.
    pub fn connect_tcp(addr: impl ToSocketAddrs, deadline: Option<Instant>) -> Result<Self> {
        let mut last = None;
        for a in resolve(addr)? {
            match Self::connect_addr(a.into(), deadline) {
                Ok(s) => return Ok(s),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap())
    }

    pub fn connect_unix(path: impl AsRef<Path>, deadline: Option<Instant>) -> Result<Self> {
        Self::connect_addr(SockAddr::unix(path)?, deadline)
    }

    pub fn local_addr(&self) -> Option<String> {
        addr_string(&self.sock.local_addr().ok()?)
    }

    pub fn peer_addr(&self) -> Option<String> {
        addr_string(&self.sock.peer_addr().ok()?)
    }

    /// Read some bytes. `Ok(0)` means the peer closed the connection.
    pub fn read(&mut self, buf: &mut [u8], deadline: Option<Instant>) -> Result<usize> {
        let s = &self.sock;
        retry(s.as_fd(), true, deadline, || (&*s).read(buf))
    }

    /// Read what's there now without waiting (`WouldBlock` gives Ok(None)).
    pub fn try_read(&mut self, buf: &mut [u8]) -> Result<Option<usize>> {
        match (&self.sock).read(buf) {
            Ok(n) => Ok(Some(n)),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn write(&mut self, buf: &[u8], deadline: Option<Instant>) -> Result<usize> {
        let s = &self.sock;
        retry(s.as_fd(), false, deadline, || (&*s).write(buf))
    }

    pub fn write_all(&mut self, mut buf: &[u8], deadline: Option<Instant>) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf, deadline)? {
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
            match self.read(&mut buf, None)? {
                0 => return Ok(out.len() - start),
                n => out.extend_from_slice(&buf[..n]),
            }
        }
    }

    pub fn shutdown(&self, how: Shutdown) -> Result<()> {
        Ok(self.sock.shutdown(how)?)
    }

    /// Half-close: tell the peer we're done writing.
    pub fn shutdown_write(&self) -> Result<()> {
        self.shutdown(Shutdown::Write)
    }

    pub fn raw_fd(&self) -> RawFd {
        self.sock.as_raw_fd()
    }
}

impl AsFd for Stream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.sock.as_fd()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        scheduler::deregister_io(self.sock.as_fd());
    }
}

/// `host:port` / `[v6]:port` / Unix path, as PHP prints socket names.
fn addr_string(a: &SockAddr) -> Option<String> {
    if let Some(s) = a.as_socket() {
        return Some(s.to_string());
    }
    a.as_pathname().map(|p| p.to_string_lossy().into_owned()).or(Some(String::new()))
}

/// A listening stream socket (TCP or Unix domain).
pub struct Listener {
    sock: Socket,
}

impl Listener {
    fn listen(addr: SockAddr) -> Result<Self> {
        let proto = if addr.is_unix() { None } else { Some(Protocol::TCP) };
        let sock = Socket::new(addr.domain(), Type::STREAM, proto)?;
        if !addr.is_unix() {
            sock.set_reuse_address(true)?;
        }
        sock.set_nonblocking(true)?;
        sock.bind(&addr)?;
        sock.listen(1024)?;
        register(&sock)?;
        Ok(Self { sock })
    }

    pub fn bind_tcp(addr: impl ToSocketAddrs) -> Result<Self> {
        Self::listen(resolve(addr)?[0].into())
    }

    pub fn bind_unix(path: impl AsRef<Path>) -> Result<Self> {
        Self::listen(SockAddr::unix(path)?)
    }

    pub fn local_addr(&self) -> Option<String> {
        addr_string(&self.sock.local_addr().ok()?)
    }

    /// Park until a client connects (or `deadline` passes).
    pub fn accept(&self, deadline: Option<Instant>) -> Result<(Stream, Option<String>)> {
        let (s, peer) = retry(self.sock.as_fd(), true, deadline, || self.sock.accept())?;
        Ok((Stream::new(s)?, addr_string(&peer)))
    }

    pub fn raw_fd(&self) -> RawFd {
        self.sock.as_raw_fd()
    }
}

impl AsFd for Listener {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.sock.as_fd()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        scheduler::deregister_io(self.sock.as_fd());
    }
}

/// TCP listener (the demo HTTP server).
pub struct TcpListener(Listener);

impl TcpListener {
    pub fn bind(addr: impl ToSocketAddrs) -> Result<Self> {
        Listener::bind_tcp(addr).map(TcpListener)
    }

    pub fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.sock.local_addr()?.as_socket().expect("TCP address"))
    }

    /// Park until a client connects.
    pub fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (s, peer) = retry(self.0.sock.as_fd(), true, None, || self.0.sock.accept())?;
        let peer = peer.as_socket().expect("TCP address");
        Ok((TcpStream(Stream::new(s)?), peer))
    }
}

/// TCP stream without deadlines.
pub struct TcpStream(Stream);

impl TcpStream {
    /// Park until the TCP handshake finishes.
    pub fn connect(addr: impl ToSocketAddrs) -> Result<Self> {
        Stream::connect_tcp(addr, None).map(TcpStream)
    }

    pub fn peer_addr(&self) -> Result<SocketAddr> {
        Ok(self.0.sock.peer_addr()?.as_socket().expect("TCP address"))
    }

    /// Read some bytes. `Ok(0)` means the peer closed the connection.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf, None)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.0.write(buf, None)
    }

    pub fn write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.0.write_all(buf, None)
    }

    /// Read until the peer closes its side.
    pub fn read_to_end(&mut self, out: &mut Vec<u8>) -> Result<usize> {
        self.0.read_to_end(out)
    }

    /// Half-close: tell the peer we're done writing.
    pub fn shutdown_write(&self) -> Result<()> {
        self.0.shutdown_write()
    }
}

/// A non-blocking pipe end (a child process's stdin/stdout/stderr).
pub struct Pipe {
    fd: std::os::fd::OwnedFd,
}

impl Pipe {
    /// Take ownership of `fd`, make it non-blocking and register it.
    pub fn from_fd(fd: std::os::fd::OwnedFd) -> Result<Self> {
        // SAFETY: fcntl on an fd we own.
        unsafe {
            let raw = fd.as_raw_fd();
            let flags = libc::fcntl(raw, libc::F_GETFL);
            libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        register(&fd)?;
        Ok(Pipe { fd })
    }

    fn sys<R>(&self, readable: bool, deadline: Option<Instant>, op: impl FnMut() -> io::Result<R>) -> Result<R> {
        retry(self.fd.as_fd(), readable, deadline, op)
    }

    /// Read some bytes; Ok(0) at end of stream.
    pub fn read(&mut self, buf: &mut [u8], deadline: Option<Instant>) -> Result<usize> {
        let fd = self.fd.as_raw_fd();
        self.sys(true, deadline, || {
            // SAFETY: valid buffer and fd.
            let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
            if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
        })
    }

    pub fn write(&mut self, buf: &[u8], deadline: Option<Instant>) -> Result<usize> {
        let fd = self.fd.as_raw_fd();
        self.sys(false, deadline, || {
            // SAFETY: valid buffer and fd.
            let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
            if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
        })
    }

    pub fn write_all(&mut self, mut buf: &[u8], deadline: Option<Instant>) -> Result<()> {
        while !buf.is_empty() {
            match self.write(buf, deadline)? {
                0 => return Err(io::Error::from(io::ErrorKind::WriteZero).into()),
                n => buf = &buf[n..],
            }
        }
        Ok(())
    }
}

impl AsFd for Pipe {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        scheduler::deregister_io(self.fd.as_fd());
    }
}
