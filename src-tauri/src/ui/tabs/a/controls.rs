//! The `controls` tab: toggle tiles (Wi-Fi, Ethernet, Bluetooth + paired-device popover, dark theme) and the four power tiles (press twice).
//! Native view over `pill_ui_core::panel::a::controls::Controls`; spec = `src/tab-controls.js` / `tab-controls.css`. There are no sliders in this tab.
//!
//! Wiring (see `docs/native-tabs-a.md`): backend events `sys:changed`, `pill:show`, `pill:hide`, `settings:changed` -> `Controls::on_event`; the
//! commands the model returns (`sys_state`, `sys_set`, `sys_power`) run through `Cx::call_async` and come back to `on_result`; the model clock
//! (5 s poll, 15 s UAC wait, 3 s arm window, 2.5 s flash) is advanced by a 250 ms ticker that only wakes the view, never a frame by itself.
//! Animation cues of the model drive: `tile:<id>` the busy pulse of the icon circle, `power:<a>` the draining red wash, `bt-pop` the popover.
mod layout;
mod paint;
#[cfg(test)]
mod tests;

use super::core::{Base, Incoming};
use crate::ui::view::{Cx, Msg, View};
use glasskit::canvas::{mul, scale_about, translation, Canvas};
use glasskit::controls::Interact;
use glasskit::ease::{curves, dur, Tween};
use glasskit::geom::Rect;
use glasskit::input::{Buttons, Cursor, InputEvent, KeyCode, Modifiers, MouseButton, Response};
use glasskit::lang::Lang;
use glasskit::scroll::ScrollView;
use glasskit::theme::Theme;
use paint::Tok;
use pill_ui_core::panel::a::controls::{Action, Controls, Popover, View as Model};
use pill_ui_core::panel::a::{EnTr, RuTr, Tr};
use serde_json::Value;

/// How often the model clock is advanced while the tab is shown.
const TICK_MS: u64 = 250;
const EVENTS: [&str; 4] = ["sys:changed", "pill:show", "pill:hide", "settings:changed"];
const POWER_ACTIONS: [&str; 4] = ["lock", "sleep", "restart", "shutdown"];

struct TileState {
    id: String,
    intr: Interact,
    on: Tween<f32>,
}

/// What a click landed on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hit {
    Tile(usize),
    List,
    Power(usize),
    PopButton,
}

pub struct ControlsView {
    model: Controls,
    base: Base,
    ru: bool,
    v: Model,
    tiles: Vec<TileState>,
    power: Vec<Interact>,
    list: Interact,
    pbtn: Interact,
    focus: Option<usize>,
    /// the popover as last shown, kept while its exit cue plays
    last_pop: Option<Popover>,
    scroll: ScrollView,
    size: (f32, f32),
    /// the system monitor's meters (CPU / RAM / GPU) over the tiles, fed by the island (`Meters`); `None` = no row
    meters: Option<pill_ui_core::live::sysmon::MetersView>,
    meter_w: Vec<(String, Tween<f32>)>,
}

/// The island hands the Controls tab the latest meter sample (`LiveCmd::SysMeters`) as `Msg::Custom(Box<Meters>)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Meters(pub pill_ui_core::live::sysmon::MetersView);

/// True for that message (the island routes it to this tab).
pub(crate) fn is_meters(b: &(dyn std::any::Any + Send)) -> bool {
    b.is::<Meters>()
}

/// Height of the meter row (44) plus its gap (12).
const METER_ROW: f32 = 56.0;

impl ControlsView {
    pub fn new() -> ControlsView {
        let mut c = ControlsView {
            model: Controls::new(),
            base: Base::new(),
            ru: false,
            v: Model { tiles: vec![], power: vec![], popover: None },
            tiles: vec![],
            power: (0..4).map(|_| Interact::new()).collect(),
            list: Interact::new(),
            pbtn: Interact::new(),
            focus: None,
            last_pop: None,
            scroll: ScrollView::new(),
            size: (super::W, super::tab_h("controls").unwrap_or(331.0) - super::CHROME_H),
            meters: None,
            meter_w: vec![],
        };
        c.sync();
        c
    }

