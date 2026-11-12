//! The layout of the Settings window: which page, which section (group) and in which order every row stands.
//!
//! The rows themselves (label, hint, control, default, key) are defined once in `base.rs`, `system.rs`, `phone.rs` and `ai.rs` as the old
//! 16 pages. This file only takes them out of a pool by id and arranges them into the ten pages of the sidebar: the things people reach for
//! first at the top in named sections, the rarely needed ones in trailing `.adv()` sections that the window folds behind one collapsed
//! "Advanced" disclosure. A row cannot be lost or doubled: `Pool::take` removes it, `Pool::finish` checks that nothing is left behind.
//!
//! Moved rows keep their old page name in their search keywords ("behavior", "Живые события", ...), so the old names still find them.
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

    pub(super) fn sec(&mut self, title: Tr, ids: &[&str]) -> Group {
        group(title, self.take(ids))
    }

    pub(super) fn note(&mut self, title: Tr, footer: Tr, ids: &[&str]) -> Group {
        group_note(title, footer, self.take(ids))
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
            p.sec(Tr::NONE, &["pet"]),
            p.sec(Tr::NONE, &["pet.side", "pet.size", "pet.keepDuringMusic"]),
            p.sec(tr("Свёрнутый остров", "Folded island"), &["showTime", "showDate", "showWxIcon", "showTemp", "showSeconds", "clock24", "showBattery"]),
            p.sec(tr("Положение и прозрачность", "Position and transparency"), &["island.colorCustom", "opacityCol", "opacityExp", "offsetX", "underTaskbar"]).adv(),
            p.sec(tr("Анимация и плавность", "Animation and smoothness"), &["reduceMotion", "fps", "edgeIdleFps", "ui.gpu"]).adv(),
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
            p.sec(tr("Запуск и язык", "Start and language"), &["language", "general.autostart"]),
            p.sec(tr("Когда прятать остров", "When to hide the pill"), &["hideInFullscreen", "fullscreenPeek", "autoHide"]),
            p.sec(tr("Тихие часы", "Quiet hours"), &["quiet.on", "quiet.from", "quiet.to", "quiet.days"]),
            p.note(tr("Поиск в браузере", "Search in the browser"), browser, &["search.engine", "search.customUrl"]),
            p.sec(tr("Прятать остров: подробности", "Hiding the pill: details"), &["fullscreenPeekEdge", "autoHideKeepDesktop"]).adv(),
            p.sec(tr("Программы-исключения", "Program lists"), &["hideInApps", "behavior.hideAddActive", "showInApps", "behavior.showAddActive"]).adv(),
            p.sec(tr("Сквозной режим", "Pass-through mode"), &["passThrough", "passThroughOpacity"]).adv(),
            p.sec(tr("Скриншоты и запись экрана", "Screenshots and screen recording"), &["showInCapture"]).adv(),
            p.note(tr("Поиск файлов (Everything)", "File search (Everything)"), everything, &["search.everything", "everything.autostart"]).adv(),
            p.sec(tr("Копия настроек", "Settings backup"), &["general.backup"]).adv(),
            p.sec(tr("Первый запуск", "First run"), &["general.wizard"]).adv(),
        ],
    )
}

pub(super) fn hotkeys(p: &mut Pool) -> Page {
    let mut pg = page(
        "hotkeys",
        "hotkeys",
        tr("Клавиши", "Hotkeys"),
        vec![
            p.sec(tr("Основные", "Main"), &["hotkey.toggle", "hotkey.command", "hotkey.chat", "hotkey.voice", "hotkey.clipboard", "hotkey.pastePlain", "hotkey.mic"]),
            p.sec(tr("Экран", "Screen"), &["hotkey.record", "hotkey.shotArea", "hotkey.shotFull", "hotkey.ocr", "hotkey.pipette", "hotkey.qr"]),
            p.sec(tr("Ярлыки лаунчера", "Launcher shortcuts"), &["hotkeys.launcher"]),
            p.sec(tr("Пока без сочетания", "Not assigned yet"), &["hotkey.uninstall", "hotkey.passThrough", "hotkey.theme", "hotkey.musicView", "hotkey.vpn"]).adv(),
        ],
    );
    pg.head = tr("Горячие клавиши", "Hotkeys");
    pg
}

