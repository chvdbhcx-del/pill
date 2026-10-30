//! CSS value math: 2x3 matrices, transform function lists and their interpolation (function-wise when the lists match, through the
//! decomposed matrix otherwise), decomposed static transforms, colours. No allocation: lists are fixed arrays of at most 4 functions.

pub type Mat = [f32; 6]; // CSS matrix(a, b, c, d, e, f): x' = a x + c y + e, y' = b x + d y + f
pub const IDENT: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
/// decomposed 2D transform: [tx, ty, rotation deg, sx, sy, skew deg]
pub type Dec = [f32; 6];
pub const DEC_IDENT: Dec = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0];

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// a * b: b is applied first, then a (CSS / column-vector convention)
pub fn mul(a: Mat, b: Mat) -> Mat {
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}

pub fn translate(x: f32, y: f32) -> Mat {
    [1.0, 0.0, 0.0, 1.0, x, y]
}
pub fn scale(x: f32, y: f32) -> Mat {
    [x, 0.0, 0.0, y, 0.0, 0.0]
}
pub fn rotate(deg: f32) -> Mat {
    let (s, c) = deg.to_radians().sin_cos();
    [c, s, -s, c, 0.0, 0.0]
}
pub fn skew(ax: f32, ay: f32) -> Mat {
    [1.0, ay.to_radians().tan(), ax.to_radians().tan(), 1.0, 0.0, 0.0]
}

pub fn from_dec(d: Dec) -> Mat {
    if d == DEC_IDENT {
        return IDENT;
    }
    let (s, c) = d[2].to_radians().sin_cos();
    let k = d[5].to_radians().tan();
    [d[3] * c, d[3] * s, d[4] * (k * c - s), d[4] * (k * s + c), d[0], d[1]]
}

/// CSS "unmatrix" for 2D; the inverse of `from_dec`.
pub fn to_dec(m: Mat) -> Dec {
    let [mut a, mut b, mut c, mut d, e, f] = m;
    let mut sx = a.hypot(b);
    if sx != 0.0 {
        a /= sx;
        b /= sx;
    }
    let mut sk = a * c + b * d;
    c -= a * sk;
    d -= b * sk;
    let sy = c.hypot(d);
    if sy != 0.0 {
        c /= sy;
        d /= sy;
        sk /= sy;
    }
    if a * d < b * c {
        a = -a;
        b = -b;
        sk = -sk;
        sx = -sx;
    }
    [e, f, b.atan2(a).to_degrees(), sx, sy, sk.atan().to_degrees()]
}

pub fn lerp_dec(a: Dec, b: Dec, t: f32) -> Dec {
    let mut o = [0.0; 6];
    for i in 0..6 {
        o[i] = lerp(a[i], b[i], t);
    }
    o
}

// ---------------------------------------------------------------- transform function lists (keyframes)

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TfFn {
    /// translate: x px, x %, y px, y %
    T(f32, f32, f32, f32),
    S(f32, f32),
    R(f32),
    K(f32, f32),
    M(Mat),
}

