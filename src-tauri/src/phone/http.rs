// Tiny HTTP/1.1 for the phone link: parse one request head from a buffered socket, write one response (always `Connection: close`).
use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::net::IpAddr;

const MAX_HEAD: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;

#[derive(Debug, Default)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    /// lower-case names
    pub headers: HashMap<String, String>,
}

impl Req {
    pub fn header(&self, k: &str) -> Option<&str> {
        self.headers.get(k).map(String::as_str)
    }
    /// `Authorization: Bearer <token>`
    pub fn bearer(&self) -> Option<&str> {
        let v = self.header("authorization")?;
        let (scheme, tok) = v.split_once(' ')?;
        scheme.eq_ignore_ascii_case("bearer").then(|| tok.trim()).filter(|t| !t.is_empty())
    }
    pub fn content_length(&self) -> Option<u64> {
        self.header("content-length")?.trim().parse().ok()
    }
    /// Language of the first Accept-Language entry: "ru" or "en".
    pub fn lang(&self) -> &'static str {
        match self.header("accept-language").and_then(|v| v.split([',', ';', '-']).next()).map(|s| s.trim().to_ascii_lowercase()) {
            Some(l) if l == "ru" || l == "uk" || l == "be" => "ru",
            _ => "en",
        }
    }
}

pub fn pct_decode(s: &str, plus_is_space: bool) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() => {
                out.push(u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("0"), 16).unwrap_or(0));
                i += 3;
            }
            b'+' if plus_is_space => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn pct_encode(s: &str) -> String {
    let mut o = String::new();
    for &c in s.as_bytes() {
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.' | b'~') {
            o.push(c as char);
        } else {
            o.push_str(&format!("%{c:02X}"));
        }
    }
    o
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&').filter(|p| !p.is_empty()).map(|p| match p.split_once('=') {
        Some((k, v)) => (pct_decode(k, true), pct_decode(v, true)),
        None => (pct_decode(p, true), String::new()),
    }).collect()
}

/// Reads the request line and the headers (up to the blank line). `Err` = malformed, too long or the peer went away.
pub fn read_head(r: &mut impl BufRead) -> io::Result<Req> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let mut used = 0usize;
    let mut line = String::new();
    let mut next = |r: &mut dyn BufRead, line: &mut String, used: &mut usize| -> io::Result<()> {
        line.clear();
        let n = r.take((MAX_HEAD - *used + 1) as u64).read_line(line)?;
        *used += n;
        if n == 0 || *used > MAX_HEAD || !line.ends_with('\n') {
            return Err(bad("head"));
        }
        Ok(())
    };
    next(r, &mut line, &mut used)?;
    let mut parts = line.trim_end().split(' ');
    let (method, target, ver) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if method.is_empty() || !target.starts_with('/') || !ver.starts_with("HTTP/1.") {
        return Err(bad("request line"));
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p, parse_query(q)),
        None => (target, HashMap::new()),
    };
    let mut req = Req { method: method.to_ascii_uppercase(), path: pct_decode(path, false), query, headers: HashMap::new() };
    loop {
        next(r, &mut line, &mut used)?;
        let l = line.trim_end();
        if l.is_empty() {
            return Ok(req);
        }
        if req.headers.len() >= MAX_HEADERS {
            return Err(bad("headers"));
        }
        let (k, v) = l.split_once(':').ok_or_else(|| bad("header"))?;
        req.headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
    }
}

