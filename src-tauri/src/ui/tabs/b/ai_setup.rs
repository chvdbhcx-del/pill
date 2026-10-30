//! The AI setup wizard (Settings > AI setup, also opened from the Chat tab): native view of `pill_ui_core::panel::b::ai_setup::AiSetup`.
//! Port of src/ai-setup.js + ai-setup.css (+ the settings.css `.card .row .chip .badge .btn .sw` classes it uses).
//!
//! * One scrolling page that fills the rect it is given (`margin` px of side padding, default 16): intro, "Voice to text (Whisper)" with the
//!   local card (switch, model chips, download plan, progress, delete) and the cloud card, "AI chat" with the local servers, the Ollama setup
//!   or its model catalog, the cloud card, and the footer.
//! * The page is laid out once per model change (`build`, a pure function over `ms::View` that produces a display list in content
//!   coordinates, including the click rectangles) and painted from that list every frame, so a frame costs no text measuring.
//! * Nothing starts without a click: every download button shows source and size (the model builds those strings).
//! * No API key is handled here: the model only asks `voice_has_key` for a yes/no; keys are typed on the Voice / AI settings pages, which the
//!   two "cloud" buttons open (`TabFx::GoPage`).
//! * Motion: hover/press on every control (`kit::Clicks`), chips pop when chosen, the remove button dips when armed, blocks that appear (progress,
//!   plan, result lines, the Ollama block) fade + rise, the switch knob springs, bars ease / slide (glasskit `ProgressBar`), spinner while a job
//!   runs. With reduced motion everything lands on its end state.
//! ponytail: the model drop-down of a running server is a click-to-cycle pill (a popup `Select` needs its own window); blocks that
//! disappear (a finished progress box) vanish at once instead of fading out.
use super::kit::{self, button, button_w, Bk, Calls, Clicks, Cues, Icons};
use crate::ui::view::{Cx, Msg, View};
use glasskit::canvas::Canvas;
use glasskit::controls::{ProgressBar, Spinner, Toggle, Widget};
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, Response};
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::ai_setup::{self as ms, Action, AiSetup, OllamaBlock, OllamaControl, Plan, ProgressView};
use pill_ui_core::panel::b::{Anim, AnimCue, Lang};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------------------------------------------------------------------
// Display list
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Id {
    Chip(usize),
    Install,
    Enable,
    Remove,
    Cancel,
    UseServer(usize),
    Pick(usize),
    OllamaDownload,
    OllamaRun,
    Pull(usize),
    UsePulled,
    CheckAgain,
    GoVoice,
    GoAi,
    Later,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Job {
    Whisper,
    OllamaSetup,
    OllamaPull,
}

impl Job {
    const ALL: [Job; 3] = [Job::Whisper, Job::OllamaSetup, Job::OllamaPull];
    fn id(self) -> &'static str {
        match self {
            Job::Whisper => "whisper",
            Job::OllamaSetup => "ollama-setup",
            Job::OllamaPull => "ollama-pull",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Col {
    Text,
    Text2,
    Dim,
    Faint,
    Green,
    Red,
    Link,
}

fn col(th: &Theme, c: Col) -> Color {
    match c {
        Col::Text => th.text,
        Col::Text2 => th.text2,
        Col::Dim => th.dim,
        Col::Faint => th.faint,
        Col::Green => th.green,
        Col::Red => th.red,
        Col::Link => th.link,
    }
}

#[derive(Clone, Debug)]
enum K {
    Text { s: String, st: Style, c: Col },
    Card,
    Sep,
    Btn { id: Id, label: String, kind: Bk, enabled: bool },
    Chip { id: Id, name: String, size: String, on: bool, disk: Option<String>, inuse: Option<String>, cue: String },
    Switch { enabled: bool },
    Bar(Job),
    Spin,
    Badge { s: String, pro: bool },
    Picker { id: Id, label: String, enabled: bool },
}

#[derive(Clone, Debug)]
struct Item {
    r: Rect,
    k: K,
    /// index into `Built::groups` (0 = always shown); a group that appears gets the enter animation
    grp: usize,
}

#[derive(Default)]
struct Built {
    items: Vec<Item>,
    groups: Vec<String>,
    hits: Vec<(Id, Rect)>,
    switch: Option<Rect>,
    height: f32,
}

enum Ctl {
    Btn(Id, String, Bk, bool),
    Switch,
    Picker(Id, String, bool),
    Note(String),
}

const GAP: f32 = 8.0;

struct Lay<'a, 'c> {
    cv: &'a Canvas<'c>,
    th: &'a Theme,
    w: f32,
    y: f32,
    b: Built,
    grp: usize,
    card: Option<(usize, f32)>,
    first: bool,
    switch_on_enabled: bool,
}

impl<'a, 'c> Lay<'a, 'c> {
    fn sub_style() -> Style {
        Style::new(11.0, 400)
    }

    fn mh(&self, s: &str, st: Style, w: f32) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        self.cv.measure(s, st.wrap(), w.max(10.0)).1.ceil()
    }

    fn push(&mut self, r: Rect, k: K) {
        self.b.items.push(Item { r, k, grp: self.grp });
    }

    fn hit(&mut self, id: Id, r: Rect) {
        self.b.hits.push((id, r));
    }

    fn group(&mut self, key: &str) {
        self.b.groups.push(key.to_string());
        self.grp = self.b.groups.len() - 1;
    }

    fn ungroup(&mut self) {
        self.grp = 0;
    }

    fn text(&mut self, x: f32, y: f32, w: f32, s: &str, st: Style, c: Col) -> f32 {
        let h = self.mh(s, st, w);
        if h > 0.0 {
            self.push(Rect::new(x, y, w, h), K::Text { s: s.to_string(), st: st.wrap(), c });
        }
        h
    }

    fn caption(&mut self, s: &str, first: bool) {
        if !first {
            self.y += 24.0;
        }
        let st = Style::new(11.0, 600).track(0.9);
        let h = self.text(4.0, self.y, self.w - 8.0, &s.to_uppercase(), st, Col::Dim);
        self.y += h + 8.0;
    }

    fn begin_card(&mut self) {
        self.b.items.push(Item { r: Rect::new(0.0, self.y, self.w, 0.0), k: K::Card, grp: 0 });
        self.card = Some((self.b.items.len() - 1, self.y));
        self.first = true;
        self.y += 1.0;
    }

    fn end_card(&mut self) {
        if let Some((i, y0)) = self.card.take() {
            self.y += 1.0;
            self.b.items[i].r.h = self.y - y0;
        }
        self.y += 12.0;
    }

    fn sep(&mut self) {
        if !self.first {
            self.b.items.push(Item { r: Rect::new(1.0, self.y, self.w - 2.0, 1.0), k: K::Sep, grp: self.grp });
        }
        self.first = false;
    }

    fn btn_h(kind: Bk, small: bool) -> f32 {
        let _ = kind;
        if small {
            28.0
        } else {
            32.0
        }
    }

    /// A `.row`: title + description on the left, controls on the right; `extra` is a coloured result line under the description.
    fn row(&mut self, title: &str, sub: &str, sub_c: Col, ctls: Vec<Ctl>, extra: Option<(String, Col, String)>) {
        self.sep();
        let th = self.th;
        let ws: Vec<f32> = ctls
            .iter()
            .map(|c| match c {
                Ctl::Btn(_, l, k, _) => button_w(self.cv, th, l, false, *k),
                Ctl::Switch => 44.0,
                Ctl::Picker(_, l, _) => (self.cv.text_width(l, th.small(400)) + 40.0).clamp(110.0, 230.0),
                Ctl::Note(s) => self.cv.text_width(s, Self::sub_style()).min(230.0).ceil() + 2.0,
            })
            .collect();
        let total: f32 = ws.iter().sum::<f32>() + GAP * ws.len().saturating_sub(1) as f32;
        let lw = self.w - 32.0 - if total > 0.0 { total + 16.0 } else { 0.0 };
        let th_h = self.mh(title, th.body().weight(500), lw).max(18.0);
        let sub_h = self.mh(sub, Self::sub_style(), lw);
        let ex_h = extra.as_ref().map_or(0.0, |(s, _, _)| self.mh(s, Self::sub_style(), lw));
        let content = th_h + if sub_h > 0.0 { 2.0 + sub_h } else { 0.0 } + if ex_h > 0.0 { 2.0 + ex_h } else { 0.0 };
        let rh = (content + 22.0).max(56.0);
        let y0 = self.y;
        let mut y = y0 + (rh - content) / 2.0;
        self.push(Rect::new(16.0, y, lw, th_h), K::Text { s: title.to_string(), st: th.body().weight(500).wrap(), c: Col::Text });
        y += th_h;
        if sub_h > 0.0 {
            y += 2.0;
            self.push(Rect::new(16.0, y, lw, sub_h), K::Text { s: sub.to_string(), st: Self::sub_style().wrap(), c: sub_c });
            y += sub_h;
        }
        if let Some((s, c, key)) = extra {
            if ex_h > 0.0 {
                y += 2.0;
                self.group(&key);
                self.push(Rect::new(16.0, y, lw, ex_h), K::Text { s, st: Self::sub_style().wrap(), c });
                self.ungroup();
            }
        }
        let mut x = self.w - 16.0 - total;
        for (c, wd) in ctls.into_iter().zip(ws) {
            match c {
                Ctl::Btn(id, label, kind, enabled) => {
                    let r = Rect::new(x, y0 + (rh - 32.0) / 2.0, wd, 32.0);
                    self.push(r, K::Btn { id, label, kind, enabled });
                    if enabled {
                        self.hit(id, r);
                    }
                }
                Ctl::Switch => {
                    let r = Rect::new(x, y0 + (rh - 24.0) / 2.0, 44.0, 24.0);
                    self.push(r, K::Switch { enabled: self.switch_on_enabled });
                    self.b.switch = Some(r);
                }
                Ctl::Picker(id, label, enabled) => {
                    let r = Rect::new(x, y0 + (rh - 32.0) / 2.0, wd, 32.0);
                    self.push(r, K::Picker { id, label, enabled });
                    if enabled {
                        self.hit(id, r);
                    }
                }
                Ctl::Note(s) => {
                    let h = self.mh(&s, Self::sub_style(), wd);
                    self.push(Rect::new(x, y0 + (rh - h) / 2.0, wd, h), K::Text { s, st: Self::sub_style().wrap(), c: Col::Dim });
                }
            }
            x += wd + GAP;
        }
        self.y = y0 + rh;
    }

    fn pad_begin(&mut self) {
        self.sep();
        self.y += 12.0;
    }

    fn pad_end(&mut self) {
        self.y += 12.0;
    }

    fn small_btn(&mut self, x: f32, y: f32, id: Id, label: &str, kind: Bk, enabled: bool) -> Rect {
        let w = button_w(self.cv, self.th, label, false, kind);
        let r = Rect::new(x, y, w, Self::btn_h(kind, true));
        self.push(r, K::Btn { id, label: label.to_string(), kind, enabled });
        if enabled {
            self.hit(id, r);
        }
        r
    }

    /// Progress box of a job (bar + line + Cancel while it runs, the result line when it is over).
    fn progress(&mut self, job: Job, pv: &ProgressView) {
        if !pv.visible {
            return;
        }
        self.group(&format!("prog:{}", job.id()));
        self.pad_begin();
        let x = 16.0;
        let iw = self.w - 32.0;
        if pv.running {
            let cw = button_w(self.cv, self.th, &pv.cancel_label, false, Bk::Danger);
            self.push(Rect::new(x, self.y + 7.0, 14.0, 14.0), K::Spin);
            self.push(Rect::new(x + 22.0, self.y, iw - cw - 22.0 - 12.0, 28.0), K::Text { s: pv.line.clone(), st: self.th.small(400).ellipsis(), c: Col::Text2 });
            let y = self.y;
            self.small_btn(x + iw - cw, y, Id::Cancel, &pv.cancel_label.clone(), Bk::Danger, true);
            self.y += 28.0 + 8.0;
            self.push(Rect::new(x, self.y, iw, 6.0), K::Bar(job));
            self.y += 6.0;
        } else if let Some((ok, text)) = &pv.result {
            let h = self.text(x, self.y, iw, text, Self::sub_style(), if *ok { Col::Green } else { Col::Red });
            self.y += h;
        }
        self.pad_end();
        self.ungroup();
    }
}

fn build(cv: &Canvas, th: &Theme, vw: &ms::View, w: f32, lang: Lang, switch_enabled: bool) -> Built {
    let mut l = Lay { cv, th, w, y: 0.0, b: Built::default(), grp: 0, card: None, first: true, switch_on_enabled: switch_enabled };
    l.b.groups.push(String::new());
    let (ru, check_label) = (lang == Lang::Ru, if lang == Lang::Ru { "Проверить снова" } else { "Check again" });
    let _ = ru;
    match vw {
        ms::View::Unavailable(msg) => {
            let h = l.text(4.0, 0.0, w - 8.0, msg, th.caption().weight(400), Col::Faint);
            l.y = h + 12.0;
            let y = l.y;
            l.small_btn(4.0, y, Id::CheckAgain, check_label, Bk::Normal, true);
            l.y += 28.0;
        }
        ms::View::Page { intro, voice, chat, footer } => {
            let h = l.text(4.0, 0.0, w - 8.0, intro, th.body(), Col::Text2);
            l.y = h + 4.0 + 20.0;

            // ---- voice
            l.caption(&voice.caption, true);
            l.begin_card();
            let mut ctls = vec![];
            if let Some((_, en)) = voice.switch {
                l.switch_on_enabled = en;
                ctls.push(Ctl::Switch);
            }
            l.row(&voice.local_title, &voice.local_sub, if voice.local_running { Col::Green } else { Col::Dim }, ctls, None);
            l.pad_begin();
            let mut x = 16.0;
            let mut y = l.y;
            for (i, c) in voice.chips.iter().enumerate() {
                let st = th.body();
                let szs = Style::new(11.0, 400);
                let (nw, sw) = (cv.text_width(&c.name, st), cv.text_width(&c.size_text, szs));
                let bw = |s: &Option<String>| s.as_ref().map_or(0.0, |s| cv.text_width(s, Style::new(11.0, 600)) + 12.0 + 8.0);
                let cw = (12.0 + nw + 8.0 + sw + bw(&c.on_disk_badge) + bw(&c.in_use_badge) + 12.0).ceil();
                if x + cw > w - 16.0 && x > 16.0 {
                    x = 16.0;
                    y += 30.0 + 8.0;
                }
                let r = Rect::new(x, y, cw, 30.0);
                l.push(r, K::Chip { id: Id::Chip(i), name: c.name.clone(), size: c.size_text.clone(), on: c.on, disk: c.on_disk_badge.clone(), inuse: c.in_use_badge.clone(), cue: format!("chip:{}", c.id) });
                l.hit(Id::Chip(i), r);
                x += cw + 8.0;
            }
            l.y = y + 30.0 + 10.0;
            let h = l.text(16.0, l.y, w - 32.0, &voice.model_note, Lay::sub_style(), Col::Dim);
            l.y += h;
            l.pad_end();
            // plan
            l.group(match voice.plan {
                Plan::Download { .. } => "plan:download",
                Plan::Enable { .. } => "plan:enable",
                Plan::AllSet { .. } => "plan:allset",
            });
            l.pad_begin();
            match &voice.plan {
                Plan::Download { heading, items, button, hint, enabled } => {
                    let h = l.text(16.0, l.y, w - 32.0, heading, Lay::sub_style(), Col::Dim);
                    l.y += h + 6.0;
                    for it in items {
                        l.push(Rect::new(16.0, l.y + 8.0, 4.0, 4.0), K::Badge { s: String::new(), pro: false });
                        let h = l.text(28.0, l.y, w - 44.0, it, th.small(400), Col::Text2);
                        l.y += h.max(18.0);
                    }
                    l.y += 10.0;
                    let bw = button_w(cv, th, button, false, Bk::Primary);
                    let r = Rect::new(16.0, l.y, bw, 32.0);
                    l.push(r, K::Btn { id: Id::Install, label: button.clone(), kind: Bk::Primary, enabled: *enabled });
                    if *enabled {
                        l.hit(Id::Install, r);
                    }
                    let hh = l.mh(hint, Lay::sub_style(), w - 32.0 - bw - 8.0);
                    l.push(Rect::new(16.0 + bw + 8.0, l.y + (32.0 - hh) / 2.0, w - 32.0 - bw - 8.0, hh), K::Text { s: hint.clone(), st: Lay::sub_style().wrap(), c: Col::Dim });
                    l.y += 32.0;
                }
                Plan::Enable { label, enabled } => {
                    let bw = button_w(cv, th, label, false, Bk::Primary);
                    let r = Rect::new(16.0, l.y, bw, 32.0);
                    l.push(r, K::Btn { id: Id::Enable, label: label.clone(), kind: Bk::Primary, enabled: *enabled });
                    if *enabled {
                        l.hit(Id::Enable, r);
                    }
                    l.y += 32.0;
                }
                Plan::AllSet { text } => {
                    let h = l.text(16.0, l.y, w - 32.0, text, Lay::sub_style(), Col::Green);
                    l.y += h;
                }
            }
            l.pad_end();
            l.ungroup();
            l.progress(Job::Whisper, &voice.progress);
            // folder + delete
            l.pad_begin();
            let h = l.text(16.0, l.y, w - 32.0, &voice.folder_note, Lay::sub_style(), Col::Dim);
            l.y += h;
            if let Some((label, en)) = &voice.delete {
                l.y += 10.0;
                let y = l.y;
                l.small_btn(16.0, y, Id::Remove, label, Bk::Danger, *en);
                l.y += 28.0;
            }
            l.pad_end();
            l.end_card();
            l.begin_card();
            l.row(&voice.cloud_title, &voice.cloud_sub, Col::Dim, vec![Ctl::Btn(Id::GoVoice, voice.cloud_button.clone(), Bk::Normal, true)], None);
            l.end_card();

            // ---- chat
            l.caption(&chat.caption, false);
            l.begin_card();
            l.row(&chat.local_title, &chat.local_sub, if chat.local_found { Col::Green } else { Col::Dim }, vec![Ctl::Btn(Id::CheckAgain, chat.check_again.clone(), Bk::Normal, true)], None);
            for (i, s) in chat.servers.iter().enumerate() {
                let extra = s.result.as_ref().map(|(ok, t)| (t.clone(), if *ok { Col::Green } else { Col::Red }, format!("use:{}", s.id)));
                l.row(&s.name, &s.sub, Col::Dim, vec![Ctl::Picker(Id::Pick(i), s.selected.clone(), s.model_picker_enabled), Ctl::Btn(Id::UseServer(i), s.button.clone(), Bk::Normal, s.button_enabled)], extra);
            }
            match &chat.ollama {
                OllamaBlock::Setup { title, sub, control, progress } => {
                    l.group("ollama:setup");
                    let ctl = match control {
                        OllamaControl::Installed { note } => Ctl::Note(note.clone()),
                        OllamaControl::RunInstaller { label, enabled } => Ctl::Btn(Id::OllamaRun, label.clone(), Bk::Primary, *enabled),
                        OllamaControl::Download { label, enabled } => Ctl::Btn(Id::OllamaDownload, label.clone(), Bk::Primary, *enabled),
                    };
                    l.row(title, sub, Col::Dim, vec![ctl], None);
                    l.ungroup();
                    l.progress(Job::OllamaSetup, progress);
                }
                OllamaBlock::Models { note, rows, progress, use_pulled } => {
                    l.group("ollama:models");
                    l.pad_begin();
                    let h = l.text(16.0, l.y, w - 32.0, note, Lay::sub_style(), Col::Dim);
                    l.y += h + 8.0;
                    for (i, r) in rows.iter().enumerate() {
                        if i > 0 {
                            l.push(Rect::new(16.0, l.y, w - 32.0, 1.0), K::Sep);
                        }
                        let rh = 36.0;
                        let nw = cv.text_width(&r.name, th.small(500).mono()).ceil() + 2.0;
                        l.push(Rect::new(16.0, l.y, nw, rh), K::Text { s: r.name.clone(), st: th.small(500).mono(), c: Col::Text });
                        l.push(Rect::new(16.0 + nw + 12.0, l.y, 120.0, rh), K::Text { s: r.size_text.clone(), st: Lay::sub_style(), c: Col::Dim });
                        if let Some(p) = &r.pulled {
                            let bw = cv.text_width(p, Style::new(11.0, 600)) + 12.0;
                            l.push(Rect::new(w - 16.0 - bw, l.y + 10.0, bw, 16.0), K::Badge { s: p.clone(), pro: false });
                        } else if let Some((label, en)) = &r.download {
                            let bw = button_w(cv, th, label, false, Bk::Normal);
                            let rr = Rect::new(w - 16.0 - bw, l.y + 4.0, bw, 28.0);
                            l.push(rr, K::Btn { id: Id::Pull(i), label: label.clone(), kind: Bk::Normal, enabled: *en });
                            if *en {
                                l.hit(Id::Pull(i), rr);
                            }
                        }
                        l.y += rh;
                    }
                    l.pad_end();
                    l.ungroup();
                    l.progress(Job::OllamaPull, progress);
                    if let Some((label, en)) = use_pulled {
                        l.group("ollama:use");
                        l.pad_begin();
                        let bw = button_w(cv, th, label, false, Bk::Primary);
                        let r = Rect::new(16.0, l.y, bw, 32.0);
                        l.push(r, K::Btn { id: Id::UsePulled, label: label.clone(), kind: Bk::Primary, enabled: *en });
                        if *en {
                            l.hit(Id::UsePulled, r);
                        }
                        l.y += 32.0;
                        l.pad_end();
                        l.ungroup();
                    }
                }
            }
            l.end_card();
            l.begin_card();
            l.row(&chat.cloud_title, &chat.cloud_sub, Col::Dim, vec![Ctl::Btn(Id::GoAi, chat.cloud_button.clone(), Bk::Normal, true)], None);
            l.end_card();

            // ---- footer
            l.begin_card();
            l.row(&footer.title, &footer.sub, Col::Dim, vec![Ctl::Btn(Id::Later, footer.button.clone(), Bk::Normal, footer.button_enabled)], None);
            l.end_card();
            l.y -= 12.0 - 4.0;
            let h = l.text(4.0, l.y, w - 8.0, &footer.note, Lay::sub_style(), Col::Faint);
            l.y += h + 12.0;
        }
    }
    l.b.height = l.y;
    l.b
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------------------------------------------------

pub struct AiSetupScreen {
    m: AiSetup,
    calls: Calls,
    clicks: Clicks<Id>,
    cues: Cues,
    icons: Icons,
    vw: ms::View,
    built: Built,
    /// bumped by every model change; the layout is rebuilt when it (or the width / scale / language) differs
    rev: u64,
    laid: Option<(u64, u32, u32, bool)>,
    seen: HashSet<String>,
    first_layout: bool,
    scroll: ScrollView,
    bars: HashMap<Job, ProgressBar>,
    spinner: Spinner,
    toggle: Toggle,
    mounted: bool,
    lang: Lang,
    carry: f64,
    rect: Rect,
    now_ms: f64,
    /// side padding of the page (the settings page may set 0 when it already pads)
    pub margin: f32,
}

impl Default for AiSetupScreen {
    fn default() -> Self {
        AiSetupScreen::new()
    }
}

impl AiSetupScreen {
    pub fn new() -> AiSetupScreen {
        let m = AiSetup::new(Lang::En);
        let vw = m.view();
        AiSetupScreen {
            m,
            calls: Calls::new(),
            clicks: Clicks::new(),
            cues: Cues::new(),
            icons: Icons::new(),
            vw,
            built: Built::default(),
            rev: 1,
            laid: None,
            seen: HashSet::new(),
            first_layout: true,
            scroll: ScrollView::new(),
            bars: HashMap::new(),
            spinner: Spinner::new(),
            toggle: Toggle::new(false).spring(0.45),
            mounted: false,
            lang: Lang::En,
            carry: 0.0,
            rect: Rect::default(),
            now_ms: 0.0,
            margin: 16.0,
        }
    }

    /// The model changed: re-read the view, retarget the bars and the switch, and let the next draw lay out again.
    fn refresh(&mut self) {
        self.vw = self.m.view();
        self.rev += 1;
        let mut states: Vec<(Job, Option<&ProgressView>)> = Vec::new();
        let mut sw = None;
        if let ms::View::Page { voice, chat, .. } = &self.vw {
            sw = voice.switch;
            states.push((Job::Whisper, Some(&voice.progress)));
            match &chat.ollama {
                OllamaBlock::Setup { progress, .. } => states.push((Job::OllamaSetup, Some(progress))),
                OllamaBlock::Models { progress, .. } => states.push((Job::OllamaPull, Some(progress))),
            }
        }
        for job in Job::ALL {
            let pv = states.iter().find(|(j, _)| *j == job).and_then(|(_, p)| *p).filter(|p| p.running);
            let bar = self.bars.entry(job).or_insert_with(ProgressBar::indeterminate);
            match pv {
                Some(p) => {
                    bar.set_visible(true);
                    if p.indeterminate {
                        bar.set_indeterminate();
                    } else {
                        bar.set_value(p.fill_pct as f32 / 100.0);
                    }
                }
                None => bar.set_visible(false),
            }
        }
        self.spinner.set_active(states.iter().any(|(_, p)| p.map_or(false, |p| p.running)));
        if let Some((on, en)) = sw {
            if self.toggle.on() != on {
                self.toggle.set_on(on, true);
            }
            self.toggle.set_enabled(en);
        }
    }

    /// After every model call: run its backend calls, route its effects, play its cues.
    fn settle(&mut self, calls: Vec<pill_ui_core::panel::b::BackendCall>, cx: &mut Cx) {
        self.calls.run(cx, calls);
        let fx = kit::route_effects(self.m.fx.take_effects(), cx);
        debug_assert!(fx.iter().all(|e| !matches!(e, pill_ui_core::panel::b::Effect::CopyText(_))));
        // the progress bars ease on their own; a `progress:*` cue would only make the whole box flash
        self.cues.play_all(self.m.fx.take_anims().into_iter().filter(|c| !c.target.starts_with("progress:")).collect());
        self.refresh();
        cx.request_frame();
    }

    fn act(&mut self, id: Id, cx: &mut Cx) {
        let (voice, chat) = match &self.vw {
            ms::View::Page { voice, chat, .. } => (Some(voice.clone()), Some(chat.clone())),
            _ => (None, None),
        };
        let a = match id {
            Id::Chip(i) => voice.and_then(|v| v.chips.get(i).map(|c| c.id.clone())).map(|m| {
                self.cues.play(AnimCue { target: format!("chip:{m}"), kind: Anim::Pop { from_scale: 0.88, ms: 400 } });
                Action::SelectWhisperModel(m)
            }),
            Id::Install => Some(Action::WhisperInstall),
            Id::Enable => Some(Action::WhisperEnable),
            Id::Remove => Some(Action::WhisperRemove),
            Id::Cancel => Some(Action::CancelJob),
            Id::UseServer(i) => chat.and_then(|c| c.servers.get(i).map(|s| Action::UseServer { server: s.id.clone() })),
            Id::Pick(i) => chat.and_then(|c| {
                c.servers.get(i).filter(|s| s.models.len() > 1).map(|s| {
                    let at = s.models.iter().position(|m| *m == s.selected).unwrap_or(0);
                    Action::PickServerModel { server: s.id.clone(), model: s.models[(at + 1) % s.models.len()].clone() }
                })
            }),
            Id::OllamaDownload => Some(Action::OllamaDownload),
            Id::OllamaRun => Some(Action::OllamaRunInstaller),
            Id::Pull(i) => chat.and_then(|c| match c.ollama {
                OllamaBlock::Models { rows, .. } => rows.get(i).map(|r| Action::OllamaPull(r.name.clone())),
                _ => None,
            }),
            Id::UsePulled => Some(Action::UseOllamaModel),
            Id::CheckAgain => Some(Action::CheckAgain),
            Id::GoVoice => Some(Action::GoVoice),
            Id::GoAi => Some(Action::GoAi),
            Id::Later => Some(Action::ToggleLater),
        };
        if let Some(a) = a {
            let calls = self.m.on_action(a);
            self.settle(calls, cx);
        }
    }

    fn relayout(&mut self, cv: &Canvas, th: &Theme, w: f32, scale: f32) {
        let key = (self.rev, w.to_bits(), scale.to_bits(), th.dark);
        if self.laid == Some(key) {
            return;
        }
        let en = matches!(&self.vw, ms::View::Page { voice, .. } if voice.switch.map_or(true, |(_, e)| e));
        self.built = build(cv, th, &self.vw, w, self.lang, en);
        self.laid = Some(key);
        self.clicks.begin();
        for (id, r) in self.built.hits.clone() {
            self.clicks.add(id, r);
        }
        // groups that were not on the page before fade + rise in (not on the very first layout)
        let now: HashSet<String> = self.built.groups.iter().filter(|g| !g.is_empty()).cloned().collect();
        if !self.first_layout {
            for g in &now {
                if !self.seen.contains(g) {
                    self.cues.play(AnimCue { target: g.clone(), kind: Anim::Enter { y: 6.0, ms: 220 } });
                }
            }
        }
        self.first_layout = false;
        self.seen = now;
    }

    fn content_ev(&self, ev: &InputEvent) -> InputEvent {
        self.scroll.to_content(ev, self.rect).translated(-self.margin, -self.margin.min(16.0))
    }
}

impl View for AiSetupScreen {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.clicks.reduced = cx.reduced_motion;
        let mut r = self.scroll.event(ev, self.rect);
        if r.consumed || r.capture {
            return r;
        }
        let cev = self.content_ev(ev);
        // the switch first (it is not a Clicks target: the Toggle owns its press / drag)
        if let Some(sr) = self.built.switch {
            let enabled = self.toggle.state().enabled;
            let before = self.toggle.on();
            let tr = self.toggle.event(&cev, sr);
            if tr.changed && enabled {
                let on = self.toggle.on();
                if on != before {
                    let calls = self.m.on_action(Action::WhisperSwitch(on));
                    self.settle(calls, cx);
                }
            }
            if tr.consumed || tr.capture {
                return r.merge(tr);
            }
        }
        let (cr, clicked) = self.clicks.event(&cev);
        r = r.merge(cr);
        if let Some(id) = clicked {
            self.act(id, cx);
        }
        r
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Event { name, payload, .. } => {
                let calls = self.m.on_event(&name, &payload);
                self.settle(calls, cx);
            }
            Msg::Reply { ticket, result } => {
                if let Some((tag, reply)) = self.calls.take(ticket, result) {
                    let calls = self.m.on_reply(tag, reply);
                    self.settle(calls, cx);
                }
            }
            Msg::Open(_) => {
                let calls = self.m.mount();
                self.settle(calls, cx);
            }
            Msg::Custom(_) => {}
        }
        Response::redraw()
    }

    fn on_show(&mut self, cx: &mut Cx) {
        cx.subscribe("aisetup:progress");
        self.lang = kit::lang_of(cx);
        self.m.set_lang(self.lang);
        // the status is read on every show: the installer may have finished while the page was away
        let calls = self.m.mount();
        self.mounted = true;
        self.settle(calls, cx);
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.now_ms = cx.now_ms;
        let l = kit::lang_of(cx);
        if l != self.lang {
            self.lang = l;
            self.m.set_lang(l);
            self.refresh();
            self.laid = None;
        }
        self.m.fx.reduce_motion = cx.reduced_motion;
        self.cues.reduced = cx.reduced_motion;
        self.clicks.reduced = cx.reduced_motion;
        let ms_ = kit::ms_of(dt_ms, &mut self.carry);
        if ms_ > 0 {
            let was = self.m.view();
            self.m.tick(ms_);
            if was != self.vw {
                // the delete button disarms after 3 s
                self.refresh();
            }
        }
        let dt = dt_ms as f32 / 1000.0;
        let mut busy = self.cues.step(dt_ms as f32);
        busy |= self.clicks.step(dt);
        busy |= self.scroll.step(dt);
        busy |= self.toggle.step(dt);
        busy |= self.spinner.step(dt);
        for b in self.bars.values_mut() {
            busy |= b.step(dt);
        }
        // the 3 s disarm timer needs frames while the delete button is armed
        busy |= matches!(&self.vw, ms::View::Page { voice, .. } if voice.delete.as_ref().map_or(false, |(l, _)| l.starts_with("Really") || l.starts_with("Точно")));
        busy
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.rect = rect;
        let w = (rect.w - 2.0 * self.margin).max(200.0);
        self.relayout(cv, th, w, cx.scale);
        self.scroll.set_content_height(self.built.height + 2.0 * self.margin.min(16.0), rect.h);
        let me = &*self;
        let m = me.margin;
        me.scroll.draw(cv, rect, th, |cv, _| {
            cv.with_offset(m, m.min(16.0), |cv| {
                for it in &me.built.items {
                    // skip what is far outside the viewport
                    let top = it.r.y + m.min(16.0) - me.scroll.offset();
                    if top > rect.h + 8.0 || top + it.r.h < -8.0 {
                        continue;
                    }
                    if it.grp > 0 {
                        me.cues.apply(cv, &me.built.groups[it.grp], it.r.center(), |cv| me.paint(cv, th, it));
                    } else {
                        me.paint(cv, th, it);
                    }
                }
            });
        });
    }
}

impl AiSetupScreen {
    fn paint(&self, cv: &Canvas, th: &Theme, it: &Item) {
        let r = it.r;
        match &it.k {
            K::Text { s, st, c } => cv.text(r, s, *st, col(th, *c)),
            K::Card => {
                cv.fill_rr(r, th.m.radius, th.card);
                cv.stroke_rr(r, th.m.radius, 1.0, th.border);
            }
            K::Sep => cv.fill_rect(r, th.line),
            K::Btn { id, label, kind, enabled } => {
                let go = |cv: &Canvas| button(cv, th, &self.icons, r, label, None, *kind, self.clicks.hover_amt(id), self.clicks.press_scale(id, 0.96), *enabled);
                if *id == Id::Remove {
                    self.cues.apply(cv, "whisper-remove", r.center(), go);
                } else {
                    go(cv);
                }
            }
            K::Chip { id, name, size, on, disk, inuse, cue } => {
                self.cues.apply(cv, cue, r.center(), |cv| {
                    let h = self.clicks.hover_amt(id);
                    cv.with_scale_about(r.center(), self.clicks.press_scale(id, 0.93), |cv| {
                        let bg = if *on { th.accent_soft } else { th.seg_bg.mix(th.btn_hover, h * 0.6) };
                        cv.fill_rr(r, 8.0, bg);
                        if *on {
                            cv.stroke_rr(r, 8.0, 1.0, th.accent.fade(0.55));
                        }
                        let mut x = r.x + 12.0;
                        let nw = cv.text_width(name, th.body());
                        cv.text(Rect::new(x, r.y, nw + 2.0, r.h), name, th.body(), if *on { th.text } else { th.text2.mix(th.text, 0.4) });
                        x += nw + 8.0;
                        let szs = Style::new(11.0, 400);
                        let sw = cv.text_width(size, szs);
                        cv.text(Rect::new(x, r.y, sw + 2.0, r.h), size, szs, th.dim);
                        x += sw + 8.0;
                        for (b, pro) in [(disk, false), (inuse, true)] {
                            if let Some(t) = b {
                                let bw = cv.text_width(t, Style::new(11.0, 600)) + 12.0;
                                badge(cv, th, Rect::new(x, r.cy() - 8.0, bw, 16.0), t, pro);
                                x += bw + 8.0;
                            }
                        }
                    });
                });
            }
            K::Switch { enabled } => {
                cv.with_opacity(1.0, |cv| self.toggle.draw(cv, th, r));
                let _ = enabled;
            }
            K::Bar(job) => {
                if let Some(b) = self.bars.get(job) {
                    b.draw(cv, th, r);
                }
            }
            K::Spin => self.spinner.draw(cv, th, r),
            K::Badge { s, pro } => {
                if s.is_empty() {
                    cv.fill_circle(r.center(), 2.0, th.dim);
                } else {
                    badge(cv, th, r, s, *pro);
                }
            }
            K::Picker { id, label, enabled } => {
                let h = if *enabled { self.clicks.hover_amt(id) } else { 0.0 };
                cv.with_opacity(if *enabled { 1.0 } else { 0.45 }, |cv| {
                    cv.fill_rr(r, th.m.field_r.min(8.0), th.field.mix(th.btn_hover, h * 0.5));
                    cv.text(Rect::new(r.x + 12.0, r.y, r.w - 34.0, r.h), label, th.small(400).ellipsis(), th.text);
                    let c = Rect::new(r.right() - 22.0, r.cy() - 2.5, 10.0, 5.0);
                    cv.line((c.x, c.y), (c.cx(), c.bottom()), 1.5, th.dim);
                    cv.line((c.cx(), c.bottom()), (c.right(), c.y), 1.5, th.dim);
                });
            }
        }
    }
}

/// `.badge.free` (green) / `.badge.pro` (link on accent wash).
fn badge(cv: &Canvas, th: &Theme, r: Rect, s: &str, pro: bool) {
    let (bg, fg) = if pro { (th.accent_soft, th.link) } else { (th.green_bg, th.green) };
    cv.fill_rr(r, 4.0, bg);
    cv.text(r, s, Style::new(11.0, 600).center(), fg);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::{motion_lock, render_view_cx, save_png, Sim};
    use crate::ui::view::{Cx, Fakes, Services};
    use glasskit::input::{Buttons, Modifiers, MouseButton};
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::rc::Rc;

    const FIXTURES: &str = include_str!("../../../../../tools/dev/fixtures/tabs-b/ai_setup.json");

    /// Lets a test keep a handle on the view that the `Sim` owns.
    struct Shared(Rc<RefCell<AiSetupScreen>>);
    impl View for Shared {
        fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
            self.0.borrow_mut().event(ev, cx)
        }
        fn message(&mut self, m: Msg, cx: &mut Cx) -> Response {
            self.0.borrow_mut().message(m, cx)
        }
        fn step(&mut self, dt: f64, cx: &mut Cx) -> bool {
            self.0.borrow_mut().step(dt, cx)
        }
        fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
            self.0.borrow_mut().draw(cv, th, rect, cx)
        }
        fn on_show(&mut self, cx: &mut Cx) {
            self.0.borrow_mut().on_show(cx)
        }
    }

    struct T {
        s: Sim,
        v: Rc<RefCell<AiSetupScreen>>,
        f: Fakes,
    }

    fn scenario(name: &str) -> Value {
        let all: Value = serde_json::from_str(FIXTURES).unwrap();
        all["scenarios"][name].clone()
    }

    /// A sim of the page over fake services answering from a fixture scenario; replies and events are delivered.
    fn sim(name: &str) -> T {
        let sc = scenario(name);
        let (svc, f) = Services::fake();
        for (cmd, v) in sc["replies"].as_object().unwrap() {
            f.api.reply(cmd, Ok(v.clone()));
        }
        let v = Rc::new(RefCell::new(AiSetupScreen::new()));
        let mut s = Sim::with_services(Box::new(Shared(v.clone())), svc);
        s.show();
        s.pump();
        for e in sc["events"].as_array().into_iter().flatten() {
            f.bus.emit(e["name"].as_str().unwrap(), e["payload"].clone());
        }
        s.pump();
        T { s, v, f }
    }

    fn pic(t: &mut T, name: &str, size: (f32, f32), scale: f32, dark: bool) -> glasskit::testkit::Image {
        let th = if dark { Theme::settings_dark() } else { Theme::pill_light() };
        t.s.cx.scale = scale;
        // the first render lays the page out, frames settle bars / cues, the second render is the picture
        render_view_cx(&mut *t.s.view, &t.s.cx, size, scale, &th);
        t.s.run(400);
        let img = render_view_cx(&mut *t.s.view, &t.s.cx, size, scale, &th);
        let d = kit::snap::dir();
        let _ = std::fs::create_dir_all(&d);
        save_png(&img, d.join(format!("{name}-{}-{}x.png", if dark { "dark" } else { "light" }, scale as u32)));
        img
    }

    fn mv(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
    }
    fn click(t: &mut T, p: (f32, f32)) {
        t.s.send(&mv(p.0, p.1));
        t.s.send(&InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
        t.s.send(&InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE });
        t.s.pump();
    }
    /// Window coordinates of the centre of click target `id` on a tall page (everything visible).
    fn target(t: &mut T, id: Id) -> (f32, f32) {
        render_view_cx(&mut *t.s.view, &t.s.cx, (700.0, 1600.0), 1.0, &Theme::settings_dark());
        let v = t.v.borrow();
        let r = v.clicks.rect_of(&id).unwrap_or_else(|| panic!("no target {id:?}"));
        (r.cx() + v.margin, r.cy() + v.margin.min(16.0) - v.scroll.offset())
    }
    fn cmds(f: &Fakes) -> Vec<String> {
        f.api.calls().into_iter().map(|c| c.name).collect()
    }

    #[test]
    fn mount_reads_the_status_and_subscribes() {
        let t = sim("fresh");
        assert_eq!(cmds(&t.f), vec!["aisetup_status", "voice_has_key"]);
        assert_eq!(t.f.bus.subscribers("aisetup:progress"), 1);
        assert!(matches!(t.v.borrow().vw, ms::View::Page { .. }));
        assert!(t.f.api.calls().iter().all(|c| c.caller == "test"));
    }

    #[test]
    fn nothing_downloads_without_a_click_and_the_install_button_does() {
        let mut t = sim("fresh");
        assert!(!cmds(&t.f).iter().any(|c| c.starts_with("aisetup_whisper") || c.contains("ollama")));
        let p = target(&mut t, Id::Install);
        click(&mut t, p);
        let c = t.f.api.calls();
        let call = c.iter().find(|c| c.name == "aisetup_whisper_install").expect("install call");
        assert_eq!(call.args, json!({ "model": "small" }));
    }

    #[test]
    fn choosing_a_chip_changes_the_plan_and_pops() {
        let mut t = sim("fresh");
        let p = target(&mut t, Id::Chip(2));
        click(&mut t, p);
        assert_eq!(t.v.borrow().m.selected_model(), "turbo");
        assert!(!t.v.borrow().cues.is_empty(), "the chip pops");
        let p = target(&mut t, Id::Install);
        click(&mut t, p);
        let args = t.f.api.calls().into_iter().find(|c| c.name == "aisetup_whisper_install").unwrap().args;
        assert_eq!(args, json!({ "model": "turbo" }));
    }

    #[test]
    fn progress_events_move_the_bar_and_lock_the_buttons() {
        let mut t = sim("downloading");
        {
            let v = t.v.borrow();
            let ms::View::Page { voice, .. } = &v.vw else { panic!() };
            assert!(voice.progress.running && (41..=42).contains(&voice.progress.fill_pct));
            assert!(matches!(voice.plan, Plan::Download { enabled: false, .. }));
            assert!(v.built.hits.iter().all(|(id, _)| *id != Id::Install), "a locked button is not a target");
        }
        t.f.bus.emit("aisetup:progress", json!({ "id": "whisper", "item": "small", "phase": "download", "done": 300_000_000.0, "total": 487_601_967.0, "message": "" }));
        t.s.pump();
        t.s.run(300);
        {
            let v = t.v.borrow();
            let ms::View::Page { voice, .. } = &v.vw else { panic!() };
            assert_eq!(voice.progress.fill_pct, 61);
            assert!((v.bars.get(&Job::Whisper).unwrap().value().unwrap() - 0.61).abs() < 0.01);
        }
        let p = target(&mut t, Id::Cancel);
        click(&mut t, p);
        assert!(cmds(&t.f).contains(&"aisetup_cancel".to_string()));
    }

    #[test]
    fn a_failed_install_shows_the_error_line() {
        let mut t = sim("error");
        render_view_cx(&mut *t.s.view, &t.s.cx, (700.0, 1600.0), 1.0, &Theme::settings_dark());
        let v = t.v.borrow();
        let ms::View::Page { voice, .. } = &v.vw else { panic!() };
        assert_eq!(voice.progress.result, Some((false, "SHA-256 mismatch".into())));
        assert!(v.built.items.iter().any(|i| matches!(&i.k, K::Text { s, c: Col::Red, .. } if s == "SHA-256 mismatch")));
    }

    #[test]
    fn unreadable_state_offers_check_again() {
        let (svc, f) = Services::fake();
        f.api.reply("aisetup_status", Err("boom".into()));
        let v = Rc::new(RefCell::new(AiSetupScreen::new()));
        let mut s = Sim::with_services(Box::new(Shared(v.clone())), svc);
        s.show();
        s.pump();
        let mut t = T { s, v, f };
        let p = target(&mut t, Id::CheckAgain);
        let n = cmds(&t.f).len();
        click(&mut t, p);
        assert_eq!(cmds(&t.f).len(), n + 2, "status + voice_has_key again");
    }

    #[test]
    fn go_buttons_forward_to_the_island_not_the_backend() {
        let mut t = sim("ready");
        let _ = crate::ui::tabs::b::take_fx();
        let p = target(&mut t, Id::GoAi);
        click(&mut t, p);
        assert_eq!(crate::ui::tabs::b::take_fx(), vec![crate::ui::tabs::fx::TabFx::GoPage("ai")]);
    }

    #[test]
    fn the_page_never_holds_a_key() {
        // the only secret-adjacent call is a yes / no; the view has no text field at all
        let t = sim("ready");
        let c = t.f.api.calls();
        assert_eq!(c.iter().find(|c| c.name == "voice_has_key").unwrap().args, json!({}));
        assert!(!format!("{c:?}").to_lowercase().contains("sk-"));
    }

    #[test]
    fn switch_and_server_buttons_send_their_calls() {
        let mut t = sim("ready");
        let p = target(&mut t, Id::UseServer(1));
        click(&mut t, p);
        let c = t.f.api.calls();
        let u = c.iter().find(|c| c.name == "aisetup_ai_use").expect("use call");
        assert_eq!(u.args, json!({ "server": "freellm", "model": "auto" }));
        // the picker cycles to the next model
        let p = target(&mut t, Id::Pick(1));
        click(&mut t, p);
        let p = target(&mut t, Id::UseServer(1));
        click(&mut t, p);
        let last = t.f.api.calls().into_iter().filter(|c| c.name == "aisetup_ai_use").last().unwrap();
        assert_eq!(last.args, json!({ "server": "freellm", "model": "gpt-oss-120b" }));
    }

    #[test]
    fn layout_fits_the_width_and_is_taller_than_a_screen() {
        let mut t = sim("fresh");
        pic(&mut t, "aisetup-probe", (700.0, 300.0), 1.0, true);
        let v = t.v.borrow();
        assert!(v.built.height > 700.0, "{}", v.built.height);
        let w = 700.0 - 32.0;
        for it in &v.built.items {
            assert!(it.r.right() <= w + 0.5 && it.r.x >= -0.5, "{:?} sticks out of the column ({w})", it.r);
        }
    }

    #[test]
    fn scrolling_moves_the_content() {
        let mut t = sim("fresh");
        let th = Theme::settings_dark();
        render_view_cx(&mut *t.s.view, &t.s.cx, (700.0, 400.0), 1.0, &th);
        t.s.send(&InputEvent::Wheel { pos: (300.0, 200.0), delta: (0.0, 300.0), mods: Modifiers::NONE, precise: true });
        t.s.run(200);
        render_view_cx(&mut *t.s.view, &t.s.cx, (700.0, 400.0), 1.0, &th);
        assert!(t.v.borrow().scroll.offset() > 100.0);
    }

    #[test]
    fn reduced_motion_plays_no_cues() {
        let _g = motion_lock();
        let mut t = sim("fresh");
        t.s.cx.reduced_motion = true;
        t.s.step_frame();
        let p = target(&mut t, Id::Chip(0));
        click(&mut t, p);
        assert!(t.v.borrow().cues.is_empty());
    }

    #[test]
    fn a_settled_page_is_idle() {
        let _g = motion_lock();
        glasskit::ease::force_reduced(Some(false));
        let mut t = sim("ready");
        render_view_cx(&mut *t.s.view, &t.s.cx, (700.0, 560.0), 1.0, &Theme::settings_dark());
        t.s.run(2000);
        let idle = !t.s.frame.wants_frame();
        glasskit::ease::force_reduced(None);
        assert!(idle, "a settled page costs no frames");
    }

    #[test]
    fn snapshots_of_the_four_states() {
        let _g = motion_lock();
        glasskit::ease::force_reduced(Some(false));
        for name in ["fresh", "ready", "downloading", "error"] {
            let mut t = sim(name);
            for dark in [true, false] {
                for scale in [1.0, 2.0] {
                    let img = pic(&mut t, &format!("aisetup-{name}"), (700.0, 560.0), scale, dark);
                    assert!(img.count(None, |p| p[3] > 0) > 1000);
                }
                pic(&mut t, &format!("aisetup-{name}-full"), (700.0, 1500.0), 1.0, dark);
            }
        }
        glasskit::ease::force_reduced(Some(true));
        let mut t = sim("downloading");
        t.s.cx.reduced_motion = true;
        pic(&mut t, "aisetup-downloading-reduced", (700.0, 560.0), 1.0, true);
        glasskit::ease::force_reduced(None);
    }
}
