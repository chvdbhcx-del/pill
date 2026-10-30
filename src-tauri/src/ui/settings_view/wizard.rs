//! The first-run wizard (page "welcome"): four steps built from the same rows as the pages, a title row with Skip, progress dots, Back / Next.
use super::nav::RISE;
use super::page::*;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use crate::ui::view::Cx;
use glasskit::controls::{Button, ButtonKind, Widget};
use glasskit::ease::{curves, Tween};
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::schema::{schema, Action, ButtonStyle, Control, Row};
use pill_ui_core::settings::tr::{tr, Tr};
use pill_ui_core::settings::wizard::{self, Content, Dot};
use serde_json::Value;
use std::sync::OnceLock;

pub struct WizState {
    on: bool,
    dx: f32,
    dy: f32,
    pub dots: [Tween<f32>; 4],
}

impl Default for WizState {
    fn default() -> Self {
        WizState { on: false, dx: 0.0, dy: 0.0, dots: std::array::from_fn(|_| Tween::at_rest(1.0)) }
    }
}

impl WizState {
    pub fn active(&self) -> bool {
        self.on
    }
    pub fn end_chrome(&mut self) {
        self.on = false;
    }
    pub fn enter_dx(&self) -> (f32, f32) {
        (self.dx, self.dy)
    }
}

/// The two rows the wizard adds to the page rows: "Change the keys" (step 3) and the AI setup button (step 4).
fn extra_row(i: usize) -> &'static Row {
    static ROWS: OnceLock<[Row; 2]> = OnceLock::new();
    let rows = ROWS.get_or_init(|| {
        [
            Row::new("wizard.hotkeys", "", tr("Поменять клавиши", "Change the keys"), Control::Button { label: tr("Открыть «Клавиши»", "Open Hotkeys"), action: Action::OpenPage("hotkeys"), style: ButtonStyle::Small }, Value::Null),
            Row::new("wizard.ai", "", tr("ИИ и голос в текст", "AI and voice to text"), Control::Button { label: tr("Установка ИИ", "AI setup"), action: Action::OpenPage("aisetup"), style: ButtonStyle::Small }, Value::Null)
                .hint(tr("По желанию. Пока вы сами не нажмёте кнопку, ничего не скачивается, не включается и не отправляется в сеть.", "Optional. Nothing is downloaded, turned on or sent over the network until you press a button.")),
        ]
    });
    &rows[i]
}

impl SettingsView {
    pub(super) fn start_wizard(&mut self, cx: &mut Cx) {
        self.model.start_wizard();
        self.wiz.on = true;
        self.leaving = None;
        self.want_page = None;
        self.rebuild_wizard();
        cx.request_frame();
    }

    /// Draws the current step again (step change, language change inside the wizard).
    pub(super) fn rebuild_wizard(&mut self) {
        if !self.model.wizard.is_active() {
            return;
        }
        let reduce = self.reduce();
        let spec = self.model.wizard.enter_anim(reduce);
        self.wiz.dx = spec.dx as f32;
        self.wiz.dy = spec.dy as f32;
        let pg = self.build_wizard_page();
        self.page = pg;
        self.enter_t = Tween::new(0.0, 1.0, spec.dur_ms as f32 / 1000.0, RISE);
        let step = self.model.wizard.step() as usize;
        for (i, d) in self.wiz.dots.iter_mut().enumerate() {
            let to = if i + 1 == step { 1.5 } else { 1.0 };
            if reduce {
                *d = Tween::at_rest(to);
            } else {
                d.retarget(to, 0.4, curves::SPRING_POP);
            }
        }
        self.ui.reset();
        self.focus.set_order(&self.focus_order());
    }

