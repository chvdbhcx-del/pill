//! The pure part of the music widget: the track as the widget sees it, when it is on screen, where everything stands in its window and what a
//! click on a spot means. No drawing, no window, no clock of its own (every function gets `now`), so all of it is unit-tested.
//!
//! The window is one fixed frame, [`WIN`] logical px, that never resizes: the capsule (collapsed) and the card (open) both stand flush against
//! the window's DOCK edge (the bottom for `media.widget.edge` = bottom, the top otherwise) and the body between them grows away from that edge,
//! like the island does. Everything outside the body is click-through (the view reports the body as its hit region).
//!
//! In the taskbar (`media.widget.edge` = taskbar) the window is taller ([`WIN_BAR`]: its floor is the monitor's bottom, over the bar, with room
//! above it for the open card) and the folded capsule is a compact one that fits the bar strip; see [`Stand`].
use crate::ui::host::geom::{self, mirror_y, Align, Dock};
use glasskit::geom::Rect;
use serde_json::Value;

/// Width of the capsule and of the card.
pub const BODY_W: f32 = 320.0;
/// Height of the collapsed capsule and of the open card.
pub const CAPSULE_H: f32 = 56.0;
pub const CARD_H: f32 = 200.0;
/// Room around the body inside the window (soft shadow, the overshoot of the springs).
pub const PAD: f32 = 12.0;
/// The shadow of the body: it must stay inside [`PAD`] (the window ends there), so `SHADOW_BLUR + SHADOW_DY < PAD` on every side.
pub const SHADOW_BLUR: f32 = 8.0;
pub const SHADOW_DY: f32 = 2.0;
/// The window at size 100 %, logical px: the unit every layout number below is in ("base px"). The widget is laid out in base px and drawn
/// scaled by `k` (the island's `size`), see [`Stand::origin`].
pub const WIN: (f32, f32) = (BODY_W + 2.0 * PAD, CARD_H + 2.0 * PAD);
/// The `size` range of the island (80..200 %) as a factor: the widget follows it.
pub const KMIN: f32 = 0.8;
pub const KMAX: f32 = 2.0;
/// `size` (percent) as the widget's factor.
pub fn size_k(pct: f64) -> f32 {
    (pct / 100.0).clamp(KMIN as f64, KMAX as f64) as f32
}
/// The real window never changes: big enough for the largest size, click-through outside the body. The body stands in it by [`Stand::origin`].
pub const WIN_REAL: (f32, f32) = (WIN.0 * KMAX, WIN.1 * KMAX);
/// In the taskbar the window's side is this far from the body's anchored side, whatever the size (so a change of size never moves the window).
pub const BAR_PAD: f32 = PAD * KMAX;
/// The folded capsule in the taskbar: narrower than the card, as tall as the bar allows (see [`Stand::fold_h`]).
pub const BAR_FOLD_W: f32 = 288.0;
/// The window in the taskbar: the open card floats [`geom::BAR_FLOAT_GAP`] above the tallest bar the widget lives in ([`geom::WIDGET_BAR_MAX_H`]).
pub const WIN_BAR: (f32, f32) = (WIN.0, PAD + CARD_H + geom::BAR_FLOAT_GAP + geom::WIDGET_BAR_MAX_H);
/// The real window in the taskbar: the open card at the largest size floats [`geom::BAR_FLOAT_GAP`] above the tallest bar (real px).
pub const WIN_BAR_REAL: (f32, f32) = (WIN.0 * KMAX, (PAD + CARD_H + geom::BAR_FLOAT_GAP) * KMAX + geom::WIDGET_BAR_MAX_H);
/// How long a paused track keeps the widget on screen.
pub const PAUSE_GRACE_MS: f64 = 45_000.0;
/// The expansion past which the card (not the capsule) answers clicks.
pub const CARD_AT: f32 = 0.55;

// ------------------------------------------------------------------------------------------------------------------------------ the track

/// `media:changed` / `media_state` as the widget uses it (privacy: only what the island's own player shows, nothing is stored or sent).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub has_session: bool,
    pub playing: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub thumb_id: u64,
    /// `data:` URL, only present when the cover changed
    pub thumbnail: Option<String>,
    /// average cover colour
    pub accent: Option<[u8; 3]>,
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn n(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64))).unwrap_or(0)
}

/// `#rrggbb` -> rgb; anything else is `None` (an event may carry no accent at all).
pub fn parse_hex(h: &str) -> Option<[u8; 3]> {
    let h = h.strip_prefix('#')?;
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([c(0)?, c(2)?, c(4)?])
}

/// "Spotify.exe" -> "Spotify" (what the island's player shows for a track without a title).
pub fn app_name(app: &str) -> String {
    let cut = if app.to_lowercase().ends_with(".exe") { &app[..app.len() - 4] } else { app };
    cut.trim().to_string()
}

impl Track {
    pub fn from_json(v: &Value) -> Track {
        Track {
            has_session: v.get("has_session").and_then(Value::as_bool).unwrap_or(false),
            playing: v.get("playing").and_then(Value::as_bool).unwrap_or(false),
            title: s(v, "title"),
            artist: s(v, "artist"),
            album: s(v, "album"),
            app: s(v, "app"),
            position_ms: n(v, "position_ms"),
            duration_ms: n(v, "duration_ms"),
            thumb_id: n(v, "thumb_id"),
            thumbnail: v.get("thumbnail").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string),
            accent: v.get("accent").and_then(Value::as_str).and_then(parse_hex),
        }
    }

    /// The big line: the title, else the player's name.
    pub fn heading(&self) -> String {
        if self.title.trim().is_empty() {
            app_name(&self.app)
        } else {
            self.title.clone()
        }
    }

    /// The small line: the artist, else (for a track with a title) the player's name.
    pub fn sub(&self) -> String {
        if !self.artist.trim().is_empty() {
            self.artist.clone()
        } else if !self.title.trim().is_empty() {
            app_name(&self.app)
        } else {
            String::new()
        }
    }

    /// What makes it "another track" (the text cross-fades when this changes; a seek or a pause does not).
    pub fn identity(&self) -> String {
        format!("{}\u{1}{}\u{1}{}", self.title, self.artist, self.app)
    }
}

/// `m:ss` (`h:mm:ss` from an hour on).
pub fn clock_text(ms: u64) -> String {
    let t = ms / 1000;
    let (h, m, sec) = (t / 3600, t / 60 % 60, t % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

/// How far into the track, 0..1 (0 for a stream without a length).
pub fn fraction(pos_ms: u64, dur_ms: u64) -> f32 {
    if dur_ms == 0 {
        0.0
    } else {
        (pos_ms as f64 / dur_ms as f64).clamp(0.0, 1.0) as f32
    }
}

// ------------------------------------------------------------------------------------------------------------------------------ presence

/// When the widget is on screen: while a track plays, and for [`PAUSE_GRACE_MS`] after it was paused. A paused track seen for the first time
/// (the app started with a player already paused) does not call the widget up, and neither does a missing session. The module switch
/// (`module.media`) is the privacy gate: off = never.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Presence {
    on: bool,
    has: bool,
    playing: bool,
    paused_at: Option<f64>,
}

impl Presence {
    pub fn set_enabled(&mut self, on: bool) {
        self.on = on;
    }

    /// A new reading of the player; `now` in ms on any steady clock.
    pub fn update(&mut self, has_session: bool, playing: bool, now: f64) {
        let was_playing = self.has && self.playing;
        self.has = has_session;
        self.playing = has_session && playing;
        self.paused_at = if !has_session || self.playing {
            None
        } else if was_playing {
            Some(now) // playing -> paused: the grace starts
        } else {
            self.paused_at // paused before: it does not start over, and a player first seen paused never starts it
        };
    }

    pub fn visible(&self, now: f64) -> bool {
        self.on && self.has && (self.playing || self.paused_at.is_some_and(|t| now - t < PAUSE_GRACE_MS))
    }

}

// ------------------------------------------------------------------------------------------------------------------------------ geometry

/// How the body stands in its window: the dock, and for the taskbar dock the bar it stands in (`bar`, logical px, 0 = there is none: the plain
/// bottom placement) and the side it is anchored to (`align`, the placement's, which may differ from the setting when the island is in the way).
/// A plain `Dock` converts to a `Stand` with no bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stand {
    pub dock: Dock,
    pub align: Align,
    /// the bar's height in BASE px (the real height over `k`), 0 = no bar
    pub bar: f32,
    /// device px per logical px of the monitor the window stands on: the window is a whole number of device px, so its floor (the monitor's
    /// bottom, in the bar) and the capsule's edges are snapped to device px by [`Stand::surface`] and [`Stand::fold_h`]
    pub scale: f32,
    /// the size factor (`size` / 100, see [`size_k`]): one base px is `k` logical px. 1.0 = the layout at 100 %.
    pub k: f32,
    /// logical px the whole layout is shifted along the window: 0 at rest; the widget's window moved by this much under the body (the island next
    /// to it changed its width), so the body starts here and glides back to 0 (`view.rs`)
    pub dx: f32,
}

