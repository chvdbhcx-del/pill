// Alert of a finished timer / reminder: (1) which sound plays and (2) the set of alerts still waiting for an answer.
//
// Sound: settings `timer.sound` = default (the Settings > Sound signal) | chime | bell | beep | sys:<name> (a Windows system sound) | file,
// `timer.soundFile` = the WAV for "file". Played through winmm PlaySound on the same path as the built-in signals; nothing is bundled.
// Waiting: a fired timer or reminder is "waiting" until Done / Snooze / OK / Again (or the Note tab does the same). While anything waits the
// island glows softly; this module only keeps the set and tells the island with `alert:wait {on}`. A waiting alert nobody answers is dropped
// after `EXPIRE_MS` so the glow can never stay on for ever.
use crate::ui::bus;
use serde_json::json;
use std::sync::Mutex;

const EXPIRE_MS: i64 = 10 * 60_000;

/// `timer.sound` values that name a Windows system sound, with the alias PlaySound knows them by.
pub const SYSTEM: &[(&str, &str)] = &[
    ("sys:notify", "Notification.Default"),
    ("sys:reminder", "Notification.Reminder"),
    ("sys:mail", "Notification.Mail"),
    ("sys:alarm", "Notification.Looping.Alarm"),
    ("sys:asterisk", "SystemAsterisk"),
    ("sys:exclamation", "SystemExclamation"),
];

#[derive(Debug, Clone, PartialEq)]
pub enum Sound {
    /// One of the synthesised signals: chime | bell | beep.
    Tone(String),
    /// A Windows system sound alias.
    System(&'static str),
    /// A WAV file chosen by the user.
    File(String),
}

/// What to play for the settings. `tone` is the Settings > Sound signal (`sound.reminderTone`), the fallback for everything unusable.
pub fn resolve(sound: &str, file: &str, tone: &str) -> Sound {
    let sound = sound.trim();
    if let Some((_, alias)) = SYSTEM.iter().find(|(k, _)| *k == sound) {
        return Sound::System(alias);
    }
    match sound {
        "chime" | "bell" | "beep" => Sound::Tone(sound.to_string()),
        "file" => {
            let f = file.trim().trim_matches('"');
            if f.to_lowercase().ends_with(".wav") { Sound::File(f.to_string()) } else { Sound::Tone(tone.to_string()) }
        }
        _ => Sound::Tone(tone.to_string()),
    }
}

fn configured() -> Sound {
    resolve(
        &crate::settings::get_str("timer.sound", "default"),
        &crate::settings::get_str("timer.soundFile", ""),
        &crate::settings::get_str("sound.reminderTone", "chime"),
    )
}

/// Plays a sound on its own thread; a newer sound replaces an older one.
pub fn play(s: Sound) {
    match s {
        Sound::Tone(t) => crate::timers::play_tone(&t),
        Sound::System(alias) => play_win(alias, true),
        Sound::File(p) => {
            // a missing / huge / unreadable file falls back to the Settings signal instead of silence
            let ok = std::fs::metadata(&p).is_ok_and(|m| m.is_file() && m.len() <= 32 << 20);
            if ok { play_win(&p, false) } else { crate::timers::play_tone(&crate::settings::get_str("sound.reminderTone", "chime")) }
        }
    }
}

fn play_win(name: &str, alias: bool) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_FILENAME, SND_NODEFAULT, SND_SYNC};
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    std::thread::spawn(move || unsafe {
        let kind = if alias { SND_ALIAS } else { SND_FILENAME };
        let _ = PlaySoundW(PCWSTR(wide.as_ptr()), HMODULE::default(), kind | SND_SYNC | SND_NODEFAULT);
    });
}

/// The signal of a timer / reminder that just went off (the on/off switch `sound.reminders` is the caller's).
pub fn play_alert() {
    play(configured());
}

/// "Проверить" in Settings > Sound: the chosen alert, or the given built-in signal.
pub fn preview(tone: Option<String>) {
    match tone {
        Some(t) => play(resolve(&crate::settings::get_str("timer.sound", "default"), &crate::settings::get_str("timer.soundFile", ""), &t)),
        None => play_alert(),
    }
}

// ---------------------------------------------------------------------------------------------- alerts waiting for an answer

