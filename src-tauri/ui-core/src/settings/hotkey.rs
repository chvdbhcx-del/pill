//! Hotkeys page logic: the action table, combo parsing/conflicts (a pure port of `settingswin.rs`), and the key recorder
//! state machine of `settings.js` (Esc cancels, Backspace/Delete clears, a combo needs a modifier). The OS side
//! (`RegisterHotKey` probe, registration) stays in the backend: the UI sends `hotkey_set { action, combo }` and shows the
//! reason it gets back (`invalid | reserved | used | taken`).
use crate::settings::tr::{tr, Tr};
use serde_json::Value;

pub struct HotkeyAction {
    /// `hotkey.<id>` in settings.json.
    pub id: &'static str,
    pub title: Tr,
    pub hint: Tr,
    /// Default combo, "" = not assigned.
    pub default: &'static str,
}

const fn a(id: &'static str, title: Tr, hint: Tr, default: &'static str) -> HotkeyAction {
    HotkeyAction { id, title, hint, default }
}

/// The rows of Settings > Hotkeys, in screen order (the first six also appear on step 3 of the wizard).
pub const ACTIONS: [HotkeyAction; 16] = [
    a("toggle", tr("Открыть / закрыть остров", "Open / close the pill"), Tr::NONE, "Ctrl+Alt+I"),
    a("command", tr("Командная строка", "Command bar"), tr("Программы, калькулятор, конвертер, вопрос ИИ", "Apps, calculator, converter, ask the AI"), "Alt+Shift+Space"),
    a("chat", tr("Чат с ИИ", "AI chat"), tr("Остров открывается на вкладке «Чат», курсор сразу в поле ввода", "The pill opens on the Chat tab with the cursor in the input"), "Ctrl+Alt+A"),
    a("voice", tr("Голос в текст", "Voice to text"), tr("Нажми, скажи, нажми ещё раз: текст окажется в буфере", "Press, speak, press again: the text lands on the clipboard"), "Ctrl+Alt+D"),
    a("record", tr("Запись экрана: начать / остановить", "Screen recording: start / stop"), Tr::NONE, "Ctrl+Alt+R"),
    a("shotArea", tr("Скриншот области", "Screenshot of an area"), Tr::NONE, "Ctrl+Alt+S"),
    a("shotFull", tr("Скриншот всего экрана", "Screenshot of the whole screen"), Tr::NONE, "Ctrl+Alt+Shift+S"),
    a("ocr", tr("Текст с экрана (OCR)", "Text from the screen (OCR)"), Tr::NONE, "Ctrl+Alt+O"),
    // In the JS page but absent from settingswin::ACTIONS: hotkey_set answers reason "action" for it (see tests::uninstall_row).
    a("uninstall", tr("Полное удаление программ", "Complete app uninstall"), tr("В командной строке: u, del или «удалить» + название. Свой деинсталлятор, затем остатки (в Корзину)", "In the command bar: u, del or \"uninstall\" + a name. Its own uninstaller, then leftovers (to the Recycle Bin)"), ""),
    a("clipboard", tr("История буфера", "Clipboard history"), Tr::NONE, "Ctrl+Alt+V"),
    a("pastePlain", tr("Вставить без форматирования", "Paste without formatting"), tr("Текст из Word или с сайта без шрифтов и цветов", "Text from Word or a website without fonts and colours"), "Ctrl+Alt+Shift+V"),
    a("mic", tr("Микрофон: выключить / включить", "Microphone: mute / unmute"), Tr::NONE, "Ctrl+Alt+M"),
    a("pipette", tr("Пипетка", "Colour picker"), Tr::NONE, "Ctrl+Alt+C"),
    a("qr", tr("QR-код с экрана", "QR code from the screen"), Tr::NONE, "Ctrl+Alt+Shift+Q"),
    a("passThrough", tr("Сквозной режим: включить / выключить", "Pass-through mode: on / off"), Tr::NONE, ""),
    a("vpn", tr("VPN: включить / выключить", "VPN: on / off"), tr("Пока ничего не делает: переключателя VPN в Pill ещё нет, клавиша только запоминается", "Does nothing yet: Pill has no VPN switch, the key is only remembered"), ""),
];

