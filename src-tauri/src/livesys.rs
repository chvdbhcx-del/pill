// System live events: volume HUD, microphone/camera in use, downloads. New file of the Pill app, used by main.rs + src/live-system.js;
// user request: "делай все оставшееся" (live-events producers for volume, mic/camera, downloads). Owner: live-events lane.
//  - live:volume   {level 0..100, muted}            IAudioEndpointVolume change callback on the default render device. Registered only while
//                                                    Settings live.volume AND (module.media OR module.controls) are on; unregistered when off.
//  - live:privacy  {mic, cam, apps:[exe names]}     poll (1.5 s) of HKCU ...\CapabilityAccessManager\ConsentStore\{microphone,webcam} (read-only).
//                                                    Only while live.micCam is on and the pill is visible. No audio/video is ever opened or read.
//  - live:download {state: progress|done|gone, id, name, bytes, total:null, path}
//                                                    1 s poll of the Downloads folder (known folder API) for browser temp files. Only while live.downloads
//                                                    is on and the pill is visible. total is always null (browsers do not publish it).
// One supervisor thread (500 ms base tick); when every switch is off it only reads three booleans per tick.
use crate::ui::bus;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle};
use windows::core::{implement, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows::Win32::Media::Audio::Endpoints::{
    IAudioEndpointVolume, IAudioEndpointVolumeCallback, IAudioEndpointVolumeCallback_Impl,
};
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, AUDIO_VOLUME_NOTIFICATION_DATA};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::ProcessStatus::EnumProcesses;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_QWORD, REG_VALUE_TYPE,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{SHGetKnownFolderPath, FOLDERID_Downloads, KF_FLAG_DEFAULT};

static HIDDEN: AtomicBool = AtomicBool::new(false);
static LAST_PRIV: Mutex<Option<(bool, bool, Vec<String>)>> = Mutex::new(None);
static LAST_VOL: Mutex<(u32, bool)> = Mutex::new((u32::MAX, false));

// ================= pure parts (unit-tested) =================

