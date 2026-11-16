//! Energy rules: Pill saves power by explicit, visible rules (Settings > Look > Energy), never by silently lowering quality.
//!
//! * **Sense** (event driven, no polling): Energy Saver (`GUID_POWER_SAVING_STATUS`), AC / battery (`GUID_ACDC_POWER_SOURCE`), the display
//!   (`GUID_CONSOLE_DISPLAY_STATE`, and `GUID_SESSION_DISPLAY_STATE` which a Remote Desktop session reports as off while the client is
//!   minimised), the session (`WTS_SESSION_LOCK` / `UNLOCK`, connect / disconnect) and `SM_REMOTESESSION`. A hidden top-level window on the UI
//!   thread receives them (a message-only window gets neither); it only flips bits in an atomic and posts `WM_ENERGY` to the host.
//! * **Decide** ([`resolve`], pure): the senses + the user's switches -> pause, or an fps cap, and the reason.
//! * **Apply** (`win32.rs::apply_energy`, on `WM_ENERGY`): every window's `FrameState::set_energy`. A paused window wants no frame, so the loop
//!   sleeps with no deadline of its own (the cursor poll is skipped as well); lifting the pause redraws at once and fades the window in.
//!
//! Nothing here ticks while idle: the only wake-ups are the OS notifications above and the settings changes.
use serde_json::{Map, Value};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use windows::core::{w, GUID};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY, POWERBROADCAST_SETTING, SYSTEM_POWER_STATUS};
use windows::Win32::System::RemoteDesktop::{WTSRegisterSessionNotification, WTSUnRegisterSessionNotification};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, PostMessageW, RegisterClassExW, SM_REMOTESESSION, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WNDCLASSEXW};

/// The cap Energy Saver / battery apply.
pub const SAVER_FPS: u32 = 30;
/// Posted to the host's message window whenever the senses or the switches changed.
pub const WM_ENERGY: u32 = WM_APP + 0x2B0;

// ---------------------------------------------------------------------------------------------------------------------------------------
// the decision (pure)
// ---------------------------------------------------------------------------------------------------------------------------------------

/// The user's switches (settings keys `energy.*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rules {
    /// 30 fps while Windows Energy Saver / battery saver is on (`energy.saver`, default on)
    pub saver: bool,
    /// 30 fps while running on battery (`energy.battery`, default off)
    pub battery: bool,
    /// draw nothing while the session is locked or disconnected (`energy.pauseLocked`, default on)
    pub pause_locked: bool,
    /// ... while the display is off (`energy.pauseDisplayOff`, default on)
    pub pause_display: bool,
    /// ... while a Remote Desktop client is minimised (`energy.pauseRdp`, default on)
    pub pause_rdp: bool,
}

impl Rules {
    pub const DEFAULT: Rules = Rules { saver: true, battery: false, pause_locked: true, pause_display: true, pause_rdp: true };
}

impl Default for Rules {
    fn default() -> Rules {
        Rules::DEFAULT
    }
}

/// What the OS says right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sensed {
    pub saver: bool,
    pub on_battery: bool,
    /// the session is locked or not connected to a display
    pub locked: bool,
    /// the physical display is off (`GUID_CONSOLE_DISPLAY_STATE`)
    pub console_off: bool,
    /// this session's display is off (`GUID_SESSION_DISPLAY_STATE`; in a Remote Desktop session: the client is minimised)
    pub session_off: bool,
    /// running inside a Remote Desktop session (`SM_REMOTESESSION`)
    pub remote: bool,
}

/// Why Pill is capped or paused (shown in Settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    None,
    Saver,
    Battery,
    Locked,
    DisplayOff,
    RdpHidden,
}

impl Reason {
    pub fn text(self, ru: bool) -> &'static str {
        match self {
            Reason::None => "",
            Reason::Saver => if ru { "Экономия заряда" } else { "Energy Saver" },
            Reason::Battery => if ru { "Работа от батареи" } else { "on battery" },
            Reason::Locked => if ru { "сеанс заблокирован" } else { "session locked" },
            Reason::DisplayOff => if ru { "экран выключен" } else { "display off" },
            Reason::RdpHidden => if ru { "удалённый рабочий стол свёрнут" } else { "remote desktop minimised" },
        }
    }
}

