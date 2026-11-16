//! What the preview shows for a setting: a lookup from the settings.json key (and, for a page opened without a change, the page) to a
//! [`Scene`], and the caption that says what is on show. Keyed by the settings key and not by row id or page, so a regrouping of the rows
//! never breaks it. A key that is not here keeps the scene that was on (the default island on a fresh window).
use pill_ui_core::panel::a::home_mods as hmod;
use pill_ui_core::settings::schema::{schema, Control, Dynamic};
use pill_ui_core::settings::tr::{tr, Lang, Tr};

/// The tabs the preview can open (the ones with a native view that works from sample data alone).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Home,
    Weather,
    Clipboard,
}

impl Tab {
    pub fn id(self) -> &'static str {
        match self {
            Tab::Home => "home",
            Tab::Weather => "weather",
            Tab::Clipboard => "clipboard",
        }
    }
}

/// The reactions of the folded island that a Live events switch turns on and off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flash {
    Copy,
    Kbd,
    Vdesk,
    Volume,
    Notif,
    Battery,
    Usb,
    Download,
    Reminder,
    Recording,
    Music,
    Privacy,
    Toast,
    /// a download in progress (the chip or the ring by the clock)
    Progress,
    Steam,
    Shelf,
    Agents,
    Phone,
    Calendar,
    Mail,
    Obs,
    /// Caps Lock / Num Lock chips, the Bluetooth chip and the Focus Assist hold (`indicators.rs`, `focus.rs`)
    Caps,
    Num,
    Bluetooth,
    Focus,
}

impl Flash {
    /// Every reaction the preview can play.
    pub const ALL: [Flash; 21] = [
        Flash::Copy,
        Flash::Kbd,
        Flash::Vdesk,
        Flash::Volume,
        Flash::Notif,
        Flash::Battery,
        Flash::Usb,
        Flash::Download,
        Flash::Reminder,
        Flash::Recording,
        Flash::Music,
        Flash::Privacy,
        Flash::Toast,
        Flash::Progress,
        Flash::Steam,
        Flash::Shelf,
        Flash::Agents,
        Flash::Phone,
        Flash::Calendar,
        Flash::Mail,
        Flash::Obs,
    ];

    /// The `live.*` switch that lets this reaction appear (`None` = no switch of its own).
    pub fn switch(self) -> Option<&'static str> {
        Some(match self {
            Flash::Copy => "live.copy",
            Flash::Kbd => "live.keyboardLayout",
            Flash::Vdesk => "live.virtualDesktopFlash",
            Flash::Volume => "live.volume",
            Flash::Notif => "live.notifications",
            Flash::Battery => "live.battery",
            Flash::Usb => "live.usb",
            Flash::Download => "live.dlDone",
            Flash::Reminder => "live.reminders",
            Flash::Recording => "live.recording",
            Flash::Music => "live.music",
            Flash::Privacy => "live.micCam",
            Flash::Progress => "live.downloads",
            Flash::Steam => "live.steam",
            Flash::Shelf => "live.shelf",
            Flash::Agents => "live.agents",
            Flash::Phone => "live.phone",
            Flash::Calendar => "live.calendar",
            Flash::Mail => "live.mail",
            Flash::Obs => "live.obs",
            Flash::Toast => return None,
            Flash::Caps => "live.capsLock",
            Flash::Num => "live.numLock",
            Flash::Bluetooth => "live.btBattery",
            Flash::Focus => "notify.respectFocus",
        })
    }

    fn title(self) -> Tr {
        match self {
            Flash::Copy => tr("Событие: копирование", "Event: copy"),
            Flash::Kbd => tr("Событие: раскладка клавиатуры", "Event: keyboard layout"),
            Flash::Vdesk => tr("Событие: рабочий стол", "Event: virtual desktop"),
            Flash::Volume => tr("Событие: громкость", "Event: volume"),
            Flash::Notif => tr("Событие: уведомление", "Event: notification"),
            Flash::Battery => tr("Событие: батарея", "Event: battery"),
            Flash::Usb => tr("Событие: USB-диск", "Event: USB drive"),
            Flash::Download => tr("Событие: загрузка готова", "Event: download finished"),
            Flash::Reminder => tr("Событие: напоминание", "Event: reminder"),
            Flash::Recording => tr("Событие: запись экрана", "Event: screen recording"),
            Flash::Music => tr("Событие: музыка", "Event: music"),
            Flash::Privacy => tr("Событие: микрофон и камера", "Event: microphone and camera"),
            Flash::Toast => tr("Карточка на острове", "A card on the island"),
            Flash::Progress => tr("Событие: загрузка идёт", "Event: download in progress"),
            Flash::Steam => tr("Событие: загрузка Steam", "Event: Steam download"),
            Flash::Shelf => tr("Событие: полка файлов", "Event: file shelf"),
            Flash::Agents => tr("Событие: ИИ-агенты", "Event: AI agents"),
            Flash::Phone => tr("Событие: сообщение с телефона", "Event: message from the phone"),
            Flash::Calendar => tr("Событие: календарь", "Event: calendar"),
            Flash::Mail => tr("Событие: почта", "Event: mail"),
            Flash::Obs => tr("Событие: запись в OBS", "Event: OBS recording"),
            Flash::Caps => tr("Индикатор: Caps Lock", "Indicator: Caps Lock"),
            Flash::Num => tr("Индикатор: Num Lock", "Indicator: Num Lock"),
            Flash::Bluetooth => tr("Индикатор: Bluetooth-устройство", "Indicator: Bluetooth device"),
            Flash::Focus => tr("«Фокусировка» Windows: карточка ждёт", "Windows Focus Assist: a card waits"),
        }
    }
}

