//! Network and system lookups: services and protocols, reverse DNS, DNS
//! records (a small resolver over UDP using /etc/resolv.conf), network
//! interfaces, syslog, ftok.

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::net::{IpAddr, UdpSocket};
use std::time::Duration;

use crate::builtins::{arg, has, int, s};
use crate::interp::{BuiltinFn, Interp, R};
use crate::value::{Key, PhpArray, Value};

fn text(it: &Interp, a: &[Value], i: usize) -> R<String> {
    Ok(crate::pstr::lossy(&s(it, a, i)?).into_owned())
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}

fn key(k: &str) -> Key {
    Key::from_bytes(k.as_bytes())
}

// ---- DNS ---------------------------------------------------------------------------

pub const DNS_A: i64 = 1;
pub const DNS_NS: i64 = 2;
pub const DNS_CNAME: i64 = 16;
pub const DNS_SOA: i64 = 32;
pub const DNS_PTR: i64 = 2048;
pub const DNS_HINFO: i64 = 4096;
pub const DNS_CAA: i64 = 8192;
pub const DNS_MX: i64 = 16384;
pub const DNS_TXT: i64 = 32768;
pub const DNS_SRV: i64 = 33554432;
pub const DNS_NAPTR: i64 = 67108864;
pub const DNS_AAAA: i64 = 134217728;
pub const DNS_A6: i64 = 16777216;
pub const DNS_ANY: i64 = 268435456;

/// (PHP flag, wire type, name)
const TYPES: &[(i64, u16, &str)] = &[
    (DNS_A, 1, "A"),
    (DNS_NS, 2, "NS"),
    (DNS_CNAME, 5, "CNAME"),
    (DNS_SOA, 6, "SOA"),
    (DNS_PTR, 12, "PTR"),
    (DNS_HINFO, 13, "HINFO"),
    (DNS_MX, 15, "MX"),
    (DNS_TXT, 16, "TXT"),
    (DNS_AAAA, 28, "AAAA"),
    (DNS_SRV, 33, "SRV"),
    (DNS_NAPTR, 35, "NAPTR"),
    (DNS_CAA, 257, "CAA"),
];

fn nameservers() -> Vec<String> {
    let conf = std::fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let ns: Vec<String> = conf.lines().filter_map(|l| l.trim().strip_prefix("nameserver")).map(|s| s.trim().to_string()).collect();
    if ns.is_empty() { vec!["127.0.0.1".into()] } else { ns }
}

fn read_name(msg: &[u8], mut i: usize) -> Option<(String, usize)> {
    let mut labels = vec![];
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *msg.get(i)? as usize;
        if len == 0 {
            i += 1;
            break;
        }
        if len & 0xC0 == 0xC0 {
            let ptr = ((len & 0x3F) << 8) | *msg.get(i + 1)? as usize;
            if end.is_none() {
                end = Some(i + 2);
            }
            jumps += 1;
            if jumps > 32 {
                return None;
            }
            i = ptr;
            continue;
        }
        labels.push(crate::pstr::lossy(msg.get(i + 1..i + 1 + len)?).into_owned());
        i += 1 + len;
    }
    Some((labels.join("."), end.unwrap_or(i)))
}

struct Rr {
    name: String,
    ty: u16,
    ttl: u32,
    data: Vec<u8>,
    data_at: usize,
}

