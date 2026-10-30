//! What the panel tabs ask of the island (`ui::tabs::fx::TabFx`) turned into shell / hub effects. No window, no GPU: unit tested.
//!
//! Effects that only touch the model happen inside [`Core::tab_fx`]; the ones that need the window or the backend come back as [`Act`] for the view.
//!
//! * `Toast` / `Card`    -> a card in the shell queue (`Core::show_card`), the notifications tab records it.
//! * `Live`              -> ignored: the A tabs' rows (music, shelf count, drop card) are owned by the hub providers (`MediaLive`, `ShelfLive`), which
//!                          listen to the same events; a second writer would double the row.
//! * `SetLive`           -> a one-line text row in the shell's slot table (key = row id, like `Pill.setLive(key, ..)`).
//! * `React` / `Unreact` -> the reaction resolver. `EdgePulse` -> the edge halo. `Badge` -> the tab button counter.
//! * `Collapse` / `Expand` / `SetPinned` / `DragPull` / `EdgeColor` -> the shell.
use super::core::Core;
use crate::ui::tabs::fx::TabFx;
use pill_ui_core::live::model::{Accent, Action, ActionKind, CardAction, CardSpec, Elem, IconName, Rgba, RowSpec, TextStyle, Tone};
use pill_ui_core::panel::b::{LiveRow, ToastAction, ToastRun, ToastSpec};
use pill_ui_core::shell::lives::LiveInput;
use pill_ui_core::shell::react::ReactOpts;
use serde_json::{json, Value};

