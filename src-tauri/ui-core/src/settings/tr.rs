//! ru/en text pair and the language enum used by every string of the settings schema.
//!
//! Mirrors `src/i18n-lite.js`: the `language` setting is `auto | ru | en`, `auto` follows the system UI language
//! (Russian when it starts with "ru", English otherwise) and `t({ru, en})` falls back en -> ru -> "".
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    #[default]
    Ru,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Ru, Lang::En];

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

    /// `setting` is the value of the `language` key, `system` the system UI language tag ("ru-RU", "en-US", ...).
    pub fn resolve(setting: &str, system: &str) -> Lang {
        match setting {
            "ru" => Lang::Ru,
            "en" => Lang::En,
            _ if system.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("ru")) => Lang::Ru,
            _ => Lang::En,
        }
    }
}

/// A (ru, en) pair of static strings. `Tr::NONE` (both empty) marks "no text", for example a row without a hint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Tr {
    pub ru: &'static str,
    pub en: &'static str,
}

/// Short constructor used by the schema data: `tr("Язык", "Language")`.
pub const fn tr(ru: &'static str, en: &'static str) -> Tr {
    Tr { ru, en }
}

/// The same text in both languages (names such as "MP4", "Ollama", "12 h").
pub const fn same(s: &'static str) -> Tr {
    Tr { ru: s, en: s }
}

impl Tr {
    pub const NONE: Tr = Tr { ru: "", en: "" };

    pub fn get(&self, lang: Lang) -> &'static str {
        let (a, b) = match lang {
            Lang::Ru => (self.ru, self.en),
            Lang::En => (self.en, self.ru),
        };
        if a.is_empty() {
            b
        } else {
            a
        }
    }

    pub fn is_blank(&self) -> bool {
        self.ru.is_empty() && self.en.is_empty()
    }

    /// Both languages are present (the integrity tests require it for every label).
    pub fn is_complete(&self) -> bool {
        !self.ru.trim().is_empty() && !self.en.trim().is_empty()
    }
}

/// Choose between two runtime strings (texts with numbers or names in them are built by the caller).
pub fn pick<'a>(lang: Lang, ru: &'a str, en: &'a str) -> &'a str {
    match lang {
        Lang::Ru => ru,
        Lang::En => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_follows_the_system_language() {
        assert_eq!(Lang::resolve("auto", "ru-RU"), Lang::Ru);
        assert_eq!(Lang::resolve("auto", "RU"), Lang::Ru);
        assert_eq!(Lang::resolve("auto", "en-US"), Lang::En);
        assert_eq!(Lang::resolve("auto", ""), Lang::En);
        assert_eq!(Lang::resolve("en", "ru-RU"), Lang::En);
        assert_eq!(Lang::resolve("ru", "en-US"), Lang::Ru);
        assert_eq!(Lang::resolve("xx", "de-DE"), Lang::En);
    }

    #[test]
    fn tr_falls_back_to_the_other_language() {
        let t = tr("Язык", "Language");
        assert_eq!((t.get(Lang::Ru), t.get(Lang::En)), ("Язык", "Language"));
        assert_eq!(tr("", "only en").get(Lang::Ru), "only en");
        assert_eq!(tr("только ru", "").get(Lang::En), "только ru");
        assert!(Tr::NONE.is_blank() && !t.is_blank());
        assert!(t.is_complete() && !tr("a", " ").is_complete());
        assert_eq!(same("MP4").get(Lang::Ru), "MP4");
        assert_eq!(pick(Lang::En, "да", "yes"), "yes");
    }
}
