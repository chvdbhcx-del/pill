//! The Shelf tab: file chips with thumbnails, selection, context menu, remove / clear, native drag-out and drop-in, drawn natively.
//! Logic is `pill_ui_core::panel::a::shelf::Shelf`; this file feeds it input, backend events and results, plays its animation cues and paints
//! its view with glasskit. Spec: `src/tab-shelf.js` + `tab-shelf.css` (648 x 260 tab body).
//!
//! * Events: `shelf:changed`, `settings:changed`, `pill:hide`. Commands: `shelf_list`, `shelf_add`, `shelf_remove`, `shelf_clear`, `shelf_open`,
//!   `shelf_reveal`, `shelf_copy_path`, `shelf_drag_out` (blocks its worker until the OLE drop is over: the reply resets the ghost state) and
//!   `settings_get` (the model reads `module.shelf`, `live.shelf`, `lastTab`, `pinned`).
//! * Drops arrive through `View::on_drop` (the island forwards them while this tab is open); the model's `tauri://drag-*` handlers are driven by
//!   the phases, the drop position is passed in tab-local logical px (the fly-in origin).
//! * The folded-island drop card (`Shelf::peek`) and the count badge belong to the island: they need a model that outlives this tab.
mod layout;
mod paint;
#[cfg(test)]
mod tests;

use super::core::{Base, Incoming};
use super::draw::{ImageCache, Source};
use super::icons;
use super::play::Sample;
use crate::ui::view::{Cx, DropEvent, DropPayload, DropPhase, Msg, View};
use glasskit::canvas::{self, Canvas};
use glasskit::color::Color;
use glasskit::controls::Interact;
use glasskit::ease::{curves, dur, Tween};
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, MouseButton, Response};
use glasskit::lang::Lang;
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use layout::*;
use pill_ui_core::panel::a::anim::AnimKind;
use pill_ui_core::panel::a::launcher::Pt as MPt;
use pill_ui_core::panel::a::shelf::{Action, Chip, ChipTile, MenuView, Shelf, View as SV};
use pill_ui_core::panel::a::util::Wall;
use pill_ui_core::panel::a::{BackendCall, EnTr, Fx, RuTr, Tr};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use windows::Foundation::Numerics::Matrix3x2;

const MENU_W: f32 = 140.0;
const MENU_ITEM_H: f32 = 34.0;
const MENU_PAD: f32 = 4.0;
/// The ticker only keeps the model clock honest while no frame runs (the highlight / restore timers are coarse).
const TICK_MS: u64 = 250;
const EVENTS: [&str; 3] = ["shelf:changed", "settings:changed", "pill:hide"];
/// Cue targets of the folded-island drop card and badge: not ours, never played here.
const OURS: [&str; 5] = ["chip:", "list", "zone", "dragall", "menu"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Press {
    None,
    Chip,
    DragAll,
    Clear,
}

pub struct ShelfView {
    model: Shelf,
    base: Base,
    v: SV,
    ru: bool,
    sv: ScrollView,
    chips: HashMap<i64, Interact>,
    drag_btn: Interact,
    clear_btn: Interact,
    drag_rect: Rect,
    clear_rect: Rect,
    hover: Option<i64>,
    hover_x: bool,
    menu_hover: Option<usize>,
    last_menu: Option<MenuView>,
    press: Press,
    over_t: Tween<f32>,
    last_ms: f64,
    now_ms: f64,
    fresh_t0: HashMap<i64, f64>,
    flip_from: HashMap<i64, (f32, f32)>,
    last_pos: HashMap<i64, (f32, f32)>,
    flying: HashSet<i64>,
    keys: HashMap<i64, String>,
}

fn tr(ru: bool) -> &'static dyn Tr {
    if ru {
        &RuTr
    } else {
        &EnTr
    }
}

fn data_url_bytes(url: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let (head, body) = url.split_once(',')?;
    head.starts_with("data:").then_some(())?;
    base64::engine::general_purpose::STANDARD.decode(body.trim()).ok()
}

fn key_of(id: i64, url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    format!("{id}#{:x}", h.finish())
}

/// Scale about `c` by `s`, rotate `deg` about `c`, then move by `(dx, dy)`.
fn xf(c: Pt, s: f32, deg: f32, dx: f32, dy: f32) -> Matrix3x2 {
    let (sn, cs) = deg.to_radians().sin_cos();
    let rot = Matrix3x2 { M11: cs, M12: sn, M21: -sn, M22: cs, M31: c.0 - c.0 * cs + c.1 * sn, M32: c.1 - c.0 * sn - c.1 * cs };
    canvas::mul(&canvas::mul(&canvas::scale_about(c, s, s), &rot), &canvas::translation(dx, dy))
}

