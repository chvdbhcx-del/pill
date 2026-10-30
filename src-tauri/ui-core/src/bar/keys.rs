//! Keyboard model of the bar: the key event the renderer hands in, the run modes (`cmd_run`'s `mode` argument) and the mapping from a key press to
//! an intent, in the exact order of the `keydown` handler in bar.js (Esc, arrows, F4, Tab, Ctrl+P, Ctrl+Shift+C, Enter).
//! Extension over the JS (asked for by the native port): PageUp / PageDown move the selection by a page and Ctrl+Home / Ctrl+End jump to the
//! first / last row. Plain Home / End are left to the text field's caret, like the JS.

/// A key press as the renderer reports it (DOM-like: `key` is the produced text or name, `code` the physical key).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: String,
    /// physical key name (`KeyP`, `KeyC`, `KeyU`, `Enter`); empty when unknown
    pub code: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    /// the event target is an editable field (Backspace edits it instead of being "typing for the search box")
    pub target_is_field: bool,
    /// the event target is the search input itself (`e.target === q`)
    pub target_is_search: bool,
}

impl KeyEvent {
    pub fn new(key: &str) -> KeyEvent {
        KeyEvent { key: key.into(), ..Default::default() }
    }
    pub fn with_code(mut self, code: &str) -> KeyEvent {
        self.code = code.into();
        self
    }
    pub fn ctrl(mut self) -> KeyEvent {
        self.ctrl = true;
        self
    }
    pub fn shift(mut self) -> KeyEvent {
        self.shift = true;
        self
    }
    pub fn alt(mut self) -> KeyEvent {
        self.alt = true;
        self
    }
    pub fn meta(mut self) -> KeyEvent {
        self.meta = true;
        self
    }
    pub fn in_field(mut self) -> KeyEvent {
        self.target_is_field = true;
        self
    }
    pub fn in_search(mut self) -> KeyEvent {
        self.target_is_search = true;
        self.target_is_field = true;
        self
    }
}

/// `cmd_run`'s `mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    Open,
    /// show in the folder (Ctrl+Enter)
    Folder,
    /// run as administrator (Ctrl+Shift+Enter)
    Admin,
    /// copy only (Shift+Enter; for clipboard rows)
    Copy,
    /// copy the path (Ctrl+Shift+C)
    Path,
    /// pin to the island's launcher strip (Ctrl+P)
    Pin,
    /// the row's trash button: put the uninstall trigger in the box (never sent to the backend)
    Uninstall,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RunMode::Open => "open",
            RunMode::Folder => "folder",
            RunMode::Admin => "admin",
            RunMode::Copy => "copy",
            RunMode::Path => "path",
            RunMode::Pin => "pin",
            RunMode::Uninstall => "uninstall",
        }
    }
    /// A hover-button / context-menu `data-m` value.
    pub fn parse(s: &str) -> Option<RunMode> {
        Some(match s {
            "open" => RunMode::Open,
            "folder" => RunMode::Folder,
            "admin" => RunMode::Admin,
            "copy" => RunMode::Copy,
            "path" => RunMode::Path,
            "pin" => RunMode::Pin,
            "uninstall" => RunMode::Uninstall,
            _ => return None,
        })
    }
}

/// `modeOf(e)`: Ctrl+Shift = admin, Ctrl = folder, Shift = copy, otherwise open. Used for Enter and for clicks.
pub fn mode_of(ctrl: bool, shift: bool) -> RunMode {
    match (ctrl, shift) {
        (true, true) => RunMode::Admin,
        (true, false) => RunMode::Folder,
        (false, true) => RunMode::Copy,
        (false, false) => RunMode::Open,
    }
}

/// How many rows PageUp / PageDown move (the list shows about this many at once).
pub const PAGE_ROWS: usize = 6;

