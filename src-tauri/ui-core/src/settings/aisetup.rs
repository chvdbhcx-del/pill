//! AI setup page (`ai-setup.js`): what the local-voice (Whisper) and local-chat (Ollama / found servers) cards show for an
//! `aisetup_status` answer, and the calls their buttons make. Nothing here downloads or starts anything: every download is a
//! button whose label already names the source host and the size (the page promise "nothing happens without your click").
//! Progress lines of the running jobs are in `flows::aisetup_progress`.
use crate::settings::calls::BackendCall;
use crate::settings::flows::size;
use crate::settings::tr::{pick, tr, Lang, Tr};
use serde_json::Value;

pub const WHISPER_PORT_NOTE: Tr = tr("порт 8179 (если занят, берётся следующий свободный; ваш собственный сервер на 8178 не затрагивается)", "port 8179 (the next free one if busy; your own server on 8178 is left alone)");

/// Display names and notes of the three Whisper models.
pub fn model_name(id: &str) -> &str {
    match id {
        "base" => "Base",
        "small" => "Small",
        "turbo" => "Large-v3-turbo (q5)",
        other => other,
    }
}

pub fn model_note(id: &str) -> Tr {
    match id {
        "base" => tr("Самая быстрая и лёгкая. Русский и редкие слова распознаёт заметно хуже.", "The fastest and lightest. Clearly weaker on Russian and rare words."),
        "small" => tr("Хороший баланс: русский и английский, работает быстро даже на процессоре. Рекомендуется.", "A good balance: Russian and English, fast even on a CPU. Recommended."),
        "turbo" => tr("Лучшее качество из трёх. Версия для процессора: заметно медленнее Small, фраза в 10 секунд может обрабатываться несколько секунд.", "The best quality of the three. A CPU build: clearly slower than Small, a 10-second phrase can take several seconds."),
        _ => Tr::NONE,
    }
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}
fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// The model chip that is selected: the remembered one if it exists in the list, else "small".
pub fn selected_model<'a>(w: &Value, sel: &'a str) -> &'a str {
    if arr(w, "models").iter().any(|m| s(m, "id") == sel) { sel } else { "small" }
}

