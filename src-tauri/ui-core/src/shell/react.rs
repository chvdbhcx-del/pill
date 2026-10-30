//! Edge reactions: the `REACT` preset table and the priority resolver of pill.js (`PRIO`, `react`, `unreact`, `setReaction`, `resolve`, `poke`).
//!
//! Sources (agents, media, shelf, reminders...) ask for a preset with `react(source, name, { ttl, color })`; the highest priority wins,
//! a finished transient falls back to whatever is still active, several sources never fight. `set_reaction` is the low-level setter /
//! preview and temporarily overrides the resolver. After 5 minutes without activity (and the pointer away) the pill goes to `sleep`;
//! `poke()` wakes it. The resolver owns its three timers (resolve re-check, sleep, preview end) as monotonic deadlines the host drives
//! through `on_time`. Every method that can change what is shown returns `Some(Paint)`; the shell then restarts the liquid animation
//! (`LQ.rt = 0`) and notifies observers (`Pill.onReaction`).

/// The seven named presets (`Pill.reactions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Preset {
    #[default]
    Calm,
    Music,
    Thinking,
    Done,
    Attention,
    File,
    Sleep,
}

impl Preset {
    pub const ALL: [Preset; 7] = [Preset::Calm, Preset::Music, Preset::Thinking, Preset::Done, Preset::Attention, Preset::File, Preset::Sleep];
    pub fn name(self) -> &'static str {
        match self {
            Preset::Calm => "calm",
            Preset::Music => "music",
            Preset::Thinking => "thinking",
            Preset::Done => "done",
            Preset::Attention => "attention",
            Preset::File => "file",
            Preset::Sleep => "sleep",
        }
    }
    pub fn parse(s: &str) -> Option<Preset> {
        Preset::ALL.iter().copied().find(|p| p.name() == s)
    }
    /// `PRIO`: attention 6, thinking 5, file 4, done 3, music 2, sleep 1, calm 0.
    pub fn prio(self) -> u8 {
        match self {
            Preset::Attention => 6,
            Preset::Thinking => 5,
            Preset::File => 4,
            Preset::Done => 3,
            Preset::Music => 2,
            Preset::Sleep => 1,
            Preset::Calm => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveKind {
    Sine,
    Beat,
    Settle,
    Zig,
    Smile,
    Bowl,
}

/// `ball`: the small black ball hanging under the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallKind {
    None,
    /// `true`: whole time the reaction is active
    Always,
    /// `'start'`: only the first 2.6 s
    Start,
}

/// One row of the JS `REACT` table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReactDef {
    /// preset colour as CSS (`''` = none: the accent / edge colour is used)
    pub col: &'static str,
    pub kind: WaveKind,
    pub wl: f64,
    pub spd: f64,
    /// `amp` (1 when the JS row has none)
    pub amp: f64,
    pub a0: f64,
    pub tau: f64,
    pub ball: BallKind,
    pub noline: bool,
}

const D: ReactDef = ReactDef { col: "", kind: WaveKind::Sine, wl: 0.0, spd: 0.0, amp: 1.0, a0: 0.0, tau: 0.0, ball: BallKind::None, noline: false };

/// `REACT[name]`. Measured on the Character page of the real app: colour, amplitude, wavelength, motion.
pub fn def(p: Preset) -> ReactDef {
    match p {
        // idle: 2 long waves drifting left 20-30 px/s, +-2.5 px (+-3.5 expanded)
        Preset::Calm => ReactDef { kind: WaveKind::Sine, wl: 170.0, spd: 25.0, amp: 1.0, ..D },
        // coral wave pulsing every 1.1 s, 1.5 -> 6.5 px (audio:bands spikes replace it when they arrive)
        Preset::Music => ReactDef { col: "#d8836a", kind: WaveKind::Beat, wl: 22.0, spd: 30.0, ..D },
        // lavender slow travelling wave + a small black ball hanging under it
        Preset::Thinking => ReactDef { col: "#9e8fe8", kind: WaveKind::Sine, wl: 64.0, spd: 46.0, amp: 1.15, ball: BallKind::Always, ..D },
        // green, irregular, relaxes to nearly flat in ~2.5 s
        Preset::Done => ReactDef { col: "#7fae45", kind: WaveKind::Settle, wl: 46.0, spd: 34.0, a0: 8.0, tau: 0.9, ball: BallKind::Start, ..D },
        // amber dense zig-zag shimmer
        Preset::Attention => ReactDef { col: "#e8a040", kind: WaveKind::Zig, wl: 11.0, amp: 1.2, ..D },
        // steel blue, bumpy at first, then a smooth smile curve
        Preset::File => ReactDef { col: "#5a78a8", kind: WaveKind::Smile, wl: 52.0, tau: 0.7, ..D },
        // no line: a smooth sagging bowl
        Preset::Sleep => ReactDef { kind: WaveKind::Bowl, noline: true, ..D },
    }
}

