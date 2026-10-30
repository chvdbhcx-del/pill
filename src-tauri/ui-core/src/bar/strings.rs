//! Every user-visible string of the command bar (bar.js `L`, `SYS`, `SUBS`, `VERR`), as `(ru, en)` pairs for `Tr::tr(lang)`.
//! Texts are copied verbatim from the JS so the two UIs read the same; the backend sends English and these are the Russian twins.

pub type S = (&'static str, &'static str);

pub const PH: S = ("Приложения, файлы, заметки, буфер или > команда", "Search apps, files, notes, clipboard, or > run");
pub const TIP: S = (
    "Попробуй 5*8, 5 км в милях, lock, > ipconfig. Фильтры: c буфер, n заметки, f файлы, a приложения, s настройки",
    "Try 5*8, 5 km in mi, lock, > ipconfig. Filters: c clipboard, n notes, f files, a apps, s settings",
);
pub const CLIP: S = ("Буфер", "Clipboard");
pub const NOTE: S = ("Заметки", "Notes");
pub const SHELF: S = ("Полка", "Shelf");
pub const REMINDER: S = ("Напоминание", "Reminder");
pub const SETTING: S = ("Настройки", "Settings");
pub const ACTION: S = ("Действие", "Action");
pub const TAB: S = ("Вкладка", "Tab");
pub const APP: S = ("Приложение", "Application");
pub const FILE: S = ("Файл", "File");
pub const FOLDER: S = ("Папка", "Folder");
pub const SHELL: S = ("Команда", "Command");
pub const COPY: S = ("Скопировать", "Copy");
pub const RECENT: S = ("Недавнее", "Recent");
pub const AGAIN: S = ("Ещё раз Enter", "Press Enter again");
pub const UNINSTALL: S = ("Удалить", "Uninstall");
pub const UNINSTALL_RISKY: S = ("Системный компонент: от него могут зависеть другие программы", "System component: other programs may depend on it");
pub const ASK: S = ("Спросить ИИ", "Ask AI");
pub const ASK_TIP: S = ("Напиши вопрос после «? », «ии » или «ai »", "Type your question after \"? \", \"ии \" or \"ai \"");
pub const ASK_GO: S = ("Enter — отправить", "Enter: send");
pub const THINK: S = ("Думаю…", "Thinking…");
pub const COPIED: S = ("Скопировано", "Copied");
pub const COPY_ANS: S = ("Копировать ответ", "Copy the answer");
pub const COPY_CODE: S = ("Копировать", "Copy");
pub const DLG: S = ("Диалог · Esc — закончить", "Dialogue · Esc to end");
pub const DLG_PH: S = ("Продолжить разговор…", "Continue the conversation…");
pub const NEW_CONV: S = ("Новый разговор", "New conversation");
pub const NEW_CONV_SUB: S = ("забыть историю и искать как обычно", "forget the history and search as usual");
pub const ASK_OFF: S = ("ИИ выключен. Enter — открыть Настройки → ИИ и включить", "AI is off. Enter: open Settings > AI to turn it on");
pub const ASK_NO_URL: S = ("Не указан адрес. Enter — открыть Настройки → ИИ", "No API address. Enter: open Settings > AI");
pub const ASK_NO_MODEL: S = ("Не выбрана модель. Enter — открыть Настройки → ИИ", "No model chosen. Enter: open Settings > AI");
pub const ASK_FAIL: S = ("ИИ не ответил", "The AI did not answer");
pub const MIC_TIP: S = ("Диктовка (F4)", "Dictate (F4)");
pub const MIC_STOP: S = ("Остановить диктовку (F4)", "Stop dictation (F4)");
pub const MIC_BUSY: S = ("Распознаю…", "Transcribing…");
pub const VOICE_OFF: S = ("Голос в текст не настроен. Нажми здесь: Настройки → «Голос в текст»", "Voice to text is not set up. Click here: Settings > Voice to text");
pub const VOICE_FAIL: S = ("Диктовка не удалась", "Dictation failed");
pub const OPEN_TIP: S = (
    "Enter — открыть · Ctrl+Enter — показать в папке · Ctrl+Shift+Enter — от администратора · Ctrl+Shift+C — скопировать путь",
    "Enter: open · Ctrl+Enter: show in folder · Ctrl+Shift+Enter: as administrator · Ctrl+Shift+C: copy path",
);
pub const PATH_TIP: S = ("Скопировать путь (Ctrl+Shift+C)", "Copy path (Ctrl+Shift+C)");
pub const ADMIN_TIP: S = ("Запустить от имени администратора (Ctrl+Shift+Enter)", "Run as administrator (Ctrl+Shift+Enter)");
pub const FOLDER_TIP: S = ("Показать в папке (Ctrl+Enter)", "Show in folder (Ctrl+Enter)");
pub const CTX_OPEN: S = ("Открыть", "Open");
pub const CTX_FOLDER: S = ("Показать в папке", "Show in folder");
pub const PIN_TIP: S = ("Закрепить в Pill (Ctrl+P)", "Pin to Pill (Ctrl+P)");
pub const CTX_PIN: S = ("Закрепить в Pill", "Pin to Pill");
pub const UNINST_TIP: S = ("Удалить приложение…", "Uninstall…");
pub const CTX_UNINSTALL: S = ("Удалить…", "Uninstall…");
pub const PINNED: S = ("Закреплено в Pill", "Pinned to Pill");
pub const PINNED_ALREADY: S = ("Уже закреплено", "Already pinned");
pub const CTX_PATH: S = ("Скопировать путь", "Copy path");
pub const CTX_ADMIN: S = ("Запустить от имени администратора", "Run as administrator");

