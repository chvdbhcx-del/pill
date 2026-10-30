//! Minimal SVG path-data parser: M L H V C S Q T A Z (absolute and relative), everything normalised to absolute M / L / C / Z.
//! Arcs become cubic Beziers (<= 90 degrees each) and quadratics become cubics, so two paths with the same command structure
//! interpolate number by number (the CSS `d` animation of the lens iris and the stickman limbs).
//! ponytail: `lerp` snaps for paths that do not share a command structure (the pets never animate such pairs); the lead may
//! replace this file with glasskit's `svgpath` once it lands.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    M(f32, f32),
    L(f32, f32),
    C(f32, f32, f32, f32, f32, f32),
    Z,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub segs: Vec<Seg>,
}

struct Lex<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lex<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b',' | b'\n' | b'\t' | b'\r') {
            self.i += 1;
        }
    }
    fn peek_num(&mut self) -> bool {
        self.ws();
        self.i < self.s.len() && matches!(self.s[self.i], b'0'..=b'9' | b'-' | b'+' | b'.')
    }
    fn num(&mut self) -> Option<f32> {
        self.ws();
        let st = self.i;
        if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
            self.i += 1;
        }
        let mut dot = false;
        while self.i < self.s.len() {
            match self.s[self.i] {
                b'0'..=b'9' => self.i += 1,
                b'.' if !dot => {
                    dot = true;
                    self.i += 1
                }
                b'e' | b'E' => {
                    self.i += 1;
                    if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
        std::str::from_utf8(&self.s[st..self.i]).ok()?.parse().ok()
    }
    /// arc flags are single characters and may be written without a separator ("a1 1 0 00.5.5")
    fn flag(&mut self) -> Option<bool> {
        self.ws();
        let c = *self.s.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

pub fn parse(d: &str) -> Option<Path> {
    let mut lx = Lex { s: d.as_bytes(), i: 0 };
    let mut out = Vec::new();
    let (mut cx, mut cy, mut sx, mut sy) = (0f32, 0f32, 0f32, 0f32);
    let (mut lcx, mut lcy) = (0f32, 0f32); // last cubic control point (for S)
    let (mut lqx, mut lqy) = (0f32, 0f32); // last quad control point (for T)
    let mut prev = b' ';
    let mut cmd = b' ';
    loop {
        lx.ws();
        if lx.i >= lx.s.len() {
            break;
        }
        let c = lx.s[lx.i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            lx.i += 1;
            if cmd == b'z' || cmd == b'Z' {
                out.push(Seg::Z);
                cx = sx;
                cy = sy;
                prev = b'Z';
                continue;
            }
        } else if !lx.peek_num() || cmd == b' ' || cmd == b'z' || cmd == b'Z' {
            return None;
        } else if cmd == b'M' {
            cmd = b'L';
        } else if cmd == b'm' {
            cmd = b'l';
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (lx.num()? + ox, lx.num()? + oy);
                out.push(Seg::M(x, y));
                cx = x;
                cy = y;
                sx = x;
                sy = y;
            }
            b'L' => {
                let (x, y) = (lx.num()? + ox, lx.num()? + oy);
                out.push(Seg::L(x, y));
                cx = x;
                cy = y;
            }
            b'H' => {
                let x = lx.num()? + ox;
                out.push(Seg::L(x, cy));
                cx = x;
            }
            b'V' => {
                let y = lx.num()? + oy;
                out.push(Seg::L(cx, y));
                cy = y;
            }
            b'C' => {
                let v = [lx.num()? + ox, lx.num()? + oy, lx.num()? + ox, lx.num()? + oy, lx.num()? + ox, lx.num()? + oy];
                out.push(Seg::C(v[0], v[1], v[2], v[3], v[4], v[5]));
                lcx = v[2];
                lcy = v[3];
                cx = v[4];
                cy = v[5];
            }
            b'S' => {
                let (x1, y1) = if matches!(prev, b'C' | b'S') { (2.0 * cx - lcx, 2.0 * cy - lcy) } else { (cx, cy) };
                let v = [lx.num()? + ox, lx.num()? + oy, lx.num()? + ox, lx.num()? + oy];
                out.push(Seg::C(x1, y1, v[0], v[1], v[2], v[3]));
                lcx = v[0];
                lcy = v[1];
                cx = v[2];
                cy = v[3];
            }
            b'Q' => {
                let v = [lx.num()? + ox, lx.num()? + oy, lx.num()? + ox, lx.num()? + oy];
                out.push(quad(cx, cy, v[0], v[1], v[2], v[3]));
                lqx = v[0];
                lqy = v[1];
                cx = v[2];
                cy = v[3];
            }
            b'T' => {
                let (qx, qy) = if matches!(prev, b'Q' | b'T') { (2.0 * cx - lqx, 2.0 * cy - lqy) } else { (cx, cy) };
                let (x, y) = (lx.num()? + ox, lx.num()? + oy);
                out.push(quad(cx, cy, qx, qy, x, y));
                lqx = qx;
                lqy = qy;
                cx = x;
                cy = y;
            }
            b'A' => {
                let (rx, ry, rot) = (lx.num()?, lx.num()?, lx.num()?);
                let (large, sweep) = (lx.flag()?, lx.flag()?);
                let (x, y) = (lx.num()? + ox, lx.num()? + oy);
                arc(&mut out, cx, cy, [rx, ry, rot], large, sweep, x, y);
                cx = x;
                cy = y;
            }
            _ => return None,
        }
        prev = cmd.to_ascii_uppercase();
    }
    Some(Path { segs: out })
}

fn quad(x0: f32, y0: f32, qx: f32, qy: f32, x: f32, y: f32) -> Seg {
    Seg::C(x0 + 2.0 / 3.0 * (qx - x0), y0 + 2.0 / 3.0 * (qy - y0), x + 2.0 / 3.0 * (qx - x), y + 2.0 / 3.0 * (qy - y), x, y)
}

/// SVG implementation notes F.6: endpoint -> centre parameterisation, then <= 90 degree cubic pieces.
#[allow(clippy::too_many_arguments)]
fn arc(out: &mut Vec<Seg>, x0: f32, y0: f32, r: [f32; 3], large: bool, sweep: bool, x: f32, y: f32) {
    let [rx, ry, rot_deg] = r;
    if (x0 == x && y0 == y) || rx == 0.0 || ry == 0.0 {
        out.push(Seg::L(x, y));
        return;
    }
    let (mut rx, mut ry) = (rx.abs() as f64, ry.abs() as f64);
    let (x0, y0, x, y) = (x0 as f64, y0 as f64, x as f64, y as f64);
    let phi = (rot_deg as f64).to_radians();
    let (cp, sp) = (phi.cos(), phi.sin());
    let (dx, dy) = ((x0 - x) / 2.0, (y0 - y) / 2.0);
    let (x1, y1) = (cp * dx + sp * dy, -sp * dx + cp * dy);
    let lam = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lam > 1.0 {
        let s = lam.sqrt();
        rx *= s;
        ry *= s;
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = if den == 0.0 { 0.0 } else { (num / den).max(0.0).sqrt() };
    if large == sweep {
        k = -k;
    }
    let (cxp, cyp) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (cp * cxp - sp * cyp + (x0 + x) / 2.0, sp * cxp + cp * cyp + (y0 + y) / 2.0);
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let th1 = ang(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut dth = ang((x1 - cxp) / rx, (y1 - cyp) / ry, (-x1 - cxp) / rx, (-y1 - cyp) / ry);
    if !sweep && dth > 0.0 {
        dth -= std::f64::consts::TAU;
    } else if sweep && dth < 0.0 {
        dth += std::f64::consts::TAU;
    }
    let n = (dth.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = dth / n as f64;
    let t = 4.0 / 3.0 * (step / 4.0).tan();
    let pt = |a: f64| (cx + rx * a.cos() * cp - ry * a.sin() * sp, cy + rx * a.cos() * sp + ry * a.sin() * cp);
    let dpt = |a: f64| (-rx * a.sin() * cp - ry * a.cos() * sp, -rx * a.sin() * sp + ry * a.cos() * cp);
    let mut a0 = th1;
    for i in 0..n {
        let a1 = a0 + step;
        let (p0, p1) = (pt(a0), pt(a1));
        let (d0, d1) = (dpt(a0), dpt(a1));
        let end = if i == n - 1 { (x, y) } else { p1 };
        out.push(Seg::C((p0.0 + t * d0.0) as f32, (p0.1 + t * d0.1) as f32, (p1.0 - t * d1.0) as f32, (p1.1 - t * d1.1) as f32, end.0 as f32, end.1 as f32));
        a0 = a1;
    }
}

/// Number-by-number interpolation; `None` when the two paths do not share a command structure.
pub fn lerp(a: &Path, b: &Path, t: f32) -> Option<Path> {
    let mut out = Path { segs: Vec::with_capacity(a.segs.len()) };
    lerp_into(a, b, t, &mut out).then_some(out)
}

/// Same as `lerp`, into a reusable buffer (no allocation once it has grown). Returns false (buffer untouched) on a structure mismatch.
pub fn lerp_into(a: &Path, b: &Path, t: f32, out: &mut Path) -> bool {
    if a.segs.len() != b.segs.len() {
        return false;
    }
    let m = |p: f32, q: f32| p + (q - p) * t;
    out.segs.clear();
    for (s, e) in a.segs.iter().zip(&b.segs) {
        out.segs.push(match (*s, *e) {
            (Seg::M(x, y), Seg::M(x2, y2)) => Seg::M(m(x, x2), m(y, y2)),
            (Seg::L(x, y), Seg::L(x2, y2)) => Seg::L(m(x, x2), m(y, y2)),
            (Seg::C(a1, a2, a3, a4, a5, a6), Seg::C(b1, b2, b3, b4, b5, b6)) => Seg::C(m(a1, b1), m(a2, b2), m(a3, b3), m(a4, b4), m(a5, b5), m(a6, b6)),
            (Seg::Z, Seg::Z) => Seg::Z,
            _ => {
                out.segs.clear();
                return false;
            }
        });
    }
    true
}

impl Path {
    /// Control-point bounds (an over-estimate for curves).
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut add = |x: f32, y: f32| {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        };
        for s in &self.segs {
            match *s {
                Seg::M(x, y) | Seg::L(x, y) => add(x, y),
                Seg::C(a, b, c, d, e, f) => {
                    add(a, b);
                    add(c, d);
                    add(e, f);
                }
                Seg::Z => {}
            }
        }
        (x0 <= x1).then_some([x0, y0, x1, y1])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_and_shorthand() {
        let p = parse("M10 10h5v5H0z").unwrap();
        assert_eq!(p.segs, vec![Seg::M(10.0, 10.0), Seg::L(15.0, 10.0), Seg::L(15.0, 15.0), Seg::L(0.0, 15.0), Seg::Z]);
        let p = parse("m1 1 2 2").unwrap(); // implicit lineto after moveto
        assert_eq!(p.segs, vec![Seg::M(1.0, 1.0), Seg::L(3.0, 3.0)]);
    }

    #[test]
    fn numbers_without_separators() {
        let p = parse("M0-4.6L1.1-1.1L4.6 0").unwrap();
        assert_eq!(p.segs[0], Seg::M(0.0, -4.6));
        assert_eq!(p.segs[1], Seg::L(1.1, -1.1));
        let q = parse("M.5.5l.5.5").unwrap();
        assert_eq!(q.segs[1], Seg::L(1.0, 1.0));
    }

    #[test]
    fn smooth_curves() {
        let p = parse("M0 0C1 1 2 1 3 0S5 -1 6 0").unwrap();
        assert_eq!(p.segs[2], Seg::C(4.0, -1.0, 5.0, -1.0, 6.0, 0.0));
        let q = parse("M0 0Q1 2 2 0T4 0").unwrap();
        assert_eq!(q.segs.len(), 3);
        assert!(matches!(q.segs[2], Seg::C(.., x, y) if x == 4.0 && y == 0.0));
    }

    #[test]
    fn arc_circle_stays_on_the_circle() {
        let p = parse("M 7.5 25 A 16.5 16.5 0 1 0 40.5 25 A 16.5 16.5 0 1 0 7.5 25 Z").unwrap();
        // 2 half circles of 2 pieces each, plus M and Z
        assert_eq!(p.segs.len(), 6);
        let mut cur = (7.5f32, 25.0f32);
        for s in &p.segs {
            if let Seg::C(.., x, y) = *s {
                let d = ((x - 24.0).powi(2) + (y - 25.0).powi(2)).sqrt();
                assert!((d - 16.5).abs() < 0.01, "end point off the circle: {d}");
                cur = (x, y);
            }
        }
        assert!((cur.0 - 7.5).abs() < 1e-3 && (cur.1 - 25.0).abs() < 1e-3);
    }

    #[test]
    fn arc_flags_without_separator() {
        assert_eq!(parse("M0 0a5 5 0 1010 0").unwrap(), parse("M0 0a5 5 0 1 0 10 0").unwrap());
    }

    #[test]
    fn lerp_same_structure() {
        let a = parse("M 20 29 L 17.5 38 L 17 46.5").unwrap();
        let b = parse("M 20 29 L 22.5 38 L 23 46.5").unwrap();
        assert_eq!(lerp(&a, &b, 0.5).unwrap().segs[1], Seg::L(20.0, 38.0));
        assert!(lerp(&a, &parse("M 0 0 L 1 1").unwrap(), 0.5).is_none());
        let mut buf = Path::default();
        assert!(lerp_into(&a, &b, 1.0, &mut buf));
        assert_eq!(buf, b);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("M 1").is_none());
        assert!(parse("X 1 2").is_none());
        assert!(parse("12 3").is_none());
    }
}