    fn tr(&self) -> &'static dyn Tr {
        if self.ru {
            &RuTr
        } else {
            &EnTr
        }
    }

    /// The box of the tiles and the power row: the content box, shortened by the meter row while it is shown.
    fn grid(&self) -> Rect {
        let b = self.bounds();
        if self.meters.as_ref().is_some_and(|m| m.visible && !m.rows.is_empty()) {
            Rect::new(b.x, b.y + METER_ROW, b.w, (b.h - METER_ROW).max(0.0))
        } else {
            b
        }
    }

    fn set_meters(&mut self, m: pill_ui_core::live::sysmon::MetersView) {
        for r in &m.rows {
            match self.meter_w.iter_mut().find(|(k, _)| *k == r.key) {
                Some((_, t)) => t.retarget(r.pct / 100.0, dur::SLOW, curves::EASE_OUT),
                None => self.meter_w.push((r.key.clone(), Tween::at_rest(r.pct / 100.0))),
            }
        }
        self.meters = Some(m);
    }

    fn bounds(&self) -> Rect {
        if self.base.rect.w > 0.0 {
            self.base.rect
        } else {
            Rect::new(0.0, 0.0, self.size.0, self.size.1)
        }
    }

    /// Rebuild the view-model and everything derived from it (interaction states, cue clean-up). Cheap: called after every model change.
    fn sync(&mut self) {
        self.v = self.model.view(self.tr());
        if self.tiles.len() != self.v.tiles.len() || self.tiles.iter().zip(&self.v.tiles).any(|(s, t)| s.id != t.id) {
            self.tiles = self.v.tiles.iter().map(|t| TileState { id: t.id.clone(), intr: Interact::new(), on: Tween::at_rest(t.on as u8 as f32) }).collect();
            self.focus = None;
        }
        for (s, t) in self.tiles.iter_mut().zip(&self.v.tiles) {
            s.intr.set_enabled(!t.disabled);
            s.on.retarget(t.on as u8 as f32, dur::MED, curves::EASE_OUT);
            if !t.busy {
                self.base.player.release(&format!("tile:{}", t.id));
            }
        }
        for (p, a) in self.v.power.iter().zip(POWER_ACTIONS) {
            if !p.armed {
                self.base.player.release(&format!("power:{a}"));
            }
        }
        if let Some(p) = &self.v.popover {
            self.last_pop = Some(p.clone());
        }
    }

    fn run(&mut self, cx: &mut Cx, calls: Vec<pill_ui_core::panel::a::BackendCall>) {
        self.base.calls.run(cx, calls);
    }

    /// After any model change: route the effects, rebuild derived state.
    fn after(&mut self, cx: &mut Cx) {
        let fx = self.model.take_fx();
        self.base.route_fx(cx, fx);
        self.sync();
    }

    fn act(&mut self, cx: &mut Cx, a: Action) {
        let calls = self.model.on_action(a);
        self.run(cx, calls);
        self.after(cx);
    }

    fn apply_hit(&mut self, cx: &mut Cx, h: Hit) {
        match h {
            Hit::Tile(i) => {
                if let Some(t) = self.v.tiles.get(i).filter(|t| !t.disabled) {
                    let id = t.id.clone();
                    self.act(cx, Action::Toggle(id));
                }
            }
            Hit::List => self.act(cx, Action::TogglePopover),
            Hit::Power(i) => self.act(cx, Action::Power(POWER_ACTIONS[i].into())),
            Hit::PopButton => self.act(cx, Action::OpenBtSettings),
        }
    }

    fn pop_layout(&self) -> Option<layout::Pop> {
        let p = self.v.popover.as_ref().or(self.last_pop.as_ref().filter(|_| self.base.player.is_active("bt-pop")))?;
        Some(layout::popover(self.bounds(), p.rows.len()))
    }

    fn bt_tile(&self) -> Option<usize> {
        self.v.tiles.iter().position(|t| t.list_button.is_some())
    }

    fn list_rect(&self, tiles: &[Rect]) -> Option<Rect> {
        let i = self.bt_tile()?;
        (!self.v.tiles[i].disabled).then(|| layout::list_button(tiles[i]))
    }

    /// Focusable things in Tab order: enabled tiles, the list button, the power tiles.
    fn focus_items(&self) -> Vec<Hit> {
        let mut v: Vec<Hit> = self.v.tiles.iter().enumerate().filter(|(_, t)| !t.disabled).map(|(i, _)| Hit::Tile(i)).collect();
        if self.bt_tile().map_or(false, |i| !self.v.tiles[i].disabled) {
            v.push(Hit::List);
        }
        v.extend((0..self.v.power.len()).map(Hit::Power));
        v
    }

    fn set_focus(&mut self, idx: Option<usize>) {
        self.focus = idx;
        let items = self.focus_items();
        let cur = idx.and_then(|i| items.get(i).copied());
        for (i, s) in self.tiles.iter_mut().enumerate() {
            s.intr.set_focus(cur == Some(Hit::Tile(i)), true);
        }
        self.list.set_focus(cur == Some(Hit::List), true);
        for (i, p) in self.power.iter_mut().enumerate() {
            p.set_focus(cur == Some(Hit::Power(i)), true);
        }
    }

    fn key(&mut self, code: KeyCode, mods: Modifiers, cx: &mut Cx) -> Response {
        match code {
            KeyCode::Escape if self.model.wants_escape() => {
                self.act(cx, Action::ClosePopover);
                Response { consumed: true, redraw: true, ..Response::NONE }
            }
            KeyCode::Tab => {
                let n = self.focus_items().len();
                if n == 0 {
                    return Response::NONE;
                }
                let next = match (self.focus, mods.shift) {
                    (None, false) => Some(0),
                    (None, true) => Some(n - 1),
                    (Some(i), false) if i + 1 < n => Some(i + 1),
                    (Some(i), true) if i > 0 => Some(i - 1),
                    _ => None, // leaves the tab
                };
                let leaves = next.is_none();
                self.set_focus(next);
                Response { consumed: !leaves, redraw: true, ..Response::NONE }
            }
            KeyCode::Enter | KeyCode::Space => {
                let hit = self.focus.and_then(|i| self.focus_items().get(i).copied());
                match hit {
                    Some(h) => {
                        self.apply_hit(cx, h);
                        Response { consumed: true, redraw: true, ..Response::NONE }
                    }
                    None => Response::NONE,
                }
            }
            _ => Response::NONE,
        }
    }
}

