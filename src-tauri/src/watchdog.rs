// Watchdog for the UI thread: if the thread that draws the island stops answering for 15 s, write a minidump into
// %LOCALAPPDATA%\Pill\logs and start the app again (at most 3 times an hour), then end this process.
//
// How it knows: the UI thread counts every message it handles and every wake of its loop (`beat`, one relaxed increment, no clock). This
// thread wakes once per second. A UI thread that is merely idle sleeps in its message wait and counts nothing, so after 5 s without a count
// the watchdog posts it a message (`ping`, once per 5 s while quiet); a thread that is alive answers at once. 15 s without any count despite
// the pings = hung. Nothing is polled faster, nothing runs on the UI thread by itself.
//
// Not a hang: the machine was asleep (a power suspend / resume message, or this thread itself not running for 5+ s), a session unlock or
// logon (the desktop switch can stall every window for a moment), and the first 30 s after start. After each of those the clock restarts.
//
// Privacy: the dump holds this process's memory and stays on this PC (the logs folder, newest 5 kept); nothing reads or sends it.
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::Instant;

/// A quiet UI thread is pinged after this long without a count.
pub const PING_MS: u64 = 5_000;
/// No count for this long, pings included: hung.
pub const HUNG_MS: u64 = 15_000;
/// This thread's own ticks are this far apart: the machine was asleep (or frozen), not the UI thread.
pub const GAP_MS: u64 = 5_000;
/// No verdict for this long after a resume, an unlock or the start.
pub const GRACE_MS: u64 = 30_000;
pub const MAX_RESTARTS: usize = 3;
pub const WINDOW_S: u64 = 3_600;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Verdict {
    Ok,
    /// post the UI thread a message so it counts
    Ping,
    /// dump + restart (once per hang)
    Hung,
}

/// The decision logic, with the clock passed in (milliseconds, any origin) so tests can drive it.
pub struct Dog {
    beat_ms: u64,
    tick_ms: u64,
    grace_until: u64,
    ping_at: Option<u64>,
    fired: bool,
}

impl Dog {
    /// Starts with the start-up grace.
    pub fn new(now: u64) -> Dog {
        Dog { beat_ms: now, tick_ms: now, grace_until: now + GRACE_MS, ping_at: None, fired: false }
    }

    /// The UI thread was seen alive.
    pub fn beat(&mut self, now: u64) {
        self.beat_ms = now;
        self.ping_at = None;
        self.fired = false;
    }

    /// Power resume / session unlock / logon: forgive the past, wait `GRACE_MS` before any verdict.
    pub fn resumed(&mut self, now: u64) {
        self.beat_ms = now;
        self.ping_at = None;
        self.fired = false;
        self.grace_until = now + GRACE_MS;
    }

    /// Once a second.
    pub fn tick(&mut self, now: u64) -> Verdict {
        if now.saturating_sub(self.tick_ms) > GAP_MS {
            self.resumed(now); // this thread was not running: the whole machine slept (or stalled)
        }
        self.tick_ms = now;
        let quiet = now.saturating_sub(self.beat_ms);
        if quiet >= HUNG_MS && now >= self.grace_until {
            if self.fired {
                return Verdict::Ok;
            }
            self.fired = true;
            return Verdict::Hung;
        }
        if quiet >= PING_MS && self.ping_at.is_none_or(|p| now.saturating_sub(p) >= PING_MS) {
            self.ping_at = Some(now);
            return Verdict::Ping;
        }
        Verdict::Ok
    }
}

/// May the app restart itself once more? `history` = unix seconds of the restarts so far.
pub fn may_restart(history: &[u64], now_s: u64) -> bool {
    history.iter().filter(|&&t| now_s.saturating_sub(t) < WINDOW_S).count() < MAX_RESTARTS
}

/// The history to store after a restart at `now_s`: old entries dropped.
pub fn recorded(history: &[u64], now_s: u64) -> Vec<u64> {
    let mut v: Vec<u64> = history.iter().copied().filter(|&t| now_s.saturating_sub(t) < WINDOW_S).collect();
    v.push(now_s);
    v
}

/// Names of the dumps to delete so that `keep` newest remain (names sort by their unix time, zero-padded).
pub fn stale_dumps(mut names: Vec<String>, keep: usize) -> Vec<String> {
    names.retain(|n| n.starts_with("pill-hang-") && n.ends_with(".dmp"));
    names.sort();
    let cut = names.len().saturating_sub(keep);
    names.truncate(cut);
    names
}