/// The outcome: draw nothing, or draw at most `fps_cap` frames a second, or carry on as before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub paused: bool,
    pub fps_cap: Option<u32>,
    pub reason: Reason,
}

impl Verdict {
    pub const FREE: Verdict = Verdict { paused: false, fps_cap: None, reason: Reason::None };
}

/// The rules in order: a pause beats a cap, and among pauses the one that is most certain (locked, display, remote) names the reason.
pub fn resolve(r: &Rules, s: &Sensed) -> Verdict {
    // the physical display does not matter to a remote session; its own display state does (the client minimised)
    let display_off = if s.remote { false } else { s.console_off || s.session_off };
    let rdp_hidden = s.remote && s.session_off;
    let pause = |reason| Verdict { paused: true, fps_cap: None, reason };
    let cap = |reason| Verdict { paused: false, fps_cap: Some(SAVER_FPS), reason };
    if r.pause_locked && s.locked {
        pause(Reason::Locked)
    } else if r.pause_display && display_off {
        pause(Reason::DisplayOff)
    } else if r.pause_rdp && rdp_hidden {
        pause(Reason::RdpHidden)
    } else if r.saver && s.saver {
        cap(Reason::Saver)
    } else if r.battery && s.on_battery {
        cap(Reason::Battery)
    } else {
        Verdict::FREE
    }
}

/// What the settings window shows: the measured cost and the cap with its reason. `None` for the sample = the meter has not reported yet.
pub fn cost_lines(ru: bool, v: &Verdict, sample: Option<&super::perf::Sample>) -> (String, String) {
    let measured = match sample {
        None => (if ru { "Измеряется…" } else { "Measuring…" }).to_string(),
        Some(s) if s.island_fps < 0.05 => {
            if ru { format!("Остров в покое: кадры не рисуются · процессор ≈ {:.1} %", s.proc_cpu) } else { format!("The island is idle: no frames drawn · CPU ≈ {:.1} %", s.proc_cpu) }
        }
        Some(s) => {
            if ru {
                format!("{:.1} мс на кадр · {:.0} кадров/с · процессор ≈ {:.1} %", s.island_ms, s.island_fps, s.proc_cpu)
            } else {
                format!("{:.1} ms per frame · {:.0} fps · CPU ≈ {:.1} %", s.island_ms, s.island_fps, s.proc_cpu)
            }
        }
    };
    let why = v.reason.text(ru);
    let cap = match (v.paused, v.fps_cap) {
        (true, _) => if ru { format!("Остановлено: {why}") } else { format!("Paused: {why}") },
        (_, Some(n)) => if ru { format!("{n} кадров/с: {why}") } else { format!("{n} fps: {why}") },
        _ => (if ru { "Ограничения энергии нет: частота кадров по настройкам и экрану" } else { "No energy limit: the frame rate follows your settings and the display" }).to_string(),
    };
    (measured, cap)
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// shared state: written by the notification window and the settings hook, read by the host
// ---------------------------------------------------------------------------------------------------------------------------------------

const S_SAVER: u32 = 1;
const S_BATTERY: u32 = 2;
const S_LOCKED: u32 = 4;
const S_CONSOLE_OFF: u32 = 8;
const S_SESSION_OFF: u32 = 16;
const S_REMOTE: u32 = 32;

const R_SAVER: u32 = 1;
const R_BATTERY: u32 = 2;
const R_LOCKED: u32 = 4;
const R_DISPLAY: u32 = 8;
const R_RDP: u32 = 16;

static SENSED: AtomicU32 = AtomicU32::new(0);
static RULES: AtomicU32 = AtomicU32::new(R_SAVER | R_LOCKED | R_DISPLAY | R_RDP); // = Rules::DEFAULT
/// the host's message window (where `WM_ENERGY` goes) and our notification window
static HOST: AtomicIsize = AtomicIsize::new(0);
static WIN: AtomicIsize = AtomicIsize::new(0);

fn pack_sensed(s: &Sensed) -> u32 {
    [(s.saver, S_SAVER), (s.on_battery, S_BATTERY), (s.locked, S_LOCKED), (s.console_off, S_CONSOLE_OFF), (s.session_off, S_SESSION_OFF), (s.remote, S_REMOTE)].iter().fold(0, |a, (on, b)| if *on { a | b } else { a })
}

fn unpack_sensed(b: u32) -> Sensed {
    Sensed { saver: b & S_SAVER != 0, on_battery: b & S_BATTERY != 0, locked: b & S_LOCKED != 0, console_off: b & S_CONSOLE_OFF != 0, session_off: b & S_SESSION_OFF != 0, remote: b & S_REMOTE != 0 }
}

fn pack_rules(r: &Rules) -> u32 {
    [(r.saver, R_SAVER), (r.battery, R_BATTERY), (r.pause_locked, R_LOCKED), (r.pause_display, R_DISPLAY), (r.pause_rdp, R_RDP)].iter().fold(0, |a, (on, b)| if *on { a | b } else { a })
}

fn unpack_rules(b: u32) -> Rules {
    Rules { saver: b & R_SAVER != 0, battery: b & R_BATTERY != 0, pause_locked: b & R_LOCKED != 0, pause_display: b & R_DISPLAY != 0, pause_rdp: b & R_RDP != 0 }
}

pub fn sensed() -> Sensed {
    unpack_sensed(SENSED.load(Ordering::Relaxed))
}

pub fn rules() -> Rules {
    unpack_rules(RULES.load(Ordering::Relaxed))
}

/// The verdict for the senses and switches as they are now (cheap: two atomic loads).
pub fn verdict() -> Verdict {
    resolve(&rules(), &sensed())
}

fn edit_sensed(f: impl Fn(Sensed) -> Sensed) -> bool {
    let old = SENSED.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |b| Some(pack_sensed(&f(unpack_sensed(b))))).unwrap_or(0);
    SENSED.load(Ordering::Relaxed) != old
}

