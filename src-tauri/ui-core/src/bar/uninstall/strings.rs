//! Strings of the uninstall panel (uninstall-ui.js `L`), copied verbatim as `(ru, en)` pairs, plus the size formatter and the lookups that
//! localise what the backend reports (item kinds, scan stages, per-item notes). Unknown stages / notes show the backend's English text.

use crate::bar::lang::{Lang, Tr};

pub type S = (&'static str, &'static str);

pub const UNINSTALL: S = ("Удалить", "Uninstall");
pub const CANCEL: S = ("Отмена", "Cancel");
pub const CLOSE: S = ("Закрыть", "Close");
pub const FOLDER: S = ("Папка: ", "Folder: ");
pub const STORE: S = ("Приложение из Microsoft Store", "Microsoft Store app");
pub const RISKY: S = ("Системный компонент: от него могут зависеть другие программы.", "System component: other programs may depend on it.");
pub const SILENT: S = ("Тихо, без вопросов (если установщик это умеет)", "Silent, no prompts (if the uninstaller supports it)");
pub const HOW: S = (
    "Сначала запустится собственный деинсталлятор приложения. Потом Pill глубоко поищет остатки по всему ПК и покажет список: ничего не удаляется, пока ты не отметишь и не нажмёшь кнопку.",
    "The app\u{2019}s own uninstaller runs first. Then Pill searches the whole PC for leftovers and shows a list: nothing is deleted until you tick it and press the button.",
);
pub const RUNNING: S = ("Идёт удаление. Закончи его в окне деинсталлятора, Pill его не закрывает.", "Uninstalling. Finish it in the uninstaller\u{2019}s window; Pill never closes it.");
pub const WAITING: S = ("Жду, пока приложение исчезнет из списка установленных…", "Waiting for the app to leave the installed list…");
pub const CHECK_AGAIN: S = ("Проверить снова", "Check again");
pub const HIDE: S = ("Скрыть", "Hide");
pub const SCANNING: S = ("Ищу остатки по всему ПК…", "Searching the whole PC for leftovers…");
pub const FOUND: S = ("найдено: ", "found: ");
pub const STOP_WAIT: S = ("Не ждать", "Stop waiting");
pub const STOP_SCAN: S = ("Остановить поиск", "Stop searching");
pub const S_RUNNING: S = (": деинсталлятор работает", ": the uninstaller is running");
pub const S_SCANNING: S = (": ищу остатки, найдено ", ": searching for leftovers, found ");
pub const S_DELETING: S = (": удаляю выбранное…", ": deleting the selection\u{2026}");
pub const S_ADMIN: S = (": жду разрешения администратора", ": waiting for administrator permission");
pub const S_REVIEW: S = (": остатков к проверке: ", ": leftovers to review: ");
pub const S_DONE: S = (": готово, сводка", ": done, summary");
pub const S_STILL: S = (": всё ещё установлено", ": still installed");
pub const S_ERROR: S = (": ошибка", ": failed");
pub const STRIP_TIP: S = ("Открыть панель удаления (Ctrl+U)", "Open the uninstall panel (Ctrl+U)");
pub const STILL: S = (
    "Приложение всё ещё установлено (деинсталлятор отменён или ещё работает). Ничего не удалено.",
    "The app is still installed (the uninstaller was cancelled or is still running). Nothing was deleted.",
);
pub const LEFTOVERS: S = ("Остатки: ", "Leftovers: ");
pub const NONE: S = ("Остатков не найдено. Приложение удалено начисто.", "No leftovers found. The app is gone.");
pub const ITEMS: S = (" объектов", " items");
pub const CERTAIN: S = ("Точно его", "Certain");
pub const PROBABLE: S = ("Вероятно его", "Probable");
pub const ADMIN: S = ("Нужны права администратора", "Needs administrator");
pub const CERTAIN_TIP: S = (
    "Указывают в папку установки, несут GUID продукта или называются как приложение. Отмечены.",
    "Point into the install folder, carry the product GUID or are named like the app. Pre-ticked.",
);
pub const PROBABLE_TIP: S = (
    "Содержат полное название приложения как слова или это общая папка издателя. Не отмечены.",
    "Contain the app\u{2019}s full name as words, or a shared publisher folder. Not ticked.",
);
pub const SEL_ALL: S = ("Все «точно»", "All certain");
pub const SEL_ALL_TIP: S = ("Отметить всё, что точно принадлежит приложению", "Tick everything that certainly belongs to the app");
pub const SELECTED: S = ("Выбрано: ", "Selected: ");
pub const DEL: S = ("Удалить выбранное", "Delete selected");
pub const ADMIN_DEL: S = ("Удалить системные остатки (администратор)", "Remove system leftovers (administrator)");
pub const ADMIN_SCAN: S = ("Поискать ещё как администратор", "Search more as administrator");
pub const ADMIN_ERR: S = ("Права администратора не получены: ", "Administrator step did not run: ");
pub const CANCELLED: S = ("запрос Windows отклонён", "the Windows prompt was declined");
pub const MANUAL: S = ("Только список, Pill это не трогает (драйверы и т. п.):", "Listed only, Pill does not touch these (drivers and the like):");
pub const SAFE: S = (
    "Файлы уходят в Корзину, ключи реестра сначала сохраняются в .reg-копию (её можно импортировать обратно).",
    "Files go to the Recycle Bin; registry keys are saved to a .reg backup first (you can import it back).",
);
pub const HIDDEN_OTHERS: S = (
    " объектов в профилях других пользователей не показаны: удали их из сеанса этого пользователя.",
    " items in other users\u{2019} profiles are not shown: remove them from that user\u{2019}s own session.",
);
pub const CUT: S = (
    "Поиск остановлен по лимиту времени или числа объектов: список может быть неполным.",
    "The scan hit its time or entry limit: the list may be incomplete.",
);
pub const DELETING: S = ("Удаляю выбранное…", "Deleting the selection…");
pub const ELEV_SCAN: S = ("Жду разрешения Windows: поиск от администратора…", "Waiting for the Windows permission prompt: searching as administrator…");
pub const ELEV_DEL: S = ("Жду разрешения Windows: удаляю системные остатки…", "Waiting for the Windows permission prompt: removing system leftovers…");
pub const DONE_TITLE: S = ("Готово", "Done");
pub const REMOVED: S = ("Удалено: ", "Removed ");
pub const FILES_W: S = ("файлов и папок", "files and folders");
pub const KEYS_W: S = ("ключей реестра", "registry keys");
pub const OTHER_W: S = ("служб и задач", "services and tasks");
pub const NEED_ADMIN: S = ("Нужны права администратора: ", "Need administrator: ");
pub const FAILED: S = ("Не удалось:", "Could not delete:");
pub const BACKUP: S = ("Копия реестра: ", "Registry backup: ");
pub const LOG: S = ("Журнал: ", "Log: ");
pub const OPEN_BK: S = ("Открыть папку с копиями", "Open backups folder");
pub const ERR: S = ("Не получилось: ", "Failed: ");

