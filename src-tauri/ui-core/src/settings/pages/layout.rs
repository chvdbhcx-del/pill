//! The layout of the Settings window: which page, which section (group) and in which order every row stands.
//!
//! The rows themselves (label, hint, control, default, key) are defined once in `base.rs`, `system.rs`, `phone.rs` and `ai.rs` as the old
//! 16 pages. This file only takes them out of a pool by id and arranges them into the ten pages of the sidebar.
//!
//! The arrangement is BY FEATURE: every feature (Weather, Media player, Clipboard history, Notes and timer, Phone, Calendar, Downloads,
//! AI agents, Launcher, File shelf, Screen recording, Voice to text, Virtual desktops ...) is ONE section whose title is its name. In it
//! stand, in this order, its on/off switch(es), its events, its parameters. The rare and technical rows of a feature go into a trailing
//! `.adv()` section named "<feature>: <what>", which the window folds behind the one collapsed "Advanced" disclosure of the page. Each
//! feature stands on the page where it fits best (the pages are only themes). A row cannot be lost or doubled: `Pool::take` removes it,
//! `Pool::finish` checks that nothing is left behind.
//!
//! Three groups of switches overlap on purpose and are NOT merged, only placed side by side and labelled apart (`Pool::edit`): Weather
//! (`net.weather` asks the internet, `module.weather` is the tab), Phone (`phone.enabled` is the link itself, `module.phone` the tab,
//! `live.phone` the cards) and File shelf (`module.shelf` is the shelf, `live.shelf` the badge on the island).
//!
//! Where a page fetches data when it shows up (`settings_view/nav.rs::page_shown`) and which page polls statuses (Events) decides some
//! placements: rows with a status line stay on Events, the phone widgets on Phone, Everything and the autostart switch on General.
//!
//! Moved rows keep their old page name in their search keywords ("behavior", "Живые события", ...) and now also the name of their
//! section ("Weather", "Погода", ...), so the old names and the feature names both find them.
use crate::settings::schema::*;
use crate::settings::tr::{tr, Tr};
use std::collections::HashMap;

/// The rows of the old pages, each remembering the page it came from.
pub(super) struct Pool {
    rows: Vec<Option<Row>>,
    at: HashMap<&'static str, usize>,
    old: Vec<Page>,
}

impl Pool {
    pub(super) fn new(old: Vec<Page>) -> Pool {
        let mut rows = Vec::new();
        let mut at = HashMap::new();
        for p in &old {
            for r in p.rows() {
                // the old page's name (id and both titles) joins the keywords: "behavior" still finds the rows of the old Behavior page
                let mut kw: Vec<&'static str> = r.search_keywords.to_vec();
                kw.extend([p.id, p.title.ru, p.title.en]);
                let mut r = r.clone();
                r.search_keywords = Box::leak(kw.into_boxed_slice());
                assert!(at.insert(r.id, rows.len()).is_none(), "row {} defined twice", r.id);
                rows.push(Some(r));
            }
        }
        Pool { rows, at, old }
    }

    /// The rows `ids`, in that order; each can be taken once.
    pub(super) fn take(&mut self, ids: &[&str]) -> Vec<Row> {
        ids.iter()
            .map(|id| {
                let i = *self.at.get(id).unwrap_or_else(|| panic!("no row {id}"));
                self.rows[i].take().unwrap_or_else(|| panic!("row {id} placed twice"))
            })
            .collect()
    }

    /// The rows `ids`; the name of their section joins their search keywords (searching "weather" finds every weather row).
    fn feature(&mut self, title: Tr, ids: &[&str]) -> Vec<Row> {
        let mut rows = self.take(ids);
        if !title.is_blank() {
            for r in &mut rows {
                let mut kw: Vec<&'static str> = r.search_keywords.to_vec();
                kw.extend([title.ru, title.en]);
                r.search_keywords = Box::leak(kw.into_boxed_slice());
            }
        }
        rows
    }

    pub(super) fn sec(&mut self, title: Tr, ids: &[&str]) -> Group {
        group(title, self.feature(title, ids))
    }

    pub(super) fn note(&mut self, title: Tr, footer: Tr, ids: &[&str]) -> Group {
        group_note(title, footer, self.feature(title, ids))
    }

