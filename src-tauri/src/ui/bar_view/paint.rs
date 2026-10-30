//! Drawing the bar: theme, flex-style layout of the body, search row, answer, dialogue hint, result list, message line, context menu and the liquid
//! outline. Colours and sizes are the CSS of `bar.css` / `theme.css` (through glasskit's `Theme` and the model's `layout` constants).
use super::anim::Look;
use super::*;
use glasskit::canvas::{mul, scale_about, translation};
use glasskit::color::Color;
use glasskit::markdown;
use glasskit::richtext::MarkdownLabels;
use glasskit::svgpath::{build_geometry, Affine, Path, Seg};
use glasskit::text::Style;
use pill_ui_core::bar::model::MicState;
use pill_ui_core::bar::query::Seg as Piece;
use pill_ui_core::bar::results::RowView;
use pill_ui_core::bar::wave;
use windows::Foundation::Numerics::Matrix3x2;

const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

/// `scale(s) rotate(deg)` about `c`.
pub(crate) fn rot_scale_about(c: Pt, s: f32, deg: f32) -> Matrix3x2 {
    let (sn, cs) = deg.to_radians().sin_cos();
    let (m11, m12, m21, m22) = (s * cs, s * sn, -s * sn, s * cs);
    Matrix3x2 { M11: m11, M12: m12, M21: m21, M22: m22, M31: c.0 - c.0 * m11 - c.1 * m21, M32: c.1 - c.0 * m12 - c.1 * m22 }
}

/// Applies an animation `Look` around the rectangle `r` (opacity, move, scale, rotation) and draws `f` inside.
pub(crate) fn with_look(cv: &Canvas, l: &Look, r: Rect, f: impl FnOnce(&Canvas)) {
    if l.alpha <= 0.001 {
        return;
    }
    let c = r.center();
    let tf = |cv: &Canvas, f: Box<dyn FnOnce(&Canvas) + '_>| {
        if l.sx == 1.0 && l.sy == 1.0 && l.rot_deg == 0.0 && l.dx == 0.0 && l.dy == 0.0 {
            f(cv)
        } else {
            let m = mul(&mul(&rot_scale_about(c, 1.0, l.rot_deg), &scale_about(c, l.sx, l.sy)), &translation(l.dx, l.dy));
            cv.with_transform(m, |cv| f(cv))
        }
    };
    cv.with_opacity(l.alpha, |cv| tf(cv, Box::new(f)));
}

/// `text`, cut with an ellipsis so that it is at most `max_w` wide in style `st`.
pub(crate) fn fit_text(cv: &Canvas, s: &str, st: Style, max_w: f32) -> String {
    if cv.text_width(s, st) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let t: String = chars[..mid].iter().collect::<String>() + "\u{2026}";
        if cv.text_width(&t, st) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_string() + "\u{2026}"
}

/// Draws query-highlighted title pieces from `x`, at most `max_w` wide; returns the width used.
fn marked(cv: &Canvas, x: f32, y: f32, h: f32, max_w: f32, segs: &[Piece], base: Style, mark_w: u16, col: Color) -> f32 {
    let mut used = 0.0;
    for s in segs {
        if s.text.is_empty() {
            continue;
        }
        let st = Style { weight: if s.mark { mark_w } else { base.weight }, ..base };
        let room = max_w - used;
        if room <= 1.0 {
            break;
        }
        let t = fit_text(cv, &s.text, st, room);
        let w = cv.text_width(&t, st);
        cv.text(Rect::new(x + used, y, w + 3.0, h), &t, st, col);
        used += w;
        if t != s.text {
            break;
        }
    }
    used
}

impl BarView {
    pub(super) fn resolve_theme(&self, base: &Theme) -> Theme {
        match self.vm.theme.as_str() {
            "light" => Theme::pill_light(),
            "auto" => {
                if self.light_os {
                    Theme::pill_light()
                } else {
                    Theme::pill_dark()
                }
            }
            "dark" => Theme::pill_dark(),
            _ => *base,
        }
    }

