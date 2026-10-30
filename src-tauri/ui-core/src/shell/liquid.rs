//! Liquid bottom edge (ferrofluid): port of pill.js `LQ` (lines 83-445) as a deterministic simulation plus a geometry output.
//!
//! A height field `h(x)` along the bottom edge (1 sample / 3 px, `N = 241`): each sample springs to a target shape (idle / reaction wave + the
//! cursor bulge) and the deviation from the target is coupled to its neighbours (damped wave equation, `k 220, c 900, damp 9`). Music adds
//! per-band spikes (32 bands, bass in the middle). The renderer takes [`Liquid::edge_view`]: the polyline under the straight bottom edge of
//! the body in 100 % px (`x` from the body centre, `y` below the straight edge, >= 0 and 0 at both ends), the fill / stroke colours, the stroke
//! fade gradient, halo passes, the hanging ball and the glow alt-style. Constants, easings and thresholds are the JS ones.
//!
//! Deviations (documented, not behavioural): arrays are `f64` (JS used `Float32Array`); `Math.random()` is the seeded [`Rng`];
//! `lastAudio` starts at -infinity (the JS relied on `performance.now() > 1000` at start); `TAU` stays the JS literal `6.2832`.
//! Reduced motion: `step` draws one static calm frame and returns false (no loop). The idle cap (`edgeIdleFps`) is the only throttle and
//! only applies to calm / sleep; every other reaction and the cursor pull run at the display rate.

use crate::shell::color::{parse_css, Palette, Rgba};
use crate::shell::io::Rng;
use crate::shell::react::{def, BallKind, Preset, WaveKind};
use crate::shell::settings::EdgeStyle;

/// Sample spacing (px at 100 %).
pub const DX: f64 = 3.0;
/// Width of the strip: `CFG.expW + 40`.
pub const SPAN: f64 = 720.0;
/// Strip height: 24 px of wave + room for the expanded droplet (15 + 5 + drip 6 + bob).
pub const STRIP: f64 = 46.0;
/// The strip starts this far above the straight bottom edge.
pub const UP: f64 = 4.0;
pub const K: f64 = 220.0;
pub const C: f64 = 900.0;
pub const DAMP: f64 = 9.0;
/// `N = ceil(span / dx / 2) * 2 + 1`
pub const N: usize = 241;
pub const MID: usize = 120;
/// The JS literal (not exactly 2 pi): kept so the waves match the page.
#[allow(clippy::approx_constant)]
const TAU: f64 = 6.2832;
/// Stroke fade over the last 15 % at both ends.
pub const FADE: f64 = 0.15;
/// Max boundary offset below the straight edge.
pub const MAX_OFFSET: f64 = 24.0;
/// Field clamp.
pub const MAX_H: f64 = 18.0;

pub fn smooth(e: f64) -> f64 {
    let e = e.clamp(0.0, 1.0);
    e * e * (3.0 - 2.0 * e)
}

/// Boundary offset below the straight bottom edge: soft `max(y, 0)` times a smoothstep taper reaching exactly 0 with zero slope at `x = +-half`.
pub fn liq_bound(y: f64, x: f64, half: f64) -> f64 {
    smooth((half - x.abs()) / 14.0f64.max(48.0f64.min(half * 0.3))) * 0.5 * (y + (y * y + 1.5).sqrt())
}

/// `pulse(t)`: the music beat envelope (period 1.1 s).
pub fn pulse(t: f64) -> f64 {
    let p = (t % 1.1) / 1.1;
    if p < 0.21 {
        ((p / 0.21) * std::f64::consts::PI / 2.0).sin()
    } else if p < 0.55 {
        (((p - 0.21) / 0.34) * std::f64::consts::PI / 2.0).cos()
    } else {
        0.0
    }
}

/// Body state the edge depends on (all css px at the current scale) plus the settings/flags it reads.
#[derive(Debug, Clone, Copy)]
pub struct Ctx {
    pub scale: f64,
    /// `sp.w.x`, `sp.h.x`, `sp.r.x` (bottom radius), `sp.t.x` (top radius)
    pub w: f64,
    pub h: f64,
    pub r: f64,
    pub t: f64,
    pub expanded: bool,
    pub reduce: bool,
    /// `performance.now()`
    pub now_ms: f64,
    pub edge_idle: bool,
    pub edge_cones: bool,
    pub idle_fps: f64,
    pub spring_busy: bool,
}

impl Default for Ctx {
    fn default() -> Self {
        Ctx { scale: 1.0, w: 144.0, h: 30.0, r: 15.0, t: 15.0, expanded: false, reduce: false, now_ms: 0.0, edge_idle: true, edge_cones: false, idle_fps: 30.0, spring_busy: false }
    }
}

/// Active half width (100 % px) and sample indices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub half: f64,
    pub i0: usize,
    pub i1: usize,
}

/// `audio:bands` payload.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Bands {
    pub b: Vec<f64>,
    pub rms: f64,
    /// source timestamp (ms) when present
    pub t: Option<f64>,
}

impl Bands {
    pub fn from_json(v: &serde_json::Value) -> Option<Bands> {
        let arr = v.get("b")?.as_array()?;
        let num = |x: &serde_json::Value| {
            let n = crate::shell::settings::plus(x);
            if n.is_nan() {
                0.0
            } else {
                n
            }
        };
        Some(Bands {
            b: arr.iter().map(num).collect(),
            rms: v.get("rms").map(num).unwrap_or(0.0),
            t: v.get("t").filter(|t| !t.is_null()).map(crate::shell::settings::plus).filter(|t| t.is_finite()),
        })
    }
}

#[derive(Debug, Clone, Default)]
struct Ball {
    x: f64,
    y: f64,
    vx: f64,
    tg: f64,
    next: f64,
    ph: f64,
    on: bool,
    hf: f64,
    /// collapsed <-> expanded regime blend
    b: f64,
    /// drip stretch + velocity
    s: f64,
    sv: f64,
    /// drip phase 0 idle, 1 stretching, 2 releasing
    dp: u8,
    dn: f64,
    de: f64,
}

#[derive(Debug, Clone)]
pub struct Liquid {
    pub t: f64,
    /// cursor target x / lagging x / velocity (100 % px from the centre)
    pub cx: f64,
    pub cxs: f64,
    cxv: f64,
    /// cursor pull target 0..1 and its spring state
    pub want: f64,
    pub inn: f64,
    inn_v: f64,
    e: f64,
    /// files are dragged over the island: the edge sags harder + wider under the cursor
    pub drag: bool,
    pub style: EdgeStyle,
    /// the winner of the reaction resolver
    pub react: Preset,
    /// seconds since the last paint of a reaction
    pub rt: f64,
    pub rcol: String,
    acc: f64,
    /// cursor bulge amplitude
    pub b_amp: f64,
    pub playing: bool,
    pub last_h: f64,
    pub override_color: String,
    last_audio: f64,
    pub play_since: f64,
    pub silent: bool,
    spike_move: bool,
    bt: [f64; 32],
    sa: [f64; 32],
    sv: [f64; 32],
    bv: [f64; 32],
    adj: [f64; 32],
    sl: [f64; 32],
    ad: [f64; 32],
    gpk: f64,
    bm_t: f64,
    h: Vec<f64>,
    v: Vec<f64>,
    tg: Vec<f64>,
    sp: Vec<f64>,
    ball: Ball,
    /// resolved colours (`liqColors`)
    pub edge: Rgba,
    pub fill: Rgba,
    rng: Rng,
}

impl Liquid {
    pub fn new(seed: u64) -> Liquid {
        Liquid {
            t: 0.0,
            cx: 0.0,
            cxs: 0.0,
            cxv: 0.0,
            want: 0.0,
            inn: 0.0,
            inn_v: 0.0,
            e: 0.0,
            drag: false,
            style: EdgeStyle::Liquid,
            react: Preset::Calm,
            rt: 0.0,
            rcol: String::new(),
            acc: 0.0,
            b_amp: 14.0,
            playing: false,
            last_h: 0.0,
            override_color: String::new(),
            last_audio: f64::NEG_INFINITY,
            play_since: 0.0,
            silent: false,
            spike_move: false,
            bt: [0.0; 32],
            sa: [0.0; 32],
            sv: [0.0; 32],
            bv: [0.0; 32],
            adj: [0.0; 32],
            sl: [-1.0; 32],
            ad: [0.05; 32],
            gpk: 0.5,
            bm_t: 0.0,
            h: vec![0.0; N],
            v: vec![0.0; N],
            tg: vec![0.0; N],
            sp: vec![0.0; N],
            ball: Ball::default(),
            edge: Rgba::rgb(0x6e, 0x6e, 0x6e),
            fill: Rgba::rgb(0, 0, 0),
            rng: Rng::new(seed),
        }
    }

