//! The `home` tab: the media card (cover, title, seek, transport), the volume row with the output-device list, the 2x4 tile grid and the launcher
//! strip. Native port of `src/tab-home.js` over the `pill_ui_core::panel::a::home` model.
//!
//! # Wiring
//! | what happens | model call | backend |
//! |---|---|---|
//! | tab shown | `Home::on_show` (+ `init_calls` the first time) | `media_state`, `settings_get`, `media_sessions`, `media_volume_get`, `media_devices`, `launcher_list` |
//! | `media:changed`, `live:volume`, `settings:changed`, `pill:show|hide`, `launcher:changed` | `on_event` | `media_pause_updates` / `media_resume_updates` |
//! | transport buttons | `Action::Ctl` | `media_control {action}` |
//! | seek press / drag / release | `SeekDrag` / `SeekCommit` | `media_seek {ms}` |
//! | volume press / drag / wheel / keys / mute | `VolPointer*` / `VolWheel` / `VolKey` / `MuteClick` | `media_volume_set {v}`, `media_mute {muted}` |
//! | device chip, list item | `DevToggle`, `DevPick` | `media_devices`, `media_set_default_device {id}` |
//! | source label | `SourceClick` | `media_select {id}` |
//! | tiles | `Tile(id)` | none: `TabFx::StartCapture` / `OpenCommandBar` to the island |
//! | launcher icon click / x / right-click / Delete / "+" | `Launcher(Open|Remove|AddHint)` | `launcher_open`, `launcher_remove`, toast |
//! | launcher drag-reorder | `Launcher(ReorderPress|Move|Release)` | `launcher_move {id,newIndex}` |
//! | file drop on the strip (`View::on_drop`) | `Launcher::on_event("tauri://drag-*")` | `launcher_add {paths}` |
//!
//! # Time and frames
//! The model keeps its own clock; the view advances it by the real time between calls (`advance`), from frames while something animates and from a 250 ms
//! ticker otherwise, so a playing track costs four wakes a second (the seek fill moves half a pixel a second) and an idle tab costs nothing.
//! Animation cues (`title`, `artist`, `play`, `art`, `launcher`, `link:<id>`) play on the kit `Player`; hover, press, the volume glide, the device
//! list, the cover tint and the reorder shifts are glasskit tweens. Reduce motion jumps all of them to their end state.
mod css;
mod layout;
mod paint;
#[cfg(test)]
mod tests;

use super::core::{Base, Incoming};
use super::draw::{decode_data_url, Source};
use crate::ui::view::{Cx, DropEvent, DropPayload, DropPhase, Msg, View};
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::controls::Interact;
use glasskit::ease::{curves, Tween};
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::lang::Lang;
use glasskit::scroll::{ScrollState, ScrollView};
use glasskit::theme::Theme;
use pill_ui_core::panel::a::cover::{downsample, pick_colors, CoverColors};
use pill_ui_core::panel::a::home::{self as hm, Action, Ctl, Home, VolKey};
use pill_ui_core::panel::a::launcher::{self as lc, Action as LAction};
use pill_ui_core::panel::a::{BackendCall, EnTr, RuTr, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

static RU: RuTr = RuTr;
static EN: EnTr = EnTr;

fn tr(cx: &Cx) -> &'static dyn Tr {
    if cx.lang == Lang::Ru {
        &RU
    } else {
        &EN
    }
}

/// Real-time period of the model clock while nothing animates.
const TICK_MS: u64 = 250;
/// Key of the "+" tile in the position tables (launcher ids are positive).
const ADD_ID: i64 = i64::MIN;

/// Posted by the cover-colour worker.
/// True for the answer of the cover-colour worker (the island routes it to this tab).
pub(crate) fn is_cover_ready(b: &(dyn std::any::Any + Send)) -> bool {
    b.is::<CoverReady>()
}

struct CoverReady {
    key: String,
    colors: Option<CoverColors>,
}

