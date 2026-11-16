//! `PILL_PERF=1`: every 5 s the host writes one line to stderr (the log file in a release build): per window how many frames were drawn and
//! what a frame cost (step / draw / present), how many times the loop woke up, and how much CPU the UI thread and the whole process used.
//! Off by default; when off, the stopwatches below do nothing.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes};

const REPORT_EVERY: Duration = Duration::from_secs(5);
/// While the settings window shows the frame cost the meter reports faster (and only to `last()`, not to stderr).
const SAMPLE_EVERY: Duration = Duration::from_secs(2);

fn env_on() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| std::env::var_os("PILL_PERF").is_some_and(|v| v != "0"))
}

/// The settings window is open and shows the frame cost (`set_sampling`): the meter runs although `PILL_PERF` is not set.
static SAMPLING: AtomicBool = AtomicBool::new(false);

pub fn on() -> bool {
    env_on() || SAMPLING.load(Ordering::Relaxed)
}

/// Switches the meter on for the settings window's cost line (`true` when it opens, `false` when it closes: then it costs nothing again).
pub fn set_sampling(on: bool) {
    SAMPLING.store(on, Ordering::Relaxed);
    if on {
        S.with(|s| *s.borrow_mut() = None); // the UI thread's meter starts a fresh interval (a stale one would average over the gap)
    } else {
        *LAST.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// What the last report measured, for the settings window's cost line.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub at: Instant,
    pub secs: f32,
    /// frames per second and milliseconds per frame (step + draw + present) of the island window (`PILL_LABEL`)
    pub island_fps: f32,
    pub island_ms: f32,
    pub ui_cpu: f32,
    pub proc_cpu: f32,
}

/// The label of the island's window: the one whose frame cost the settings line shows (the settings window's own frames do not count).
pub const ISLAND: &str = "pill";

static LAST: Mutex<Option<Sample>> = Mutex::new(None);

/// The newest sample, if the meter has produced one since `set_sampling(true)`.
pub fn last() -> Option<Sample> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A stopwatch that costs nothing when `PILL_PERF` is off.
pub struct Lap(Option<Instant>);

impl Lap {
    pub fn start() -> Lap {
        Lap(on().then(Instant::now))
    }
    /// Time since the previous lap (or the start).
    pub fn lap(&mut self) -> Duration {
        match self.0 {
            Some(t) => {
                let n = Instant::now();
                self.0 = Some(n);
                n - t
            }
            None => Duration::ZERO,
        }
    }
}

#[derive(Default)]
struct Acc {
    frames: u32,
    step: Duration,
    draw: Duration,
    present: Duration,
}

struct State {
    since: Instant,
    wakes: u32,
    /// why the island asked for another frame (reason -> steps it was true in)
    why: BTreeMap<&'static str, u32>,
    wins: BTreeMap<String, Acc>,
    ui_cpu: Duration,
    proc_cpu: Duration,
}

thread_local! {
    static S: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn ft(a: FILETIME, b: FILETIME) -> Duration {
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
    Duration::from_nanos((t(a) + t(b)) * 100)
}

/// Kernel + user time of the calling (UI) thread and of the process.
fn cpu_times() -> (Duration, Duration) {
    unsafe {
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ui = if GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u).is_ok() { ft(k, u) } else { Duration::ZERO };
        let pr = if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).is_ok() { ft(k, u) } else { Duration::ZERO };
        (ui, pr)
    }
}

fn with_state(f: impl FnOnce(&mut State)) {
    S.with(|s| {
        let mut g = s.borrow_mut();
        let st = g.get_or_insert_with(|| {
            let (ui_cpu, proc_cpu) = cpu_times();
            State { since: Instant::now(), wakes: 0, why: BTreeMap::new(), wins: BTreeMap::new(), ui_cpu, proc_cpu }
        });
        f(st);
        if st.since.elapsed() >= if env_on() { REPORT_EVERY } else { SAMPLE_EVERY } {
            report(st);
        }
    });
}

fn report(st: &mut State) {
    let secs = st.since.elapsed().as_secs_f64();
    let (ui, pr) = cpu_times();
    let pct = |d: Duration| d.as_secs_f64() / secs * 100.0;
    let isl = st.wins.get(ISLAND);
    let per_ms = |a: &Acc| (a.step + a.draw + a.present).as_secs_f64() * 1000.0 / a.frames.max(1) as f64;
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some(Sample {
        at: Instant::now(),
        secs: secs as f32,
        island_fps: isl.map_or(0.0, |a| (a.frames as f64 / secs) as f32),
        island_ms: isl.map_or(0.0, |a| per_ms(a) as f32),
        ui_cpu: pct(ui.saturating_sub(st.ui_cpu)) as f32,
        proc_cpu: pct(pr.saturating_sub(st.proc_cpu)) as f32,
    });
    if !env_on() {
        *st = State { since: Instant::now(), wakes: 0, why: BTreeMap::new(), wins: BTreeMap::new(), ui_cpu: ui, proc_cpu: pr };
        return;
    }
    let mut line = format!("perf {secs:.1}s: wakes {} | ui thread {:.1}% cpu, process {:.1}% cpu", st.wakes, pct(ui.saturating_sub(st.ui_cpu)), pct(pr.saturating_sub(st.proc_cpu)));
    for (label, a) in &st.wins {
        if a.frames == 0 {
            continue;
        }
        let per = |d: Duration| d.as_secs_f64() * 1000.0 / a.frames as f64;
        line += &format!(" | {label}: {} frames ({:.0}/s), ms/frame step {:.2} draw {:.2} present {:.2}", a.frames, a.frames as f64 / secs, per(a.step), per(a.draw), per(a.present));
    }
    if !st.why.is_empty() {
        line += " | frame requested by:";
        for (k, n) in &st.why {
            line += &format!(" {k}={n}");
        }
    }
    eprintln!("{line}");
    *st = State { since: Instant::now(), wakes: 0, why: BTreeMap::new(), wins: BTreeMap::new(), ui_cpu: ui, proc_cpu: pr };
}

/// One drawn frame of window `label`.
pub fn frame(label: &str, step: Duration, draw: Duration, present: Duration) {
    if !on() {
        return;
    }
    with_state(|st| {
        let a = st.wins.entry(label.to_string()).or_default();
        a.frames += 1;
        a.step += step;
        a.draw += draw;
        a.present += present;
    });
}

/// The reasons (name, is it true now) why a view asked for another frame; counts the true ones.
pub fn why(reasons: &[(&'static str, bool)]) {
    if on() {
        with_state(|st| {
            for (k, v) in reasons {
                if *v {
                    *st.why.entry(k).or_default() += 1;
                }
            }
        });
    }
}

/// The loop woke up (message, timer or frame signal).
pub fn wake() {
    if on() {
        with_state(|st| st.wakes += 1);
    }
}