pub(super) fn modules(p: &mut Pool) -> Page {
    let mods = p.footnote("modules");
    let vpn = p.footer("modules", 2);
    let tabs = p.footer("modules", 4);
    let home = p.footer("modules", 3);
    let profiles = p.footer("modules", 0);
    page(
        "modules",
        "modules",
        tr("Модули", "Modules"),
        vec![
            p.note(
                tr("Модули", "Modules"),
                mods,
                &["module.media", "module.shelf", "module.launcher", "module.fileSearch", "module.clipboard", "module.text", "module.chat", "module.note", "module.weather", "module.controls", "module.phone", "module.notifs", "module.vpn"],
            ),
            p.note(tr("Профили", "Profiles"), profiles, &["modules.profiles"]),
            p.sec(tr("Погода", "Weather"), &["net.weather", "weather.city", "weather.units", "weather.refresh"]),
            p.note(tr("Вкладки острова", "Island tabs"), tabs, &["modules.tabs"]),
            p.note(tr("Что на главной", "What is on Home"), home, &["modules.home"]),
            p.note(tr("VPN: свой клиент", "VPN: your own client"), vpn, &["vpn.own.name", "vpn.own.connect", "vpn.own.disconnect", "vpn.admin"]).adv(),
        ],
    )
}

pub(super) fn live(p: &mut Pool) -> Page {
    let mut pg = page(
        "live",
        "live",
        tr("События", "Events"),
        vec![
            p.sec(tr("Медиа и экран", "Media and screen"), &["live.volume", "live.music", "live.micCam", "live.recording", "live.obs"]),
            p.sec(tr("Загрузки и диски", "Downloads and drives"), &["live.downloads", "live.dlDone", "live.steam", "live.usb"]),
            p.sec(tr("Система", "System"), &["live.battery", "live.sysmon", "live.keyboardLayout", "live.virtualDesktopFlash", "live.copy", "live.shelf"]),
            p.sec(tr("Сообщения и напоминания", "Messages and reminders"), &["live.notifications", "live.notifications.allow", "live.reminders", "live.phone"]),
            p.sec(tr("Календарь", "Calendar"), &["live.calendar", "live.calendar.path"]),
            p.sec(tr("Почта (IMAP)", "Mail (IMAP)"), &["live.mail", "mail.host", "mail.port", "mail.user", "mail.mailbox", "mail.password", "live.mail.test"]),
            p.sec(tr("ИИ-агенты в терминале", "AI agents in the terminal"), &["live.agents", "live.agents.snippets", "agents.answerFromIsland"]),
            p.sec(tr("Карточки и история", "Cards and history"), &["cards.duration", "history.keep", "history.mail"]).adv(),
            p.sec(tr("Загрузки: подробности", "Downloads: details"), &["live.downloadRing", "live.dlIgnoreExt"]).adv(),
            p.sec(tr("Метки у часов", "Chips by the clock"), &["live.keyboardLayoutAlways", "live.virtualDesktop"]).adv(),
            p.sec(tr("OBS: подключение", "OBS: connection"), &["live.obs.port", "live.obs.password"]).adv(),
        ],
    );
    pg.footnote = p.footnote("live");
    pg
}

pub(super) fn clipboard(p: &mut Pool) -> Page {
    let rec = p.footer("clipboard", 1);
    let mut pg = page(
        "clipboard",
        "clipboard",
        tr("Буфер и скриншоты", "Clipboard"),
        vec![
            p.sec(tr("История буфера", "Clipboard history"), &["clipboard.maxItems", "clipboard.retentionDays", "clipboard.private", "clipboard.skipSecrets", "clipboard.wipe"]),
            p.sec(tr("Скриншоты", "Screenshots"), &["capture.shotAction", "capture.annotate", "capture.folder"]),
            p.note(tr("Запись экрана", "Screen recording"), rec, &["capture.format", "capture.fps", "capture.recordSystemAudio", "capture.recordMic", "capture.videoFolder"]),
            p.sec(tr("Буфер: вставка, исключения, лимиты", "Clipboard: pasting, exclusions, limits"), &["clipboard.clickPaste", "clipboard.denyList", "clipboard.maxImageMb"]).adv(),
            p.sec(tr("Скриншоты и запись: подробности", "Screenshots and recording: details"), &["capture.encoder", "capture.revealAfterSave"]).adv(),
        ],
    );
    // the sidebar has room for a short name only
    pg.head = tr("Буфер и скриншоты", "Clipboard and screenshots");
    pg
}