    pub(super) fn paint(&mut self, cv: &Canvas, base: &Theme, _rect: Rect, cx: &Cx) {
        self.now = cx.now_ms;
        self.hits.clear();
        let th = self.resolve_theme(base);
        if self.vm.parked && !self.anim.busy() && !self.vm.visible {
            return; // `.body.off`: transparent between two openings
        }
        self.measure(cv, &th);
        self.lay = self.compute_lay();
        self.sync_scrolls();
        let look = self.anim.look(&AnimTarget::Body);
        let body = self.lay.body;
        let origin = (body.cx(), 0.0);
        let m = mul(&scale_about(origin, look.sx, look.sy), &translation(look.dx, look.dy));
        cv.with_opacity(look.alpha, |cv| {
            cv.with_transform(m, |cv| {
                if !th.dark {
                    cv.shadow_rr(body, L::BODY_RADIUS, 30.0, 10.0, th.shadow);
                }
                cv.fill_rr(body, L::BODY_RADIUS, th.bg);
                cv.with_clip_rr(body, L::BODY_RADIUS, |cv| self.paint_content(cv, &th, cx));
                self.paint_wave(cv, &th, body);
            });
        });
        self.paint_context(cv, &th);
        self.hits.push((body, Tgt::Body));
        self.do_scroll();
    }

    // ---- measuring and layout ----

    fn measure(&mut self, cv: &Canvas, th: &Theme) {
        let kbd_st = Style::new(11.0, 600);
        self.m.kbd_w = (cv.text_width(&self.vm.hotkey, kbd_st) + 16.0).ceil();
        // the answer
        match &self.vm.answer {
            None => {
                self.m.ans_content = 0.0;
                self.md.view = None;
                self.md.src.clear();
            }
            Some(a) => {
                let w = L::WINDOW_W - 2.0 * L::ANSWER_MARGIN.1 - 2.0 * L::ANSWER_PAD.1;
                let copied = if self.bar.lang() == pill_ui_core::bar::Lang::Ru { "Скопировано" } else { "Copied" };
                let labels = (a.copy_code_label.clone(), copied.to_string());
                let mut md_th = *th;
                md_th.card = th.raised2; // the code frame of the web: `--raised-2` on the `--raised` answer
                let stale = self.md.view.is_none() || self.md.width != w || self.md.dark != th.dark || self.md.labels != labels;
                if a.text.is_empty() {
                    self.md.view = None;
                    self.md.src.clear();
                } else if stale || self.md.src != a.text {
                    let blocks = markdown::parse(&a.text);
                    let l = MarkdownLabels { copy: &labels.0, copied: &labels.1 };
                    let ok = match (stale, self.md.view.as_mut()) {
                        (false, Some(v)) => v.update(cv, blocks, &md_th).is_ok(),
                        _ => match MarkdownView::new(cv, blocks, w, &md_th, &l) {
                            Ok(v) => {
                                self.md.view = Some(v);
                                true
                            }
                            Err(_) => false,
                        },
                    };
                    if ok {
                        self.md.src = a.text.clone();
                        self.md.width = w;
                        self.md.dark = th.dark;
                        self.md.labels = labels;
                    }
                }
                let md_h = self.md.view.as_ref().map_or(0.0, |v| v.height());
                self.m.ans_content = if a.thinking { 20.0 + if md_h > 0.0 { 8.0 + md_h } else { 0.0 } } else { md_h };
            }
        }
        // the uninstall panel
        self.m.panel_content = 0.0;
        self.panel_frame = None;
        if let Some(p) = self.vm.uninstall.panel.clone() {
            let f = self.build_panel(cv, th, &p, L::WINDOW_W - 2.0 * L::PANEL_MARGIN.1 - 2.0 * L::PANEL_PAD.1);
            self.m.panel_content = f.content_h;
            self.panel_frame = Some(f);
        }
        // the message line wraps
        self.m.msg_h = 0.0;
        if let Some(msg) = &self.vm.message {
            let w = L::WINDOW_W - 2.0 * L::MESSAGE_PAD.1;
            let (_, h) = cv.measure(&msg.text, Style::new(12.0, 400).wrap(), w);
            let lines = (h / 15.0).round().max(1.0);
            self.m.msg_h = L::MESSAGE_PAD.0 + L::MESSAGE_PAD.2 + 18.0 * lines;
        }
    }

