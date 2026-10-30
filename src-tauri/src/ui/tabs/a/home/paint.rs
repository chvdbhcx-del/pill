//! Drawing of the Home tab. Pure reads of `HomeView` state computed in `step` / `refresh`; nothing here mutates or allocates beyond strings of text.
use super::{css, layout as L, Hit, HomeView};
use crate::ui::tabs::a::{draw, icons};
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;

const NEUTRAL_ACC: Color = Color::rgb8(0x8b, 0x8b, 0x8b);

fn rotation(c: (f32, f32), deg: f32) -> Matrix3x2 {
    let (s, k) = deg.to_radians().sin_cos();
    Matrix3x2 { M11: k, M12: s, M21: -s, M22: k, M31: c.0 - c.0 * k + c.1 * s, M32: c.1 - c.0 * s - c.1 * k }
}

/// A dashed 1 px rounded outline (`border: 1px dashed`): Chrome draws 3 px dashes with 3 px gaps.
fn dashed_rr(cv: &Canvas, r: Rect, rad: f32, color: Color) {
    unsafe {
        let Ok(factory) = cv.dc.GetFactory() else { return };
        let props = D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_FLAT, endCap: D2D1_CAP_STYLE_FLAT, dashCap: D2D1_CAP_STYLE_FLAT, lineJoin: D2D1_LINE_JOIN_MITER, miterLimit: 10.0, dashStyle: D2D1_DASH_STYLE_CUSTOM, dashOffset: 0.0 };
        let Ok(style) = factory.CreateStrokeStyle(&props, Some(&[3.0, 3.0])) else { return };
        let b = cv.res.brush();
        b.SetColor(&color.d2d());
        let i = r.inset(0.5);
        let rr = D2D1_ROUNDED_RECT { rect: glasskit::gfx::rf(i.x, i.y, i.right(), i.bottom()), radiusX: rad - 0.5, radiusY: rad - 0.5 };
        cv.dc.DrawRoundedRectangle(&rr, b, 1.0, &style);
    }
}

/// The 4 px slider track with its hard-stop fill (`linear-gradient(to right, fill p%, track p%)`) on a rounded body.
fn track(cv: &Canvas, r: Rect, pct: f32, fill: Color, bg: Color) {
    let t = Rect::new(r.x, r.cy() - 2.0, r.w, 4.0);
    cv.fill_rr(t, 2.0, bg);
    let w = (t.w * pct.clamp(0.0, 1.0)).max(0.0);
    if w > 0.0 {
        cv.with_clip_rr(t, 2.0, |cv| cv.fill_rect(Rect::new(t.x, t.y, w, t.h), fill));
    }
}

/// The 12 px thumb, centred on the thumb travel (6 px inset at each end).
fn thumb(cv: &Canvas, r: Rect, pct: f32, alpha: f32, scale: f32) {
    if alpha <= 0.01 {
        return;
    }
    let c = (r.x + 6.0 + pct.clamp(0.0, 1.0) * (r.w - 12.0), r.cy());
    cv.with_opacity(alpha, |cv| {
        cv.fill_circle(c, 6.5 * scale, Color::rgba(0.0, 0.0, 0.0, 0.25));
        cv.fill_circle(c, 6.0 * scale, Color::rgb8(255, 255, 255));
    });
}

impl HomeView {
    pub(super) fn paint(&self, cv: &Canvas, th: &Theme, ox: f32, oy: f32) {
        let o = |r: Rect| r.offset(ox, oy);
        self.paint_player(cv, th, ox, oy);
        self.paint_volume(cv, th, ox, oy);
        for (i, t) in self.v.tiles.iter().enumerate() {
            self.paint_tile(cv, th, o(L::tile(i)), Hit::Tile(i), t);
        }
        self.paint_launcher(cv, th, ox, oy);
        self.paint_menu(cv, th, ox, oy);
    }

    fn hov(&self, h: Hit) -> f32 {
        self.hover_amount(h)
    }

    fn press(&self, h: Hit, depth: f32) -> f32 {
        self.hov.get(&h).map_or(1.0, |i| i.press_scale(depth))
    }

