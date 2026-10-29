//! QR encoder of the Text tab, a line-by-line port of `qr()` in src/tab-text.js.
//!
//! Byte mode, error correction L, versions 1-10 (up to 271 UTF-8 bytes), the mask is chosen by penalty score.
//! ponytail: ECC L only and 271 bytes max, like the JS; extend QR_ECC / QR_BLK to go bigger.

const QR_ECC: [usize; 11] = [0, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18];
const QR_BLK: [usize; 11] = [0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4];

pub const MAX_BYTES: usize = 271;

#[derive(Clone, Debug, PartialEq)]
pub struct Qr {
    pub size: usize,
    pub version: usize,
    pub mask: usize,
    modules: Vec<bool>,
}
impl Qr {
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.modules[y * self.size + x]
    }
    /// Rows of '#' / '.' (tests and debugging).
    pub fn rows(&self) -> Vec<String> {
        (0..self.size).map(|y| (0..self.size).map(|x| if self.get(x, y) { '#' } else { '.' }).collect()).collect()
    }
}

fn gf_mul(x: u32, y: u32) -> u32 {
    let mut z = 0u32;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x11d);
        z ^= ((y >> i) & 1) * x;
    }
    z
}
fn rs_divisor(deg: usize) -> Vec<u32> {
    let mut r = vec![0u32; deg];
    r[deg - 1] = 1;
    let mut root = 1u32;
    for _ in 0..deg {
        for j in 0..deg {
            r[j] = gf_mul(r[j], root);
            if j + 1 < deg {
                r[j] ^= r[j + 1];
            }
        }
        root = gf_mul(root, 2);
    }
    r
}
fn rs_remainder(data: &[u32], div: &[u32]) -> Vec<u32> {
    let mut r = vec![0u32; div.len()];
    for &b in data {
        let f = b ^ r.remove(0);
        r.push(0);
        for (i, c) in div.iter().enumerate() {
            r[i] ^= gf_mul(*c, f);
        }
    }
    r
}
fn qr_raw(v: usize) -> usize {
    let mut r = (16 * v + 128) * v + 64;
    if v >= 2 {
        let n = v / 7 + 2;
        r -= (25 * n - 10) * n - 55;
        if v >= 7 {
            r -= 36;
        }
    }
    r
}
fn qr_data_cw(v: usize) -> usize {
    qr_raw(v) / 8 - QR_ECC[v] * QR_BLK[v]
}

