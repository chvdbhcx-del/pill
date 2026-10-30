//! Result handling of the settings window's actions: which message (text + kind) each backend answer produces, and the small
//! state machines around them (update check/install, two-step confirmations, progress boxes, polled status lines). Pure port of
//! the inline logic of `settings.js`, `settings-live-ext.js`, `ai-setup.js` and `voice-ui.js`. The renderer shows a `Msg` in the
//! row's message line (entering with `AnimKind::MessageEnter`) and polls the status commands on its own timers.
use crate::settings::calls::BackendCall;
use crate::settings::tr::{pick, tr, Lang, Tr};
use serde_json::Value;

/// Message kinds = the CSS classes of the web page (`note`, `note ok`, `note bad`, `note warn`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Plain,
    Ok,
    Bad,
    Warn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Msg {
    pub text: String,
    pub kind: Kind,
}

impl Msg {
    pub fn new(text: impl Into<String>, kind: Kind) -> Msg {
        Msg { text: text.into(), kind }
    }
    pub fn ok(t: impl Into<String>) -> Msg {
        Msg::new(t, Kind::Ok)
    }
    pub fn bad(t: impl Into<String>) -> Msg {
        Msg::new(t, Kind::Bad)
    }
    pub fn warn(t: impl Into<String>) -> Msg {
        Msg::new(t, Kind::Warn)
    }
    pub fn plain(t: impl Into<String>) -> Msg {
        Msg::new(t, Kind::Plain)
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn n(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
/// `x.error || fallback`.
fn err_or(v: &Value, fallback: &str) -> String {
    let e = s(v, "error");
    if e.is_empty() {
        fallback.to_string()
    } else {
        e
    }
}

// ------------------------------------------------------------------------------------------------ updates

pub const CHECK_NOW: Tr = tr("Проверить сейчас", "Check now");
pub const INSTALL_RESTART: Tr = tr("Установить и перезапустить", "Install and restart");

/// "Check now" -> (found) "Install and restart". Notes of the found version are cut to 300 chars.
#[derive(Debug, Default)]
pub struct UpdateFlow {
    found: bool,
    busy: bool,
}

impl UpdateFlow {
    pub fn button_label(&self, lang: Lang) -> &'static str {
        if self.found { INSTALL_RESTART } else { CHECK_NOW }.get(lang)
    }
    pub fn is_busy(&self) -> bool {
        self.busy
    }
    /// A click: the first one checks, after a found update the next installs. None while busy.
    pub fn click(&mut self, lang: Lang) -> Option<(BackendCall, Msg)> {
        if self.busy {
            return None;
        }
        self.busy = true;
        Some(if self.found {
            (BackendCall::UpdateInstall, Msg::plain(pick(lang, "Скачиваю и устанавливаю…", "Downloading and installing…")))
        } else {
            (BackendCall::UpdateCheck, Msg::plain(pick(lang, "Проверяю…", "Checking…")))
        })
    }
    pub fn on_check(&mut self, lang: Lang, res: &Value) -> Msg {
        self.busy = false;
        match s(res, "status").as_str() {
            "available" => {
                self.found = true;
                let notes: String = s(res, "notes").chars().take(300).collect();
                Msg::ok(format!("{}{}{}", pick(lang, "Доступна версия ", "Version available: "), s(res, "version"), if notes.is_empty() { String::new() } else { format!(". {notes}") }))
            }
            "none" => Msg::ok(pick(lang, "У вас последняя версия.", "You are up to date.")),
            _ => Msg::bad(format!("{}{}", pick(lang, "Не удалось проверить: ", "Could not check: "), s(res, "error"))),
        }
    }
    /// After a successful install the app restarts, so only an error shows a line.
    pub fn on_install(&mut self, lang: Lang, res: &Value) -> Option<Msg> {
        self.busy = false;
        (s(res, "status") == "error").then(|| Msg::bad(format!("{}{}", pick(lang, "Не удалось установить: ", "Install failed: "), s(res, "error"))))
    }
}

pub fn report_result(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        let log = b(res, "log");
        Msg::ok(match lang {
            Lang::Ru => format!("Папка отчёта открыта в Загрузках. Версия, настройки без ключей и паролей{}. Ничего не отправляется: приложите файлы сами.", if log { ", хвост лога" } else { ", лога нет" }),
            Lang::En => format!("The report folder is open in Downloads: version, settings without keys or passwords{}. Nothing is sent: attach the files yourself.", if log { ", the log tail" } else { ", no log" }),
        })
    } else {
        Msg::bad(format!("{}{}", pick(lang, "Не удалось создать отчёт: ", "Could not create the report: "), s(res, "error")))
    }
}

pub fn data_dir_note(lang: Lang, dir: &str) -> String {
    format!("{}{}", pick(lang, "Данные и настройки лежат здесь: ", "Data and settings live here: "), dir)
}

// ------------------------------------------------------------------------------------------------ settings backup

pub const MAX_IMPORT_BYTES: u64 = 1_048_576;

pub fn counts(lang: Lang, applied: i64, skipped: i64) -> String {
    match lang {
        Lang::Ru => format!("Применено {applied}, пропущено {skipped}"),
        Lang::En => format!("Applied {applied}, skipped {skipped}"),
    }
}

/// The file picked for import. Err = the message for a file that must not be read at all.
pub fn import_call(lang: Lang, size: u64, text: &str) -> Result<BackendCall, Msg> {
    if size > MAX_IMPORT_BYTES {
        return Err(Msg::bad(pick(lang, "Файл больше 1 МБ", "The file is larger than 1 MB")));
    }
    Ok(BackendCall::SettingsImport { json_text: text.to_string() })
}

pub fn read_error(lang: Lang) -> Msg {
    Msg::bad(pick(lang, "Не удалось прочитать файл", "Could not read the file"))
}

/// Answer of `settings_import`; `reload` = the page must re-read `settings_get` afterwards.
pub fn import_result(lang: Lang, res: &Value) -> (Msg, bool) {
    if res.is_null() {
        return (Msg::bad(pick(lang, "Не удалось импортировать", "Import failed")), false);
    }
    let (applied, skipped) = (n(res, "applied") as i64, n(res, "skipped") as i64);
    if b(res, "ok") {
        let e = s(res, "error");
        let c = counts(lang, applied, skipped);
        (if e.is_empty() { Msg::ok(c) } else { Msg::bad(format!("{c} · {e}")) }, true)
    } else {
        let tail = if skipped != 0 { format!(" · {}", counts(lang, 0, skipped)) } else { String::new() };
        (Msg::bad(format!("{}{}", s(res, "error"), tail)), false)
    }
}

pub fn export_result(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        Msg::ok(format!("{}{}", pick(lang, "Сохранено: ", "Saved: "), s(res, "path")))
    } else {
        Msg::bad(err_or(res, pick(lang, "Не удалось сохранить", "Could not save")))
    }
}