    pub(super) fn compute_lay(&self) -> Lay {
        let vm = &self.vm;
        let u = &vm.uninstall;
        let mut lay = Lay::default();
        // search row
        let kbd_w = if self.m.kbd_w > 0.0 { self.m.kbd_w } else { 120.0 };
        lay.input = default_input_rect(kbd_w);
        lay.kbd = Rect::new(L::WINDOW_W - L::SEARCH_PAD.1 - kbd_w, (L::SEARCH_H - 22.0) / 2.0, kbd_w, 22.0);
        lay.mic = Rect::new(lay.kbd.x - 10.0 - L::MIC_SIZE, (L::SEARCH_H - L::MIC_SIZE) / 2.0, L::MIC_SIZE, L::MIC_SIZE);
        // flex children
        let has_ans = vm.answer.is_some();
        let has_dlg = vm.dialog_hint.is_some();
        let has_panel = u.panel.is_some();
        let n_rows = self.entries.len();
        let list_shown = !u.list_hidden && n_rows > 0;
        let strip_h = if u.strip.is_some() { 34.0 + L::PANEL_MARGIN.1 } else { 0.0 };
        let msg_h = if vm.message.is_some() { self.m.msg_h.max(36.0) } else { 0.0 };
        let fixed = L::SEARCH_H + if has_dlg { 22.0 } else { 0.0 } + strip_h + msg_h + if has_ans { L::ANSWER_MARGIN.1 } else { 0.0 } + if has_panel { L::PANEL_MARGIN.1 } else { 0.0 };
        let ans_basis = match &vm.answer {
            Some(a) if a.thinking && self.m.ans_content <= 20.0 => 2.0 * L::ANSWER_PAD.0 + 20.0,
            Some(_) => (self.m.ans_content + 2.0 * L::ANSWER_PAD.0).min(if self.md.view.is_some() { L::ANSWER_MD_MAX_H } else { L::ANSWER_MAX_H }),
            None => 0.0,
        };
        let panel_basis = if has_panel { (self.m.panel_content + L::PANEL_PAD.0).min(L::PANEL_MAX_H) } else { 0.0 };
        let src = Rows { entries: &self.entries };
        let list_content: f32 = (0..n_rows).map(|i| src.height(i, 548.0)).sum();
        let list_basis = if list_shown { list_content + L::LIST_PAD.1 } else { 0.0 };
        let sizes = flow(fixed, &[(ans_basis, true), (panel_basis, true), (list_basis, true)], L::BODY_MAX_H);
        let mut y = L::SEARCH_H;
        if has_ans {
            lay.ans = Some(Rect::new(L::ANSWER_MARGIN.0, y, L::WINDOW_W - 2.0 * L::ANSWER_MARGIN.0, sizes[0]));
            y += sizes[0] + L::ANSWER_MARGIN.1;
        }
        if has_dlg {
            lay.dlg = Some(Rect::new(0.0, y, L::WINDOW_W, 22.0));
            y += 22.0;
        }
        if has_panel {
            lay.panel = Some(Rect::new(L::PANEL_MARGIN.0, y, L::WINDOW_W - 2.0 * L::PANEL_MARGIN.0, sizes[1]));
            y += sizes[1] + L::PANEL_MARGIN.1;
        }
        if list_shown {
            lay.list = Some(Rect::new(L::LIST_PAD.0, y, L::WINDOW_W - 2.0 * L::LIST_PAD.0, (sizes[2] - L::LIST_PAD.1).max(0.0)));
            y += sizes[2];
        }
        if u.strip.is_some() {
            lay.strip = Some(Rect::new(L::PANEL_MARGIN.0, y, L::WINDOW_W - 2.0 * L::PANEL_MARGIN.0, 34.0));
            y += strip_h;
        }
        if vm.message.is_some() {
            lay.msg = Some(Rect::new(0.0, y, L::WINDOW_W, msg_h));
            y += msg_h;
        }
        lay.body = Rect::new(0.0, 0.0, L::WINDOW_W, y.min(L::BODY_MAX_H));
        lay
    }

