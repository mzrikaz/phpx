//! MySQL / MariaDB client protocol (what mysqlnd speaks): handshake and
//! authentication (mysql_native_password, caching_sha2_password with RSA
//! or TLS full auth), text queries, server-side prepared statements with
//! the binary row format, multiple result sets.

use std::time::{Duration, Instant};

use phpx_runtime::net::Stream;
use phpx_runtime::tls::{self, TlsStream};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use super::{Cell, Column, DbError, ResultSet};

const CLIENT_LONG_PASSWORD: u32 = 1;
const CLIENT_FOUND_ROWS: u32 = 2;
const CLIENT_LONG_FLAG: u32 = 4;
const CLIENT_CONNECT_WITH_DB: u32 = 8;
const CLIENT_LOCAL_FILES: u32 = 0x80;
const CLIENT_PROTOCOL_41: u32 = 0x200;
const CLIENT_SSL: u32 = 0x800;
const CLIENT_TRANSACTIONS: u32 = 0x2000;
const CLIENT_SECURE_CONNECTION: u32 = 0x8000;
const CLIENT_MULTI_STATEMENTS: u32 = 0x1_0000;
const CLIENT_MULTI_RESULTS: u32 = 0x2_0000;
const CLIENT_PS_MULTI_RESULTS: u32 = 0x4_0000;
const CLIENT_PLUGIN_AUTH: u32 = 0x8_0000;
const CLIENT_PLUGIN_AUTH_LENENC: u32 = 0x20_0000;

pub const SERVER_STATUS_IN_TRANS: u16 = 1;
const SERVER_MORE_RESULTS_EXISTS: u16 = 8;

pub const TYPE_DECIMAL: u8 = 0;
pub const TYPE_TINY: u8 = 1;
pub const TYPE_SHORT: u8 = 2;
pub const TYPE_LONG: u8 = 3;
pub const TYPE_FLOAT: u8 = 4;
pub const TYPE_DOUBLE: u8 = 5;
pub const TYPE_NULL: u8 = 6;
pub const TYPE_TIMESTAMP: u8 = 7;
pub const TYPE_LONGLONG: u8 = 8;
pub const TYPE_INT24: u8 = 9;
pub const TYPE_DATE: u8 = 10;
pub const TYPE_TIME: u8 = 11;
pub const TYPE_DATETIME: u8 = 12;
pub const TYPE_YEAR: u8 = 13;
pub const TYPE_NEWDECIMAL: u8 = 246;
pub const TYPE_VAR_STRING: u8 = 253;
pub const TYPE_STRING: u8 = 254;

pub const FLAG_UNSIGNED: u16 = 0x20;
pub const FLAG_BINARY: u16 = 0x80;

/// Connection options (PDO DSN / mysqli_real_connect arguments).
#[derive(Clone, Debug, Default)]
pub struct Opts {
    pub host: String,
    pub port: u16,
    pub socket: Option<String>,
    pub user: String,
    pub pass: String,
    pub db: Option<String>,
    pub charset: Option<String>,
    pub timeout: Option<Duration>,
    pub found_rows: bool,
    pub multi_statements: bool,
    pub local_infile: bool,
    /// TLS: Some(options) to require it.
    pub ssl: Option<tls::Options>,
    pub init_command: Option<String>,
}

enum Io {
    Plain(Stream),
    Tls(Box<TlsStream>),
}

pub struct Conn {
    io: Option<Io>,
    rbuf: Vec<u8>,
    rpos: usize,
    seq: u8,
    caps: u32,
    pub server_version: String,
    pub thread_id: u32,
    pub status: u16,
    pub charset: u8,
    pub affected: u64,
    pub insert_id: u64,
    pub warnings: u16,
    pub info: String,
    host_info: String,
    /// More result sets are pending from the last command.
    more: bool,
    pending_stmt: Option<(u32, Vec<Column>)>,
}

/// A prepared statement on the server.
pub struct Stmt {
    pub id: u32,
    pub params: usize,
    pub columns: Vec<Column>,
}

fn io_error(e: phpx_runtime::Error) -> DbError {
    DbError::new("HY000", 2006, format!("MySQL server has gone away ({e})"))
}