// ------------------------------------------------------------------------------------------------ confirmations

/// Two-step confirm: the first click arms, the second (within `window_ms`, if any) confirms. Clipboard "Wipe…" has no timeout
/// and a Cancel button; the AI setup "Delete from disk" disarms itself after 3 s.
#[derive(Debug)]
pub struct Confirm {
    armed_at: Option<u64>,
    window_ms: Option<u64>,
}

impl Confirm {
    pub fn new(window_ms: Option<u64>) -> Confirm {
        Confirm { armed_at: None, window_ms }
    }
    /// Wipe history: waits for "Yes, wipe everything" or Cancel.
    pub fn wipe() -> Confirm {
        Confirm::new(None)
    }
    /// Delete the Whisper files: "Really delete?" for 3 s.
    pub fn delete_from_disk() -> Confirm {
        Confirm::new(Some(3000))
    }
    pub fn is_armed(&self, now_ms: u64) -> bool {
        match (self.armed_at, self.window_ms) {
            (Some(at), Some(w)) => now_ms.saturating_sub(at) < w,
            (Some(_), None) => true,
            _ => false,
        }
    }
    /// A click. True = confirmed (do it now); false = armed (or re-armed after the window ran out).
    pub fn click(&mut self, now_ms: u64) -> bool {
        if self.is_armed(now_ms) {
            self.armed_at = None;
            true
        } else {
            self.armed_at = Some(now_ms);
            false
        }
    }
    pub fn cancel(&mut self) {
        self.armed_at = None;
    }
}

pub const WIPE: Tr = tr("Стереть…", "Wipe…");
pub const WIPE_YES: Tr = tr("Да, стереть всё", "Yes, wipe everything");
pub const CANCEL: Tr = tr("Отмена", "Cancel");
pub const WIPE_DONE: Tr = tr("Готово: история пуста", "Done: the history is empty");
pub const DELETE_FROM_DISK: Tr = tr("Удалить с диска", "Delete from disk");
pub const REALLY_DELETE: Tr = tr("Точно удалить?", "Really delete?");

pub fn clip_entries_line(lang: Lang, count: i64) -> String {
    format!("{}{}", pick(lang, "В истории записей: ", "Entries in the history: "), count)
}

// ------------------------------------------------------------------------------------------------ profiles

pub const MAX_PROFILES: usize = 8;
pub const PROFILE_NAME_MAX: usize = 24;

/// What the name field sends: control characters dropped, trimmed, cut to 24 chars. None = nothing to save.
pub fn profile_name(raw: &str) -> Option<String> {
    let nm: String = raw.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(PROFILE_NAME_MAX).collect();
    let nm = nm.trim().to_string();
    (!nm.is_empty()).then_some(nm)
}

pub fn profile_enabled_line(lang: Lang, on: usize) -> String {
    match lang {
        Lang::Ru => format!("Включено: {on}"),
        Lang::En => format!("On: {on}"),
    }
}

pub fn profiles_empty() -> Tr {
    tr("Профилей пока нет. Настройте модули и события ниже, введите название и сохраните.", "No profiles yet. Set the modules and live events below, type a name and save.")
}

/// Answer of `profile_save|apply|delete`. `applied_name`: shown in the "Applied" line.
pub fn profile_result(lang: Lang, res: &Value, ok_text: &str) -> Msg {
    if b(res, "ok") {
        Msg::ok(ok_text)
    } else if s(res, "error") == "full" {
        Msg::bad(pick(lang, "Не больше 8 профилей: удалите ненужный", "At most 8 profiles: delete one first"))
    } else {
        Msg::bad(pick(lang, "Не удалось", "Failed"))
    }
}

pub fn profile_applied_text(lang: Lang, name: &str) -> String {
    match lang {
        Lang::Ru => format!("Применён профиль «{name}»"),
        Lang::En => format!("Applied “{name}”"),
    }
}

pub fn profile_saved_text(lang: Lang) -> &'static str {
    pick(lang, "Сохранено", "Saved")
}

// ------------------------------------------------------------------------------------------------ Everything and downloads

/// `(b / 1e6).toFixed(1) + " МБ"`.
pub fn mb(lang: Lang, bytes: f64) -> String {
    format!("{:.1} {}", bytes / 1e6, pick(lang, "МБ", "MB"))
}

/// The AI-setup size: GB with one decimal from 1 GB, else whole MB.
pub fn size(lang: Lang, bytes: f64) -> String {
    if bytes >= 1e9 {
        format!("{:.1}{}", bytes / 1e9, pick(lang, " ГБ", " GB"))
    } else {
        format!("{}{}", (bytes / 1e6).round() as i64, pick(lang, " МБ", " MB"))
    }
}

/// One `aisetup:progress` event ({id, item, done, total, phase, message}).
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub id: String,
    pub item: String,
    pub done: f64,
    pub total: f64,
    pub phase: String,
    pub message: String,
}