/// Sleep after this long without activity (pointer away, pill collapsed).
pub const SLEEP_AFTER_MS: f64 = 300_000.0;
/// `poke()` is a no-op when the previous one was less than this ago (unless asleep).
pub const POKE_GAP_MS: f64 = 1000.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub name: Preset,
    pub color: String,
    /// wall ms; 0 = stays until `unreact`
    pub until: i64,
    tag: u64,
}

/// What the liquid edge has to switch to (a change of the effective reaction, or a fresh event of the same preset).
#[derive(Debug, Clone, PartialEq)]
pub struct Paint {
    pub name: Preset,
    pub color: String,
}

/// `{ ttlMs, color }` of `Pill.react` / `Pill.setReaction`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReactOpts {
    pub ttl_ms: i64,
    pub color: String,
}
impl ReactOpts {
    pub fn ttl(ttl_ms: i64) -> ReactOpts {
        ReactOpts { ttl_ms, color: String::new() }
    }
}

/// Current time in both bases: `mono` = performance.now(), `wall` = Date.now().
#[derive(Debug, Clone, Copy)]
pub struct Times {
    pub mono: f64,
    pub wall: i64,
}

#[derive(Debug, Clone)]
pub struct Reactions {
    /// insertion ordered like the JS `Map` (ties keep the earlier source)
    rx: Vec<(String, Entry)>,
    /// `preview !== 0`: `set_reaction` with a ttl is showing
    preview: bool,
    next_tag: u64,
    rtag: Option<u64>,
    cur_name: Preset,
    cur_color: String,
    last_poke: f64,
    rx_deadline: Option<f64>,
    sleep_deadline: Option<f64>,
    preview_deadline: Option<f64>,
}

impl Default for Reactions {
    fn default() -> Self {
        Reactions::new()
    }
}

impl Reactions {
    pub fn new() -> Reactions {
        Reactions {
            rx: Vec::new(),
            preview: false,
            next_tag: 1,
            rtag: None,
            cur_name: Preset::Calm,
            cur_color: String::new(),
            // deviation: the JS had performance.now() since page load here; -inf lets the very first poke arm the sleep timer
            last_poke: f64::NEG_INFINITY,
            rx_deadline: None,
            sleep_deadline: None,
            preview_deadline: None,
        }
    }

    /// The effective reaction as last painted (`LQ.react`, `LQ.rcol`).
    pub fn current(&self) -> (Preset, &str) {
        (self.cur_name, &self.cur_color)
    }
    pub fn has(&self, source: &str) -> bool {
        self.rx.iter().any(|(k, _)| k == source)
    }
    pub fn get(&self, source: &str) -> Option<&Entry> {
        self.rx.iter().find(|(k, _)| k == source).map(|(_, e)| e)
    }
    pub fn is_preview(&self) -> bool {
        self.preview
    }
    pub fn sources(&self) -> Vec<&str> {
        self.rx.iter().map(|(k, _)| k.as_str()).collect()
    }
    /// Earliest pending deadline (monotonic ms) among the resolver's timers: the host should call `on_time` then.
    pub fn next_deadline(&self) -> Option<f64> {
        [self.rx_deadline, self.sleep_deadline, self.preview_deadline].into_iter().flatten().fold(None, |a: Option<f64>, d| Some(a.map_or(d, |x| x.min(d))))
    }

    fn fresh_tag(&mut self) -> u64 {
        self.next_tag += 1;
        self.next_tag
    }

    /// `paintReact`: a fresh event of the same preset (different tag) restarts the animation.
    fn paint(&mut self, name: Preset, color: &str, tag: Option<u64>) -> Option<Paint> {
        if name == self.cur_name && color == self.cur_color && tag == self.rtag {
            return None;
        }
        self.rtag = tag;
        self.cur_name = name;
        self.cur_color = color.to_string();
        Some(Paint { name, color: color.to_string() })
    }

