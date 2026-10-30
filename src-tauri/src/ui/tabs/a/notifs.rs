//! The `notifs` tab: the notification history (what the island showed), grouped by day. Native port of `src/tab-notifs.js` / `tab-notifs.css`.
//!
//! * Model: `pill_ui_core::panel::a::notifs::Notifs` (storage commands `notif_history_*`, dedupe, unread bookkeeping, `Fx`).
//! * Layout (logical px, the tab box is 648 x 260): head 32 (count label + clear button), list (day headers 22 sticky, rows 44 / 46, 12 px
//!   radius, 32 px tone tile), foot 18 (+ 4 margin). Empty state is centred in the list area.
//! * Motion: rows enter staggered (`row:<id>`), unread dots pop (`dot:<id>`), a removed row collapses (its height shrinks and the rows below slide
//!   up with it; the row stays as a "ghost" until the cue ends), "clear" fades every row out, a new row FLIPs the list (`list`), the empty state
//!   enters (`empty`). Hover swaps time for the remove button. Reduce motion: no cues are played, every change lands at once.
//! * The island hands the view the cards it showed as `Msg::Custom(Box<ToastRec>)` (the model records them); see `docs/native-tabs-a.md`.
#[cfg(test)]
mod tests;

use super::core::{Base, Incoming};
use super::icons;
use crate::ui::view::{Cx, Msg, View};
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::controls::Interact;
use glasskit::ease::{curves, dur, Tween};
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, MouseButton, Response};
use glasskit::lang::Lang;
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::notifs::{Action, Item, Notifs, Row, ToastRec, View as NView};
use pill_ui_core::panel::a::util::Wall;
use pill_ui_core::panel::a::{BackendCall, EnTr, RuTr, Tr};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const HEAD_H: f32 = 32.0;
const FOOT_H: f32 = 18.0;
const FOOT_GAP: f32 = 4.0;
const DAY_H: f32 = 22.0;
const ROW_R: f32 = 12.0;
/// Rows are 4 px narrower than the box (the list pads 8 px on the right and hangs 4 px out).
const ROW_W: f32 = super::W - 4.0;
const X_BTN: f32 = 26.0;
const END_W: f32 = 84.0;
const EVENTS: [&str; 1] = ["settings:changed"];

