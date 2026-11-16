// Global hotkeys about the island itself (run from hotkeys.rs): the dark / light theme toggle and the silent cycle of the pill's music
// view (both only write a setting; the island follows `settings:changed` by itself, and no card is shown), and keyboard mode (`keys`).
use crate::ui::a11y::target::{self, Target};
use pill_ui_core::shell::tint;
use serde_json::{json, Map};
use tauri::AppHandle;

/// Hotkey "keys" (default Win+Alt+J): keyboard control of the island card. First press: the card opens and the keys are its (Tab / arrows move
/// the focus ring, Enter and Space act, Esc closes and returns the focus); second press: the music widget takes the keys, when it is on screen;
/// third: off. The views do the rest: they listen to `a11y:keys` (see `ui/a11y/target.rs`, `island/view_kbd.rs`, `musicwidget/view.rs`).
pub fn keyboard_mode(app: &AppHandle) {
    let (now, up) = (target::active(), target::widget_up());
    let next = target::next(now, up);
    if next == Some(Target::Island) {
        crate::pill::expand(app, None, "hotkey");
    }
    let say = move |t: Option<Target>| {
        let _ = crate::ui::bus::emit(target::EVENT, json!({ "target": Target::name(t) }));
    };
    if now.is_some() && next.is_some() {
        // the keyboard changes hands: the old owner gives it back first (so the user's window is the one the new owner remembers), then the new one takes it
        say(None);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(140));
            say(next);
        });
    } else {
        say(next);
    }
}

/// Next value of `musicPill`: title -> mini -> lyrics -> title. Lyrics take part only when `lyrics_on` (the lyrics feature is chosen, or
/// the view already is lyrics); an unknown value starts the round at the title.
pub fn next_music_view(cur: &str, lyrics_on: bool) -> &'static str {
    let ring: &[&str] = if lyrics_on || cur == "lyrics" { &["title", "mini", "lyrics"] } else { &["title", "mini"] };
    let next = ring.iter().position(|v| *v == cur).map_or(0, |i| (i + 1) % ring.len());
    ring[next]
}

/// Hotkey "theme": dark <-> light. A chosen island colour decides the text by itself, so it is cleared by the toggle.
pub fn toggle_theme(app: &AppHandle) {
    let color = [crate::settings::get_str("island.colorCustom", ""), crate::settings::get_str("island.color", "")].iter().find_map(|s| tint::parse_hex(s));
    let (theme, clear) = tint::toggled_theme(&crate::settings::get_str("theme", "dark"), color, crate::ui::island::sys::os_prefers_light());
    let mut patch = Map::new();
    patch.insert("theme".into(), json!(theme));
    if clear {
        patch.insert("island.color".into(), json!(""));
        patch.insert("island.colorCustom".into(), json!(""));
    }
    crate::settings::settings_set(app.clone(), patch);
}

/// Hotkey "musicView": only while something plays; nothing happens (and nothing is shown) otherwise.
pub fn cycle_music_view(app: &AppHandle) {
    if !crate::media::playing() {
        return;
    }
    let next = next_music_view(&crate::settings::get_str("musicPill", "title"), crate::settings::get_bool("lyrics.enabled", false));
    let mut patch = Map::new();
    patch.insert("musicPill".into(), json!(next));
    crate::settings::settings_set(app.clone(), patch);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_music_view_goes_round() {
        assert_eq!((next_music_view("title", false), next_music_view("mini", false), next_music_view("lyrics", false)), ("mini", "title", "title"));
        assert_eq!((next_music_view("title", true), next_music_view("mini", true), next_music_view("lyrics", true)), ("mini", "lyrics", "title"));
        assert_eq!(next_music_view("standard", true), "title", "an unknown view starts the round");
    }
}
