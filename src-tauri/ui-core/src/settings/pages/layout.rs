//! The layout of the Settings window: which page, which section (group) and in which order every row stands.
//!
//! The rows themselves (label, hint, control, default, key) are defined once in `base.rs`, `system.rs`, `phone.rs` and `ai.rs` as the old
//! 16 pages. This file only takes them out of a pool by id and arranges them into the pages of the sidebar.
//!
//! SIXTEEN pages, each one coherent family (`nav::NAV`, under four small group headings): Island (Look = look + clock and date + animation;
//! Edge and character = edge + pet; Tabs and Home), Features (Weather; Media and sound = media player + volume and microphone;
//! Voice to text; Clipboard; Screen capture = screenshots + recording; Tools = notes and timer, launcher, file shelf, text tools, system controls,
//! VPN; Phone link; AI = assistant, agents, local AI), Events (Notifications and cards = notifications, downloads, battery, keyboard layout,
//! virtual desktops, USB drives; Calendar and mail = calendar, mail, OBS), App (General, Hotkeys, Updates).
//!
//! Every function is a titled SECTION of its page: its on/off switch first, then its events (the `live.*` rows), then its parameters. The rare
//! and technical rows of all the functions of a page go into trailing `.adv()` sections that the window folds behind the one collapsed
//! "Advanced" disclosure of THAT page (`Part` / `pg`). A row cannot be lost or doubled: `Pool::take` removes it, `Pool::finish` checks that
//! nothing is left behind.
//!
//! Three groups of switches overlap on purpose and are NOT merged, only placed side by side and labelled apart (`Pool::edit`): Weather
//! (`net.weather` asks the internet, `module.weather` is the tab), Phone (`phone.enabled` is the link itself, `module.phone` the tab,
//! `live.phone` the cards) and File shelf (`module.shelf` is the shelf, `live.shelf` the badge on the island).
//!
//! Everything that belongs to no single function (start and language, hiding, quiet hours, command bar search, the settings window's own switch,
//! backup, first run) stands together on ONE page, `general`.
//!
//! Where a page fetches data when it shows up (`settings_view/nav.rs::page_shown`) and which pages poll statuses (Notifications and cards,
//! Calendar and mail, Phone) is decided per page id there.
//!
//! Moved rows keep their old page name in their search keywords ("behavior", "Живые события", ...) and also the name of their section
//! ("Weather", "Погода", "Voice to text", ...), so the old names and the function names both find them.
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

impl Pool {
    /// The one main section of a function page: no caption (the page's own heading names it), but the page's name joins the search keywords.
    fn main(&mut self, name: Tr, ids: &[&str]) -> Group {
        group(Tr::NONE, self.feature(name, ids))
    }

    /// [`Pool::main`] with a note under the card.
    fn main_note(&mut self, name: Tr, footer: Tr, ids: &[&str]) -> Group {
        group_note(Tr::NONE, footer, self.feature(name, ids))
    }
}

/// One function on a page: its sections in view and its rare ones (`.adv()`), which the page gathers behind its one "Advanced" disclosure.
pub(super) struct Part {
    main: Vec<Group>,
    adv: Vec<Group>,
}

fn part(main: Vec<Group>, adv: Vec<Group>) -> Part {
    Part { main, adv }
}

/// A page of one or more functions: sidebar name `title`, heading `head` (blank = the sidebar name). The sections of every function come
/// first, in order; the rare ones of all of them follow as the trailing `advanced` run.
fn pg(id: &'static str, icon: &'static str, title: Tr, head: Tr, parts: Vec<Part>) -> Page {
    let (mut groups, mut adv) = (Vec::new(), Vec::new());
    for q in parts {
        groups.extend(q.main);
        adv.extend(q.adv);
    }
    groups.extend(adv);
    let mut p = page(id, icon, title, groups);
    p.head = head;
    p
}

// ------------------------------------------------------------------------------------------------ Island

