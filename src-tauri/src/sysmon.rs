// System monitor of the island (CPU / RAM / NVIDIA GPUs). New file of the Pill app; user request: "давай делать все", item: system monitor.
// Command `sysmon_get` -> { cpu: percent, ram: { usedMb, totalMb }, gpus: [{ index, name, util, tempC, memUsedMb, memTotalMb, powerW }] }.
// The same payload goes to window "pill" as event `sysmon:tick` every 2 s, but only while `live.sysmon` is on AND the pill window is visible.
// CPU: GetSystemTimes deltas. RAM: GlobalMemoryStatusEx. GPU: ONE persistent `nvidia-smi --query-gpu=... --format=csv,noheader,nounits -lms 2000` child
// (a reader thread keeps the newest line per GPU), alive only while ticks are wanted, in a kill-on-close Job Object (no NVIDIA = `gpus: []`, looked for again every 60 s).
// PRIVACY: reads only load counters, nothing is stored or sent anywhere.
use serde::Serialize;
use serde_json::{json, Value};
use std::ffi::c_void;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL_MS: u64 = 2000;
const SMI_TIMEOUT: Duration = Duration::from_millis(2000); // wait for the first line of a fresh child
const SMI_STALE: Duration = Duration::from_millis(3 * POLL_MS); // no line for this long = the child hangs
const SMI_RETRY: Duration = Duration::from_secs(60); // nvidia-smi missing / failing: look again this late
const QUERY: &str = "--query-gpu=index,name,utilization.gpu,temperature.gpu,memory.used,memory.total,power.draw";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub index: u32,
    pub name: String,
    pub util: Option<u32>,
    pub temp_c: Option<u32>,
    pub mem_used_mb: Option<u64>,
    pub mem_total_mb: Option<u64>,
    pub power_w: Option<f64>,
}

// ---------------------------------------------------------------- parsing (pure, unit-tested)
/// "[N/A]", "N/A", "[Not Supported]", "" -> None.
fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

/// One csv line "0, NVIDIA GeForce RTX 4090, 5, 33, 5006, 24564, 83.88". The last five columns are numeric, so a comma inside the name is harmless.
pub fn parse_line(line: &str) -> Option<Gpu> {
    let p: Vec<&str> = line.split(',').map(str::trim).collect();
    if p.len() < 7 {
        return None;
    }
    let tail = p.len() - 5;
    Some(Gpu {
        index: p[0].parse().ok()?,
        name: p[1..tail].join(", "),
        util: num(p[tail]).map(|v| v.round().min(100.0) as u32),
        temp_c: num(p[tail + 1]).map(|v| v.round() as u32),
        mem_used_mb: num(p[tail + 2]).map(|v| v.round() as u64),
        mem_total_mb: num(p[tail + 3]).map(|v| v.round() as u64),
        power_w: num(p[tail + 4]),
    })
}

// ---------------------------------------------------------------- CPU / RAM
fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

fn times() -> Option<(u64, u64)> {
    let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut i), Some(&mut k), Some(&mut u)).ok()? };
    Some((ft(i), ft(k) + ft(u))) // kernel time already contains idle time
}

static CPU_PREV: Mutex<Option<(u64, u64)>> = Mutex::new(None);

/// Busy percent since the previous call. With no previous sample (first call, or after a long pause) it primes with a short 250 ms window.
fn cpu_pct() -> f64 {
    let mut prev = CPU_PREV.lock().unwrap_or_else(|e| e.into_inner());
    if prev.is_none() {
        *prev = times();
        std::thread::sleep(Duration::from_millis(250));
    }
    let (Some(now), Some((pi, pt))) = (times(), *prev) else { return 0.0 };
    *prev = Some(now);
    let (di, dt) = (now.0.saturating_sub(pi), now.1.saturating_sub(pt));
    if dt == 0 {
        return 0.0;
    }
    ((dt.saturating_sub(di)) as f64 / dt as f64 * 100.0).clamp(0.0, 100.0)
}

fn ram() -> Value {
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_err() {
        return json!({ "usedMb": 0, "totalMb": 0 });
    }
    let mb = |b: u64| b / (1024 * 1024);
    json!({ "usedMb": mb(m.ullTotalPhys.saturating_sub(m.ullAvailPhys)), "totalMb": mb(m.ullTotalPhys) })
}