    /// New words for a row whose neighbours make its old ones ambiguous (the overlapping switches). Nothing else about the row changes;
    /// `tests.rs` lists these ids and holds every other row to its old label and hint.
    pub(super) fn edit(&mut self, id: &str, label: Option<Tr>, hint: Option<Tr>) {
        let i = *self.at.get(id).unwrap_or_else(|| panic!("no row {id}"));
        let r = self.rows[i].as_mut().unwrap_or_else(|| panic!("row {id} is already placed"));
        if let Some(l) = label {
            r.label = l;
        }
        if let Some(h) = hint {
            r.hint = h;
        }
    }

    /// Rows that are deliberately gone (see the commit message and the report); they must still exist so a typo cannot hide behind this.
    pub(super) fn drop_rows(&mut self, ids: &[&str]) {
        self.take(ids);
    }

    /// The footer of group `n` of the old page `page`.
    fn footer(&self, page: &str, n: usize) -> Tr {
        self.old.iter().find(|p| p.id == page).and_then(|p| p.groups.get(n)).map(|g| g.footer).unwrap_or(Tr::NONE)
    }

    fn footnote(&self, page: &str) -> Tr {
        self.old.iter().find(|p| p.id == page).map(|p| p.footnote).unwrap_or(Tr::NONE)
    }

    pub(super) fn finish(self) {
        let left: Vec<&str> = self.rows.iter().flatten().map(|r| r.id).collect();
        assert!(left.is_empty(), "rows left out of the layout: {left:?}");
    }
}

pub(super) fn appearance(p: &mut Pool) -> Page {
    // the reaction note without its first sentence: the row "Island" already says that the edge style applies at once
    let reactions = tr(
        "Реакции включаются сами: агенты, музыка, файлы, напоминания; через 5 минут тишины остров засыпает.",
        "Reactions fire on their own: agents, music, files, reminders; after 5 quiet minutes the island falls asleep.",
    );
    page(
        "appearance",
        "appearance",
        tr("Внешний вид", "Appearance"),
        vec![
            p.sec(tr("Вид острова", "Island look"), &["shape", "theme", "island.color", "size", "monitor"]),
            p.note(tr("Край острова и персонаж", "Island edge and character"), reactions, &["character", "character.liquid", "character.reactions"]),
            p.sec(tr("Питомец", "Pet"), &["pet", "pet.side", "pet.size", "pet.keepDuringMusic"]),
            p.sec(tr("Часы и дата", "Clock and date"), &["showTime", "showDate", "showSeconds", "clock24"]),
            p.sec(tr("Анимация", "Animation"), &["reduceMotion"]),
            p.sec(tr("Вид острова: цвет, прозрачность, положение", "Island look: colour, transparency, position"), &["island.colorCustom", "opacityCol", "opacityExp", "offsetX", "underTaskbar"]).adv(),
            p.sec(tr("Анимация: частота кадров и видеокарта", "Animation: frame rate and GPU"), &["fps", "edgeIdleFps", "ui.gpu"]).adv(),
        ],
    )
}

pub(super) fn general(p: &mut Pool) -> Page {
    let browser = p.footer("search", 2);
    let everything = p.footer("search", 0);
    page(
        "general",
        "general",
        tr("Основные", "General"),
        vec![
            p.sec(tr("Запуск и язык", "Start and language"), &["language", "general.autostart", "settings.preview"]),
            p.sec(tr("Прятать остров", "Hiding the pill"), &["hideInFullscreen", "fullscreenPeek", "autoHide"]),
            p.sec(tr("Тихие часы", "Quiet hours"), &["quiet.on", "quiet.from", "quiet.to", "quiet.days"]),
            p.note(tr("Поиск в командной строке", "Command bar search"), browser, &["module.fileSearch", "search.engine", "search.customUrl"]),
            p.sec(tr("Прятать остров: подробности и списки программ", "Hiding the pill: details and program lists"), &["fullscreenPeekEdge", "autoHideKeepDesktop", "hideInApps", "behavior.hideAddActive", "showInApps", "behavior.showAddActive"]).adv(),
            p.sec(tr("Сквозной режим", "Pass-through mode"), &["passThrough", "passThroughOpacity"]).adv(),
            p.note(tr("Поиск файлов: Everything", "File search: Everything"), everything, &["search.everything", "everything.autostart"]).adv(),
            p.sec(tr("Копия настроек", "Settings backup"), &["general.backup"]).adv(),
            p.sec(tr("Первый запуск", "First run"), &["general.wizard"]).adv(),
        ],
    )
}

