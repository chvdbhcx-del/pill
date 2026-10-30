//! The Weather tab: sky card (condition gradient, big temperature, stats, sun times), the 8-slot hourly strip, the 7-day list with temperature
//! range bars, and the off / city / error / loading cards. Port of `src/tab-weather.js` + `tab-weather.css` on top of
//! `pill_ui_core::panel::a::weather::Weather`; nothing is fetched until the user enabled it (the model keeps weather opt-in).
//!
//! Wiring (details in `docs/native-tabs-a.weather.part.md`): `on_show` subscribes `settings:changed` and starts the model refresh; replies of
//! `settings_get | weather_get | settings_set | weather_search_city | weather_set_city` go to `Weather::on_result` (a rejected command is fed as
//! `null`, like the JS wrapper's `inv`); `Fx::Anim` cues play on the `Player` (`gradient` crossfade, `refresh` spin) and the view adds enter
//! cues of its own for card changes, result rows and day rows. Text input goes through `weather::field::Field` (input is applied on the next frame).
mod field;
#[cfg(test)]
mod tests;
mod wxicons;

use super::core::{Base, Incoming};
use super::draw;
use super::icons;
use crate::ui::view::{Cx, Msg, View};
use field::Field;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::controls::{Button, ButtonKind, Interact, Widget};
use glasskit::edit::EditEvent;
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, Response};
use glasskit::lang::Lang;
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::weather::{Action, DayView, HourView, OkView, Weather, View as Vw};
use pill_ui_core::panel::a::{Anim, EnTr, RuTr, Tr};
use serde_json::Value;
use std::cell::Cell;

/// Content height of the tab (`tabH` 417 minus the panel chrome).
const H: f32 = 417.0 - super::CHROME_H;
const LEFT_W: f32 = 214.0;
const GAP: f32 = 12.0;
const BLUE: Color = Color::rgb8(0x7f, 0xb2, 0xff);
const WHITE: Color = Color::rgb8(255, 255, 255);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Loading,
    Off,
    City,
    Error,
    Ok,
}

fn kind_of(v: &Vw) -> Kind {
    match v {
        Vw::Loading { .. } => Kind::Loading,
        Vw::Off(_) => Kind::Off,
        Vw::City(_) => Kind::City,
        Vw::Error(_) => Kind::Error,
        Vw::Ok(_) => Kind::Ok,
    }
}

/// Buttons of the cards, by index into `bw` (their measured widths).
const B_ENABLE: usize = 0;
const B_SEARCH: usize = 1;
const B_CANCEL: usize = 2;
const B_RETRY: usize = 3;
const B_CITY: usize = 4;
const B_OFF: usize = 5;

pub struct WeatherView {
    model: Weather,
    base: Base,
    vw: Vw,
    ru: bool,
    kind: Kind,
    /// the sky card's name last drawn and the rows count last seen (to start enter cues on change)
    nres: usize,
    contact: Field,
    city: Field,
    btn: [Button; 6],
    bw: Cell<[f32; 6]>,
    text_h: Cell<f32>,
    city_it: Interact,
    rf_it: Interact,
    unit_it: [Interact; 2],
    res_it: Vec<Interact>,
    res_scroll: ScrollView,
    /// keyboard focus was requested from the host (so it is given back)
    kbd: bool,
}

impl WeatherView {
    pub fn new() -> WeatherView {
        let th = Theme::pill_dark();
        let mut base = Base::new();
        base.rect = Rect::new(0.0, 0.0, super::W, H);
        let mut model = Weather::new();
        model.set_tz_min(local_tz_min());
        let mk = |l: &str, k: ButtonKind| Button::new(l).kind(k);
        let mut v = WeatherView {
            vw: model.view(&EnTr),
            model,
            base,
            ru: false,
            kind: Kind::Loading,
            nres: 0,
            contact: Field::new(&th, ""),
            city: Field::new(&th, ""),
            btn: [mk("", ButtonKind::Primary), mk("", ButtonKind::Primary), mk("", ButtonKind::Ghost), mk("", ButtonKind::Normal), mk("", ButtonKind::Ghost), mk("", ButtonKind::Ghost)],
            bw: Cell::new([110.0, 70.0, 70.0, 90.0, 100.0, 90.0]),
            text_h: Cell::new(36.0),
            city_it: Interact::new(),
            rf_it: Interact::new(),
            unit_it: [Interact::new(), Interact::new()],
            res_it: Vec::new(),
            res_scroll: ScrollView::new(),
            kbd: false,
        };
        v.contact.input.style = v.contact.input.style.center(); // .wx-contact { text-align: center }
        v
    }

    #[cfg(test)]
    pub(crate) fn model_mut(&mut self) -> &mut Weather {
        &mut self.model
    }

