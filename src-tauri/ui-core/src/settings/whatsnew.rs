//! "What's new" after an update: reads the CHANGELOG (embedded at build time, no network), decides when the card shows, and
//! builds the card model. Port of `src-tauri/src/whatsnew.rs` + `whatsnew-live.js`. `lastSeenVersion` (settings.json, never
//! exported) remembers the version the user has been told about.
use crate::settings::calls::BackendCall;
use crate::settings::tr::{tr, Lang, Tr};
use serde_json::json;

pub const MAX_NOTES: usize = 6;
pub const MAX_LINE: usize = 160;
/// The card stays up this long.
pub const CARD_TTL_MS: u32 = 14_000;
/// The check runs this long after the island settled (it is idempotent, so a reload cannot show the card twice).
pub const CHECK_DELAY_MS: u32 = 4_000;

/// The top-level bullets of the `## [version]` section, markdown marks dropped, each capped. None: no such section.
pub fn notes_for(changelog: &str, version: &str) -> Option<Vec<String>> {
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
pub fn should_show(stored: &str, running: &str, enabled: bool) -> bool {
    enabled && !stored.is_empty() && stored != running
}

/// Result of `whatsnew_check`: the version to store and, when an update happened, the card.
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    /// Write `lastSeenVersion = running` when it differs from what was stored.
    pub store: Option<String>,
    pub card: Option<Card>,
}

pub fn check(stored: &str, running: &str, enabled: bool, changelog: &str) -> Check {
    Check {
        store: (stored != running).then(|| running.to_string()),
        card: should_show(stored, running, enabled).then(|| Card { version: running.to_string(), notes: notes_for(changelog, running).unwrap_or_default() }),
    }
}

/// The toast: "Pill 0.3.8" with the notes joined by " · " (or "Pill was updated"), buttons Close and All changes.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub version: String,
    pub notes: Vec<String>,
}

pub const UPDATED: Tr = tr("Приложение обновлено", "Pill was updated");
pub const CLOSE: Tr = tr("Закрыть", "Close");
pub const ALL_CHANGES: Tr = tr("Все изменения", "All changes");

impl Card {
    pub fn title(&self) -> String {
        format!("Pill {}", self.version)
    }
    pub fn sub(&self, lang: Lang) -> String {
        if self.notes.is_empty() {
            UPDATED.get(lang).to_string()
        } else {
            self.notes.join(" · ")
        }
    }
    /// "All changes" opens Settings > Updates.
    pub fn all_changes_call() -> BackendCall {
        BackendCall::SettingsOpen { page: Some("updates".into()) }
    }
}

/// The static "What is new" disclosure on the Updates page.
pub const PAGE_NOTES: [Tr; 1] = [tr(
    "Отдельное окно настроек, как у оригинала: персонаж, общее, клавиши, ИИ, погода, звук, живые события, буфер, модули.",
    "A separate settings window: character, general, hotkeys, AI, weather, sound, live events, clipboard, modules.",
)];

/// The `whatsNew` switch lives at this key (default on).
pub const SWITCH_KEY: &str = "whatsNew";

/// The stored version key.
pub fn store_call(version: &str) -> BackendCall {
    BackendCall::put("lastSeenVersion", json!(version))
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

    #[test]
    fn check_stores_the_version_and_builds_the_card() {
        let c = check("0.3.6", "0.3.7", true, LOG);
        assert_eq!(c.store.as_deref(), Some("0.3.7"));
        let card = c.card.unwrap();
        assert_eq!((card.title().as_str(), card.notes.len()), ("Pill 0.3.7", 3));
        assert_eq!(card.sub(Lang::En), "The Path button is shorter. · Second note · Third note");
        let fresh = check("", "0.3.7", true, LOG);
        assert!(fresh.card.is_none() && fresh.store.is_some(), "a fresh install stores the version without a card");
        let same = check("0.3.7", "0.3.7", true, LOG);
        assert!(same.card.is_none() && same.store.is_none());
        let off = check("0.3.6", "0.3.7", false, LOG);
        assert!(off.card.is_none() && off.store.is_some(), "the version is remembered even when the card is off");
        let empty = check("0.3.6", "9.9.9", true, LOG).card.unwrap();
        assert_eq!(empty.sub(Lang::Ru), "Приложение обновлено");
        assert_eq!(Card::all_changes_call(), BackendCall::SettingsOpen { page: Some("updates".into()) });
        assert_eq!(store_call("1.0.0").command(), "settings_set");
    }

    #[test]
    fn the_real_changelog_has_notes_for_the_current_version() {
        // CHANGELOG.md is the one the app embeds; the newest numbered section must yield at least one note
        let log = include_str!("../../../../CHANGELOG.md");
        let v = log.lines().find_map(|l| l.strip_prefix("## [").and_then(|r| r.split(']').next()).filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))).expect("a versioned section");
        assert!(!notes_for(log, v).unwrap().is_empty(), "{v}");
    }
}