fn query(host: &str, ty: u16) -> Option<(Vec<u8>, Vec<Rr>)> {
    let id: u16 = (std::process::id() as u16) ^ (ty.wrapping_mul(7919));
    let mut q = vec![];
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&ty.to_be_bytes());
    q.extend_from_slice(&1u16.to_be_bytes());
    for ns in nameservers() {
        let Ok(sock) = UdpSocket::bind(if ns.contains(':') { "[::]:0" } else { "0.0.0.0:0" }) else { continue };
        let _ = sock.set_read_timeout(Some(Duration::from_secs(3)));
        let addr = if ns.contains(':') { format!("[{ns}]:53") } else { format!("{ns}:53") };
        if sock.send_to(&q, &addr).is_err() {
            continue;
        }
        let mut buf = vec![0u8; 4096];
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        buf.truncate(n);
        if buf.len() < 12 || buf[0..2] != id.to_be_bytes() {
            continue;
        }
        let (qd, an) = (u16::from_be_bytes([buf[4], buf[5]]), u16::from_be_bytes([buf[6], buf[7]]));
        let mut i = 12;
        for _ in 0..qd {
            let (_, e) = read_name(&buf, i)?;
            i = e + 4;
        }
        let mut rrs = vec![];
        for _ in 0..an {
            let (name, e) = read_name(&buf, i)?;
            i = e;
            let b = buf.get(i..i + 10)?;
            let rty = u16::from_be_bytes([b[0], b[1]]);
            let ttl = u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
            let len = u16::from_be_bytes([b[8], b[9]]) as usize;
            let data = buf.get(i + 10..i + 10 + len)?.to_vec();
            rrs.push(Rr { name, ty: rty, ttl, data, data_at: i + 10 });
            i += 10 + len;
        }
        return Some((buf, rrs));
    }
    None
}

fn rr_value(msg: &[u8], rr: &Rr) -> Option<PhpArray> {
    let mut a = PhpArray::new();
    a.insert(key("host"), Value::str(&rr.name));
    a.insert(key("class"), Value::str("IN"));
    a.insert(key("ttl"), Value::Int(rr.ttl as i64));
    let tname = TYPES.iter().find(|t| t.1 == rr.ty)?.2;
    a.insert(key("type"), Value::str(tname));
    let d = &rr.data;
    let name_at = |off: usize| read_name(msg, rr.data_at + off).map(|x| x.0).unwrap_or_default();
    match rr.ty {
        1 if d.len() == 4 => {
            a.insert(key("ip"), Value::str(&format!("{}.{}.{}.{}", d[0], d[1], d[2], d[3])));
        }
        28 if d.len() == 16 => {
            let ip = std::net::Ipv6Addr::from(<[u8; 16]>::try_from(&d[..]).ok()?);
            a.insert(key("ipv6"), Value::str(&ip.to_string()));
        }
        2 | 5 | 12 => {
            a.insert(key("target"), Value::str(&name_at(0)));
        }
        15 => {
            a.insert(key("pri"), Value::Int(u16::from_be_bytes([d[0], d[1]]) as i64));
            a.insert(key("target"), Value::str(&name_at(2)));
        }
        16 => {
            let mut parts = vec![];
            let mut i = 0;
            while i < d.len() {
                let l = d[i] as usize;
                parts.push(Value::bytes(d.get(i + 1..i + 1 + l)?.to_vec()));
                i += 1 + l;
            }
            let joined: Vec<u8> = parts.iter().flat_map(|p| p.scalar_bytes().unwrap_or_default().to_vec()).collect();
            a.insert(key("txt"), Value::bytes(joined));
            a.insert(key("entries"), Value::array(PhpArray::from_list(parts)));
        }
        33 => {
            a.insert(key("pri"), Value::Int(u16::from_be_bytes([d[0], d[1]]) as i64));
            a.insert(key("weight"), Value::Int(u16::from_be_bytes([d[2], d[3]]) as i64));
            a.insert(key("port"), Value::Int(u16::from_be_bytes([d[4], d[5]]) as i64));
            a.insert(key("target"), Value::str(&name_at(6)));
        }
        6 => {
            let (mname, e1) = read_name(msg, rr.data_at)?;
            let (rname, e2) = read_name(msg, e1)?;
            let n = |o: usize| u32::from_be_bytes(msg[e2 + o..e2 + o + 4].try_into().unwrap()) as i64;
            a.insert(key("mname"), Value::str(&mname));
            a.insert(key("rname"), Value::str(&rname));
            a.insert(key("serial"), Value::Int(n(0)));
            a.insert(key("refresh"), Value::Int(n(4)));
            a.insert(key("retry"), Value::Int(n(8)));
            a.insert(key("expire"), Value::Int(n(12)));
            a.insert(key("minimum-ttl"), Value::Int(n(16)));
        }
        _ => {}
    }
    Some(a)
}

