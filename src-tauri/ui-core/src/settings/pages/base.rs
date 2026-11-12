//! Pages: Character, General, Appearance, Behavior (with Quiet hours), Hotkeys.
use super::leak;
use crate::settings::hotkey;
use crate::settings::schema::*;
use crate::settings::tr::{same, tr, Tr};
use serde_json::{json, Value};

pub fn character() -> Page {
    let mut p = page(
        "character",
        "character",
        tr("Персонаж", "Character"),
        vec![
            group(
                Tr::NONE,
                vec![custom("character.reactions", tr("Посмотреть реакцию:", "Preview a reaction:"), Tr::NONE, "character.reactions")
                    .kw(&["reaction", "preview", "реакция", "калм", "calm", "music", "музыка", "sleep", "сон"])],
            ),
            group(
                tr("Остров", "Island"),
                vec![Row::new("character", "character", tr("Остров", "Island"), Control::CharacterPicker, json!("liquid"))
                    .hint(tr("Стиль края применяется к острову сразу.", "The edge style applies to the island at once."))
                    .kw(&["style", "edge", "стиль", "край", "liquid", "жидкий", "tail", "хвост", "legs", "ножки", "glow", "свечение", "plain", "без украшений"]),
                    toggle("character.liquid", tr("Жидкий край под персонажем", "Liquid edge under the character"), tr("Для стилей «хвост» и «ножки»: они стоят на жидком крае острова, а не на ровной линии", "For the tail and legs styles: they stand on the island's liquid edge instead of a flat line"), true)
                        .enabled(Cond::Any(vec![Cond::Eq("character", json!("tail")), Cond::Eq("character", json!("legs"))]))
                        .kw(&["liquid", "жидкий", "edge", "край", "tail", "legs"])],
            ),
            group(
                tr("Питомец", "Pet"),
                vec![Row::new("pet", "pet", tr("Питомец", "Pet"), Control::PetsPicker, json!("cat"))
                    .hint(tr("Маленький персонаж живёт внутри острова", "A small character living inside the island"))
                    .kw(&["pet", "питомец", "кот", "cat", "персонаж", "character", "mascot", "талисман"])],
            ),
            group(
                Tr::NONE,
                vec![
                    seg("pet.side", tr("Сторона", "Side"), tr("С какого края острова стоит питомец", "Which end of the island the pet stands at"), so(&[("left", tr("Слева", "Left")), ("right", tr("Справа", "Right"))]), json!("right"))
                        .visible(Cond::Ne("pet", json!("none"))),
                    seg("pet.size", tr("Размер", "Size"), tr("Питомец следует за реакциями острова; щелчок по нему его гладит", "The pet follows the island reactions; click it to pet it"), so(&[("s", same("S")), ("m", same("M")), ("l", same("L")), ("xl", same("XL"))]), json!("m"))
                        .visible(Cond::Ne("pet", json!("none"))),
                    toggle("pet.keepDuringMusic", tr("Оставаться во время музыки", "Stay while music plays"), tr("Выключите, чтобы питомец уходил, пока играет музыка", "Turn off and the pet steps aside while music plays"), true)
                        .visible(Cond::Ne("pet", json!("none")))
                        .kw(&["music", "музыка", "pet", "питомец"]),
                ],
            ),
        ],
    );
    p.footnote = tr(
        "Стиль края применяется к острову сразу. Реакции включаются сами: агенты, музыка, файлы, напоминания; через 5 минут тишины остров засыпает.",
        "The edge style applies to the island at once. Reactions fire on their own: agents, music, files, reminders; after 5 quiet minutes the island falls asleep.",
    );
    p
}