impl View for ControlsView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some(self.size)
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.ru = cx.lang == Lang::Ru;
        self.model.set_language_ru(self.ru);
        self.model.set_os_reduce_motion(cx.reduced_motion);
        self.base.show(cx, &EVENTS, TICK_MS);
        let calls = self.model.on_show();
        self.run(cx, calls);
        self.after(cx);
        cx.request_frame();
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.model.on_hide();
        self.base.hide();
        self.base.player.clear();
        self.set_focus(None);
        self.after(cx);
        self.last_pop = None;
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        let before = self.v.clone();
        match self.base.classify(msg, cx) {
            Incoming::Event { name, payload } => {
                let calls = self.model.on_event(&name, &payload);
                self.run(cx, calls);
            }
            Incoming::Result { cmd, args, result } => {
                let calls = self.model.on_result(cmd, &args, &result.unwrap_or(Value::Null));
                self.run(cx, calls);
            }
            Incoming::Tick(dt) => {
                let calls = self.model.tick(dt);
                self.run(cx, calls);
            }
            Incoming::Other(Msg::Custom(b)) => match b.downcast::<Meters>() {
                Ok(m) => {
                    let changed = self.meters.as_ref() != Some(&m.0);
                    self.set_meters(m.0);
                    if self.base.is_shown() {
                        cx.request_frame();
                    }
                    return if changed { Response::redraw() } else { Response::NONE };
                }
                Err(_) => return Response::NONE,
            },
            Incoming::Image(_) | Incoming::Other(_) => return Response::NONE,
        }
        self.after(cx);
        if self.v != before || self.base.player.busy() {
            Response::redraw()
        } else {
            Response::NONE
        }
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        if let InputEvent::Key { code, mods, down: true, .. } = ev {
            return self.key(*code, *mods, cx);
        }
        let b = self.grid();
        let tiles = layout::tiles(b, self.v.tiles.len());
        let power = layout::power(b);
        let list_r = self.list_rect(&tiles);
        let pop = self.v.popover.as_ref().and_then(|p| Some(layout::popover(b, p.rows.len())));
        let pos = ev.pos();
        let over_pop = matches!((pos, pop), (Some(p), Some(pl)) if pl.outer.contains(p.0, p.1));
        let mut resp = Response::NONE;
        let mut hits: Vec<Hit> = vec![];

        // outside press closes the popover (it still reaches whatever is under the pointer, like the web's capture listener)
        if let InputEvent::MouseDown { pos, button: MouseButton::Left, .. } = ev {
            if pop.is_some() && !over_pop && !list_r.map_or(false, |r| r.contains(pos.0, pos.1)) {
                self.act(cx, Action::ClosePopover);
            }
        }
        if let Some(pl) = pop {
            let (r, p) = self.pbtn.pointer(ev, |q| pl.button.contains(q.0, q.1));
            resp = resp.merge(r);
            if p.click {
                hits.push(Hit::PopButton);
            }
            self.scroll.set_content_height(pl.content_h, pl.list.h);
            if matches!(ev, InputEvent::Wheel { .. }) || self.scroll.bar.dragging() {
                resp = resp.merge(self.scroll.event(ev, pl.list));
            } else if matches!(ev, InputEvent::MouseDown { .. } | InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. }) {
                resp = resp.merge(self.scroll.bar.event(ev, pl.list, &mut self.scroll.state));
            }
        }
        // under the popover nothing is hot
        let far = InputEvent::MouseMove { pos: (-1.0e4, -1.0e4), mods: Modifiers::NONE, buttons: Buttons::NONE };
        let tev = if over_pop && !matches!(ev, InputEvent::MouseUp { .. }) { &far } else { ev };

        if let Some(lr) = list_r {
            let (r, p) = self.list.pointer(tev, |q| lr.contains(q.0, q.1));
            resp = resp.merge(r);
            if p.click {
                hits.push(Hit::List);
            }
        }
        for (i, r) in tiles.iter().enumerate() {
            let skip = list_r.filter(|_| self.v.tiles[i].list_button.is_some());
            let (rr, p) = self.tiles[i].intr.pointer(tev, |q| r.contains(q.0, q.1) && !skip.map_or(false, |l| l.contains(q.0, q.1)));
            resp = resp.merge(rr);
            if p.click {
                hits.push(Hit::Tile(i));
            }
        }
        for (i, r) in power.iter().enumerate() {
            let (rr, p) = self.power[i].pointer(tev, |q| r.contains(q.0, q.1));
            resp = resp.merge(rr);
            if p.click {
                hits.push(Hit::Power(i));
            }
        }
        if let (Some(p), true) = (pos, over_pop) {
            resp.consumed = true;
            resp.cursor = resp.cursor.or(Some(Cursor::Arrow));
            let _ = p;
        }
        for h in hits {
            self.apply_hit(cx, h);
            resp.redraw = true;
        }
        resp
    }

    fn step(&mut self, dt_ms: f64, _cx: &mut Cx) -> bool {
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.base.player.step(dt_ms);
        for s in &mut self.tiles {
            busy |= s.intr.step(dt);
            busy |= s.on.step(dt);
        }
        for p in &mut self.power {
            busy |= p.step(dt);
        }
        for (_, t) in &mut self.meter_w {
            busy |= t.step(dt);
        }
        busy |= self.list.step(dt) | self.pbtn.step(dt) | self.scroll.step(dt);
        busy
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, _cx: &Cx) {
        self.base.rect = rect;
        let k = Tok::of(th);
        self.draw_meters(cv, &k, rect);
        let rect_all = rect;
        let rect = self.grid();
        let tiles = layout::tiles(rect, self.v.tiles.len());
        for (i, (t, r)) in self.v.tiles.iter().zip(&tiles).enumerate() {
            let s = &self.tiles[i];
            let look = paint::TileLook {
                on: s.on.value().clamp(0.0, 1.0),
                hover: s.intr.hover(),
                scale: s.intr.press_scale(0.96),
                circle_scale: s.intr.press_scale(0.92),
                circle_alpha: self.base.player.sample(&format!("tile:{}", t.id)).alpha,
                list_hover: if t.list_button.is_some() { self.list.hover() } else { 0.0 },
                ring: s.intr.ring(),
            };
            paint::tile(cv, th, &k, *r, t, &look);
            if t.list_button.is_some() && !t.disabled {
                cv.focus_ring(layout::list_button(*r), 12.0, th.accent, self.list.ring());
            }
        }
        for (i, (p, r)) in self.v.power.iter().zip(layout::power(rect)).enumerate() {
            let wash = if p.armed { self.base.player.sample(&format!("power:{}", POWER_ACTIONS[i])).alpha } else { 0.0 };
            let wash = if self.base.player.is_active(&format!("power:{}", POWER_ACTIONS[i])) { wash } else { 0.0 };
            let look = paint::PowerLook { hover: self.power[i].hover(), scale: self.power[i].press_scale(0.96), wash, ring: self.power[i].ring() };
            paint::power(cv, th, &k, r, p, &look);
        }
        self.draw_popover(cv, th, &k, rect_all);
    }
}