// ---------------------------------------------------------------- GPU (nvidia-smi)
static SMI: Mutex<Option<PathBuf>> = Mutex::new(None);

fn find_smi() -> Option<PathBuf> {
    let mut cache = SMI.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = cache.as_ref().filter(|p| p.is_file()) {
        return Some(p.clone());
    }
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|v| std::env::split_paths(&v).collect::<Vec<_>>()).map(|d| d.join("nvidia-smi.exe"));
    let fixed = [r"C:\Windows\System32\nvidia-smi.exe", r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe"].map(PathBuf::from);
    let found = on_path.chain(fixed).find(|p| p.is_file());
    *cache = found.clone(); // None is retried after SMI_RETRY (see start): a driver installed later is found then
    found
}

/// The one persistent `nvidia-smi -lms` child and the newest line it printed per GPU.
struct Smi {
    child: Option<Child>,
    job: isize, // kill-on-close Job Object: the child dies with Pill
    id: u64,    // bumped on every start / stop: a reader thread of an older child sees the change and ends
    gpus: Vec<Gpu>,
    at: Option<Instant>,       // last line
    retry_at: Option<Instant>, // no (re)start before this: nvidia-smi missing / failing
}

static FEED: Mutex<Smi> = Mutex::new(Smi { child: None, job: 0, id: 0, gpus: Vec::new(), at: None, retry_at: None });
static DATA: Condvar = Condvar::new();

fn feed() -> MutexGuard<'static, Smi> {
    FEED.lock().unwrap_or_else(|e| e.into_inner())
}

/// Newest value per GPU index (a loop-mode sample is one line per GPU).
fn merge(gpus: &mut Vec<Gpu>, g: Gpu) {
    match gpus.iter_mut().find(|x| x.index == g.index) {
        Some(x) => *x = g,
        None => {
            gpus.push(g);
            gpus.sort_by_key(|x| x.index);
        }
    }
}

/// Kills the child, closes its Job Object and drops the data.
fn reap(f: &mut Smi) {
    if let Some(mut c) = f.child.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    if f.job != 0 {
        unsafe {
            let _ = CloseHandle(HANDLE(f.job as *mut c_void));
        }
        f.job = 0;
    }
    f.id += 1;
    f.gpus.clear();
    f.at = None;
}

/// Nobody needs GPU data right now (switch off / pill hidden): no child at all.
fn stop() {
    let mut f = feed();
    if f.child.is_some() {
        reap(&mut f);
        DATA.notify_all();
    }
}

fn start(f: &mut Smi) {
    let now = Instant::now();
    if f.child.is_some() || f.retry_at.is_some_and(|t| now < t) {
        return;
    }
    f.retry_at = Some(now + SMI_RETRY); // cleared on success below: a missing driver is looked for again after SMI_RETRY, not every tick
    let Some(exe) = find_smi() else { return };
    let Ok(mut child) = Command::new(exe)
        .args([QUERY, "--format=csv,noheader,nounits", "-lms", &POLL_MS.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    else {
        return;
    };
    let Some(out) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return;
    };
    f.job = crate::capture::job_for(&child);
    f.child = Some(child);
    f.id += 1;
    f.retry_at = None;
    let id = f.id;
    std::thread::spawn(move || {
        let mut got = false;
        for g in BufReader::new(out).lines().map_while(Result::ok).filter_map(|l| parse_line(&l)) {
            let mut f = feed();
            if f.id != id {
                return;
            }
            merge(&mut f.gpus, g);
            f.at = Some(Instant::now());
            got = true;
            DATA.notify_all();
        }
        // end of output: the child exited (or stop() killed it)
        let mut f = feed();
        if f.id == id {
            reap(&mut f);
            f.retry_at = Some(Instant::now() + if got { Duration::from_secs(5) } else { SMI_RETRY });
            DATA.notify_all();
        }
    });
}

fn gpus() -> Vec<Gpu> {
    let mut f = feed();
    if f.at.is_some_and(|t| t.elapsed() >= SMI_STALE) {
        reap(&mut f); // the child hangs: start a new one
    }
    start(&mut f);
    // the first sample after a start waits for the child's first line (tens of ms), like the old one-shot call did
    if f.child.is_some() && f.at.is_none() {
        f = DATA.wait_timeout_while(f, SMI_TIMEOUT, |f| f.child.is_some() && f.at.is_none()).unwrap_or_else(|e| e.into_inner()).0;
    }
    f.gpus.clone()
}

// ---------------------------------------------------------------- sampling
static LAST: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

fn sample() -> Value {
    let v = json!({ "cpu": (cpu_pct() * 10.0).round() / 10.0, "ram": ram(), "gpus": gpus() });
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), v.clone()));
    v
}