    fn tr(&self) -> &'static dyn Tr {
        if self.ru {
            &RuTr
        } else {
            &EnTr
        }
    }

    // ------------------------------------------------------------------------------------------------------------ model plumbing

    /// Run follow-up calls, refresh the cached view-model, route effects, react to a card change.
    fn pump(&mut self, cx: &mut Cx, calls: Vec<pill_ui_core::panel::a::BackendCall>) {
        self.base.calls.run(cx, calls);
        self.sync(cx);
    }

    fn sync(&mut self, cx: &mut Cx) {
        self.vw = self.model.view(self.tr());
        let fx = self.model.take_fx();
        self.base.route_fx(cx, fx);
        self.labels();
        let k = kind_of(&self.vw);
        if k != self.kind {
            self.enter(k, cx);
        }
        if let Vw::City(c) = &self.vw {
            if c.results.len() != self.nres {
                self.nres = c.results.len();
                self.res_it = (0..self.nres).map(|_| Interact::new()).collect();
                for i in 0..self.nres {
                    self.base.player.play(&format!("res:{i}"), Anim::row_stagger(i), self.base.reduced);
                }
            }
            self.btn[B_SEARCH].set_enabled(!c.searching);
        }
        if self.base.player.busy() {
            cx.request_frame();
        }
    }

    fn labels(&mut self) {
        let (set, texts): (bool, [&str; 6]) = match &self.vw {
            Vw::Off(o) => (true, [&o.enable, "", "", "", "", ""]),
            Vw::City(c) => (true, ["", &c.search, c.cancel.as_deref().unwrap_or(""), "", "", ""]),
            Vw::Error(e) => (true, ["", "", "", &e.retry, &e.change_city, &e.turn_off]),
            _ => (false, [""; 6]),
        };
        if set {
            for (b, t) in self.btn.iter_mut().zip(texts) {
                if !t.is_empty() && b.label != t {
                    b.label = t.to_string();
                }
            }
        }
        match &self.vw {
            Vw::Off(o) => self.contact.input.placeholder = o.contact_placeholder.clone(),
            Vw::City(c) => self.city.input.placeholder = c.placeholder.clone(),
            _ => {}
        }
    }

    /// The card changed: reset fields, play the card's enter cue, move keyboard focus.
    fn enter(&mut self, k: Kind, cx: &mut Cx) {
        let from = self.kind;
        self.kind = k;
        self.res_scroll = ScrollView::new();
        self.nres = 0;
        self.base.player.play("body", Anim::enter(6.0), self.base.reduced);
        match &self.vw {
            Vw::Off(o) => {
                self.contact.set_text(&o.contact);
                self.contact.input.set_focused(false);
            }
            Vw::City(c) => {
                self.city.set_text(&c.prefill);
                self.city.input.set_focused(true);
            }
            Vw::Ok(_) if from != Kind::Ok => {
                for i in 0..7 {
                    self.base.player.play(&format!("day:{i}"), Anim::row_stagger(i), self.base.reduced);
                }
            }
            _ => {}
        }
        let want = matches!(k, Kind::City);
        if want != self.kbd {
            self.kbd = want;
            cx.set_focus(want);
        }
    }

    fn act(&mut self, a: Action, cx: &mut Cx) {
        let calls = self.model.on_action(a);
        self.pump(cx, calls);
    }

    // ------------------------------------------------------------------------------------------------------------ layout

    fn o(&self) -> Rect {
        self.base.rect
    }

    /// Items of a centred card column: (width, height, extra margin-top), laid out with the CSS `gap: 8px`, centred in the tab.
    fn column(&self, items: &[(f32, f32, f32)]) -> Vec<Rect> {
        let o = self.o();
        let total: f32 = items.iter().enumerate().map(|(i, it)| it.1 + it.2 + if i > 0 { 8.0 } else { 0.0 }).sum();
        let mut y = o.y + ((o.h - total) / 2.0).max(0.0);
        items
            .iter()
            .enumerate()
            .map(|(i, &(w, h, mt))| {
                y += mt + if i > 0 { 8.0 } else { 0.0 };
                let r = Rect::new(o.cx() - w / 2.0, y, w, h);
                y += h;
                r
            })
            .collect()
    }

    fn bwid(&self, i: usize) -> f32 {
        self.bw.get()[i]
    }

    /// Rects of a card's elements in tab coordinates: `[badge, title, text?, ...mode specific]`.
    fn card(&self) -> Vec<Rect> {
        let th = self.text_h.get();
        match &self.vw {
            Vw::Off(_) => self.column(&[(44.0, 44.0, 0.0), (508.0, 20.0, 4.0), (460.0, th, 0.0), (340.0, 32.0, 6.0), (self.bwid(B_ENABLE).max(110.0), 32.0, 4.0)]),
            Vw::City(c) => {
                let n = c.results.len() as f32;
                let list_h = if n > 0.0 { (n * 34.0 - 2.0).min(130.0) } else { 0.0 };
                let msg_h = if c.msg.is_empty() { 0.0 } else { 18.0 };
                let mut items = vec![(44.0, 44.0, 0.0), (508.0, 20.0, 4.0), (460.0, th, 0.0), (380.0, 32.0, 6.0), (380.0, msg_h, 0.0), (380.0, list_h, 0.0)];
                if c.cancel.is_some() {
                    items.push((self.bwid(B_CANCEL), 32.0, 0.0));
                }
                self.column(&items)
            }
            Vw::Error(e) => {
                let row = self.bwid(B_RETRY) + self.bwid(B_CITY) + self.bwid(B_OFF) + 12.0;
                if e.text.is_empty() {
                    self.column(&[(44.0, 44.0, 0.0), (508.0, 20.0, 4.0), (row, 32.0, 4.0)])
                } else {
                    self.column(&[(44.0, 44.0, 0.0), (508.0, 20.0, 4.0), (460.0, th, 0.0), (row, 32.0, 4.0)])
                }
            }
            _ => vec![],
        }
    }

    fn search_rects(&self, row: Rect) -> (Rect, Rect) {
        let bw = self.bwid(B_SEARCH);
        (Rect::new(row.x, row.y, row.w - bw - 8.0, 32.0), Rect::new(row.right() - bw, row.y, bw, 32.0))
    }

    fn err_buttons(&self, row: Rect) -> [Rect; 3] {
        let (a, b, c) = (self.bwid(B_RETRY), self.bwid(B_CITY), self.bwid(B_OFF));
        [Rect::new(row.x, row.y, a, 32.0), Rect::new(row.x + a + 6.0, row.y, b, 32.0), Rect::new(row.x + a + b + 12.0, row.y, c, 32.0)]
    }

    fn city_rect(&self) -> Rect {
        let o = self.o();
        Rect::new(o.x + 14.0, o.y + 16.0, 150.0, 24.0)
    }
    fn rf_rect(&self) -> Rect {
        let o = self.o();
        Rect::new(o.x + LEFT_W - 14.0 + 5.0 - 24.0, o.y + 16.0, 24.0, 24.0)
    }
    fn days_rect(&self) -> Rect {
        let o = self.o();
        Rect::new(o.x + LEFT_W + GAP, o.y + 94.0 + GAP, o.w - LEFT_W - GAP, o.h - 94.0 - GAP)
    }
    fn unit_rect(&self, i: usize) -> Rect {
        let d = self.days_rect();
        let x = d.x + 10.0 + if i == 0 { 0.0 } else { 26.0 + 2.0 };
        Rect::new(x, d.bottom() - 6.0 - 20.0, 26.0, 20.0)
    }
    fn res_view(&self, list: Rect) -> Rect {
        list
    }
    fn res_row(&self, list: Rect, i: usize) -> Rect {
        Rect::new(list.x, list.y + i as f32 * 34.0 - self.res_scroll.offset(), list.w, 32.0)
    }
}