pub(super) fn look(p: &mut Pool) -> Page {
    pg(
        "look",
        "palette",
        tr("Вид", "Look"),
        tr("Внешний вид", "Appearance"),
        vec![
            part(
                vec![p.sec(tr("Вид острова", "Island look"), &["shape", "theme", "island.color", "size", "monitor"])],
                vec![p.sec(tr("Цвет, прозрачность, положение", "Colour, transparency, position"), &["island.colorCustom", "opacityCol", "opacityExp", "offsetX", "underTaskbar"]).adv()],
            ),
            part(vec![p.sec(tr("Часы и дата", "Clock and date"), &["showTime", "showDate", "showSeconds", "clock24"])], vec![]),
            part(vec![p.sec(tr("Анимация", "Animation"), &["reduceMotion"])], vec![p.sec(tr("Частота кадров и видеокарта", "Frame rate and GPU"), &["fps", "edgeIdleFps", "ui.gpu"]).adv()]),
        ],
    )
}

pub(super) fn edge(p: &mut Pool) -> Page {
    // the reaction note without its first sentence: the row "Island" already says that the edge style applies at once
    let reactions = tr(
        "Реакции включаются сами: агенты, музыка, файлы, напоминания; через 5 минут тишины остров засыпает.",
        "Reactions fire on their own: agents, music, files, reminders; after 5 quiet minutes the island falls asleep.",
    );
    pg(
        "edge",
        "character",
        tr("Край и персонаж", "Edge and character"),
        tr("Край острова и персонаж", "Island edge and character"),
        vec![
            part(vec![p.note(tr("Край острова", "Island edge"), reactions, &["character", "character.liquid", "character.reactions"])], vec![]),
            part(vec![p.sec(tr("Питомец", "Pet"), &["pet", "pet.side", "pet.size", "pet.keepDuringMusic"])], vec![]),
        ],
    )
}

pub(super) fn tabs(p: &mut Pool) -> Page {
    let profiles = p.footer("modules", 0);
    let tabs = tr(
        "Скрытая вкладка исчезает из полосы вкладок, но по-прежнему открывается своей горячей клавишей и командой. Это только про полосу: чтобы функция перестала работать, выключи её модуль. Этот переключатель стоит на странице самой функции (страницы «Функции» и «События» слева: «Медиа и звук» (Медиаплеер), «Буфер обмена», «Погода», «Инструменты» (Заметки и таймер, Лаунчер, Полка файлов, Текстовые инструменты, Управление системой, VPN), «Уведомления и карточки»; Чат с ИИ: «ИИ»; Телефон: «Связь с телефоном»; Поиск файлов: «Общие», раздел «Поиск в командной строке»).",
        "A hidden tab leaves the tab strip but still opens with its hotkey and from the command bar. This only concerns the strip: to stop a feature from working, switch its module off. That switch is on the feature's own page (the Features and Events lists on the left: Media and sound (Media player), Clipboard, Weather, Tools (Notes and timer, Launcher, File shelf, Text tools, System controls, VPN), Notifications and cards; AI chat: the AI page; Phone: Phone link; File search: General, the Command bar search section).",
    );
    let home = tr(
        "«Плеер» и «Ярлыки» это те же переключатели, что «Медиаплеер» (страница «Медиа и звук») и «Лаунчер» (страница «Инструменты»). «Недавно скопировано» показывает последние записи истории буфера прямо на главной (нужна «История буфера»): текст виден всем, кто смотрит на экран, поэтому по умолчанию выключено. Ничего не уходит с компьютера.",
        "“Player” and “Shortcuts” are the same switches as “Media player” (the Media and sound page) and “Launcher” (the Tools page). “Recently copied” shows the latest clipboard history entries right on Home (needs “Clipboard history”): anyone looking at the screen can read that text, so it is off by default. Nothing leaves the computer.",
    );
    let title = tr("Вкладки и главная", "Tabs and Home");
    let mut page = pg(
        "tabs",
        "home",
        title,
        Tr::NONE,
        vec![part(
            vec![
                p.note(tr("Вкладки острова", "Island tabs"), tabs, &["modules.tabs"]),
                p.note(tr("Что на главной", "What is on Home"), home, &["modules.home"]),
                p.note(tr("Профили", "Profiles"), profiles, &["modules.profiles"]),
            ],
            vec![],
        )],
    );
    page.footnote = p.footnote("modules");
    page
}

// ------------------------------------------------------------------------------------------------ App (General, Hotkeys, Updates)

