//! Pages: Modules, Live events, Clipboard, File search, Sound, Weather.
use crate::settings::schema::*;
use crate::settings::tr::{same, tr, Tr};
use serde_json::{json, Value};

const MODS: [(&str, Tr, Tr); 11] = [
    ("media", tr("Медиаплеер", "Media player"), tr("Обложка, управление и эквалайзер", "Cover, controls and equaliser")),
    ("shelf", tr("Полка файлов", "File shelf"), tr("Перетащи файлы на остров, они полежат, пока не понадобятся", "Drop files on the pill, they wait until you need them")),
    ("launcher", tr("Лаунчер", "Launcher"), tr("Ярлыки программ на вкладке «Главная»", "Program shortcuts on the Home tab")),
    ("fileSearch", tr("Поиск файлов в командной строке", "File search in the command bar"), tr("Файлы и папки через Everything или поиск Windows", "Files and folders through Everything or Windows search")),
    ("clipboard", tr("История буфера", "Clipboard history"), tr("Вкладка «Буфер»", "The Clipboard tab")),
    ("text", tr("Текстовые инструменты", "Text tools"), tr("Вкладка «Текст»: регистр, раскладка, JSON, Base64", "The Text tab: case, layout, JSON, Base64")),
    ("chat", tr("Чат с ИИ", "AI chat"), tr("Вкладка «Чат»", "The Chat tab")),
    ("note", tr("Заметки и таймер", "Notes and timer"), tr("Вкладка «Заметка», напоминания", "The Note tab, reminders")),
    ("weather", tr("Погода", "Weather"), tr("Вкладка «Погода»", "The Weather tab")),
    ("controls", tr("Управление системой", "System controls"), tr("Вкладка «Управление»: сеть, Bluetooth, тема, питание", "The Controls tab: network, Bluetooth, theme, power")),
    ("notifs", tr("Центр уведомлений", "Notification centre"), tr("Вкладка «Уведомления»: история карточек острова", "The Notifications tab: history of the island cards")),
];

pub fn modules() -> Page {
    let mut p = page(
        "modules",
        "modules",
        tr("Модули", "Modules"),
        vec![
            group_note(
                tr("Профили", "Profiles"),
                tr(
                    "В профиль входят только переключатели модулей и живых событий. Сеть (ИИ, погода, почта, календарь), ключи, пути и клавиши профиль не меняет.",
                    "A profile holds only the module and live-event switches. It never changes network features (AI, weather, mail, calendar), keys, paths or hotkeys.",
                ),
                vec![custom("modules.profiles", tr("Профили", "Profiles"), tr("Названные состояния модулей и живых событий: сохранить текущее, применить, удалить (не больше 8)", "Named states of the modules and live events: save the current one, apply, delete (at most 8)"), "modules.profiles")
                    .kw(&["profile", "scene", "профиль", "сцена", "работа", "игра", "work", "game"])],
            ),
            group(
                tr("Модули", "Modules"),
                MODS.iter()
                    .map(|(id, t, h)| {
                        let key: &'static str = super::leak(format!("module.{id}"));
                        toggle(key, *t, *h, true).kw(&["module", "tab", "модуль", "вкладка"])
                    })
                    .collect(),
            ),
            group(
                tr("История уведомлений", "Notification history"),
                vec![
                    toggle(
                        "history.keep",
                        tr("Хранить историю", "Keep history"),
                        tr(
                            "Карточки острова запоминаются в центре уведомлений: 14 дней, до 200 записей, только на этом компьютере",
                            "Island cards are remembered in the notification centre: 14 days, up to 200 entries, on this computer only",
                        ),
                        true,
                    )
                    .kw(&["history", "notifications", "история", "уведомления"]),
                    toggle(
                        "history.mail",
                        tr("Хранить текст писем и уведомлений Windows", "Keep mail and Windows notification text"),
                        tr(
                            "Выключено: хранится только заголовок карточки (отправитель письма, заголовок уведомления Windows). Текст и тема не сохраняются",
                            "Off: only the card title is kept (mail sender, Windows notification title). Body and subject are not stored",
                        ),
                        false,
                    )
                    .enabled(Cond::On("history.keep")),
                ],
            ),
        ],
    );
    p.footnote = tr(
        "Выключенный модуль сразу прячет свою вкладку и перестаёт работать в фоне: история буфера не пишется, плеер не опрашивается, погода не запрашивается. Сами данные (история, заметки, полка, ярлыки) остаются. Напоминания и таймеры срабатывают, даже если вкладка «Заметка» выключена.",
        "A module that is off hides its tab at once and stops its background work: the clipboard history is not written, the player is not polled, the weather is not requested. The data itself (history, notes, shelf, shortcuts) stays. Reminders and timers still fire when the Note tab is off.",
    );
    p
}