/// `SYS`: power rows, keyed by the backend payload; (title, icon name).
pub fn system_row(payload: &str) -> Option<(S, &'static str)> {
    Some(match payload {
        "lock" => (("Заблокировать компьютер", "Lock the computer"), "lock"),
        "sleep" => (("Спящий режим", "Sleep"), "moon"),
        "hibernate" => (("Гибернация", "Hibernate"), "moon"),
        "restart" => (("Перезагрузка", "Restart"), "refresh"),
        "shutdown" => (("Выключение", "Shut down"), "power"),
        "signout" => (("Выйти из учётной записи", "Sign out"), "logout"),
        _ => return None,
    })
}

/// `SUBS`: backend subtitles that have a Russian twin.
pub fn known_sub(en: &str) -> Option<S> {
    Some(match en {
        "Opens your default browser" => ("Откроется в браузере по умолчанию", "Opens your default browser"),
        "Press Enter twice to confirm" => ("Нажми Enter дважды для подтверждения", "Press Enter twice to confirm"),
        "Runs exactly what you type" => ("Выполняется ровно то, что ты введёшь", "Runs exactly what you type"),
        "cmd /c, hidden window" => ("cmd /c, окно скрыто", "cmd /c, hidden window"),
        _ => return None,
    })
}

/// `L[key]` lookup by the `message` key the backend answers with for a pin (`pinned` / `pinnedAlready`).
pub fn message_key(key: &str) -> Option<S> {
    match key {
        "pinned" => Some(PINNED),
        "pinnedAlready" => Some(PINNED_ALREADY),
        _ => None,
    }
}

/// Voice error codes -> text (`VERR`); `None` = unknown code (the generic failure text with the message is used).
pub enum VoiceErr {
    /// show the "voice is not set up" text, clickable to Settings > Voice to text
    NotSetUp,
    Text(S),
}

pub fn voice_error(code: &str) -> Option<VoiceErr> {
    Some(match code {
        "off" | "no_key" | "no_endpoint" => VoiceErr::NotSetUp,
        "mic" => VoiceErr::Text(("Нет доступа к микрофону", "Cannot use the microphone")),
        "too_short" => VoiceErr::Text(("Слишком короткая запись: скажи что-нибудь подольше", "Recording too short: say something longer")),
        "silent" => VoiceErr::Text(("Микрофон ничего не услышал: проверь микрофон по умолчанию", "The microphone heard nothing: check the default microphone")),
        "empty" => VoiceErr::Text(("Речь не распознана", "No speech recognised")),
        _ => return None,
    })
}