pub(super) fn general(p: &mut Pool) -> Page {
    // One page for everything that is not about a single function: start and language, hiding, quiet hours, the command bar's search and the
    // settings window itself. What is rare (details of hiding, pass-through, Everything, backup, first run) is folded into the one Advanced part.
    let browser = p.footer("search", 2);
    let everything = p.footer("search", 0);
    let title = tr("Общие", "General");
    pg(
        "general",
        "general",
        title,
        Tr::NONE,
        vec![part(
            vec![
                p.sec(tr("Запуск и язык", "Start and language"), &["language", "general.autostart"]),
                p.sec(tr("Прятать остров", "Hiding the pill"), &["hideInFullscreen", "fullscreenPeek", "autoHide"]),
                p.sec(tr("Тихие часы", "Quiet hours"), &["quiet.on", "quiet.from", "quiet.to", "quiet.days"]),
                p.note(tr("Поиск в командной строке", "Command bar search"), browser, &["module.fileSearch", "search.engine", "search.customUrl"]),
                p.sec(tr("Окно настроек", "Settings window"), &["settings.preview"]),
            ],
            vec![
                p.sec(tr("Подробности и списки программ", "Details and program lists"), &["fullscreenPeekEdge", "autoHideKeepDesktop", "hideInApps", "behavior.hideAddActive", "showInApps", "behavior.showAddActive"]).adv(),
                p.sec(tr("Сквозной режим", "Pass-through mode"), &["passThrough", "passThroughOpacity"]).adv(),
                p.note(tr("Поиск файлов: Everything", "File search: Everything"), everything, &["search.everything", "everything.autostart"]).adv(),
                p.sec(tr("Копия настроек", "Settings backup"), &["general.backup"]).adv(),
                p.sec(tr("Первый запуск", "First run"), &["general.wizard"]).adv(),
            ],
        )],
    )
}

pub(super) fn hotkeys(p: &mut Pool) -> Page {
    // Hotkeys stay on one page on purpose: the recorder checks a combo against all the others, and a list of every key is what people look for
    pg(
        "hotkeys",
        "keyboard",
        tr("Клавиши", "Hotkeys"),
        tr("Горячие клавиши", "Hotkeys"),
        vec![part(
            vec![
                p.sec(tr("Остров, поиск, чат, диктовка и буфер", "Island, command bar, chat, dictation and clipboard"), &["hotkey.toggle", "hotkey.command", "hotkey.chat", "hotkey.voice", "hotkey.clipboard", "hotkey.pastePlain", "hotkey.mic"]),
                p.sec(tr("Экран: запись, снимки, текст и цвет", "Screen: recording, shots, text and colour"), &["hotkey.record", "hotkey.shotArea", "hotkey.shotFull", "hotkey.ocr", "hotkey.pipette", "hotkey.qr"]),
                p.sec(tr("Ярлыки лаунчера", "Launcher shortcuts"), &["hotkeys.launcher"]),
            ],
            vec![p.sec(tr("Пока без сочетания", "Not assigned yet"), &["hotkey.uninstall", "hotkey.passThrough", "hotkey.theme", "hotkey.musicView", "hotkey.vpn"]).adv()],
        )],
    )
}

pub(super) fn updates(p: &mut Pool) -> Page {
    pg(
        "updates",
        "updates",
        tr("Обновления", "Updates"),
        tr("Обновления и о программе", "Updates and about"),
        vec![part(
            vec![
                p.sec(tr("Версия и обновления", "Version and updates"), &["updates.check", "updateAuto"]),
                p.sec(tr("Что нового", "What is new"), &["whatsNew", "updates.whatsnew"]),
                p.sec(tr("Помощь", "Help"), &["updates.report"]),
            ],
            vec![],
        )],
    )
}

// ------------------------------------------------------------------------------------------------ Features

pub(super) fn weather(p: &mut Pool) -> Page {
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
    let title = tr("Погода", "Weather");
    pg("weather", "weather", title, Tr::NONE, vec![part(vec![p.main(title, &["net.weather", "module.weather", "weather.city", "weather.units", "weather.refresh", "showWxIcon", "showTemp"])], vec![])])
}

