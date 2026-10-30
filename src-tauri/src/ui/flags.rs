// Feature flags of the native-UI migration. Read-only; nothing uses them yet (later phases switch a surface to native code when its flag is on).
//   native_pill() / native_bar() / native_settings() / native_capture()   settings.json keys ui.native.pill|bar|settings|capture, default false
//   web_ui_forced()                                                       `pill.exe --web-ui` or env PILL_WEB_UI=1: the WebView UI, whatever the keys say
//   native_all_requested()                                                `pill.exe --native` or env PILL_NATIVE=1: all four surfaces native for this run (keys untouched)
// A forced web UI wins: every native_* answers false while it is on, so a broken native surface can always be bypassed from the command line
// (`--native --web-ui` is the WebView UI).
#![allow(dead_code)] // the first user arrives with the native surfaces

use std::sync::OnceLock;

use crate::settings;

/// `args` = the process arguments (program name first), `env` = the value of PILL_WEB_UI if set.
fn forced_from<I: IntoIterator<Item = String>>(args: I, env: Option<&str>) -> bool {
    args.into_iter().skip(1).any(|a| a == "--web-ui") || env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

/// Command line / environment switch that forces the WebView UI. Read once per run.
pub fn web_ui_forced() -> bool {
    static FORCED: OnceLock<bool> = OnceLock::new();
    *FORCED.get_or_init(|| forced_from(std::env::args(), std::env::var("PILL_WEB_UI").ok().as_deref()))
}

/// `--native` among the arguments (program name first) or PILL_NATIVE=1 / true.
fn native_all_from<I: IntoIterator<Item = String>>(args: I, env: Option<&str>) -> bool {
    args.into_iter().skip(1).any(|a| a == "--native") || env.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true"))
}

/// Command line / environment switch that turns every native surface on for this run. Read once per run.
pub fn native_all_requested() -> bool {
    static ALL: OnceLock<bool> = OnceLock::new();
    *ALL.get_or_init(|| native_all_from(std::env::args(), std::env::var("PILL_NATIVE").ok().as_deref()))
}

/// The one rule: native when (the key is on OR `--native`) AND the WebView UI is not forced.
fn resolve3(setting: bool, all: bool, forced: bool) -> bool {
    (setting || all) && !forced
}

#[cfg(test)]
fn resolve(setting: bool, forced: bool) -> bool {
    resolve3(setting, false, forced)
}

fn flag(surface: &str) -> bool {
    resolve3(settings::get_bool(&format!("ui.native.{surface}"), false), native_all_requested(), web_ui_forced())
}

/// Any of the four native surfaces is on in this run.
pub fn any_native() -> bool {
    native_pill() || native_bar() || native_settings() || native_capture()
}

pub fn native_pill() -> bool {
    flag("pill")
}

pub fn native_bar() -> bool {
    flag("bar")
}

pub fn native_settings() -> bool {
    flag("settings")
}

pub fn native_capture() -> bool {
    flag("capture")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn web_ui_switch() {
        assert!(!forced_from(args(&["pill.exe"]), None));
        assert!(forced_from(args(&["pill.exe", "--web-ui"]), None));
        assert!(forced_from(args(&["pill.exe", "--open", "agents", "--web-ui"]), None));
        assert!(!forced_from(args(&["--web-ui"]), None), "the first argument is the program, not a switch");
        assert!(!forced_from(args(&["pill.exe", "--web-ui-x"]), None));
        assert!(forced_from(args(&["pill.exe"]), Some("1")));
        assert!(forced_from(args(&["pill.exe"]), Some(" TRUE ")));
        assert!(!forced_from(args(&["pill.exe"]), Some("0")));
        assert!(!forced_from(args(&["pill.exe"]), Some("")));
    }

    #[test]
    fn native_switch() {
        assert!(!native_all_from(args(&["pill.exe"]), None));
        assert!(native_all_from(args(&["pill.exe", "--native"]), None));
        assert!(native_all_from(args(&["pill.exe", "--open", "agents", "--native"]), None));
        assert!(!native_all_from(args(&["--native"]), None), "the first argument is the program, not a switch");
        assert!(!native_all_from(args(&["pill.exe", "--native-x"]), None));
        assert!(!native_all_from(args(&["pill.exe", "--web-ui"]), None));
        assert!(native_all_from(args(&["pill.exe"]), Some("1")));
        assert!(native_all_from(args(&["pill.exe"]), Some(" True ")));
        assert!(!native_all_from(args(&["pill.exe"]), Some("0")));
        assert!(!native_all_from(args(&["pill.exe"]), Some("")));
    }

    #[test]
    fn native_all_turns_the_keys_on_and_web_ui_still_wins() {
        assert!(resolve3(false, true, false), "--native without a key");
        assert!(resolve3(true, false, false), "the key alone");
        assert!(!resolve3(false, false, false), "nothing asked");
        assert!(!resolve3(true, true, true), "--web-ui beats both");
        assert!(!resolve3(false, true, true), "--native --web-ui is the WebView UI");
    }

    #[test]
    fn forced_web_ui_beats_the_setting() {
        assert!(resolve(true, false));
        assert!(!resolve(true, true));
        assert!(!resolve(false, false));
        assert!(!resolve(false, true));
    }
}
