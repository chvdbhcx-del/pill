//! The procedural characters of the capsule (the `character` setting): a tail curling out of the bottom-right corner, or white oval eyes inside the
//! capsule with two thin legs under its bottom edge. A line-by-line port of src/characters.js (poses, gaze, blink, steps): the same maths, the
//! same constants, drawn with Direct2D lines / ellipses (no geometry is created per frame). Layout (where the origin sits) belongs to the shell.
//!
//! * tail: origin = the root on the island edge, +x outward; `draw_tail(dc, x, y, scale)`
//! * eyes: origin = the middle between the two eyes; `draw_eyes(dc, cx, cy, scale)`; the shell aligns `cy` with the text line of the live row
//! * legs: origin = the middle of the island's bottom edge; `draw_legs(dc, x, y, scale)`
use glasskit::gfx::col;
use glasskit::Color;
use std::mem::ManuallyDrop;
use windows::core::Result;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

const TAU: f32 = std::f32::consts::TAU;
const INK: [f32; 3] = [0x17 as f32 / 255.0, 0x14 as f32 / 255.0, 0x1f as f32 / 255.0];

fn hex(h: u32) -> [f32; 3] {
    [((h >> 16) & 255) as f32 / 255.0, ((h >> 8) & 255) as f32 / 255.0, (h & 255) as f32 / 255.0]
}