/// Endpoint scalar 0.0..1.0 -> whole percent 0..100 (NaN and out-of-range values are clamped).
fn scalar_to_pct(v: f32) -> u32 {
    if v.is_nan() {
        return 0;
    }
    (v.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// ConsentStore value pair: in use when it was started and never stopped. Missing values = not in use.
fn consent_active(start: Option<u64>, stop: Option<u64>) -> bool {
    start.unwrap_or(0) != 0 && stop == Some(0)
}

/// NonPackaged subkey names are file paths with `\` replaced by `#`: "C:#Program Files#X#x.exe" -> "C:\Program Files\X\x.exe".
fn decode_path(sub: &str) -> String {
    sub.replace('#', "\\")
}

fn exe_of(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// Packaged app key "Microsoft.WindowsCamera_8wekyb3d8bbwe" -> "Microsoft.WindowsCamera".
fn pkg_name(sub: &str) -> String {
    sub.split('_').next().unwrap_or(sub).to_string()
}

/// A browser temp file in the Downloads folder. `final_name` is the name the file will get when the download ends, if the temp name says so.
#[derive(Debug, PartialEq)]
struct TempInfo {
    final_name: Option<String>,
}

const TEMP_EXT: &[&str] = &[".crdownload", ".part", ".partial", ".download", ".tmp"];

fn temp_info(name: &str) -> Option<TempInfo> {
    let low = name.to_lowercase();
    let ext = TEMP_EXT.iter().find(|e| low.ends_with(**e))?;
    let stem = &name[..name.len() - ext.len()];
    if stem.is_empty() {
        return None;
    }
    // "Unconfirmed 123.crdownload" and every .tmp say nothing about the final name; "report.pdf.crdownload" / "setup.exe.part" do.
    let named = *ext != ".tmp" && !stem.to_lowercase().starts_with("unconfirmed ") && stem.rfind('.').is_some_and(|i| i > 0 && i + 1 < stem.len());
    Some(TempInfo { final_name: named.then(|| stem.to_string()) })
}

#[derive(Debug, Clone, PartialEq)]
struct Cand {
    name: String,
    age_ms: u64,
    is_new: bool,
}

/// Which file is the finished download of a vanished temp file. A candidate is a non-temp file modified within the last ~20 s.
/// The expected final name wins (even if it is not new); else the freshest new file.
fn choose_done(expected: Option<&str>, cands: &[Cand]) -> Option<usize> {
    if let Some(e) = expected {
        if let Some(i) = cands.iter().position(|c| c.name.eq_ignore_ascii_case(e)) {
            return Some(i);
        }
    }
    cands.iter().enumerate().filter(|(_, c)| c.is_new).min_by_key(|(_, c)| c.age_ms).map(|(i, _)| i)
}

/// `p` is an existing file strictly inside `dir` (both canonicalised, so `..` and links do not help).
fn is_inside(dir: &Path, p: &Path) -> bool {
    let (Ok(d), Ok(f)) = (std::fs::canonicalize(dir), std::fs::canonicalize(p)) else { return false };
    f.is_file() && f.starts_with(&d) && f != d
}

// ================= volume =================

#[implement(IAudioEndpointVolumeCallback)]
struct VolCb {}

impl IAudioEndpointVolumeCallback_Impl for VolCb_Impl {
    fn OnNotify(&self, p: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> windows::core::Result<()> {
        if p.is_null() {
            return Ok(());
        }
        let d = unsafe { &*p };
        let cur = (scalar_to_pct(d.fMasterVolume), d.bMuted.as_bool());
        {
            let mut last = LAST_VOL.lock().unwrap_or_else(|e| e.into_inner());
            if *last == cur {
                return Ok(());
            }
            *last = cur;
        }
        let _ = bus::emit("live:volume", json!({ "level": cur.0, "muted": cur.1 }));
        Ok(())
    }
}

struct VolReg {
    ep: IAudioEndpointVolume,
    cb: IAudioEndpointVolumeCallback,
    dev_id: String,
}

impl Drop for VolReg {
    fn drop(&mut self) {
        unsafe {
            let _ = self.ep.UnregisterControlChangeNotify(&self.cb);
        }
    }
}

fn default_render_id() -> Option<String> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let id = dev.GetId().ok()?;
        let s = id.to_string().ok();
        CoTaskMemFree(Some(id.0 as *const c_void));
        s
    }
}

fn vol_register(_app: &AppHandle) -> Option<VolReg> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let id = dev.GetId().ok()?;
        let dev_id = id.to_string().unwrap_or_default();
        CoTaskMemFree(Some(id.0 as *const c_void));
        let ep = dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()?;
        // baseline, so the first notification after registering is compared with the real current value (no HUD at startup)
        if let (Ok(v), Ok(m)) = (ep.GetMasterVolumeLevelScalar(), ep.GetMute()) {
            *LAST_VOL.lock().unwrap_or_else(|e| e.into_inner()) = (scalar_to_pct(v), m.as_bool());
        }
        let cb: IAudioEndpointVolumeCallback = VolCb {}.into();
        ep.RegisterControlChangeNotify(&cb).ok()?;
        Some(VolReg { ep, cb, dev_id })
    }
}

fn vol_wanted() -> bool {
    use crate::settings::get_bool;
    get_bool("live.volume", true) && (get_bool("module.media", true) || get_bool("module.controls", true))
}

// ================= microphone / camera =================

const CONSENT: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn reg_open(parent: HKEY, sub: &str) -> Option<Key> {
    let w = wide(sub);
    let mut h = HKEY::default();
    let r = unsafe { RegOpenKeyExW(parent, PCWSTR(w.as_ptr()), 0, KEY_READ, &mut h) };
    (r == ERROR_SUCCESS).then_some(Key(h))
}

fn reg_subkeys(k: &Key) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..512u32 {
        let mut buf = [0u16; 300];
        let mut n = buf.len() as u32;
        let r = unsafe { RegEnumKeyExW(k.0, i, PWSTR(buf.as_mut_ptr()), &mut n, None, PWSTR::null(), None, None) };
        if r != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..n as usize]));
    }
    out
}

fn reg_qword(k: &Key, name: &str) -> Option<u64> {
    let w = wide(name);
    let (mut data, mut n, mut ty) = ([0u8; 8], 8u32, REG_VALUE_TYPE::default());
    let r = unsafe { RegQueryValueExW(k.0, PCWSTR(w.as_ptr()), None, Some(&mut ty), Some(data.as_mut_ptr()), Some(&mut n)) };
    (r == ERROR_SUCCESS && ty == REG_QWORD && n == 8).then(|| u64::from_le_bytes(data))
}

fn key_active(k: &Key) -> bool {
    consent_active(reg_qword(k, "LastUsedTimeStart"), reg_qword(k, "LastUsedTimeStop"))
}

