// Feature flags of the native-UI migration. Read-only; nothing uses them yet (later phases switch a surface to native code when its flag is on).
//   native_pill() / native_bar() / native_settings() / native_capture()   settings.json keys ui.native.pill|bar|settings|capture, default false
//   web_ui_forced()                                                       `pill.exe --web-ui` or env PILL_WEB_UI=1: the WebView UI, whatever the keys say
// A forced web UI wins: every native_* answers false while it is on, so a broken native surface can always be bypassed from the command line.
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

fn resolve(setting: bool, forced: bool) -> bool {
    setting && !forced
}

fn flag(surface: &str) -> bool {
    resolve(settings::get_bool(&format!("ui.native.{surface}"), false), web_ui_forced())
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
    fn forced_web_ui_beats_the_setting() {
        assert!(resolve(true, false));
        assert!(!resolve(true, true));
        assert!(!resolve(false, false));
        assert!(!resolve(false, true));
    }
}