impl Progress {
    pub fn from_event(v: &Value) -> Option<Progress> {
        v.is_object().then(|| Progress { id: s(v, "id"), item: s(v, "item"), done: n(v, "done"), total: n(v, "total"), phase: s(v, "phase"), message: s(v, "message") })
    }
    pub fn is_final(&self) -> bool {
        matches!(self.phase.as_str(), "done" | "error" | "cancelled")
    }
    /// The total is known and the running count has not overshot it by more than 2 %.
    pub fn known(&self) -> bool {
        self.total > 0.0 && self.done <= self.total * 1.02
    }
    pub fn percent(&self) -> u8 {
        if self.known() {
            ((self.done / self.total * 100.0).floor()).min(100.0) as u8
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProgressView {
    pub text: String,
    /// Fill width in percent (0 while indeterminate).
    pub percent: u8,
    /// The bar shows its running stripe instead of a fill.
    pub indeterminate: bool,
}

/// Progress line of the Everything card (events whose id starts with "everything-").
pub fn everything_progress(lang: Lang, q: &Progress) -> ProgressView {
    let known = q.known();
    let nm = if q.item == "es" { "es.exe".to_string() } else { pick(lang, "установщик Everything", "Everything installer").to_string() };
    let text = if q.phase == "verify" {
        pick(lang, "Проверяю SHA-256…", "Verifying SHA-256…").to_string()
    } else {
        format!("{nm} · {}{}", mb(lang, q.done), if known { format!(" / {} · {} %", mb(lang, q.total), q.percent()) } else { String::new() })
    };
    ProgressView { text, percent: if known && q.phase == "download" { q.percent() } else { 0 }, indeterminate: !known || q.phase == "verify" }
}

fn item_name(lang: Lang, item: &str) -> String {
    match item {
        "engine" => pick(lang, "движок whisper.cpp", "whisper.cpp engine"),
        "base" => pick(lang, "модель Base", "Base model"),
        "small" => pick(lang, "модель Small", "Small model"),
        "turbo" => pick(lang, "модель Large-v3-turbo", "Large-v3-turbo model"),
        "installer" => pick(lang, "установщик Ollama", "Ollama installer"),
        other => other,
    }
    .to_string()
}

/// Progress line of the AI setup page (Whisper install, Ollama installer, model pull).
pub fn aisetup_progress(lang: Lang, q: &Progress) -> ProgressView {
    let known = q.known();
    let text = match q.phase.as_str() {
        "verify" => pick(lang, "Проверяю SHA-256…", "Verifying SHA-256…").to_string(),
        "unpack" => pick(lang, "Распаковываю…", "Unpacking…").to_string(),
        "start" => pick(lang, "Запускаю сервер…", "Starting the server…").to_string(),
        "pull" => format!("{} · {}{}", q.item, q.message, if known && q.done > 0.0 { format!(" · {} %", q.percent()) } else { String::new() }),
        _ => format!("{} · {}{}", item_name(lang, &q.item), size(lang, q.done), if known { format!(" / {} · {} %", size(lang, q.total), q.percent()) } else { String::new() }),
    };
    let stripe = (!known && q.phase != "pull") || matches!(q.phase.as_str(), "verify" | "unpack" | "start");
    ProgressView { text, percent: if known && matches!(q.phase.as_str(), "download" | "pull") { q.percent() } else { 0 }, indeterminate: stripe }
}

/// Final line of a download job.
pub fn progress_final(lang: Lang, q: &Progress) -> Msg {
    match q.phase.as_str() {
        "done" => Msg::ok(pick(lang, "Скачано и проверено (SHA-256). Нажмите «Запустить установщик»: Windows спросит разрешение.", "Downloaded and verified (SHA-256). Press “Run installer”: Windows will ask for permission.")),
        "cancelled" => Msg::bad(pick(lang, "Отменено. Недокачанный файл удалён.", "Cancelled. The partial file was deleted.")),
        _ => Msg::bad(if q.message.is_empty() { "error" } else { q.message.as_str() }),
    }
}

/// `everything_status` -> what the card shows.
#[derive(Clone, Debug, PartialEq)]
pub struct EverythingView {
    pub text: String,
    pub ok: bool,
    pub show_download: bool,
    pub show_run: bool,
}

pub fn everything_view(lang: Lang, st: &Value) -> EverythingView {
    let (installed, es, running, setup_ready) = (b(st, "installed"), b(st, "es"), b(st, "running"), b(st, "setupReady"));
    let ok = installed && es;
    let text = if ok {
        format!("{}{}", pick(lang, "Установлено: Everything и es.exe найдены", "Installed: Everything and es.exe found"), if running { pick(lang, ", Everything запущен", ", Everything is running") } else { "" })
    } else if installed {
        pick(lang, "Everything есть, нет es.exe (командной строки поиска)", "Everything is there, es.exe (its command-line search) is missing").to_string()
    } else {
        format!("{}{}.", pick(lang, "Everything не установлен: командная строка ищет файлы через него. Официальные файлы с voidtools.com, ", "Everything is not installed: the command bar searches files through it. Official files from voidtools.com, "), mb(lang, n(st, "size")))
    };
    EverythingView { text, ok, show_download: !(ok || (if installed { es } else { setup_ready && es })), show_run: !(installed || !setup_ready) }
}

pub fn everything_start_error(lang: Lang, res: &Value) -> Msg {
    Msg::bad(err_or(res, pick(lang, "Не удалось начать", "Could not start")))
}

pub fn everything_installer_result(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        Msg::ok(pick(lang, "Установщик запущен: Windows спросит разрешение. Когда он закончит, статус обновится сам.", "The installer has started: Windows will ask for permission. This line updates when it finishes."))
    } else {
        Msg::bad(err_or(res, pick(lang, "Windows не смогла запустить установщик", "Windows could not start the installer")))
    }
}

/// After "Run installer" the status is read again every 5 s for up to 3 minutes (36 tries).
pub const EVERYTHING_POLL_MS: u64 = 5000;
pub const EVERYTHING_POLL_TRIES: u32 = 36;

// ------------------------------------------------------------------------------------------------ AI, voice, weather, snippets

pub fn ai_key_placeholder(lang: Lang, has_key: bool) -> &'static str {
    if has_key { pick(lang, "•••••••• (ключ сохранён)", "•••••••• (key saved)") } else { pick(lang, "не нужен для локальных серверов", "not needed for local servers") }
}

pub fn key_saved(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        Msg::ok(pick(lang, "Ключ сохранён: зашифрован Windows, хранится только здесь.", "Key saved: encrypted by Windows, stored only on this computer."))
    } else {
        Msg::bad(err_or(res, "error"))
    }
}

pub fn ai_models_result(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        let c = res.get("models").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
        Msg::plain(if c > 0 { format!("{}{}", pick(lang, "Моделей: ", "Models: "), c) } else { pick(lang, "Сервер не вернул моделей", "The server returned no models").to_string() })
    } else {
        Msg::bad(err_or(res, "error"))
    }
}

pub fn ai_test_result(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        Msg::ok(format!("{} · {} {}", pick(lang, "Работает", "Works"), n(res, "ms"), pick(lang, "мс", "ms")))
    } else {
        Msg::bad(err_or(res, pick(lang, "Нет ответа", "No answer")))
    }
}

