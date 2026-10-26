// Windows notifications -> island cards (GateGuard note: new file of the Pill app, used by main.rs + src/live-ext.js + src/settings-live-ext.js;
// user request: "делай все оставшееся", part: Live-events producers OBS/Steam/Notifications).
// WinRT UserNotificationListener (Windows.UI.Notifications.Management), polled every 2 s while live.notifications is on AND Windows granted access.
// READ-ONLY: the original Windows notification is never removed or changed, so Windows may still show its own popup (Focus Assist hides it).
// Windows normally gives this API only to apps with package identity: from an unpackaged exe the calls may fail -> status "unsupported", the worker ends.
// Status for Settings: granted | denied | pending (not asked yet) | unsupported | off. Event to window "pill": `live:notification` {app, title, body, id}.
// PRIVACY: titles and bodies are only forwarded to our own page (shown as text), never logged, stored or sent anywhere.
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Listener};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::NotificationKinds;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

const POLL_MS: u64 = 2_000;
const MAX_PER_POLL: usize = 5;

// ---------------------------------------------------------------- pure parts (unit-tested)
/// Control and bidi/zero-width characters out, whitespace collapsed, capped at `max` chars with an ellipsis.
pub fn clean(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .filter(|c| !matches!(*c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}'))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let out = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > max {
        out.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}"
    } else {
        out
    }
}

/// Ids not seen before. The first call only learns the notifications that already sit in the Action Center (`primed` false -> nothing is new).
pub fn fresh(seen: &mut HashSet<u32>, primed: &mut bool, now: &[u32]) -> Vec<u32> {
    let new: Vec<u32> = if *primed { now.iter().copied().filter(|i| !seen.contains(i)).collect() } else { Vec::new() };
    *seen = now.iter().copied().collect();
    *primed = true;
    new
}

// ---------------------------------------------------------------- lifecycle
static GEN: AtomicU64 = AtomicU64::new(0);
static ACTIVE: Mutex<bool> = Mutex::new(false);
static STATUS: Mutex<&'static str> = Mutex::new("off");

fn set_status(s: &'static str) {
    if let Ok(mut g) = STATUS.lock() {
        *g = s;
    }
}

fn sync(app: &AppHandle) {
    let want = crate::settings::get_bool("live.notifications", true);
    let mut a = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
    if *a == want {
        return;
    }
    *a = want;
    let g = crate::wake::bump(&GEN);
    if want {
        let app = app.clone();
        std::thread::spawn(move || run(app, g));
    } else {
        set_status("off");
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let h2 = h.clone();
    h.listen("settings:changed", move |_| sync(&h2));
    sync(&h);
}

/// Asks Windows for the access status; `request` shows the system consent prompt (only from the Settings button).
fn access(request: bool) -> &'static str {
    let Ok(l) = UserNotificationListener::Current() else { return "unsupported" };
    let Ok(st) = l.GetAccessStatus() else { return "unsupported" };
    let map = |s: UserNotificationListenerAccessStatus| match s {
        UserNotificationListenerAccessStatus::Allowed => "granted",
        UserNotificationListenerAccessStatus::Denied => "denied",
        _ => "pending",
    };
    let cur = map(st);
    if cur != "pending" || !request {
        return cur;
    }
    match l.RequestAccessAsync().and_then(|op| op.get()) {
        Ok(s) => match map(s) {
            "pending" => "unsupported", // asked, Windows gave no decision: the API is not available to this unpackaged app
            r => r,
        },
        Err(_) => "unsupported",
    }
}

fn read_one(n: &windows::UI::Notifications::UserNotification) -> Option<Value> {
    let app = n.AppInfo().ok()?.DisplayInfo().ok()?.DisplayName().ok()?.to_string();
    let bindings = n.Notification().ok()?.Visual().ok()?.Bindings().ok()?;
    let b = bindings.clone().into_iter().find(|b| b.Template().map(|t| t.to_string() == "ToastGeneric").unwrap_or(false)).or_else(|| bindings.GetAt(0).ok())?;
    let texts: Vec<String> = b.GetTextElements().ok()?.into_iter().filter_map(|t| t.Text().ok().map(|s| s.to_string())).collect();
    let title = clean(texts.first().map(String::as_str).unwrap_or(""), 80);
    let body = clean(&texts.iter().skip(1).cloned().collect::<Vec<_>>().join(" "), 200);
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(json!({ "app": clean(&app, 40), "title": title, "body": body, "id": n.Id().ok()? }))
}

fn poll(l: &UserNotificationListener, seen: &mut HashSet<u32>, primed: &mut bool) -> Option<Vec<Value>> {
    let list = l.GetNotificationsAsync(NotificationKinds::Toast).ok()?.get().ok()?;
    let items: Vec<_> = list.into_iter().collect();
    let ids: Vec<u32> = items.iter().filter_map(|n| n.Id().ok()).collect();
    let new = fresh(seen, primed, &ids);
    Some(items.iter().filter(|n| n.Id().ok().is_some_and(|i| new.contains(&i))).take(MAX_PER_POLL).filter_map(read_one).collect())
}

fn nap(gen: u64, ms: u64) -> bool {
    crate::wake::nap(&GEN, gen, ms)
}

fn run(app: AppHandle, gen: u64) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let (mut seen, mut primed) = (HashSet::new(), false);
    while GEN.load(Ordering::SeqCst) == gen {
        let st = access(false);
        if GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        set_status(st);
        match st {
            "unsupported" => return, // nothing to poll; the Settings button re-tests
            "granted" => {
                let Ok(l) = UserNotificationListener::Current() else { return };
                if let Some(v) = poll(&l, &mut seen, &mut primed) {
                    for p in v {
                        let _ = app.emit_to("pill", "live:notification", p);
                    }
                }
                if !nap(gen, POLL_MS) {
                    return;
                }
            }
            _ => {
                primed = false; // after a grant, what is already in the Action Center is not "new"
                if !nap(gen, 10_000) {
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- commands
#[tauri::command(async)]
pub fn notif_status() -> Value {
    let st = *STATUS.lock().unwrap_or_else(|e| e.into_inner());
    json!({ "status": st })
}

/// The Settings button: asks Windows (system prompt if it is still undecided), then restarts the worker so it picks the result up.
#[tauri::command(async)]
pub fn notif_request(app: AppHandle) -> Value {
    let st = access(true);
    if crate::settings::get_bool("live.notifications", true) {
        set_status(st);
        let g = crate::wake::bump(&GEN);
        if st != "unsupported" {
            let app = app.clone();
            std::thread::spawn(move || run(app, g));
        }
    }
    json!({ "status": st })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_and_caps() {
        assert_eq!(clean("  a\u{202e}b\n\tc \u{0007}d ", 50), "ab c d");
        assert_eq!(clean("abcdef", 4), "abc\u{2026}");
        assert_eq!(clean("", 4), "");
    }

    #[test]
    fn first_poll_primes_then_only_new() {
        let (mut seen, mut primed) = (HashSet::new(), false);
        assert!(fresh(&mut seen, &mut primed, &[1, 2]).is_empty());
        assert_eq!(fresh(&mut seen, &mut primed, &[1, 2, 3]), vec![3]);
        assert!(fresh(&mut seen, &mut primed, &[1, 2, 3]).is_empty());
        assert_eq!(fresh(&mut seen, &mut primed, &[3, 4, 5]), vec![4, 5]);
    }
}
