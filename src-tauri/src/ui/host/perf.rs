//! `PILL_PERF=1`: every 5 s the host writes one line to stderr (the log file in a release build): per window how many frames were drawn and
//! what a frame cost (step / draw / present), how many times the loop woke up, and how much CPU the UI thread and the whole process used.
//! Off by default; when off, the stopwatches below do nothing.
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes};

const REPORT_EVERY: Duration = Duration::from_secs(5);

pub fn on() -> bool {
    static T: OnceLock<bool> = OnceLock::new();
    *T.get_or_init(|| std::env::var_os("PILL_PERF").is_some_and(|v| v != "0"))
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
        if st.since.elapsed() >= REPORT_EVERY {
            report(st);
        }
    });
}

fn report(st: &mut State) {
    let secs = st.since.elapsed().as_secs_f64();
    let (ui, pr) = cpu_times();
    let pct = |d: Duration| d.as_secs_f64() / secs * 100.0;
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
