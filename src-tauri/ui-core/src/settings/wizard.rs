//! First-run wizard (page id "welcome"): four short steps, Skip always visible, finished by a stored flag.
//!
//! Who gets it: `first_run_gate` (a port of `settings.rs::first_run_gate`): only a start with no settings file and nothing
//! stored is a new install; every other start marks `firstRunDone` and never shows it. General > "Run again" starts it by hand.
//! Nothing opt-in is turned on and nothing is downloaded by the wizard: module switches start at their defaults, the AI step
//! only points to AI setup. Steps slide in from the side they came from (data: `anim::spec`).
use crate::settings::anim::{self, AnimKind, AnimSpec};
use crate::settings::calls::BackendCall;
use crate::settings::hotkey;
use crate::settings::schema::schema;
use crate::settings::tr::{tr, Lang, Tr};
use serde_json::{json, Map, Value};

pub const STEPS: u8 = 4;
pub const TITLE: Tr = tr("Добро пожаловать в Pill", "Welcome to Pill");
pub const SKIP: Tr = tr("Пропустить", "Skip");
pub const BACK: Tr = tr("Назад", "Back");
pub const NEXT: Tr = tr("Далее", "Next");
pub const DONE: Tr = tr("Готово", "Done");

/// The three outcomes of the start-up check.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FirstRun {
    /// New install: open the wizard (the backend waits ~2.5 s after start).
    Show,
    /// An existing install without the flag (every older version): store `firstRunDone = true`, no wizard.
    MarkExisting,
    Done,
}

/// `file_existed`: settings.json was on disk at startup; `stored`: what it held (no defaults merged).
pub fn first_run_gate(file_existed: bool, stored: &Map<String, Value>) -> FirstRun {
    if stored.get("firstRunDone").and_then(Value::as_bool) == Some(true) {
        FirstRun::Done
    } else if file_existed || !stored.is_empty() {
        FirstRun::MarkExisting
    } else {
        FirstRun::Show
    }
}

/// "Шаг 2 из 4" / "Step 2 of 4".
pub fn step_label(lang: Lang, step: u8) -> String {
    match lang {
        Lang::Ru => format!("Шаг {step} из {STEPS}"),
        Lang::En => format!("Step {step} of {STEPS}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dot {
    Past,
    Current,
    Future,
}

/// What a wizard step lists.
#[derive(Clone, Debug, PartialEq)]
pub enum Content {
    /// Step 1: the language row (`language`).
    Language,
    /// Step 2: every `module.*` switch, with the note above.
    Modules { note: Tr },
    /// Step 3: the first six hotkeys as read-only keycaps, the note above, a button to the Hotkeys page.
    Hotkeys { note: Tr, change_label: Tr, open_label: Tr },
    /// Step 4: the AI / voice row with a button to the AI setup page.
    Ai { label: Tr, hint: Tr, button: Tr },
}

pub fn content(step: u8) -> Content {
    match step {
        1 => Content::Language,
        2 => Content::Modules { note: tr("Включите то, чем будете пользоваться. Выключенный модуль прячет свою вкладку; всё можно изменить позже в «Модули».", "Keep what you will use. A module that is off hides its tab; you can change it later in Modules.") },
        3 => Content::Hotkeys {
            note: tr("Главные сочетания клавиш. Остальные и смена любой клавиши — на странице «Клавиши».", "The main shortcuts. The rest, and changing any key, are on the Hotkeys page."),
            change_label: tr("Поменять клавиши", "Change the keys"),
            open_label: tr("Открыть «Клавиши»", "Open Hotkeys"),
        },
        _ => Content::Ai {
            label: tr("ИИ и голос в текст", "AI and voice to text"),
            hint: tr("По желанию. Пока вы сами не нажмёте кнопку, ничего не скачивается, не включается и не отправляется в сеть.", "Optional. Nothing is downloaded, turned on or sent over the network until you press a button."),
            button: tr("Установка ИИ", "AI setup"),
        },
    }
}

/// The ids of the module rows step 2 shows (`module.<id>` rows of the Modules page, in order).
pub fn module_row_ids() -> Vec<&'static str> {
    schema().page("modules").map(|p| p.rows().filter(|r| r.key.starts_with("module.")).map(|r| r.id).collect()).unwrap_or_default()
}

/// The `(action, title, default)` triples step 3 shows.
pub fn hotkey_rows() -> Vec<(&'static str, Tr, &'static str)> {
    hotkey::wizard_actions().iter().map(|a| (a.id, a.title, a.default)).collect()
}

/// Everything the renderer needs for the current step.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub step: u8,
    pub title: &'static str,
    pub step_label: String,
    pub dots: Vec<Dot>,
    pub content: Content,
    pub back_visible: bool,
    pub next_label: &'static str,
    pub skip_label: &'static str,
    pub back_label: &'static str,
}