    fn acc(&self) -> Color {
        css::parse(&self.v.player.accent).unwrap_or(NEUTRAL_ACC)
    }

    // ------------------------------------------------------------------------------------------------------------------ player

    fn paint_player(&self, cv: &Canvas, th: &Theme, ox: f32, oy: f32) {
        let o = |r: Rect| r.offset(ox, oy);
        let card = o(L::PLAYER);
        let p = &self.v.player;
        let rad = th.m.radius;
        // background: the cover gradient, else a tinted corner glow of the accent
        if let (Some((a, b)), false) = (self.cov_tint(), p.none) {
            draw::linear_deg(cv, card, rad, 100.0, &[(0.0, css::mix(a, 30.0, th.card)), (0.28, css::mix(b, 14.0, th.card)), (0.52, th.card), (1.0, th.card)]);
        } else {
            let base = if th.dark { Color::rgb8(0x1b, 0x1b, 0x1b) } else { Color::rgb8(255, 255, 255) };
            let mixed = css::mix(self.acc(), if th.dark { 17.0 } else { 22.0 }, base);
            cv.fill_rr(card, rad, th.card);
            draw::radial(cv, card, rad, (card.x, card.y), card.w * 1.3, card.h * 1.5, &[(0.0, mixed), (0.72, th.card), (1.0, th.card)]);
        }
        cv.with_clip_rr(card, rad, |cv| {
            if p.none {
                let c = card.center();
                icons::draw(cv, "note2", Rect::new(c.0 - 13.0, c.1 - 26.0, 26.0, 26.0), th.dim, 1.7);
                cv.text(Rect::new(card.x, c.1 + 8.0, card.w, 18.0), &p.empty_text, Style::new(13.0, 400).center().ellipsis(), th.dim);
                return;
            }
            self.paint_art(cv, th, o(L::ART));
            // source label (a button when several players run)
            let src_c = th.dim.mix(th.text, if p.source_multi { self.hov(Hit::Src) } else { 0.0 });
            cv.text(o(L::SRC), &p.source.to_uppercase(), Style::new(10.0, 600).track(0.6).ellipsis(), src_c);
            let swap = |key: &str, r: Rect, s: &str, st: Style, c: Color| {
                let sm = self.base.player.sample(key);
                cv.with_opacity(sm.alpha, |cv| cv.with_offset(sm.dx, sm.dy, |cv| cv.text(r, s, st, c)));
            };
            swap("title", o(L::TITLE), &p.title, Style::new(16.0, 600).ellipsis(), th.text);
            swap("artist", o(L::ARTIST), &p.artist_line, Style::new(13.0, 400).ellipsis(), th.text2);
            // seek
            let seek = o(L::SEEK);
            let track_c = if th.dark { Color::rgb8(0x35, 0x35, 0x35) } else { Color::rgb8(0xc9, 0xc9, 0xc9) };
            track(cv, seek, p.seek.fill_pct as f32 / 100.0, self.acc(), track_c);
            let small = Style::new(11.0, 400).mono();
            cv.text(o(L::T0), &p.seek.elapsed, small, th.dim);
            cv.text(o(L::T1), &p.seek.remaining, small.right(), th.dim);
            // transport
            for (h, r, glyph, size) in [(Hit::Prev, L::PREV, "prev", 18.0), (Hit::Next, L::NEXT, "next", 18.0)] {
                let r = o(r);
                let k = self.hov(h);
                cv.with_scale_about(r.center(), self.press(h, 0.94), |cv| {
                    cv.fill_circle(r.center(), 14.0, th.hover.fade(k));
                    icons::draw(cv, glyph, r.centered(size, size), th.text2.mix(th.text, k), 1.7);
                });
            }
            let m = o(L::MAIN);
            let (bg, fg) = if th.dark { (Color::rgb8(255, 255, 255), Color::rgb8(0, 0, 0)) } else { (Color::rgb8(0x15, 0x15, 0x15), Color::rgb8(255, 255, 255)) };
            let s = self.press(Hit::Main, 0.94) * (1.0 + 0.06 * self.hov(Hit::Main));
            cv.with_scale_about(m.center(), s, |cv| {
                cv.fill_circle(m.center(), 18.0, bg);
                let g = self.base.player.sample("play");
                cv.with_opacity(g.alpha, |cv| cv.with_scale_about(m.center(), g.scale, |cv| icons::draw(cv, p.play_glyph, m.centered(20.0, 20.0), fg, 1.7)));
            });
        });
    }

