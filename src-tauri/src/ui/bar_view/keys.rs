//! Keyboard translation: glasskit's `InputEvent::Key` / `Char` (Win32 virtual keys and composed text) into the DOM-like `KeyEvent` the bar model
//! was written against (`key` = produced text or name, `code` = physical key), plus the pure routing rules: which keys the bar consumes and which
//! reach the search box, and the panel's own focus order.
use glasskit::input::{KeyCode, Modifiers};
use pill_ui_core::bar::keys::{intent_of, is_ctrl_u, Intent, KeyEvent};
use pill_ui_core::bar::uninstall::view::{Button, Control, PanelView, Part};

/// `KeyEvent` for a key press that is not text. Letters / digits are only reported with Ctrl or Alt held (shortcuts); plain ones arrive as
/// `Char` and go through [`char_key`]. `None` = a key the bar knows nothing about.
pub fn key_event(code: KeyCode, mods: Modifiers) -> Option<KeyEvent> {
    let named = |k: &str| Some(KeyEvent::new(k));
    let mut e = match code {
        KeyCode::Escape => named("Escape")?,
        KeyCode::Enter => named("Enter")?,
        KeyCode::Tab => named("Tab")?,
        KeyCode::Backspace => named("Backspace")?,
        KeyCode::Delete => named("Delete")?,
        KeyCode::Insert => named("Insert")?,
        KeyCode::Space => KeyEvent::new(" ").with_code("Space"),
        KeyCode::Left => named("ArrowLeft")?,
        KeyCode::Right => named("ArrowRight")?,
        KeyCode::Up => named("ArrowUp")?,
        KeyCode::Down => named("ArrowDown")?,
        KeyCode::Home => named("Home")?,
        KeyCode::End => named("End")?,
        KeyCode::PageUp => named("PageUp")?,
        KeyCode::PageDown => named("PageDown")?,
        KeyCode::F(n) => KeyEvent::new(&format!("F{n}")),
        KeyCode::Char(c) if mods.ctrl || mods.alt || mods.meta => {
            let c = c.to_ascii_uppercase();
            let code = if c.is_ascii_digit() { format!("Digit{c}") } else if c.is_ascii_alphabetic() { format!("Key{c}") } else { String::new() };
            KeyEvent::new(&c.to_ascii_lowercase().to_string()).with_code(&code)
        }
        _ => return None,
    };
    e.ctrl = mods.ctrl;
    e.shift = mods.shift;
    e.alt = mods.alt;
    e.meta = mods.meta;
    Some(e)
}

/// `KeyEvent` of typed text (one composed character). Control characters never get here (the translator drops them).
pub fn char_key(c: char) -> KeyEvent {
    KeyEvent::new(&c.to_string())
}

/// Does the bar swallow this key press (it is not for the search box)? Mirrors the web: the uninstall panel's capture handler first (Ctrl+U,
/// and while the box is on screen Esc / Enter / arrows / Tab / F4 / Ctrl+Shift+..), then the bar's own `keydown`.
pub fn bar_consumes(e: &KeyEvent, panel_open: bool) -> bool {
    if is_ctrl_u(e) {
        return true;
    }
    if panel_open {
        if e.key == "Escape" {
            return true;
        }
        let typing = !e.target_is_search && pill_ui_core::bar::keys::is_typing(Some(e));
        let own = matches!(e.key.as_str(), "Enter" | "ArrowUp" | "ArrowDown" | "Tab" | "F4") || (e.ctrl && e.shift);
        if own && !typing {
            return true;
        }
    }
    !matches!(intent_of(e), Intent::Pass)
}

/// One focusable control of the panel, in reading order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Focusable {
    pub control: Control,
    pub enabled: bool,
    pub default: bool,
}

fn from_button(b: &Button) -> Focusable {
    Focusable { control: b.control, enabled: b.enabled, default: b.default_focus }
}

/// The controls Tab walks through: ticks of the confirm card, the link, every button, the checklist footer (top to bottom, left to right).
pub fn panel_focusables(p: &PanelView) -> Vec<Focusable> {
    let mut v = Vec::new();
    for part in &p.parts {
        match part {
            Part::Check { control, .. } => v.push(Focusable { control: *control, enabled: !p.busy, default: false }),
            Part::Link(b) => v.push(from_button(b)),
            Part::Buttons(bs) => v.extend(bs.iter().map(from_button)),
            Part::Footer(f) => v.extend(f.buttons.iter().map(from_button)),
            _ => {}
        }
    }
    v
}

/// The focus after Tab / Shift+Tab (wraps; disabled controls are skipped). `from` = the focused index, if any.
pub fn next_focus(list: &[Focusable], from: Option<usize>, dir: i32) -> Option<usize> {
    let n = list.len();
    if n == 0 {
        return None;
    }
    let mut i = from.map_or(if dir >= 0 { n - 1 } else { 0 }, |f| f.min(n - 1));
    for _ in 0..n {
        i = (i as i64 + dir.signum() as i64).rem_euclid(n as i64) as usize;
        if list[i].enabled {
            return Some(i);
        }
    }
    None
}

