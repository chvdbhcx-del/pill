// Which process plays the current media session, and its Windows audio sessions.
// Used by audio.rs (per-app capture of the player's own sound) and media.rs (the Home volume strip in "player" mode).
// PRIVACY: only process names / ids and session volumes are read locally; nothing is stored or sent.
// The selection logic is pure (unit-tested); the Win32 parts are thin wrappers around it.
use std::sync::Mutex;
use windows::core::{Interface, PWSTR};
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eRender, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::Storage::Packaging::Appx::GetPackageFamilyName;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::ProcessStatus::EnumProcesses;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE,
};

/// Who the current player is: the session's app id (AUMID) and, for players that are not Windows media sessions (AIMP, Winamp),
/// the process id taken from their window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hint {
    pub app_id: String,
    pub pid: Option<u32>,
}

impl Hint {
    pub fn is_empty(&self) -> bool {
        self.app_id.is_empty() && self.pid.is_none()
    }
}

static HINT: Mutex<Hint> = Mutex::new(Hint { app_id: String::new(), pid: None });

/// media.rs calls this after every read of the player state.
pub fn set_hint(h: Hint) {
    *HINT.lock().unwrap_or_else(|e| e.into_inner()) = h;
}

pub fn hint() -> Hint {
    HINT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// ---------- pure selection ----------

/// A running process as far as the selection cares: pid, exe name without ".exe" (lower case) and package family name (lower case, may be empty).
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub exe: String,
    pub pfn: String,
}

/// Names an AUMID can stand for: "Spotify.exe" -> spotify, "Chrome" -> chrome, "com.squirrel.slack.slack" -> slack.
fn id_stems(app_id: &str) -> Vec<String> {
    let id = app_id.rsplit('!').next().unwrap_or(app_id).trim().to_lowercase();
    let whole = id.strip_suffix(".exe").unwrap_or(&id).to_string();
    let last = whole.rsplit('.').next().unwrap_or(&whole).to_string();
    let mut v = vec![whole];
    if !last.is_empty() && !v.contains(&last) {
        v.push(last);
    }
    v.retain(|s| !s.is_empty());
    v
}

/// Does the process belong to the player with this app id? A packaged app ("<family>!<app>") matches by package family name, any other by exe name.
pub fn matches(app_id: &str, p: &Proc) -> bool {
    let id = app_id.trim();
    if id.is_empty() {
        return false;
    }
    if let Some((fam, _)) = id.split_once('!') {
        if !p.pfn.is_empty() && p.pfn == fam.to_lowercase() {
            return true;
        }
    }
    !p.exe.is_empty() && id_stems(id).iter().any(|s| *s == p.exe)
}

/// Every pid whose audio sessions are the player's: the hinted pid alone for window-based players, else all processes matching the app id.
pub fn matching_pids(h: &Hint, procs: &[Proc]) -> Vec<u32> {
    if let Some(pid) = h.pid {
        return vec![pid];
    }
    procs.iter().filter(|p| matches(&h.app_id, p)).map(|p| p.pid).collect()
}

/// The pid to capture with process loopback (the target's whole process tree is recorded): the hinted pid; else among the matching processes the one
/// that currently has an active audio session; else the lowest pid (usually the first started, i.e. the root of a browser's process tree).
pub fn pick_target(h: &Hint, procs: &[Proc], active_audio: &[u32]) -> Option<u32> {
    if let Some(pid) = h.pid {
        return Some(pid);
    }
    let m = matching_pids(h, procs);
    m.iter().copied().filter(|p| active_audio.contains(p)).min().or_else(|| m.iter().copied().min())
}

/// Where the spectrum comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// WASAPI process loopback of this pid (and its children).
    Process(u32),
    /// Loopback of the default render device (the original behaviour, and the fallback).
    DefaultDevice,
}

/// Setting `audio.perApp` off, or no player process found -> the default device.
pub fn choose_source(per_app: bool, target: Option<u32>) -> Source {
    match (per_app, target) {
        (true, Some(pid)) if pid != 0 => Source::Process(pid),
        _ => Source::DefaultDevice,
    }
}