    fn paint_art(&self, cv: &Canvas, th: &Theme, r: Rect) {
        cv.shadow_rr(r, 14.0, 16.0, 6.0, Color::rgba(0.0, 0.0, 0.0, 0.35));
        cv.fill_rr(r, 14.0, th.raised);
        let fade = self.base.player.sample("art");
        let cur = self.art_key.as_deref().filter(|k| self.base.images.contains(k));
        let mut drew = false;
        if let (Some(prev), true) = (self.art_prev.as_deref(), fade.active) {
            drew |= self.base.images.draw(cv, prev, r, 14.0, 1.0, draw::Fit::Cover);
        }
        if let Some(k) = cur {
            let a = if fade.active { fade.alpha } else { 1.0 };
            drew |= self.base.images.draw(cv, k, r, 14.0, a, draw::Fit::Cover);
        }
        if !drew {
            icons::draw(cv, "note2", r.centered(34.0, 34.0), th.dim, 1.5);
        }
    }

    // ------------------------------------------------------------------------------------------------------------------ volume row

    fn paint_volume(&self, cv: &Canvas, th: &Theme, ox: f32, oy: f32) {
        let o = |r: Rect| r.offset(ox, oy);
        let row = o(L::VOL_ROW);
        cv.fill_rr(row, th.m.radius, th.card);
        let v = &self.v.volume;
        let mute = o(L::MUTE);
        let km = self.hov(Hit::Mute);
        cv.with_scale_about(mute.center(), self.press(Hit::Mute, 0.94), |cv| icons::draw(cv, v.glyph, mute.centered(18.0, 18.0), th.dim.mix(th.text, km), 1.6));
        // slider: white fill (ink on the light card), thumb grows in on hover / drag
        let s = o(L::VOL);
        let (fill, tr) = if th.dark { (Color::rgb8(255, 255, 255), th.track) } else { (Color::rgb8(0x15, 0x15, 0x15), th.track) };
        let pct = (v.shown / 100.0) as f32;
        track(cv, s, pct, fill, tr);
        let hv = self.hov(Hit::Vol).max(if self.vol_drag { 1.0 } else { 0.0 });
        let sc = if self.vol_drag { 1.18 } else { 0.7 + 0.4 * hv };
        thumb(cv, s, pct, hv, sc);
        cv.text(o(L::VOL_NUM), &v.number.to_string(), Style::new(12.0, 400).mono().right(), th.dim);
        // device chip
        let d = o(L::DEV);
        let hd = self.hov(Hit::Dev).max(self.menu_p.value().clamp(0.0, 1.0));
        let (c0, c1) = if th.dark { (Color::rgb8(0x1d, 0x1d, 0x1d), Color::rgb8(0x26, 0x26, 0x26)) } else { (Color::rgb8(0xd6, 0xd6, 0xd6), Color::rgb8(0xd6, 0xd6, 0xd6)) };
        cv.with_scale_about(d.center(), self.press(Hit::Dev, 0.96), |cv| {
            cv.fill_rr(d, 10.0, c0.mix(c1, hd));
            icons::draw(cv, "device", Rect::new(d.x + 8.0, d.cy() - 8.0, 16.0, 16.0), th.dim, 1.6);
            cv.text(Rect::new(d.x + 32.0, d.cy() - 9.0, d.w - 32.0 - 30.0, 18.0), &self.v.devices.button_text, Style::new(12.0, 500).ellipsis(), th.text2);
            let chev = Rect::new(d.right() - 8.0 - 14.0, d.cy() - 7.0, 14.0, 14.0);
            cv.with_transform(rotation(chev.center(), 180.0 * self.menu_p.value().clamp(0.0, 1.0)), |cv| icons::draw(cv, "chevron", chev, th.dim, 1.8));
        });
    }