/// The button that takes the keyboard when a step waits for a decision (`.dflt`), else the first enabled one.
pub fn default_focus(list: &[Focusable]) -> Option<usize> {
    list.iter().position(|f| f.default && f.enabled).or_else(|| list.iter().position(|f| f.enabled))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl() -> Modifiers {
        Modifiers::CTRL
    }

    #[test]
    fn named_keys_and_modifiers() {
        let e = key_event(KeyCode::Enter, Modifiers::CTRL_SHIFT).unwrap();
        assert_eq!((e.key.as_str(), e.ctrl, e.shift, e.alt), ("Enter", true, true, false));
        assert_eq!(key_event(KeyCode::F(4), Modifiers::NONE).unwrap().key, "F4");
        assert_eq!(key_event(KeyCode::Down, Modifiers::NONE).unwrap().key, "ArrowDown");
        assert_eq!(key_event(KeyCode::Space, Modifiers::NONE).unwrap().key, " ");
        assert!(key_event(KeyCode::Other(0xFF), Modifiers::NONE).is_none());
    }

    #[test]
    fn plain_letters_are_text_not_keys_and_ctrl_letters_are_shortcuts() {
        assert!(key_event(KeyCode::Char('A'), Modifiers::NONE).is_none(), "typing goes through Char");
        assert!(key_event(KeyCode::Char('A'), Modifiers::SHIFT).is_none());
        let u = key_event(KeyCode::Char('U'), ctrl()).unwrap();
        assert_eq!((u.key.as_str(), u.code.as_str()), ("u", "KeyU"));
        assert!(is_ctrl_u(&u));
        let p = key_event(KeyCode::Char('P'), ctrl()).unwrap();
        assert_eq!(intent_of(&p), Intent::Pin);
        let c = key_event(KeyCode::Char('C'), Modifiers::CTRL_SHIFT).unwrap();
        assert_eq!(intent_of(&c), Intent::CopyPath);
        assert_eq!(key_event(KeyCode::Char('7'), ctrl()).unwrap().code, "Digit7");
    }

    #[test]
    fn typed_text_is_typing_for_the_panel_rules() {
        let k = char_key('щ');
        assert_eq!(k.key, "щ");
        assert!(pill_ui_core::bar::keys::is_typing(Some(&k)));
        assert!(!pill_ui_core::bar::keys::is_typing(Some(&char_key(' '))));
    }

    #[test]
    fn consumption_rules() {
        let k = |c, m| key_event(c, m).unwrap();
        // the box is closed: only the bar's own shortcuts
        assert!(bar_consumes(&k(KeyCode::Down, Modifiers::NONE), false));
        assert!(bar_consumes(&k(KeyCode::Escape, Modifiers::NONE), false));
        assert!(bar_consumes(&k(KeyCode::Enter, Modifiers::NONE), false));
        assert!(!bar_consumes(&k(KeyCode::Left, Modifiers::NONE), false), "caret keys belong to the text box");
        assert!(!bar_consumes(&k(KeyCode::Backspace, Modifiers::NONE), false));
        assert!(!bar_consumes(&k(KeyCode::Char('C'), ctrl()), false), "plain Ctrl+C copies the selection");
        assert!(!bar_consumes(&k(KeyCode::Home, Modifiers::NONE), false));
        assert!(bar_consumes(&k(KeyCode::Home, ctrl()), false));
        // Ctrl+U always
        assert!(bar_consumes(&k(KeyCode::Char('U'), ctrl()), false));
        // the box is open: Enter, arrows, Tab are the panel's
        assert!(bar_consumes(&k(KeyCode::Tab, Modifiers::NONE), true));
        // typing is never consumed
        assert!(!bar_consumes(&char_key('a'), true));
        let mut a = char_key('a');
        a.target_is_search = true;
        assert!(!bar_consumes(&a, true));
    }

    fn f(c: Control, enabled: bool, default: bool) -> Focusable {
        Focusable { control: c, enabled, default }
    }

    #[test]
    fn tab_walks_enabled_controls_and_wraps() {
        let l = vec![f(Control::Start, true, false), f(Control::Cancel, true, true), f(Control::Close, false, false)];
        assert_eq!(default_focus(&l), Some(1));
        assert_eq!(next_focus(&l, Some(1), 1), Some(0), "the disabled one is skipped, then wrap");
        assert_eq!(next_focus(&l, Some(0), -1), Some(1));
        assert_eq!(next_focus(&l, None, 1), Some(0));
        assert_eq!(next_focus(&[], None, 1), None);
        assert_eq!(next_focus(&[f(Control::Start, false, false)], None, 1), None);
        assert_eq!(default_focus(&[f(Control::Start, true, false), f(Control::Close, true, false)]), Some(0));
    }
}
