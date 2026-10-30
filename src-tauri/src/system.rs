// System controls for the Controls tab (S10 / F08): Ethernet, Wi-Fi, Bluetooth, dark theme, volume, mic mute, power. Owner: system agent (lane L5).
// Reads need no elevation. Adapter admin state needs it: done by launching `netsh` with the "runas" verb, so Windows shows UAC (the app never elevates itself).
// Power actions are two-step: the first call only arms a 3 s window in Rust, the second call inside it runs the action (see `decide`).
use crate::ui::bus;
use serde::Serialize;
use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothDevice};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation};
use windows::Win32::Foundation::{CloseHandle, BOOLEAN, HANDLE, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eCapture, eRender, EDataFlow};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Registry::{RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD};
use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, SW_HIDE, SW_SHOWNORMAL, WM_SETTINGCHANGE};

const NO_WINDOW: u32 = 0x0800_0000;
const PERSONALIZE: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
const CONFIRM_MS: u64 = 3000;
const IF_ETHERNET: u32 = 6;
const IF_WIFI: u32 = 71;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static ARMED: Mutex<Option<(String, u64)>> = Mutex::new(None);
static BT_CACHE: Mutex<Option<(Instant, Vec<BtDevice>)>> = Mutex::new(None);

#[link(name = "user32")]
extern "system" {
    fn LockWorkStation() -> i32;
}

pub fn init(app: &tauri::App) {
    let _ = APP.set(app.handle().clone());
}

// ---------- state ----------

#[derive(Serialize, Clone)]
pub struct Nic {
    available: bool,
    on: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ssid: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct BtDevice {
    name: String,
    connected: bool,
    // ponytail: no battery: reading it needs additionalProperties (IIterable) which windows 0.58 cannot build without an extra crate.
}

#[derive(Serialize, Clone)]
pub struct Bt {
    available: bool,
    on: bool,
    devices: Vec<BtDevice>,
}

#[derive(Serialize, Clone)]
pub struct SysState {
    ethernet: Nic,
    wifi: Nic,
    bluetooth: Bt,
    dark: bool,
    volume: u32,
    muted: bool,
    #[serde(rename = "micMuted", skip_serializing_if = "Option::is_none")]
    mic_muted: Option<bool>,
}

fn com() {
    // Ignore the result: S_FALSE / RPC_E_CHANGED_MODE just mean COM is already initialised on this thread.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

/// Volume / mute endpoint of the output (Settings > Sound `sound.volumeDevice`) or the microphone (`sound.micDevice`);
/// empty setting = the Windows default device.
fn endpoint(flow: EDataFlow) -> windows::core::Result<IAudioEndpointVolume> {
    crate::media::endpoint_for(flow, if flow == eCapture { "sound.micDevice" } else { "sound.volumeDevice" })
}

fn get_audio() -> (u32, bool) {
    endpoint(eRender)
        .and_then(|ep| unsafe { Ok(((ep.GetMasterVolumeLevelScalar()? * 100.0).round() as u32, ep.GetMute()?.as_bool())) })
        .unwrap_or((0, false))
}

fn get_mic_muted() -> Option<bool> {
    endpoint(eCapture).and_then(|ep| unsafe { Ok(ep.GetMute()?.as_bool()) }).ok()
}

/// Hotkey "microphone: mute / unmute" (hotkeys.rs). Does nothing when there is no microphone.
pub fn toggle_mic() {
    if let Some(m) = get_mic_muted() {
        let _ = endpoint(eCapture).and_then(|ep| unsafe { ep.SetMute(!m, std::ptr::null()) });
    }
}

fn get_dark() -> bool {
    // Missing value means light.
    let (mut data, mut len) = (1u32, 4u32);
    unsafe {
        let _ = RegGetValueW(HKEY_CURRENT_USER, PERSONALIZE, w!("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut data as *mut u32 as *mut _), Some(&mut len));
    }
    data == 0
}

fn set_dark(dark: bool) {
    let val: u32 = if dark { 0 } else { 1 };
    unsafe {
        for name in [w!("AppsUseLightTheme"), w!("SystemUsesLightTheme")] {
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, PERSONALIZE, name, REG_DWORD.0, Some(&val as *const u32 as *const _), 4);
        }
        let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(w!("ImmersiveColorSet").as_ptr() as isize), SMTO_ABORTIFHUNG, 2000, None);
    }
}

/// Physical adapters of one interface type as (alias, administratively up). Disabled adapters are listed too (GetIfTable2 sees all NDIS interfaces).
fn adapters(if_type: u32) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return out;
        }
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
        for r in rows {
            let f = r.InterfaceAndOperStatusFlags._bitfield;
            // bit0 HardwareInterface, bit7 EndPointInterface: skips Hyper-V / WSL / VPN virtual adapters.
            if r.Type != if_type || f & 1 == 0 || f & 0x80 != 0 {
                continue;
            }
            let end = r.Alias.iter().position(|&c| c == 0).unwrap_or(r.Alias.len());
            out.push((String::from_utf16_lossy(&r.Alias[..end]), r.AdminStatus.0 == 1));
        }
        FreeMibTable(t as *const _);
    }
    out
}

