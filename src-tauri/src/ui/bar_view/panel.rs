//! The uninstall panel and its status strip, drawn from the model's `UninstallView`: confirm card, running / scanning / deleting / elevating pages,
//! the leftovers checklist (groups, ticks, sticky footer), summary, still / error. `build_panel` lays the parts out (flow layout in the box's
//! content coordinates, text heights measured with the real fonts), `paint_panel` draws them under the scroll offset, with the entry animations of
//! `uninstall-ui.js` (box, step, staggered parts, group fold, row leave, shake, tick draw) played from the model's cues.
use super::anim::Look;
use super::paint::{fit_text, with_look};
use super::*;
use glasskit::color::Color;
use glasskit::text::{Align, Style};
use pill_ui_core::bar::uninstall::view::{Button, ButtonStyle, GroupView, ItemRowView, Part, ProgressKind, StripIcon, StripTone, StripView};

/// A coloured role of the theme, resolved at paint time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Role {
    Text,
    Text2,
    Dim,
    Faint,
    Red,
    Accent,
}

fn role(th: &Theme, r: Role) -> Color {
    match r {
        Role::Text => th.text,
        Role::Text2 => th.text2,
        Role::Dim => th.dim,
        Role::Faint => th.faint,
        Role::Red => th.red,
        Role::Accent => th.accent,
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PKind {
    Text { text: String, size: f32, weight: u16, role: Role, lh: f32 },
    Warn { text: String },
    Check { control: Control, label: String, checked: bool },
    Progress { elev: bool },
    Shield,
    Done,
    ScanLine { label: String, found: u64, stage: String },
    Link(Button),
    Buttons(Vec<(Rect, Button)>),
    Group(GroupView),
    Item(ItemRowView),
    Delete { text: String },
    Footer { selected: Option<String>, buttons: Vec<(Rect, Button)> },
}

#[derive(Clone, Debug)]
pub(crate) struct PItem {
    /// index of the model part (the stagger target)
    pub part: usize,
    /// relative to the content origin (the box's inner left, below its top padding)
    pub rect: Rect,
    pub kind: PKind,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PanelFrame {
    pub content_h: f32,
    pub items: Vec<PItem>,
}

const UBTN_H: f32 = 28.0;

fn lines_of(cv: &Canvas, text: &str, size: f32, weight: u16, width: f32) -> f32 {
    let st = Style::new(size, weight);
    let one = cv.measure("Ag", st, 4096.0).1.max(1.0);
    let (_, h) = cv.measure(text, st.wrap(), width.max(8.0));
    (h / one).round().max(1.0)
}

fn button_w(cv: &Canvas, b: &Button) -> f32 {
    let t = cv.text_width(b.label.trim_start(), Style::new(12.0, 400)).ceil();
    t + 28.0 + if b.icon.is_some() { 20.0 } else { 0.0 }
}

/// A row of buttons, right aligned, wrapping with 8 px gaps. Returns the placed buttons and the row block's height.
fn button_rows(cv: &Canvas, bs: &[Button], width: f32, y: f32) -> (Vec<(Rect, Button)>, f32) {
    let mut rows: Vec<Vec<(f32, &Button)>> = vec![vec![]];
    let mut used = 0.0;
    for b in bs {
        let w = button_w(cv, b);
        if !rows.last().unwrap().is_empty() && used + 8.0 + w > width {
            rows.push(vec![]);
            used = 0.0;
        }
        used += if rows.last().unwrap().is_empty() { 0.0 } else { 8.0 };
        used += w;
        rows.last_mut().unwrap().push((w, b));
    }
    let mut out = Vec::new();
    for (ri, row) in rows.iter().enumerate() {
        let total: f32 = row.iter().map(|r| r.0).sum::<f32>() + 8.0 * (row.len().saturating_sub(1)) as f32;
        let mut x = width - total;
        for (w, b) in row {
            out.push((Rect::new(x, y + ri as f32 * (UBTN_H + 8.0), *w, UBTN_H), (*b).clone()));
            x += w + 8.0;
        }
    }
    (out, rows.len() as f32 * UBTN_H + (rows.len() as f32 - 1.0) * 8.0)
}

impl BarView {
    /// Lays the parts of `p` out in a column `w` wide.
    pub(super) fn build_panel(&mut self, cv: &Canvas, _th: &Theme, p: &PanelView, w: f32) -> PanelFrame {
        let mut items: Vec<PItem> = Vec::new();
        let mut y = 6.0f32; // the 6 px of the first heading's margin (it does not collapse into the box's own padding)
        let text = |items: &mut Vec<PItem>, part: usize, y: &mut f32, t: &str, size: f32, weight: u16, lh: f32, role: Role, mt: f32, mb: f32, x: f32| {
            *y += mt;
            let n = lines_of(cv, t, size, weight, w - x);
            let h = n * lh;
            items.push(PItem { part, rect: Rect::new(x, *y, w - x, h), kind: PKind::Text { text: t.to_string(), size, weight, role, lh } });
            *y += h + mb;
        };
        let mut first = true;
        for (pi, part) in p.parts.iter().enumerate() {
            match part {
                Part::Heading(t) => {
                    text(&mut items, pi, &mut y, t, 14.0, 600, 20.0, Role::Text, if first { 0.0 } else { 6.0 }, 2.0, 0.0);
                }
                Part::Sub(t) => text(&mut items, pi, &mut y, t, 11.0, 400, 16.0, Role::Faint, 0.0, 0.0, 0.0),
                Part::SubHead(t) => text(&mut items, pi, &mut y, t, 12.0, 400, 18.0, Role::Dim, 8.0, 0.0, 0.0),
                Part::Note(t) | Part::Busy(t) => text(&mut items, pi, &mut y, t, 12.0, 400, 16.0, Role::Dim, 8.0, 0.0, 0.0),
                Part::Path { text: t, fail } => text(&mut items, pi, &mut y, t, 12.0, 400, 16.0, if *fail { Role::Red } else { Role::Text2 }, 0.0, 0.0, 0.0),
                Part::Extra { text: t } => text(&mut items, pi, &mut y, t, 12.0, 400, 16.0, Role::Text2, 0.0, 0.0, 0.0),
                Part::Warn { text: t, .. } => {
                    y += 8.0;
                    let n = lines_of(cv, t, 12.0, 400, w - 20.0);
                    let h = 12.0 + 16.0 * n;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, w, h), kind: PKind::Warn { text: t.clone() } });
                    y += h;
                }
                Part::Check { control, label, checked } => {
                    y += 8.0;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, w, 18.0), kind: PKind::Check { control: *control, label: label.clone(), checked: *checked } });
                    y += 18.0;
                }
                Part::Progress(k) => {
                    y += 10.0;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, w, 3.0), kind: PKind::Progress { elev: *k == ProgressKind::Elevating } });
                    y += 3.0;
                }
                Part::Shield => {
                    y += 6.0;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, 28.0, 28.0), kind: PKind::Shield });
                    y += 28.0;
                }
                Part::ScanLine { label, found, stage } => {
                    y += 8.0;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, w, 16.0), kind: PKind::ScanLine { label: label.clone(), found: *found, stage: stage.clone() } });
                    y += 16.0;
                }
                Part::DoneHead { title, line, need_admin, failed_head, failed } => {
                    y += 4.0;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, 34.0, 34.0), kind: PKind::Done });
                    let x = 46.0;
                    let y0 = y;
                    let mut cy = y;
                    text(&mut items, pi, &mut cy, title, 14.0, 600, 20.0, Role::Text, 6.0, 2.0, x);
                    text(&mut items, pi, &mut cy, line, 11.0, 400, 16.0, Role::Faint, 0.0, 0.0, x);
                    if let Some(n) = need_admin {
                        text(&mut items, pi, &mut cy, n, 11.0, 400, 16.0, Role::Faint, 0.0, 0.0, x);
                    }
                    y = cy.max(y0 + 34.0);
                    if let Some(h) = failed_head {
                        text(&mut items, pi, &mut y, h, 12.0, 400, 18.0, Role::Dim, 8.0, 0.0, 0.0);
                        for f in failed {
                            text(&mut items, pi, &mut y, f, 12.0, 400, 16.0, Role::Red, 0.0, 0.0, 0.0);
                        }
                    }
                }
                Part::Link(b) => {
                    y += 6.0;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, cv.text_width(&b.label, Style::new(12.0, 400)).ceil(), 18.0), kind: PKind::Link(b.clone()) });
                    y += 18.0;
                }
                Part::Buttons(bs) => {
                    y += 12.0;
                    let (placed, h) = button_rows(cv, bs, w, 0.0);
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, w, h), kind: PKind::Buttons(placed) });
                    y += h;
                }
                Part::Group(g) => {
                    if g.hidden {
                        continue;
                    }
                    let tw = self.chev_target(g.index, g.open);
                    let _ = tw;
                    y += 8.0;
                    let look = self.anim.look(&AnimTarget::Group(g.index));
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, w, 26.0), kind: PKind::Group(g.clone()) });
                    y += 26.0;
                    if g.open || look.height < 1.0 || self.anim.is_held(&AnimTarget::Group(g.index)) {
                        let mut body = 0.0;
                        let mut rows = Vec::new();
                        for r in &g.rows {
                            let rl = self.anim.look(&AnimTarget::ChecklistRow(r.pick));
                            if r.gone && (rl.height <= 0.001 || !self.anim.busy() && !self.anim.is_held(&AnimTarget::ChecklistRow(r.pick))) {
                                continue;
                            }
                            let n = lines_of(cv, &r.path, 12.0, 400, w - 12.0 - 26.0);
                            let mn = lines_of(cv, &r.meta, 11.0, 400, w - 12.0 - 26.0);
                            let h = 8.0 + 16.0 * n + 16.0 * mn;
                            let eff = (h + 2.0) * rl.height;
                            rows.push((r.clone(), body + 2.0, h, rl.height));
                            body += eff;
                        }
                        let gh = if self.anim.is_held(&AnimTarget::Group(g.index)) && !g.open { 0.0 } else { body * if g.open || look.height < 1.0 { look.height } else { 1.0 } };
                        let _ = gh;
                        let scale = look.height;
                        for (r, off, h, k) in rows {
                            items.push(PItem { part: pi, rect: Rect::new(0.0, y + off * scale.max(0.0).min(1.0).max(if scale >= 1.0 { 1.0 } else { scale }), w, h * k), kind: PKind::Item(r) });
                        }
                        y += body * scale;
                    }
                }
                Part::DeleteProgress { text: t } => {
                    y += 4.0;
                    items.push(PItem { part: pi, rect: Rect::new(0.0, y, w, 37.0), kind: PKind::Delete { text: t.clone() } });
                    y += 37.0 + 4.0;
                }
                Part::Footer(f) => {
                    y += 10.0;
                    let (placed, _) = button_rows(cv, &f.buttons.iter().cloned().collect::<Vec<_>>(), w + 32.0 - 32.0, 0.0);
                    // one nowrap row, right aligned inside the footer's padding
                    let total: f32 = f.buttons.iter().map(|b| button_w(cv, b)).sum::<f32>() + 8.0 * f.buttons.len().saturating_sub(1) as f32;
                    let mut x = w - total;
                    let mut row = Vec::new();
                    for b in &f.buttons {
                        let bw = button_w(cv, b);
                        row.push((Rect::new(x, 8.0, bw, UBTN_H), b.clone()));
                        x += bw + 8.0;
                    }
                    let _ = placed;
                    items.push(PItem { part: pi, rect: Rect::new(-16.0, y, w + 32.0, 46.0), kind: PKind::Footer { selected: f.selected.clone(), buttons: row } });
                    y += 46.0;
                }
            }
            first = false;
        }
        PanelFrame { content_h: y + 12.0, items }
    }

    fn chev_target(&mut self, g: usize, open: bool) -> f32 {
        let t = self.chev.entry(g).or_insert_with(|| Tween::at_rest(if open { 1.0 } else { 0.0 }));
        t.retarget(if open { 1.0 } else { 0.0 }, 0.38, curves::SPRING_POP);
        t.value()
    }

    // ---- painting ----

    pub(super) fn paint_panel(&mut self, cv: &Canvas, th: &Theme, cx: &Cx) {
        let Some(box_r) = self.lay.panel else { return };
        let Some(frame) = self.panel_frame.take() else { return };
        let Some(pv) = self.vm.uninstall.panel.clone() else {
            self.panel_frame = Some(frame);
            return;
        };
        let bl = self.anim.look(&AnimTarget::PanelBox);
        // a folded box (Collapse) clips its content to the shrinking height
        let visible_h = (box_r.h * bl.height).max(0.0);
        let clip = Rect::new(box_r.x, box_r.y, box_r.w, visible_h);
        let step = self.anim.look(&AnimTarget::PanelStep);
        self.hits.push((box_r, Tgt::Body)); // the box blocks what is under it
        let mut hits: Vec<(Rect, Tgt)> = Vec::new();
        let off = self.uni_scroll.st.offset();
        let ox = box_r.x + L::PANEL_PAD.1;
        let oy = box_r.y + L::PANEL_PAD.0 - off;
        with_look(cv, &Look { height: 1.0, ..bl }, box_r, |cv| {
            cv.with_clip(clip, |cv| {
                cv.fill_rr(Rect::new(box_r.x, box_r.y, box_r.w, box_r.h), L::PANEL_RADIUS, th.raised);
                cv.with_clip_rr(box_r, L::PANEL_RADIUS, |cv| {
                    with_look(cv, &step, box_r, |cv| {
                        let mut footer: Option<&PItem> = None;
                        for it in &frame.items {
                            if matches!(it.kind, PKind::Footer { .. }) {
                                footer = Some(it);
                                continue;
                            }
                            self.paint_item(cv, th, cx, it, (ox, oy), box_r, &pv, &mut hits);
                        }
                        if let Some(f) = footer {
                            // sticky: the footer stays at the bottom of the box while the list scrolls
                            let nat = oy + f.rect.y;
                            let y = nat.min(box_r.bottom() - f.rect.h);
                            let mut f2 = f.clone();
                            f2.rect.y = y - oy;
                            self.paint_item(cv, th, cx, &f2, (ox, oy), box_r, &pv, &mut hits);
                        }
                    });
                });
            });
        });
        self.uni_scroll.bar.draw(cv, box_r, &self.uni_scroll.st, th);
        for (r, t) in hits {
            if let Some(c) = clip_to(r, box_r) {
                self.hits.push((c, t));
            }
        }
        self.panel_frame = Some(frame);
    }

    fn focus_ring(&self, control: Control) -> bool {
        self.ring && matches!(self.focus, Focus::Panel(Some(i)) if self.focusables.get(i).is_some_and(|f| f.control == control))
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_item(&self, cv: &Canvas, th: &Theme, cx: &Cx, it: &PItem, o: Pt, box_r: Rect, pv: &PanelView, hits: &mut Vec<(Rect, Tgt)>) {
        let r = it.rect.offset(o.0, o.1);
        if r.bottom() < box_r.y - 40.0 || r.y > box_r.bottom() + 40.0 {
            return;
        }
        let mut pl = self.anim.look(&AnimTarget::PanelPart(it.part));
        if let PKind::Warn { .. } = it.kind {
            // `u-shake`: the warning shakes once
            if let Some(Part::Warn { shake: true, .. }) = pv.parts.get(it.part) {
                pl.dx += self.anim.look(&AnimTarget::PanelStep).dx;
            }
        }
        let busy = pv.busy;
        with_look(cv, &pl, r, |cv| match &it.kind {
            PKind::Text { text, size, weight, role: ro, lh } => {
                let st = Style::new(*size, *weight).wrap();
                cv.text(Rect::new(r.x, r.y + ((lh - cv.measure("Ag", Style::new(*size, *weight), 4096.0).1).max(0.0) / 2.0).floor(), r.w, r.h), text, st, role(th, *ro));
            }
            PKind::Warn { text } => {
                cv.fill_rr(r, 10.0, th.amber_bg);
                cv.text(Rect::new(r.x + 10.0, r.y + 6.0, r.w - 20.0, r.h - 12.0), text, Style::new(12.0, 400).wrap(), th.amber);
            }
            PKind::Check { control, label, checked } => {
                let key = Tgt::Ctl(*control).key();
                self.tick_box(cv, th, Rect::new(r.x, r.y + 1.0, 16.0, 16.0), *checked, self.fx.hov(key), 1.0, self.focus_ring(*control));
                let lab = format!("{}", label.trim_start());
                cv.text(Rect::new(r.x + 16.0 + 6.0 + 3.0, r.y, r.w - 25.0, 18.0), &lab, Style::new(12.0, 400), th.text2);
                let hit = Rect::new(r.x, r.y, 16.0 + 9.0 + cv.text_width(&lab, Style::new(12.0, 400)), 18.0);
                hits.push((hit, Tgt::Ctl(*control)));
            }
            PKind::Progress { elev } => self.progress(cv, th, r, *elev, cx),
            PKind::Shield => {
                let k = if self.reduced { 0.0 } else { 0.5 - 0.5 * ((cx.now_ms % 1600.0) / 1600.0 * std::f64::consts::TAU).cos() as f32 };
                let s = 1.0 + 0.12 * k;
                cv.with_opacity(0.75 + 0.25 * k, |cv| cv.with_scale_about(r.center(), s, |cv| self.icons.draw16(cv, "shield", SHIELD, 1.3, r, th.amber)));
            }
            PKind::Done => {
                let big = r;
                self.icons.draw16(cv, "ok", "<circle cx=\"8\" cy=\"8\" r=\"6.5\"/><path d=\"M5 8.4l2.1 2.1L11 6\"/>", 1.2, big, th.green);
            }
            PKind::ScanLine { label, found, stage } => {
                let st = Style::new(12.0, 400);
                let n = format!("{found}");
                let nb = Style::new(12.0, 600);
                let mut x = r.x;
                cv.text(Rect::new(x, r.y, 200.0, 16.0), label, st, th.dim);
                x += cv.text_width(label, st);
                cv.text(Rect::new(x, r.y, 80.0, 16.0), &n, nb, th.text);
                x += cv.text_width(&n, nb);
                cv.text(Rect::new(x, r.y, 24.0, 16.0), " \u{b7} ", st, th.dim);
                x += cv.text_width(" \u{b7} ", st);
                let l = self.anim.look(&AnimTarget::ScanStage);
                with_look(cv, &l, Rect::new(x, r.y, 300.0, 16.0), |cv| cv.text(Rect::new(x, r.y, r.right() - x, 16.0), stage, st, th.text2));
            }
            PKind::Link(b) => {
                let key = Tgt::Ctl(b.control).key();
                let col = th.accent.fade(1.0 - 0.25 * self.fx.hov(key));
                cv.text(r, &b.label, Style::new(12.0, 400), col);
                cv.fill_rect(Rect::new(r.x, r.y + 14.0, r.w, 1.0), col);
                hits.push((r, Tgt::Ctl(b.control)));
            }
            PKind::Buttons(bs) => {
                for (br, b) in bs {
                    let br2 = br.offset(r.x, r.y);
                    self.ubtn(cv, th, br2, b, busy);
                    if b.enabled && !busy {
                        hits.push((br2, Tgt::Ctl(b.control)));
                    }
                }
            }
            PKind::Group(g) => self.group_header(cv, th, r, g, hits),
            PKind::Item(row) => self.item_row(cv, th, r, row, hits),
            PKind::Delete { text } => {
                let l = self.anim.look(&AnimTarget::DeleteProgress);
                with_look(cv, &l, r, |cv| {
                    self.progress(cv, th, Rect::new(r.x, r.y, r.w, 3.0), false, cx);
                    cv.text(Rect::new(r.x, r.y + 3.0 + 6.0 + 8.0, r.w, 16.0), text, Style::new(12.0, 400), th.dim);
                });
            }
            PKind::Footer { selected, buttons } => {
                cv.fill_rect(r, th.raised);
                if let Some(s) = selected {
                    let st = Style::new(12.0, 400);
                    let left = buttons.first().map_or(r.right() - 16.0, |b| r.x + 16.0 + b.0.x - 16.0 - 8.0);
                    let t = fit_text(cv, s, st, (left - (r.x + 16.0)).max(20.0));
                    cv.text(Rect::new(r.x + 16.0, r.y + 8.0, (left - r.x - 16.0).max(20.0), UBTN_H), &t, st, th.dim);
                }
                for (br, b) in buttons {
                    let br2 = br.offset(r.x + 16.0, r.y);
                    self.ubtn(cv, th, br2, b, busy);
                    if b.enabled && !busy {
                        hits.push((br2, Tgt::Ctl(b.control)));
                    }
                }
            }
        });
    }

    fn progress(&self, cv: &Canvas, th: &Theme, r: Rect, elev: bool, cx: &Cx) {
        let col = if elev { th.amber } else { th.accent };
        cv.fill_rr(r, 2.0, th.raised2);
        cv.with_clip_rr(r, 2.0, |cv| {
            let bw = r.w * L::PROGRESS_SLIDE_SHARE;
            if self.reduced {
                cv.fill_rr(Rect::new(r.x, r.y, r.w, r.h), 2.0, col.fade(0.5));
            } else {
                let p = ((cx.now_ms % 1300.0) / 1300.0) as f32;
                let e = curves::EASE_IN_OUT.at(p);
                let x = r.x + (-1.0 + 3.7 * e) * bw;
                cv.fill_rr(Rect::new(x, r.y, bw, r.h), 2.0, col);
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn tick_box(&self, cv: &Canvas, th: &Theme, r: Rect, checked: bool, hov: f32, draw: f32, ring: bool) {
        let c = r.center();
        cv.with_scale_about(c, if checked { 1.08 } else { 1.0 }, |cv| {
            if checked {
                cv.fill_rr(r, 5.0, th.accent);
                let k = r.w / 16.0;
                let pts = [(3.5, 8.5), (6.5, 11.5), (12.5, 4.5)];
                let total = 4.243 + 9.22;
                let mut left = total * draw.clamp(0.0, 1.0);
                for w in pts.windows(2) {
                    let (dx, dy): (f32, f32) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
                    let len = (dx * dx + dy * dy).sqrt();
                    let t = (left / len).min(1.0);
                    if t <= 0.0 {
                        break;
                    }
                    let a = (r.x + w[0].0 * k, r.y + w[0].1 * k);
                    let b = (a.0 + (w[1].0 - w[0].0) * k * t, a.1 + (w[1].1 - w[0].1) * k * t);
                    cv.line(a, b, 2.0 * k, th.bg);
                    left -= len;
                }
            } else {
                cv.stroke_rr(r, 5.0, 1.5, th.faint.mix(th.dim, hov));
            }
        });
        if ring {
            cv.focus_ring(r, 5.0, th.accent, 1.0);
        }
    }

    fn ubtn(&self, cv: &Canvas, th: &Theme, r: Rect, b: &Button, panel_busy: bool) {
        let key = Tgt::Ctl(b.control).key();
        let disabled = !b.enabled || panel_busy;
        let hov = if disabled { 0.0 } else { self.fx.hov(key) };
        let prs = if disabled { 0.0 } else { self.fx.prs(key) };
        let (bg, fg) = match b.style {
            ButtonStyle::Danger => (th.red_bg, th.red),
            ButtonStyle::Ghost => (Color::rgba(0.0, 0.0, 0.0, 0.0), th.dim.mix(th.text, 0.35 * hov)),
            ButtonStyle::Plain if b.default_focus => (th.accent_soft, th.accent),
            ButtonStyle::Plain => (th.raised2, th.text),
        };
        let lift = -1.0 * hov;
        cv.with_opacity(if disabled { 0.4 } else { 1.0 }, |cv| {
            cv.with_scale_about(r.center(), 1.0 - 0.04 * prs, |cv| {
                cv.with_offset(0.0, lift, |cv| {
                    cv.fill_rr(r, L::BUTTON_RADIUS, bg);
                    cv.fill_rr(r, L::BUTTON_RADIUS, Color::rgba(1.0, 1.0, 1.0, if th.dark { 0.07 } else { 0.25 } * hov));
                    let label = b.label.trim_start();
                    let st = Style::new(12.0, 400);
                    let tw = cv.text_width(label, st);
                    let mut x = r.x + (r.w - tw - if b.icon.is_some() { 20.0 } else { 0.0 }) / 2.0;
                    if b.icon.is_some() {
                        self.icons.draw16(cv, "shield", SHIELD, 1.5, Rect::new(x, r.cy() - 7.0, 14.0, 14.0), fg);
                        x += 20.0;
                    }
                    cv.text(Rect::new(x, r.y, tw + 4.0, r.h), label, st, fg);
                });
            });
            if self.focus_ring(b.control) {
                cv.focus_ring(r, L::BUTTON_RADIUS, th.accent, 1.0);
            }
        });
    }

    fn group_header(&self, cv: &Canvas, th: &Theme, r: Rect, g: &GroupView, hits: &mut Vec<(Rect, Tgt)>) {
        let key = Tgt::Ctl(Control::ToggleGroup(g.index)).key();
        let hov = self.fx.hov(key);
        cv.fill_rr(r, 8.0, th.raised2.fade(hov));
        let open = self.chev.get(&g.index).map_or(if g.open { 1.0 } else { 0.0 }, |t| t.value());
        let ci = Rect::new(r.x + 2.0, r.cy() - 7.0, 14.0, 14.0);
        cv.with_transform(super::paint::rot_scale_about(ci.center(), 1.0, -90.0 * (1.0 - open)), |cv| self.icons.draw16(cv, "chev", "<path d=\"M5 6l3 3 3-3\"/>", 1.6, ci, th.dim));
        let tx = r.x + 2.0 + 14.0 + 6.0;
        let st = Style::new(12.0, 600);
        cv.text(Rect::new(tx, r.y, 300.0, r.h), &g.title, st, th.text);
        let mut x = tx + cv.text_width(&g.title, st) + 6.0;
        let n = format!("{}", g.count);
        cv.text(Rect::new(x, r.y, 60.0, r.h), &n, st, th.dim);
        x += cv.text_width(&n, st) + 6.0;
        let sz = g.size_text.trim_start_matches(|c: char| c == ' ' || c == '\u{b7}').trim_start().to_string();
        if !sz.is_empty() {
            cv.text(Rect::new(x - 3.0, r.y, 160.0, r.h), &format!("\u{b7} {sz}"), Style::new(12.0, 400), th.dim);
        }
        hits.push((r, Tgt::Ctl(Control::ToggleGroup(g.index))));
        if self.focus_ring(Control::ToggleGroup(g.index)) {
            cv.focus_ring(r, 8.0, th.accent, 1.0);
        }
    }

    fn item_row(&self, cv: &Canvas, th: &Theme, r: Rect, row: &ItemRowView, hits: &mut Vec<(Rect, Tgt)>) {
        let key = Tgt::Ctl(Control::ToggleItem(row.pick)).key();
        let hov = self.fx.hov(key);
        let prs = self.fx.prs(key);
        let rl = self.anim.look(&AnimTarget::ChecklistRow(row.pick));
        let k = rl.alpha;
        let shake = self.anim.look(&AnimTarget::ChecklistRow(row.pick));
        let rr = r.offset(shake.dx, 0.0);
        cv.with_clip(Rect::new(r.x - 20.0, r.y, r.w + 40.0, r.h.max(0.0)), |cv| {
            cv.with_opacity(k, |cv| {
                cv.with_scale_about(rr.center(), 1.0 - 0.01 * prs, |cv| {
                    if row.failed {
                        cv.fill_rr(rr, 10.0, th.red_bg);
                    } else {
                        cv.fill_rr(rr, 10.0, th.raised2.fade(hov));
                    }
                    let tick = self.anim.look(&AnimTarget::Tick(row.pick));
                    self.tick_box(cv, th, Rect::new(rr.x + 6.0, rr.y + 5.0, 16.0, 16.0), row.checked, hov, tick.draw, self.focus_ring(Control::ToggleItem(row.pick)));
                    let tx = rr.x + 6.0 + 16.0 + 10.0;
                    let w = rr.w - (tx - rr.x) - 6.0;
                    let n = lines_of(cv, &row.path, 12.0, 400, w);
                    cv.text(Rect::new(tx, rr.y + 4.0, w, 16.0 * n), &row.path, Style::new(12.0, 400).wrap(), th.text2);
                    cv.text(Rect::new(tx, rr.y + 4.0 + 16.0 * n, w, rr.h - 8.0 - 16.0 * n), &row.meta, Style::new(11.0, 400).wrap(), if row.failed { th.red } else { th.faint });
                })
            })
        });
        if !row.gone {
            hits.push((r, Tgt::Ctl(Control::ToggleItem(row.pick))));
        }
    }

    // ---- the status strip ----

    pub(super) fn paint_strip(&mut self, cv: &Canvas, th: &Theme, cx: &Cx) {
        let (Some(s), Some(sv)) = (self.lay.strip, self.vm.uninstall.strip.clone()) else { return };
        let look = self.anim.look(&AnimTarget::Strip);
        let key = Tgt::Strip.key();
        let hov = self.fx.hov(key);
        let prs = self.fx.prs(key);
        let label = self.anim.look(&AnimTarget::StripLabel);
        let pop = self.anim.look(&AnimTarget::Strip);
        with_look(cv, &look, s, |cv| {
            cv.with_scale_about(s.center(), (1.0 - 0.015 * prs) * pop.sx, |cv| {
                cv.fill_rr(s, L::STRIP_RADIUS, th.raised.mix(th.raised2, hov));
                if pop.sx > 1.0 || pop.sx < 1.0 {
                    cv.stroke_rr(s, L::STRIP_RADIUS, 2.0, th.green.fade((1.0 - pop.sx).abs() * 8.0));
                }
                cv.with_clip_rr(s, L::STRIP_RADIUS, |cv| self.strip_body(cv, th, s, &sv, &label, cx));
            })
        });
        self.hits.push((s, Tgt::Strip));
    }

    fn strip_body(&self, cv: &Canvas, th: &Theme, s: Rect, sv: &StripView, label: &Look, cx: &Cx) {
        let tone = match sv.tone {
            StripTone::Accent => th.accent,
            StripTone::Green => th.green,
            StripTone::Amber => th.amber,
        };
        let ic = Rect::new(s.x + 12.0, s.y + 8.0, 16.0, 16.0);
        match sv.icon {
            StripIcon::Spinner => {
                let rot = if self.reduced { 0.0 } else { ((cx.now_ms % 900.0) / 900.0 * 360.0) as f32 };
                cv.arc(ic.center(), 6.0, 1.5, rot, 229.0, tone);
            }
            StripIcon::Check => self.icons.draw16(cv, "s-ok", "<circle cx=\"8\" cy=\"8\" r=\"6.5\"/><path d=\"M5 8.4l2.1 2.1L11 6\"/>", 1.5, ic, tone),
            StripIcon::Warn => self.icons.draw16(cv, "s-warn", "<path d=\"M8 2l6.5 11.5h-13z\"/><path d=\"M8 6.5v3.2M8 11.6v.1\"/>", 1.5, ic, tone),
        }
        // the keycap
        let kst = Style::new(10.0, 600);
        let kw = cv.text_width(sv.shortcut, kst).ceil() + 12.0;
        let kr = Rect::new(s.right() - 12.0 - kw, s.y + 8.0, kw, 18.0);
        cv.fill_rr(kr, 6.0, th.raised2);
        cv.text(kr, sv.shortcut, kst.center(), th.dim);
        // name, message, count
        let tx = ic.right() + 10.0;
        let max_w = kr.x - 10.0 - tx;
        with_look(cv, label, Rect::new(tx, s.y + 8.0, max_w, 16.0), |cv| {
            let nst = Style::new(12.0, 600);
            let mst = Style::new(12.0, 400);
            let name = fit_text(cv, &sv.name, nst, max_w * 0.5);
            let nw = cv.text_width(&name, nst);
            cv.text(Rect::new(tx, s.y + 8.0, nw + 3.0, 16.0), &name, nst, th.text);
            let room = max_w - nw;
            let count = sv.count.map(|c| c.to_string()).unwrap_or_default();
            let cw = if count.is_empty() { 0.0 } else { cv.text_width(&count, nst) };
            let msg = fit_text(cv, &sv.message, mst, (room - cw).max(0.0));
            cv.text(Rect::new(tx + nw, s.y + 8.0, room, 16.0), &msg, mst, th.text2);
            if !count.is_empty() {
                cv.text(Rect::new(tx + nw + cv.text_width(&msg, mst), s.y + 8.0, cw + 4.0, 16.0), &count, nst, th.text);
            }
        });
        // the running bar along the bottom
        if sv.bar {
            let tr = Rect::new(s.x + 12.0, s.bottom() - 3.0 - 2.0, s.w - 24.0, 2.0);
            cv.fill_rr(tr, 1.0, th.raised2);
            cv.with_clip(tr, |cv| {
                if self.reduced {
                    cv.fill_rect(tr, th.accent.fade(0.5));
                } else {
                    let bw = tr.w * L::PROGRESS_SLIDE_SHARE;
                    let p = ((cx.now_ms % 1300.0) / 1300.0) as f32;
                    let x = tr.x + (-1.0 + 3.7 * curves::EASE_IN_OUT.at(p)) * bw;
                    cv.fill_rr(Rect::new(x, tr.y, bw, tr.h), 1.0, th.accent);
                }
            });
        }
        let _ = Align::Left;
    }
}

const SHIELD: &str = "<path d=\"M8 1.5l5.5 2v4.2c0 3.3-2.3 5.6-5.5 6.8-3.2-1.2-5.5-3.5-5.5-6.8V3.5z\"/>";

fn clip_to(r: Rect, to: Rect) -> Option<Rect> {
    let (x0, y0) = (r.x.max(to.x), r.y.max(to.y));
    let (x1, y1) = (r.right().min(to.right()), r.bottom().min(to.bottom()));
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}