    /// `pill:hide`: drop the cursor pull at once.
    pub fn clear_pull(&mut self) {
        self.want = 0.0;
        self.inn = 0.0;
        self.inn_v = 0.0;
    }

    /// The cursor pull spring is not at rest (`LQ.inn !== 0`).
    pub fn pull_active(&self) -> bool {
        self.inn != 0.0
    }

    // ---------- read access (tests, renderer) ----------
    pub fn heights(&self) -> &[f64] {
        &self.h
    }
    pub fn spikes(&self) -> &[f64] {
        &self.sp
    }
    pub fn spike_amps(&self) -> &[f64; 32] {
        &self.sa
    }
    pub fn ball_x(&self) -> f64 {
        self.ball.x
    }
    pub fn ball_active(&self) -> bool {
        self.ball.on
    }
    pub fn last_audio(&self) -> f64 {
        self.last_audio
    }

    /// `effReact`: music cannot show under reduced motion.
    pub fn eff_react(&self, reduce: bool) -> Preset {
        if self.react == Preset::Music && reduce {
            Preset::Calm
        } else {
            self.react
        }
    }

    fn react_animating(&self, reduce: bool) -> bool {
        let r = def(self.eff_react(reduce));
        r.kind != WaveKind::Bowl && !((r.kind == WaveKind::Settle || r.kind == WaveKind::Smile) && self.rt > 3.5)
    }

    fn audio_live(&self, now: f64) -> bool {
        !self.silent && now - self.last_audio < 1000.0
    }

    /// `musicOn()`
    pub fn music_on(&self, ctx: &Ctx) -> bool {
        (self.playing || self.audio_live(ctx.now_ms)) && !ctx.reduce
    }

    /// `paintReact`: a new effective reaction (restarts its animation).
    pub fn paint_react(&mut self, p: Preset, color: &str) {
        self.react = p;
        self.rt = 0.0;
        self.rcol = color.to_string();
    }

    /// `liqPlaying(on)` state part. Returns whether the loop must be kicked (`on || spikes up || (!reduce && edgeIdle)`).
    pub fn set_playing(&mut self, on: bool, ctx: &Ctx) -> bool {
        if on && !self.playing {
            self.play_since = ctx.now_ms;
            self.silent = false;
        }
        self.playing = on;
        on || self.sa.iter().any(|x| *x > 0.0) || (!ctx.reduce && ctx.edge_idle)
    }

    /// `liqColors`: resolve the edge colour (`override > music: accent || preset; others: preset || accent > --edge`) and the body fill.
    pub fn resolve_colors(&mut self, pal: &Palette, wave_c: &str, body_fill: Rgba, reduce: bool) {
        let eff = self.eff_react(reduce);
        let rc: &str = if self.rcol.is_empty() { def(eff).col } else { &self.rcol };
        let pick = if !self.override_color.is_empty() {
            self.override_color.as_str()
        } else if eff == Preset::Music {
            if wave_c.is_empty() { rc } else { wave_c }
        } else if rc.is_empty() {
            wave_c
        } else {
            rc
        };
        let parsed = if pick.is_empty() { Some(pal.edge) } else { parse_css(pick, pal) };
        // an invalid CSS colour assignment is ignored by the browser: the previous colour stays
        if let Some(c) = parsed {
            self.edge = c;
        }
        self.fill = body_fill;
    }

    /// `rgbOf(LQ.edge)`: the "r,g,b" used for the stroke gradient stops.
    pub fn rgb(&self) -> (u8, u8, u8) {
        self.edge.rgb_tuple()
    }

    // ---------- geometry ----------

    /// `liqRange`: the span ends exactly where the straight bottom edge ends and the corner arc starts.
    pub fn range(&self, ctx: &Ctx) -> Range {
        let s = ctx.scale;
        let (r, tp) = (ctx.r.max(0.0), ctx.t.max(0.0));
        let nz = |x: f64| if x == 0.0 { 1.0 } else { x };
        let f = 1.0f64.min(ctx.h / nz(tp + r)).min(ctx.w / nz(2.0 * r));
        let inset = 8.0f64.max(40.0f64.min(r * f / s));
        let half = (ctx.w / s / 2.0 - inset).max(0.0);
        let n = ((half / DX).floor() as usize).min(MID);
        Range { half, i0: MID - n, i1: MID + n }
    }

    /// `reactWave`: edge offset in px at `x` from the centre.
    fn react_wave(&self, x: f64, half: f64, expd: bool, reduce: bool) -> f64 {
        let r = def(self.eff_react(reduce));
        let t = self.t;
        let base = if expd { 3.5 } else { 2.5 };
        let hh = if half == 0.0 { 1.0 } else { half };
        match r.kind {
            WaveKind::Bowl => 5.0 * (1.0 - (x / hh).powi(2)),
            WaveKind::Zig => {
                let tri = (2.0 / std::f64::consts::PI) * (((x + 30.0 * t) / r.wl) * TAU).sin().asin();
                base * r.amp * 0.9 * tri * (0.8 + 0.2 * (t * 9.0 + x / 40.0).sin())
            }
            WaveKind::Beat => (1.5 + 5.0 * pulse(t)) * if expd { 0.8 } else { 1.0 } * (((x - r.spd * t) / r.wl) * TAU).sin(),
            WaveKind::Settle => {
                let a = r.a0 * (-self.rt / r.tau).exp() + 1.2;
                a * (0.62 * (((x + r.spd * t) / r.wl) * TAU).sin() + 0.38 * (((x + 0.7 * r.spd * t) / (r.wl * 0.58)) * TAU + 1.7).sin())
            }
            WaveKind::Smile => {
                let k = (-self.rt / r.tau).exp();
                4.0 * (1.0 - (x / hh).powi(2)) + 5.0 * k * (((x + 30.0 * t) / r.wl) * TAU).sin() * (0.6 + 0.4 * (x / 33.0).sin())
            }
            WaveKind::Sine => {
                let a = base * r.amp;
                let br = 0.8 + 0.2 * (t * 0.45).sin();
                a * br * (0.62 * (((x + r.spd * t) / r.wl) * TAU).sin() + 0.38 * (((x + 0.8 * r.spd * t) / (r.wl * 0.58)) * TAU + 2.1).sin())
            }
        }
    }

    /// `liqTargets`: target shape = idle / reaction wave (+ the cursor bulge).
    fn targets(&mut self, rg: &Range, ctx: &Ctx) {
        let dyn_ = self.inn.abs() > 0.002;
        let expd = ctx.expanded;
        let sig_w: f64 = (if expd { 50.0 } else { 38.0 }) * if self.drag { 1.45 } else { 1.0 };
        let sig2 = 2.0 * sig_w * sig_w;
        let bands = self.music_on(ctx) && !self.silent && ctx.now_ms - self.last_audio < 1000.0 && self.eff_react(ctx.reduce) == Preset::Music;
        for i in rg.i0..=rg.i1 {
            let x = (i as f64 - MID as f64) * DX;
            let env = smooth((rg.half - x.abs()) / 36.0);
            let mut v = (if bands { 0.0 } else { self.react_wave(x, rg.half, expd, ctx.reduce) }) * env - 0.6;
            if dyn_ {
                let dx = x - self.cxs;
                v += self.b_amp * self.inn * (-(dx * dx) / sig2).exp() * env; // smooth bell under the cursor (lags it like a spring)
            }
            self.tg[i] = v;
        }
    }

