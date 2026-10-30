//! Chat popovers: the history list (`.ch-hist`) and the AI-settings panel (`.ch-set`), plus the click-target keys of the whole Chat tab.
//!
//! Both are plain painters over the model's `HistView` / `SettingsView`; the state kept here is only presentation: the exit animation (the model
//! flips `open` at once and emits an `Exit` cue, the panel must stay on screen for those 160 ms), the FLIP of history rows, scrolling, and the
//! settings fields. Click targets go into the owner's `Clicks<K>`; later registrations win, so a popover registers a body target first (it eats
//! clicks that would fall through to the chat) and its buttons after.
use super::kit::{self, Bk, Clicks, Cues, Field, Icons};
use glasskit::canvas::Canvas;
use glasskit::controls::{Select, Widget};
use glasskit::ease::{curves, dur, Tween};
use glasskit::geom::Rect;
use glasskit::input::InputEvent;
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::chat::{HistRow, HistView};
use pill_ui_core::panel::b::chat_settings::{Msg as SMsg, SettingsView, Tone};
use std::collections::HashMap;

/// Every click target of the Chat tab.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum K {
    Compose,
    Attach,
    Mic,
    Send,
    Hist,
    Gear,
    Idea(usize),
    CopyMsg(usize),
    Retry,
    ErrRetry,
    ErrSettings,
    ErrX,
    FileX(usize),
    Enable,
    HNew,
    HRow(i64),
    HDel(i64),
    HBody,
    SBody,
    SClose,
    SPreset(usize),
    SScan,
    SSaveKey,
    SRmKey,
    STest,
    SOff,
    SModel,
}

pub const HIST_W: f32 = 260.0;
pub const HIST_MAX_H: f32 = 230.0;
const ROW_H: f32 = 44.0;

fn pop_bg(th: &Theme) -> Color {
    if th.dark {
        Color::rgb8(0x0c, 0x0c, 0x0c)
    } else {
        th.menu_bg
    }
}

/// The frame of a popover: shadow, fill, 1 px border.
fn frame(cv: &Canvas, th: &Theme, r: Rect) {
    cv.shadow_rr(r, 16.0, 18.0, 8.0, th.shadow);
    cv.fill_rr(r, 16.0, pop_bg(th));
    cv.stroke_rr(r, 16.0, 1.0, th.border);
}

/// Tracks `open` of a model view: while the exit cue plays the panel is still drawn (`left_ms`).
#[derive(Default)]
struct Presence {
    was: bool,
    left_ms: f32,
}

impl Presence {
    const EXIT_MS: f32 = 160.0;

    fn sync(&mut self, open: bool, reduced: bool) {
        if self.was && !open && !reduced {
            self.left_ms = Self::EXIT_MS;
        }
        if open {
            self.left_ms = 0.0;
        }
        self.was = open;
    }

    fn visible(&self) -> bool {
        self.was || self.left_ms > 0.0
    }