pub(super) fn hotkeys(p: &mut Pool) -> Page {
    // Hotkeys stay on one page on purpose: the recorder checks a combo against all the others, and a list of every key is what people look for
    let mut pg = page(
        "hotkeys",
        "hotkeys",
        tr("Клавиши", "Hotkeys"),
        vec![
            p.sec(tr("Остров, поиск, чат, диктовка и буфер", "Island, command bar, chat, dictation and clipboard"), &["hotkey.toggle", "hotkey.command", "hotkey.chat", "hotkey.voice", "hotkey.clipboard", "hotkey.pastePlain", "hotkey.mic"]),
            p.sec(tr("Экран: запись, снимки, текст и цвет", "Screen: recording, shots, text and colour"), &["hotkey.record", "hotkey.shotArea", "hotkey.shotFull", "hotkey.ocr", "hotkey.pipette", "hotkey.qr"]),
            p.sec(tr("Ярлыки лаунчера", "Launcher shortcuts"), &["hotkeys.launcher"]),
            p.sec(tr("Пока без сочетания", "Not assigned yet"), &["hotkey.uninstall", "hotkey.passThrough", "hotkey.theme", "hotkey.musicView", "hotkey.vpn"]).adv(),
        ],
    );
    pg.head = tr("Горячие клавиши", "Hotkeys");
    pg
}

pub(super) fn modules(p: &mut Pool) -> Page {
    let vpn = p.footer("modules", 2);
    let profiles = p.footer("modules", 0);
    let tabs = tr(
        "Скрытая вкладка исчезает из полосы вкладок, но по-прежнему открывается своей горячей клавишей и командой. Это только про полосу: чтобы функция перестала работать, выключи её модуль. Этот переключатель стоит в разделе самой функции (Медиаплеер: «Звук и голос», История буфера: «Буфер и скриншоты», Чат с ИИ: «ИИ», Телефон: «Телефон», Центр уведомлений: «События», Поиск файлов: «Основные»; остальные на этой странице).",
        "A hidden tab leaves the tab strip but still opens with its hotkey and from the command bar. This only concerns the strip: to stop a feature from working, switch its module off. That switch is in the feature's own section (Media player: Sound and voice, Clipboard history: Clipboard and screenshots, AI chat: AI, Phone: Phone, Notification centre: Events, File search: General; the rest are on this page).",
    );
    let home = tr(
        "«Плеер» и «Ярлыки» это те же переключатели, что «Медиаплеер» (страница «Звук и голос») и «Лаунчер» (на этой странице). «Недавно скопировано» показывает последние записи истории буфера прямо на главной (нужна «История буфера»): текст виден всем, кто смотрит на экран, поэтому по умолчанию выключено. Ничего не уходит с компьютера.",
        "“Player” and “Shortcuts” are the same switches as “Media player” (the Sound and voice page) and “Launcher” (on this page). “Recently copied” shows the latest clipboard history entries right on Home (needs “Clipboard history”): anyone looking at the screen can read that text, so it is off by default. Nothing leaves the computer.",
    );
    // Weather: the forecast switch asks the internet (off by default), the module switch is the tab. Two switches, told apart by their words.
    p.edit(
        "net.weather",
        Some(tr("Прогноз из интернета", "Forecast from the internet")),
        Some(tr(
            "Выключено по умолчанию. Включённое, оно обращается только к api.met.no и nominatim.openstreetmap.org. Без него вкладке «Погода» и значку у часов нечего показывать",
            "Off by default. When on, it talks only to api.met.no and nominatim.openstreetmap.org. Without it the Weather tab and the chip by the clock have nothing to show",
        )),
    );
    p.edit(
        "module.weather",
        Some(tr("Вкладка «Погода»", "Weather tab")),
        Some(tr(
            "Показывает вкладку. Выключено: вкладка скрыта и погода не запрашивается, даже если прогноз включён выше",
            "Shows the tab. Off: the tab is hidden and the weather is not requested, even if the forecast above is on",
        )),
    );
    // File shelf: the shelf itself, and the badge it puts on the folded island
    p.edit("live.shelf", Some(tr("Значок с числом файлов", "File-count badge")), None);
    let mut pg = page(
        "modules",
        "modules",
        tr("Модули", "Modules"),
        vec![
            p.sec(tr("Погода", "Weather"), &["net.weather", "module.weather", "weather.city", "weather.units", "weather.refresh", "showWxIcon", "showTemp"]),
            p.sec(tr("Заметки и таймер", "Notes and timer"), &["module.note", "live.reminders", "sound.reminders", "sound.reminderTone", "sound.play"]),
            p.sec(tr("Лаунчер", "Launcher"), &["module.launcher"]),
            p.sec(tr("Полка файлов", "File shelf"), &["module.shelf", "live.shelf"]),
            p.sec(tr("Текстовые инструменты", "Text tools"), &["module.text"]),
            p.sec(tr("Управление системой", "System controls"), &["module.controls", "live.sysmon"]),
            p.sec(tr("VPN", "VPN"), &["module.vpn"]),
            p.note(tr("Вкладки острова", "Island tabs"), tabs, &["modules.tabs"]),
            p.note(tr("Что на главной", "What is on Home"), home, &["modules.home"]),
            p.note(tr("Профили", "Profiles"), profiles, &["modules.profiles"]),
            p.sec(tr("Заметки и таймер: свой звук", "Notes and timer: own sound"), &["timer.sound", "timer.soundFile"]).adv(),
            p.note(tr("VPN: свой клиент", "VPN: your own client"), vpn, &["vpn.own.name", "vpn.own.connect", "vpn.own.disconnect", "vpn.admin"]).adv(),
        ],
    );
    pg.footnote = p.footnote("modules");
    pg
}

