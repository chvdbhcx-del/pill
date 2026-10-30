//! Live rows: layout (`Elem` vocabulary -> positioned boxes, the numbers of live-*.css) and painting. The layout is generic over a text
//! meter so it is unit-tested without a GPU; the shell is told the measured size (`Event::LiveMeasured`) and sizes the capsule from it.
use super::gpu::{c8, Gpu};
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::{Canvas, Color};
use pill_ui_core::live::model::{palette, Accent, ButtonStyle, ColorRole, Elem, RowSpec, TextStyle};
use glasskit::theme::Theme;

/// Gap between elements of a row (`.lp.lv { gap: 8px }`), side padding of a row (`.lp { padding: 0 16px }`).
pub const GAP: f32 = 8.0;
pub const PAD_X: f32 = 16.0;
/// Content height of an ordinary row (the capsule is 30 px; a row never forces it taller).
pub const ROW_H: f32 = 24.0;

/// Text width provider. The canvas measures with DirectWrite; tests use [`Mono`].
pub trait Meter {
    fn w(&self, s: &str, st: Style) -> f32;
}
impl Meter for Canvas<'_> {
    fn w(&self, s: &str, st: Style) -> f32 {
        self.text_width(s, st)
    }
}
/// Fixed advance per character (`k` x font size): deterministic widths for layout tests.
pub struct Mono(pub f32);
impl Meter for Mono {
    fn w(&self, s: &str, st: Style) -> f32 {
        s.chars().count() as f32 * st.size * self.0 + st.track * s.chars().count() as f32
    }
}

/// Font style + colour of a named [`TextStyle`] (sizes from the doc comments of `live::model::TextStyle`).
pub fn text_style(ts: TextStyle, th: &Theme) -> (Style, Color) {
    match ts {
        TextStyle::Code => (Style::new(14.0, 600).mono().track(0.56), th.text),
        TextStyle::Label => (Style::new(14.0, 600), th.text),
        TextStyle::Dim => (Style::new(13.0, 500), th.dim),
        TextStyle::Status => (Style::new(13.0, 500), th.text),
        TextStyle::StatusAmber => (Style::new(13.0, 500), th.amber),
        TextStyle::Clock => (Style::new(13.0, 600).mono(), th.text),
        TextStyle::RecLabel => (Style::new(12.0, 600).track(0.48), c8(palette::OBS_LABEL)),
        TextStyle::VoiceTimer => (Style::new(13.0, 700).mono(), th.text),
        TextStyle::Name => (Style::new(13.0, 600).ellipsis(), th.text),
        TextStyle::Game => (Style::new(13.0, 600).ellipsis(), th.text),
        TextStyle::Percent => (Style::new(13.0, 600), th.accent),
        TextStyle::Small => (Style::new(11.0, 500), th.dim),
        TextStyle::FileName => (Style::new(14.0, 600).ellipsis(), th.text),
        TextStyle::VolNum => (Style::new(14.0, 600).mono().right(), th.text),
        TextStyle::VolNumMuted => (Style::new(14.0, 600).mono().right(), th.dim),
        TextStyle::Count => (Style::new(13.0, 600), th.text),
        TextStyle::Metric => (Style::new(13.0, 700).mono(), th.text),
    }
}

pub fn role_color(r: ColorRole, th: &Theme) -> Color {
    match r {
        ColorRole::Text => th.text,
        ColorRole::Dim => th.dim,
        ColorRole::Accent => th.accent,
        ColorRole::Amber => th.amber,
        ColorRole::Green => th.green,
        ColorRole::Red => th.red,
        ColorRole::Rgb(c) => c8(c),
    }
}

pub fn accent_color(a: Option<Accent>, th: &Theme) -> Color {
    match a {
        Some(Accent::Rgb(c)) => c8(c),
        _ => th.accent,
    }
}

const BADGE_STYLE: Style = Style::new(10.0, 700).track(0.5);