/// Encode `text`; `Err("qr-too-long")` above 271 UTF-8 bytes.
pub fn qr(text: &str) -> Result<Qr, String> {
    let bytes = text.as_bytes();
    let mut ver = 1usize;
    while ver <= 10 && 4 + if ver < 10 { 8 } else { 16 } + 8 * bytes.len() > qr_data_cw(ver) * 8 {
        ver += 1;
    }
    if ver > 10 {
        return Err("qr-too-long".into());
    }
    let mut bits: Vec<u8> = Vec::new();
    let put = |bits: &mut Vec<u8>, val: u32, n: usize| {
        for i in (0..n).rev() {
            bits.push(((val >> i) & 1) as u8);
        }
    };
    put(&mut bits, 4, 4);
    put(&mut bits, bytes.len() as u32, if ver < 10 { 8 } else { 16 });
    for &b in bytes {
        put(&mut bits, b as u32, 8);
    }
    let cap_bits = qr_data_cw(ver) * 8;
    let n = 4.min(cap_bits - bits.len());
    put(&mut bits, 0, n);
    let n = (8 - bits.len() % 8) % 8;
    put(&mut bits, 0, n);
    let mut p = 0xecu32;
    while bits.len() < cap_bits {
        put(&mut bits, p, 8);
        p ^= 0xec ^ 0x11;
    }
    let data: Vec<u32> = bits.chunks(8).map(|c| c.iter().fold(0u32, |a, b| (a << 1) | *b as u32)).collect();

    // blocks + Reed-Solomon, interleaved
    let nb = QR_BLK[ver];
    let eb = QR_ECC[ver];
    let raw = qr_raw(ver) / 8;
    let n_short = nb - raw % nb;
    let short_len = raw / nb;
    let div = rs_divisor(eb);
    let mut blocks: Vec<Vec<u32>> = Vec::new();
    let mut k = 0;
    for i in 0..nb {
        let len = short_len - eb + if i < n_short { 0 } else { 1 };
        let mut dat: Vec<u32> = data[k..k + len].to_vec();
        k += len;
        let ecc = rs_remainder(&dat, &div);
        if i < n_short {
            dat.push(0);
        }
        dat.extend(ecc);
        blocks.push(dat);
    }
    let mut all: Vec<u32> = Vec::new();
    for i in 0..blocks[0].len() {
        for (j, b) in blocks.iter().enumerate() {
            if i != short_len - eb || j >= n_short {
                all.push(b[i]);
            }
        }
    }

    // matrix
    let size = ver * 4 + 17;
    let mut m = vec![vec![false; size]; size];
    let mut fnm = vec![vec![false; size]; size];
    fn set_f(m: &mut [Vec<bool>], f: &mut [Vec<bool>], x: i64, y: i64, d: bool) {
        m[y as usize][x as usize] = d;
        f[y as usize][x as usize] = true;
    }
    for i in 0..size as i64 {
        set_f(&mut m, &mut fnm, 6, i, i % 2 == 0);
        set_f(&mut m, &mut fnm, i, 6, i % 2 == 0);
    }
    let si = size as i64;
    for (cx, cy) in [(3i64, 3i64), (si - 4, 3), (3, si - 4)] {
        for dy in -4i64..=4 {
            for dx in -4i64..=4 {
                let d = dx.abs().max(dy.abs());
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && x < si && y >= 0 && y < si {
                    set_f(&mut m, &mut fnm, x, y, d != 2 && d != 4);
                }
            }
        }
    }
    if ver > 1 {
        let n = (ver / 7 + 2) as i64;
        let step = (((ver * 4 + 4) as i64 + (n * 2 - 2) - 1) / (n * 2 - 2)) * 2;
        let mut pos = vec![6i64];
        let mut p = si - 7;
        while (pos.len() as i64) < n {
            pos.insert(1, p);
            p -= step;
        }
        for (i, &cy) in pos.iter().enumerate() {
            for (j, &cx) in pos.iter().enumerate() {
                let (i, j) = (i as i64, j as i64);
                if (i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0) {
                    continue;
                }
                for dy in -2i64..=2 {
                    for dx in -2i64..=2 {
                        set_f(&mut m, &mut fnm, cx + dx, cy + dy, dx.abs().max(dy.abs()) != 1);
                    }
                }
            }
        }
    }
    let bit = |v: u32, i: usize| ((v >> i) & 1) != 0;
    let draw_format = |m: &mut Vec<Vec<bool>>, f: &mut Vec<Vec<bool>>, mask: u32| {
        let d = (1u32 << 3) | mask; // ECC L = 01
        let mut rem = d;
        for _ in 0..10 {
            rem = (rem << 1) ^ ((rem >> 9) * 0x537);
        }
        let b = ((d << 10) | rem) ^ 0x5412;
        for i in 0..=5usize {
            set_f(m, f, 8, i as i64, bit(b, i));
        }
        set_f(m, f, 8, 7, bit(b, 6));
        set_f(m, f, 8, 8, bit(b, 7));
        set_f(m, f, 7, 8, bit(b, 8));
        for i in 9..15usize {
            set_f(m, f, 14 - i as i64, 8, bit(b, i));
        }
        for i in 0..8usize {
            set_f(m, f, si - 1 - i as i64, 8, bit(b, i));
        }
        for i in 8..15usize {
            set_f(m, f, 8, si - 15 + i as i64, bit(b, i));
        }
        set_f(m, f, 8, si - 8, true);
    };
    draw_format(&mut m, &mut fnm, 0);
    if ver >= 7 {
        let mut rem = ver as u32;
        for _ in 0..12 {
            rem = (rem << 1) ^ ((rem >> 11) * 0x1f25);
        }
        let b = ((ver as u32) << 12) | rem;
        for i in 0..18usize {
            let a = si - 11 + (i % 3) as i64;
            let c = (i / 3) as i64;
            set_f(&mut m, &mut fnm, a, c, bit(b, i));
            set_f(&mut m, &mut fnm, c, a, bit(b, i));
        }
    }
    let mut i = 0usize;
    let mut right = si - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vert in 0..si {
            for j in 0..2 {
                let x = (right - j) as usize;
                let y = if ((right + 1) & 2) == 0 { si - 1 - vert } else { vert } as usize;
                if !fnm[y][x] && i < all.len() * 8 {
                    m[y][x] = bit(all[i >> 3], 7 - (i & 7));
                    i += 1;
                }
            }
        }
        right -= 2;
    }
    let mask_fn = |k: usize, x: usize, y: usize| -> bool {
        match k {
            0 => (x + y) % 2 == 0,
            1 => y % 2 == 0,
            2 => x % 3 == 0,
            3 => (x + y) % 3 == 0,
            4 => (x / 3 + y / 2) % 2 == 0,
            5 => (x * y) % 2 + (x * y) % 3 == 0,
            6 => ((x * y) % 2 + (x * y) % 3) % 2 == 0,
            _ => (((x + y) % 2) + ((x * y) % 3)) % 2 == 0,
        }
    };
    let apply_mask = |m: &mut Vec<Vec<bool>>, f: &Vec<Vec<bool>>, k: usize| {
        for y in 0..size {
            for x in 0..size {
                if !f[y][x] && mask_fn(k, x, y) {
                    m[y][x] = !m[y][x];
                }
            }
        }
    };
    let penalty = |m: &Vec<Vec<bool>>| -> i64 {
        let mut p = 0i64;
        let mut dark = 0i64;
        let mut lines: Vec<Vec<u8>> = Vec::new();
        for y in 0..size {
            lines.push(m[y].iter().map(|v| *v as u8).collect());
        }
        for x in 0..size {
            lines.push((0..size).map(|y| m[y][x] as u8).collect());
        }
        let pats: [[u8; 11]; 2] = [[1, 0, 1, 1, 1, 0, 1, 0, 0, 0, 0], [0, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1]];
        for l in &lines {
            let mut s = 0;
            while s < l.len() {
                let mut e = s;
                while e < l.len() && l[e] == l[s] {
                    e += 1;
                }
                if e - s >= 5 {
                    p += 3 + (e - s - 5) as i64;
                }
                s = e;
            }
            for pat in &pats {
                for w in l.windows(11) {
                    if w == pat {
                        p += 40;
                    }
                }
            }
        }
        for y in 0..size {
            for x in 0..size {
                if m[y][x] {
                    dark += 1;
                }
                if x < size - 1 && y < size - 1 && m[y][x] == m[y][x + 1] && m[y][x] == m[y + 1][x] && m[y][x] == m[y + 1][x + 1] {
                    p += 3;
                }
            }
        }
        let area = (size * size) as i64;
        let dev = (dark * 20 - area * 10).abs();
        p + ((dev + area - 1) / area - 1) * 10
    };
    let mut best = 0usize;
    let mut best_p = i64::MAX;
    for k in 0..8 {
        apply_mask(&mut m, &fnm, k);
        draw_format(&mut m, &mut fnm, k as u32);
        let p = penalty(&m);
        apply_mask(&mut m, &fnm, k);
        if p < best_p {
            best_p = p;
            best = k;
        }
    }
    apply_mask(&mut m, &fnm, best);
    draw_format(&mut m, &mut fnm, best as u32);
    Ok(Qr { size, version: ver, mask: best, modules: m.into_iter().flatten().collect() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_like_js_test() {
        let q = qr("https://example.com").unwrap();
        assert_eq!(q.size, q.version * 4 + 17);
        assert!(q.get(0, 0) && q.get(6, 6) && !q.get(7, 7) && q.get(q.size - 1, 0) && q.get(0, q.size - 1));
        assert_eq!(qr(&"x".repeat(271)).unwrap().version, 10);
        assert_eq!(qr(&"x".repeat(272)).unwrap_err(), "qr-too-long");
    }
    #[test]
    fn utf8_counts_bytes() {
        // 136 two-byte Cyrillic letters = 272 bytes: too long, 135 fit
        assert!(qr(&"я".repeat(136)).is_err());
        assert!(qr(&"я".repeat(135)).is_ok());
    }
    #[test]
    fn versions_grow_with_length() {
        assert_eq!(qr("a").unwrap().version, 1);
        assert_eq!(qr(&"a".repeat(17)).unwrap().version, 1);
        assert_eq!(qr(&"a".repeat(18)).unwrap().version, 2);
    }
    fn fnv(s: &str) -> u32 {
        s.bytes().fold(0x811c9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x01000193))
    }
    /// Module grids produced by the JS `qr()` (node, same inputs, see text_js_reference.json) must match bit for bit.
    #[test]
    fn matches_js_reference() {
        let r: serde_json::Value = serde_json::from_str(include_str!("text_js_reference.json")).unwrap();
        let cases = r["qr"].as_array().unwrap();
        assert!(cases.len() >= 8);
        for c in cases {
            let text = c["text"].as_str().unwrap();
            let q = qr(text).unwrap();
            assert_eq!(q.version as u64, c["version"].as_u64().unwrap(), "version of {text:?}");
            assert_eq!(q.mask as u64, c["mask"].as_u64().unwrap(), "mask of {text:?}");
            assert_eq!(fnv(&q.rows().join("
")) as u64, c["fnv"].as_u64().unwrap(), "modules of {text:?}");
            if let Some(rows) = c["rows"].as_array() {
                let want: Vec<&str> = rows.iter().map(|x| x.as_str().unwrap()).collect();
                assert_eq!(q.rows(), want);
            }
        }
    }
}
