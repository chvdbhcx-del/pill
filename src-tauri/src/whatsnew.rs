// "What's new" card after an update (GateGuard note: new file of the Pill app, used by main.rs + src/whatsnew-live.js; user request:
// show the notes of the new version once after an update). The notes come from CHANGELOG.md embedded at build time: no network.
// `lastSeenVersion` (settings.json, never exported) remembers the version the user has already been told about.
use serde_json::{json, Value};

const CHANGELOG: &str = include_str!("../../CHANGELOG.md");
const MAX_NOTES: usize = 6;
const MAX_LINE: usize = 160;

/// The top-level bullets of the `## [version]` section, markdown marks dropped, each capped. None: no such section.
fn notes_for(changelog: &str, version: &str) -> Option<Vec<String>> {
    let head = format!("## [{version}]");
    let mut lines = changelog.lines().skip_while(|l| !l.starts_with(&head));
    lines.next()?;
    let notes = lines
        .take_while(|l| !l.starts_with("## ["))
        .filter_map(|l| l.strip_prefix("- "))
        .map(|l| l.replace(['`', '*'], "").trim().to_string())
        .filter(|l| !l.is_empty())
        .map(|l| if l.chars().count() > MAX_LINE { l.chars().take(MAX_LINE - 1).collect::<String>() + "…" } else { l })
        .take(MAX_NOTES)
        .collect();
    Some(notes)
}

/// Show the card? Not on a fresh install (nothing stored), not for the same version, not when the switch is off.
fn should_show(stored: &str, running: &str, enabled: bool) -> bool {
    enabled && !stored.is_empty() && stored != running
}

/// Called once by the pill page a few seconds after it loaded. Stores the running version and returns
/// { version, notes } when an update happened since the last run, else null.
#[tauri::command]
pub fn whatsnew_check() -> Value {
    let running = env!("CARGO_PKG_VERSION");
    let stored = crate::settings::get_str("lastSeenVersion", "");
    if stored != running {
        crate::settings::set("lastSeenVersion", json!(running));
    }
    if should_show(&stored, running, crate::settings::get_bool("whatsNew", true)) {
        json!({ "version": running, "notes": notes_for(CHANGELOG, running).unwrap_or_default() })
    } else {
        Value::Null
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = "# Changelog\n\n## [Unreleased]\n\n### Added\n- Pending thing\n\n## [0.3.7] - 2026-10-28\n\n### Changed\n- The `Path` button is **shorter**.\n  - nested detail\n- Second note\n\n### Added\n- Third note\n\n## [0.3.6] - 2026-10-27\n\n- Old note\n";

    #[test]
    fn section_found() {
        assert_eq!(notes_for(LOG, "0.3.7").unwrap(), ["The Path button is shorter.", "Second note", "Third note"]);
        assert_eq!(notes_for(LOG, "0.3.6").unwrap(), ["Old note"]);
    }

    #[test]
    fn section_missing() {
        assert!(notes_for(LOG, "0.3.8").is_none());
        assert!(notes_for(LOG, "0.3").is_none()); // a prefix of a version is not the version
        assert_eq!(notes_for("## [1.0.0] - x\n\nno bullets\n", "1.0.0").unwrap().len(), 0);
    }

    #[test]
    fn long_notes_are_capped() {
        let long = format!("## [1.0.0]\n- {}\n{}", "ж".repeat(500), (0..20).map(|i| format!("- n{i}\n")).collect::<String>());
        let n = notes_for(&long, "1.0.0").unwrap();
        assert_eq!(n.len(), MAX_NOTES);
        assert_eq!(n[0].chars().count(), MAX_LINE);
        assert!(n[0].ends_with('…'));
    }

    #[test]
    fn card_only_after_an_update() {
        assert!(!should_show("", "0.3.8", true)); // fresh install
        assert!(!should_show("0.3.8", "0.3.8", true));
        assert!(!should_show("0.3.7", "0.3.8", false)); // switched off
        assert!(should_show("0.3.7", "0.3.8", true));
    }
}