/// "Requests go only to: a, b · calls: 3" from one entry of `net_features_status().features`.
pub fn net_hosts_line(lang: Lang, f: &Value) -> String {
    let hosts: Vec<&str> = f.get("hosts").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let off = f.get("on").and_then(Value::as_bool) == Some(false);
    format!(
        "{}{} · {}{}{}",
        pick(lang, "Запросы уходят только на: ", "Requests go only to: "),
        if hosts.is_empty() { "—".to_string() } else { hosts.join(", ") },
        pick(lang, "вызовов: ", "calls: "),
        n(f, "calls"),
        if off { format!(" · {}", pick(lang, "сейчас выключено", "currently off")) } else { String::new() }
    )
}

/// `voice_has_key` answer: where the key comes from decides the placeholder and whether "Remove" shows.
pub fn voice_key_placeholder(lang: Lang, source: &str) -> &'static str {
    match source {
        "voice" => pick(lang, "•••••••• (ключ сохранён)", "•••••••• (key saved)"),
        "ai" => pick(lang, "•••••••• (используется ключ ИИ-помощника)", "•••••••• (the AI assistant key is used)"),
        _ => pick(lang, "не нужен для локальных серверов", "not needed for local servers"),
    }
}

pub fn voice_remove_visible(source: &str) -> bool {
    source == "voice"
}

pub fn voice_mic_result(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        Msg::ok(pick(lang, "Микрофон работает. Запись не сохранена и никуда не отправлена.", "The microphone works. Nothing was saved or sent."))
    } else {
        Msg::bad(err_or(res, pick(lang, "Нет ответа", "No answer")))
    }
}

pub fn voice_service_result(lang: Lang, res: &Value) -> Msg {
    if b(res, "ok") {
        Msg::ok(format!("{} · {} {} · {}", pick(lang, "Работает", "Works"), n(res, "ms"), pick(lang, "мс", "ms"), s(res, "host")))
    } else if s(res, "code") == "off" {
        Msg::warn(pick(lang, "Сначала включи «Голос в текст» выше", "Switch “Voice to text” on above first"))
    } else {
        Msg::bad(err_or(res, pick(lang, "Нет ответа", "No answer")))
    }
}

/// The long hint of the "Voice to text" switch, with the real hotkey in it ("" = not assigned).
pub fn voice_enabled_hint(lang: Lang, hotkey: &str) -> String {
    match lang {
        Lang::Ru => format!("Выключен по умолчанию. Включённый, он слушает микрофон только пока идёт диктовка: {}, скажи, затем нажми ещё раз или отпусти клавишу (режим ниже; Esc тоже останавливает): текст окажется в буфере. Запись уходит только на адрес ниже и только когда ты её закончил.", if hotkey.is_empty() { "горячая клавиша не назначена" } else { hotkey }),
        Lang::En => format!("Off by default. When on, it listens to the microphone only while you dictate: {}, speak, then press again or release the key (mode below; Esc stops too): the text lands on the clipboard. The recording goes only to the address below, and only when you finish it.", if hotkey.is_empty() { "no hotkey assigned" } else { hotkey }),
    }
}

pub fn weather_city_title(lang: Lang, city: &str) -> String {
    format!("{}{}", pick(lang, "Город: ", "City: "), if city.is_empty() { pick(lang, "не выбран", "not chosen") } else { city })
}

/// A city query shorter than two characters is not sent.
pub fn weather_query_ok(q: &str) -> bool {
    q.trim().chars().count() >= 2
}

pub fn weather_search_result(lang: Lang, res: &Value) -> Msg {
    if res.is_null() {
        return Msg::bad(pick(lang, "Нет ответа", "No answer"));
    }
    match s(res, "status").as_str() {
        "off" => Msg::warn(pick(lang, "Сначала включи погоду выше: поиск обращается к серверу карт.", "Switch the weather on above first: the search talks to a map server.")),
        "ok" => {
            let empty = res.get("results").and_then(Value::as_array).is_none_or(|a| a.is_empty());
            Msg::plain(if empty { pick(lang, "Ничего не найдено", "Nothing found") } else { "" })
        }
        _ => Msg::bad(err_or_message(res)),
    }
}

fn err_or_message(res: &Value) -> String {
    let m = s(res, "message");
    if m.is_empty() { "error".to_string() } else { m }
}

pub const CITY_SAVED: Tr = tr("Город сохранён", "City saved");

pub fn weather_refresh_result(lang: Lang, res: &Value) -> Msg {
    if res.is_null() {
        return Msg::bad(pick(lang, "Нет ответа", "No answer"));
    }
    match s(res, "status").as_str() {
        "off" => Msg::warn(pick(lang, "Погода выключена", "Weather is off")),
        "nocity" => Msg::warn(pick(lang, "Сначала выбери город", "Choose a city first")),
        "ok" => {
            let temp = res.get("now").and_then(|x| x.get("temp")).and_then(Value::as_f64).map(|t| format!(", {}°", t.round() as i64)).unwrap_or_default();
            Msg::ok(format!("{}{}{}", pick(lang, "Обновлено: ", "Updated: "), s(res, "city"), temp))
        }
        _ => Msg::bad(err_or_message(res)),
    }
}

pub fn snippets_text(lang: Lang, res: &Value) -> String {
    if res.is_null() {
        return pick(lang, "Не удалось получить фрагменты", "Could not get the snippets").to_string();
    }
    format!("Claude Code - {}\n{}\n\nCodex - {}\n{}", s(res, "claude_code_where"), s(res, "claude_code"), s(res, "codex_where"), s(res, "codex"))
}

// ------------------------------------------------------------------------------------------------ live-event status lines

/// How long a result of the mail password / connection test stays on the status line.
pub const MAIL_HOLD_SAVE_MS: u64 = 6000;
pub const MAIL_HOLD_TEST_RUN_MS: u64 = 60_000;
pub const MAIL_HOLD_TEST_DONE_MS: u64 = 8000;

/// Poll periods (ms) of the status lines.
pub fn poll_ms(id: &str) -> u64 {
    match id {
        "obs" => 2000,
        _ => 3000,
    }
}

fn off_line(lang: Lang) -> Msg {
    Msg::plain(pick(lang, "Выключено", "Off"))
}