// ---------- Win32 ----------

fn stem_of(path: &str) -> String {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

fn list_procs(want_pfn: bool) -> Vec<Proc> {
    let mut ids = vec![0u32; 4096];
    let mut got = 0u32;
    if unsafe { EnumProcesses(ids.as_mut_ptr(), (ids.len() * 4) as u32, &mut got) }.is_err() {
        return Vec::new();
    }
    ids.truncate(got as usize / 4);
    let mut out = Vec::new();
    for pid in ids.into_iter().filter(|p| *p != 0) {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let mut buf = [0u16; 520];
            let mut n = buf.len() as u32;
            let exe = if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok() {
                stem_of(&String::from_utf16_lossy(&buf[..n as usize]))
            } else {
                String::new()
            };
            let mut pfn = String::new();
            if want_pfn {
                let mut b = [0u16; 160];
                let mut l = b.len() as u32;
                if GetPackageFamilyName(h, &mut l, PWSTR(b.as_mut_ptr())).0 == 0 && l > 1 {
                    pfn = String::from_utf16_lossy(&b[..l as usize - 1]).to_lowercase();
                }
            }
            let _ = CloseHandle(h);
            out.push(Proc { pid, exe, pfn });
        }
    }
    out
}

/// Calls `f(pid, active, volume)` for every audio session of every active render device (a mixer such as Sonar or Voicemeeter keeps the player's
/// session on a virtual device, so the default device alone is not enough).
fn each_session(mut f: impl FnMut(u32, bool, &ISimpleAudioVolume)) {
    crate::media::com();
    unsafe {
        let Ok(en) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return };
        let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return };
        for i in 0..col.GetCount().unwrap_or(0) {
            let Ok(dev) = col.Item(i) else { continue };
            let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(se) = mgr.GetSessionEnumerator() else { continue };
            for j in 0..se.GetCount().unwrap_or(0) {
                let Ok(c) = se.GetSession(j) else { continue };
                let (Ok(c2), Ok(vol)) = (c.cast::<IAudioSessionControl2>(), c.cast::<ISimpleAudioVolume>()) else { continue };
                let pid = c2.GetProcessId().unwrap_or(0);
                let active = c.GetState().map(|s| s == AudioSessionStateActive).unwrap_or(false);
                f(pid, active, &vol);
            }
        }
    }
}

/// The process to capture for the current player (None: no player, or no process found).
pub fn resolve_target() -> Option<u32> {
    let h = hint();
    if h.is_empty() {
        return None;
    }
    if h.pid.is_some() {
        return pick_target(&h, &[], &[]);
    }
    let procs = list_procs(h.app_id.contains('!'));
    let mut active = Vec::new();
    each_session(|pid, on, _| {
        if on {
            active.push(pid);
        }
    });
    pick_target(&h, &procs, &active)
}

/// Is the process still running?
pub fn alive(pid: u32) -> bool {
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) => {
                let r = WaitForSingleObject(h, 0);
                let _ = CloseHandle(h);
                r != WAIT_OBJECT_0
            }
            Err(_) => false,
        }
    }
}

/// The player's pids for the volume strip: the process walk is the slow part and a drag asks dozens of times a second, so a hint's answer
/// is kept for 3 s.
static PIDS: Mutex<Option<(Hint, std::time::Instant, Vec<u32>)>> = Mutex::new(None);

fn cached_pids(h: &Hint) -> Vec<u32> {
    let mut c = PIDS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((ch, at, v)) = c.as_ref() {
        if ch == h && at.elapsed() < std::time::Duration::from_secs(3) {
            return v.clone();
        }
    }
    let v = matching_pids(h, &if h.pid.is_some() { Vec::new() } else { list_procs(h.app_id.contains('!')) });
    *c = Some((h.clone(), std::time::Instant::now(), v.clone()));
    v
}

/// Runs `f` on the player's audio sessions; returns how many there were.
fn on_player_sessions(mut f: impl FnMut(&ISimpleAudioVolume)) -> usize {
    let h = hint();
    if h.is_empty() {
        return 0;
    }
    let pids = cached_pids(&h);
    let mut n = 0;
    each_session(|pid, _, v| {
        if pids.contains(&pid) {
            n += 1;
            f(v);
        }
    });
    n
}

