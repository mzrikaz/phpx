//! ext/fileinfo: finfo, finfo_* and mime_content_type(), answering the
//! way libmagic does for the formats web apps meet (images, documents,
//! archives, office files, audio/video, scripts and text with its charset).

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::builtins::{arg, has, int, s};
use crate::class::Class;
use crate::interp::{BuiltinFn, Interp, NativeMethod, R};
use crate::value::{Internal, NativeData, Obj, Value};

const MIME_TYPE: i64 = 16;
const MIME_ENCODING: i64 = 1024;
const EXTENSION: i64 = 16777216;

/// What libmagic says about some bytes.
struct Magic {
    mime: String,
    desc: String,
    /// FILEINFO_EXTENSION's answer.
    ext: &'static str,
}

fn magic(mime: &str, desc: impl Into<String>, ext: &'static str) -> Magic {
    Magic { mime: mime.into(), desc: desc.into(), ext }
}

fn be16(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 2).map_or(0, |b| u16::from_be_bytes([b[0], b[1]]) as u32)
}

fn be32(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 4).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn le16(d: &[u8], i: usize) -> u32 {
    d.get(i..i + 2).map_or(0, |b| u16::from_le_bytes([b[0], b[1]]) as u32)
}

/// libmagic's text classes: (charset, description) or None for binary.
fn text_class(d: &[u8]) -> Option<(&'static str, &'static str)> {
    let texty = |b: u8| (0x20..0x7f).contains(&b) || (7..=13).contains(&b) || b == 0x1b;
    if d.iter().all(|&b| texty(b)) {
        return Some(("us-ascii", "ASCII text"));
    }
    if let Ok(s) = std::str::from_utf8(d) {
        if s.chars().all(|c| !c.is_control() || matches!(c, '\u{7}'..='\u{d}' | '\u{1b}')) {
            return Some(("utf-8", "Unicode text, UTF-8 text"));
        }
    }
    if d.iter().all(|&b| texty(b) || b >= 0xa0) {
        return Some(("iso-8859-1", "ISO-8859 text"));
    }
    None
}

/// ", with CRLF line terminators" & co.
fn line_info(d: &[u8]) -> String {
    let (mut crlf, mut lf, mut cr) = (false, false, false);
    let mut i = 0;
    let (mut longest, mut cur) = (0, 0);
    while i < d.len() {
        match d[i] {
            b'\r' if d.get(i + 1) == Some(&b'\n') => {
                crlf = true;
                i += 1;
                longest = longest.max(cur);
                cur = 0;
            }
            b'\r' => {
                cr = true;
                longest = longest.max(cur);
                cur = 0;
            }
            b'\n' => {
                lf = true;
                longest = longest.max(cur);
                cur = 0;
            }
            _ => cur += 1,
        }
        i += 1;
    }
    longest = longest.max(cur);
    let mut out = String::new();
    if longest > 300 {
        out.push_str(&format!(", with very long lines ({longest})"));
    }
    let kinds: Vec<&str> = [(crlf, "CRLF"), (cr, "CR"), (lf, "LF")].iter().filter(|k| k.0).map(|k| k.1).collect();
    match kinds.as_slice() {
        [] => out.push_str(", with no line terminators"),
        ["LF"] => {}
        k => out.push_str(&format!(", with {} line terminators", k.join(", "))),
    }
    out
}

