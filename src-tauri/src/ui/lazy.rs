// Lazily created windows with idle teardown: the window exists only while it is in use (and for `idle_secs` after it was hidden), so its WebView2
// renderer is not kept in memory for nothing. Today one entry: the command bar (`bar`).
//   get_or_create(app, label)   the live window, built (hidden) when it does not exist. Call it from a worker thread: building a WebView2 window
//                               on the event-loop thread after startup can deadlock (settingswin.rs, capture.rs do the same).
//   try_get(app, label)         the live window if any, never blocks (safe on the main thread)
//   note_hidden / note_shown    the owner tells when the window went away from the screen / came back; hiding arms the idle timer
// The timer is one thread per label at a time. It re-reads the idle time from the spec on every wake-up (0 = never destroy), skips the teardown
// while the window is visible or `pinned()` says it must stay (an uninstall job lives in the bar page), and otherwise `destroy()`s the window.
use crate::ui::windows;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, WebviewWindow};

/// How often a pinned window is looked at again once its idle time is over.
const PINNED_RETRY: Duration = Duration::from_secs(15);
/// How long destroy() waits for the window to really leave the window list before the next creation may start.
const GONE_WAIT: Duration = Duration::from_millis(1500);

struct Spec {
    label: &'static str,
    build: fn(&AppHandle) -> tauri::Result<WebviewWindow>,
    /// Called on every freshly built window (event handlers).
    attach: fn(&AppHandle, &WebviewWindow),
    /// Called before a build: the owner forgets everything it knew about the previous page (e.g. "page is ready").
    reset: fn(),
    /// Seconds hidden before teardown; 0 = never.
    idle_secs: fn() -> u64,
    /// True while the window must not be destroyed even when hidden.
    pinned: fn() -> bool,
}

const SPECS: &[Spec] = &[Spec {
    label: windows::BAR,
    build: windows::build_bar,
    attach: crate::command::attach_bar,
    reset: crate::command::bar_reset,
    idle_secs: crate::command::lazy_bar_secs,
    pinned: crate::uninstall::job_active,
}];

fn spec(label: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.label == label)
}

/// Serializes build and destroy of lazy windows (two creations of one label would fail, a creation must not overtake a teardown).
static GATE: Mutex<()> = Mutex::new(());

#[derive(Default)]
struct Idle {
    hidden_at: Option<Instant>,
    armed: bool,
}
static IDLE: Mutex<Option<HashMap<&'static str, Idle>>> = Mutex::new(None);

fn lk<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn with_idle<R>(label: &'static str, f: impl FnOnce(&mut Idle) -> R) -> R {
    f(lk(&IDLE).get_or_insert_with(HashMap::new).entry(label).or_default())
}

/// The live window, if there is one. Never blocks: when a build or teardown is in progress it answers None (the caller takes its slow path).
pub fn try_get(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    let _g = GATE.try_lock().ok()?;
    app.get_webview_window(label)
}

/// The live window, built hidden when there is none. May block (it waits for a running teardown and for the build): worker threads only.
pub fn get_or_create(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    let s = spec(label)?;
    if label == windows::BAR && crate::ui::bar_view::bar_is_native() {
        return None; // the command bar is a native window in this run: no WebView is built for it
    }
    let _g = lk(&GATE);
    if let Some(w) = app.get_webview_window(s.label) {
        return Some(w);
    }
    (s.reset)();
    match (s.build)(app) {
        Ok(w) => {
            (s.attach)(app, &w);
            Some(w)
        }
        Err(e) => {
            eprintln!("ui: cannot create window {}: {e}", s.label);
            None
        }
    }
}

/// The window left the screen: start counting its idle time.
pub fn note_hidden(app: &AppHandle, label: &str) {
    let Some(s) = spec(label) else { return };
    let start = with_idle(s.label, |i| {
        i.hidden_at = Some(Instant::now());
        !std::mem::replace(&mut i.armed, true)
    });
    if !start {
        return; // the running timer thread sees the new hidden_at on its next wake-up
    }
    let app = app.clone();
    std::thread::spawn(move || watch(&app, s));
}

/// The window is on screen (or being opened): the idle count is off.
pub fn note_shown(label: &str) {
    if let Some(s) = spec(label) {
        with_idle(s.label, |i| i.hidden_at = None);
    }
}

#[derive(Debug, PartialEq)]
enum Step {
    Stop,
    Sleep(Duration),
    Destroy,
}

/// What the idle thread does next. `hidden_for` is None when the window was shown again meanwhile.
fn next_step(hidden_for: Option<Duration>, idle_secs: u64, visible: bool, pinned: bool) -> Step {
    let Some(hidden_for) = hidden_for else { return Step::Stop };
    if idle_secs == 0 || visible {
        return Step::Stop;
    }
    let limit = Duration::from_secs(idle_secs);
    if hidden_for < limit {
        return Step::Sleep(limit - hidden_for);
    }
    if pinned {
        return Step::Sleep(PINNED_RETRY);
    }
    Step::Destroy
}

fn watch(app: &AppHandle, s: &'static Spec) {
    loop {
        let hidden_for = with_idle(s.label, |i| i.hidden_at.map(|t| t.elapsed()));
        let visible = app.get_webview_window(s.label).is_some_and(|w| w.is_visible().unwrap_or(false));
        let step = next_step(hidden_for, (s.idle_secs)(), visible, (s.pinned)());
        match step {
            Step::Sleep(d) => std::thread::sleep(d),
            Step::Stop => {
                with_idle(s.label, |i| i.armed = false);
                return;
            }
            Step::Destroy => {
                let _g = lk(&GATE);
                // re-check under the gate: a creation or a show may have raced the decision
                let shown_again = with_idle(s.label, |i| i.hidden_at.is_none());
                let visible = app.get_webview_window(s.label).is_some_and(|w| w.is_visible().unwrap_or(false));
                if !shown_again && !visible {
                    if let Some(w) = app.get_webview_window(s.label) {
                        let _ = w.destroy();
                        let t0 = Instant::now();
                        while app.get_webview_window(s.label).is_some() && t0.elapsed() < GONE_WAIT {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                    }
                }
                with_idle(s.label, |i| {
                    i.armed = false;
                    i.hidden_at = None;
                });
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const S: fn(u64) -> Duration = Duration::from_secs;

    #[test]
    fn idle_steps() {
        assert_eq!(next_step(None, 120, false, false), Step::Stop); // shown again
        assert_eq!(next_step(Some(S(1)), 0, false, false), Step::Stop); // 0 = never destroy
        assert_eq!(next_step(Some(S(500)), 120, true, false), Step::Stop); // on screen
        assert_eq!(next_step(Some(S(20)), 120, false, false), Step::Sleep(S(100))); // not yet: sleep the remainder
        assert_eq!(next_step(Some(S(120)), 120, false, false), Step::Destroy);
        assert_eq!(next_step(Some(S(900)), 120, false, true), Step::Sleep(PINNED_RETRY)); // pinned (uninstall job): look again later
    }

    #[test]
    fn spec_lookup() {
        assert!(spec("bar").is_some());
        assert!(spec("pill").is_none());
    }
}