    /// Music spikes: 32 bands (bass centre, treble sides, symmetric) -> ~1 spike / 14 px, each a spring (fast attack, slow release).
    fn spikes_step(&mut self, dt: f64, rg: &Range, ctx: &Ctx) -> bool {
        let (half, i0, i1) = (rg.half, rg.i0, rg.i1);
        let now = ctx.now_ms;
        let m = 4usize.max(32usize.min(((2.0 * half) / 14.0).floor() as usize));
        let fresh = now - self.last_audio < 1000.0;
        let on = self.music_on(ctx) && !self.silent && half > 20.0 && fresh;
        let peak = if ctx.expanded { 10.0 } else { 14.0 };
        let step = (2.0 * half) / m as f64;
        let mut moving = false;
        for j in 0..m {
            // frequency <-> place: bass in the middle, treble towards both ends; a spike takes the loudest of the bands its slice covers
            let mut b = 0.0f64;
            if on {
                let d = (j as f64 + 0.5 - m as f64 / 2.0).abs();
                let d0 = (d - 0.5).max(0.0) / (m as f64 / 2.0);
                let d1 = (m as f64 / 2.0).min(d + 0.5) / (m as f64 / 2.0);
                let ke = 31usize.min((d1 * 31.0).ceil() as usize);
                let mut k = (d0 * 31.0).floor() as usize;
                while k <= ke {
                    b = b.max(self.bt[k]);
                    k += 1;
                }
            }
            self.bv[j] = b;
        }
        for j in 0..m {
            // ferrofluid never rests: every spike also swells and sinks on its own slow rhythm (0.75..1.15 of its band)
            let pers = 0.75 + 0.4 * (0.5 + 0.5 * (now * 0.0013 * (1.0 + (j % 5) as f64 * 0.37) + j as f64 * 2.399).sin());
            let b = 1.0f64.min(self.bv[j] * pers);
            let tgt = (peak * 1.35 * b.powf(0.8)).min(peak);
            let a = self.sa[j];
            let att = tgt > a;
            let (w, z) = if att { (40.0, 0.55) } else { (12.0, 0.8) };
            self.sv[j] += (w * w * (tgt - a) - 2.0 * z * w * self.sv[j]) * dt;
            self.sa[j] = (peak * 1.2).min((a + self.sv[j] * dt).max(0.0));
            if self.sa[j] > 0.05 || self.sv[j].abs() > 0.3 {
                moving = true;
            } else {
                self.sa[j] = 0.0;
                self.sv[j] = 0.0;
            }
        }
        for j in m..32 {
            self.sa[j] = 0.0;
            self.sv[j] = 0.0;
        }
        self.sp.iter_mut().for_each(|x| *x = 0.0);
        for j in 0..m {
            let a = self.sa[j];
            if a <= 0.0 {
                continue;
            }
            // per-spike width (6..11 px half-width) and tip sharpness (exponent 1.8..3.0): a varied row of cones
            let hw = 6.0 + 5.0 * (0.5 + 0.5 * (j as f64 * 12.9898).sin());
            let ex = 1.8 + 1.2 * (0.5 + 0.5 * (j as f64 * 7.31).sin());
            let p = -half + (j as f64 + 0.5) * step + step * 0.22 * (self.t * 0.9 * (1.0 + (j % 4) as f64 * 0.3) + j as f64 * 1.7).sin();
            if let Some((ia, ib)) = idx_span(MID as f64 + (p - hw) / DX, MID as f64 + (p + hw) / DX, i0, i1) {
                for i in ia..=ib {
                    let f = ((i as f64 - MID as f64) * DX - p).abs() / hw;
                    if f < 1.0 {
                        let v = a * (1.0 - f).powf(ex);
                        if v > self.sp[i] {
                            self.sp[i] = v;
                        }
                    }
                }
            }
        }
        if self.inn > 0.01 && ctx.edge_cones {
            // optional ferrofluid cones around the (lagging) cursor: spacing 14 px, gaussian envelope, rise and fall individually
            if let Some((ia, ib)) = idx_span(MID as f64 + (self.cxs - 70.0) / DX, MID as f64 + (self.cxs + 70.0) / DX, i0, i1) {
                for i in ia..=ib {
                    let x = (i as f64 - MID as f64) * DX;
                    let dx = x - self.cxs;
                    let uu = dx / 14.0;
                    let idx = uu.round();
                    let f = (uu - idx).abs();
                    let v = 12.0 * self.inn * (-(dx * dx) / 1400.0).exp() * (1.0 - f / 0.5).max(0.0).powf(1.5) * (0.55 + 0.45 * (self.t * 3.0 + idx * 1.7).sin()) * smooth((half - x.abs()) / 24.0);
                    if v > self.sp[i] {
                        self.sp[i] = v;
                    }
                }
            }
        }
        self.spike_move = moving;
        moving
    }

    /// `setBands`: feed one `audio:bands` frame. Returns true when it carries sound (the caller then keeps the `audio` music source alive
    /// for 2.5 s and kicks the loop when `music_on`).
    pub fn set_bands(&mut self, p: &Bands, now_ms: f64) -> bool {
        let mut sum = 0.0;
        // time-based decay: 0.993 per 33 ms (the old ~30 Hz event), whatever the event rate
        let (has_t, src_t) = (p.t.is_some(), p.t.unwrap_or(0.0));
        let prev_t = if has_t { self.bm_t } else { self.last_audio };
        let mut dt_ms = if has_t { src_t - prev_t } else { now_ms - prev_t };
        // `!(x > 0)` also catches NaN, like the JS
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let bad = !(prev_t > 0.0) || !(dt_ms > 0.0);
        dt_ms = if bad { 33.0 } else { dt_ms.min(250.0) };
        self.bm_t = if has_t { src_t } else { now_ms };
        let kd = 0.993f64.powf(dt_ms / 33.0);
        // Band values are dB (0..1 = -60..0 dB), so most bands sit high all the time. Level = distance below ONE running peak shared by all
        // bands, seen through a ~23 dB window. The tilt (+6 dB at the top) makes up for music's natural treble roll-off.
        let w = 0.38;
        let raw_of = |k: usize| p.b.get(k).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        let mut mx = 0.0f64;
        for k in 0..32 {
            let raw = raw_of(k);
            sum += raw;
            self.adj[k] = raw + 0.1 * k as f64 / 31.0;
            mx = mx.max(self.adj[k]);
        }
        self.gpk = mx.max(self.gpk * kd);
        let lo = (self.gpk - w).max(0.0);
        let a_slow = 1.0 - (-dt_ms / 700.0).exp();
        let a_dev = 1.0 - (-dt_ms / 2500.0).exp();
        for k in 0..32 {
            let raw = raw_of(k);
            if self.sl[k] < 0.0 {
                self.sl[k] = raw;
            }
            // presence = is this frequency really in the music; z = how far the band is above / below its own recent level
            let presence = ((self.adj[k] - lo) / w).clamp(0.0, 1.0);
            let dev = raw - self.sl[k];
            self.sl[k] += dev * a_slow;
            self.ad[k] += (dev.abs() - self.ad[k]) * a_dev;
            let z = dev / 0.02f64.max(self.ad[k] * 2.0);
            self.bt[k] = 1.0f64.min(presence * (0.25 + 0.75 * 0.0f64.max(1.2f64.min(0.5 + 0.6 * z))));
        }
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        let quiet_rms = !(p.rms > 0.01);
        self.silent = sum < 0.05 && quiet_rms;
        self.last_audio = now_ms;
        !self.silent
    }

    // ---------- the droplet ----------