const LIVE: [(&str, Tr, Tr, bool); 17] = [
    ("live.volume", tr("Громкость", "Volume"), tr("Полоска при изменении громкости, колесо мыши над островом", "A bar when the volume changes, the mouse wheel over the pill"), true),
    ("live.music", tr("Музыка", "Music"), tr("Название трека и эквалайзер в пилюле, пока играет музыка", "The track title and an equaliser in the pill while music plays"), true),
    ("live.micCam", tr("Микрофон и камера", "Microphone and camera"), tr("Точки в острове, пока приложение слушает или снимает", "Dots in the pill while an app listens or films"), true),
    ("live.downloads", tr("Загрузки", "Downloads"), tr("Прогресс и карточка с готовым файлом", "Progress and a card with the finished file"), true),
    ("live.recording", tr("Запись экрана", "Screen recording"), tr("Красная точка и таймер в пилюле, пока идёт запись", "A red dot and a timer in the pill while a recording runs"), true),
    ("live.notifications", tr("Уведомления", "Notifications"), tr("Карточки с событиями вместо всплывающих окон", "Event cards instead of pop-up windows"), true),
    ("live.reminders", tr("Напоминания и таймеры", "Reminders and timers"), tr("Карточка, когда напоминание или таймер сработали", "A card when a reminder or a timer goes off"), true),
    ("live.keyboardLayout", tr("Раскладка клавиатуры", "Keyboard layout"), tr("Короткая подсказка при смене языка ввода и Caps Lock", "A short hint when the input language or Caps Lock changes"), true),
    ("live.copy", tr("Реакция на копирование", "Reaction to copying"), tr("Остров слегка вздрагивает, когда в буфер попал новый текст", "The pill twitches slightly when new text reaches the clipboard"), true),
    ("live.obs", tr("OBS: запись и трансляция", "OBS: recording and streaming"), tr("Пока OBS пишет или транслирует, в пилюле горит REC или LIVE, готовая запись приходит карточкой. В OBS включите сервер WebSocket (Сервис → Настройки сервера WebSocket)", "While OBS records or streams, the pill shows REC or LIVE and the finished recording arrives as a card. In OBS, enable the WebSocket server (Tools → WebSocket Server Settings)"), false),
    ("live.agents", tr("ИИ-агенты в терминале", "AI agents in the terminal"), tr("Claude Code и Codex: в пилюле видно, что агент работает, ждёт ответа или закончил, а когда он ждёт или закончил — приходит карточка", "Claude Code and Codex: the pill shows that an agent works, waits for an answer or has finished, and a card arrives when it waits or finishes"), true),
    ("live.steam", tr("Загрузки Steam", "Steam downloads"), tr("Игры и обновления, которые качает Steam: сколько осталось и скорость, а в конце карточка", "Games and updates Steam downloads: time left and speed, and a card at the end"), true),
    ("live.sysmon", tr("Монитор системы", "System monitor"), tr("Загрузка процессора, памяти и видеокарт NVIDIA (с температурой) на вкладке «Управление». Предупреждение, если видеокарта от 85 °C или процессор от 95 % дольше 10 с", "CPU, memory and NVIDIA GPU load (with temperature) on the Controls tab. A warning when a GPU is at 85 °C or more, or the CPU is at 95 % for over 10 s"), true),
    ("live.dlDone", tr("Готовый файл в «Загрузках»", "Finished file in Downloads"), tr("Карточка «Загружено: имя файла», когда файл докачался в папку «Загрузки»; клик по карточке открывает папку с выделенным файлом", "A “Downloaded: file name” card when a file finishes in your Downloads folder; a click on the card opens the folder with the file selected"), true),
    ("live.shelf", tr("Полка файлов", "File shelf"), tr("Значок с числом файлов на свёрнутом острове, пока полка не пуста, и счётчик на 5 секунд, когда файлы добавили или убрали", "A file-count badge on the folded pill while the shelf is not empty, and a 5-second counter when files are added or removed"), true),
    ("live.usb", tr("Флешки и съёмные диски", "USB and removable drives"), tr("Карточка, когда съёмный диск подключили или отключили", "A card when a removable drive is connected or removed"), true),
    ("live.battery", tr("Батарея", "Battery"), tr("Предупреждение при заряде 20 % и 10 % на ноутбуке, а также «Заряжается» и «Заряжено»; на компьютере без батареи ничего не делает", "A warning at 20 % and 10 % on a laptop, plus “Charging” and “Fully charged”; does nothing on a PC without a battery"), true),
];