pub(super) fn sound(p: &mut Pool) -> Page {
    page(
        "sound",
        "sound",
        tr("Звук и голос", "Sound and voice"),
        vec![
            p.sec(tr("Громкость и микрофон", "Volume and microphone"), &["sound.volumeDevice", "sound.micDevice", "audio.volumeTarget"]),
            p.sec(tr("Музыка", "Music"), &["musicPill", "audio.reactive", "music.lyrics", "home.player", "home.lyrics"]),
            p.sec(tr("Напоминания и таймеры", "Reminders and timers"), &["sound.reminders", "sound.reminderTone", "sound.play"]),
            p.sec(tr("Голос в текст", "Voice to text"), &["voice.enabled", "voice.provider", "voice.key", "voice.language", "voice.mode", "voice.paste", "voice.test-mic", "voice.test-service"]),
            p.sec(tr("Музыка: источники звука", "Music: sound sources"), &["audio.perApp", "media.legacyPlayers"]).adv(),
            p.sec(tr("Звук напоминаний: свой файл", "Reminder sound: your own file"), &["timer.sound", "timer.soundFile"]).adv(),
            p.sec(tr("Голос в текст: сервис", "Voice to text: the service"), &["voice.endpoint", "voice.model", "voice.autostop"]).adv(),
        ],
    )
}

pub(super) fn phone(p: &mut Pool) -> Page {
    let (link, apps, clip, relay) = (p.footer("phone", 0), p.footer("phone", 1), p.footer("phone", 2), p.footer("phone", 4));
    page(
        "phone",
        "phone",
        tr("Телефон", "Phone"),
        vec![
            p.note(tr("Связь с телефоном", "Phone link"), link, &["phone.enabled", "phone.link", "phone.devices"]),
            p.note(tr("Уведомления", "Notifications"), apps, &["phone.apps"]),
            p.note(tr("Буфер обмена", "Clipboard"), clip, &["phone.clipboard", "phone.clipToPhone", "phone.clipFromPhone"]),
            p.sec(tr("Файлы", "Files"), &["phone.downloadDir", "phone.autoAccept"]),
            p.note(tr("Через интернет (ретранслятор)", "Over the internet (relay)"), relay, &["phone.relay"]),
            p.sec(tr("Файлы и приложение: подробности", "Files and the app: details"), &["phone.maxFileMB", "phone.apkUrl"]).adv(),
            p.sec(tr("Ретранслятор: адрес и предел", "Relay: address and size cap"), &["phone.relayUrl", "phone.relayMaxFileMB"]).adv(),
        ],
    )
}

pub(super) fn ai(p: &mut Pool) -> Page {
    // "Set up a cloud AI" only jumped to the assistant section that now stands right above on the same page
    p.drop_rows(&["aisetup.cloud-chat"]);
    page(
        "ai",
        "ai",
        tr("ИИ", "AI"),
        vec![
            p.sec(tr("ИИ-помощник", "AI assistant"), &["net.ai", "ai.provider", "ai.key", "ai.baseUrl", "ai.model", "ai.list", "ai.test"]),
            p.sec(tr("Установка ИИ на этот компьютер", "Installing AI on this computer"), &["aisetup.intro"]),
            p.sec(tr("Голос в текст (Whisper)", "Voice to text (Whisper)"), &["aisetup.whisper", "aisetup.cloud-voice"]),
            p.sec(tr("ИИ-чат", "AI chat"), &["aisetup.chat"]),
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