/// Lower-case full image paths of the running processes. Empty when the list cannot be read (then the registry is trusted).
fn running_images() -> HashSet<String> {
    let mut out = HashSet::new();
    let mut pids = vec![0u32; 4096];
    let mut need = 0u32;
    unsafe {
        if EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut need).is_err() {
            return out;
        }
        for &pid in pids.iter().take(need as usize / 4).filter(|p| **p != 0) {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
            let mut buf = [0u16; 520];
            let mut n = buf.len() as u32;
            if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok() {
                out.insert(String::from_utf16_lossy(&buf[..n as usize]).to_lowercase());
            }
            let _ = CloseHandle(h);
        }
    }
    out
}

/// Apps that use `kind` ("microphone" | "webcam") right now. A desktop (NonPackaged) entry whose exe is no longer running is a stale
/// leftover of a crashed app (its Stop value never got written) and is ignored.
fn consent_apps(kind: &str, running: &mut Option<HashSet<String>>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(root) = reg_open(HKEY_CURRENT_USER, &format!("{CONSENT}\\{kind}")) else { return out };
    for name in reg_subkeys(&root) {
        if name == "NonPackaged" {
            let Some(np) = reg_open(root.0, &name) else { continue };
            for s in reg_subkeys(&np) {
                let Some(k) = reg_open(np.0, &s) else { continue };
                if !key_active(&k) {
                    continue;
                }
                let path = decode_path(&s);
                let run = running.get_or_insert_with(running_images);
                if run.is_empty() || run.contains(&path.to_lowercase()) {
                    out.push(exe_of(&path));
                }
            }
        } else if reg_open(root.0, &name).is_some_and(|k| key_active(&k)) {
            out.push(pkg_name(&name));
        }
    }
    out
}

fn poll_privacy(app: &AppHandle) {
    let mut running = None;
    let mic = consent_apps("microphone", &mut running);
    let cam = consent_apps("webcam", &mut running);
    let mut apps: Vec<String> = mic.iter().chain(cam.iter()).cloned().collect();
    apps.sort();
    apps.dedup();
    apps.truncate(4);
    set_privacy(app, (!mic.is_empty(), !cam.is_empty(), apps));
}

fn set_privacy(_app: &AppHandle, now: (bool, bool, Vec<String>)) {
    let mut last = LAST_PRIV.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref() == Some(&now) || (last.is_none() && !now.0 && !now.1) {
        return;
    }
    let _ = bus::emit("live:privacy", json!({ "mic": now.0, "cam": now.1, "apps": now.2 }));
    *last = Some(now);
}

// ================= downloads =================

pub(crate) fn downloads_dir() -> PathBuf {
    unsafe {
        if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, HANDLE::default()) {
            let s = p.to_string().ok();
            CoTaskMemFree(Some(p.0 as *const c_void));
            if let Some(s) = s.filter(|s| !s.is_empty()) {
                return PathBuf::from(s);
            }
        }
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Downloads")
}

const STALE: Duration = Duration::from_secs(15); // a temp file that stopped growing this long ago is not shown any more
const FRESH_MS: u64 = 20_000; // the finished file was written at most this long ago

struct Temp {
    size: u64,
    last_change: Instant,
    shown: bool, // seen growing
    fresh: bool, // appeared after the first poll (not a leftover from an earlier session)
    final_name: Option<String>,
}

#[derive(Default)]
struct Dl {
    temps: HashMap<String, Temp>,
    known: HashSet<String>,
    primed: bool,
}

impl Dl {
    fn reset(&mut self) {
        *self = Dl::default();
    }