    /// `resolve()`: drop expired entries, pick the highest priority, re-arm the re-check timer.
    fn resolve(&mut self, t: &Times) -> Option<Paint> {
        self.rx_deadline = None;
        let mut next = f64::INFINITY;
        self.rx.retain(|(_, e)| !(e.until != 0 && e.until <= t.wall));
        let mut best: Option<usize> = None;
        for (i, (_, e)) in self.rx.iter().enumerate() {
            if e.until != 0 {
                next = next.min(e.until as f64);
            }
            if best.is_none_or(|b| e.name.prio() > self.rx[b].1.name.prio()) {
                best = Some(i);
            }
        }
        if next.is_finite() {
            self.rx_deadline = Some(t.mono + (next - t.wall as f64).max(30.0) + 5.0);
        }
        if self.preview {
            return None;
        }
        match best {
            Some(i) => {
                let (name, color, tag) = {
                    let e = &self.rx[i].1;
                    (e.name, e.color.clone(), e.tag)
                };
                self.paint(name, &color, Some(tag))
            }
            None => self.paint(Preset::Calm, "", None),
        }
    }

    /// Activity or the pointer: wake from sleep and restart the idle timer (5 min with nothing happening and the pointer away -> sleep).
    pub fn poke(&mut self, t: &Times) -> Option<Paint> {
        if t.mono - self.last_poke < POKE_GAP_MS && !self.has("sleep") {
            return None;
        }
        self.last_poke = t.mono;
        self.sleep_deadline = Some(t.mono + SLEEP_AFTER_MS);
        if self.delete("sleep") {
            return self.resolve(t);
        }
        None
    }

    fn delete(&mut self, source: &str) -> bool {
        let n = self.rx.len();
        self.rx.retain(|(k, _)| k != source);
        self.rx.len() != n
    }

    /// `Pill.react(source, name, opts)`. `quiet`: quiet hours are on (muted sources are un-reacted instead).
    pub fn react(&mut self, source: &str, name: &str, o: &ReactOpts, t: &Times, quiet: bool) -> Option<Paint> {
        let name = Preset::parse(name)?;
        if source.is_empty() {
            return None;
        }
        if quiet && crate::shell::quiet::is_quiet_source(source) {
            return self.unreact(source, t);
        }
        if let Some(old) = self.get(source) {
            // same standing state again (agents:changed, media:changed)
            if o.ttl_ms == 0 && old.until == 0 && old.name == name && old.color == o.color {
                return None;
            }
        }
        let mut out = if name != Preset::Sleep { self.poke(t) } else { None };
        let tag = self.fresh_tag();
        let e = Entry { name, color: o.color.clone(), until: if o.ttl_ms != 0 { t.wall + o.ttl_ms } else { 0 }, tag };
        match self.rx.iter_mut().find(|(k, _)| k == source) {
            Some((_, slot)) => *slot = e,
            None => self.rx.push((source.to_string(), e)),
        }
        out = self.resolve(t).or(out);
        out
    }

    pub fn unreact(&mut self, source: &str, t: &Times) -> Option<Paint> {
        if self.delete(source) {
            self.resolve(t)
        } else {
            None
        }
    }

    /// Keep an existing entry alive (the `audio:bands` handler: real sound = music baseline, re-checked by the resolver timer).
    pub fn extend(&mut self, source: &str, until: i64) -> bool {
        match self.rx.iter_mut().find(|(k, _)| k == source) {
            Some((_, e)) => {
                e.until = until;
                true
            }
            None => false,
        }
    }

    /// `Pill.setReaction(name, opts)`: low-level setter / preview. Unknown names fall back to calm. With a ttl the resolved state returns
    /// afterwards. Always repaints (fresh tag). Returns the preset actually applied and the paint.
    pub fn set_reaction(&mut self, name: &str, o: &ReactOpts, t: &Times) -> (Preset, Option<Paint>) {
        let p = Preset::parse(name).unwrap_or(Preset::Calm);
        self.preview_deadline = None;
        self.preview = o.ttl_ms != 0 && p != Preset::Calm;
        let tag = self.fresh_tag();
        let paint = self.paint(p, &o.color, Some(tag));
        if self.preview {
            self.preview_deadline = Some(t.mono + o.ttl_ms as f64);
        }
        (p, paint)
    }