    // ------------------------------------------------------------------------------------------------------------------ tiles

    fn paint_tile(&self, cv: &Canvas, th: &Theme, r: Rect, h: Hit, t: &pill_ui_core::panel::a::home::TileView) {
        let k = if t.soon { 0.0 } else { self.hov(h) };
        let kb = self.hov(h);
        cv.with_scale_about(r.center(), if t.soon { 1.0 } else { self.press(h, 0.96) }, |cv| {
            cv.fill_rr(r, th.m.radius, th.card.mix(th.card_hover, kb));
            let ic = Rect::new(r.x + 12.0, r.cy() - 9.0, 18.0, 18.0);
            let ia = if t.soon { 0.68 } else { 1.0 };
            cv.with_scale_about(ic.center(), 1.0 + 0.1 * k, |cv| icons::draw(cv, t.icon, ic, th.text2.fade(ia), 1.6));
            let la = if t.soon { 0.85 } else { 1.0 };
            cv.text(Rect::new(ic.right() + 8.0, r.y, r.right() - 6.0 - (ic.right() + 8.0), r.h), &t.label, Style::new(12.0, 600).ellipsis(), th.text.fade(la));
        });
    }

    // ------------------------------------------------------------------------------------------------------------------ launcher

    fn paint_launcher(&self, cv: &Canvas, th: &Theme, ox: f32, oy: f32) {
        let o = |r: Rect| r.offset(ox, oy);
        let strip = o(L::STRIP);
        let lv = &self.v.launcher;
        if !lv.visible {
            return;
        }
        cv.fill_rr(strip, th.m.radius, if lv.over { th.bg } else { th.card });
        if lv.over {
            cv.fill_rr(strip, th.m.radius, th.accent_soft);
            cv.stroke_rr(strip, th.m.radius, 1.0, th.accent);
        }
        let ink = if th.dark { Color::rgb8(255, 255, 255) } else { Color::rgb8(0, 0, 0) };
        let add = Rect::new(self.add_x + ox, L::add(0).y + oy, L::ADD, L::ADD);
        let ha = self.hov(Hit::Add);
        cv.with_clip(strip, |cv| {
            cv.with_scale_about(add.center(), self.press(Hit::Add, 0.94), |cv| {
                cv.fill_rr(add, 10.0, th.hover.fade(ha));
                let edge = if lv.over { th.accent } else if th.dark { Color::rgba(1.0, 1.0, 1.0, 0.2 + 0.2 * ha) } else { Color::rgba(0.0, 0.0, 0.0, 0.22) };
                dashed_rr(cv, add, 10.0, edge);
                icons::draw(cv, "plus", add.centered(20.0, 20.0), if lv.over { th.accent } else { th.dim }, 1.5);
            });
            if !lv.has {
                cv.text(Rect::new(add.right() + 10.0, strip.y, strip.right() - add.right() - 14.0, strip.h), &lv.hint, Style::new(12.0, 500), th.faint);
            }
            // the dragged icon last, on top
            let order = self.apps.iter().filter(|a| !a.dragging).chain(self.apps.iter().filter(|a| a.dragging));
            for a in order {
                if let Some(t) = lv.tiles.iter().find(|t| t.id == a.id) {
                    self.paint_app(cv, th, a, t, ox, oy, ink);
                }
            }
        });
        // the remove badge overlaps the card above, so it is drawn outside the strip clip
        if !lv.reordering {
            for a in &self.apps {
                let hv = self.hov(Hit::App(a.id));
                if hv > 0.01 && layout_visible(&a.rect) {
                    let b = L::app_x(a.rect.offset(ox, oy - self.lift(a.id)));
                    let hb = self.hov(Hit::AppX(a.id));
                    cv.with_opacity(hv * a.alpha, |cv| {
                        cv.with_scale_about(b.center(), 0.7 + 0.3 * hv, |cv| {
                            cv.fill_circle(b.center(), 8.0, Color::rgb8(0x4a, 0x4a, 0x4a).mix(Color::rgb8(0x66, 0x66, 0x66), hb));
                            icons::draw(cv, "x", b.centered(10.0, 10.0), Color::rgb8(255, 255, 255), 2.4);
                        })
                    });
                }
            }
        }
    }

