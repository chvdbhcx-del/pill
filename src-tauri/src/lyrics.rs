// Song lyrics from LRCLIB (https://lrclib.net, an open database). Opt-in: setting `music.lyrics`, OFF by default.
//
// What leaves the PC when it is on: title, artist, album and duration of the track that is playing right now, to lrclib.net only (net.rs, Feature::Lyrics
// enforces the switch and the host). Nothing else, and only for the track being played; a track that was looked up once is answered from the disk cache
// %LOCALAPPDATA%\Pill\lyrics (a "not found" is cached for 7 days, so a missing song is not asked again and again).
//
// Flow: media.rs calls `on_track` after each `media:changed`; a new track starts ONE worker thread (after an 800 ms debounce, so skipping through a playlist
// asks for nothing) that reads the cache or asks LRCLIB, then emits `lyrics:changed` { track, lines: [[ms, text], ..] | null } on the bus. The UI never
// blocks: it only gets the event (or asks `lyrics_state` for the current answer). Parsing and "which line is being sung" live in ui-core (live/lyrics.rs).
use crate::net::{self, Feature, Opts};
use crate::ui::bus;
use pill_ui_core::live::lyrics::{parse_lrc, track_key, Lyrics};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

const DEBOUNCE_MS: u64 = 800;
const MISS_TTL_MS: i64 = 7 * 24 * 3600 * 1000;
const CACHE_MAX_FILES: usize = 400;

#[derive(Clone, Default)]
struct Track {
    key: String,
    artist: String,
    title: String,
    album: String,
    duration_ms: u64,
}

#[derive(Default)]
struct State {
    track: Track,
    /// the payload for `track` once it is known
    answer: Option<Value>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

pub fn enabled() -> bool {
    crate::settings::get_bool("music.lyrics", false)
}

/// media.rs: the media state was read. Starts a lookup when the track changed.
pub fn on_track(has_session: bool, artist: &str, title: &str, album: &str, duration_ms: u64) {
    let t = if has_session && !title.trim().is_empty() {
        Track { key: track_key(artist, title, album, duration_ms), artist: artist.trim().into(), title: title.trim().into(), album: album.trim().into(), duration_ms }
    } else {
        Track::default()
    };
    let changed = with(|s| {
        if s.track.key == t.key {
            return false;
        }
        s.track = t.clone();
        s.answer = None;
        true
    });
    if changed && !t.key.is_empty() && enabled() {
        spawn_lookup(t);
    }
}

/// settings.rs: a patch was applied. Turning the switch on looks up the track that is playing now.
pub fn on_settings(patch: &serde_json::Map<String, Value>) {
    if !patch.contains_key("music.lyrics") {
        return;
    }
    if enabled() {
        let t = with(|s| (s.answer.is_none() && !s.track.key.is_empty()).then(|| s.track.clone()));
        if let Some(t) = t {
            spawn_lookup(t);
        }
    } else {
        with(|s| s.answer = None);
        let key = with(|s| s.track.key.clone());
        let _ = bus::emit("lyrics:changed", json!({ "track": key, "lines": null }));
    }
}

/// Command `lyrics_state`: the answer for the track that is playing (`{track, lines}`), `null` while unknown or when the setting is off.
pub fn lyrics_state() -> Value {
    if !enabled() {
        return Value::Null;
    }
    with(|s| s.answer.clone()).unwrap_or(Value::Null)
}

fn spawn_lookup(t: Track) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
        let current = |k: &str| with(|s| s.track.key == k) && enabled();
        if !current(&t.key) {
            return;
        }
        let lyrics = resolve(&t);
        if !current(&t.key) {
            return; // skipped on meanwhile
        }
        let payload = match &lyrics {
            Some(l) => l.to_payload(&t.key),
            None => json!({ "track": t.key, "lines": null }),
        };
        with(|s| s.answer = Some(payload.clone()));
        let _ = bus::emit("lyrics:changed", payload);
    });
}

// ---------------------------------------------------------------------------------------------------------------------- cache

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Pill").join("lyrics")
}

/// 64-bit FNV-1a of the track key: the cache file name (the key itself has separators and arbitrary text).
fn file_name(key: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}.json")
}

/// `Some(Some(lrc))` hit, `Some(None)` cached miss, `None` unknown / expired.
fn cache_read(dir: &std::path::Path, key: &str, now: i64) -> Option<Option<String>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(file_name(key))).ok()?).ok()?;
    if v.get("key")?.as_str()? != key {
        return None; // a hash collision
    }
    match v.get("lrc").and_then(Value::as_str) {
        Some(l) if !l.is_empty() => Some(Some(l.to_string())),
        _ if now - v.get("at")?.as_i64()? < MISS_TTL_MS => Some(None),
        _ => None,
    }
}

fn cache_write(dir: &std::path::Path, key: &str, lrc: Option<&str>, now: i64) {
    let _ = std::fs::create_dir_all(dir);
    let body = json!({ "key": key, "lrc": lrc.unwrap_or(""), "at": now }).to_string();
    let tmp = dir.join(format!("{}.tmp", file_name(key)));
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(file_name(key)));
    }
    prune(dir);
}