fn reason(code: u16) -> &'static str {
    match code {
        101 => "Switching Protocols",
        200 => "OK",
        206 => "Partial Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

/// Status line + headers (Content-Length set from `len`).
pub fn head(code: u16, ctype: &str, len: u64, extra: &[(&str, String)]) -> Vec<u8> {
    let mut s = format!("HTTP/1.1 {code} {}\r\nContent-Type: {ctype}\r\nContent-Length: {len}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n", reason(code));
    for (k, v) in extra {
        s.push_str(&format!("{k}: {v}\r\n"));
    }
    s.push_str("\r\n");
    s.into_bytes()
}

pub fn respond(w: &mut impl Write, code: u16, ctype: &str, body: &[u8], extra: &[(&str, String)]) -> io::Result<()> {
    w.write_all(&head(code, ctype, body.len() as u64, extra))?;
    w.write_all(body)?;
    w.flush()
}

pub fn json(w: &mut impl Write, code: u16, v: &serde_json::Value) -> io::Result<()> {
    respond(w, code, "application/json; charset=utf-8", v.to_string().as_bytes(), &[])
}

pub fn err(w: &mut impl Write, code: u16, what: &str) -> io::Result<()> {
    json(w, code, &serde_json::json!({ "error": what }))
}

/// Private or link-local IPv4 (10/8, 172.16/12, 192.168/16, 169.254/16); `loopback` is the tests' switch.
pub fn peer_allowed(ip: IpAddr, loopback: bool) -> bool {
    let v4 = match ip {
        IpAddr::V4(a) => a,
        IpAddr::V6(a) => match a.to_ipv4_mapped() {
            Some(a) => a,
            None => return false,
        },
    };
    v4.is_private() || v4.is_link_local() || (loopback && v4.is_loopback())
}

/// `Range: bytes=a-b | a- | -n` against a file of `size` bytes: inclusive (start, end), or None when unsatisfiable. A malformed header means "whole file".
pub enum Range {
    Whole,
    Part(u64, u64),
    Bad,
}
pub fn parse_range(h: Option<&str>, size: u64) -> Range {
    let Some(h) = h else { return Range::Whole };
    let Some(spec) = h.trim().strip_prefix("bytes=") else { return Range::Whole };
    if spec.contains(',') {
        return Range::Whole; // multi-range: the whole file is a valid answer
    }
    let Some((a, b)) = spec.split_once('-') else { return Range::Whole };
    let (a, b) = (a.trim(), b.trim());
    if size == 0 {
        return Range::Bad;
    }
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(s), Ok(e)) if s <= e && s < size => Range::Part(s, e.min(size - 1)),
        (Ok(s), Err(_)) if b.is_empty() && s < size => Range::Part(s, size - 1),
        (Err(_), Ok(n)) if a.is_empty() && n > 0 => Range::Part(size.saturating_sub(n), size - 1),
        (Ok(_), Ok(_)) | (Ok(_), Err(_)) if !a.is_empty() => Range::Bad,
        (Err(_), Ok(_)) if a.is_empty() => Range::Bad,
        _ => Range::Whole,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_a_request() {
        let mut c = Cursor::new(b"GET /p?c=AB3D&n=My%20PC+x HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer tok123\r\nAccept-Language: ru-RU,ru;q=0.9\r\n\r\nBODY".to_vec());
        let r = read_head(&mut c).unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str()), ("GET", "/p"));
        assert_eq!(r.query["c"], "AB3D");
        assert_eq!(r.query["n"], "My PC x");
        assert_eq!(r.bearer(), Some("tok123"));
        assert_eq!(r.lang(), "ru");
    }

    #[test]
    fn rejects_garbage_and_oversized_heads() {
        assert!(read_head(&mut Cursor::new(b"NOPE\r\n\r\n".to_vec())).is_err());
        assert!(read_head(&mut Cursor::new(b"GET /x HTTP/1.1\r\nbroken header\r\n\r\n".to_vec())).is_err());
        let mut big = b"GET /x HTTP/1.1\r\n".to_vec();
        big.extend(std::iter::repeat(b'a').take(MAX_HEAD + 10));
        assert!(read_head(&mut Cursor::new(big)).is_err());
        assert!(read_head(&mut Cursor::new(b"GET /x HTTP/1.1\r\nHost: x".to_vec())).is_err()); // no terminator: the peer went away
    }

    #[test]
    fn percent_coding() {
        assert_eq!(pct_decode("a%20b%2", false), "a b%2");
        assert_eq!(pct_decode("%zz", false), "%zz");
        assert_eq!(pct_encode("Pixel 8/ü"), "Pixel%208%2F%C3%BC");
    }

    #[test]
    fn only_private_peers() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for ok in ["10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.0.7", "169.254.9.9", "::ffff:192.168.1.5"] {
            assert!(peer_allowed(ip(ok), false), "{ok}");
        }
        for no in ["8.8.8.8", "172.32.0.1", "127.0.0.1", "::1", "2001:db8::1", "100.64.0.1"] {
            assert!(!peer_allowed(ip(no), false), "{no}");
        }
        assert!(peer_allowed(ip("127.0.0.1"), true));
    }

    #[test]
    fn ranges() {
        assert!(matches!(parse_range(None, 100), Range::Whole));
        assert!(matches!(parse_range(Some("bytes=10-19"), 100), Range::Part(10, 19)));
        assert!(matches!(parse_range(Some("bytes=90-"), 100), Range::Part(90, 99)));
        assert!(matches!(parse_range(Some("bytes=-10"), 100), Range::Part(90, 99)));
        assert!(matches!(parse_range(Some("bytes=50-500"), 100), Range::Part(50, 99)));
        assert!(matches!(parse_range(Some("bytes=100-"), 100), Range::Bad));
        assert!(matches!(parse_range(Some("bytes=9-3"), 100), Range::Bad));
        assert!(matches!(parse_range(Some("bytes=0-1,5-6"), 100), Range::Whole));
        assert!(matches!(parse_range(Some("items=1-2"), 100), Range::Whole));
        assert!(matches!(parse_range(Some("bytes=0-"), 0), Range::Bad));
    }
}