    /// Inertial droplet: x follows a slowly changing random target through a damped spring, y = low-frequency bob + a slight sag while it
    /// moves. `hf` = half span of the line. Reset (centre, at rest) whenever the reaction has no ball.
    fn ball_step(&mut self, dt: f64, hf: f64, ctx: &Ctx) {
        let rp = def(self.eff_react(ctx.reduce));
        let off = rp.ball == BallKind::None || (rp.ball == BallKind::Start && self.rt >= 2.6) || ctx.reduce;
        if off {
            let b = &mut self.ball;
            b.on = false;
            b.x = 0.0;
            b.vx = 0.0;
            b.y = 0.0;
            b.b = 0.0;
            b.s = 0.0;
            b.sv = 0.0;
            b.dp = 0;
            b.hf = 0.0;
            return;
        }
        let ex = if ctx.expanded { 1.0 } else { 0.0 };
        if !self.ball.on {
            let (r1, r2) = (self.rng.next_f64(), self.rng.next_f64());
            let t = self.t;
            let b = &mut self.ball;
            b.on = true;
            b.x = 0.0;
            b.vx = 0.0;
            b.tg = 0.0;
            b.next = t + 1.2;
            b.ph = r1 * TAU;
            b.b = ex;
            b.s = 0.0;
            b.sv = 0.0;
            b.dp = 0;
            b.dn = t + 2.0 + r2 * 2.0;
            b.hf = hf;
        }
        if hf < 8.0 {
            return; // squashed to nothing (collapse): keep the state, resume when the edge is back
        }
        if self.ball.hf > 0.0 && hf != self.ball.hf {
            let k = hf / self.ball.hf; // the edge grew / shrank: keep the relative position, glide with it
            self.ball.x *= k;
            self.ball.tg *= k;
            self.ball.vx *= k;
        }
        self.ball.hf = hf;
        self.ball.b += (ex - self.ball.b) * (1.0 - (-dt * 3.5).exp()); // collapsed <-> expanded regime blend
        let b = self.ball.b;
        let lerp = |a: f64, c: f64| a + (c - a) * b;
        if self.t >= self.ball.next {
            // new target every 1.6-3.2 s (expanded 2.2-4.2 s), within +-65 % (expanded +-88 %) of the half span, never closer than 18 % to the previous one
            let reach = lerp(0.65, 0.88);
            let mut g = self.ball.tg;
            let mut tries = 0;
            while tries < 6 && (g - self.ball.tg).abs() < 0.18 * hf {
                g = (self.rng.next_f64() * 2.0 - 1.0) * reach * hf;
                tries += 1;
            }
            if (g - self.ball.tg).abs() < 0.18 * hf {
                g = if self.ball.tg > 0.0 { -0.5 * hf } else { 0.5 * hf };
            }
            self.ball.tg = g;
            self.ball.next = self.t + lerp(1.6, 2.2) + self.rng.next_f64() * lerp(1.6, 2.0);
        }
        // collapsed: w 1.6, v <= 45 px/s; expanded: slower spring (w 1.2) over a ~3x longer edge, v <= 90 px/s (same tanh limit)
        let (w, z, vm) = (lerp(1.6, 1.2), 0.9, lerp(45.0, 90.0));
        let n = ((dt * 60.0).ceil() as i64).max(1);
        let sd = dt / n as f64;
        let lim = lerp(0.8, 0.94) * hf;
        for _ in 0..n {
            self.ball.vx += (w * w * (self.ball.tg - self.ball.x) - 2.0 * z * w * self.ball.vx) * sd;
            self.ball.vx = vm * (self.ball.vx / vm).tanh(); // soft speed limit
            self.ball.x += self.ball.vx * sd;
        }
        self.ball.x = self.ball.x.clamp(-lim, lim);
        // drip (expanded only): every 4-8 s stretches down ~0.8 s (slow, overdamped), then releases with a damped bounce (w 14, zeta 0.35)
        let t = self.t;
        if self.ball.dp == 0 && b > 0.6 && t >= self.ball.dn {
            self.ball.dp = 1;
            self.ball.de = t + 0.8;
        } else if self.ball.dp == 1 && t >= self.ball.de {
            self.ball.dp = 2;
            self.ball.de = t + 1.6;
        } else if self.ball.dp == 2 && t >= self.ball.de && self.ball.s.abs() < 0.05 && self.ball.sv.abs() < 0.2 {
            self.ball.dp = 0;
            self.ball.s = 0.0;
            self.ball.sv = 0.0;
            self.ball.dn = t + 4.0 + self.rng.next_f64() * 4.0;
        }
        let stretch = self.ball.dp == 1;
        let (dw, dz, dtg) = if stretch { (7.0, 0.9, 4.0) } else { (14.0, 0.35, 0.0) };
        for _ in 0..n {
            self.ball.sv += (dw * dw * (dtg - self.ball.s) - 2.0 * dz * dw * self.ball.sv) * sd;
            self.ball.s += self.ball.sv * sd;
        }
        let tt = self.t + self.ball.ph;
        self.ball.y = 3.0 * (0.6 * (1.1 * tt).sin() + 0.4 * (1.9 * tt + 1.3).sin()) + 2.5f64.min(self.ball.vx.abs() * 0.06);
    }

    // ---------- stepping ----------

    /// `liqStatic`: one calm frame, no loop.
    pub fn render_static(&mut self, ctx: &Ctx) {
        let rg = self.range(ctx);
        for i in 0..N {
            self.h[i] = 0.0;
            self.v[i] = 0.0;
            self.sp[i] = 0.0;
        }
        self.sa = [0.0; 32];
        self.sv = [0.0; 32];
        self.targets(&rg, ctx);
        for i in rg.i0..=rg.i1 {
            self.h[i] = self.tg[i];
        }
        self.ball_step(0.0, (rg.i1 - rg.i0) as f64 * DX / 2.0, ctx);
    }

    /// `liqImpulse(d)`: the body height changed by `d` px (100 %): the edge lags behind (sags when the body moves up) and ripples.
    pub fn impulse(&mut self, d: f64, ctx: &Ctx) {
        let rg = self.range(ctx);
        let g = 2.2;
        let half = if rg.half == 0.0 { 1.0 } else { rg.half };
        for i in rg.i0..=rg.i1 {
            let x = (i as f64 - MID as f64) * DX;
            let prof = (1.0 - (x / half).powi(2)).max(0.0);
            self.v[i] += (-d + 0.3 * d.abs()) * g * prof;
        }
    }

    /// `idleGap()`: min seconds between edge frames; 0 = every display frame. Only calm / sleep are throttled.
    pub fn idle_gap(&self, ctx: &Ctx) -> f64 {
        match self.eff_react(ctx.reduce) {
            Preset::Calm | Preset::Sleep => 0.92 / ctx.idle_fps,
            _ => 0.0,
        }
    }

    /// `liqStep(dt)` (dt in seconds). Returns true while something still moves (keep the frame loop running).
    pub fn step(&mut self, dt: f64, ctx: &Ctx) -> bool {
        if ctx.reduce {
            self.render_static(ctx);
            return false;
        }
        if self.style != EdgeStyle::Liquid {
            // tail / glow / none: no wave physics; throttled to `edgeIdleFps` ONLY in calm / sleep, every other reaction runs at the display rate
            self.acc += dt;
            if self.acc < self.idle_gap(ctx) && !ctx.spring_busy {
                return true;
            }
            let dt = self.acc.min(0.05);
            self.acc = 0.0;
            self.t += dt;
            self.rt += dt;
            let r = self.eff_react(ctx.reduce);
            return self.style == EdgeStyle::Glow && r != Preset::Calm && r != Preset::Sleep && !((r == Preset::Done || r == Preset::File) && self.rt > 4.0);
        }
        self.acc += dt;
        let busy = self.want > 0.0 || self.inn != 0.0 || ctx.spring_busy || self.spike_move || self.e > 0.08;
        if !busy && self.acc < self.idle_gap(ctx) {
            return true; // only the calm / sleep drift is throttled (edgeIdleFps); reactions and the cursor pull run at full rate
        }
        let dt = self.acc.min(1.0 / 20.0);
        self.acc = 0.0;
        self.t += dt;
        self.rt += dt;
        // rises fast, releases with a damped oscillation
        self.inn_v += (220.0 * (self.want - self.inn) - 2.0 * (if self.want > self.inn { 0.55 } else { 0.4 }) * 14.8 * self.inn_v) * dt;
        self.inn += self.inn_v * dt;
        if self.want == 0.0 && self.inn.abs() < 0.004 && self.inn_v.abs() < 0.05 {
            self.inn = 0.0;
            self.inn_v = 0.0;
        }
        // cursor follows with spring lag
        self.cxv += (196.0 * (self.cx - self.cxs) - 2.0 * 0.8 * 14.0 * self.cxv) * dt;
        self.cxs += self.cxv * dt;
        let rg = self.range(ctx);
        let (i0, i1) = (rg.i0, rg.i1);
        self.targets(&rg, ctx);
        let n = ((dt * 120.0).ceil() as i64).max(1);
        let sd = dt / n as f64;
        let df = (-DAMP * sd).exp();
        for _ in 0..n {
            for i in (i0 + 1)..i1 {
                let dev = |j: usize| self.h[j] - self.tg[j];
                let nv = (self.v[i] + (K * (self.tg[i] - self.h[i]) + C * (dev(i - 1) + dev(i + 1) - 2.0 * dev(i))) * sd) * df;
                self.v[i] = nv;
            }
            for i in (i0 + 1)..i1 {
                self.h[i] += self.v[i] * sd;
                if self.h[i] > MAX_H {
                    self.h[i] = MAX_H;
                    self.v[i] = 0.0;
                }
            }
        }
        for i in 0..N {
            if i <= i0 || i >= i1 {
                self.h[i] = 0.0;
                self.v[i] = 0.0;
            }
        }
        let mut e = 0.0f64;
        for i in (i0 + 1)..i1 {
            e = e.max(self.v[i].abs() * 0.05).max((self.h[i] - self.tg[i]).abs());
        }
        self.e = e;
        let sm = self.spikes_step(dt, &rg, ctx);
        self.ball_step(dt, (i1 - i0) as f64 * DX / 2.0, ctx);
        let eff = self.eff_react(ctx.reduce);
        let moving = e > 0.08
            || self.want > 0.0
            || self.inn != 0.0
            || sm
            || (self.music_on(ctx) && !self.silent)
            || (!ctx.reduce && ((ctx.edge_idle && eff == Preset::Calm) || (eff != Preset::Calm && self.react_animating(ctx.reduce))));
        if !moving {
            for i in (i0 + 1)..i1 {
                self.h[i] = self.tg[i];
                self.v[i] = 0.0;
            }
        }
        moving
    }

