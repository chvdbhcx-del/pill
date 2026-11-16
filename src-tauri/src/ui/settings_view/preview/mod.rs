//! The live preview: a panel on the right of the settings window that shows the real island, miniature, fed by the settings as they are being
//! changed (no save, no debounce: the window's own copy of the values), with sample data for everything that is not on screen right now
//! (weather, clipboard, reactions of the folded island). See docs/native-settings.md, "Live preview".
//!
//! * `live.rs`    the sandbox: a second `IslandView` on a `Cx` of its own (own bus, own queue, sample backend), driven by `step`
//! * `scene.rs`   which setting shows what (key -> `Scene`), what is handed over (`shareable`), the captions
//! * `sample.rs`  the invented backend: weather, clipboard, the payloads of the reactions
//! * this file    state, layout (width that animates in and out), the camera (zoom and height follow the island), painting
//!
//! Idle cost: the sandbox runs for a few seconds after a change (`SETTLE_MS`, longer only while something really moves) and then parks:
//! its queue stops waking the window, the open tab stops its ticker, and nothing is stepped or redrawn until the next change.
mod live;
mod sample;
pub mod scene;
#[cfg(test)]
mod tests;

use super::flow::{go, tick};
use super::paint::{SIDE_W, TITLE_H};
use super::pal::mix;
use super::rows::Dc;
use super::txt;
use super::ui::{key, Act};
use super::{open, SettingsView};
use crate::ui::host::geom::{Align as Side, Dock, ALIGN_PAD};
use crate::ui::liquid::{Feel, Flow};
use crate::ui::view::Cx;
use glasskit::canvas::{mul, scale_about, translation};
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::text::{Align, Style, VAlign};
use live::Live;
use pill_ui_core::settings::schema::schema;
use scene::{handled, of_focus, of_key, of_page, shareable, Flash, Look, Scene, CAMERA};
use serde_json::{json, Map, Value};

/// Width of the panel (css px) and the least width the page needs next to it (a card, 24 px on each side): below that the panel stays away.
pub const PV_W: f32 = 320.0;
pub const MIN_PAGE_W: f32 = 580.0;
/// The least window width that has room for both: the title of the setting `settings.preview` says it.
pub const MIN_WINDOW_W: f32 = SIDE_W + 2.0 + MIN_PAGE_W + PV_W;
const SIDE_PAD: f32 = 16.0;
const HEAD_H: f32 = 48.0;
const CAP_H: f32 = 120.0;
const STAGE_TOP: f32 = 10.0;
/// How far the camera stands off the island sideways (room for a pet's tail, the edge glow).
const FOCUS_PAD_X: f32 = 26.0;
/// The sandbox runs this long after a change; a little longer only while something really moves, never past `MAX_MS`.
const SETTLE_MS: f64 = 3000.0;
const MAX_MS: f64 = 9000.0;
/// The open panel needs the folded island out of the way first.
const FOLD_MS: f64 = 520.0;
/// The sketch of a monitor (css px of a small screen) that the camera looks at for the scenes about where things stand: the island and the music
/// widget are drawn at their real size on it, so the side, the edge and the shift along the edge read as they would on a screen.
const SCREEN_W: f32 = 880.0;
const SCREEN_H: f32 = 480.0;