impl ControlsView {
    /// `.sm-row` of `sysmon-live.css`: equal columns, 44 px high, a label + value line and a 4 px bar (the bar width eases like the web's .5 s transition).
    fn draw_meters(&self, cv: &Canvas, k: &Tok, rect: Rect) {
        use glasskit::text::Style;
        let Some(m) = self.meters.as_ref().filter(|m| m.visible && !m.rows.is_empty()) else { return };
        let n = m.rows.len() as f32;
        let w = (rect.w - 12.0 * (n - 1.0)) / n;
        for (i, r) in m.rows.iter().enumerate() {
            let b = Rect::new(rect.x + i as f32 * (w + 12.0), rect.y, w, 44.0);
            cv.fill_rr(b, 14.0, k.off);
            cv.text(Rect::new(b.x + 12.0, b.y + 8.0, w - 24.0, 16.0), &r.label, Style::new(10.0, 600).top(), k.dim);
            cv.text(Rect::new(b.x + 12.0, b.y + 8.0, w - 24.0, 16.0), &r.value, Style::new(12.0, 600).top().right().mono().ellipsis(), if r.hot { k.warn } else { k.fg });
            let track = Rect::new(b.x + 12.0, b.bottom() - 8.0 - 4.0, w - 24.0, 4.0);
            cv.fill_rr(track, 2.0, k.raised);
            let f = self.meter_w.iter().find(|(key, _)| *key == r.key).map_or(r.pct / 100.0, |(_, t)| t.value()).clamp(0.0, 1.0);
            if f > 0.0 {
                cv.fill_rr(Rect::new(track.x, track.y, (track.w * f).max(4.0), 4.0), 2.0, if r.hot { k.warn } else { k.accent });
            }
        }
    }