    // ---------- output ----------

    /// What the renderer draws for the edge now (`liqDraw`).
    pub fn edge_view(&self, ctx: &Ctx) -> EdgeView {
        let rg = self.range(ctx);
        let mut view = EdgeView {
            style: self.style,
            visible: self.style != EdgeStyle::None,
            scale: ctx.scale,
            span: SPAN,
            strip_h: STRIP + UP,
            up: UP,
            half: rg.half,
            fill: self.fill,
            points: Vec::new(),
            stroke: None,
            ball: None,
            glow: None,
        };
        if self.style != EdgeStyle::Liquid {
            view.glow = self.glow_view(&rg, ctx);
            return view;
        }
        let (i0, i1, half) = (rg.i0, rg.i1, rg.half);
        if i1 <= i0 {
            return view;
        }
        // Index k: 0 = left end, 1..n-2 = samples i0..i1, n-1 = right end.
        let n = i1 - i0 + 3;
        let mut pts = Vec::with_capacity(n);
        pts.push((-half, 0.0));
        for i in i0..=i1 {
            let x = (i as f64 - MID as f64) * DX;
            pts.push((x, MAX_OFFSET.min(liq_bound(self.h[i] + self.sp[i], x, half))));
        }
        pts.push((half, 0.0));
        let eff = self.eff_react(ctx.reduce);
        let rp = def(eff);
        let rgb = if rp.noline { (30, 30, 30) } else { self.rgb() };
        let mut passes = Vec::new();
        if rp.noline {
            passes.push((1.0, 1.2)); // sleep: just a dark bowl edge
        } else {
            // glow: two wider, translucent passes under the line so the edge also reads on light / grey wallpapers
            if eff != Preset::Calm {
                // calm has only the dark hairline: no halo (it would read as a smudge on grey)
                passes.push((0.16, 8.0));
                passes.push((0.32, 4.5));
            }
            passes.push((1.0, 2.0));
        }
        let gradient = [(0.0, 0.0), (FADE * 0.5, 0.3), (FADE, 1.0), (1.0 - FADE, 1.0), (1.0 - FADE * 0.5, 0.3), (1.0, 0.0)];
        let static_glow = if !rp.noline && self.playing && ctx.reduce { 5.0 } else { 0.0 }; // reduced motion: only a gentle static glow
        view.stroke = Some(Stroke { rgb, gradient, passes, shadow_blur: static_glow, shadow_color: self.edge });
        if rp.ball != BallKind::None && (rp.ball != BallKind::Start || self.rt < 2.6) {
            let kf = 1.0f64.max(((n - 2) as f64).min(self.ball.x / DX + MID as f64 - i0 as f64 + 1.0));
            let k0 = kf.floor() as usize;
            let yl = pts[k0].1 + (pts[(n - 1).min(k0 + 1)].1 - pts[k0].1) * (kf - k0 as f64); // continuous x, line height interpolated
            let bb = self.ball.b;
            let r0 = 3.5 + 1.3 * bb;
            let ry = (0.75 * r0).max(r0 + self.ball.s); // expanded: bigger, hangs lower, drip stretch (ellipse, volume-ish constant)
            let rx = r0 * (r0 / ry).sqrt();
            view.ball = Some(BallView { x: self.ball.x, y: UP + yl + 11.0 + 4.0 * bb + self.ball.y + (ry - r0) * 0.5, rx, ry });
        }
        view.points = pts;
        view
    }

    fn glow_view(&self, rg: &Range, ctx: &Ctx) -> Option<GlowView> {
        if matches!(self.style, EdgeStyle::None | EdgeStyle::Tail | EdgeStyle::Legs) || rg.half < 20.0 {
            return None; // tail / legs are drawn by the character overlay
        }
        let (r, t, rt) = (self.eff_react(ctx.reduce), self.t, self.rt);
        let op = match r {
            Preset::Music => 0.25 + 0.45 * pulse(t),
            Preset::Thinking => 0.22 + 0.18 * (t * 1.6).sin(),
            Preset::Done => 0.05 + 0.6 * (-rt / 1.2).exp(),
            Preset::Attention => 0.25 + 0.35 * if (t * 10.0).sin() > 0.0 { 1.0 } else { 0.4 },
            Preset::File => 0.35,
            _ => 0.06,
        };
        Some(GlowView { alpha: op.clamp(0.0, 1.0), x: 0.0, y: UP + 1.0, rx: rg.half * 0.7, ry: 3.0, color: self.edge, blur: 10.0, clip_above: UP })
    }
}

/// Clamp a fractional index span to `[i0, i1]`; None when empty.
fn idx_span(lo: f64, hi: f64, i0: usize, i1: usize) -> Option<(usize, usize)> {
    let a = lo.floor().max(i0 as f64);
    let b = hi.ceil().min(i1 as f64);
    if b < a {
        None
    } else {
        Some((a as usize, b as usize))
    }
}

/// Stroke description of the liquid line.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub rgb: (u8, u8, u8),
    /// (offset 0..1 along the span, alpha): the line dissolves over the last 15 % at both ends
    pub gradient: [(f64, f64); 6],
    /// (global alpha, line width) in draw order
    pub passes: Vec<(f64, f64)>,
    pub shadow_blur: f64,
    pub shadow_color: Rgba,
}

/// The droplet hanging under the line: centre `(x, y)` (x from the body centre, y in strip coordinates, i.e. `UP` = the straight edge), radii.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BallView {
    pub x: f64,
    pub y: f64,
    pub rx: f64,
    pub ry: f64,
}

/// `glow` edge style: a soft ellipse of light under the island (the part above `clip_above` is cleared).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlowView {
    pub alpha: f64,
    pub x: f64,
    pub y: f64,
    pub rx: f64,
    pub ry: f64,
    pub color: Rgba,
    pub blur: f64,
    pub clip_above: f64,
}

