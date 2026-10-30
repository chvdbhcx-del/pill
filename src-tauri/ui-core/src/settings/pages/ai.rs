//! Pages: AI, AI setup, Voice to text, Updates. The provider chips, model lists, Whisper/Ollama installers and status lines are
//! `Control::Custom` widgets (their data and texts: `providers.rs`, `flows.rs`); every key they write is a real row key.
use crate::settings::schema::*;
use crate::settings::tr::{same, tr, Tr};
use serde_json::json;

pub fn ai() -> Page {
    page(
        "ai",
        "ai",
        tr("ИИ", "AI"),
        vec![group(
            Tr::NONE,
            vec![
                toggle("net.ai", tr("ИИ-помощник", "AI assistant"), tr("Выключен по умолчанию. Запросы уходят только на адрес ниже и только когда ты пишешь в чате или в командной строке.", "Off by default. Requests go only to the address below, and only when you write in the chat or the command bar."), false)
                    .kw(&["ai", "chat", "assistant", "llm", "ии", "чат", "помощник", "network", "сеть"]),
                Row::new("ai.provider", "ai.provider", tr("Сервис ИИ", "AI service"), Control::Custom("ai.providers"), json!("custom"))
                    .hint(tr("Ollama, FreeLLMAPI, Groq, Google Gemini, OpenRouter, DeepSeek или свой адрес", "Ollama, FreeLLMAPI, Groq, Google Gemini, OpenRouter, DeepSeek or your own address"))
                    .kw(&["provider", "ollama", "groq", "gemini", "openrouter", "deepseek", "freellmapi", "провайдер", "сервис"]),
                secret(
                    "ai.key",
                    tr("Ключ API", "API key"),
                    tr("Ключ выбранного сервиса. Хранится только на этом компьютере, зашифрован Windows, не вшит в приложение и никому не передаётся.", "The key of the chosen service. Stored only on this computer, encrypted by Windows; it is not built into the app and not shared with anyone."),
                    tr("не нужен для локальных серверов", "not needed for local servers"),
                    Action::AiSetKey,
                )
                .kw(&["api key", "key", "ключ", "token", "секрет"]),
                Row::new(
                    "ai.baseUrl",
                    "ai.baseUrl",
                    tr("Адрес API", "API address"),
                    Control::Text { secret: false, kind: TextKind::Url, placeholder: same("http://127.0.0.1:11434/v1"), mono: false, commit: Commit::Settings },
                    json!(""),
                )
                .kw(&["address", "url", "endpoint", "адрес"]),
                Row::new(
                    "ai.model",
                    "ai.model",
                    tr("Модель", "Model"),
                    Control::Text { secret: false, kind: TextKind::Plain, placeholder: tr("имя модели", "model name"), mono: true, commit: Commit::Settings },
                    json!(""),
                )
                .kw(&["model", "модель"]),
                button("ai.list", tr("Список", "List"), Tr::NONE, tr("Список", "List"), Action::AiList, ButtonStyle::Normal),
                button("ai.test", tr("Проверить", "Test"), Tr::NONE, tr("Проверить", "Test"), Action::AiTest, ButtonStyle::Normal),
            ],
        )],
    )
}

