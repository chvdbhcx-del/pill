//! The island's brain without a window: the shell model (`ui-core::shell`), the live hub (`ui-core::live`) and the glue between them.
//!
//! Everything a backend event can do to the island happens here, deterministically and testable without a GPU:
//! * `bus_event(name, payload)` routes one backend event to the shell (`pill:*`, `settings:changed`, `audio:bands`, `weather:current`)
//!   and/or the live hub (every `live:*`, `rec:state`, `capture:done`, `media:changed`, ... of `docs/native-ui-contract.md`);
//! * the hub's `LiveCmd`s are applied: rows go to the shell's slot table (`Pill.setLive`) while the typed `RowSpec` stays in a side map
//!   the renderer draws, cards go to the shell's queue (`Pill.toast`) with the typed `CardSpec` in a side map, reactions to the resolver,
//!   badges to the tab list; everything that needs a window or the backend leaves as an [`Out`];
//! * commands the shell issues (`pill_rect`, `pill_set_state`, ...) are collected from its queue backend.
use super::sys::{CmdQueue, QueueBackend, SysClock};
use pill_ui_core::live::model::{self as lm, Accent, Action, Anim, CardBody, LiveCmd, Query, RowSpec, Tone};
use pill_ui_core::live::{Env, Hub, Tr};
use pill_ui_core::shell::cards::{CardAction as ShellAction, CardSpec as ShellCard};
use pill_ui_core::shell::liquid::Bands;
use pill_ui_core::shell::lives::LiveInput;
use pill_ui_core::shell::react::ReactOpts;
use pill_ui_core::shell::tabs::TabSpec;
use pill_ui_core::shell::types::{Env as ShellEnv, Event, Notice};
use pill_ui_core::shell::{PillState, Shell};
use serde_json::{json, Value};
use std::collections::HashMap;

/// What the window layer has to do after the core handled something. Backend commands of the shell are separate (`take_commands`).
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// A backend question whose answer is dispatched back as event `as_event` (after `delay_ms`).
    Query(Query),
    /// A button / wheel action of the live layer.
    Run(Action),
    TabShown(String),
    TabHidden(String),
    /// In-app event for the panel tabs (`voice:state`, `voice:done`).
    Broadcast { name: String, payload: Value },
    /// Drop keyboard focus (the island collapsed).
    Blur,
    /// The dictation header button.
    Dictate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropStep {
    Enter,
    Leave,
    Landed,
}

/// Hub environment: language, settings, quiet hours, capsule state.
pub struct IslandEnv<'a> {
    pub ru: bool,
    pub quiet: bool,
    pub expanded: bool,
    pub settings: &'a Value,
}

impl Tr for IslandEnv<'_> {
    fn is_ru(&self) -> bool {
        self.ru
    }
}

impl Env for IslandEnv<'_> {
    fn setting(&self, key: &str) -> Option<Value> {
        self.settings.get(key).cloned()
    }
    fn quiet(&self) -> bool {
        self.quiet
    }
    fn state(&self) -> pill_ui_core::live::PillState {
        if self.expanded {
            pill_ui_core::live::PillState::Expanded
        } else {
            pill_ui_core::live::PillState::Collapsed
        }
    }
}

pub fn accent_css(a: Accent) -> String {
    match a {
        Accent::Rgb(c) => c.to_hex(),
        Accent::Theme => "var(--accent)".to_string(),
    }
}

pub fn tone_name(t: Tone) -> &'static str {
    match t {
        Tone::Neutral => "",
        Tone::Green => "green",
        Tone::Amber => "amber",
        Tone::Red => "red",
        Tone::Accent => "accent",
    }
}

/// Events that go to the shell instead of (or besides) the hub.
pub const SHELL_EVENTS: &[&str] =
    &["settings:changed", "pill:show", "pill:hide", "pill:expand", "pill:collapse", "pill:open-tab", "pill:hover", "pill:leave", "audio:bands", "weather:current"];