pub(super) fn media(p: &mut Pool) -> Page {
    let read = tr(
        "Что читается. Громкость: только уровень и «без звука» устройства вывода по умолчанию. Микрофон и камера: в реестре Windows (только чтение, ветка вашего пользователя) отметки о том, какое приложение сейчас пользуется устройством; звук и изображение не открываются и не читаются. Пока переключатель выключен, это не опрашивается.",
        "What is read. Volume: only the level and the mute flag of the default output device. Microphone and camera: marks in the Windows registry (read-only, your user branch) saying which app is using the device right now; sound and picture are never opened or read. While a switch is off, none of this is polled.",
    );
    let title = tr("Медиа и звук", "Media and sound");
    pg(
        "media",
        "note2",
        title,
        Tr::NONE,
        vec![
            part(
                vec![p.sec(tr("Медиаплеер", "Media player"), &["module.media", "live.music", "musicPill", "audio.reactive", "music.lyrics", "home.player", "home.lyrics"])],
                vec![p.sec(tr("Источники звука", "Sound sources"), &["audio.perApp", "media.legacyPlayers"]).adv()],
            ),
            part(vec![p.note(tr("Громкость и микрофон", "Volume and microphone"), read, &["live.volume", "sound.volumeDevice", "audio.volumeTarget", "sound.micDevice", "live.micCam"])], vec![]),
        ],
    )
}

pub(super) fn voice(p: &mut Pool) -> Page {
    // "Set up in the cloud" for voice only jumped to the recognition-service chips that now stand in the same section
    p.drop_rows(&["aisetup.cloud-voice"]);
    let title = tr("Голос в текст", "Voice to text");
    pg(
        "voice",
        "mic",
        title,
        Tr::NONE,
        vec![part(
            vec![p.main(title, &["voice.enabled", "voice.mode", "voice.language", "voice.paste", "voice.provider", "voice.key", "aisetup.whisper", "voice.test-mic", "voice.test-service"])],
            vec![p.sec(tr("Сервис распознавания", "Recognition service"), &["voice.endpoint", "voice.model", "voice.autostop"]).adv()],
        )],
    )
}

pub(super) fn clipboard(p: &mut Pool) -> Page {
    let title = tr("Буфер обмена", "Clipboard");
    let head = tr("История буфера", "Clipboard history");
    pg(
        "clipboard",
        "clipboard",
        title,
        head,
        vec![part(
            vec![p.main(head, &["module.clipboard", "live.copy", "clipboard.maxItems", "clipboard.retentionDays", "clipboard.private", "clipboard.skipSecrets", "clipboard.wipe"])],
            vec![p.sec(tr("Вставка, исключения, лимиты", "Pasting, exclusions, limits"), &["clipClickPaste", "clipboard.denyList", "clipboard.maxImageMb"]).adv()],
        )],
    )
}

pub(super) fn capture(p: &mut Pool) -> Page {
    let rec = p.footer("clipboard", 1);
    let title = tr("Захват экрана", "Screen capture");
    pg(
        "capture",
        "area",
        title,
        Tr::NONE,
        vec![
            part(
                vec![p.sec(tr("Скриншоты", "Screenshots"), &["capture.shotAction", "capture.annotate", "capture.folder"])],
                vec![p.sec(tr("Скриншоты: подробности", "Screenshots: details"), &["capture.revealAfterSave", "showInCapture"]).adv()],
            ),
            part(
                vec![p.note(tr("Запись экрана", "Screen recording"), rec, &["live.recording", "capture.format", "capture.fps", "capture.recordSystemAudio", "capture.recordMic", "capture.videoFolder"])],
                vec![p.sec(tr("Запись: кодировщик", "Recording: encoder"), &["capture.encoder"]).adv()],
            ),
        ],
    )
}

