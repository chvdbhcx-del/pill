//! The Wi-Fi network list of the Controls tab: a card that drops in under the tiles, a scrolling list of networks (signal bars, a lock, the
//! state), a Disconnect button on the connected one and a password drawer that opens under the network being joined.
//! The logic is `pill_ui_core::panel::a::wifi_list`; this file is geometry, input and painting. All movement is [`Flow`]s: the card
//! (`pop`), the drawer, the message line and the rows fading in. They are retargeted in [`WifiUi::sync`] (never while drawing) and
//! [`WifiUi::step`] says whether a frame is still needed.
//! The password field is a masked `TextInput` (`Field`: input is applied on the next draw). Its text is read once, on Connect / Enter, and the
//! field is cleared at the same moment.
use super::paint::{self, Tok};
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::a::touch::Touch;
use crate::ui::tabs::a::weather::field::Field;
use glasskit::canvas::Canvas;
use glasskit::edit::EditEvent;
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::scroll::ScrollView;
use glasskit::text::{Align, Style};
use glasskit::theme::Theme;
use pill_ui_core::panel::a::controls::Action;
use pill_ui_core::panel::a::wifi_list::{RowState, WifiRow, WifiView};

pub const W: f32 = 300.0;
const TOP: f32 = 44.0;
const PAD: f32 = 11.0;
const HEADER_H: f32 = 26.0;
pub const ROW_H: f32 = 36.0;
const GAP: f32 = 2.0;
/// Height of the password drawer under a row: field 30, note line, padding.
pub const DRAWER_H: f32 = 78.0;
const LIST_MAX: f32 = 190.0;
const RADIUS: f32 = 18.0;

/// The card and its list viewport for `b` (the 648 x 260 content box) and a list of `content_h`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geo {
    pub outer: Rect,
    pub header: Rect,
    pub list: Rect,
    pub content_h: f32,
}

pub fn geo(b: Rect, content_h: f32) -> Geo {
    let list_h = content_h.clamp(40.0, LIST_MAX);
    let h = 1.0 + PAD + HEADER_H + list_h + PAD;
    let outer = Rect::new(b.cx() - W / 2.0, b.y + TOP, W, h);
    let x = outer.x + PAD;
    let w = W - 2.0 * PAD;
    let header = Rect::new(x, outer.y + PAD, w, HEADER_H);
    Geo { outer, header, list: Rect::new(x, header.bottom(), w, list_h), content_h }
}

/// Top of every row inside the scrolling content, and the content height. `ask` = (row index, drawer 0..1): the drawer pushes the rows after it down.
pub fn rows_y(n: usize, ask: Option<(usize, f32)>) -> (Vec<f32>, f32) {
    let extra = |i: usize| ask.filter(|a| a.0 < i).map_or(0.0, |a| a.1 * DRAWER_H);
    let ys: Vec<f32> = (0..n).map(|i| i as f32 * (ROW_H + GAP) + extra(i)).collect();
    let total = if n == 0 { 0.0 } else { n as f32 * (ROW_H + GAP) - GAP + ask.map_or(0.0, |a| a.1 * DRAWER_H) };
    (ys, total)
}

/// The Connect button and the field of a drawer that starts under the row `row` (a rect in content coordinates).
fn drawer_parts(row: Rect) -> (Rect, Rect) {
    let top = row.bottom() + 6.0;
    let go = Rect::new(row.right() - 10.0 - 84.0, top, 84.0, 30.0);
    (Rect::new(row.x + 10.0, top, go.x - 8.0 - (row.x + 10.0), 30.0), go)
}

struct RowUi {
    ssid: String,
    it: Touch,
    disc: Touch,
    seen: Flow,
}

pub struct WifiUi {
    /// card presence: lands with a little overshoot
    pop: Flow,
    drawer: Flow,
    msg: Flow,
    open: bool,
    /// the view as last shown, kept while the card leaves
    view: Option<WifiView>,
    msg_text: String,
    ask_ix: Option<usize>,
    ask_ssid: Option<String>,
    rows: Vec<RowUi>,
    go: Touch,
    field: Field,
    scroll: ScrollView,
    out: Vec<Action>,
}