/// What a key press means to the bar (after the uninstall panel had its chance, see `Bar::on_event`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// close the context menu, or toggle the bar off
    Escape,
    /// select next / previous row (wraps)
    Move(i32),
    /// PageUp / PageDown: move by `PAGE_ROWS` (clamped, no wrap)
    Page(i32),
    /// Ctrl+Home / Ctrl+End
    Edge { last: bool },
    /// F4 dictation
    Dictate,
    /// Tab / Shift+Tab: jump to the next / previous group of rows
    Group(i32),
    /// Ctrl+P
    Pin,
    /// Ctrl+Shift+C
    CopyPath,
    /// Enter with its modifiers
    Run(RunMode),
    /// not a bar shortcut: the text field gets it
    Pass,
}

/// The `keydown` handler of bar.js as a pure mapping, in the same order of tests.
pub fn intent_of(e: &KeyEvent) -> Intent {
    let k = e.key.as_str();
    if k == "Escape" {
        Intent::Escape
    } else if k == "ArrowDown" {
        Intent::Move(1)
    } else if k == "ArrowUp" {
        Intent::Move(-1)
    } else if k == "F4" && !e.alt {
        Intent::Dictate
    } else if k == "Tab" {
        Intent::Group(if e.shift { -1 } else { 1 })
    } else if e.ctrl && !e.shift && !e.alt && e.code == "KeyP" {
        Intent::Pin
    } else if e.ctrl && e.shift && e.code == "KeyC" {
        Intent::CopyPath
    } else if k == "Enter" {
        Intent::Run(mode_of(e.ctrl, e.shift))
    } else if k == "PageDown" && !e.ctrl && !e.alt {
        Intent::Page(1)
    } else if k == "PageUp" && !e.ctrl && !e.alt {
        Intent::Page(-1)
    } else if k == "Home" && e.ctrl && !e.alt {
        Intent::Edge { last: false }
    } else if k == "End" && e.ctrl && !e.alt {
        Intent::Edge { last: true }
    } else {
        Intent::Pass
    }
}

/// Does the browser-style default of this key have to be suppressed (`preventDefault`)? Everything the bar handles except `Pass`.
pub fn consumes(i: &Intent) -> bool {
    !matches!(i, Intent::Pass)
}

/// Ctrl+U anywhere in the bar brings the uninstall panel back (checked before every other shortcut).
pub fn is_ctrl_u(e: &KeyEvent) -> bool {
    e.ctrl && !e.shift && !e.alt && e.code == "KeyU"
}