pub fn general() -> Page {
    page(
        "general",
        "general",
        tr("Основное", "General"),
        vec![
            group(
                Tr::NONE,
                vec![
                    seg(
                        "language",
                        tr("Язык", "Language"),
                        tr("По умолчанию — как в Windows", "By default, the same as Windows"),
                        so(&[("auto", tr("Как в Windows", "Windows")), ("ru", same("Русский")), ("en", same("English"))]),
                        json!("auto"),
                    )
                    .kw(&["language", "язык", "русский", "english", "locale"]),
                    Row::new("general.autostart", "", tr("Запускать вместе с Windows", "Start with Windows"), Control::BackendToggle(BackendToggle::Autostart), Value::Null)
                        .kw(&["autostart", "startup", "автозапуск", "автозагрузка", "запуск", "boot"]),
                    toggle(
                        "settings.preview",
                        tr("Живое превью справа", "Live preview on the right"),
                        tr("Показывает остров с вашими настройками, пока вы их меняете. Нужно окно пошире", "Shows the island with your settings while you change them. Needs a wider window"),
                        true,
                    )
                    .kw(&["preview", "live", "превью", "предпросмотр", "просмотр", "образец"]),
                    button("general.wizard", tr("Мастер первого запуска", "First-run wizard"), tr("Короткая настройка: язык, модули, клавиши", "A short setup: language, modules, hotkeys"), tr("Запустить снова", "Run again"), Action::RunWizard, ButtonStyle::Small)
                        .kw(&["wizard", "setup", "welcome", "мастер", "приветствие", "первый запуск"]),
                ],
            ),
            group(
                tr("Копия настроек", "Settings backup"),
                vec![custom(
                    "general.backup",
                    tr("Резервная копия настроек", "Settings backup"),
                    tr("Экспорт кладёт файл в «Загрузки». Ключи и пароли в него не попадают", "Export puts a file in Downloads. Keys and passwords are not included"),
                    "general.backup",
                )
                .kw(&["export", "import", "backup", "экспорт", "импорт", "копия", "резервная"])],
            ),
        ],
    )
}

/// Options of the "Island colour" select: the theme's own, then the nine ready colours (their hex is the stored value).
fn island_colors() -> Vec<Opt> {
    let mut v = vec![opt(json!(""), tr("По теме", "From the theme"))];
    v.extend(crate::shell::tint::PRESETS.iter().map(|p| opt(json!(p.hex), p.name)));
    v
}