/// The status line under a live row. `enabled`: the switch is on; `s`: the answer of the matching status command
/// (`obs_status`, `steam_status`, `liveplus_ics_status`, `mail_status`, `notif_status`). None: no answer yet, keep the old line.
pub fn status_line(lang: Lang, id: &str, enabled: bool, st: &Value) -> Option<Msg> {
    if st.is_null() {
        return None;
    }
    Some(match id {
        "obs" => obs_line(lang, enabled, st),
        "steam" => steam_line(lang, enabled, st),
        "calendar" => calendar_line(lang, enabled, st),
        "mail" => mail_line(lang, enabled, st, 0.0),
        "notifications" => notif_line(lang, enabled, st.get("status").and_then(Value::as_str).unwrap_or("")),
        _ => return None,
    })
}

fn obs_line(lang: Lang, enabled: bool, st: &Value) -> Msg {
    if !enabled {
        return off_line(lang);
    }
    match s(st, "state").as_str() {
        "connected" => Msg::ok(pick(lang, "OBS подключён", "OBS connected")),
        "auth" => Msg::warn(pick(lang, "OBS просит пароль или он неверный: введите его ниже", "OBS asks for a password, or it is wrong: enter it below")),
        _ => {
            let port = n(st, "port") as i64;
            Msg::warn(match lang {
                Lang::Ru => format!("OBS не найден (порт {port}). Запустите OBS и включите Сервис → Настройки сервера WebSocket → «Включить сервер WebSocket»"),
                Lang::En => format!("OBS not found (port {port}). Start OBS and enable Tools → WebSocket Server Settings → \"Enable WebSocket server\""),
            })
        }
    }
}

fn steam_line(lang: Lang, enabled: bool, st: &Value) -> Msg {
    if !enabled {
        off_line(lang)
    } else if !b(st, "installed") {
        Msg::warn(pick(lang, "Steam не найден на этом компьютере", "Steam not found on this computer"))
    } else if b(st, "running") {
        Msg::ok(pick(lang, "Steam запущен: слежу за загрузками (файлы steamapps читаются только на чтение)", "Steam is running: watching downloads (the steamapps files are only read)"))
    } else {
        Msg::plain(pick(lang, "Steam не запущен: начну следить, когда он откроется", "Steam is not running: I will start watching when it opens"))
    }
}

/// "in 1 d" / "2 h 5 min" / "45 min" for the minutes until the next calendar event.
pub fn ics_when(lang: Lang, minutes: i64) -> String {
    if minutes >= 1440 {
        format!("{}{}", minutes / 1440, pick(lang, " дн.", " d"))
    } else if minutes >= 60 {
        format!("{}{}{}{}", minutes / 60, pick(lang, " ч ", " h "), minutes % 60, pick(lang, " мин", " min"))
    } else {
        format!("{}{}", minutes, pick(lang, " мин", " min"))
    }
}

fn calendar_line(lang: Lang, enabled: bool, st: &Value) -> Msg {
    if !enabled {
        return off_line(lang);
    }
    let host = s(st, "host");
    let fetch_err = s(st, "fetchErr");
    let src = if host.is_empty() { String::new() } else { format!(" {}", match lang { Lang::Ru => format!("(с {host})"), Lang::En => format!("(from {host})") }) };
    let events = n(st, "events") as i64;
    match s(st, "state").as_str() {
        "nopath" => Msg::warn(pick(lang, "Укажите путь к файлу .ics или ссылку https:// / webcal://", "Enter the path to an .ics file or an https:// / webcal:// link")),
        "bad" => Msg::bad(match s(st, "why").as_str() {
            "http" => pick(lang, "Ссылка http:// без шифрования не принимается: используйте https:// или webcal://", "A plain http:// link is not accepted: use https:// or webcal://"),
            "scheme" => pick(lang, "Поддерживаются путь к файлу и ссылки https:// и webcal://, другие схемы (file://, ftp:// и т. п.) не принимаются", "A file path and https:// or webcal:// links are supported; other schemes (file://, ftp://, ...) are rejected"),
            _ => pick(lang, "Ссылка не распознана", "The link is not valid"),
        }),
        "missing" if !host.is_empty() => {
            let busy = b(st, "busy");
            if busy || fetch_err.is_empty() {
                Msg::warn(match lang { Lang::Ru => format!("Загружаю календарь с {host}…"), Lang::En => format!("Downloading the calendar from {host}…") })
            } else {
                Msg::bad(match lang { Lang::Ru => format!("Не удалось загрузить календарь с {host}: {fetch_err}"), Lang::En => format!("Could not download the calendar from {host}: {fetch_err}") })
            }
        }
        "missing" => Msg::bad(pick(lang, "Файл не найден или не читается", "The file was not found or cannot be read")),
        "big" => Msg::bad(pick(lang, "Файл слишком большой (больше 8 МБ)", "The file is too large (over 8 MB)")),
        _ if !fetch_err.is_empty() => Msg::warn(match lang {
            Lang::Ru => format!("Показана сохранённая копия{src}; последняя загрузка не удалась: {fetch_err}"),
            Lang::En => format!("Showing the saved copy{src}; the last download failed: {fetch_err}"),
        }),
        _ => match st.get("next").filter(|x| !x.is_null()) {
            None => Msg::warn(match lang { Lang::Ru => format!("Событий в файле: {events}. Предстоящих со временем начала нет"), Lang::En => format!("Events in the file: {events}. None upcoming with a start time") }),
            Some(next) => {
                let when = ics_when(lang, n(next, "inMin") as i64);
                let title = s(next, "title");
                Msg::ok(match lang {
                    Lang::Ru => format!("Событий в календаре{src}: {events}. Ближайшее через {when}: {title}"),
                    Lang::En => format!("Events in the calendar{src}: {events}. Next in {when}: {title}"),
                })
            }
        },
    }
}

/// "just now" / "5 min ago" / "2 h ago" for a unix time `at` seen at `now_s`.
pub fn ago(lang: Lang, now_s: f64, at: f64) -> String {
    let d = (now_s - at).max(0.0);
    if d < 60.0 {
        pick(lang, "только что", "just now").to_string()
    } else if d < 3600.0 {
        format!("{}{}", (d / 60.0).floor() as i64, pick(lang, " мин назад", " min ago"))
    } else {
        format!("{}{}", (d / 3600.0).floor() as i64, pick(lang, " ч назад", " h ago"))
    }
}

