//! The pure logic behind the animated weather card: which scene a forecast icon is, the lightning schedule, the rain / snow particles, the
//! counting temperature, the refresh spin and the entrance stagger. Nothing here draws or touches the window, so all of it is unit-tested.
//!
//! Time is an `f64` of seconds since the card was created and every loop is a function of it (`cycle`), so a long-running window never
//! loses float precision and the particles need no per-frame state or allocation.
use crate::ui::liquid::{Feel, Flow};

/// What the sky card looks like: the forecast icon id of the MET client (`net::icon_for`) read as a scene.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scene {
    ClearDay,
    ClearNight,
    PartlyDay,
    PartlyNight,
    Cloudy,
    Fog,
    Rain,
    Sleet,
    Snow,
    Thunder,
}

impl Scene {
    /// The scene of an icon id (`clear-day`, `partly-night`, `cloud`, `rain`, ...). Unknown ids are `Cloudy`, like the static icon.
    pub fn of(icon: &str) -> Scene {
        match icon {
            "clear-day" => Scene::ClearDay,
            "clear-night" => Scene::ClearNight,
            "partly-day" => Scene::PartlyDay,
            "partly-night" => Scene::PartlyNight,
            "fog" => Scene::Fog,
            "rain" => Scene::Rain,
            "sleet" => Scene::Sleet,
            "snow" => Scene::Snow,
            "thunder" => Scene::Thunder,
            _ => Scene::Cloudy,
        }
    }

    pub fn night(self) -> bool {
        matches!(self, Scene::ClearNight | Scene::PartlyNight)
    }

    /// Falling things on the card behind the text: the kind of particle and how many (by intensity 0 light, 1 normal, 2 heavy).
    pub fn precip(self, level: u8) -> Option<(Fall, usize)> {
        let l = level.min(2) as usize;
        match self {
            Scene::Rain => Some((Fall::Rain, [6, 10, 16][l])),
            Scene::Thunder => Some((Fall::Rain, [10, 14, 18][l])),
            Scene::Snow => Some((Fall::Flake, [6, 10, 16][l])),
            Scene::Sleet => Some((Fall::Mixed, [8, 10, 14][l])),
            _ => None,
        }
    }

    /// Drops inside the icon itself (under its cloud): the kind and the count.
    pub fn icon_drops(self, level: u8) -> Option<(Fall, usize)> {
        let l = level.min(2) as usize;
        match self {
            Scene::Rain | Scene::Thunder => Some((Fall::Rain, [3, 4, 6][l])),
            Scene::Snow => Some((Fall::Flake, [3, 4, 5][l])),
            Scene::Sleet => Some((Fall::Mixed, 4)),
            _ => None,
        }
    }