/// Everything the pointer can be over.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Hit {
    Src,
    Prev,
    Main,
    Next,
    Seek,
    Mute,
    Vol,
    Dev,
    Tile(usize),
    Add,
    App(i64),
    AppX(i64),
    MenuItem(usize),
}

/// A launcher icon as the next frame draws it (computed in `step` / `refresh`, never in `draw`).
#[derive(Clone, Debug)]
struct AppDraw {
    id: i64,
    rect: Rect,
    alpha: f32,
    scale: f32,
    dragging: bool,
}

pub struct HomeView {
    base: Base,
    model: Home,
    v: hm::View,
    clock: Option<f64>,
    started: bool,
    hov: HashMap<Hit, Interact>,
    hover: Option<Hit>,
    down: Option<Hit>,
    seek_drag: bool,
    vol_drag: bool,
    vol_focus: bool,
    menu_open: bool,
    menu_a: Tween<f32>,
    menu_p: Tween<f32>,
    menu_scroll: ScrollView,
    art_key: Option<String>,
    art_prev: Option<String>,
    cov_last: Option<CoverColors>,
    cov_from: Option<(Color, Color)>,
    cov_to: Option<(Color, Color)>,
    cov_t: Tween<f32>,
    strip: ScrollState,
    apps: Vec<AppDraw>,
    add_x: f32,
    flip_from: HashMap<i64, f32>,
    off: HashMap<i64, Tween<f32>>,
    gone: HashMap<i64, Tween<f32>>,
    vol_prev: f64,
    vol_kick: u8,
}

impl HomeView {
    pub fn new() -> HomeView {
        let model = Home::new();
        let v = model.view(&EN);
        HomeView {
            base: Base::new(),
            model,
            v,
            clock: None,
            started: false,
            hov: HashMap::new(),
            hover: None,
            down: None,
            seek_drag: false,
            vol_drag: false,
            vol_focus: false,
            menu_open: false,
            menu_a: Tween::at_rest(0.0),
            menu_p: Tween::at_rest(0.0),
            menu_scroll: ScrollView::new(),
            art_key: None,
            art_prev: None,
            cov_last: None,
            cov_from: None,
            cov_to: None,
            cov_t: Tween::at_rest(1.0),
            strip: ScrollState::new(),
            apps: vec![],
            add_x: 0.0,
            flip_from: HashMap::new(),
            off: HashMap::new(),
            gone: HashMap::new(),
            vol_prev: 0.0,
            vol_kick: 0,
        }
    }

    // ------------------------------------------------------------------------------------------------------------------ plumbing

    /// Advance the model clock to `cx.now_ms` (fires due timers: sessions refresh, launcher removal).
    fn advance(&mut self, cx: &mut Cx) {
        let now = cx.now_ms;
        let dt = self.clock.map_or(0.0, |c| (now - c).max(0.0));
        self.clock = Some(now);
        if dt > 0.0 {
            let calls = self.model.tick(dt);
            self.run(cx, calls);
        }
    }

    fn run(&mut self, cx: &mut Cx, calls: Vec<BackendCall>) {
        self.base.calls.run(cx, calls);
    }

    /// After any model input: drain its effects, rebuild the view-model, ask for a frame.
    fn after(&mut self, cx: &mut Cx) {
        if let Some((key, url)) = self.model.take_cover_request() {
            let w = cx.waker();
            let k2 = key.clone();
            let job = move || {
                let colors = decode_data_url(&url, 64).and_then(|d| {
                    let rgba: Vec<u8> = d
                        .bgra
                        .chunks_exact(4)
                        .flat_map(|p| {
                            let a = p[3] as u32;
                            let un = |c: u8| if a == 0 || a == 255 { c } else { ((c as u32 * 255 / a).min(255)) as u8 };
                            [un(p[2]), un(p[1]), un(p[0]), p[3]]
                        })
                        .collect();
                    pick_colors(&downsample(&rgba, d.w as usize, d.h as usize))
                });
                w.post_custom(CoverReady { key, colors });
            };
            if !glasskit::jobs::pool::global().submit(Box::new(job)) {
                self.model.set_cover_colors(&k2, None);
            }
        }
        let before_flip = self.base.player.is_active("launcher");
        let fxs = self.model.take_fx();
        let _ = self.base.route_fx(cx, fxs);
        if !before_flip && self.base.player.is_active("launcher") {
            self.flip_from = self.apps.iter().map(|a| (a.id, a.rect.x)).chain(std::iter::once((ADD_ID, self.add_x))).collect();
        }
        self.refresh(cx);
        cx.request_frame();
    }

