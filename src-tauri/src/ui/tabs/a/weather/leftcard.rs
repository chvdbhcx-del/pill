//! The left sky card of the Weather tab, alive: two blurred colour blobs drifting behind the content (colours follow the scene), a faint
//! precipitation layer, a lightning flash that brightens the card, the animated icon (`anicon`) that leans a few px towards the pointer, the
//! temperature counting to its value, the texts and stat tiles rising in one after the other, and the refresh icon turning while it loads.
//!
//! Cost: it moves only while the tab is live (`on_show` .. `on_hide`) and Reduce motion is off. Its frames are reported to the host as
//! ambient (`Cx::ambient_frame`) when nothing else moves, so the island paces them at ~30 fps and idles once the tab is closed.
use super::sky::{particle, wave, Counter, Lightning, Rise, Scene, Spin};
use super::{anicon, draw, icons, rotated, stat_key, stat_markup, txt, wxicons, WeatherView, REFRESH, SUNRISE, SUNSET, PIN, WHITE};
use crate::ui::liquid::{Feel, Flow, Flow2};
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::{Pt, Rect};
use glasskit::input::InputEvent;
use glasskit::text::Style;
use pill_ui_core::panel::a::weather::OkView;

/// How far the icon leans towards the pointer, px.
const LEAN: f32 = 3.5;
/// Longest frame step the clock accepts (a hitch must not make the rain jump a whole screen).
const MAX_DT_MS: f64 = 100.0;

pub struct Left {
    t: f64,
    live: bool,
    scene: Scene,
    seen: bool,
    level: u8,
    /// the scene the blobs come from while they cross-fade (`blend` 0 = all `from`, 1 = all `scene`)
    from: Scene,
    blend: Flow,
    counter: Counter,
    rise: Rise,
    spin: Spin,
    lean: Flow2,
    lean_to: Pt,
    light: Lightning,
    flash: f32,
}

impl Left {
    pub fn new() -> Left {
        Left {
            t: 0.0,
            live: false,
            scene: Scene::Cloudy,
            seen: false,
            level: 1,
            from: Scene::Cloudy,
            blend: Flow::new(1.0, Feel::FADE.within(900.0)),
            counter: Counter::new(),
            rise: Rise::new(),
            spin: Spin::default(),
            lean: Flow2::px((0.0, 0.0), Feel::DRAG),
            lean_to: (0.0, 0.0),
            light: Lightning::new(0x5EED_1DEA),
            flash: 0.0,
        }
    }

    /// The tab is on screen (true) or gone (false): nothing moves while it is gone.
    pub fn set_live(&mut self, live: bool) {
        self.live = live;
        if !live {
            self.lean_to = (0.0, 0.0);
            self.lean.jump((0.0, 0.0));
            self.flash = 0.0;
        }
    }

    /// The card appears (the tab opened, the forecast arrived): the number counts up again and the texts rise again.
    pub fn rewind(&mut self, reduced: bool) {
        if reduced {
            return;
        }
        self.counter.rewind();
        self.rise.rewind();
    }

    /// The pointer over the card, `None` when it is elsewhere: the icon leans towards it.
    pub fn point(&mut self, p: Option<Pt>, card: Rect, reduced: bool) {
        self.lean_to = match p {
            Some(p) if !reduced && card.contains(p.0, p.1) => lean_for(p, card),
            _ => (0.0, 0.0),
        };
    }

    pub fn event(&mut self, ev: &InputEvent, card: Rect, reduced: bool) {
        match ev {
            InputEvent::MouseMove { pos, .. } => self.point(Some(*pos), card, reduced),
            InputEvent::MouseLeave => self.point(None, card, reduced),
            _ => {}
        }
    }