    /// How fast the drops fall, in trips per second (a light shower drifts, a heavy one pours).
    pub fn fall_rate(self, level: u8) -> f64 {
        let l = level.min(2) as usize;
        match self {
            Scene::Snow => [0.10, 0.14, 0.2][l],
            Scene::Sleet => [0.3, 0.4, 0.5][l],
            _ => [0.55, 0.8, 1.2][l],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fall {
    Rain,
    Flake,
    /// even particles are rain, odd ones snow
    Mixed,
}

/// Loop position 0..1 of something that repeats every `period` seconds.
pub fn cycle(t: f64, period: f64) -> f32 {
    ((t / period).rem_euclid(1.0)) as f32
}

/// `sin` of a loop: -1..1.
pub fn wave(t: f64, period: f64, phase: f32) -> f32 {
    ((cycle(t, period) + phase) * std::f32::consts::TAU).sin()
}

/// 0..1 hash of (particle, salt): the same numbers every run (no RNG state, no seed to carry around).
pub fn h01(i: usize, salt: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x9E37_79B1) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297A_2D39);
    x ^= x >> 15;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// One particle at an instant: position inside the box (0..w, 0..h), opacity 0..1 (it fades in at the top and out at the bottom),
/// `fall` 0..1 along its trip and the kind (`Mixed` resolved).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Particle {
    pub x: f32,
    pub y: f32,
    pub a: f32,
    pub fall: f32,
    pub flake: bool,
}

/// Particle `i` of a field `w` x `h` at time `t`. `rate` = trips per second, `slant` = sideways travel over one trip as a fraction of `w`,
/// `sway` = px of flake wobble. Always inside the box, whatever `t` is.
pub fn particle(i: usize, t: f64, kind: Fall, rate: f64, w: f32, h: f32, slant: f32, sway: f32) -> Particle {
    let speed = 0.75 + 0.5 * h01(i, 3) as f64;
    let fall = (h01(i, 2) as f64 + t * rate * speed).rem_euclid(1.0) as f32;
    let flake = match kind {
        Fall::Rain => false,
        Fall::Flake => true,
        Fall::Mixed => i % 2 == 1,
    };
    let wob = if flake { sway * wave(t, 2.6 + 2.0 * h01(i, 4) as f64, h01(i, 5)) } else { 0.0 };
    let x = (h01(i, 1) * w + wob + slant * w * fall).rem_euclid(w.max(1.0));
    let a = (fall / 0.12).min(1.0) * ((1.0 - fall) / 0.25).min(1.0);
    Particle { x, y: fall * h, a: a.clamp(0.0, 1.0), fall, flake }
}

// ------------------------------------------------------------------------------------------------------------------ lightning

/// The flash schedule of a thunderstorm: one flash every 4 to 8 seconds (the first one 2 to 5 s after the start), each a soft double
/// pulse of 0.7 s. Deterministic from the seed.
#[derive(Clone, Debug)]
pub struct Lightning {
    rng: u32,
    /// seconds until the next flash starts (while idle)
    wait: f32,
    /// seconds since the running flash started
    age: Option<f32>,
}

/// How long a flash lasts, seconds.
pub const FLASH_S: f32 = 0.7;

fn bump(x: f32, c: f32, w: f32) -> f32 {
    let k = (1.0 - (x - c).abs() / w).max(0.0);
    k * k * (3.0 - 2.0 * k)
}

/// Brightness 0..1 of a flash `age` seconds after it started: a quick first pulse, a softer second one 0.4 s later (2.5 pulses a second at most).
pub fn flash_env(age: f32) -> f32 {
    if !(0.0..FLASH_S).contains(&age) {
        return 0.0;
    }
    bump(age, 0.07, 0.07).max(0.55 * bump(age, 0.42, 0.16))
}

impl Lightning {
    pub fn new(seed: u32) -> Lightning {
        let mut l = Lightning { rng: seed | 1, wait: 0.0, age: None };
        l.wait = 2.0 + 3.0 * l.next01();
        l
    }

    fn next01(&mut self) -> f32 {
        // xorshift32
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Advance by `dt` seconds (callers keep it small); returns the flash brightness now. Between flashes there are always >= 4 s of calm.
    pub fn step(&mut self, dt: f32) -> f32 {
        let mut dt = dt.max(0.0);
        if let Some(a) = self.age {
            let left = FLASH_S - a;
            if dt < left {
                self.age = Some(a + dt);
                return flash_env(a + dt);
            }
            dt -= left;
            self.age = None;
            // 4..8 s from the start of this flash to the start of the next
            self.wait = 4.0 + 4.0 * self.next01() - FLASH_S;
        }
        self.wait -= dt;
        if self.wait <= 0.0 {
            let a = (-self.wait).clamp(0.0, FLASH_S * 0.5);
            self.age = Some(a);
            return flash_env(a);
        }
        0.0
    }

    /// Is a flash running?
    pub fn flashing(&self) -> bool {
        self.age.is_some()
    }

    /// Brightness now, without advancing.
    pub fn level(&self) -> f32 {
        self.age.map_or(0.0, flash_env)
    }
}

// ------------------------------------------------------------------------------------------------------------------ the temperature counting

/// Splits "-4°" / "41°" / "12 °C" into (number, text after it). The prefix before the number is kept too.
pub fn parse_temp(s: &str) -> Option<(String, i32, String)> {
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let end = s[start..].find(|c: char| !c.is_ascii_digit()).map_or(s.len(), |e| start + e);
    let (pre, n) = (&s[..start], s[start..end].parse::<i32>().ok()?);
    let neg = pre.ends_with('-') || pre.ends_with('\u{2212}');
    let keep = pre.trim_end_matches(|c| c == '-' || c == '\u{2212}');
    Some((keep.to_string(), if neg { -n } else { n }, s[end..].to_string()))
}

/// The big temperature: counts to the new value on a liquid flow (when the tab opens it counts up from 0). `text()` is the number as shown.
pub struct Counter {
    flow: Flow,
    pre: String,
    post: String,
    target: Option<i32>,
    shown: Option<i32>,
    buf: String,
    raw: String,
}

impl Counter {
    pub fn new() -> Counter {
        Counter { flow: Flow::px(0.0, Feel { settle_ms: 900.0, damping: 1.0 }), pre: String::new(), post: String::new(), target: None, shown: None, buf: String::new(), raw: String::new() }
    }