    fn act(&mut self, cx: &mut Cx, a: Action) {
        let calls = self.model.on_action(a, tr(cx));
        self.vol_kick = 4;
        self.run(cx, calls);
        self.after(cx);
    }

    /// Rebuild the view-model and everything derived from it (images, tweens, positions).
    fn refresh(&mut self, cx: &mut Cx) {
        self.model.set_os_reduce_motion(cx.reduced_motion);
        self.base.reduced = cx.reduced_motion;
        self.v = self.model.view(tr(cx));
        self.sync_layout();
        self.sync_images(cx);
        self.sync_cover_tint();
        self.sync_menu();
        self.compute_apps();
    }

    fn sync_layout(&mut self) {
        let s = layout::STRIP;
        self.model.launcher_mut().set_layout(lc::Rect { left: s.x, top: s.y, right: s.right(), bottom: s.bottom() }, layout::APP_PITCH, 1.0, 1.0);
        let n = self.v.launcher.tiles.len();
        let content = layout::APPS_X * 2.0 + n as f32 * layout::APP_PITCH + layout::ADD;
        self.strip.set_metrics(content, s.w);
    }

    fn cover_key(&self) -> Option<String> {
        let m = self.model.media()?;
        let url = self.v.player.cover_url.as_ref()?;
        Some(format!("cover:{}:{}", m.thumb_id, url.len()))
    }

    fn sync_images(&mut self, cx: &mut Cx) {
        let w = cx.waker();
        let key = self.cover_key().filter(|_| !self.v.player.none);
        if key != self.art_key {
            self.art_prev = self.art_key.take().filter(|k| self.base.images.contains(k));
            self.art_key = key.clone();
            if let (Some(k), Some(u)) = (&key, &self.v.player.cover_url) {
                self.base.images.request(&w, k, Source::DataUrl(u.clone()));
            }
            // the cue fires with the model's render; the fade must start when the new picture is there
            if key.as_ref().map_or(true, |k| !self.base.images.contains(k)) {
                self.base.player.release("art");
            } else if self.art_prev.is_some() {
                self.base.player.play("art", pill_ui_core::panel::a::Anim::cover_fade(), cx.reduced_motion);
            }
        }
        for t in &self.v.launcher.tiles {
            if let Some(u) = &t.icon {
                self.base.images.request(&w, &icon_key(t.id, u), Source::DataUrl(u.clone()));
            }
        }
    }

    fn sync_cover_tint(&mut self) {
        let now = self.v.player.cover_gradient.clone();
        if now == self.cov_last {
            return;
        }
        let cur = self.cov_tint();
        self.cov_last = now.clone();
        let to = now.and_then(|c| Some((css::parse(&c.a)?, css::parse(&c.b)?)));
        match (cur, to) {
            (Some(from), Some(_)) => {
                self.cov_from = Some(from);
                self.cov_t = Tween::new(0.0, 1.0, 0.6, curves::EASE_OUT);
            }
            _ => {
                self.cov_from = None;
                self.cov_t = Tween::at_rest(1.0);
            }
        }
        self.cov_to = to;
    }

    /// The cover tint on screen right now (mid-fade between two covers).
    fn cov_tint(&self) -> Option<(Color, Color)> {
        let to = self.cov_to?;
        match self.cov_from {
            Some(f) if !self.cov_t.finished() => {
                let t = self.cov_t.value();
                Some((f.0.mix(to.0, t), f.1.mix(to.1, t)))
            }
            _ => Some(to),
        }
    }