/// A strict JSON value (libmagic's is_json), object or array at the top.
fn is_json(d: &[u8]) -> bool {
    fn ws(d: &[u8], i: &mut usize) {
        while *i < d.len() && matches!(d[*i], b' ' | b'\t' | b'\n' | b'\r') {
            *i += 1;
        }
    }
    fn value(d: &[u8], i: &mut usize, depth: u32) -> bool {
        if depth > 500 {
            return false;
        }
        ws(d, i);
        match d.get(*i) {
            Some(b'{') => {
                *i += 1;
                ws(d, i);
                if d.get(*i) == Some(&b'}') {
                    *i += 1;
                    return true;
                }
                loop {
                    ws(d, i);
                    if !string(d, i) {
                        return false;
                    }
                    ws(d, i);
                    if d.get(*i) != Some(&b':') {
                        return false;
                    }
                    *i += 1;
                    if !value(d, i, depth + 1) {
                        return false;
                    }
                    ws(d, i);
                    match d.get(*i) {
                        Some(b',') => *i += 1,
                        Some(b'}') => {
                            *i += 1;
                            return true;
                        }
                        _ => return false,
                    }
                }
            }
            Some(b'[') => {
                *i += 1;
                ws(d, i);
                if d.get(*i) == Some(&b']') {
                    *i += 1;
                    return true;
                }
                loop {
                    if !value(d, i, depth + 1) {
                        return false;
                    }
                    ws(d, i);
                    match d.get(*i) {
                        Some(b',') => *i += 1,
                        Some(b']') => {
                            *i += 1;
                            return true;
                        }
                        _ => return false,
                    }
                }
            }
            Some(b'"') => string(d, i),
            Some(b't') => lit(d, i, b"true"),
            Some(b'f') => lit(d, i, b"false"),
            Some(b'n') => lit(d, i, b"null"),
            Some(b'-' | b'0'..=b'9') => {
                let start = *i;
                while *i < d.len() && matches!(d[*i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    *i += 1;
                }
                *i > start
            }
            _ => false,
        }
    }
    fn string(d: &[u8], i: &mut usize) -> bool {
        if d.get(*i) != Some(&b'"') {
            return false;
        }
        *i += 1;
        while *i < d.len() {
            match d[*i] {
                b'\\' => *i += 2,
                b'"' => {
                    *i += 1;
                    return true;
                }
                c if c < 0x20 => return false,
                _ => *i += 1,
            }
        }
        false
    }
    fn lit(d: &[u8], i: &mut usize, w: &[u8]) -> bool {
        if d[*i..].starts_with(w) {
            *i += w.len();
            true
        } else {
            false
        }
    }
    let mut i = 0;
    ws(d, &mut i);
    if !matches!(d.get(i), Some(b'{' | b'[')) {
        return false;
    }
    if !value(d, &mut i, 0) {
        return false;
    }
    ws(d, &mut i);
    i == d.len()
}

/// Office/ODF/EPUB/JAR files are ZIP archives told apart by their entries.
fn zip_kind(d: &[u8]) -> Magic {
    let name_len = le16(d, 26) as usize;
    let first = d.get(30..30 + name_len).unwrap_or(&[]);
    if first == b"mimetype" {
        let extra = le16(d, 28) as usize;
        let start = 30 + name_len + extra;
        let body = &d[start.min(d.len())..];
        let mt: Vec<u8> = body.iter().take_while(|b| b.is_ascii_graphic()).copied().collect();
        let mt = String::from_utf8_lossy(&mt).into_owned();
        let desc = match mt.as_str() {
            "application/epub+zip" => "EPUB document",
            "application/vnd.oasis.opendocument.text" => "OpenDocument Text",
            "application/vnd.oasis.opendocument.spreadsheet" => "OpenDocument Spreadsheet",
            "application/vnd.oasis.opendocument.presentation" => "OpenDocument Presentation",
            _ => "Zip data (MIME type \"{mt}\"?)",
        };
        if !mt.is_empty() {
            return magic(&mt, desc, "???");
        }
    }
    let window = &d[..d.len().min(8192)];
    let has = |needle: &[u8]| window.windows(needle.len()).any(|w| w == needle);
    if has(b"[Content_Types].xml") || has(b"_rels/.rels") {
        if has(b"word/") {
            return magic("application/vnd.openxmlformats-officedocument.wordprocessingml.document", "Microsoft Word 2007+", "docx");
        }
        if has(b"xl/") {
            return magic("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", "Microsoft Excel 2007+", "xlsx");
        }
        if has(b"ppt/") {
            return magic("application/vnd.openxmlformats-officedocument.presentationml.presentation", "Microsoft PowerPoint 2007+", "pptx");
        }
        return magic("application/octet-stream", "Microsoft OOXML", "???");
    }
    if has(b"META-INF/MANIFEST.MF") {
        return magic("application/java-archive", "Java archive data (JAR)", "jar");
    }
    let ver = le16(d, 4);
    magic("application/zip", format!("Zip archive data, at least v{}.{} to extract", ver / 10, ver % 10), "zip")
}

fn detect(d: &[u8]) -> Magic {
    if d.is_empty() {
        return magic("application/x-empty", "empty", "???");
    }
    let starts = |p: &[u8]| d.starts_with(p);
    if starts(b"\x89PNG\r\n\x1a\n") {
        let (w, h, depth, color, inter) = (be32(d, 16), be32(d, 20), d.get(24).copied().unwrap_or(0), d.get(25).copied().unwrap_or(0), d.get(28).copied().unwrap_or(0));
        let kind = match color {
            0 => "grayscale",
            2 => "/color RGB",
            3 => "colormap",
            4 => "gray+alpha",
            _ => "/color RGBA",
        };
        let sep = if kind.starts_with('/') { "" } else { " " };
        let il = if inter == 1 { "interlaced" } else { "non-interlaced" };
        return magic("image/png", format!("PNG image data, {w} x {h}, {depth}-bit{sep}{kind}, {il}"), "png");
    }
    if starts(b"\xff\xd8\xff") {
        return magic("image/jpeg", "JPEG image data", "jpeg/jpg/jpe/jfif");
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        let v = String::from_utf8_lossy(&d[3..6]).into_owned();
        return magic("image/gif", format!("GIF image data, version {v}, {} x {}", le16(d, 6), le16(d, 8)), "gif");
    }
    if starts(b"RIFF") && d.get(8..12) == Some(b"WEBP") {
        return magic("image/webp", "RIFF (little-endian) data, Web/P image", "webp");
    }
    if starts(b"RIFF") && d.get(8..12) == Some(b"WAVE") {
        return magic("audio/x-wav", "RIFF (little-endian) data, WAVE audio", "wav");
    }
    if starts(b"RIFF") && d.get(8..12) == Some(b"AVI ") {
        return magic("video/x-msvideo", "RIFF (little-endian) data, AVI", "avi");
    }
    if starts(b"BM") && d.len() > 14 && be16(d, 6) == 0 {
        return magic("image/bmp", "PC bitmap", "bmp");
    }
    if starts(b"\0\0\x01\0") {
        return magic("image/vnd.microsoft.icon", "MS Windows icon resource", "ico");
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return magic("image/tiff", "TIFF image data", "tif/tiff");
    }
    if d.get(4..8) == Some(b"ftyp") {
        let brand = d.get(8..12).unwrap_or(b"");
        return match brand {
            b"avif" | b"avis" => magic("image/avif", "ISO Media, AVIF Image", "avif"),
            b"heic" | b"heix" | b"mif1" => magic("image/heic", "ISO Media, HEIF Image", "heic"),
            b"qt  " => magic("video/quicktime", "ISO Media, Apple QuickTime movie", "mov"),
            b"M4A " => magic("audio/x-m4a", "ISO Media, Apple iTunes ALAC/AAC-LC (.M4A) Audio", "m4a"),
            _ => magic("video/mp4", "ISO Media, MP4 v2", "mp4"),
        };
    }
    if starts(b"\x1aE\xdf\xa3") {
        return magic("video/webm", "WebM", "webm");
    }
    if starts(b"OggS") {
        return magic("audio/ogg", "Ogg data", "ogg");
    }
    if starts(b"ID3") || (d.len() > 1 && d[0] == 0xff && d[1] & 0xe0 == 0xe0) {
        return magic("audio/mpeg", "Audio file with ID3", "mp3");
    }
    if starts(b"fLaC") {
        return magic("audio/flac", "FLAC audio bitstream data", "flac");
    }
    if starts(b"%PDF-") {
        let v: String = d[5..].iter().take_while(|b| b.is_ascii_digit() || **b == b'.').map(|&b| b as char).collect();
        return magic("application/pdf", format!("PDF document, version {v}"), "pdf");
    }
    if starts(b"PK\x03\x04") {
        if d.len() < 30 {
            return magic("application/octet-stream", "data", "???");
        }
        return zip_kind(d);
    }
    if starts(b"PK\x05\x06") {
        return magic("application/zip", "Zip archive data (empty)", "zip");
    }
    if starts(b"\x1f\x8b") {
        return magic("application/gzip", "gzip compressed data", "???");
    }
    if starts(b"BZh") {
        return magic("application/x-bzip2", "bzip2 compressed data", "bz2");
    }
    if starts(b"\xfd7zXZ\0") {
        return magic("application/x-xz", "XZ compressed data", "xz");
    }
    if starts(b"7z\xbc\xaf\x27\x1c") {
        return magic("application/x-7z-compressed", "7-zip archive data", "7z/cb7");
    }
    if starts(b"Rar!\x1a\x07") {
        return magic("application/x-rar", "RAR archive data", "rar/cbr");
    }
    if starts(b"\x28\xb5\x2f\xfd") {
        return magic("application/zstd", "Zstandard compressed data", "zst");
    }
    if d.get(257..262) == Some(b"ustar") {
        return magic("application/x-tar", "POSIX tar archive", "tar/gtar");
    }
    if starts(b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1") {
        return magic("application/CDFV2", "Composite Document File V2 Document", "???");
    }
    if starts(b"SQLite format 3\0") {
        return magic("application/vnd.sqlite3", "SQLite 3.x database", "sqlite/sqlite3/db/db3/dbe/sdb");
    }
    if starts(b"\x7fELF") {
        return magic("application/x-executable", "ELF executable", "???");
    }
    if starts(b"\0asm") {
        return magic("application/wasm", "WebAssembly (wasm) binary module", "wasm");
    }
    if starts(b"wOFF") {
        return magic("font/woff", "Web Open Font Format", "woff");
    }
    if starts(b"wOF2") {
        return magic("font/woff2", "Web Open Font Format (Version 2)", "woff2");
    }
    let Some((_, text)) = text_class(d) else {
        return magic("application/octet-stream", "data", "???");
    };
    let lines = line_info(d);
    let trimmed: &[u8] = {
        let mut i = 0;
        if d.starts_with(b"\xef\xbb\xbf") {
            i = 3;
        }
        while i < d.len() && d[i].is_ascii_whitespace() {
            i += 1;
        }
        &d[i..]
    };
    let lower: Vec<u8> = trimmed.iter().take(512).map(|b| b.to_ascii_lowercase()).collect();
    if d.starts_with(b"#!") {
        let first = d.split(|b| *b == b'\n').next().unwrap_or(b"");
        let interp = String::from_utf8_lossy(first);
        let (mime, kind) = if interp.contains("php") {
            ("text/x-php", "PHP script")
        } else if interp.contains("python") {
            ("text/x-script.python", "Python script")
        } else if interp.contains("node") {
            ("application/javascript", "Node.js script")
        } else if interp.contains("perl") {
            ("text/x-perl", "Perl script")
        } else if interp.contains("bash") {
            ("text/x-shellscript", "Bourne-Again shell script")
        } else {
            ("text/x-shellscript", "POSIX shell script")
        };
        return magic(mime, format!("{kind}, {text} executable{lines}"), "???");
    }
    if lower.starts_with(b"<?php") {
        return magic("text/x-php", format!("PHP script, {text}{lines}"), "???");
    }
    if lower.starts_with(b"<?xml") {
        let svg = trimmed.windows(4).take(2048).any(|w| w == b"<svg");
        if svg {
            return magic("image/svg+xml", format!("SVG Scalable Vector Graphics image, {text}{lines}"), "svg");
        }
        return magic("text/xml", format!("XML 1.0 document, {text}{lines}"), "???");
    }
    if lower.starts_with(b"<svg") {
        return magic("image/svg+xml", format!("SVG Scalable Vector Graphics image, {text}{lines}"), "svg");
    }
    if lower.starts_with(b"<!doctype html") || lower.starts_with(b"<html") || lower.starts_with(b"<head") || lower.starts_with(b"<body") {
        return magic("text/html", format!("HTML document, {text}{lines}"), "???");
    }
    if is_json(d) {
        return magic("application/json", "JSON text data", "???");
    }
    magic("text/plain", format!("{text}{lines}"), "???")
}

fn charset(d: &[u8]) -> &'static str {
    if d.is_empty() {
        return "binary";
    }
    text_class(d).map_or("binary", |t| t.0)
}