impl Default for WeatherView {
    fn default() -> Self {
        WeatherView::new()
    }
}

/// The PC's UTC offset in minutes (JS `-getTimezoneOffset()`).
fn local_tz_min() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut z = TIME_ZONE_INFORMATION::default();
    let id = unsafe { GetTimeZoneInformation(&mut z) };
    let extra = match id {
        2 => z.DaylightBias,
        _ => z.StandardBias,
    };
    -(z.Bias + extra) as i64
}

fn hex(c: u32) -> Color {
    Color::rgb8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// The sky gradient of `.wx-g-<icon>` as stops over 180deg.
fn sky(icon: &str) -> Vec<(f32, Color)> {
    let s = |v: &[(f32, u32)]| v.iter().map(|&(p, c)| (p, hex(c))).collect::<Vec<_>>();
    match icon {
        "clear-day" => s(&[(0.0, 0x2f78c4), (0.55, 0x4684b9), (1.0, 0x7d7a67)]),
        "partly-day" => s(&[(0.0, 0x3a73b4), (0.55, 0x4b78a3), (1.0, 0x6f7480)]),
        "clear-night" => s(&[(0.0, 0x101a38), (0.6, 0x141b30), (1.0, 0x161a26)]),
        "partly-night" => s(&[(0.0, 0x151d36), (0.6, 0x171c2e), (1.0, 0x181b27)]),
        "cloud" => s(&[(0.0, 0x1c2133), (1.0, 0x181c2b)]),
        "fog" => s(&[(0.0, 0x2a2f3a), (1.0, 0x22262f)]),
        "rain" => s(&[(0.0, 0x1a2433), (1.0, 0x161d29)]),
        "sleet" => s(&[(0.0, 0x1e2735), (1.0, 0x181f2b)]),
        "snow" => s(&[(0.0, 0x27303f), (1.0, 0x1d2330)]),
        "thunder" => s(&[(0.0, 0x1f1d33), (1.0, 0x171627)]),
        _ => s(&[(0.0, 0x161a28), (1.0, 0x161a28)]),
    }
}

fn txt(cv: &Canvas, r: Rect, s: &str, st: Style, c: Color) {
    if !s.is_empty() {
        cv.text(r, s, st, c);
    }
}

/// Draw `f` under an animation sample (opacity, offset, scale about `c`).
fn animated(cv: &Canvas, s: super::play::Sample, c: Pt, f: impl FnOnce(&Canvas)) {
    if !s.active {
        return f(cv);
    }
    cv.with_opacity(s.alpha, |cv| cv.with_offset(s.dx, s.dy, |cv| cv.with_scale_about(c, s.scale, f)));
}

fn rotated(cv: &Canvas, c: Pt, deg: f32, f: impl FnOnce(&Canvas)) {
    if deg == 0.0 {
        return f(cv);
    }
    let (s, k) = deg.to_radians().sin_cos();
    let m = windows::Foundation::Numerics::Matrix3x2 { M11: k, M12: s, M21: -s, M22: k, M31: c.0 - c.0 * k + c.1 * s, M32: c.1 - c.0 * s - c.1 * k };
    cv.with_transform(m, f);
}

impl WeatherView {
    // ------------------------------------------------------------------------------------------------------------ drawing

    fn draw_ok(&self, cv: &Canvas, th: &Theme, v: &OkView) {
        let o = self.o();
        let left = Rect::new(o.x, o.y, LEFT_W, o.h);
        // sky: the new gradient, the previous one fading out on top of it
        let day = matches!(v.gradient.as_str(), "clear-day" | "partly-day");
        draw::linear_deg(cv, left, 16.0, 180.0, &sky(&v.gradient));
        if let Some(g) = &v.ghost_gradient {
            let s = self.base.player.sample("gradient");
            if s.active {
                let a = 1.0 - s.p.clamp(0.0, 1.0);
                let st: Vec<_> = sky(g).into_iter().map(|(p, c)| (p, c.fade(a))).collect();
                draw::linear_deg(cv, left, 16.0, 180.0, &st);
            }
        }
        let (lx, top) = (left.x + 14.0, left.y + 16.0);
        // header: city button + refresh
        let cr = self.city_rect();
        let tw = cv.text_width(&v.city, Style::new(13.0, 600)).ceil();
        let cw = (14.0 + 6.0 + tw + 4.0).min(150.0);
        let press = self.city_it.press_scale(0.96);
        cv.with_scale_about((cr.x + cw / 2.0, cr.cy()), press, |cv| {
            cv.with_opacity(1.0 - 0.2 * self.city_it.hover(), |cv| {
                icons::draw_markup(cv, "wx-pin", PIN, Rect::new(lx, top + 5.0, 14.0, 14.0), WHITE, 1.7);
                txt(cv, Rect::new(lx + 20.0, top, cw - 24.0, 24.0), &v.city, Style::new(13.0, 600).ellipsis(), WHITE);
            })
        });
        let rf = self.rf_rect();
        let spin = self.base.player.sample("refresh");
        let hov = self.rf_it.hover();
        cv.with_scale_about(rf.center(), self.rf_it.press_scale(0.92), |cv| {
            cv.fill_circle(rf.center(), 12.0, WHITE.alpha(0.12 * hov));
            rotated(cv, rf.center(), spin.rot, |cv| icons::draw_markup(cv, "wx-refresh", REFRESH, rf.centered(14.0, 14.0), WHITE.alpha(0.75 + 0.25 * hov), 1.7));
        });
        // big temperature
        let by = top + 32.0;
        wxicons::draw(cv, &v.icon, Rect::new(lx, by + 3.0, 50.0, 50.0));
        txt(cv, Rect::new(lx + 60.0, by, 130.0, 56.0), &v.temp, Style::new(56.0, 600).track(-1.12), WHITE);
        txt(cv, Rect::new(lx, by + 68.0, 186.0, 20.0), &v.condition, Style::new(14.0, 600).ellipsis(), WHITE);
        txt(cv, Rect::new(lx, by + 92.0, 186.0, 16.0), &v.feels_line, Style::new(11.0, 400).ellipsis(), WHITE.alpha(0.72));
        // stats + sun, pinned to the bottom (14 px padding)
        let sun_y = left.bottom() - 14.0 - 15.0;
        let sy = sun_y - 12.0 - 50.0;
        for (i, (st, w)) in v.stats.iter().zip([78.0, 48.0, 48.0]).enumerate() {
            let x = lx + [0.0, 84.0, 138.0][i];
            cv.fill_rr(Rect::new(x, sy, w, 50.0), 10.0, if day { Color::rgba(40.0 / 255.0, 50.0 / 255.0, 60.0 / 255.0, 0.38) } else { WHITE.alpha(0.09) });
            icons::draw_markup(cv, stat_key(st.icon), stat_markup(st.icon), Rect::new(x + 8.0, sy + 8.0, 14.0, 14.0), WHITE.alpha(0.6), 1.7);
            txt(cv, Rect::new(x + 8.0, sy + 28.0, w - 16.0, 14.0), &st.text, Style::new(11.0, 600).mono().ellipsis(), WHITE);
        }
        let sc = WHITE.alpha(0.78);
        let st = Style::new(11.0, 400).mono();
        match (&v.sun_rise, &v.sun_set, &v.sun_text) {
            (Some(a), b, _) => {
                icons::draw_markup(cv, "wx-sunrise", SUNRISE, Rect::new(lx + 8.0, sun_y, 14.0, 14.0), sc, 1.7);
                txt(cv, Rect::new(lx + 28.0, sun_y - 1.0, 60.0, 16.0), a, st, sc);
                if let Some(b) = b {
                    icons::draw_markup(cv, "wx-sunset", SUNSET, Rect::new(lx + 92.0, sun_y, 14.0, 14.0), sc, 1.7);
                    txt(cv, Rect::new(lx + 112.0, sun_y - 1.0, 70.0, 16.0), b, st, sc);
                }
            }
            (_, _, Some(t)) => txt(cv, Rect::new(lx + 8.0, sun_y - 1.0, 170.0, 16.0), t, st, sc),
            _ => {}
        }

        // hourly strip
        let hours = Rect::new(o.x + LEFT_W + GAP, o.y, o.w - LEFT_W - GAP, 94.0);
        cv.fill_rr(hours, 16.0, th.card);
        for (i, h) in v.hours.iter().take(8).enumerate() {
            self.draw_hour(cv, th, hours, i, h);
        }
        // daily list
        let days = self.days_rect();
        cv.fill_rr(days, 16.0, th.card);
        for (i, d) in v.days.iter().take(7).enumerate() {
            let r = Rect::new(days.x + 16.0, days.y + 8.0 + i as f32 * 29.0, days.w - 32.0, 29.0);
            animated(cv, self.base.player.sample(&format!("day:{i}")), r.center(), |cv| self.draw_day(cv, th, r, d));
        }
        for (i, (label, on)) in [("°C", v.metric), ("°F", !v.metric)].into_iter().enumerate() {
            let r = self.unit_rect(i);
            let hv = self.unit_it[i].hover();
            let b = Rect::new(r.x, r.cy() - 8.5, r.w, 17.0);
            if on {
                cv.fill_rr(b, 6.0, th.raised);
            }
            let c = if on { th.text2 } else { th.faint.mix(th.text, hv) };
            cv.with_scale_about(b.center(), self.unit_it[i].press_scale(0.94), |cv| txt(cv, b, label, Style::new(11.0, 400).center(), c));
        }
        let src = Rect::new(days.x + 120.0, days.bottom() - 26.0, days.w - 136.0, 20.0);
        txt(cv, src, &v.source, Style::new(11.0, 400).right().ellipsis(), th.dim);
    }

    fn draw_hour(&self, cv: &Canvas, th: &Theme, card: Rect, i: usize, h: &HourView) {
        let x = card.x + 11.0 + i as f32 * 50.0;
        let y0 = card.y + 12.0;
        let col = Rect::new(x, y0, 50.0, 80.0);
        animated(cv, self.base.player.sample(&format!("hour:{i}")), col.center(), |cv| {
            let c = |dy: f32, hh: f32| Rect::new(x, y0 + dy, 50.0, hh);
            txt(cv, c(0.0, 14.0), &h.label, Style::new(11.0, 400).mono().center(), th.dim);
            wxicons::draw(cv, &h.icon, Rect::new(x + 13.0, y0 + 18.0, 24.0, 24.0));
            txt(cv, c(46.0, 16.0), &h.temp, Style::new(12.0, 600).mono().center(), th.text);
            txt(cv, c(64.0, 12.0), &h.precip, Style::new(10.0, 400).mono().center(), BLUE);
        });
    }

    fn draw_day(&self, cv: &Canvas, th: &Theme, r: Rect, d: &DayView) {
                let x = r.x;
        let name_c = if d.today { th.text } else if th.dark { hex(0xd4d4d4) } else { th.text2 };
        txt(cv, Rect::new(r.x, r.y, 58.0, r.h), &d.name, Style::new(12.0, 600), name_c);
        wxicons::draw(cv, &d.icon, Rect::new(x + 58.0 + 2.0, r.cy() - 11.0, 22.0, 22.0));
        txt(cv, Rect::new(x + 84.0, r.y, 20.0, r.h), &d.lo, Style::new(12.0, 400).mono().right(), th.dim);
        let bw = r.w - 58.0 - 26.0 - 30.0 - 34.0 - 54.0;
        let bar_r = Rect::new(x + 114.0, r.cy() - 2.0, bw, 4.0);
        cv.fill_rr(bar_r, 2.0, if th.dark { WHITE.alpha(0.1) } else { Color::rgba(0.0, 0.0, 0.0, 0.1) });
        let f = Rect::new(bar_r.x + bar_r.w * d.bar_left as f32 / 100.0, bar_r.y, bar_r.w * d.bar_width as f32 / 100.0, 4.0);
        let px = |c: [u8; 3]| Color::rgb8(c[0], c[1], c[2]);
        draw::linear_deg(cv, f, 2.0, 90.0, &[(0.0, px(d.color_lo)), (1.0, px(d.color_hi))]);
        txt(cv, Rect::new(bar_r.right() + 10.0, r.y, 24.0, r.h), &d.hi, Style::new(12.0, 600).mono(), th.text);
        txt(cv, Rect::new(r.right() - 54.0, r.y, 54.0, r.h), &d.precip, Style::new(12.0, 400).mono().right(), BLUE);
    }

    /// The centred cards (off, city, error, loading): returns after drawing everything of the card.
    fn draw_card(&self, cv: &Canvas, th: &Theme) {
        let o = self.o();
        if let Vw::Loading { text } = &self.vw {
            txt(cv, o, text, Style::new(12.0, 400).center(), th.dim);
            return;
        }
        // the card column depends on the measured text height: measure first so the very first frame is laid out right
        let text0 = match &self.vw {
            Vw::Off(x) => x.text.clone(),
            Vw::City(x) => x.text.clone(),
            Vw::Error(x) => x.text.clone(),
            _ => String::new(),
        };
        if !text0.is_empty() {
            let (_, h) = cv.measure(&text0, Style::new(12.0, 400).wrap().center(), 460.0);
            self.text_h.set(h.ceil().max(18.0));
        }
        let rects = self.card();
        let (badge_icon, badge_key, title, text) = match &self.vw {
            Vw::Off(x) => (CLOUDSUN, "wx-cloudsun", x.title.clone(), x.text.clone()),
            Vw::City(x) => (PIN, "wx-pin", x.title.clone(), x.text.clone()),
            Vw::Error(x) => (CLOUDSUN, "wx-cloudsun", x.title.clone(), x.text.clone()),
            _ => return,
        };
        let s = self.base.player.sample("body");
        animated(cv, s, o.center(), |cv| {
            cv.fill_circle(rects[0].center(), 22.0, th.accent_soft);
            icons::draw_markup(cv, badge_key, badge_icon, rects[0].centered(21.0, 21.0), th.accent, 1.7);
            txt(cv, rects[1], &title, Style::new(16.0, 600).center(), th.text);
            let ts = Style::new(12.0, 400).wrap().center();
            if !text.is_empty() {
                let (_, h) = cv.measure(&text, ts, 460.0);
                self.text_h.set(h.ceil().max(18.0));
                cv.text(rects[2], &text, ts, th.dim);
            }
            match &self.vw {
                Vw::Off(_) => {
                    self.contact.input.draw(cv, rects[3], th);
                    self.draw_btn(cv, th, B_ENABLE, rects[4]);
                }
                Vw::City(c) => {
                    let (inp, go) = self.search_rects(rects[3]);
                    self.city.input.draw(cv, inp, th);
                    self.draw_btn(cv, th, B_SEARCH, go);
                    txt(cv, rects[4], &c.msg, Style::new(12.0, 400).center(), th.dim);
                    if !c.results.is_empty() {
                        let list = rects[5];
                        cv.with_clip(list, |cv| {
                            for (i, r) in c.results.iter().enumerate() {
                                let rr = self.res_row(list, i);
                                if rr.bottom() < list.y || rr.y > list.bottom() {
                                    continue;
                                }
                                let it = &self.res_it[i];
                                animated(cv, self.base.player.sample(&format!("res:{i}")), rr.center(), |cv| {
                                    cv.with_scale_about(rr.center(), it.press_scale(0.985), |cv| {
                                        cv.fill_rr(rr, 10.0, th.raised.fade(it.hover()));
                                        let nw = cv.text_width(&r.name, Style::new(13.0, 600)).ceil();
                                        txt(cv, Rect::new(rr.x + 12.0, rr.y + 8.0, nw + 2.0, 16.0), &r.name, Style::new(13.0, 600), th.text);
                                        txt(cv, Rect::new(rr.x + 12.0 + nw + 10.0, rr.y + 8.0, (rr.w - nw - 34.0).max(0.0), 16.0), &r.detail, Style::new(11.0, 400).ellipsis(), th.dim);
                                    })
                                });
                            }
                        });
                    }
                    if c.cancel.is_some() {
                        self.draw_btn(cv, th, B_CANCEL, rects[6]);
                    }
                }
                Vw::Error(e) => {
                    let row = *rects.last().unwrap();
                    let bs = self.err_buttons(row);
                    self.draw_btn(cv, th, B_RETRY, bs[0]);
                    self.draw_btn(cv, th, B_CITY, bs[1]);
                    self.draw_btn(cv, th, B_OFF, bs[2]);
                    let _ = e;
                }
                _ => {}
            }
        });
    }

    fn draw_btn(&self, cv: &Canvas, th: &Theme, i: usize, r: Rect) {
        self.btn[i].draw(cv, th, r);
    }
}

// the line icons of tab-weather.js (UI set), 24 x 24 grid
const PIN: &str = r#"<path d="M20 10c0 6-8 12-8 12s-8-6-8-12a8 8 0 0 1 16 0z"/><circle cx="12" cy="10" r="3"/>"#;
const REFRESH: &str = r#"<path d="M20 11a8 8 0 0 0-14.5-4M4 4v4h4M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4"/>"#;
const WIND: &str = r#"<path d="M3 8h10a3 3 0 1 0-3-3M3 12h15a3 3 0 1 1-3 3M3 16h7a2.5 2.5 0 1 1-2.5 2.5"/>"#;
const DROP: &str = r#"<path d="M12 3s6 6.2 6 10.5a6 6 0 0 1-12 0C6 9.200 12 3 12 3z"/><path d="M9.500 14a2.600 2.600 0 0 0 2.500 2.500"/>"#;
const UMBRELLA: &str = r#"<path d="M3 12a9 9 0 0 1 18 0z"/><path d="M12 12v6.500a2 2 0 0 0 4 0"/><path d="M12 3v1"/>"#;
const SUNRISE: &str = r#"<path d="M12 3v7M8.500 6.500 12 3l3.500 3.500M3 20h18M6 16a6 6 0 0 1 12 0"/>"#;
const SUNSET: &str = r#"<path d="M12 10V3M8.500 6.500 12 10l3.500-3.500M3 20h18M6 16a6 6 0 0 1 12 0"/>"#;
const CLOUDSUN: &str = r#"<path d="M12 2v2M4.900 4.900l1.400 1.400M20 12h2M19.100 4.900l-1.400 1.400M15.900 12.600a4 4 0 0 0-5.900-4.100"/><path d="M13 22H7a5 5 0 1 1 4.900-6H13a3 3 0 0 1 0 6z"/>"#;

fn stat_markup(name: &str) -> &'static str {
    match name {
        "wind" => WIND,
        "drop" => DROP,
        _ => UMBRELLA,
    }
}