/// (width, height) of one element; `Flex` is (0, 0).
pub fn elem_size(e: &Elem, m: &dyn Meter, th: &Theme) -> (f32, f32) {
    match e {
        Elem::Text { text, style, max_w } => {
            let (st, _) = text_style(*style, th);
            let mut w = m.w(text, st);
            if matches!(style, TextStyle::VolNum | TextStyle::VolNumMuted) {
                w = w.max(m.w("000", st));
            }
            if matches!(style, TextStyle::VoiceTimer) {
                w = w.max(28.0);
            }
            (max_w.map_or(w, |mw| w.min(mw)), 18.0)
        }
        Elem::Clock { text, style, .. } => {
            let (st, _) = text_style(*style, th);
            (m.w(text, st), 18.0)
        }
        Elem::Icon { size, .. } => (*size, *size),
        Elem::Eq { .. } => (23.0, 16.0),
        Elem::Cover { size, .. } => (*size, *size),
        Elem::Dot { size, .. } => (*size, *size),
        Elem::Spinner { size } | Elem::Check { size, .. } | Elem::Ring { size, .. } => (*size, *size),
        Elem::Bar { width, height, .. } => (*width, *height),
        Elem::Levels { bars, .. } => (bars.len() as f32 * 4.0 + bars.len().saturating_sub(1) as f32 * 3.0, 22.0),
        Elem::Spacer(w) => (*w, 0.0),
        Elem::Flex => (0.0, 0.0),
        Elem::Sep => (1.0, 14.0),
        Elem::Group(g) => {
            let (mut w, mut h) = (0.0f32, 0.0f32);
            for (i, c) in g.iter().enumerate() {
                let (cw, ch) = elem_size(c, m, th);
                w += cw + if i > 0 { 4.0 } else { 0.0 };
                h = h.max(ch);
            }
            (w, h)
        }
        Elem::Badge { text } => (m.w(text, BADGE_STYLE) + 12.0, 16.0),
        Elem::Button { label, icon, style, .. } => match style {
            ButtonStyle::Stop { width } => (*width, 24.0),
            ButtonStyle::IconRound => {
                let _ = (label, icon);
                (22.0, 22.0)
            }
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub i: usize,
    /// left edge relative to the row's left edge (padding included)
    pub x: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowLayout {
    pub w: f32,
    pub h: f32,
    pub items: Vec<Placed>,
}

/// Lays a row out in css px at 100 %: padding 16, gap 8, `width` is the minimum, `Flex` takes the slack (else the content is centred).
pub fn layout_row(row: &RowSpec, m: &dyn Meter, th: &Theme) -> RowLayout {
    let sizes: Vec<(f32, f32)> = row.elements.iter().map(|e| elem_size(e, m, th)).collect();
    let n = sizes.len();
    let mut total = 2.0 * PAD_X;
    let mut prev_spacer = true;
    for (i, e) in row.elements.iter().enumerate() {
        let gap = if i == 0 || prev_spacer { 0.0 } else { GAP };
        total += sizes[i].0 + gap;
        prev_spacer = matches!(e, Elem::Spacer(_));
    }
    let target = row.width.max(total);
    let flexes = row.elements.iter().filter(|e| matches!(e, Elem::Flex)).count();
    let extra = target - total;
    let (lead, per_flex) = if flexes > 0 { (0.0, extra / flexes as f32) } else { (extra / 2.0, 0.0) };
    let mut x = PAD_X + lead;
    let mut items = Vec::with_capacity(n);
    let mut h = ROW_H;
    prev_spacer = true;
    for (i, e) in row.elements.iter().enumerate() {
        if i > 0 && !prev_spacer {
            x += GAP;
        }
        let (w, eh) = sizes[i];
        let w = if matches!(e, Elem::Flex) { per_flex } else { w };
        items.push(Placed { i, x, w, h: eh });
        x += w;
        h = h.max(eh);
        prev_spacer = matches!(e, Elem::Spacer(_));
    }
    RowLayout { w: target, h: if row.tall { h + 16.0 } else { h }, items }
}

/// Per-frame animation inputs of a row.
#[derive(Clone, Copy)]
pub struct Anim {
    /// ms on the host clock (loops)
    pub t: f64,
    /// ms since the row appeared (check mark drawing)
    pub age: f64,
    pub reduced: bool,
}

/// Clickable parts found while painting: (rect in window px, row element index).
pub type Hits = Vec<(Rect, usize)>;

/// Paints `row` with its left edge at `x0` and its vertical centre at `cy` (css px at 100 %; the caller has scaled the canvas).
pub fn paint_row(cv: &Canvas, th: &Theme, gpu: &mut Gpu, row: &RowSpec, lay: &RowLayout, x0: f32, cy: f32, an: Anim, hits: &mut Hits) {
    for p in &lay.items {
        let e = &row.elements[p.i];
        let r = Rect::new(x0 + p.x, cy - p.h / 2.0, p.w, p.h);
        paint_elem(cv, th, gpu, row, e, r, an, p.i, hits);
    }
}

fn sin01(t: f64, period_ms: f64, delay_ms: f64) -> f32 {
    // ease-in-out between 0 and 1 and back (`alternate` of the equaliser keyframes)
    let ph = ((t - delay_ms) / period_ms).rem_euclid(1.0);
    (0.5 - 0.5 * (ph * std::f64::consts::TAU).cos()) as f32
}

#[allow(clippy::too_many_arguments)]
fn paint_elem(cv: &Canvas, th: &Theme, gpu: &mut Gpu, row: &RowSpec, e: &Elem, r: Rect, an: Anim, idx: usize, hits: &mut Hits) {
    let pulse = |ms: Option<u32>| -> f32 {
        match ms {
            Some(p) if !an.reduced => 0.75 + 0.25 * (std::f64::consts::TAU * an.t / p as f64).cos() as f32,
            _ => 1.0,
        }
    };
    match e {
        Elem::Text { text, style, .. } => {
            let (st, c) = text_style(*style, th);
            cv.text(r, text, if matches!(style, TextStyle::Name | TextStyle::Game | TextStyle::FileName) { st.ellipsis() } else { st }, c);
        }
        Elem::Clock { text, style, .. } => {
            let (st, c) = text_style(*style, th);
            cv.text(r, text, st, c);
        }
        Elem::Icon { name, size, color } => gpu.icon(cv, name.as_str(), Rect::new(r.x, r.y, *size, *size), role_color(*color, th), 1.7),
        Elem::Eq { accent, playing } => {
            let c = accent_color(*accent, th);
            const H: [f32; 4] = [0.55, 1.0, 0.85, 0.6];
            const P: [f64; 4] = [800.0, 600.0, 1000.0, 750.0];
            const D: [f64; 4] = [0.0, -300.0, -500.0, -100.0];
            for b in 0..4 {
                let scale = if *playing && !an.reduced { 0.45 + 0.55 * sin01(an.t, P[b], D[b]) } else { 0.45 };
                let h = r.h * if b == 1 { 1.0 } else { H[b] } * scale;
                let x = r.x + b as f32 * 6.5;
                cv.fill_rr(Rect::new(x, r.cy() - h / 2.0, 3.5, h), 1.75, c);
            }
        }
        Elem::Cover { id, url, size } => gpu.cover(cv, th, *id, url.as_deref(), Rect::new(r.x, r.y, *size, *size), an.t),
        Elem::Dot { color, size, halo, pulse_ms } => {
            let c = c8(*color);
            let k = pulse(*pulse_ms);
            if *halo {
                cv.fill_circle(r.center(), size / 2.0 + 4.0, c.alpha(0.16 * k));
            }
            cv.fill_circle(r.center(), size / 2.0, c.fade(k));
        }
        Elem::Spinner { size } => {
            let a = if an.reduced { 0.0 } else { ((an.t / 800.0).fract() * 360.0) as f32 };
            cv.stroke_circle(r.center(), size / 2.0 - 1.0, 2.0, th.raised2);
            cv.arc(r.center(), size / 2.0 - 1.0, 2.0, a, 90.0, th.text);
        }
        Elem::Check { size, color } => {
            let c = role_color(*color, th);
            let k = if an.reduced { 1.0 } else { ((an.age / 360.0) as f32).clamp(0.0, 1.0) };
            cv.stroke_circle(r.center(), size / 2.0 - 1.0, 1.7, c);
            let (cx, cy, s) = (r.cx(), r.cy(), *size);
            let (a, b, d) = ((cx - 0.22 * s, cy + 0.0), (cx - 0.06 * s, cy + 0.16 * s), (cx + 0.24 * s, cy - 0.16 * s));
            let seg1 = (k * 2.0).clamp(0.0, 1.0);
            let seg2 = (k * 2.0 - 1.0).clamp(0.0, 1.0);
            cv.line(a, (a.0 + (b.0 - a.0) * seg1, a.1 + (b.1 - a.1) * seg1), 1.8, c);
            if seg2 > 0.0 {
                cv.line(b, (b.0 + (d.0 - b.0) * seg2, b.1 + (d.1 - b.1) * seg2), 1.8, c);
            }
        }
        Elem::Ring { value, size, glide_ms } => {
            let key = format!("{}#{}", row.id, idx);
            let rr = size / 2.0 - 1.25;
            cv.stroke_circle(r.center(), rr, 2.5, th.raised2);
            match value {
                Some(v) => {
                    let v = gpu.glide(&key, *v, *glide_ms as f64, an.t, an.reduced);
                    cv.arc(r.center(), rr, 2.5, 0.0, 360.0 * v.clamp(0.0, 1.0), th.accent);
                }
                None => {
                    let a = if an.reduced { 0.0 } else { ((an.t / 900.0).fract() * 360.0) as f32 };
                    cv.arc(r.center(), rr, 2.5, a, 100.0, th.accent);
                }
            }
        }
        Elem::Bar { value, width, height, glide_ms, muted } => {
            let key = format!("{}#{}", row.id, idx);
            let v = if *muted { *value } else { gpu.glide(&key, *value, *glide_ms as f64, an.t, an.reduced) };
            cv.fill_rr(r, height / 2.0, Color::rgba(1.0, 1.0, 1.0, 0.22).mix(th.text.alpha(0.22), if th.dark { 0.0 } else { 1.0 }));
            let w = (width * v.clamp(0.0, 1.0)).max(if v > 0.0 { *height } else { 0.0 });
            cv.fill_rr(Rect::new(r.x, r.y, w, r.h), height / 2.0, if *muted { th.dim } else { th.text });
        }
        Elem::Levels { bars, color } => {
            let c = c8(*color);
            for (i, b) in bars.iter().enumerate() {
                let k = b.clamp(0.14, 1.0);
                let h = r.h * k;
                cv.fill_rr(Rect::new(r.x + i as f32 * 7.0, r.cy() - h / 2.0, 4.0, h), 2.0, c);
            }
        }
        Elem::Spacer(_) | Elem::Flex => {}
        Elem::Sep => cv.fill_rect(Rect::new(r.x, r.y, 1.0, r.h), th.line),
        Elem::Group(g) => {
            let mut x = r.x;
            for c in g {
                let (w, h) = elem_size(c, &Wrap(cv), th);
                paint_elem(cv, th, gpu, row, c, Rect::new(x, r.cy() - h / 2.0, w, h), an, idx, hits);
                x += w + 4.0;
            }
        }
        Elem::Badge { text } => {
            cv.fill_rr(r, 5.0, th.raised2);
            cv.text(r, text, BADGE_STYLE.center(), th.dim);
        }
        Elem::Button { label, icon, style, .. } => {
            hits.push((r, idx));
            match style {
                ButtonStyle::Stop { .. } => {
                    cv.fill_rr(r, 12.0, c8(palette::STOP_FILL));
                    cv.fill_rr(Rect::new(r.x + 10.0, r.cy() - 4.0, 8.0, 8.0), 2.0, c8(palette::STOP_SQUARE));
                    if let Some(l) = label {
                        cv.text(Rect::new(r.x + 24.0, r.y, r.w - 28.0, r.h), l, Style::new(13.0, 600), c8(palette::STOP_TEXT));
                    }
                }
                ButtonStyle::IconRound => {
                    cv.fill_circle(r.center(), 11.0, th.raised2);
                    if let Some(i) = icon {
                        gpu.icon(cv, i.as_str(), r.inset(4.0), th.text, 1.7);
                    }
                }
            }
        }
    }
}

struct Wrap<'a, 'b>(&'a Canvas<'b>);
impl Meter for Wrap<'_, '_> {
    fn w(&self, s: &str, st: Style) -> f32 {
        self.0.text_width(s, st)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pill_ui_core::live::model::{IconName, RowSpec};

    fn th() -> Theme {
        Theme::pill_dark()
    }

    #[test]
    fn rows_flow_left_to_right_with_gap_and_padding() {
        let row = RowSpec::new("t", 1).with(Elem::icon(IconName::Copy, 18.0)).with(Elem::text("Copied", TextStyle::Label));
        let l = layout_row(&row, &Mono(0.5), &th());
        let text_w = 6.0 * 14.0 * 0.5;
        assert_eq!(l.items[0].x, 16.0);
        assert_eq!(l.items[1].x, 16.0 + 18.0 + 8.0);
        assert!((l.w - (16.0 + 18.0 + 8.0 + text_w + 16.0)).abs() < 0.01, "{}", l.w);
    }

    #[test]
    fn the_minimum_width_is_filled_by_flex_or_centres_the_content() {
        let flexed = RowSpec::new("rec", 1).width(258.0).with(Elem::Dot { color: palette::REC_DOT, size: 10.0, halo: false, pulse_ms: None }).with(Elem::Flex).with(Elem::Badge { text: "MP4".into() });
        let l = layout_row(&flexed, &Mono(0.5), &th());
        assert_eq!(l.w, 258.0);
        let badge = l.items[2];
        assert!((badge.x + badge.w - (258.0 - 16.0)).abs() < 0.01, "the badge is pushed to the right padding: {badge:?}");
        let plain = RowSpec::new("x", 1).width(200.0).with(Elem::icon(IconName::Bolt, 20.0));
        let l = layout_row(&plain, &Mono(0.5), &th());
        assert_eq!(l.w, 200.0);
        assert!((l.items[0].x - 90.0).abs() < 0.01, "no flex: centred, got {}", l.items[0].x);
    }

    #[test]
    fn text_max_width_clips_and_fixed_width_slots_hold() {
        let long = Elem::text_max("a very long file name indeed.zip", TextStyle::FileName, 100.0);
        assert_eq!(elem_size(&long, &Mono(0.5), &th()).0, 100.0);
        let vol = Elem::text("5", TextStyle::VolNum);
        assert_eq!(elem_size(&vol, &Mono(0.5), &th()).0, 3.0 * 14.0 * 0.5 + 3.0 * 0.0, "volume digits reserve three characters");
        assert_eq!(elem_size(&Elem::Levels { bars: vec![0.5; 5], color: palette::VOICE }, &Mono(0.5), &th()), (5.0 * 4.0 + 4.0 * 3.0, 22.0));
        assert_eq!(elem_size(&Elem::Eq { accent: None, playing: true }, &Mono(0.5), &th()), (23.0, 16.0));
    }

    #[test]
    fn spacers_replace_the_gap_and_tall_rows_grow() {
        let row = RowSpec::new("s", 1).with(Elem::Spacer(4.0)).with(Elem::Spacer(4.0)).with(Elem::icon(IconName::X, 10.0));
        let l = layout_row(&row, &Mono(0.5), &th());
        assert_eq!(l.items[2].x, 16.0 + 8.0);
        let tall = RowSpec::new("d", 1).tall().with(Elem::icon(IconName::Shelf, 40.0));
        assert_eq!(layout_row(&tall, &Mono(0.5), &th()).h, 56.0);
    }
}