    /// The forecast says `text` now (called every frame; cheap when unchanged).
    pub fn set(&mut self, text: &str) {
        if self.raw == text {
            return;
        }
        self.raw.clear();
        self.raw.push_str(text);
        match parse_temp(text) {
            Some((pre, n, post)) => {
                self.pre = pre;
                self.post = post;
                if self.target.is_none() {
                    self.flow.jump(0.0);
                }
                self.target = Some(n);
                self.flow.to(n as f32);
            }
            None => {
                self.target = None;
                self.shown = None;
            }
        }
        self.refresh_text(true);
    }

    /// The tab opens: count up from zero again.
    pub fn rewind(&mut self) {
        if let Some(n) = self.target {
            self.flow.jump(0.0);
            self.flow.to(n as f32);
            self.refresh_text(true);
        }
    }

    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.flow.step(dt_ms);
        self.refresh_text(false);
        self.flow.moving()
    }

    fn refresh_text(&mut self, force: bool) {
        let Some(t) = self.target else { return };
        let v = if self.flow.moving() { self.flow.at().round() as i32 } else { t };
        if force || self.shown != Some(v) {
            self.shown = Some(v);
            self.buf.clear();
            use std::fmt::Write;
            let _ = write!(self.buf, "{}{}{}", self.pre, v, self.post);
        }
    }

    /// What to draw: the counting number, or the model's own text when it holds no number.
    pub fn text(&self) -> &str {
        if self.target.is_some() {
            &self.buf
        } else {
            &self.raw
        }
    }

    pub fn moving(&self) -> bool {
        self.flow.moving()
    }
}

// ------------------------------------------------------------------------------------------------------------------ refresh spin

/// The refresh icon: turns while a refresh runs, then glides on to the next full turn so it never stops at a tilt.
#[derive(Default)]
pub struct Spin {
    pub deg: f32,
}

impl Spin {
    /// Returns true while it turns.
    pub fn step(&mut self, on: bool, dt_s: f32) -> bool {
        if on {
            self.deg = (self.deg + 420.0 * dt_s).rem_euclid(360.0);
            return true;
        }
        if self.deg == 0.0 {
            return false;
        }
        let left = 360.0 - self.deg;
        let adv = (left * 9.0 * dt_s).max(40.0 * dt_s);
        self.deg = if adv >= left || left < 0.5 { 0.0 } else { self.deg + adv };
        self.deg != 0.0
    }
}

// ------------------------------------------------------------------------------------------------------------------ entrance

/// Elements of the card that rise in one after the other.
pub const RISE_N: usize = 9;
const RISE_GAP_MS: f32 = 45.0;

/// Short staggered rise of the card's texts and tiles: each element is a `Flow` 0 -> 1 that starts `i * 45 ms` after the card opened.
pub struct Rise {
    f: [Flow; RISE_N],
    t: f32,
}

impl Rise {
    pub fn new() -> Rise {
        Rise { f: [Flow::new(1.0, Feel::FLOW); RISE_N], t: 1e9 }
    }

    /// The card appears: everything drops to its start state and rises again.
    pub fn rewind(&mut self) {
        for f in &mut self.f {
            f.jump(0.0);
            f.to(0.0);
        }
        self.t = 0.0;
    }

    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.t += dt_ms as f32;
        let mut busy = false;
        for (i, f) in self.f.iter_mut().enumerate() {
            f.to(if self.t >= i as f32 * RISE_GAP_MS { 1.0 } else { 0.0 });
            f.step(dt_ms);
            busy |= f.moving();
        }
        busy
    }

