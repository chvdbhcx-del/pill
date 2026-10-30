//! AI quick actions for a clipboard item (src/ai-actions.js): a panel over the Clipboard tab with Translate / Shorten / Fix / Explain.
//! Opened from a clipboard row's sparkle button (`Action::Open{id}`); the original item is never changed and nothing is stored.
//!
//! Backend: `ai_clip_text {id}`, `ai_transform {text, action}` (-> `{ok, text, clipped}` | `{ok:false, error}`),
//! `ai_transform_cancel`, `clip_paste_text {text, copyOnly}` (-> "pasted" | "copied" | "copied_only"), `settings_open {page:"ai"}`.
//! Screens: Menu -> Working (spinner + Cancel) -> Result (output text, Copy / Paste / Back / Close) | Error (message, Retry / Enable AI /
//! Back / Close). Every request carries a job number; a reply whose job is stale (Back, Cancel, Close or a new Open happened) is
//! dropped. Escape closes the panel, not the island (the renderer sends `Action::Close`). Model output is plain text, never markup.
//! Deliberate difference: opening the panel while a request still runs cancels that request (the JS only ignored its answer).
//! The host should hand keyboard focus back to the app below before `Copy`/`Paste` runs (the JS blurred the input and waited 120 ms).

use super::{Anim, BackendCall, Effect, Fx, Lang, Reply, Tags, ToastSpec, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Translate,
    Shorten,
    Fix,
    Explain,
}
pub const ACTS: [Act; 4] = [Act::Translate, Act::Shorten, Act::Fix, Act::Explain];
impl Act {
    pub fn id(self) -> &'static str {
        match self {
            Act::Translate => "translate",
            Act::Shorten => "shorten",
            Act::Fix => "fix",
            Act::Explain => "explain",
        }
    }
    pub fn from_id(s: &str) -> Option<Act> {
        ACTS.into_iter().find(|a| a.id() == s)
    }
    fn icon(self) -> &'static str {
        match self {
            Act::Translate => "globe",
            Act::Shorten => "convert",
            Act::Fix => "edit",
            Act::Explain => "sparkles",
        }
    }
    fn label(self, l: Lang) -> &'static str {
        match self {
            Act::Translate => l.t("Перевести", "Translate"),
            Act::Shorten => l.t("Сократить", "Shorten"),
            Act::Fix => l.t("Исправить ошибки", "Fix spelling and grammar"),
            Act::Explain => l.t("Объяснить просто", "Explain simply"),
        }
    }
    fn sub(self, l: Lang) -> &'static str {
        match self {
            Act::Translate => l.t("на английский или русский", "to English or Russian"),
            Act::Shorten => l.t("примерно вдвое", "to about half"),
            Act::Fix => l.t("орфография и грамматика", "spelling and grammar"),
            Act::Explain => l.t("простыми словами", "in plain words"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// the sparkle button of clipboard item `id`
    Open { id: i64 },
    Close,
    Run(Act),
    Cancel,
    /// "Back"
    Menu,
    Copy,
    Paste,
    /// "Turn AI on" (opens Settings > AI)
    EnableAi,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub act: Act,
    pub icon: &'static str,
    pub label: String,
    pub sub: String,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    Menu(Vec<MenuItem>),
    Working { cancel_label: String, working_label: String },
    Result { text: String, clipped_note: Option<String>, copy: String, paste: String, back: String, close: String },
    Error { text: String, retry: Option<String>, enable: Option<String>, back: String, close: String },
}
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub open: bool,
    pub title: String,
    pub close_title: String,
    pub screen: Option<Screen>,
}

#[derive(Debug)]
enum St {
    Closed,
    Menu,
    Working(Act),
    Result { act: Act, text: String, clipped: bool },
    Error { act: Act, code: String },
}
#[derive(Debug)]
enum Pend {
    Clip { job: u64, act: Act },
    Transform { job: u64, act: Act },
    Put { copy_only: bool },
    Other,
}

pub struct AiActions {
    pub fx: Fx,
    lang: Lang,
    tags: Tags,
    pend: HashMap<u64, Pend>,
    st: St,
    job: u64,
    running: bool,
    clip_id: i64,
    closed: bool,
}