// live.calendar and live.mail follow in the page builder (they carry extra fields); both are off by default (network opt-in).
pub fn live() -> Page {
    let mut rows: Vec<Row> = Vec::new();
    for (key, t, h, def) in LIVE.iter() {
        let r = toggle(key, *t, *h, *def).kw(&["live", "event", "событие", "карточка", "card"]);
        let is_obs = *key == "live.obs";
        let is_steam = *key == "live.steam";
        let is_notif = *key == "live.notifications";
        rows.push(if is_obs { r.status("obs") } else if is_steam { r.status("steam") } else if is_notif { r.status("notifications") } else { r });
        if is_obs {
            rows.push(
                Row::new("live.obs.port", "live.obs.port", tr("Порт сервера WebSocket в OBS", "WebSocket server port in OBS"), Control::Number { min: 1, max: 65535 }, json!(4455))
                    .hint(tr("По умолчанию 4455", "Default 4455"))
                    .enabled(Cond::On("live.obs")),
            );
            rows.push(
                secret(
                    "live.obs.password",
                    tr("Пароль сервера WebSocket", "WebSocket server password"),
                    tr(
                        "Только этот компьютер (127.0.0.1). Пароль хранится зашифрованным (DPAPI) и никуда не отправляется, кроме самого OBS. Пустое поле и «Сохранить» стирают пароль.",
                        "This computer only (127.0.0.1). The password is stored encrypted (DPAPI) and goes nowhere but to OBS itself. An empty field and \"Save\" erase it.",
                    ),
                    tr("Пароль сервера WebSocket (если задан в OBS)", "WebSocket server password (if set in OBS)"),
                    Action::ObsSetPassword,
                )
                .enabled(Cond::On("live.obs")),
            );
        }
        if *key == "live.notifications" {
            rows.push(button("live.notifications.allow", tr("Доступ к уведомлениям Windows", "Windows notification access"), tr("Pill не убирает исходное уведомление: Windows может показать и своё всплывающее окно, если не включён режим фокусировки. Доступ разрешается один раз: Параметры → Конфиденциальность → Уведомления.", "Pill does not remove the original notification: Windows may still show its own popup unless Focus Assist is on. Access is granted once: Settings → Privacy → Notifications."), tr("Разрешить доступ", "Allow access"), Action::NotifAllow, ButtonStyle::Small).enabled(Cond::On("live.notifications")));
        }
        if *key == "live.agents" {
            rows.push(button("live.agents.snippets", tr("Фрагменты для настроек агентов", "Agent settings snippets"), tr("Pill сам не меняет настройки агентов. «Показать» выведет фрагменты: хуки для ~/.claude/settings.json (Claude Code) и notify для ~/.codex/config.toml (Codex). Для новых сессий.", "Pill never edits agent settings itself. “Show” prints the snippets: hooks for ~/.claude/settings.json (Claude Code) and notify for ~/.codex/config.toml (Codex). For new sessions."), tr("Показать", "Show"), Action::AgentsSnippets, ButtonStyle::Small));
        }
    }
    // the calendar and the mailbox are the two network opt-ins of this page (off by default)
    let (cal_t, cal_h) = (tr("Ближайшее событие календаря", "Next calendar event"), tr("За 10 минут до события из файла .ics или ссылки-подписки (https:// / webcal://). Файл только читается, ссылка скачивается раз в 30 минут, без аккаунтов", "Ten minutes before an event from an .ics file or a subscription link (https:// / webcal://). A file is only read, a link is fetched every 30 minutes, no accounts"));
    rows.push(toggle("live.calendar", cal_t, cal_h, false).status("calendar").kw(&["calendar", "ics", "календарь", "webcal"]));
    rows.push(
        Row::new(
            "live.calendar.path",
            "live.calendar.path",
            tr("Файл или ссылка календаря", "Calendar file or link"),
            Control::Text { secret: false, kind: TextKind::Path, placeholder: tr("Путь к файлу .ics (C:\\Users\\Имя\\calendar.ics) или ссылка https:// / webcal://", "Path to an .ics file (C:\\Users\\Name\\calendar.ics) or an https:// / webcal:// link"), mono: false, commit: Commit::Settings },
            json!(""),
        )
        .hint(tr(
            "Файл с диска только читается. Ссылка-подписка (https:// или webcal://, например «секретный адрес в формате iCal» Google Календаря) скачивается раз в 30 минут, не больше 2 МБ; последняя копия лежит рядом с настройками, так что работает и без интернета. Адрес нигде не записывается, в журнал попадает только имя сервера. Понимаются время UTC и местное, события на весь день (без напоминания) и повторы DAILY / WEEKLY / MONTHLY / YEARLY; часовой пояс TZID не учитывается, время считается местным. Чтобы вставить путь, в Проводнике: Shift + правая кнопка → «Копировать как путь».",
            "A file on disk is only read. A subscription link (https:// or webcal://, e.g. the “secret address in iCal format” of Google Calendar) is downloaded every 30 minutes, 2 MB at most; the last copy is kept next to the settings, so it also works offline. The address is never logged, only the server name. UTC and local times, all-day events (no reminder) and DAILY / WEEKLY / MONTHLY / YEARLY repeats are understood; a TZID zone is ignored and the time is taken as local. To paste a path, in Explorer: Shift + right click → “Copy as path”.",
        ))
        .enabled(Cond::On("live.calendar"))
        .block(),
    );
    let (mail_t, mail_h) = (tr("Новые письма (IMAP)", "New mail (IMAP)"), tr("Карточка «Новое письмо» с отправителем и темой. Читаются только заголовки: письма не открываются и не помечаются прочитанными. Только TLS, раз в 2 минуты", "A “New mail” card with sender and subject. Only headers are read: messages are not opened or marked as read. TLS only, every 2 minutes"));
    rows.push(toggle("live.mail", mail_t, mail_h, false).status("mail").kw(&["mail", "imap", "email", "почта", "письма", "gmail"]));
    let on = Cond::On("live.mail");
    rows.push(text("mail.host", tr("Сервер IMAP", "IMAP server"), Tr::NONE, TextKind::Host, tr("Сервер IMAP, например imap.gmail.com", "IMAP server, e.g. imap.gmail.com"), "").enabled(on.clone()));
    rows.push(
        Row::new("mail.port", "mail.port", tr("Порт IMAP", "IMAP port"), Control::Number { min: 1, max: 65535 }, json!(993))
            .hint(tr("Порт IMAP с шифрованием TLS (обычно 993). Незашифрованный порт 143 не используется", "IMAP port with TLS encryption (usually 993). The unencrypted port 143 is not used"))
            .enabled(on.clone()),
    );
    rows.push(text("mail.user", tr("Логин", "Login"), tr("Обычно адрес почты", "Usually the e-mail address"), TextKind::Plain, tr("Логин (обычно адрес почты)", "Login (usually the e-mail address)"), "").enabled(on.clone()));
    rows.push(text("mail.mailbox", tr("Папка", "Folder"), tr("Папка, за которой следить (по умолчанию INBOX)", "Folder to watch (INBOX by default)"), TextKind::Plain, same("INBOX"), "INBOX").enabled(on.clone()));
    rows.push(
        secret(
            "mail.password",
            tr("Пароль приложения", "App password"),
            tr(
                "Gmail, Яндекс и Mail.ru не принимают обычный пароль: создайте «пароль приложения» в настройках безопасности аккаунта и включите доступ по IMAP. Pill подключается только по TLS (порт 993), раз в 2 минуты смотрит заголовки новых писем (отправитель и тема), не читает тексты и вложения и не помечает письма прочитанными. Пароль хранится зашифрованным (Windows DPAPI) и после сохранения больше нигде не показывается. Пустое поле и «Сохранить пароль» стирают пароль.",
                "Gmail, Yandex and Mail.ru do not accept the normal password: create an “app password” in the account security settings and enable IMAP access. Pill connects over TLS only (port 993), looks at the headers of new messages every 2 minutes (sender and subject), never reads bodies or attachments and never marks mail as read. The password is stored encrypted (Windows DPAPI) and is never shown again after saving. An empty field and “Save password” erase it.",
            ),
            tr("Пароль приложения", "App password"),
            Action::MailSetPassword,
        )
        .enabled(on.clone()),
    );
    rows.push(button("live.mail.test", tr("Проверить подключение", "Test connection"), Tr::NONE, tr("Проверить подключение", "Test connection"), Action::MailTest, ButtonStyle::Small).enabled(on));
    let mut p = page("live", "live", tr("Живые события", "Live events"), vec![group(Tr::NONE, rows)]);
    p.footnote = tr(
        "Что читается. Громкость: только уровень и «без звука» устройства вывода по умолчанию. Микрофон и камера: в реестре Windows (только чтение, ветка вашего пользователя) отметки о том, какое приложение сейчас пользуется устройством; звук и изображение не открываются и не читаются. Загрузки: только имена и размеры файлов во временных браузерных файлах (.crdownload, .part, .tmp) в папке «Загрузки»; содержимое файлов не читается. Готовый файл: только имена и размеры файлов в «Загрузках». Съёмные диски: список букв дисков. Батарея: уровень заряда и подключено ли питание. Календарь: только файл .ics, который вы указали. Пока переключатель выключен, ничего из этого не опрашивается.",
        "What is read. Volume: only the level and the mute flag of the default output device. Microphone and camera: marks in the Windows registry (read-only, your user branch) saying which app is using the device right now; sound and picture are never opened or read. Downloads: only the names and sizes of browser temp files (.crdownload, .part, .tmp) in your Downloads folder; file contents are not read. Finished file: only the names and sizes of files in Downloads. Removable drives: the list of drive letters. Battery: the charge level and whether the power is plugged in. Calendar: only the .ics file you point to. While a switch is off, none of this is polled.",
    );
    p
}

