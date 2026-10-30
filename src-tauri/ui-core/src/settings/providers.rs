//! Provider presets of the AI page (chat) and the Voice page (speech recognition): the chip rows, their help text and the
//! settings patches a click produces. Pure port of `PROV` in settings.js / voice-ui.js. Nothing here talks to a network;
//! both features stay off until the user flips `net.ai` / `voice.enabled`.
use crate::settings::tr::{same, tr, Tr};
use serde_json::{json, Map, Value};

pub struct Provider {
    pub id: &'static str,
    pub name: Tr,
    /// Extra badge ("local").
    pub tag: Tr,
    /// "free" badge.
    pub free: bool,
    /// Shows the "does not work in every country" warning.
    pub region: bool,
    pub url: &'static str,
    pub key_url: &'static str,
    /// AI only: `ai.preset` value ("" = none, stored as "custom").
    pub preset: &'static str,
    /// Voice only: `voice.model`.
    pub model: &'static str,
    pub help: Tr,
}

#[allow(clippy::too_many_arguments)]
const fn p(id: &'static str, name: Tr, tag: Tr, free: bool, region: bool, url: &'static str, key_url: &'static str, preset: &'static str, model: &'static str, help: Tr) -> Provider {
    Provider { id, name, tag, free, region, url, key_url, preset, model, help }
}

const LOCAL: Tr = tr("локально", "local");

pub const AI: [Provider; 7] = [
    p("ollama", same("Ollama"), LOCAL, false, false, "http://127.0.0.1:11434/v1", "", "ollama", "", tr("Модели на этом компьютере. Ключ не нужен.", "Models on this computer. No key needed.")),
    p("freellm", same("FreeLLMAPI"), LOCAL, false, false, "http://127.0.0.1:3001/v1", "", "freellm", "", tr("Свой шлюз на этом компьютере. Ключ не нужен.", "Your own gateway on this computer. No key needed.")),
    p("groq", same("Groq"), Tr::NONE, true, true, "https://api.groq.com/openai/v1", "https://console.groq.com/keys", "", "", tr("Бесплатно, без карты: около 1000 запросов в день. Тот же ключ подходит для диктовки.", "Free, no card: about 1000 requests a day. The same key works for dictation.")),
    p("gemini", same("Google Gemini"), Tr::NONE, true, true, "https://generativelanguage.googleapis.com/v1beta/openai", "https://aistudio.google.com/apikey", "", "", tr("Бесплатный тариф Google AI Studio: картинки, длинные документы. Тот же ключ подходит для диктовки.", "The free Google AI Studio tier: pictures, long documents. The same key works for dictation.")),
    p("openrouter", same("OpenRouter"), Tr::NONE, true, true, "https://openrouter.ai/api/v1", "https://openrouter.ai/keys", "", "", tr("Много моделей за одним ключом, у части есть бесплатный лимит.", "Many models behind one key, some with a free allowance.")),
    p("deepseek", same("DeepSeek"), Tr::NONE, false, false, "https://api.deepseek.com/v1", "https://platform.deepseek.com/api_keys", "", "", tr("Платный сервис с низкими ценами.", "A paid service with low prices.")),
    p("custom", tr("Другой", "Custom"), Tr::NONE, false, false, "", "", "", "", tr("Любой OpenAI-совместимый адрес: LM Studio, vLLM, Polza.ai, OmniRoute, APINET.", "Any OpenAI-compatible address: LM Studio, vLLM, Polza.ai, OmniRoute, APINET.")),
];

