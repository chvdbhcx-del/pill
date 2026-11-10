// Players that do not report to Windows' media session list: AIMP and Winamp (WACUP). They are read through their own remote interfaces
// (aimp.rs: AIMP2_RemoteInfo window + file mapping; winamp.rs: the "Winamp v1.x" window and WM_USER IPC) and merged into the same
// `media:changed` data by media.rs, so the Home player, the mini player and the music pill show them like any other player.
// A player that is not running costs one FindWindow per 5 s while the pill is open (nothing while it is hidden); a running one is read once per second.
// PRIVACY: everything is read from the local player process; nothing is stored or sent. Setting `media.legacyPlayers` (default on) switches it off.
mod aimp;
mod winamp;

use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, SendMessageTimeoutW, SMTO_ABORTIFHUNG};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Aimp,
    Winamp,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Aimp, Kind::Winamp];

    /// The session id media.rs and the page use ("ext:" cannot clash with a Windows app id).
    pub fn id(self) -> &'static str {
        match self {
            Kind::Aimp => "ext:aimp",
            Kind::Winamp => "ext:winamp",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Aimp => "AIMP",
            Kind::Winamp => "Winamp",
        }
    }

    pub fn from_id(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == id)
    }
}

/// What a running player reports.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub kind: Kind,
    pub pid: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
}

pub fn enabled() -> bool {
    crate::settings::get_bool("media.legacyPlayers", true)
}

/// Every supported player that is running right now.
pub fn read_all() -> Vec<Info> {
    if !enabled() {
        return Vec::new();
    }
    [aimp::read(), winamp::read()].into_iter().flatten().collect()
}

/// "play_pause" | "next" | "prev"; false when the player is not running or did not answer.
pub fn control(kind: Kind, action: &str) -> bool {
    match kind {
        Kind::Aimp => aimp::control(action),
        Kind::Winamp => winamp::control(action),
    }
}

pub fn seek(kind: Kind, ms: u64) -> bool {
    match kind {
        Kind::Aimp => aimp::seek(ms),
        Kind::Winamp => winamp::seek(ms),
    }
}

/// Which player the page shows: `selected` is the session the user picked (media.rs), `sys_*` describe Windows' current session,
/// `ext` the running legacy players as (kind, playing). None = Windows' own session.
/// A picked player always wins; otherwise a playing legacy player wins over a Windows session that is not playing, and any running legacy
/// player is shown when Windows has no session at all (so an idle AIMP still gets its controls).
pub fn choose(selected: Option<&str>, sys_has_session: bool, sys_playing: bool, ext: &[(Kind, bool)]) -> Option<Kind> {
    match selected.filter(|s| !s.is_empty()) {
        Some(id) => Kind::from_id(id).filter(|k| ext.iter().any(|e| e.0 == *k)),
        None if sys_playing => None,
        None => ext.iter().find(|e| e.1).or_else(|| ext.first().filter(|_| !sys_has_session)).map(|e| e.0),
    }
}

/// Did the position jump (the user dragged AIMP's seek bar) instead of advancing at normal speed? The page interpolates between events,
/// so only a jump needs a new one.
pub fn position_jumped(prev_pos: u64, elapsed_ms: u64, playing: bool, now_pos: u64) -> bool {
    let expected = prev_pos + if playing { elapsed_ms } else { 0 };
    expected.abs_diff(now_pos) > 2000
}

// ---------- Win32 helpers shared by aimp.rs and winamp.rs ----------

/// SendMessage with a time limit (a hung player must not block the poll thread or a button press).
pub(crate) fn send(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> Option<isize> {
    let mut out = 0usize;
    let r = unsafe { SendMessageTimeoutW(hwnd, msg, WPARAM(wp), LPARAM(lp), SMTO_ABORTIFHUNG, 300, Some(&mut out)) };
    (r.0 != 0).then_some(out as isize)
}

pub(crate) fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

// ---------- poll thread ----------

/// Notices changes in the running players and asks media.rs for a refresh. Sleeps on media.rs's condvar: a pill that is hidden wakes it
/// only every 30 s, a visible one every 5 s without a legacy player and every second with one.
fn poll() {
    let mut last = String::new();
    let mut pos: Option<(u64, Instant, bool)> = None;
    loop {
        if !crate::media::is_active() || !enabled() {
            last.clear();
            pos = None;
            crate::media::wait_change(Duration::from_secs(30));
            continue;
        }
        let infos = read_all();
        let mut key: String = infos
            .iter()
            .map(|i| format!("{:?}|{}|{}|{}|{}|{}", i.kind, i.title, i.artist, i.playing, i.duration_ms, i.pid))
            .collect::<Vec<_>>()
            .join(";");
        let jumped = match (&pos, infos.first()) {
            (Some((p, at, playing)), Some(i)) => position_jumped(*p, at.elapsed().as_millis() as u64, *playing, i.position_ms),
            _ => false,
        };
        pos = infos.first().map(|i| (i.position_ms, Instant::now(), i.playing));
        if jumped {
            key.push_str(&format!("|jump{}", infos[0].position_ms));
        }
        if key != last {
            last = key;
            crate::media::refresh();
        }
        crate::media::wait_change(Duration::from_secs(if infos.is_empty() { 5 } else { 1 }));
    }
}

pub fn init() {
    std::thread::spawn(poll);
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Kind = Kind::Aimp;
    const W: Kind = Kind::Winamp;

    #[test]
    fn ids_round_trip() {
        for k in Kind::ALL {
            assert_eq!(Kind::from_id(k.id()), Some(k));
        }
        assert_eq!(Kind::from_id("Spotify.exe"), None);
    }

    #[test]
    fn windows_session_wins_while_it_plays() {
        assert_eq!(choose(None, true, true, &[(A, true)]), None);
    }

    #[test]
    fn a_playing_legacy_player_beats_an_idle_session() {
        assert_eq!(choose(None, true, false, &[(A, false), (W, true)]), Some(W));
        assert_eq!(choose(None, false, false, &[(A, true)]), Some(A));
    }

    #[test]
    fn idle_legacy_player_only_when_windows_has_nothing() {
        assert_eq!(choose(None, false, false, &[(A, false)]), Some(A));
        assert_eq!(choose(None, true, false, &[(A, false)]), None, "a paused Windows session keeps the card");
        assert_eq!(choose(None, false, false, &[]), None);
    }

    #[test]
    fn a_picked_player_wins_and_a_foreign_pick_does_not() {
        assert_eq!(choose(Some("ext:winamp"), true, true, &[(A, true), (W, false)]), Some(W));
        assert_eq!(choose(Some("ext:winamp"), true, false, &[(A, true)]), None, "picked player is not running: back to Windows");
        assert_eq!(choose(Some("Spotify.exe"), false, false, &[(A, true)]), None, "the user picked a Windows app");
        assert_eq!(choose(Some(""), false, false, &[(A, true)]), Some(A), "empty pick = automatic");
    }

    #[test]
    fn a_seek_is_a_jump_normal_playback_is_not() {
        assert!(!position_jumped(10_000, 1000, true, 11_050));
        assert!(position_jumped(10_000, 1000, true, 60_000), "dragged forward");
        assert!(position_jumped(10_000, 1000, true, 2_000), "dragged back");
        assert!(!position_jumped(10_000, 5000, false, 10_300), "paused: stays");
        assert!(position_jumped(10_000, 5000, false, 15_000), "paused: but moved");
    }
}
