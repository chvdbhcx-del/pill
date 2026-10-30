// AI chat: threads + messages in SQLite (tables chat_threads / chat_messages, migration at the store.rs marker) and the
// streaming orchestration against the user's OpenAI-compatible endpoint. Owner: net/chat agent.
// All sockets are in net.rs; this file never touches the network itself. History is local; the key is DPAPI-wrapped.
// Events to the page: chat:delta {thread, run, text}, chat:done {thread, run, ok, stopped, error, id}.
use crate::ui::bus;
use crate::net;
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(1);
static RUNS: Mutex<Option<HashMap<i64, (u64, net::Abort)>>> = Mutex::new(None); // thread -> (run id, abort)

const MAX_TEXT: usize = 100_000; // chars per message
const MAX_CONTEXT: usize = 120_000; // chars sent per request (oldest messages dropped first)

pub fn init(app: &tauri::App) {
    let _ = APP.set(app.handle().clone());
}

fn emit(name: &str, payload: Value) {
    if let Some(_a) = APP.get() {
        let _ = bus::emit(name, payload);
    }
}

fn runs<R>(f: impl FnOnce(&mut HashMap<i64, (u64, net::Abort)>) -> R) -> R {
    let mut g = RUNS.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(HashMap::new))
}

// ------------------------------------------------------------------------------------------------ storage

fn new_thread(title: &str) -> Option<i64> {
    let t: String = title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(48).collect();
    crate::store::with(|c| {
        c.execute("insert into chat_threads(title, created_at) values (?1, ?2)", params![t, crate::store::now_ms()])?;
        Ok(c.last_insert_rowid())
    })
}

fn thread_exists(id: i64) -> bool {
    crate::store::with(|c| c.query_row("select count(*) from chat_threads where id = ?1", params![id], |r| r.get::<_, i64>(0))).unwrap_or(0) > 0
}

fn add_message(thread: i64, role: &str, content: &str) -> Option<Value> {
    let now = crate::store::now_ms();
    let id = crate::store::with(|c| {
        c.execute("insert into chat_messages(thread_id, role, content, created_at) values (?1, ?2, ?3, ?4)", params![thread, role, content, now])?;
        Ok(c.last_insert_rowid())
    })?;
    Some(json!({ "id": id, "role": role, "content": content, "created_at": now }))
}