impl From<Dock> for Stand {
    fn from(dock: Dock) -> Stand {
        Stand { dock, align: Align::Left, bar: 0.0, scale: 1.0, k: 1.0, dx: 0.0 }
    }
}

impl Stand {
    /// Device px per BASE px: the grid every layout number is snapped to.
    pub fn bs(&self) -> f32 {
        self.scale.max(0.25) * self.k.max(0.1)
    }

    /// A base length snapped to the nearest device px.
    pub fn px(&self, v: f32) -> f32 {
        let k = self.bs();
        geom::phys(v, k) as f32 / k
    }

    /// The real window (logical px): the same for every size.
    pub fn real_win(&self) -> (f32, f32) {
        if self.dock == Dock::Taskbar {
            WIN_BAR_REAL
        } else {
            WIN_REAL
        }
    }

    /// Where the base layout's origin stands in the real window (logical px, a whole number of device px): window = base * `k` + origin. The
    /// base window (`surface`) hangs on the side the widget is docked to: its dock edge on the real window's top / bottom edge and, along the edge,
    /// its left / right side on the real window's side (in the bar: the body's anchored side [`BAR_PAD`] from it) or its middle on the middle.
    /// So at 100 % a left docked body stands `PAD` from the screen's side and edge, and a change of `k` only grows it away from the corner.
    pub fn origin(&self) -> (f32, f32) {
        let m = self.scale.max(0.25);
        let real = self.real_win();
        let (rw, rh) = (geom::phys(real.0, m), geom::phys(real.1, m));
        let s = self.surface();
        let (bw, bh) = ((s.0 * self.k * m).round() as i32, (s.1 * self.k * m).round() as i32);
        let bar = self.in_bar();
        let ox = match self.align {
            Align::Center => ((rw - bw) as f32 / 2.0).round() as i32,
            Align::Left if bar => ((BAR_PAD - self.px(PAD) * self.k) * m).round() as i32,
            Align::Left => 0,
            Align::Right if bar => rw - (BAR_PAD * m).round() as i32 - ((s.0 - self.px(PAD)) * self.k * m).round() as i32,
            Align::Right => rw - bw,
        };
        let oy = if self.dock == Dock::Top { 0 } else { rh - bh };
        ((ox as f32 + (self.dx * m).round()) / m, oy as f32 / m)
    }

    /// A base point -> the real window, and back.
    pub fn to_real(&self, p: (f32, f32)) -> (f32, f32) {
        let o = self.origin();
        (p.0 * self.k + o.0, p.1 * self.k + o.1)
    }

    pub fn to_base(&self, p: (f32, f32)) -> (f32, f32) {
        let o = self.origin();
        ((p.0 - o.0) / self.k, (p.1 - o.1) / self.k)
    }

    pub fn rect_to_real(&self, r: Rect) -> Rect {
        let o = self.origin();
        Rect::new(r.x * self.k + o.0, r.y * self.k + o.1, r.w * self.k, r.h * self.k)
    }

    /// The window as the host really makes it (logical px): a whole number of device px, so up to half a device px off [`Stand::win`]. Its
    /// bottom is where bottom-standing bodies stand, so the layout works from this and never from the unrounded size.
    pub fn surface(&self) -> (f32, f32) {
        let w = self.win();
        (self.px(w.0), self.px(w.1))
    }

    /// The capsule stands inside the taskbar strip right now.
    pub fn in_bar(&self) -> bool {
        self.dock == Dock::Taskbar && self.bar > 0.0
    }

    /// The window's size in base px (it never changes for a given dock); the real window is [`Stand::real_win`].
    pub fn win(&self) -> (f32, f32) {
        if self.dock == Dock::Taskbar {
            WIN_BAR
        } else {
            WIN
        }
    }

    /// Height of the folded capsule: [`CAPSULE_H`], or what the bar leaves (`geom::bar_fold_cap`) when that is less. In the bar it is a whole
    /// number of device px with an EVEN number of them left over, so the margin above and below it is the same whole number of device px
    /// (no half-pixel rows: crisp edges, truly centred) at every DPI.
    pub fn fold_h(&self) -> f32 {
        if self.in_bar() {
            // the cap is a REAL length (the bar's margin does not grow with the size): the capsule only gets as big as the bar allows
            let (k, cap) = (self.bs(), geom::bar_fold_cap(self.bar * self.k) / self.k);
            let (bar_p, mut h_p) = (geom::phys(self.bar, k), geom::phys(cap.min(CAPSULE_H), k));
            if (bar_p - h_p) % 2 != 0 {
                h_p -= 1;
            }
            h_p.max(1) as f32 / k
        } else {
            CAPSULE_H
        }
    }

    /// Where a rectangle `w` wide stands inside `b` (the body): flush with the anchored side, or in the middle.
    fn side_x(&self, b: Rect, w: f32) -> f32 {
        match self.align {
            Align::Left => b.x,
            Align::Right => b.right() - w,
            Align::Center => b.x + (b.w - w) / 2.0,
        }
    }
}

/// The body (what is drawn and what clicks hit) for an expansion `p` (0 = capsule, 1 = card, may overshoot a little) and a slide `dy` away from
/// the dock edge (the enter / leave motion: positive = further toward the edge). Window coordinates. In the bar the capsule is centred in it
/// (`geom::bar_gap`) and the body rises with `p` to float above it, growing from the capsule's width to the card's on its anchored side.
pub fn body(st: impl Into<Stand>, p: f32, dy: f32) -> Rect {
    let st = st.into();
    let win = st.surface();
    if st.in_bar() {
        let fh = st.fold_h();
        let h = (fh + (CARD_H - fh) * p).max(8.0);
        let fw = st.px(BAR_FOLD_W);
        let w = (fw + (BODY_W - fw) * p).max(8.0);
        let pad = st.px(PAD);
        let x = match st.align {
            Align::Left => pad,
            Align::Right => win.0 - pad - w,
            // snapped when folded, and the snap fades out while it opens so that nothing steps
            Align::Center => (win.0 - w) / 2.0 + (st.px((win.0 - fw) / 2.0) - (win.0 - fw) / 2.0) * (1.0 - p.clamp(0.0, 1.0)),
        };
        // open: the sides on device px too (folded they are snapped above; in between they glide)
        let (x, w) = if p >= 1.0 { (st.px(x), st.px(x + w) - st.px(x)) } else { (x, w) };
        // the vertical edges on device px (a size where the float gap is not a whole number of them would blur the open card's outline)
        let (top, bottom) = (st.px(win.1 - geom::bar_gap(st.bar, fh, p) + dy - h), st.px(win.1 - geom::bar_gap(st.bar, fh, p) + dy));
        return Rect::new(x, top, w, bottom - top);
    }
    let h = (CAPSULE_H + (CARD_H - CAPSULE_H) * p).max(8.0);
    let y = match st.dock {
        Dock::Top => PAD - dy,
        // the plain bottom edge, and the taskbar dock where there is no bar to stand in
        Dock::Bottom | Dock::Taskbar => mirror_y(PAD - dy, h, win.1),
    };
    // every edge on a device px (at rest, at any size and DPI): crisp outline, the same margin on all sides
    let (x, top, bottom) = (st.px(PAD), st.px(y), st.px(y + h));
    Rect::new(x, top, st.px(PAD + BODY_W) - x, bottom - top)
}

/// The corner radius of the body: the capsule's full half circle closing into the card's [`CARD_R`] (one radius for all four corners).
pub fn body_radius(p: f32, h: f32) -> f32 {
    (28.0 - (28.0 - CARD_R) * p.clamp(0.0, 1.0)).min(h / 2.0)
}

