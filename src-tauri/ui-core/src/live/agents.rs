//! Coding-agent status in the collapsed pill (port of agents-live.js; Rust side agents.rs).
//!
//! * `agents:changed {sessions, working, waiting}` -> live row `agents`: waiting (amber dot, priority 70, outline amber, edge `attention`)
//!   beats working (spinner, priority 60, edge `thinking`).
//! * `agents:done {session, tool, title, cwd}` -> green check flash (row `agent-done`, priority 75, 1.6 s) + liquid pulse + a card
//!   with Dismiss / Path / Folder / VS Code (the last only when `code` is installed: [`AgentsIo::code_installed`]).
//! * The read-only sessions tab: [`AgentsLive::list_view`] (waiting first, the order comes from agents.rs) and the tab badge.
//!
//! PRIVACY: titles come from files written by the user's own hooks; they are only ever text.
//! Quiet hours: sources `agents` and `agent-done` are muted (see `board::QUIET_SRC`).

use std::any::Any;
use std::collections::HashMap;

use serde_json::{json, Value};

use super::env::Env;
use super::events::edge;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const FLASH_MS: u32 = 1600;
pub const CARD_MS: u32 = 9000;
/// The sessions tab repaints this often while it is shown (the "7 min" ages).
pub const LIST_REFRESH_MS: u32 = 15_000;

/// The one question the done card asks the backend (`agents_tools`).
pub trait AgentsIo {
    /// `tools.code`: the VS Code button is offered only when `code` is installed.
    fn code_installed(&mut self) -> bool;
}

/// No editor known: the card has no VS Code button.
#[derive(Debug, Default)]
pub struct NoAgentTools;

impl AgentsIo for NoAgentTools {
    fn code_installed(&mut self) -> bool {
        false
    }
}

fn tool_name(tool: &str) -> &'static str {
    match tool {
        "claude-code" => "Claude Code",
        "codex" => "Codex",
        _ => "",
    }
}

fn state_label(env: &dyn Env, state: &str) -> String {
    match state {
        "waiting" => env.tr("ждёт ответа", "waiting"),
        "working" => env.tr("работает", "working"),
        "done" => env.tr("готово", "done"),
        _ => env.tr("нет вестей", "no news"),
    }
}

/// One line of the sessions tab.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentRowView {
    /// `tool/session`
    pub id: String,
    /// project (cwd folder), else the tool's name, else the raw tool id
    pub name: String,
    pub title: String,
    /// waiting | working | done | stale
    pub state: String,
    /// `ждёт ответа · 7 мин`
    pub state_text: String,
    /// The state changed since the last paint: pop the dot (scale .3 -> 1 over .55 s).
    pub pop_dot: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ListView {
    /// Shown instead of the rows when there are no sessions.
    pub empty: Option<String>,
    pub rows: Vec<AgentRowView>,
    /// The tab was just opened: the rows come in one after another (stagger by index) instead of flipping.
    pub fresh: bool,
}

pub struct AgentsLive {
    io: Box<dyn AgentsIo>,
    snap: Value,
    /// liveOn('agents') at the last settings change
    was_on: Option<bool>,
    seen: HashMap<String, String>,
}

impl AgentsLive {
    pub fn new(io: Box<dyn AgentsIo>) -> AgentsLive {
        AgentsLive { io, snap: json!({}), was_on: None, seen: HashMap::new() }
    }

    fn count(&self, k: &str) -> i32 {
        int32(self.snap.get(k)).max(0)
    }

    fn sessions(&self) -> Vec<&Value> {
        self.snap.get("sessions").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default()
    }