fn history(thread: i64) -> Vec<(String, String)> {
    crate::store::with(|c| {
        let mut st = c.prepare("select role, content from chat_messages where thread_id = ?1 order by id")?;
        let rows = st.query_map(params![thread], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap_or_default()
}

// ------------------------------------------------------------------------------------------------ SSE

#[derive(Debug, PartialEq)]
enum Sse {
    Text(String),
    Done,
    Err(String),
    Other,
}

fn error_text(v: &Value) -> Option<String> {
    let e = v.get("error")?;
    Some(e.get("message").and_then(Value::as_str).or_else(|| e.as_str()).unwrap_or("the server reported an error").to_string())
}

fn parse_sse_line(line: &str) -> Sse {
    let Some(d) = line.trim().strip_prefix("data:") else { return Sse::Other };
    let d = d.trim();
    if d == "[DONE]" {
        return Sse::Done;
    }
    let Ok(v) = serde_json::from_str::<Value>(d) else { return Sse::Other };
    if let Some(m) = error_text(&v) {
        return Sse::Err(m);
    }
    match v.pointer("/choices/0/delta/content").or_else(|| v.pointer("/choices/0/text")).and_then(Value::as_str) {
        Some(t) if !t.is_empty() => Sse::Text(t.to_string()),
        _ => Sse::Other, // role-only first chunk, reasoning deltas, finish_reason, usage...
    }
}

/// "HTTP 401: Incorrect API key" from an error response.
pub fn http_error(r: &net::Resp) -> String {
    let text = r.text();
    let msg = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| error_text(&v).or_else(|| v.get("detail").and_then(Value::as_str).map(str::to_string)).or_else(|| v.get("message").and_then(Value::as_str).map(str::to_string)))
        .unwrap_or_else(|| text.chars().take(200).collect());
    let hint = match r.status {
        401 | 403 => " (check the API key)",
        404 => " (check the API address and the model name)",
        429 => " (rate limit)",
        _ => "",
    };
    format!("HTTP {}: {}{hint}", r.status, msg.trim())
}

// ------------------------------------------------------------------------------------------------ run

fn start_run(thread: i64, cfg: net::AiCfg) -> Result<u64, String> {
    let mut msgs: Vec<Value> = Vec::new();
    let sys = crate::settings::get_str("ai.system", "");
    if !sys.trim().is_empty() {
        msgs.push(json!({ "role": "system", "content": sys }));
    }
    let hist = history(thread);
    let mut budget = MAX_CONTEXT;
    let mut keep: Vec<&(String, String)> = Vec::new();
    for m in hist.iter().rev() {
        let n = m.1.chars().count();
        if n > budget && !keep.is_empty() {
            break;
        }
        budget = budget.saturating_sub(n);
        keep.push(m);
    }
    for m in keep.into_iter().rev() {
        msgs.push(json!({ "role": m.0, "content": m.1 }));
    }
    let run = SEQ.fetch_add(1, Ordering::Relaxed);
    let abort = net::Abort::new();
    let busy = runs(|r| {
        if r.contains_key(&thread) {
            true
        } else {
            r.insert(thread, (run, abort.clone()));
            false
        }
    });
    if busy {
        return Err("This chat is still answering. Stop it first.".into());
    }
    std::thread::spawn(move || run_chat(thread, run, cfg, msgs, abort));
    Ok(run)
}

fn run_chat(thread: i64, run: u64, cfg: net::AiCfg, messages: Vec<Value>, abort: net::Abort) {
    let body = json!({ "model": cfg.model, "messages": messages, "stream": true }).to_string();
    let mut acc = String::new();
    let mut pending = String::new();
    let mut last_emit = Instant::now();
    let mut buf: Vec<u8> = Vec::new();
    let mut other: Vec<u8> = Vec::new();
    let mut stream_err: Option<String> = None;

    let mut line_in = |line: &[u8], acc: &mut String, pending: &mut String| {
        let s = String::from_utf8_lossy(line);
        let s = s.trim_end();
        if s.is_empty() || s.starts_with(':') {
            return;
        }
        match parse_sse_line(s) {
            Sse::Text(t) => {
                acc.push_str(&t);
                pending.push_str(&t);
            }
            Sse::Done => {}
            Sse::Err(m) => stream_err = Some(m),
            Sse::Other => {
                if other.len() < (1 << 20) {
                    other.extend_from_slice(line); // maybe a plain JSON answer from a server that ignored stream:true
                }
            }
        }
    };

    let res = {
        let mut cb = |chunk: &[u8]| -> bool {
            buf.extend_from_slice(chunk);
            while let Some(p) = buf.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buf.drain(..=p).collect();
                line_in(&line, &mut acc, &mut pending);
            }
            if !pending.is_empty() && last_emit.elapsed() > Duration::from_millis(16) {
                emit("chat:delta", json!({ "thread": thread, "run": run, "text": std::mem::take(&mut pending) }));
                last_emit = Instant::now();
            }
            !abort.is_set()
        };
        let r = net::ai_chat(&cfg, body.as_bytes(), 300_000, Some(&abort), Some(&mut cb));
        r
    };
    if !buf.is_empty() {
        let rest = std::mem::take(&mut buf);
        line_in(&rest, &mut acc, &mut pending);
    }

    let stopped = abort.is_set();
    let mut error: Option<String> = None;
    match &res {
        Ok(r) if r.status / 100 == 2 => {
            if acc.is_empty() {
                // not an SSE stream: accept a plain chat.completion JSON
                if let Ok(v) = serde_json::from_slice::<Value>(&other) {
                    if let Some(m) = error_text(&v) {
                        error = Some(m);
                    } else if let Some(t) = v.pointer("/choices/0/message/content").and_then(Value::as_str) {
                        acc = t.to_string();
                        pending = t.to_string();
                    }
                }
            }
        }
        Ok(r) => error = Some(http_error(r)),
        Err(e) if !stopped => error = Some(e.clone()),
        Err(_) => {}
    }
    if error.is_none() && acc.is_empty() && !stopped {
        error = Some(stream_err.take().unwrap_or_else(|| "The model returned no text.".into()));
    }
    if !pending.is_empty() {
        emit("chat:delta", json!({ "thread": thread, "run": run, "text": pending }));
    }
    let mut id = Value::Null;
    if !acc.trim().is_empty() {
        let text: String = acc.chars().take(MAX_TEXT).collect();
        if let Some(m) = add_message(thread, "assistant", &text) {
            id = m["id"].clone();
        }
    }
    runs(|r| {
        if r.get(&thread).is_some_and(|(id, _)| *id == run) {
            r.remove(&thread);
        }
    });
    emit("chat:done", json!({ "thread": thread, "run": run, "ok": error.is_none(), "stopped": stopped, "error": error, "id": id }));
}

// ------------------------------------------------------------------------------------------------ commands

/// Send a user message and start streaming the answer. thread = null/unknown creates a new chat.
pub fn chat_send(thread: Option<i64>, text: String, model: Option<String>) -> Value {
    let text: String = text.trim().chars().take(MAX_TEXT).collect();
    if text.is_empty() {
        return json!({ "ok": false, "error": "Empty message." });
    }
    let cfg = match net::ai_cfg(model.as_deref(), true) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let existing = thread.filter(|t| thread_exists(*t));
    if let Some(t) = existing {
        if runs(|r| r.contains_key(&t)) {
            return json!({ "ok": false, "error": "This chat is still answering. Stop it first." });
        }
    }
    let Some(tid) = existing.or_else(|| new_thread(&text)) else { return json!({ "ok": false, "error": "Cannot open the chat database." }) };
    if existing.is_some() {
        // a thread created by chat_new has an empty title: name it after its first message
        let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(48).collect();
        crate::store::with(|c| c.execute("update chat_threads set title = ?2 where id = ?1 and title = ''", params![tid, t]));
    }
    let Some(user) = add_message(tid, "user", &text) else { return json!({ "ok": false, "error": "Cannot save the message." }) };
    match start_run(tid, cfg) {
        Ok(run) => json!({ "ok": true, "thread": tid, "run": run, "user": user }),
        Err(e) => json!({ "ok": false, "thread": tid, "error": e, "user": user }),
    }
}

/// Re-ask the last user message: drops the assistant answer(s) after it and streams a new one.
pub fn chat_retry(thread: i64, model: Option<String>) -> Value {
    let cfg = match net::ai_cfg(model.as_deref(), true) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    if runs(|r| r.contains_key(&thread)) {
        return json!({ "ok": false, "error": "This chat is still answering. Stop it first." });
    }
    crate::store::with(|c| {
        c.execute("delete from chat_messages where thread_id = ?1 and role = 'assistant' and id > coalesce((select max(id) from chat_messages where thread_id = ?1 and role = 'user'), 0)", params![thread])
    });
    if !history(thread).iter().any(|m| m.0 == "user") {
        return json!({ "ok": false, "error": "Nothing to retry." });
    }
    match start_run(thread, cfg) {
        Ok(run) => json!({ "ok": true, "thread": thread, "run": run }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// Stop the running answer of a thread (or of every thread). The partial text is kept; chat:done follows with stopped = true.
pub fn chat_stop(thread: Option<i64>) -> bool {
    let list: Vec<net::Abort> = runs(|r| r.iter().filter(|(t, _)| thread.map_or(true, |x| x == **t)).map(|(_, (_, a))| a.clone()).collect());
    for a in &list {
        a.abort();
    }
    !list.is_empty()
}

pub fn chat_threads() -> Value {
    crate::store::with(|c| {
        let mut st = c.prepare(
            "select t.id, t.title, t.created_at,
                    coalesce((select max(created_at) from chat_messages m where m.thread_id = t.id), t.created_at) as upd,
                    (select count(*) from chat_messages m where m.thread_id = t.id)
             from chat_threads t order by upd desc, t.id desc limit 200",
        )?;
        let rows = st.query_map([], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "title": r.get::<_, String>(1)?, "created_at": r.get::<_, i64>(2)?, "updated_at": r.get::<_, i64>(3)?, "count": r.get::<_, i64>(4)? })))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .map(Value::from)
    .unwrap_or_else(|| json!([]))
}

pub fn chat_messages(thread: i64) -> Value {
    crate::store::with(|c| {
        let mut st = c.prepare("select id, role, content, created_at from chat_messages where thread_id = ?1 order by id limit 5000")?;
        let rows = st.query_map(params![thread], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "role": r.get::<_, String>(1)?, "content": r.get::<_, String>(2)?, "created_at": r.get::<_, i64>(3)? })))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    })
    .map(Value::from)
    .unwrap_or_else(|| json!([]))
}

