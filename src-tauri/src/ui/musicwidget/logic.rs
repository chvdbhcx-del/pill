//! The pure part of the music widget: the track as the widget sees it, when it is on screen, where everything stands in its window and what a
//! click on a spot means. No drawing, no window, no clock of its own (every function gets `now`), so all of it is unit-tested.
//!
//! The look is the island's own player: the open card IS the Home player card (its geometry, `Geo`, is used as it is, in the style `home.player`
//! says) and the folded capsule is the island's mini music strip (cover, title, previous / play / next), standing alone.
//!
//! The window is one fixed frame, [`WIN`] logical px, that never resizes: the capsule (collapsed) and the card (open) both stand flush against
//! the window's DOCK edge (the bottom for `media.widget.edge` = bottom, the top otherwise) and the body between them grows away from that edge,
//! like the island does. Everything outside the body is click-through (the view reports the body as its hit region).
//!
//! In the taskbar (`media.widget.edge` = taskbar) the window is taller ([`WIN_BAR`]: its floor is the monitor's bottom, over the bar, with room
//! above it for the open card) and the folded capsule is a compact one that fits the bar strip; see [`Stand`].
use crate::ui::host::geom::{self, mirror_y, Align, Dock};
use glasskit::geom::Rect;
use crate::ui::tabs::a::home::player::Geo;
use pill_ui_core::panel::a::home_player::PlayerStyle;
use serde_json::Value;

/// Width of the folded capsule (screen edges) and of the card: the card is the island's Home player card, 412 x 154 (`tabs::a::home::layout::PLAYER`).
pub const CAP_W: f32 = 320.0;
pub const BODY_W: f32 = 412.0;
/// Height of the collapsed capsule and of the open card.
pub const CAPSULE_H: f32 = 56.0;
pub const CARD_H: f32 = 154.0;
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

    /// The Home card's second line: the artist and the album, `artist - album` (either may be missing).
    pub fn artist_line(&self) -> String {
        [self.artist.as_str(), self.album.as_str()].iter().filter(|x| !x.trim().is_empty()).copied().collect::<Vec<_>>().join(" - ")
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
    /// the island's `home.player` style: the layout of the card
    pub style: PlayerStyle,
}

impl From<Dock> for Stand {
    fn from(dock: Dock) -> Stand {
        Stand { dock, align: Align::Left, bar: 0.0, scale: 1.0, k: 1.0, dx: 0.0, style: PlayerStyle::default() }
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

    /// Width of the folded capsule: the bar's slot ([`BAR_FOLD_W`]) in the taskbar strip, else [`CAP_W`] (a whole number of device px).
    pub fn fold_w(&self) -> f32 {
        self.px(if self.in_bar() { BAR_FOLD_W } else { CAP_W })
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
    let (fh, fw) = (st.fold_h(), st.fold_w());
    let h = (fh + (CARD_H - fh) * p).max(8.0);
    // the width grows from the capsule's to the card's on the anchored side (the middle for the centre)
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
    let (top, bottom) = if st.in_bar() {
        // the vertical edges on device px (a size where the float gap is not a whole number of them would blur the open card's outline)
        (st.px(win.1 - geom::bar_gap(st.bar, fh, p) + dy - h), st.px(win.1 - geom::bar_gap(st.bar, fh, p) + dy))
    } else {
        let y = match st.dock {
            Dock::Top => PAD - dy,
            // the plain bottom edge, and the taskbar dock where there is no bar to stand in
            Dock::Bottom | Dock::Taskbar => mirror_y(PAD - dy, h, win.1),
        };
        // every edge on a device px (at rest, at any size and DPI): crisp outline, the same margin on all sides
        (st.px(y), st.px(y + h))
    };
    Rect::new(x, top, w, bottom - top)
}

/// The corner radius of the body: the capsule's full half circle closing into the card's [`CARD_R`] (the island's `Theme::m.radius`).
pub fn body_radius(p: f32, h: f32) -> f32 {
    (CAPSULE_H / 2.0 - (CAPSULE_H / 2.0 - CARD_R) * p.clamp(0.0, 1.0)).min(h / 2.0)
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

/// Every rectangle of the card, relative to its frame `c`: the Home player card's `Geo` (the island's own numbers) moved to where the card stands.
pub struct CardLayout {
    pub cover: Rect,
    /// the source label (the player's name)
    pub src: Rect,
    pub title: Rect,
    pub sub: Rect,
    /// the progress track (the loudness bars in the widget)
    pub bar: Rect,
    pub elapsed: Rect,
    pub total: Rect,
    pub btn: [Rect; 3],
    /// title and artist centred (the mirror style) or left
    pub centered: bool,
}

/// The card's corner radius: the island's (`Metrics::PILL.radius`, what `th.m.radius` is for the island's own cards).
pub const CARD_R: f32 = glasskit::theme::Metrics::PILL.radius;
/// The biggest cover of the card (the compact style's), what the picture is decoded for.
pub const COVER: f32 = 88.0;
/// The cover's corner radius in the card (the island's `paint_art`) and in the capsule (the island's mini strip, `Gpu::cover`).
pub const CARD_COVER_R: f32 = 14.0;
pub const CAP_COVER_R: f32 = 7.0;

pub fn card_layout(c: Rect, style: PlayerStyle) -> CardLayout {
    let g = Geo::new(style, false);
    let o = |r: Rect| r.offset(c.x, c.y);
    CardLayout { cover: o(g.art), src: o(g.src), title: o(g.title), sub: o(g.artist), bar: o(g.seek), elapsed: o(g.t0), total: o(g.t1), btn: [o(g.prev), o(g.main), o(g.next)], centered: g.centered }
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
    let (a, b) = (capsule_layout_in(st, cap), card_layout(card, st.style));
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
        cover_r: lerp(ar, CARD_COVER_R, t),
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
        let l = card_layout(card, st.style);
        if st.in_bar() && (l.cover.contains(pos.0, pos.1) || l.title.contains(pos.0, pos.1) || l.sub.contains(pos.0, pos.1) || pos.1 > card.bottom()) {
            return Hit::Header;
        }
    } else if st.in_bar() && pos.0 < pt.btn[0].x {
        return Hit::Header;
    }
    Hit::Body
}

/// The loudness bars in the rectangle `r` (the progress track's place): `n` bars with `gap` between them, bar `i` as `(x, width)`. Pure, so the
/// picture and the seek agree.
pub fn wave_cell(r: Rect, n: usize, gap: f32, i: usize) -> (f32, f32) {
    let w = (r.w - gap * (n.saturating_sub(1)) as f32) / n.max(1) as f32;
    (r.x + i as f32 * (w + gap), w)
}

#[cfg(test)]
#[path = "logic_tests.rs"]
mod tests;
