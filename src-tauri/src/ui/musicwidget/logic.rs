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
pub const CARD_H: f32 = 184.0;
/// Room around the body inside the window (soft shadow, the overshoot of the springs).
pub const PAD: f32 = 12.0;
/// The fixed window, logical px.
pub const WIN: (f32, f32) = (BODY_W + 2.0 * PAD, CARD_H + 2.0 * PAD);
/// The folded capsule in the taskbar: narrower than the card, as tall as the bar allows (see [`Stand::fold_h`]).
pub const BAR_FOLD_W: f32 = 288.0;
/// The window in the taskbar: the open card floats [`geom::BAR_FLOAT_GAP`] above the tallest bar the widget lives in ([`geom::WIDGET_BAR_MAX_H`]).
pub const WIN_BAR: (f32, f32) = (WIN.0, PAD + CARD_H + geom::BAR_FLOAT_GAP + geom::WIDGET_BAR_MAX_H);
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
    pub bar: f32,
}

impl From<Dock> for Stand {
    fn from(dock: Dock) -> Stand {
        Stand { dock, align: Align::Left, bar: 0.0 }
    }
}

impl Stand {
    /// The capsule stands inside the taskbar strip right now.
    pub fn in_bar(&self) -> bool {
        self.dock == Dock::Taskbar && self.bar > 0.0
    }

    /// The window's size, logical px (it never changes for a given dock).
    pub fn win(&self) -> (f32, f32) {
        if self.dock == Dock::Taskbar {
            WIN_BAR
        } else {
            WIN
        }
    }