/// The answer for `flags`.
fn answer(d: &[u8], flags: i64) -> String {
    let m = detect(d);
    if flags & EXTENSION != 0 {
        return m.ext.to_string();
    }
    match (flags & MIME_TYPE != 0, flags & MIME_ENCODING != 0) {
        (true, true) => format!("{}; charset={}", m.mime, charset(d)),
        (true, false) => m.mime,
        (false, true) => charset(d).to_string(),
        _ => m.desc,
    }
}

/// Bytes of a path for detection (libmagic reads at most ~1 MiB), or a
/// directory/other answer.
enum Source {
    Bytes(Vec<u8>),
    Dir,
}

fn read_path(it: &Interp, func: &str, path: &str) -> R<Option<Source>> {
    if path.is_empty() {
        return Err(it.error("ValueError", &format!("{func}(): Argument #1 ($filename) cannot be empty")));
    }
    let p = std::path::PathBuf::from(path);
    if p.is_dir() {
        return Ok(Some(Source::Dir));
    }
    match std::fs::File::open(&p) {
        Ok(f) => {
            use std::io::Read;
            let mut buf = vec![];
            let _ = f.take(1 << 20).read_to_end(&mut buf);
            Ok(Some(Source::Bytes(buf)))
        }
        Err(_) => {
            it.warn(&format!("{func}({path}): Failed to open stream: No such file or directory"));
            Ok(None)
        }
    }
}