/// The actions the wizard's third step lists.
pub fn wizard_actions() -> &'static [HotkeyAction] {
    &ACTIONS[..6]
}

pub fn action(id: &str) -> Option<&'static HotkeyAction> {
    ACTIONS.iter().find(|x| x.id == id)
}

pub fn key_of(action: &str) -> String {
    format!("hotkey.{action}")
}

/// Combos owned by the app itself: pill.rs registers Ctrl+Alt+Q = quit with a raw RegisterHotKey.
pub const RESERVED: &[(&str, &str)] = &[("Ctrl+Alt+Q", "quit")];

pub const MOD_ALT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

fn key_vk(k: &str) -> Option<(String, u32)> {
    let up = k.to_ascii_uppercase();
    let b = up.as_bytes();
    if b.len() == 1 && b[0].is_ascii_alphanumeric() {
        return Some((up.clone(), b[0] as u32));
    }
    if let Some(n) = up.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        return Some((format!("F{n}"), 0x70 + n - 1));
    }
    let (name, vk) = match up.as_str() {
        "SPACE" => ("Space", 0x20),
        "ENTER" | "RETURN" => ("Enter", 0x0D),
        "TAB" => ("Tab", 0x09),
        "UP" | "ARROWUP" => ("Up", 0x26),
        "DOWN" | "ARROWDOWN" => ("Down", 0x28),
        "LEFT" | "ARROWLEFT" => ("Left", 0x25),
        "RIGHT" | "ARROWRIGHT" => ("Right", 0x27),
        "HOME" => ("Home", 0x24),
        "END" => ("End", 0x23),
        "PAGEUP" => ("PageUp", 0x21),
        "PAGEDOWN" => ("PageDown", 0x22),
        "INSERT" => ("Insert", 0x2D),
        "DELETE" | "DEL" => ("Delete", 0x2E),
        _ => return None,
    };
    Some((name.to_string(), vk))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Combo {
    /// Normalised text, for example "Ctrl+Alt+I".
    pub text: String,
    pub mods: u32,
    pub vk: u32,
}

/// "alt+shift+space" -> "Alt+Shift+Space". Needs exactly one key and a modifier (a bare F1-F24 is allowed).
pub fn parse_combo(spec: &str) -> Option<Combo> {
    let (mut mods, mut key) = (0u32, None);
    for part in spec.split('+').map(str::trim) {
        match part.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" => mods |= MOD_CTRL,
            "ALT" => mods |= MOD_ALT,
            "SHIFT" => mods |= MOD_SHIFT,
            "WIN" | "SUPER" | "META" => mods |= MOD_WIN,
            _ => {
                if key.is_some() {
                    return None;
                }
                key = Some(key_vk(part)?);
            }
        }
    }
    let (name, vk) = key?;
    let fkey = (0x70..=0x87).contains(&vk);
    if mods == 0 && !fkey {
        return None;
    }
    let mut s = String::new();
    for (m, n) in [(MOD_CTRL, "Ctrl"), (MOD_ALT, "Alt"), (MOD_SHIFT, "Shift"), (MOD_WIN, "Win")] {
        if mods & m != 0 {
            s.push_str(n);
            s.push('+');
        }
    }
    s.push_str(&name);
    Some(Combo { text: s, mods, vk })
}

/// Who already holds `norm` (an action other than `action`, or a reserved combo). `bound`: (action, combo) pairs.
pub fn conflict(norm: &str, action: &str, bound: &[(String, String)]) -> Option<String> {
    if let Some((_, by)) = RESERVED.iter().find(|(c, _)| c.eq_ignore_ascii_case(norm)) {
        return Some(by.to_string());
    }
    bound.iter().find(|(a, c)| a != action && !c.is_empty() && c.eq_ignore_ascii_case(norm)).map(|(a, _)| a.clone())
}

/// A known action: one of the table, or `launcher.<id>` with a short safe id.
pub fn is_known_action(action: &str) -> bool {
    ACTIONS.iter().any(|x| x.id == action && x.id != "uninstall") || (action.starts_with("launcher.") && action.len() < 60 && action.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-'))
}

/// Why a combo was refused: the `reason` codes of `hotkey_set` / `hotkey_check`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyError {
    Invalid,
    Reserved,
    Used,
    Taken,
    Action,
}