/// (id, is a timer, armed at ms)
#[derive(Default)]
pub struct Waiting(Vec<(i64, bool, i64)>);

impl Waiting {
    pub fn active(&self) -> bool {
        !self.0.is_empty()
    }
    pub fn arm(&mut self, id: i64, timer: bool, now: i64) {
        self.0.retain(|e| e.0 != id);
        self.0.push((id, timer, now));
    }
    pub fn answer(&mut self, id: i64) {
        self.0.retain(|e| e.0 != id);
    }
    pub fn answer_timers(&mut self) {
        self.0.retain(|e| !e.1);
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn expire(&mut self, now: i64) {
        self.0.retain(|e| now - e.2 < EXPIRE_MS);
    }
}

static WAIT: Mutex<Waiting> = Mutex::new(Waiting(Vec::new()));

/// Runs `f` on the set and tells the island when "anything waits" flips.
fn with(f: impl FnOnce(&mut Waiting)) {
    let (was, now) = {
        let mut w = WAIT.lock().unwrap_or_else(|e| e.into_inner());
        let was = w.active();
        f(&mut w);
        (was, w.active())
    };
    if was != now {
        let _ = bus::emit("alert:wait", json!({ "on": now }));
    }
}

/// A timer (`timer`) or reminder went off and waits for Done / Snooze / OK / Again. Off with the live-event switch for reminders.
pub fn arm(id: i64, timer: bool, now: i64) {
    if crate::settings::get_bool("live.reminders", true) {
        with(|w| w.arm(id, timer, now));
    }
}

/// Done / Snooze / delete of one reminder.
pub fn answer(id: i64) {
    with(|w| w.answer(id));
}

/// A new timer started or the timer was cancelled: the finished-timer alert is answered.
pub fn answer_timers() {
    with(|w| w.answer_timers());
}

/// Called by the scheduler loop (at least every 30 s).
pub fn expire(now: i64) {
    with(|w| w.expire(now));
}

/// "OK" on a finished-timer card, "Done" on a repeating reminder (which has nothing to finish): `id` 0 = everything.
pub fn alert_ack(id: i64) {
    with(|w| if id == 0 { w.clear() } else { w.answer(id) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_choice_falls_back_to_the_settings_signal() {
        assert_eq!(resolve("default", "", "bell"), Sound::Tone("bell".into()));
        assert_eq!(resolve("", "", "chime"), Sound::Tone("chime".into()));
        assert_eq!(resolve("beep", "", "chime"), Sound::Tone("beep".into()));
        assert_eq!(resolve("sys:reminder", "", "chime"), Sound::System("Notification.Reminder"));
        assert_eq!(resolve("file", "C:\\s\\ding.WAV", "chime"), Sound::File("C:\\s\\ding.WAV".into()));
        assert_eq!(resolve("file", "\"C:\\s\\ding.wav\"", "chime"), Sound::File("C:\\s\\ding.wav".into()), "pasted with quotes");
        assert_eq!(resolve("file", "C:\\s\\song.mp3", "bell"), Sound::Tone("bell".into()), "only WAV");
        assert_eq!(resolve("file", "", "bell"), Sound::Tone("bell".into()), "no file chosen");
        assert_eq!(resolve("sys:nope", "", "chime"), Sound::Tone("chime".into()));
    }

    #[test]
    fn every_system_sound_has_a_distinct_key_and_alias() {
        for (i, (k, a)) in SYSTEM.iter().enumerate() {
            assert!(k.starts_with("sys:") && !a.is_empty());
            assert!(SYSTEM.iter().skip(i + 1).all(|(k2, a2)| k != k2 && a != a2));
        }
    }

    #[test]
    fn waits_until_answered_and_expires() {
        let mut w = Waiting::default();
        assert!(!w.active());
        w.arm(7, false, 1000);
        w.arm(9, true, 2000);
        w.answer(7);
        assert!(w.active(), "the timer still waits");
        w.answer_timers();
        assert!(!w.active());
        w.arm(7, false, 0);
        w.arm(7, false, 500); // the same reminder again: one entry
        w.answer(7);
        assert!(!w.active());
        w.arm(1, false, 0);
        w.expire(EXPIRE_MS - 1);
        assert!(w.active());
        w.expire(EXPIRE_MS);
        assert!(!w.active(), "nobody answered for 10 minutes: the glow ends");
    }
}