fn wifi_ssid() -> Option<String> {
    NetworkInformation::GetConnectionProfiles().ok()?.into_iter().find_map(|p| {
        let up = p.GetNetworkConnectivityLevel().ok()? != NetworkConnectivityLevel::None;
        (up && p.IsWlanConnectionProfile().unwrap_or(false)).then(|| p.ProfileName().ok().map(|s| s.to_string())).flatten()
    })
}

fn nic(if_type: u32) -> Nic {
    let a = adapters(if_type);
    let on = a.iter().any(|x| x.1);
    Nic { available: !a.is_empty(), on, ssid: if if_type == IF_WIFI && on { wifi_ssid() } else { None } }
}

fn bt_radio() -> Option<Radio> {
    com();
    Radio::GetRadiosAsync().ok()?.get().ok()?.into_iter().find(|r| r.Kind().ok() == Some(RadioKind::Bluetooth))
}

fn bt_devices() -> Vec<BtDevice> {
    if let Some((t, v)) = &*BT_CACHE.lock().unwrap_or_else(|e| e.into_inner()) {
        if t.elapsed() < Duration::from_secs(4) {
            return v.clone();
        }
    }
    com();
    let run = || -> windows::core::Result<Vec<BtDevice>> {
        let sel = BluetoothDevice::GetDeviceSelectorFromPairingState(true)?;
        let mut out = Vec::new();
        for info in DeviceInformation::FindAllAsyncAqsFilter(&sel)?.get()? {
            let connected = BluetoothDevice::FromIdAsync(&info.Id()?)
                .and_then(|o| o.get())
                .and_then(|d| d.ConnectionStatus())
                .map(|s| s == BluetoothConnectionStatus::Connected)
                .unwrap_or(false);
            out.push(BtDevice { name: info.Name()?.to_string(), connected });
        }
        out.sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    };
    let v = run().unwrap_or_default();
    *BT_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), v.clone()));
    v
}

fn build_state() -> SysState {
    let (volume, muted) = get_audio();
    let radio = bt_radio();
    let on = radio.as_ref().and_then(|r| r.State().ok()) == Some(RadioState::On);
    SysState {
        ethernet: nic(IF_ETHERNET),
        wifi: nic(IF_WIFI),
        bluetooth: Bt { available: radio.is_some(), on, devices: if radio.is_some() { bt_devices() } else { vec![] } },
        dark: get_dark(),
        volume,
        muted,
        mic_muted: get_mic_muted(),
    }
}

fn emit_state() {
    if let Some(_app) = APP.get() {
        let _ = bus::emit("sys:changed", build_state());
    }
}

pub async fn sys_state() -> SysState {
    build_state()
}

// ---------- set ----------

fn res(status: &str, message: &str) -> Value {
    json!({ "status": status, "message": message })
}

