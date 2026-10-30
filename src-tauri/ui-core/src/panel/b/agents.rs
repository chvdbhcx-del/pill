//! Agents: coding-agent status (src/agents-live.js). One model feeds three surfaces:
//! * the live row of the collapsed pill (`Effect::SetLive{key:"agents"}`: waiting = amber, priority 70, beats working =
//!   spinner, priority 60) with the matching edge reaction (`attention` / `thinking`),
//! * the "agent finished" flash + card (`agents:done`): green check row (priority 75, 1.6 s) when the pill is collapsed, edge
//!   ripple 0.9 (skipped in quiet hours), card with Dismiss / Path / Folder / VS Code (VS Code only when `agents_tools` says `code`),
//! * the read-only "Agents" tab (order 66, opened by `pill.exe --open agents`): every tracked session, waiting first (the order
//!   comes from agents.rs), project = cwd folder, state text + age, rows enter staggered when the tab opens, move with a flip when
//!   the order changes, and a state change pops the row's dot. The unread count (`waiting`) goes to the tab badge.
//!
//! Backend: `agents_list`, `agents_dismiss {session}`, `agents_open_folder {session}`, `agents_open_code {session}`, `agents_tools`,
//! `agents_hook_snippets`; events `agents:changed`, `agents:done`, `settings:changed`.
//! PRIVACY: session titles come from the user's own hook files and are only ever returned as plain strings (never markup).