/// What the folded island is looked at for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Island,
    Clock,
    Pet,
    Edge,
    Motion,
    /// where the island stands on the screen (the sketch of a monitor with its taskbar)
    Place,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// the folded island
    Pill(Look),
    /// the open island on a tab
    Panel(Tab),
    /// the folded island with one reaction
    Flash(Flash),
    /// the standalone music widget (and the island without a player), on the sketch of a monitor
    Widget,
}

impl Scene {
    pub const DEFAULT: Scene = Scene::Pill(Look::Island);

    /// The heading under the preview.
    pub fn title(self, lang: Lang) -> &'static str {
        match self {
            Scene::Pill(Look::Island) => tr("Свёрнутый остров", "Folded island"),
            Scene::Pill(Look::Clock) => tr("Свёрнутый остров: часы и значки", "Folded island: clock and icons"),
            Scene::Pill(Look::Pet) => tr("Питомец", "Pet"),
            Scene::Pill(Look::Edge) => tr("Край острова и персонаж", "Island edge and character"),
            Scene::Pill(Look::Motion) => tr("Анимация острова", "Island animation"),
            Scene::Pill(Look::Place) => tr("Место острова на экране", "Where the island stands on the screen"),
            Scene::Widget => tr("Виджет плеера", "Player widget"),
            Scene::Panel(Tab::Home) => tr("Открытый остров: главная и вкладки", "Open island: Home and tabs"),
            Scene::Panel(Tab::Weather) => tr("Вкладка «Погода»", "Weather tab"),
            Scene::Panel(Tab::Clipboard) => tr("Вкладка «Буфер»", "Clipboard tab"),
            Scene::Flash(f) => f.title(),
        }
        .get(lang)
    }

    /// The sentence under the heading. `off` = the switch behind this reaction is off, so on the real island nothing would appear.
    pub fn note(self, lang: Lang, off: bool) -> &'static str {
        if off {
            return tr("Сейчас выключено: на острове это не появится", "Switched off: this will not appear on the island").get(lang);
        }
        match self {
            Scene::Pill(Look::Motion) => tr("Нажмите «Повторить», чтобы увидеть движение ещё раз", "Press Replay to see the motion again"),
            Scene::Pill(Look::Place) => tr("Набросок монитора с панелью задач внизу: край, сторона и сдвиг вдоль края", "A sketch of a monitor with its taskbar at the bottom: the edge, the side and the shift along it"),
            Scene::Pill(_) => tr("Так выглядит свёрнутый остров с этими настройками", "This is the folded island with these settings"),
            Scene::Widget => tr("Пример трека: ваша музыка здесь не показывается. Остров без плеера, пока виджет включён", "A sample track: your own music is never shown here. The island has no player while the widget is on"),
            Scene::Panel(_) => tr("Пример данных: ваши данные здесь не показываются", "Sample data: your own data is never shown here"),
            Scene::Flash(_) => tr("Пример: так это появится на острове", "A sample of how it appears on the island"),
        }
        .get(lang)
    }

    /// Does the scene need the island folded (the reactions only happen then)?
    pub fn folded(self) -> bool {
        !matches!(self, Scene::Panel(_))
    }

    /// Is it shown on the sketch of a whole monitor (a fixed camera, the taskbar under the stage) and not close up on the island?
    pub fn screen(self) -> bool {
        matches!(self, Scene::Widget | Scene::Pill(Look::Place))
    }

    /// The switch whose being off means that nothing of this scene appears on the real island.
    pub fn switch(self) -> Option<&'static str> {
        match self {
            Scene::Flash(f) => f.switch(),
            Scene::Widget => Some("media.widget"),
            _ => None,
        }
    }
}