/// Keep the cache small: past CACHE_MAX_FILES the oldest quarter goes.
fn prune(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= CACHE_MAX_FILES {
        return;
    }
    files.sort();
    for (_, p) in files.into_iter().take(CACHE_MAX_FILES / 4) {
        let _ = std::fs::remove_file(p);
    }
}

// ---------------------------------------------------------------------------------------------------------------------- LRCLIB

/// The two lookups: with album and duration, then without (an album spelled differently would miss the first one).
fn urls(t: &Track) -> Vec<String> {
    let q = |extra: bool| {
        let mut u = format!("https://lrclib.net/api/get?artist_name={}&track_name={}", net::urlenc(&t.artist), net::urlenc(&t.title));
        if extra {
            u.push_str(&format!("&album_name={}", net::urlenc(&t.album)));
            if t.duration_ms > 0 {
                u.push_str(&format!("&duration={}", t.duration_ms / 1000));
            }
        }
        u
    };
    let full = !t.album.is_empty() || t.duration_ms > 0;
    if full {
        vec![q(true), q(false)]
    } else {
        vec![q(false)]
    }
}

/// `syncedLyrics` of an LRCLIB answer (`None` for plain-only / instrumental tracks).
fn synced_of(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    v.get("syncedLyrics").and_then(Value::as_str).filter(|s| !parse_lrc(s).is_empty()).map(str::to_string)
}

fn resolve(t: &Track) -> Option<Lyrics> {
    let (dir, now) = (cache_dir(), crate::store::now_ms());
    if let Some(hit) = cache_read(&dir, &t.key, now) {
        return hit.map(|l| parse_lrc(&l));
    }
    let mut definite = false; // every lookup answered (200 / 404): safe to cache a miss
    for url in urls(t) {
        let r = net::request(&Opts { feature: Feature::Lyrics, method: "GET", url: &url, headers: &[("Accept", "application/json")], body: None, max_body: 1 << 20, recv_ms: 10_000, decompress: true, abort: None }, None);
        match r {
            Ok(r) if r.status == 200 => {
                if let Some(lrc) = synced_of(&r.text()) {
                    cache_write(&dir, &t.key, Some(&lrc), now);
                    return Some(parse_lrc(&lrc));
                }
                definite = true;
            }
            Ok(r) if r.status == 404 => definite = true,
            Ok(_) | Err(_) => {
                definite = false; // rate limit, server error, no network: ask again next time
                break;
            }
        }
    }
    if definite {
        cache_write(&dir, &t.key, None, now);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!("pill-lyrics-test-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn urls_carry_album_and_duration_first_then_fall_back() {
        let t = Track { key: "k".into(), artist: "AC/DC".into(), title: "Back In Black".into(), album: "Back In Black".into(), duration_ms: 255_700 };
        let u = urls(&t);
        assert_eq!(u.len(), 2);
        assert_eq!(u[0], "https://lrclib.net/api/get?artist_name=AC%2FDC&track_name=Back%20In%20Black&album_name=Back%20In%20Black&duration=255");
        assert_eq!(u[1], "https://lrclib.net/api/get?artist_name=AC%2FDC&track_name=Back%20In%20Black");
        let bare = Track { artist: "A".into(), title: "B".into(), ..Default::default() };
        assert_eq!(urls(&bare).len(), 1);
    }

    #[test]
    fn only_synced_lyrics_count() {
        assert_eq!(synced_of(r#"{"syncedLyrics":"[00:01.00]hi","plainLyrics":"hi"}"#).as_deref(), Some("[00:01.00]hi"));
        assert_eq!(synced_of(r#"{"syncedLyrics":null,"plainLyrics":"hi"}"#), None);
        assert_eq!(synced_of(r#"{"syncedLyrics":"no stamps here"}"#), None);
        assert_eq!(synced_of("garbage"), None);
    }

    #[test]
    fn cache_keeps_hits_and_misses_and_expires_misses() {
        let d = temp();
        let (key, now) = ("a\u{1}b\u{1}\u{1}200", 1_000_000_000_000i64);
        assert_eq!(cache_read(&d, key, now), None);
        cache_write(&d, key, Some("[00:01.00]x"), now);
        assert_eq!(cache_read(&d, key, now + MISS_TTL_MS * 10), Some(Some("[00:01.00]x".into())), "a hit never expires");
        cache_write(&d, "other", None, now);
        assert_eq!(cache_read(&d, "other", now + 1000), Some(None));
        assert_eq!(cache_read(&d, "other", now + MISS_TTL_MS + 1), None, "a miss is retried after a week");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_cache_is_pruned() {
        let d = temp();
        std::fs::create_dir_all(&d).unwrap();
        for i in 0..CACHE_MAX_FILES + 5 {
            std::fs::write(d.join(format!("{i}.json")), "{}").unwrap();
        }
        prune(&d);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), CACHE_MAX_FILES + 5 - CACHE_MAX_FILES / 4);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn only_lrclib_may_be_called_and_a_track_without_title_has_no_key() {
        assert_eq!(net::allowed_hosts(Feature::Lyrics), vec!["lrclib.net".to_string()]);
        assert!(net::host_matches("lrclib.net", "lrclib.net") && !net::host_matches("lrclib.net", "evil.example"));
        assert_eq!(track_key("", "", "", 0), track_key("", "  ", "", 0));
    }
}