    fn poll(&mut self, _app: &AppHandle) {
        let dir = downloads_dir();
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut names: HashSet<String> = HashSet::new();
        let mut live_temps: HashMap<String, (u64, Option<String>)> = HashMap::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            match temp_info(&name) {
                Some(ti) => {
                    if let Ok(m) = e.metadata() {
                        if m.is_file() {
                            live_temps.insert(name, (m.len(), ti.final_name));
                        }
                    }
                }
                None => {
                    names.insert(name);
                }
            }
        }
        let now = Instant::now();
        let prime = !self.primed;
        for (name, (size, final_name)) in &live_temps {
            let id = name.to_lowercase();
            let t = self.temps.entry(id.clone()).or_insert_with(|| Temp { size: *size, last_change: now, shown: false, fresh: !prime, final_name: final_name.clone() });
            if *size != t.size {
                t.size = *size;
                t.last_change = now;
                t.shown = true;
            }
            if t.shown && now.duration_since(t.last_change) < STALE {
                let shown_name = t.final_name.clone().unwrap_or_default();
                let _ = bus::emit("live:download", json!({
                    "state": "progress", "id": id, "name": shown_name, "bytes": t.size, "total": Value::Null,
                    "path": dir.join(name).to_string_lossy(),
                }));
            }
        }
        // vanished temp files: finished (renamed) or cancelled (deleted)
        let gone: Vec<String> = self.temps.keys().filter(|k| !live_temps.keys().any(|n| n.to_lowercase() == **k)).cloned().collect();
        let mut cands: Option<Vec<Cand>> = None;
        for id in gone {
            let Some(t) = self.temps.remove(&id) else { continue };
            if !(t.shown || t.fresh) {
                continue;
            }
            let list = cands.get_or_insert_with(|| {
                names
                    .iter()
                    .filter_map(|n| {
                        let age = std::fs::metadata(dir.join(n)).ok().filter(|m| m.is_file())?.modified().ok()?;
                        let age_ms = SystemTime::now().duration_since(age).unwrap_or_default().as_millis() as u64;
                        (age_ms <= FRESH_MS).then(|| Cand { name: n.clone(), age_ms, is_new: !self.known.contains(n) })
                    })
                    .collect()
            });
            match choose_done(t.final_name.as_deref(), list) {
                Some(i) => {
                    let c = list.remove(i);
                    let p = dir.join(&c.name);
                    let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(t.size);
                    let _ = bus::emit("live:download", json!({
                        "state": "done", "id": id, "name": c.name, "bytes": bytes, "total": Value::Null, "path": p.to_string_lossy(),
                    }));
                }
                None => {
                    let _ = bus::emit("live:download", json!({ "state": "gone", "id": id }));
                }
            }
        }
        self.known = names;
        self.primed = true;
    }
}

// ================= supervisor =================

fn run(app: AppHandle) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut vol: Option<VolReg> = None;
    let mut vol_retry = 0u64;
    let mut dl = Dl::default();
    let mut tick = 0u64;
    loop {
        std::thread::sleep(Duration::from_millis(500));
        tick += 1;
        let hidden = HIDDEN.load(Ordering::Relaxed);
        if tick % 2 == 0 {
            if vol_wanted() {
                if vol.is_none() && tick >= vol_retry {
                    vol = vol_register(&app);
                    if vol.is_none() {
                        vol_retry = tick + 10; // no audio device yet: look again in 5 s
                    }
                } else if vol.as_ref().is_some_and(|v| tick % 6 == 0 && default_render_id().is_some_and(|id| id != v.dev_id)) {
                    vol = None; // the default output changed: follow it
                    vol_retry = 0;
                }
            } else if vol.is_some() {
                vol = None; // Drop unregisters the callback
            }
            if crate::settings::get_bool("live.downloads", true) {
                if !hidden {
                    dl.poll(&app);
                }
            } else {
                dl.reset();
            }
        }
        if tick % 3 == 0 {
            if crate::settings::get_bool("live.micCam", true) {
                if !hidden {
                    poll_privacy(&app);
                }
            } else {
                set_privacy(&app, (false, false, Vec::new())); // clears the dots once, then does nothing
            }
        }
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    std::thread::spawn(move || run(h));
}

// ================= commands =================

/// The page tells whether the pill is visible (pill:show / pill:hide): the mic/camera and Downloads polls pause while it is hidden.
pub fn livesys_visible(on: bool) {
    HIDDEN.store(!on, Ordering::Relaxed);
}

/// Last known privacy state for a page that loaded after the event: {mic, cam, apps}.
pub fn livesys_privacy() -> Value {
    let l = LAST_PRIV.lock().unwrap_or_else(|e| e.into_inner());
    match l.as_ref() {
        Some((m, c, a)) => json!({ "mic": m, "cam": c, "apps": a }),
        None => json!({ "mic": false, "cam": false, "apps": [] }),
    }
}