fn stat_key(name: &str) -> &'static str {
    match name {
        "wind" => "wx-wind",
        "drop" => "wx-drop",
        _ => "wx-umbrella",
    }
}

impl View for WeatherView {
    fn size_hint(&self) -> Option<(f32, f32)> {
        Some((super::W, H))
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.ru = cx.lang == Lang::Ru;
        self.model.set_language_ru(self.ru);
        self.model.set_os_reduce_motion(cx.reduced_motion);
        self.base.show(cx, &["settings:changed"], 250);
        let calls = self.model.on_show();
        self.pump(cx, calls);
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.model.on_hide();
        self.base.hide();
        if self.kbd {
            self.kbd = false;
            cx.set_focus(false);
        }
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match self.base.classify(msg, cx) {
            Incoming::Event { name, payload } => {
                let calls = self.model.on_event(&name, &payload);
                self.pump(cx, calls);
            }
            Incoming::Result { cmd, args, result } => {
                // a rejected command is a `null` answer, like the JS wrapper
                let v = result.unwrap_or(Value::Null);
                let calls = self.model.on_result(cmd, &args, &v);
                self.pump(cx, calls);
            }
            Incoming::Tick(dt) => {
                self.model.set_os_reduce_motion(cx.reduced_motion);
                let calls = self.model.tick(dt);
                self.pump(cx, calls);
                if self.city.focused() || self.contact.focused() {
                    self.city.input.step((dt / 1000.0) as f32);
                    self.contact.input.step((dt / 1000.0) as f32);
                    cx.request_frame();
                }
            }
            Incoming::Image(_) | Incoming::Other(_) => return Response::NONE,
        }
        Response::redraw()
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let mut r = Response::NONE;
        let mut acts: Vec<Action> = Vec::new();
        let pos_in = |rect: Rect| ev.pos().map_or(false, |p| rect.contains(p.0, p.1));
        match self.kind {
            Kind::Ok => {
                let cr = self.city_rect();
                let (a, p) = self.city_it.pointer(ev, |q| cr.contains(q.0, q.1));
                r = r.merge(a);
                if p.click {
                    acts.push(Action::ChangeCity);
                }
                let rf = self.rf_rect();
                let (a, p) = self.rf_it.pointer(ev, |q| rf.contains(q.0, q.1));
                r = r.merge(a);
                if p.click {
                    acts.push(Action::Refresh);
                }
                for i in 0..2 {
                    let ur = self.unit_rect(i);
                    let (a, p) = self.unit_it[i].pointer(ev, |q| ur.contains(q.0, q.1));
                    r = r.merge(a);
                    if p.click {
                        acts.push(Action::SetUnits(if i == 0 { "metric" } else { "imperial" }));
                    }
                }
            }
            Kind::Off => {
                let rs = self.card();
                r = r.merge(self.contact.queue(ev, rs[3]));
                let b = self.btn[B_ENABLE].event(ev, rs[4]);
                r = r.merge(b);
                if b.changed {
                    acts.push(Action::Enable { contact: self.contact.text() });
                }
            }
            Kind::City => {
                let rs = self.card();
                let (inp, go) = self.search_rects(rs[3]);
                r = r.merge(self.city.queue(ev, inp));
                let b = self.btn[B_SEARCH].event(ev, go);
                r = r.merge(b);
                if b.changed {
                    acts.push(Action::Search { query: self.city.text() });
                }
                if let Vw::City(c) = &self.vw {
                    let list = rs[5];
                    let n = c.results.len();
                    if n > 0 {
                        self.res_scroll.set_content_height(n as f32 * 34.0 - 2.0, list.h);
                        if matches!(ev, InputEvent::Wheel { .. }) && pos_in(list) {
                            r = r.merge(self.res_scroll.event(ev, list));
                        }
                        for i in 0..n.min(self.res_it.len()) {
                            let rr = self.res_row(list, i);
                            let visible = rr.intersect_y(list);
                            let (a, p) = self.res_it[i].pointer(ev, |q| visible.map_or(false, |v| v.contains(q.0, q.1)));
                            r = r.merge(a);
                            if p.click {
                                acts.push(Action::PickCity(i));
                            }
                        }
                    }
                    if c.cancel.is_some() {
                        let b = self.btn[B_CANCEL].event(ev, rs[6]);
                        r = r.merge(b);
                        if b.changed {
                            acts.push(Action::CancelCity);
                        }
                    }
                }
            }
            Kind::Error => {
                let rs = self.card();
                let bs = self.err_buttons(*rs.last().unwrap());
                for (i, (b, act)) in [(B_RETRY, Action::Retry), (B_CITY, Action::ChangeCity), (B_OFF, Action::TurnOff)].into_iter().enumerate() {
                    let x = self.btn[b].event(ev, bs[i]);
                    r = r.merge(x);
                    if x.changed {
                        acts.push(act);
                    }
                }
            }
            Kind::Loading => {}
        }
        for a in acts {
            self.act(a, cx);
            r.redraw = true;
        }
        if r.cursor == Some(Cursor::Arrow) {
            r.cursor = None;
        }
        r
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.base.player.step(dt_ms);
        busy |= self.city_it.step(dt) | self.rf_it.step(dt) | self.unit_it[0].step(dt) | self.unit_it[1].step(dt);
        for it in &mut self.res_it {
            busy |= it.step(dt);
        }
        for b in &mut self.btn {
            busy |= b.step(dt);
        }
        busy |= self.res_scroll.step(dt);
        busy |= self.contact.input.step(dt) & self.contact.focused();
        busy |= self.city.input.step(dt) & self.city.focused();
        // typed input is applied in `draw`; its events (Enter = search) are read here, on the frame after
        for e in self.city.take_events() {
            if e == EditEvent::Submit {
                let q = self.city.text();
                self.act(Action::Search { query: q }, cx);
            }
        }
        let _ = self.contact.take_events();
        busy | self.city.pending() | self.contact.pending()
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.base.rect = rect;
        self.contact.flush(cv);
        self.city.flush(cv);
        let st = th.btn_text();
        let mut w = self.bw.get();
        for (i, b) in self.btn.iter().enumerate() {
            if !b.label.is_empty() {
                let s = if i == B_ENABLE || i == B_SEARCH { st.weight(600) } else { st };
                w[i] = (cv.text_width(&b.label, s).ceil() + 2.0 * th.m.btn_pad_x).max(if i == B_ENABLE { 110.0 } else { 0.0 });
            }
        }
        self.bw.set(w);
        let _ = cx;
        match &self.vw {
            Vw::Ok(v) => self.draw_ok(cv, th, v),
            _ => self.draw_card(cv, th),
        }
    }
}

trait RectExt {
    fn intersect_y(&self, clip: Rect) -> Option<Rect>;
}
impl RectExt for Rect {
    /// The part of the row inside the list box (rows scroll under its edges).
    fn intersect_y(&self, clip: Rect) -> Option<Rect> {
        let (t, b) = (self.y.max(clip.y), self.bottom().min(clip.bottom()));
        (b > t).then(|| Rect::new(self.x, t, self.w, b - t))
    }
}