fn notify_host() {
    let h = HOST.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as *mut std::ffi::c_void), WM_ENERGY, WPARAM(0), LPARAM(0));
        }
    }
}

/// Reads the `energy.*` switches (defaults = [`Rules::DEFAULT`]).
fn load_rules() -> Rules {
    use crate::settings::get_bool;
    let d = Rules::DEFAULT;
    Rules {
        saver: get_bool("energy.saver", d.saver),
        battery: get_bool("energy.battery", d.battery),
        pause_locked: get_bool("energy.pauseLocked", d.pause_locked),
        pause_display: get_bool("energy.pauseDisplayOff", d.pause_display),
        pause_rdp: get_bool("energy.pauseRdp", d.pause_rdp),
    }
}

/// Hook of `settings::apply_patch` (any thread): re-reads the switches when one of them changed and tells the host.
pub fn on_settings(patch: &Map<String, Value>) {
    if patch.keys().any(|k| k.starts_with("energy.")) {
        RULES.store(pack_rules(&load_rules()), Ordering::Relaxed);
        notify_host();
    }
}

/// Safety net: a window got real input while the rules say "paused" (a missed unlock): the pause is lifted until the next event.
pub fn lift_pause() -> bool {
    edit_sensed(|s| Sensed { locked: false, console_off: false, session_off: false, ..s })
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// the notification window (UI thread)
// ---------------------------------------------------------------------------------------------------------------------------------------

const WM_POWERBROADCAST: u32 = 0x0218;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const PBT_APMRESUMESUSPEND: usize = 0x7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;

const GUID_CONSOLE_DISPLAY_STATE: GUID = GUID::from_u128(0x6fe69556_704a_47a0_8f24_c28d936fda47);
const GUID_SESSION_DISPLAY_STATE: GUID = GUID::from_u128(0x2b84c20e_ad23_4ddf_93db_05ffbd7efca5);
const GUID_POWER_SAVING_STATUS: GUID = GUID::from_u128(0xe00958c0_c213_4ace_ac77_fecceed2eea5);
const GUID_ACDC_POWER_SOURCE: GUID = GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);