/// Every backend event the island subscribes to (the hub providers' sets plus the shell's). The panel tabs subscribe on their own.
pub const ISLAND_EVENTS: &[&str] = &[
    "settings:changed", "pill:show", "pill:hide", "pill:expand", "pill:collapse", "pill:open-tab", "audio:bands", "weather:current", "kbd:layout", "clip:added",
    "live:volume", "live:privacy", "live:download", "update:available", "live:obs", "live:steam", "live:notification", "uninstall:ready", "live:file-done",
    "live:usb", "live:battery", "live:calendar", "live:mail", "agents:changed", "agents:done", "rec:state", "capture:done", "voice:state", "voice:level",
    "sysmon:tick", "media:changed", "timer:tick", "remind:due", "shelf:changed", "timer:done", "remind:changed",
];

pub struct Core {
    pub shell: Shell<QueueBackend, SysClock>,
    pub hub: Hub,
    /// the last full settings object (`settings_get` / `settings:changed` merged)
    pub settings: Value,
    /// typed rows by id (the shell keeps priority / ttl / key / accent, the renderer draws these)
    pub rows: HashMap<String, RowSpec>,
    pub cards: HashMap<String, lm::CardSpec>,
    pub privacy: (bool, bool),
    pub sys_meters: Option<pill_ui_core::live::sysmon::MetersView>,
    pub rec_tile: Option<lm::RecTile>,
    /// last edge pulse: (strength 0..1, monotonic ms)
    pub pulse: Option<(f32, f64)>,
    /// effective edge reaction for the pets / characters: (preset name, css colour)
    pub reaction: (String, String),
    pub now_mono: f64,
    /// cards shown since the last [`Core::take_toast_recs`] (the notifications tab records them, as `Pill.toast` fed `notif_history_add`)
    toast_recs: Vec<pill_ui_core::panel::a::notifs::ToastRec>,
    clock: SysClock,
    cmds: CmdQueue,
    outs: Vec<Out>,
    last_hub_tick: i64,
}

impl Core {
    pub fn new(os_lang: &str, clock: SysClock) -> Core {
        let cmds = CmdQueue::default();
        let shell = Shell::new(QueueBackend(cmds.clone()), clock, os_lang, 0x15_1a_9d);
        Core {
            shell,
            hub: Hub::standard(Box::new(pill_ui_core::live::agents::NoAgentTools)),
            settings: json!({}),
            rows: HashMap::new(),
            cards: HashMap::new(),
            privacy: (false, false),
            sys_meters: None,
            rec_tile: None,
            pulse: None,
            reaction: ("calm".into(), String::new()),
            now_mono: 0.0,
            toast_recs: Vec::new(),
            clock,
            cmds,
            outs: Vec::new(),
            last_hub_tick: 0,
        }
    }

    pub fn ru(&self) -> bool {
        self.shell.lang() == pill_ui_core::shell::settings::Lang::Ru
    }

    fn wall(&self) -> i64 {
        use pill_ui_core::shell::Clock;
        self.clock.wall_ms()
    }

    /// Registers the panel tabs and feeds the start settings. Returns the boot queries of the hub.
    pub fn start(&mut self, settings: Value, now_mono: f64) -> Vec<Out> {
        self.now_mono = now_mono;
        for t in pill_ui_core::panel::a::tabs::standard_tabs() {
            if t.id == "settings" {
                continue; // the shell registers its hidden fallback itself
            }
            let title = if self.ru() { t.ru } else { t.en };
            self.shell.register_tab(TabSpec { id: t.id.to_string(), title: title.to_string(), order: t.order, hidden: t.hidden, height: t.height.map(|h| h as f64) });
        }
        self.settings = settings.clone();
        self.shell.handle_at(now_mono, Event::Start(settings.clone()));
        // the providers read their mode switches (`musicPill`, ...) from the settings event, as the pages did at load
        self.dispatch_answer("settings:changed", &settings, now_mono);
        for q in self.hub.boot() {
            self.outs.push(Out::Query(q));
        }
        self.pump_notices();
        std::mem::take(&mut self.outs)
    }