pub fn chat_new(title: Option<String>) -> Value {
    match new_thread(title.as_deref().unwrap_or("")) {
        Some(id) => json!({ "id": id }),
        None => Value::Null,
    }
}

pub fn chat_delete(thread: i64) -> bool {
    chat_stop(Some(thread));
    crate::store::with(|c| {
        c.execute("delete from chat_messages where thread_id = ?1", params![thread])?;
        c.execute("delete from chat_threads where id = ?1", params![thread])
    })
    .is_some_and(|n| n > 0)
}

/// Open a link from an answer in the default browser (never inside the webview). http(s) only.
pub fn chat_open_link(url: String) -> bool {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let u = url.trim();
    let ok = u.len() <= 2000 && net::parse_url(u).is_ok();
    if !ok {
        return false;
    }
    unsafe { ShellExecuteW(HWND::default(), &HSTRING::from("open"), &HSTRING::from(u), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL).0 as isize > 32 }
}

/// One-token test call used by the AI settings block. {ok, ms, model} or {ok:false, error}.
pub fn ai_test() -> Value {
    let cfg = match net::ai_cfg(None, true) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e }),
    };
    let body = json!({ "model": cfg.model, "messages": [{ "role": "user", "content": "Reply with one word: ok" }], "max_tokens": 1, "temperature": 0, "stream": false }).to_string();
    let t = Instant::now();
    match net::ai_chat(&cfg, body.as_bytes(), 60_000, None, None) {
        Ok(r) if r.status / 100 == 2 => json!({ "ok": true, "ms": t.elapsed().as_millis() as u64, "model": cfg.model }),
        Ok(r) => json!({ "ok": false, "error": http_error(&r) }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// Model ids offered by the endpoint (OpenAI `data[].id` or Ollama `models[].name`), to fill the picker.
pub fn ai_models() -> Value {
    let cfg = match net::ai_cfg(None, false) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e, "models": [] }),
    };
    match net::ai_models(&cfg) {
        Ok(r) if r.status / 100 == 2 => {
            let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
            let mut m: Vec<String> = v["data"].as_array().into_iter().flatten().filter_map(|x| x["id"].as_str()).map(str::to_string).collect();
            if m.is_empty() {
                m = v["models"].as_array().into_iter().flatten().filter_map(|x| x["name"].as_str().or(x["id"].as_str())).map(str::to_string).collect();
            }
            m.sort();
            m.dedup();
            m.truncate(300);
            json!({ "ok": true, "models": m })
        }
        Ok(r) => json!({ "ok": false, "error": http_error(&r), "models": [] }),
        Err(e) => json!({ "ok": false, "error": e, "models": [] }),
    }
}