pub struct Preview {
    live: Option<Live>,
    /// the values the sandbox has been told (filtered)
    snap: Map<String, Value>,
    scene: Scene,
    /// the sandbox island is open (as last asked)
    open: bool,
    flip: bool,
    /// events to fire when the sandbox clock reaches the time: (at, name, payload)
    due: Vec<(f64, &'static str, Value)>,
    page: &'static str,
    /// the settings key of the row that has the keyboard / click focus (a function section of a merged page brings its own scene)
    focus_key: Option<&'static str>,
    /// an old page name ("pet", "battery") was opened: the scene of that function, for the page that is coming (`SettingsView::goto_anchor`)
    pub(super) hint: Option<Scene>,
    active: bool,
    /// host clock of the last change
    t0: f64,
    replay: bool,
    /// 0 = no panel, 1 = full width
    vis: Flow,
    /// camera: scale, the stage height, centre x of the island in the sandbox window
    zoom: Flow,
    stage_h: Flow,
    cx_world: f32,
    placed: bool,
    /// the caption fades in when the scene changes
    note: Flow,
    /// 1 = the camera looks at the sketch of a whole monitor (`Scene::screen`), 0 = close up on the island
    mode: Flow,
    /// on the sketch: how far (css px of the screen) the island and the widget are shifted along their edge (`offsetX`, `media.widget.offset`),
    /// and how far the floor is raised for the taskbar they keep clear of (`underTaskbar`)
    dx_i: Flow,
    dx_w: Flow,
    lift_i: Flow,
    lift_w: Flow,
    /// the strip of the imaginary taskbar along the floor of the stage
    strip: Flow,
}

impl Preview {
    pub fn new() -> Preview {
        Preview {
            live: None,
            snap: Map::new(),
            scene: Scene::DEFAULT,
            open: false,
            flip: false,
            due: Vec::new(),
            page: "",
            focus_key: None,
            hint: None,
            active: false,
            t0: 0.0,
            replay: false,
            vis: Flow::new(0.0, Feel::DRAG),
            zoom: Flow::new(1.0, Feel::DRAG),
            stage_h: Flow::px(110.0, Feel::FLOW),
            cx_world: live::VW / 2.0,
            placed: false,
            note: Flow::new(1.0, Feel::FADE),
            mode: Flow::new(0.0, Feel::FLOW),
            dx_i: Flow::px(0.0, Feel::FLOW),
            dx_w: Flow::px(0.0, Feel::FLOW),
            lift_i: Flow::px(0.0, Feel::FLOW),
            lift_w: Flow::px(0.0, Feel::FLOW),
            strip: Flow::new(0.0, Feel::FLOW),
        }
    }

    /// The width the panel takes now (it grows in and out).
    pub fn width(&self) -> f32 {
        PV_W * self.vis.at01()
    }

    pub fn replay(&mut self) {
        self.replay = true;
    }

    pub fn scene(&self) -> Scene {
        self.scene
    }

    /// Is the sandbox running (awake and being stepped)?
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn exists(&self) -> bool {
        self.live.is_some()
    }

    fn touch(&mut self, now: f64) {
        self.active = true;
        self.t0 = now;
        if let Some(l) = &mut self.live {
            l.wake();
        }
    }

    /// Hands the settings that changed since the last look to the sandbox (the camera's own keys stay here); the scene of the first one that
    /// shows something; and whether anything the preview shows has changed.
    fn sync(&mut self, values: &Map<String, Value>) -> (Map<String, Value>, Option<Scene>, bool) {
        let mut patch = Map::new();
        let mut scene = None;
        let mut touched = false;
        for (k, v) in values {
            let share = shareable(k);
            if !share && !CAMERA.contains(&k.as_str()) {
                continue;
            }
            let old = self.snap.get(k).cloned().or_else(|| schema().default_of(k).cloned());
            if old.as_ref() != Some(v) {
                if share {
                    patch.insert(k.clone(), v.clone());
                }
                touched = true;
                scene = scene.or_else(|| of_key(k));
            }
            self.snap.insert(k.clone(), v.clone());
        }
        (patch, scene, touched)
    }

    /// The scene a page opens with. On the Media page the player widget is what its switches are about, so with the widget on the sketch shows
    /// the widget; a row that is about the island's own player (focus on it) still shows the island's music reaction: the island keeps its player.
    fn resolve(&self, sc: Scene) -> Scene {
        if sc == Scene::Flash(Flash::Music) && self.snap.get("media.widget") == Some(&json!(true)) {
            Scene::Widget
        } else {
            sc
        }
    }