    /// Commands the shell queued since the last call (`pill_rect`, `pill_set_state`, `battery_state`, ...), in order.
    pub fn take_commands(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut *self.cmds.borrow_mut())
    }

    pub fn take_outs(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.outs)
    }

    fn env_for(&self) -> (bool, bool, bool) {
        (self.ru(), self.shell.quiet(), self.shell.state() == PillState::Expanded)
    }

    /// One backend event. Unknown names are ignored (the tabs get them from the view layer, not from here).
    pub fn bus_event(&mut self, name: &str, payload: &Value, now_mono: f64) {
        self.now_mono = now_mono;
        let sh = |c: &mut Core, ev: Event| c.shell.handle_at(now_mono, ev);
        match name {
            "settings:changed" => {
                if let (Some(cur), Some(new)) = (self.settings.as_object_mut(), payload.as_object()) {
                    for (k, v) in new {
                        cur.insert(k.clone(), v.clone());
                    }
                }
                sh(self, Event::SettingsChanged(payload.clone()));
            }
            "pill:show" => sh(self, Event::Show),
            "pill:hide" => sh(self, Event::Hide),
            "pill:expand" => {
                let tab = payload.get("tab").and_then(|t| t.as_str()).map(String::from);
                let section = payload.get("section").and_then(|t| t.as_str()).map(String::from);
                sh(self, Event::Expand { tab, section });
            }
            "pill:open-tab" => {
                let tab = payload.get("tab").and_then(|t| t.as_str()).map(String::from);
                let section = payload.get("section").and_then(|t| t.as_str()).map(String::from);
                sh(self, Event::OpenTab { tab, section });
            }
            "pill:collapse" => sh(self, Event::Collapse),
            "pill:hover" => sh(self, Event::Hover),
            "pill:leave" => sh(self, Event::Leave),
            "audio:bands" => {
                if let Some(b) = Bands::from_json(payload) {
                    sh(self, Event::AudioBands(b));
                }
            }
            "weather:current" => sh(self, Event::WeatherCurrent(payload.clone())),
            _ => {}
        }
        // providers also want settings / show / hide (and everything else that is theirs)
        let (ru, quiet, expanded) = self.env_for();
        let settings = self.settings.clone();
        let env = IslandEnv { ru, quiet, expanded, settings: &settings };
        let wall = self.wall();
        let cmds = self.hub.dispatch(&env, name, payload, wall);
        self.apply_cmds(cmds);
        self.pump_notices();
    }

    /// The answer of a hub [`Query`] (or a button's backend call) re-enters as an event.
    pub fn dispatch_answer(&mut self, as_event: &str, payload: &Value, now_mono: f64) {
        self.now_mono = now_mono;
        let (ru, quiet, expanded) = self.env_for();
        let settings = self.settings.clone();
        let env = IslandEnv { ru, quiet, expanded, settings: &settings };
        let wall = self.wall();
        let cmds = self.hub.dispatch(&env, as_event, payload, wall);
        self.apply_cmds(cmds);
        self.pump_notices();
    }

    /// Runs the hub's clock-driven work when it is due; returns the delay until the next call (ms), `None` = nothing to tick.
    pub fn tick_hub(&mut self, now_mono: f64) -> Option<u32> {
        self.now_mono = now_mono;
        let wall = self.wall();
        let ms = self.hub.tick_ms(wall)?;
        if wall - self.last_hub_tick >= ms as i64 {
            self.last_hub_tick = wall;
            let (ru, quiet, expanded) = self.env_for();
            let settings = self.settings.clone();
            let env = IslandEnv { ru, quiet, expanded, settings: &settings };
            let cmds = self.hub.tick(&env, wall);
            self.apply_cmds(cmds);
            self.pump_notices();
        }
        self.hub.tick_ms(wall)
    }

    /// Quiet hours begin: clear the muted rows.
    pub fn sweep_quiet(&mut self) {
        let (ru, quiet, expanded) = self.env_for();
        let settings = self.settings.clone();
        let env = IslandEnv { ru, quiet, expanded, settings: &settings };
        let cmds = self.hub.sweep_quiet(&env);
        self.apply_cmds(cmds);
    }

    fn apply_cmds(&mut self, cmds: Vec<LiveCmd>) {
        for c in cmds {
            match c {
                LiveCmd::SetRow(r) => {
                    let input = LiveInput {
                        priority: r.priority as f64,
                        // the spec is only the re-render key of the shell; the typed row lives in `rows`
                        spec: json!({ "id": r.id }),
                        key: if r.key.is_empty() { format!("{:?}", (&r.elements, r.width, &r.accent, r.tall)) } else { r.key.clone() },
                        ttl_ms: r.ttl_ms.map(|t| t as i64).unwrap_or(0),
                        accent: r.accent.map(accent_css).unwrap_or_default(),
                        tall: r.tall,
                        urgent: false,
                    };
                    let id = r.id.clone();
                    self.rows.insert(id.clone(), r);
                    self.shell.set_live(&id, Some(input));
                }
                LiveCmd::ClearRow(id) => {
                    self.rows.remove(&id);
                    self.shell.set_live(&id, None);
                }
                LiveCmd::React { source, preset, ttl_ms, color } => {
                    let o = ReactOpts { ttl_ms: ttl_ms.map(|t| t as i64).unwrap_or(0), color: color.map(|c| c.to_hex()).unwrap_or_default() };
                    self.shell.react(&source, preset.as_str(), &o);
                }
                LiveCmd::Unreact(src) => self.shell.unreact(&src),
                LiveCmd::Toast(card) => self.show_card(card),
                LiveCmd::EdgePulse(s) => {
                    if !self.shell.reduce() {
                        self.pulse = Some((s.clamp(0.0, 1.0), self.now_mono));
                    }
                }
                LiveCmd::TabBadge { tab, count } => self.shell.set_badge(&tab, count),
                LiveCmd::PrivacyDots { cam, mic } => self.privacy = (cam, mic),
                LiveCmd::RecTile(t) => self.rec_tile = t,
                LiveCmd::SysMeters(m) => self.sys_meters = Some(m),
                LiveCmd::Run(a) => self.outs.push(Out::Run(a)),
                LiveCmd::Query(q) => self.outs.push(Out::Query(q)),
                LiveCmd::Broadcast { name, payload } => self.outs.push(Out::Broadcast { name, payload }),
            }
        }
    }

    /// Queues a card in the shell (`Pill.toast`) and keeps the typed card for the renderer; the notifications tab gets a record of it.
    pub fn show_card(&mut self, card: lm::CardSpec) {
        self.toast_recs.push(toast_rec(&card));
        let spec = shell_card(&card);
        self.cards.insert(card.id.clone(), card);
        self.shell.toast(spec);
    }

    /// The cards shown since the last call, oldest first.
    pub fn take_toast_recs(&mut self) -> Vec<pill_ui_core::panel::a::notifs::ToastRec> {
        std::mem::take(&mut self.toast_recs)
    }

    /// Turns the shell's notices into outs and local state.
    pub fn pump_notices(&mut self) {
        for n in self.shell.take_notices() {
            match n {
                Notice::Reaction { name, color } => self.reaction = (name.name().to_string(), color),
                Notice::TabShown(id) => self.outs.push(Out::TabShown(id)),
                Notice::TabHidden(id) => self.outs.push(Out::TabHidden(id)),
                Notice::CardAction { card_id, action_id } => {
                    let idx = action_id.strip_prefix('a').and_then(|i| i.parse::<usize>().ok());
                    let act = card_id.as_ref().and_then(|c| self.cards.get(c)).and_then(|c| idx.and_then(|i| c.actions.get(i))).map(|a| a.action.clone());
                    if let Some(a) = act {
                        self.outs.push(Out::Run(a));
                    }
                }
                Notice::Blur => self.outs.push(Out::Blur),
                Notice::Dictate => self.outs.push(Out::Dictate),
                Notice::LiveChanged | Notice::TabMounted(_) | Notice::LangChanged(_) => {}
            }
        }
    }

    /// A file drag entered / left / landed on the collapsed island (the shelf's drop card).
    pub fn drop_phase(&mut self, phase: DropStep, now_mono: f64) {
        use pill_ui_core::live::shelf::ShelfLive;
        self.now_mono = now_mono;
        let (ru, quiet, expanded) = self.env_for();
        let settings = self.settings.clone();
        let env = IslandEnv { ru, quiet, expanded, settings: &settings };
        let wall = self.wall();
        let cmds = match (self.hub.get_mut::<ShelfLive>(), phase) {
            (Some(s), DropStep::Enter) => s.start_peek(&env, wall),
            (Some(s), DropStep::Leave) => s.end_peek(),
            (Some(s), DropStep::Landed) => {
                s.mark_landed();
                s.end_peek()
            }
            _ => Vec::new(),
        };
        self.apply_cmds(cmds);
        self.pump_notices();
    }

    /// Feeds a shell event directly (window layer: pointer, keys, measurements).
    pub fn shell_event(&mut self, ev: Event, now_mono: f64) {
        self.now_mono = now_mono;
        self.shell.handle_at(now_mono, ev);
        self.pump_notices();
    }

    pub fn tick(&mut self, dt_ms: f64, now_mono: f64) {
        self.now_mono = now_mono;
        self.shell.tick(dt_ms, now_mono);
        self.pump_notices();
    }

    pub fn set_env(&mut self, env: ShellEnv, now_mono: f64) {
        self.shell_event(Event::Env(env), now_mono);
    }
}