    /// Scroll metrics of the list, answer and panel follow the layout.
    fn sync_scrolls(&mut self) {
        if let Some(v) = self.lay.list {
            let entries = std::mem::take(&mut self.entries);
            self.list.update(&Rows { entries: &entries }, v);
            self.entries = entries;
        }
        if let Some(a) = self.lay.ans {
            let inner_h = (a.h - 2.0 * L::ANSWER_PAD.0).max(0.0);
            self.ans_scroll.st.set_metrics(self.m.ans_content, inner_h);
        } else {
            self.ans_scroll.st.set_metrics(0.0, 0.0);
        }
        if let Some(p) = self.lay.panel {
            self.uni_scroll.st.set_metrics(self.m.panel_content + L::PANEL_PAD.0, p.h);
        } else {
            self.uni_scroll.st.set_metrics(0.0, 0.0);
        }
    }

    // ---- the body ----

    fn paint_content(&mut self, cv: &Canvas, th: &Theme, cx: &Cx) {
        self.paint_search(cv, th, cx);
        if let Some(a) = self.lay.ans {
            let l = self.anim.look(&AnimTarget::Answer);
            with_look(cv, &l, a, |cv| self.paint_answer(cv, th, a, cx));
            self.hits.push((a, Tgt::Answer));
        }
        if let Some(d) = self.lay.dlg {
            if let Some(t) = &self.vm.dialog_hint {
                cv.text(Rect::new(26.0, d.y, 508.0, 16.0), t, Style::new(12.0, 400), th.faint);
            }
        }
        if self.lay.panel.is_some() {
            self.paint_panel(cv, th, cx);
        }
        if let Some(v) = self.lay.list {
            let l = self.anim.look(&AnimTarget::List);
            if !l.is_rest() || !self.vm.uninstall.list_hidden {
                with_look(cv, &l, v, |cv| self.paint_list(cv, th, v));
            }
        }
        if self.lay.strip.is_some() {
            self.paint_strip(cv, th, cx);
        }
        if let Some(m) = self.lay.msg {
            self.paint_message(cv, th, m);
        }
    }

    fn paint_search(&mut self, cv: &Canvas, th: &Theme, cx: &Cx) {
        let (kbd, mic, f) = (self.lay.kbd, self.lay.mic, self.lay.input);
        self.icons.draw(cv, "search", 1.8, Rect::new(L::SEARCH_PAD.0, (L::SEARCH_H - 18.0) / 2.0, 18.0, 18.0), th.dim);
        // the text box: glasskit paints a field with a focus ring at its edge; the box is laid out 4 px bigger and clipped so only text, caret and selection show
        let mut it = *th;
        it.field = TRANSPARENT;
        let rect = self.input_rect();
        let input = &self.input;
        cv.with_clip(f.inset_xy(-2.0, -2.0), |cv| input.draw(cv, rect, &it));
        // microphone
        let mk = Tgt::Mic.key();
        let hov = self.fx.hov(mk);
        let rec = self.vm.mic.state == MicState::Recording;
        let busy = self.vm.mic.state == MicState::Transcribing;
        let press = self.fx.prs(mk);
        cv.with_scale_about(mic.center(), 1.0 - 0.04 * press, |cv| {
            if rec {
                cv.fill_circle(mic.center(), 14.0, th.red_bg);
            } else {
                cv.fill_circle(mic.center(), 14.0, th.raised.fade(hov));
            }
            let col = if rec { th.red } else { th.dim.mix(th.text, hov) };
            cv.with_opacity(if busy { 0.6 } else { 1.0 }, |cv| self.icons.draw(cv, "mic", 1.7, mic.centered(18.0, 18.0), if busy { th.text } else { col }));
            if rec {
                let p = if self.reduced { 0.0 } else { ((cx.now_ms % 1100.0) / 1100.0) as f32 };
                let k = 0.5 - 0.5 * (p * std::f32::consts::TAU).cos(); // ease-in-out between 1 and .35 / 1 and 1.35
                let dot = (mic.right() - 2.0 - 3.5, mic.y + 2.0 + 3.5);
                cv.with_opacity(1.0 - 0.65 * k, |cv| cv.fill_circle(dot, 3.5 * (1.0 + 0.35 * k), th.red));
            }
        });
        self.hits.push((mic, Tgt::Mic));
        // the hotkey keycap
        cv.fill_rr(kbd, 7.0, th.raised);
        cv.text(kbd, &self.vm.hotkey, Style::new(11.0, 600).center(), th.dim);
        // the whole row left of the microphone is the text box
        self.hits.push((Rect::new(0.0, 0.0, mic.x, L::SEARCH_H), Tgt::Input));
        self.hits.push((mic, Tgt::Mic));
    }

