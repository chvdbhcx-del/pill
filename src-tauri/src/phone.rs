// Pill Phone link (PC side of docs/phone-protocol.md): a small HTTP + WebSocket server on the PC's private LAN address that the Android app talks to.
// It runs ONLY while Settings > Phone > "Phone link" (`phone.enabled`) is on; off = no thread, no socket. The server blocks on sockets (no polling).
// Mirrored notifications, file transfers and pairing live in phone/server.rs; the pieces: ws.rs (WebSocket), http.rs, reg.rs (devices, codes,
// limiter), files.rs (names, transfer table), page.rs (the QR landing page).
// Events (bus): `phone:state` {running, ip, port, pcName, devices[], connected}, `phone:notif` {kind: posted|removed|read|reply, ...},
// `phone:files` {kind: offer|start|progress|done|sent|failed|offered|accepted|rejected, transfer}.
// PRIVACY / SECURITY: tokens only as SHA-256 in %LOCALAPPDATA%\Pill\phone\devices.json; pairing codes one-use, 5 minutes; private-network peers only;
// neither tokens nor message text are ever logged; notification text is kept in memory only.
use crate::ui::bus;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

mod files;
mod http;
mod info;
mod page;
mod reg;
mod relay;
mod relay_files;
mod server;
mod v2;
mod ws;
mod wsclient;
#[cfg(test)]
mod relay_tests;
#[cfg(test)]
mod tests;

/// Bus event names (also the literals the contract test looks for).
pub const EVENTS: [&str; 3] = ["phone:state", "phone:notif", "phone:files"];
pub const DEFAULT_APK_URL: &str = "https://github.com/chvdbhcx-del/pill/releases/download/phone-latest/PillPhone.apk";
const DEFAULT_PORT: u16 = 41717;

struct Running {
    server: server::Server,
}
static SERVER: Mutex<Option<Running>> = Mutex::new(None);
static ERROR: Mutex<Option<&'static str>> = Mutex::new(None);
static SYNC: Mutex<()> = Mutex::new(()); // one start / stop at a time

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn pc_name() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "PC".into())
}

fn downloads_dir() -> PathBuf {
    let custom = crate::settings::get_str("phone.downloadDir", "");
    if !custom.trim().is_empty() {
        return PathBuf::from(custom.trim());
    }
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Downloads").join("Pill")
}

fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Pill").join("phone")
}

fn cfg() -> server::Cfg {
    let mb = crate::settings::get_f64("phone.maxFileMB", 4096.0).clamp(1.0, 1_048_576.0);
    let apk = crate::settings::get_str("phone.apkUrl", DEFAULT_APK_URL);
    server::Cfg {
        downloads: downloads_dir(),
        max_bytes: (mb * 1024.0 * 1024.0) as u64,
        auto_accept: crate::settings::get_bool("phone.autoAccept", true),
        apk_url: if apk.starts_with("https://") || apk.starts_with("http://") { apk } else { DEFAULT_APK_URL.into() },
        muted: crate::settings::get_list("phone.mutedApps"),
        pc_name: pc_name(),
        relay_on: crate::settings::get_bool("phone.relay", true),
        relay_url: crate::settings::get_str("phone.relayUrl", ""),
        relay_max_bytes: (crate::settings::get_f64("phone.relayMaxFileMB", 100.0).clamp(1.0, 4096.0) * 1024.0 * 1024.0) as u64,
    }
}

fn start() {
    let base = crate::settings::get_f64("phone.port", DEFAULT_PORT as f64).clamp(1024.0, 65000.0) as u16;
    let Some(ip) = server::detect_ip() else {
        *lock(&ERROR) = Some("no_private_ip");
        return;
    };
    for port in base..base.saturating_add(20) {
        let opts = server::Opts {
            dir: data_dir(),
            bind: ip,
            port,
            allow_loopback: false,
            cfg: Arc::new(cfg),
            emit: Arc::new(|name, v| {
                let _ = bus::emit(name, v);
            }),
        };
        if let Ok(s) = server::Server::start(opts) {
            if port != base {
                crate::settings::set("phone.port", json!(port)); // remembered: the next start tries this one first
            }
            *lock(&ERROR) = None;
            *lock(&SERVER) = Some(Running { server: s });
            return;
        }
    }
    *lock(&ERROR) = Some("port_busy");
}