/// The poll thread and the command share one CPU baseline, so a command call right after a tick reuses the tick.
fn sample_cached() -> Value {
    if let Some((at, v)) = LAST.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if at.elapsed() < Duration::from_millis(1500) {
            return v.clone();
        }
    }
    sample()
}

#[tauri::command]
pub async fn sysmon_get() -> Value {
    tauri::async_runtime::spawn_blocking(sample_cached).await.unwrap_or(Value::Null)
}

pub fn init(app: &tauri::App) {
    let app: AppHandle = app.handle().clone();
    // one thread for the whole run; while the switch is off or the pill is hidden it only wakes once a second and does nothing
    std::thread::spawn(move || loop {
        if crate::settings::get_bool("live.sysmon", true) && !crate::pill::is_hidden() {
            let _ = app.emit_to("pill", "sysmon:tick", sample());
            std::thread::sleep(Duration::from_millis(POLL_MS));
        } else {
            *CPU_PREV.lock().unwrap_or_else(|e| e.into_inner()) = None; // the next delta must not span the pause
            stop(); // no tick consumer: no nvidia-smi child either
            std::thread::sleep(Duration::from_millis(1000));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_line() {
        let g = parse_line("0, NVIDIA GeForce RTX 4090, 5, 33, 5006, 24564, 83.88").unwrap();
        assert_eq!((g.index, g.name.as_str(), g.util, g.temp_c), (0, "NVIDIA GeForce RTX 4090", Some(5), Some(33)));
        assert_eq!((g.mem_used_mb, g.mem_total_mb, g.power_w), (Some(5006), Some(24564), Some(83.88)));
    }

    #[test]
    fn tolerates_na_and_decimals() {
        let g = parse_line("1, NVIDIA GeForce RTX 5090, 7, [N/A], 100, 32607, [N/A]").unwrap();
        assert_eq!((g.temp_c, g.power_w, g.util), (None, None, Some(7)));
        assert_eq!(parse_line("0, X, 5, 33.36, 1, 2, 3").unwrap().temp_c, Some(33));
    }

    #[test]
    fn comma_in_name_and_garbage() {
        assert_eq!(parse_line("0, A, B, 1, 2, 3, 4, 5").unwrap().name, "A, B");
        assert!(parse_line("").is_none() && parse_line("garbage").is_none() && parse_line("x, N, 1, 2, 3, 4, 5").is_none());
        assert!(parse_line("NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.").is_none());
    }

    #[test]
    fn merge_keeps_newest_per_index() {
        let (a, b) = (parse_line("1, B, 1, 2, 3, 4, 5").unwrap(), parse_line("0, A, 1, 2, 3, 4, 5").unwrap());
        let mut v = Vec::new();
        merge(&mut v, a);
        merge(&mut v, b);
        assert_eq!(v.iter().map(|g| g.index).collect::<Vec<_>>(), [0, 1]); // sorted by index
        merge(&mut v, parse_line("0, A, 99, 2, 3, 4, 5").unwrap());
        assert_eq!((v.len(), v[0].util), (2, Some(99))); // replaced, not appended
    }

    #[test]
    fn persistent_child_starts_and_stops() {
        let g = gpus(); // empty without NVIDIA; with it the first sample already has data
        assert_eq!(g.is_empty(), feed().child.is_none() || feed().at.is_none());
        stop();
        let f = feed();
        assert!(f.child.is_none() && f.gpus.is_empty() && f.job == 0);
    }

    #[test]
    fn cpu_and_ram_are_sane() {
        let c = cpu_pct();
        assert!((0.0..=100.0).contains(&c));
        assert!(ram()["totalMb"].as_u64().unwrap() > 0);
    }
}