impl AiActions {
    pub fn new(lang: Lang) -> Self {
        Self { fx: Fx::default(), lang, tags: Tags::new(7), pend: HashMap::new(), st: St::Closed, job: 0, running: false, clip_id: 0, closed: false }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
    }
    pub fn is_open(&self) -> bool {
        !matches!(self.st, St::Closed)
    }
    /// True once after the panel closed (the clipboard tab's `onClose`).
    pub fn take_closed(&mut self) -> bool {
        std::mem::take(&mut self.closed)
    }
    fn call(&mut self, cmd: &'static str, args: Value, p: Pend) -> BackendCall {
        let tag = self.tags.next();
        self.pend.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }
    /// every screen change re-enters the body (y 4, 250 ms)
    fn body(&mut self) {
        self.fx.anim("ai-body", Anim::Enter { y: 4.0, ms: 250 });
    }

    fn close(&mut self) -> Vec<BackendCall> {
        if !self.is_open() {
            return vec![];
        }
        let mut calls = vec![];
        if self.running {
            calls.push(self.call("ai_transform_cancel", json!({}), Pend::Other));
        }
        self.job += 1;
        self.running = false;
        self.st = St::Closed;
        self.closed = true;
        self.fx.anim("ai-panel", Anim::Exit { y: 4.0, scale: 1.0, ms: 180 });
        calls
    }
    fn menu(&mut self) {
        self.running = false;
        self.st = St::Menu;
        self.body();
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Open { id } => {
                let mut calls = vec![];
                if self.running {
                    calls.push(self.call("ai_transform_cancel", json!({}), Pend::Other));
                }
                self.clip_id = id;
                self.job += 1;
                self.running = false;
                self.fx.anim("ai-panel", Anim::Enter { y: 6.0, ms: 220 });
                self.menu();
                calls
            }
            Action::Close => self.close(),
            Action::Menu => {
                if !self.is_open() {
                    return vec![];
                }
                self.job += 1;
                self.menu();
                vec![]
            }
            Action::Run(act) => {
                if !self.is_open() {
                    return vec![];
                }
                self.job += 1;
                self.running = true;
                self.st = St::Working(act);
                self.body();
                self.fx.anim("ai-wait", Anim::Spin);
                let job = self.job;
                vec![self.call("ai_clip_text", json!({ "id": self.clip_id }), Pend::Clip { job, act })]
            }
            Action::Cancel => {
                self.job += 1;
                self.running = false;
                let c = self.call("ai_transform_cancel", json!({}), Pend::Other);
                self.menu();
                vec![c]
            }
            Action::Copy => self.put(true),
            Action::Paste => self.put(false),
            Action::EnableAi => vec![self.call("settings_open", json!({ "page": "ai" }), Pend::Other)],
        }
    }

    fn put(&mut self, copy_only: bool) -> Vec<BackendCall> {
        let St::Result { text, .. } = &self.st else { return vec![] };
        let text = text.clone();
        vec![self.call("clip_paste_text", json!({ "text": text, "copyOnly": copy_only }), Pend::Put { copy_only })]
    }

    fn fail(&mut self, act: Act, code: &str) {
        self.running = false;
        if code == "cancelled" {
            return self.menu();
        }
        self.st = St::Error { act, code: code.to_string() };
        self.body();
    }

    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        let Some(p) = self.pend.remove(&tag) else { return vec![] };
        match p {
            Pend::Clip { job, act } => {
                if job != self.job {
                    return vec![];
                }
                let text = r.v.as_str().unwrap_or("");
                if !r.ok || text.trim_matches(super::text::js_ws).is_empty() {
                    self.fail(act, "empty");
                    return vec![];
                }
                vec![self.call("ai_transform", json!({ "text": text, "action": act.id() }), Pend::Transform { job, act })]
            }
            Pend::Transform { job, act } => {
                if job != self.job {
                    return vec![];
                }
                self.running = false;
                if r.ok && r.v["ok"].as_bool().unwrap_or(false) {
                    self.st = St::Result { act, text: r.v["text"].as_str().unwrap_or("").to_string(), clipped: r.v["clipped"].as_bool().unwrap_or(false) };
                    self.body();
                } else {
                    let code = if r.ok { r.v["error"].as_str().filter(|e| !e.is_empty()).unwrap_or("unknown").to_string() } else { "unknown".to_string() };
                    self.fail(act, &code);
                }
                vec![]
            }
            Pend::Put { copy_only } => {
                let res = if r.ok { r.v.as_str().unwrap_or("") } else { "" };
                let title = match res {
                    "pasted" => self.lang.t("Вставлено", "Pasted"),
                    "copied" => self.lang.t("Скопировано", "Copied"),
                    "copied_only" => self.lang.t("Скопировано (цель с правами админа)", "Copied (target is elevated)"),
                    _ => self.lang.t("Не получилось", "Failed"),
                };
                self.fx.eff(Effect::Card(ToastSpec { id: "clip-ai".into(), icon: if copy_only { "copy" } else { "clipboard" }.into(), title: title.into(), ttl_ms: 1800, ..Default::default() }));
                let mut calls = vec![];
                if res == "pasted" || res == "copied_only" {
                    calls = self.close();
                    self.fx.eff(Effect::CollapsePill);
                }
                calls
            }
            Pend::Other => vec![],
        }
    }

    pub fn view(&self) -> View {
        let l = self.lang;
        let (title, screen) = match &self.st {
            St::Closed => (String::new(), None),
            St::Menu => (
                l.t("ИИ-действия", "AI actions").to_string(),
                Some(Screen::Menu(ACTS.iter().map(|a| MenuItem { act: *a, icon: a.icon(), label: a.label(l).into(), sub: a.sub(l).into() }).collect())),
            ),
            St::Working(a) => (a.label(l).into(), Some(Screen::Working { cancel_label: l.t("Отмена", "Cancel").into(), working_label: l.t("Думаю…", "Working…").into() })),
            St::Result { act, text, clipped } => (
                act.label(l).into(),
                Some(Screen::Result {
                    text: text.clone(),
                    clipped_note: clipped.then(|| l.t("Обработаны первые 8000 символов.", "Only the first 8000 characters were processed.").to_string()),
                    copy: l.t("Копировать", "Copy").into(),
                    paste: l.t("Вставить", "Paste").into(),
                    back: l.t("Назад", "Back").into(),
                    close: l.t("Закрыть", "Close").into(),
                }),
            ),
            St::Error { act, code } => {
                let text = match code.as_str() {
                    "empty" => l.t("У этой записи нет текста.", "This item has no text.").to_string(),
                    "net_off" => l.t("ИИ выключен. Включите его в настройках: текст уйдёт только на адрес, который вы там укажете.", "AI is switched off. Turn it on in Settings: the text goes only to the address you set there.").to_string(),
                    c => c.to_string(),
                };
                (
                    act.label(l).into(),
                    Some(Screen::Error {
                        text,
                        retry: (code != "net_off" && code != "empty").then(|| l.t("Повторить", "Retry").to_string()),
                        enable: (code == "net_off").then(|| l.t("Включить ИИ", "Turn AI on").to_string()),
                        back: l.t("Назад", "Back").into(),
                        close: l.t("Закрыть", "Close").into(),
                    }),
                )
            }
        };
        View { open: self.is_open(), title, close_title: l.t("Закрыть", "Close").into(), screen }
    }
    /// The action a Retry button re-runs.
    pub fn retry_act(&self) -> Option<Act> {
        match &self.st {
            St::Error { act, .. } => Some(*act),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened() -> AiActions {
        let mut a = AiActions::new(Lang::En);
        a.on_action(Action::Open { id: 42 });
        a
    }
    fn clip(text: &str) -> Reply {
        Reply::ok(json!(text))
    }
    fn tr_ok(text: &str) -> Reply {
        Reply::ok(json!({ "ok": true, "text": text, "clipped": false }))
    }
    /// Run an action to its transform call.
    fn to_transform(a: &mut AiActions, act: Act, text: &str) -> BackendCall {
        let c = a.on_action(Action::Run(act));
        assert_eq!((c[0].cmd, c[0].args.clone()), ("ai_clip_text", json!({ "id": 42 })));
        let mut next = a.on_reply(c[0].tag, clip(text));
        assert_eq!(next.len(), 1);
        next.remove(0)
    }

    #[test]
    fn menu_lists_the_four_actions() {
        let a = opened();
        let v = a.view();
        assert!(v.open && v.title == "AI actions");
        let Some(Screen::Menu(items)) = v.screen else { panic!() };
        assert_eq!(items.iter().map(|i| (i.act.id(), i.icon, i.label.as_str())).collect::<Vec<_>>(), vec![("translate", "globe", "Translate"), ("shorten", "convert", "Shorten"), ("fix", "edit", "Fix spelling and grammar"), ("explain", "sparkles", "Explain simply")]);
        let mut a = a;
        a.set_lang(Lang::Ru);
        let Some(Screen::Menu(items)) = a.view().screen else { panic!() };
        assert_eq!((items[3].label.as_str(), items[3].sub.as_str()), ("Объяснить просто", "простыми словами"));
        assert!(!AiActions::new(Lang::En).view().open);
    }
    #[test]
    fn run_fetches_the_clip_text_then_transforms_then_shows_the_result() {
        let mut a = opened();
        let t = to_transform(&mut a, Act::Translate, "привет");
        assert_eq!((t.cmd, t.args.clone()), ("ai_transform", json!({ "text": "привет", "action": "translate" })));
        let v = a.view();
        assert_eq!(v.title, "Translate");
        assert!(matches!(v.screen, Some(Screen::Working { .. })));
        a.on_reply(t.tag, Reply::ok(json!({ "ok": true, "text": "hello", "clipped": true })));
        let Some(Screen::Result { text, clipped_note, .. }) = a.view().screen else { panic!() };
        assert_eq!(text, "hello");
        assert_eq!(clipped_note.as_deref(), Some("Only the first 8000 characters were processed."));
    }
    #[test]
    fn empty_clip_text_is_an_error_without_retry() {
        let mut a = opened();
        let c = a.on_action(Action::Run(Act::Fix));
        assert!(a.on_reply(c[0].tag, clip("   \n ")).is_empty());
        let Some(Screen::Error { text, retry, enable, .. }) = a.view().screen else { panic!() };
        assert_eq!(text, "This item has no text.");
        assert!(retry.is_none() && enable.is_none());
        let c = a.on_action(Action::Run(Act::Fix));
        a.on_reply(c[0].tag, Reply::ok(Value::Null));
        assert!(matches!(a.view().screen, Some(Screen::Error { .. })));
    }
    #[test]
    fn net_off_offers_enable_other_errors_offer_retry() {
        let mut a = opened();
        let t = to_transform(&mut a, Act::Shorten, "text");
        a.on_reply(t.tag, Reply::ok(json!({ "ok": false, "error": "net_off" })));
        let Some(Screen::Error { text, retry, enable, .. }) = a.view().screen else { panic!() };
        assert!(text.starts_with("AI is switched off."));
        assert_eq!((retry, enable.as_deref()), (None, Some("Turn AI on")));
        assert_eq!(a.on_action(Action::EnableAi)[0].args, json!({ "page": "ai" }));
        let t = to_transform(&mut a, Act::Shorten, "text");
        a.on_reply(t.tag, Reply::ok(json!({ "ok": false, "error": "HTTP 500: oops" })));
        let Some(Screen::Error { text, retry, .. }) = a.view().screen else { panic!() };
        assert_eq!((text.as_str(), retry.as_deref()), ("HTTP 500: oops", Some("Retry")));
        assert_eq!(a.retry_act(), Some(Act::Shorten));
        let t = to_transform(&mut a, Act::Shorten, "text");
        a.on_reply(t.tag, Reply::ok(json!({ "ok": false })));
        let Some(Screen::Error { text, .. }) = a.view().screen else { panic!() };
        assert_eq!(text, "unknown");
        let t = to_transform(&mut a, Act::Shorten, "text");
        a.on_reply(t.tag, Reply::err("transport"));
        assert!(matches!(a.view().screen, Some(Screen::Error { .. })));
    }
    #[test]
    fn cancelled_returns_to_the_menu_quietly() {
        let mut a = opened();
        let t = to_transform(&mut a, Act::Explain, "text");
        a.on_reply(t.tag, Reply::ok(json!({ "ok": false, "error": "cancelled" })));
        assert!(matches!(a.view().screen, Some(Screen::Menu(_))));
    }
    #[test]
    fn cancel_button_cancels_and_drops_the_late_answer() {
        let mut a = opened();
        let t = to_transform(&mut a, Act::Translate, "text");
        let c = a.on_action(Action::Cancel);
        assert_eq!(c[0].cmd, "ai_transform_cancel");
        assert!(matches!(a.view().screen, Some(Screen::Menu(_))));
        a.on_reply(t.tag, tr_ok("late"));
        assert!(matches!(a.view().screen, Some(Screen::Menu(_))), "a stale answer does not replace the menu");
    }
    #[test]
    fn stale_clip_text_does_not_start_a_transform() {
        let mut a = opened();
        let c = a.on_action(Action::Run(Act::Fix));
        a.on_action(Action::Menu);
        assert!(a.on_reply(c[0].tag, clip("text")).is_empty());
    }
    #[test]
    fn close_while_running_cancels_the_request_and_reports_closed() {
        let mut a = opened();
        a.on_action(Action::Run(Act::Fix));
        let c = a.on_action(Action::Close);
        assert_eq!(c[0].cmd, "ai_transform_cancel");
        assert!(!a.is_open() && a.take_closed() && !a.take_closed());
        assert!(a.on_action(Action::Close).is_empty(), "closing a closed panel is a no-op");
        assert!(a.on_action(Action::Run(Act::Fix)).is_empty());
        // reopening while a request runs cancels it
        let mut a = opened();
        a.on_action(Action::Run(Act::Fix));
        let c = a.on_action(Action::Open { id: 7 });
        assert_eq!(c[0].cmd, "ai_transform_cancel");
        assert!(matches!(a.view().screen, Some(Screen::Menu(_))));
    }
    #[test]
    fn paste_closes_and_collapses_copy_stays_open() {
        let mut a = opened();
        let t = to_transform(&mut a, Act::Fix, "teh text");
        a.on_reply(t.tag, tr_ok("the text"));
        let c = a.on_action(Action::Copy);
        assert_eq!((c[0].cmd, c[0].args.clone()), ("clip_paste_text", json!({ "text": "the text", "copyOnly": true })));
        a.on_reply(c[0].tag, Reply::ok(json!("copied")));
        let Effect::Card(card) = a.fx.take_effects().remove(0) else { panic!() };
        assert_eq!((card.id.as_str(), card.icon.as_str(), card.title.as_str(), card.ttl_ms), ("clip-ai", "copy", "Copied", 1800));
        assert!(a.is_open());
        let c = a.on_action(Action::Paste);
        assert_eq!(c[0].args["copyOnly"], json!(false));
        a.on_reply(c[0].tag, Reply::ok(json!("pasted")));
        let fx = a.fx.take_effects();
        assert!(matches!(&fx[0], Effect::Card(c) if c.title == "Pasted" && c.icon == "clipboard"));
        assert!(fx.contains(&Effect::CollapsePill));
        assert!(!a.is_open() && a.take_closed());
    }
    #[test]
    fn put_results_elevated_and_failed() {
        let mut a = opened();
        let t = to_transform(&mut a, Act::Fix, "x");
        a.on_reply(t.tag, tr_ok("y"));
        let c = a.on_action(Action::Paste);
        a.on_reply(c[0].tag, Reply::ok(json!("copied_only")));
        let fx = a.fx.take_effects();
        assert!(matches!(&fx[0], Effect::Card(c) if c.title == "Copied (target is elevated)"));
        assert!(!a.is_open());
        let mut a = opened();
        let t = to_transform(&mut a, Act::Fix, "x");
        a.on_reply(t.tag, tr_ok("y"));
        let c = a.on_action(Action::Paste);
        a.on_reply(c[0].tag, Reply::ok(json!("nope")));
        assert!(matches!(&a.fx.take_effects()[0], Effect::Card(c) if c.title == "Failed"));
        assert!(a.is_open(), "a failed paste leaves the panel open");
        assert!(a.on_action(Action::Copy).len() == 1);
        let mut a = opened();
        assert!(a.on_action(Action::Paste).is_empty(), "nothing to paste on the menu screen");
    }
    #[test]
    fn animations_and_reduce_motion() {
        let mut a = AiActions::new(Lang::En);
        a.on_action(Action::Open { id: 1 });
        let cues = a.fx.take_anims();
        assert!(cues.iter().any(|c| c.target == "ai-panel" && c.kind == Anim::Enter { y: 6.0, ms: 220 }));
        assert!(cues.iter().any(|c| c.target == "ai-body"));
        a.on_action(Action::Run(Act::Fix));
        assert!(a.fx.take_anims().iter().any(|c| c.kind == Anim::Spin));
        a.on_action(Action::Close);
        assert!(a.fx.take_anims().iter().any(|c| matches!(c.kind, Anim::Exit { .. })));
        a.fx.reduce_motion = true;
        a.on_action(Action::Open { id: 1 });
        assert!(a.fx.take_anims().is_empty());
    }
    #[test]
    fn russian_screens() {
        let mut a = AiActions::new(Lang::Ru);
        a.on_action(Action::Open { id: 42 });
        assert_eq!(a.view().title, "ИИ-действия");
        a.on_action(Action::Run(Act::Fix));
        let Some(Screen::Working { cancel_label, working_label }) = a.view().screen else { panic!() };
        assert_eq!((cancel_label.as_str(), working_label.as_str()), ("Отмена", "Думаю…"));
        assert_eq!(Act::from_id("explain"), Some(Act::Explain));
        assert_eq!(Act::from_id("x"), None);
    }
}