    /// One frame. Returns (something moves for real, only the ambient loop is left).
    pub fn step(&mut self, dt_ms: f64, v: Option<&OkView>, reduced: bool) -> (bool, bool) {
        let Some(v) = v else { return (false, false) };
        let dt = dt_ms.min(MAX_DT_MS);
        let sc = Scene::of(&v.icon);
        if !self.seen {
            self.seen = true;
            self.scene = sc;
            self.from = sc;
            self.blend.jump(1.0);
        } else if sc != self.scene {
            self.from = self.scene;
            self.scene = sc;
            self.blend.jump(0.0);
        }
        self.level = v.intensity;
        self.blend.to(1.0);
        self.blend.step(dt);
        self.counter.set(&v.temp);
        let mut real = self.blend.moving() | self.counter.step(dt) | self.rise.step(dt);
        real |= self.spin.step(v.spinning && !reduced, (dt / 1000.0) as f32);
        self.lean.chase(self.lean_to, dt);
        real |= self.lean.moving();
        let ambient = self.live && !reduced;
        if ambient {
            self.t += dt / 1000.0;
            self.flash = if self.scene == Scene::Thunder { self.light.step((dt / 1000.0) as f32) } else { 0.0 };
        } else {
            self.flash = 0.0;
        }
        (real, ambient)
    }

    #[cfg(test)]
    pub fn clock(&self) -> f64 {
        self.t
    }
    #[cfg(test)]
    pub fn flash(&self) -> f32 {
        self.flash
    }
    #[cfg(test)]
    pub fn lean_at(&self) -> Pt {
        self.lean.at()
    }
    #[cfg(test)]
    pub fn count_text(&self) -> &str {
        self.counter.text()
    }
    #[cfg(test)]
    pub fn scene(&self) -> Scene {
        self.scene
    }
    #[cfg(test)]
    pub fn set_clock(&mut self, t: f64) {
        self.t = t;
    }
    #[cfg(test)]
    pub fn force_flash(&mut self, f: f32) {
        self.flash = f;
    }
}

/// The icon's lean for a pointer at `p` over `card`: -LEAN..LEAN on each axis, zero at the centre.
pub fn lean_for(p: Pt, card: Rect) -> Pt {
    let nx = ((p.0 - card.cx()) / (card.w / 2.0)).clamp(-1.0, 1.0);
    let ny = ((p.1 - card.cy()) / (card.h / 2.0)).clamp(-1.0, 1.0);
    (nx * LEAN, ny * LEAN)
}

/// Colours of the two blobs behind the card and how strong they are.
pub fn blob_colors(s: Scene) -> [(Color, f32); 2] {
    let c = |r: u8, g: u8, b: u8| Color::rgb8(r, g, b);
    match s {
        Scene::ClearDay => [(c(255, 200, 90), 0.26), (c(120, 190, 255), 0.22)],
        Scene::PartlyDay => [(c(255, 210, 120), 0.20), (c(140, 190, 255), 0.22)],
        Scene::ClearNight => [(c(90, 100, 230), 0.28), (c(160, 110, 235), 0.20)],
        Scene::PartlyNight => [(c(95, 110, 220), 0.24), (c(120, 140, 200), 0.18)],
        Scene::Cloudy => [(c(150, 165, 195), 0.18), (c(105, 120, 150), 0.20)],
        Scene::Fog => [(c(195, 205, 220), 0.16), (c(150, 160, 178), 0.16)],
        Scene::Rain => [(c(90, 150, 225), 0.22), (c(70, 100, 165), 0.22)],
        Scene::Sleet => [(c(140, 180, 225), 0.20), (c(120, 130, 175), 0.20)],
        Scene::Snow => [(c(205, 225, 255), 0.20), (c(150, 180, 235), 0.20)],
        Scene::Thunder => [(c(160, 100, 235), 0.26), (c(80, 115, 205), 0.22)],
    }
}

impl WeatherView {
    pub(super) fn left_event(&mut self, ev: &InputEvent) {
        let card = Rect::new(self.o().x, self.o().y, super::LEFT_W, self.o().h);
        let reduced = self.base.reduced;
        self.left.event(ev, card, reduced);
    }

