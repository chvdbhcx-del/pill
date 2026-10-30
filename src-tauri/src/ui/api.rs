// In-process command facade for the native UI (no WebView): `invoke('name', {camelCase})` as a plain function call. See docs/native-api.md.
//   call(cmd, args, caller)            synchronous, for WORKER threads (it blocks on `async fn` commands); result = what `await invoke(..)` resolves with
//   call_raw(cmd, args, caller, body)  the one command with a raw request body (`capture_commit` with a marked-up PNG; `args` is the `x-pill-args` JSON)
//   call_async(cmd, args, caller, done) runs `call` on the worker pool and hands the result to `done` (on that worker thread)
//   dispatch_calls(calls, caller, on_reply) the renderers' helper: a batch of (cmd, args) pairs, each answered with (index, result)
//   capture_frame(label)               the frozen monitor frame (raw RGBA) without a JSON detour
// Events come from `ui::bus::subscribe`. The `#[tauri::command]` functions are untouched: the facade calls the very same functions.
// Never call any of this from the native UI thread or from inside the tokio runtime (it blocks); use `call_async` / `dispatch_calls` there.
// Arguments are never logged (they can carry API keys and passwords).
#![allow(dead_code)] // the native UI (later phases) is the caller; until then only the tests are

mod args;
mod dispatch;
mod pool;

#[allow(unused_imports)] // for the native renderers
pub use args::{to_camel, to_snake};

use serde_json::Value;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, OnceLock};
use tauri::AppHandle;

pub type Reply = Result<Value, String>;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Store the app handle the commands run with. Call once from `setup()`; later calls are ignored.
pub fn init(app: &AppHandle) {
    let _ = APP.set(app.clone());
}

fn guarded(cmd: &str, f: impl FnOnce(&AppHandle) -> Reply) -> Reply {
    let Some(app) = APP.get() else { return Err("ui::api is not initialised (call ui::api::init first)".into()) };
    match catch_unwind(AssertUnwindSafe(|| f(app))) {
        Ok(r) => r,
        Err(p) => {
            let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown".into());
            Err(format!("panic: command `{cmd}`: {msg}"))
        }
    }
}

/// Run a command like `invoke(cmd, args)` does. `args` is the JS argument object (camelCase keys; `Value::Null` for none). `caller` is the label of the
/// native window that asks (`pill`, `bar`, `settings`, `capture-N`); no command reads it today. A panic inside the command comes back as `Err("panic: ..")`.
/// Blocks: worker threads only.
pub fn call(cmd: &str, args: Value, caller: &str) -> Reply {
    guarded(cmd, |app| dispatch::dispatch(app, cmd, &args, caller, None))
}

/// `call` for a request with a raw body: today only `capture_commit` (the PNG of the marked-up area; `args` = the JSON of header `x-pill-args`).
pub fn call_raw(cmd: &str, args: Value, caller: &str, body: Vec<u8>) -> Reply {
    guarded(cmd, |app| dispatch::dispatch(app, cmd, &args, caller, Some(body)))
}

/// The frozen frame of one monitor of the running capture session (w*h*4 bytes RGBA, `capture_session` has w/h), shared, not copied.
pub fn capture_frame(label: &str) -> Option<Arc<Vec<u8>>> {
    crate::capture::capture_frame_arc(label)
}

/// `call` on the worker pool; `done` runs on the worker thread that finished the command (so keep it short: post a message to the UI thread).
pub fn call_async(cmd: &str, args: Value, caller: &str, done: impl FnOnce(Reply) + Send + 'static) {
    let (cmd, caller) = (cmd.to_string(), caller.to_string());
    pool::submit(move || done(call(&cmd, args, &caller)));
}

/// Same for a raw-body call.
pub fn call_raw_async(cmd: &str, args: Value, caller: &str, body: Vec<u8>, done: impl FnOnce(Reply) + Send + 'static) {
    let (cmd, caller) = (cmd.to_string(), caller.to_string());
    pool::submit(move || done(call_raw(&cmd, args, &caller, body)));
}