    fn build_wizard_page(&mut self) -> Page {
        let lang = self.lang();
        let mut pg = Page::empty();
        pg.id = "welcome".into();
        pg.wizard = true;
        pg.blocks.push(Block::WizTop);
        let step = self.model.wizard.step();
        let mut rows: Vec<RowRt> = Vec::new();
        let mut mk = |this: &mut SettingsView, r: &'static Row, pg: &mut Page| {
            let f = this.fid();
            pg.fids.push(f);
            let rt = new_row(r, &this.model, &this.th, &this.dy, lang, f);
            rt
        };
        match wizard::content(step) {
            Content::Language => {
                if let Some(r) = schema().row("language") {
                    rows.push(mk(self, r, &mut pg));
                }
            }
            Content::Modules { note } => {
                pg.blocks.push(Block::Note(note.get(lang).to_string()));
                for id in wizard::module_row_ids() {
                    if let Some(r) = schema().row(id) {
                        rows.push(mk(self, r, &mut pg));
                    }
                }
            }
            Content::Hotkeys { note, .. } => {
                pg.blocks.push(Block::Note(note.get(lang).to_string()));
                for (action, _, _) in wizard::hotkey_rows() {
                    if let Some(r) = schema().row(&format!("hotkey.{action}")) {
                        let mut rt = mk(self, r, &mut pg);
                        rt.ctl = Ctl::Keys(self.model.combo_of(action));
                        rt.fid = 0;
                        rows.push(rt);
                    }
                }
                rows.push(mk(self, extra_row(0), &mut pg));
            }
            Content::Ai { .. } => rows.push(mk(self, extra_row(1), &mut pg)),
        }
        pg.blocks.push(Block::Card(rows));
        pg.blocks.push(Block::WizNav);
        pg
    }
}

impl SettingsView {
    fn wiz_button(&mut self, d: &Dc, id: &str, label: &str, kind: ButtonKind, small: bool, act: Act, right: f32, y: f32) -> f32 {
        let (cv, th) = (d.cv, d.th);
        let k = key(&format!("wiz:{id}"));
        let b = self.ui.button(k, || Button::new(label).kind(kind).small(small));
        b.label = label.to_string();
        b.kind = kind;
        b.small = small;
        let w = b.natural_size(cv, th).0;
        let r = Rect::new(right - w, y, w, if small { 28.0 } else { 32.0 });
        b.draw(cv, th, r);
        self.ui.add(k, r, act, true, true, true);
        w
    }

    pub(super) fn paint_wiz_top(&mut self, d: &Dc, y: f32) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let Some(v) = self.model.wizard.view(d.lang) else { return 0.0 };
        cv.text(Rect::new(4.0, y + 8.0, CARD_W - 120.0, 24.0), v.title, Style::new(20.0, 600), p.text);
        self.wiz_button(d, "skip", v.skip_label, ButtonKind::Normal, true, Act::WizSkip, CARD_W, y + 6.0);
        let sy = y + 48.0;
        cv.text(Rect::new(4.0, sy, 200.0, 16.0), &super::txt::upper(&v.step_label), Style::new(11.0, 600).track(0.88), p.cap);
        let mut x = CARD_W - 6.0;
        for (i, dot) in v.dots.iter().enumerate().rev() {
            let s = self.wiz.dots[i].value().max(0.5);
            let col = match dot {
                Dot::Current => p.accent,
                Dot::Past => p.accent_line,
                Dot::Future => d.th.track,
            };
            cv.fill_circle((x - 3.0, sy + 8.0), 3.0 * s, col);
            x -= 14.0;
        }
        72.0 + 0.0
    }

    pub(super) fn paint_wiz_nav(&mut self, d: &Dc, y: f32) -> f32 {
        let Some(v) = self.model.wizard.view(d.lang) else { return 0.0 };
        let top = y + 12.0;
        let w = self.wiz_button(d, "next", v.next_label, ButtonKind::Primary, false, Act::WizNext, CARD_W, top);
        if v.back_visible {
            self.wiz_button(d, "back", v.back_label, ButtonKind::Normal, false, Act::WizBack, CARD_W - w - 8.0, top);
        }
        12.0 + 32.0
    }
}

pub(super) fn tr_unused(_: Tr) {}