pub(super) fn tools(p: &mut Pool) -> Page {
    let own = p.footer("modules", 2);
    // File shelf: the shelf itself, and the badge it puts on the folded island
    p.edit("live.shelf", Some(tr("Значок с числом файлов", "File-count badge")), None);
    let launcher = tr("Сочетания клавиш для ярлыков лаунчера стоят на странице «Клавиши».", "The hotkeys of the launcher shortcuts are on the Hotkeys page.");
    pg(
        "tools",
        "apps",
        tr("Инструменты", "Tools"),
        Tr::NONE,
        vec![
            part(
                vec![p.sec(tr("Заметки и таймер", "Notes and timer"), &["module.note", "live.reminders", "sound.reminders", "sound.reminderTone", "sound.play"])],
                vec![p.sec(tr("Заметки: свой звук", "Notes: own sound"), &["timer.sound", "timer.soundFile"]).adv()],
            ),
            part(vec![p.note(tr("Лаунчер", "Launcher"), launcher, &["module.launcher"])], vec![]),
            part(vec![p.sec(tr("Полка файлов", "File shelf"), &["module.shelf", "live.shelf"])], vec![]),
            part(vec![p.sec(tr("Текстовые инструменты", "Text tools"), &["module.text"])], vec![]),
            part(vec![p.sec(tr("Управление системой", "System controls"), &["module.controls", "live.sysmon"])], vec![]),
            part(
                vec![p.sec(tr("VPN", "VPN"), &["module.vpn"])],
                vec![p.note(tr("VPN: свой клиент", "VPN: your own client"), own, &["vpn.own.name", "vpn.own.connect", "vpn.own.disconnect", "vpn.admin"]).adv()],
            ),
        ],
    )
}

// ------------------------------------------------------------------------------------------------ Events

pub(super) fn notifications(p: &mut Pool) -> Page {
    // the Russian sidebar name is shorter than the heading: "Уведомления и карточки" does not fit the selected item
    let (title, head) = (tr("Уведомления", "Notifications and cards"), tr("Уведомления и карточки", "Notifications and cards"));
    let downloads = tr(
        "Что читается. Загрузки: только имена и размеры файлов во временных браузерных файлах (.crdownload, .part, .tmp) в папке «Загрузки»; содержимое файлов не читается. Готовый файл: только имена и размеры файлов в «Загрузках». Пока переключатель выключен, это не опрашивается.",
        "What is read. Downloads: only the names and sizes of browser temp files (.crdownload, .part, .tmp) in your Downloads folder; file contents are not read. Finished file: only the names and sizes of files in Downloads. While a switch is off, none of this is polled.",
    );
    let battery = tr(
        "Что читается. Батарея: уровень заряда и подключено ли питание. Пока переключатель выключен, это не опрашивается.",
        "What is read. Battery: the charge level and whether the power is plugged in. While the switch is off, none of this is polled.",
    );
    let usb = tr("Что читается. Съёмные диски: список букв дисков. Пока переключатель выключен, это не опрашивается.", "What is read. Removable drives: the list of drive letters. While the switch is off, none of this is polled.");
    pg(
        "notifications",
        "bell",
        title,
        head,
        vec![
            part(
                vec![p.sec(tr("Уведомления", "Notifications"), &["module.notifs", "live.notifications", "live.notifications.allow"])],
                vec![p.sec(tr("Время показа и история", "Display time and history"), &["cards.duration", "history.keep", "history.mail"]).adv()],
            ),
            part(
                vec![p.note(tr("Загрузки", "Downloads"), downloads, &["live.downloads", "live.dlDone", "live.steam"])],
                vec![p.sec(tr("Загрузки: подробности", "Downloads: details"), &["live.downloadRing", "live.dlIgnoreExt"]).adv()],
            ),
            part(vec![p.note(tr("Батарея", "Battery"), battery, &["live.battery", "showBattery"])], vec![]),
            part(vec![p.sec(tr("Раскладка клавиатуры", "Keyboard layout"), &["live.keyboardLayout", "live.keyboardLayoutAlways"])], vec![]),
            part(vec![p.sec(tr("Виртуальные рабочие столы", "Virtual desktops"), &["live.virtualDesktopFlash", "live.virtualDesktop"])], vec![]),
            part(vec![p.note(tr("Флешки и съёмные диски", "USB and removable drives"), usb, &["live.usb"])], vec![]),
        ],
    )
}