/// Explorer with the finished download selected. Only a file inside the Downloads folder is accepted.
pub fn livesys_reveal(path: String) -> bool {
    use std::os::windows::process::CommandExt;
    let p = PathBuf::from(&path);
    if !is_inside(&downloads_dir(), &p) {
        return false;
    }
    let Ok(c) = std::fs::canonicalize(&p) else { return false };
    let s = c.to_string_lossy();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
    // one argv element for explorer.exe (no cmd.exe, no shell string); CREATE_NO_WINDOW
    std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{s}\"")).creation_flags(0x0800_0000).spawn().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_scalar() {
        assert_eq!(scalar_to_pct(0.0), 0);
        assert_eq!(scalar_to_pct(0.68), 68);
        assert_eq!(scalar_to_pct(0.675), 68);
        assert_eq!(scalar_to_pct(1.0), 100);
        assert_eq!(scalar_to_pct(1.7), 100);
        assert_eq!(scalar_to_pct(-0.2), 0);
        assert_eq!(scalar_to_pct(f32::NAN), 0);
    }

    #[test]
    fn consent_values() {
        assert!(consent_active(Some(133_000_000_000_000_000), Some(0)));
        assert!(!consent_active(Some(133_000_000_000_000_000), Some(133_000_000_000_000_100))); // stopped
        assert!(!consent_active(None, Some(0))); // never started
        assert!(!consent_active(Some(0), Some(0)));
        assert!(!consent_active(Some(5), None)); // no Stop value: unknown, not in use
    }

    #[test]
    fn consent_names() {
        let p = decode_path("C:#Program Files#Google#Chrome#Application#chrome.exe");
        assert_eq!(p, r"C:\Program Files\Google\Chrome\Application\chrome.exe");
        assert_eq!(exe_of(&p), "chrome.exe");
        assert_eq!(pkg_name("Microsoft.WindowsCamera_8wekyb3d8bbwe"), "Microsoft.WindowsCamera");
        assert_eq!(pkg_name("plain"), "plain");
    }

    #[test]
    fn temp_names() {
        let n = |s: &str| temp_info(s).map(|t| t.final_name);
        assert_eq!(n("report.pdf.crdownload"), Some(Some("report.pdf".into())));
        assert_eq!(n("SETUP.EXE.CRDOWNLOAD"), Some(Some("SETUP.EXE".into())));
        assert_eq!(n("Unconfirmed 123456.crdownload"), Some(None));
        assert_eq!(n("movie.mkv.part"), Some(Some("movie.mkv".into())));
        assert_eq!(n("abc123.part"), Some(None)); // no extension in the stem: final name unknown
        assert_eq!(n("a1b2c3.tmp"), Some(None));
        assert_eq!(n("data.csv.tmp"), Some(None)); // .tmp never names the final file
        assert_eq!(n("photo.jpg"), None);
        assert_eq!(n("notes.txt"), None);
        assert_eq!(n(".crdownload"), None);
        assert_eq!(n("part"), None);
    }

    #[test]
    fn done_choice() {
        let c = |name: &str, age_ms, is_new| Cand { name: name.into(), age_ms, is_new };
        let list = vec![c("old.zip", 15_000, false), c("a.pdf", 4000, true), c("b.pdf", 900, true)];
        assert_eq!(choose_done(Some("a.pdf"), &list), Some(1)); // the expected name wins
        assert_eq!(choose_done(Some("OLD.zip"), &list), Some(0)); // even if not new (case-insensitive)
        assert_eq!(choose_done(None, &list), Some(2)); // freshest new file
        assert_eq!(choose_done(Some("missing.bin"), &list), Some(2));
        assert_eq!(choose_done(None, &[c("old.zip", 100, false)]), None); // nothing new: the download was cancelled
        assert_eq!(choose_done(None, &[]), None);
    }

    #[test]
    fn reveal_only_inside_downloads() {
        let base = std::env::temp_dir().join(format!("pill-livesys-{}", std::process::id()));
        let dl = base.join("Downloads");
        std::fs::create_dir_all(&dl).unwrap();
        std::fs::write(dl.join("a.txt"), b"x").unwrap();
        std::fs::write(base.join("secret.txt"), b"x").unwrap();
        assert!(is_inside(&dl, &dl.join("a.txt")));
        assert!(!is_inside(&dl, &base.join("secret.txt")));
        assert!(!is_inside(&dl, &dl.join("..").join("secret.txt")));
        assert!(!is_inside(&dl, &dl)); // the folder itself
        assert!(!is_inside(&dl, &dl.join("missing.txt")));
        let _ = std::fs::remove_dir_all(&base);
    }
}
