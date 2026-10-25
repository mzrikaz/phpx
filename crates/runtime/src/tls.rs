//! TLS client streams (rustls) on top of `net::Stream`.
//!
//! Handshake, reads and writes park the task like plain socket I/O. Roots
//! come from the system store (what PHP's OpenSSL uses by default) unless a
//! CA file is given.

use std::io::{self, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme};

use crate::error::{Error, Result};
use crate::net::Stream;

/// PHP's `ssl` stream context options that matter for a client.
#[derive(Clone, Debug)]
pub struct Options {
    pub verify_peer: bool,
    pub verify_peer_name: bool,
    pub allow_self_signed: bool,
    pub cafile: Option<PathBuf>,
    pub capath: Option<PathBuf>,
    /// Name to verify (and send as SNI) instead of the connect host.
    pub peer_name: Option<String>,
    pub sni: bool,
    /// ALPN protocols, most preferred first (e.g. `h2`, `http/1.1`).
    pub alpn: Vec<Vec<u8>>,
}

impl Default for Options {
    fn default() -> Self {
        Options { verify_peer: true, verify_peer_name: true, allow_self_signed: false, cafile: None, capath: None, peer_name: None, sni: true, alpn: vec![] }
    }
}

fn tls_err(msg: impl std::fmt::Display) -> Error {
    Error::Io(io::ErrorKind::Other, msg.to_string())
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    static P: OnceLock<Arc<rustls::crypto::CryptoProvider>> = OnceLock::new();
    P.get_or_init(|| Arc::new(rustls::crypto::ring::default_provider())).clone()
}

fn system_roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut store = RootCertStore::empty();
            for c in rustls_native_certs::load_native_certs().certs {
                let _ = store.add(c);
            }
            Arc::new(store)
        })
        .clone()
}

fn roots(o: &Options) -> Result<Arc<RootCertStore>> {
    if o.cafile.is_none() && o.capath.is_none() {
        return Ok(system_roots());
    }
    let mut store = RootCertStore::empty();
    let mut files = vec![];
    files.extend(o.cafile.clone());
    if let Some(dir) = &o.capath {
        for e in std::fs::read_dir(dir).map_err(Error::from)?.flatten() {
            files.push(e.path());
        }
    }
    for f in files {
        for c in CertificateDer::pem_file_iter(&f).map_err(tls_err)?.flatten() {
            let _ = store.add(c);
        }
    }
    Ok(Arc::new(store))
}

/// Chain check via webpki, relaxed per the PHP options.
#[derive(Debug)]
struct Verifier {
    inner: Arc<WebPkiServerVerifier>,
    verify_peer: bool,
    verify_name: bool,
    allow_self_signed: bool,
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        inter: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if !self.verify_peer {
            return Ok(ServerCertVerified::assertion());
        }
        match self.inner.verify_server_cert(end, inter, name, ocsp, now) {
            Ok(v) => Ok(v),
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidForName | rustls::CertificateError::NotValidForNameContext { .. }))
                if !self.verify_name =>
            {
                Ok(ServerCertVerified::assertion())
            }
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer)) if self.allow_self_signed && inter.is_empty() => {
                Ok(ServerCertVerified::assertion())
            }
            Err(e) => Err(e),
        }
    }

    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }

    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

fn config(o: &Options) -> Result<Arc<ClientConfig>> {
    let roots = roots(o)?;
    let builder = ClientConfig::builder_with_provider(provider()).with_safe_default_protocol_versions().map_err(tls_err)?;
    let mut cfg = if o.verify_peer && o.verify_peer_name && !o.allow_self_signed {
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        let inner = WebPkiServerVerifier::builder_with_provider(roots, provider()).build().map_err(tls_err)?;
        let v = Verifier { inner, verify_peer: o.verify_peer, verify_name: o.verify_peer_name, allow_self_signed: o.allow_self_signed };
        builder.dangerous().with_custom_certificate_verifier(Arc::new(v)).with_no_client_auth()
    };
    cfg.alpn_protocols = o.alpn.clone();
    cfg.enable_sni = o.sni;
    Ok(Arc::new(cfg))
}

/// Default-options configs are built once per thread (loading roots and
/// building the verifier costs milliseconds).
fn cached_config(o: &Options) -> Result<Arc<ClientConfig>> {
    thread_local! {
        static DEFAULT: std::cell::RefCell<Option<Arc<ClientConfig>>> = const { std::cell::RefCell::new(None) };
    }
    let plain = o.verify_peer && o.verify_peer_name && !o.allow_self_signed && o.cafile.is_none() && o.capath.is_none() && o.sni && o.alpn.is_empty();
    if !plain {
        return config(o);
    }
    DEFAULT.with(|d| {
        if let Some(c) = &*d.borrow() {
            return Ok(c.clone());
        }
        let c = config(o)?;
        *d.borrow_mut() = Some(c.clone());
        Ok(c)
    })
}

