//! Small system seams of the island: the wall clock the shell model reads (with a test override), the sink for the commands the shell
//! issues, and two registry reads (OS light mode, OS language). Everything here is cheap and allocation-free on the hot path.
use pill_ui_core::shell::{Backend, Clock};
use serde_json::Value;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Wall clock: `Date.now()` and `-getTimezoneOffset()`. Tests pin it with [`SysClock::fixed`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SysClock {
    fixed: Option<(i64, i32)>,
}

impl SysClock {
    pub fn real() -> SysClock {
        SysClock { fixed: None }
    }
    /// `wall_ms` since the epoch (UTC) and the local offset in minutes east of UTC.
    pub fn fixed(wall_ms: i64, offset_min: i32) -> SysClock {
        SysClock { fixed: Some((wall_ms, offset_min)) }
    }
}

impl Clock for SysClock {
    fn wall_ms(&self) -> i64 {
        self.fixed.map(|f| f.0).unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0))
    }
    fn utc_offset_min(&self) -> i32 {
        self.fixed.map(|f| f.1).unwrap_or_else(local_offset_min)
    }
}

/// Minutes east of UTC right now (Windows time-zone API, daylight saving included).
pub fn local_offset_min() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let extra = if id == 2 { tz.DaylightBias } else { tz.StandardBias };
    -(tz.Bias + extra)
}

pub type CmdQueue = Rc<RefCell<Vec<(String, Value)>>>;

/// The shell's `Backend`: commands are queued here and the view executes them after every shell call (it owns the `Cx`).
#[derive(Clone, Default)]
pub struct QueueBackend(pub CmdQueue);

impl Backend for QueueBackend {
    fn invoke(&mut self, cmd: &str, args: Value) {
        self.0.borrow_mut().push((cmd.to_string(), args));
    }
}

fn reg_dword(sub: &str, name: &str) -> Option<u32> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut v = 0u32;
    let mut n = 4u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(sub), &HSTRING::from(name), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut n)) };
    r.is_ok().then_some(v)
}

/// `prefers-color-scheme: light` of the apps (`AppsUseLightTheme`).
pub fn os_prefers_light() -> bool {
    reg_dword(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", "AppsUseLightTheme") == Some(1)
}

/// `navigator.language`: the user's locale name ("ru-RU"), "en-US" when unknown.
pub fn os_lang() -> String {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let mut buf = [0u16; 85];
    let mut n = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(r"Control Panel\International"), &HSTRING::from("LocaleName"), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut n)) };
    if r.is_err() {
        return "en-US".into();
    }
    let len = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_clock_is_exact_and_real_clock_moves_forward() {
        let c = SysClock::fixed(1_793_363_640_000, 120);
        assert_eq!((c.wall_ms(), c.utc_offset_min()), (1_793_363_640_000, 120));
        assert!(SysClock::real().wall_ms() > 1_700_000_000_000);
        assert!(local_offset_min().abs() <= 14 * 60);
    }

    #[test]
    fn backend_queues_in_order() {
        let q = CmdQueue::default();
        let mut b = QueueBackend(q.clone());
        b.invoke("a", Value::Null);
        b.invoke("b", serde_json::json!({"x": 1}));
        let got: Vec<String> = q.borrow().iter().map(|c| c.0.clone()).collect();
        assert_eq!(got, ["a", "b"]);
    }
}