pub fn appearance() -> Page {
    let mut monitor = seg("monitor", tr("Монитор", "Monitor"), Tr::NONE, so(&[("primary", tr("Основной", "Primary"))]), json!("primary"));
    if let Control::Segmented { dynamic, .. } = &mut monitor.control {
        *dynamic = Dynamic::Monitors;
    }
    let fps_label = |v: i64| if v == 0 { tr("Авто", "Auto") } else { same(Box::leak(v.to_string().into_boxed_str())) };
    let fps = |vals: &[i64]| -> Vec<Opt> { vals.iter().map(|v| opt(if *v == 0 { json!("auto") } else { json!(v) }, fps_label(*v))).collect() };
    page(
        "appearance",
        "appearance",
        tr("Внешний вид", "Appearance"),
        vec![
            group(
                Tr::NONE,
                vec![
                    seg("shape", tr("Форма острова", "Island shape"), Tr::NONE, so(&[("notch", tr("Вырез", "Notch")), ("capsule", tr("Капсула", "Capsule"))]), json!("notch")).kw(&["shape", "форма", "notch", "capsule"]),
                    seg(
                        "theme",
                        tr("Тема острова", "Island theme"),
                        tr("Цвета самого острова. Тему Windows переключает центр управления", "The colours of the pill itself. The Windows theme is switched by the control centre"),
                        so(&[("dark", tr("Тёмная", "Dark")), ("light", tr("Светлая", "Light")), ("auto", tr("Как в Windows", "Windows"))]),
                        json!("dark"),
                    )
                    .kw(&["theme", "тема", "dark", "light", "тёмная", "светлая", "цвет"]),
                    select(
                        "island.color",
                        tr("Цвет острова", "Island colour"),
                        tr("Готовые цвета. Текст сам становится светлым или тёмным под цвет. «По теме» — чёрный или светлый, как в теме выше", "Ready colours. The text turns light or dark to suit the colour. “From the theme” is black or light, as the theme above says"),
                        island_colors(),
                        json!(""),
                    )
                    .kw(&["color", "colour", "island", "цвет", "остров", "фон", "background"]),
                    text(
                        "island.colorCustom",
                        tr("Свой цвет острова", "Custom island colour"),
                        tr("Любой цвет в виде #RRGGBB, например #3a2a5c. Если задан, он главнее готового цвета выше; пустое поле его убирает", "Any colour as #RRGGBB, for example #3a2a5c. When set, it beats the ready colour above; an empty field removes it"),
                        TextKind::Plain,
                        same("#RRGGBB"),
                        "",
                    )
                    .kw(&["color", "colour", "hex", "custom", "цвет", "свой"]),
                    slider("size", tr("Размер острова", "Island size"), tr("На большом мониторе остров можно сделать крупнее", "On a large monitor the pill can be made bigger"), 80.0, 200.0, 5.0, Unit::Percent, 100.0).kw(&["size", "scale", "размер", "масштаб", "крупнее"]),
                    slider("opacityCol", tr("Прозрачность свёрнутого острова", "Transparency of the folded island"), tr("Пилюля и её карточки", "The pill and its cards"), 0.0, 60.0, 5.0, Unit::Percent, 0.0).kw(&["opacity", "transparency", "прозрачность"]),
                    slider("opacityExp", tr("Прозрачность раскрытого острова", "Transparency of the open island"), Tr::NONE, 0.0, 60.0, 5.0, Unit::Percent, 0.0).kw(&["opacity", "transparency", "прозрачность"]),
                    slider(
                        "offsetX",
                        tr("Положение вдоль верхнего края", "Position along the top edge"),
                        tr(
                            "0 — посередине, минус — левее, плюс — правее. Свёрнутый остров стоит над серединой окна, раскрытый может дойти до края монитора",
                            "0 is the middle, minus is further left, plus further right. The folded pill stays above the middle; the open one can reach the monitor edge",
                        ),
                        -50.0,
                        50.0,
                        1.0,
                        Unit::Percent,
                        0.0,
                    )
                    .kw(&["position", "offset", "положение", "смещение", "влево", "вправо"]),
                    toggle("underTaskbar", tr("Под панелью задач, если она сверху", "Below the taskbar if it is at the top"), tr("Остров встаёт под панель, а не поверх её кнопок", "The pill sits below the bar, not over its buttons"), true).kw(&["taskbar", "панель задач"]),
                    monitor.kw(&["monitor", "display", "screen", "монитор", "экран"]),
                ],
            ),
            group(
                tr("Свёрнутый остров", "Folded island"),
                vec![
                    toggle("showTime", tr("Время", "Time"), tr("Пока на острове ничего не происходит", "While nothing else is happening on the pill"), true).kw(&["clock", "часы"]),
                    toggle("showDate", tr("Дата", "Date"), tr("Рядом со временем", "Next to the time"), false),
                    toggle("showWxIcon", tr("Значок погоды", "Weather icon"), tr("Слева от времени. Нужна включённая погода", "Left of the time. Needs the weather switched on"), true).enabled(Cond::On("net.weather")),
                    toggle("showTemp", tr("Температура", "Temperature"), tr("Число справа от времени. Нужна включённая погода", "The number right of the time. Needs the weather switched on"), true).enabled(Cond::On("net.weather")),
                    toggle("showSeconds", tr("Секунды", "Seconds"), Tr::NONE, false),
                    seg("clock24", tr("Формат часов", "Clock format"), Tr::NONE, vec![opt(json!(false), same("12 h")), opt(json!(true), same("24 h"))], json!(true)).kw(&["12", "24", "clock format", "формат времени"]),
                    toggle("showBattery", tr("Заряд батареи", "Battery charge"), tr("На компьютере с батареей", "On a computer that has a battery"), true),
                    toggle("reduceMotion", tr("Уменьшить анимацию", "Reduce motion"), Tr::NONE, false).kw(&["animation", "motion", "анимация", "движение", "accessibility", "reduced"]),
                ],
            ),
            group(
                tr("Плавность", "Smoothness"),
                vec![
                    seg(
                        "fps",
                        tr("Частота кадров", "Frame rate"),
                        tr("Авто = частота монитора. Больше кадров — плавнее, но выше нагрузка на видеокарту", "Auto = the monitor refresh rate. More frames are smoother but cost more GPU"),
                        fps(&[0, 30, 60, 120, 144, 240]),
                        json!("auto"),
                    )
                    .kw(&["fps", "smooth", "плавность", "кадры", "refresh"]),
                    seg(
                        "edgeIdleFps",
                        tr("Кадры в покое", "Idle edge frame rate"),
                        tr("Для спокойного края острова, когда ничего не происходит", "For the calm island edge while nothing is happening"),
                        fps(&[15, 30, 60]),
                        json!(30),
                    )
                    .kw(&["fps", "idle", "покой"]),
                    toggle(
                        "ui.gpu",
                        tr("Ускорение видеокартой", "GPU acceleration"),
                        tr(
                            "Выключите, если остров мерцает, пропадает или видеодрайвер зависает: рисование перейдёт на процессор. Применится после перезапуска Pill",
                            "Turn off if the pill flickers, disappears or the graphics driver hangs: drawing moves to the processor. Applied after Pill restarts",
                        ),
                        true,
                    )
                    .restart()
                    .kw(&["gpu", "hardware", "software", "warp", "render", "видеокарта", "ускорение", "процессор", "перезапуск"]),
                ],
            ),
        ],
    )
}