    fn render(&self, env: &dyn Env) -> Vec<LiveCmd> {
        // Settings > Live events > AI agents
        if !env.live_on("agents") {
            return vec![LiveCmd::clear("agents"), LiveCmd::unreact("agents")];
        }
        let (waiting, working) = (self.count("waiting"), self.count("working"));
        if waiting > 0 {
            // which session: the project (cwd folder) of the waiting ones, newest first; "+N" when they belong to different projects
            let mut projects: Vec<String> = Vec::new();
            for s in self.sessions() {
                if s.get("state").and_then(Value::as_str) == Some("waiting") && truthy(s.get("project")) {
                    let p = text(s.get("project"));
                    if !projects.contains(&p) {
                        projects.push(p);
                    }
                }
            }
            let which = match projects.first() {
                Some(first) => format!(" · {first}{}", if projects.len() > 1 { format!(" +{}", projects.len() - 1) } else { String::new() }),
                None => String::new(),
            };
            let head = if waiting == 1 { env.tr("Нужен ваш ответ", "Needs your input") } else { env.tr(&format!("Ждут вас: {waiting}"), &format!("{waiting} need your input")) };
            let row = RowSpec::new("agents", 70)
                .accent(Accent::AMBER)
                .anim(Anim::Pop, Anim::None, Anim::Fade)
                .with(Elem::Dot { color: palette::AMBER, size: 10.0, halo: true, pulse_ms: Some(1300) })
                .with(Elem::text(format!("{head}{which}"), TextStyle::StatusAmber));
            vec![LiveCmd::set(row), LiveCmd::react("agents", Preset::Attention, None, None)] // edge: stays while any agent waits for input
        } else if working > 0 {
            let txt = if working == 1 { env.tr("Агент работает", "Agent working") } else { env.tr(&format!("Агентов в работе: {working}"), &format!("{working} agents working")) };
            let row = RowSpec::new("agents", 60)
                .anim(Anim::Pop, Anim::None, Anim::Fade)
                .with(Elem::Spinner { size: 13.0 })
                .with(Elem::text(txt, TextStyle::Status));
            vec![LiveCmd::set(row), LiveCmd::react("agents", Preset::Thinking, None, None)] // edge: stays while any agent works
        } else {
            vec![LiveCmd::clear("agents"), LiveCmd::unreact("agents")]
        }
    }

    /// `feed(snap)`: remember the snapshot, redraw the row and the tab badge.
    pub fn feed(&mut self, env: &dyn Env, snap: &Value) -> Vec<LiveCmd> {
        self.snap = if truthy(Some(snap)) { snap.clone() } else { json!({}) };
        let mut out = self.render(env);
        out.push(LiveCmd::TabBadge { tab: "agents".into(), count: self.count("waiting") as u32 });
        out
    }

    fn done(&mut self, env: &dyn Env, p: &Value) -> Vec<LiveCmd> {
        if p.is_null() || !truthy(p.get("session")) || !env.live_on("agents") {
            return Vec::new();
        }
        let session = text(p.get("session"));
        let cwd = text_or_empty(p.get("cwd"));
        let mut out = edge(env, 0.9);
        out.push(LiveCmd::react("agent-done", Preset::Done, Some(4000), None));
        let proj = last_folder(&cwd); // which session: the folder it works in
        if env.collapsed() {
            let label = format!("{}{}", env.tr("Готово", "Done"), if proj.is_empty() { String::new() } else { format!(" · {proj}") });
            out.push(LiveCmd::set(
                RowSpec::new("agent-done", 75)
                    .ttl(FLASH_MS)
                    .accent(Accent::GREEN)
                    .anim(Anim::Pop, Anim::None, Anim::Fade)
                    .with(Elem::Check { size: 18.0, color: ColorRole::Green })
                    .with(Elem::text(label, TextStyle::Status)),
            ));
        }
        let tool = tool_name(&text_or_empty(p.get("tool")));
        let mut actions = vec![CardAction::new(env.tr("Скрыть", "Dismiss"), ActionKind::Normal, Action::invoke("agents_dismiss", json!({ "session": session })))];
        if !cwd.is_empty() {
            actions.push(CardAction::new(env.tr("Путь", "Path"), ActionKind::White, Action::CopyText(cwd.clone())));
            // the backend looks the folder up by session id
            actions.push(CardAction::new(env.tr("Папка", "Folder"), ActionKind::White, Action::invoke("agents_open_folder", json!({ "session": session }))));
            if self.io.code_installed() {
                actions.push(CardAction::new("VS Code", ActionKind::White, Action::invoke("agents_open_code", json!({ "session": session }))));
            }
        }
        let sub = join_nonempty(&[plain_title(&text_or_empty(p.get("title"))), tool.to_string()], " · ");
        let card = CardSpec::text(
            format!("agent-done:{session}"),
            CARD_MS,
            format!("{}{}", env.tr("Агент закончил", "Agent finished"), if proj.is_empty() { String::new() } else { format!(" · {proj}") }),
            Some(if sub.is_empty() { env.tr("Сеанс завершён", "Session complete") } else { sub }),
        )
        .icon(IconName::Check)
        .tone(Tone::Green)
        .actions(actions);
        let mut card = card;
        card.icon = Some(CardIcon::Check);
        out.push(LiveCmd::Toast(card));
        out
    }