pub fn aisetup() -> Page {
    page(
        "aisetup",
        "aisetup",
        tr("Установка ИИ", "AI setup"),
        vec![
            group(
                Tr::NONE,
                vec![custom(
                    "aisetup.intro",
                    tr("Установка ИИ", "AI setup"),
                    tr("Голос в текст и ИИ-чат можно скачать на этот компьютер или подключить облачный сервис. Ничего не скачивается и не запускается без вашего нажатия; у кнопки указано, откуда и сколько.", "Voice to text and the AI chat can run on this computer or through a cloud service. Nothing is downloaded or started without your click; the button shows the source and size."),
                    "aisetup.intro",
                )
                .kw(&["install", "download", "установка", "скачать", "whisper", "ollama", "local model", "локальная модель"])],
            ),
            group(
                tr("Голос в текст (Whisper)", "Voice to text (Whisper)"),
                vec![
                    custom("aisetup.whisper", tr("Локально, на этом компьютере", "Locally, on this computer"), tr("Pill запускает whisper.cpp скрыто, только на 127.0.0.1, и закрывает его вместе с собой. Запись не покидает компьютер, ключ не нужен.", "Pill runs whisper.cpp hidden, on 127.0.0.1 only, and closes it with itself. The recording stays on this computer and no key is needed."), "aisetup.whisper")
                        .kw(&["whisper", "voice", "dictation", "голос", "диктовка", "speech"]),
                    button("aisetup.cloud-voice", tr("Через облако", "Through the cloud"), tr("Groq и Gemini бесплатны, OpenAI платный. Нужен ключ API; запись уходит на сервер выбранного сервиса только когда вы закончили диктовку.", "Groq and Gemini are free, OpenAI is paid. An API key is needed; the recording goes to the chosen service only when you finish dictating."), tr("Выбрать сервис и ключ", "Choose service and key"), Action::OpenPage("voice"), ButtonStyle::Normal),
                ],
            ),
            group(
                tr("ИИ-чат", "AI chat"),
                vec![
                    custom("aisetup.chat", tr("Локальная модель", "Local model"), tr("Запущенных локальных серверов (Ollama, LM Studio, llama.cpp, FreeLLMAPI) не найдено. Запросы к ним идут только на 127.0.0.1.", "No local servers (Ollama, LM Studio, llama.cpp, FreeLLMAPI) found. Requests to them go to 127.0.0.1 only."), "aisetup.chat")
                        .kw(&["ollama", "lm studio", "llama.cpp", "model", "модель"]),
                    button("aisetup.cloud-chat", tr("Через облако", "Through the cloud"), tr("Любой OpenAI-совместимый сервис (OpenAI, OpenRouter, Groq, свой сервер). Нужны адрес API и ключ; ключ шифруется Windows и хранится только здесь.", "Any OpenAI-compatible service (OpenAI, OpenRouter, Groq, your own server). An API address and a key are needed; the key is encrypted by Windows and stays here."), tr("Настроить облачный ИИ", "Set up a cloud AI"), Action::OpenPage("ai"), ButtonStyle::Normal),
                ],
            ),
            group(
                Tr::NONE,
                vec![Row::new(
                    "aiSetupDone",
                    "aiSetupDone",
                    tr("Настрою позже", "I will set this up later"),
                    Control::Button { label: tr("Позже", "Later"), action: Action::AiSetupLater, style: ButtonStyle::Normal },
                    json!(false),
                )
                .hint(tr("Страница остаётся в настройках, в разделе «Установка ИИ»: вернуться можно в любой момент.", "The page stays in Settings under “AI setup”: you can come back at any time."))],
            ),
        ],
    )
}

pub fn voice() -> Page {
    page(
        "voice",
        "voice",
        tr("Голос в текст", "Voice to text"),
        vec![
            group(
                Tr::NONE,
                vec![
                    toggle(
                        "voice.enabled",
                        tr("Голос в текст", "Voice to text"),
                        tr("Выключен по умолчанию. Включённый, он слушает микрофон только пока идёт диктовка: горячая клавиша (страница «Клавиши»), скажи, затем нажми ещё раз или отпусти клавишу (режим ниже; Esc тоже останавливает): текст окажется в буфере. Запись уходит только на адрес ниже и только когда ты её закончил.", "Off by default. When on, it listens to the microphone only while you dictate: the hotkey (Hotkeys page), speak, then press again or release the key (mode below; Esc stops too): the text lands on the clipboard. The recording goes only to the address below, and only when you finish it."),
                        false,
                    )
                    .kw(&["voice", "dictation", "speech", "голос", "диктовка", "микрофон", "network", "сеть"]),
                    seg(
                        "voice.mode",
                        tr("Режим горячей клавиши", "Hotkey mode"),
                        tr("Удерживание короче 0,35 с считается случайным нажатием: запись отменяется без ошибки и ничего не отправляется", "A hold shorter than 0.35 s counts as an accidental tap: the recording is dropped silently and nothing is sent"),
                        so(&[("toggle", tr("Нажать и говорить, нажать ещё раз", "Press, speak, press again")), ("hold", tr("Удерживать, пока говорю", "Hold while speaking"))]),
                        json!("toggle"),
                    )
                    .enabled(Cond::On("voice.enabled")),
                    Row::new("voice.provider", "voice.provider", tr("Сервис распознавания", "Recognition service"), Control::Custom("voice.providers"), json!("groq"))
                        .hint(tr("Groq · Whisper, Google Gemini, OpenAI или свой сервер", "Groq · Whisper, Google Gemini, OpenAI or your own server"))
                        .kw(&["provider", "groq", "whisper", "gemini", "openai", "сервис"]),
                    secret(
                        "voice.key",
                        tr("Ключ API", "API key"),
                        tr("Хранится только на этом компьютере, зашифрован Windows и никому не передаётся. Если для этого адреса уже есть ключ ИИ-помощника, он подойдёт и здесь.", "Stored only on this computer, encrypted by Windows and not shared with anyone. If the AI assistant already has a key for this address, it is used here too."),
                        tr("не нужен для локальных серверов", "not needed for local servers"),
                        Action::VoiceSetKey,
                    ),
                    Row::new(
                        "voice.endpoint",
                        "voice.endpoint",
                        tr("Адрес API", "API address"),
                        Control::Text { secret: false, kind: TextKind::Url, placeholder: same("https://api.groq.com/openai/v1"), mono: false, commit: Commit::Settings },
                        json!("https://api.groq.com/openai/v1"),
                    ),
                    Row::new(
                        "voice.model",
                        "voice.model",
                        tr("Модель", "Model"),
                        Control::Text { secret: false, kind: TextKind::Plain, placeholder: tr("имя модели", "model name"), mono: true, commit: Commit::Settings },
                        json!(""),
                    ),
                    seg("voice.language", tr("Язык речи", "Spoken language"), tr("Подсказка сервису: так он реже путает языки", "A hint for the service: it mixes languages up less often"), so(&[("auto", tr("Авто", "Detect")), ("ru", same("Русский")), ("en", same("English"))]), json!("auto")).kw(&["language", "язык"]),
                    toggle("voice.paste", tr("Вставлять текст сразу", "Paste the text at once"), tr("В окно, где стоял курсор. Текст остаётся в буфере в любом случае. Не работает в окнах, запущенных от администратора", "Into the window where the caret was. The text stays on the clipboard either way. It does not work in windows run as administrator"), false),
                    toggle("voice.autostop", tr("Останавливать при тишине", "Stop on silence"), tr("Запись сама заканчивается через 4 секунды тишины после речи. В любом случае она ограничена 120 секундами", "The recording ends by itself after 4 seconds of silence following speech. It is capped at 120 seconds either way"), false).kw(&["silence", "тишина"]),
                ],
            ),
            group(
                Tr::NONE,
                vec![
                    button("voice.test-mic", tr("Проверить микрофон", "Test the microphone"), tr("Записывает 2 секунды и показывает уровень. Ничего не отправляется.", "Records 2 seconds and shows the level. Nothing is sent."), tr("Проверить микрофон", "Test the microphone"), Action::VoiceTestMic, ButtonStyle::Normal),
                    button("voice.test-service", tr("Проверить сервис", "Test the service"), tr("Отправляет 1 секунду тишины, чтобы проверить адрес, ключ и модель.", "Sends 1 second of silence to check the address, key and model."), tr("Проверить сервис", "Test the service"), Action::VoiceTestService, ButtonStyle::Normal),
                ],
            ),
        ],
    )
}