pub(super) fn live(p: &mut Pool) -> Page {
    // what is read: the old footnote of the page, without the volume and microphone sentences (they went to the Sound and voice page)
    let read = tr(
        "Что читается. Загрузки: только имена и размеры файлов во временных браузерных файлах (.crdownload, .part, .tmp) в папке «Загрузки»; содержимое файлов не читается. Готовый файл: только имена и размеры файлов в «Загрузках». Съёмные диски: список букв дисков. Батарея: уровень заряда и подключено ли питание. Календарь: только файл .ics, который вы указали. Пока переключатель выключен, ничего из этого не опрашивается.",
        "What is read. Downloads: only the names and sizes of browser temp files (.crdownload, .part, .tmp) in your Downloads folder; file contents are not read. Finished file: only the names and sizes of files in Downloads. Removable drives: the list of drive letters. Battery: the charge level and whether the power is plugged in. Calendar: only the .ics file you point to. While a switch is off, none of this is polled.",
    );
    let mut pg = page(
        "live",
        "live",
        tr("События", "Events"),
        vec![
            p.sec(tr("Уведомления и карточки", "Notifications and cards"), &["module.notifs", "live.notifications", "live.notifications.allow"]),
            p.sec(tr("Загрузки", "Downloads"), &["live.downloads", "live.dlDone", "live.steam"]),
            p.sec(tr("Флешки и съёмные диски", "USB and removable drives"), &["live.usb"]),
            p.sec(tr("Батарея", "Battery"), &["live.battery", "showBattery"]),
            p.sec(tr("Раскладка клавиатуры", "Keyboard layout"), &["live.keyboardLayout", "live.keyboardLayoutAlways"]),
            p.sec(tr("Виртуальные рабочие столы", "Virtual desktops"), &["live.virtualDesktopFlash", "live.virtualDesktop"]),
            p.sec(tr("OBS: запись и трансляция", "OBS: recording and streaming"), &["live.obs"]),
            p.sec(tr("Календарь", "Calendar"), &["live.calendar", "live.calendar.path"]),
            p.sec(tr("Почта (IMAP)", "Mail (IMAP)"), &["live.mail", "mail.host", "mail.port", "mail.user", "mail.mailbox", "mail.password", "live.mail.test"]),
            p.sec(tr("Уведомления и карточки: время показа и история", "Notifications and cards: display time and history"), &["cards.duration", "history.keep", "history.mail"]).adv(),
            p.sec(tr("Загрузки: подробности", "Downloads: details"), &["live.downloadRing", "live.dlIgnoreExt"]).adv(),
            p.sec(tr("OBS: подключение", "OBS: connection"), &["live.obs.port", "live.obs.password"]).adv(),
        ],
    );
    pg.footnote = read;
    pg
}