fn mail_line(lang: Lang, enabled: bool, st: &Value, now_s: f64) -> Msg {
    if !enabled {
        return off_line(lang);
    }
    match s(st, "state").as_str() {
        "nosetup" => Msg::warn(s(st, "message")),
        "error" => Msg::bad(s(st, "message")),
        "ok" => Msg::ok(match lang {
            Lang::Ru => format!("Ящик проверен {}, непрочитанных писем: {}", ago(lang, now_s, n(st, "at")), n(st, "unseen") as i64),
            Lang::En => format!("Mailbox checked {}, unread messages: {}", ago(lang, now_s, n(st, "at")), n(st, "unseen") as i64),
        }),
        _ => Msg::plain(pick(lang, "Подключаюсь…", "Connecting…")),
    }
}

/// The mailbox line needs the clock for "5 min ago"; everything else is the same as `status_line`.
pub fn mail_status_line(lang: Lang, enabled: bool, st: &Value, now_s: f64) -> Msg {
    mail_line(lang, enabled, st, now_s)
}

/// Placeholder of the mail password field: saved or not.
pub fn mail_password_placeholder(lang: Lang, has_password: bool) -> &'static str {
    if has_password { pick(lang, "••••••• сохранён", "••••••• saved") } else { pick(lang, "Пароль приложения", "App password") }
}

pub fn mail_password_saved(lang: Lang, ok: bool) -> Msg {
    if ok {
        Msg::ok(pick(lang, "Пароль сохранён (зашифрован Windows DPAPI)", "Password saved (encrypted with Windows DPAPI)"))
    } else {
        Msg::bad(pick(lang, "Не удалось сохранить пароль", "Could not save the password"))
    }
}

pub fn mail_testing(lang: Lang) -> Msg {
    Msg::plain(pick(lang, "Проверяю подключение…", "Testing the connection…"))
}

pub fn mail_test_result(lang: Lang, res: &Value) -> Msg {
    if res.is_null() {
        return Msg::bad(pick(lang, "Проверка не удалась", "The test failed"));
    }
    Msg::new(s(res, "message"), if b(res, "ok") { Kind::Ok } else { Kind::Bad })
}

pub fn obs_password_saved(lang: Lang, ok: bool) -> Option<Msg> {
    (!ok).then(|| Msg::bad(pick(lang, "Не удалось сохранить пароль", "Could not save the password")))
}

/// What the notification-access button and line show for `notif_status().status`.
pub fn notif_line(lang: Lang, enabled: bool, status: &str) -> Msg {
    if !enabled {
        return off_line(lang);
    }
    match status {
        "granted" => Msg::ok(pick(lang, "Доступ к уведомлениям разрешён: карточки приходят", "Notification access granted: cards arrive")),
        "denied" => Msg::bad(pick(lang, "Доступ запрещён. Разрешите в Параметры → Конфиденциальность → Уведомления", "Access denied. Allow it in Settings → Privacy → Notifications")),
        "unsupported" => Msg::warn(pick(lang, "Windows не даёт этому приложению читать уведомления (обычно нужен пакет приложения). Карточки уведомлений недоступны, переключатель ничего не изменит.", "Windows does not let this app read notifications (it usually needs an app package). Notification cards are unavailable; the switch changes nothing.")),
        _ => Msg::warn(pick(lang, "Windows ещё не спрашивал разрешение. Разрешите в Параметры → Конфиденциальность → Уведомления или нажмите кнопку", "Windows has not been asked yet. Allow it in Settings → Privacy → Notifications or press the button")),
    }
}