    /// Drifting blobs of one scene, `k` = its share of the picture.
    fn blobs(&self, cv: &Canvas, left: Rect, s: Scene, k: f32) {
        if k < 0.01 {
            return;
        }
        let t = self.left.t;
        let (w, h) = (left.w, left.h);
        let wob = |p1: f64, p2: f64, ph: f32| (wave(t, p1, ph), wave(t, p2, ph + 0.25));
        let (a0, a1) = wob(23.0, 29.0, 0.0);
        let (b0, b1) = wob(31.0, 37.0, 0.4);
        let cols = blob_colors(s);
        let centres = [(left.x + w * 0.30 + 46.0 * a0, left.y + h * 0.26 + 34.0 * a1), (left.x + w * 0.74 + 40.0 * b0, left.y + h * 0.72 + 46.0 * b1)];
        for ((col, a), c) in cols.into_iter().zip(centres) {
            draw::radial(cv, left, 16.0, c, w * 0.95, w * 0.95, &[(0.0, col.alpha(a * k)), (1.0, col.alpha(0.0))]);
        }
    }

    /// The faint rain / snow behind the texts.
    fn precip(&self, cv: &Canvas, left: Rect) {
        let Some((kind, n)) = self.left.scene.precip(self.left.level) else { return };
        let (rate, t) = (self.left.scene.fall_rate(self.left.level) * 0.7, self.left.t);
        for i in 0..n {
            let p = particle(i, t, kind, rate, left.w, left.h, -0.05, 7.0);
            let c = (left.x + p.x, left.y + p.y);
            if p.flake {
                cv.fill_circle(c, 1.0 + 1.1 * super::sky::h01(i, 6), Color::rgba(1.0, 1.0, 1.0, 0.42 * p.a));
            } else {
                let len = 9.0 + 6.0 * super::sky::h01(i, 7);
                cv.line((c.0 + len * 0.25, (c.1 - len).max(left.y)), c, 1.1, Color::rgb8(0xb8, 0xd4, 0xff).alpha(0.30 * p.a));
            }
        }
    }

    /// The living background: sky gradient (drawn by the caller), the blobs, the weather, the flash.
    fn sky_life(&self, cv: &Canvas, left: Rect) {
        if !self.left.live || self.base.reduced {
            return;
        }
        let l = &self.left;
        self.blobs(cv, left, l.scene, l.blend.at01());
        if l.blend.at01() < 0.99 {
            self.blobs(cv, left, l.from, 1.0 - l.blend.at01());
        }
        self.precip(cv, left);
        if l.flash > 0.01 {
            cv.fill_rr(left, 16.0, WHITE.alpha(0.10 * l.flash));
        }
    }

    /// Draws `f` risen `i`-th of the staggered entrance.
    fn risen(&self, cv: &Canvas, i: usize, f: impl FnOnce(&Canvas)) {
        let (a, dy) = self.left.rise.at(i);
        if a >= 0.999 && dy.abs() < 0.05 {
            return f(cv);
        }
        cv.with_opacity(a, |cv| cv.with_offset(0.0, dy, f));
    }