impl ShelfView {
    pub fn new() -> ShelfView {
        let mut model = Shelf::new();
        model.set_expanded(true);
        model.set_metrics(1.0, (super::W, super::tab_h("shelf").unwrap_or(331.0) - super::CHROME_H));
        let mut base = Base::new();
        base.images = ImageCache::new(6 * 1024 * 1024, 96);
        base.rect = Rect::new(0.0, 0.0, super::W, super::tab_h("shelf").unwrap_or(331.0) - super::CHROME_H);
        let v = model.view(&EnTr, Wall::new(0, 0));
        ShelfView {
            model,
            base,
            v,
            ru: false,
            sv: ScrollView::new(),
            chips: HashMap::new(),
            drag_btn: Interact::new(),
            clear_btn: Interact::new(),
            drag_rect: Rect::default(),
            clear_rect: Rect::default(),
            hover: None,
            hover_x: false,
            menu_hover: None,
            last_menu: None,
            press: Press::None,
            over_t: Tween::at_rest(0.0),
            last_ms: 0.0,
            now_ms: 0.0,
            fresh_t0: HashMap::new(),
            flip_from: HashMap::new(),
            last_pos: HashMap::new(),
            flying: HashSet::new(),
            keys: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------------------------------------------------ model plumbing

    fn frame(&self) -> Frame {
        // tab-local: events are translated by the drawn origin, `draw` shifts the canvas by it
        frame(Rect::new(0.0, 0.0, self.base.rect.w, self.base.rect.h), !self.v.empty)
    }

    fn cols(&self) -> usize {
        cols(self.frame().list.w)
    }

    fn sync_scroll(&mut self) {
        let f = self.frame();
        let c = self.cols();
        self.sv.set_content_height(content_h(self.v.chips.len(), c), f.list.h);
    }

    /// The model asked for `fx`: keep our cues, point fly-ins at the chip's real slot, drop the folded-island ones.
    fn filter_fx(&mut self, fxs: Vec<Fx>) -> Vec<Fx> {
        let f = self.frame();
        let c = self.cols();
        let mut out = Vec::new();
        for fx in fxs {
            match fx {
                Fx::Anim { target, mut anim } => {
                    if !OURS.iter().any(|p| target.starts_with(p)) {
                        continue;
                    }
                    if target == "list" {
                        self.flip_from = self.last_pos.clone();
                    }
                    if let (AnimKind::FlyIn { from_dx, from_dy, .. }, Some(id)) = (&mut anim.kind, target.strip_prefix("chip:").and_then(|s| s.parse::<i64>().ok())) {
                        if let Some(i) = self.model.items().iter().position(|it| it.id == id) {
                            let r = chip_rect(i, c);
                            let centre = MPt { x: f.list.x + r.cx(), y: f.list.y + r.cy() - self.sv.offset() };
                            let (dx, dy) = self.model.fly_vector(centre, 1.0);
                            *from_dx = dx;
                            *from_dy = dy;
                            self.flying.insert(id);
                        }
                    }
                    out.push(Fx::Anim { target, anim });
                }
                other => out.push(other),
            }
        }
        out
    }

    /// Run the model's calls, route its effects, rebuild the cached view.
    fn sync(&mut self, cx: &mut Cx, calls: Vec<BackendCall>) {
        let fxs = self.model.take_fx();
        self.sync_with(cx, calls, fxs);
    }

    fn sync_with(&mut self, cx: &mut Cx, calls: Vec<BackendCall>, fxs: Vec<Fx>) {
        self.base.calls.run(cx, calls);
        let fxs = self.filter_fx(fxs);
        self.base.route_fx(cx, fxs);
        self.refresh(cx);
        cx.request_frame();
    }

    fn refresh(&mut self, cx: &mut Cx) {
        self.v = self.model.view(tr(self.ru), Wall::new(0, 0));
        self.sync_scroll();
        let ids: HashSet<i64> = self.v.chips.iter().map(|c| c.id).collect();
        self.chips.retain(|id, _| ids.contains(id));
        self.fresh_t0.retain(|id, _| ids.contains(id));
        self.flying.retain(|id| ids.contains(id));
        self.keys.retain(|id, _| ids.contains(id));
        let waker = cx.waker();
        for ch in &self.v.chips {
            self.chips.entry(ch.id).or_insert_with(Interact::new);
            if ch.fresh {
                self.fresh_t0.entry(ch.id).or_insert(self.now_ms);
            }
            if let ChipTile::Image(url) = &ch.tile {
                let key = key_of(ch.id, url);
                if self.keys.get(&ch.id) != Some(&key) {
                    if !self.base.images.contains(&key) {
                        if let Some(b) = data_url_bytes(url) {
                            self.base.images.request(&waker, &key, Source::Bytes(b));
                        }
                    }
                    self.keys.insert(ch.id, key);
                }
            }
        }
        if let Some(m) = &self.v.menu {
            self.last_menu = Some(m.clone());
        }
        // an exit that finished and whose chip is back (a failed call) or gone: let go of its held end state
        for t in self.base.player.finished() {
            let keep = t.strip_prefix("chip:").and_then(|s| s.parse::<i64>().ok()).map_or(false, |id| self.v.chips.iter().any(|c| c.id == id && c.hidden));
            if !keep && !(t == "menu" && self.v.menu.is_none()) {
                self.base.player.release(&t);
            }
        }
        if self.v.menu.is_none() && !self.base.player.is_active("menu") && self.base.player.sample("menu") == Sample::REST {
            self.last_menu = None;
        }
        if (self.v.over) != (self.over_t.target() > 0.5) {
            self.over_t.retarget(self.v.over as u8 as f32, dur::MED, curves::EASE_OUT);
        }
    }

    fn advance(&mut self, cx: &mut Cx) -> Vec<BackendCall> {
        let dt = (cx.now_ms - self.last_ms).clamp(0.0, 1000.0);
        self.last_ms = cx.now_ms;
        self.now_ms = cx.now_ms;
        if dt > 0.0 {
            self.model.tick(dt)
        } else {
            vec![]
        }
    }

    fn enter_ctx(&mut self, cx: &Cx) {
        self.ru = cx.lang == Lang::Ru;
        self.model.set_os_reduce_motion(cx.reduced_motion);
        self.base.reduced = cx.reduced_motion;
        self.now_ms = cx.now_ms;
    }

    fn feed_result(&mut self, cmd: &str, args: &Value, result: Result<Value, String>) -> Vec<BackendCall> {
        match (cmd, result) {
            ("settings_get", Ok(v)) => {
                self.model.set_settings(&v);
                vec![]
            }
            ("settings_get", Err(_)) | ("shelf_list", Err(_)) => vec![],
            // a rejected command still has to reset the ghosts / restore a hidden chip: the model reads Null as "nothing happened"
            (_, Err(_)) => self.model.on_result(cmd, args, &Value::Null, tr(self.ru)),
            (_, Ok(v)) => self.model.on_result(cmd, args, &v, tr(self.ru)),
        }
    }

    fn act(&mut self, cx: &mut Cx, a: Action) {
        let calls = self.model.on_action(a);
        self.sync(cx, calls);
    }

    // ------------------------------------------------------------------------------------------------------------ hit testing

    fn chip_at(&self, p: Pt) -> Option<usize> {
        let f = self.frame();
        if !f.list.contains(p.0, p.1) {
            return None;
        }
        hit_chip((p.0 - f.list.x, p.1 - f.list.y + self.sv.offset()), self.v.chips.len(), self.cols())
    }

    fn chip_rect_vp(&self, i: usize) -> Rect {
        let f = self.frame();
        let r = chip_rect(i, self.cols());
        r.offset(f.list.x, f.list.y - self.sv.offset())
    }

    fn menu_rect(&self) -> Option<(Rect, usize)> {
        let m = self.v.menu.as_ref()?;
        let n = m.entries.len();
        Some((Rect::new(m.x, m.y, MENU_W, MENU_PAD * 2.0 + MENU_ITEM_H * n as f32), n))
    }

    fn menu_item_at(&self, p: Pt) -> Option<usize> {
        let (r, n) = self.menu_rect()?;
        if !r.contains(p.0, p.1) {
            return None;
        }
        let i = ((p.1 - r.y - MENU_PAD) / MENU_ITEM_H).floor();
        (i >= 0.0 && (i as usize) < n).then_some(i as usize)
    }

    fn hover_to(&mut self, id: Option<i64>) -> bool {
        if self.hover == id {
            return false;
        }
        if let Some(old) = self.hover.and_then(|o| self.chips.get_mut(&o)) {
            old.set_hover(false);
        }
        if let Some(new) = id.and_then(|o| self.chips.get_mut(&o)) {
            new.set_hover(true);
        }
        self.hover = id;
        true
    }

    fn release_all(&mut self) {
        for c in self.chips.values_mut() {
            c.set_press(false);
        }
        self.drag_btn.set_press(false);
        self.clear_btn.set_press(false);
    }

    /// Whether anything runs forever or still moves (frames wanted).
    fn moving(&self, cx: &Cx) -> bool {
        let reduced = cx.reduced_motion || glasskit::ease::reduced();
        let looping = !reduced && (self.v.empty || self.v.over);
        looping || self.base.player.busy() || self.chips.values().any(|c| c.hover() > 0.0 && c.hover() < 1.0) || self.over_t.progress() < 1.0
    }

    // ------------------------------------------------------------------------------------------------------------ painting

    fn sample(&self, target: &str) -> Sample {
        self.base.player.sample(target)
    }

    fn draw_chip(&self, cv: &Canvas, th: &Theme, i: usize, ch: &Chip, pos: (f32, f32), now: f64) {
        let target = format!("chip:{}", ch.id);
        let s = self.sample(&target);
        let ix = self.chips.get(&ch.id);
        let (hov, pressk) = ix.map_or((0.0, 1.0), |x| (x.hover(), x.press_scale(0.97)));
        let mut alpha = s.alpha.clamp(0.0, 1.0);
        let mut scale = s.scale * pressk;
        if ch.ghost {
            alpha *= 0.38;
            scale *= 0.94;
        }
        if ch.missing {
            alpha *= 0.45;
        }
        if self.flying.contains(&ch.id) && s.active {
            scale *= 0.3 + 0.7 * s.p.clamp(0.0, 1.4);
        }
        if alpha <= 0.003 || (ch.hidden && !s.active) {
            return;
        }
        let lift = if ch.missing || ch.ghost { 0.0 } else { -2.0 * hov };
        let r = Rect::new(pos.0, pos.1, CHIP_W, CHIP_H);
        let m = xf(r.center(), scale, s.rot, s.dx, s.dy + lift);
        let _ = i;
        cv.with_opacity(alpha, |cv| {
            cv.with_transform(m, |cv| {
                let sel = ch.selected;
                let fresh = self.fresh_t0.get(&ch.id).map_or(0.0, |t0| 1.0 - curves::EASE_OUT.at(((now - t0) / 1500.0) as f32).clamp(0.0, 1.0));
                if sel {
                    cv.fill_rr(r, 12.0, th.accent_soft);
                    cv.stroke_rr(r, 12.0, 1.0, th.accent);
                } else {
                    if hov > 0.0 && !ch.missing && !ch.ghost {
                        cv.shadow_rr(r, 12.0, 12.0, 6.0, Color::rgba(0.0, 0.0, 0.0, 0.32 * hov));
                        cv.fill_rr(r, 12.0, th.card.fade(hov));
                    }
                    if fresh > 0.0 {
                        cv.fill_rr(r, 12.0, th.accent_soft.fade(fresh));
                        cv.stroke_rr(r, 12.0, 1.0, th.accent.fade(fresh));
                    }
                }
                if ch.ghost {
                    paint::dashed_rr(cv, r, 12.0, th.border);
                }
                let ico = Rect::new(r.cx() - 18.0, r.y + 5.0, 36.0, 36.0);
                match &ch.tile {
                    ChipTile::Folder => icons::draw_markup(cv, "folder_fill", r#"<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" fill="currentColor"/>"#, ico, Color::rgb8(0xe7, 0xb4, 0x43), 1.5),
                    ChipTile::Doc { label, color } => paint::doc_tile(cv, ico, label, paint::hex(color)),
                    ChipTile::Image(url) => {
                        let drawn = self.keys.get(&ch.id).map_or(false, |k| self.base.images.draw(cv, k, ico, 8.0, 1.0, super::draw::Fit::Cover));
                        if !drawn {
                            let _ = url;
                            cv.fill_rr(ico, 8.0, th.raised);
                        }
                    }
                }
                let name_r = Rect::new(r.x + 5.0, r.y + 49.0, CHIP_W - 10.0, 32.0);
                let name_st = Style::new(12.0, 400).center().wrap();
                let name = clamp2(cv, &ch.name, name_st, name_r.w);
                cv.with_clip(name_r, |cv| cv.text(name_r, &name, name_st, th.text));
                if ch.missing {
                    let (w, h) = cv.measure(&name, name_st, name_r.w);
                    let lines = ((h / 16.0).round() as usize).clamp(1, 2);
                    for l in 0..lines {
                        let y = name_r.y + 8.0 + 16.0 * l as f32;
                        let lw = if lines == 1 { w } else { name_r.w - 8.0 };
                        cv.line((r.cx() - lw / 2.0, y), (r.cx() + lw / 2.0, y), 1.0, th.text);
                    }
                }
                cv.text(Rect::new(r.x + 5.0, r.y + 85.0, CHIP_W - 10.0, 14.0), &ch.sub, Style::new(11.0, 400).center().ellipsis(), th.dim);
                // the remove button pops in with the hover
                if hov > 0.0 && !ch.ghost {
                    let xb = x_btn(r);
                    let red = self.hover == Some(ch.id) && self.hover_x;
                    cv.with_opacity(hov, |cv| {
                        cv.with_scale_about(xb.center(), 0.5 + 0.5 * hov, |cv| {
                            cv.fill_circle(xb.center(), 9.0, if red { th.red } else { th.raised2 });
                            icons::draw(cv, "x", xb.centered(11.0, 11.0), if red { Color::rgb8(255, 255, 255) } else { th.text }, 2.2);
                        });
                    });
                }
            });
        });
    }

    fn draw_head(&mut self, cv: &Canvas, th: &Theme, head: Rect) {
        let drag_txt = self.v.drag_all_label.clone();
        let clear_txt = self.v.clear_label.clone();
        let tw = cv.text_width(&drag_txt, Style::new(13.0, 600));
        let dr = Rect::new(head.x, head.y, 12.0 + 14.0 + 8.0 + tw + 16.0, HEAD_H);
        let cw_txt = cv.text_width(&clear_txt, Style::new(13.0, 500));
        let cr = Rect::new(head.right() - (12.0 + 16.0 + 8.0 + cw_txt + 12.0), head.y, 12.0 + 16.0 + 8.0 + cw_txt + 12.0, HEAD_H);
        self.drag_rect = dr;
        self.clear_rect = cr;
        let bump = self.sample("dragall");
        // Drag all
        let k = self.drag_btn.press_scale(0.96);
        cv.with_scale_about(dr.center(), k, |cv| {
            cv.fill_rr(dr, 12.0, if self.drag_btn.hovered() { th.card_hover } else { th.card });
            icons::draw(cv, "grip", Rect::new(dr.x + 12.0, dr.cy() - 7.0, 14.0, 14.0), th.dim, 1.5);
            let tr_ = Rect::new(dr.x + 12.0 + 14.0 + 8.0, dr.y, tw + 2.0, dr.h);
            cv.with_scale_about(tr_.center(), bump.scale, |cv| cv.text(tr_, &drag_txt, Style::new(13.0, 600), th.text));
        });
        // Clear
        let k = self.clear_btn.press_scale(0.96);
        let hv = self.clear_btn.hover();
        cv.with_scale_about(cr.center(), k, |cv| {
            cv.fill_rr(cr, 12.0, th.hover.fade(hv));
            let c = th.dim.mix(th.text, hv);
            icons::draw(cv, "trash", Rect::new(cr.x + 12.0, cr.cy() - 8.0, 16.0, 16.0), c, 1.6);
            cv.text(Rect::new(cr.x + 12.0 + 16.0 + 8.0, cr.y, cw_txt + 2.0, cr.h), &clear_txt, Style::new(13.0, 500), c);
        });
    }

    fn draw_empty(&self, cv: &Canvas, th: &Theme, zone: Rect, cx: &Cx) {
        let text_st = Style::new(12.0, 400).center().wrap();
        let (_, th_) = cv.measure(&self.v.empty_state.text, text_st, 300.0);
        let text_h = ((th_ / 18.0).round().max(1.0)) * 18.0;
        let total = 84.0 + 4.0 + 8.0 + 18.0 + 8.0 + text_h;
        let y = zone.cy() - total / 2.0;
        let art = Rect::new(zone.cx() - 60.0, y, 120.0, 84.0);
        let reduced = cx.reduced_motion || glasskit::ease::reduced();
        paint::empty_art(cv, art, cx.now_ms, reduced, th.bg, th.text2);
        cv.text(Rect::new(zone.x, y + 84.0 + 4.0 + 8.0, zone.w, 18.0), &self.v.empty_state.title, Style::new(13.0, 600).center(), th.text);
        cv.text(Rect::new(zone.cx() - 150.0, y + 84.0 + 4.0 + 8.0 + 18.0 + 8.0, 300.0, text_h), &self.v.empty_state.text, text_st, th.faint);
    }

    fn draw_menu(&self, cv: &Canvas, th: &Theme) {
        let Some(m) = self.v.menu.as_ref().or(self.last_menu.as_ref()) else { return };
        let s = self.sample("menu");
        if s.alpha <= 0.003 {
            return;
        }
        let r = Rect::new(m.x, m.y, MENU_W, MENU_PAD * 2.0 + MENU_ITEM_H * m.entries.len() as f32);
        let mx = xf((r.x, r.y), s.scale, 0.0, s.dx, s.dy);
        cv.with_opacity(s.alpha.clamp(0.0, 1.0), |cv| {
            cv.with_transform(mx, |cv| {
                cv.shadow_rr(r, 12.0, 14.0, 10.0, th.shadow);
                cv.fill_rr(r, 12.0, th.raised);
                cv.stroke_rr(r, 12.0, 1.0, th.border);
                for (i, e) in m.entries.iter().enumerate() {
                    let ir = Rect::new(r.x + MENU_PAD, r.y + MENU_PAD + i as f32 * MENU_ITEM_H, MENU_W - 2.0 * MENU_PAD, MENU_ITEM_H);
                    if self.menu_hover == Some(i) && self.v.menu.is_some() {
                        cv.fill_rr(ir, 8.0, th.hover);
                    }
                    cv.text(Rect::new(ir.x + 12.0, ir.y, ir.w - 24.0, ir.h), &e.label, Style::new(13.0, 500).ellipsis(), th.text);
                }
            });
        });
    }
}

/// `-webkit-line-clamp: 2`: cut the name with an ellipsis so it fits two lines.
fn clamp2(cv: &Canvas, s: &str, st: Style, w: f32) -> String {
    let fits = |t: &str| cv.measure(t, st, w).1 <= 2.0 * 16.5 + 1.0;
    if fits(s) {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let t: String = chars[..mid].iter().collect::<String>() + "…";
        if fits(&t) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>() + "…"
}

impl View for ShelfView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((super::W, super::tab_h("shelf")? - super::CHROME_H))
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.enter_ctx(cx);
        self.last_ms = cx.now_ms;
        self.base.show(cx, &EVENTS, TICK_MS);
        self.model.set_expanded(true);
        let mut calls = self.model.on_show();
        calls.push(BackendCall::bare("settings_get"));
        self.sync(cx, calls);
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.base.hide();
        self.model.on_hide();
        self.press = Press::None;
        self.release_all();
        self.sync(cx, vec![]);
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.enter_ctx(cx);
        let mut calls = vec![];
        match self.base.classify(msg, cx) {
            Incoming::Event { name, payload } => calls = self.model.on_event(&name, &payload),
            Incoming::Result { cmd, args, result } => calls = self.feed_result(cmd, &args, result),
            Incoming::Tick(_) => {
                // the coarse model clock: an idle shelf must not repaint 4 times a second, only when the clock changed something
                let calls = self.advance(cx);
                let fxs = self.model.take_fx();
                if calls.is_empty() && fxs.is_empty() && self.model.view(tr(self.ru), Wall::new(0, 0)) == self.v {
                    return Response::NONE;
                }
                self.sync_with(cx, calls, fxs);
                return Response::redraw();
            }
            Incoming::Image(_) | Incoming::Other(_) => {}
        }
        self.sync(cx, calls);
        Response::redraw()
    }

    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        self.enter_ctx(cx);
        let o = self.base.rect;
        let local = (ev.pos.0 - o.x, ev.pos.1 - o.y);
        let files = |p: &Option<DropPayload>| -> Option<Vec<String>> {
            match p {
                Some(DropPayload::Files(f)) => Some(f.iter().map(|x| x.to_string_lossy().into_owned()).collect()),
                _ => None,
            }
        };
        let calls = match ev.phase {
            DropPhase::Enter => {
                if files(&ev.payload).is_none() {
                    return Response::NONE;
                }
                self.model.on_event("tauri://drag-enter", &Value::Null)
            }
            DropPhase::Over => return Response::consumed(),
            DropPhase::Leave => self.model.on_event("tauri://drag-leave", &Value::Null),
            DropPhase::Drop => {
                let Some(paths) = files(&ev.payload) else { return Response::NONE };
                self.model.on_event("tauri://drag-drop", &json!({ "paths": paths, "position": { "x": local.0, "y": local.1 } }))
            }
        };
        self.sync(cx, calls);
        if matches!(ev.phase, DropPhase::Enter | DropPhase::Drop) {
            Response { consumed: true, redraw: true, ..Response::NONE }
        } else {
            Response::redraw()
        }
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.enter_ctx(cx);
        let o = self.base.rect;
        let ev = ev.translated(-o.x, -o.y);
        let f = self.frame();
        self.sync_scroll();
        let mut resp = Response::NONE;
        match &ev {
            InputEvent::MouseMove { pos, .. } => {
                let p = *pos;
                let was_x = self.hover_x;
                let hi = self.chip_at(p);
                let id = hi.and_then(|i| self.v.chips.get(i)).map(|c| c.id);
                let mut changed = self.hover_to(id);
                self.hover_x = hi.map_or(false, |i| x_btn(self.chip_rect_vp(i)).contains(p.0, p.1));
                changed |= was_x != self.hover_x;
                let mi = self.menu_item_at(p);
                if mi != self.menu_hover {
                    self.menu_hover = mi;
                    changed = true;
                }
                changed |= self.drag_btn.set_hover(self.drag_rect.contains(p.0, p.1) && !self.v.empty);
                changed |= self.clear_btn.set_hover(self.clear_rect.contains(p.0, p.1) && !self.v.empty);
                if self.press == Press::Chip || self.press == Press::DragAll {
                    let calls = self.model.on_action(Action::PointerMove { x: p.0, y: p.1 });
                    if !calls.is_empty() {
                        self.press = Press::None;
                        self.release_all();
                    }
                    self.sync(cx, calls);
                    resp.capture = false;
                    changed = true;
                }
                let over_chip = hi.and_then(|i| self.v.chips.get(i)).map_or(false, |c| !c.missing);
                if over_chip || self.drag_btn.hovered() || self.clear_btn.hovered() || mi.is_some() {
                    resp.cursor = Some(Cursor::Hand);
                }
                resp.redraw = changed;
            }
            InputEvent::MouseLeave => {
                let a = self.hover_to(None);
                self.hover_x = false;
                self.menu_hover = None;
                resp.redraw = a | self.drag_btn.set_hover(false) | self.clear_btn.set_hover(false);
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, mods, clicks } => {
                let p = *pos;
                if self.v.menu.is_some() {
                    if let Some(i) = self.menu_item_at(p) {
                        self.act(cx, Action::MenuPick(i));
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                    self.act(cx, Action::MenuClose);
                    resp.redraw = true;
                }
                let r = self.sv.event(&ev, f.list);
                if r.consumed || r.capture {
                    return r;
                }
                if !self.v.empty && self.drag_rect.contains(p.0, p.1) {
                    self.drag_btn.set_press(true);
                    self.press = Press::DragAll;
                    self.act(cx, Action::DragAllPress { x: p.0, y: p.1 });
                    return Response { consumed: true, redraw: true, capture: true, ..Response::NONE };
                }
                if !self.v.empty && self.clear_rect.contains(p.0, p.1) {
                    self.clear_btn.set_press(true);
                    self.press = Press::Clear;
                    return Response { consumed: true, redraw: true, capture: true, ..Response::NONE };
                }
                if let Some(i) = self.chip_at(p) {
                    let id = self.v.chips[i].id;
                    if self.hover_x && x_btn(self.chip_rect_vp(i)).contains(p.0, p.1) {
                        self.act(cx, Action::RemoveChip(id));
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                    if *clicks >= 2 {
                        self.act(cx, Action::DoubleClick(id));
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                    if let Some(c) = self.chips.get_mut(&id) {
                        c.set_press(true);
                    }
                    self.press = Press::Chip;
                    self.act(cx, Action::ChipPress { id, x: p.0, y: p.1, ctrl: mods.ctrl, button0: true });
                    return Response { consumed: true, redraw: true, capture: true, ..Response::NONE };
                }
            }
            InputEvent::MouseUp { pos, button, .. } => {
                let p = *pos;
                match button {
                    MouseButton::Left => {
                        let pressed = self.press;
                        self.press = Press::None;
                        self.release_all();
                        let r = self.sv.event(&ev, f.list);
                        let calls = self.model.on_action(Action::PointerRelease);
                        self.sync(cx, calls);
                        if pressed == Press::Clear && self.clear_rect.contains(p.0, p.1) {
                            self.act(cx, Action::Clear);
                        }
                        resp = r.merge(Response::redraw());
                    }
                    MouseButton::Right => {
                        if let Some(i) = self.chip_at(p) {
                            let id = self.v.chips[i].id;
                            self.act(cx, Action::ContextMenu { id, x: p.0, y: p.1 });
                            resp = Response { consumed: true, redraw: true, ..Response::NONE };
                        }
                    }
                    _ => {}
                }
            }
            InputEvent::Wheel { .. } | InputEvent::Key { .. } => {
                if let InputEvent::Key { code: glasskit::input::KeyCode::Escape, down: true, .. } = &ev {
                    if self.model.wants_escape() {
                        self.act(cx, Action::MenuClose);
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                }
                resp = self.sv.event(&ev, f.list);
            }
            _ => {}
        }
        if resp.redraw || self.moving(cx) {
            cx.request_frame();
        }
        resp
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.enter_ctx(cx);
        let calls = self.advance(cx);
        if !calls.is_empty() || self.model_clock_running() {
            self.sync(cx, calls);
        }
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.base.player.step(dt_ms);
        for c in self.chips.values_mut() {
            busy |= c.step(dt);
        }
        busy |= self.drag_btn.step(dt) | self.clear_btn.step(dt) | self.over_t.step(dt) | self.sv.step(dt);
        busy | self.moving(cx) | self.model_clock_running()
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.base.rect = rect;
        cv.with_offset(rect.x, rect.y, |cv| self.draw_local(cv, th, cx));
    }

    fn on_dpi(&mut self, _scale: f32, _cx: &mut Cx) {
        // thumbnails are decoded at 96 px, enough for 36 logical px at 2.5x; bitmaps are re-made per device context by the cache
    }
}

impl ShelfView {
    /// Everything in tab-local coordinates (origin = the tab's top-left).
    fn draw_local(&mut self, cv: &Canvas, th: &Theme, cx: &Cx) {
        self.sync_scroll();
        let f = self.frame();
        let now = cx.now_ms;
        let zone = f.zone;
        let reduced = cx.reduced_motion || glasskit::ease::reduced();
        // the zone: transparent border, dashed while empty (breathing) or while a drag is over it (pulsing)
        let over = self.over_t.value().clamp(0.0, 1.0);
        let ring = self.sample("zone");
        let t_s = (now / 1000.0) as f32;
        if over > 0.0 {
            let pulse = if reduced { 0.0 } else { 0.5 - 0.5 * (t_s * std::f32::consts::TAU / 1.1).cos() };
            let bg = th.accent_soft.fade(1.0).mix(th.accent.alpha(0.26), pulse);
            cv.fill_rr(zone, ZONE_R, bg.fade(over));
            paint::dashed_rr(cv, zone, ZONE_R, th.accent.fade(over));
        } else if self.v.empty {
            let breathe = if reduced { 0.0 } else { 0.5 - 0.5 * (t_s * std::f32::consts::TAU / 3.4).cos() };
            paint::dashed_rr(cv, zone, ZONE_R, th.text.alpha(0.16 + 0.22 * breathe));
        }
        if !self.v.empty {
            self.draw_head(cv, th, f.head);
            let chips = std::mem::take(&mut self.v.chips);
            let c = self.cols();
            let flip = self.sample("list");
            let mut new_pos = HashMap::new();
            self.sv.draw(cv, f.list, th, |cv, _| {
                for (i, ch) in chips.iter().enumerate() {
                    let r = chip_rect(i, c);
                    new_pos.insert(ch.id, (r.x, r.y));
                    let mut pos = (r.x, r.y);
                    if flip.active {
                        if let Some(&(fx, fy)) = self.flip_from.get(&ch.id) {
                            pos = (fx + (r.x - fx) * flip.p, fy + (r.y - fy) * flip.p);
                        }
                    }
                    // chips are drawn in list content coordinates; `last_pos` / `flip_from` use the same space
                    self.draw_chip(cv, th, i, ch, pos, now);
                }
            });
            self.last_pos = new_pos;
            self.v.chips = chips;
        } else {
            self.draw_empty(cv, th, zone, cx);
        }
        // "landed" ring
        if ring.active && ring.alpha > 0.0 {
            cv.with_scale_about(zone.center(), 0.985 + 0.045 * ring.p.clamp(0.0, 1.0), |cv| cv.stroke_rr(zone.inset(-1.0), ZONE_R, 1.5, th.accent.fade((ring.alpha * 1.5).min(1.0))));
        }
        // "drop files here"
        if over > 0.01 {
            let veil = if th.dark { Color::rgba(0.0, 0.0, 0.0, 0.78) } else { Color::rgba(1.0, 1.0, 1.0, 0.8) };
            cv.fill_rr(zone, ZONE_R, veil.fade(over));
            let dy = 8.0 * (1.0 - over);
            cv.with_opacity(over, |cv| {
                icons::draw(cv, "download", Rect::new(zone.cx() - 11.0, zone.cy() - 11.0 - 13.0 + dy, 22.0, 22.0), th.accent, 1.7);
                cv.text(Rect::new(zone.x, zone.cy() + 4.0 + dy, zone.w, 18.0), &self.v.drop_title, Style::new(13.0, 600).center(), th.accent);
            });
        }
        self.draw_menu(cv, th);
    }
}

impl ShelfView {
    /// The model has a timer that must run on frames (a removal waiting for its exit, a fresh highlight).
    fn model_clock_running(&self) -> bool {
        self.v.chips.iter().any(|c| c.fresh || c.hidden) || self.v.landed
    }
}