pub const VOICE: [Provider; 4] = [
    p("groq", same("Groq · Whisper"), Tr::NONE, true, true, "https://api.groq.com/openai/v1", "https://console.groq.com/keys", "", "whisper-large-v3-turbo", tr("Бесплатно, без карты: до 8 часов речи в день, ответ за секунду. Хорошо понимает русский.", "Free, no card: up to 8 hours of speech a day, answers within a second. Understands Russian well.")),
    p("gemini", same("Google Gemini"), Tr::NONE, true, true, "https://generativelanguage.googleapis.com/v1beta/openai", "https://aistudio.google.com/apikey", "", "gemini-2.5-flash", tr("Бесплатный тариф Google AI Studio. Запись уходит как аудио в модель Gemini, она возвращает текст.", "The free Google AI Studio tier. The recording goes to a Gemini model as audio and it returns the text.")),
    p("openai", same("OpenAI"), Tr::NONE, false, false, "https://api.openai.com/v1", "https://platform.openai.com/api-keys", "", "whisper-1", tr("Платный сервис. Модель whisper-1 или gpt-4o-mini-transcribe.", "A paid service. The whisper-1 or gpt-4o-mini-transcribe model.")),
    p("custom", tr("Другой / локальный", "Other / local"), Tr::NONE, false, false, "http://127.0.0.1:8000/v1", "", "", "", tr("Любой сервер с маршрутом …/audio/transcriptions (faster-whisper-server, LocalAI, whisper.cpp). Для 127.0.0.1 ключ не нужен, запись остаётся на компьютере.", "Any server with a …/audio/transcriptions route (faster-whisper-server, LocalAI, whisper.cpp). For 127.0.0.1 no key is needed and the recording stays here.")),
];

pub const AI_REGION_WARNING: Tr = tr("Groq, Gemini и OpenRouter работают не во всех странах: если сервис не отвечает, включите VPN или выберите другой.", "Groq, Gemini and OpenRouter do not work in every country: if the service does not answer, turn on a VPN or pick another one.");
pub const VOICE_REGION_WARNING: Tr = tr("Groq и Gemini доступны не во всех странах: если сервис не отвечает, включи VPN или выбери другой из списка.", "Groq and Gemini are not available in every country: if the service does not answer, turn on a VPN or pick another one from the list.");
pub const GET_KEY: Tr = tr("Получить ключ", "Get a key");
pub const BADGE_FREE: Tr = tr("бесплатно", "free");

fn find<'a>(list: &'a [Provider], id: &str) -> Option<&'a Provider> {
    list.iter().find(|x| x.id == id)
}

fn obj(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// The chip that is on: the stored `ai.provider` if known, else the preset whose URL is the configured one, else "custom".
pub fn ai_current(stored: &str, base_url: &str) -> &'static str {
    if let Some(x) = find(&AI, stored) {
        return x.id;
    }
    AI.iter().find(|x| !x.url.is_empty() && x.url == base_url).map(|x| x.id).unwrap_or("custom")
}

/// A click on an AI provider chip.
pub fn ai_pick(id: &str) -> Option<Map<String, Value>> {
    let x = find(&AI, id)?;
    let mut m = obj(vec![("ai.provider", json!(x.id)), ("ai.preset", json!(if x.preset.is_empty() { "custom" } else { x.preset }))]);
    if !x.url.is_empty() {
        m.insert("ai.baseUrl".into(), json!(x.url));
    }
    Some(m)
}

/// The API address field was edited: the provider follows the URL.
pub fn ai_url_changed(url: &str) -> Map<String, Value> {
    let v = url.trim();
    let m = AI.iter().find(|x| x.url == v);
    obj(vec![
        ("ai.baseUrl", json!(v)),
        ("ai.provider", json!(m.map(|x| x.id).unwrap_or("custom"))),
        ("ai.preset", json!(m.map(|x| if x.preset.is_empty() { "custom" } else { x.preset }).unwrap_or("custom"))),
    ])
}

pub fn voice_current(stored: &str) -> &'static str {
    find(&VOICE, stored).map(|x| x.id).unwrap_or("groq")
}

fn is_local_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    ["http://127.0.0.1", "https://127.0.0.1", "http://localhost", "https://localhost"].iter().any(|p| l.starts_with(p))
}