/// Glyphs the shared icon set lacks (`tab-notifs.js` EXTRA).
const EXTRA: [(&str, &str); 3] = [
    ("mail", r#"<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="m3.5 7.5 8.5 6 8.5-6"/>"#),
    ("calendar", r#"<rect x="3.5" y="5" width="17" height="15" rx="2.5"/><path d="M3.5 10h17M8 3v4M16 3v4"/>"#),
    ("usb", r#"<path d="M12 3v13M12 16a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM12 11l4-3v-2M12 13l-4-3v-2"/><rect x="14.5" y="3.5" width="3" height="3" rx=".5"/><circle cx="8" cy="7" r="1.2"/>"#),
];

fn glyph(cv: &Canvas, name: &str, r: Rect, c: Color, sw: f32) {
    if icons::has(name) {
        icons::draw(cv, name, r, c, sw);
    } else if let Some((k, m)) = EXTRA.iter().find(|(n, _)| *n == name) {
        icons::draw_markup(cv, k, m, r, c, sw);
    } else {
        icons::draw(cv, "bell", r, c, sw);
    }
}

/// Local clock: epoch ms and the PC's UTC offset in minutes (east positive).
fn local_wall() -> Wall {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let off = unsafe {
        let mut tz = windows::Win32::System::Time::TIME_ZONE_INFORMATION::default();
        let id = windows::Win32::System::Time::GetTimeZoneInformation(&mut tz);
        -(tz.Bias + match id { 1 => tz.StandardBias, 2 => tz.DaylightBias, _ => 0 })
    };
    Wall::new(ms, off as i64)
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    Day(String),
    Row(Row),
}

#[derive(Clone, Debug, PartialEq)]
struct DItem {
    kind: Kind,
    /// Removed from the model but still playing its exit cue.
    ghost: bool,
}

impl DItem {
    fn id(&self) -> Option<i64> {
        match &self.kind {
            Kind::Row(r) => Some(r.id),
            Kind::Day(_) => None,
        }
    }
    fn base_h(&self) -> f32 {
        match &self.kind {
            Kind::Day(_) => DAY_H,
            Kind::Row(r) if r.sub.is_empty() => 44.0,
            Kind::Row(_) => 46.0,
        }
    }
}

fn tone(th: &Theme, t: &str) -> Option<(Color, Color)> {
    Some(match t {
        "green" => (th.green_bg, th.green),
        "amber" => (th.amber_bg, th.amber),
        "red" => (th.red_bg, th.red),
        "accent" => (th.accent_soft, th.accent),
        _ => return None,
    })
}

/// Static layout of the list in content coordinates.
struct Lay {
    tops: Vec<f32>,
    hs: Vec<f32>,
    total: f32,
}

pub struct NotifsView {
    base: Base,
    model: Notifs,
    lang: Lang,
    items: Vec<DItem>,
    nview: Option<NView>,
    scroll: ScrollView,
    clear_it: Interact,
    /// Row hover amounts (0..1), settled ones are dropped.
    hov: HashMap<i64, Tween<f32>>,
    hovered: Option<i64>,
    xh: Option<(i64, Tween<f32>)>,
    press_x: Option<i64>,
    /// Rows removed by the user: collapse (height shrinks); other ghosts only fade.
    collapsing: HashSet<i64>,
    /// FLIP: row id -> old top minus new top (content px) while the `list` cue runs; ids that were not there before.
    flip: HashMap<i64, f32>,
    flip_new: HashSet<i64>,
    list_loaded: bool,
    started: bool,
    now_override: Option<Wall>,
}

impl NotifsView {
    pub fn new() -> NotifsView {
        NotifsView {
            base: Base::new(),
            model: Notifs::new(),
            lang: Lang::En,
            items: vec![],
            nview: None,
            scroll: ScrollView::new(),
            clear_it: Interact::new(),
            hov: HashMap::new(),
            hovered: None,
            xh: None,
            press_x: None,
            collapsing: HashSet::new(),
            flip: HashMap::new(),
            flip_new: HashSet::new(),
            list_loaded: false,
            started: false,
            now_override: None,
        }
    }

    #[cfg(test)]
    fn with_now(mut self, w: Wall) -> NotifsView {
        self.now_override = Some(w);
        self
    }

    fn now(&self) -> Wall {
        self.now_override.unwrap_or_else(local_wall)
    }

    fn tr(&self) -> Box<dyn Tr> {
        if self.lang == Lang::Ru {
            Box::new(RuTr)
        } else {
            Box::new(EnTr)
        }
    }

    // ------------------------------------------------------------------------------------------------------------ geometry

    fn frame(&self) -> Rect {
        if self.base.rect.w > 0.0 {
            self.base.rect
        } else {
            Rect::new(0.0, 0.0, super::W, 260.0)
        }
    }

    fn list_rect(&self) -> Rect {
        let f = self.frame();
        Rect::new(f.x, f.y + HEAD_H, f.w, f.h - HEAD_H - FOOT_H - FOOT_GAP)
    }

    fn lay(&self) -> Lay {
        let (mut tops, mut hs, mut y) = (Vec::with_capacity(self.items.len()), Vec::with_capacity(self.items.len()), 0.0);
        for it in &self.items {
            let mut h = it.base_h();
            if it.ghost {
                if let Some(id) = it.id().filter(|id| self.collapsing.contains(id)) {
                    h *= (1.0 - self.base.player.sample(&format!("row:{id}")).p).clamp(0.0, 1.0);
                }
            }
            tops.push(y);
            hs.push(h);
            y += h;
        }
        Lay { tops, hs, total: y }
    }

    /// Top of every live row by id (static heights): the FLIP "before" and "after".
    fn static_tops(&self) -> HashMap<i64, f32> {
        let mut y = 0.0;
        let mut m = HashMap::new();
        for it in self.items.iter().filter(|i| !i.ghost) {
            if let Some(id) = it.id() {
                m.insert(id, y);
            }
            y += it.base_h();
        }
        m
    }

    fn clear_rect(&self, cv: Option<&Canvas>) -> Rect {
        let f = self.frame();
        let label = self.nview.as_ref().map_or("", |v| v.clear_label.as_str());
        let tw = cv.map_or(label.chars().count() as f32 * 6.5, |c| c.text_width(label, Style::new(12.0, 600)));
        let w = 8.0 + 15.0 + 8.0 + tw + 12.0;
        Rect::new(f.right() - 4.0 - w, f.y + 2.0, w, 28.0)
    }

    fn row_rect(&self, lay: &Lay, i: usize) -> Rect {
        let l = self.list_rect();
        Rect::new(l.x, l.y + lay.tops[i] - self.scroll.offset(), ROW_W, lay.hs[i])
    }

    fn x_rect(row: Rect) -> Rect {
        Rect::new(row.right() - 8.0 - X_BTN, row.cy() - X_BTN / 2.0, X_BTN, X_BTN)
    }

    fn row_at(&self, p: (f32, f32)) -> Option<usize> {
        let l = self.list_rect();
        if !l.contains(p.0, p.1) || p.0 > l.x + ROW_W {
            return None;
        }
        let lay = self.lay();
        (0..self.items.len()).find(|&i| {
            let it = &self.items[i];
            matches!(it.kind, Kind::Row(_)) && !it.ghost && self.row_rect(&lay, i).contains(p.0, p.1)
        })
    }

    // ------------------------------------------------------------------------------------------------------------ model plumbing

    fn run(&mut self, cx: &mut Cx, calls: Vec<BackendCall>) {
        self.base.calls.run(cx, calls);
    }

    /// Rebuild the drawn list from the model, keeping rows that are still playing an exit cue as ghosts.
    fn refresh(&mut self) {
        let tr = self.tr();
        let v = self.model.view(&*tr, self.now());
        let fresh: Vec<DItem> = v
            .items
            .iter()
            .map(|i| DItem { kind: match i { Item::Day(d) => Kind::Day(d.clone()), Item::Row(r) => Kind::Row(r.clone()) }, ghost: false })
            .collect();
        let live: HashSet<i64> = fresh.iter().filter_map(|i| i.id()).collect();
        // ghosts: old rows that left the model but whose cue still runs, attached after the nearest surviving predecessor
        let mut after: Vec<(Option<i64>, DItem)> = vec![];
        let mut prev: Option<i64> = None;
        for it in &self.items {
            let Some(id) = it.id() else { continue };
            if live.contains(&id) {
                prev = Some(id);
            } else if self.base.player.sample(&format!("row:{id}")).active {
                after.push((prev, DItem { kind: it.kind.clone(), ghost: true }));
            }
        }
        let mut out = fresh;
        for (p, g) in after {
            let at = match p {
                Some(pid) => out.iter().position(|i| i.id() == Some(pid)).map_or(out.len(), |k| k + 1),
                None => usize::from(matches!(out.first().map(|i| &i.kind), Some(Kind::Day(_)))),
            };
            out.insert(at.min(out.len()), g);
        }
        self.items = out;
        self.nview = Some(v);
        let total = self.lay().total;
        let vh = self.list_rect().h;
        self.scroll.set_content_height(total, vh);
    }

    /// Drop ghosts whose cue ended.
    fn reap(&mut self) -> bool {
        let done: Vec<String> = self.base.player.finished();
        let mut any = false;
        for t in done {
            if let Some(id) = t.strip_prefix("row:").and_then(|s| s.parse::<i64>().ok()) {
                if self.items.iter().any(|i| i.ghost && i.id() == Some(id)) {
                    self.base.player.release(&t);
                    self.collapsing.remove(&id);
                    any = true;
                }
            }
        }
        if any {
            self.refresh();
        }
        any
    }

    /// After every model call: drain `Fx`, rebuild, start the FLIP if the `list` cue just began.
    fn settle(&mut self, cx: &mut Cx, before: Option<HashMap<i64, f32>>) {
        self.model.set_os_reduce_motion(cx.reduced_motion);
        let list_was = self.base.player.is_active("list");
        let _ = self.base.route_fx(cx, self.model.take_fx());
        self.refresh();
        if !list_was && self.base.player.is_active("list") {
            if let Some(old) = before {
                let new = self.static_tops();
                self.flip.clear();
                self.flip_new.clear();
                for (id, nt) in &new {
                    match old.get(id) {
                        Some(ot) if (ot - nt).abs() > 0.5 => {
                            self.flip.insert(*id, ot - nt);
                        }
                        None => {
                            self.flip_new.insert(*id);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn feed_result(&mut self, cx: &mut Cx, cmd: &'static str, args: &Value, result: Result<Value, String>) {
        match (cmd, result) {
            ("settings_get", Ok(v)) => {
                self.model.set_settings(&v);
            }
            ("settings_get", Err(_)) => {}
            (_, Ok(v)) => {
                let before = Some(self.static_tops());
                let calls = self.model.on_result(cmd, args, &v);
                self.run(cx, calls);
                if cmd == "notif_history_list" {
                    self.list_loaded = true;
                    if self.base.is_shown() {
                        let calls = self.model.on_show();
                        self.run(cx, calls);
                    }
                }
                self.settle(cx, before);
            }
            (c, Err(_)) => {
                // a rejected clear: the rows come back (the cue would hold them invisible)
                if c == "notif_history_clear" {
                    self.base.player.clear();
                    self.refresh();
                }
            }
        }
    }

    fn act(&mut self, cx: &mut Cx, a: Action) {
        if let Action::Remove(id) = a {
            self.collapsing.insert(id);
        }
        let before = Some(self.static_tops());
        let calls = self.model.on_action(a);
        self.run(cx, calls);
        self.settle(cx, before);
        // let the ghost rows keep their place in the scroll range
        cx.request_frame();
    }

    fn retarget_hov(&mut self, id: i64, to: f32) {
        let cur = self.hov.get(&id).map_or(0.0, |t| t.value());
        let mut t = Tween::at_rest(cur);
        t.retarget(to, dur::FAST, curves::EASE_OUT);
        self.hov.insert(id, t);
    }

    fn set_hover(&mut self, id: Option<i64>) -> bool {
        if self.hovered == id {
            return false;
        }
        if let Some(old) = self.hovered {
            self.retarget_hov(old, 0.0);
        }
        if let Some(n) = id {
            self.retarget_hov(n, 1.0);
        }
        self.hovered = id;
        true
    }

    fn hover_amount(&self, id: i64) -> f32 {
        self.hov.get(&id).map_or(0.0, |t| t.value().clamp(0.0, 1.0))
    }

    fn set_xh(&mut self, id: Option<i64>) -> bool {
        let cur = self.xh.as_ref().map(|(i, t)| (*i, t.value()));
        match (cur, id) {
            (Some((i, _)), Some(n)) if i == n => false,
            (None, None) => false,
            (c, n) => {
                let from = if c.map(|x| x.0) == n { c.map_or(0.0, |x| x.1) } else { 0.0 };
                if let Some(n) = n {
                    let mut t = Tween::at_rest(from);
                    t.retarget(1.0, dur::FAST, curves::EASE_OUT);
                    self.xh = Some((n, t));
                } else {
                    self.xh = None;
                }
                true
            }
        }
    }

    fn pointer_move(&mut self, p: (f32, f32)) -> bool {
        let lay = self.lay();
        let idx = self.row_at(p);
        let id = idx.and_then(|i| self.items[i].id());
        let mut changed = self.set_hover(id);
        let xid = idx.filter(|&i| Self::x_rect(self.row_rect(&lay, i)).contains(p.0, p.1)).and_then(|i| self.items[i].id());
        changed |= self.set_xh(xid);
        changed
    }
}

impl View for NotifsView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((super::W, super::tab_h("notifs")? - super::CHROME_H))
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.lang = cx.lang;
        self.base.show(cx, &EVENTS, 250);
        self.model.set_os_reduce_motion(cx.reduced_motion);
        if !self.started {
            self.started = true;
            let mut calls = self.model.init_calls();
            calls.push(BackendCall::bare("settings_get"));
            self.run(cx, calls);
        }
        let calls = self.model.on_show();
        self.run(cx, calls);
        self.settle(cx, None);
        cx.request_frame();
    }

    fn on_hide(&mut self, _cx: &mut Cx) {
        self.base.hide();
        self.model.on_hide();
        self.refresh();
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.lang = cx.lang;
        let before = self.nview.clone();
        match self.base.classify(msg, cx) {
            Incoming::Event { name, payload } => {
                let b = Some(self.static_tops());
                let calls = self.model.on_event(&name, &payload);
                self.run(cx, calls);
                self.settle(cx, b);
            }
            Incoming::Result { cmd, args, result } => self.feed_result(cx, cmd, &args, result),
            Incoming::Tick(dt) => {
                let calls = self.model.tick(dt);
                self.run(cx, calls);
                self.settle(cx, None);
            }
            Incoming::Other(Msg::Custom(b)) => {
                if let Ok(rec) = b.downcast::<ToastRec>() {
                    let now = self.now().ms;
                    let calls = self.model.on_toast(&rec, now);
                    self.run(cx, calls);
                }
            }
            Incoming::Image(_) | Incoming::Other(_) => {}
        }
        if self.nview != before || self.base.player.busy() {
            cx.request_frame();
            Response::redraw()
        } else {
            Response::NONE
        }
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let mut r = Response::NONE;
        let clear_vis = self.nview.as_ref().map_or(false, |v| v.clear_visible);
        let crect = self.clear_rect(None);
        if clear_vis {
            let (cr, ptr) = self.clear_it.pointer(ev, |q| crect.contains(q.0, q.1));
            r = r.merge(cr);
            if ptr.click {
                self.act(cx, Action::Clear);
                r.redraw = true;
            }
            if cr.capture || ptr.down {
                return r;
            }
        }
        let list = self.list_rect();
        let sr = self.scroll.event(ev, list);
        if sr.consumed || sr.capture {
            return r.merge(sr);
        }
        r = r.merge(sr);
        match ev {
            InputEvent::MouseMove { pos, .. } => {
                if self.pointer_move(*pos) {
                    r.redraw = true;
                }
                if self.xh.is_some() {
                    r.cursor = Some(Cursor::Hand);
                }
            }
            InputEvent::MouseLeave => {
                r.redraw |= self.set_hover(None) | self.set_xh(None);
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => {
                if let Some(i) = self.row_at(*pos) {
                    let lay = self.lay();
                    if Self::x_rect(self.row_rect(&lay, i)).contains(pos.0, pos.1) {
                        self.press_x = self.items[i].id();
                        r.consumed = true;
                    }
                }
            }
            InputEvent::MouseUp { pos, button: MouseButton::Left, .. } => {
                if let Some(id) = self.press_x.take() {
                    let lay = self.lay();
                    let hit = (0..self.items.len()).any(|i| self.items[i].id() == Some(id) && !self.items[i].ghost && Self::x_rect(self.row_rect(&lay, i)).contains(pos.0, pos.1));
                    if hit {
                        self.act(cx, Action::Remove(id));
                        self.pointer_move(*pos);
                        r.redraw = true;
                    }
                    r.consumed = true;
                }
            }
            _ => {}
        }
        if self.base.player.busy() || self.hov.values().any(|t| !t.finished()) {
            cx.request_frame();
        }
        r
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        let dt = (dt_ms / 1000.0) as f32;
        self.base.reduced = cx.reduced_motion;
        let mut busy = self.base.player.step(dt_ms);
        busy |= self.scroll.step(dt);
        busy |= self.clear_it.step(dt);
        for t in self.hov.values_mut() {
            busy |= t.step(dt);
        }
        let hv = self.hovered;
        self.hov.retain(|id, t| !t.finished() || t.target() > 0.0 || Some(*id) == hv);
        if let Some((_, t)) = &mut self.xh {
            busy |= t.step(dt);
        }
        if self.reap() {
            busy = true;
        }
        if !self.base.player.is_active("list") && !self.flip.is_empty() {
            self.flip.clear();
            self.flip_new.clear();
        }
        // collapsing rows change the scroll range every frame
        if self.items.iter().any(|i| i.ghost) {
            let total = self.lay().total;
            let vh = self.list_rect().h;
            self.scroll.set_content_height(total, vh);
        }
        busy
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.base.rect = rect;
        if self.nview.is_none() {
            self.lang = cx.lang;
            self.refresh();
        }
        let v = self.nview.clone().expect("refreshed above");
        // ---- head
        let count_col = if v.off { th.amber } else { th.dim };
        cv.text(Rect::new(rect.x + 12.0, rect.y, rect.w - 160.0, HEAD_H), &v.count.to_uppercase(), th.label_caps(), count_col);
        if v.clear_visible {
            self.draw_clear(cv, th, &v);
        }
        // ---- list or empty state
        if v.empty {
            self.draw_empty(cv, th, &v, rect);
        } else {
            self.draw_list(cv, th);
        }
        // ---- foot
        let foot = Rect::new(rect.x + 12.0, rect.bottom() - FOOT_H, rect.w - 24.0, FOOT_H);
        cv.text(foot, &v.footer, Style::new(11.0, 400).ellipsis(), if v.off { th.amber } else { th.faint });
    }
}

impl NotifsView {
    fn draw_clear(&self, cv: &Canvas, th: &Theme, v: &NView) {
        let r = self.clear_rect(Some(cv));
        let h = self.clear_it.hover();
        let (bg, fg) = if v.clear_armed { (th.red_bg, th.red) } else { (th.hover.fade(h), th.dim.mix(th.text, h)) };
        cv.with_scale_about(r.center(), self.clear_it.press_scale(0.96), |cv| {
            cv.fill_rr(r, 10.0, bg);
            glyph(cv, "trash", Rect::new(r.x + 8.0, r.cy() - 7.5, 15.0, 15.0), fg, 1.7);
            cv.text(Rect::new(r.x + 31.0, r.y, r.w - 31.0, r.h), &v.clear_label, Style::new(12.0, 600), fg);
        });
    }

    fn draw_empty(&self, cv: &Canvas, th: &Theme, v: &NView, rect: Rect) {
        let s = self.base.player.sample("empty");
        let area = Rect::new(rect.x, rect.y + HEAD_H, rect.w, rect.h - HEAD_H - FOOT_H - FOOT_GAP);
        let tw = 300.0;
        let (_, txt_h) = cv.measure(&v.empty_text, Style::new(12.0, 400).wrap(), tw);
        let total = 48.0 + 8.0 + 18.0 + 8.0 + txt_h.max(18.0);
        let y0 = area.cy() - total / 2.0;
        cv.with_opacity(s.alpha, |cv| {
            cv.with_offset(s.dx, s.dy, |cv| {
                let ic = Rect::new(rect.cx() - 24.0, y0, 48.0, 48.0);
                cv.fill_rr(ic, 16.0, th.card);
                glyph(cv, "bell", ic.centered(26.0, 26.0), th.text2, 1.5);
                cv.text(Rect::new(rect.x, y0 + 56.0, rect.w, 18.0), &v.empty_title, Style::new(13.0, 600).center(), th.text);
                cv.text(Rect::new(rect.cx() - tw / 2.0, y0 + 82.0, tw, txt_h.max(18.0)), &v.empty_text, Style::new(12.0, 400).wrap().center(), th.faint);
            });
        });
    }

    fn draw_list(&self, cv: &Canvas, th: &Theme) {
        let list = self.list_rect();
        let lay = self.lay();
        let off = self.scroll.offset();
        let anim = self.base.player.busy();
        let flip_p = self.base.player.sample("list");
        cv.with_clip(list, |cv| {
            for (i, it) in self.items.iter().enumerate() {
                let (top, h) = (list.y + lay.tops[i] - off, lay.hs[i]);
                if top + h < list.y || top > list.bottom() || h <= 0.05 {
                    continue;
                }
                let Kind::Row(row) = &it.kind else { continue };
                let rr = Rect::new(list.x, top, ROW_W, it.base_h());
                let (mut a, mut dx, mut dy, mut sc) = (1.0, 0.0, 0.0, 1.0);
                if anim {
                    let s = self.base.player.sample(&format!("row:{}", row.id));
                    (a, dx, dy, sc) = (s.alpha, s.dx, s.dy, s.scale);
                    if flip_p.active {
                        if let Some(d) = self.flip.get(&row.id) {
                            dy += d * (1.0 - flip_p.p);
                        }
                        if self.flip_new.contains(&row.id) {
                            a *= flip_p.p.clamp(0.0, 1.0);
                            dy += 4.0 * (1.0 - flip_p.p);
                        }
                    }
                }
                // a collapsing row is clipped to its shrinking height
                let clip = Rect::new(list.x, top, ROW_W + 4.0, h);
                cv.with_clip(clip, |cv| {
                    cv.with_opacity(a.clamp(0.0, 1.0), |cv| {
                        cv.with_scale_about(rr.center(), sc, |cv| cv.with_offset(dx, dy, |cv| self.draw_row(cv, th, row, rr, it.ghost)));
                    });
                });
            }
            // sticky day headers, painted over the rows
            let heads: Vec<usize> = (0..self.items.len()).filter(|&i| matches!(self.items[i].kind, Kind::Day(_))).collect();
            for (k, &i) in heads.iter().enumerate() {
                let Kind::Day(label) = &self.items[i].kind else { continue };
                let mut y = lay.tops[i].max(off);
                if let Some(&n) = heads.get(k + 1) {
                    y = y.min(lay.tops[n] - DAY_H);
                }
                let y = list.y + y - off;
                if y + DAY_H < list.y || y > list.bottom() {
                    continue;
                }
                let r = Rect::new(list.x, y, ROW_W, DAY_H);
                cv.fill_rect(r, th.bg);
                cv.text(Rect::new(r.x + 12.0, r.y, r.w - 24.0, DAY_H), &label.to_uppercase(), th.label_caps(), th.dim);
            }
        });
        self.scroll.bar.draw(cv, list, &self.scroll.state, th);
    }

    fn draw_row(&self, cv: &Canvas, th: &Theme, row: &Row, rr: Rect, ghost: bool) {
        let hv = if ghost { 0.0 } else { self.hover_amount(row.id) };
        let xh = self.xh.as_ref().filter(|(i, _)| *i == row.id).map_or(0.0, |(_, t)| t.value().clamp(0.0, 1.0));
        cv.fill_rr(rr, ROW_R, th.raised.fade(hv));
        let cy = rr.cy();
        if row.unread {
            let s = self.base.player.sample(&format!("dot:{}", row.id)).scale.max(0.0);
            cv.fill_circle((rr.x + 9.0, cy), 3.0 * s, th.accent);
        }
        // tile
        let tile = Rect::new(rr.x + 18.0, cy - 16.0, 32.0, 32.0);
        let (tbg, tfg) = tone(th, &row.tone).unwrap_or((th.raised.mix(th.raised2, hv), th.text2));
        cv.fill_rr(tile, 9.0, tbg);
        glyph(cv, &row.icon, tile.centered(17.0, 17.0), tfg, 1.7);
        // text
        let bx = tile.right() + 12.0;
        let end_r = rr.right() - 8.0;
        let bw = (end_r - END_W - bx).max(40.0);
        let tw = if row.unread { 600 } else { 400 };
        if row.sub.is_empty() {
            cv.text(Rect::new(bx, cy - 9.0, bw, 18.0), &row.title, Style::new(13.0, tw).ellipsis(), th.text);
        } else {
            cv.text(Rect::new(bx, cy - 17.0, bw, 18.0), &row.title, Style::new(13.0, tw).ellipsis(), th.text);
            cv.text(Rect::new(bx, cy + 3.0, bw, 14.0), &row.sub, Style::new(11.0, 500).ellipsis(), th.dim);
        }
        // time <-> remove button
        cv.text(Rect::new(end_r - END_W, cy - 7.0, END_W, 14.0), &row.time, Style::new(11.0, 400).right(), th.faint.fade(1.0 - hv));
        if hv > 0.01 {
            let xr = Self::x_rect(rr);
            cv.with_opacity(hv, |cv| {
                cv.with_scale_about(xr.center(), 0.85 + 0.15 * hv, |cv| {
                    cv.fill_rr(xr, 8.0, th.raised2.fade(xh));
                    glyph(cv, "x", xr.centered(14.0, 14.0), th.dim.mix(th.text, xh), 2.0);
                });
            });
        }
    }
}