/// A keystroke that is typing for the search box, not one of the panel's own controls (Space / Enter / Tab / arrows / Esc stay with the panel).
/// `isTyping` of uninstall-state.js; `None` is the JS `null`.
pub fn is_typing(e: Option<&KeyEvent>) -> bool {
    let Some(e) = e else { return false };
    if e.ctrl || e.alt || e.meta {
        return false;
    }
    if e.key == "Backspace" || e.key == "Delete" {
        return !e.target_is_field;
    }
    e.key.chars().count() == 1 && e.key != " "
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_follow_modifiers() {
        assert_eq!(mode_of(false, false), RunMode::Open);
        assert_eq!(mode_of(true, false), RunMode::Folder);
        assert_eq!(mode_of(false, true), RunMode::Copy);
        assert_eq!(mode_of(true, true), RunMode::Admin);
        assert_eq!(RunMode::parse("admin"), Some(RunMode::Admin));
        assert_eq!(RunMode::parse("bogus"), None);
        for m in [RunMode::Open, RunMode::Folder, RunMode::Admin, RunMode::Copy, RunMode::Path, RunMode::Pin, RunMode::Uninstall] {
            assert_eq!(RunMode::parse(m.as_str()), Some(m));
        }
    }

    #[test]
    fn intents_in_handler_order() {
        assert_eq!(intent_of(&KeyEvent::new("Escape")), Intent::Escape);
        assert_eq!(intent_of(&KeyEvent::new("ArrowDown")), Intent::Move(1));
        assert_eq!(intent_of(&KeyEvent::new("ArrowUp")), Intent::Move(-1));
        assert_eq!(intent_of(&KeyEvent::new("F4")), Intent::Dictate);
        assert_eq!(intent_of(&KeyEvent::new("F4").alt()), Intent::Pass, "Alt+F4 closes windows");
        assert_eq!(intent_of(&KeyEvent::new("Tab")), Intent::Group(1));
        assert_eq!(intent_of(&KeyEvent::new("Tab").shift()), Intent::Group(-1));
        assert_eq!(intent_of(&KeyEvent::new("p").with_code("KeyP").ctrl()), Intent::Pin);
        assert_eq!(intent_of(&KeyEvent::new("p").with_code("KeyP").ctrl().shift()), Intent::Pass);
        assert_eq!(intent_of(&KeyEvent::new("C").with_code("KeyC").ctrl().shift()), Intent::CopyPath);
        assert_eq!(intent_of(&KeyEvent::new("c").with_code("KeyC").ctrl()), Intent::Pass, "plain Ctrl+C copies text in the field");
        assert_eq!(intent_of(&KeyEvent::new("Enter")), Intent::Run(RunMode::Open));
        assert_eq!(intent_of(&KeyEvent::new("Enter").ctrl()), Intent::Run(RunMode::Folder));
        assert_eq!(intent_of(&KeyEvent::new("Enter").ctrl().shift()), Intent::Run(RunMode::Admin));
        assert_eq!(intent_of(&KeyEvent::new("Enter").shift()), Intent::Run(RunMode::Copy));
        assert_eq!(intent_of(&KeyEvent::new("a")), Intent::Pass);
    }

    #[test]
    fn layer_ctrl_p_works_on_non_latin_layouts() {
        // `code` is the physical key: the Russian layout types "з" on KeyP
        assert_eq!(intent_of(&KeyEvent::new("з").with_code("KeyP").ctrl()), Intent::Pin);
        assert_eq!(intent_of(&KeyEvent::new("с").with_code("KeyC").ctrl().shift()), Intent::CopyPath);
        assert!(is_ctrl_u(&KeyEvent::new("г").with_code("KeyU").ctrl()));
    }

    #[test]
    fn page_and_edge_keys() {
        assert_eq!(intent_of(&KeyEvent::new("PageDown")), Intent::Page(1));
        assert_eq!(intent_of(&KeyEvent::new("PageUp")), Intent::Page(-1));
        assert_eq!(intent_of(&KeyEvent::new("Home").ctrl()), Intent::Edge { last: false });
        assert_eq!(intent_of(&KeyEvent::new("End").ctrl()), Intent::Edge { last: true });
        assert_eq!(intent_of(&KeyEvent::new("Home")), Intent::Pass, "plain Home moves the caret");
        assert_eq!(intent_of(&KeyEvent::new("End")), Intent::Pass);
        assert!(consumes(&Intent::Dictate) && !consumes(&Intent::Pass));
    }

    #[test]
    fn typing_detection_matches_the_js_vectors() {
        let key = |k: &str| KeyEvent::new(k);
        for k in ["a", "Z", "щ", "7", ".", "-", "😀"] {
            assert!(is_typing(Some(&key(k))), "{k}");
        }
        assert!(is_typing(Some(&key("Backspace"))) && is_typing(Some(&key("Delete"))));
        for k in [" ", "Enter", "Tab", "Escape", "ArrowDown", "ArrowUp", "F4", "Shift", "Control", "Home"] {
            assert!(!is_typing(Some(&key(k))), "{k}");
        }
        assert!(!is_typing(Some(&key("u").ctrl())) && !is_typing(Some(&key("c").meta())) && !is_typing(Some(&key("x").alt())), "shortcuts are not typing");
        assert!(!is_typing(Some(&key("Backspace").in_field())), "a field edits itself");
        assert!(!is_typing(None));
    }
}