fn set_bluetooth(on: bool) -> Value {
    let Some(r) = bt_radio() else { return res("error", "no bluetooth radio") };
    if Radio::RequestAccessAsync().ok().and_then(|a| a.get().ok()) != Some(RadioAccessStatus::Allowed) {
        return res("error", "radio access denied");
    }
    let ok = r.SetStateAsync(if on { RadioState::On } else { RadioState::Off }).and_then(|op| op.get()).is_ok();
    *BT_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    if ok { res("ok", "") } else { res("error", "set state failed") }
}

/// Launch `file params` elevated (UAC). Returns the process handle as isize, or the Win32 error (1223 = the user declined UAC).
fn runas(file: &str, params: &str) -> Result<isize, u32> {
    let (f, p) = (HSTRING::from(file), HSTRING::from(params));
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: PCWSTR(p.as_ptr()),
        nShow: SW_HIDE.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei).map_err(|e| (e.code().0 as u32) & 0xFFFF)? };
    Ok(sei.hProcess.0 as isize)
}

fn wait_exit(h: isize, ms: u32) -> Option<u32> {
    let h = HANDLE(h as *mut _);
    unsafe {
        if WaitForSingleObject(h, ms).0 != 0 {
            return None; // still running (WAIT_TIMEOUT)
        }
        let mut code = 1u32;
        let _ = GetExitCodeProcess(h, &mut code);
        let _ = CloseHandle(h);
        Some(code)
    }
}

/// Enable / disable all physical adapters of a type. Names come from our own enumeration (never from the page) and are charset-checked.
fn set_nic(if_type: u32, on: bool) -> Value {
    let todo: Vec<String> = adapters(if_type).into_iter().filter(|a| a.1 != on).map(|a| a.0).collect();
    if todo.is_empty() {
        return res("ok", "");
    }
    if todo.iter().any(|n| n.is_empty() || !n.chars().all(|c| c.is_alphanumeric() || " ._()#-".contains(c))) {
        return res("error", "unsupported adapter name");
    }
    let verb = if on { "enable" } else { "disable" };
    let cmd = todo.iter().map(|n| format!("netsh interface set interface name=\"{n}\" admin={verb}")).collect::<Vec<_>>().join(" & ");
    let h = match runas("cmd.exe", &format!("/c {cmd}")) {
        Ok(h) => h,
        Err(1223) => return res("error", "UAC declined"),
        Err(e) => return res("error", &format!("launch failed ({e})")),
    };
    match wait_exit(h, 10_000) {
        Some(0) => {
            emit_state();
            res("ok", "")
        }
        Some(c) => res("error", &format!("netsh exit code {c}")),
        None => {
            // Still waiting on UAC / netsh: report "needs_uac" now, tell the page when it finishes.
            std::thread::spawn(move || {
                wait_exit(h, 120_000);
                emit_state();
            });
            res("needs_uac", "waiting for the elevated netsh")
        }
    }
}

pub async fn sys_set(key: String, value: Value) -> Value {
    let b = value.as_bool();
    let r = match (key.as_str(), b) {
        ("bluetooth", Some(on)) => set_bluetooth(on),
        ("dark", Some(on)) => {
            set_dark(on);
            res("ok", "")
        }
        ("ethernet", Some(on)) => tauri::async_runtime::spawn_blocking(move || set_nic(IF_ETHERNET, on)).await.unwrap_or_else(|_| res("error", "task failed")),
        ("wifi", Some(on)) => tauri::async_runtime::spawn_blocking(move || set_nic(IF_WIFI, on)).await.unwrap_or_else(|_| res("error", "task failed")),
        ("muted", Some(m)) => {
            crate::media::media_mute(m).await;
            res("ok", "")
        }
        ("micMuted", Some(m)) => match endpoint(eCapture).and_then(|ep| unsafe { ep.SetMute(m, std::ptr::null()) }) {
            Ok(_) => res("ok", ""),
            Err(_) => res("error", "no microphone"),
        },
        ("volume", _) => match value.as_f64() {
            Some(v) => {
                crate::media::media_volume_set(v.clamp(0.0, 100.0).round() as u32).await;
                res("ok", "")
            }
            None => res("error", "bad value"),
        },
        // Connecting a paired device has no WinRT API (it happens through RFCOMM/GATT sessions): hand over to the Settings page instead.
        ("bluetooth_settings", _) => {
            let _ = open_uri("ms-settings:bluetooth");
            res("ok", "")
        }
        _ => res("error", "unknown key or bad value"),
    };
    if r["status"] == "ok" && key != "volume" && key != "bluetooth_settings" {
        emit_state();
    }
    r
}