    fn paint_answer(&mut self, cv: &Canvas, th: &Theme, a: Rect, cx: &Cx) {
        cv.fill_rr(a, L::ANSWER_RADIUS, th.raised);
        let inner = Rect::new(a.x + L::ANSWER_PAD.1, a.y + L::ANSWER_PAD.0, a.w - 2.0 * L::ANSWER_PAD.1, (a.h - 2.0 * L::ANSWER_PAD.0).max(0.0));
        let Some(ans) = self.vm.answer.clone() else { return };
        let off = self.ans_scroll.st.offset();
        cv.with_clip_rr(a, L::ANSWER_RADIUS, |cv| {
            cv.with_clip(inner, |cv| {
                let mut y = inner.y - off;
                if ans.thinking {
                    let c = (inner.x + 7.0, y + 10.0);
                    cv.stroke_circle(c, 6.0, 2.0, th.raised2);
                    let rot = if self.reduced { 0.0 } else { ((cx.now_ms % 800.0) / 800.0 * 360.0) as f32 };
                    cv.arc(c, 6.0, 2.0, -45.0 + rot, 90.0, th.text);
                    cv.text(Rect::new(inner.x + 26.0, y, inner.w - 26.0, 20.0), &ans.thinking_label, Style::new(14.0, 400), th.dim);
                    y += 28.0;
                }
                if let Some(md) = self.md.view.as_ref() {
                    let mut md_th = *th;
                    md_th.card = th.raised2;
                    md.draw(cv, (inner.x, y), &md_th);
                }
            })
        });
        self.ans_scroll.bar.draw(cv, a, &self.ans_scroll.st, th);
    }

    // ---- the result list ----

    fn paint_list(&mut self, cv: &Canvas, th: &Theme, v: Rect) {
        let off = self.list.state.offset();
        let vis = self.list.visible_range();
        let sel = self.vm.selected;
        let sel_confirm = self.vm.rows.get(sel).is_some_and(|r| r.confirm);
        let mut hits: Vec<(Rect, Tgt)> = Vec::new();
        cv.with_clip(v, |cv| {
            // the highlight glides under the rows
            if !self.vm.rows.is_empty() && !sel_confirm && sel < self.vm.rows.len() {
                let y = v.y + self.ind.value() - off;
                cv.fill_rr(Rect::new(v.x, y, v.w, L::ROW_H), L::ROW_RADIUS, th.raised);
            }
            for ei in vis {
                let Some(e) = self.entries.get(ei).cloned() else { continue };
                let top = v.y + self.list.row_top(ei) - off;
                let h = self.list.row_height(ei);
                let r = Rect::new(v.x, top, v.w, h);
                match e {
                    Entry::Row(i) => {
                        if let Some(row) = self.vm.rows.get(i) {
                            hits.push((r, Tgt::Row(i)));
                            hits.extend(self.paint_row(cv, th, row, r));
                        }
                    }
                    Entry::Tip(t) => {
                        cv.text(Rect::new(r.x + TIP_PAD.1, r.y + TIP_PAD.0, r.w - 2.0 * TIP_PAD.1, r.h - 2.0 * TIP_PAD.0), &t, Style::new(13.0, 400).wrap(), th.dim);
                    }
                    Entry::Hint(t) => {
                        let b = Rect::new(r.x, r.y + 4.0, r.w, r.h - 12.0);
                        cv.fill_rr(b, 12.0, th.amber_bg);
                        cv.text(Rect::new(b.x + 18.0, b.y + 8.0, b.w - 36.0, b.h - 16.0), &t, Style::new(12.0, 400).wrap(), th.amber);
                    }
                }
            }
        });
        self.list.bar.draw(cv, v, &self.list.state, th);
        // the rows are clickable only inside the viewport
        for (r, t) in hits {
            if let Some(c) = clip_rect(r, v) {
                self.hits.push((c, t));
            }
        }
    }