pub fn dump_name(unix_s: u64) -> String {
    format!("pill-hang-{unix_s:012}.dmp")
}

fn parse_history(text: &str) -> Vec<u64> {
    text.lines().filter_map(|l| l.trim().parse().ok()).collect()
}

fn unix_s() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// the UI side: counting and the ping target
// ---------------------------------------------------------------------------------------------------------------------------------------

static COUNT: AtomicU64 = AtomicU64::new(0);
static PING_TARGET: AtomicIsize = AtomicIsize::new(0);
static RESUMED: AtomicBool = AtomicBool::new(false);

/// The UI thread handled a message / woke up.
#[inline]
pub fn beat() {
    COUNT.fetch_add(1, Ordering::Relaxed);
}

/// The window the ping is posted to (the host's message window); also where `WM_APP + 1` wakes the loop.
pub fn set_ping_target(hwnd: isize) {
    PING_TARGET.store(hwnd, Ordering::Release);
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// the watchdog thread
// ---------------------------------------------------------------------------------------------------------------------------------------

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Diagnostics::Debug::{MiniDumpWriteDump, MINIDUMP_TYPE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::RemoteDesktop::WTSRegisterSessionNotification;
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, PostMessageW, RegisterClassExW, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, WINDOW_EX_STYLE,
    WINDOW_STYLE, WNDCLASSEXW, WM_APP,
};

const WM_POWERBROADCAST: u32 = 0x0218;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;

unsafe extern "system" fn wd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // PBT_APMSUSPEND 4, PBT_APMRESUMESUSPEND 7, PBT_APMRESUMEAUTOMATIC 0x12
        WM_POWERBROADCAST if matches!(wp.0, 0x4 | 0x7 | 0x12) => RESUMED.store(true, Ordering::Relaxed),
        // WTS_CONSOLE_CONNECT 1, WTS_REMOTE_CONNECT 3, WTS_SESSION_LOGON 5, WTS_SESSION_UNLOCK 8
        WM_WTSSESSION_CHANGE if matches!(wp.0, 0x1 | 0x3 | 0x5 | 0x8) => RESUMED.store(true, Ordering::Relaxed),
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// A hidden top-level window: the only way to receive power broadcasts and session notifications.
fn notify_window() -> Option<HWND> {
    unsafe {
        let hinst = GetModuleHandleW(None).ok()?;
        let cls = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(wd_proc), hInstance: hinst.into(), lpszClassName: w!("PillWatchdog"), ..Default::default() };
        RegisterClassExW(&cls);
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PillWatchdog"), w!("pill-watchdog"), WINDOW_STYLE(0x8000_0000 /* WS_POPUP, never shown */), 0, 0, 0, 0, None, None, hinst, None).ok()?;
        let _ = WTSRegisterSessionNotification(hwnd, 0 /* NOTIFY_FOR_THIS_SESSION */);
        Some(hwnd)
    }
}

fn ping() {
    let h = PING_TARGET.load(Ordering::Acquire);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as *mut std::ffi::c_void), WM_APP + 1, WPARAM(0), LPARAM(0));
        }
    }
}

fn logs_dir() -> Option<std::path::PathBuf> {
    crate::applog::log_path()?.parent().map(std::path::Path::to_path_buf)
}

fn write_dump(dir: &std::path::Path, unix: u64) -> Option<std::path::PathBuf> {
    use std::os::windows::io::AsRawHandle;
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(dump_name(unix));
    let file = std::fs::File::create(&path).ok()?;
    // normal + thread info + unloaded modules + process/thread data: stacks of every thread, a few MB
    let kind = MINIDUMP_TYPE(0 | 0x1000 | 0x20 | 0x100);
    let ok = unsafe { MiniDumpWriteDump(GetCurrentProcess(), GetCurrentProcessId(), HANDLE(file.as_raw_handle()), kind, None, None, None) }.is_ok();
    drop(file);
    if !ok {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let names: Vec<String> = rd.filter_map(|e| e.ok()?.file_name().into_string().ok()).collect();
        for old in stale_dumps(names, 5) {
            let _ = std::fs::remove_file(dir.join(old));
        }
    }
    Some(path)
}