impl HotkeyError {
    pub fn from_reason(r: &str) -> Option<HotkeyError> {
        Some(match r {
            "invalid" => HotkeyError::Invalid,
            "reserved" => HotkeyError::Reserved,
            "used" => HotkeyError::Used,
            "taken" => HotkeyError::Taken,
            "action" => HotkeyError::Action,
            _ => return None,
        })
    }
    pub fn reason(self) -> &'static str {
        match self {
            HotkeyError::Invalid => "invalid",
            HotkeyError::Reserved => "reserved",
            HotkeyError::Used => "used",
            HotkeyError::Taken => "taken",
            HotkeyError::Action => "action",
        }
    }
    /// The line shown under the row (red). `Action` and unknown reasons read "Could not save", like the JS page.
    pub fn message(self) -> Tr {
        match self {
            HotkeyError::Invalid => tr("Нужна клавиша-модификатор: Ctrl, Alt, Shift или Win", "A modifier key is needed: Ctrl, Alt, Shift or Win"),
            HotkeyError::Reserved => tr("Это сочетание зарезервировано: Ctrl+Alt+Q закрывает Pill", "This shortcut is reserved: Ctrl+Alt+Q quits Pill"),
            HotkeyError::Used => tr("Уже занято другим действием", "Already used by another action"),
            HotkeyError::Taken => tr("Занято другой программой", "Taken by another program"),
            HotkeyError::Action => COULD_NOT_SAVE,
        }
    }
}

pub const COULD_NOT_SAVE: Tr = tr("Не удалось сохранить", "Could not save");

/// The part of the backend check the UI can do without the OS: normalisation, reserved and used-by-another-action.
/// An empty combo clears the binding and is always fine. `Err(Taken)` only comes from the backend.
pub fn check_local(combo: &str, action: &str, bound: &[(String, String)]) -> Result<String, HotkeyError> {
    if combo.trim().is_empty() {
        return Ok(String::new());
    }
    if !is_known_action(action) {
        return Err(HotkeyError::Action);
    }
    let c = parse_combo(combo).ok_or(HotkeyError::Invalid)?;
    match conflict(&c.text, action, bound) {
        Some(by) if by == "quit" => Err(HotkeyError::Reserved),
        Some(_) => Err(HotkeyError::Used),
        None => Ok(c.text),
    }
}

/// Result of `hotkey_set` as the page reads it.
#[derive(Clone, Debug, PartialEq)]
pub enum HotkeyOutcome {
    /// Stored; the row now shows this combo ("" = not assigned) and clears its message.
    Saved(String),
    /// Refused; the row keeps the old combo and shows the message in red.
    Refused(HotkeyError),
}

pub fn outcome(result: &Value) -> HotkeyOutcome {
    if result.get("ok") == Some(&Value::Bool(true)) {
        return HotkeyOutcome::Saved(result.get("combo").and_then(Value::as_str).unwrap_or("").to_string());
    }
    HotkeyOutcome::Refused(result.get("reason").and_then(Value::as_str).and_then(HotkeyError::from_reason).unwrap_or(HotkeyError::Action))
}

// ------------------------------------------------------------------------------------------------ recorder

/// A key press as the renderer reports it (DOM-style: `key` for Escape/Backspace, `code` for the physical key).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: String,
    pub code: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

/// `KeyA` -> "A", `Digit5` -> "5", `F1`..`F24`, and the named keys; None for modifiers and everything unsupported.
pub fn code_key(code: &str) -> Option<String> {
    let b = code.as_bytes();
    if code.len() == 4 && code.starts_with("Key") && b[3].is_ascii_uppercase() {
        return Some(code[3..].to_string());
    }
    if code.len() == 6 && code.starts_with("Digit") && b[5].is_ascii_digit() {
        return Some(code[5..].to_string());
    }
    if let Some(n) = code.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()).filter(|n| (1..=24).contains(n)) {
        if code == format!("F{n}") {
            return Some(code.to_string());
        }
    }
    Some(
        match code {
            "Space" => "Space",
            "Enter" => "Enter",
            "Tab" => "Tab",
            "ArrowUp" => "Up",
            "ArrowDown" => "Down",
            "ArrowLeft" => "Left",
            "ArrowRight" => "Right",
            "Home" => "Home",
            "End" => "End",
            "PageUp" => "PageUp",
            "PageDown" => "PageDown",
            "Insert" => "Insert",
            "Delete" => "Delete",
            _ => return None,
        }
        .to_string(),
    )
}

