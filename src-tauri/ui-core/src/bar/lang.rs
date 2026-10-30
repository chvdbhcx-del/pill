//! Language handling for the bar: `Lang` (ru / en) and the `Tr` trait that turns a `(ru, en)` pair into the string for a language.
//!
//! The JS pages use `{ ru: '...', en: '...' }` objects and `t(o)`; here the same table is a `(&str, &str)` tuple, so every string in the
//! bar and the uninstall panel is declared once, next to its other-language twin, and `Tr::tr(lang)` picks one.

/// Interface language. The bar follows `settings_get().language` (`ru` / `en`, anything else falls back to the OS language).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lang {
    Ru,
    #[default]
    En,
}

impl Lang {
    /// `ru`, `ru-RU`, `RU` -> Ru; everything else -> En (`/^ru/i` in the JS).
    pub fn from_locale(tag: &str) -> Lang {
        if tag.trim().to_ascii_lowercase().starts_with("ru") {
            Lang::Ru
        } else {
            Lang::En
        }
    }

    /// The setting value: exactly `ru` or `en` wins, anything else uses the OS locale `os`.
    pub fn from_setting(value: Option<&str>, os: &str) -> Lang {
        match value {
            Some("ru") => Lang::Ru,
            Some("en") => Lang::En,
            _ => Lang::from_locale(os),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }
}

/// A string with a Russian and an English form: `("Отмена", "Cancel").tr(lang)`.
pub trait Tr {
    fn tr(&self, lang: Lang) -> &'static str;
}

impl Tr for (&'static str, &'static str) {
    fn tr(&self, lang: Lang) -> &'static str {
        match lang {
            Lang::Ru => self.0,
            Lang::En => self.1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_and_setting() {
        assert_eq!(Lang::from_locale("ru-RU"), Lang::Ru);
        assert_eq!(Lang::from_locale("RU"), Lang::Ru);
        assert_eq!(Lang::from_locale("en-US"), Lang::En);
        assert_eq!(Lang::from_locale(""), Lang::En);
        assert_eq!(Lang::from_setting(Some("en"), "ru-RU"), Lang::En);
        assert_eq!(Lang::from_setting(Some("ru"), "en-US"), Lang::Ru);
        assert_eq!(Lang::from_setting(Some("auto"), "ru-RU"), Lang::Ru);
        assert_eq!(Lang::from_setting(None, "de-DE"), Lang::En);
    }

    #[test]
    fn tuples_translate() {
        let s: (&'static str, &'static str) = ("Отмена", "Cancel");
        assert_eq!(s.tr(Lang::Ru), "Отмена");
        assert_eq!(s.tr(Lang::En), "Cancel");
        assert_eq!(Lang::Ru.code(), "ru");
    }
}