/// Volume (0..100) and mute of the player's own audio session (the first one found), None without a session.
pub fn session_volume() -> Option<(u32, bool)> {
    let mut out = None;
    on_player_sessions(|v| {
        if out.is_none() {
            if let (Ok(l), Ok(m)) = unsafe { (v.GetMasterVolume(), v.GetMute()) } {
                out = Some(((l * 100.0).round() as u32, m.as_bool()));
            }
        }
    });
    out
}

/// Sets the player's session volume (0..100); false when the player has no audio session.
pub fn set_session_volume(v: u32) -> bool {
    on_player_sessions(|s| unsafe {
        let _ = s.SetMasterVolume(v.min(100) as f32 / 100.0, std::ptr::null());
    }) > 0
}

pub fn set_session_mute(m: bool) -> bool {
    on_player_sessions(|s| unsafe {
        let _ = s.SetMute(m, std::ptr::null());
    }) > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, exe: &str, pfn: &str) -> Proc {
        Proc { pid, exe: exe.into(), pfn: pfn.into() }
    }
    fn app(id: &str) -> Hint {
        Hint { app_id: id.into(), pid: None }
    }

    #[test]
    fn app_ids_match_their_process() {
        assert!(matches("Spotify.exe", &p(1, "spotify", "")));
        assert!(matches("Chrome", &p(1, "chrome", "")));
        assert!(matches("com.squirrel.slack.slack", &p(1, "slack", "")));
        assert!(!matches("Spotify.exe", &p(1, "chrome", "")));
        assert!(!matches("", &p(1, "", "")), "an empty id and an unnamed process are not a match");
        // packaged app: by package family name, the exe name is irrelevant
        let id = "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic";
        assert!(matches(id, &p(1, "microsoft.media.player", "microsoft.zunemusic_8wekyb3d8bbwe")));
        assert!(!matches(id, &p(1, "notepad", "microsoft.notepad_8wekyb3d8bbwe")));
    }

    #[test]
    fn target_prefers_the_process_that_makes_sound() {
        let procs = vec![p(10, "chrome", ""), p(20, "chrome", ""), p(30, "firefox", "")];
        assert_eq!(pick_target(&app("Chrome"), &procs, &[20, 30]), Some(20));
        // nothing audible yet: the lowest pid (the browser's root, whose tree holds the audio service)
        assert_eq!(pick_target(&app("Chrome"), &procs, &[]), Some(10));
        assert_eq!(pick_target(&app("Chrome"), &procs, &[30]), Some(10), "an unrelated audible process is ignored");
        assert_eq!(pick_target(&app("Tidal"), &procs, &[20]), None);
        assert_eq!(pick_target(&Hint::default(), &procs, &[20]), None);
    }

    #[test]
    fn window_players_bring_their_own_pid() {
        let h = Hint { app_id: "ext:aimp".into(), pid: Some(777) };
        assert_eq!(pick_target(&h, &[], &[]), Some(777));
        assert_eq!(matching_pids(&h, &[p(1, "aimp", "")]), vec![777]);
        assert_eq!(matching_pids(&app("Chrome"), &[p(10, "chrome", ""), p(11, "chrome", ""), p(5, "x", "")]), vec![10, 11]);
    }

    #[test]
    fn source_falls_back_to_the_default_device() {
        assert_eq!(choose_source(true, Some(42)), Source::Process(42));
        assert_eq!(choose_source(false, Some(42)), Source::DefaultDevice, "audio.perApp off");
        assert_eq!(choose_source(true, None), Source::DefaultDevice, "no media session / no process");
        assert_eq!(choose_source(true, Some(0)), Source::DefaultDevice, "pid 0 is the idle process");
    }

    #[test]
    fn exe_stem_from_a_path() {
        assert_eq!(stem_of(r"C:\Program Files\Spotify\Spotify.EXE"), "spotify");
        assert_eq!(stem_of("chrome.exe"), "chrome");
    }
}