    fn lift(&self, id: i64) -> f32 {
        if self.v.launcher.reordering {
            0.0
        } else {
            2.0 * self.hov(Hit::App(id)).max(self.hov(Hit::AppX(id)))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_app(&self, cv: &Canvas, th: &Theme, a: &super::AppDraw, t: &pill_ui_core::panel::a::launcher::Tile, ox: f32, oy: f32, ink: Color) {
        let h = Hit::App(t.id);
        let hv = self.hov(h).max(self.hov(Hit::AppX(t.id)));
        let r = a.rect.offset(ox, oy - self.lift(t.id));
        let s = a.scale * if t.dragging { 1.0 } else { self.press(h, 0.94) };
        cv.with_opacity(a.alpha, |cv| {
            cv.with_scale_about(r.center(), s, |cv| {
                cv.fill_rr(r, 10.0, ink.alpha(0.06 * hv));
                let ir = r.centered(32.0, 32.0);
                let drew = t.icon.as_ref().map_or(false, |u| self.base.images.draw(cv, &super::icon_key(t.id, u), ir, 0.0, 1.0, draw::Fit::Contain));
                if !drew {
                    icons::draw(cv, t.glyph, r.centered(26.0, 26.0), th.text2, 1.5);
                }
            })
        });
    }

    // ------------------------------------------------------------------------------------------------------------------ device list

    fn paint_menu(&self, cv: &Canvas, th: &Theme, ox: f32, oy: f32) {
        let a = self.menu_a.value().clamp(0.0, 1.0);
        if a <= 0.005 {
            return;
        }
        let items = &self.v.devices.items;
        let r = L::menu(items.len()).offset(ox, oy);
        let p = self.menu_p.value();
        let (bg, hover, text) = if th.dark { (Color::rgb8(0x17, 0x17, 0x17), Color::rgb8(0x22, 0x22, 0x22), Color::rgb8(0xe8, 0xe8, 0xe8)) } else { (th.raised, th.raised2, th.text) };
        // grows out of the chip below it: scale .98 -> 1 about the bottom-right corner, 4 px rise
        let m = glasskit::canvas::mul(&glasskit::canvas::scale_about((r.right(), r.bottom()), 0.98 + 0.02 * p, 0.98 + 0.02 * p), &glasskit::canvas::translation(0.0, 4.0 * (1.0 - p)));
        cv.with_opacity(a, |cv| {
            cv.with_transform(m, |cv| {
                cv.shadow_rr(r, 14.0, 30.0, 12.0, Color::rgba(0.0, 0.0, 0.0, 0.6));
                cv.fill_rr(r, 14.0, bg);
                let view = r.inset_xy(0.0, 5.0);
                let sv = &self.menu_scroll;
                cv.with_clip_rr(r, 14.0, |cv| {
                    sv.draw(cv, view, th, |cv, content| {
                        for (i, d) in items.iter().enumerate() {
                            let row = Rect::new(0.0, i as f32 * L::MENU_ROW, content.w, L::MENU_ROW);
                            let hov = self.hover == Some(Hit::MenuItem(i));
                            if hov {
                                cv.fill_rect(row, hover);
                            }
                            if d.current {
                                icons::draw(cv, "check", Rect::new(row.x + 12.0, row.cy() - 7.0, 14.0, 14.0), Color::rgb8(0x8f, 0x8f, 0xff), 2.2);
                            }
                            cv.text(Rect::new(row.x + 36.0, row.y, row.w - 36.0 - 16.0, row.h), &d.name, Style::new(13.0, 400).ellipsis(), text);
                        }
                    });
                });
            });
        });
    }
}

fn layout_visible(r: &Rect) -> bool {
    r.right() > L::STRIP.x && r.x < L::STRIP.right()
}