/// The scene for a setting that has just changed; `None` = this setting shows nothing here.
pub fn of_key(key: &str) -> Option<Scene> {
    use Scene::*;
    Some(match key {
        "shape" | "theme" | "island.color" | "island.colorCustom" | "size" | "opacityCol" | "opacityExp" | "language" | "edge.position" | "edge.align" | "a11y.contrast" => Pill(Look::Island),
        "offsetX" | "underTaskbar" => Pill(Look::Place),
        "media.widget" | "media.widget.edge" | "media.widget.align" | "media.widget.offset" => Widget,
        "showTime" | "showDate" | "showWxIcon" | "showTemp" | "showSeconds" | "clock24" | "showBattery" => Pill(Look::Clock),
        "pet" | "pet.side" | "pet.size" | "pet.keepDuringMusic" => Pill(Look::Pet),
        "character" | "character.liquid" => Pill(Look::Edge),
        "reduceMotion" | "fps" | "edgeIdleFps" => Pill(Look::Motion),
        "net.weather" | "weather.units" | "weather.city" => Panel(Tab::Weather),
        "clipClickPaste" | "clipboard.maxItems" | "clipboard.maxImageMb" | "clipboard.retentionDays" | "clipboard.skipSecrets" | "clipboard.denyList" => Panel(Tab::Clipboard),
        "home.lyrics" | "home.player" | "music.lyrics" | "audio.volumeTarget" | "tabs.hidden" => Panel(Tab::Home),
        "live.copy" => Flash(self::Flash::Copy),
        "live.keyboardLayout" | "live.keyboardLayoutAlways" => Flash(self::Flash::Kbd),
        "live.virtualDesktopFlash" | "live.virtualDesktop" => Flash(self::Flash::Vdesk),
        "live.volume" => Flash(self::Flash::Volume),
        "live.notifications" => Flash(self::Flash::Notif),
        "live.battery" => Flash(self::Flash::Battery),
        "live.usb" => Flash(self::Flash::Usb),
        "live.dlDone" => Flash(self::Flash::Download),
        "live.downloads" | "live.downloadRing" => Flash(self::Flash::Progress),
        "live.steam" => Flash(self::Flash::Steam),
        "live.shelf" => Flash(self::Flash::Shelf),
        "live.agents" | "agents.answerFromIsland" => Flash(self::Flash::Agents),
        "live.phone" => Flash(self::Flash::Phone),
        "live.calendar" => Flash(self::Flash::Calendar),
        "live.mail" => Flash(self::Flash::Mail),
        "live.obs" => Flash(self::Flash::Obs),
        "live.reminders" | "sound.reminders" | "sound.reminderTone" => Flash(self::Flash::Reminder),
        "live.recording" | "capture.format" | "capture.fps" | "capture.recordSystemAudio" | "capture.recordMic" => Flash(self::Flash::Recording),
        "live.music" | "musicPill" | "audio.reactive" => Flash(self::Flash::Music),
        "live.micCam" => Flash(self::Flash::Privacy),
        "live.capsLock" => Flash(self::Flash::Caps),
        "live.numLock" => Flash(self::Flash::Num),
        "live.btBattery" => Flash(self::Flash::Bluetooth),
        "notify.respectFocus" => Flash(self::Flash::Focus),
        "cards.duration" | "capture.shotAction" | "capture.revealAfterSave" => Flash(self::Flash::Toast),
        k if k.starts_with("module.") || hmod::is_home_key(k) => Panel(Tab::Home),
        _ => return None,
    })
}