/// "running on 127.0.0.1:8179 · model Small" / "enabled, but the server is not running ..." / "not installed or off".
pub fn whisper_state(lang: Lang, w: &Value) -> (String, bool) {
    let model = model_name(s(w, "model")).to_string();
    let running = w.get("running").filter(|x| !x.is_null() && **x != Value::Bool(false));
    match running {
        Some(port) => {
            let port = port.as_i64().map(|p| p.to_string()).unwrap_or_else(|| port.to_string());
            (pick(lang, &format!("работает на 127.0.0.1:{port} · модель {model}"), &format!("running on 127.0.0.1:{port} · model {model}")).to_string(), true)
        }
        None if b(w, "enabled") => (pick(lang, "включён, но сервер не запущен (см. server.log в папке ai/whisper)", "enabled, but the server is not running (see server.log in the ai/whisper folder)").to_string(), false),
        None => (pick(lang, "не установлен или выключен", "not installed or off").to_string(), false),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Need {
    pub what: String,
    pub host: String,
    pub size: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WhisperAction {
    /// "Download 540 MB and turn on" (the model of the call).
    Download { label: String, call: BackendCall },
    /// "Turn on" / "Switch to this model".
    Enable { label: &'static str, call: BackendCall },
    /// Everything is installed and this model runs.
    Running,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WhisperPlan {
    pub need: Vec<Need>,
    pub total: f64,
    pub action: WhisperAction,
}

/// What a click on the primary button of the Whisper card does for the selected model.
pub fn whisper_plan(lang: Lang, w: &Value, sel: &str) -> WhisperPlan {
    let sel = selected_model(w, sel);
    let engine = w.get("engine").cloned().unwrap_or(Value::Null);
    let m = arr(w, "models").iter().find(|m| s(m, "id") == sel).cloned().unwrap_or(Value::Null);
    let mut need = Vec::new();
    if !b(&engine, "installed") {
        let tag = s(&engine, "tag");
        need.push(Need { what: pick(lang, &format!("Движок whisper.cpp {tag} (Windows, процессор)"), &format!("whisper.cpp {tag} engine (Windows, CPU)")).to_string(), host: s(&engine, "host").to_string(), size: f(&engine, "size") });
    }
    if !b(&m, "installed") {
        let n = model_name(sel);
        need.push(Need { what: pick(lang, &format!("Модель {n}"), &format!("{n} model")).to_string(), host: s(&m, "host").to_string(), size: f(&m, "size") });
    }
    let total: f64 = need.iter().map(|n| n.size).sum();
    let running = w.get("running").is_some_and(|x| !x.is_null() && *x != Value::Bool(false));
    let action = if !need.is_empty() {
        let sz = size(lang, total);
        WhisperAction::Download { label: pick(lang, &format!("Скачать {sz} и включить"), &format!("Download {sz} and turn on")).to_string(), call: BackendCall::AisetupWhisperInstall { model: sel.to_string() } }
    } else if !(running && s(w, "model") == sel) {
        WhisperAction::Enable { label: if running { pick(lang, "Переключить на эту модель", "Switch to this model") } else { pick(lang, "Включить", "Turn on") }, call: BackendCall::AisetupWhisperSet { on: true, model: Some(sel.to_string()) } }
    } else {
        WhisperAction::Running
    };
    WhisperPlan { need, total, action }
}

pub const WHISPER_ALL_GOOD: Tr = tr("Всё установлено и работает. Диктовка: Ctrl+Alt+D (клавишу можно сменить во вкладке «Клавиши»).", "Everything is installed and running. Dictate with Ctrl+Alt+D (the key can be changed under Hotkeys).");

/// The on/off switch of the card: shown once the engine and at least one model are installed.
pub fn whisper_switch_visible(w: &Value) -> bool {
    b(w.get("engine").unwrap_or(&Value::Null), "installed") && arr(w, "models").iter().any(|m| b(m, "installed"))
}

/// The switch was flipped: turn on with the selected model if it is installed (else without naming one), or off.
pub fn whisper_switch_call(w: &Value, sel: &str, on: bool) -> BackendCall {
    let installed = arr(w, "models").iter().any(|m| s(m, "id") == sel && b(m, "installed"));
    BackendCall::AisetupWhisperSet { on, model: if on && installed { Some(sel.to_string()) } else { None } }
}

/// The "Folder: ... · port ..." line.
pub fn whisper_folder_note(lang: Lang, dir: &str) -> String {
    format!("{}{}\\whisper · {}", pick(lang, "Папка: ", "Folder: "), dir, WHISPER_PORT_NOTE.get(lang))
}

// ------------------------------------------------------------------------------------------------ chat card

/// Header row: servers that already answer on this computer.
pub fn servers_line(lang: Lang, servers: &Value) -> (String, bool) {
    let up: Vec<&str> = servers.as_array().map(|a| a.iter().filter(|x| b(x, "up")).map(|x| s(x, "name")).collect()).unwrap_or_default();
    if up.is_empty() {
        (pick(lang, "Запущенных локальных серверов (Ollama, LM Studio, llama.cpp, FreeLLMAPI) не найдено. Запросы к ним идут только на 127.0.0.1.", "No local servers (Ollama, LM Studio, llama.cpp, FreeLLMAPI) found. Requests to them go to 127.0.0.1 only.").to_string(), false)
    } else {
        (format!("{}{}.", pick(lang, "Найдено на этом компьютере: ", "Found on this computer: "), up.join(", ")), true)
    }
}

/// One found server: "3 models · http://127.0.0.1:11434/v1".
pub fn server_sub(lang: Lang, models: usize, url: &str) -> String {
    if models > 0 {
        format!("{models}{}{url}", pick(lang, " моделей · ", " models · "))
    } else {
        url.to_string()
    }
}

/// Is the chat already using this server (`status.ai.on` and the same base URL)?
pub fn used_here(ai: &Value, server_url: &str) -> bool {
    b(ai, "on") && s(ai, "baseUrl").trim_end_matches('/') == server_url
}

pub fn use_button_label(lang: Lang, used_with_selected_model: bool) -> &'static str {
    if used_with_selected_model { pick(lang, "Используется", "In use") } else { pick(lang, "Использовать", "Use") }
}

pub fn use_call(server: &str, model: &str) -> BackendCall {
    BackendCall::AisetupAiUse { server: server.to_string(), model: model.to_string() }
}

#[derive(Clone, Debug, PartialEq)]
pub enum OllamaStep {
    /// Installed but not running: only a note.
    StartIt(Tr),
    /// The installer is on disk: "Run the installer".
    RunInstaller { label: &'static str, call: BackendCall },
    /// "Download the installer (≈ 1.2 GB)".
    DownloadInstaller { label: String, call: BackendCall },
    /// Ollama answers: show the catalog of small models.
    Catalog,
}

pub fn ollama_step(lang: Lang, o: &Value) -> OllamaStep {
    if b(o, "up") {
        OllamaStep::Catalog
    } else if b(o, "installed") {
        OllamaStep::StartIt(tr("Ollama установлена, но не запущена: откройте её из меню Пуск.", "Ollama is installed but not running: open it from the Start menu."))
    } else if o.get("installer").is_some_and(|i| b(i, "ready")) {
        OllamaStep::RunInstaller { label: pick(lang, "Запустить установщик", "Run the installer"), call: BackendCall::AisetupOllamaRunInstaller }
    } else {
        let sz = size(lang, f(o.get("installer").unwrap_or(&Value::Null), "size"));
        OllamaStep::DownloadInstaller { label: pick(lang, &format!("Скачать установщик (≈ {sz})"), &format!("Download the installer (≈ {sz})")).to_string(), call: BackendCall::AisetupOllamaDownload }
    }
}

/// The hint of the Ollama row: the installer is no longer needed, or where it comes from and how big it is.
pub fn ollama_hint(lang: Lang, o: &Value) -> String {
    if b(o, "installed") {
        pick(lang, "Официальный установщик уже не нужен.", "The official installer is not needed any more.").to_string()
    } else {
        let sz = size(lang, f(o.get("installer").unwrap_or(&Value::Null), "size"));
        match lang {
            Lang::Ru => format!("Официальный установщик с ollama.com (перенаправляет на GitHub), около {sz}. Скачивание его не запускает: запуск отдельной кнопкой, дальше запрос Windows. Хеша для проверки нет: сверьте издателя в окне Windows."),
            Lang::En => format!("The official installer from ollama.com (redirects to GitHub), about {sz}. Downloading does not start it: running is a separate button, then the Windows prompt. There is no published hash: check the publisher in the prompt."),
        }
    }
}

/// A catalog model is "already pulled" when Ollama lists it (with or without the `:latest` tag).
pub fn have_model(servers: &Value, name: &str) -> bool {
    servers.as_array().and_then(|a| a.iter().find(|x| s(x, "id") == "ollama")).is_some_and(|o| arr(o, "models").iter().any(|m| m.as_str().is_some_and(|m| m == name || m == format!("{name}:latest"))))
}

pub fn pull_call(model: &str) -> BackendCall {
    BackendCall::AisetupOllamaPull { model: model.to_string() }
}

/// "Later" / "Show at start again" at the foot of the page.
pub fn later_button(lang: Lang, done: bool) -> &'static str {
    if done { pick(lang, "Показывать при запуске", "Show at start again") } else { pick(lang, "Позже", "Later") }
}

pub fn later_title(lang: Lang, done: bool) -> &'static str {
    if done { pick(lang, "Это окно больше не открывается само при запуске", "This window no longer opens by itself at start") } else { pick(lang, "Настрою позже", "I will set this up later") }
}

/// Pressing it: when done, clear the flag and stay; else set it and close the settings window. No-op while a job runs.
pub fn later_calls(done: bool, busy: bool) -> Vec<BackendCall> {
    if busy && !done {
        vec![]
    } else if done {
        vec![BackendCall::put("aiSetupDone", Value::Bool(false))]
    } else {
        vec![BackendCall::put("aiSetupDone", Value::Bool(true)), BackendCall::SettingsClose]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn whisper(engine: bool, models: &[(&str, bool)], running: Value, model: &str) -> Value {
        json!({
            "enabled": !running.is_null(), "running": running, "model": model,
            "engine": {"installed": engine, "tag": "v1.7", "host": "github.com", "size": 4_000_000},
            "models": models.iter().map(|(id, inst)| json!({"id": id, "installed": inst, "size": 150_000_000, "host": "huggingface.co"})).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn nothing_installed_offers_one_download_with_the_total() {
        let w = whisper(false, &[("base", false), ("small", false)], Value::Null, "small");
        let p = whisper_plan(Lang::En, &w, "small");
        assert_eq!(p.need.len(), 2);
        assert_eq!((p.need[0].what.as_str(), p.need[0].host.as_str()), ("whisper.cpp v1.7 engine (Windows, CPU)", "github.com"));
        assert_eq!(p.need[1].what, "Small model");
        assert_eq!(p.total, 154_000_000.0);
        assert_eq!(p.action, WhisperAction::Download { label: "Download 154 MB and turn on".into(), call: BackendCall::AisetupWhisperInstall { model: "small".into() } });
        assert!(!whisper_switch_visible(&w));
        assert_eq!(whisper_state(Lang::En, &w), ("not installed or off".to_string(), false));
    }

    #[test]
    fn installed_but_off_turns_on_and_running_switches_models() {
        let w = whisper(true, &[("base", true), ("small", true)], Value::Null, "small");
        let p = whisper_plan(Lang::Ru, &w, "small");
        assert!(p.need.is_empty());
        assert_eq!(p.action, WhisperAction::Enable { label: "Включить", call: BackendCall::AisetupWhisperSet { on: true, model: Some("small".into()) } });
        assert!(whisper_switch_visible(&w));
        let running = whisper(true, &[("base", true), ("small", true)], json!(8179), "small");
        assert_eq!(whisper_plan(Lang::En, &running, "small").action, WhisperAction::Running);
        match whisper_plan(Lang::En, &running, "base").action {
            WhisperAction::Enable { label, .. } => assert_eq!(label, "Switch to this model"),
            a => panic!("{a:?}"),
        }
        assert_eq!(whisper_state(Lang::En, &running), ("running on 127.0.0.1:8179 · model Small".to_string(), true));
        assert_eq!(whisper_state(Lang::Ru, &whisper(true, &[], json!(null), "small")).0, "не установлен или выключен");
        let enabled_no_server = json!({"enabled": true, "running": null, "model": "base", "engine": {}, "models": []});
        assert!(whisper_state(Lang::En, &enabled_no_server).0.starts_with("enabled, but the server is not running"));
        assert_eq!(selected_model(&running, "turbo"), "small", "an unknown chip falls back to Small");
    }

    #[test]
    fn switch_call_names_the_model_only_when_it_is_installed() {
        let w = whisper(true, &[("base", true), ("small", false)], Value::Null, "base");
        assert_eq!(whisper_switch_call(&w, "base", true), BackendCall::AisetupWhisperSet { on: true, model: Some("base".into()) });
        assert_eq!(whisper_switch_call(&w, "small", true), BackendCall::AisetupWhisperSet { on: true, model: None });
        assert_eq!(whisper_switch_call(&w, "base", false), BackendCall::AisetupWhisperSet { on: false, model: None });
        assert!(whisper_folder_note(Lang::En, "C:\\ai").starts_with("Folder: C:\\ai\\whisper · port 8179"));
        assert!(model_note("small").is_complete() && model_note("nope").is_blank());
        assert_eq!(model_name("turbo"), "Large-v3-turbo (q5)");
    }

    #[test]
    fn local_chat_card() {
        let servers = json!([{"id": "ollama", "name": "Ollama", "url": "http://127.0.0.1:11434/v1", "up": true, "models": ["qwen:latest", "llama3"]}, {"id": "lmstudio", "name": "LM Studio", "up": false, "models": []}]);
        assert_eq!(servers_line(Lang::En, &servers), ("Found on this computer: Ollama.".to_string(), true));
        assert!(!servers_line(Lang::Ru, &json!([])).1);
        assert_eq!(server_sub(Lang::En, 2, "http://x"), "2 models · http://x");
        assert_eq!(server_sub(Lang::En, 0, "http://x"), "http://x");
        assert!(used_here(&json!({"on": true, "baseUrl": "http://127.0.0.1:11434/v1/"}), "http://127.0.0.1:11434/v1"));
        assert!(!used_here(&json!({"on": false, "baseUrl": "http://127.0.0.1:11434/v1"}), "http://127.0.0.1:11434/v1"));
        assert_eq!((use_button_label(Lang::En, true), use_button_label(Lang::Ru, false)), ("In use", "Использовать"));
        assert!(have_model(&servers, "qwen") && have_model(&servers, "llama3") && !have_model(&servers, "phi"));
        assert_eq!(use_call("ollama", "qwen").command(), "aisetup_ai_use");
        assert_eq!(pull_call("phi").args(), json!({"model": "phi"}));
    }

    #[test]
    fn ollama_steps() {
        let up = json!({"up": true});
        assert_eq!(ollama_step(Lang::En, &up), OllamaStep::Catalog);
        assert!(matches!(ollama_step(Lang::En, &json!({"up": false, "installed": true})), OllamaStep::StartIt(_)));
        assert_eq!(ollama_step(Lang::En, &json!({"installer": {"ready": true}})), OllamaStep::RunInstaller { label: "Run the installer", call: BackendCall::AisetupOllamaRunInstaller });
        match ollama_step(Lang::En, &json!({"installer": {"ready": false, "size": 1_200_000_000u64}})) {
            OllamaStep::DownloadInstaller { label, call } => assert_eq!((label.as_str(), call), ("Download the installer (≈ 1.2 GB)", BackendCall::AisetupOllamaDownload)),
            s => panic!("{s:?}"),
        }
        assert!(ollama_hint(Lang::En, &json!({"installer": {"size": 300_000_000}})).contains("about 300 MB"));
        assert_eq!(ollama_hint(Lang::Ru, &json!({"installed": true})), "Официальный установщик уже не нужен.");
    }

    #[test]
    fn the_later_button() {
        assert_eq!(later_button(Lang::En, false), "Later");
        assert_eq!(later_title(Lang::En, true), "This window no longer opens by itself at start");
        assert_eq!(later_calls(false, false), vec![BackendCall::put("aiSetupDone", json!(true)), BackendCall::SettingsClose]);
        assert_eq!(later_calls(true, false), vec![BackendCall::put("aiSetupDone", json!(false))]);
        assert!(later_calls(false, true).is_empty(), "disabled while a download runs");
    }
}