    /// Height of the folded capsule: [`CAPSULE_H`], or what the bar leaves (`geom::bar_fold_cap`) when that is less.
    pub fn fold_h(&self) -> f32 {
        if self.in_bar() {
            geom::bar_fold_cap(self.bar).min(CAPSULE_H)
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
    let win = st.win();
    if st.in_bar() {
        let fh = st.fold_h();
        let h = (fh + (CARD_H - fh) * p).max(8.0);
        let w = (BAR_FOLD_W + (BODY_W - BAR_FOLD_W) * p).max(8.0);
        let x = match st.align {
            Align::Left => PAD,
            Align::Right => win.0 - PAD - w,
            Align::Center => (win.0 - w) / 2.0,
        };
        return Rect::new(x, win.1 - geom::bar_gap(st.bar, fh, p) + dy - h, w, h);
    }
    let h = (CAPSULE_H + (CARD_H - CAPSULE_H) * p).max(8.0);
    let y = match st.dock {
        Dock::Top => PAD - dy,
        // the plain bottom edge, and the taskbar dock where there is no bar to stand in
        Dock::Bottom | Dock::Taskbar => mirror_y(PAD - dy, h, win.1),
    };
    Rect::new(PAD, y, BODY_W, h)
}

/// What answers the cursor: the body, and in the bar also the strip down to where the folded capsule stands, so that the cursor resting on the
/// capsule keeps the card open while the card floats above the bar (otherwise it would fold the moment it rose off the cursor).
pub fn hit_rect(st: impl Into<Stand>, p: f32, dy: f32) -> Rect {
    let st = st.into();
    let b = body(st, p, dy);
    if !st.in_bar() {
        return b;
    }
    let floor = st.win().1 - geom::bar_gap(st.bar, st.fold_h(), 0.0) + dy;
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

/// The capsule in the taskbar strip: a small cover, one line of text, three small buttons and a hairline progress bar, scaled to the strip
/// (`c.h` is what the bar leaves, see [`Stand::fold_h`]). The bar is dropped (zero height) when there is no room for it under the buttons.
pub fn bar_capsule_layout(c: Rect) -> CapsuleLayout {
    let h = c.h;
    let cs = (h - 8.0).clamp(14.0, 40.0);
    let bs = (h - 10.0).clamp(16.0, 28.0);
    let step = bs + 2.0;
    let m = ((h - cs) / 2.0).max(2.0);
    let b = |i: f32| Rect::new(c.right() - m - (3.0 - i) * step + 1.0, c.y + (h - bs) / 2.0, bs, bs);
    let btn = [b(0.0), b(1.0), b(2.0)];
    let cover = Rect::new(c.x + m, c.y + (h - cs) / 2.0, cs, cs);
    let text_x = cover.right() + 8.0;
    let bar = if h >= 26.0 { Rect::new(c.x + h / 2.0, c.bottom() - 3.0, (c.w - h).max(0.0), 1.5) } else { Rect::new(c.x, c.bottom(), 0.0, 0.0) };
    CapsuleLayout { cover, text: Rect::new(text_x, c.y, (btn[0].x - 6.0 - text_x).max(0.0), h), btn, bar }
}

/// The capsule's layout for the way the body stands: the compact one in the bar.
pub fn capsule_layout_in(st: impl Into<Stand>, c: Rect) -> CapsuleLayout {
    if st.into().in_bar() {
        bar_capsule_layout(c)
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

pub fn card_layout(c: Rect) -> CardLayout {
    let tx = c.x + 16.0 + 88.0 + 14.0;
    let tw = c.right() - 16.0 - tx;
    let cy = c.y + 160.0;
    let mid = c.x + c.w / 2.0;
    CardLayout {
        cover: Rect::new(c.x + 16.0, c.y + 16.0, 88.0, 88.0),
        title: Rect::new(tx, c.y + 20.0, tw, 22.0),
        sub: Rect::new(tx, c.y + 44.0, tw, 18.0),
        album: Rect::new(tx, c.y + 64.0, tw, 16.0),
        bar: Rect::new(c.x + 16.0, c.y + 116.0, c.w - 32.0, 6.0),
        elapsed: Rect::new(c.x + 16.0, c.y + 124.0, 80.0, 14.0),
        total: Rect::new(c.right() - 16.0 - 80.0, c.y + 124.0, 80.0, 14.0),
        btn: [Rect::new(mid - 64.0 - 18.0, cy - 18.0, 36.0, 36.0), Rect::new(mid - 22.0, cy - 22.0, 44.0, 44.0), Rect::new(mid + 64.0 - 18.0, cy - 18.0, 36.0, 36.0)],
    }
}

/// What is under a point of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    Button(Btn),
    /// the progress bar of the card; the fraction of the track the x stands for
    Seek(f32),
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
    let (cap, card) = frames(st, p, dy);
    let grab = |r: Rect| Rect::new(r.x - 4.0, r.y - 8.0, r.w + 8.0, r.h + 16.0).contains(pos.0, pos.1);
    if p >= CARD_AT {
        let l = card_layout(card);
        if let Some(i) = l.btn.iter().position(|r| r.contains(pos.0, pos.1)) {
            return Hit::Button(BTNS[i]);
        }
        if grab(l.bar) {
            return Hit::Seek(seek_at(l.bar, pos.0));
        }
    } else {
        let l = capsule_layout_in(st, cap);
        if let Some(i) = l.btn.iter().position(|r| r.contains(pos.0, pos.1)) {
            return Hit::Button(BTNS[i]);
        }
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
        // half way: the capsule's buttons while it is mostly closed
        assert_eq!(hit(d, CARD_AT - 0.01, 0.0, play_c), Hit::Button(Btn::Play));
        assert_eq!(seek_at(c.bar, c.bar.x + c.bar.w * 3.0), 1.0);
    }

    // ----- in the taskbar

    fn in_bar(align: Align, bar: f32) -> Stand {
        Stand { dock: Dock::Taskbar, align, bar }
    }

    /// The bar heights of the common taskbars at the common scalings, as the view is told them (logical px).
    fn bars() -> Vec<f32> {
        let mut v = Vec::new();
        for dpi in [96u32, 120, 144, 192] {
            let s = geom::scale_from_dpi(dpi);
            for bar in [40.0f32, 48.0, 52.0] {
                v.push(geom::phys(bar, s) as f32 / s);
            }
        }
        v
    }

    #[test]
    fn the_folded_capsule_sits_inside_the_bar_centred_and_fits_it() {
        for bar in bars() {
            for align in [Align::Left, Align::Center, Align::Right] {
                let st = in_bar(align, bar);
                let b = body(st, 0.0, 0.0);
                let (floor, top) = (WIN_BAR.1, WIN_BAR.1 - bar);
                assert!(b.y >= top - 0.01 && b.bottom() <= floor + 0.01, "{bar}: {b:?} inside the strip [{top}, {floor}]");
                assert!(((b.y - top) - (floor - b.bottom())).abs() < 0.01, "{bar}: centred in the bar");
                assert!(b.h <= bar - 8.0 + 0.01 && b.h <= CAPSULE_H && b.h >= 16.0, "{bar}: {} tall", b.h);
                assert_eq!(b.w, BAR_FOLD_W);
                match align {
                    Align::Left => assert_eq!(b.x, PAD),
                    Align::Right => assert_eq!(b.right(), WIN_BAR.0 - PAD),
                    Align::Center => assert!((b.cx() - WIN_BAR.0 / 2.0).abs() < 0.01),
                }
                assert!(b.x >= 0.0 && b.right() <= WIN_BAR.0, "inside the window");
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
        for bar in bars() {
            let st = in_bar(Align::Left, bar);
            let (cap, _) = frames(st, 0.0, 0.0);
            let l = capsule_layout_in(st, cap);
            for r in [l.cover, l.text, l.btn[0], l.btn[1], l.btn[2]] {
                assert!(r.x >= cap.x - 0.01 && r.right() <= cap.right() + 0.01 && r.y >= cap.y - 0.01 && r.bottom() <= cap.bottom() + 0.01, "{bar}: {r:?} in {cap:?}");
            }
            assert!(l.cover.h >= 14.0 && l.cover.h <= cap.h - 4.0, "{bar}: a small cover");
            assert!(l.btn[0].right() <= l.btn[1].x + 0.01 && l.btn[1].right() <= l.btn[2].x + 0.01, "the buttons do not overlap");
            assert!(l.cover.right() < l.text.x && l.text.right() <= l.btn[0].x && l.text.w > 100.0, "{bar}: one line of text between the cover and the buttons: {:?}", l.text);
            // the progress is a hairline under the buttons, or it is gone
            if l.bar.h > 0.0 {
                assert!(l.bar.h <= 2.0 && l.bar.y >= l.btn[1].bottom() && l.bar.bottom() <= cap.bottom() && l.bar.x >= cap.x && l.bar.right() <= cap.right(), "{bar}: {:?}", l.bar);
            }
        }
        // a bar so thin that nothing fits under the buttons has no progress bar at all
        let thin = capsule_layout_in(in_bar(Align::Left, 24.0), frames(in_bar(Align::Left, 24.0), 0.0, 0.0).0);
        assert_eq!(thin.bar.h, 0.0);
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
        assert_eq!(hit(st, 1.0, 0.0, (folded.cx(), folded.cy())), Hit::Body);
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
}
