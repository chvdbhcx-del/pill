//! What the preview shows for a setting: a lookup from the settings.json key (and, for a page opened without a change, the page) to a
//! [`Scene`], and the caption that says what is on show. Keyed by the settings key and not by row id or page, so a regrouping of the rows
//! never breaks it. A key that is not here keeps the scene that was on (the default island on a fresh window).
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
}

impl Flash {
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
            Flash::Toast => return None,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene {
    /// the folded island
    Pill(Look),
    /// the open island on a tab
    Panel(Tab),
    /// the folded island with one reaction
    Flash(Flash),
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
            Scene::Pill(_) => tr("Так выглядит свёрнутый остров с этими настройками", "This is the folded island with these settings"),
            Scene::Panel(_) => tr("Пример данных: ваши данные здесь не показываются", "Sample data: your own data is never shown here"),
            Scene::Flash(_) => tr("Пример: так это появится на острове", "A sample of how it appears on the island"),
        }
        .get(lang)
    }

    /// Does the scene need the island folded (the reactions only happen then)?
    pub fn folded(self) -> bool {
        !matches!(self, Scene::Panel(_))
    }
}

/// The scene for a setting that has just changed; `None` = this setting shows nothing here.
pub fn of_key(key: &str) -> Option<Scene> {
    use Scene::*;
    Some(match key {
        "shape" | "theme" | "island.color" | "island.colorCustom" | "size" | "opacityCol" | "opacityExp" | "language" => Pill(Look::Island),
        "showTime" | "showDate" | "showWxIcon" | "showTemp" | "showSeconds" | "clock24" | "showBattery" => Pill(Look::Clock),
        "pet" | "pet.side" | "pet.size" | "pet.keepDuringMusic" => Pill(Look::Pet),
        "character" | "character.liquid" => Pill(Look::Edge),
        "reduceMotion" | "fps" | "edgeIdleFps" => Pill(Look::Motion),
        "net.weather" | "weather.units" | "weather.city" => Panel(Tab::Weather),
        "clipClickPaste" | "clipboard.maxItems" | "clipboard.maxImageMb" | "clipboard.retentionDays" | "clipboard.skipSecrets" | "clipboard.denyList" => Panel(Tab::Clipboard),
        "home.lyrics" | "home.player" | "music.lyrics" => Panel(Tab::Home),
        "live.copy" => Flash(self::Flash::Copy),
        "live.keyboardLayout" | "live.keyboardLayoutAlways" => Flash(self::Flash::Kbd),
        "live.virtualDesktopFlash" | "live.virtualDesktop" => Flash(self::Flash::Vdesk),
        "live.volume" => Flash(self::Flash::Volume),
        "live.notifications" => Flash(self::Flash::Notif),
        "live.battery" => Flash(self::Flash::Battery),
        "live.usb" => Flash(self::Flash::Usb),
        "live.dlDone" => Flash(self::Flash::Download),
        "live.reminders" | "sound.reminders" | "sound.reminderTone" => Flash(self::Flash::Reminder),
        "live.recording" => Flash(self::Flash::Recording),
        "live.music" | "musicPill" | "audio.reactive" => Flash(self::Flash::Music),
        "live.micCam" => Flash(self::Flash::Privacy),
        "cards.duration" | "capture.shotAction" | "capture.revealAfterSave" => Flash(self::Flash::Toast),
        k if k.starts_with("module.") => Panel(Tab::Home),
        _ => return None,
    })
}

/// The scene a page starts with when it is opened.
pub fn of_page(page: &str) -> Scene {
    match page {
        "look" => Scene::Pill(Look::Island),
        "edge" => Scene::Pill(Look::Edge),
        "pet" => Scene::Pill(Look::Pet),
        "clock" => Scene::Pill(Look::Clock),
        "animation" => Scene::Pill(Look::Motion),
        "tabs" | "launcher" | "shelf" | "text" | "controls" | "vpn" => Scene::Panel(Tab::Home),
        "weather" => Scene::Panel(Tab::Weather),
        "clipboard" => Scene::Panel(Tab::Clipboard),
        "media" => Scene::Flash(Flash::Music),
        "volume" => Scene::Flash(Flash::Volume),
        "screenshots" => Scene::Flash(Flash::Toast),
        "recording" => Scene::Flash(Flash::Recording),
        "notes" => Scene::Flash(Flash::Reminder),
        "notifications" => Scene::Flash(Flash::Notif),
        "downloads" => Scene::Flash(Flash::Download),
        "battery" => Scene::Flash(Flash::Battery),
        "keyboard" => Scene::Flash(Flash::Kbd),
        "desktops" => Scene::Flash(Flash::Vdesk),
        "usb" => Scene::Flash(Flash::Usb),
        // Start, Hiding, Quiet hours, Hotkeys, Command bar, Voice to text, OBS, Calendar, Mail, Phone link, the AI pages, Updates: the folded island
        _ => Scene::DEFAULT,
    }
}

/// Settings the preview island must not see: they would hide it, mute it or point at the user's machine.
const BLOCKED: &[&str] = &[
    "autoHide", "autoHideKeepDesktop", "passThrough", "passThroughOpacity", "hideInFullscreen", "fullscreenPeek", "fullscreenPeekEdge", "hideInApps", "showInApps", "showInCapture", "monitor",
    "offsetX", "underTaskbar", "firstRunDone", "quiet.", "settings.",
];

/// Free text that is safe to hand over: a colour.
const TEXT_OK: &[&str] = &["island.colorCustom"];

/// May this setting go to the preview island? Only the switches, choices and sliders of the schema: no text a user typed (cities, folders,
/// addresses, user names), no device ids, no hotkeys, nothing the schema does not know.
pub fn shareable(key: &str) -> bool {
    if BLOCKED.iter().any(|b| key == *b || (b.ends_with('.') && key.starts_with(b))) {
        return false;
    }
    if TEXT_OK.contains(&key) {
        return true;
    }
    let Some(row) = schema().row_for_key(key) else { return false };
    match &row.control {
        Control::Segmented { dynamic, .. } | Control::Select { dynamic, .. } => *dynamic == Dynamic::None,
        Control::Toggle | Control::Slider { .. } | Control::Number { .. } | Control::PetsPicker | Control::CharacterPicker | Control::Weekdays => true,
        _ => false,
    }
}
