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
use crate::ui::liquid::{Feel, Flow};
use crate::ui::view::Cx;
use glasskit::canvas::{mul, scale_about, translation};
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::text::{Align, Style, VAlign};
use live::Live;
use pill_ui_core::settings::schema::schema;
use scene::{of_focus, of_key, of_page, shareable, Scene};
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

    /// Hands the settings that changed since the last look to the sandbox; the scene of the first one that shows something.
    fn sync(&mut self, values: &Map<String, Value>) -> (Map<String, Value>, Option<Scene>) {
        let mut patch = Map::new();
        let mut scene = None;
        for (k, v) in values {
            if !shareable(k) {
                continue;
            }
            let old = self.snap.get(k).cloned().or_else(|| schema().default_of(k).cloned());
            if old.as_ref() != Some(v) {
                patch.insert(k.clone(), v.clone());
                scene = scene.or_else(|| of_key(k));
            }
            self.snap.insert(k.clone(), v.clone());
        }
        (patch, scene)
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
            let values = shared_values(model.values());
            pv.snap = values.clone();
            let live = Live::new(cx, values, &open::system_lang());
            live.emit("weather:current", sample::weather_now());
            pv.live = Some(live);
            pv.open = false;
            pv.page = model.current;
            let sc = pv.hint.take().unwrap_or_else(|| of_page(model.current));
            pv.scene = sc;
            pv.go_scene(sc, reduce);
            pv.touch(now);
        }
        // the settings that changed, at once (the window's own copy, not what the backend has saved yet)
        let (patch, sc) = pv.sync(model.values());
        if !patch.is_empty() {
            pv.touch(now);
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
            let sc = pv.hint.take().unwrap_or_else(|| of_page(model.current));
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
            let (fw, fh) = (bw + 2.0 * FOCUS_PAD_X, bh + 6.0);
            let k = ((PV_W - 2.0 * SIDE_PAD) / fw.max(1.0)).min((sh_max - STAGE_TOP - 12.0) / fh.max(1.0)).clamp(0.1, 2.0);
            let h = (fh * k + STAGE_TOP + 12.0).clamp(84.0, sh_max);
            pv.cx_world = bl + bw / 2.0;
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
        busy |= tick(&mut pv.zoom, dt);
        busy |= tick(&mut pv.stage_h, dt);
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
                // the stage: a desktop behind the island, which hangs from its top edge
                let stage = Rect::new(area.x + SIDE_PAD, area.y + HEAD_H, sw, self.pv.stage_h.at().max(60.0));
                let (c0, c1) = if p.dark { (Color::rgb8(0x1b, 0x22, 0x33), Color::rgb8(0x2c, 0x3a, 0x57)) } else { (Color::rgb8(0xa9, 0xb8, 0xd2), Color::rgb8(0xcb, 0xd6, 0xe8)) };
                cv.with_clip_rr(stage, 12.0, |cv| {
                    cv.fill_rect(stage, c0);
                    cv.fill_rect(Rect::new(stage.x, stage.y, stage.w, stage.h * 0.55), mix(c0, c1, 0.7));
                    if let Some(live) = &mut self.pv.live {
                        let k = self.pv.zoom.at().clamp(0.1, 2.0);
                        let m = mul(&scale_about((0.0, 0.0), k, k), &translation(stage.cx() - self.pv.cx_world * k, stage.y + STAGE_TOP));
                        cv.with_transform(m, |cv| live.draw(cv, d.th, cx));
                    }
                });
                cv.stroke_rr(stage, 12.0, 1.0, p.card_line);
                // the caption
                let sc = self.pv.scene;
                let off = matches!(sc, Scene::Flash(f) if f.switch().is_some_and(|k| !switch_on(&self.model, k)));
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

fn pick<'a>(lang: pill_ui_core::settings::tr::Lang, ru: &'a str, en: &'a str) -> &'a str {
    pill_ui_core::settings::tr::pick(lang, ru, en)
}