/// The socket as blocking io::Read/Write for rustls (parks instead).
struct Io {
    sock: Stream,
    deadline: Option<Instant>,
}

fn io_err(e: Error) -> io::Error {
    match e {
        Error::Io(k, m) => io::Error::new(k, m),
        other => io::Error::other(other.to_string()),
    }
}

impl Read for Io {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.sock.read(buf, self.deadline).map_err(io_err)
    }
}

impl Write for Io {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sock.write(buf, self.deadline).map_err(io_err)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub struct TlsStream {
    conn: ClientConnection,
    io: Io,
}

impl TlsStream {
    /// Handshake as a client over a connected socket.
    pub fn connect(sock: Stream, host: &str, o: &Options, deadline: Option<Instant>) -> Result<Self> {
        let name = o.peer_name.as_deref().unwrap_or(host);
        let name = ServerName::try_from(name.trim_start_matches('[').trim_end_matches(']').to_string()).map_err(tls_err)?;
        let conn = ClientConnection::new(cached_config(o)?, name).map_err(tls_err)?;
        let mut s = TlsStream { conn, io: Io { sock, deadline } };
        while s.conn.is_handshaking() {
            s.conn.complete_io(&mut s.io).map_err(Error::from)?;
        }
        s.io.deadline = None;
        Ok(s)
    }

    /// Negotiated ALPN protocol.
    pub fn alpn(&self) -> Option<&[u8]> {
        self.conn.alpn_protocol()
    }

    /// (protocol, cipher, bits) in OpenSSL's naming, as PHP reports them.
    pub fn info(&self) -> Option<(String, String, u32)> {
        let suite = format!("{:?}", self.conn.negotiated_cipher_suite()?.suite());
        let proto = match self.conn.protocol_version()? {
            rustls::ProtocolVersion::TLSv1_3 => "TLSv1.3",
            rustls::ProtocolVersion::TLSv1_2 => "TLSv1.2",
            _ => "TLSv1",
        };
        let bits = if suite.contains("_256_") || suite.contains("CHACHA20") { 256 } else { 128 };
        let name = match suite.strip_prefix("TLS13_") {
            Some(rest) => format!("TLS_{rest}"),
            None => {
                // TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 -> ECDHE-RSA-AES128-GCM-SHA256
                let s = suite.trim_start_matches("TLS_");
                let (kx, enc) = s.split_once("_WITH_").unwrap_or(("", s));
                let enc = enc.replace("AES_128", "AES128").replace("AES_256", "AES256").replace("CHACHA20_POLY1305_SHA256", "CHACHA20-POLY1305").replace('_', "-");
                if kx.is_empty() { enc } else { format!("{}-{enc}", kx.replace('_', "-")) }
            }
        };
        Some((proto.to_string(), name, bits))
    }

    /// Decrypted bytes ready without touching the socket.
    pub fn buffered(&mut self) -> usize {
        self.conn.process_new_packets().map(|s| s.plaintext_bytes_to_read()).unwrap_or(0)
    }

    /// Read some plaintext. `Ok(0)` at end of stream (clean or not).
    pub fn read(&mut self, buf: &mut [u8], deadline: Option<Instant>) -> Result<usize> {
        self.io.deadline = deadline;
        loop {
            match self.conn.reader().read(buf) {
                Ok(n) => return Ok(n),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(0),
                Err(e) => return Err(e.into()),
            }
            if self.conn.wants_write() {
                self.conn.write_tls(&mut self.io).map_err(Error::from)?;
            }
            if self.conn.read_tls(&mut self.io).map_err(Error::from)? == 0 {
                // Peer closed without close_notify: drain, then EOF.
                self.conn.process_new_packets().map_err(tls_err)?;
                return match self.conn.reader().read(buf) {
                    Ok(n) => Ok(n),
                    Err(_) => Ok(0),
                };
            }
            self.conn.process_new_packets().map_err(tls_err)?;
        }
    }

    pub fn write_all(&mut self, buf: &[u8], deadline: Option<Instant>) -> Result<()> {
        self.io.deadline = deadline;
        self.conn.writer().write_all(buf).map_err(Error::from)?;
        while self.conn.wants_write() {
            self.conn.write_tls(&mut self.io).map_err(Error::from)?;
        }
        Ok(())
    }

    /// Send close_notify.
    pub fn close(&mut self) {
        self.conn.send_close_notify();
        self.io.deadline = Some(Instant::now() + std::time::Duration::from_secs(1));
        while self.conn.wants_write() {
            if self.conn.write_tls(&mut self.io).is_err() {
                break;
            }
        }
    }

    pub fn socket(&self) -> &Stream {
        &self.io.sock
    }
}

impl AsFd for TlsStream {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.io.sock.as_fd()
    }
}