    fn sync_menu(&mut self) {
        let open = self.v.devices.open;
        if open != self.menu_open {
            self.menu_open = open;
            if open {
                self.menu_a.retarget(1.0, 0.22, curves::EASE_OUT);
                self.menu_p.retarget(1.0, 0.38, curves::SPRING_SETTLE);
                self.menu_scroll.state.scroll_to(0.0, false);
            } else {
                self.menu_a.retarget(0.0, 0.14, curves::EASE_IN);
                self.menu_p.retarget(0.0, 0.14, curves::EASE_IN);
            }
        }
        let n = self.v.devices.items.len();
        let r = layout::menu(n);
        self.menu_scroll.set_content_height(10.0 + n as f32 * layout::MENU_ROW, r.h);
    }

    /// Positions and looks of the launcher icons for the next frame (drag shifts, the FLIP glide, the pop-in, the removal fade).
    fn compute_apps(&mut self) {
        let flip = self.base.player.sample("launcher");
        let flip_on = self.base.player.is_active("launcher");
        if !flip_on {
            self.flip_from.clear();
        }
        let scroll = self.strip.offset();
        let mut out = Vec::with_capacity(self.v.launcher.tiles.len());
        for (i, t) in self.v.launcher.tiles.iter().enumerate() {
            let off = self.off.entry(t.id).or_insert_with(|| Tween::at_rest(t.offset_x));
            if t.dragging {
                off.jump(t.offset_x);
            } else {
                off.retarget(t.offset_x, 0.2, curves::EASE_OUT_EXPO);
            }
            let mut rect = layout::app(i).offset(off.value() - scroll, 0.0);
            if flip_on {
                if let Some(ox) = self.flip_from.get(&t.id) {
                    rect.x = ox + (rect.x - ox) * flip.p;
                }
            }
            let pop = self.base.player.sample(&format!("link:{}", t.id));
            let g = if t.gone { self.gone.entry(t.id).or_insert_with(|| Tween::new(0.0, 1.0, 0.16, curves::EASE_OUT)).value() } else { 0.0 };
            let missing = if t.missing { 0.4 } else { 1.0 };
            out.push(AppDraw {
                id: t.id,
                rect,
                alpha: pop.alpha * (1.0 - g) * missing * if t.dragging { 0.85 } else { 1.0 },
                scale: pop.scale * (1.0 - 0.3 * g),
                dragging: t.dragging,
            });
        }
        self.gone.retain(|id, _| self.v.launcher.tiles.iter().any(|t| t.id == *id && t.gone));
        self.off.retain(|id, _| self.v.launcher.tiles.iter().any(|t| t.id == *id));
        let mut add = layout::add(self.v.launcher.tiles.len()).x - scroll;
        if flip_on {
            if let Some(ox) = self.flip_from.get(&ADD_ID) {
                add = ox + (add - ox) * flip.p;
            }
        }
        self.add_x = add;
        self.apps = out;
    }

    // ------------------------------------------------------------------------------------------------------------------ hit testing

    fn local(&self, p: Pt) -> Pt {
        (p.0 - self.base.rect.x, p.1 - self.base.rect.y)
    }

    fn menu_visible(&self) -> bool {
        self.menu_open || self.menu_a.value() > 0.01
    }