pub(super) fn calendar(p: &mut Pool) -> Page {
    let calendar = tr(
        "Что читается. Календарь: только файл .ics, который вы указали. Пока переключатель выключен, это не опрашивается.",
        "What is read. Calendar: only the .ics file you point to. While the switch is off, none of this is polled.",
    );
    pg(
        "calendar",
        "calendar",
        tr("Календарь и почта", "Calendar and mail"),
        tr("Календарь, почта и OBS", "Calendar, mail and OBS"),
        vec![
            part(vec![p.note(tr("Календарь", "Calendar"), calendar, &["live.calendar", "live.calendar.path"])], vec![]),
            part(vec![p.sec(tr("Почта (IMAP)", "Mail (IMAP)"), &["live.mail", "mail.host", "mail.port", "mail.user", "mail.mailbox", "mail.password", "live.mail.test"])], vec![]),
            part(vec![p.sec(tr("OBS: запись и трансляция", "OBS: recording and streaming"), &["live.obs"])], vec![p.sec(tr("OBS: подключение", "OBS: connection"), &["live.obs.port", "live.obs.password"]).adv()]),
        ],
    )
}

// ------------------------------------------------------------------------------------------------ Phone link and AI (in the Features group)

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
    pg(
        "phone",
        "phone",
        tr("Связь с телефоном", "Phone link"),
        Tr::NONE,
        vec![part(
            vec![
                p.main_note(tr("Связь с телефоном", "Phone link"), link, &["phone.enabled", "module.phone", "live.phone", "phone.link", "phone.devices"]),
                p.note(tr("Приложения, чьи уведомления приходят", "Apps whose notifications arrive"), apps, &["phone.apps"]),
                p.note(tr("Общий буфер с телефоном", "Clipboard shared with the phone"), clip, &["phone.clipboard", "phone.clipToPhone", "phone.clipFromPhone"]),
                p.sec(tr("Файлы с телефона", "Files from the phone"), &["phone.downloadDir", "phone.autoAccept"]),
                p.note(tr("Через интернет (ретранслятор)", "Over the internet (relay)"), relay, &["phone.relay"]),
            ],
            vec![
                p.sec(tr("Файлы и приложение: подробности", "Files and the app: details"), &["phone.maxFileMB", "phone.apkUrl"]).adv(),
                p.sec(tr("Ретранслятор: адрес и предел", "Relay: address and size cap"), &["phone.relayUrl", "phone.relayMaxFileMB"]).adv(),
            ],
        )],
    )
}

pub(super) fn ai(p: &mut Pool) -> Page {
    // "Set up a cloud AI" only jumped to the assistant section that now stands right above on the same page
    p.drop_rows(&["aisetup.cloud-chat"]);
    p.edit(
        "aisetup.intro",
        None,
        Some(tr(
            "ИИ-чат можно запустить на этом компьютере или подключить облачный сервис. Голос в текст ставится на странице «Голос в текст». Ничего не скачивается и не запускается без вашего нажатия; у кнопки указано, откуда и сколько.",
            "The AI chat can run on this computer or through a cloud service. Voice to text is installed on the Voice to text page. Nothing is downloaded or started without your click; the button shows the source and size.",
        )),
    );
    p.edit(
        "aiSetupDone",
        None,
        Some(tr("Раздел остаётся в настройках, на странице «ИИ», в разделе «Локальный ИИ»: вернуться можно в любой момент.", "The section stays in Settings, on the AI page under Local AI: you can come back at any time.")),
    );
    let title = tr("ИИ", "AI");
    pg(
        "ai",
        "sparkles",
        title,
        tr("ИИ: помощник, агенты и локальный ИИ", "AI: assistant, agents and local AI"),
        vec![
            part(vec![p.sec(tr("ИИ-помощник и чат", "AI assistant and chat"), &["net.ai", "module.chat", "ai.provider", "ai.key", "ai.baseUrl", "ai.model", "ai.list", "ai.test"])], vec![]),
            part(vec![p.sec(tr("ИИ-агенты в терминале", "AI agents in the terminal"), &["live.agents", "live.agents.snippets", "agents.answerFromIsland"])], vec![]),
            part(vec![p.sec(tr("Локальный ИИ-чат на этом компьютере", "Local AI chat on this computer"), &["aisetup.intro", "aisetup.chat", "aiSetupDone"])], vec![]),
        ],
    )
}