fn answer_source(src: Source, flags: i64) -> String {
    match src {
        Source::Bytes(b) => answer(&b, flags),
        Source::Dir => {
            if flags & MIME_TYPE != 0 && flags & MIME_ENCODING != 0 {
                "directory; charset=binary".into()
            } else if flags & MIME_ENCODING != 0 {
                "binary".into()
            } else {
                "directory".into()
            }
        }
    }
}

// ---- finfo objects --------------------------------------------------------------------

struct Finfo(Cell<i64>);

impl NativeData for Finfo {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn clone_data(&self) -> Option<Box<dyn NativeData>> {
        None
    }
}

fn flags_of(o: &Obj) -> i64 {
    match &*o.internal() {
        Internal::Data(d) => d.as_any().downcast_ref::<Finfo>().map_or(0, |f| f.0.get()),
        _ => 0,
    }
}

fn fi_construct(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let flags = if has(&a, 0) { int(it, &a, 0)? } else { 0 };
    o.unwrap().set_internal(Internal::Data(Box::new(Finfo(Cell::new(flags)))));
    Ok(Value::Null)
}

fn call_flags(it: &Interp, o: &Obj, a: &[Value], i: usize) -> R<i64> {
    Ok(match arg(a, i) {
        Value::Null => flags_of(o),
        v => match it.to_int(&v)? {
            0 => flags_of(o),
            n => n,
        },
    })
}