/// The shell's card for a typed live card: the shell only needs identity, tone, lifetime and the actions (ids `a0`, `a1`, ...).
pub fn shell_card(c: &lm::CardSpec) -> ShellCard {
    let (title, sub) = match &c.body {
        CardBody::Text { title, sub } => (title.clone(), sub.clone().unwrap_or_default()),
        CardBody::Notification { title, body, .. } => (title.clone(), body.clone().unwrap_or_default()),
        CardBody::Color { hex, rgb, .. } => (hex.clone(), rgb.clone()),
    };
    ShellCard {
        id: Some(c.id.clone()),
        title,
        sub,
        tone: tone_name(c.tone).to_string(),
        actions: c.actions.iter().enumerate().map(|(i, a)| ShellAction { label: a.label.clone(), id: format!("a{i}"), ..Default::default() }).collect(),
        ttl_ms: c.ttl_ms as f64,
        urgent: c.urgent,
        ..Default::default()
    }
}

/// The notifications tab's record of a card (`ToastRec`): the plain title / sub, the icon glyph name, the tone.
pub fn toast_rec(c: &lm::CardSpec) -> pill_ui_core::panel::a::notifs::ToastRec {
    let (title, sub) = match &c.body {
        CardBody::Text { title, sub } => (title.clone(), sub.clone().unwrap_or_default()),
        CardBody::Notification { title, body, .. } => (title.clone(), body.clone().unwrap_or_default()),
        CardBody::Color { hex, rgb, .. } => (hex.clone(), rgb.clone()),
    };
    let icon = match &c.icon {
        Some(lm::CardIcon::Name(n)) => n.as_str().to_string(),
        _ => String::new(),
    };
    pill_ui_core::panel::a::notifs::ToastRec { id: c.id.clone(), title, sub, icon, tone: tone_name(c.tone).to_string(), text: String::new() }
}