/// The "Allow access" button shows while Windows has not granted (or cannot grant) access and the switch is on.
pub fn notif_allow_visible(enabled: bool, status: &str) -> bool {
    enabled && matches!(status, "pending" | "denied" | "unsupported")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn update_flow_checks_then_installs() {
        let mut f = UpdateFlow::default();
        assert_eq!(f.button_label(Lang::En), "Check now");
        let (call, msg) = f.click(Lang::En).unwrap();
        assert_eq!((call, msg.text.as_str()), (BackendCall::UpdateCheck, "Checking…"));
        assert!(f.click(Lang::En).is_none(), "busy");
        let m = f.on_check(Lang::En, &json!({"status": "available", "version": "0.4.0", "notes": "x".repeat(400)}));
        assert_eq!(m.kind, Kind::Ok);
        assert!(m.text.starts_with("Version available: 0.4.0. xxx") && m.text.chars().count() == "Version available: 0.4.0. ".len() + 300);
        assert_eq!(f.button_label(Lang::Ru), "Установить и перезапустить");
        assert_eq!(f.click(Lang::Ru).unwrap().0, BackendCall::UpdateInstall);
        assert_eq!(f.on_install(Lang::En, &json!({"status": "error", "error": "no net"})).unwrap().text, "Install failed: no net");
        assert!(f.on_install(Lang::En, &json!({"status": "ok"})).is_none());
        let mut g = UpdateFlow::default();
        g.click(Lang::En);
        assert_eq!(g.on_check(Lang::Ru, &json!({"status": "none"})).text, "У вас последняя версия.");
        g.click(Lang::En);
        assert_eq!(g.on_check(Lang::En, &json!({"status": "error", "error": "offline"})), Msg::bad("Could not check: offline"));
        g.click(Lang::En);
        assert_eq!(g.on_check(Lang::En, &Value::Null).kind, Kind::Bad, "no answer is an error line");
    }

    #[test]
    fn report_and_backup_messages() {
        assert!(report_result(Lang::En, &json!({"ok": true, "log": true})).text.contains("the log tail"));
        assert!(report_result(Lang::Ru, &json!({"ok": true, "log": false})).text.contains("лога нет"));
        assert_eq!(report_result(Lang::En, &json!({"ok": false, "error": "disk"})).text, "Could not create the report: disk");
        assert_eq!(counts(Lang::Ru, 3, 1), "Применено 3, пропущено 1");
        assert_eq!(import_call(Lang::En, 2_000_000, "{}").unwrap_err().text, "The file is larger than 1 MB");
        assert!(import_call(Lang::En, 10, "{}").is_ok());
        assert_eq!(import_result(Lang::En, &json!({"ok": true, "applied": 5, "skipped": 2})), (Msg::ok("Applied 5, skipped 2"), true));
        assert_eq!(import_result(Lang::En, &json!({"ok": true, "applied": 5, "skipped": 0, "error": "partial"})), (Msg::bad("Applied 5, skipped 0 · partial"), true));
        assert_eq!(import_result(Lang::En, &json!({"ok": false, "error": "bad json", "skipped": 4})), (Msg::bad("bad json · Applied 0, skipped 4"), false));
        assert_eq!(import_result(Lang::Ru, &Value::Null), (Msg::bad("Не удалось импортировать"), false));
        assert_eq!(export_result(Lang::En, &json!({"ok": true, "path": "C:\\d\\s.json"})).text, "Saved: C:\\d\\s.json");
        assert_eq!(export_result(Lang::En, &Value::Null).text, "Could not save");
    }

    #[test]
    fn confirmations() {
        let mut c = Confirm::delete_from_disk();
        assert!(!c.click(0) && c.is_armed(2999) && !c.is_armed(3000), "armed for 3 s");
        assert!(c.click(1000), "second click confirms");
        assert!(!c.click(10_000) && !c.click(14_000), "an expired arm re-arms instead of confirming");
        let mut w = Confirm::wipe();
        assert!(!w.click(0));
        assert!(w.is_armed(10_000_000), "no expiry");
        w.cancel();
        assert!(!w.is_armed(1) && !w.click(1));
    }

    #[test]
    fn profile_names() {
        assert_eq!(profile_name("  Work\n ").as_deref(), Some("Work"));
        assert_eq!(profile_name("   "), None);
        assert_eq!(profile_name(&"x".repeat(60)).unwrap().chars().count(), PROFILE_NAME_MAX);
        assert_eq!(profile_result(Lang::En, &json!({"ok": false, "error": "full"}), "").text, "At most 8 profiles: delete one first");
        assert_eq!(profile_result(Lang::En, &json!({"ok": false}), "").text, "Failed");
        assert_eq!(profile_applied_text(Lang::Ru, "Игра"), "Применён профиль «Игра»");
    }

    #[test]
    fn progress_lines() {
        let p = Progress::from_event(&json!({"id": "everything-1", "item": "installer", "done": 5_000_000, "total": 10_000_000, "phase": "download"})).unwrap();
        let v = everything_progress(Lang::En, &p);
        assert_eq!((v.text.as_str(), v.percent, v.indeterminate), ("Everything installer · 5.0 MB / 10.0 MB · 50 %", 50, false));
        let unknown = Progress { total: 0.0, ..p.clone() };
        let v = everything_progress(Lang::Ru, &unknown);
        assert_eq!((v.text.as_str(), v.percent, v.indeterminate), ("установщик Everything · 5.0 МБ", 0, true));
        let verify = Progress { phase: "verify".into(), ..p.clone() };
        assert_eq!(everything_progress(Lang::En, &verify).text, "Verifying SHA-256…");
        assert!(everything_progress(Lang::En, &verify).indeterminate);
        assert!(!Progress { done: 10_300_000.0, ..p.clone() }.known(), "overshoot beyond 2 % is unknown");
        assert_eq!(Progress { done: 10_100_000.0, ..p.clone() }.percent(), 100);
        let w = Progress::from_event(&json!({"id": "whisper", "item": "small", "done": 2_500_000_000u64, "total": 5_000_000_000u64, "phase": "download"})).unwrap();
        assert_eq!(aisetup_progress(Lang::En, &w).text, "Small model · 2.5 GB / 5.0 GB · 50 %");
        let pull = Progress { phase: "pull".into(), item: "qwen".into(), message: "pulling".into(), ..w.clone() };
        let v = aisetup_progress(Lang::En, &pull);
        assert_eq!((v.text.as_str(), v.percent, v.indeterminate), ("qwen · pulling · 50 %", 50, false));
        assert!(aisetup_progress(Lang::En, &Progress { phase: "unpack".into(), ..w.clone() }).indeterminate);
        assert_eq!(size(Lang::En, 350_000_000.0), "350 MB");
        assert!(Progress::from_event(&json!(3)).is_none());
        assert!(Progress { phase: "cancelled".into(), ..w }.is_final());
        assert_eq!(progress_final(Lang::En, &Progress { phase: "error".into(), message: "".into(), ..p }).text, "error");
    }

    #[test]
    fn everything_card_buttons() {
        let none = everything_view(Lang::En, &json!({"installed": false, "es": false, "size": 3_000_000, "setupReady": false}));
        assert!(none.text.ends_with("voidtools.com, 3.0 MB.") && none.show_download && !none.show_run);
        let ready = everything_view(Lang::En, &json!({"installed": false, "es": true, "size": 3_000_000, "setupReady": true}));
        assert!(!ready.show_download && ready.show_run, "downloaded: only 'Run installer'");
        let ok = everything_view(Lang::Ru, &json!({"installed": true, "es": true, "running": true}));
        assert!(ok.ok && !ok.show_download && !ok.show_run && ok.text.ends_with(", Everything запущен"));
        let no_es = everything_view(Lang::En, &json!({"installed": true, "es": false}));
        assert!(no_es.show_download && !no_es.show_run && !no_es.ok);
    }

    #[test]
    fn ai_voice_weather_messages() {
        assert_eq!(ai_models_result(Lang::En, &json!({"ok": true, "models": ["a", "b"]})).text, "Models: 2");
        assert_eq!(ai_models_result(Lang::En, &json!({"ok": true, "models": []})).text, "The server returned no models");
        assert_eq!(ai_test_result(Lang::En, &json!({"ok": true, "ms": 120})), Msg::ok("Works · 120 ms"));
        assert_eq!(ai_test_result(Lang::En, &Value::Null), Msg::bad("No answer"));
        assert_eq!(net_hosts_line(Lang::En, &json!({"hosts": ["a.com", "b.com"], "calls": 4, "on": false})), "Requests go only to: a.com, b.com · calls: 4 · currently off");
        assert_eq!(net_hosts_line(Lang::Ru, &json!({"hosts": [], "calls": 0})), "Запросы уходят только на: — · вызовов: 0");
        assert_eq!(voice_key_placeholder(Lang::En, "ai"), "•••••••• (the AI assistant key is used)");
        assert!(voice_remove_visible("voice") && !voice_remove_visible("ai"));
        assert_eq!(voice_service_result(Lang::En, &json!({"ok": false, "code": "off"})).kind, Kind::Warn);
        assert_eq!(voice_service_result(Lang::En, &json!({"ok": true, "ms": 90, "host": "api.groq.com"})).text, "Works · 90 ms · api.groq.com");
        assert!(voice_enabled_hint(Lang::En, "Ctrl+Alt+D").contains(": Ctrl+Alt+D, speak"));
        assert!(voice_enabled_hint(Lang::Ru, "").contains("горячая клавиша не назначена"));
        assert_eq!(weather_city_title(Lang::En, ""), "City: not chosen");
        assert!(!weather_query_ok(" a ") && weather_query_ok("Берлин"));
        assert_eq!(weather_search_result(Lang::En, &json!({"status": "off"})).kind, Kind::Warn);
        assert_eq!(weather_search_result(Lang::En, &json!({"status": "ok", "results": []})).text, "Nothing found");
        assert_eq!(weather_search_result(Lang::En, &json!({"status": "ok", "results": [{"name": "x"}]})).text, "");
        assert_eq!(weather_search_result(Lang::En, &json!({"status": "err", "message": "boom"})).text, "boom");
        assert_eq!(weather_refresh_result(Lang::En, &json!({"status": "ok", "city": "Kazan", "now": {"temp": 12.6}})).text, "Updated: Kazan, 13°");
        assert_eq!(weather_refresh_result(Lang::En, &json!({"status": "nocity"})).text, "Choose a city first");
        assert_eq!(snippets_text(Lang::En, &json!({"claude_code_where": "a", "claude_code": "b", "codex_where": "c", "codex": "d"})), "Claude Code - a\nb\n\nCodex - c\nd");
    }

    #[test]
    fn status_lines_follow_the_live_ext_page() {
        assert_eq!(status_line(Lang::En, "obs", false, &json!({"state": "connected"})).unwrap().text, "Off");
        assert_eq!(status_line(Lang::En, "obs", true, &json!({"state": "connected"})).unwrap(), Msg::ok("OBS connected"));
        assert_eq!(status_line(Lang::En, "obs", true, &json!({"state": "auth"})).unwrap().kind, Kind::Warn);
        assert!(status_line(Lang::En, "obs", true, &json!({"state": "none", "port": 4455})).unwrap().text.starts_with("OBS not found (port 4455)"));
        assert!(status_line(Lang::En, "obs", true, &Value::Null).is_none(), "no answer keeps the old line");
        assert_eq!(status_line(Lang::En, "steam", true, &json!({"installed": false})).unwrap().kind, Kind::Warn);
        assert_eq!(status_line(Lang::En, "steam", true, &json!({"installed": true, "running": true})).unwrap().kind, Kind::Ok);
        assert_eq!(status_line(Lang::En, "steam", true, &json!({"installed": true, "running": false})).unwrap().kind, Kind::Plain);
        assert_eq!(status_line(Lang::En, "notifications", true, &json!({"status": "granted"})).unwrap().kind, Kind::Ok);
        assert!(notif_allow_visible(true, "pending") && !notif_allow_visible(true, "granted") && !notif_allow_visible(false, "denied"));
        assert!(status_line(Lang::En, "bogus", true, &json!({})).is_none());
        assert_eq!(poll_ms("obs"), 2000);
        assert_eq!(poll_ms("mail"), 3000);
    }

    #[test]
    fn calendar_and_mail_lines() {
        let cal = |v: Value| status_line(Lang::En, "calendar", true, &v).unwrap();
        assert_eq!(cal(json!({"state": "nopath"})).kind, Kind::Warn);
        assert_eq!(cal(json!({"state": "bad", "why": "http"})).text, "A plain http:// link is not accepted: use https:// or webcal://");
        assert_eq!(cal(json!({"state": "bad", "why": "scheme"})).kind, Kind::Bad);
        assert_eq!(cal(json!({"state": "missing", "host": "cal.example", "busy": true})).text, "Downloading the calendar from cal.example…");
        assert_eq!(cal(json!({"state": "missing", "host": "cal.example", "fetchErr": "timeout"})).text, "Could not download the calendar from cal.example: timeout");
        assert_eq!(cal(json!({"state": "missing"})).text, "The file was not found or cannot be read");
        assert_eq!(cal(json!({"state": "big"})).text, "The file is too large (over 8 MB)");
        assert_eq!(cal(json!({"state": "ok", "host": "h", "fetchErr": "x"})).text, "Showing the saved copy (from h); the last download failed: x");
        assert_eq!(cal(json!({"state": "ok", "events": 3})).text, "Events in the file: 3. None upcoming with a start time");
        assert_eq!(cal(json!({"state": "ok", "events": 3, "next": {"inMin": 125, "title": "Standup"}})), Msg::ok("Events in the calendar: 3. Next in 2 h 5 min: Standup"));
        assert_eq!((ics_when(Lang::En, 2880), ics_when(Lang::Ru, 45), ics_when(Lang::En, 60)), ("2 d".to_string(), "45 мин".to_string(), "1 h 0 min".to_string()));
        assert_eq!(mail_status_line(Lang::En, true, &json!({"state": "ok", "at": 940.0, "unseen": 2}), 1000.0), Msg::ok("Mailbox checked 1 min ago, unread messages: 2"));
        assert_eq!(ago(Lang::Ru, 100.0, 90.0), "только что");
        assert_eq!(ago(Lang::En, 20_000.0, 10_000.0), "2 h ago");
        assert_eq!(mail_status_line(Lang::En, true, &json!({"state": "nosetup", "message": "Fill in the host"}), 0.0).kind, Kind::Warn);
        assert_eq!(mail_status_line(Lang::En, true, &json!({"state": "?"}), 0.0).text, "Connecting…");
        assert_eq!(mail_test_result(Lang::En, &json!({"ok": true, "message": "Connected"})), Msg::ok("Connected"));
        assert_eq!(mail_test_result(Lang::En, &Value::Null).text, "The test failed");
        assert_eq!(mail_password_placeholder(Lang::En, true), "••••••• saved");
        assert!(obs_password_saved(Lang::En, true).is_none() && obs_password_saved(Lang::En, false).is_some());
        assert!(mail_password_saved(Lang::En, true).text.contains("DPAPI"));
    }
}