// tip / feet colours per state (notes-B section 8)
fn tip_color(s: usize) -> [f32; 3] {
    hex([0xf5e9dc, 0xf08c78, 0xa99be8, 0x8fbf4a, 0xe8a040, 0x8fbf4a, 0x666666][s])
}
fn foot_color(s: usize) -> [f32; 3] {
    hex([0xe0522a, 0xf08c78, 0xa99be8, 0x8fbf4a, 0xe8a040, 0x8fbf4a, 0x7a7a7a][s])
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let x = ((x - a) / (b - a)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// tail pose of characters.js `tailPose`: (length, width, tip radius) and the heading function in degrees (0 = right, + = down), u = 0 root .. 1 tip
pub fn tail_pose(s: usize, t: f32, rt: f32, e: f32) -> (f32, f32, f32, impl Fn(f32) -> f32) {
    let arc = |h0: f32, h1: f32, u: f32| h0 + (h1 - h0) * (u * (2.0 - u));
    let (l, w, r) = match s {
        1 => (36.0, 4.4, 3.6),
        2 => (34.0, 4.2, 3.5),
        3 => (36.0, 4.4, 3.6),
        4 => (30.0, 7.0, 5.6),
        5 => (38.0, 2.8, 3.0),
        6 => (34.0, 4.2, 3.4),
        _ => (38.0, 4.4, 3.6),
    };
    let f = move |u: f32| -> f32 {
        match s {
            1 => {
                let a = 0.55 + 0.45 * e;
                let ph = TAU * t / 0.7;
                arc(8.0, 78.0, u) + 26.0 * a * (ph - 1.8 * u).sin() * (0.2 + u)
            }
            2 => 10.0 + 150.0 * smoothstep(0.0, 0.5, u) - 250.0 * smoothstep(0.38, 1.0, u) - 10.0 * (t * 0.9 - 3.0 * u).sin() * u,
            3 => {
                let k = (-rt / 0.9).exp();
                arc(16.0, 84.0, u) + 20.0 * k * (rt * 10.0 - u).sin() * u
            }
            4 => arc(14.0, 80.0, u) + 2.5 * (t * 24.0).sin() * u,
            5 => arc(18.0, 82.0, u) + 4.0 * (t * 2.2 - 2.0 * u).sin() * u,
            6 => arc(38.0, 100.0, u) + 3.0 * (t * 0.8).sin() * u,
            _ => arc(14.0, 88.0, u) + 5.0 * (t * 1.3 - 2.4 * u).sin() * (0.3 + u),
        }
    };
    (l, w, r, f)
}

/// the 36-segment spine of the tail (local px at 100 %): returns the tip
pub fn tail_spine(s: usize, t: f32, rt: f32, e: f32, out: &mut [(f32, f32); 37]) -> (f32, f32, f32, f32) {
    let (l, w, r, f) = tail_pose(s, t, rt, e);
    let n = 36;
    let seg = l / n as f32;
    let (mut x, mut y) = (0.0f32, 0.0f32);
    out[0] = (0.0, 0.0);
    for i in 0..n {
        let th = f((i as f32 + 0.5) / n as f32).to_radians();
        x += th.cos() * seg;
        y += th.sin() * seg;
        out[i + 1] = (x, y);
    }
    (x, y, w, r)
}

#[derive(Clone, Copy, Debug, Default)]
struct Beat {
    n: u32,
    t0: f64,
    last: f64,
}

pub struct Characters {
    pub state: usize,
    t: f32,
    rt: f32,
    reduced: bool,
    tint: Option<Color>,
    /// eye colour: white on the black island, dark on the light one
    eye_col: [f32; 3],
    look: [f32; 2],
    look_t: [f32; 2],
    // audio
    pk: f32,
    en: f32,
    es: f32,
    prev_en: f32,
    last_band: f64,
    beat: Beat,
    now: f64,
    // legs stepping
    step_leg: usize,
    step_t0: f32,
    last_n: u32,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
}

impl Characters {
    pub fn new(dc: &ID2D1DeviceContext) -> Result<Characters> {
        unsafe {
            let f: ID2D1Factory = dc.GetFactory()?;
            let sp = D2D1_STROKE_STYLE_PROPERTIES {
                startCap: D2D1_CAP_STYLE_ROUND,
                endCap: D2D1_CAP_STYLE_ROUND,
                dashCap: D2D1_CAP_STYLE_ROUND,
                lineJoin: D2D1_LINE_JOIN_ROUND,
                miterLimit: 4.0,
                dashStyle: D2D1_DASH_STYLE_SOLID,
                dashOffset: 0.0,
            };
            Ok(Characters {
                state: 0,
                t: 0.0,
                rt: 99.0,
                reduced: false,
                tint: None,
                eye_col: hex(0xf4f1ea),
                look: [0.0; 2],
                look_t: [0.0; 2],
                pk: 0.2,
                en: 0.0,
                es: 0.0,
                prev_en: 0.0,
                last_band: -1e9,
                beat: Beat { n: 0, t0: 0.0, last: -1e9 },
                now: 0.0,
                step_leg: 0,
                step_t0: -9.0,
                last_n: 0,
                brush: dc.CreateSolidColorBrush(&col(1.0, 1.0, 1.0, 1.0), None)?,
                round: f.CreateStrokeStyle(&sp, None)?,
            })
        }
    }

    /// `name` = calm | music | thinking | done | attention | file | sleep (or "ai"); `colour` = the reaction's own colour, else the state's palette
    pub fn set_state(&mut self, name: &str, colour: Option<Color>) {
        self.state = match name {
            "ai" => 2,
            n => super::engine::state_index(n),
        };
        self.tint = colour;
        self.rt = 0.0;
    }

    pub fn set_reduced(&mut self, on: bool) {
        self.reduced = on;
    }

    /// dark eyes on the light island theme
    pub fn set_light_island(&mut self, light: bool) {
        self.eye_col = if light { hex(0x1c1c1e) } else { hex(0xf4f1ea) };
    }

    /// Vector from the middle of the eyes to the cursor, logical px (any distance); None = no gaze source. look = direction * |d| / (|d| + 60).
    pub fn set_gaze(&mut self, d: Option<(f32, f32)>) {
        self.look_t = match d {
            Some((dx, dy)) if self.state != 6 && !self.reduced => {
                let n = 1.0 / (dx.hypot(dy) + 60.0);
                [dx * n, dy * n]
            }
            _ => [0.0, 0.0],
        };
    }

    /// Average of the first eight audio bands (0..1) as delivered by `audio:bands`; drives the tail beat, the legs' steps (with `now_ms`).
    pub fn feed_audio(&mut self, avg: f32, now_ms: f64) {
        let dt = if self.last_band > -1e8 { (now_ms - self.last_band).clamp(1.0, 200.0) } else { 33.0 };
        self.last_band = now_ms;
        self.pk = avg.max(self.pk * 0.995f32.powf(dt as f32 / 33.0)).max(0.05);
        self.en = (avg / self.pk).min(1.0);
        self.beat.last = now_ms;
        if self.en > 0.72 && self.prev_en <= 0.72 && now_ms - self.beat.t0 > 320.0 {
            self.beat.n += 1;
            self.beat.t0 = now_ms;
        }
        self.prev_en = self.en;
    }

    /// Fixed clock for tools and tests: absolute time `t` and seconds since the state was set `rt` (seconds), gaze set directly.
    pub fn set_clock(&mut self, t: f32, rt: f32, look: [f32; 2]) {
        self.t = t;
        self.rt = rt;
        self.look = look;
        self.look_t = look;
    }

    pub fn tick(&mut self, dt_ms: f32) {
        let dt = dt_ms.max(0.0) / 1000.0;
        self.now += dt_ms.max(0.0) as f64;
        if self.reduced {
            self.t = 0.0;
            self.rt = 99.0;
            self.look = [0.0; 2];
            return;
        }
        self.t += dt;
        self.rt += dt;
        let fresh = self.now - self.beat.last < 900.0;
        if fresh {
            self.es += (self.en - self.es) * (dt * 12.0).min(1.0);
        }
        let k = 1.0 - (-dt * 14.0).exp();
        for i in 0..2 {
            self.look[i] += (self.look_t[i] - self.look[i]) * k;
        }
    }

    fn energy(&self) -> Option<f32> {
        (self.now - self.beat.last < 900.0 && !self.reduced).then_some(self.es)
    }

    fn color(&self, base: [f32; 3]) -> [f32; 3] {
        match self.tint {
            Some(c) => [c.r, c.g, c.b],
            None => base,
        }
    }

    fn set_brush(&self, c: [f32; 3], a: f32) {
        unsafe { self.brush.SetColor(&col(c[0], c[1], c[2], a)) };
    }

    fn line(&self, dc: &ID2D1DeviceContext, a: (f32, f32), b: (f32, f32), w: f32) {
        unsafe { dc.DrawLine(D2D_POINT_2F { x: a.0, y: a.1 }, D2D_POINT_2F { x: b.0, y: b.1 }, &self.brush, w, &self.round) };
    }

    fn polyline(&self, dc: &ID2D1DeviceContext, p: &[(f32, f32)], w: f32) {
        for s in p.windows(2) {
            self.line(dc, s[0], s[1], w);
        }
    }

    fn push_alpha(&self, dc: &ID2D1DeviceContext, a: f32) {
        let lp = D2D1_LAYER_PARAMETERS1 {
            contentBounds: D2D_RECT_F { left: -1.0e5, top: -1.0e5, right: 1.0e5, bottom: 1.0e5 },
            geometricMask: ManuallyDrop::new(None),
            maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            maskTransform: Matrix3x2::identity(),
            opacity: a,
            opacityBrush: ManuallyDrop::new(None),
            layerOptions: D2D1_LAYER_OPTIONS1_NONE,
        };
        unsafe { dc.PushLayer(&lp, None::<&ID2D1Layer>) };
    }

    fn place(&self, dc: &ID2D1DeviceContext, x: f32, y: f32, scale: f32) -> Matrix3x2 {
        let mut base = Matrix3x2::identity();
        unsafe {
            dc.GetTransform(&mut base);
            // local (x, y, scale) first, then the caller's transform: row-vector product local * base
            let local = Matrix3x2 { M11: scale, M12: 0.0, M21: 0.0, M22: scale, M31: x, M32: y };
            dc.SetTransform(&mul3(&local, &base));
        }
        base
    }

    fn z_glyph(&self, dc: &ID2D1DeviceContext, x: f32, y: f32, s: f32, a: f32, c: [f32; 3]) {
        // "M-2 -2.4h4l-4 4.8h4" scaled by s about (x, y)
        let p = [(-2.0, -2.4), (2.0, -2.4), (-2.0, 2.4), (2.0, 2.4)];
        self.set_brush(c, a);
        let q: Vec<(f32, f32)> = p.iter().map(|(px, py)| (x + px * s, y + py * s)).collect();
        self.polyline(dc, &q, 1.1 * s);
    }

    // ---------------------------------------------------------------- tail
    pub fn draw_tail(&self, dc: &ID2D1DeviceContext, ox: f32, oy: f32, scale: f32) {
        let mut sp = [(0.0f32, 0.0f32); 37];
        let e = self.energy().unwrap_or(0.75);
        let (tx, ty, w, r) = tail_spine(self.state, self.t, self.rt, e, &mut sp);
        let base = self.place(dc, ox, oy, scale);
        unsafe {
            // dark rim at 50 %: a layer so overlapping round segments do not double up
            self.push_alpha(dc, 0.5);
            self.set_brush(INK, 1.0);
            self.polyline(dc, &sp, w + 1.8);
            dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: tx, y: ty }, radiusX: r + 0.9, radiusY: r + 0.9 }, &self.brush);
            dc.PopLayer();
            let body = if self.state == 4 { hex(0xf2b13a) } else { hex(0xf0a050) };
            self.set_brush(body, 1.0);
            self.polyline(dc, &sp, w);
            self.set_brush(self.color(tip_color(self.state)), 1.0);
            dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: tx, y: ty }, radiusX: r, radiusY: r }, &self.brush);
            if self.state == 6 {
                let q = (self.t % 2.5) / 2.5;
                self.z_glyph(dc, tx + 6.0, ty - 5.0 - 7.0 * q, 0.9, (std::f32::consts::PI * q).sin() * 0.9, hex(0x9a9aa6));
            }
            dc.SetTransform(&base);
        }
    }

    // ---------------------------------------------------------------- legs
    fn quad(a: (f32, f32), c: (f32, f32), b: (f32, f32), out: &mut [(f32, f32); 13]) {
        for (i, o) in out.iter_mut().enumerate() {
            let t = i as f32 / 12.0;
            let u = 1.0 - t;
            *o = (u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1);
        }
    }

    /// The two legs stand on the middle of the island's bottom edge. `&mut`: the music steps keep a little state (which foot, when).
    pub fn draw_legs(&mut self, dc: &ID2D1DeviceContext, ox: f32, oy: f32, scale: f32) {
        let (s, t, rt) = (self.state, self.t, self.rt);
        let mut lift = [0.0f32; 2];
        let (mut len, mut sway, mut bend, mut tilt) = (15.0f32, 0.0f32, 0.0f32, 0.0f32);
        match s {
            1 => {
                // one foot lifts ~6 px and the leg bends; the legs alternate on every detected beat, else every 0.5 s
                let fresh = self.now - self.beat.last < 900.0;
                if t < self.step_t0 {
                    self.step_t0 = t;
                }
                if fresh {
                    if self.beat.n != self.last_n {
                        self.last_n = self.beat.n;
                        self.step_leg ^= 1;
                        self.step_t0 = t;
                    }
                } else if t - self.step_t0 > 0.5 {
                    self.step_leg ^= 1;
                    self.step_t0 = t;
                }
                lift[self.step_leg] = 6.0 * (std::f32::consts::PI * ((t - self.step_t0) / 0.42).min(1.0)).sin();
            }
            2 => sway = 1.2 * (t * 1.5).sin(),
            3 => {
                let k = (-rt / 1.2).exp();
                bend = 3.2 * k * (1.0 + 0.3 * (rt * 11.0).cos());
                tilt = 16.0 * k;
            }
            4 => len = 16.5,
            _ => {}
        }
        let c = self.color(foot_color(s));
        let base = self.place(dc, ox, oy, scale);
        let mut pts = [(0.0f32, 0.0f32); 13];
        for (k, i) in [-1.0f32, 1.0].into_iter().enumerate() {
            let hx = i * 6.5 + sway * 0.3;
            let ax = i * 6.5 + sway + i * bend * 0.5;
            let ay = len - lift[k] - bend * 0.4;
            let kx = i * 6.5 + sway * 0.6 + i * (bend + lift[k] * 0.55);
            let ky = len * 0.5 - lift[k] * 0.4;
            Self::quad((hx, -0.4), (kx, ky), (ax, ay), &mut pts);
            unsafe {
                self.push_alpha(dc, 0.38);
                self.set_brush(hex(0x6a6a74), 1.0);
                self.polyline(dc, &pts, 3.4);
                dc.PopLayer();
                self.set_brush(hex(0x050505), 1.0);
                self.polyline(dc, &pts, 2.2);
                // the foot: an ellipse tilted about its centre
                let (fx, fy) = (ax + i * 0.9, ay + 1.4);
                let mut cur = Matrix3x2::identity();
                dc.GetTransform(&mut cur);
                let (sn, cs) = (tilt * i).to_radians().sin_cos();
                let rot = Matrix3x2 { M11: cs, M12: sn, M21: -sn, M22: cs, M31: fx - fx * cs + fy * sn, M32: fy - fx * sn - fy * cs };
                dc.SetTransform(&mul3(&rot, &cur));
                let e = D2D1_ELLIPSE { point: D2D_POINT_2F { x: fx, y: fy }, radiusX: 3.8, radiusY: 2.1 };
                self.set_brush(c, 1.0);
                dc.FillEllipse(&e, &self.brush);
                self.set_brush([0.04, 0.04, 0.047], 0.55);
                dc.DrawEllipse(&e, &self.brush, 0.8, None::<&ID2D1StrokeStyle>);
                dc.SetTransform(&cur);
            }
        }
        unsafe { dc.SetTransform(&base) };
    }

    // ---------------------------------------------------------------- eyes
    pub fn draw_eyes(&self, dc: &ID2D1DeviceContext, cx: f32, cy: f32, scale: f32) {
        let s = self.state;
        let kind = if s == 6 {
            2
        } else if s == 1 || s == 3 || s == 5 {
            1
        } else {
            0
        }; // 0 open, 1 happy, 2 shut
        let p = self.t % 4.6;
        let blink = if s == 6 || p > 0.14 { 1.0 } else { 1.0 - 0.88 * (std::f32::consts::PI * p / 0.14).sin() }; // a quick blink every ~4.6 s
        let rx = if s == 4 { 3.6 } else { 3.0 };
        let ry = (if s == 4 {
            7.2
        } else if s == 2 {
            5.4
        } else {
            6.0
        }) * blink;
        let (ox, oy) = if s == 2 { (0.8, -1.0) } else { (0.0, 0.0) };
        let (shx, shy) = if s == 6 { (0.0, 0.0) } else { (self.look[0] * 2.4 + ox, self.look[1] * 1.8 + oy) }; // pupil-less ovals shift toward the cursor
        let base = self.place(dc, cx + shx * scale, cy + shy * scale, scale);
        self.set_brush(self.eye_col, 1.0);
        unsafe {
            for i in [-1.0f32, 1.0] {
                let ex = i * 6.8;
                match kind {
                    0 => dc.FillEllipse(&D2D1_ELLIPSE { point: D2D_POINT_2F { x: ex, y: 0.0 }, radiusX: rx, radiusY: ry }, &self.brush),
                    1 => self.arc_path(dc, ex - 3.6, 2.4, ex, -4.8, ex + 3.6, 2.4),
                    _ => self.arc_path(dc, ex - 3.6, -1.8, ex, 4.2, ex + 3.6, -1.8),
                }
            }
            if s == 6 {
                let q = (self.t % 2.5) / 2.5;
                // the z is not shifted by the gaze (the eyes group is not translated while asleep)
                self.z_glyph(dc, 6.8 + 7.0, -4.0 - 6.0 * q, 0.9, (std::f32::consts::PI * q).sin() * 0.9, hex(0x9a9aa6));
            }
            dc.SetTransform(&base);
        }
    }

    /// stroke of "M a Q c b" (the happy / shut eye arcs), width 2, round
    fn arc_path(&self, dc: &ID2D1DeviceContext, ax: f32, ay: f32, qx: f32, qy: f32, bx: f32, by: f32) {
        let mut pts = [(0.0f32, 0.0f32); 13];
        Self::quad((ax, ay), (qx, qy), (bx, by), &mut pts);
        self.polyline(dc, &pts, 2.0);
    }
}