    fn step(&mut self, dt_ms: f32) -> bool {
        if self.left_ms > 0.0 {
            self.left_ms = (self.left_ms - dt_ms).max(0.0);
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------------------------------------------------------------------

struct Gone {
    row: HistRow,
    y: f32,
    fade: Tween<f32>,
}

#[derive(Default)]
pub struct HistPop {
    pres: Presence,
    last: Option<HistView>,
    ys: HashMap<i64, Tween<f32>>,
    gone: Vec<Gone>,
    scroll: ScrollView,
    rect: Rect,
}

impl HistPop {
    pub fn new() -> HistPop {
        HistPop::default()
    }

    pub fn sync(&mut self, v: &HistView, reduced: bool) {
        // rows that vanished fold away in place while the rows below slide up
        if let Some(old) = &self.last {
            for r in &old.rows {
                if !v.rows.iter().any(|n| n.id == r.id) && !self.gone.iter().any(|g| g.row.id == r.id) && v.open && !reduced {
                    let y = self.ys.get(&r.id).map_or(0.0, |t| t.value());
                    let mut fade = Tween::at_rest(1.0);
                    fade.retarget(0.0, dur::MED, curves::EASE_IN);
                    self.gone.push(Gone { row: r.clone(), y, fade });
                }
            }
        }
        self.pres.sync(v.open, reduced);
        if v.open || self.pres.visible() {
            self.last = Some(v.clone());
        }
    }

    pub fn visible(&self) -> bool {
        self.pres.visible()
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn step(&mut self, dt_ms: f32) -> bool {
        let dt = dt_ms / 1000.0;
        let mut busy = self.pres.step(dt_ms);
        for t in self.ys.values_mut() {
            busy |= t.step(dt);
        }
        for g in &mut self.gone {
            busy |= g.fade.step(dt);
        }
        self.gone.retain(|g| !g.fade.finished());
        busy |= self.scroll.step(dt);
        busy
    }

    pub fn wheel(&mut self, ev: &InputEvent) -> glasskit::input::Response {
        self.scroll.event(ev, self.rect.inset_xy(0.0, 1.0))
    }

    pub fn draw(&mut self, cv: &Canvas, th: &Theme, ic: &Icons, clicks: &mut Clicks<K>, cues: &Cues, area: Rect, reduced: bool) {
        let Some(v) = self.last.clone().filter(|_| self.pres.visible()) else { return };
        let n = v.rows.len();
        let body = 6.0 + 32.0 + 4.0 + if n == 0 { 32.0 } else { n as f32 * ROW_H } + 6.0;
        let h = body.min(HIST_MAX_H);
        let r = Rect::new(area.right() - HIST_W, area.y + 32.0, HIST_W, h);
        self.rect = r;
        let x = cues.xf("chat-history");
        let exit_k = if self.pres.was { 1.0 } else { (self.pres.left_ms / Presence::EXIT_MS).clamp(0.0, 1.0) };
        let alpha = if self.pres.was { x.alpha } else { exit_k };
        let (dy, sc) = if self.pres.was { (x.dy, x.scale) } else { (-6.0 * (1.0 - exit_k), 0.97 + 0.03 * exit_k) };
        let live = self.pres.was;
        let about = (r.right(), r.y);
        cv.with_opacity(alpha, |cv| {
            cv.with_offset(0.0, dy, |cv| {
                cv.with_scale_about(about, sc, |cv| {
                    frame(cv, th, r);
                    if live {
                        clicks.add(K::HBody, r);
                    }
                    let inner = r.inset_xy(6.0, 6.0);
                    // content scrolls under a clip of the panel
                    let view = Rect::new(r.x, r.y + 1.0, r.w, r.h - 2.0);
                    self.scroll.set_content_height(body - 2.0, view.h);
                    let off = self.scroll.offset();
                    cv.with_clip_rr(r.inset(1.0), 15.0, |cv| {
                        let mut y = inner.y - 1.0 - off + 1.0;
                        // New chat
                        let nr = Rect::new(inner.x, y, inner.w, 32.0);
                        if live {
                            clicks.add(K::HNew, nr);
                        }
                        let (hv, ps) = (clicks.hover_amt(&K::HNew), clicks.press_scale(&K::HNew, 0.96));
                        cv.with_scale_about(nr.center(), ps, |cv| {
                            cv.fill_rr(nr, th.m.btn_r, th.btn.mix(th.btn_hover, hv));
                            ic.draw(cv, "c-compose", Rect::new(nr.x + 12.0, nr.cy() - 7.0, 14.0, 14.0), th.text);
                            cv.text(Rect::new(nr.x + 12.0 + 14.0 + 6.0, nr.y, nr.w - 40.0, nr.h), &v.new_label, th.btn_text(), th.text);
                        });
                        y += 32.0 + 4.0;
                        if let Some(e) = &v.empty_text {
                            cv.text(Rect::new(inner.x + 12.0, y, inner.w - 24.0, 32.0), e, th.small(400), th.dim);
                        }
                        for g in &self.gone {
                            self.draw_row(cv, th, ic, clicks, inner, &g.row, g.y - off + 1.0, g.fade.value(), false);
                        }
                        for (i, row) in v.rows.iter().enumerate() {
                            let want = y - 0.0 + i as f32 * ROW_H + off; // content-space y
                            let t = self.ys.entry(row.id).or_insert_with(|| Tween::at_rest(want));
                            if (t.target() - want).abs() > 0.01 {
                                if reduced {
                                    t.jump(want);
                                } else {
                                    t.retarget(want, dur::MED, curves::EASE_OUT);
                                }
                            }
                            let cy = t.value() - off;
                            self.draw_row(cv, th, ic, clicks, inner, row, cy, 1.0, live);
                        }
                    });
                    self.scroll.bar.draw(cv, view, &self.scroll.state, th);
                });
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_row(&self, cv: &Canvas, th: &Theme, ic: &Icons, clicks: &mut Clicks<K>, inner: Rect, row: &HistRow, y: f32, fade: f32, live: bool) {
        let rr = Rect::new(inner.x, y, inner.w, ROW_H);
        if live {
            clicks.add(K::HRow(row.id), Rect::new(rr.x, rr.y, rr.w - 28.0, rr.h));
        }
        let hv = clicks.hover_amt(&K::HRow(row.id)).max(clicks.hover_amt(&K::HDel(row.id)));
        let dr = Rect::new(rr.right() - 4.0 - 24.0, rr.cy() - 12.0, 24.0, 24.0);
        if live {
            clicks.add(K::HDel(row.id), dr);
        }
        cv.with_opacity(fade, |cv| {
            let ps = clicks.press_scale(&K::HRow(row.id), 0.98);
            cv.with_scale_about(rr.center(), ps, |cv| {
                if row.selected {
                    cv.fill_rr(rr, 10.0, th.raised);
                } else {
                    cv.fill_rr(rr, 10.0, th.raised.fade(hv));
                }
                cv.text(Rect::new(rr.x + 12.0, rr.y + 6.0, rr.w - 12.0 - 36.0, 16.0), &row.title, Style::new(13.0, 400).ellipsis(), th.text);
                cv.text(Rect::new(rr.x + 12.0, rr.y + 24.0, rr.w - 12.0 - 36.0, 14.0), &row.date, Style::new(11.0, 400), th.dim);
            });
            let dh = clicks.hover_amt(&K::HDel(row.id));
            cv.with_opacity(hv, |cv| {
                cv.fill_rr(dr, 12.0, th.hover.fade(dh));
                ic.draw(cv, "trash", dr.centered(14.0, 14.0), th.icon.mix(th.text, dh));
            });
        });
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// AI settings
// ---------------------------------------------------------------------------------------------------------------------------------------

pub struct SetPop {
    pres: Presence,
    last: Option<SettingsView>,
    pub scroll: ScrollView,
    pub select: Select,
    models: Vec<String>,
    rect: Rect,
    /// where the model dropdown was drawn (window coordinates), for its events
    pub model_rect: Rect,
    dots: HashMap<usize, Tween<f32>>,
}

impl Default for SetPop {
    fn default() -> Self {
        SetPop::new()
    }
}

fn tone_color(th: &Theme, t: Tone) -> Color {
    match t {
        Tone::Plain => th.text2,
        Tone::Dim => th.dim,
        Tone::Ok => th.green,
        Tone::Bad => th.red,
    }
}

impl SetPop {
    pub fn new() -> SetPop {
        SetPop { pres: Presence::default(), last: None, scroll: ScrollView::new(), select: Select::new(&[], None), models: vec![], rect: Rect::default(), model_rect: Rect::default(), dots: HashMap::new() }
    }

    pub fn sync(&mut self, v: &SettingsView, reduced: bool) {
        self.pres.sync(v.open, reduced);
        if v.open || self.pres.visible() {
            self.last = Some(v.clone());
        }
        if v.models != self.models {
            self.models = v.models.clone();
            let refs: Vec<&str> = self.models.iter().map(|s| s.as_str()).collect();
            let sel = refs.iter().position(|m| *m == v.model);
            self.select = Select::new(&refs, sel);
        } else if let Some(i) = self.models.iter().position(|m| *m == v.model) {
            if self.select.selected() != Some(i) {
                self.select.set_selected(Some(i));
            }
        }
    }

    pub fn visible(&self) -> bool {
        self.pres.visible()
    }

    pub fn is_open(&self) -> bool {
        self.pres.was
    }

    pub fn rect(&self) -> Rect {
        self.rect
    }

    pub fn show_models(&self) -> bool {
        !self.models.is_empty()
    }

    pub fn step(&mut self, dt_ms: f32) -> bool {
        let dt = dt_ms / 1000.0;
        let mut busy = self.pres.step(dt_ms);
        busy |= self.scroll.step(dt);
        busy |= self.select.step(dt);
        for t in self.dots.values_mut() {
            busy |= t.step(dt);
        }
        busy
    }

    /// Lays the panel out and paints it. Returns the focus request of the three fields (index into `[url, model, key]`).
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, cv: &Canvas, th: &Theme, ic: &Icons, clicks: &mut Clicks<K>, cues: &Cues, area: Rect, f: [&mut Field; 3], reduced: bool) -> Option<usize> {
        let Some(v) = self.last.clone().filter(|_| self.pres.visible()) else { return None };
        let r = area;
        self.rect = r;
        let live = self.pres.was;
        let x = cues.xf("ai-settings");
        let exit_k = if live { 1.0 } else { (self.pres.left_ms / Presence::EXIT_MS).clamp(0.0, 1.0) };
        let alpha = if live { x.alpha } else { exit_k };
        let (dy, sc) = if live { (x.dy, x.scale) } else { (-6.0 * (1.0 - exit_k), 0.97 + 0.03 * exit_k) };
        let mut want_focus = None;
        let [url, mdl, key] = f;
        cv.with_opacity(alpha, |cv| {
            cv.with_offset(0.0, dy, |cv| {
                cv.with_scale_about((r.cx(), r.y), sc, |cv| {
                    cv.fill_rr(r, 16.0, pop_bg(th));
                    cv.stroke_rr(r, 16.0, 1.0, th.border);
                    if live {
                        clicks.add(K::SBody, r);
                    }
                    let view = Rect::new(r.x, r.y + 1.0, r.w, r.h - 2.0);
                    let cw = r.w - 32.0;
                    let ch = self.content_h(&v);
                    self.scroll.set_content_height(ch + 32.0, view.h);
                    let off = self.scroll.offset();
                    cv.with_clip_rr(r.inset(1.0), 15.0, |cv| {
                        let ox = r.x + 16.0;
                        let mut y = r.y + 16.0 - off;
                        // header
                        cv.text(Rect::new(ox, y, cw - 40.0, 28.0), &v.title, Style::new(14.0, 600), th.text);
                        let cb = Rect::new(ox + cw + 8.0 - 28.0, y, 28.0, 28.0);
                        if live {
                            clicks.add(K::SClose, cb);
                        }
                        kit::button(cv, th, ic, cb, "", Some("x"), Bk::Icon, clicks.hover_amt(&K::SClose), clicks.press_scale(&K::SClose, 0.92), true);
                        y += 28.0 + 8.0;
                        // presets
                        let sr = Rect::new(ox, y, 0.0, 32.0);
                        let st = Style::new(12.0, 500);
                        let mut widths = vec![];
                        for p in v.presets.iter() {
                            let tw = cv.text_width(&p.label, st);
                            widths.push(24.0 + tw + if p.id != "custom" { 12.0 } else { 0.0 });
                        }
                        let total: f32 = widths.iter().sum::<f32>() + 4.0 * (widths.len() as f32 - 1.0) + 8.0;
                        let track = Rect::new(sr.x, sr.y, total, 32.0);
                        cv.fill_rr(track, 12.0, th.raised);
                        let mut bx = sr.x + 4.0;
                        for (i, p) in v.presets.iter().enumerate() {
                            let b = Rect::new(bx, sr.y + 4.0, widths[i], 24.0);
                            if live {
                                clicks.add(K::SPreset(i), b);
                            }
                            let hv = clicks.hover_amt(&K::SPreset(i));
                            let ps = clicks.press_scale(&K::SPreset(i), 0.95);
                            let on = self.dots.entry(i).or_insert_with(|| Tween::at_rest(p.on as u8 as f32));
                            let want = p.on as u8 as f32;
                            if on.target() != want {
                                if reduced {
                                    on.jump(want);
                                } else {
                                    on.retarget(want, dur::FAST, curves::EASE_OUT);
                                }
                            }
                            let onv = on.value().clamp(0.0, 1.0);
                            cv.with_scale_about(b.center(), ps, |cv| {
                                cv.fill_rr(b, 8.0, th.raised2.fade(onv));
                                let fg = th.dim.mix(th.text, onv.max(hv));
                                let mut tx = b.x + 12.0;
                                if p.id != "custom" {
                                    let up = p.up == Some(true);
                                    let dot = th.dim.mix(th.green, up as u8 as f32);
                                    if up {
                                        cv.fill_circle((tx + 3.0, b.cy()), 6.0, th.green.alpha(0.18));
                                    }
                                    cv.fill_circle((tx + 3.0, b.cy()), 3.0, dot.fade(if up { 1.0 } else { 0.5 }));
                                    tx += 12.0;
                                }
                                cv.text(Rect::new(tx, b.y, b.w, b.h), &p.label, st, fg);
                            });
                            bx += widths[i] + 4.0;
                        }
                        y += 32.0 + 8.0;
                        if !v.scan_msg.text.is_empty() {
                            y = para(cv, th, ox, y, cw, &v.scan_msg) - 8.0;
                        }
                        // url
                        cv.text(Rect::new(ox, y, cw, 16.0), &v.url_label, th.small(400), th.dim);
                        y += 16.0 + 4.0;
                        let ur = Rect::new(ox, y, cw, 32.0);
                        url.ti.placeholder = v.url_placeholder.clone();
                        if field(cv, th, url, ur) {
                            want_focus = Some(0);
                        }
                        y += 32.0 + 8.0;
                        if !v.url_error.is_empty() {
                            y = para(cv, th, ox, y - 4.0, cw, &SMsg { text: v.url_error.clone(), tone: Tone::Bad }) + 0.0;
                        }
                        // model
                        cv.text(Rect::new(ox, y, cw, 16.0), &v.model_label, th.small(400), th.dim);
                        y += 16.0 + 4.0;
                        let sw = cv.text_width(&v.scan_label, th.btn_text()) + 2.0 * th.m.btn_pad_x;
                        let mr = Rect::new(ox, y, cw - sw - 8.0, 32.0);
                        self.model_rect = mr;
                        if self.models.is_empty() {
                            mdl.ti.placeholder = v.model_placeholder.clone();
                            if field(cv, th, mdl, mr) {
                                want_focus = Some(1);
                            }
                        } else {
                            if live {
                                clicks.add(K::SModel, mr);
                            }
                            self.select.set_work_area(Rect::new(-10_000.0, -10_000.0, 20_000.0, 20_000.0));
                            self.select.draw(cv, th, mr);
                        }
                        let br = Rect::new(mr.right() + 8.0, y, sw, 32.0);
                        if live {
                            clicks.add(K::SScan, br);
                        }
                        raised_btn(cv, th, ic, clicks, K::SScan, br, &v.scan_label, !v.scan_busy);
                        y += 32.0 + 8.0;
                        // key
                        cv.text(Rect::new(ox, y, cw, 16.0), &v.key_label, th.small(400), th.dim);
                        y += 16.0 + 4.0;
                        let save_w = cv.text_width(&v.save_label, th.btn_text()) + 2.0 * th.m.btn_pad_x;
                        let rm_w = if v.has_key { cv.text_width(&v.remove_label, th.btn_text()) + 2.0 * th.m.btn_pad_x } else { 0.0 };
                        let kw = cw - save_w - 8.0 - if v.has_key { rm_w + 8.0 } else { 0.0 };
                        let kr = Rect::new(ox, y, kw, 32.0);
                        key.ti.password = true;
                        key.ti.placeholder = v.key_placeholder.clone();
                        if field(cv, th, key, kr) {
                            want_focus = Some(2);
                        }
                        let sb = Rect::new(kr.right() + 8.0, y, save_w, 32.0);
                        if live {
                            clicks.add(K::SSaveKey, sb);
                        }
                        raised_btn(cv, th, ic, clicks, K::SSaveKey, sb, &v.save_label, true);
                        if v.has_key {
                            let rb = Rect::new(sb.right() + 8.0, y, rm_w, 32.0);
                            if live {
                                clicks.add(K::SRmKey, rb);
                            }
                            kit::button(cv, th, ic, rb, &v.remove_label, None, Bk::Danger, clicks.hover_amt(&K::SRmKey), clicks.press_scale(&K::SRmKey, 0.96), true);
                        }
                        y += 32.0 + 8.0;
                        if !v.key_msg.text.is_empty() {
                            y = para(cv, th, ox, y - 4.0, cw, &v.key_msg);
                        }
                        // test row
                        let tw = cv.text_width(&v.test_label, th.btn_text()) + 2.0 * th.m.btn_pad_x;
                        let tr = Rect::new(ox, y, tw, 32.0);
                        if live {
                            clicks.add(K::STest, tr);
                        }
                        raised_btn(cv, th, ic, clicks, K::STest, tr, &v.test_label, !v.test_busy);
                        let ow = cv.text_width(&v.off_label, th.btn_text()) + 2.0 * th.m.btn_pad_x;
                        let or_ = Rect::new(ox + cw - ow + 14.0, y, ow, 32.0);
                        if live {
                            clicks.add(K::SOff, or_);
                        }
                        kit::button(cv, th, ic, or_, &v.off_label, None, Bk::Ghost, clicks.hover_amt(&K::SOff), clicks.press_scale(&K::SOff, 0.96), true);
                        if !v.test_msg.text.is_empty() {
                            cv.text(Rect::new(tr.right() + 8.0, y, (or_.x - tr.right() - 16.0).max(0.0), 32.0), &v.test_msg.text, th.small(400).ellipsis(), tone_color(th, v.test_msg.tone));
                        }
                        y += 32.0 + 8.0;
                        if !v.hosts.is_empty() {
                            cv.text(Rect::new(ox, y, cw, 32.0), &v.hosts, Style::new(12.0, 400).wrap(), th.dim);
                        }
                        // the dropdown floats above everything else in the panel
                        self.select.draw_popup(cv, th);
                    });
                    self.scroll.bar.draw(cv, view, &self.scroll.state, th);
                });
            });
        });
        want_focus
    }

    fn content_h(&self, v: &SettingsView) -> f32 {
        let mut h = 28.0 + 8.0 + 32.0 + 8.0;
        if !v.scan_msg.text.is_empty() {
            h += 16.0;
        }
        h += 16.0 + 4.0 + 32.0 + 8.0;
        if !v.url_error.is_empty() {
            h += 12.0;
        }
        h += 16.0 + 4.0 + 32.0 + 8.0;
        h += 16.0 + 4.0 + 32.0 + 8.0;
        if !v.key_msg.text.is_empty() {
            h += 12.0;
        }
        h += 32.0 + 8.0;
        if !v.hosts.is_empty() {
            h += 32.0;
        }
        h
    }
}

/// One-line status text (`.ch-sm`); returns the y below it.
fn para(cv: &Canvas, th: &Theme, x: f32, y: f32, w: f32, m: &SMsg) -> f32 {
    let st = Style::new(12.0, 400).wrap();
    let (_, h) = cv.measure(&m.text, st, w);
    cv.text(Rect::new(x, y, w, h.max(16.0)), &m.text, st, tone_color(th, m.tone));
    y + h.max(16.0) + 8.0
}

/// A settings field (`.ch-set .field`: 32 px, raised fill, 12 px text).
fn field(cv: &Canvas, th: &Theme, f: &mut Field, r: Rect) -> bool {
    let mut t = *th;
    t.field = th.raised;
    f.ti.style = Style::new(12.0, 400);
    f.ti.radius = 12.0;
    f.ti.pad = (12.0, 0.0);
    let req = f.pump(cv, r);
    f.ti.draw(cv, r, &t);
    req
}

/// `.ch-set .btn`: a raised button.
#[allow(clippy::too_many_arguments)]
fn raised_btn(cv: &Canvas, th: &Theme, _ic: &Icons, clicks: &Clicks<K>, k: K, r: Rect, label: &str, enabled: bool) {
    let hv = clicks.hover_amt(&k);
    let ps = clicks.press_scale(&k, 0.96);
    cv.with_opacity(if enabled { 1.0 } else { 0.45 }, |cv| {
        cv.with_scale_about(r.center(), ps, |cv| {
            cv.fill_rr(r, th.m.btn_r, th.raised.mix(th.raised2, hv * 0.7));
            cv.text(r, label, th.btn_text().center(), th.text);
        });
    });
}