/// What the OS reports without an event: the power status (battery saver, AC) and whether this is a remote session.
fn read_now() -> Sensed {
    let mut s = unpack_sensed(SENSED.load(Ordering::Relaxed));
    unsafe {
        let mut p = SYSTEM_POWER_STATUS::default();
        if GetSystemPowerStatus(&mut p).is_ok() {
            s.saver = p.SystemStatusFlag & 1 != 0;
            s.on_battery = p.ACLineStatus == 0;
        }
        s.remote = GetSystemMetrics(SM_REMOTESESSION) != 0;
    }
    s
}

/// A power-setting broadcast: `Some(new state)` when it was one of ours.
fn on_setting(s: Sensed, guid: GUID, value: u32) -> Option<Sensed> {
    Some(if guid == GUID_CONSOLE_DISPLAY_STATE {
        Sensed { console_off: value == 0, ..s } // 0 off, 1 on, 2 dimmed
    } else if guid == GUID_SESSION_DISPLAY_STATE {
        Sensed { session_off: value == 0, ..s }
    } else if guid == GUID_POWER_SAVING_STATUS {
        Sensed { saver: value != 0, ..s }
    } else if guid == GUID_ACDC_POWER_SOURCE {
        Sensed { on_battery: value == 1, ..s } // 0 AC, 1 battery, 2 UPS
    } else {
        return None;
    })
}

/// A session change: lock / disconnect pause, unlock / connect / logon resume. After a reconnect the state is not known, so it errs towards drawing.
fn on_session(s: Sensed, code: usize) -> Option<Sensed> {
    Some(match code {
        0x7 | 0x2 | 0x4 => Sensed { locked: true, ..s },                            // lock, console disconnect, remote disconnect
        0x8 | 0x5 | 0x1 | 0x3 => Sensed { locked: false, session_off: false, ..s }, // unlock, logon, console / remote connect
        _ => return None,
    })
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let changed = match msg {
        WM_POWERBROADCAST if wp.0 == PBT_POWERSETTINGCHANGE && lp.0 != 0 => {
            let ps = &*(lp.0 as *const POWERBROADCAST_SETTING);
            if ps.DataLength >= 4 {
                let v = std::ptr::read_unaligned(ps.Data.as_ptr() as *const u32);
                edit_sensed(|s| on_setting(s, ps.PowerSetting, v).unwrap_or(s))
            } else {
                false
            }
        }
        // after a sleep the display / power events of the wake may have been missed: start from what the OS says
        WM_POWERBROADCAST if wp.0 == PBT_APMRESUMESUSPEND || wp.0 == PBT_APMRESUMEAUTOMATIC => {
            let now = read_now();
            edit_sensed(|s| Sensed { console_off: false, session_off: false, ..Sensed { saver: now.saver, on_battery: now.on_battery, remote: now.remote, ..s } })
        }
        WM_WTSSESSION_CHANGE => {
            let remote = GetSystemMetrics(SM_REMOTESESSION) != 0;
            edit_sensed(|s| Sensed { remote, ..on_session(s, wp.0).unwrap_or(s) })
        }
        _ => false,
    };
    if changed {
        notify_host();
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

// the subscription handles (HPOWERNOTIFY as integers), released in `detach`
static REGS: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());

/// Called once on the UI thread with the host's message window: reads the starting state, creates the notification window and subscribes.
pub fn attach(host_msg: HWND) {
    HOST.store(host_msg.0 as isize, Ordering::Release);
    RULES.store(pack_rules(&load_rules()), Ordering::Relaxed);
    SENSED.store(pack_sensed(&read_now()), Ordering::Relaxed);
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else { return };
        let cls = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(proc), hInstance: hinst.into(), lpszClassName: w!("PillEnergy"), ..Default::default() };
        RegisterClassExW(&cls);
        // a hidden top-level window (WS_POPUP, never shown): message-only windows receive neither power settings nor session notifications
        let Ok(hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PillEnergy"), w!("pill-energy"), WINDOW_STYLE(0x8000_0000), 0, 0, 0, 0, None, None, hinst, None) else { return };
        WIN.store(hwnd.0 as isize, Ordering::Release);
        let _ = WTSRegisterSessionNotification(hwnd, 0 /* NOTIFY_FOR_THIS_SESSION */);
        let mut regs = REGS.lock().unwrap_or_else(|e| e.into_inner());
        for g in [GUID_CONSOLE_DISPLAY_STATE, GUID_SESSION_DISPLAY_STATE, GUID_POWER_SAVING_STATUS, GUID_ACDC_POWER_SOURCE] {
            // DEVICE_NOTIFY_WINDOW_HANDLE = 0
            if let Ok(h) = RegisterPowerSettingNotification(windows::Win32::Foundation::HANDLE(hwnd.0), &g, windows::Win32::UI::WindowsAndMessaging::REGISTER_NOTIFICATION_FLAGS(0)) {
                regs.push(h.0);
            }
        }
    }
}