/// The modifiers held, in the order the combo is written.
pub fn held_mods(e: &KeyEvent) -> Vec<&'static str> {
    [(e.ctrl, "Ctrl"), (e.alt, "Alt"), (e.shift, "Shift"), (e.meta, "Win")].into_iter().filter(|(on, _)| *on).map(|(_, n)| n).collect()
}

/// Keycap text: Shift is drawn as ⇧.
pub fn cap_text(part: &str) -> &str {
    if part == "Shift" {
        "⇧"
    } else {
        part
    }
}

/// The caps of a stored combo; empty for "not assigned" (the renderer then shows `NOT_ASSIGNED`).
pub fn key_caps(combo: &str) -> Vec<String> {
    if combo.is_empty() {
        return vec![];
    }
    combo.split('+').map(|p| cap_text(p).to_string()).collect()
}

pub const NOT_ASSIGNED: Tr = tr("не назначено", "not assigned");
pub const PRESS_SHORTCUT: Tr = tr("Нажмите сочетание…", "Press the shortcut…");
pub const RECORDER_TOOLTIP: Tr = tr("Нажмите, затем сочетание клавиш. Esc — отмена, Backspace — сбросить", "Click, then press the shortcut. Esc cancels, Backspace clears");

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecStep {
    /// Stop recording, show the old combo again.
    Cancel,
    /// Stop recording and commit "" (clears the binding).
    Clear,
    /// Only modifiers so far: show their caps followed by " …".
    Partial(Vec<&'static str>),
    /// Stop recording and send `hotkey_set { combo }`.
    Commit(String),
}

/// At most one recorder is active in the window (`rec` in settings.js); starting another stops the first.
#[derive(Debug, Default)]
pub struct Recorder {
    active: Option<String>,
}

impl Recorder {
    /// Click / Enter / Space on a combo box. Returns the action whose recording was stopped to make room, if any;
    /// None also when `action` is already recording (the click does nothing then).
    pub fn start(&mut self, action: &str) -> Option<String> {
        if self.active.as_deref() == Some(action) {
            return None;
        }
        self.active.replace(action.to_string())
    }
    pub fn is_recording(&self, action: &str) -> bool {
        self.active.as_deref() == Some(action)
    }
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }
    pub fn stop(&mut self) -> Option<String> {
        self.active.take()
    }
    /// A key goes down while recording. Returns None when nothing is recording.
    pub fn key_down(&mut self, e: &KeyEvent) -> Option<RecStep> {
        self.active.as_ref()?;
        if e.key == "Escape" {
            self.active = None;
            return Some(RecStep::Cancel);
        }
        let mods = held_mods(e);
        if (e.key == "Backspace" || e.key == "Delete") && mods.is_empty() {
            self.active = None;
            return Some(RecStep::Clear);
        }
        match code_key(&e.code) {
            None => Some(RecStep::Partial(mods)),
            Some(k) => {
                self.active = None;
                let mut parts: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
                parts.push(k);
                Some(RecStep::Commit(parts.join("+")))
            }
        }
    }
    /// Mouse down outside the box, or the window lost focus.
    pub fn cancel(&mut self) -> bool {
        self.active.take().is_some()
    }
}

/// One launcher shortcut row: `hotkey.launcher.<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherShortcut {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub action: String,
    pub key: String,
}

/// From the `launcher_list` answer (an array of {id, title, icon?}); anything else gives an empty list.
pub fn launcher_shortcuts(list: &Value) -> Vec<LauncherShortcut> {
    let Some(a) = list.as_array() else { return vec![] };
    a.iter()
        .filter_map(|l| {
            let id = l.get("id").and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_i64().map(|n| n.to_string())))?;
            let title = l.get("title").and_then(Value::as_str).unwrap_or("").to_string();
            let icon = l.get("icon").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
            Some(LauncherShortcut { action: format!("launcher.{id}"), key: format!("hotkey.launcher.{id}"), id, title, icon })
        })
        .collect()
}