/// `L.kind[k]`: item kinds; unknown ones show the raw word.
pub fn kind_label(kind: &str, lang: Lang) -> String {
    let t: Option<S> = match kind {
        "folder" => Some(("Папка", "Folder")),
        "file" => Some(("Файл", "File")),
        "shortcut" => Some(("Ярлык", "Shortcut")),
        "key" => Some(("Ключ реестра", "Registry key")),
        "value" => Some(("Значение реестра", "Registry value")),
        "autorun" => Some(("Автозапуск", "Autostart")),
        "service" => Some(("Служба", "Service")),
        "task" => Some(("Задача", "Task")),
        "item" => Some(("Объект", "Item")),
        _ => None,
    };
    t.map(|t| t.tr(lang).to_string()).unwrap_or_else(|| kind.to_string())
}

/// `L.stage[step]`: scan stages as the backend names them.
pub fn stage(step: &str, lang: Lang) -> String {
    let t: Option<S> = match step {
        "starting" => Some(("запуск", "starting")),
        "files and folders" => Some(("файлы и папки", "files and folders")),
        "registry (user)" => Some(("реестр (пользователь)", "registry (user)")),
        "registry (system)" => Some(("реестр (система)", "registry (system)")),
        "Windows Installer records" => Some(("записи Windows Installer", "Windows Installer records")),
        "firewall rules" => Some(("правила брандмауэра", "firewall rules")),
        "services" => Some(("службы", "services")),
        "scheduled tasks" => Some(("задачи планировщика", "scheduled tasks")),
        _ => None,
    };
    t.map(|t| t.tr(lang).to_string()).unwrap_or_else(|| step.to_string())
}