/// The scene for a row that has just got the focus (Tab, a click): the same as when it changes, except the `module.*` switches, which only
/// say "a tab" and would pull the preview to Home from the middle of a page (editing one still does).
pub fn of_focus(key: &str) -> Option<Scene> {
    if key.starts_with("module.") || hmod::is_home_key(key) {
        None
    } else {
        of_key(key)
    }
}

/// The scene a page starts with when it is opened (the first function of the page; a row of another function brings its own, `of_key` / `of_focus`).
pub fn of_page(page: &str) -> Scene {
    match page {
        "look" => Scene::Pill(Look::Island),
        "edge" => Scene::Pill(Look::Edge),
        "tabs" | "tools" => Scene::Panel(Tab::Home),
        "weather" => Scene::Panel(Tab::Weather),
        "clipboard" => Scene::Panel(Tab::Clipboard),
        "media" => Scene::Flash(Flash::Music),
        "capture" => Scene::Flash(Flash::Toast),
        "notifications" => Scene::Flash(Flash::Notif),
        // General (start, hiding, quiet hours, search, the preview switch), Hotkeys, Calendar and mail, Phone link, AI, Updates: the folded island
        _ => Scene::DEFAULT,
    }
}

/// Settings the preview island must not see: they would hide it, mute it or point at the user's machine.
const BLOCKED: &[&str] = &[
    "autoHide", "autoHideKeepDesktop", "passThrough", "passThroughOpacity", "hideInFullscreen", "fullscreenPeek", "fullscreenPeekEdge", "hideInApps", "showInApps", "showInCapture", "monitor", "edge.monitor",
    "offsetX", "underTaskbar", "firstRunDone", "quiet.", "settings.",
];

/// Where the island and the music widget stand along their edge is the host's work (the window is placed on the monitor), not the view's: the
/// sandbox never hears these. The preview's own camera reads them from the window's settings and moves the island and the widget on the
/// sketch of the monitor (`Look::Place`, `Scene::Widget`).
pub const CAMERA: &[&str] = &["offsetX", "underTaskbar", "media.widget.offset"];

/// Free text that is safe to hand over: a colour. And the keys the Home and tab-strip chips write (`home.*`, `tabs.hidden`: switches and
/// a list of tab ids from the fixed set; no schema row owns them).
const TEXT_OK: &[&str] = &["island.colorCustom", "tabs.hidden"];

/// Does the preview show this setting at all: the sandbox island hears it or the camera reads it?
pub fn handled(key: &str) -> bool {
    shareable(key) || CAMERA.contains(&key)
}

/// May this setting go to the preview island? Only the switches, choices and sliders of the schema: no text a user typed (cities, folders,
/// addresses, user names), no device ids, no hotkeys, nothing the schema does not know.
pub fn shareable(key: &str) -> bool {
    if BLOCKED.iter().any(|b| key == *b || (b.ends_with('.') && key.starts_with(b))) {
        return false;
    }
    if TEXT_OK.contains(&key) || hmod::is_home_key(key) {
        return true;
    }
    let Some(row) = schema().row_for_key(key) else { return false };
    match &row.control {
        Control::Segmented { dynamic, .. } | Control::Select { dynamic, .. } => *dynamic == Dynamic::None,
        Control::Toggle | Control::Slider { .. } | Control::Number { .. } | Control::PetsPicker | Control::CharacterPicker | Control::Weekdays => true,
        _ => false,
    }
}