impl Default for WifiUi {
    fn default() -> Self {
        WifiUi::new()
    }
}

impl WifiUi {
    pub fn new() -> WifiUi {
        let mut field = Field::new(&Theme::pill_dark(), "");
        field.input.password = true;
        WifiUi {
            pop: Flow::new(0.0, Feel::POP),
            drawer: Flow::new(0.0, Feel::FLOW),
            msg: Flow::new(0.0, Feel::FADE),
            open: false,
            view: None,
            msg_text: String::new(),
            ask_ix: None,
            ask_ssid: None,
            rows: vec![],
            go: Touch::new(),
            field,
            scroll: ScrollView::new(),
            out: vec![],
        }
    }

    /// The card is on screen (open, or still leaving).
    pub fn shown(&self) -> bool {
        self.view.is_some() && (self.open || self.pop.at01() > 0.01)
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The password field has the keyboard.
    pub fn typing(&self) -> bool {
        self.open && self.field.focused()
    }

    /// Retarget everything to the model's view (called after every model change, never from `draw`).
    pub fn sync(&mut self, v: Option<&WifiView>) {
        self.open = v.is_some();
        self.pop.to(if self.open { 1.0 } else { 0.0 });
        let Some(v) = v else {
            self.ask_ssid = None;
            self.drawer.to(0.0);
            self.msg.to(0.0);
            self.field.set_text("");
            self.field.input.set_focused(false);
            return;
        };
        let mut next: Vec<RowUi> = Vec::with_capacity(v.rows.len());
        for r in &v.rows {
            match self.rows.iter().position(|o| o.ssid == r.ssid) {
                Some(i) => next.push(self.rows.swap_remove(i)),
                None => {
                    let mut seen = Flow::new(0.0, Feel::FADE);
                    seen.to(1.0);
                    next.push(RowUi { ssid: r.ssid.clone(), it: Touch::new(), disc: Touch::new(), seen });
                }
            }
        }
        self.rows = next;
        let ask = v.rows.iter().position(|r| r.ask.is_some());
        let ssid = ask.map(|i| v.rows[i].ssid.clone());
        if ssid != self.ask_ssid {
            self.field.set_text(""); // a new drawer never starts with the last password
            self.field.input.set_focused(ssid.is_some());
            if let Some(i) = ask {
                self.field.input.placeholder = v.rows[i].ask.as_ref().map(|a| a.placeholder.clone()).unwrap_or_default();
                let (ys, _) = rows_y(v.rows.len(), None);
                self.scroll.state.scroll_to(ys[i], true);
            }
            self.ask_ssid = ssid;
        }
        if ask.is_some() {
            self.ask_ix = ask;
        }
        self.drawer.to(if ask.is_some() { 1.0 } else { 0.0 });
        if let Some(m) = &v.flash {
            self.msg_text = m.clone();
        }
        self.msg.to(if v.flash.is_some() { 1.0 } else { 0.0 });
        self.view = Some(v.clone());
    }

    /// Actions the view produced on its own (Enter in the field); the tab applies them to the model.
    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.out)
    }

    fn ask_pair(&self) -> Option<(usize, f32)> {
        self.ask_ix.map(|i| (i, self.drawer.at().clamp(0.0, 1.0)))
    }

    fn content(&self) -> (Vec<f32>, f32) {
        match &self.view {
            Some(v) if !v.rows.is_empty() => rows_y(v.rows.len(), self.ask_pair()),
            _ => (vec![], 40.0),
        }
    }

    /// Row rectangles on screen (scrolled), the viewport and the geometry.
    pub(super) fn rects(&self, b: Rect) -> (Geo, Vec<Rect>) {
        let (ys, total) = self.content();
        let g = geo(b, total);
        let off = self.scroll.offset();
        (g, ys.iter().map(|y| Rect::new(g.list.x, g.list.y + y - off, g.list.w, ROW_H)).collect())
    }

    /// Advance the motion; true while a frame is still needed.
    pub fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = false;
        for f in [&mut self.pop, &mut self.drawer, &mut self.msg] {
            f.step(dt_ms);
            busy |= f.moving();
        }
        for r in &mut self.rows {
            r.seen.step(dt_ms);
            busy |= r.seen.moving() | r.it.step(dt_ms) | r.disc.step(dt_ms);
        }
        busy |= self.go.step(dt_ms) | self.scroll.step((dt_ms / 1000.0) as f32);
        if self.open && self.field.focused() {
            busy |= self.field.input.step((dt_ms / 1000.0) as f32);
        }
        for e in self.field.take_events() {
            match e {
                EditEvent::Submit => self.submit(),
                EditEvent::Cancel => self.out.push(Action::WifiCancelAsk),
                _ => {}
            }
        }
        if !self.open && self.pop.at01() <= 0.01 {
            self.view = None; // the card has left
        }
        busy | self.field.pending()
    }

    fn submit(&mut self) {
        let pw = self.field.text();
        if !pw.is_empty() {
            self.field.set_text("");
            self.out.push(Action::WifiSubmit(pw));
        }
    }

    /// Pointer and keyboard. `keep` = the Wi-Fi tile's list button (a press there is the tile's, not an outside click).
    pub fn event(&mut self, ev: &InputEvent, b: Rect, keep: Option<Rect>) -> (Response, Vec<Action>) {
        let mut acts = vec![];
        if !self.open {
            return (Response::NONE, acts);
        }
        let (g, rows) = self.rects(b);
        let pos = ev.pos();
        let over = pos.is_some_and(|p| g.outer.contains(p.0, p.1));
        let mut resp = Response::NONE;
        if let InputEvent::MouseDown { pos, button: MouseButton::Left, .. } = ev {
            if !g.outer.contains(pos.0, pos.1) && !keep.is_some_and(|r| r.contains(pos.0, pos.1)) {
                acts.push(Action::ClosePopover);
                return (resp, acts);
            }
        }
        let drawer = self.drawer.at().clamp(0.0, 1.0);
        let ask_row = self.ask_ix.and_then(|i| rows.get(i).copied());
        let parts = ask_row.map(|r| drawer_parts(r));
        // keys and text go to the field while it has the keyboard
        if matches!(ev, InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_)) {
            if let (true, Some((f, _))) = (self.field.focused(), parts) {
                if matches!(ev, InputEvent::Key { code: KeyCode::Escape, down: true, .. }) {
                    acts.push(Action::WifiCancelAsk);
                    return (Response { consumed: true, redraw: true, ..Response::NONE }, acts);
                }
                return (self.field.queue(ev, f), acts);
            }
            return (Response::NONE, acts);
        }
        if let (Some((f, go)), true) = (parts, drawer > 0.5) {
            resp = resp.merge(self.field.queue(ev, f));
            let (r, p) = self.go.pointer(ev, |q| go.contains(q.0, q.1));
            resp = resp.merge(r);
            if p.click {
                self.submit();
            }
        }
        let visible = |r: Rect| {
            let (y0, y1) = (r.y.max(g.list.y), r.bottom().min(g.list.bottom()));
            (y1 > y0).then(|| Rect::new(r.x, y0, r.w, y1 - y0))
        };
        let far = InputEvent::MouseMove { pos: (-1.0e4, -1.0e4), mods: glasskit::input::Modifiers::NONE, buttons: glasskit::input::Buttons::NONE };
        let tev = if !over && !matches!(ev, InputEvent::MouseUp { .. }) { &far } else { ev };
        if let Some(v) = self.view.clone() {
            for (i, (row, r)) in v.rows.iter().zip(rows.iter()).enumerate() {
                let vis = visible(*r);
                let disc = disc_rect(*r);
                let on_disc = row.disconnect.is_some();
                if let Some(ui) = self.rows.get_mut(i) {
                    let (a, p) = ui.disc.pointer(tev, |q| on_disc && vis.is_some_and(|v| v.contains(q.0, q.1)) && disc.contains(q.0, q.1));
                    resp = resp.merge(a);
                    if p.click && on_disc {
                        acts.push(Action::WifiDisconnect);
                    }
                    let (a, p) = ui.it.pointer(tev, |q| vis.is_some_and(|v| v.contains(q.0, q.1)) && !(on_disc && disc.contains(q.0, q.1)));
                    resp = resp.merge(a);
                    if p.click && row.state == RowState::Normal {
                        acts.push(Action::WifiPick(row.ssid.clone()));
                    }
                }
            }
        }
        self.scroll.set_content_height(g.content_h, g.list.h);
        if matches!(ev, InputEvent::Wheel { .. }) || self.scroll.bar.dragging() {
            resp = resp.merge(self.scroll.event(ev, g.list));
        } else if matches!(ev, InputEvent::MouseDown { .. } | InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. }) {
            resp = resp.merge(self.scroll.bar.event(ev, g.list, &mut self.scroll.state));
        }
        if over {
            resp.consumed = true;
            resp.cursor = resp.cursor.or(Some(Cursor::Arrow));
        }
        if !acts.is_empty() {
            resp.redraw = true;
        }
        (resp, acts)
    }

    pub fn draw(&mut self, cv: &Canvas, th: &Theme, k: &Tok, b: Rect) {
        self.field.flush(cv);
        let Some(v) = self.view.clone() else { return };
        let alpha = self.pop.at01();
        if alpha < 0.005 {
            return;
        }
        let (g, rects) = self.rects(b);
        self.scroll.set_content_height(g.content_h, g.list.h);
        let p = self.pop.at();
        let (scale, dy) = (0.96 + 0.04 * p.min(1.12), (1.0 - p.min(1.0)) * -10.0);
        cv.with_opacity(alpha, |cv| {
            cv.with_offset(0.0, dy, |cv| {
                cv.with_scale_about((g.outer.cx(), g.outer.y), scale, |cv| {
                    cv.shadow_rr(g.outer, RADIUS, 30.0, 10.0, glasskit::color::Color::rgba(0.0, 0.0, 0.0, 0.55));
                    cv.fill_rr(g.outer, RADIUS, k.raised);
                    cv.stroke_rr(g.outer, RADIUS, 1.0, k.border);
                    let h = Rect::new(g.header.x + 8.0, g.header.y + 2.0, g.header.w - 16.0, 16.0);
                    cv.text(h, &v.title, Style::new(12.0, 600).ellipsis(), k.dim);
                    let m = self.msg.at01();
                    if m > 0.01 {
                        cv.text(h, &self.msg_text, Style::new(11.0, 500).right().ellipsis(), k.warn.fade(m));
                    }
                    cv.with_clip(g.list, |cv| {
                        if let Some(e) = &v.empty {
                            cv.text(Rect::new(g.list.x + 8.0, g.list.y + 12.0, g.list.w - 16.0, 16.0), e, Style::new(12.0, 400).center(), k.dim);
                        }
                        for (i, (row, r)) in v.rows.iter().zip(rects.iter()).enumerate() {
                            if r.bottom() < g.list.y || r.y > g.list.bottom() {
                                continue;
                            }
                            let ui = self.rows.get(i);
                            self.draw_row(cv, k, row, *r, ui);
                            if let (Some(a), Some(ai)) = (&row.ask, self.ask_ix) {
                                if ai == i {
                                    self.draw_drawer(cv, th, k, *r, a);
                                }
                            }
                        }
                    });
                    self.scroll.bar.draw(cv, g.list, &self.scroll.state, th);
                });
            });
        });
    }

    fn draw_row(&self, cv: &Canvas, k: &Tok, row: &WifiRow, r: Rect, ui: Option<&RowUi>) {
        let seen = ui.map_or(1.0, |u| u.seen.at01());
        let hov = ui.map_or(0.0, |u| u.it.hover());
        let scale = ui.map_or(1.0, |u| u.it.press_scale(0.985));
        let dim = if row.dim { 0.5 } else { 1.0 } * seen;
        cv.with_opacity(dim, |cv| {
            cv.with_scale_about(r.center(), scale, |cv| {
                if hov > 0.0 && !row.dim {
                    cv.fill_rr(r, 10.0, k.hover.fade(hov));
                }
                let connected = row.state == RowState::Connected;
                signal(cv, Rect::new(r.x + 10.0, r.cy() - 8.0, 16.0, 16.0), row.bars, if connected { k.accent } else { k.fg }, k.circle_off);
                let right = r.right() - 10.0;
                let mut text_right = right;
                if row.disconnect.is_none() {
                    if row.secured {
                        paint::icon(cv, "lock", Rect::new(right - 13.0, r.cy() - 6.5, 13.0, 13.0), k.dim);
                        text_right -= 19.0;
                    }
                    if !row.detail.is_empty() {
                        let dw = cv.text_width(&row.detail, Style::new(11.0, 400)).ceil();
                        let mut st = Style::new(11.0, 400);
                        st.align = Align::Right;
                        cv.text(Rect::new(text_right - dw, r.y, dw, r.h), &row.detail, st, if row.state == RowState::Connecting { k.accent } else { k.dim });
                        text_right -= dw + 8.0;
                    }
                } else if let Some(label) = &row.disconnect {
                    let d = disc_rect(r);
                    let hv = ui.map_or(0.0, |u| u.disc.hover());
                    cv.with_scale_about(d.center(), ui.map_or(1.0, |u| u.disc.press_scale(0.95)), |cv| {
                        cv.fill_rr(d, 11.0, k.on.mix(k.hover, hv));
                        cv.text(d, label, Style::new(11.0, 600).center().ellipsis(), k.fg);
                    });
                    text_right = d.x - 8.0;
                }
                cv.text(Rect::new(r.x + 34.0, r.y, (text_right - (r.x + 34.0)).max(0.0), r.h), &row.ssid, Style::new(13.0, if connected { 600 } else { 400 }).ellipsis(), if connected { k.accent } else { k.fg });
            });
        });
    }

    fn draw_drawer(&self, cv: &Canvas, th: &Theme, k: &Tok, row: Rect, a: &pill_ui_core::panel::a::wifi_list::AskView) {
        let d = self.drawer.at01();
        if d < 0.01 {
            return;
        }
        let (f, go) = drawer_parts(row);
        let area = Rect::new(row.x, row.bottom(), row.w, (DRAWER_H * self.drawer.at()).max(0.0));
        cv.with_clip(area, |cv| {
            cv.with_opacity(d, |cv| {
                self.field.input.draw(cv, f, th);
                let hv = self.go.hover();
                cv.with_scale_about(go.center(), self.go.press_scale(0.96), |cv| {
                    cv.fill_rr(go, 9.0, k.accent.mix(k.fg, 0.14 * hv));
                    cv.text(go, &a.button, Style::new(12.0, 600).center().ellipsis(), k.accent_fg);
                });
                if let Some(n) = &a.note {
                    cv.text(Rect::new(f.x, f.bottom() + 6.0, row.w - 20.0, 14.0), n, Style::new(11.0, 400).ellipsis(), k.warn);
                }
            });
        });
    }
}