/// The helper the renderers use for the `BackendCall`s their models emit: every `(cmd, args)` pair starts at once on the pool and its result
/// arrives as `on_reply(index_in_calls, result)` (from a worker thread, in completion order, not call order). The model's own ticket / tag /
/// `as_event` stays with the renderer, keyed by that index (docs/native-api.md says how each ui-core `BackendCall` maps).
pub fn dispatch_calls(calls: impl IntoIterator<Item = (String, Value)>, caller: &str, on_reply: impl Fn(usize, Reply) + Send + Sync + 'static) {
    let on_reply = Arc::new(on_reply);
    for (i, (cmd, args)) in calls.into_iter().enumerate() {
        let f = on_reply.clone();
        call_async(&cmd, args, caller, move |r| f(i, r));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::Path;

    const DISPATCH_SRC: &str = include_str!("api/dispatch.rs");
    const MAIN_SRC: &str = include_str!("../main.rs");
    const CONTRACT: &str = include_str!("../../../docs/native-ui-contract.md");

    /// `"name" =>` arms of the command table.
    fn arms() -> BTreeSet<String> {
        DISPATCH_SRC
            .lines()
            .filter_map(|l| {
                let l = l.trim_start().strip_prefix('"')?;
                let (name, rest) = l.split_once('"')?;
                (rest.trim_start().starts_with("=>") && !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
                    .then(|| name.to_string())
            })
            .collect()
    }

    /// Command names of the contract table: the `### Commands` section, first backticked cell of each row.
    fn contract_commands() -> BTreeSet<String> {
        let sec = CONTRACT.split("## Commands").nth(1).expect("Commands section");
        sec.lines()
            .filter(|l| l.starts_with("| `"))
            .filter_map(|l| l.trim_start_matches("| `").split('`').next().map(str::to_string))
            .collect()
    }

    fn contract_events() -> BTreeSet<String> {
        let sec = CONTRACT.split("## Events").nth(1).and_then(|s| s.split("## Commands").next()).expect("Events section");
        sec.lines()
            .filter(|l| l.starts_with("| `"))
            .filter_map(|l| l.trim_start_matches("| `").split('`').next().map(str::to_string))
            .collect()
    }

    /// Names inside `generate_handler![ .. ]` of main.rs (`module::name`).
    fn handler_commands() -> BTreeSet<String> {
        let start = MAIN_SRC.find("generate_handler![").expect("generate_handler") + "generate_handler![".len();
        let body = &MAIN_SRC[start..MAIN_SRC[start..].find(']').map(|e| start + e).unwrap()];
        body.lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .flat_map(|l| l.split(',').map(str::trim).map(str::to_string).collect::<Vec<_>>())
            .filter_map(|t| t.rsplit_once("::").map(|(_, n)| n.to_string()))
            .filter(|n| !n.is_empty())
            .collect()
    }

    #[test]
    fn every_contract_command_has_a_dispatch_arm() {
        let (contract, arms) = (contract_commands(), arms());
        assert!(contract.len() >= 170, "contract parse looks broken: {}", contract.len());
        let missing: Vec<_> = contract.difference(&arms).collect();
        assert!(missing.is_empty(), "contract commands without an arm in ui/api/dispatch.rs: {missing:?}");
    }

    #[test]
    fn every_arm_is_registered_in_main() {
        let (handler, arms) = (handler_commands(), arms());
        assert!(handler.len() >= 170, "handler parse looks broken: {}", handler.len());
        let stray: Vec<_> = arms.difference(&handler).collect();
        assert!(stray.is_empty(), "dispatch arms that main.rs invoke_handler does not register: {stray:?}");
        let unreachable: Vec<_> = handler.difference(&arms).collect();
        assert!(unreachable.is_empty(), "registered commands without an arm: {unreachable:?}");
    }

    #[test]
    fn contract_and_handler_agree() {
        assert_eq!(contract_commands(), handler_commands(), "docs/native-ui-contract.md is stale: regenerate it");
    }

    #[test]
    fn no_arm_is_listed_twice() {
        let n = DISPATCH_SRC.lines().filter(|l| l.trim_start().starts_with('"') && l.contains("\" =>")).count();
        assert_eq!(n, arms().len(), "a command has two arms (the second would never run)");
    }

    #[test]
    fn calls_before_init_are_errors_not_panics() {
        // the unit tests never start an app, so the handle is unset here
        if APP.get().is_none() {
            let e = call("settings_get", Value::Null, "pill").unwrap_err();
            assert!(e.contains("not initialised"), "{e}");
            assert!(call_raw("capture_commit", Value::Null, "capture-0", vec![1]).is_err());
        }
    }

    #[test]
    fn call_async_and_dispatch_calls_answer_every_call() {
        use std::sync::mpsc::channel;
        use std::sync::Mutex;
        use std::time::Duration;
        let (tx, rx) = channel();
        call_async("pill_toggle", Value::Null, "pill", move |r| tx.send(r).unwrap());
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap().is_err());

        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        dispatch_calls(vec![("a".to_string(), Value::Null), ("b".to_string(), Value::Null), ("c".to_string(), Value::Null)], "bar", move |i, r| {
            tx.lock().unwrap().send((i, r.is_err())).unwrap();
        });
        let mut got: Vec<_> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(5)).unwrap()).collect();
        got.sort();
        assert_eq!(got, vec![(0, true), (1, true), (2, true)]);
    }

    #[test]
    fn a_panic_in_a_command_is_an_error() {
        let r = catch_unwind(AssertUnwindSafe(|| -> Reply { panic!("kaput") }));
        assert!(r.is_err(), "sanity: the closure panics");
        // the same wrapping `guarded` uses, with the handle check bypassed
        let msg = match catch_unwind(AssertUnwindSafe(|| -> Reply { panic!("kaput") })) {
            Ok(r) => r,
            Err(p) => Err(format!("panic: {}", p.downcast_ref::<&str>().copied().unwrap_or("?"))),
        };
        assert_eq!(msg, Err("panic: kaput".to_string()));
    }

    /// Every event name of the contract is produced by some `bus::emit` path, i.e. appears as a string literal in a non-UI source file. A name that
    /// only a `app.emit*` call used would not reach `bus::subscribe`; the second test below greps for such calls.
    #[test]
    fn every_contract_event_has_a_producer_literal() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut text = String::new();
        for e in std::fs::read_dir(&src).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "rs") {
                text.push_str(&std::fs::read_to_string(&p).unwrap());
            }
        }
        let events = contract_events();
        assert!(events.len() >= 56, "event parse looks broken: {}", events.len());
        let missing: Vec<_> = events.iter().filter(|e| !text.contains(&format!("\"{e}\""))).collect();
        assert!(missing.is_empty(), "contract events nobody emits: {missing:?}");
    }

    #[test]
    fn no_event_bypasses_the_bus() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut bad = Vec::new();
        for e in std::fs::read_dir(&src).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "rs") {
                for (n, l) in std::fs::read_to_string(&p).unwrap().lines().enumerate() {
                    let code = l.split("//").next().unwrap_or("");
                    if [".emit(", ".emit_to(", ".emit_str(", ".emit_str_to(", ".emit_filter("].iter().any(|k| code.contains(k)) {
                        bad.push(format!("{}:{}", p.file_name().unwrap().to_string_lossy(), n + 1));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "events sent around ui::bus (native subscribers would miss them): {bad:?}");
    }
}