    fn hit(&self, p: Pt) -> Option<Hit> {
        let (x, y) = p;
        if self.menu_open {
            let m = layout::menu(self.v.devices.items.len());
            if m.contains(x, y) {
                let i = ((y - m.y - 5.0 + self.menu_scroll.offset()) / layout::MENU_ROW).floor();
                return (i >= 0.0 && (i as usize) < self.v.devices.items.len() && y >= m.y + 5.0 && y < m.bottom() - 5.0).then_some(Hit::MenuItem(i as usize));
            }
        }
        if layout::STRIP.contains(x, y) || self.apps.iter().any(|a| a.rect.contains(x, y)) {
            for a in self.apps.iter().rev() {
                let badge = layout::app_x(a.rect);
                if self.hover_amount(Hit::App(a.id)) > 0.3 && !self.v.launcher.reordering && badge.contains(x, y) {
                    return Some(Hit::AppX(a.id));
                }
            }
            for a in self.apps.iter().rev() {
                if a.rect.contains(x, y) && layout::STRIP.contains(x, y) {
                    return Some(Hit::App(a.id));
                }
            }
            let add = Rect::new(self.add_x, layout::add(0).y, layout::ADD, layout::ADD);
            if add.contains(x, y) && layout::STRIP.contains(x, y) {
                return Some(Hit::Add);
            }
        }
        if self.v.player.none && layout::PLAYER.contains(x, y) {
            return None;
        }
        if !self.v.player.none {
            if hit_rounded(layout::MAIN, 18.0, p) {
                return Some(Hit::Main);
            }
            if hit_rounded(layout::PREV, 14.0, p) {
                return Some(Hit::Prev);
            }
            if hit_rounded(layout::NEXT, 14.0, p) {
                return Some(Hit::Next);
            }
            if self.v.player.source_multi && layout::SRC.contains(x, y) {
                return Some(Hit::Src);
            }
            if self.v.player.seek.enabled && layout::SEEK.contains(x, y) {
                return Some(Hit::Seek);
            }
        }
        if layout::MUTE.contains(x, y) {
            return Some(Hit::Mute);
        }
        if Rect::new(layout::VOL.x, layout::VOL.y - 4.0, layout::VOL.w, layout::VOL.h + 8.0).contains(x, y) {
            return Some(Hit::Vol);
        }
        if layout::DEV.contains(x, y) {
            return Some(Hit::Dev);
        }
        (0..self.v.tiles.len()).find(|&i| layout::tile(i).contains(x, y)).map(Hit::Tile)
    }

    fn hover_amount(&self, h: Hit) -> f32 {
        self.hov.get(&h).map_or(0.0, |i| i.hover())
    }

    fn set_hover(&mut self, h: Option<Hit>) -> bool {
        if h == self.hover {
            return false;
        }
        if let Some(o) = self.hover {
            if let Some(i) = self.hov.get_mut(&o) {
                i.set_hover(false);
            }
            // leaving an icon's badge keeps the icon lifted
            if let Hit::AppX(id) = o {
                if h != Some(Hit::App(id)) {
                    if let Some(i) = self.hov.get_mut(&Hit::App(id)) {
                        i.set_hover(false);
                    }
                }
            }
        }
        if let Some(n) = h {
            self.hov.entry(n).or_insert_with(Interact::new).set_hover(true);
            if let Hit::AppX(id) = n {
                self.hov.entry(Hit::App(id)).or_insert_with(Interact::new).set_hover(true);
            }
        }
        self.hover = h;
        true
    }

    fn set_press(&mut self, h: Option<Hit>, on: bool) {
        if let Some(h) = h {
            self.hov.entry(h).or_insert_with(Interact::new).set_press(on);
        }
    }

    fn cursor_of(&self, h: Option<Hit>) -> Option<Cursor> {
        match h? {
            Hit::Tile(i) if self.v.tiles.get(i).map_or(true, |t| t.soon) => None,
            _ => Some(Cursor::Hand),
        }
    }

    fn seek_value(&self, x: f32) -> f64 {
        let r = layout::SEEK;
        hm::vol_at(x, r.x, r.w, r.w) as f64 / 100.0 * self.v.player.seek.max_ms as f64
    }

    fn vol_value(&self, x: f32) -> f32 {
        hm::vol_at(x, layout::VOL.x, layout::VOL.w, layout::VOL.w)
    }