/// The wizard state: the step on screen, the direction of the last move and the step last drawn (for the slide).
#[derive(Debug, Default)]
pub struct Wizard {
    step: u8,
    dir: i8,
    shown: u8,
}

impl Wizard {
    pub fn is_active(&self) -> bool {
        self.step != 0
    }
    pub fn step(&self) -> u8 {
        self.step
    }

    /// "Run again" / the `welcome` page request: back to step 1. (The caller stops the key recorder and the page cleanups.)
    pub fn start(&mut self) {
        self.step = 1;
        self.shown = 0;
        self.dir = 0;
    }

    pub fn back(&mut self) {
        if self.step > 1 {
            self.step -= 1;
            self.dir = -1;
        }
    }

    /// Next on steps 1-3; on the last step it finishes. Returns the finishing calls in that case.
    pub fn advance(&mut self) -> Option<Vec<BackendCall>> {
        if self.step == STEPS {
            return Some(self.finish("general"));
        }
        if self.step > 0 {
            self.step += 1;
            self.dir = 1;
        }
        None
    }

    pub fn skip(&mut self) -> Vec<BackendCall> {
        self.finish("general")
    }

    /// The step-3 and step-4 buttons end the wizard and open another page ("hotkeys", "aisetup").
    pub fn finish_to(&mut self, page: &str) -> Vec<BackendCall> {
        self.finish(page)
    }

    /// Mark done and leave: store `firstRunDone`, rebuild the sidebar (the wizard hid its chrome), show `page`.
    fn finish(&mut self, page: &str) -> Vec<BackendCall> {
        self.step = 0;
        self.dir = 0;
        self.shown = 0;
        vec![BackendCall::put("firstRunDone", json!(true)), BackendCall::UiRebuildSidebar, BackendCall::UiShowPage { page: page.to_string() }]
    }

    /// Leave without storing the flag (a `settings:page` request to another page while the wizard is up).
    pub fn abandon(&mut self) {
        self.step = 0;
        self.dir = 0;
        self.shown = 0;
    }

    /// The animation for drawing the current step, then remembers what was shown. A new step slides in from the side it came
    /// from (+28 / -28 px); the first draw rises 10 px; redrawing the same step (language change) rises 4 px.
    pub fn enter_anim(&mut self, reduce: bool) -> AnimSpec {
        let kind = if self.shown != 0 && self.shown != self.step {
            if self.dir < 0 {
                AnimKind::WizardBack
            } else {
                AnimKind::WizardForward
            }
        } else if self.shown != 0 {
            AnimKind::WizardRefresh
        } else {
            AnimKind::WizardFirst
        };
        self.shown = self.step;
        self.dir = 0;
        anim::spec(kind, reduce)
    }

    pub fn view(&self, lang: Lang) -> Option<View> {
        if !self.is_active() {
            return None;
        }
        Some(View {
            step: self.step,
            title: TITLE.get(lang),
            step_label: step_label(lang, self.step),
            dots: (1..=STEPS).map(|k| if k == self.step { Dot::Current } else if k < self.step { Dot::Past } else { Dot::Future }).collect(),
            content: content(self.step),
            back_visible: self.step > 1,
            next_label: if self.step == STEPS { DONE.get(lang) } else { NEXT.get(lang) },
            skip_label: SKIP.get(lang),
            back_label: BACK.get(lang),
        })
    }