fn fi_file(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = o.unwrap();
    let path = String::from_utf8_lossy(&s(it, &a, 0)?).into_owned();
    let flags = call_flags(it, o, &a, 1)?;
    Ok(match read_path(it, "finfo::file", &path)? {
        Some(src) => Value::str(&answer_source(src, flags)),
        None => Value::Bool(false),
    })
}

fn fi_buffer(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let o = o.unwrap();
    let data = s(it, &a, 0)?;
    let flags = call_flags(it, o, &a, 1)?;
    Ok(Value::str(&answer(&data, flags)))
}

fn fi_set_flags(it: &Interp, o: Option<&Rc<Obj>>, _: &Rc<Class>, a: Vec<Value>) -> R<Value> {
    let flags = int(it, &a, 0)?;
    if let Internal::Data(d) = &*o.unwrap().internal() {
        if let Some(f) = d.as_any().downcast_ref::<Finfo>() {
            f.0.set(flags);
        }
    }
    Ok(Value::Bool(true))
}

pub fn natives() -> Vec<(&'static str, &'static str, NativeMethod)> {
    vec![
        ("finfo", "__construct", fi_construct),
        ("finfo", "file", fi_file),
        ("finfo", "buffer", fi_buffer),
        ("finfo", "set_flags", fi_set_flags),
    ]
}