pub(super) fn clipboard(p: &mut Pool) -> Page {
    let rec = p.footer("clipboard", 1);
    let mut pg = page(
        "clipboard",
        "clipboard",
        tr("Буфер и скриншоты", "Clipboard"),
        vec![
            p.sec(tr("История буфера", "Clipboard history"), &["module.clipboard", "live.copy", "clipboard.maxItems", "clipboard.retentionDays", "clipboard.private", "clipboard.skipSecrets", "clipboard.wipe"]),
            p.sec(tr("Скриншоты", "Screenshots"), &["capture.shotAction", "capture.annotate", "capture.folder"]),
            p.note(tr("Запись экрана", "Screen recording"), rec, &["live.recording", "capture.format", "capture.fps", "capture.recordSystemAudio", "capture.recordMic", "capture.videoFolder"]),
            p.sec(tr("История буфера: вставка, исключения, лимиты", "Clipboard history: pasting, exclusions, limits"), &["clipClickPaste", "clipboard.denyList", "clipboard.maxImageMb"]).adv(),
            p.sec(tr("Скриншоты и запись: подробности", "Screenshots and recording: details"), &["capture.encoder", "capture.revealAfterSave", "showInCapture"]).adv(),
        ],
    );
    // the sidebar has room for a short name only
    pg.head = tr("Буфер и скриншоты", "Clipboard and screenshots");
    pg
}

pub(super) fn sound(p: &mut Pool) -> Page {
    // "Set up in the cloud" for voice only jumped to the recognition-service chips that now stand in the same section
    p.drop_rows(&["aisetup.cloud-voice"]);
    let read = tr(
        "Что читается. Громкость: только уровень и «без звука» устройства вывода по умолчанию. Микрофон и камера: в реестре Windows (только чтение, ветка вашего пользователя) отметки о том, какое приложение сейчас пользуется устройством; звук и изображение не открываются и не читаются. Пока переключатель выключен, это не опрашивается.",
        "What is read. Volume: only the level and the mute flag of the default output device. Microphone and camera: marks in the Windows registry (read-only, your user branch) saying which app is using the device right now; sound and picture are never opened or read. While a switch is off, none of this is polled.",
    );
    page(
        "sound",
        "sound",
        tr("Звук и голос", "Sound and voice"),
        vec![
            p.note(tr("Громкость и микрофон", "Volume and microphone"), read, &["live.volume", "sound.volumeDevice", "audio.volumeTarget", "sound.micDevice", "live.micCam"]),
            p.sec(tr("Медиаплеер", "Media player"), &["module.media", "live.music", "musicPill", "audio.reactive", "music.lyrics", "home.player", "home.lyrics"]),
            p.sec(tr("Голос в текст", "Voice to text"), &["voice.enabled", "voice.mode", "voice.language", "voice.paste", "voice.provider", "voice.key", "aisetup.whisper", "voice.test-mic", "voice.test-service"]),
            p.sec(tr("Медиаплеер: источники звука", "Media player: sound sources"), &["audio.perApp", "media.legacyPlayers"]).adv(),
            p.sec(tr("Голос в текст: сервис", "Voice to text: the service"), &["voice.endpoint", "voice.model", "voice.autostop"]).adv(),
        ],
    )
}