/// `L.note[note]`: why an item is listed.
pub fn note(note: &str, lang: Lang) -> String {
    let t: Option<S> = match note {
        "install folder" => Some(("папка установки", "install folder")),
        "inside the install folder" => Some(("внутри папки установки", "inside the install folder")),
        "shared with another installed app" => Some(("общая с другой программой", "shared with another installed app")),
        "named like the app" => Some(("называется как приложение", "named like the app")),
        "named like the app, under the publisher" => Some(("как приложение, внутри папки издателя", "named like the app, under the publisher")),
        "named like the app, in a place for your own files" => Some((
            "как приложение, но в папке для твоих файлов: проверь перед удалением",
            "named like the app, but in a place for your own files: check before deleting",
        )),
        "named like the app, at the top of a drive" => Some((
            "как приложение, в корне диска: там могут лежать твои файлы",
            "named like the app, at the top of a drive: it may hold your own files",
        )),
        "install folder, but it is a folder of its own at the top of a drive and not named like the app" => Some((
            "папка установки, но это отдельная папка в корне диска с чужим именем",
            "install folder, but a folder of its own at the top of a drive, not named like the app",
        )),
        "install folder, but it holds several programs" => Some(("папка установки, но в ней несколько программ", "install folder, but it holds several programs")),
        "key shared with other versions of the product" => Some(("ключ общий с другими версиями продукта", "key shared with other versions of the product")),
        "named like the publisher" => Some(("называется как издатель", "named like the publisher")),
        "contains the app's full name" => Some(("в названии есть полное имя приложения", "contains the app\u{2019}s full name")),
        "publisher folder holding only this app" => Some(("папка издателя, в ней только это приложение", "publisher folder holding only this app")),
        "publisher folder, other apps may use it" => Some(("папка издателя: её могут использовать другие программы", "publisher folder, other apps may use it")),
        "emptied folder that held the app" => Some(("пустая папка, где лежало приложение", "emptied folder that held the app")),
        "installer file named like the app" => Some(("установщик с именем приложения", "installer file named like the app")),
        "Start Menu folder named like the app" => Some(("папка в меню «Пуск»", "Start Menu folder")),
        "shortcut named like the app" => Some(("ярлык с именем приложения", "shortcut named like the app")),
        "points into the install folder" => Some(("ведёт в папку установки", "points into the install folder")),
        "key under the publisher" => Some(("ключ издателя с именем приложения", "key under the publisher")),
        "publisher key holding only this app" => Some(("ключ издателя, в нём только это приложение", "publisher key holding only this app")),
        "uninstall entry of this product, its uninstaller is gone" => Some(("запись об удалении, деинсталлятора уже нет", "uninstall entry, its uninstaller is gone")),
        "App Paths entry in the install folder" => Some(("запись App Paths ведёт в папку установки", "App Paths entry in the install folder")),
        "COM class or handler in the install folder" => Some(("COM-класс или обработчик из папки установки", "COM class or handler in the install folder")),
        "carries the product GUID" => Some(("несёт GUID продукта", "carries the product GUID")),
        "starts the app at sign-in" => Some(("запускает приложение при входе", "starts the app at sign-in")),
        "install folder path in a history list" => Some(("путь к папке установки в списке истории", "install folder path in a history list")),
        "association with a removed handler" => Some(("ассоциация с удалённым обработчиком", "association with a removed handler")),
        "Windows Installer record of this product" => Some(("запись Windows Installer об этом продукте", "Windows Installer record of this product")),
        "service runs a file in the install folder" => Some(("служба запускает файл из папки установки", "service runs a file in the install folder")),
        "scheduled task starts a file in the install folder" => Some(("задача запускает файл из папки установки", "scheduled task starts a file in the install folder")),
        "task named like the app" => Some(("задача с именем приложения", "task named like the app")),
        "firewall rule for a program in the install folder" => Some(("правило брандмауэра для программы из папки установки", "firewall rule for a program in the install folder")),
        _ => None,
    };
    t.map(|t| t.tr(lang).to_string()).unwrap_or_else(|| note.to_string())
}

/// JS `toFixed(1)` rounds ties up; Rust's `{:.1}` rounds ties to even. This follows the JS.
fn fixed1(v: f64) -> String {
    format!("{:.1}", (v * 10.0 + 0.5).floor() / 10.0)
}

/// `size(bytes)`: GB with one decimal, MB / KB rounded, bytes as is.
pub fn size(b: u64) -> String {
    const KB: u64 = 1 << 10;
    const MB: u64 = 1 << 20;
    const GB: u64 = 1 << 30;
    if b >= GB {
        format!("{} GB", fixed1(b as f64 / GB as f64))
    } else if b >= MB {
        format!("{} MB", (b as f64 / MB as f64).round() as u64)
    } else if b >= KB {
        format!("{} KB", (b as f64 / KB as f64).round() as u64)
    } else if b > 0 {
        format!("{b} B")
    } else {
        "0 B".to_string()
    }
}

/// ` · 12 MB`, or nothing for 0 (the group / selection size text).
pub fn size_suffix(b: u64) -> String {
    if b == 0 {
        String::new()
    } else {
        format!(" \u{b7} {}", size(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_the_js_formatter() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(512), "512 B");
        assert_eq!(size(1024), "1 KB");
        assert_eq!(size(1536), "2 KB", "Math.round rounds .5 up");
        assert_eq!(size(5 << 20), "5 MB");
        assert_eq!(size(1 << 30), "1.0 GB");
        assert_eq!(size(1 << 28), "256 MB");
        assert_eq!(size((3u64 << 30) + (1 << 29)), "3.5 GB");
        // the tie case: 1.25 GB is an exact tie and JS toFixed(1) gives 1.3
        assert_eq!(size((1u64 << 30) + (1 << 28)), "1.3 GB");
        assert_eq!(size_suffix(0), "");
        assert_eq!(size_suffix(2048), " \u{b7} 2 KB");
    }

    #[test]
    fn lookups_localise_and_fall_back() {
        assert_eq!(kind_label("key", Lang::Ru), "Ключ реестра");
        assert_eq!(kind_label("weird", Lang::Ru), "weird");
        assert_eq!(stage("registry (user)", Lang::Ru), "реестр (пользователь)");
        assert_eq!(stage("something new", Lang::Ru), "something new");
        assert_eq!(note("carries the product GUID", Lang::Ru), "несёт GUID продукта");
        assert_eq!(note("contains the app's full name", Lang::En), "contains the app\u{2019}s full name");
        assert_eq!(note("unknown note", Lang::En), "unknown note");
        assert_eq!(note("Start Menu folder named like the app", Lang::En), "Start Menu folder");
    }
}