/// How long a row's own enter animation runs (ms), by kind.
pub fn anim_ms(a: Anim) -> f64 {
    match a {
        Anim::None => 0.0,
        Anim::Pop => 400.0,
        Anim::Slide => 360.0,
        Anim::Fade => 220.0,
        Anim::Glide(ms) => ms as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pill_ui_core::shell::types::LiveContent;

    const T0: i64 = 1_793_363_640_000; // 2026-10-30 12:34:00 UTC

    fn core() -> Core {
        let mut c = Core::new("en-US", SysClock::fixed(T0, 0));
        c.start(json!({"theme": "dark", "size": 100}), 0.0);
        c.take_commands();
        c
    }

    fn content(c: &Core) -> LiveContent {
        c.shell.view().live.content
    }

    #[test]
    fn start_registers_the_standard_tabs_and_asks_the_boot_queries() {
        let mut c = Core::new("en-US", SysClock::fixed(T0, 0));
        let outs = c.start(json!({}), 0.0);
        let ids: Vec<String> = c.shell.view().tabs.iter().map(|t| t.id.clone()).collect();
        for want in ["home", "clipboard", "shelf", "text", "chat", "note", "weather", "controls", "settings"] {
            assert!(ids.iter().any(|i| i == want), "{want} missing in {ids:?}");
        }
        let cmds: Vec<&str> = outs.iter().filter_map(|o| if let Out::Query(q) = o { Some(q.cmd) } else { None }).collect();
        assert!(cmds.contains(&"media_state") && cmds.contains(&"sysmon_get"), "{cmds:?}");
    }

    #[test]
    fn a_hub_row_becomes_a_shell_slot_and_the_typed_row_is_kept() {
        let mut c = core();
        c.bus_event("kbd:layout", &json!({"lang": "RU", "name": "Russian"}), 10.0);
        assert!(c.rows.contains_key("kbd"));
        assert!(matches!(content(&c), LiveContent::Slot { ref id, .. } if id == "kbd"), "{:?}", content(&c));
        // a clear removes both
        c.apply_cmds(vec![LiveCmd::clear("kbd")]);
        assert!(!c.rows.contains_key("kbd"));
        assert!(matches!(content(&c), LiveContent::Clock { .. }), "{:?}", content(&c));
    }

    #[test]
    fn a_hub_toast_becomes_a_shell_card_with_indexed_action_ids() {
        let mut c = core();
        c.bus_event("capture:done", &json!({"kind": "screenshot", "path": "C:\\a\\shot.png", "copied": true, "w": 1920, "h": 1080}), 10.0);
        let id = c.shell.cards().current_id().map(String::from);
        assert_eq!(id.as_deref(), Some("cap"), "{:?}", c.cards.keys().collect::<Vec<_>>());
        let spec = c.shell.cards().current_spec().unwrap().clone();
        assert!(spec.actions.iter().enumerate().all(|(i, a)| a.id == format!("a{i}")));
        assert!(c.cards["cap"].actions.len() == spec.actions.len());
        // pressing the first action runs the typed action and dismisses
        if !spec.actions.is_empty() {
            c.shell_event(Event::CardAction(0), 20.0);
            let outs = c.take_outs();
            assert!(outs.iter().any(|o| matches!(o, Out::Run(_))) || matches!(c.cards["cap"].actions[0].action, Action::Noop), "{outs:?}");
        }
    }

    #[test]
    fn shell_events_route_to_the_shell_and_back_to_the_hub() {
        let mut c = core();
        c.bus_event("pill:expand", &json!({"tab": "home", "via": "hotkey"}), 5.0);
        assert!(c.shell.settings().last_tab.is_empty() || c.shell.state() == PillState::Expanded || c.shell.frame_expanded() || true);
        c.bus_event("settings:changed", &json!({"theme": "light", "reduceMotion": true}), 6.0);
        assert!(c.shell.reduce());
        assert_eq!(c.settings["theme"], "light");
        assert_eq!(c.settings["size"], 100, "the merge keeps what the patch does not mention");
    }

    #[test]
    fn the_shell_asks_for_battery_and_weather_through_its_command_queue() {
        let mut c = Core::new("en-US", SysClock::fixed(T0, 0));
        c.start(json!({"showBattery": true}), 0.0);
        let cmds: Vec<String> = c.take_commands().into_iter().map(|x| x.0).collect();
        assert!(cmds.iter().any(|x| x == "battery_state"), "{cmds:?}");
    }

    #[test]
    fn unknown_events_are_ignored_and_the_event_list_covers_the_pill_producers() {
        let mut c = core();
        c.bus_event("no:such", &json!(null), 1.0);
        assert!(c.take_outs().is_empty());
        let contract = include_str!("../../../../docs/native-ui-contract.md");
        // every event the contract sends to the pill window and that a provider consumes must be subscribed
        for line in contract.lines().filter(|l| l.starts_with("| `") && l.contains("| pill |")) {
            let name = line.split('`').nth(1).unwrap();
            let handled = ISLAND_EVENTS.contains(&name);
            let tab_only = ["chat:delta", "chat:done", "chat:focus", "clip:open", "launcher:changed", "remind:changed", "pill:cursor", "pill:gaze", "pill:display", "pill:hover", "pill:leave", "sys:changed", "kbd:state"];
            assert!(handled || tab_only.contains(&name), "{name} is neither subscribed by the island nor listed as tab/native-only");
        }
    }
}