fn type_code(name: &str) -> Option<u16> {
    let n = name.to_ascii_uppercase();
    if n == "ANY" {
        return Some(255);
    }
    TYPES.iter().find(|t| t.2 == n).map(|t| t.1)
}

fn dns_get_record(it: &Interp, a: &[Value]) -> R<Value> {
    let host = text(it, a, 0)?;
    let flags = if has(a, 1) { int(it, a, 1)? } else { DNS_ANY };
    let wanted: Vec<u16> = if flags == DNS_ANY || flags == DNS_ANY | DNS_A6 {
        vec![1, 2, 5, 6, 15, 16, 28, 33]
    } else {
        TYPES.iter().filter(|t| flags & t.0 != 0).map(|t| t.1).collect()
    };
    let mut out = PhpArray::new();
    for ty in wanted {
        if let Some((msg, rrs)) = query(&host, ty) {
            for rr in rrs.iter().filter(|r| r.ty == ty) {
                if let Some(v) = rr_value(&msg, rr) {
                    out.push(Value::array(v));
                }
            }
        }
    }
    Ok(Value::array(out))
}

fn mx_records(host: &str) -> Vec<(i64, String)> {
    let mut out = vec![];
    if let Some((msg, rrs)) = query(host, 15) {
        for rr in rrs.iter().filter(|r| r.ty == 15 && r.data.len() > 2) {
            let pri = u16::from_be_bytes([rr.data[0], rr.data[1]]) as i64;
            let target = read_name(&msg, rr.data_at + 2).map(|x| x.0).unwrap_or_default();
            out.push((pri, target));
        }
    }
    out
}

// ---- net_get_interfaces -------------------------------------------------------------

fn sockaddr_str(sa: *const libc::sockaddr) -> Option<(i32, String)> {
    if sa.is_null() {
        return None;
    }
    let fam = unsafe { (*sa).sa_family } as i32;
    match fam {
        libc::AF_INET => {
            let a = unsafe { &*(sa as *const libc::sockaddr_in) };
            Some((fam, std::net::Ipv4Addr::from(u32::from_be(a.sin_addr.s_addr)).to_string()))
        }
        libc::AF_INET6 => {
            let a = unsafe { &*(sa as *const libc::sockaddr_in6) };
            Some((fam, std::net::Ipv6Addr::from(a.sin6_addr.s6_addr).to_string()))
        }
        _ => None,
    }
}

