// Minimal RFC 6455 server side for the phone link: the handshake (SHA-1 + base64 of the key), reading client frames (always masked,
// fragments joined, ping / pong / close handled by the caller through `Msg`) and writing unmasked server frames. Text frames only carry JSON.
use base64::Engine;
use std::io::{self, Read, Write};

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
/// A single message larger than this closes the socket (icons are 96x96 PNGs, a few KB).
pub const MAX_MSG: usize = 1 << 20;

/// SHA-1 (only for the WebSocket accept key; nothing secret depends on it).
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// `Sec-WebSocket-Accept` for a client key.
pub fn accept_key(key: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(sha1(format!("{}{}", key.trim(), GUID).as_bytes()))
}

#[derive(Debug, PartialEq)]
pub enum Msg {
    Text(String),
    /// a binary message (v2 relay frames); the LAN server treats one as a protocol error
    Bin(Vec<u8>),
    Ping(Vec<u8>),
    Pong,
    Close(u16),
}

/// One server frame (FIN set, unmasked).
pub fn frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => f.push(n as u8),
        n if n <= 0xFFFF => {
            f.push(126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(payload);
    f
}
pub fn text_frame(s: &str) -> Vec<u8> {
    frame(0x1, s.as_bytes())
}
pub fn binary_frame(b: &[u8]) -> Vec<u8> {
    frame(0x2, b)
}
pub fn close_frame(code: u16) -> Vec<u8> {
    frame(0x8, &code.to_be_bytes())
}
pub fn pong_frame(payload: &[u8]) -> Vec<u8> {
    frame(0xA, &payload[..payload.len().min(125)])
}

/// Reads one complete message as a server (client frames must be masked). Errors (also timeouts) end the connection.
pub fn read_msg(r: &mut impl Read) -> io::Result<Msg> {
    read_msg_as(r, true)
}

/// `masked_peer`: true when reading a client's frames (the server side), false when reading a server's frames (the relay client).
pub fn read_msg_as(r: &mut impl Read, masked_peer: bool) -> io::Result<Msg> {
    let mut text: Vec<u8> = Vec::new();
    let mut binary = false;
    loop {
        let mut h = [0u8; 2];
        r.read_exact(&mut h)?;
        let (fin, op, masked) = (h[0] & 0x80 != 0, h[0] & 0x0F, h[1] & 0x80 != 0);
        let mut len = (h[1] & 0x7F) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            len = u64::from_be_bytes(b);
        }
        if masked != masked_peer || len > MAX_MSG as u64 || (op >= 8 && len > 125) || text.len() + len as usize > MAX_MSG {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad frame"));
        }
        let mut mask = [0u8; 4];
        if masked {
            r.read_exact(&mut mask)?;
        }
        let mut p = vec![0u8; len as usize];
        r.read_exact(&mut p)?;
        if masked {
            for (i, b) in p.iter_mut().enumerate() {
                *b ^= mask[i % 4];
            }
        }
        match op {
            0x0..=0x2 => {
                binary |= op == 0x2;
                text.extend_from_slice(&p);
                if fin {
                    return if binary { Ok(Msg::Bin(text)) } else { String::from_utf8(text).map(Msg::Text).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "utf8")) };
                }
            }
            0x8 => return Ok(Msg::Close(if p.len() >= 2 { u16::from_be_bytes([p[0], p[1]]) } else { 1005 })),
            0x9 => return Ok(Msg::Ping(p)),
            0xA => return Ok(Msg::Pong),
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "opcode")),
        }
    }
}

pub fn write_all(w: &mut impl Write, bytes: &[u8]) -> io::Result<()> {
    w.write_all(bytes)?;
    w.flush()
}

/// A masked client frame (RFC 6455: everything a client sends is masked), FIN set.
pub fn masked_frame(op: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    client_frame_with(op, true, payload, mask)
}

/// A masked client frame, the way a phone builds it (tests of this module and of the server).
#[cfg(test)]
pub fn client_frame(op: u8, fin: bool, payload: &[u8]) -> Vec<u8> {
    client_frame_with(op, fin, payload, [0x12u8, 0x34, 0x56, 0x78])
}

fn client_frame_with(op: u8, fin: bool, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut f = vec![if fin { 0x80 } else { 0 } | op];
    match payload.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n if n <= 0xFFFF => {
            f.push(0x80 | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(0x80 | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(&mask);
    f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha1_known_vectors() {
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn rfc_accept_key() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn frames_roundtrip_fragments_and_control() {
        let mut v = client_frame(0x1, false, b"he");
        v.extend(client_frame(0x0, true, b"llo"));
        v.extend(client_frame(0x9, true, b"p"));
        v.extend(client_frame(0x1, true, &vec![b'x'; 300]));
        v.extend(client_frame(0x8, true, &4001u16.to_be_bytes()));
        let mut c = std::io::Cursor::new(v);
        assert_eq!(read_msg(&mut c).unwrap(), Msg::Text("hello".into()));
        assert_eq!(read_msg(&mut c).unwrap(), Msg::Ping(b"p".to_vec()));
        assert_eq!(read_msg(&mut c).unwrap(), Msg::Text("x".repeat(300)));
        assert_eq!(read_msg(&mut c).unwrap(), Msg::Close(4001));
    }

    #[test]
    fn unmasked_and_oversized_frames_are_refused() {
        assert!(read_msg(&mut std::io::Cursor::new(vec![0x81, 0x01, b'x'])).is_err()); // clients must mask
        let mut big = vec![0x81, 0x80 | 127];
        big.extend_from_slice(&((MAX_MSG as u64) + 1).to_be_bytes());
        big.extend_from_slice(&[0; 4]);
        assert!(read_msg(&mut std::io::Cursor::new(big)).is_err());
    }

    #[test]
    fn server_frames_have_the_right_length_forms() {
        assert_eq!(text_frame("hi"), vec![0x81, 2, b'h', b'i']);
        let f = frame(1, &vec![0; 200]);
        assert_eq!((f[1], f[2], f[3]), (126, 0, 200));
        assert_eq!(close_frame(4001), vec![0x88, 2, 0x0F, 0xA1]);
    }
}