/// Length-encoded integer reader over a packet.
struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, p: 0 }
    }
    fn u8(&mut self) -> u8 {
        let v = self.b.get(self.p).copied().unwrap_or(0);
        self.p += 1;
        v
    }
    fn u16(&mut self) -> u16 {
        let v = u16::from_le_bytes([self.u8(), self.u8()]);
        v
    }
    fn u24(&mut self) -> u32 {
        u32::from_le_bytes([self.u8(), self.u8(), self.u8(), 0])
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes([self.u8(), self.u8(), self.u8(), self.u8()])
    }
    fn u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        for x in &mut b {
            *x = self.u8();
        }
        u64::from_le_bytes(b)
    }
    fn lenenc(&mut self) -> Option<u64> {
        match self.u8() {
            0xFB => None,
            0xFC => Some(self.u16() as u64),
            0xFD => Some(self.u24() as u64),
            0xFE => Some(self.u64()),
            n => Some(n as u64),
        }
    }
    fn bytes(&mut self, n: usize) -> &'a [u8] {
        let end = (self.p + n).min(self.b.len());
        let s = &self.b[self.p.min(end)..end];
        self.p = end;
        s
    }
    fn lenenc_bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.lenenc()? as usize;
        Some(self.bytes(n))
    }
    fn nul_str(&mut self) -> &'a [u8] {
        let start = self.p.min(self.b.len());
        let end = self.b[start..].iter().position(|&c| c == 0).map_or(self.b.len(), |i| start + i);
        self.p = end + 1;
        &self.b[start..end]
    }
    fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.p.min(self.b.len())..];
        self.p = self.b.len();
        s
    }
}