fn interfaces() -> Value {
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut ifap) } != 0 {
        return Value::Bool(false);
    }
    let mut out = PhpArray::new();
    let mut p = ifap;
    while !p.is_null() {
        let ifa = unsafe { &*p };
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let k = key(&name);
        let mut entry = match out.get(&k) {
            Some(Value::Array(x)) => (*x).clone(),
            _ => {
                let mut e = PhpArray::new();
                e.insert(key("unicast"), Value::array(PhpArray::new()));
                e
            }
        };
        let mut uni = match entry.get(&key("unicast")) {
            Some(Value::Array(x)) => (*x).clone(),
            _ => PhpArray::new(),
        };
        let mut u = PhpArray::new();
        u.insert(key("flags"), Value::Int(ifa.ifa_flags as i64));
        let fam = if ifa.ifa_addr.is_null() { 0 } else { (unsafe { (*ifa.ifa_addr).sa_family }) as i64 };
        u.insert(key("family"), Value::Int(fam));
        if let Some((_, addr)) = sockaddr_str(ifa.ifa_addr) {
            u.insert(key("address"), Value::str(&addr));
        }
        if let Some((_, mask)) = sockaddr_str(ifa.ifa_netmask) {
            u.insert(key("netmask"), Value::str(&mask));
        }
        uni.push(Value::array(u));
        entry.insert(key("unicast"), Value::array(uni));
        entry.insert(key("up"), Value::Bool(ifa.ifa_flags & libc::IFF_UP as u32 != 0));
        out.insert(k, Value::array(entry));
        p = ifa.ifa_next;
    }
    unsafe { libc::freeifaddrs(ifap) };
    Value::array(out)
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("getservbyname", |it, _, a| {
        let (svc, proto) = (cstr(&text(it, a, 0)?), cstr(&text(it, a, 1)?));
        let e = unsafe { libc::getservbyname(svc.as_ptr(), proto.as_ptr()) };
        Ok(if e.is_null() { Value::Bool(false) } else { Value::Int(u16::from_be(unsafe { (*e).s_port } as u16) as i64) })
    });
    t.insert("getservbyport", |it, _, a| {
        let port = int(it, a, 0)?;
        let proto = cstr(&text(it, a, 1)?);
        let e = unsafe { libc::getservbyport((port as u16).to_be() as i32, proto.as_ptr()) };
        Ok(if e.is_null() { Value::Bool(false) } else { Value::bytes(unsafe { CStr::from_ptr((*e).s_name) }.to_bytes().to_vec()) })
    });
    t.insert("getprotobyname", |it, _, a| {
        let name = cstr(&text(it, a, 0)?);
        let e = unsafe { libc::getprotobyname(name.as_ptr()) };
        Ok(if e.is_null() { Value::Bool(false) } else { Value::Int(unsafe { (*e).p_proto } as i64) })
    });
    t.insert("getprotobynumber", |it, _, a| {
        let n = int(it, a, 0)?;
        let e = unsafe { libc::getprotobynumber(n as i32) };
        Ok(if e.is_null() { Value::Bool(false) } else { Value::bytes(unsafe { CStr::from_ptr((*e).p_name) }.to_bytes().to_vec()) })
    });
    t.insert("gethostbyaddr", |it, _, a| {
        let ip = text(it, a, 0)?;
        let Ok(addr) = ip.parse::<IpAddr>() else {
            it.warn("gethostbyaddr(): Address is not a valid IPv4 or IPv6 address");
            return Ok(Value::Bool(false));
        };
        let rev = match addr {
            IpAddr::V4(v) => {
                let o = v.octets();
                format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
            }
            IpAddr::V6(v) => {
                let hex: String = v.octets().iter().map(|b| format!("{b:02x}")).collect();
                let mut s: Vec<String> = hex.chars().rev().map(|c| c.to_string()).collect();
                s.push("ip6.arpa".into());
                s.join(".")
            }
        };
        if let Ok(hosts) = std::fs::read_to_string("/etc/hosts") {
            for l in hosts.lines() {
                let mut f = l.split_whitespace();
                if f.next().and_then(|x| x.parse::<IpAddr>().ok()) == Some(addr) {
                    if let Some(h) = f.next() {
                        return Ok(Value::str(h));
                    }
                }
            }
        }
        Ok(match query(&rev, 12) {
            Some((msg, rrs)) => match rrs.iter().find(|r| r.ty == 12) {
                Some(rr) => Value::str(&read_name(&msg, rr.data_at).map(|x| x.0).unwrap_or(ip)),
                None => Value::str(&ip),
            },
            None => Value::str(&ip),
        })
    });
    let check: BuiltinFn = |it, _, a| {
        let host = text(it, a, 0)?;
        let ty = if has(a, 1) { text(it, a, 1)? } else { "MX".into() };
        let Some(code) = type_code(&ty) else {
            return Err(it.error("ValueError", &format!("checkdnsrr(): Argument #2 ($type) must be a valid DNS record type")));
        };
        Ok(Value::Bool(query(&host, code).is_some_and(|(_, rrs)| !rrs.is_empty())))
    };
    t.insert("checkdnsrr", check);
    t.insert("dns_check_record", check);
    let mx: BuiltinFn = |it, _, a| {
        let host = text(it, a, 0)?;
        let recs = mx_records(&host);
        if a.len() > 1 {
            a[1] = Value::array(PhpArray::from_list(recs.iter().map(|r| Value::str(&r.1)).collect::<Vec<_>>()));
        }
        if a.len() > 2 {
            a[2] = Value::array(PhpArray::from_list(recs.iter().map(|r| Value::Int(r.0)).collect::<Vec<_>>()));
        }
        Ok(Value::Bool(!recs.is_empty()))
    };
    t.insert("getmxrr", mx);
    t.insert("dns_get_mx", mx);
    t.insert("dns_get_record", |it, _, a| dns_get_record(it, a));
    t.insert("net_get_interfaces", |_, _, _| Ok(interfaces()));
    t.insert("ftok", |it, _, a| {
        let path = text(it, a, 0)?;
        let proj = s(it, a, 1)?;
        if proj.len() != 1 {
            return Err(it.error("ValueError", "ftok(): Argument #2 ($project_id) must be a single character"));
        }
        if path.is_empty() {
            return Err(it.error("ValueError", "ftok(): Argument #1 ($filename) cannot be empty"));
        }
        let p = cstr(&path);
        let k = unsafe { libc::ftok(p.as_ptr(), proj[0] as i32) };
        if k == -1 {
            it.warn(&format!("ftok(): ftok() failed - {}", std::io::Error::last_os_error()));
        }
        Ok(Value::Int(k as i64))
    });
    t.insert("openlog", |it, _, a| {
        // openlog() keeps the pointer: leak the ident.
        let ident = Box::leak(cstr(&text(it, a, 0)?).into_boxed_c_str());
        let (opt, fac) = (int(it, a, 1)?, int(it, a, 2)?);
        unsafe { libc::openlog(ident.as_ptr(), opt as i32, fac as i32) };
        Ok(Value::Bool(true))
    });
    t.insert("syslog", |it, _, a| {
        let pri = int(it, a, 0)?;
        let msg = cstr(&text(it, a, 1)?);
        unsafe { libc::syslog(pri as i32, c"%s".as_ptr(), msg.as_ptr()) };
        Ok(Value::Bool(true))
    });
    t.insert("closelog", |_, _, _| {
        unsafe { libc::closelog() };
        Ok(Value::Bool(true))
    });
    t.insert("stream_socket_pair", |it, _, a| {
        let (domain, ty, proto) = (int(it, a, 0)?, int(it, a, 1)?, int(it, a, 2)?);
        let mut fds = [0i32; 2];
        if unsafe { libc::socketpair(domain as i32, ty as i32, proto as i32, fds.as_mut_ptr()) } != 0 {
            it.warn(&format!("stream_socket_pair(): Failed to create sockets: [{}]: {}", std::io::Error::last_os_error().raw_os_error().unwrap_or(0), std::io::Error::last_os_error()));
            return Ok(Value::Bool(false));
        }
        let mut out = PhpArray::new();
        for fd in fds {
            let owned = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
            match phpx_runtime::net::Pipe::from_fd(owned) {
                Ok(p) => {
                    out.push(crate::sockets::pipe_stream(crate::sockets::Sock::pipe(p, None), "r+"));
                }
                Err(e) => return Err(crate::native::rt_error(it, e)),
            }
        }
        Ok(Value::array(out))
    });
    t.insert("stream_socket_recvfrom", |it, f, a| {
        let mut args = vec![arg(a, 0), arg(a, 1)];
        let r = (it.builtins["fread"])(it, f, &mut args)?;
        if a.len() > 3 {
            a[3] = Value::str("");
        }
        Ok(r)
    });
    t.insert("stream_socket_sendto", |it, f, a| {
        let mut args = vec![arg(a, 0), arg(a, 1)];
        (it.builtins["fwrite"])(it, f, &mut args)
    });
    t.insert("get_headers", |it, _, a| {
        let url = text(it, a, 0)?;
        match url.split_once("://") {
            Some((scheme, _)) => it.warn(&format!("get_headers(): Unable to find the wrapper \"{scheme}\" - did you forget to enable it when you configured PHP?")),
            None => it.warn(&format!("get_headers({url}): Failed to open stream: No such file or directory")),
        }
        Ok(Value::Bool(false))
    });
    let _ = arg;
}