    /// Fire every due timer. `busy`: the pointer is on the pill or it is not collapsed (the sleep timer then only re-arms).
    pub fn on_time(&mut self, t: &Times, busy: bool) -> Option<Paint> {
        let mut out = None;
        if self.preview_deadline.is_some_and(|d| d <= t.mono) {
            self.preview_deadline = None;
            self.preview = false;
            self.rtag = Some(self.fresh_tag());
            out = self.resolve(t).or(out);
        }
        if self.rx_deadline.is_some_and(|d| d <= t.mono) {
            out = self.resolve(t).or(out);
        }
        if self.sleep_deadline.is_some_and(|d| d <= t.mono) {
            self.sleep_deadline = None;
            if busy {
                out = self.poke(t).or(out);
            } else {
                out = self.react("sleep", "sleep", &ReactOpts::default(), t, false).or(out);
            }
        }
        out
    }

    /// `pill:hide` and friends: forget the timers (sources stay).
    pub fn cancel_timers(&mut self) {
        self.rx_deadline = None;
        self.sleep_deadline = None;
        self.preview_deadline = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(mono: f64, wall: i64) -> Times {
        Times { mono, wall }
    }
    fn name(p: &Option<Paint>) -> Option<&'static str> {
        p.as_ref().map(|p| p.name.name())
    }

    #[test]
    fn table_matches_the_js_react_rows() {
        let m = def(Preset::Music);
        assert_eq!((m.col, m.kind, m.wl, m.spd), ("#d8836a", WaveKind::Beat, 22.0, 30.0));
        let th = def(Preset::Thinking);
        assert_eq!((th.col, th.wl, th.spd, th.amp, th.ball), ("#9e8fe8", 64.0, 46.0, 1.15, BallKind::Always));
        let d = def(Preset::Done);
        assert_eq!((d.kind, d.a0, d.tau, d.ball), (WaveKind::Settle, 8.0, 0.9, BallKind::Start));
        assert_eq!((def(Preset::Attention).wl, def(Preset::Attention).amp), (11.0, 1.2));
        assert_eq!((def(Preset::File).tau, def(Preset::File).col), (0.7, "#5a78a8"));
        assert!(def(Preset::Sleep).noline);
        assert_eq!(def(Preset::Calm).col, "");
        assert_eq!(def(Preset::Calm).amp, 1.0);
    }

    #[test]
    fn priorities() {
        let order = [Preset::Attention, Preset::Thinking, Preset::File, Preset::Done, Preset::Music, Preset::Sleep, Preset::Calm];
        for w in order.windows(2) {
            assert!(w[0].prio() > w[1].prio());
        }
        assert_eq!(Preset::parse("thinking"), Some(Preset::Thinking));
        assert_eq!(Preset::parse("nope"), None);
        for p in Preset::ALL {
            assert_eq!(Preset::parse(p.name()), Some(p));
        }
    }

    #[test]
    fn highest_priority_wins_and_falls_back() {
        let mut r = Reactions::new();
        let now = t(5000.0, 1_000_000);
        assert_eq!(name(&r.react("media", "music", &ReactOpts::default(), &now, false)), Some("music"));
        assert_eq!(name(&r.react("agents", "thinking", &ReactOpts::default(), &now, false)), Some("thinking"));
        // lower priority arriving later does not take over
        assert_eq!(r.react("shelf", "file", &ReactOpts::default(), &now, false), None);
        assert_eq!(r.current().0, Preset::Thinking);
        assert_eq!(name(&r.unreact("agents", &now)), Some("file")); // falls back to the next best, not to music
        assert_eq!(name(&r.unreact("shelf", &now)), Some("music"));
        assert_eq!(name(&r.unreact("media", &now)), Some("calm"));
        assert_eq!(r.unreact("media", &now), None); // nothing left to remove
    }

    #[test]
    fn ties_keep_the_earlier_source() {
        let mut r = Reactions::new();
        let now = t(5000.0, 1);
        r.react("a", "done", &ReactOpts { ttl_ms: 0, color: "#111111".into() }, &now, false);
        let p = r.react("b", "done", &ReactOpts { ttl_ms: 0, color: "#222222".into() }, &now, false);
        assert_eq!(p, None); // `>` not `>=`: a (earlier) keeps the win, the painted entry is unchanged
        assert_eq!(r.current().1, "#111111");
    }