fn finfo_arg(it: &Interp, a: &[Value], func: &str) -> R<Rc<Obj>> {
    match arg(a, 0) {
        Value::Object(o) if o.class.is_a("finfo") => Ok(o),
        v => Err(it.error("TypeError", &format!("{func}(): Argument #1 ($finfo) must be of type finfo, {} given", v.type_name()))),
    }
}

pub fn register(t: &mut HashMap<&'static str, BuiltinFn>) {
    t.insert("finfo_open", |it, _, a| {
        let cls = it.classes.borrow().get("finfo").cloned().expect("finfo class");
        let o = it.new_instance(&cls)?;
        fi_construct(it, Some(&o), &cls, a.to_vec())?;
        Ok(Value::Object(o))
    });
    t.insert("finfo_close", |it, _, a| {
        finfo_arg(it, a, "finfo_close")?;
        Ok(Value::Bool(true))
    });
    t.insert("finfo_set_flags", |it, _, a| {
        let o = finfo_arg(it, a, "finfo_set_flags")?;
        let cls = o.class.clone();
        fi_set_flags(it, Some(&o), &cls, a[1..].to_vec())
    });
    t.insert("finfo_file", |it, _, a| {
        let o = finfo_arg(it, a, "finfo_file")?;
        let path = String::from_utf8_lossy(&s(it, a, 1)?).into_owned();
        let flags = call_flags(it, &o, a, 2)?;
        Ok(match read_path(it, "finfo_file", &path)? {
            Some(src) => Value::str(&answer_source(src, flags)),
            None => Value::Bool(false),
        })
    });
    t.insert("finfo_buffer", |it, _, a| {
        let o = finfo_arg(it, a, "finfo_buffer")?;
        let data = s(it, a, 1)?;
        let flags = call_flags(it, &o, a, 2)?;
        Ok(Value::str(&answer(&data, flags)))
    });
    t.insert("mime_content_type", |it, _, a| {
        if let Value::Resource(_) = arg(a, 0) {
            let data = (it.builtins["stream_get_contents"])(it, &mut crate::interp::Frame::new(it.file_ctx(0), 0), &mut [arg(a, 0)])?;
            return Ok(Value::str(&answer(&it.to_str(&data)?, MIME_TYPE)));
        }
        let path = String::from_utf8_lossy(&s(it, a, 0)?).into_owned();
        Ok(match read_path(it, "mime_content_type", &path)? {
            Some(src) => Value::str(&answer_source(src, MIME_TYPE)),
            None => Value::Bool(false),
        })
    });
}