    /// (opacity, dy px) of element `i`: settled = (1, 0).
    pub fn at(&self, i: usize) -> (f32, f32) {
        let p = self.f[i.min(RISE_N - 1)].at();
        (p.clamp(0.0, 1.0), (1.0 - p) * 9.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    #[test]
    fn every_icon_id_of_the_met_client_maps_to_its_own_scene() {
        // the ids net::icon_for can answer
        let want = [
            ("clear-day", Scene::ClearDay),
            ("clear-night", Scene::ClearNight),
            ("partly-day", Scene::PartlyDay),
            ("partly-night", Scene::PartlyNight),
            ("cloud", Scene::Cloudy),
            ("fog", Scene::Fog),
            ("rain", Scene::Rain),
            ("sleet", Scene::Sleet),
            ("snow", Scene::Snow),
            ("thunder", Scene::Thunder),
            ("", Scene::Cloudy),
            ("surprise", Scene::Cloudy),
        ];
        for (id, s) in want {
            assert_eq!(Scene::of(id), s, "{id}");
        }
        for code in ["clearsky_day", "fair_night", "partlycloudy_night", "cloudy", "fog", "lightrain", "heavyrainshowers_day", "sleet", "heavysnow", "rainandthunder"] {
            let _ = Scene::of(crate::net::icon_for(code)); // total
        }
        assert_eq!(Scene::of(crate::net::icon_for("partlycloudy_night")), Scene::PartlyNight);
        assert_eq!(Scene::of(crate::net::icon_for("heavyrainandthunder")), Scene::Thunder);
        assert!(Scene::ClearNight.night() && Scene::PartlyNight.night() && !Scene::ClearDay.night());
    }

    #[test]
    fn only_wet_scenes_have_precipitation_and_heavier_means_more_and_faster() {
        for s in [Scene::ClearDay, Scene::ClearNight, Scene::PartlyDay, Scene::PartlyNight, Scene::Cloudy, Scene::Fog] {
            assert!(s.precip(1).is_none() && s.icon_drops(1).is_none(), "{s:?}");
        }
        for s in [Scene::Rain, Scene::Thunder, Scene::Snow, Scene::Sleet] {
            let (n0, n2) = (s.precip(0).unwrap().1, s.precip(2).unwrap().1);
            assert!(n0 < n2 && n2 <= 24, "{s:?} {n0} {n2}");
            assert!(s.fall_rate(0) < s.fall_rate(2));
            assert!(s.icon_drops(2).unwrap().1 <= 8);
        }
        assert!(Scene::Snow.fall_rate(1) < Scene::Rain.fall_rate(1), "flakes drift slower than rain");
        assert_eq!(Scene::Rain.precip(9), Scene::Rain.precip(2), "an unknown level is clamped");
    }

    #[test]
    fn particles_stay_inside_the_card_and_fade_at_both_ends_for_any_time() {
        let (w, h) = (214.0, 340.0);
        for kind in [Fall::Rain, Fall::Flake, Fall::Mixed] {
            for i in 0..24 {
                for k in 0..400 {
                    let t = k as f64 * 0.137 + 1.0e7; // far into a long run
                    let p = particle(i, t, kind, 1.2, w, h, 0.12, 6.0);
                    assert!((0.0..=w).contains(&p.x) && (0.0..=h).contains(&p.y), "{p:?}");
                    assert!((0.0..=1.0).contains(&p.a));
                    if p.fall < 0.01 || p.fall > 0.99 {
                        assert!(p.a < 0.1, "invisible at the ends: {p:?}");
                    }
                }
            }
        }
        assert!(!particle(0, 3.0, Fall::Rain, 1.0, 10.0, 10.0, 0.0, 0.0).flake);
        assert!(particle(0, 3.0, Fall::Flake, 1.0, 10.0, 10.0, 0.0, 0.0).flake);
        assert!(particle(1, 3.0, Fall::Mixed, 1.0, 10.0, 10.0, 0.0, 0.0).flake && !particle(2, 3.0, Fall::Mixed, 1.0, 10.0, 10.0, 0.0, 0.0).flake);
    }

    #[test]
    fn particles_are_deterministic_spread_out_and_move_down() {
        let a = particle(5, 12.3, Fall::Rain, 1.0, 100.0, 100.0, 0.0, 0.0);
        assert_eq!(a, particle(5, 12.3, Fall::Rain, 1.0, 100.0, 100.0, 0.0, 0.0));
        let xs: Vec<f32> = (0..16).map(|i| particle(i, 0.0, Fall::Rain, 1.0, 100.0, 100.0, 0.0, 0.0).x).collect();
        let (lo, hi) = (xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max));
        assert!(hi - lo > 50.0, "spread over the width: {lo}..{hi}");
        let (p0, p1) = (particle(3, 0.20, Fall::Rain, 1.0, 100.0, 100.0, 0.0, 0.0), particle(3, 0.21, Fall::Rain, 1.0, 100.0, 100.0, 0.0, 0.0));
        assert!(p1.y > p0.y || p1.fall < p0.fall, "it falls");
    }

    /// Flash start times (s) over `secs` of simulated time at `fps`.
    fn flashes(seed: u32, secs: f32, fps: f32) -> Vec<f32> {
        let mut l = Lightning::new(seed);
        let (mut t, mut was, mut out) = (0.0f32, false, vec![]);
        while t < secs {
            let e = l.step(1.0 / fps);
            t += 1.0 / fps;
            assert!((0.0..=1.0).contains(&e), "brightness {e}");
            if l.flashing() && !was {
                out.push(t);
            }
            was = l.flashing();
        }
        out
    }

    #[test]
    fn lightning_comes_every_four_to_eight_seconds_and_is_deterministic() {
        let a = flashes(7, 600.0, 30.0);
        assert!(a.len() > 60 && a.len() < 150, "{} flashes in 10 minutes", a.len());
        assert!(a[0] > 1.9 && a[0] < 5.2, "first flash at {}", a[0]);
        for w in a.windows(2) {
            let gap = w[1] - w[0];
            assert!(gap >= 3.95 && gap <= 8.2, "gap {gap}");
        }
        assert_eq!(a, flashes(7, 600.0, 30.0), "same seed, same storm");
        assert_ne!(a, flashes(8, 600.0, 30.0), "another seed, another storm");
        assert!(flashes(7, 600.0, 120.0).len().abs_diff(a.len()) <= 2, "the frame rate does not change the rate of flashes");
    }

    #[test]
    fn a_flash_is_soft_never_a_strobe_and_a_long_frame_does_not_stack_flashes() {
        // at most ~2.5 pulses a second: the envelope has two peaks 0.35 s apart, nothing faster
        let mut peaks = 0;
        let mut prev = [0.0f32; 2];
        for k in 0..((FLASH_S * 1000.0) as usize) {
            let e = flash_env(k as f32 / 1000.0);
            if prev[1] > prev[0] && prev[1] >= e && prev[1] > 0.2 {
                peaks += 1;
            }
            prev = [prev[1], e];
        }
        assert_eq!(peaks, 2);
        assert_eq!(flash_env(-0.1), 0.0);
        assert_eq!(flash_env(FLASH_S + 0.01), 0.0);
        let mut l = Lightning::new(3);
        // one 30 s hitch: at most a single flash, and the next is not immediate
        let mut starts = 0;
        for dt in [30.0, 0.016, 0.016, 0.016] {
            let was = l.flashing();
            l.step(dt);
            starts += (l.flashing() && !was) as u32;
        }
        assert_eq!(starts, 1);
        while l.flashing() {
            l.step(0.01);
        }
        let mut gap = 0.0;
        while !l.flashing() {
            l.step(0.01);
            gap += 0.01;
            assert!(gap < 8.0);
        }
        assert!(gap > 3.0, "calm after the flash: {gap}");
    }

    #[test]
    fn temp_text_splits_into_number_and_the_rest() {
        assert_eq!(parse_temp("5°"), Some((String::new(), 5, "°".into())));
        assert_eq!(parse_temp("-4°"), Some((String::new(), -4, "°".into())));
        assert_eq!(parse_temp("\u{2212}12°C"), Some((String::new(), -12, "°C".into())));
        assert_eq!(parse_temp("41 °F"), Some((String::new(), 41, " °F".into())));
        assert_eq!(parse_temp("--"), None);
        assert_eq!(parse_temp(""), None);
    }

    #[test]
    fn the_temperature_counts_up_from_zero_lands_exactly_and_then_idles() {
        let _m = motion_on();
        let mut c = Counter::new();
        c.set("21°");
        assert!(c.moving());
        let mut seen = vec![];
        for _ in 0..600 {
            c.step(8.0);
            let n = parse_temp(c.text()).unwrap().1;
            if seen.last() != Some(&n) {
                seen.push(n);
            }
        }
        assert_eq!(c.text(), "21°");
        assert!(!c.moving(), "settled = idle");
        assert!(seen.len() > 8 && seen.windows(2).all(|w| w[1] >= w[0] || w[0] > 21), "counts through the numbers: {seen:?}");
        assert!(seen.iter().all(|&n| (-1..=23).contains(&n)), "does not wander off: {seen:?}");
        // a new value counts from where it is (and down this time), a negative one lands on the minus sign
        c.set("-4°");
        c.step(8.0);
        assert!(c.moving());
        for _ in 0..600 {
            c.step(8.0);
        }
        assert_eq!(c.text(), "-4°");
        // the tab opens again: from zero up to the same value
        c.rewind();
        assert_eq!(c.text(), "0°");
        for _ in 0..600 {
            c.step(8.0);
        }
        assert_eq!(c.text(), "-4°");
        // text without a number is shown as it is
        c.set("--");
        assert_eq!(c.text(), "--");
    }

    #[test]
    fn counting_under_reduced_motion_is_instant() {
        let _lock = crate::ui::host::testing::motion_lock();
        glasskit::ease::force_reduced(Some(true));
        let mut c = Counter::new();
        c.set("17°");
        c.step(16.0);
        assert_eq!(c.text(), "17°");
        assert!(!c.moving());
        glasskit::ease::force_reduced(None);
    }

    #[test]
    fn the_refresh_icon_turns_while_busy_and_comes_to_rest_upright() {
        let mut s = Spin::default();
        assert!(!s.step(false, 0.016), "idle stays idle");
        for _ in 0..30 {
            assert!(s.step(true, 0.016));
        }
        assert!(s.deg > 0.0);
        let mut n = 0;
        while s.step(false, 0.016) {
            n += 1;
            assert!(n < 300, "it must come to rest");
        }
        assert_eq!(s.deg, 0.0);
    }

    #[test]
    fn the_card_rises_in_one_element_after_the_other() {
        let _m = motion_on();
        let mut r = Rise::new();
        assert_eq!(r.at(3), (1.0, 0.0), "a card that is already up stays up");
        assert!(!r.step(16.0));
        r.rewind();
        assert_eq!(r.at(0).0, 0.0);
        for _ in 0..6 {
            r.step(16.0);
        }
        let (first, last) = (r.at(0), r.at(RISE_N - 1));
        assert!(first.0 > last.0 + 0.1, "the first one is ahead: {first:?} vs {last:?}");
        assert!(first.1 < 9.0);
        let mut n = 0;
        while r.step(8.0) {
            n += 1;
            assert!(n < 600);
        }
        for i in 0..RISE_N {
            assert_eq!(r.at(i), (1.0, 0.0));
        }
    }

    #[test]
    fn rise_lands_at_once_under_reduced_motion() {
        let _lock = crate::ui::host::testing::motion_lock();
        glasskit::ease::force_reduced(Some(true));
        let mut r = Rise::new();
        r.rewind();
        r.step(16.0);
        r.step(16.0 * 60.0);
        assert_eq!(r.at(8), (1.0, 0.0));
        glasskit::ease::force_reduced(None);
    }

    #[test]
    fn loops_are_exact_at_any_age() {
        assert!((cycle(1.0e9 + 2.5, 5.0) - 0.5).abs() < 1e-3);
        assert!((wave(0.0, 4.0, 0.0)).abs() < 1e-6);
        assert!((wave(1.0, 4.0, 0.0) - 1.0).abs() < 1e-5);
        for i in 0..50 {
            for s in 0..5 {
                let v = h01(i, s);
                assert!((0.0..1.0).contains(&v));
            }
        }
    }
}