    /// Changing the language inside the wizard: the sidebar and the wizard are rebuilt in place (the pages stay closed).
    pub fn language_changed_calls(lang_setting: &str) -> Vec<BackendCall> {
        vec![BackendCall::put("language", json!(lang_setting)), BackendCall::UiSetLanguage { lang: lang_setting.to_string() }, BackendCall::UiRebuildSidebar, BackendCall::UiRedrawWizard]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    // ---- first_run_wizard_gate of settings.rs, one to one ----
    #[test]
    fn first_run_gate_cases() {
        let none = Map::new();
        let some = cfg(json!({ "theme": "dark", "lastTab": "home" }));
        let done = cfg(json!({ "firstRunDone": true }));
        let undone = cfg(json!({ "firstRunDone": false, "theme": "dark" }));
        assert_eq!(first_run_gate(false, &none), FirstRun::Show); // no file, nothing stored: a new install
        assert_eq!(first_run_gate(true, &none), FirstRun::MarkExisting); // an empty or corrupt file that was moved aside
        assert_eq!(first_run_gate(true, &some), FirstRun::MarkExisting); // an existing install without the flag
        assert_eq!(first_run_gate(false, &some), FirstRun::MarkExisting);
        assert_eq!(first_run_gate(true, &undone), FirstRun::MarkExisting);
        assert_eq!(first_run_gate(true, &done), FirstRun::Done);
        assert_eq!(first_run_gate(false, &done), FirstRun::Done);
    }

    #[test]
    fn four_steps_forward_and_back() {
        let mut w = Wizard::default();
        assert!(!w.is_active() && w.view(Lang::Ru).is_none());
        w.start();
        let v = w.view(Lang::En).unwrap();
        assert_eq!((v.step, v.step_label.as_str(), v.back_visible, v.next_label, v.skip_label), (1, "Step 1 of 4", false, "Next", "Skip"));
        assert_eq!(v.dots, [Dot::Current, Dot::Future, Dot::Future, Dot::Future]);
        assert!(w.advance().is_none() && w.advance().is_none() && w.advance().is_none());
        let v = w.view(Lang::Ru).unwrap();
        assert_eq!((v.step, v.next_label, v.step_label.as_str()), (4, "Готово", "Шаг 4 из 4"));
        assert_eq!(v.dots, [Dot::Past, Dot::Past, Dot::Past, Dot::Current]);
        w.back();
        assert_eq!(w.step(), 3);
        w.back();
        w.back();
        w.back();
        assert_eq!(w.step(), 1, "Back stops at step 1");
    }

    #[test]
    fn done_and_skip_store_the_flag_and_open_general() {
        let mut w = Wizard::default();
        w.start();
        let calls = w.skip();
        assert_eq!(calls, vec![BackendCall::put("firstRunDone", json!(true)), BackendCall::UiRebuildSidebar, BackendCall::UiShowPage { page: "general".into() }]);
        assert!(!w.is_active());
        w.start();
        w.advance();
        w.advance();
        w.advance();
        assert_eq!(w.advance().unwrap()[2], BackendCall::UiShowPage { page: "general".into() });
        w.start();
        assert_eq!(w.finish_to("hotkeys")[2], BackendCall::UiShowPage { page: "hotkeys".into() });
        w.start();
        w.abandon();
        assert!(!w.is_active());
    }

    #[test]
    fn steps_slide_in_from_the_side_they_came_from() {
        let mut w = Wizard::default();
        w.start();
        let a = w.enter_anim(false);
        assert_eq!((a.kind, a.dy, a.dx), (AnimKind::WizardFirst, 10, 0));
        w.advance();
        let a = w.enter_anim(false);
        assert_eq!((a.kind, a.dx, a.dur_ms), (AnimKind::WizardForward, 28, 500));
        w.back();
        assert_eq!(w.enter_anim(false).kind, AnimKind::WizardBack);
        assert_eq!(w.enter_anim(false).kind, AnimKind::WizardRefresh, "a redraw of the same step");
        w.advance();
        assert!(w.enter_anim(true).is_instant(), "Reduce motion: no slide");
    }

    #[test]
    fn step_contents_come_from_the_schema() {
        assert!(matches!(content(1), Content::Language));
        assert_eq!(module_row_ids().len(), 11);
        assert!(module_row_ids().iter().all(|id| schema().row(id).is_some_and(|r| r.default == json!(true))));
        let hk = hotkey_rows();
        assert_eq!(hk.len(), 6);
        assert_eq!((hk[0].0, hk[0].2), ("toggle", "Ctrl+Alt+I"));
        match content(4) {
            Content::Ai { button, .. } => assert_eq!(button.en, "AI setup"),
            c => panic!("{c:?}"),
        }
        for s in 1..=STEPS {
            if let Content::Modules { note } | Content::Hotkeys { note, .. } = content(s) {
                assert!(note.is_complete());
            }
        }
    }

    #[test]
    fn language_inside_the_wizard_redraws_in_place() {
        let c = Wizard::language_changed_calls("en");
        assert_eq!(c.len(), 4);
        assert_eq!(c[2], BackendCall::UiRebuildSidebar);
        assert_eq!(c[3], BackendCall::UiRedrawWizard);
    }
}