    /// The sessions tab (`paintList`). `fresh` = the tab was just opened. Call again every [`LIST_REFRESH_MS`] while it is shown.
    pub fn list_view(&mut self, env: &dyn Env, now: i64, fresh: bool) -> ListView {
        let ss = self.sessions();
        if ss.is_empty() {
            return ListView {
                empty: Some(env.tr(
                    "Нет сеансов. Подключите хуки: Настройки → Живые события → ИИ-агенты в терминале",
                    "No sessions. Set up the hooks: Settings → Live events → AI agents in the terminal",
                )),
                rows: Vec::new(),
                fresh,
            };
        }
        let mut rows = Vec::new();
        let mut next_seen = HashMap::new();
        for x in &ss {
            let tool = text(x.get("tool"));
            let id = format!("{tool}/{}", text(x.get("session")));
            let state = text(x.get("state"));
            let project = text_or_empty(x.get("project"));
            let name = if !project.is_empty() {
                project
            } else if !tool_name(&tool).is_empty() {
                tool_name(&tool).to_string()
            } else {
                tool.clone()
            };
            let age = now.saturating_sub(num_or0(x.get("ts")).trunc() as i64);
            let pop_dot = self.seen.get(&id).is_some_and(|was| *was != state);
            next_seen.insert(id.clone(), state.clone());
            rows.push(AgentRowView {
                id,
                name,
                title: plain_title(&text_or_empty(x.get("title"))),
                state_text: format!("{} · {}", state_label(env, &state), dur_short(age, env.is_ru())),
                state,
                pop_dot,
            });
        }
        self.seen = next_seen;
        ListView { empty: None, rows, fresh }
    }

    /// The tab's registration: title, order, icon.
    pub fn tab_title(env: &dyn Env) -> String {
        env.tr("Агенты", "Agents")
    }
    pub const TAB_ORDER: i32 = 66;
}

/// `window.PillAgentsHookText`: the ready-to-paste hook text for a Settings section, built from `agents_hook_snippets`.
pub fn agents_hook_text(env: &dyn Env, s: &Value) -> String {
    if s.is_null() {
        return String::new();
    }
    let mut lines: Vec<String> = vec![
        env.tr("Статус агентов: хуки пишет ваш собственный конфиг, приложение их не меняет.", "Agent status: the hooks live in your own config, the app never edits it."),
        String::new(),
        format!("Claude Code - {}", text(s.get("claude_code_where"))),
        text(s.get("claude_code")),
        String::new(),
        format!("Codex - {}", text(s.get("codex_where"))),
        text(s.get("codex")),
        String::new(),
        format!("{}{}", env.tr("Папка статусов: ", "Status folder: "), text(s.get("dir"))),
        String::new(),
    ];
    if let Some(u) = s.get("uncertain").and_then(Value::as_array) {
        lines.extend(u.iter().map(|x| format!("- {}", display(Some(x)))));
    }
    lines.join("\n")
}