    /// One result row. Returns the hit rectangles of its hover buttons.
    fn paint_row(&self, cv: &Canvas, th: &Theme, row: &RowView, rect: Rect) -> Vec<(Rect, Tgt)> {
        let i = row.index;
        let look = self.anim.look(&AnimTarget::Row(i));
        let off = self.row_off.get(&i).map_or(0.0, |t| t.value());
        let press = self.fx.prs(Tgt::Row(i).key());
        let mut hits = Vec::new();
        let sel = row.selected;
        let moved = rect.offset(look.dx, look.dy + off);
        cv.with_opacity(look.alpha, |cv| {
            cv.with_scale_about(moved.center(), 1.0 - 0.01 * press, |cv| {
                let r = moved;
                let cy = r.cy();
                if row.confirm {
                    cv.fill_rr(r, L::ROW_RADIUS, th.red_bg);
                }
                if look.flash > 0.001 {
                    cv.fill_rr(r, L::ROW_RADIUS, th.accent_soft.fade(look.flash));
                }
                let uninstall = row.kind == pill_ui_core::bar::protocol::Kind::Uninstall;
                let k = if sel && uninstall { curves::EASE_OUT.at((self.sel_age / 0.38).min(1.0)) } else { 0.0 };
                // icon column
                let ix = r.x + L::ROW_PAD.0;
                let ico = Rect::new(ix, cy - 9.0, L::ROW_ICON, L::ROW_ICON);
                let drew = row.shell_icon.as_deref().is_some_and(|u| {
                    self.shell.draw(cv, u, Rect::new(ix - 2.0, cy - 11.0, L::ROW_SHELL_ICON, L::ROW_SHELL_ICON), 1.0)
                });
                if !drew {
                    let tint = th.text2.mix(th.red, k);
                    cv.with_transform(rot_scale_about(ico.center(), 1.0 + 0.14 * k, -6.0 * k), |cv| self.icons.draw(cv, &row.icon, 1.6, ico, tint));
                }
                // right side: label + enter icon, buttons
                let right = r.right() - L::ROW_PAD.1;
                let enter = Rect::new(right - 14.0, cy - 7.0, 14.0, 14.0);
                let lab_st = Style::new(12.0, 400);
                let lab_w = if row.label.is_empty() { 0.0 } else { cv.text_width(&row.label, lab_st).min(200.0).ceil() };
                let act_left = if lab_w > 0.0 { enter.x - 10.0 - lab_w } else { enter.x };
                let lab_col = if row.confirm || (sel && uninstall) { th.red } else { th.dim };
                if lab_w > 0.0 {
                    let t = fit_text(cv, &row.label, lab_st, 200.0);
                    cv.text(Rect::new(act_left, cy - 10.0, lab_w + 2.0, 20.0), &t, lab_st, if sel && uninstall && !row.confirm { th.dim.mix(th.red, k) } else { lab_col });
                }
                if sel {
                    let a = curves::EASE_OUT.at((self.sel_age / 0.12).min(1.0));
                    self.icons.draw(cv, "enter", 1.7, enter, th.dim.fade(a));
                }
                let mut text_right = act_left - 12.0;
                if sel && !row.buttons.is_empty() {
                    let n = row.buttons.len() as f32;
                    let w = n * 26.0 + (n - 1.0) * 2.0;
                    let bx = act_left - 12.0 - w;
                    text_right = bx - 12.0;
                    for (b, a) in row.buttons.iter().enumerate() {
                        let br = Rect::new(bx + b as f32 * 28.0, cy - 13.0, 26.0, 26.0);
                        hits.push((br, Tgt::RowBtn(i, a.mode)));
                        let t = self.sel_age - 0.024 * b as f32;
                        let (alpha, pop) = if t <= 0.0 { (0.0, 0.5) } else { (curves::EASE_OUT.at((t / 0.22).min(1.0)), lerp(0.5, 1.0, curves::SPRING_POP.at((t / 0.4).min(1.0)))) };
                        let key = Tgt::RowBtn(i, a.mode).key();
                        let hov = self.fx.hov(key);
                        let pr = self.fx.prs(key);
                        let pin = if a.mode == RunMode::Pin { self.anim.look(&AnimTarget::PinButton(i)) } else { Look::REST };
                        cv.with_opacity(alpha, |cv| {
                            cv.with_scale_about(br.center(), pop * (1.0 - 0.14 * pr), |cv| {
                                cv.fill_rr(br, L::HOVER_BUTTON_RADIUS, th.raised2.fade(hov));
                                let ir = br.centered(16.0, 16.0);
                                with_look(cv, &pin, ir, |cv| self.icons.draw(cv, a.icon, 1.6, ir, th.dim.mix(th.text, hov)));
                            })
                        });
                    }
                }
                // title and subtitle
                let tx = r.x + L::ROW_PAD.0 + L::ROW_ICON + L::ROW_GAP;
                let avail = (text_right - tx).max(0.0);
                let (tst, th_h) = if row.big { (Style::new(16.0, 600), 24.0) } else { (Style::new(13.0, 400), 20.0) };
                let tw = marked(cv, tx, cy - th_h / 2.0, th_h, if row.big { avail } else { avail.max(0.0) }, &row.title, tst, 600, th.text);
                if row.show_sub && !row.sub.is_empty() {
                    let sx = tx + tw + 10.0;
                    let room = text_right - sx;
                    if room > 24.0 {
                        let st = Style::new(11.0, 400);
                        let t = fit_text(cv, &row.sub, st, room);
                        cv.text(Rect::new(sx, cy - 10.0, room, 20.0), &t, st, th.faint);
                    }
                }
            })
        });
        hits
    }

