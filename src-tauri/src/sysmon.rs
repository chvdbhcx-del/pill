// System monitor of the island (CPU / RAM / NVIDIA GPUs). New file of the Pill app; user request: "давай делать все", item: system monitor.
// Command `sysmon_get` -> { cpu: percent, ram: { usedMb, totalMb }, gpus: [{ index, name, util, tempC, memUsedMb, memTotalMb, powerW }] }.
// The same payload goes to window "pill" as event `sysmon:tick` every 2 s, but only while `live.sysmon` is on AND the pill window is visible.
// CPU: GetSystemTimes deltas. RAM: GlobalMemoryStatusEx. GPU: `nvidia-smi --query-gpu=... --format=csv,noheader,nounits` (no NVIDIA = `gpus: []`).
// PRIVACY: reads only load counters, nothing is stored or sent anywhere.
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::GetSystemTimes;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const POLL_MS: u64 = 2000;
const SMI_TIMEOUT: Duration = Duration::from_millis(2000);
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

pub fn parse_gpus(out: &str) -> Vec<Gpu> {
    out.lines().filter_map(parse_line).collect()
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
    *cache = found.clone(); // None is not sticky: a driver installed later is found on the next poll
    found
}

fn gpus() -> Vec<Gpu> {
    let Some(exe) = find_smi() else { return Vec::new() };
    let Ok(mut child) = Command::new(exe).args([QUERY, "--format=csv,noheader,nounits"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).creation_flags(CREATE_NO_WINDOW).spawn() else {
        return Vec::new();
    };
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if t0.elapsed() < SMI_TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    parse_gpus(&out)
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
        if crate::settings::get_bool("live.sysmon", false) && !crate::pill::is_hidden() {
            let _ = app.emit_to("pill", "sysmon:tick", sample());
            std::thread::sleep(Duration::from_millis(POLL_MS));
        } else {
            *CPU_PREV.lock().unwrap_or_else(|e| e.into_inner()) = None; // the next delta must not span the pause
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
        assert_eq!(parse_gpus("0, A, 1, 2, 3, 4, 5\n1, B, 1, 2, 3, 4, 5\n").len(), 2);
        assert!(parse_gpus("NVIDIA-SMI has failed because it couldn't communicate with the NVIDIA driver.").is_empty());
    }

    #[test]
    fn cpu_and_ram_are_sane() {
        let c = cpu_pct();
        assert!((0.0..=100.0).contains(&c));
        assert!(ram()["totalMb"].as_u64().unwrap() > 0);
    }
}