/// A click on a voice provider chip. `current_url` is the text of the address field, `current_model` the stored `voice.model`.
pub fn voice_pick(id: &str, current_url: &str, current_model: &str) -> Option<Map<String, Value>> {
    let x = find(&VOICE, id)?;
    let mut m = obj(vec![("voice.provider", json!(x.id))]);
    if x.id != "custom" || !is_local_url(current_url) {
        m.insert("voice.endpoint".into(), json!(x.url));
    }
    m.insert("voice.model".into(), json!(if x.id == "custom" { current_model } else { x.model }));
    Some(m)
}

pub fn voice_url_changed(url: &str) -> Map<String, Value> {
    let v = url.trim();
    let id = VOICE.iter().find(|x| x.url == v).map(|x| x.id).unwrap_or("custom");
    obj(vec![("voice.endpoint", json!(v)), ("voice.provider", json!(id))])
}

/// The address the voice field shows: the stored endpoint, or the Groq URL.
pub fn voice_url(stored: &str) -> &str {
    if stored.is_empty() {
        VOICE[0].url
    } else {
        stored
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_chips_write_provider_preset_and_url() {
        let m = ai_pick("ollama").unwrap();
        assert_eq!((m["ai.provider"].as_str(), m["ai.preset"].as_str(), m["ai.baseUrl"].as_str()), (Some("ollama"), Some("ollama"), Some("http://127.0.0.1:11434/v1")));
        let m = ai_pick("groq").unwrap();
        assert_eq!(m["ai.preset"], "custom");
        let m = ai_pick("custom").unwrap();
        assert!(!m.contains_key("ai.baseUrl"), "Custom keeps the address the user typed");
        assert!(ai_pick("nope").is_none());
    }

    #[test]
    fn ai_url_edit_picks_the_matching_chip() {
        let m = ai_url_changed("  http://127.0.0.1:3001/v1 ");
        assert_eq!((m["ai.provider"].as_str(), m["ai.preset"].as_str()), (Some("freellm"), Some("freellm")));
        let m = ai_url_changed("https://example.org/v1");
        assert_eq!((m["ai.provider"].as_str(), m["ai.preset"].as_str()), (Some("custom"), Some("custom")));
        assert_eq!(ai_url_changed("")["ai.provider"], "custom", "an empty address is the Custom chip");
        assert_eq!(ai_current("", "https://api.deepseek.com/v1"), "deepseek");
        assert_eq!(ai_current("groq", "x"), "groq");
        assert_eq!(ai_current("bogus", ""), "custom");
    }

    #[test]
    fn voice_chips() {
        let m = voice_pick("gemini", "", "").unwrap();
        assert_eq!((m["voice.endpoint"].as_str(), m["voice.model"].as_str()), (Some(VOICE[1].url), Some("gemini-2.5-flash")));
        let m = voice_pick("custom", "http://localhost:9000/v1", "my-model").unwrap();
        assert!(!m.contains_key("voice.endpoint"), "a local custom address is kept");
        assert_eq!(m["voice.model"], "my-model");
        let m = voice_pick("custom", "https://remote/v1", "my-model").unwrap();
        assert_eq!(m["voice.endpoint"], "http://127.0.0.1:8000/v1");
        assert_eq!(voice_url_changed("https://api.openai.com/v1")["voice.provider"], "openai");
        assert_eq!(voice_url_changed("http://x")["voice.provider"], "custom");
        assert_eq!((voice_current("zzz"), voice_current("openai")), ("groq", "openai"));
        assert_eq!(voice_url(""), "https://api.groq.com/openai/v1");
    }

    #[test]
    fn presets_are_well_formed() {
        for list in [&AI[..], &VOICE[..]] {
            let mut ids = std::collections::HashSet::new();
            for x in list {
                assert!(ids.insert(x.id) && x.name.is_complete() && x.help.is_complete(), "{}", x.id);
                assert!(x.url.is_empty() || x.url.starts_with("http://127.0.0.1") || x.url.starts_with("https://"), "{} must be https or loopback", x.id);
            }
        }
    }
}