pub const LAUNCHER_FOOTNOTE: Tr = tr("Ярлыки лаунчера: каждому можно дать свою клавишу, и он будет открываться ею из любой программы.", "Launcher shortcuts: give each item its own key and it opens from any program.");
pub const LAUNCHER_EMPTY_TITLE: Tr = tr("Лаунчер пуст", "The launcher is empty");
pub const LAUNCHER_EMPTY_HINT: Tr = tr("Добавь программы на вкладке «Главная»: перетащи их на остров", "Add programs on the Home tab: drop them on the pill");

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(key: &str, code: &str, mods: &str) -> KeyEvent {
        KeyEvent { key: key.into(), code: code.into(), ctrl: mods.contains('c'), alt: mods.contains('a'), shift: mods.contains('s'), meta: mods.contains('w') }
    }

    #[test]
    fn combos_normalise() {
        assert_eq!(parse_combo("alt+shift+space").unwrap().text, "Alt+Shift+Space");
        assert_eq!(parse_combo("Shift + Ctrl + v").unwrap().text, "Ctrl+Shift+V");
        assert_eq!(parse_combo("Ctrl+Alt+i").unwrap(), Combo { text: "Ctrl+Alt+I".into(), mods: MOD_CTRL | MOD_ALT, vk: 'I' as u32 });
        assert_eq!((parse_combo("f9").unwrap().text, parse_combo("f9").unwrap().vk), ("F9".to_string(), 0x78));
        assert_eq!(parse_combo("win+arrowup").unwrap().text, "Win+Up");
        for bad in ["A", "Ctrl+", "Ctrl+A+B", "Ctrl+F25", "Ctrl+Alt", ""] {
            assert!(parse_combo(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn conflicts_and_local_check() {
        let b = vec![("toggle".to_string(), "Ctrl+Alt+I".to_string()), ("qr".to_string(), String::new()), ("chat".to_string(), "Ctrl+Alt+A".to_string())];
        assert_eq!(conflict("Ctrl+Alt+I", "chat", &b).as_deref(), Some("toggle"));
        assert_eq!(conflict("Ctrl+Alt+I", "toggle", &b), None, "own combo is no conflict");
        assert_eq!(conflict("ctrl+alt+a", "toggle", &b).as_deref(), Some("chat"));
        assert_eq!(conflict("Ctrl+Alt+Q", "qr", &b).as_deref(), Some("quit"));
        assert_eq!(check_local("ctrl+alt+i", "chat", &b), Err(HotkeyError::Used));
        assert_eq!(check_local("Ctrl+Alt+Q", "qr", &b), Err(HotkeyError::Reserved));
        assert_eq!(check_local("Ctrl", "qr", &b), Err(HotkeyError::Invalid));
        assert_eq!(check_local("ctrl+alt+z", "qr", &b).as_deref(), Ok("Ctrl+Alt+Z"));
        assert_eq!(check_local("", "qr", &b).as_deref(), Ok(""));
        assert_eq!(check_local("Ctrl+Alt+Z", "nope", &b), Err(HotkeyError::Action));
        assert_eq!(check_local("Ctrl+Alt+Z", "launcher.a-1", &b).as_deref(), Ok("Ctrl+Alt+Z"));
        assert!(!is_known_action("launcher.a b") && is_known_action("launcher.x"));
    }

    #[test]
    fn every_default_parses_is_unique_and_not_reserved() {
        let mut seen = std::collections::HashSet::new();
        for x in ACTIONS.iter().filter(|x| !x.default.is_empty()) {
            assert!(parse_combo(x.default).is_some(), "{}", x.id);
            assert_eq!(parse_combo(x.default).unwrap().text, x.default, "{} default is stored normalised", x.id);
            assert!(seen.insert(x.default.to_ascii_lowercase()), "duplicate default {}", x.default);
            assert!(conflict(x.default, x.id, &[]).is_none(), "{} hits a reserved combo", x.id);
        }
        assert_eq!(wizard_actions().iter().map(|x| x.id).collect::<Vec<_>>(), ["toggle", "command", "chat", "voice", "record", "shotArea"]);
    }

    #[test]
    fn recorder_follows_the_js_state_machine() {
        let mut r = Recorder::default();
        assert_eq!(r.key_down(&ev("a", "KeyA", "c")), None, "idle recorder ignores keys");
        assert_eq!(r.start("toggle"), None);
        assert_eq!(r.start("toggle"), None, "a second click on the same box does nothing");
        assert!(r.is_recording("toggle"));
        assert_eq!(r.key_down(&ev("Control", "ControlLeft", "c")), Some(RecStep::Partial(vec!["Ctrl"])));
        assert_eq!(r.key_down(&ev("Alt", "AltLeft", "ca")), Some(RecStep::Partial(vec!["Ctrl", "Alt"])));
        assert_eq!(r.key_down(&ev("k", "KeyK", "cas")), Some(RecStep::Commit("Ctrl+Alt+Shift+K".into())));
        assert!(r.active().is_none());
        r.start("chat");
        assert_eq!(r.start("voice").as_deref(), Some("chat"), "starting another stops the first");
        assert_eq!(r.key_down(&ev("Escape", "Escape", "")), Some(RecStep::Cancel));
        r.start("chat");
        assert_eq!(r.key_down(&ev("Backspace", "Backspace", "")), Some(RecStep::Clear));
        r.start("chat");
        assert_eq!(r.key_down(&ev("Backspace", "Backspace", "c")), Some(RecStep::Partial(vec!["Ctrl"])), "Backspace with a modifier is not a clear");
        assert!(r.cancel() && !r.cancel(), "click outside / blur");
        r.start("chat");
        assert_eq!(r.key_down(&ev("F5", "F5", "")), Some(RecStep::Commit("F5".into())));
    }

    #[test]
    fn code_keys_and_caps() {
        assert_eq!(code_key("KeyQ").as_deref(), Some("Q"));
        assert_eq!(code_key("Digit7").as_deref(), Some("7"));
        assert_eq!(code_key("F24").as_deref(), Some("F24"));
        assert_eq!(code_key("F25"), None);
        assert_eq!(code_key("F0"), None);
        assert_eq!(code_key("ArrowLeft").as_deref(), Some("Left"));
        assert_eq!(code_key("ShiftLeft"), None);
        assert_eq!(code_key("Keya"), None);
        assert_eq!(key_caps("Ctrl+Shift+V"), ["Ctrl", "⇧", "V"]);
        assert!(key_caps("").is_empty());
    }

    #[test]
    fn backend_answers_map_to_messages() {
        assert_eq!(outcome(&json!({"ok": true, "combo": "Ctrl+Alt+K"})), HotkeyOutcome::Saved("Ctrl+Alt+K".into()));
        assert_eq!(outcome(&json!({"ok": true, "combo": ""})), HotkeyOutcome::Saved(String::new()));
        assert_eq!(outcome(&json!({"ok": false, "reason": "taken"})), HotkeyOutcome::Refused(HotkeyError::Taken));
        assert_eq!(outcome(&json!({"ok": false, "reason": "weird"})), HotkeyOutcome::Refused(HotkeyError::Action));
        assert_eq!(outcome(&json!(null)), HotkeyOutcome::Refused(HotkeyError::Action));
        assert!(HotkeyError::Reserved.message().ru.contains("Ctrl+Alt+Q"));
        for e in [HotkeyError::Invalid, HotkeyError::Reserved, HotkeyError::Used, HotkeyError::Taken, HotkeyError::Action] {
            assert_eq!(HotkeyError::from_reason(e.reason()), Some(e));
            assert!(e.message().is_complete());
        }
    }

    #[test]
    fn launcher_list_becomes_shortcuts() {
        let l = launcher_shortcuts(&json!([{"id": "a1", "title": "Notepad", "icon": "data:x"}, {"id": 7, "title": "Calc"}, {"title": "no id"}]));
        assert_eq!(l.len(), 2);
        assert_eq!((l[0].action.as_str(), l[0].key.as_str(), l[0].icon.as_deref()), ("launcher.a1", "hotkey.launcher.a1", Some("data:x")));
        assert_eq!(l[1].id, "7");
        assert!(launcher_shortcuts(&json!(null)).is_empty());
    }

    #[test]
    fn uninstall_row() {
        // the JS page lists it, the backend does not know it: documented here so it is not "fixed" silently
        assert!(action("uninstall").is_some() && !is_known_action("uninstall"));
    }
}