fn disc_rect(r: Rect) -> Rect {
    Rect::new(r.right() - 10.0 - 88.0, r.cy() - 11.0, 88.0, 22.0)
}

/// Four signal bars in a 16 x 16 box; `n` of them lit.
fn signal(cv: &Canvas, r: Rect, n: u8, on: glasskit::color::Color, off: glasskit::color::Color) {
    for i in 0..4u8 {
        let h = 4.0 + 3.0 * i as f32;
        let x = r.x + i as f32 * 4.0;
        cv.fill_rr(Rect::new(x, r.bottom() - h, 3.0, h), 1.0, if i < n { on } else { off });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pill_ui_core::panel::a::wifi_list::AskView;

    fn row(ssid: &str, ask: bool, connected: bool) -> WifiRow {
        WifiRow {
            ssid: ssid.into(),
            bars: 3,
            secured: true,
            state: if connected { RowState::Connected } else { RowState::Normal },
            detail: String::new(),
            dim: false,
            ask: ask.then(|| AskView { placeholder: "pw".into(), button: "Connect".into(), note: None }),
            disconnect: connected.then(|| "Disconnect".into()),
        }
    }

    fn view(rows: Vec<WifiRow>) -> WifiView {
        WifiView { title: "Wi-Fi networks".into(), rows, empty: None, flash: None }
    }

    #[test]
    fn the_drawer_pushes_the_rows_after_it_down_and_grows_the_content() {
        let (ys, total) = rows_y(3, None);
        assert_eq!(ys, [0.0, 38.0, 76.0]);
        assert_eq!(total, 3.0 * 38.0 - 2.0);
        let (ys, total) = rows_y(3, Some((1, 1.0)));
        assert_eq!(ys, [0.0, 38.0, 76.0 + DRAWER_H]);
        assert_eq!(total, 3.0 * 38.0 - 2.0 + DRAWER_H);
        let (ys, _) = rows_y(3, Some((1, 0.5)));
        assert_eq!(ys[2], 76.0 + DRAWER_H / 2.0);
        assert_eq!(rows_y(0, Some((0, 1.0))), (vec![], 0.0));
    }

    #[test]
    fn the_card_hangs_under_the_middle_and_caps_its_list() {
        let b = Rect::new(0.0, 0.0, 648.0, 260.0);
        let g = geo(b, 3.0 * 38.0 - 2.0);
        assert_eq!((g.outer.x, g.outer.y, g.outer.w), (174.0, 44.0, W));
        assert_eq!(g.list.h, 112.0);
        assert_eq!(geo(b, 900.0).list.h, LIST_MAX);
        assert_eq!(geo(b, 0.0).list.h, 40.0, "the empty text still has room");
    }

    #[test]
    fn the_connect_button_and_the_field_sit_inside_the_row() {
        let r = Rect::new(100.0, 50.0, 278.0, ROW_H);
        let (f, go) = drawer_parts(r);
        assert!(f.x >= r.x && go.right() <= r.right() && f.right() < go.x);
        assert_eq!((f.y, f.h), (go.y, 30.0));
        assert!(go.bottom() < r.bottom() + DRAWER_H);
        assert!(disc_rect(r).right() < r.right() && disc_rect(r).h < ROW_H);
    }

    #[test]
    fn the_card_drops_in_and_out_on_springs_and_goes_away_when_it_has_left() {
        let _m = crate::ui::host::testing::motion_on();
        let mut u = WifiUi::new();
        assert!(!u.shown() && !u.step(16.0));
        u.sync(Some(&view(vec![row("Home", false, true), row("Cafe", false, false)])));
        assert!(u.is_open() && u.shown());
        assert!(u.step(16.0), "moving");
        let mut guard = 0;
        while u.step(16.0) && guard < 400 {
            guard += 1;
        }
        assert!(guard < 400 && u.pop.at01() > 0.99, "settled open");
        u.sync(None);
        assert!(!u.is_open() && u.shown(), "still on screen while it leaves");
        while u.step(16.0) && guard < 800 {
            guard += 1;
        }
        assert!(!u.shown() && u.view.is_none() && !u.step(16.0), "gone, and idle");
    }

    #[test]
    fn a_password_drawer_opens_for_the_asking_row_and_forgets_its_text() {
        let _m = crate::ui::host::testing::motion_on();
        let mut u = WifiUi::new();
        u.sync(Some(&view(vec![row("Home", false, true), row("Cafe", true, false)])));
        assert_eq!(u.ask_ix, Some(1));
        u.field.set_text("secret-pw");
        u.sync(Some(&view(vec![row("Home", false, true), row("Cafe", false, false)])));
        u.sync(Some(&view(vec![row("Home", false, true), row("Cafe", true, false)])));
        assert_eq!(u.field.text(), "", "a new drawer starts empty");
        u.sync(None);
        assert_eq!(u.field.text(), "");
    }
}