    fn activate(&mut self, h: Hit, cx: &mut Cx) {
        match h {
            Hit::Prev => self.act(cx, Action::Ctl(Ctl::Prev)),
            Hit::Main => self.act(cx, Action::Ctl(Ctl::PlayPause)),
            Hit::Next => self.act(cx, Action::Ctl(Ctl::Next)),
            Hit::Src => self.act(cx, Action::SourceClick),
            Hit::Mute => self.act(cx, Action::MuteClick),
            Hit::Dev => self.act(cx, Action::DevToggle),
            Hit::Tile(i) => {
                if let Some(t) = self.v.tiles.get(i).filter(|t| !t.soon) {
                    let id = t.id.to_string();
                    self.act(cx, Action::Tile(id));
                }
            }
            Hit::Add => self.act(cx, Action::Launcher(LAction::AddHint)),
            Hit::App(id) => self.act(cx, Action::Launcher(LAction::Open(id))),
            Hit::AppX(id) => self.act(cx, Action::Launcher(LAction::Remove(id))),
            Hit::MenuItem(i) => {
                if let Some(d) = self.v.devices.items.get(i) {
                    let id = d.id.clone();
                    self.act(cx, Action::DevPick(id));
                }
            }
            Hit::Seek | Hit::Vol => {}
        }
    }

    fn on_move(&mut self, p: Pt, cx: &mut Cx) -> Response {
        let mut r = Response::redraw();
        if self.seek_drag {
            let v = self.seek_value(p.0);
            self.act(cx, Action::SeekDrag(v));
            r.capture = true;
            return r.with_cursor(Cursor::Hand);
        }
        if self.vol_drag {
            let v = self.vol_value(p.0);
            self.act(cx, Action::VolPointerMove(v));
            r.capture = true;
            return r.with_cursor(Cursor::Hand);
        }
        if matches!(self.down, Some(Hit::App(_))) {
            self.act(cx, Action::Launcher(LAction::ReorderMove { x: p.0 }));
            r.capture = true;
            return r.with_cursor(Cursor::Hand);
        }
        let h = self.hit(p);
        let changed = self.set_hover(h);
        r.redraw = changed;
        r.cursor = self.cursor_of(h);
        r
    }