/// Starts / stops the server to match `phone.enabled`. Called at start-up and on every `settings:changed`.
fn sync() {
    let _one = lock(&SYNC);
    let want = crate::settings::get_bool("phone.enabled", false);
    let mut g = lock(&SERVER);
    let have = g.is_some();
    if want && !have {
        drop(g);
        start();
        crate::phone_fw::ensure_async(); // a "Public" network blocks the phone unless Windows Firewall allows Pill on the LAN (one UAC prompt)
        let _ = bus::emit("phone:state", phone_status());
    } else if !want && have {
        let r = g.take();
        drop(g);
        crate::phone_fw::reset();
        *lock(&ERROR) = None;
        if let Some(r) = r {
            r.server.stop(); // closes every socket, frees the port
        }
        let _ = bus::emit("phone:state", phone_status());
    } else if want && have {
        // phone.relay / phone.relayUrl may have changed: connect or tear down the relay links
        let sh = g.as_ref().map(|r| r.server.shared.clone());
        drop(g);
        if let Some(sh) = sh {
            sh.relay_sync();
        }
    }
}

pub fn init(_app: &tauri::App) {
    bus::subscribe("settings:changed", |_| sync());
    sync();
}

fn with<T>(f: impl FnOnce(&server::Shared) -> T) -> Option<T> {
    lock(&SERVER).as_ref().map(|r| f(&r.server.shared))
}

fn off() -> String {
    "Phone link is off".into()
}

// ---------------------------------------------------------------- commands

pub fn phone_status() -> Value {
    let enabled = crate::settings::get_bool("phone.enabled", false);
    let mut v = with(|s| s.status()).unwrap_or_else(|| json!({ "running": false, "ip": null, "port": null, "pcName": pc_name(), "devices": [], "connected": 0 }));
    v["enabled"] = json!(enabled);
    v["error"] = json!(*lock(&ERROR));
    v
}

/// The current one-time pairing code and the URL the QR code encodes. `renew` makes a new code (the old one dies).
pub fn phone_pair_code(renew: Option<bool>) -> Result<Value, String> {
    with(|s| s.pair_code(renew.unwrap_or(false))).ok_or_else(off)
}

pub fn phone_devices() -> Vec<Value> {
    with(|s| s.devices()).unwrap_or_default()
}

pub fn phone_revoke(id: String) -> bool {
    with(|s| s.revoke(&id)).unwrap_or(false)
}

pub fn phone_notifs() -> Vec<Value> {
    with(|s| s.notifs()).unwrap_or_default()
}

/// `action`: dismiss | read | reply (`text` is the reply).
pub fn phone_notif_action(key: String, action: String, text: Option<String>) -> Result<Value, String> {
    with(|s| s.notif_action(&key, &action, text.as_deref().unwrap_or(""))).ok_or_else(off)?.map(|_| json!({ "ok": true }))
}

pub fn phone_files() -> Vec<Value> {
    with(|s| s.transfers.list().iter().map(|t| t.public()).collect()).unwrap_or_default()
}

pub fn phone_send_file(path: String, device: Option<String>) -> Result<Value, String> {
    with(|s| s.send_file(std::path::Path::new(&path), device.as_deref())).ok_or_else(off)?
}

/// Several files at once (a drop on the island). Returns how many were offered; the first error text when none was.
pub fn send_files(paths: &[String]) -> Result<usize, String> {
    let mut err = None;
    let mut n = 0;
    for p in paths {
        match phone_send_file(p.clone(), None) {
            Ok(_) => n += 1,
            Err(e) => err = Some(e),
        }
    }
    if n == 0 { Err(err.unwrap_or_else(|| "nothing to send".into())) } else { Ok(n) }
}

pub fn phone_file_decide(id: String, accept: bool) -> Result<Value, String> {
    with(|s| s.decide(&id, accept)).ok_or_else(off)?.map(|_| json!({ "ok": true }))
}

/// Packages that ever sent a notification, with their names (for the muted-apps list), and the muted ones.
pub fn phone_apps() -> Value {
    let apps = with(|s| s.registry().apps()).unwrap_or_else(|| reg::Registry::load(data_dir()).apps());
    json!({ "apps": apps, "muted": crate::settings::get_list("phone.mutedApps") })
}

/// Opens a received / sent file, or shows it in its folder (`reveal`). Programs and scripts from a phone are only ever revealed, never run.
pub fn phone_open_file(id: String, reveal: Option<bool>) -> bool {
    let Some(p) = with(|s| s.transfers.get(&id).and_then(|t| t.path)).flatten().filter(|p| p.exists()) else { return false };
    if reveal.unwrap_or(false) || files::risky_to_open(&p) {
        use std::os::windows::process::CommandExt;
        return std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", p.display())).creation_flags(0x0800_0000).spawn().is_ok();
    }
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe { ShellExecuteW(HWND::default(), &HSTRING::from("open"), &HSTRING::from(p.to_string_lossy().as_ref()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL).0 as isize > 32 }
}