pub fn clipboard() -> Page {
    let deny: Value = json!(["keepass", "keepassxc", "1password", "bitwarden", "lastpass", "dashlane", "enpass", "mstsc"]);
    let mut p = page(
        "clipboard",
        "clipboard",
        tr("Буфер обмена", "Clipboard"),
        vec![
            group(
                Tr::NONE,
                vec![
                    toggle("clipboard.clickPaste", tr("Клик по записи вставляет её", "A click on an entry pastes it"), tr("Иначе запись только копируется. Shift+клик всегда только копирует", "Otherwise the entry is only copied. Shift+click always only copies"), true).kw(&["clipboard", "paste", "буфер", "вставить"]),
                    seg("clipboard.maxItems", tr("Размер истории", "History size"), tr("Закреплённые записи не считаются", "Pinned entries do not count"), no(&[(50, same("50")), (200, same("200")), (500, same("500")), (1000, same("1000"))]), json!(1000)).kw(&["history", "size", "история", "размер"]),
                    seg(
                        "clipboard.retentionDays",
                        tr("Хранить записи", "Keep entries"),
                        tr("Старше этого срока записи стираются, закреплённые остаются", "Older entries are erased, pinned ones stay"),
                        no(&[(7, tr("7 дн.", "7 d")), (30, tr("30 дн.", "30 d")), (90, tr("90 дн.", "90 d")), (365, tr("год", "1 y"))]),
                        json!(30),
                    )
                    .kw(&["retention", "days", "срок", "хранить"]),
                    select("clipboard.maxImageMb", tr("Самая большая картинка", "Largest picture"), tr("Картинки крупнее этого размера в историю не попадают", "Pictures larger than this do not enter the history"), no(&[(64, same("64 MB")), (256, same("256 MB")), (512, same("512 MB")), (1024, same("1024 MB"))]), json!(512)).kw(&["image", "picture", "картинка"]),
                    Row::new("clipboard.private", "", tr("Приватный режим", "Private mode"), Control::BackendToggle(BackendToggle::ClipboardPrivate), Value::Null)
                        .hint(tr("История не записывается, пока включено", "Nothing is recorded while it is on"))
                        .kw(&["private", "pause", "приватный", "пауза"]),
                    toggle("clipboard.skipSecrets", tr("Не записывать пароли и ключи", "Skip passwords and keys"), tr("Эвристика: длинные случайные строки, номера карт, токены не попадают в историю", "A heuristic: long random strings, card numbers and tokens do not enter the history"), true).kw(&["password", "secret", "token", "пароль", "ключ"]),
                    Row::new(
                        "clipboard.denyList",
                        "clipboard.denyList",
                        tr("Не запоминать копирование из программ", "Do not remember copying from programs"),
                        Control::Lines { rows: 5, mono: true, placeholder: Tr::NONE, exe_suffix: true },
                        deny,
                    )
                    .hint(tr("По одной на строку, имя процесса. Менеджеры паролей, которые помечают копирование, и так пропускаются", "One per line, the process name. Password managers that mark their copies are skipped anyway"))
                    .kw(&["deny", "exclude", "password manager", "исключения", "менеджер паролей"])
                    .block(),
                    button("clipboard.wipe", tr("Стереть историю", "Wipe history"), tr("Удаляет все записи и картинки, закреплённые тоже", "Deletes every entry and picture, pinned ones too"), tr("Стереть…", "Wipe…"), Action::ClipWipe, ButtonStyle::Danger).kw(&["wipe", "clear", "delete", "стереть", "очистить", "удалить"]),
                ],
            ),
            group_note(
                tr("Скриншоты и запись экрана", "Screenshots and screen recording"),
                tr(
                    "Запись идёт через ffmpeg: его нужно поставить самому (Pill ничего не скачивает). Если он лежит в нестандартном месте, путь к ffmpeg.exe можно указать в settings.json, ключ capture.ffmpeg.",
                    "Recording goes through ffmpeg: install it yourself (Pill downloads nothing). If it lives in an unusual place, set the path to ffmpeg.exe in settings.json, key capture.ffmpeg.",
                ),
                vec![
                    seg(
                        "capture.format",
                        tr("Формат записи экрана", "Screen recording format"),
                        tr("MP4 — для длинных записей, GIF — для коротких роликов в чат (до минуты, 15 кадров/с; после остановки GIF собирается несколько секунд)", "MP4 for long recordings, GIF for short chat clips (up to a minute, 15 fps; a GIF takes a few seconds to build after you stop)"),
                        so(&[("mp4", same("MP4")), ("gif", same("GIF"))]),
                        json!("mp4"),
                    )
                    .kw(&["recording", "video", "gif", "mp4", "запись", "видео"]),
                    seg("capture.fps", tr("Частота кадров записи", "Recording frame rate"), tr("Для MP4. 60 кадров плавнее, но файл больше и компьютеру тяжелее", "For MP4. 60 frames is smoother, but the file is bigger and the computer works harder"), no(&[(30, same("30")), (60, same("60"))]), json!(30))
                        .visible(Cond::Eq("capture.format", json!("mp4")))
                        .kw(&["fps", "кадры"]),
                    seg("capture.encoder", tr("Кодировщик видео", "Video encoder"), tr("Авто берёт видеокарту NVIDIA (NVENC), если она работает, иначе процессор (x264)", "Auto uses the NVIDIA card (NVENC) when it works, otherwise the processor (x264)"), so(&[("auto", tr("Авто", "Auto")), ("nvenc", same("NVENC")), ("x264", same("x264"))]), json!("auto"))
                        .visible(Cond::Eq("capture.format", json!("mp4")))
                        .kw(&["encoder", "nvenc", "x264", "кодировщик"]),
                    seg(
                        "capture.shotAction",
                        tr("Скриншот по Enter и целого экрана", "Screenshot on Enter and of the whole screen"),
                        tr("Что делают Enter, двойной щелчок в выделении и скриншот всего экрана. Кнопки «Копировать» и «Сохранить» в панели работают как подписано", "What Enter, a double click in the selection and the whole-screen shot do. The Copy and Save buttons in the panel do what they say"),
                        so(&[("both", tr("Копия и файл", "Copy and file")), ("copy", tr("Только копия", "Copy only")), ("save", tr("Только файл", "File only"))]),
                        json!("both"),
                    )
                    .kw(&["screenshot", "скриншот"]),
                    toggle("capture.revealAfterSave", tr("Показать файл в папке после сохранения", "Show the file in its folder after saving"), tr("Открывает Проводник с выделенным скриншотом", "Opens Explorer with the screenshot selected"), false),
                    text("capture.folder", tr("Папка скриншотов", "Screenshot folder"), tr("Пусто = Изображения\\Screenshots\\Pill", "Empty = Pictures\\Screenshots\\Pill"), TextKind::Path, same("C:\\…"), "").block(),
                    text("capture.videoFolder", tr("Папка видео", "Video folder"), tr("Пусто = Видео\\Pill", "Empty = Videos\\Pill"), TextKind::Path, same("C:\\…"), "").block(),
                ],
            ),
        ],
    );
    p.footnote = Tr::NONE;
    p
}