    /// Puts the sandbox island into `sc` (animated by the island itself). `fresh`: the scene is new, not a replay.
    fn go_scene(&mut self, sc: Scene, reduce: bool) {
        let prev = self.scene;
        if sc != self.scene {
            self.scene = sc;
            if reduce {
                self.note.jump(1.0);
            } else {
                self.note.jump(0.0);
                self.note.to(1.0);
            }
        }
        self.due.clear();
        let Some(live) = &mut self.live else { return };
        // the music widget stands on the stage in its own scene, and on the sketch of the screen that shows where the island stands (when it is on)
        if sc == Scene::Widget || (sc == Scene::Pill(Look::Place) && self.snap.get("media.widget") == Some(&json!(true))) {
            if self.replay {
                live.widget_restart();
            } else {
                live.widget_show();
            }
        } else {
            live.widget_leave();
        }
        // what the last reaction left on the island (a card, the music row, the recording capsule) goes before the next scene
        let left = matches!(prev, Scene::Flash(_));
        if left {
            live.clear_reactions();
        }
        let now = live.vnow;
        match sc {
            Scene::Panel(tab) => {
                live.emit(if self.open { "pill:open-tab" } else { "pill:expand" }, json!({ "tab": tab.id(), "via": "preview" }));
                self.open = true;
            }
            Scene::Pill(_) => {
                if self.open {
                    live.emit("pill:collapse", Value::Null);
                    self.open = false;
                }
            }
            Scene::Widget => {
                if self.open {
                    live.emit("pill:collapse", Value::Null);
                    self.open = false;
                }
            }
            Scene::Flash(f) => {
                let mut wait = if left { 300.0 } else { 80.0 };
                if self.open {
                    live.emit("pill:collapse", Value::Null);
                    self.open = false;
                    wait = FOLD_MS;
                }
                for (d, name, payload) in sample::flash_events(f, self.flip) {
                    self.due.push((now + wait + d, name, payload));
                }
                self.flip = !self.flip;
            }
        }
    }
}

impl Default for Preview {
    fn default() -> Self {
        Preview::new()
    }
}

/// The settings the sandbox starts with: everything `shareable` the window has.
fn shared_values(values: &Map<String, Value>) -> Map<String, Value> {
    values.iter().filter(|(k, _)| shareable(k)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// What the preview has looked at so far: the sandbox's settings and the camera's own.
fn seen_values(values: &Map<String, Value>) -> Map<String, Value> {
    values.iter().filter(|(k, _)| handled(k)).map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// A `live.*` switch as the island reads it: absent = on.
fn switch_on(model: &pill_ui_core::settings::SettingsModel, key: &str) -> bool {
    match model.get(key) {
        Value::Null => true,
        v => v == json!(true),
    }
}

impl SettingsView {
    /// Is there room, and does the user want it?
    pub(super) fn preview_wanted(&self) -> bool {
        let wiz = self.wiz.active() || self.page.wizard;
        let total = self.size.0 - if self.maximized { 0.0 } else { 2.0 } - SIDE_W;
        self.loaded && !wiz && self.model.get_bool("settings.preview") && total - PV_W >= MIN_PAGE_W
    }

    /// Per frame: the panel's width, the sandbox (settings in, events out, one island step), the camera. True while the window needs frames for it.
    pub(super) fn step_preview(&mut self, dt: f32, cx: &mut Cx) -> bool {
        let reduce = self.reduce();
        let want = self.preview_wanted();
        let fk = self.focus.focused().and_then(|f| self.page.rows().find(|r| r.fid == f)).map(|r| r.row.key).filter(|k| !k.is_empty());
        let pv = &mut self.pv;
        if want != (pv.vis.target() > 0.5) {
            go(&mut pv.vis, want as u8 as f32, reduce);
        }
        let mut busy = tick(&mut pv.vis, dt);
        if !want {
            if !busy {
                pv.live = None; // nothing of it stays alive while the panel is away
                pv.active = false;
                pv.placed = false;
            }
            return busy;
        }
        let now = cx.now_ms;
        let model = &self.model;
        if pv.live.is_none() {
            let ov = self.data.look.overlay(model.values());
            let values = shared_values(&ov);
            pv.snap = seen_values(&ov);
            let live = Live::new(cx, values, &open::system_lang());
            live.emit("weather:current", sample::weather_now());
            pv.live = Some(live);
            pv.open = false;
            pv.page = model.current;
            let sc = pv.hint.take().unwrap_or_else(|| pv.resolve(of_page(model.current)));
            pv.scene = sc;
            pv.go_scene(sc, reduce);
            pv.touch(now);
        }
        // the settings that changed, at once (the window's own copy, not what the backend has saved yet)
        let (patch, sc, touched) = pv.sync(&self.data.look.overlay(model.values()));
        if touched {
            pv.touch(now);
        }
        if !patch.is_empty() {
            if let Some(l) = &mut pv.live {
                l.patch(patch);
            }
        }
        // the focus moved to a row of another function of the same page (Tab, a click): that function's scene comes up
        let focus_scene = if fk != pv.focus_key { fk.and_then(of_focus) } else { None };
        if let Some(sc) = sc {
            pv.go_scene(sc, reduce);
        } else if pv.page != model.current {
            pv.page = model.current;
            pv.touch(now);
            let sc = pv.hint.take().unwrap_or_else(|| pv.resolve(of_page(model.current)));
            pv.go_scene(sc, reduce);
        } else if let Some(fs) = focus_scene.filter(|s| *s != pv.scene) {
            pv.touch(now);
            pv.go_scene(fs, reduce);
        } else if pv.replay {
            pv.touch(now);
            let s = pv.scene;
            pv.go_scene(s, reduce);
        }
        pv.replay = false;
        pv.focus_key = fk;
        busy |= tick(&mut pv.note, dt);
        // the camera's own settings (where along its edge the island and the widget stand, whether they keep clear of the taskbar)
        let off = |k: &str| model.get(k).as_f64().unwrap_or(0.0) as f32;
        let (off_island, off_widget, clear_of_bar) = (off("offsetX"), off("media.widget.offset"), model.get("underTaskbar") != json!(false));
        let screen = pv.scene.screen();
        go(&mut pv.mode, screen as u8 as f32, reduce);
        let Some(live) = &mut pv.live else { return busy };
        if !pv.active && live.has_mail() {
            pv.active = true;
            pv.t0 = now;
            live.wake();
        }
        if pv.active {
            let dt_ms = dt as f64 * 1000.0;
            let due_now: Vec<(f64, &'static str, Value)> = {
                let t = live.vnow + dt_ms;
                let (go_now, later): (Vec<_>, Vec<_>) = pv.due.drain(..).partition(|d| d.0 <= t);
                pv.due = later;
                go_now
            };
            for (_, name, payload) in due_now {
                live.emit(name, payload);
            }
            let moving = live.step(cx, dt_ms, reduce);
            // the camera: the island's body decides how much of the sandbox window to look at
            let (bl, bw, bh) = live.body();
            let sh_max = (self.size.1 - TITLE_H - 2.0 - HEAD_H - CAP_H - SIDE_PAD).max(100.0);
            let (k, h) = if screen {
                // the sketch of a monitor: one fixed scale, so that the size of the island and of the widget reads against the screen
                let k = (PV_W - 2.0 * SIDE_PAD) / SCREEN_W;
                (k, (SCREEN_H * k).clamp(84.0, sh_max))
            } else {
                // a corner island needs room beside it on the stage, or the corner cannot be told from the middle
                let room = if live.aligned() == Side::Center { 1.0 } else { 1.7 };
                // in a taskbar the stage shows the bar under the island too
                let (fw, fh) = ((bw + 2.0 * FOCUS_PAD_X) * room, bh + 6.0 + if live.in_bar() { live::BAR_H } else { 0.0 });
                let k = ((PV_W - 2.0 * SIDE_PAD) / fw.max(1.0)).min((sh_max - STAGE_TOP - 12.0) / fh.max(1.0)).clamp(0.1, 2.0);
                (k, (fh * k + STAGE_TOP + 12.0).clamp(84.0, sh_max))
            };
            pv.cx_world = bl + bw / 2.0;
            // on the sketch: the shift along the edge (a centred body only, and never off the screen), the floor raised over the taskbar
            let shift = |pct: f32, w: f32| (pct / 100.0 * SCREEN_W).clamp(-(SCREEN_W - w).max(0.0) / 2.0, (SCREEN_W - w).max(0.0) / 2.0);
            let in_bar = live.in_bar();
            go(&mut pv.dx_i, if live.aligned() == Side::Center && !in_bar { shift(off_island, bw) } else { 0.0 }, reduce);
            go(&mut pv.lift_i, if live.at_bottom() && !in_bar && clear_of_bar { live::BAR_H } else { 0.0 }, reduce);
            let pg = live.player();
            // the sketch's monitor is small: the room the widget has is measured by its window at 100 % (the window is as big as the largest size needs)
            go(&mut pv.dx_w, pg.filter(|g| g.align == Side::Center).map_or(0.0, |g| shift(off_widget, g.win.0 / crate::ui::musicwidget::logic::KMAX)), reduce);
            go(&mut pv.lift_w, if pg.is_some_and(|g| g.dock == Dock::Bottom) && clear_of_bar { live::BAR_H } else { 0.0 }, reduce);
            go(&mut pv.strip, (screen || in_bar || live.player_in_bar()) as u8 as f32, reduce);
            if pv.placed && !reduce {
                pv.zoom.to(k);
                pv.stage_h.to(h);
            } else if bw > 1.0 {
                pv.zoom.jump(k);
                pv.stage_h.jump(h);
                pv.placed = true;
            }
            let age = now - pv.t0;
            let keep = age < SETTLE_MS || (moving && age < MAX_MS) || !pv.due.is_empty();
            if !keep {
                live.sleep();
                pv.active = false;
            }
            busy |= pv.active;
        }
        for f in [&mut pv.zoom, &mut pv.stage_h, &mut pv.mode, &mut pv.dx_i, &mut pv.dx_w, &mut pv.lift_i, &mut pv.lift_w, &mut pv.strip] {
            busy |= tick(f, dt);
        }
        busy
    }

    /// Paints the panel into `r` (the part of the window it has grown to). Registers its two buttons.
    pub(super) fn paint_preview(&mut self, d: &Dc, r: Rect, cx: &Cx) {
        let vis = self.pv.vis.at01();
        if r.w < 1.0 || vis <= 0.003 {
            return;
        }
        let (cv, p) = (d.cv, d.p);
        let area = Rect::new(r.right() - PV_W, r.y, PV_W, r.h);
        let sw = PV_W - 2.0 * SIDE_PAD;
        let lang = d.lang;
        cv.fill_rect(Rect::new(r.x, r.y, 1.0, r.h), p.side_line);
        cv.with_clip(r, |cv| {
            cv.with_opacity(vis, |cv| {
                // header: the name, then the two buttons at the right
                let head = Rect::new(area.x + SIDE_PAD + 2.0, area.y + 18.0, 120.0, 16.0);
                cv.text(head, &txt::upper(pick(lang, "Превью", "Preview")), Style::new(11.0, 600).track(0.88), p.cap);
                let mut right = area.right() - SIDE_PAD;
                right -= self.pv_chip(d, right, area.y + 14.0, pick(lang, "Скрыть", "Hide"), Act::PvHide) + 6.0;
                if self.pv.exists() {
                    self.pv_chip(d, right, area.y + 14.0, pick(lang, "Повторить", "Replay"), Act::PvReplay);
                }
                // the stage: a desktop behind the island, which hangs from its top edge (or stands on its bottom edge)
                let stage = Rect::new(area.x + SIDE_PAD, area.y + HEAD_H, sw, self.pv.stage_h.at().max(60.0));
                let (c0, c1) = if p.dark { (Color::rgb8(0x1b, 0x22, 0x33), Color::rgb8(0x2c, 0x3a, 0x57)) } else { (Color::rgb8(0xa9, 0xb8, 0xd2), Color::rgb8(0xcb, 0xd6, 0xe8)) };
                cv.with_clip_rr(stage, 12.0, |cv| {
                    cv.fill_rect(stage, c0);
                    cv.fill_rect(Rect::new(stage.x, stage.y, stage.w, stage.h * 0.55), mix(c0, c1, 0.7));
                    let (k, m) = (self.pv.zoom.at().clamp(0.1, 2.0), self.pv.mode.at01());
                    let (dx_i, dx_w, lift_i, lift_w, strip_a) = (self.pv.dx_i.at(), self.pv.dx_w.at(), self.pv.lift_i.at(), self.pv.lift_w.at(), self.pv.strip.at01());
                    if let Some(live) = &mut self.pv.live {
                        // the floor of the stage: a little above its edge close up, the very edge on the sketch of a monitor
                        let floor = lerp(stage.bottom() - STAGE_TOP, stage.bottom(), m);
                        // the taskbar the island or the widget stands in (or keeps clear of): a strip along the floor of the stage, under them
                        if strip_a > 0.003 {
                            let bar = live::BAR_H * k;
                            let strip = Rect::new(stage.x, floor - bar, stage.w, stage.bottom() - floor + bar);
                            cv.fill_rect(strip, (if p.dark { Color::rgba(0.0, 0.0, 0.0, 0.42) } else { Color::rgba(1.0, 1.0, 1.0, 0.5) }).fade(strip_a));
                            cv.fill_rect(Rect::new(strip.x, strip.y, strip.w, 1.0), (if p.dark { Color::rgba(1.0, 1.0, 1.0, 0.10) } else { Color::rgba(0.0, 0.0, 0.0, 0.12) }).fade(strip_a));
                        }
                        // the island hangs from the top edge of the sandbox window, or stands on its bottom edge: the camera follows (on the sketch it
                        // stands above the taskbar, or over it when it may)
                        let (bl, bw, _) = live.body();
                        let top = if live.at_bottom() { floor - lift_i * m * k - live::VH * k } else { lerp(stage.y + STAGE_TOP, stage.y, m) };
                        // and a corner island stands in the corner of the stage, a centred one in the middle (shifted along the edge on the sketch)
                        let (pad_a, pad_b) = (SIDE_PAD, ALIGN_PAD * k);
                        let left = match live.aligned() {
                            Side::Center => stage.cx() + dx_i * m * k - self.pv.cx_world * k,
                            Side::Left => lerp(stage.x + pad_a, stage.x + pad_b, m) - bl * k,
                            Side::Right => lerp(stage.right() - pad_a, stage.right() - pad_b, m) - (bl + bw) * k,
                        };
                        let xf = mul(&scale_about((0.0, 0.0), k, k), &translation(left, top));
                        cv.with_transform(xf, |cv| live.draw(cv, d.th, cx));
                        // the music widget: its window stands against the edge of the monitor on the side it is docked to
                        if let Some(g) = live.player() {
                            let (ww, wh) = (g.win.0 * k, g.win.1 * k);
                            let x = match g.align {
                                Side::Left => stage.x,
                                Side::Right => stage.right() - ww,
                                Side::Center => stage.cx() - ww / 2.0 + dx_w * k,
                            };
                            let y = match g.dock {
                                Dock::Top => stage.y,
                                Dock::Bottom => stage.bottom() - lift_w * k - wh,
                                Dock::Taskbar => stage.bottom() - wh,
                            };
                            let xf = mul(&scale_about((0.0, 0.0), k, k), &translation(x, y));
                            cv.with_transform(xf, |cv| live.draw_player(cv, d.th, cx));
                        }
                    }
                });
                cv.stroke_rr(stage, 12.0, 1.0, p.card_line);
                // the caption
                let sc = self.pv.scene;
                let off = sc.switch().is_some_and(|k| !switch_on(&self.model, k)) || (sc == Scene::Widget && !switch_on(&self.model, "module.media"));
                let a = self.pv.note.at01();
                let (t_st, n_st) = (Style::new(13.0, 600), Style::new(12.0, 400));
                let mut y = stage.bottom() + 14.0;
                let (title, note) = (sc.title(lang), sc.note(lang, off));
                let th = txt::wrapped_h(cv, title, t_st, sw - 4.0, 18.0);
                cv.with_opacity(a, |cv| {
                    txt::para(cv, Rect::new(stage.x + 2.0, y, sw - 4.0, th), title, t_st, p.text);
                    y += th + 4.0;
                    let nh = txt::wrapped_h(cv, note, n_st, sw - 4.0, 16.0);
                    txt::para(cv, Rect::new(stage.x + 2.0, y, sw - 4.0, nh), note, n_st, if off { p.amber } else { p.dim });
                });
            })
        });
        // the sandbox asked for a frame while it was parked (a draw re-measured its island): wake it
        if self.pv.live.as_ref().is_some_and(|l| !l.is_awake() && l.has_mail()) {
            cx.post(crate::ui::view::Msg::Custom(Box::new(super::Local::Pv)));
        }
    }

    /// A small text button; returns its width.
    fn pv_chip(&mut self, d: &Dc, right: f32, y: f32, label: &str, act: Act) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let st = Style::new(11.0, 500);
        let w = cv.text_width(label, st) + 20.0;
        let r = Rect::new(right - w, y, w, 24.0);
        let k = key(&format!("pv:{label}"));
        self.ui.add(k, r, act, false, false, true);
        let (hv, pr) = (self.ui.hover_of(k), self.ui.press_of(k));
        cv.with_scale_about((r.cx(), r.cy()), 1.0 - 0.04 * pr, |cv| {
            cv.fill_rr(r, 8.0, mix(p.ctl, p.ctl_hover, hv));
            cv.text(r, label, Style { align: Align::Center, valign: VAlign::Middle, wrap: false, ..st }, mix(p.text_ctl, p.text, hv));
        });
        w
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn pick<'a>(lang: pill_ui_core::settings::tr::Lang, ru: &'a str, en: &'a str) -> &'a str {
    pill_ui_core::settings::tr::pick(lang, ru, en)
}