impl TfFn {
    fn same_kind(&self, o: &TfFn) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(o)
    }
    fn identity_of(&self) -> TfFn {
        match self {
            TfFn::T(..) => TfFn::T(0.0, 0.0, 0.0, 0.0),
            TfFn::S(..) => TfFn::S(1.0, 1.0),
            TfFn::R(_) => TfFn::R(0.0),
            TfFn::K(..) => TfFn::K(0.0, 0.0),
            TfFn::M(_) => TfFn::M(IDENT),
        }
    }
    fn lerp(&self, o: &TfFn, t: f32) -> TfFn {
        match (self, o) {
            (TfFn::T(a, b, c, d), TfFn::T(e, f, g, h)) => TfFn::T(lerp(*a, *e, t), lerp(*b, *f, t), lerp(*c, *g, t), lerp(*d, *h, t)),
            (TfFn::S(a, b), TfFn::S(c, d)) => TfFn::S(lerp(*a, *c, t), lerp(*b, *d, t)),
            (TfFn::R(a), TfFn::R(b)) => TfFn::R(lerp(*a, *b, t)),
            (TfFn::K(a, b), TfFn::K(c, d)) => TfFn::K(lerp(*a, *c, t), lerp(*b, *d, t)),
            (TfFn::M(a), TfFn::M(b)) => TfFn::M(from_dec(lerp_dec(to_dec(*a), to_dec(*b), t))),
            _ => *self,
        }
    }
    fn to_mat(self, w: f32, h: f32) -> Mat {
        match self {
            TfFn::T(xp, xr, yp, yr) => translate(xp + xr * 0.01 * w, yp + yr * 0.01 * h),
            TfFn::S(x, y) => scale(x, y),
            TfFn::R(d) => rotate(d),
            TfFn::K(x, y) => skew(x, y),
            TfFn::M(m) => m,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TfList {
    pub n: u8,
    pub f: [TfFn; 4],
}

impl Default for TfList {
    fn default() -> Self {
        TfList { n: 0, f: [TfFn::R(0.0); 4] }
    }
}

impl TfList {
    pub fn from_slice(v: &[TfFn]) -> Option<TfList> {
        if v.len() > 4 {
            return None;
        }
        let mut l = TfList::default();
        l.f[..v.len()].copy_from_slice(v);
        l.n = v.len() as u8;
        Some(l)
    }
    pub fn items(&self) -> &[TfFn] {
        &self.f[..self.n as usize]
    }
    /// The static decomposed transform as a function list (so it can be an implicit keyframe).
    pub fn from_dec(d: Dec) -> TfList {
        let mut v = Vec::with_capacity(4);
        if d[0] != 0.0 || d[1] != 0.0 {
            v.push(TfFn::T(d[0], 0.0, d[1], 0.0));
        }
        if d[2] != 0.0 {
            v.push(TfFn::R(d[2]));
        }
        if d[5] != 0.0 {
            v.push(TfFn::K(d[5], 0.0));
        }
        if d[3] != 1.0 || d[4] != 1.0 {
            v.push(TfFn::S(d[3], d[4]));
        }
        TfList::from_slice(&v).unwrap_or_default()
    }
    pub fn to_mat(&self, w: f32, h: f32) -> Mat {
        let mut m = IDENT;
        for f in self.items() {
            m = mul(m, f.to_mat(w, h));
        }
        m
    }
    /// CSS: matching prefix of function kinds, the longer list's tail pairs with identities; any other mismatch interpolates the
    /// matrices (decomposed).
    pub fn lerp(&self, o: &TfList, t: f32, w: f32, h: f32) -> TfList {
        let (a, b) = (self.items(), o.items());
        let common = a.len().min(b.len());
        let prefix_ok = (0..common).all(|i| a[i].same_kind(&b[i]));
        let tail_ok = prefix_ok && (a.len() == b.len() || (a.len() < b.len() && true) || (a.len() > b.len() && true));
        if tail_ok {
            let mut out = TfList::default();
            let n = a.len().max(b.len());
            for i in 0..n {
                let (x, y) = match (a.get(i), b.get(i)) {
                    (Some(x), Some(y)) => (*x, *y),
                    (Some(x), None) => (*x, x.identity_of()),
                    (None, Some(y)) => (y.identity_of(), *y),
                    (None, None) => unreachable!(),
                };
                out.f[i] = x.lerp(&y, t);
            }
            out.n = n as u8;
            return out;
        }
        let ma = to_dec(self.to_mat(w, h));
        let mb = to_dec(o.to_mat(w, h));
        TfList { n: 1, f: [TfFn::M(from_dec(lerp_dec(ma, mb, t))), TfFn::R(0.0), TfFn::R(0.0), TfFn::R(0.0)] }
    }
}

// ---------------------------------------------------------------- colour

pub fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t), lerp(a[3], b[3], t)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn dec_roundtrip() {
        for d in [[3.0, -2.0, 25.0, 1.5, 0.8, 0.0], [0.0, 5.0, -170.0, 0.6, 0.6, 12.0], [1.0, 1.0, 0.0, -1.0, 1.0, 0.0]] {
            let m = from_dec(d);
            let d2 = to_dec(m);
            let m2 = from_dec(d2);
            for i in 0..6 {
                assert!(close(m[i], m2[i]), "{d:?} {m:?} {m2:?}");
            }
        }
    }

    #[test]
    fn matrix_product_order() {
        // translate(10,0) after scale(2): a point at x=1 goes to 2 then 12
        let m = mul(translate(10.0, 0.0), scale(2.0, 2.0));
        assert!(close(m[0] * 1.0 + m[4], 12.0));
    }

    #[test]
    fn list_lerp_matching() {
        let a = TfList::from_slice(&[TfFn::T(0.0, 0.0, 0.0, 4.0), TfFn::S(1.0, 1.0)]).unwrap();
        let b = TfList::from_slice(&[TfFn::T(0.0, 0.0, 0.0, -16.0), TfFn::S(0.5, 2.0)]).unwrap();
        let m = a.lerp(&b, 0.5, 48.0, 48.0);
        assert_eq!(m.items()[0], TfFn::T(0.0, 0.0, 0.0, -6.0));
        assert_eq!(m.items()[1], TfFn::S(0.75, 1.5));
        // percent translation resolves against the box height
        let mat = TfList::from_slice(&[TfFn::T(0.0, 0.0, 0.0, 50.0)]).unwrap().to_mat(10.0, 40.0);
        assert!(close(mat[5], 20.0));
    }

    #[test]
    fn list_lerp_pads_with_identity() {
        let a = TfList::from_slice(&[]).unwrap();
        let b = TfList::from_slice(&[TfFn::S(0.0, 0.0)]).unwrap();
        assert_eq!(a.lerp(&b, 0.5, 1.0, 1.0).items()[0], TfFn::S(0.5, 0.5));
    }

    #[test]
    fn list_lerp_mismatch_uses_matrices() {
        let a = TfList::from_slice(&[TfFn::R(0.0)]).unwrap();
        let b = TfList::from_slice(&[TfFn::S(2.0, 2.0)]).unwrap();
        let m = a.lerp(&b, 0.5, 1.0, 1.0).to_mat(1.0, 1.0);
        assert!(close(m[0], 1.5) && close(m[3], 1.5));
    }

    #[test]
    fn rotation_midpoint() {
        let a = TfList::from_slice(&[TfFn::R(-16.0)]).unwrap();
        let b = TfList::from_slice(&[TfFn::R(18.0)]).unwrap();
        let m = a.lerp(&b, 0.5, 1.0, 1.0);
        assert_eq!(m.items()[0], TfFn::R(1.0));
    }

    #[test]
    fn dec_to_list_matches_matrix() {
        let d = [2.0, 3.0, 30.0, 1.2, 0.9, 0.0];
        let a = TfList::from_dec(d).to_mat(1.0, 1.0);
        let b = from_dec(d);
        for i in 0..6 {
            assert!(close(a[i], b[i]));
        }
    }
}