/// Starts the app again (it waits for this process to end, see `wait_for_predecessor`) and ends this one. False when it could not start.
fn relaunch() -> bool {
    use std::os::windows::process::CommandExt;
    let Ok(exe) = std::env::current_exe() else { return false };
    let started = std::process::Command::new(exe).arg("--relaunch-wait").arg(std::process::id().to_string()).creation_flags(0x0800_0000 /* CREATE_NO_WINDOW */).spawn().is_ok();
    if started {
        unsafe {
            let _ = TerminateProcess(GetCurrentProcess(), 3);
        }
    }
    started
}

fn on_hung() {
    eprintln!("watchdog: the UI thread has not answered for {} s", HUNG_MS / 1000);
    let Some(dir) = logs_dir() else { return };
    let now = unix_s();
    match write_dump(&dir, now) {
        Some(p) => eprintln!("watchdog: minidump written to {}", p.display()),
        None => eprintln!("watchdog: the minidump could not be written"),
    }
    let hist_path = dir.join("watchdog-restarts.txt");
    let hist = std::fs::read_to_string(&hist_path).map(|t| parse_history(&t)).unwrap_or_default();
    if !may_restart(&hist, now) {
        eprintln!("watchdog: {MAX_RESTARTS} restarts in the last hour already; not restarting again");
        return;
    }
    let next = recorded(&hist, now).iter().map(u64::to_string).collect::<Vec<_>>().join("\n");
    let _ = std::fs::write(&hist_path, next);
    eprintln!("watchdog: restarting Pill");
    if !relaunch() {
        eprintln!("watchdog: the restart could not be started");
    }
}

/// `pill.exe --relaunch-wait <pid>` (started by the watchdog): wait until the hung predecessor is gone, so the single-instance check passes.
pub fn wait_for_predecessor() {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--relaunch-wait" {
            if let Some(pid) = args.next().and_then(|p| p.parse::<u32>().ok()) {
                unsafe {
                    if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
                        WaitForSingleObject(h, 30_000);
                        let _ = CloseHandle(h);
                    }
                }
            }
            return;
        }
    }
}