    pub(super) fn draw_left(&self, cv: &Canvas, v: &OkView) {
        let o = self.o();
        let left = Rect::new(o.x, o.y, super::LEFT_W, o.h);
        // sky: the new gradient, the previous one fading out on top of it
        let day = matches!(v.gradient.as_str(), "clear-day" | "partly-day");
        draw::linear_deg(cv, left, 16.0, 180.0, &super::sky(&v.gradient));
        if let Some(g) = &v.ghost_gradient {
            let s = self.base.player.sample("gradient");
            if s.active {
                let a = 1.0 - s.p.clamp(0.0, 1.0);
                let st: Vec<_> = super::sky(g).into_iter().map(|(p, c)| (p, c.fade(a))).collect();
                draw::linear_deg(cv, left, 16.0, 180.0, &st);
            }
        }
        self.sky_life(cv, left);
        let (lx, top) = (left.x + 14.0, left.y + 16.0);
        // header: city button + refresh
        let cr = self.city_rect();
        let tw = cv.text_width(&v.city, Style::new(13.0, 600)).ceil();
        let cw = (14.0 + 6.0 + tw + 4.0).min(150.0);
        let press = self.city_it.press_scale(0.96);
        let rf = self.rf_rect();
        let hov = self.rf_it.hover();
        self.risen(cv, 0, |cv| {
            cv.with_scale_about((cr.x + cw / 2.0, cr.cy()), press, |cv| {
                cv.with_opacity(1.0 - 0.2 * self.city_it.hover(), |cv| {
                    icons::draw_markup(cv, "wx-pin", PIN, Rect::new(lx, top + 5.0, 14.0, 14.0), WHITE, 1.7);
                    txt(cv, Rect::new(lx + 20.0, top, cw - 24.0, 24.0), &v.city, Style::new(13.0, 600).ellipsis(), WHITE);
                })
            });
            cv.with_scale_about(rf.center(), self.rf_it.press_scale(0.92), |cv| {
                cv.fill_circle(rf.center(), 12.0, WHITE.alpha(0.12 * hov));
                let spin = if self.left.spin.deg != 0.0 { self.left.spin.deg } else { self.base.player.sample("refresh").rot };
                rotated(cv, rf.center(), spin, |cv| icons::draw_markup(cv, "wx-refresh", REFRESH, rf.centered(14.0, 14.0), WHITE.alpha(0.75 + 0.25 * hov), 1.7));
            });
        });
        // big icon and temperature
        let by = top + 32.0;
        let icon_r = Rect::new(lx, by + 3.0, 50.0, 50.0);
        let (lean, live) = (self.left.lean.at(), self.left.live && !self.base.reduced);
        self.risen(cv, 1, |cv| {
            if live {
                let pose = anicon::Pose { t: self.left.t, level: self.left.level, flash: self.left.flash };
                cv.with_offset(lean.0, lean.1, |cv| anicon::draw(cv, self.left.scene, icon_r, &pose));
            } else {
                wxicons::draw(cv, &v.icon, icon_r);
            }
        });
        let shown = match self.left.counter.text() {
            "" => v.temp.as_str(),
            t => t,
        };
        self.risen(cv, 2, |cv| txt(cv, Rect::new(lx + 60.0, by, 130.0, 56.0), shown, Style::new(56.0, 600).track(-1.12), WHITE));
        self.risen(cv, 3, |cv| txt(cv, Rect::new(lx, by + 68.0, 186.0, 20.0), &v.condition, Style::new(14.0, 600).ellipsis(), WHITE));
        self.risen(cv, 4, |cv| txt(cv, Rect::new(lx, by + 92.0, 186.0, 16.0), &v.feels_line, Style::new(11.0, 400).ellipsis(), WHITE.alpha(0.72)));
        // stats + sun, pinned to the bottom (14 px padding)
        let sun_y = left.bottom() - 14.0 - 15.0;
        let sy = sun_y - 12.0 - 50.0;
        for (i, (st, w)) in v.stats.iter().zip([78.0, 48.0, 48.0]).enumerate() {
            let x = lx + [0.0, 84.0, 138.0][i];
            self.risen(cv, 5 + i, |cv| {
                cv.fill_rr(Rect::new(x, sy, w, 50.0), 10.0, if day { Color::rgba(40.0 / 255.0, 50.0 / 255.0, 60.0 / 255.0, 0.38) } else { WHITE.alpha(0.09) });
                icons::draw_markup(cv, stat_key(st.icon), stat_markup(st.icon), Rect::new(x + 8.0, sy + 8.0, 14.0, 14.0), WHITE.alpha(0.6), 1.7);
                txt(cv, Rect::new(x + 8.0, sy + 28.0, w - 16.0, 14.0), &st.text, Style::new(11.0, 600).mono().ellipsis(), WHITE);
            });
        }
        let sc = WHITE.alpha(0.78);
        let st = Style::new(11.0, 400).mono();
        self.risen(cv, 8, |cv| match (&v.sun_rise, &v.sun_set, &v.sun_text) {
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
        });
    }
}