/// Unsubscribes and destroys the window (the host is shutting down).
pub fn detach() {
    HOST.store(0, Ordering::Release);
    unsafe {
        for h in REGS.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            let _ = UnregisterPowerSettingNotification(HPOWERNOTIFY(h));
        }
        let w = WIN.swap(0, Ordering::AcqRel);
        if w != 0 {
            let hwnd = HWND(w as *mut std::ffi::c_void);
            let _ = WTSUnRegisterSessionNotification(hwnd);
            let _ = DestroyWindow(hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Sensed {
        Sensed::default()
    }
    fn on() -> Rules {
        Rules { saver: true, battery: true, pause_locked: true, pause_display: true, pause_rdp: true }
    }

    #[test]
    fn nothing_sensed_nothing_limited() {
        assert_eq!(resolve(&Rules::DEFAULT, &s()), Verdict::FREE);
        assert_eq!(resolve(&on(), &s()), Verdict::FREE);
    }

    #[test]
    fn energy_saver_caps_at_30_and_says_why() {
        let v = resolve(&Rules::DEFAULT, &Sensed { saver: true, ..s() });
        assert_eq!(v, Verdict { paused: false, fps_cap: Some(30), reason: Reason::Saver });
        let (_, cap) = cost_lines(true, &v, None);
        assert_eq!(cap, "30 кадров/с: Экономия заряда");
        assert_eq!(cost_lines(false, &v, None).1, "30 fps: Energy Saver");
        // the switch off: no cap, even though the OS asks for saving
        assert_eq!(resolve(&Rules { saver: false, ..Rules::DEFAULT }, &Sensed { saver: true, ..s() }), Verdict::FREE);
    }

    #[test]
    fn battery_is_opt_in_and_saver_names_the_reason_first() {
        let on_bat = Sensed { on_battery: true, ..s() };
        assert_eq!(resolve(&Rules::DEFAULT, &on_bat), Verdict::FREE, "off by default");
        assert_eq!(resolve(&on(), &on_bat), Verdict { paused: false, fps_cap: Some(30), reason: Reason::Battery });
        assert_eq!(resolve(&on(), &Sensed { saver: true, ..on_bat }).reason, Reason::Saver);
    }

    #[test]
    fn a_pause_beats_a_cap_and_each_pause_has_its_own_switch() {
        let both = Sensed { saver: true, locked: true, ..s() };
        assert_eq!(resolve(&on(), &both), Verdict { paused: true, fps_cap: None, reason: Reason::Locked });
        assert_eq!(resolve(&Rules { pause_locked: false, ..on() }, &both).reason, Reason::Saver, "locked rule off: the cap stays");
        let off = Sensed { console_off: true, ..s() };
        assert_eq!(resolve(&on(), &off).reason, Reason::DisplayOff);
        assert_eq!(resolve(&Rules { pause_display: false, ..on() }, &off), Verdict::FREE);
        // a display that dims (state 2) is not off: only value 0 sets the bit (see `on_setting`)
        assert_eq!(on_setting(s(), GUID_CONSOLE_DISPLAY_STATE, 2).unwrap().console_off, false);
        assert_eq!(on_setting(s(), GUID_CONSOLE_DISPLAY_STATE, 0).unwrap().console_off, true);
    }

    #[test]
    fn a_remote_session_follows_its_own_display_not_the_consoles() {
        let rdp = Sensed { remote: true, ..s() };
        assert_eq!(resolve(&on(), &Sensed { console_off: true, ..rdp }), Verdict::FREE, "the machine's monitor is off, the client still looks");
        assert_eq!(resolve(&on(), &Sensed { session_off: true, ..rdp }), Verdict { paused: true, fps_cap: None, reason: Reason::RdpHidden });
        assert_eq!(resolve(&Rules { pause_rdp: false, ..on() }, &Sensed { session_off: true, ..rdp }), Verdict::FREE);
        // locally the session display state counts as the display
        assert_eq!(resolve(&on(), &Sensed { session_off: true, ..s() }).reason, Reason::DisplayOff);
    }

    #[test]
    fn session_events_pause_and_resume() {
        let locked = on_session(s(), 0x7).unwrap();
        assert!(locked.locked);
        assert!(!on_session(locked, 0x8).unwrap().locked, "unlock");
        assert!(on_session(s(), 0x4).unwrap().locked, "a disconnected session shows nothing");
        let hidden = Sensed { locked: true, session_off: true, ..s() };
        let back = on_session(hidden, 0x3).unwrap();
        assert!(!back.locked && !back.session_off, "a reconnect errs towards drawing");
        assert_eq!(on_session(s(), 0x9), None, "other codes are ignored");
    }

    #[test]
    fn power_settings_map_to_the_senses() {
        assert!(on_setting(s(), GUID_POWER_SAVING_STATUS, 1).unwrap().saver);
        assert!(!on_setting(Sensed { saver: true, ..s() }, GUID_POWER_SAVING_STATUS, 0).unwrap().saver);
        assert!(on_setting(s(), GUID_ACDC_POWER_SOURCE, 1).unwrap().on_battery);
        assert!(!on_setting(s(), GUID_ACDC_POWER_SOURCE, 2).unwrap().on_battery, "a UPS is not a battery");
        assert_eq!(on_setting(s(), GUID::from_u128(1), 1), None);
    }

    #[test]
    fn the_bit_packing_round_trips() {
        for b in 0..64u32 {
            assert_eq!(pack_sensed(&unpack_sensed(b)), b);
        }
        for b in 0..32u32 {
            assert_eq!(pack_rules(&unpack_rules(b)), b);
        }
        assert_eq!(pack_rules(&Rules::DEFAULT), R_SAVER | R_LOCKED | R_DISPLAY | R_RDP);
    }

    #[test]
    fn the_cost_lines_say_what_is_measured_and_what_is_capped() {
        let smp = super::super::perf::Sample { at: std::time::Instant::now(), secs: 2.0, island_fps: 29.6, island_ms: 0.84, ui_cpu: 0.8, proc_cpu: 1.26 };
        let (m, c) = cost_lines(false, &Verdict::FREE, Some(&smp));
        assert_eq!(m, "0.8 ms per frame · 30 fps · CPU ≈ 1.3 %");
        assert!(c.starts_with("No energy limit"));
        let idle = super::super::perf::Sample { island_fps: 0.0, ..smp.clone() };
        assert!(cost_lines(true, &Verdict::FREE, Some(&idle)).0.starts_with("Остров в покое"));
        assert_eq!(cost_lines(false, &Verdict::FREE, None).0, "Measuring…");
        let paused = resolve(&on(), &Sensed { locked: true, ..s() });
        assert_eq!(cost_lines(false, &paused, None).1, "Paused: session locked");
    }
}