impl Provider for AgentsLive {
    fn name(&self) -> &'static str {
        "agents"
    }
    fn events(&self) -> &'static [&'static str] {
        &["agents:changed", "agents:done", "agents:refetch", "settings:changed"]
    }
    fn boot(&self) -> Vec<Query> {
        // events sent before this UI existed are gone: ask once
        vec![Query::new("agents_list", "agents:changed")]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        match name {
            "agents:changed" | "agents:refetch" => self.feed(env, p),
            "agents:done" => self.done(env, p),
            "settings:changed" => {
                // the switch works at once, both ways
                let now_on = env.live_on("agents");
                let was = self.was_on.replace(now_on);
                match was {
                    // the first settings event only records the baseline
                    None => Vec::new(),
                    Some(w) if w == now_on => Vec::new(),
                    Some(_) => vec![LiveCmd::Query(Query::new("agents_list", "agents:refetch"))],
                }
            }
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::{MemEnv, PillState};

    const T0: i64 = 1_700_000_000_000;

    struct Tools(bool);
    impl AgentsIo for Tools {
        fn code_installed(&mut self) -> bool {
            self.0
        }
    }
    fn agents(code: bool) -> AgentsLive {
        AgentsLive::new(Box::new(Tools(code)))
    }
    fn sess(tool: &str, session: &str, state: &str, project: &str, title: &str, ts: i64) -> Value {
        json!({"tool": tool, "session": session, "state": state, "title": title, "cwd": format!("C:\\dev\\{project}"), "project": project, "ts": ts})
    }
    fn snap(sessions: Vec<Value>) -> Value {
        let c = |s: &str| sessions.iter().filter(|x| x["state"] == s).count();
        json!({"working": c("working"), "waiting": c("waiting"), "sessions": sessions})
    }
    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no row in {cmds:?}"))
    }
    fn toast(cmds: &[LiveCmd]) -> &CardSpec {
        cmds.iter().find_map(|c| if let LiveCmd::Toast(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no toast in {cmds:?}"))
    }
    fn label(r: &RowSpec) -> String {
        match r.elements.last() {
            Some(Elem::Text { text, .. }) => text.clone(),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn waiting_row_one_project() {
        let mut a = agents(false);
        let env = MemEnv::default();
        let cmds = a.feed(&env, &snap(vec![sess("claude-code", "s1", "waiting", "island-replica", "t", T0)]));
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms), ("agents", 70, None));
        assert_eq!(r.accent, Some(Accent::AMBER));
        assert_eq!(label(r), "Нужен ваш ответ · island-replica");
        assert_eq!(r.elements[0], Elem::Dot { color: palette::AMBER, size: 10.0, halo: true, pulse_ms: Some(1300) });
        assert!(cmds.contains(&LiveCmd::react("agents", Preset::Attention, None, None)));
        assert_eq!(cmds.last(), Some(&LiveCmd::TabBadge { tab: "agents".into(), count: 1 }));
    }

    #[test]
    fn waiting_row_several_projects_and_en() {
        let mut a = agents(false);
        let en = MemEnv::en();
        let cmds = a.feed(
            &en,
            &snap(vec![
                sess("codex", "a", "waiting", "alpha", "", T0),
                sess("codex", "b", "waiting", "beta", "", T0),
                sess("codex", "c", "waiting", "alpha", "", T0),
                sess("codex", "d", "working", "gamma", "", T0),
            ]),
        );
        assert_eq!(label(row(&cmds)), "3 need your input · alpha +1");
        let ru = MemEnv::default();
        let cmds = a.feed(&ru, &a.snap.clone());
        assert_eq!(label(row(&cmds)), "Ждут вас: 3 · alpha +1");
    }

    #[test]
    fn waiting_without_projects() {
        let mut a = agents(false);
        let env = MemEnv::default();
        let cmds = a.feed(&env, &json!({"waiting": 2, "working": 0, "sessions": [{"state": "waiting", "project": ""}]}));
        assert_eq!(label(row(&cmds)), "Ждут вас: 2");
    }

    #[test]
    fn working_row() {
        let mut a = agents(false);
        let env = MemEnv::default();
        let cmds = a.feed(&env, &snap(vec![sess("codex", "a", "working", "p", "", T0)]));
        let r = row(&cmds);
        assert_eq!((r.priority, r.accent), (60, None));
        assert_eq!(r.elements[0], Elem::Spinner { size: 13.0 });
        assert_eq!(label(r), "Агент работает");
        assert!(cmds.contains(&LiveCmd::react("agents", Preset::Thinking, None, None)));
        let cmds = a.feed(&MemEnv::en(), &snap(vec![sess("codex", "a", "working", "p", "", T0), sess("codex", "b", "working", "p", "", T0)]));
        assert_eq!(label(row(&cmds)), "2 agents working");
        let cmds = a.feed(&env, &snap(vec![sess("codex", "a", "working", "p", "", T0), sess("codex", "b", "working", "p", "", T0), sess("codex", "c", "working", "p", "", T0)]));
        assert_eq!(label(row(&cmds)), "Агентов в работе: 3");
        let cmds = a.feed(&MemEnv::en(), &snap(vec![sess("codex", "a", "working", "p", "", T0)]));
        assert_eq!(label(row(&cmds)), "Agent working");
    }

    #[test]
    fn waiting_beats_working_and_idle_clears() {
        let mut a = agents(false);
        let env = MemEnv::default();
        let cmds = a.feed(&env, &snap(vec![sess("codex", "a", "working", "p", "", T0), sess("codex", "b", "waiting", "q", "", T0)]));
        assert_eq!(row(&cmds).priority, 70);
        let cmds = a.feed(&env, &snap(vec![sess("codex", "a", "done", "p", "", T0)]));
        assert_eq!(&cmds[..2], &[LiveCmd::clear("agents"), LiveCmd::unreact("agents")]);
        assert_eq!(cmds[2], LiveCmd::TabBadge { tab: "agents".into(), count: 0 });
        // a null / empty snapshot is an empty list
        assert_eq!(a.feed(&env, &Value::Null)[0], LiveCmd::clear("agents"));
        assert_eq!(a.feed(&env, &json!({"waiting": -4, "working": "x"}))[0], LiveCmd::clear("agents"));
    }

    #[test]
    fn switch_off_clears_everything() {
        let mut a = agents(false);
        let mut env = MemEnv::default();
        env.set("live.agents", json!(false));
        let cmds = a.feed(&env, &snap(vec![sess("codex", "a", "waiting", "p", "", T0)]));
        assert_eq!(&cmds[..2], &[LiveCmd::clear("agents"), LiveCmd::unreact("agents")]);
        assert!(a.done(&env, &json!({"session": "s"})).is_empty());
    }

    #[test]
    fn settings_toggle_refetches_only_on_a_change() {
        let mut a = agents(false);
        let mut env = MemEnv::default();
        assert!(a.on_event(&env, "settings:changed", &Value::Null, T0).is_empty(), "baseline");
        assert!(a.on_event(&env, "settings:changed", &Value::Null, T0).is_empty());
        env.set("live.agents", json!(false));
        assert_eq!(a.on_event(&env, "settings:changed", &Value::Null, T0), vec![LiveCmd::Query(Query::new("agents_list", "agents:refetch"))]);
        assert!(a.on_event(&env, "settings:changed", &Value::Null, T0).is_empty());
        env.unset("live.agents");
        assert_eq!(a.on_event(&env, "settings:changed", &Value::Null, T0).len(), 1);
        // the answer (even a failed one) redraws
        let cmds = a.on_event(&env, "agents:refetch", &Value::Null, T0);
        assert_eq!(cmds[0], LiveCmd::clear("agents"));
        assert_eq!(a.boot(), vec![Query::new("agents_list", "agents:changed")]);
    }

    #[test]
    fn done_flash_and_card() {
        let mut a = agents(true);
        let env = MemEnv::default();
        let cmds = a.done(&env, &json!({"session": "abc", "tool": "claude-code", "title": "## Fix the **login** bug", "cwd": "C:\\dev\\island-replica"}));
        assert_eq!(cmds[0], LiveCmd::EdgePulse(0.9));
        assert_eq!(cmds[1], LiveCmd::react("agent-done", Preset::Done, Some(4000), None));
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms, r.accent), ("agent-done", 75, Some(1600), Some(Accent::GREEN)));
        assert_eq!(r.elements[0], Elem::Check { size: 18.0, color: ColorRole::Green });
        assert_eq!(label(r), "Готово · island-replica");
        let t = toast(&cmds);
        assert_eq!((t.id.as_str(), t.ttl_ms, t.tone, t.icon.clone()), ("agent-done:abc", 9000, Tone::Green, Some(CardIcon::Check)));
        assert_eq!(
            t.body,
            CardBody::Text { title: "Агент закончил · island-replica".into(), sub: Some("Fix the login bug · Claude Code".into()) }
        );
        let acts: Vec<(&str, ActionKind)> = t.actions.iter().map(|x| (x.label.as_str(), x.kind)).collect();
        assert_eq!(acts, [("Скрыть", ActionKind::Normal), ("Путь", ActionKind::White), ("Папка", ActionKind::White), ("VS Code", ActionKind::White)]);
        assert_eq!(t.actions[0].action, Action::invoke("agents_dismiss", json!({"session": "abc"})));
        assert_eq!(t.actions[1].action, Action::CopyText("C:\\dev\\island-replica".into()));
        assert_eq!(t.actions[2].action, Action::invoke("agents_open_folder", json!({"session": "abc"})));
        assert_eq!(t.actions[3].action, Action::invoke("agents_open_code", json!({"session": "abc"})));
    }

    #[test]
    fn done_without_cwd_title_or_code() {
        let mut a = agents(false);
        let env = MemEnv::en();
        let cmds = a.done(&env, &json!({"session": "x", "tool": "mystery", "title": "", "cwd": ""}));
        let t = toast(&cmds);
        assert_eq!(t.body, CardBody::Text { title: "Agent finished".into(), sub: Some("Session complete".into()) });
        assert_eq!(t.actions.len(), 1);
        assert_eq!(t.actions[0].label, "Dismiss");
        assert_eq!(label(row(&cmds)), "Done");
        // cwd but no VS Code
        let cmds = a.done(&env, &json!({"session": "x", "tool": "codex", "title": "t", "cwd": "/home/me/app/"}));
        let t = toast(&cmds);
        assert_eq!(t.actions.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), ["Dismiss", "Path", "Folder"]);
        assert_eq!(t.body, CardBody::Text { title: "Agent finished · app".into(), sub: Some("t · Codex".into()) });
        // a title that is only markdown marks falls back to the tool
        let cmds = a.done(&env, &json!({"session": "x", "tool": "codex", "title": "###", "cwd": ""}));
        assert_eq!(toast(&cmds).body, CardBody::Text { title: "Agent finished".into(), sub: Some("Codex".into()) });
    }

    #[test]
    fn done_expanded_has_no_flash_but_still_pulses() {
        let mut a = agents(false);
        let mut env = MemEnv::default();
        env.state = PillState::Expanded;
        let cmds = a.done(&env, &json!({"session": "x"}));
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::SetRow(_))));
        assert!(cmds.contains(&LiveCmd::EdgePulse(0.9)));
        assert!(cmds.iter().any(|c| matches!(c, LiveCmd::Toast(_))));
    }

    #[test]
    fn done_gates_and_quiet() {
        let mut a = agents(false);
        let mut env = MemEnv::default();
        assert!(a.done(&env, &Value::Null).is_empty());
        assert!(a.done(&env, &json!({"tool": "codex"})).is_empty(), "no session id");
        env.quiet = true;
        let cmds = a.done(&env, &json!({"session": "x"}));
        assert!(!cmds.contains(&LiveCmd::EdgePulse(0.9)), "no pulse during quiet hours");
    }

    #[test]
    fn done_unicode_project() {
        let mut a = agents(false);
        let env = MemEnv::default();
        let cmds = a.done(&env, &json!({"session": "x", "cwd": "C:\\Проекты\\остров 🌴"}));
        assert_eq!(label(row(&cmds)), "Готово · остров 🌴");
    }

    #[test]
    fn list_view_rows_and_ages() {
        let mut a = agents(false);
        let env = MemEnv::default();
        a.feed(
            &env,
            &snap(vec![
                sess("claude-code", "s1", "waiting", "island", "**Plan** it", T0 - 5_000),
                json!({"tool": "codex", "session": "s2", "state": "working", "title": "", "cwd": "", "project": "", "ts": T0 - 125 * 60_000}),
                json!({"tool": "other", "session": "s3", "state": "stale", "title": "x", "cwd": "", "project": "", "ts": T0}),
            ]),
        );
        let v = a.list_view(&env, T0, true);
        assert!(v.fresh && v.empty.is_none());
        assert_eq!(v.rows.len(), 3);
        assert_eq!(v.rows[0].id, "claude-code/s1");
        assert_eq!((v.rows[0].name.as_str(), v.rows[0].title.as_str()), ("island", "Plan it"));
        assert_eq!(v.rows[0].state_text, "ждёт ответа · 5 с");
        assert_eq!((v.rows[1].name.as_str(), v.rows[1].state_text.as_str()), ("Codex", "работает · 2 ч 5 мин"));
        assert_eq!((v.rows[2].name.as_str(), v.rows[2].state_text.as_str()), ("other", "нет вестей · 0 с"));
        assert!(v.rows.iter().all(|r| !r.pop_dot), "first paint pops nothing");
        let en = a.list_view(&MemEnv::en(), T0, false);
        assert_eq!(en.rows[0].state_text, "waiting · 5s");
    }

    #[test]
    fn list_view_pops_a_changed_state_once() {
        let mut a = agents(false);
        let env = MemEnv::default();
        a.feed(&env, &snap(vec![sess("codex", "s", "waiting", "p", "", T0)]));
        a.list_view(&env, T0, true);
        a.feed(&env, &snap(vec![sess("codex", "s", "working", "p", "", T0)]));
        assert!(a.list_view(&env, T0, false).rows[0].pop_dot);
        assert!(!a.list_view(&env, T0, false).rows[0].pop_dot, "same state again: no pop");
    }

    #[test]
    fn list_view_empty_hint() {
        let mut a = agents(false);
        let v = a.list_view(&MemEnv::default(), T0, true);
        assert!(v.rows.is_empty());
        assert_eq!(v.empty.as_deref(), Some("Нет сеансов. Подключите хуки: Настройки → Живые события → ИИ-агенты в терминале"));
        assert!(a.list_view(&MemEnv::en(), T0, true).empty.unwrap().starts_with("No sessions."));
    }

    #[test]
    fn hook_text() {
        let env = MemEnv::en();
        let s = json!({"claude_code_where": "~/.claude/settings.json", "claude_code": "{...}", "codex_where": "~/.codex/config.toml", "codex": "notify = []", "dir": "C:\\agents", "uncertain": ["a", "b"]});
        let t = agents_hook_text(&env, &s);
        assert!(t.starts_with("Agent status: the hooks live in your own config, the app never edits it.\n\nClaude Code - ~/.claude/settings.json\n{...}\n\nCodex - ~/.codex/config.toml\nnotify = []\n\nStatus folder: C:\\agents\n\n"), "{t}");
        assert!(t.ends_with("- a\n- b"));
        assert_eq!(agents_hook_text(&env, &Value::Null), "");
        assert!(agents_hook_text(&MemEnv::default(), &json!({})).contains("Папка статусов: "));
    }

    #[test]
    fn tab_registration() {
        assert_eq!(AgentsLive::tab_title(&MemEnv::default()), "Агенты");
        assert_eq!(AgentsLive::tab_title(&MemEnv::en()), "Agents");
        assert_eq!(AgentsLive::TAB_ORDER, 66);
    }
}