fn put_lenenc(out: &mut Vec<u8>, n: u64) {
    if n < 251 {
        out.push(n as u8);
    } else if n < 1 << 16 {
        out.push(0xFC);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else if n < 1 << 24 {
        out.push(0xFD);
        out.extend_from_slice(&(n as u32).to_le_bytes()[..3]);
    } else {
        out.push(0xFE);
        out.extend_from_slice(&n.to_le_bytes());
    }
}

fn xor(a: &[u8], b: &[u8]) -> Vec<u8> {
    a.iter().zip(b.iter().cycle()).map(|(x, y)| x ^ y).collect()
}

fn native_password(pass: &[u8], scramble: &[u8]) -> Vec<u8> {
    if pass.is_empty() {
        return vec![];
    }
    let h1 = Sha1::digest(pass);
    let h2 = Sha1::digest(h1);
    let mut h = Sha1::new();
    h.update(scramble);
    h.update(h2);
    xor(&h1, &h.finalize())
}

fn sha2_password(pass: &[u8], scramble: &[u8]) -> Vec<u8> {
    if pass.is_empty() {
        return vec![];
    }
    let h1 = Sha256::digest(pass);
    let h2 = Sha256::digest(h1);
    let mut h = Sha256::new();
    h.update(h2);
    h.update(scramble);
    xor(&h1, &h.finalize())
}

/// MySQL charset ids for SET NAMES-less connects.
fn charset_id(name: &str) -> u8 {
    match name.to_ascii_lowercase().as_str() {
        "utf8mb4" => 45,
        "utf8" | "utf8mb3" => 33,
        "latin1" => 8,
        "binary" => 63,
        "ascii" => 11,
        _ => 45,
    }
}

impl Conn {
    pub fn connect(o: &Opts) -> Result<Conn, DbError> {
        let deadline = o.timeout.map(|t| Instant::now() + t);
        let (stream, host_info) = match &o.socket {
            Some(path) => (Stream::connect_unix(path, deadline), "Localhost via UNIX socket".to_string()),
            None => (Stream::connect_tcp((o.host.as_str(), o.port), deadline), format!("{} via TCP/IP", o.host)),
        };
        let stream = stream.map_err(|e| {
            let msg = e.to_string();
            let msg = msg.trim_start_matches("io: ");
            let (code, text) = if msg.contains("refused") {
                (2002, "Connection refused".to_string())
            } else if msg.contains("timed out") {
                (2002, "Connection timed out".to_string())
            } else if msg.contains("No such file") {
                (2002, "No such file or directory".to_string())
            } else if msg.contains("lookup") || msg.contains("Name or service") || msg.contains("resolve") {
                (2002, "php_network_getaddresses: getaddrinfo for ".to_string() + &o.host + " failed: Name or service not known")
            } else {
                (2002, msg.to_string())
            };
            DbError::new("HY000", code, text)
        })?;
        let mut c = Conn {
            io: Some(Io::Plain(stream)),
            rbuf: Vec::with_capacity(16384),
            rpos: 0,
            seq: 0,
            caps: 0,
            server_version: String::new(),
            thread_id: 0,
            status: 0,
            charset: 45,
            affected: 0,
            insert_id: 0,
            warnings: 0,
            info: String::new(),
            host_info,
            more: false,
            pending_stmt: None,
        };
        c.handshake(o)?;
        if let Some(cmd) = &o.init_command {
            c.query(cmd)?;
        }
        Ok(c)
    }

    pub fn host_info(&self) -> &str {
        &self.host_info
    }

    pub fn in_transaction(&self) -> bool {
        self.status & SERVER_STATUS_IN_TRANS != 0
    }

    // ---- packets -------------------------------------------------------------

    fn read_raw(&mut self, want: usize) -> Result<(), DbError> {
        while self.rbuf.len() - self.rpos < want {
            if self.rpos > 0 && self.rpos == self.rbuf.len() {
                self.rbuf.clear();
                self.rpos = 0;
            }
            let mut chunk = [0u8; 16384];
            let n = match self.io.as_mut() {
                Some(Io::Plain(s)) => s.read(&mut chunk, None),
                Some(Io::Tls(t)) => t.read(&mut chunk, None),
                None => return Err(DbError::new("HY000", 2006, "MySQL server has gone away")),
            }
            .map_err(io_error)?;
            if n == 0 {
                self.io = None;
                return Err(DbError::new("HY000", 2006, "MySQL server has gone away"));
            }
            self.rbuf.extend_from_slice(&chunk[..n]);
        }
        Ok(())
    }

    /// One logical packet (joins 16 MiB fragments).
    fn read_packet(&mut self) -> Result<Vec<u8>, DbError> {
        let mut out = Vec::new();
        loop {
            self.read_raw(4)?;
            let h = &self.rbuf[self.rpos..self.rpos + 4];
            let len = u32::from_le_bytes([h[0], h[1], h[2], 0]) as usize;
            self.seq = h[3].wrapping_add(1);
            self.rpos += 4;
            self.read_raw(len)?;
            out.extend_from_slice(&self.rbuf[self.rpos..self.rpos + len]);
            self.rpos += len;
            if len < 0xFF_FFFF {
                return Ok(out);
            }
        }
    }

    fn write_packet(&mut self, payload: &[u8]) -> Result<(), DbError> {
        let mut buf = Vec::with_capacity(payload.len() + 8);
        let mut chunks = payload.chunks(0xFF_FFFF).peekable();
        if payload.is_empty() {
            buf.extend_from_slice(&[0, 0, 0, self.seq]);
            self.seq = self.seq.wrapping_add(1);
        }
        while let Some(ch) = chunks.next() {
            buf.extend_from_slice(&(ch.len() as u32).to_le_bytes()[..3]);
            buf.push(self.seq);
            self.seq = self.seq.wrapping_add(1);
            buf.extend_from_slice(ch);
            if ch.len() == 0xFF_FFFF && chunks.peek().is_none() {
                buf.extend_from_slice(&[0, 0, 0, self.seq]);
                self.seq = self.seq.wrapping_add(1);
            }
        }
        match self.io.as_mut() {
            Some(Io::Plain(s)) => s.write_all(&buf, None),
            Some(Io::Tls(t)) => t.write_all(&buf, None),
            None => return Err(DbError::new("HY000", 2006, "MySQL server has gone away")),
        }
        .map_err(io_error)
    }

    fn command(&mut self, cmd: u8, body: &[u8]) -> Result<(), DbError> {
        if self.more {
            self.drain()?;
        }
        self.seq = 0;
        let mut p = Vec::with_capacity(body.len() + 1);
        p.push(cmd);
        p.extend_from_slice(body);
        self.write_packet(&p)
    }

    fn err_packet(p: &[u8]) -> DbError {
        let mut r = Reader::new(&p[1..]);
        let code = r.u16() as i64;
        let (state, msg) = if r.b.get(r.p) == Some(&b'#') {
            r.u8();
            (String::from_utf8_lossy(r.bytes(5)).into_owned(), String::from_utf8_lossy(r.rest()).into_owned())
        } else {
            ("HY000".to_string(), String::from_utf8_lossy(r.rest()).into_owned())
        };
        DbError::new(&state, code, msg)
    }

    fn ok_packet(&mut self, p: &[u8]) {
        let mut r = Reader::new(&p[1..]);
        self.affected = r.lenenc().unwrap_or(0);
        self.insert_id = r.lenenc().unwrap_or(0);
        self.status = r.u16();
        self.warnings = r.u16();
        self.info = String::from_utf8_lossy(r.rest()).into_owned();
        self.more = self.status & SERVER_MORE_RESULTS_EXISTS != 0;
    }

    fn eof_packet(&mut self, p: &[u8]) {
        let mut r = Reader::new(&p[1..]);
        self.warnings = r.u16();
        self.status = r.u16();
        self.more = self.status & SERVER_MORE_RESULTS_EXISTS != 0;
    }

    fn is_eof(p: &[u8]) -> bool {
        p.first() == Some(&0xFE) && p.len() < 9
    }

    // ---- connection setup -------------------------------------------------------

    fn handshake(&mut self, o: &Opts) -> Result<(), DbError> {
        let p = self.read_packet()?;
        if p.first() == Some(&0xFF) {
            return Err(Self::err_packet(&p));
        }
        let mut r = Reader::new(&p);
        let _proto = r.u8();
        self.server_version = String::from_utf8_lossy(r.nul_str()).into_owned();
        self.thread_id = r.u32();
        let mut scramble = r.bytes(8).to_vec();
        r.u8();
        let mut caps = r.u16() as u32;
        let _charset = r.u8();
        self.status = r.u16();
        caps |= (r.u16() as u32) << 16;
        let auth_len = r.u8() as usize;
        r.bytes(10);
        if caps & CLIENT_SECURE_CONNECTION != 0 {
            let n = auth_len.saturating_sub(8).max(13);
            let part2 = r.bytes(n);
            scramble.extend_from_slice(&part2[..part2.len().saturating_sub(1)]);
        }
        let plugin = if caps & CLIENT_PLUGIN_AUTH != 0 { String::from_utf8_lossy(r.nul_str()).into_owned() } else { "mysql_native_password".into() };

        let mut want = CLIENT_LONG_PASSWORD
            | CLIENT_LONG_FLAG
            | CLIENT_PROTOCOL_41
            | CLIENT_TRANSACTIONS
            | CLIENT_SECURE_CONNECTION
            | CLIENT_MULTI_RESULTS
            | CLIENT_PS_MULTI_RESULTS
            | CLIENT_PLUGIN_AUTH
            | CLIENT_PLUGIN_AUTH_LENENC;
        if o.db.as_deref().is_some_and(|d| !d.is_empty()) {
            want |= CLIENT_CONNECT_WITH_DB;
        }
        if o.found_rows {
            want |= CLIENT_FOUND_ROWS;
        }
        if o.multi_statements {
            want |= CLIENT_MULTI_STATEMENTS;
        }
        if o.local_infile {
            want |= CLIENT_LOCAL_FILES;
        }
        if o.ssl.is_some() {
            want |= CLIENT_SSL;
        }
        self.caps = want & caps;
        self.charset = charset_id(o.charset.as_deref().unwrap_or("utf8mb4"));

        if let Some(ssl) = &o.ssl {
            if caps & CLIENT_SSL == 0 {
                return Err(DbError::new("HY000", 2026, "SSL connection error: SSL is required but the server doesn't support it"));
            }
            let mut req = Vec::with_capacity(32);
            req.extend_from_slice(&self.caps.to_le_bytes());
            req.extend_from_slice(&(16u32 << 20).to_le_bytes());
            req.push(self.charset);
            req.extend_from_slice(&[0u8; 23]);
            self.write_packet(&req)?;
            let Some(Io::Plain(s)) = self.io.take() else { unreachable!() };
            let host = if o.host.is_empty() { "localhost".to_string() } else { o.host.clone() };
            let t = TlsStream::connect(s, &host, ssl, o.timeout.map(|t| Instant::now() + t)).map_err(|e| DbError::new("HY000", 2026, format!("SSL connection error: {e}")))?;
            self.io = Some(Io::Tls(Box::new(t)));
        }

        let auth = self.scramble_for(&plugin, o.pass.as_bytes(), &scramble);
        let mut resp = Vec::with_capacity(128);
        resp.extend_from_slice(&self.caps.to_le_bytes());
        resp.extend_from_slice(&(16u32 << 20).to_le_bytes());
        resp.push(self.charset);
        resp.extend_from_slice(&[0u8; 23]);
        resp.extend_from_slice(o.user.as_bytes());
        resp.push(0);
        put_lenenc(&mut resp, auth.len() as u64);
        resp.extend_from_slice(&auth);
        if self.caps & CLIENT_CONNECT_WITH_DB != 0 {
            resp.extend_from_slice(o.db.as_deref().unwrap_or("").as_bytes());
            resp.push(0);
        }
        resp.extend_from_slice(plugin.as_bytes());
        resp.push(0);
        self.write_packet(&resp)?;
        self.auth_result(o, plugin, scramble)
    }

    fn scramble_for(&self, plugin: &str, pass: &[u8], scramble: &[u8]) -> Vec<u8> {
        match plugin {
            "caching_sha2_password" => sha2_password(pass, scramble),
            "mysql_clear_password" => {
                let mut v = pass.to_vec();
                v.push(0);
                v
            }
            _ => native_password(pass, scramble),
        }
    }

    fn secure(&self) -> bool {
        matches!(self.io, Some(Io::Tls(_))) || self.host_info.contains("UNIX socket")
    }

    fn auth_result(&mut self, o: &Opts, mut plugin: String, mut scramble: Vec<u8>) -> Result<(), DbError> {
        loop {
            let p = self.read_packet()?;
            match p.first() {
                Some(0x00) => {
                    self.ok_packet(&p);
                    return Ok(());
                }
                Some(0xFF) => return Err(Self::err_packet(&p)),
                Some(0xFE) => {
                    // Auth switch request.
                    let mut r = Reader::new(&p[1..]);
                    plugin = String::from_utf8_lossy(r.nul_str()).into_owned();
                    let data = r.rest();
                    scramble = data[..data.len().saturating_sub(1)].to_vec();
                    let auth = self.scramble_for(&plugin, o.pass.as_bytes(), &scramble);
                    self.write_packet(&auth)?;
                }
                Some(0x01) if plugin == "caching_sha2_password" => match p.get(1) {
                    Some(3) => {} // fast auth ok; OK packet follows
                    Some(4) => {
                        // Full authentication.
                        if self.secure() {
                            let mut pw = o.pass.as_bytes().to_vec();
                            pw.push(0);
                            self.write_packet(&pw)?;
                        } else {
                            self.write_packet(&[2])?;
                            let key = self.read_packet()?;
                            if key.first() == Some(&0xFF) {
                                return Err(Self::err_packet(&key));
                            }
                            let pem = String::from_utf8_lossy(&key[1..]).into_owned();
                            let mut pw = o.pass.as_bytes().to_vec();
                            pw.push(0);
                            let data = xor(&pw, &scramble);
                            use rsa::pkcs8::DecodePublicKey;
                            let pk = rsa::RsaPublicKey::from_public_key_pem(&pem)
                                .or_else(|_| {
                                    use rsa::pkcs1::DecodeRsaPublicKey;
                                    rsa::RsaPublicKey::from_pkcs1_pem(&pem)
                                })
                                .map_err(|e| DbError::new("HY000", 2061, format!("Authentication requires secure connection: bad server key ({e})")))?;
                            let enc = pk
                                .encrypt(&mut rand_core::OsRng, rsa::Oaep::new::<Sha1>(), &data)
                                .map_err(|e| DbError::new("HY000", 2061, format!("RSA encryption failed: {e}")))?;
                            self.write_packet(&enc)?;
                        }
                    }
                    _ => return Err(DbError::new("HY000", 2027, "Malformed packet")),
                },
                _ => return Err(DbError::new("HY000", 2027, "Malformed packet")),
            }
        }
    }

    // ---- results --------------------------------------------------------------------

    fn column_def(p: &[u8]) -> Column {
        let mut r = Reader::new(p);
        let _catalog = r.lenenc_bytes();
        let schema = r.lenenc_bytes().unwrap_or_default();
        let table = r.lenenc_bytes().unwrap_or_default();
        let org_table = r.lenenc_bytes().unwrap_or_default();
        let name = r.lenenc_bytes().unwrap_or_default();
        let org_name = r.lenenc_bytes().unwrap_or_default();
        let _ = r.lenenc();
        let charset = r.u16();
        let length = r.u32();
        let kind = r.u8();
        let flags = r.u16();
        let decimals = r.u8();
        let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        Column { name: s(name), table: s(table), org_table: s(org_table), schema: s(schema), org_name: s(org_name), kind, flags, length, decimals, charset }
    }

    /// After a command: OK, ERR or a result set. `binary`: prepared statement rows.
    fn read_result(&mut self, binary: bool) -> Result<ResultSet, DbError> {
        let p = self.read_packet()?;
        match p.first() {
            Some(0x00) => {
                self.ok_packet(&p);
                Ok(ResultSet { affected: self.affected, insert_id: self.insert_id, warnings: self.warnings, info: self.info.clone(), ..Default::default() })
            }
            Some(0xFF) => {
                self.more = false;
                Err(Self::err_packet(&p))
            }
            Some(0xFB) => {
                self.more = false;
                Err(DbError::new("HY000", 2068, "LOAD DATA LOCAL INFILE is forbidden, check related settings like mysqli.allow_local_infile|mysqli.local_infile_directory or PDO::MYSQL_ATTR_LOCAL_INFILE|PDO::MYSQL_ATTR_LOCAL_INFILE_DIRECTORY"))
            }
            _ => {
                let n = Reader::new(&p).lenenc().unwrap_or(0) as usize;
                let mut columns = Vec::with_capacity(n);
                for _ in 0..n {
                    let c = self.read_packet()?;
                    columns.push(Self::column_def(&c));
                }
                let e = self.read_packet()?;
                if Self::is_eof(&e) {
                    self.eof_packet(&e);
                }
                let mut rows = Vec::new();
                loop {
                    let p = self.read_packet()?;
                    if Self::is_eof(&p) {
                        self.eof_packet(&p);
                        break;
                    }
                    if p.first() == Some(&0xFF) {
                        self.more = false;
                        return Err(Self::err_packet(&p));
                    }
                    rows.push(if binary { binary_row(&p, &columns) } else { text_row(&p, n) });
                }
                self.affected = rows.len() as u64;
                Ok(ResultSet { columns, rows, affected: u64::MAX, insert_id: 0, warnings: self.warnings, info: String::new() })
            }
        }
    }

    /// Discard result sets still pending from the last command.
    fn drain(&mut self) -> Result<(), DbError> {
        while self.more {
            self.more = false;
            let _ = self.read_result(false)?;
        }
        Ok(())
    }

    pub fn more_results(&self) -> bool {
        self.more
    }

    /// The next pending result set (multi-statement / CALL results).
    pub fn next_result(&mut self, binary: bool) -> Result<Option<ResultSet>, DbError> {
        if !self.more {
            return Ok(None);
        }
        self.more = false;
        self.read_result(binary).map(Some)
    }

    // ---- commands -------------------------------------------------------------------

    pub fn query(&mut self, sql: &str) -> Result<ResultSet, DbError> {
        self.query_bytes(sql.as_bytes())
    }

    pub fn query_bytes(&mut self, sql: &[u8]) -> Result<ResultSet, DbError> {
        self.command(0x03, sql)?;
        self.read_result(false)
    }

    pub fn ping(&mut self) -> Result<(), DbError> {
        self.command(0x0E, &[])?;
        self.read_result(false).map(|_| ())
    }

    pub fn select_db(&mut self, db: &str) -> Result<(), DbError> {
        self.command(0x02, db.as_bytes())?;
        self.read_result(false).map(|_| ())
    }

    pub fn prepare(&mut self, sql: &[u8]) -> Result<Stmt, DbError> {
        self.command(0x16, sql)?;
        let p = self.read_packet()?;
        if p.first() == Some(&0xFF) {
            return Err(Self::err_packet(&p));
        }
        let mut r = Reader::new(&p[1..]);
        let id = r.u32();
        let ncols = r.u16() as usize;
        let nparams = r.u16() as usize;
        if nparams > 0 {
            for _ in 0..nparams {
                self.read_packet()?;
            }
            let e = self.read_packet()?;
            if !Self::is_eof(&e) {
                return Err(DbError::new("HY000", 2027, "Malformed packet"));
            }
        }
        let mut columns = Vec::with_capacity(ncols);
        if ncols > 0 {
            for _ in 0..ncols {
                let c = self.read_packet()?;
                columns.push(Self::column_def(&c));
            }
            let e = self.read_packet()?;
            if !Self::is_eof(&e) {
                return Err(DbError::new("HY000", 2027, "Malformed packet"));
            }
        }
        Ok(Stmt { id, params: nparams, columns })
    }

    pub fn execute(&mut self, st: &Stmt, params: &[Param]) -> Result<ResultSet, DbError> {
        let mut body = Vec::with_capacity(64);
        body.extend_from_slice(&st.id.to_le_bytes());
        body.push(0);
        body.extend_from_slice(&1u32.to_le_bytes());
        if st.params > 0 {
            let mut nulls = vec![0u8; st.params.div_ceil(8)];
            for (i, p) in params.iter().enumerate().take(st.params) {
                if matches!(p, Param::Null) {
                    nulls[i / 8] |= 1 << (i % 8);
                }
            }
            body.extend_from_slice(&nulls);
            body.push(1);
            for i in 0..st.params {
                let (t, unsigned) = match params.get(i).unwrap_or(&Param::Null) {
                    Param::Null => (TYPE_NULL, 0),
                    Param::Int(_) => (TYPE_LONGLONG, 0),
                    Param::Float(_) => (TYPE_DOUBLE, 0),
                    Param::Bytes(_) => (TYPE_VAR_STRING, 0),
                };
                body.push(t);
                body.push(unsigned);
            }
            for i in 0..st.params {
                match params.get(i).unwrap_or(&Param::Null) {
                    Param::Null => {}
                    Param::Int(n) => body.extend_from_slice(&n.to_le_bytes()),
                    Param::Float(f) => body.extend_from_slice(&f.to_le_bytes()),
                    Param::Bytes(b) => {
                        put_lenenc(&mut body, b.len() as u64);
                        body.extend_from_slice(b);
                    }
                }
            }
        }
        self.command(0x17, &body)?;
        self.read_result(true)
    }

    pub fn close_stmt(&mut self, id: u32) {
        if self.io.is_some() {
            // COM_STMT_CLOSE has no response.
            let _ = self.command(0x19, &id.to_le_bytes());
        }
    }

    pub fn reset_stmt(&mut self, id: u32) -> Result<(), DbError> {
        self.command(0x1A, &id.to_le_bytes())?;
        self.read_result(false).map(|_| ())
    }

    pub fn quit(&mut self) {
        if self.io.is_some() {
            let _ = self.command(0x01, &[]);
            if let Some(Io::Tls(t)) = &mut self.io {
                t.close();
            }
        }
        self.io = None;
        let _ = &self.pending_stmt;
    }

    pub fn connected(&self) -> bool {
        self.io.is_some()
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.quit();
    }
}

/// A bound parameter for COM_STMT_EXECUTE.
#[derive(Clone, Debug)]
pub enum Param {
    Null,
    Int(i64),
    Float(f64),
    Bytes(Vec<u8>),
}

fn text_row(p: &[u8], n: usize) -> Vec<Cell> {
    let mut r = Reader::new(p);
    (0..n).map(|_| match r.lenenc_bytes() {
        None => Cell::Null,
        Some(b) => Cell::Bytes(b.to_vec()),
    })
    .collect()
}

fn binary_row(p: &[u8], cols: &[Column]) -> Vec<Cell> {
    let n = cols.len();
    let nulls = &p[1..1 + (n + 9) / 8];
    let mut r = Reader::new(p);
    r.p = 1 + (n + 9) / 8;
    let mut out = Vec::with_capacity(n);
    for (i, c) in cols.iter().enumerate() {
        let bit = i + 2;
        if nulls[bit / 8] & (1 << (bit % 8)) != 0 {
            out.push(Cell::Null);
            continue;
        }
        let unsigned = c.flags & FLAG_UNSIGNED != 0;
        let cell = match c.kind {
            TYPE_TINY => {
                let v = r.u8();
                Cell::Int(if unsigned { v as i64 } else { v as i8 as i64 })
            }
            TYPE_SHORT | TYPE_YEAR => {
                let v = r.u16();
                Cell::Int(if unsigned || c.kind == TYPE_YEAR { v as i64 } else { v as i16 as i64 })
            }
            TYPE_LONG | TYPE_INT24 => {
                let v = r.u32();
                Cell::Int(if unsigned { v as i64 } else { v as i32 as i64 })
            }
            TYPE_LONGLONG => {
                let v = r.u64();
                if unsigned && v > i64::MAX as u64 { Cell::UInt(v) } else { Cell::Int(v as i64) }
            }
            TYPE_FLOAT => Cell::Float32(f32::from_le_bytes(r.u32().to_le_bytes())),
            TYPE_DOUBLE => Cell::Float(f64::from_le_bytes(r.u64().to_le_bytes())),
            TYPE_DATE | TYPE_DATETIME | TYPE_TIMESTAMP => {
                let len = r.u8() as usize;
                let (mut y, mut mo, mut d, mut h, mut mi, mut s, mut us) = (0u16, 0u8, 0u8, 0u8, 0u8, 0u8, 0u32);
                if len >= 4 {
                    y = r.u16();
                    mo = r.u8();
                    d = r.u8();
                }
                if len >= 7 {
                    h = r.u8();
                    mi = r.u8();
                    s = r.u8();
                }
                if len >= 11 {
                    us = r.u32();
                }
                let mut t = if c.kind == TYPE_DATE { format!("{y:04}-{mo:02}-{d:02}") } else { format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}") };
                if c.kind != TYPE_DATE && c.decimals > 0 && c.decimals <= 6 {
                    let frac = format!("{us:06}");
                    t.push('.');
                    t.push_str(&frac[..c.decimals as usize]);
                }
                Cell::Bytes(t.into_bytes())
            }
            TYPE_TIME => {
                let len = r.u8() as usize;
                let (mut neg, mut days, mut h, mut mi, mut s, mut us) = (0u8, 0u32, 0u8, 0u8, 0u8, 0u32);
                if len >= 8 {
                    neg = r.u8();
                    days = r.u32();
                    h = r.u8();
                    mi = r.u8();
                    s = r.u8();
                }
                if len >= 12 {
                    us = r.u32();
                }
                let hours = days * 24 + h as u32;
                let mut t = format!("{}{:02}:{mi:02}:{s:02}", if neg == 1 { "-" } else { "" }, hours);
                if c.decimals > 0 && c.decimals <= 6 {
                    let frac = format!("{us:06}");
                    t.push('.');
                    t.push_str(&frac[..c.decimals as usize]);
                }
                Cell::Bytes(t.into_bytes())
            }
            _ => Cell::Bytes(r.lenenc_bytes().unwrap_or_default().to_vec()),
        };
        out.push(cell);
    }
    out
}