/// row-vector product a * b (a first, then b): the Direct2D convention
fn mul3(a: &Matrix3x2, b: &Matrix3x2) -> Matrix3x2 {
    Matrix3x2 {
        M11: a.M11 * b.M11 + a.M12 * b.M21,
        M12: a.M11 * b.M12 + a.M12 * b.M22,
        M21: a.M21 * b.M11 + a.M22 * b.M21,
        M22: a.M21 * b.M12 + a.M22 * b.M22,
        M31: a.M31 * b.M11 + a.M32 * b.M21 + b.M31,
        M32: a.M31 * b.M12 + a.M32 * b.M22 + b.M32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spine_starts_at_the_root_and_hangs_down() {
        let mut p = [(0.0, 0.0); 37];
        let (x, y, w, r) = tail_spine(0, 0.0, 99.0, 0.75, &mut p);
        assert_eq!(p[0], (0.0, 0.0));
        assert!(x > 10.0 && y > 15.0, "a calm tail leaves outward and hangs down: tip ({x}, {y})");
        assert!((w - 4.4).abs() < 1e-6 && (r - 3.6).abs() < 1e-6);
        // 36 segments of length L / 36
        let l = ((p[1].0 - p[0].0).powi(2) + (p[1].1 - p[0].1).powi(2)).sqrt();
        assert!((l - 38.0 / 36.0).abs() < 1e-4);
    }

    #[test]
    fn attention_tail_is_thick_and_short() {
        let (l, w, r, _) = tail_pose(4, 0.0, 99.0, 0.5);
        assert_eq!((l, w, r), (30.0, 7.0, 5.6));
    }

    #[test]
    fn thinking_tail_loops_back() {
        // the question-mark loop: the heading climbs past 90 degrees and then falls back (js: 150 smoothstep - 250 smoothstep)
        let (_, _, _, f) = tail_pose(2, 0.0, 99.0, 0.5);
        assert!(f(0.4) > 90.0 && f(1.0) < -50.0, "{} {}", f(0.4), f(1.0));
    }

    #[test]
    fn done_wag_settles() {
        let (_, _, _, a) = tail_pose(3, 0.0, 0.0, 0.5);
        let (_, _, _, b) = tail_pose(3, 0.0, 10.0, 0.5);
        let rest = |f: &dyn Fn(f32) -> f32| (f(1.0) - (16.0 + 68.0)).abs();
        assert!(rest(&b) < 0.01, "after 10 s the tail rests on its arc");
        assert!(rest(&a) > 0.0 || a(0.5) != b(0.5));
    }

    #[test]
    fn matrix_product_matches_direct2d_order() {
        let t = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: 1.0, M31: 5.0, M32: 0.0 };
        let s = Matrix3x2 { M11: 2.0, M12: 0.0, M21: 0.0, M22: 2.0, M31: 0.0, M32: 0.0 };
        let m = mul3(&t, &s); // translate by 5, then scale by 2
        assert_eq!((m.M11, m.M31), (2.0, 10.0));
    }

    #[test]
    fn palette_has_a_colour_per_state() {
        for s in 0..7 {
            let _ = (tip_color(s), foot_color(s));
        }
    }
}