pub fn behavior() -> Page {
    let days = |v: &[&str]| Value::Array(v.iter().map(|x| json!(x)).collect());
    page(
        "behavior",
        "behavior",
        tr("Поведение", "Behavior"),
        vec![
            group(
                Tr::NONE,
                vec![
                    toggle("hideInFullscreen", tr("Прятать в полноэкранных приложениях", "Hide in full-screen apps"), tr("Игры, видео на весь экран, презентации", "Games, full-screen video, presentations"), true).kw(&["fullscreen", "game", "игры", "полноэкранный"]),
                    toggle(
                        "fullscreenPeek",
                        tr("Показывать остров поверх полноэкранных приложений по своей клавише", "Show the pill over full-screen apps with its own key"),
                        tr(
                            "Клавиша острова открывает его поверх игры или видео на весь экран. Остров прячется снова, когда закроется. Не действует в программах из списка «Прятать остров»",
                            "The pill's key opens it over a full-screen game or video. It hides again when it folds. Does not work in programs of the “Hide the pill” list",
                        ),
                        true,
                    )
                    .enabled(Cond::On("hideInFullscreen"))
                    .kw(&["fullscreen", "hotkey", "game", "поверх", "полноэкранный", "клавиша"]),
                    toggle(
                        "fullscreenPeekEdge",
                        tr("Открывать остров у верхнего края в полноэкранных приложениях", "Open the pill at the top edge in full-screen apps"),
                        tr(
                            "Задержите курсор на верхней кромке экрана на долю секунды. Выключено по умолчанию: в играх с прокруткой у края остров мог бы выскакивать сам",
                            "Rest the cursor on the top edge of the screen for a moment. Off by default: in games that scroll at the edge the pill could pop up by itself",
                        ),
                        false,
                    )
                    .enabled(Cond::All(vec![Cond::On("hideInFullscreen"), Cond::On("fullscreenPeek")]))
                    .kw(&["fullscreen", "edge", "cursor", "край", "курсор", "полноэкранный"]),
                    toggle(
                        "autoHide",
                        tr("Прятать остров, пока он не нужен", "Hide the pill until it is needed"),
                        tr(
                            "Свёрнутый остров прячется за край экрана и не закрывает вкладки и кнопки окон. Подведите курсор к верхнему краю, и он вернётся. С карточкой, записью или диктовкой выходит сам",
                            "The folded pill hides behind the screen edge and stops covering tabs and window buttons. Move the cursor to the top edge to bring it back. It comes out by itself for a card, recording or dictation",
                        ),
                        false,
                    )
                    .kw(&["auto-hide", "autohide", "автоскрытие", "спрятать", "hide"]),
                    toggle(
                        "autoHideKeepDesktop",
                        tr("Не прятать на рабочем столе", "Do not hide on the desktop"),
                        tr(
                            "Для автоскрытия: пока впереди сам рабочий стол, без окон программ, остров остаётся на виду",
                            "For auto-hide: while the desktop itself is in front, with no program windows, the pill stays in view",
                        ),
                        false,
                    )
                    .enabled(Cond::On("autoHide")),
                    Row::new(
                        "hideInApps",
                        "hideInApps",
                        tr("Прятать остров в этих программах", "Hide the pill in these programs"),
                        Control::Lines { rows: 3, mono: false, placeholder: Tr::NONE, exe_suffix: false },
                        json!([]),
                    )
                    .hint(tr(
                        "По одной на строку, имя процесса: photoshop.exe, premiere.exe. Пока такое окно впереди, остров скрыт полностью.",
                        "One per line, the process name: photoshop.exe, premiere.exe. While such a window is in front, the pill is hidden completely.",
                    ))
                    .kw(&["apps", "programs", "exclude", "программы", "исключения", "процесс"])
                    .block(),
                    button(
                        "behavior.hideAddActive",
                        tr("Добавить активную программу в список «Прятать»", "Add the active program to the “Hide” list"),
                        tr("Берёт программу, которая была впереди, пока вы не открыли настройки, и дописывает её в список выше", "Takes the program that was in front before you opened the settings and adds it to the list above"),
                        tr("Добавить", "Add"),
                        Action::HideAddActive,
                        ButtonStyle::Small,
                    )
                    .kw(&["active", "current", "add", "активная", "текущая", "добавить"]),
                    Row::new(
                        "showInApps",
                        "showInApps",
                        tr("Не прятать остров в этих программах", "Keep the pill visible in these programs"),
                        Control::Lines { rows: 3, mono: false, placeholder: Tr::NONE, exe_suffix: false },
                        json!([]),
                    )
                    .hint(tr(
                        "По одной на строку, имя процесса: code.exe, obs64.exe. Пока такое окно впереди, остров остаётся на виду: не прячется ни в полноэкранном режиме, ни автоскрытием.",
                        "One per line, the process name: code.exe, obs64.exe. While such a window is in front, the pill stays in view: full-screen mode and auto-hide do not hide it.",
                    ))
                    .kw(&["apps", "programs", "always", "visible", "exceptions", "программы", "исключения", "процесс", "всегда"])
                    .block(),
                    button(
                        "behavior.showAddActive",
                        tr("Добавить активную программу в список «Не прятать»", "Add the active program to the “Keep visible” list"),
                        tr("Берёт программу, которая была впереди, пока вы не открыли настройки, и дописывает её в список выше", "Takes the program that was in front before you opened the settings and adds it to the list above"),
                        tr("Добавить", "Add"),
                        Action::ShowAddActive,
                        ButtonStyle::Small,
                    )
                    .kw(&["active", "current", "add", "активная", "текущая", "добавить"]),
                ],
            ),
            group(
                tr("Сквозной режим", "Pass-through mode"),
                vec![
                    toggle(
                        "passThrough",
                        tr("Сквозной режим", "Pass-through mode"),
                        tr(
                            "Свёрнутый остров пропускает клики и не раскрывается под курсором. Открывается клавишами. Включить: здесь, своей клавишей или командой «сквозной»",
                            "The folded pill lets clicks through and does not open under the cursor. It opens with keys. Turn it on here, with its own key or the “pass-through” command",
                        ),
                        false,
                    )
                    .kw(&["click-through", "клики", "сквозной"]),
                    slider("passThroughOpacity", tr("Прозрачность в сквозном режиме", "Transparency in pass-through mode"), Tr::NONE, 0.0, 80.0, 5.0, Unit::Percent, 50.0).enabled(Cond::On("passThrough")),
                ],
            ),
            group(
                tr("Скриншоты и запись экрана", "Screenshots and screen recording"),
                vec![toggle(
                    "showInCapture",
                    tr("Показывать остров на скриншотах и в записи экрана", "Show the pill in screenshots and screen recordings"),
                    tr(
                        "Обычно остров не попадает в скриншоты, запись и трансляцию. Включите, если фоновая запись (например, NVIDIA) жалуется, что остров ей мешает",
                        "Normally the pill is left out of screenshots, recordings and streams. Turn it on if a background recorder (NVIDIA, for example) complains about it",
                    ),
                    true,
                )
                .kw(&["capture", "nvidia", "stream", "трансляция", "скриншот"])],
            ),
            group(
                tr("Тихие часы", "Quiet hours"),
                vec![
                    toggle(
                        "quiet.on",
                        tr("Тихие часы", "Quiet hours"),
                        tr(
                            "Пока тихо, карточки событий, вспышки агентов и импульсы не показываются и не копятся. Ваши таймеры, напоминания и срочное звучат всегда. «Фокусировку» Windows Pill не читает.",
                            "While it is quiet, event cards, agent flashes and pulses are not shown or queued. Your timers, reminders and anything urgent still ring. Pill does not read Windows Focus Assist.",
                        ),
                        false,
                    )
                    .kw(&["quiet", "do not disturb", "dnd", "silence", "тишина", "не беспокоить", "ночь", "night"]),
                    Row::new("quiet.from", "quiet.from", tr("С какого часа", "From"), Control::Time, json!("22:00")).enabled(Cond::On("quiet.on")),
                    Row::new("quiet.to", "quiet.to", tr("До какого часа", "Until"), Control::Time, json!("07:00"))
                        .hint(tr("Можно через полночь: например, с 22:00 до 07:00", "May cross midnight: for example 22:00 to 07:00"))
                        .enabled(Cond::On("quiet.on")),
                    Row::new("quiet.days", "quiet.days", tr("Дни", "Days"), Control::Weekdays, days(&["0", "1", "2", "3", "4", "5", "6"]))
                        .hint(tr("Окно, начавшееся в этот день, тянется и за полночь", "A window that starts on a day runs on past midnight"))
                        .enabled(Cond::On("quiet.on"))
                        .kw(&["weekdays", "days", "дни", "понедельник", "monday"])
                        .block(),
                ],
            ),
        ],
    )
}

pub fn hotkeys() -> Page {
    let rows: Vec<Row> = hotkey::ACTIONS
        .iter()
        .map(|a| {
            let key = leak(hotkey::key_of(a.id));
            let mut r = Row::new(key, key, a.title, Control::Hotkey { action: a.id }, json!(a.default)).hint(a.hint);
            r.search_keywords = &["hotkey", "shortcut", "горячая клавиша", "сочетание", "клавиши"];
            r
        })
        .collect();
    let mut p = page(
        "hotkeys",
        "hotkeys",
        tr("Клавиши", "Hotkeys"),
        vec![
            group(Tr::NONE, rows),
            group(
                tr("Ярлыки лаунчера", "Launcher shortcuts"),
                vec![custom("hotkeys.launcher", tr("Ярлыки лаунчера", "Launcher shortcuts"), hotkey::LAUNCHER_FOOTNOTE, "hotkeys.launcher").kw(&["launcher", "лаунчер", "программы", "apps"])],
            ),
        ],
    );
    p.head = tr("Горячие клавиши", "Hotkeys");
    p
}