    // ---- message, context menu, wave ----

    fn paint_message(&mut self, cv: &Canvas, th: &Theme, m: Rect) {
        let Some(msg) = self.vm.message.clone() else { return };
        let l = self.anim.look(&AnimTarget::Message);
        let col = if msg.ok { th.green } else { th.red };
        with_look(cv, &l, m, |cv| {
            let r = Rect::new(L::MESSAGE_PAD.1, m.y + L::MESSAGE_PAD.0, m.w - 2.0 * L::MESSAGE_PAD.1, (m.h - L::MESSAGE_PAD.0 - L::MESSAGE_PAD.2).max(0.0));
            cv.text(Rect { h: r.h, ..r }, &msg.text, Style::new(12.0, 400).wrap(), col);
            if msg.link.is_some() {
                // `text-decoration: underline`
                let w = cv.text_width(&msg.text, Style::new(12.0, 400)).min(r.w);
                cv.fill_rect(Rect::new(r.x, r.y + 15.0, w, 1.0), col);
            }
        });
        if msg.link.is_some() {
            self.hits.push((m, Tgt::Msg));
        }
    }

    fn paint_context(&mut self, cv: &Canvas, th: &Theme) {
        let Some(c) = self.vm.context.clone() else { return };
        let st = Style::new(13.0, 400);
        let key_st = Style::new(10.0, 600);
        let mut w = L::MENU_MIN_W;
        for l in &c.lines {
            let need = 2.0 * L::MENU_PAD + 20.0 + 16.0 + 10.0 + cv.text_width(&l.label, st) + if l.key.is_empty() { 0.0 } else { 16.0 + cv.text_width(l.key, key_st) };
            w = w.max(need.ceil());
        }
        let h = c.lines.len() as f32 * L::MENU_LINE_H + 2.0 * L::MENU_PAD;
        let r = Rect::new(c.x, c.y, w, h);
        let look = self.anim.look(&AnimTarget::Context { item: None });
        let origin = (c.x + c.origin.0, c.y + c.origin.1);
        let m = mul(&scale_about(origin, look.sx, look.sy), &translation(0.0, 0.0));
        let mut hits = Vec::new();
        cv.with_opacity(look.alpha, |cv| {
            cv.with_transform(m, |cv| {
                cv.shadow_rr(r, L::MENU_RADIUS, 30.0, 10.0, Color::rgba(0.0, 0.0, 0.0, 0.55));
                cv.fill_rr(r, L::MENU_RADIUS, th.bg);
                cv.stroke_rr(r, L::MENU_RADIUS, 1.0, if th.dark { Color::rgba(1.0, 1.0, 1.0, 0.09) } else { Color::rgba(0.0, 0.0, 0.0, 0.10) });
                for (k, line) in c.lines.iter().enumerate() {
                    let ir = Rect::new(r.x + L::MENU_PAD, r.y + L::MENU_PAD + k as f32 * L::MENU_LINE_H, r.w - 2.0 * L::MENU_PAD, L::MENU_LINE_H);
                    hits.push((ir, Tgt::Ctx(k)));
                    let il = self.anim.look(&AnimTarget::Context { item: Some(k) });
                    let key = Tgt::Ctx(k).key();
                    let hov = self.fx.hov(key);
                    with_look(cv, &il, ir, |cv| {
                        cv.fill_rr(ir, 10.0, th.raised.fade(hov));
                        self.icons.draw(cv, line.icon, 1.6, Rect::new(ir.x + 10.0, ir.cy() - 8.0, 16.0, 16.0), th.text2);
                        cv.text(Rect::new(ir.x + 36.0, ir.cy() - 9.0, ir.w - 46.0, 18.0), &line.label, st, th.text);
                        if !line.key.is_empty() {
                            cv.text(Rect::new(ir.x + 36.0, ir.cy() - 6.0, ir.w - 46.0 - 10.0, 12.0), line.key, key_st.right(), th.dim);
                        }
                    });
                }
            })
        });
        for h in hits {
            self.hits.push(h);
        }
        self.hits.push((r, Tgt::Body));
    }