/// What the view still has to do after [`Core::tab_fx`].
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// A backend command (`cmd_toggle`, `voice_toggle`, `settings_open`, `settings_set`).
    Call { cmd: &'static str, args: Value },
    /// Collapse first, then start a screenshot of this mode after the island is gone (`PillCapture.start`).
    Capture { mode: &'static str },
    /// Open / close the `ai_actions` overlay over the clipboard tab.
    OpenAi(i64),
    CloseAi,
    /// The overlay told us it closed.
    AiClosed,
}

const ALL_ICONS: [IconName; 34] = [
    IconName::Area, IconName::Bell, IconName::Battery, IconName::Bolt, IconName::Calendar, IconName::Check, IconName::Clipboard, IconName::Copy,
    IconName::Download, IconName::File, IconName::Folder, IconName::Keyboard, IconName::Mail, IconName::Mic, IconName::Monitor, IconName::Mute,
    IconName::Next, IconName::Note2, IconName::Pause, IconName::Play, IconName::Plus, IconName::Prev, IconName::Qr, IconName::ScanText,
    IconName::Shelf, IconName::Sparkles, IconName::Terminal, IconName::Timer, IconName::Trash, IconName::Usb, IconName::Video, IconName::Volume,
    IconName::Warn, IconName::X,
];

/// The live-row glyph of an `icons.js` name; unknown names fall back to a bell.
pub fn icon_by_name(n: &str) -> IconName {
    ALL_ICONS.iter().copied().find(|i| i.as_str() == n).unwrap_or(IconName::Bell)
}

fn tone_by_name(n: Option<&str>) -> Tone {
    match n {
        Some("green") => Tone::Green,
        Some("amber") => Tone::Amber,
        Some("red") => Tone::Red,
        Some("accent") => Tone::Accent,
        _ => Tone::Neutral,
    }
}

fn accent_by_name(n: &str) -> Option<Accent> {
    let hex = match n {
        "amber" => "#ff9f0a",
        "green" => "#30d158",
        "red" => "#ff453a",
        "accent" => return Some(Accent::Theme),
        h if h.starts_with('#') => h,
        _ => return None,
    };
    Rgba::from_hex(hex).map(Accent::Rgb)
}

/// `Pill.toast({icon, title})` of an A tab.
pub fn card_from_toast(id: &str, icon: &str, title: &str, ttl_ms: u32) -> CardSpec {
    CardSpec::text(id, ttl_ms, title, None).icon(icon_by_name(icon))
}

/// `Pill.toast` of a B tab: sub line, tone, up to a few buttons.
pub fn card_from_spec(t: &ToastSpec) -> CardSpec {
    let sub = if t.sub.is_empty() { None } else { Some(t.sub.clone()) };
    let acts = t.actions.iter().map(card_action).collect();
    CardSpec::text(t.id.clone(), t.ttl_ms, t.title.clone(), sub).icon(icon_by_name(&t.icon)).tone(tone_by_name(t.tone)).actions(acts)
}

fn card_action(a: &ToastAction) -> CardAction {
    let kind = if a.kind == Some("white") { ActionKind::White } else { ActionKind::Normal };
    let act = match &a.run {
        ToastRun::Nothing => Action::Noop,
        ToastRun::Call(cmd, args) => Action::invoke(cmd, args.clone()),
        ToastRun::CopyText(t) => Action::CopyText(t.clone()),
        ToastRun::Model(id) if id == "timer:again" => Action::TimerAgain,
        ToastRun::Model(_) => Action::Noop,
    };
    CardAction::new(a.label.clone(), kind, act)
}

/// `Pill.setLive(key, row)` of a B tab as a one-line text row.
pub fn row_from_live(key: &str, r: &LiveRow) -> RowSpec {
    let mut row = RowSpec::new(key, r.priority as i32).key(r.key.clone()).with(Elem::text(r.text.clone(), TextStyle::Label));
    row.ttl_ms = r.ttl_ms;
    if let Some(a) = r.accent.and_then(accent_by_name) {
        row = row.accent(a);
    }
    row
}

fn badge_count(text: &str) -> u32 {
    let t = text.trim();
    if t.is_empty() {
        0
    } else {
        t.parse::<u32>().unwrap_or(1)
    }
}

impl Core {
    /// Applies one tab effect to the shell / hub state and returns what the view must still do.
    pub fn tab_fx(&mut self, fx: TabFx, now: f64) -> Vec<Act> {
        self.now_mono = now;
        let mut acts = Vec::new();
        match fx {
            TabFx::Toast { id, icon, title, ttl_ms } => self.show_card(card_from_toast(id, icon, &title, ttl_ms)),
            TabFx::Card(t) => self.show_card(card_from_spec(&t)),
            TabFx::Live { .. } => {}
            TabFx::SetLive { key, row } => match row {
                Some(r) => {
                    let spec = row_from_live(&key, &r);
                    let input = LiveInput {
                        priority: spec.priority as f64,
                        spec: json!({ "id": key }),
                        key: r.key.clone(),
                        ttl_ms: spec.ttl_ms.map(|t| t as i64).unwrap_or(0),
                        accent: spec.accent.map(super::core::accent_css).unwrap_or_default(),
                        tall: false,
                        urgent: false,
                    };
                    self.rows.insert(key.clone(), spec);
                    self.shell.set_live(&key, Some(input));
                }
                None => {
                    self.rows.remove(&key);
                    self.shell.set_live(&key, None);
                }
            },
            TabFx::React { key, kind, color, ttl_ms } => {
                let o = ReactOpts { ttl_ms: ttl_ms.map(|t| t as i64).unwrap_or(0), color: color.unwrap_or_default() };
                self.shell.react(&key, kind, &o);
            }
            TabFx::Unreact { key } => self.shell.unreact(&key),
            TabFx::EdgePulse(s) => {
                if !self.shell.reduce() {
                    self.pulse = Some((s.clamp(0.0, 1.0), now));
                }
            }
            TabFx::Badge { tab, text } => self.shell.set_badge(tab, badge_count(&text)),
            TabFx::Collapse { via } => self.shell.collapse(via.unwrap_or("")),
            TabFx::Expand { tab } => self.shell.expand(Some(tab), "page"),
            TabFx::SetPinned(b) => {
                self.shell.set_pinned(b);
                acts.push(Act::Call { cmd: "settings_set", args: json!({ "patch": { "pinned": b } }) });
            }
            TabFx::OpenCommandBar => acts.push(Act::Call { cmd: "cmd_toggle", args: Value::Null }),
            TabFx::StartCapture { mode } => acts.push(Act::Capture { mode }),
            TabFx::OpenClipAi { id } => acts.push(Act::OpenAi(id)),
            TabFx::CloseClipAi => acts.push(Act::CloseAi),
            TabFx::DragPull(b) => self.shell.set_drag_pull(b),
            TabFx::EdgeColor(c) => self.shell.set_edge_color(c.as_deref().unwrap_or("")),
            TabFx::VoiceToggle => acts.push(Act::Call { cmd: "voice_toggle", args: Value::Null }),
            TabFx::GoPage(p) => acts.push(Act::Call { cmd: "settings_open", args: json!({ "page": p }) }),
            TabFx::OverlayClosed(_) => acts.push(Act::AiClosed),
        }
        self.pump_notices();
        acts
    }
}