pub fn search() -> Page {
    page(
        "search",
        "search",
        tr("Поиск файлов", "File search"),
        vec![
            group_note(
                tr("Everything: поиск файлов", "Everything: file search"),
                tr(
                    "Источник: только www.voidtools.com по https; размер и SHA-256 сверяются с записанными в Pill, при несовпадении файл удаляется. Pill сам ничего не устанавливает и не повышает права: установщик запускается вашим вторым нажатием.",
                    "Source: www.voidtools.com over https only; the size and SHA-256 are checked against the values pinned in Pill and a mismatch deletes the file. Pill installs nothing itself and never elevates: the installer starts on your second click.",
                ),
                vec![custom("search.everything", tr("Everything и es.exe", "Everything and es.exe"), Tr::NONE, "search.everything").kw(&["everything", "voidtools", "es.exe", "поиск", "файлы", "download", "скачать", "install"])],
            ),
            group(
                tr("Запуск", "Start"),
                vec![toggle("everything.autostart", tr("Запускать Everything сам", "Start Everything by itself"), tr("Если поиск файлов не отвечает, при первом поиске за сеанс запускается Everything (скрыто, в трее)", "If file search does not answer, the first search of a session starts Everything (hidden in the tray)"), true).kw(&["everything", "autostart"])],
            ),
        ],
    )
}