    #[test]
    fn ttl_expires_and_falls_back_via_the_timer() {
        let mut r = Reactions::new();
        let n0 = t(10_000.0, 1_000_000);
        r.react("media", "music", &ReactOpts::default(), &n0, false);
        let p = r.react("shelf", "file", &ReactOpts::ttl(3000), &n0, false);
        assert_eq!(name(&p), Some("file"));
        // the re-check timer: until - now (3000) + 5
        let d = r.next_deadline().unwrap();
        assert_eq!(d, 10_000.0 + 3000.0 + 5.0);
        // not yet
        assert_eq!(r.on_time(&t(12_000.0, 1_002_000), false), None);
        assert_eq!(r.current().0, Preset::File);
        // due
        let p = r.on_time(&t(13_005.0, 1_003_005), false);
        assert_eq!(name(&p), Some("music"));
        assert!(!r.has("shelf"));
    }

    #[test]
    fn resolve_timer_has_a_30ms_floor() {
        let mut r = Reactions::new();
        let n = t(100.0, 1000);
        r.react("x", "done", &ReactOpts::ttl(5), &n, false);
        assert_eq!(r.next_deadline(), Some(100.0 + 30.0 + 5.0));
    }

    #[test]
    fn same_standing_state_is_ignored_but_changes_and_ttls_repaint() {
        let mut r = Reactions::new();
        let n = t(5000.0, 1);
        assert!(r.react("agents", "thinking", &ReactOpts::default(), &n, false).is_some());
        assert_eq!(r.react("agents", "thinking", &ReactOpts::default(), &n, false), None);
        // a colour change is a change
        assert!(r.react("agents", "thinking", &ReactOpts { ttl_ms: 0, color: "#fff".into() }, &n, false).is_some());
        // a ttl event of the same preset is a fresh event and restarts the animation
        assert!(r.react("agents", "thinking", &ReactOpts::ttl(1000), &n, false).is_some());
        assert!(r.react("agents", "thinking", &ReactOpts::ttl(1000), &n, false).is_some());
    }

    #[test]
    fn unknown_names_and_empty_sources_are_ignored() {
        let mut r = Reactions::new();
        let n = t(5000.0, 1);
        assert_eq!(r.react("a", "bogus", &ReactOpts::default(), &n, false), None);
        assert_eq!(r.react("", "done", &ReactOpts::default(), &n, false), None);
        assert!(r.sources().is_empty());
    }

    #[test]
    fn quiet_hours_unreact_muted_sources_only() {
        let mut r = Reactions::new();
        let n = t(5000.0, 1);
        r.react("copy", "done", &ReactOpts::default(), &n, false);
        r.react("media", "music", &ReactOpts::default(), &n, false);
        assert_eq!(r.current().0, Preset::Done);
        // quiet: copy is muted (removed), media (not a quiet source) is unaffected
        let p = r.react("copy", "done", &ReactOpts::default(), &n, true);
        assert_eq!(name(&p), Some("music"));
        assert!(!r.has("copy"));
        assert!(r.react("media", "attention", &ReactOpts::default(), &n, true).is_some());
    }

    #[test]
    fn preview_overrides_the_resolver_and_returns() {
        let mut r = Reactions::new();
        let n0 = t(10_000.0, 1_000_000);
        r.react("media", "music", &ReactOpts::default(), &n0, false);
        let (p, paint) = r.set_reaction("attention", &ReactOpts::ttl(6000), &n0);
        assert_eq!(p, Preset::Attention);
        assert_eq!(name(&paint), Some("attention"));
        assert!(r.is_preview());
        // a higher-priority source arriving during the preview does not repaint
        assert_eq!(r.react("agents", "thinking", &ReactOpts::default(), &n0, false), None);
        assert_eq!(r.current().0, Preset::Attention);
        // preview ends: the resolved state (thinking beats music) comes back
        let paint = r.on_time(&t(16_000.0, 1_006_000), false);
        assert_eq!(name(&paint), Some("thinking"));
        assert!(!r.is_preview());
    }