/// Looks for local OpenAI-compatible servers on their usual loopback ports (in parallel, short timeouts).
/// [{id, name, url, up, models}] for Ollama, FreeLLMAPI, LM Studio, llama.cpp / llamafile.
pub fn ai_probe() -> Value {
    const CANDIDATES: [(&str, &str, &str); 4] = [
        ("ollama", "Ollama", "http://127.0.0.1:11434/v1"),
        ("freellm", "FreeLLMAPI", "http://127.0.0.1:3001/v1"),
        ("lmstudio", "LM Studio", "http://127.0.0.1:1234/v1"),
        ("llamacpp", "llama.cpp", "http://127.0.0.1:8080/v1"),
    ];
    let handles: Vec<_> = CANDIDATES
        .iter()
        .map(|&(id, name, url)| {
            std::thread::spawn(move || {
                let r = net::ai_probe(url);
                let (up, mut models) = match r {
                    Ok(r) if r.status / 100 == 2 => {
                        let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
                        let m: Vec<String> = v["data"].as_array().into_iter().flatten().filter_map(|x| x["id"].as_str()).map(str::to_string).collect();
                        (true, m)
                    }
                    // a server that answers 401/403 is up but wants a key
                    Ok(r) => (r.status == 401 || r.status == 403, Vec::new()),
                    Err(_) => (false, Vec::new()),
                };
                models.sort();
                models.dedup();
                models.truncate(300);
                json!({ "id": id, "name": name, "url": url, "up": up, "models": models })
            })
        })
        .collect();
    Value::Array(handles.into_iter().filter_map(|h| h.join().ok()).collect())
}