pub fn updates() -> Page {
    page(
        "updates",
        "updates",
        tr("Обновления", "Updates"),
        vec![group(
            Tr::NONE,
            vec![
                button(
                    "updates.check",
                    tr("Версия", "Version"),
                    tr("Pill ищет обновления по кнопке и при запуске (можно выключить ниже). Новая версия ставится только по вашему нажатию, скачивается с GitHub и проверяется по подписи.", "Pill looks for updates on the button and at launch (you can turn that off below). A new version is installed only on your click, comes from GitHub and is signature-verified."),
                    tr("Проверить сейчас", "Check now"),
                    Action::UpdateCheck,
                    ButtonStyle::Normal,
                )
                .kw(&["update", "version", "upgrade", "обновление", "версия", "обновить"]),
                toggle("updateAuto", tr("Проверять при запуске", "Check on start"), tr("Через ~20 секунд после старта один запрос к GitHub. Установить обновление всё равно нужно вручную.", "One request to GitHub ~20 s after launch. Installing is still up to you."), true).kw(&["update", "github", "network", "обновление", "сеть"]),
                toggle("whatsNew", tr("Показывать «Что нового» после обновления", "Show the what is new card after an update"), tr("Карточка с заметками версии один раз после обновления; сеть не используется", "A card with the version notes, once after an update; no network"), true).kw(&["changelog", "what's new", "что нового", "заметки"]),
                button(
                    "updates.report",
                    tr("Сообщить о проблеме", "Report a problem"),
                    tr("Создаёт папку в Загрузках: версия Pill и Windows, список включённых модулей, настройки без секретов и конец журнала. Буфер, заметки, почта и пути к вашим файлам не попадают.", "Creates a folder in Downloads: Pill and Windows versions, the enabled modules, settings without secrets and the end of the log. Clipboard, notes, mail and your file paths are left out."),
                    tr("Сообщить о проблеме", "Report a problem"),
                    Action::ReportProblem,
                    ButtonStyle::Normal,
                )
                .kw(&["bug", "report", "problem", "feedback", "проблема", "ошибка", "отчёт", "log", "лог"]),
                custom("updates.whatsnew", tr("Что нового", "What is new"), Tr::NONE, "updates.whatsnew").kw(&["changelog", "release notes", "изменения"]),
            ],
        )],
    )
}