pub(super) fn phone(p: &mut Pool) -> Page {
    let (link, apps, clip, relay) = (p.footer("phone", 0), p.footer("phone", 1), p.footer("phone", 2), p.footer("phone", 4));
    // the three switches of the Phone: the link (the server on this PC), the tab, the cards on the island
    p.edit(
        "phone.enabled",
        None,
        Some(tr(
            "Главный выключатель: Pill принимает соединения от телефона. Уведомления телефона на острове, ответы на них и файлы между телефоном и компьютером (приложение Pill Phone). Без него вкладка и карточки ниже ничего не получают",
            "The main switch: Pill accepts connections from the phone. Your phone's notifications on the island, replies to them and files between phone and PC (the Pill Phone app). Without it the tab and the cards below receive nothing",
        )),
    );
    p.edit(
        "module.phone",
        Some(tr("Вкладка «Телефон»", "Phone tab")),
        Some(tr(
            "Показывает вкладку с уведомлениями и файлами с телефона. Выключено: вкладка скрыта; сама связь включается выше",
            "Shows the tab with your phone's notifications and files. Off: the tab is hidden; the link itself is switched on above",
        )),
    );
    p.edit("live.phone", Some(tr("Карточки с телефона", "Cards from the phone")), None);
    page(
        "phone",
        "phone",
        tr("Телефон", "Phone"),
        vec![
            p.note(tr("Связь с телефоном", "Phone link"), link, &["phone.enabled", "module.phone", "live.phone", "phone.link", "phone.devices"]),
            p.note(tr("Приложения, чьи уведомления приходят", "Apps whose notifications arrive"), apps, &["phone.apps"]),
            p.note(tr("Общий буфер с телефоном", "Clipboard shared with the phone"), clip, &["phone.clipboard", "phone.clipToPhone", "phone.clipFromPhone"]),
            p.sec(tr("Файлы с телефона", "Files from the phone"), &["phone.downloadDir", "phone.autoAccept"]),
            p.note(tr("Через интернет (ретранслятор)", "Over the internet (relay)"), relay, &["phone.relay"]),
            p.sec(tr("Файлы и приложение: подробности", "Files and the app: details"), &["phone.maxFileMB", "phone.apkUrl"]).adv(),
            p.sec(tr("Ретранслятор: адрес и предел", "Relay: address and size cap"), &["phone.relayUrl", "phone.relayMaxFileMB"]).adv(),
        ],
    )
}

pub(super) fn ai(p: &mut Pool) -> Page {
    // "Set up a cloud AI" only jumped to the assistant section that now stands right above on the same page
    p.drop_rows(&["aisetup.cloud-chat"]);
    // the Whisper install moved to Sound and voice, next to the rest of the dictation rows
    p.edit(
        "aisetup.intro",
        None,
        Some(tr(
            "ИИ-чат можно запустить на этом компьютере или подключить облачный сервис. Голос в текст ставится на странице «Звук и голос». Ничего не скачивается и не запускается без вашего нажатия; у кнопки указано, откуда и сколько.",
            "The AI chat can run on this computer or through a cloud service. Voice to text is installed on the Sound and voice page. Nothing is downloaded or started without your click; the button shows the source and size.",
        )),
    );
    p.edit(
        "aiSetupDone",
        None,
        Some(tr("Раздел остаётся в настройках, на странице «ИИ»: вернуться можно в любой момент.", "The section stays in Settings, on the AI page: you can come back at any time.")),
    );
    page(
        "ai",
        "ai",
        tr("ИИ", "AI"),
        vec![
            p.sec(tr("ИИ-помощник и чат", "AI assistant and chat"), &["net.ai", "module.chat", "ai.provider", "ai.key", "ai.baseUrl", "ai.model", "ai.list", "ai.test"]),
            p.sec(tr("ИИ-агенты в терминале", "AI agents in the terminal"), &["live.agents", "live.agents.snippets", "agents.answerFromIsland"]),
            p.sec(tr("Локальный ИИ-чат на этом компьютере", "Local AI chat on this computer"), &["aisetup.intro", "aisetup.chat"]),
            p.sec(Tr::NONE, &["aiSetupDone"]),
        ],
    )
}

pub(super) fn updates(p: &mut Pool) -> Page {
    let mut pg = page(
        "updates",
        "updates",
        tr("Обновления", "Updates"),
        vec![
            p.sec(tr("Версия и обновления", "Version and updates"), &["updates.check", "updateAuto"]),
            p.sec(tr("Что нового", "What is new"), &["whatsNew", "updates.whatsnew"]),
            p.sec(tr("Помощь", "Help"), &["updates.report"]),
        ],
    );
    pg.head = tr("Обновления и о программе", "Updates and about");
    pg
}