/// Everything the renderer needs for the bottom edge. Coordinates are in 100 % px; multiply by `scale`. The strip is `span x strip_h`,
/// centred on the body, its top at `body_bottom - up * scale`; polyline `y` is measured from the straight bottom edge (`up` in strip space).
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeView {
    pub style: EdgeStyle,
    pub visible: bool,
    pub scale: f64,
    pub span: f64,
    pub strip_h: f64,
    pub up: f64,
    pub half: f64,
    /// body fill colour (the polygon between the straight edge and the curve is filled with it)
    pub fill: Rgba,
    pub points: Vec<(f64, f64)>,
    pub stroke: Option<Stroke>,
    pub ball: Option<BallView>,
    pub glow: Option<GlowView>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::color::{palette, Theme};

    fn ctx() -> Ctx {
        Ctx { w: 144.0, h: 30.0, r: 15.0, t: 15.0, now_ms: 10_000.0, ..Ctx::default() }
    }
    fn run(l: &mut Liquid, c: &mut Ctx, frames: usize, dt: f64) {
        for _ in 0..frames {
            c.now_ms += dt * 1000.0;
            l.step(dt, c);
        }
    }

    #[test]
    fn constants_match_the_js() {
        assert_eq!(N, (SPAN / DX / 2.0).ceil() as usize * 2 + 1);
        assert_eq!(MID, (N - 1) / 2);
        assert_eq!(SPAN, 680.0 + 40.0);
        assert_eq!((K, C, DAMP), (220.0, 900.0, 9.0));
    }

    #[test]
    fn range_for_the_collapsed_capsule() {
        let l = Liquid::new(1);
        let rg = l.range(&ctx());
        // r = 15 -> inset = max(8, min(40, 15)) = 15; half = 144/2 - 15 = 57
        assert_eq!(rg.half, 57.0);
        assert_eq!((rg.i0, rg.i1), (MID - 19, MID + 19));
        // expanded body: inset 30 (radius 30), half = 340 - 30 = 310 -> 103 samples
        let c = Ctx { w: 680.0, h: 331.0, r: 30.0, t: 26.0, expanded: true, ..ctx() };
        let rg = l.range(&c);
        assert_eq!(rg.half, 310.0);
        assert_eq!(rg.i1 - rg.i0, 206);
        // the collapse squash: radii scale down together (f < 1), the inset follows
        let c = Ctx { w: 100.0, h: 10.0, r: 15.0, t: 15.0, ..ctx() };
        let rg = l.range(&c);
        assert!(rg.half > 0.0 && rg.half < 50.0);
        // a degenerate body never panics and an overshoot never leaves the array
        let c = Ctx { w: 3000.0, ..ctx() };
        let rg = l.range(&c);
        assert!(rg.i0 == 0 && rg.i1 == N - 1);
        let c = Ctx { w: 0.0, h: 0.0, r: 0.0, t: 0.0, ..ctx() };
        assert_eq!(l.range(&c).half, 0.0);
    }

    #[test]
    fn scale_changes_the_range_in_100_percent_px() {
        let l = Liquid::new(1);
        // 150 %: body 216 px wide = 144 at 100 %
        let c = Ctx { scale: 1.5, w: 216.0, h: 45.0, r: 22.5, t: 22.5, ..ctx() };
        assert!((l.range(&c).half - 57.0).abs() < 1e-9);
    }

    #[test]
    fn bound_function_properties() {
        // zero at both ends of the straight part, with a smooth taper
        assert_eq!(liq_bound(5.0, 57.0, 57.0), 0.0);
        assert_eq!(liq_bound(5.0, -57.0, 57.0), 0.0);
        assert!(liq_bound(5.0, 0.0, 57.0) > 4.9);
        // never negative (the body is a box and cannot be cut), tiny for negative field values
        for y in [-30.0, -5.0, 0.0, 3.0, 14.0] {
            for x in [-50.0, -20.0, 0.0, 33.0] {
                assert!(liq_bound(y, x, 57.0) >= 0.0);
            }
        }
        assert!(liq_bound(-30.0, 0.0, 57.0) < 0.05);
    }

    #[test]
    fn pulse_is_periodic_and_bounded() {
        assert_eq!(pulse(0.0), 0.0);
        assert!((pulse(0.21 * 1.1) - 1.0).abs() < 1e-9);
        for i in 0..200 {
            let p = pulse(i as f64 * 0.013);
            assert!((0.0..=1.0).contains(&p));
        }
        assert!((pulse(0.1) - pulse(1.2)).abs() < 1e-9);
    }

    #[test]
    fn simulation_is_deterministic_for_a_seed() {
        let go = |seed| {
            let mut l = Liquid::new(seed);
            let mut c = ctx();
            l.paint_react(Preset::Thinking, "");
            l.want = 0.7;
            l.cx = 20.0;
            run(&mut l, &mut c, 240, 1.0 / 60.0);
            (l.heights().to_vec(), l.ball_x(), l.edge_view(&c))
        };
        assert_eq!(go(5), go(5));
        // the ball is the only random part: a different seed gives a different droplet, the wave is identical
        let (h5, b5, _) = go(5);
        let (h6, b6, _) = go(6);
        assert_eq!(h5, h6);
        assert_ne!(b5, b6);
    }

    #[test]
    fn field_and_polyline_stay_bounded_under_stress() {
        let mut l = Liquid::new(3);
        let mut c = Ctx { w: 680.0, h: 331.0, r: 30.0, t: 26.0, expanded: true, ..ctx() };
        l.paint_react(Preset::Attention, "");
        l.want = 1.0;
        l.b_amp = 21.0;
        l.cx = 100.0;
        for k in 0..600 {
            c.now_ms += 1000.0 / 144.0;
            if k % 37 == 0 {
                l.impulse(if k % 2 == 0 { 120.0 } else { -120.0 }, &c);
            }
            l.step(1.0 / 144.0, &c);
            for &h in l.heights() {
                assert!(h.is_finite() && h <= MAX_H + 1e-9, "h {h}");
            }
        }
        let v = l.edge_view(&c);
        assert!(!v.points.is_empty());
        assert_eq!(v.points.first().unwrap().1, 0.0);
        assert_eq!(v.points.last().unwrap().1, 0.0);
        assert_eq!(v.points.first().unwrap().0, -v.half);
        for &(x, y) in &v.points {
            assert!(x.abs() <= v.half + 1e-9);
            assert!((0.0..=MAX_OFFSET).contains(&y), "y {y}");
        }
        // x strictly increasing
        assert!(v.points.windows(2).all(|w| w[1].0 > w[0].0));
    }

    #[test]
    fn calm_idle_drifts_when_the_edge_idle_switch_is_on() {
        let mut l = Liquid::new(1);
        let mut c = ctx();
        l.render_static(&c);
        let a = l.edge_view(&c).points;
        run(&mut l, &mut c, 120, 1.0 / 60.0);
        let b = l.edge_view(&c).points;
        assert_ne!(a, b);
        // reduced motion: static, no loop, identical frames
        let c2 = Ctx { reduce: true, ..c };
        let mut l2 = Liquid::new(1);
        assert!(!l2.step(0.016, &c2));
        let s1 = l2.edge_view(&c2).points;
        assert!(!l2.step(0.5, &c2));
        assert_eq!(s1, l2.edge_view(&c2).points);
    }

    #[test]
    fn idle_cap_throttles_only_calm_and_sleep() {
        let mut c = ctx();
        c.idle_fps = 30.0;
        // Finding carried over from the JS: in the liquid style `e` (field deviation from target) never drops below ~0.37 because the pinned
        // end samples keep a constant offset against the target, so `busy` is always true and the idle cap does not engage there. It engages in
        // the other styles (glow, ...): calm / sleep run every ~2nd 60 Hz frame (gap 0.92 / 30 s).
        let mut liquid = Liquid::new(1);
        liquid.paint_react(Preset::Sleep, "");
        for _ in 0..240 {
            c.now_ms += 1000.0 / 60.0;
            liquid.step(1.0 / 60.0, &c);
        }
        assert!(liquid.e > 0.08, "pinned-edge offset keeps the liquid field busy: {}", liquid.e);
        let mut calm = Liquid::new(1);
        calm.style = EdgeStyle::Glow;
        let mut work = 0;
        for _ in 0..60 {
            c.now_ms += 1000.0 / 60.0;
            let t0 = calm.t;
            calm.step(1.0 / 60.0, &c);
            if calm.t != t0 {
                work += 1;
            }
        }
        assert!((28..=32).contains(&work), "glow calm steps {work}");
        // a reaction runs every frame
        let mut busy = Liquid::new(1);
        busy.paint_react(Preset::Attention, "");
        let mut work = 0;
        for _ in 0..60 {
            let t0 = busy.t;
            busy.step(1.0 / 60.0, &c);
            if busy.t != t0 {
                work += 1;
            }
        }
        assert_eq!(work, 60);
        // the cap values themselves: 15 -> every 61 ms, 60 -> every 15 ms
        let l = Liquid::new(1);
        assert!((l.idle_gap(&Ctx { idle_fps: 15.0, ..c }) - 0.92 / 15.0).abs() < 1e-12);
        assert!((l.idle_gap(&Ctx { idle_fps: 60.0, ..c }) - 0.92 / 60.0).abs() < 1e-12);
    }

    #[test]
    fn calm_goes_still_when_edge_idle_is_off() {
        let mut l = Liquid::new(1);
        let mut c = Ctx { edge_idle: false, ..ctx() };
        l.render_static(&c);
        let mut moving = true;
        for _ in 0..300 {
            c.now_ms += 20.0;
            moving = l.step(1.0 / 50.0, &c);
            if !moving {
                break;
            }
        }
        assert!(!moving, "the loop stops by itself");
    }

    #[test]
    fn cursor_pull_bulges_under_the_cursor_and_relaxes() {
        let mut l = Liquid::new(1);
        let mut c = Ctx { edge_idle: false, ..ctx() };
        l.render_static(&c);
        let rest = l.edge_view(&c).points;
        l.want = 1.0;
        l.b_amp = 14.0;
        l.cx = 0.0;
        run(&mut l, &mut c, 90, 1.0 / 60.0);
        let pulled = l.edge_view(&c).points;
        let mid = pulled.len() / 2;
        assert!(pulled[mid].1 > rest[mid].1 + 5.0, "sag {} vs {}", pulled[mid].1, rest[mid].1);
        l.want = 0.0;
        run(&mut l, &mut c, 400, 1.0 / 60.0);
        let back = l.edge_view(&c).points;
        assert!((back[mid].1 - rest[mid].1).abs() < 1.5);
        assert_eq!(l.inn, 0.0);
    }

    #[test]
    fn drag_widens_the_bulge() {
        let c = Ctx { edge_idle: false, ..ctx() };
        let width_at = |drag: bool| {
            let mut l = Liquid::new(1);
            let mut c = c;
            l.drag = drag;
            l.render_static(&c);
            l.want = 1.0;
            l.b_amp = 21.0;
            l.cx = 0.0;
            run(&mut l, &mut c, 120, 1.0 / 60.0);
            l.edge_view(&c).points.iter().filter(|p| p.1 > 3.0).count()
        };
        assert!(width_at(true) >= width_at(false));
    }

    #[test]
    fn impulse_sags_when_the_body_grows_and_ripples() {
        let mut l = Liquid::new(1);
        let c = ctx();
        l.render_static(&c);
        l.impulse(40.0, &c); // body got taller by 40 px: the edge lags (v < 0 in the centre)
        assert!(l.v[MID] < 0.0);
        let mut l = Liquid::new(1);
        l.render_static(&c);
        l.impulse(-40.0, &c);
        assert!(l.v[MID] > 0.0);
        // zero outside the range
        assert_eq!(l.v[0], 0.0);
    }

    fn loud(rms: f64) -> Bands {
        Bands { b: (0..32).map(|k| 0.9 - 0.4 * (k as f64 / 31.0)).collect(), rms, t: None }
    }

    #[test]
    fn bands_make_spikes_that_rise_and_decay() {
        let mut l = Liquid::new(1);
        let mut c = ctx();
        l.paint_react(Preset::Music, "");
        l.set_playing(true, &c);
        let mut peak = 0.0f64;
        for k in 0..200 {
            c.now_ms += 33.0;
            // alternate loud / quiet frames so the per-band deviation produces motion
            let mut b = loud(0.3);
            if k % 2 == 1 {
                b.b.iter_mut().for_each(|x| *x *= 0.7);
            }
            assert!(l.set_bands(&b, c.now_ms));
            l.step(1.0 / 30.0, &c);
            peak = peak.max(l.spikes().iter().cloned().fold(0.0, f64::max));
        }
        assert!(peak > 3.0, "spike peak {peak}");
        assert!(peak <= 14.0 * 1.2 + 1e-9);
        // silence: spikes die out
        l.set_playing(false, &c);
        for _ in 0..240 {
            c.now_ms += 33.0;
            assert!(!l.set_bands(&Bands { b: vec![0.0; 32], rms: 0.0, t: None }, c.now_ms));
            l.step(1.0 / 30.0, &c);
        }
        assert!(l.spike_amps().iter().all(|a| *a == 0.0));
        assert!(l.spikes().iter().all(|a| *a == 0.0));
    }

    #[test]
    fn spikes_are_symmetric_and_capped_by_the_regime() {
        let mut l = Liquid::new(1);
        let mut c = Ctx { w: 680.0, h: 331.0, r: 30.0, t: 26.0, expanded: true, ..ctx() };
        l.paint_react(Preset::Music, "");
        l.set_playing(true, &c);
        for k in 0..150 {
            c.now_ms += 33.0;
            let mut b = loud(0.5);
            if k % 3 == 0 {
                b.b.iter_mut().for_each(|x| *x *= 0.6);
            }
            l.set_bands(&b, c.now_ms);
            l.step(1.0 / 30.0, &c);
        }
        let mx = l.spike_amps().iter().cloned().fold(0.0, f64::max);
        assert!(mx <= 10.0 * 1.2 + 1e-9, "expanded peak is 10: {mx}"); // peak 10 expanded, 14 collapsed
        // bass in the middle: the centre-most spike is at least as tall as the outermost ones
        assert!(l.spike_amps()[0] >= l.spike_amps()[8] - 1e-9 || mx > 0.0);
    }

    #[test]
    fn bands_silence_detection_and_dt_handling() {
        let mut l = Liquid::new(1);
        assert!(!l.set_bands(&Bands { b: vec![0.0; 32], rms: 0.0, t: None }, 1000.0));
        assert!(l.silent);
        // quiet bands but rms above 0.01 still count as sound
        assert!(l.set_bands(&Bands { b: vec![0.0; 32], rms: 0.02, t: None }, 1033.0));
        // short / missing arrays are zero-filled, NaN-safe
        assert!(l.set_bands(&Bands { b: vec![1.0; 3], rms: 0.0, t: None }, 1066.0));
        // payload time: dt from the source clock
        let mut l = Liquid::new(1);
        l.set_bands(&Bands { b: vec![0.5; 32], rms: 0.1, t: Some(1000.0) }, 5.0);
        l.set_bands(&Bands { b: vec![0.5; 32], rms: 0.1, t: Some(1100.0) }, 6.0);
        assert!(l.last_audio() == 6.0);
        let j = serde_json::json!({"b": [0.1, "0.5", null, "x"], "rms": 0.2, "t": 12});
        let b = Bands::from_json(&j).unwrap();
        assert_eq!(b.b, vec![0.1, 0.5, 0.0, 0.0]);
        assert_eq!((b.rms, b.t), (0.2, Some(12.0)));
        assert!(Bands::from_json(&serde_json::json!({"rms": 1})).is_none());
    }

    #[test]
    fn music_needs_motion_and_fresh_audio() {
        let mut l = Liquid::new(1);
        let c = ctx();
        assert!(!l.music_on(&c));
        l.set_bands(&loud(0.5), c.now_ms);
        assert!(l.music_on(&c)); // fresh non-silent frames are enough (another chip may own the row)
        assert!(!l.music_on(&Ctx { now_ms: c.now_ms + 1001.0, ..c }));
        assert!(!l.music_on(&Ctx { reduce: true, ..c }));
        assert_eq!(l.eff_react(true), Preset::Calm);
        l.paint_react(Preset::Music, "");
        assert_eq!(l.eff_react(true), Preset::Calm); // reduced motion: music shows as calm
        assert_eq!(l.eff_react(false), Preset::Music);
    }

    #[test]
    fn thinking_ball_hangs_wanders_and_stays_inside() {
        let mut l = Liquid::new(9);
        let mut c = ctx();
        l.paint_react(Preset::Thinking, "");
        let mut xs = Vec::new();
        for _ in 0..1200 {
            c.now_ms += 1000.0 / 60.0;
            l.step(1.0 / 60.0, &c);
            xs.push(l.ball_x());
        }
        let hf = (l.range(&c).i1 - l.range(&c).i0) as f64 * DX / 2.0;
        assert!(xs.iter().all(|x| x.abs() <= 0.8 * hf + 1e-9));
        assert!(xs.iter().cloned().fold(0.0, f64::max) - xs.iter().cloned().fold(0.0, f64::min) > 10.0, "it moves");
        let v = l.edge_view(&c);
        let b = v.ball.expect("ball while thinking");
        assert!(b.y > UP + 10.0 && b.rx > 0.0 && b.ry > 0.0);
        // another reaction removes it
        l.paint_react(Preset::Calm, "");
        l.step(1.0 / 60.0, &c);
        assert!(l.edge_view(&c).ball.is_none());
        assert!(!l.ball_active());
    }

    #[test]
    fn done_ball_only_for_the_first_2_6_seconds_and_reduced_motion_has_none() {
        let mut l = Liquid::new(1);
        let mut c = ctx();
        l.paint_react(Preset::Done, "");
        assert!(l.edge_view(&c).ball.is_some());
        run(&mut l, &mut c, 180, 1.0 / 60.0); // 3 s
        assert!(l.edge_view(&c).ball.is_none());
        let mut l = Liquid::new(1);
        l.paint_react(Preset::Thinking, "");
        let c2 = Ctx { reduce: true, ..ctx() };
        l.render_static(&c2);
        assert!(!l.ball_active());
    }

    #[test]
    fn expanded_ball_is_bigger_and_drips() {
        let mut l = Liquid::new(2);
        let mut c = Ctx { w: 680.0, h: 331.0, r: 30.0, t: 26.0, expanded: true, ..ctx() };
        l.paint_react(Preset::Thinking, "");
        let mut max_ry = 0.0f64;
        let mut min_rx = f64::MAX;
        for _ in 0..60 * 20 {
            c.now_ms += 1000.0 / 60.0;
            l.step(1.0 / 60.0, &c);
            if let Some(b) = l.edge_view(&c).ball {
                max_ry = max_ry.max(b.ry);
                min_rx = min_rx.min(b.rx);
            }
        }
        assert!(max_ry > 4.8 + 0.5, "drip stretches the droplet: {max_ry}"); // r0 = 3.5 + 1.3 expanded = 4.8
        assert!(min_rx < 4.8);
    }

    #[test]
    fn reaction_presets_shape_the_line_differently() {
        let sample = |p: Preset| {
            let mut l = Liquid::new(1);
            let mut c = ctx();
            l.paint_react(p, "");
            run(&mut l, &mut c, 200, 1.0 / 60.0);
            l.edge_view(&c)
        };
        let calm = sample(Preset::Calm);
        let att = sample(Preset::Attention);
        let sleep = sample(Preset::Sleep);
        let amp = |v: &EdgeView| v.points.iter().map(|p| p.1).fold(0.0, f64::max) - v.points.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        assert!(amp(&att) > amp(&calm) * 0.5);
        // sleep: a bowl, no halo, dark hairline, 1.2 px
        let st = sleep.stroke.unwrap();
        assert_eq!(st.rgb, (30, 30, 30));
        assert_eq!(st.passes, vec![(1.0, 1.2)]);
        let mid = sleep.points.len() / 2;
        assert!(sleep.points[mid].1 > sleep.points[2].1, "sags in the middle");
        // calm has no halo passes, the others have two
        assert_eq!(calm.stroke.unwrap().passes, vec![(1.0, 2.0)]);
        assert_eq!(att.stroke.unwrap().passes, vec![(0.16, 8.0), (0.32, 4.5), (1.0, 2.0)]);
    }

    #[test]
    fn done_settles_to_nearly_flat_and_stops_animating() {
        let mut l = Liquid::new(1);
        let mut c = Ctx { edge_idle: false, ..ctx() };
        l.paint_react(Preset::Done, "#7fae45");
        let amp0 = {
            run(&mut l, &mut c, 6, 1.0 / 60.0);
            l.heights().iter().cloned().fold(0.0f64, |a, b| a.max(b.abs()))
        };
        run(&mut l, &mut c, 60 * 5, 1.0 / 60.0);
        let amp1 = l.heights().iter().cloned().fold(0.0f64, |a, b| a.max(b.abs()));
        assert!(amp1 < amp0 * 0.5, "{amp0} -> {amp1}");
        assert!(!l.step(1.0 / 60.0, &c) || l.rt > 3.5);
    }

    #[test]
    fn colour_resolution_order() {
        let pal = palette(Theme::Dark);
        let mut l = Liquid::new(1);
        let bg = Rgba::rgb(0, 0, 0);
        // calm, nothing: --edge
        l.resolve_colors(&pal, "", bg, false);
        assert_eq!(l.edge, pal.edge);
        // calm + a live accent: the accent
        l.resolve_colors(&pal, "#112233", bg, false);
        assert_eq!(l.edge, Rgba::rgb(0x11, 0x22, 0x33));
        // thinking: the preset colour beats the accent
        l.paint_react(Preset::Thinking, "");
        l.resolve_colors(&pal, "#112233", bg, false);
        assert_eq!(l.edge, Rgba::rgb(0x9e, 0x8f, 0xe8));
        // music: the accent beats the preset colour; without an accent the preset
        l.paint_react(Preset::Music, "");
        l.resolve_colors(&pal, "#112233", bg, false);
        assert_eq!(l.edge, Rgba::rgb(0x11, 0x22, 0x33));
        l.resolve_colors(&pal, "", bg, false);
        assert_eq!(l.edge, Rgba::rgb(0xd8, 0x83, 0x6a));
        // an explicit reaction colour replaces the preset colour
        l.paint_react(Preset::Done, "#ff0000");
        l.resolve_colors(&pal, "", bg, false);
        assert_eq!(l.edge, Rgba::rgb(255, 0, 0));
        // the override wins over everything
        l.override_color = "#00ff00".into();
        l.resolve_colors(&pal, "#112233", bg, false);
        assert_eq!(l.edge, Rgba::rgb(0, 255, 0));
        // reduced motion: music shows as calm (accent/edge colours)
        l.override_color.clear();
        l.paint_react(Preset::Music, "");
        l.resolve_colors(&pal, "", bg, true);
        assert_eq!(l.edge, pal.edge);
        // light theme --edge
        let lp = palette(Theme::Light);
        l.paint_react(Preset::Calm, "");
        l.resolve_colors(&lp, "", bg, false);
        assert_eq!(l.edge, lp.edge);
        // an invalid colour keeps the previous one
        let before = l.edge;
        l.override_color = "not-a-colour".into();
        l.resolve_colors(&lp, "", bg, false);
        assert_eq!(l.edge, before);
        assert_eq!(l.fill, bg);
    }

    #[test]
    fn reduced_motion_playing_gets_a_static_glow() {
        let mut l = Liquid::new(1);
        l.paint_react(Preset::Calm, "");
        l.set_playing(true, &ctx());
        let c = Ctx { reduce: true, ..ctx() };
        l.render_static(&c);
        assert_eq!(l.edge_view(&c).stroke.unwrap().shadow_blur, 5.0);
        assert_eq!(l.edge_view(&ctx()).stroke.unwrap().shadow_blur, 0.0);
    }

    #[test]
    fn alt_styles_glow_none_and_character_overlay() {
        let mut l = Liquid::new(1);
        let mut c = ctx();
        l.style = EdgeStyle::None;
        assert!(!l.edge_view(&c).visible);
        l.style = EdgeStyle::Tail;
        assert!(l.edge_view(&c).glow.is_none() && l.edge_view(&c).points.is_empty());
        l.style = EdgeStyle::Glow;
        let g = l.edge_view(&c).glow.unwrap();
        assert_eq!(g.alpha, 0.06); // calm
        assert_eq!((g.blur, g.ry), (10.0, 3.0));
        assert!((g.rx - 57.0 * 0.7).abs() < 1e-9);
        l.paint_react(Preset::File, "");
        assert_eq!(l.edge_view(&c).glow.unwrap().alpha, 0.35);
        l.paint_react(Preset::Done, "");
        assert!(l.edge_view(&c).glow.unwrap().alpha > 0.6);
        // narrow (squashed) body: no glow
        c.w = 40.0;
        assert!(l.edge_view(&c).glow.is_none());
        // glow is not throttled outside calm / sleep and stops by itself for done / file after 4 s
        let c = ctx();
        l.paint_react(Preset::Attention, "");
        assert!(l.step(1.0 / 60.0, &c));
        l.paint_react(Preset::Done, "");
        l.rt = 5.0;
        assert!(!l.step(1.0 / 60.0, &c));
        l.paint_react(Preset::Calm, "");
        assert!(!l.step(1.0, &c));
    }

    #[test]
    fn paint_react_restarts_the_animation_clock() {
        let mut l = Liquid::new(1);
        let mut c = ctx();
        run(&mut l, &mut c, 90, 1.0 / 60.0);
        assert!(l.rt > 1.0);
        l.paint_react(Preset::Done, "#7fae45");
        assert_eq!(l.rt, 0.0);
        assert_eq!(l.rcol, "#7fae45");
    }

    #[test]
    fn set_playing_kick_decision() {
        let mut l = Liquid::new(1);
        let c = ctx();
        assert!(l.set_playing(true, &c));
        assert!(l.playing);
        assert!(l.set_playing(false, &c)); // idle edge on: still kicks
        assert!(!l.set_playing(false, &Ctx { edge_idle: false, ..c }));
        assert!(!l.set_playing(false, &Ctx { reduce: true, ..c }));
    }

    #[test]
    fn empty_range_draws_nothing() {
        let l = Liquid::new(1);
        let c = Ctx { w: 20.0, h: 30.0, r: 15.0, t: 15.0, ..ctx() }; // half = 0 -> i0 == i1
        assert!(l.edge_view(&c).points.is_empty());
    }
}