pub fn sound() -> Page {
    let same_win = || vec![opt(json!(""), tr("Как в Windows", "Same as Windows"))];
    let mut out = select("sound.volumeDevice", tr("Чью громкость регулирует остров", "Whose volume the pill controls"), tr("Звук не переключается: выбирается только устройство, чью громкость показывают ползунок, колесо и карточка. Выход звука выбирают на вкладке «Главная»", "The sound is not switched: this only picks the device whose volume the slider, wheel and card show. Choose the sound output on the Home tab"), same_win(), json!(""));
    if let Control::Select { dynamic, .. } = &mut out.control {
        *dynamic = Dynamic::OutputDevices;
    }
    let mut mic = select("sound.micDevice", tr("Какой микрофон выключает и запоминает остров", "Which microphone the pill mutes and remembers"), tr("Его выключает клавиша «Микрофон» и переключатель в центре управления. Системный микрофон по умолчанию Pill не переключает", "The “Microphone” key and the control-centre switch mute it. Pill does not change the Windows default microphone"), same_win(), json!(""));
    if let Control::Select { dynamic, .. } = &mut mic.control {
        *dynamic = Dynamic::CaptureDevices;
    }
    page(
        "sound",
        "sound",
        tr("Звук", "Sound"),
        vec![
            group(
                Tr::NONE,
                vec![
                    out.kw(&["volume", "device", "громкость", "устройство", "output"]),
                    mic.kw(&["microphone", "mic", "микрофон"]),
                    toggle("audio.reactive", tr("Жидкий край по частотам", "Reactive edge"), tr("Край острова двигается под музыку: слева низкие частоты, справа высокие", "The pill edge moves with the music: low frequencies on the left, high on the right"), true).kw(&["audio", "reactive", "equalizer", "эквалайзер", "частоты"]),
                    seg(
                        "musicPill",
                        tr("Пилюля во время музыки", "Pill during music"),
                        tr("«С названием» — трек и полоса времени. «Мини-плеер» — обложка и кнопки, открывается щелчком. «С текстом» — звучащая строка песни", "“Title”: the track and a time bar. “Mini player”: cover and buttons, opens with a click. “Lyrics”: the line being sung"),
                        so(&[("title", tr("С названием", "Title")), ("mini", tr("Мини-плеер", "Mini player")), ("lyrics", tr("С текстом", "Lyrics"))]),
                        json!("title"),
                    )
                    .kw(&["music", "player", "lyrics", "музыка", "плеер", "текст песни"])
                    .block(),
                ],
            ),
            group(
                tr("Напоминания", "Reminders"),
                vec![
                    toggle("sound.reminders", tr("Звук напоминаний", "Sound for reminders"), tr("Короткий сигнал, когда срабатывает напоминание или таймер", "A short signal when a reminder or a timer goes off"), true).kw(&["reminder", "timer", "напоминание", "таймер"]),
                    seg("sound.reminderTone", tr("Сигнал", "Signal"), Tr::NONE, so(&[("chime", tr("Мягкий", "Soft")), ("bell", tr("Звонок", "Bell")), ("beep", tr("Сигнал", "Beep"))]), json!("chime")).enabled(Cond::On("sound.reminders")).kw(&["tone", "chime", "bell", "beep", "мелодия"]),
                    button("sound.play", tr("Проверить", "Play"), Tr::NONE, tr("Проверить", "Play"), Action::PlaySignal, ButtonStyle::Small).enabled(Cond::On("sound.reminders")),
                ],
            ),
        ],
    )
}