    fn draw_popover(&mut self, cv: &Canvas, th: &Theme, k: &Tok, rect: Rect) {
        let s = self.base.player.sample("bt-pop");
        let open = self.v.popover.clone();
        let shown = match (&open, &self.last_pop) {
            (Some(p), _) => Some(p.clone()),
            (None, Some(p)) if s.active && s.alpha > 0.01 => Some(p.clone()),
            _ => None,
        };
        let Some(p) = shown else {
            if open.is_none() {
                self.base.player.release("bt-pop");
                self.last_pop = None;
            }
            return;
        };
        let pl = layout::popover(rect, p.rows.len());
        self.scroll.set_content_height(pl.content_h, pl.list.h);
        let look = paint::PopLook { button_hover: self.pbtn.hover(), button_scale: self.pbtn.press_scale(0.97), scroll: self.scroll.offset() };
        // transform-origin: top centre; offset and scale come from the cue
        let m = mul(&scale_about((pl.outer.cx(), pl.outer.y), s.scale, s.scale), &translation(s.dx, s.dy));
        cv.with_opacity(s.alpha, |cv| {
            cv.with_transform(m, |cv| {
                paint::popover(cv, k, &pl, &p, &look);
                self.scroll.bar.draw(cv, pl.list, &self.scroll.state, th);
            })
        });
    }
}