/// What answers the cursor: the body, and in the bar also the strip down to where the folded capsule stands, so that the cursor resting on the
/// capsule keeps the card open while the card floats above the bar (otherwise it would fold the moment it rose off the cursor).
pub fn hit_rect(st: impl Into<Stand>, p: f32, dy: f32) -> Rect {
    let st = st.into();
    let b = body(st, p, dy);
    if !st.in_bar() {
        return b;
    }
    let floor = st.surface().1 - geom::bar_gap(st.bar, st.fold_h(), 0.0) + dy;
    Rect::new(b.x, b.y, b.w, (floor - b.y).max(b.h))
}

/// The capsule and the card at their full size (where the content is laid out; the body clips them). On the plain edges both stand flush
/// against the dock edge; in the bar both stand on the body's bottom at its anchored side, so they rise with it.
pub fn frames(st: impl Into<Stand>, p: f32, dy: f32) -> (Rect, Rect) {
    let st = st.into();
    if st.in_bar() {
        let b = body(st, p, dy);
        let fh = st.fold_h();
        return (Rect::new(st.side_x(b, BAR_FOLD_W), b.bottom() - fh, BAR_FOLD_W, fh), Rect::new(st.side_x(b, BODY_W), b.bottom() - CARD_H, BODY_W, CARD_H));
    }
    (body(st, 0.0, dy), body(st, 1.0, dy))
}

/// Which way the enter / leave motion goes along the vertical: the sign a slide away from the dock edge takes (toward the screen edge).
/// Far enough for the body to leave the window from the bar too (a bar is at most [`geom::WIDGET_BAR_MAX_H`] tall).
pub fn slide_distance() -> f32 {
    CAPSULE_H + PAD
}

/// The three transport buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Btn {
    Prev,
    Play,
    Next,
}

pub const BTNS: [Btn; 3] = [Btn::Prev, Btn::Play, Btn::Next];

impl Btn {
    pub fn index(self) -> usize {
        self as usize
    }
    /// The `media_control` action.
    pub fn action(self) -> &'static str {
        match self {
            Btn::Prev => "prev",
            Btn::Play => "play_pause",
            Btn::Next => "next",
        }
    }
}

/// Every rectangle of the capsule, relative to its frame `c`.
pub struct CapsuleLayout {
    pub cover: Rect,
    pub text: Rect,
    pub btn: [Rect; 3],
    pub bar: Rect,
}

pub fn capsule_layout(c: Rect) -> CapsuleLayout {
    let b = |i: f32, size: f32| Rect::new(c.right() - 10.0 - (3.0 - i) * 32.0 + (32.0 - size) / 2.0, c.y + (c.h - size) / 2.0 - 1.0, size, size);
    let btn = [b(0.0, 28.0), b(1.0, 32.0), b(2.0, 28.0)];
    let text_x = c.x + 8.0 + 40.0 + 10.0;
    CapsuleLayout {
        cover: Rect::new(c.x + 8.0, c.y + (c.h - 40.0) / 2.0 - 1.0, 40.0, 40.0),
        text: Rect::new(text_x, c.y + 8.0, btn[0].x - 6.0 - text_x, 38.0),
        btn,
        bar: Rect::new(c.x + 14.0, c.bottom() - 5.0, c.w - 28.0, 2.0),
    }
}

/// Gap between the title and the artist on the bar capsule's one line, and the least room the artist is worth drawing in (less: it is dropped).
pub const LINE_GAP: f32 = 8.0;
pub const SUB_MIN: f32 = 32.0;

/// What the bar capsule's one line shows in `avail` px: the heading's width (the whole title, or all there is when it is cut) and, when there is
/// room, where the artist stands (x from the line's start, width). The ARTIST gives way first: it is cut to the room left behind a whole title,
/// and dropped when that is under [`SUB_MIN`] (or the artist is shorter and does not fit its own length); the title is only cut when it is
/// alone on the line. `head` / `sub` are the natural text widths.
pub fn bar_line(avail: f32, head: f32, sub: f32) -> (f32, Option<(f32, f32)>) {
    if head >= avail {
        return (avail.max(0.0), None);
    }
    let room = avail - head - LINE_GAP;
    if sub > 0.0 && room >= SUB_MIN.min(sub) {
        (head, Some((head + LINE_GAP, sub.min(room))))
    } else {
        (head, None)
    }
}

/// The capsule in the taskbar strip: a small cover, one line of text, three small buttons and a hairline progress bar, scaled to the strip
/// (`c.h` is what the bar leaves, see [`Stand::fold_h`]; `scale` is the monitor's, for whole-device-px sizes). The ends of the capsule are
/// half circles of radius `h / 2`; the INK at both ends stands the same distance `E` (0.3 h) from the end on the centre line: the cover's left
/// edge on one side, the last glyph's right edge on the other (its hover disc reaches further out, nearly concentric with the end). The cover's
/// corner radius is `h / 2 - E`, so the centre of its corner arcs is the centre of the capsule's end. Everything centred is centred on a whole
/// number of device px. The bar is dropped (zero height) when there is no room for it under the buttons.
pub fn bar_capsule_layout(c: Rect, scale: f32) -> CapsuleLayout {
    let k = scale.max(0.25);
    let sp = |v: f32| (v * k).round() / k;
    let h = c.h;
    let h_p = (h * k).round() as i32;
    // a size that leaves the same whole number of device px above and below it
    let even = |v: f32, lo: f32| -> f32 {
        let mut p = (v.max(lo) * k).round() as i32;
        if (h_p - p) % 2 != 0 {
            p -= 1;
        }
        p.max(1) as f32 / k
    };
    let cs = even((h - 12.0).clamp(14.0, 40.0), 14.0);
    let bs = even((h - 12.0).clamp(16.0, 28.0), 16.0);
    let e = sp(h * 0.30);
    let step = bs + 2.0;
    let disc = sp((e - bs * 0.24).max(2.0)); // the last disc's right edge from the end
    let b2 = c.right() - disc - bs;
    let by = c.y + (h - bs) / 2.0;
    let btn = [Rect::new(b2 - 2.0 * step, by, bs, bs), Rect::new(b2 - step, by, bs, bs), Rect::new(b2, by, bs, bs)];
    let cover = Rect::new(c.x + e, c.y + (h - cs) / 2.0, cs, cs);
    let text_x = cover.right() + 10.0;
    let bar = if h >= 28.0 {
        let t = ((1.5 * k).floor().max(1.0)) / k;
        Rect::new(c.x + h / 2.0, c.bottom() - sp(4.0), (c.w - h).max(0.0), t)
    } else {
        Rect::new(c.x, c.bottom(), 0.0, 0.0)
    };
    CapsuleLayout { cover, text: Rect::new(text_x, c.y, (btn[0].x - 4.0 - text_x).max(0.0), h), btn, bar }
}

/// The corner radius of the cover on the bar capsule (concentric in x with the capsule's end, see [`bar_capsule_layout`]).
pub fn bar_cover_radius(h: f32) -> f32 {
    h * 0.2
}

/// The capsule's layout for the way the body stands: the compact one in the bar.
pub fn capsule_layout_in(st: impl Into<Stand>, c: Rect) -> CapsuleLayout {
    let st = st.into();
    if st.in_bar() {
        bar_capsule_layout(c, st.bs())
    } else {
        capsule_layout(c)
    }
}

/// Every rectangle of the card, relative to its frame `c`.
pub struct CardLayout {
    pub cover: Rect,
    pub title: Rect,
    pub sub: Rect,
    pub album: Rect,
    pub bar: Rect,
    pub elapsed: Rect,
    pub total: Rect,
    pub btn: [Rect; 3],
}

/// The card's even outer padding (left, top, right and bottom: the cover, the time labels, the track ends and the play disc stand this far from
/// the card's edges) and its corner radius; the cover's radius is the concentric one, [`card_cover_radius`].
pub const CARD_PAD: f32 = 16.0;
pub const CARD_R: f32 = 24.0;
pub const COVER: f32 = 88.0;

/// The cover's corner radius: the card's radius less the padding, so its corner arcs share their centre with the card's.
pub fn card_cover_radius() -> f32 {
    CARD_R - CARD_PAD
}