use super::{plural, Anim, BackendCall, Effect, Fx, Lang, LiveRow, Reply, Tags, Timer, ToastAction, ToastRun, ToastSpec, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const TAB_ID: &str = "agents";
pub const TAB_ORDER: u32 = 66;
/// `pill.exe --open agents`
pub const OPEN_ARG: &str = "agents";
pub const FLASH_MS: u32 = 1600;
pub const CARD_MS: u32 = 9000;
pub const REPAINT_MS: u32 = 15_000;

fn tool_name(t: &str) -> &str {
    match t {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        _ => "",
    }
}

/// Session titles are the first words of a prompt / reply: markdown marks `# * \` > _ [ ]` become spaces, runs collapse.
pub fn plain_title(s: &str) -> String {
    let flat: String = s.chars().map(|c| if "#*`>_[]".contains(c) { ' ' } else { c }).collect();
    flat.split(super::text::js_ws).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// "12 s" / "5 min" / "2 h 5 min"
pub fn dur(ms: i64, lang: Lang) -> String {
    let s = (ms.max(0)) / 1000;
    let (m, h) = (s / 60, s / 3600);
    if s < 60 {
        lang.t(&format!("{s} с"), &format!("{s}s")).to_string()
    } else if m < 60 {
        lang.t(&format!("{m} мин"), &format!("{m}m")).to_string()
    } else {
        lang.t(&format!("{h} ч {} мин", m % 60), &format!("{h}h {}m", m % 60)).to_string()
    }
}

/// Last folder name of a path (either separator).
pub fn project_of(cwd: &str) -> &str {
    cwd.rsplit(['/', '\\']).find(|p| !p.is_empty()).unwrap_or("")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub tool: String,
    pub session: String,
    /// waiting | working | done | stale
    pub state: String,
    pub title: String,
    pub cwd: String,
    pub project: String,
    pub ts: i64,
}
impl Session {
    fn id(&self) -> String {
        format!("{}/{}", self.tool, self.session)
    }
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub sessions: Vec<Session>,
    pub working: u32,
    pub waiting: u32,
}
impl Snapshot {
    pub fn from_json(v: &Value) -> Self {
        let n = |k: &str| v[k].as_i64().unwrap_or(0).max(0) as u32;
        let s = |x: &Value, k: &str| x[k].as_str().unwrap_or("").to_string();
        Self {
            sessions: v["sessions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|x| Session { tool: s(x, "tool"), session: s(x, "session"), state: s(x, "state"), title: s(x, "title"), cwd: s(x, "cwd"), project: s(x, "project"), ts: x["ts"].as_i64().unwrap_or(0) })
                .collect(),
            working: n("working"),
            waiting: n("waiting"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Dismiss(String),
    OpenFolder(String),
    OpenCode(String),
    CopyPath(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// `tool/session`
    pub id: String,
    pub session: String,
    pub state: String,
    pub name: String,
    pub title: String,
    pub state_text: String,
    /// index for the staggered enter (`--i`)
    pub index: usize,
    pub stagger: bool,
    /// the dot pops: the state changed since the last paint
    pub pop: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AgentsView {
    pub empty_text: Option<String>,
    pub rows: Vec<Row>,
    pub badge: u32,
}

pub struct AgentsModel {
    pub fx: Fx,
    lang: Lang,
    tags: Tags,
    pend: HashMap<u64, Pend>,
    snap: Snapshot,
    live_on: bool,
    live_was: bool,
    quiet: bool,
    collapsed: bool,
    now: i64,
    fresh_list: bool,
    staggered: bool,
    seen: HashMap<String, String>,
    pops: Vec<String>,
    popped: Vec<String>,
    repaint: Timer,
    shown: bool,
    last_live: Option<Option<LiveRow>>,
    last_react: Option<&'static str>,
}
enum Pend {
    List,
    Tools(Box<ToastSpec>),
    Other,
}

impl AgentsModel {
    pub fn new(lang: Lang) -> Self {
        Self {
            fx: Fx::default(), lang, tags: Tags::new(6), pend: HashMap::new(), snap: Snapshot::default(), live_on: true, live_was: true, quiet: false, collapsed: true, now: 0,
            fresh_list: false, staggered: false, seen: HashMap::new(), pops: vec![], popped: vec![], repaint: Timer::default(), shown: false, last_live: None, last_react: None,
        }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
    }
    /// Settings > Live events > AI agents
    pub fn set_live_on(&mut self, v: bool) {
        self.live_on = v;
    }
    pub fn set_quiet(&mut self, v: bool) {
        self.quiet = v;
    }
    /// `Pill.state === 'collapsed'`
    pub fn set_collapsed(&mut self, v: bool) {
        self.collapsed = v;
    }
    pub fn set_now(&mut self, ms: i64) {
        self.now = ms;
    }
    pub fn snapshot(&self) -> &Snapshot {
        &self.snap
    }
    fn call(&mut self, cmd: &'static str, args: Value, p: Pend) -> BackendCall {
        let tag = self.tags.next();
        self.pend.insert(tag, p);
        BackendCall::new(cmd, args, tag)
    }

    /// Events sent before the page loaded are gone: ask once.
    pub fn mount(&mut self) -> Vec<BackendCall> {
        vec![self.call("agents_list", json!({}), Pend::List)]
    }
    pub fn on_show(&mut self) {
        self.shown = true;
        self.fresh_list = true;
        self.repaint.arm(REPAINT_MS);
        self.paint_list();
    }
    pub fn on_hide(&mut self) {
        self.shown = false;
        self.repaint.cancel();
    }

    // ---- live widget ----
    fn render(&mut self) {
        let (row, react): (Option<LiveRow>, Option<&'static str>) = if !self.live_on {
            (None, None)
        } else if self.snap.waiting > 0 {
            let mut projects: Vec<&str> = vec![];
            for s in self.snap.sessions.iter().filter(|s| s.state == "waiting" && !s.project.is_empty()) {
                if !projects.contains(&s.project.as_str()) {
                    projects.push(&s.project);
                }
            }
            let which = match projects.first() {
                Some(p) => format!(" · {p}{}", if projects.len() > 1 { format!(" +{}", projects.len() - 1) } else { String::new() }),
                None => String::new(),
            };
            let n = self.snap.waiting;
            let txt = if n == 1 { self.lang.t("Нужен ваш ответ", "Needs your input").to_string() } else if self.lang.is_ru() { format!("Ждут вас: {n}") } else { format!("{n} need your input") };
            (Some(LiveRow { priority: 70, key: "wait".into(), text: format!("{txt}{which}"), accent: Some("amber"), ttl_ms: None, style: "agents-wait" }), Some("attention"))
        } else if self.snap.working > 0 {
            let n = self.snap.working;
            let txt = if n == 1 { self.lang.t("Агент работает", "Agent working").to_string() } else if self.lang.is_ru() { format!("Агентов в работе: {n}") } else { format!("{n} agents working") };
            (Some(LiveRow { priority: 60, key: "work".into(), text: txt, accent: None, ttl_ms: None, style: "agents-work" }), Some("thinking"))
        } else {
            (None, None)
        };
        if self.last_live.as_ref() != Some(&row) {
            self.fx.eff(Effect::SetLive { key: "agents".into(), row: row.clone() });
            self.last_live = Some(row);
        }
        if self.last_react != react {
            match react {
                Some(kind) => self.fx.eff(Effect::React { key: "agents".into(), kind, ttl_ms: None }), // edge: stays while any agent waits / works
                None => self.fx.eff(Effect::Unreact("agents".into())),
            }
            self.last_react = react;
        }
    }

    fn feed(&mut self, snap: Snapshot) {
        // a state change pops its dot at the next paint
        self.pops = snap.sessions.iter().filter(|s| self.seen.get(&s.id()).map_or(false, |was| *was != s.state)).map(|s| s.id()).collect();
        self.snap = snap;
        self.render();
        self.paint_list();
    }
    fn paint_list(&mut self) {
        self.fx.eff(Effect::SetBadge { tab: TAB_ID, count: self.snap.waiting });
        self.staggered = std::mem::take(&mut self.fresh_list);
        self.popped = std::mem::take(&mut self.pops);
        if self.shown {
            // freshly opened: rows come in one after another; later: waiting moves to the top by sliding, new rows drop in, gone ones fade
            self.fx.anim("agents-list", if self.staggered { Anim::Stagger } else { Anim::Flip });
            for id in self.popped.clone() {
                self.fx.anim(&format!("dot:{id}"), Anim::Pop { from_scale: 0.3, ms: 550 });
            }
        }
        self.seen = self.snap.sessions.iter().map(|s| (s.id(), s.state.clone())).collect();
    }

    // ---- finished ----
    fn on_done(&mut self, p: &Value) -> Vec<BackendCall> {
        let Some(session) = p["session"].as_str().filter(|s| !s.is_empty()) else { return vec![] };
        if !self.live_on {
            return vec![];
        }
        if !self.quiet {
            self.fx.eff(Effect::EdgePulse(0.9));
        }
        self.fx.eff(Effect::React { key: "agent-done".into(), kind: "done", ttl_ms: Some(4000) });
        let cwd = p["cwd"].as_str().unwrap_or("");
        let proj = project_of(cwd).to_string();
        if self.collapsed {
            let text = format!("{}{}", self.lang.t("Готово", "Done"), if proj.is_empty() { String::new() } else { format!(" · {proj}") });
            self.fx.eff(Effect::SetLive { key: "agent-done".into(), row: Some(LiveRow { priority: 75, key: "done".into(), text, accent: Some("green"), ttl_ms: Some(FLASH_MS), style: "agents-done" }) });
        }
        let tool = tool_name(p["tool"].as_str().unwrap_or(""));
        let sub = [plain_title(p["title"].as_str().unwrap_or("")), tool.to_string()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        let mut actions = vec![ToastAction { label: self.lang.t("Скрыть", "Dismiss").into(), kind: None, run: ToastRun::Call("agents_dismiss", json!({ "session": session })) }];
        if !cwd.is_empty() {
            actions.push(ToastAction { label: self.lang.t("Путь", "Path").into(), kind: Some("white"), run: ToastRun::CopyText(cwd.to_string()) });
            actions.push(ToastAction { label: self.lang.t("Папка", "Folder").into(), kind: Some("white"), run: ToastRun::Call("agents_open_folder", json!({ "session": session })) });
        }
        let spec = ToastSpec {
            id: format!("agent-done:{session}"),
            icon: "check".into(),
            title: format!("{}{}", self.lang.t("Агент закончил", "Agent finished"), if proj.is_empty() { String::new() } else { format!(" · {proj}") }),
            sub: if sub.is_empty() { self.lang.t("Сеанс завершён", "Session complete").to_string() } else { sub },
            tone: Some("green"),
            ttl_ms: CARD_MS,
            actions,
        };
        if cwd.is_empty() {
            self.fx.eff(Effect::Card(spec));
            return vec![];
        }
        // the VS Code button is offered only when `code` is installed
        vec![self.call("agents_tools", json!({}), Pend::Tools(Box::new(spec)))]
    }

    pub fn on_event(&mut self, name: &str, p: &Value) -> Vec<BackendCall> {
        match name {
            "agents:changed" => {
                self.feed(Snapshot::from_json(p));
                vec![]
            }
            "agents:done" => self.on_done(p),
            // the switch works at once, both ways
            "settings:changed" => {
                if self.live_on == self.live_was {
                    return vec![];
                }
                self.live_was = self.live_on;
                vec![self.call("agents_list", json!({}), Pend::List)]
            }
            _ => vec![],
        }
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Dismiss(s) => vec![self.call("agents_dismiss", json!({ "session": s }), Pend::Other)],
            Action::OpenFolder(s) => vec![self.call("agents_open_folder", json!({ "session": s }), Pend::Other)],
            Action::OpenCode(s) => vec![self.call("agents_open_code", json!({ "session": s }), Pend::Other)],
            Action::CopyPath(p) => {
                self.fx.eff(Effect::CopyText(p));
                vec![]
            }
        }
    }

    /// A card button `ToastRun::Call(..)` is run by the host; nothing to route back for the agents card.
    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        match self.pend.remove(&tag) {
            Some(Pend::List) => {
                self.feed(if r.ok { Snapshot::from_json(&r.v) } else { Snapshot::default() });
            }
            Some(Pend::Tools(mut spec)) => {
                if r.ok && r.v["code"].as_bool().unwrap_or(false) {
                    let session = spec.id.trim_start_matches("agent-done:").to_string();
                    spec.actions.push(ToastAction { label: "VS Code".into(), kind: Some("white"), run: ToastRun::Call("agents_open_code", json!({ "session": session })) });
                }
                self.fx.eff(Effect::Card(*spec));
            }
            _ => {}
        }
        vec![]
    }

    pub fn tick(&mut self, dt_ms: u32) {
        self.now += dt_ms as i64;
        if self.repaint.tick(dt_ms) {
            self.repaint.arm(REPAINT_MS);
            if self.shown {
                self.paint_list(); // ages move on
            }
        }
    }

    pub fn view(&self) -> AgentsView {
        let l = self.lang;
        let state_text = |s: &str| match s {
            "waiting" => l.t("ждёт ответа", "waiting"),
            "working" => l.t("работает", "working"),
            "done" => l.t("готово", "done"),
            _ => l.t("нет вестей", "no news"),
        };
        AgentsView {
            empty_text: self.snap.sessions.is_empty().then(|| l.t("Нет сеансов. Подключите хуки: Настройки → Живые события → ИИ-агенты в терминале", "No sessions. Set up the hooks: Settings → Live events → AI agents in the terminal").to_string()),
            rows: self
                .snap
                .sessions
                .iter()
                .enumerate()
                .map(|(i, s)| Row {
                    id: s.id(),
                    session: s.session.clone(),
                    state: s.state.clone(),
                    name: if !s.project.is_empty() { s.project.clone() } else if !tool_name(&s.tool).is_empty() { tool_name(&s.tool).to_string() } else { s.tool.clone() },
                    title: plain_title(&s.title),
                    state_text: format!("{} · {}", state_text(&s.state), dur(self.now - s.ts, l)),
                    index: i,
                    stagger: self.staggered,
                    pop: self.popped.contains(&s.id()),
                })
                .collect(),
            badge: self.snap.waiting,
        }
    }
}

/// `window.PillAgentsHookText()`: the ready-to-paste hook text of the Settings section, from `agents_hook_snippets`.
pub fn hook_text(s: &Value, lang: Lang) -> String {
    if s.is_null() {
        return String::new();
    }
    let g = |k: &str| s[k].as_str().unwrap_or("");
    let mut out = vec![
        lang.t("Статус агентов: хуки пишет ваш собственный конфиг, приложение их не меняет.", "Agent status: the hooks live in your own config, the app never edits it.").to_string(),
        String::new(),
        format!("Claude Code - {}", g("claude_code_where")),
        g("claude_code").to_string(),
        String::new(),
        format!("Codex - {}", g("codex_where")),
        g("codex").to_string(),
        String::new(),
        format!("{}{}", lang.t("Папка статусов: ", "Status folder: "), g("dir")),
        String::new(),
    ];
    out.extend(s["uncertain"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|x| format!("- {x}")));
    out.join("\n")
}

/// Waiting count text for tooltips ("3 sessions need you").
pub fn waiting_label(n: u32, lang: Lang) -> String {
    if lang.is_ru() { format!("{n} {} ждут ответа", plural(n as i64, "сеанс", "сеанса", "сеансов")) } else { format!("{n} session{} waiting", if n == 1 { "" } else { "s" }) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_761_800_000_000;

    fn sess(tool: &str, id: &str, state: &str, cwd: &str, ago_s: i64) -> Value {
        json!({ "tool": tool, "session": id, "state": state, "title": "# Fix **the** `bug`", "cwd": cwd, "project": project_of(cwd), "ts": NOW - ago_s * 1000 })
    }
    fn snap(ss: Vec<Value>) -> Value {
        let c = |st: &str| ss.iter().filter(|s| s["state"] == st).count();
        json!({ "working": c("working"), "waiting": c("waiting"), "sessions": ss })
    }
    fn model() -> AgentsModel {
        let mut m = AgentsModel::new(Lang::En);
        m.set_now(NOW);
        m
    }
    fn live(m: &mut AgentsModel) -> Vec<(String, Option<LiveRow>)> {
        m.fx.take_effects().into_iter().filter_map(|e| if let Effect::SetLive { key, row } = e { Some((key, row)) } else { None }).collect()
    }

    #[test]
    fn plain_title_strips_markdown_marks() {
        assert_eq!(plain_title("# Fix **the** `bug` > now [ok] _x_"), "Fix the bug now ok x");
        assert_eq!(plain_title("  a\n\tb  "), "a b");
        assert_eq!(plain_title("***"), "");
    }
    #[test]
    fn durations() {
        assert_eq!(dur(5_000, Lang::En), "5s");
        assert_eq!(dur(59_999, Lang::Ru), "59 с");
        assert_eq!(dur(60_000, Lang::En), "1m");
        assert_eq!(dur(3_599_000, Lang::Ru), "59 мин");
        assert_eq!(dur(3_600_000 + 5 * 60_000, Lang::En), "1h 5m");
        assert_eq!(dur(2 * 3_600_000, Lang::Ru), "2 ч 0 мин");
        assert_eq!(dur(-5000, Lang::En), "0s", "a clock skew never shows a negative age");
    }
    #[test]
    fn project_is_the_last_folder() {
        assert_eq!(project_of(r"C:\Users\me\code\pill\"), "pill");
        assert_eq!(project_of("/home/me/app"), "app");
        assert_eq!(project_of(""), "");
    }

    #[test]
    fn waiting_beats_working_in_the_live_row() {
        let mut m = model();
        m.on_event("agents:changed", &snap(vec![sess("claude-code", "a", "working", r"C:\p\one", 5), sess("codex", "b", "working", r"C:\p\two", 9)]));
        let l = live(&mut m);
        let row = l[0].1.clone().unwrap();
        assert_eq!((row.priority, row.text.as_str(), row.accent, row.style), (60, "2 agents working", None, "agents-work"));
        m.on_event("agents:changed", &snap(vec![sess("claude-code", "a", "waiting", r"C:\p\one", 5), sess("codex", "b", "working", r"C:\p\two", 9)]));
        let row = live(&mut m)[0].1.clone().unwrap();
        assert_eq!((row.priority, row.text.as_str(), row.accent), (70, "Needs your input · one", Some("amber")));
        m.on_event("agents:changed", &snap(vec![sess("claude-code", "a", "waiting", r"C:\p\one", 5), sess("codex", "b", "waiting", r"C:\p\two", 9)]));
        assert_eq!(live(&mut m)[0].1.clone().unwrap().text, "2 need your input · one +1");
        m.set_lang(Lang::Ru);
        m.on_event("agents:changed", &snap(vec![sess("claude-code", "a", "waiting", r"C:\p\one", 5), sess("codex", "b", "waiting", r"C:\p\one", 9)]));
        assert_eq!(live(&mut m)[0].1.clone().unwrap().text, "Ждут вас: 2 · one", "the same project is not counted twice");
    }
    #[test]
    fn edge_reactions_follow_the_state_and_clear() {
        let mut m = model();
        let react = |m: &mut AgentsModel| m.fx.take_effects().into_iter().filter(|e| matches!(e, Effect::React { .. } | Effect::Unreact(_))).collect::<Vec<_>>();
        m.on_event("agents:changed", &snap(vec![sess("codex", "a", "working", "", 1)]));
        assert_eq!(react(&mut m), vec![Effect::React { key: "agents".into(), kind: "thinking", ttl_ms: None }]);
        m.on_event("agents:changed", &snap(vec![sess("codex", "a", "working", "", 2)]));
        assert!(react(&mut m).is_empty(), "no re-trigger while the state is unchanged");
        m.on_event("agents:changed", &snap(vec![sess("codex", "a", "waiting", "", 2)]));
        assert_eq!(react(&mut m), vec![Effect::React { key: "agents".into(), kind: "attention", ttl_ms: None }]);
        m.on_event("agents:changed", &snap(vec![sess("codex", "a", "done", "", 2)]));
        assert_eq!(react(&mut m), vec![Effect::Unreact("agents".into())]);
    }
    #[test]
    fn switching_the_setting_off_clears_everything_and_refetches_when_it_flips() {
        let mut m = model();
        m.on_event("agents:changed", &snap(vec![sess("codex", "a", "waiting", "", 1)]));
        m.fx.take_effects();
        m.set_live_on(false);
        let calls = m.on_event("settings:changed", &Value::Null);
        assert_eq!(calls[0].cmd, "agents_list");
        let r = Reply::ok(snap(vec![sess("codex", "a", "waiting", "", 1)]));
        m.on_reply(calls[0].tag, r);
        let fx = m.fx.take_effects();
        assert!(fx.contains(&Effect::SetLive { key: "agents".into(), row: None }) && fx.contains(&Effect::Unreact("agents".into())));
        assert!(m.on_event("settings:changed", &Value::Null).is_empty(), "unchanged switch: nothing to do");
        // done events are ignored while off
        assert!(m.on_event("agents:done", &json!({ "session": "a" })).is_empty());
        assert!(m.fx.take_effects().is_empty());
    }

    #[test]
    fn done_flash_card_and_vscode_button() {
        let mut m = model();
        let calls = m.on_event("agents:done", &json!({ "session": "s1", "tool": "claude-code", "title": "## Refactor `core`", "cwd": r"C:\Users\me\pill" }));
        assert_eq!(calls[0].cmd, "agents_tools");
        let fx = m.fx.take_effects();
        assert_eq!(fx[0], Effect::EdgePulse(0.9));
        assert_eq!(fx[1], Effect::React { key: "agent-done".into(), kind: "done", ttl_ms: Some(4000) });
        let Effect::SetLive { key, row: Some(r) } = &fx[2] else { panic!("{fx:?}") };
        assert_eq!((key.as_str(), r.priority, r.text.as_str(), r.ttl_ms, r.accent), ("agent-done", 75, "Done · pill", Some(1600), Some("green")));
        assert_eq!(fx.len(), 3, "the card waits for agents_tools");
        m.on_reply(calls[0].tag, Reply::ok(json!({ "code": true })));
        let Effect::Card(c) = m.fx.take_effects().remove(0) else { panic!() };
        assert_eq!((c.id.as_str(), c.title.as_str(), c.sub.as_str(), c.tone, c.ttl_ms), ("agent-done:s1", "Agent finished · pill", "Refactor core · Claude Code", Some("green"), 9000));
        let labels: Vec<&str> = c.actions.iter().map(|a| a.label.as_str()).collect();
        assert_eq!(labels, vec!["Dismiss", "Path", "Folder", "VS Code"]);
        assert_eq!(c.actions[0].run, ToastRun::Call("agents_dismiss", json!({ "session": "s1" })));
        assert_eq!(c.actions[1].run, ToastRun::CopyText(r"C:\Users\me\pill".into()));
        assert_eq!(c.actions[3].run, ToastRun::Call("agents_open_code", json!({ "session": "s1" })));
        // no `code` on the machine: no VS Code button
        let calls = m.on_event("agents:done", &json!({ "session": "s2", "tool": "codex", "title": "", "cwd": "/w/app" }));
        m.fx.take_effects();
        m.on_reply(calls[0].tag, Reply::ok(json!({ "code": false })));
        let Effect::Card(c) = m.fx.take_effects().remove(0) else { panic!() };
        assert_eq!(c.sub, "Codex");
        assert_eq!(c.actions.len(), 3);
    }
    #[test]
    fn done_without_folder_and_in_quiet_hours() {
        let mut m = model();
        m.set_quiet(true);
        m.set_collapsed(false);
        let calls = m.on_event("agents:done", &json!({ "session": "s", "tool": "other", "title": "", "cwd": "" }));
        assert!(calls.is_empty());
        let fx = m.fx.take_effects();
        assert!(!fx.iter().any(|e| matches!(e, Effect::EdgePulse(_))), "quiet hours: no ripple");
        assert!(!fx.iter().any(|e| matches!(e, Effect::SetLive { .. })), "expanded pill: no flash row");
        let Some(Effect::Card(c)) = fx.last() else { panic!() };
        assert_eq!((c.title.as_str(), c.sub.as_str(), c.actions.len()), ("Agent finished", "Session complete", 1));
        assert!(m.on_event("agents:done", &json!({ "title": "x" })).is_empty(), "no session id: ignored");
        let mut m = model();
        m.set_lang(Lang::Ru);
        m.on_event("agents:done", &json!({ "session": "s", "cwd": "" }));
        assert!(m.fx.take_effects().iter().any(|e| matches!(e, Effect::SetLive { row: Some(r), .. } if r.text == "Готово")));
    }

    #[test]
    fn tab_rows_states_ages_and_badge() {
        let mut m = model();
        m.on_event("agents:changed", &snap(vec![sess("claude-code", "a", "waiting", r"C:\p\alpha", 30), sess("codex", "b", "working", "", 3700), sess("x", "c", "weird", "", 1)]));
        let v = m.view();
        assert_eq!(v.badge, 1);
        assert_eq!(v.rows[0].name, "alpha");
        assert_eq!(v.rows[0].title, "Fix the bug");
        assert_eq!(v.rows[0].state_text, "waiting · 30s");
        assert_eq!(v.rows[1].name, "Codex");
        assert_eq!(v.rows[1].state_text, "working · 1h 1m");
        assert_eq!((v.rows[2].name.as_str(), v.rows[2].state_text.as_str()), ("x", "no news · 1s"));
        assert_eq!(v.rows[0].id, "claude-code/a");
        assert!(v.empty_text.is_none());
        m.set_lang(Lang::Ru);
        assert_eq!(m.view().rows[0].state_text, "ждёт ответа · 30 с");
        assert!(m.fx.take_effects().contains(&Effect::SetBadge { tab: "agents", count: 1 }));
        let mut m = model();
        m.on_event("agents:changed", &snap(vec![]));
        assert!(m.view().empty_text.unwrap().starts_with("No sessions. Set up the hooks"));
        assert!(m.fx.take_effects().contains(&Effect::SetBadge { tab: "agents", count: 0 }));
    }
    #[test]
    fn rows_stagger_on_open_flip_after_and_pop_changed_dots() {
        let mut m = model();
        m.on_event("agents:changed", &snap(vec![sess("codex", "a", "working", "", 1), sess("codex", "b", "waiting", "", 1)]));
        m.on_show();
        let a = m.fx.take_anims();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].kind, Anim::Stagger);
        assert!(m.view().rows.iter().all(|r| r.stagger));
        m.on_event("agents:changed", &snap(vec![sess("codex", "b", "waiting", "", 1), sess("codex", "a", "waiting", "", 1)]));
        let a = m.fx.take_anims();
        assert!(a.iter().any(|c| c.kind == Anim::Flip));
        assert!(a.iter().any(|c| c.target == "dot:codex/a" && matches!(c.kind, Anim::Pop { .. })), "working -> waiting pops the dot");
        assert!(!a.iter().any(|c| c.target == "dot:codex/b"), "unchanged state: no pop");
        assert!(m.view().rows.iter().all(|r| !r.stagger));
        assert!(m.view().rows.iter().any(|r| r.pop));
        m.fx.reduce_motion = true;
        m.on_event("agents:changed", &snap(vec![sess("codex", "b", "working", "", 1)]));
        assert!(m.fx.take_anims().is_empty(), "Reduce motion: no cues");
    }
    #[test]
    fn ages_repaint_every_15_seconds_only_while_the_tab_is_open() {
        let mut m = model();
        m.on_event("agents:changed", &snap(vec![sess("codex", "a", "working", "", 10)]));
        m.on_show();
        m.fx.take_effects();
        m.tick(14_999);
        assert!(m.fx.take_effects().is_empty());
        m.tick(1);
        assert!(m.fx.take_effects().contains(&Effect::SetBadge { tab: "agents", count: 0 }));
        assert_eq!(m.view().rows[0].state_text, "working · 25s", "the age moved on with the clock");
        m.on_hide();
        m.tick(60_000);
        assert!(m.fx.take_effects().is_empty());
    }
    #[test]
    fn mount_asks_once_and_a_failed_list_is_empty() {
        let mut m = model();
        let c = m.mount();
        assert_eq!(c[0].cmd, "agents_list");
        m.on_reply(c[0].tag, Reply::err("x"));
        assert!(m.view().rows.is_empty());
        let c = m.mount();
        m.on_reply(c[0].tag, Reply::ok(snap(vec![sess("codex", "a", "done", "", 1)])));
        assert_eq!(m.view().rows.len(), 1);
    }
    #[test]
    fn actions_are_backend_calls_by_session_id() {
        let mut m = model();
        assert_eq!(m.on_action(Action::Dismiss("a".into()))[0].args, json!({ "session": "a" }));
        assert_eq!(m.on_action(Action::OpenFolder("a".into()))[0].cmd, "agents_open_folder");
        assert_eq!(m.on_action(Action::OpenCode("a".into()))[0].cmd, "agents_open_code");
        assert!(m.on_action(Action::CopyPath("C:\\x".into())).is_empty());
        assert_eq!(m.fx.take_effects(), vec![Effect::CopyText("C:\\x".into())]);
        assert_eq!((TAB_ID, OPEN_ARG, TAB_ORDER), ("agents", "agents", 66));
    }
    #[test]
    fn hook_text_matches_the_js_layout() {
        let s = json!({ "claude_code_where": "~/.claude/settings.json", "claude_code": "{hooks}", "codex_where": "~/.codex/config.toml", "codex": "[notify]", "dir": "C:\\x", "uncertain": ["a", "b"] });
        assert_eq!(
            hook_text(&s, Lang::En),
            "Agent status: the hooks live in your own config, the app never edits it.\n\nClaude Code - ~/.claude/settings.json\n{hooks}\n\nCodex - ~/.codex/config.toml\n[notify]\n\nStatus folder: C:\\x\n\n- a\n- b"
        );
        assert_eq!(hook_text(&Value::Null, Lang::En), "");
        assert!(hook_text(&s, Lang::Ru).starts_with("Статус агентов:"));
        assert_eq!(waiting_label(3, Lang::Ru), "3 сеанса ждут ответа");
    }
}