pub fn init() {
    let _ = std::thread::Builder::new().name("pill-watchdog".into()).spawn(|| {
        let t0 = Instant::now();
        let now_ms = move || t0.elapsed().as_millis() as u64;
        let win = notify_window();
        let mut dog = Dog::new(now_ms());
        let mut seen = COUNT.load(Ordering::Relaxed);
        let mut last = now_ms();
        loop {
            // one wake a second; power / session messages end the wait early and are handled, the tick itself stays once per second
            unsafe {
                let _ = MsgWaitForMultipleObjectsEx(None, 1000, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                if win.is_some() {
                    let mut m = MSG::default();
                    while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                        DispatchMessageW(&m);
                    }
                }
            }
            let now = now_ms();
            if RESUMED.swap(false, Ordering::Relaxed) {
                dog.resumed(now);
            }
            if now.saturating_sub(last) < 900 {
                continue;
            }
            last = now;
            let c = COUNT.load(Ordering::Relaxed);
            if c != seen {
                seen = c;
                dog.beat(now);
            }
            match dog.tick(now) {
                Verdict::Ok => {}
                Verdict::Ping => ping(),
                Verdict::Hung => on_hung(),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs a dog on a fake clock: `alive` says when the UI thread counts (it answers a ping at once). Returns (pings, hangs, first hang at).
    fn run(alive: impl Fn(u64) -> bool, secs: u64, resume_at: Option<u64>) -> (u32, u32, Option<u64>) {
        let mut d = Dog::new(0);
        let (mut pings, mut hangs, mut first) = (0, 0, None);
        for s in 1..=secs {
            let now = s * 1000;
            if resume_at == Some(s) {
                d.resumed(now);
            }
            if alive(s) {
                d.beat(now);
            }
            match d.tick(now) {
                Verdict::Ping => {
                    pings += 1;
                    // the ping reaches a live thread within the same second
                    if alive(s) {
                        d.beat(now);
                    }
                }
                Verdict::Hung => {
                    hangs += 1;
                    first.get_or_insert(s);
                }
                Verdict::Ok => {}
            }
        }
        (pings, hangs, first)
    }

    #[test]
    fn a_busy_ui_thread_is_never_pinged() {
        let (pings, hangs, _) = run(|s| s % 2 == 0, 600, None);
        assert_eq!((pings, hangs), (0, 0));
    }

    #[test]
    fn an_idle_ui_thread_is_pinged_every_five_seconds_and_answers() {
        // silent but inside the start-up grace: asked to speak, never judged
        let (pings, hangs, _) = run(|_| false, 20, None);
        assert_eq!((pings, hangs), (4, 0));
        // a live thread counts as soon as it is pinged
        let mut d = Dog::new(0);
        let mut pings = 0;
        for s in 1..=300u64 {
            let now = s * 1000;
            match d.tick(now) {
                Verdict::Ping => {
                    pings += 1;
                    d.beat(now); // the answer
                }
                Verdict::Hung => panic!("a thread that answers pings is not hung (at {s} s)"),
                Verdict::Ok => {}
            }
        }
        assert_eq!(pings, 60, "one ping per 5 s over 300 s");
    }

    #[test]
    fn a_dead_ui_thread_is_a_hang_after_fifteen_quiet_seconds_once() {
        // the start-up grace is 30 s: the first verdict can only come then
        let (pings, hangs, first) = run(|_| false, 120, None);
        assert_eq!(hangs, 1, "reported once, not every second");
        assert_eq!(first, Some(30));
        assert!(pings >= 4);
        // dead after 100 s of life: 15 s of silence is enough
        let (_, hangs, first) = run(|s| s <= 100, 200, None);
        assert_eq!((hangs, first), (1, Some(115)));
    }

    #[test]
    fn a_thread_that_comes_back_can_hang_again() {
        let (_, hangs, _) = run(|s| s <= 60 || (100..=120).contains(&s), 400, None);
        assert_eq!(hangs, 2);
    }

    #[test]
    fn a_resume_or_unlock_forgives_thirty_seconds() {
        // alive until 100, silent after, the machine "resumes" at 110: no verdict before 140
        let (_, hangs, first) = run(|s| s <= 100, 300, Some(110));
        assert_eq!((hangs, first), (1, Some(140)));
        // without the resume the verdict would have come at 115
        let (_, _, first) = run(|s| s <= 100, 300, None);
        assert_eq!(first, Some(115));
    }

    #[test]
    fn a_stalled_watchdog_means_the_machine_slept() {
        let mut d = Dog::new(0);
        for s in 1..=51u64 {
            d.beat(s * 1000);
            assert_eq!(d.tick(s * 1000), Verdict::Ok);
        }
        // the next tick comes an hour later (suspend): not a hang
        assert_eq!(d.tick(51_000 + 3_600_000), Verdict::Ok);
        // and it keeps quiet for the grace period, then asks the thread to speak
        assert_eq!(d.tick(51_000 + 3_600_000 + 5_000), Verdict::Ping);
        assert_eq!(d.tick(51_000 + 3_600_000 + 6_000), Verdict::Ok);
        d.beat(51_000 + 3_600_000 + 7_000);
        assert_eq!(d.tick(51_000 + 3_600_000 + 8_000), Verdict::Ok);
    }

    #[test]
    fn at_most_three_restarts_an_hour() {
        let now = 1_000_000;
        assert!(may_restart(&[], now));
        assert!(may_restart(&[now - 10, now - 20], now));
        assert!(!may_restart(&[now - 10, now - 20, now - 30], now));
        assert!(may_restart(&[now - 10, now - 20, now - 3_600], now), "an hour old does not count");
        let h = recorded(&[now - 7_200, now - 100], now);
        assert_eq!(h, vec![now - 100, now]);
        assert!(!may_restart(&recorded(&recorded(&recorded(&[], now), now + 1), now + 2), now + 3));
    }

    #[test]
    fn history_file_and_dump_names() {
        assert_eq!(parse_history("12\n\n  34 \nx\n56"), vec![12, 34, 56]);
        assert_eq!(dump_name(7), "pill-hang-000000000007.dmp");
        let names: Vec<String> = (1..=8).map(dump_name).chain(["pill.log".to_string(), "notes.dmp".to_string()]).collect();
        let del = stale_dumps(names, 5);
        assert_eq!(del, vec![dump_name(1), dump_name(2), dump_name(3)]);
        assert!(stale_dumps(vec![dump_name(1)], 5).is_empty());
    }
}