pub fn card_layout(c: Rect) -> CardLayout {
    let p = CARD_PAD;
    let cover = Rect::new(c.x + p, c.y + p, COVER, COVER);
    let tx = cover.right() + 14.0;
    let tw = c.right() - p - tx;
    // the three lines are one block, centred against the cover (title high, album low, the artist between)
    let (th, sh, ah, g1, g2) = (22.0, 18.0, 16.0, 2.0, 4.0);
    let top = cover.y + (cover.h - (th + g1 + sh + g2 + ah)) / 2.0;
    let sub = Rect::new(tx, top + th + g1, tw, sh);
    // the track spans the card between the padding (the knob is kept inside it, see `view`), the times hang under its ends
    let bar = Rect::new(c.x + p, cover.bottom() + 12.0, c.w - 2.0 * p, 6.0);
    let elapsed = Rect::new(bar.x, bar.bottom() + 2.0, 80.0, 14.0);
    // the transport row stands on the bottom padding: the play disc's bottom is `p` above the card's
    let (side, play) = (32.0, 40.0);
    let cy = c.bottom() - p - play / 2.0;
    let mid = c.x + c.w / 2.0;
    CardLayout {
        cover,
        title: Rect::new(tx, top, tw, th),
        sub,
        album: Rect::new(tx, sub.bottom() + g2, tw, ah),
        bar,
        elapsed,
        total: Rect::new(bar.right() - 80.0, elapsed.y, 80.0, 14.0),
        btn: [Rect::new(mid - 64.0 - side / 2.0, cy - side / 2.0, side, side), Rect::new(mid - play / 2.0, cy - play / 2.0, play, play), Rect::new(mid + 64.0 - side / 2.0, cy - side / 2.0, side, side)],
    }
}

/// The parts the capsule and the card share (the cover, the two text lines, the three buttons, the progress bar) at an expansion `p`: each one
/// stands between its place in the capsule and its place in the card, so the capsule MORPHS into the card (the cover grows, the buttons travel)
/// instead of one being swapped for the other. Only the card's album line and the times fade in. Window coordinates, like [`body`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    pub cover: Rect,
    /// the cover's corner radius (base px)
    pub cover_r: f32,
    /// the title and the artist line (in the bar the capsule has ONE line and these only mark the card's two: the view cross-fades the text there)
    pub head: Rect,
    pub sub: Rect,
    pub btn: [Rect; 3],
    pub bar: Rect,
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// A rectangle between `a` and `b` (`t` may pass 0..1 a little: a spring's overshoot).
pub fn mix(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::new(lerp(a.x, b.x, t), lerp(a.y, b.y, t), lerp(a.w, b.w, t).max(0.0), lerp(a.h, b.h, t).max(0.0))
}

pub fn parts(st: impl Into<Stand>, p: f32, dy: f32) -> Parts {
    let st = st.into();
    let (cap, card) = frames(st, p, dy);
    let (a, b) = (capsule_layout_in(st, cap), card_layout(card));
    let t = p.clamp(0.0, 1.1);
    let (ah, asub, ar) = if st.in_bar() {
        (a.text, a.text, bar_cover_radius(cap.h))
    } else {
        (Rect::new(a.text.x, a.text.y + 3.0, a.text.w, 18.0), Rect::new(a.text.x, a.text.y + 21.0, a.text.w, 15.0), 7.0)
    };
    // a bar capsule too thin for a progress bar has none to grow from: the card's own shows
    let bar = if a.bar.h > 0.0 { mix(a.bar, b.bar, t) } else { b.bar };
    Parts {
        cover: mix(a.cover, b.cover, t),
        cover_r: lerp(ar, card_cover_radius(), t),
        head: mix(ah, b.title, t),
        sub: mix(asub, b.sub, t),
        btn: std::array::from_fn(|i| mix(a.btn[i], b.btn[i], t)),
        bar,
    }
}

/// What is under a point of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Button(Btn),
    /// the progress bar of the card; the fraction of the track the x stands for
    Seek(f32),
    /// In the taskbar only: the cover or the title (of the folded capsule, of the open card, or the strip the card floats over): a click
    /// there opens the card, or folds it back. The buttons answer for themselves; hovering opens nothing (the capsule has its own buttons).
    Header,
    Body,
    None,
}

/// The seek position of an x on the bar `bar`.
pub fn seek_at(bar: Rect, x: f32) -> f32 {
    ((x - bar.x) / bar.w).clamp(0.0, 1.0)
}