    fn on_down(&mut self, p: Pt, button: MouseButton, cx: &mut Cx) -> Response {
        let h = self.hit(p);
        if self.menu_open && !matches!(h, Some(Hit::MenuItem(_)) | Some(Hit::Dev)) && !layout::menu(self.v.devices.items.len()).contains(p.0, p.1) {
            self.act(cx, Action::DevClose);
        }
        self.vol_focus = h == Some(Hit::Vol);
        if button == MouseButton::Right {
            if let Some(Hit::App(id)) | Some(Hit::AppX(id)) = h {
                self.act(cx, Action::Launcher(LAction::Remove(id)));
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
            return Response::NONE;
        }
        if button != MouseButton::Left {
            return Response::NONE;
        }
        let mut r = Response { redraw: true, consumed: h.is_some(), ..Response::NONE };
        match h {
            Some(Hit::Seek) => {
                self.seek_drag = true;
                let v = self.seek_value(p.0);
                self.act(cx, Action::SeekDrag(v));
                r.capture = true;
            }
            Some(Hit::Vol) => {
                self.vol_drag = true;
                let v = self.vol_value(p.0);
                self.act(cx, Action::VolPointerDown(v));
                r.capture = true;
            }
            Some(Hit::App(id)) => {
                self.down = h;
                self.set_press(h, true);
                self.act(cx, Action::Launcher(LAction::ReorderPress { id, x: p.0 }));
                r.capture = true;
            }
            Some(other) => {
                self.down = Some(other);
                self.set_press(h, true);
            }
            None => {}
        }
        r
    }

    fn on_up(&mut self, p: Pt, button: MouseButton, cx: &mut Cx) -> Response {
        if button != MouseButton::Left {
            return Response::NONE;
        }
        let mut r = Response::redraw();
        if self.seek_drag {
            self.seek_drag = false;
            let v = self.seek_value(p.0);
            self.act(cx, Action::SeekCommit(v));
        }
        if self.vol_drag {
            self.vol_drag = false;
            self.act(cx, Action::VolPointerUp);
        }
        let down = self.down.take();
        self.set_press(down, false);
        let was_reorder = self.v.launcher.reordering;
        if matches!(down, Some(Hit::App(_))) {
            self.act(cx, Action::Launcher(LAction::ReorderRelease));
            // the JS clears its click guard on the next macrotask: here, right after this release
            let calls = self.model.launcher_mut().tick(0.0);
            self.run(cx, calls);
        }
        let h = self.hit(p);
        if let (Some(d), true) = (down, !was_reorder) {
            if Some(d) == h {
                self.activate(d, cx);
            }
        }
        self.set_hover(h);
        r.cursor = self.cursor_of(h);
        r
    }

    fn on_wheel(&mut self, p: Pt, delta: Pt, mods: glasskit::input::Modifiers, precise: bool, cx: &mut Cx) -> Response {
        if self.menu_open && layout::menu(self.v.devices.items.len()).contains(p.0, p.1) {
            let ev = InputEvent::Wheel { pos: p, delta, mods, precise };
            let r = self.menu_scroll.event(&ev, layout::menu(self.v.devices.items.len()).inset_xy(0.0, 5.0));
            return Response { consumed: true, redraw: true, ..r };
        }
        if layout::VOL_ROW.contains(p.0, p.1) && delta.1 != 0.0 {
            self.act(cx, Action::VolWheel { delta_y: delta.1, lines: false });
            return Response { consumed: true, redraw: true, ..Response::NONE };
        }
        if layout::STRIP.contains(p.0, p.1) && self.strip.scrollable() {
            let d = if delta.0 != 0.0 { delta.0 } else if mods.shift { delta.1 } else { 0.0 };
            if d != 0.0 && self.strip.wheel(d, precise) {
                self.compute_apps();
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
        }
        Response::NONE
    }

    fn on_key(&mut self, code: KeyCode, cx: &mut Cx) -> Response {
        if code == KeyCode::Escape && self.model.wants_escape() {
            self.act(cx, Action::DevClose);
            return Response { consumed: true, redraw: true, ..Response::NONE };
        }
        if code == KeyCode::Delete {
            if let Some(Hit::App(id)) | Some(Hit::AppX(id)) = self.hover {
                self.act(cx, Action::Launcher(LAction::Remove(id)));
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
        }
        if self.vol_focus {
            let k = match code {
                KeyCode::Left => Some(VolKey::Left),
                KeyCode::Down => Some(VolKey::Down),
                KeyCode::Right => Some(VolKey::Right),
                KeyCode::Up => Some(VolKey::Up),
                KeyCode::PageDown => Some(VolKey::PageDown),
                KeyCode::PageUp => Some(VolKey::PageUp),
                KeyCode::Home => Some(VolKey::Home),
                KeyCode::End => Some(VolKey::End),
                _ => None,
            };
            if let Some(k) = k {
                self.act(cx, Action::VolKey(k));
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
        }
        Response::NONE
    }
}

fn icon_key(id: i64, url: &str) -> String {
    format!("link:{id}:{}", url.len())
}

fn hit_rounded(r: Rect, rad: f32, p: Pt) -> bool {
    glasskit::input::hit_rounded(r, rad, p)
}

impl View for HomeView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((super::W, super::tab_h("home")? - super::CHROME_H))
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.base.show(cx, &["media:changed", "live:volume", "settings:changed", "pill:show", "pill:hide", "launcher:changed"], TICK_MS);
        self.clock = Some(cx.now_ms);
        let mut calls = vec![];
        if !self.started {
            self.started = true;
            calls.extend(self.model.init_calls());
        }
        calls.extend(self.model.on_show());
        self.run(cx, calls);
        self.after(cx);
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.base.hide();
        self.model.on_hide();
        self.seek_drag = false;
        self.vol_drag = false;
        self.down = None;
        self.set_hover(None);
        self.after(cx);
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.advance(cx);
        match self.base.classify(msg, cx) {
            Incoming::Event { name, payload } => {
                let calls = self.model.on_event(&name, &payload);
                self.run(cx, calls);
            }
            Incoming::Result { cmd, args, result } => {
                let value = match result {
                    Ok(v) => v,
                    // a rejected volume write must not wedge the one-in-flight queue
                    Err(_) if cmd == "media_volume_set" => Value::Null,
                    Err(_) => return Response::NONE,
                };
                let calls = self.model.on_result(cmd, &args, &value, tr(cx));
                self.run(cx, calls);
            }
            Incoming::Tick(_) => {}
            Incoming::Image(_) => {
                if let Some(k) = self.art_key.clone() {
                    if self.art_prev.is_some() && self.base.images.contains(&k) && !self.base.player.is_active("art") {
                        self.base.player.play("art", pill_ui_core::panel::a::Anim::cover_fade(), cx.reduced_motion);
                    }
                }
            }
            Incoming::Other(Msg::Custom(b)) => match b.downcast::<CoverReady>() {
                Ok(c) => self.model.set_cover_colors(&c.key, c.colors),
                Err(_) => return Response::NONE,
            },
            Incoming::Other(_) => return Response::NONE,
        }
        self.after(cx);
        Response::redraw()
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.advance(cx);
        let r = match ev {
            InputEvent::MouseMove { pos, .. } => {
                let p = self.local(*pos);
                self.on_move(p, cx)
            }
            InputEvent::MouseDown { pos, button, .. } => {
                let p = self.local(*pos);
                self.on_down(p, *button, cx)
            }
            InputEvent::MouseUp { pos, button, .. } => {
                let p = self.local(*pos);
                self.on_up(p, *button, cx)
            }
            InputEvent::MouseLeave => {
                let c = self.set_hover(None);
                Response { redraw: c, ..Response::NONE }
            }
            InputEvent::Wheel { pos, delta, mods, precise } => {
                let p = self.local(*pos);
                self.on_wheel(p, *delta, *mods, *precise, cx)
            }
            InputEvent::Key { code, down: true, .. } => self.on_key(*code, cx),
            _ => Response::NONE,
        };
        if r.redraw {
            cx.request_frame();
        }
        r
    }

    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        self.advance(cx);
        let p = self.local(ev.pos);
        let pos = json!({ "x": p.0, "y": p.1 });
        let calls = match (&ev.phase, &ev.payload) {
            (DropPhase::Enter, _) => self.model.on_event("tauri://drag-enter", &json!({ "position": pos })),
            (DropPhase::Over, _) => self.model.on_event("tauri://drag-over", &json!({ "position": pos })),
            (DropPhase::Leave, _) => self.model.on_event("tauri://drag-leave", &Value::Null),
            (DropPhase::Drop, Some(DropPayload::Files(f))) => {
                let paths: Vec<String> = f.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                self.model.on_event("tauri://drag-drop", &json!({ "paths": paths, "position": pos }))
            }
            (DropPhase::Drop, _) => self.model.on_event("tauri://drag-leave", &Value::Null),
        };
        let took = !matches!(ev.phase, DropPhase::Leave) && (self.model.view(tr(cx)).launcher.over || calls.iter().any(|c| c.cmd == "launcher_add"));
        self.run(cx, calls);
        self.after(cx);
        if took {
            Response { consumed: true, redraw: true, ..Response::NONE }
        } else {
            Response::redraw()
        }
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.advance(cx);
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.base.player.step(dt_ms);
        for i in self.hov.values_mut() {
            busy |= i.step(dt);
        }
        busy |= self.menu_a.step(dt) | self.menu_p.step(dt) | self.cov_t.step(dt);
        busy |= self.menu_scroll.step(dt) | self.strip.step(dt);
        for t in self.off.values_mut().chain(self.gone.values_mut()) {
            busy |= t.step(dt);
        }
        // the model eases the volume itself: keep frames coming while the shown value still moves
        self.v = self.model.view(tr(cx));
        let shown = self.v.volume.shown;
        busy |= shown != self.vol_prev || self.vol_kick > 0;
        self.vol_prev = shown;
        self.vol_kick = self.vol_kick.saturating_sub(1);
        busy |= self.v.launcher.tiles.iter().any(|t| t.gone);
        self.sync_menu();
        self.compute_apps();
        busy
    }

    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, _cx: &Cx) {
        self.base.rect = Rect::new(rect.x, rect.y, super::W, rect.h);
        self.paint(cv, theme, rect.x, rect.y);
    }
}