/// Store (encrypted with DPAPI) or remove (empty string) the API key. The key never travels back to the page.
pub fn ai_set_key(key: String) -> Value {
    let k = key.trim();
    if k.is_empty() {
        crate::settings::set("ai.apiKeyEnc", json!(""));
        return json!({ "ok": true, "hasKey": false });
    }
    if k.len() > 1024 || k.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return json!({ "ok": false, "error": "That does not look like an API key." });
    }
    match net::secrets::protect(k) {
        Ok(b) => {
            crate::settings::set("ai.apiKeyEnc", json!(b));
            json!({ "ok": true, "hasKey": true })
        }
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// Non-secret AI configuration for the settings block.
pub fn ai_config() -> Value {
    let base = crate::settings::get_str("ai.baseUrl", "");
    let check = if base.trim().is_empty() { Ok(String::new()) } else { net::check_ai_base(&base) };
    json!({
        "on": net::enabled(net::Feature::Ai),
        "baseUrl": base,
        "model": crate::settings::get_str("ai.model", ""),
        "preset": crate::settings::get_str("ai.preset", "ollama"),
        "hasKey": !crate::settings::get_str("ai.apiKeyEnc", "").is_empty(),
        "urlError": check.err(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_sse_lines() {
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{"content":"Hi"}}]}"#), Sse::Text("Hi".into()));
        assert_eq!(parse_sse_line("data: [DONE]"), Sse::Done);
        assert_eq!(parse_sse_line(r#"data:{"choices":[{"delta":{"role":"assistant"}}]}"#), Sse::Other);
        assert_eq!(parse_sse_line(r#"data: {"error":{"message":"bad key"}}"#), Sse::Err("bad key".into()));
        assert_eq!(parse_sse_line(r#"data: {"choices":[{"delta":{"reasoning":"hmm"}}]}"#), Sse::Other);
        assert_eq!(parse_sse_line("event: ping"), Sse::Other);
    }

    #[test]
    fn chat_http_errors() {
        let r = net::Resp { status: 401, headers: vec![], body: br#"{"error":{"message":"Incorrect API key"}}"#.to_vec() };
        assert_eq!(http_error(&r), "HTTP 401: Incorrect API key (check the API key)");
        let r = net::Resp { status: 500, headers: vec![], body: b"oops".to_vec() };
        assert_eq!(http_error(&r), "HTTP 500: oops");
    }

    #[test]
    fn chat_dpapi_roundtrip() {
        let b = net::secrets::protect("sk-test-123").unwrap();
        assert!(!b.contains("sk-test"));
        assert_eq!(net::secrets::unprotect(&b).unwrap(), "sk-test-123");
        assert!(net::secrets::unprotect("AAAA").is_err());
    }
}