fn open_uri(uri: &str) -> windows::core::Result<()> {
    let f = HSTRING::from(uri);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        lpVerb: w!("open"),
        lpFile: PCWSTR(f.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei) }
}

// ---------- power ----------

#[derive(Debug, PartialEq)]
enum Decision {
    Confirm,
    Run,
}

/// Confirm-window state machine (pure, injectable clock in ms). A call only runs the action when `confirmed` is set AND the same action
/// was armed less than CONFIRM_MS ago; anything else (re)arms and asks for confirmation. The page's `confirmed` flag alone never suffices.
fn decide(armed: &mut Option<(String, u64)>, action: &str, confirmed: bool, now: u64) -> Decision {
    if confirmed {
        if let Some((a, t)) = armed.as_ref() {
            if a == action && now.saturating_sub(*t) <= CONFIRM_MS {
                *armed = None;
                return Decision::Run;
            }
        }
    }
    *armed = Some((action.to_string(), now));
    Decision::Confirm
}

fn shutdown_exe(arg: &str) -> bool {
    let a: Vec<&str> = if arg == "/l" { vec!["/l"] } else { vec![arg, "/t", "0"] };
    std::process::Command::new("shutdown.exe").args(a).creation_flags(NO_WINDOW).spawn().is_ok()
}

pub fn sys_power(action: String, confirmed: bool) -> Value {
    if !["lock", "sleep", "restart", "shutdown", "signout"].contains(&action.as_str()) {
        return res("error", "unknown action");
    }
    let now = START.get_or_init(Instant::now).elapsed().as_millis() as u64;
    if decide(&mut ARMED.lock().unwrap_or_else(|e| e.into_inner()), &action, confirmed, now) == Decision::Confirm {
        return json!({ "status": "confirm", "ms": CONFIRM_MS });
    }
    let ok = match action.as_str() {
        "lock" => unsafe { LockWorkStation() != 0 },
        // SetSuspendState blocks until resume, so it runs on its own thread.
        "sleep" => {
            std::thread::spawn(|| unsafe {
                let _ = SetSuspendState(BOOLEAN(0), BOOLEAN(0), BOOLEAN(0));
            });
            true
        }
        "restart" => shutdown_exe("/r"),
        "shutdown" => shutdown_exe("/s"),
        _ => shutdown_exe("/l"),
    };
    if ok { res("ok", "") } else { res("error", "action failed") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confirm_window() {
        let mut a = None;
        // first press only arms
        assert_eq!(decide(&mut a, "shutdown", false, 1000), Decision::Confirm);
        // `confirmed` without an armed action never runs
        let mut b = None;
        assert_eq!(decide(&mut b, "shutdown", true, 1000), Decision::Confirm);
        // second press inside the window runs, and disarms
        assert_eq!(decide(&mut a, "shutdown", true, 3999), Decision::Run);
        assert_eq!(decide(&mut a, "shutdown", true, 4000), Decision::Confirm);
        // too late: re-arms instead of running
        let mut c = None;
        decide(&mut c, "restart", false, 0);
        assert_eq!(decide(&mut c, "restart", true, 3001), Decision::Confirm);
        assert_eq!(decide(&mut c, "restart", true, 3500), Decision::Run);
        // a different action cannot ride on another action's arming
        let mut d = None;
        decide(&mut d, "lock", false, 0);
        assert_eq!(decide(&mut d, "shutdown", true, 100), Decision::Confirm);
        // unconfirmed second press re-arms
        assert_eq!(decide(&mut d, "shutdown", false, 200), Decision::Confirm);
    }
}