    #[test]
    fn preview_calm_or_without_ttl_is_a_plain_set() {
        let mut r = Reactions::new();
        let n = t(5000.0, 1);
        r.react("media", "music", &ReactOpts::default(), &n, false);
        let (_, paint) = r.set_reaction("calm", &ReactOpts::ttl(6000), &n);
        assert_eq!(name(&paint), Some("calm"));
        assert!(!r.is_preview());
        let (p, _) = r.set_reaction("garbage", &ReactOpts::default(), &n);
        assert_eq!(p, Preset::Calm); // unknown -> calm
        // setting the same preset again still repaints (fresh tag)
        assert!(r.set_reaction("calm", &ReactOpts::default(), &n).1.is_some());
    }

    #[test]
    fn preview_end_repaints_even_when_the_resolved_state_is_the_same() {
        let mut r = Reactions::new();
        let n0 = t(5000.0, 1);
        r.react("a", "done", &ReactOpts::default(), &n0, false);
        r.set_reaction("done", &ReactOpts::ttl(1000), &n0);
        let p = r.on_time(&t(6000.0, 1001), false);
        assert_eq!(name(&p), Some("done")); // rtag = {} => repainted, the animation restarts
    }

    #[test]
    fn sleep_after_five_minutes_and_poke_wakes() {
        let mut r = Reactions::new();
        let n0 = t(2000.0, 1);
        assert_eq!(r.poke(&n0), None);
        assert_eq!(r.next_deadline(), Some(2000.0 + SLEEP_AFTER_MS));
        // pointer away and collapsed: sleep
        let p = r.on_time(&t(2000.0 + SLEEP_AFTER_MS, 300_001), false);
        assert_eq!(name(&p), Some("sleep"));
        assert!(r.has("sleep"));
        // a poke within the 1 s gap is ignored unless asleep: asleep -> it wakes at once
        let wake = r.poke(&t(2000.0 + SLEEP_AFTER_MS + 10.0, 300_011));
        assert_eq!(name(&wake), Some("calm"));
        assert!(!r.has("sleep"));
        // and the idle timer restarted
        assert_eq!(r.next_deadline(), Some(2000.0 + SLEEP_AFTER_MS + 10.0 + SLEEP_AFTER_MS));
    }

    #[test]
    fn sleep_does_not_start_while_the_pointer_is_on_the_pill() {
        let mut r = Reactions::new();
        r.poke(&t(2000.0, 1));
        let p = r.on_time(&t(2000.0 + SLEEP_AFTER_MS, 1), true); // busy
        assert_eq!(p, None);
        assert!(!r.has("sleep"));
        assert_eq!(r.next_deadline(), Some(2000.0 + 2.0 * SLEEP_AFTER_MS)); // re-armed
    }

    #[test]
    fn pokes_within_a_second_do_not_rearm() {
        let mut r = Reactions::new();
        r.poke(&t(5000.0, 1));
        r.poke(&t(5500.0, 1));
        assert_eq!(r.next_deadline(), Some(5000.0 + SLEEP_AFTER_MS));
        r.poke(&t(6001.0, 1));
        assert_eq!(r.next_deadline(), Some(6001.0 + SLEEP_AFTER_MS));
    }

    #[test]
    fn real_activity_wakes_from_sleep_but_sleep_reaction_does_not_poke() {
        let mut r = Reactions::new();
        r.poke(&t(5000.0, 1));
        r.react("sleep", "sleep", &ReactOpts::default(), &t(5100.0, 1), false);
        assert_eq!(r.current().0, Preset::Sleep);
        // 'sleep' itself did not restart the idle timer
        assert_eq!(r.next_deadline(), Some(5000.0 + SLEEP_AFTER_MS));
        // any other reaction pokes -> deletes sleep
        let p = r.react("agents", "done", &ReactOpts::ttl(2500), &t(5200.0, 1), false);
        assert_eq!(name(&p), Some("done"));
        assert!(!r.has("sleep"));
    }

    #[test]
    fn extend_keeps_an_entry_alive() {
        let mut r = Reactions::new();
        let n = t(5000.0, 1000);
        r.react("audio", "music", &ReactOpts::ttl(2500), &n, false);
        assert!(r.extend("audio", 9000));
        assert_eq!(r.get("audio").unwrap().until, 9000);
        assert!(!r.extend("nobody", 1));
        // the old timer fires but the entry survives; the timer re-arms for the new until
        let p = r.on_time(&t(7505.0, 3505), false);
        assert_eq!(p, None);
        assert_eq!(r.current().0, Preset::Music);
        assert_eq!(r.next_deadline(), Some(7505.0 + (9000.0 - 3505.0) + 5.0));
    }
}