/// What a click at `pos` (window coordinates) would hit with the body at expansion `p`. The capsule answers while it is mostly closed, the card
/// once it is mostly open; the bar's strip is a little taller than the bar so it is easy to grab.
pub fn hit(st: impl Into<Stand>, p: f32, dy: f32, pos: (f32, f32)) -> Hit {
    let st = st.into();
    if !hit_rect(st, p, dy).contains(pos.0, pos.1) {
        return Hit::None;
    }
    let (_, card) = frames(st, p, dy);
    let grab = |r: Rect| Rect::new(r.x - 4.0, r.y - 8.0, r.w + 8.0, r.h + 16.0).contains(pos.0, pos.1);
    // the buttons answer where they are drawn, which is between their two places while the capsule morphs
    let pt = parts(st, p, dy);
    if let Some(i) = pt.btn.iter().position(|r| r.contains(pos.0, pos.1)) {
        return Hit::Button(BTNS[i]);
    }
    if p >= CARD_AT {
        if grab(pt.bar) {
            return Hit::Seek(seek_at(pt.bar, pos.0));
        }
        let l = card_layout(card);
        if st.in_bar() && (l.cover.contains(pos.0, pos.1) || l.title.contains(pos.0, pos.1) || l.sub.contains(pos.0, pos.1) || pos.1 > card.bottom()) {
            return Hit::Header;
        }
    } else if st.in_bar() && pos.0 < pt.btn[0].x {
        return Hit::Header;
    }
    Hit::Body
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn playing(title: &str) -> Value {
        json!({ "has_session": true, "playing": true, "title": title, "artist": "Band", "album": "LP", "app": "Spotify.exe", "position_ms": 61_000, "duration_ms": 200_000,
                "thumb_id": 7, "thumbnail": "data:image/png;base64,AAAA", "accent": "#aa3355" })
    }

    #[test]
    fn a_media_event_becomes_a_track_and_a_bad_one_becomes_nothing_in_particular() {
        let t = Track::from_json(&playing("Song"));
        assert_eq!((t.has_session, t.playing, t.title.as_str(), t.artist.as_str(), t.position_ms, t.duration_ms, t.thumb_id), (true, true, "Song", "Band", 61_000, 200_000, 7));
        assert_eq!(t.accent, Some([0xaa, 0x33, 0x55]));
        assert_eq!(t.thumbnail.as_deref(), Some("data:image/png;base64,AAAA"));
        let none = Track::from_json(&Value::Null);
        assert_eq!(none, Track::default());
        let t = Track::from_json(&json!({ "has_session": true, "accent": "orange", "thumbnail": "", "position_ms": -5.0, "duration_ms": 1.5e3 }));
        assert_eq!((t.accent, t.thumbnail, t.position_ms, t.duration_ms), (None, None, 0, 1500));
        assert_eq!(parse_hex("#ÿ12345"), None);
        assert_eq!(parse_hex("#0a0B0c"), Some([10, 11, 12]));
    }

    #[test]
    fn the_lines_fall_back_to_the_player_and_a_track_change_is_not_a_seek() {
        let mut t = Track::from_json(&playing("Song"));
        assert_eq!((t.heading().as_str(), t.sub().as_str()), ("Song", "Band"));
        let id = t.identity();
        t.position_ms = 99_000;
        t.playing = false;
        assert_eq!(t.identity(), id, "a seek and a pause keep the track");
        t.title = "Other".into();
        assert_ne!(t.identity(), id);
        t.artist.clear();
        assert_eq!(t.sub(), "Spotify", "no artist: the player's name");
        t.title.clear();
        assert_eq!((t.heading().as_str(), t.sub().as_str()), ("Spotify", ""));
    }

    #[test]
    fn times_and_fractions_are_plain() {
        assert_eq!((clock_text(0), clock_text(61_000), clock_text(3_599_999), clock_text(3_661_000)), ("0:00".into(), "1:01".into(), "59:59".into(), "1:01:01".into()));
        assert_eq!((fraction(50, 200), fraction(300, 200), fraction(5, 0)), (0.25, 1.0, 0.0));
    }

    #[test]
    fn the_widget_shows_while_playing_and_for_a_while_after_a_pause_and_never_without_the_switch() {
        let mut p = Presence::default();
        p.update(true, true, 0.0);
        assert!(!p.visible(0.0), "the module switch has not been read yet: privacy first");
        p.set_enabled(true);
        assert!(p.visible(0.0));
        p.update(true, false, 10_000.0);
        assert!(p.visible(10_000.0 + PAUSE_GRACE_MS - 1.0), "paused recently: still there");
        assert!(!p.visible(10_000.0 + PAUSE_GRACE_MS), "the grace is over");
        p.update(true, false, 100_000.0);
        assert!(!p.visible(100_001.0), "paused events do not extend the grace");
        p.update(true, true, 200_000.0);
        assert!(p.visible(200_000.0));
        p.update(false, false, 200_100.0);
        assert!(!p.visible(200_100.0), "no session, no widget");
        p.update(true, true, 300_000.0);
        p.set_enabled(false);
        assert!(!p.visible(300_000.0), "module.media off: gone at once");
    }

    #[test]
    fn a_player_that_starts_paused_does_not_call_the_widget_up() {
        let mut p = Presence::default();
        p.set_enabled(true);
        p.update(true, false, 5_000.0);
        assert!(!p.visible(5_000.0));
        p.update(true, true, 6_000.0);
        p.update(true, false, 9_000.0);
        assert!(p.visible(9_000.0), "but a pause after playing does");
    }

    #[test]
    fn the_body_grows_away_from_the_dock_edge_and_stays_on_it() {
        let (b0, b1) = (body(Dock::Bottom, 0.0, 0.0), body(Dock::Bottom, 1.0, 0.0));
        assert_eq!((b0.bottom(), b1.bottom(), b0.h, b1.h), (WIN.1 - PAD, WIN.1 - PAD, CAPSULE_H, CARD_H), "the bottom floor does not move while it grows up");
        assert!(b1.y >= PAD - 0.01, "and the open card fits in the window");
        let (t0, t1) = (body(Dock::Top, 0.0, 0.0), body(Dock::Top, 1.0, 0.0));
        assert_eq!((t0.y, t1.y, t1.bottom()), (PAD, PAD, PAD + CARD_H), "the top ceiling stays and the card hangs down");
        assert_eq!((b0.x, b0.w), (PAD, BODY_W));
        // leaving: slid toward the screen edge, out of the window (it is clipped there)
        assert!(body(Dock::Bottom, 0.0, slide_distance()).y > b0.y && body(Dock::Top, 0.0, slide_distance()).y < t0.y);
        assert!(body(Dock::Bottom, 0.0, slide_distance()).y >= WIN.1 - PAD - CAPSULE_H + slide_distance() - 0.01);
        // a spring may overshoot a little: never a negative height
        assert!(body(Dock::Top, -0.5, 0.0).h > 0.0);
    }

    #[test]
    fn the_capsule_and_the_card_lay_their_parts_out_inside_themselves() {
        for dock in [Dock::Bottom, Dock::Top] {
            let (cap, card) = frames(dock, 0.0, 0.0);
            let l = capsule_layout(cap);
            for r in [l.cover, l.text, l.btn[0], l.btn[1], l.btn[2], l.bar] {
                assert!(r.x >= cap.x && r.right() <= cap.right() && r.y >= cap.y && r.bottom() <= cap.bottom(), "{r:?} in {cap:?}");
            }
            assert!(l.text.w > 100.0 && l.btn[0].right() <= l.btn[1].x + 0.01 && l.btn[1].right() <= l.btn[2].x + 0.01, "the buttons do not overlap");
            assert!(l.text.right() <= l.btn[0].x, "the text stops before the buttons");
            let c = card_layout(card);
            for r in [c.cover, c.title, c.sub, c.album, c.bar, c.elapsed, c.total, c.btn[0], c.btn[1], c.btn[2]] {
                assert!(r.x >= card.x && r.right() <= card.right() && r.y >= card.y && r.bottom() <= card.bottom(), "{r:?} in {card:?}");
            }
            assert!(c.bar.bottom() <= c.elapsed.y && c.elapsed.bottom() <= c.btn[1].y, "bar, times and buttons stack without touching");
            assert!(c.cover.bottom() <= c.bar.y && c.title.x >= c.cover.right());
        }
    }

    #[test]
    fn clicks_mean_what_the_open_state_says() {
        let d = Dock::Bottom;
        let (cap, card) = frames(d, 0.0, 0.0);
        let play_c = capsule_layout(cap).btn[1].center();
        assert_eq!(hit(d, 0.0, 0.0, play_c), Hit::Button(Btn::Play));
        assert_eq!(hit(d, 0.0, 0.0, capsule_layout(cap).btn[0].center()), Hit::Button(Btn::Prev));
        assert_eq!(hit(d, 0.0, 0.0, capsule_layout(cap).cover.center()), Hit::Body);
        assert_eq!(hit(d, 0.0, 0.0, (2.0, 2.0)), Hit::None, "outside the body the window is click-through");
        // the closed capsule does not answer where the card's buttons will be
        let c = card_layout(card);
        assert_eq!(hit(d, 0.0, 0.0, (c.btn[2].center().0, c.cover.y + 4.0)), Hit::None);
        assert_eq!(hit(d, 1.0, 0.0, c.btn[2].center()), Hit::Button(Btn::Next));
        let mid = c.bar.x + c.bar.w / 2.0;
        assert_eq!(hit(d, 1.0, 0.0, (mid, c.bar.cy())), Hit::Seek(0.5));
        assert_eq!(hit(d, 1.0, 0.0, (c.bar.x - 3.0, c.bar.cy() + 6.0)), Hit::Seek(0.0), "the grab strip is a little bigger than the bar");
        assert_eq!(hit(d, 1.0, 0.0, c.title.center()), Hit::Body);
        // half way: the buttons answer where they are drawn, between their two places (the capsule's while it is mostly closed)
        assert_eq!(hit(d, CARD_AT - 0.01, 0.0, parts(d, CARD_AT - 0.01, 0.0).btn[1].center()), Hit::Button(Btn::Play));
        assert_eq!(hit(d, 0.0, 0.0, play_c), Hit::Button(Btn::Play));
        assert_eq!(seek_at(c.bar, c.bar.x + c.bar.w * 3.0), 1.0);
    }

    // ----- in the taskbar

    fn in_bar(align: Align, bar: f32) -> Stand {
        Stand { dock: Dock::Taskbar, align, bar, scale: 1.0, k: 1.0, dx: 0.0 }
    }

    /// The bar heights of the common taskbars at the common scalings (and the odd ones, 110 / 175 %), as the view is told them: the bar in
    /// logical px (a whole number of device px over the scale) and the scale.
    fn bars() -> Vec<(f32, f32)> {
        let mut v = Vec::new();
        for dpi in [96u32, 106, 120, 144, 168, 192] {
            let s = geom::scale_from_dpi(dpi);
            for bar in [40.0f32, 48.0, 52.0] {
                v.push((geom::phys(bar, s) as f32 / s, s));
            }
        }
        v
    }

    #[test]
    fn the_folded_capsule_sits_inside_the_bar_centred_and_fits_it() {
        for (bar, scale) in bars() {
            for align in [Align::Left, Align::Center, Align::Right] {
                let st = Stand { align, bar, scale, ..in_bar(align, bar) };
                let b = body(st, 0.0, 0.0);
                // the window is a whole number of device px: its floor (the monitor's bottom) is the rounded height
                let sf = st.surface();
                assert!((sf.1 - WIN_BAR.1).abs() <= 0.5 / scale + 1e-3 && (sf.1 * scale - (sf.1 * scale).round()).abs() < 1e-2);
                let (floor, top) = (sf.1, sf.1 - bar);
                assert!(b.y >= top - 0.01 && b.bottom() <= floor + 0.01, "{bar}@{scale}: {b:?} inside the strip [{top}, {floor}]");
                assert!(((b.y - top) - (floor - b.bottom())).abs() < 0.01, "{bar}@{scale}: centred in the bar");
                // whole device px on every edge, the same margin above and below
                for v in [b.y, b.bottom(), b.x, b.right()] {
                    assert!((v * scale - (v * scale).round()).abs() < 0.02, "{bar}@{scale}: {v} is on a device px: {b:?}");
                }
                assert!(b.h <= bar - 8.0 + 0.01 && b.h <= CAPSULE_H && b.h >= 16.0 - 1.0 / scale, "{bar}@{scale}: {} tall", b.h);
                assert!((b.w - BAR_FOLD_W).abs() <= 0.5 / scale + 1e-3);
                match align {
                    Align::Left => assert!((b.x - PAD).abs() <= 0.5 / scale + 1e-3),
                    Align::Right => assert!((b.right() - (WIN_BAR.0 - PAD)).abs() <= 0.5 / scale + 1e-3),
                    Align::Center => assert!((b.cx() - sf.0 / 2.0).abs() <= 0.5 / scale + 1e-3),
                }
                assert!(b.x >= 0.0 && b.right() <= sf.0, "inside the window");
            }
        }
        // the tallest bar the widget lives in still has the open card inside the window
        for bar in [24.0, 40.0, 72.0] {
            let b = body(in_bar(Align::Left, bar), 1.0, 0.0);
            assert!(b.y >= PAD - 0.01 && (WIN_BAR.1 - b.bottom() - (bar + geom::BAR_FLOAT_GAP)).abs() < 0.01, "{bar}: the open card floats 8 px above the bar: {b:?}");
            assert_eq!((b.w, b.h), (BODY_W, CARD_H));
        }
    }

    #[test]
    fn opening_raises_the_capsule_out_of_the_bar_without_a_jump_and_closing_puts_it_back() {
        let st = in_bar(Align::Left, 48.0);
        let mut prev = body(st, 0.0, 0.0);
        for i in 1..=20 {
            let b = body(st, i as f32 / 20.0, 0.0);
            assert!(b.bottom() <= prev.bottom() + 0.01 && b.y <= prev.y + 0.01, "it only rises while it opens");
            assert!((b.bottom() - prev.bottom()).abs() < 12.0 && (b.y - prev.y).abs() < 12.0, "in small steps: {prev:?} -> {b:?}");
            assert!(b.x >= PAD - 0.01 && b.x == PAD, "the anchored side stays put");
            prev = b;
        }
        // the same on the right side: the right edge stays where the bar's slot ends while the card widens to the left
        let r = in_bar(Align::Right, 48.0);
        assert_eq!((body(r, 0.0, 0.0).right(), body(r, 1.0, 0.0).right()), (WIN_BAR.0 - PAD, WIN_BAR.0 - PAD));
        // overshoot of a spring and a slide out (leaving) keep it a sane rectangle
        assert!(body(st, -0.4, 0.0).h > 0.0 && body(st, 1.05, 0.0).y >= 0.0);
        assert!(body(st, 0.0, slide_distance()).y >= WIN_BAR.1 - 0.01, "leaving slides the capsule down out of the window");
    }

    #[test]
    fn the_capsule_parts_fit_every_bar() {
        for (bar, scale) in bars() {
            let st = Stand { scale, ..in_bar(Align::Left, bar) };
            let (cap, _) = frames(st, 0.0, 0.0);
            let l = capsule_layout_in(st, cap);
            let tag = format!("{bar}@{scale}");
            for r in [l.cover, l.text, l.btn[0], l.btn[1], l.btn[2]] {
                assert!(r.x >= cap.x - 0.01 && r.right() <= cap.right() + 0.01 && r.y >= cap.y - 0.01 && r.bottom() <= cap.bottom() + 0.01, "{tag}: {r:?} in {cap:?}");
            }
            assert!(l.cover.h >= 14.0 - 1.0 / scale && l.cover.h <= cap.h - 8.0, "{tag}: a small cover with room round it");
            assert!(l.btn[0].right() <= l.btn[1].x + 0.01 && l.btn[1].right() <= l.btn[2].x + 0.01, "the buttons do not overlap");
            assert!(l.cover.right() < l.text.x && l.text.right() <= l.btn[0].x && l.text.w > 100.0, "{tag}: one line of text between the cover and the buttons: {:?}", l.text);
            // the progress is a hairline under the buttons, or it is gone
            if l.bar.h > 0.0 {
                assert!(l.bar.h <= 2.0 && l.bar.y >= l.btn[1].bottom() && l.bar.bottom() <= cap.bottom() && l.bar.x >= cap.x && l.bar.right() <= cap.right(), "{tag}: {:?}", l.bar);
            }
        }
        // a bar so thin that nothing fits under the buttons has no progress bar at all
        let thin = capsule_layout_in(in_bar(Align::Left, 24.0), frames(in_bar(Align::Left, 24.0), 0.0, 0.0).0);
        assert_eq!(thin.bar.h, 0.0);
    }

    #[test]
    fn the_bar_capsule_is_symmetrical_centred_and_its_hairline_stays_on_the_straight_part() {
        for (bar, scale) in bars() {
            let st = Stand { scale, ..in_bar(Align::Right, bar) };
            let (cap, _) = frames(st, 0.0, 0.0);
            let l = capsule_layout_in(st, cap);
            let tag = format!("{bar}@{scale}");
            let tol = 1.0 / scale + 0.01;
            // the ink at both ends of the capsule is the same distance from its end on the centre line: the cover's edge, the last glyph's edge
            let left = l.cover.x - cap.x;
            let right = cap.right() - (l.btn[2].right() - l.btn[2].w * 0.24);
            assert!((left - right).abs() <= tol, "{tag}: {left} on the left, {right} on the right");
            assert!(left >= cap.h * 0.25, "{tag}: the cover is not jammed into the rounded end ({left} of {})", cap.h);
            // the cover's corner arcs and the capsule's end share a centre (in x)
            assert!(((l.cover.x + bar_cover_radius(cap.h)) - (cap.x + cap.h / 2.0)).abs() <= tol, "{tag}: concentric");
            // everything on the centre line: the same room above and below, on whole device px
            for r in [l.cover, l.btn[0], l.btn[1], l.btn[2]] {
                assert!(((r.y - cap.y) - (cap.bottom() - r.bottom())).abs() < 0.01, "{tag}: {r:?} centred in {cap:?}");
                assert!(((r.y - cap.y) * scale - ((r.y - cap.y) * scale).round()).abs() < 0.02, "{tag}: on a device px");
            }
            assert!(l.cover.h <= cap.h - 8.0 + 0.01 && l.btn[1].h <= cap.h - 8.0 + 0.01, "{tag}: room round both");
            // the hairline lies in the straight part of the capsule (the rounded ends are never reached) and clear of its edge
            if l.bar.h > 0.0 {
                assert!(l.bar.x >= cap.x + cap.h / 2.0 - 0.01 && l.bar.right() <= cap.right() - cap.h / 2.0 + 0.01, "{tag}: {:?}", l.bar);
                assert!(cap.bottom() - l.bar.bottom() >= 2.0 - 0.01, "{tag}: off the bottom edge");
            }
        }
    }

    #[test]
    fn the_artist_gives_way_before_the_title_is_cut() {
        assert_eq!(bar_line(200.0, 80.0, 60.0), (80.0, Some((80.0 + LINE_GAP, 60.0))), "both fit");
        assert_eq!(bar_line(200.0, 150.0, 90.0), (150.0, Some((158.0, 42.0))), "a whole title and what is left of the artist, cut");
        assert_eq!(bar_line(200.0, 185.0, 90.0), (185.0, None), "no room worth an artist: it is dropped and the title is whole");
        assert_eq!(bar_line(200.0, 260.0, 50.0), (200.0, None), "only a title alone on the line is cut");
        assert_eq!(bar_line(200.0, 200.0, 50.0), (200.0, None));
        assert_eq!(bar_line(200.0, 150.0, 20.0).1, Some((158.0, 20.0)), "a short artist needs only its own length");
        assert_eq!(bar_line(200.0, 80.0, 0.0), (80.0, None), "no artist, nothing to place");
        for head in [0.0, 40.0, 120.0, 199.0, 400.0] {
            let (hw, sub) = bar_line(200.0, head, 120.0);
            assert!(hw <= 200.0 && sub.is_none_or(|(x, w)| x >= hw && x + w <= 200.0 + 0.001), "{head}: nothing runs out of the line");
            if hw < head {
                assert!(sub.is_none(), "{head}: the title is cut, so the artist is gone");
            }
        }
    }

    #[test]
    fn in_the_bar_a_click_on_the_cover_or_the_title_toggles_the_card_and_the_buttons_stay_buttons() {
        let st = in_bar(Align::Left, 48.0);
        let (cap, _) = frames(st, 0.0, 0.0);
        let l = capsule_layout_in(st, cap);
        assert_eq!(hit(st, 0.0, 0.0, l.cover.center()), Hit::Header);
        assert_eq!(hit(st, 0.0, 0.0, l.text.center()), Hit::Header);
        assert_eq!(hit(st, 0.0, 0.0, (l.text.right() + 1.0, l.text.cy())), Hit::Header, "the gap before the buttons too");
        for (i, b) in BTNS.iter().enumerate() {
            assert_eq!(hit(st, 0.0, 0.0, l.btn[i].center()), Hit::Button(*b));
        }
        let (_, card) = frames(st, 1.0, 0.0);
        let c = card_layout(card);
        assert_eq!(hit(st, 1.0, 0.0, c.cover.center()), Hit::Header);
        assert_eq!(hit(st, 1.0, 0.0, c.title.center()), Hit::Header);
        assert_eq!(hit(st, 1.0, 0.0, c.album.center()), Hit::Body, "elsewhere on the card is just the card");
        assert_eq!(hit(st, 1.0, 0.0, c.btn[0].center()), Hit::Button(Btn::Prev));
        // the screen-edge widgets have no header: hover opens them
        let (cap, _) = frames(Dock::Bottom, 0.0, 0.0);
        assert_eq!(hit(Dock::Bottom, 0.0, 0.0, capsule_layout(cap).cover.center()), Hit::Body);
    }

    #[test]
    fn in_the_bar_the_hover_strip_reaches_down_to_the_folded_capsule_and_the_clicks_follow_the_open_state() {
        let st = in_bar(Align::Left, 48.0);
        let (cap, _) = frames(st, 0.0, 0.0);
        let play = capsule_layout_in(st, cap).btn[1].center();
        assert_eq!(hit(st, 0.0, 0.0, play), Hit::Button(Btn::Play), "the small buttons of the folded capsule");
        assert_eq!(hit(st, 0.0, 0.0, (2.0, 2.0)), Hit::None, "outside the body the window is click-through");
        // open: the card floats 8 px over the bar; the cursor still resting on the capsule's place stays on the hover strip
        let open = hit_rect(st, 1.0, 0.0);
        let folded = body(st, 0.0, 0.0);
        assert!(open.contains(folded.cx(), folded.cy()) && open.bottom() >= folded.bottom() - 0.01, "{open:?} reaches {folded:?}");
        assert_eq!(hit(st, 1.0, 0.0, (folded.cx(), folded.cy())), Hit::Header, "the strip the card floats over folds it again with a click");
        let (_, card) = frames(st, 1.0, 0.0);
        let c = card_layout(card);
        assert_eq!(hit(st, 1.0, 0.0, c.btn[1].center()), Hit::Button(Btn::Play), "the card's buttons float above the bar");
        assert!(matches!(hit(st, 1.0, 0.0, (c.bar.cx(), c.bar.cy())), Hit::Seek(f) if (f - 0.5).abs() < 0.01));
        // nothing above the open card and nothing under the window's floor answers
        assert_eq!(hit(st, 1.0, 0.0, (c.bar.cx(), card.y - 6.0)), Hit::None);
        // the plain bodies keep their own rectangle
        assert_eq!(hit_rect(Dock::Bottom, 0.5, 0.0), body(Dock::Bottom, 0.5, 0.0));
    }

    #[test]
    fn a_taskbar_dock_without_a_bar_is_the_plain_bottom_widget_in_the_taller_window() {
        let st = in_bar(Align::Right, 0.0);
        assert!(!st.in_bar() && st.win() == WIN_BAR && Stand::from(Dock::Bottom).win() == WIN);
        for p in [0.0, 0.5, 1.0] {
            let (a, b) = (body(st, p, 0.0), body(Dock::Bottom, p, 0.0));
            assert_eq!((a.x, a.w, a.h, WIN_BAR.1 - a.bottom()), (b.x, b.w, b.h, PAD), "p={p}: standing on the floor of its window, {PAD} px up");
        }
        assert_eq!(frames(st, 0.3, 0.0), (body(st, 0.0, 0.0), body(st, 1.0, 0.0)));
        assert_eq!(capsule_layout_in(st, body(st, 0.0, 0.0)).btn[1], capsule_layout(body(st, 0.0, 0.0)).btn[1], "the roomy layout");
        assert_eq!(Stand::from(Dock::Top).fold_h(), CAPSULE_H);
        assert!(win_fits());
    }

    /// The window of the taskbar dock holds the card at the tallest bar and the widest body.
    fn win_fits() -> bool {
        let b = body(in_bar(Align::Center, geom::WIDGET_BAR_MAX_H), 1.0, 0.0);
        b.x >= 0.0 && b.y >= 0.0 && b.right() <= WIN_BAR.0 && b.bottom() <= WIN_BAR.1
    }

    // ----- the card, looked at

    #[test]
    fn the_card_has_the_same_padding_on_all_four_sides_and_concentric_radii() {
        let c = Rect::new(0.0, 0.0, BODY_W, CARD_H);
        let l = card_layout(c);
        let play = l.btn[1];
        // left, top, right, bottom: the cover, the track and the time labels, the play disc
        assert_eq!((l.cover.x - c.x, l.cover.y - c.y), (CARD_PAD, CARD_PAD));
        assert_eq!((l.bar.x - c.x, c.right() - l.bar.right()), (CARD_PAD, CARD_PAD), "the track keeps the padding on both sides");
        assert_eq!((l.elapsed.x, l.total.right()), (l.bar.x, l.bar.right()), "the times hang under the ends of the track");
        assert_eq!(l.elapsed.x, l.cover.x, "cover, track and time share one left edge");
        assert_eq!(c.bottom() - play.bottom(), CARD_PAD, "the play disc stands on the bottom padding (was 2 px)");
        for r in [l.title, l.sub, l.album] {
            assert!(r.right() <= c.right() - CARD_PAD + 0.01, "text stays inside the right padding: {r:?}");
        }
        // vertical rhythm: cover, track, times, transport row, in that order, none touching
        assert!(l.cover.bottom() < l.bar.y && l.bar.bottom() <= l.elapsed.y && l.elapsed.bottom() < play.y);
        // the three lines are one block centred against the cover: the same room above the title as under the album
        assert!(((l.title.y - l.cover.y) - (l.cover.bottom() - l.album.bottom())).abs() < 0.01);
        assert!(l.title.y > l.cover.y && l.album.bottom() < l.cover.bottom(), "and inside the cover's height");
        // the transport row is symmetrical: equal gaps between the discs, centred on the card
        let mid = c.cx();
        assert!(((mid - l.btn[0].cx()) - (l.btn[2].cx() - mid)).abs() < 0.01 && (play.cx() - mid).abs() < 0.01);
        assert!(((play.x - l.btn[0].right()) - (l.btn[2].x - play.right())).abs() < 0.01);
        assert!(play.w >= 40.0 && l.btn[0].w >= 32.0, "hittable");
        // one radius for all four corners; the cover's is the concentric one
        assert_eq!(body_radius(1.0, CARD_H), CARD_R);
        assert_eq!(body_radius(0.0, CAPSULE_H), CAPSULE_H / 2.0, "the capsule is a full pill");
        assert_eq!(card_cover_radius() + CARD_PAD, CARD_R, "the cover's corner arcs share the card's corner centre");
        assert!(body_radius(0.5, 120.0) < 28.0 && body_radius(0.5, 120.0) > CARD_R);
        // the shadow stays inside the window's margin on every side, at the top edge too
        assert!(SHADOW_BLUR + SHADOW_DY < PAD);
    }

    #[test]
    fn the_card_in_the_top_window_is_the_mirror_of_the_bottom_one() {
        // top: the body hangs PAD below the ceiling; bottom: it stands PAD above the floor; the open card is as tall as the window allows
        let (t, b) = (body(Dock::Top, 1.0, 0.0), body(Dock::Bottom, 1.0, 0.0));
        assert_eq!((t.y, WIN.1 - b.bottom()), (PAD, PAD));
        assert_eq!((t.h, b.h, WIN.1 - t.bottom()), (CARD_H, CARD_H, PAD), "the same padding under the card at the top edge");
        assert_eq!(t.x, b.x);
    }

    // ----- the size

    fn on_device_px(v: f32, scale: f32) -> bool {
        (v * scale - (v * scale).round()).abs() < 0.02
    }

    #[test]
    fn the_glide_shift_moves_the_whole_layout_along_the_window_and_nothing_else() {
        for (align, scale) in [(Align::Center, 1.0), (Align::Left, 1.5), (Align::Right, 2.0)] {
            let at = |dx| Stand { dock: Dock::Taskbar, align, bar: 48.0, scale, k: 1.0, dx };
            let (o, s) = (at(0.0).origin(), at(30.0).origin());
            assert!((s.0 - o.0 - 30.0).abs() <= 0.5 / scale && s.1 == o.1, "{align:?} @{scale}: {o:?} -> {s:?}");
            let r = at(0.0).rect_to_real(Rect::new(10.0, 10.0, 50.0, 20.0));
            let t = at(-30.0).rect_to_real(Rect::new(10.0, 10.0, 50.0, 20.0));
            assert!((r.x - t.x - 30.0).abs() <= 0.5 / scale && r.y == t.y && (r.w, r.h) == (t.w, t.h));
        }
    }

    #[test]
    fn size_follows_the_island_in_range_and_the_default_is_the_old_layout() {
        assert_eq!((size_k(100.0), size_k(80.0), size_k(200.0), size_k(50.0), size_k(900.0)), (1.0, 0.8, 2.0, KMIN, KMAX));
        assert_eq!(size_k(125.0), 1.25);
        // at 100 % the body stands PAD from its corner exactly where it always did
        let b = Stand::from(Dock::Bottom);
        assert_eq!(b.origin(), (0.0, WIN_REAL.1 - WIN.1));
        assert_eq!(b.rect_to_real(body(b, 0.0, 0.0)), Rect::new(PAD, WIN_REAL.1 - PAD - CAPSULE_H, BODY_W, CAPSULE_H));
        let t = Stand::from(Dock::Top);
        assert_eq!(t.rect_to_real(body(t, 0.0, 0.0)).y, PAD);
        assert_eq!(t.to_base(t.to_real((40.0, 50.0))), (40.0, 50.0));
    }

    #[test]
    fn every_size_edge_align_bar_and_dpi_keeps_the_body_in_its_window_on_device_pixels_and_anchored() {
        let scales = [1.0f32, 1.25, 1.5, 1.75, 2.0];
        for &pct in &[80.0, 100.0, 125.0, 150.0, 175.0, 200.0] {
            let k = size_k(pct);
            for &scale in &scales {
                for align in [Align::Left, Align::Center, Align::Right] {
                    for (dock, bar_real) in [(Dock::Bottom, 0.0f32), (Dock::Top, 0.0), (Dock::Taskbar, 0.0), (Dock::Taskbar, 40.0), (Dock::Taskbar, 48.0), (Dock::Taskbar, 52.0), (Dock::Taskbar, 72.0)] {
                        let bar_real = geom::phys(bar_real, scale) as f32 / scale;
                        let st = Stand { dock, align, bar: bar_real / k, scale, k, dx: 0.0 };
                        let win = st.real_win();
                        let tag = format!("{pct}% {dock:?} {align:?} bar {bar_real} @{scale}");
                        let tol = 1.5 / scale;
                        let o = st.origin();
                        assert!(on_device_px(o.0, scale) && on_device_px(o.1, scale), "{tag}: the origin is on a device px: {o:?}");
                        for p in [0.0f32, 0.5, 1.0] {
                            let r = st.rect_to_real(body(st, p, 0.0));
                            assert!(r.x >= -0.01 && r.y >= -0.01 && r.right() <= win.0 + 0.01 && r.bottom() <= win.1 + 0.01, "{tag} p={p}: {r:?} inside {win:?}");
                            if p == 0.0 || p == 1.0 {
                                for v in [r.x, r.y, r.right(), r.bottom()] {
                                    assert!(on_device_px(v, scale), "{tag} p={p}: {v} is on a device px: {r:?}");
                                }
                            }
                            // the hit region always holds the body, and a point of the body maps back into it
                            let h = st.rect_to_real(hit_rect(st, p, 0.0));
                            assert!(h.x <= r.x + 0.01 && h.right() >= r.right() - 0.01 && h.y <= r.y + 0.01 && h.bottom() >= r.bottom() - 0.01, "{tag}");
                            let c = st.to_base((r.cx(), r.cy()));
                            let b = body(st, p, 0.0);
                            assert!((c.0 - b.cx()).abs() < 0.01 && (c.1 - b.cy()).abs() < 0.01, "{tag}: to_base undoes to_real");
                        }
                        let (cap, card) = (st.rect_to_real(body(st, 0.0, 0.0)), st.rect_to_real(body(st, 1.0, 0.0)));
                        // the card is the card at any size
                        assert!((card.w - BODY_W * k).abs() <= tol && (card.h - CARD_H * k).abs() <= tol, "{tag}: the card scales: {card:?}");
                        if st.in_bar() {
                            // the capsule is as big as the size says but never taller than the bar leaves it (the old cap), centred in the bar
                            let cap_h = geom::bar_fold_cap(bar_real).min(CAPSULE_H * k);
                            assert!(cap.h <= cap_h + tol && cap.h >= (cap_h - 2.0 / scale).max(1.0), "{tag}: {} tall, cap {cap_h}", cap.h);
                            let (floor, top) = (win.1, win.1 - bar_real);
                            assert!(cap.y >= top - 0.01 && cap.bottom() <= floor + 0.01, "{tag}: inside the strip");
                            assert!(((cap.y - top) - (floor - cap.bottom())).abs() < 0.01, "{tag}: centred in the bar");
                            assert!(cap.h <= bar_real - 8.0 + 0.01 || bar_real < 24.0, "{tag}: never beyond what the bar allows");
                            assert!((cap.w - st.px(BAR_FOLD_W) * k).abs() <= tol, "{tag}: the width follows the size");
                            match align {
                                Align::Left => assert!((cap.x - BAR_PAD).abs() <= tol, "{tag}: anchored to the slot's left: {cap:?}"),
                                Align::Right => assert!((win.0 - BAR_PAD - cap.right()).abs() <= tol, "{tag}: anchored to the slot's right"),
                                Align::Center => assert!((cap.cx() - win.0 / 2.0).abs() <= tol, "{tag}: centred on the slot"),
                            }
                            // the open card floats the same gap above the bar, in the window
                            assert!((win.1 - bar_real - k * geom::BAR_FLOAT_GAP - card.bottom()).abs() <= tol, "{tag}: floats over the bar: {card:?}");
                        } else {
                            assert!((cap.h - CAPSULE_H * k).abs() <= tol && (cap.w - BODY_W * k).abs() <= tol, "{tag}: the capsule scales: {cap:?}");
                            // the margins to the corner / edge are PAD at the size, so the shadow (PAD) is never cut by the window
                            let m = PAD * k;
                            match align {
                                Align::Left => assert!((cap.x - m).abs() <= tol, "{tag}: {cap:?}"),
                                Align::Right => assert!((win.0 - cap.right() - m).abs() <= tol, "{tag}"),
                                Align::Center => assert!((cap.cx() - win.0 / 2.0).abs() <= tol, "{tag}"),
                            }
                            if dock == Dock::Top {
                                assert!((card.y - m).abs() <= tol && (card.bottom() + m - (WIN.1 * k)).abs() <= tol, "{tag}: the card hangs from the top edge: {card:?}");
                            } else {
                                assert!((win.1 - cap.bottom() - m).abs() <= tol && (win.1 - card.bottom() - m).abs() <= tol, "{tag}: it stands PAD above the floor");
                            }
                            let reach = (SHADOW_BLUR + SHADOW_DY) * k;
                            assert!(card.x - reach >= -0.01 && card.right() + reach <= win.0 + 0.01 && card.y - reach >= -0.01 && card.bottom() + reach <= win.1 + 0.01, "{tag}: the shadow fits in the window");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_corner_the_body_grows_from_does_not_move_while_the_size_changes() {
        // the animation of a size change: the dock corner / side stays put, the body grows away from it (no jump of the window either: it is fixed)
        for (dock, align) in [(Dock::Bottom, Align::Left), (Dock::Bottom, Align::Right), (Dock::Top, Align::Left), (Dock::Top, Align::Center), (Dock::Taskbar, Align::Left)] {
            let mut prev: Option<Rect> = None;
            for i in 0..=24 {
                let k = 0.8 + 1.2 * i as f32 / 24.0;
                let st = Stand { dock, align, bar: 0.0, scale: 1.0, k, dx: 0.0 };
                let r = st.rect_to_real(body(st, 0.0, 0.0));
                let corner = (match align { Align::Left => r.x, Align::Right => r.right(), Align::Center => r.cx() }, if dock == Dock::Top { r.y } else { r.bottom() });
                if let Some(p) = prev {
                    let pc = (match align { Align::Left => p.x, Align::Right => p.right(), Align::Center => p.cx() }, if dock == Dock::Top { p.y } else { p.bottom() });
                    // the anchor side moves only with the margin (PAD * dk), and never the way the growth goes
                    assert!((corner.0 - pc.0).abs() <= PAD * 0.06 + 1.01 && (corner.1 - pc.1).abs() <= PAD * 0.06 + 1.01, "{dock:?} {align:?} k={k}: {pc:?} -> {corner:?}");
                    assert!(r.w >= p.w - 1.0 && r.h >= p.h - 1.0, "it only grows");
                }
                prev = Some(r);
            }
        }
    }
}