    /// The liquid outline along the bottom edge: the body bulges below its straight edge following the curve, the outline fades at both ends.
    fn paint_wave(&self, cv: &Canvas, th: &Theme, body: Rect) {
        let w = self.bar.wave();
        let pts = w.points();
        if pts.len() < 2 {
            return;
        }
        let ox = body.x + wave::INSET;
        let oy = body.bottom() - wave::UP;
        // fill: the straight edge down to the curve, in the body colour
        let mut path = Path::new();
        path.segs.push(Seg::Move(pts[0].0, wave::UP - 1.0));
        for p in &pts {
            path.segs.push(Seg::Line(p.0, p.1));
        }
        let last = pts[pts.len() - 1];
        path.segs.push(Seg::Line(last.0, wave::UP - 1.0));
        path.segs.push(Seg::Close);
        let m = Affine::translate(ox, oy);
        if let Ok(g) = build_geometry(cv.gfx, &path, &m, true) {
            cv.geometry(&g, Some(th.bg), None);
        }
        // outline: one round-capped segment per step, its alpha from the gradient stops (0, .3, 1, 1, .3, 0 at 0 / 7.5 / 15 / 85 / 92.5 / 100 %)
        let edge = if th.dark { Color::rgb8(0x6e, 0x6e, 0x6e) } else { Color::rgb8(0xb9, 0xb9, 0xb9) };
        let total = pts[pts.len() - 1].0.max(1.0);
        let alpha_at = |x: f32| {
            let t = x / total;
            let s = &wave::GRADIENT_STOPS;
            for k in 0..s.len() - 1 {
                if t <= s[k + 1].0 {
                    let u = (t - s[k].0) / (s[k + 1].0 - s[k].0).max(1e-6);
                    return s[k].1 + (s[k + 1].1 - s[k].1) * u;
                }
            }
            0.0
        };
        for k in 0..pts.len() - 1 {
            let (a, b) = (pts[k], pts[k + 1]);
            let al = alpha_at((a.0 + b.0) / 2.0);
            cv.line((ox + a.0, oy + a.1), (ox + b.0, oy + b.1), wave::LINE_WIDTH, edge.fade(al));
        }
    }
}

fn clip_rect(r: Rect, to: Rect) -> Option<Rect> {
    let (x0, y0) = (r.x.max(to.x), r.y.max(to.y));
    let (x1, y1) = (r.right().min(to.right()), r.bottom().min(to.bottom()));
    (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