pub fn weather() -> Page {
    page(
        "weather",
        "weather",
        tr("Погода", "Weather"),
        vec![group(
            Tr::NONE,
            vec![
                toggle("net.weather", tr("Погода", "Weather"), tr("Выключена по умолчанию. Включённая, она обращается только к api.met.no и nominatim.openstreetmap.org", "Off by default. When on, it talks only to api.met.no and nominatim.openstreetmap.org"), false).kw(&["weather", "погода", "forecast", "прогноз", "network", "сеть"]),
                Row::new("weather.city", "weather.city", tr("Город", "City"), Control::Custom("weather.city"), json!(""))
                    .hint(tr("Прогноз — MET Norway, поиск городов — OpenStreetMap. Ключ и регистрация не нужны. Город хранится только на этом компьютере.", "Forecast: MET Norway; city search: OpenStreetMap. No key or registration. The city is stored only on this computer."))
                    .enabled(Cond::On("net.weather"))
                    .kw(&["city", "город", "казань", "berlin", "найти", "find"]),
                seg("weather.units", tr("Единицы", "Units"), Tr::NONE, so(&[("metric", tr("°C · м/с", "°C · m/s")), ("imperial", same("°F · mph"))]), json!("metric")).kw(&["units", "celsius", "fahrenheit", "единицы", "градусы"]),
                button("weather.refresh", tr("Обновить прогноз", "Refresh the forecast"), tr("Обычно прогноз обновляется сам, не чаще раза в полчаса", "The forecast refreshes by itself, at most once in half an hour"), tr("Обновить сейчас", "Refresh now"), Action::WeatherRefresh, ButtonStyle::Normal).enabled(Cond::On("net.weather")),
            ],
        )],
    )
}
