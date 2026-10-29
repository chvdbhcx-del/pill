//! i18n for the native Pill UI: the ru/en pair model of the web UI, in Rust.
//!
//! The web UI spells every translation inline: `t({ ru: 'Открыть', en: 'Open' })` (`Pill.t` in pill.js, `window.t` in i18n-lite.js,
//! ~800 call sites). The native UI keeps exactly that style, so a port is a mechanical copy:
//!
//! ```
//! use pill_ui_core::i18n::{Lang, LangTr, Tr};
//! let tr = LangTr::new(Lang::Ru);
//! assert_eq!(tr.t("Открыть", "Open"), "Открыть");
//! assert_eq!(LangTr::new(Lang::En).t("Открыть", "Open"), "Open");
//! ```
//!
//! What lives here
//! * [`Lang`] and the process-wide current language ([`lang`], [`set_lang`], [`apply_setting`]): the `language` setting is
//!   `auto | ru | en`; `auto` follows the system locale exactly like `nav()` in pill.js (`/^ru/i`). Unknown values fall back to auto.
//! * [`Tr`] - the trait every other area of the crate takes (`fn t(&self, ru, en) -> String`), with [`LangTr`] (a fixed language,
//!   what tests use) and [`GlobalTr`] (follows [`lang`], what the renderer uses). `&T`, `Box<T>` and `Arc<T>` implement it too.
//! * [`tr`] / [`pick`] / [`t_opt`] - borrowed-string variants; `t_opt` reproduces pill.js `o[lang] ?? o.en ?? o.ru ?? ''` for
//!   objects with a missing side.
//! * [`fmt`] - fills the `{0}`, `{1}` placeholders the extractor writes for template / concatenation parts, and the plural helpers
//!   ([`plural_ru`], [`plural_en`], [`plural`]) for the few call sites whose text depends on a count.
//! * The catalogue ([`ENTRIES`], generated into `table.rs` by `tools/dev/extract-i18n.mjs`): every static pair of the web UI,
//!   deduplicated and sorted by content-addressed id ([`id_of`]); [`lookup`] / [`lookup_pair`] / [`find_by_en`] search it. The
//!   renderer-facing code does not need it to translate (the literals are inline); it exists so a test can prove that no web
//!   string was forgotten (`known(ru, en)`), and so data-driven text (settings search index, command palette) can be looked up.
//!
//! Ids are FNV-1a 64 of `en + "\0" + ru` - a function of the text only, so regenerating the table never renumbers anything and
//! two agents adding the same string always agree on its id. Dynamic sites the extractor cannot flatten (plural expressions,
//! computed text) are counted in [`DYNAMIC_SITES`] and listed by `node tools/dev/extract-i18n.mjs --report`.
use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

mod table;
pub use table::{CALL_SITES, DYNAMIC_SITES, ENTRIES, UNIQUE};

#[cfg(test)]
mod tests;

// ------------------------------------------------------------------------------------------------- language

/// The two UI languages. `En` is the fallback everywhere (pill.js: `o[lang] ?? o.en ?? o.ru`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    Ru,
    #[default]
    En,
}

impl Lang {
    /// `"ru"` / `"en"`, the value stored in localStorage `pill.lang` and settings.json.
    pub fn code(self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }
    /// Strict parse of `"ru"` / `"en"` (case-sensitive like the JS `v === 'ru'`).
    pub fn from_code(s: &str) -> Option<Lang> {
        match s {
            "ru" => Some(Lang::Ru),
            "en" => Some(Lang::En),
            _ => None,
        }
    }
    /// pill.js `nav()`: `/^ru/i.test(navigator.language)`; the system locale is e.g. `ru-RU`, `ru_RU.UTF-8`, `en-US`.
    pub fn from_locale(locale: &str) -> Lang {
        let l = locale.trim();
        if l.len() >= 2 && l.is_char_boundary(2) && l[..2].eq_ignore_ascii_case("ru") {
            Lang::Ru
        } else {
            Lang::En
        }
    }
    /// pill.js `resolveLang(v)`: `ru`/`en` win, anything else (`auto`, empty, garbage) follows the system locale.
    pub fn resolve(setting: &str, system_locale: &str) -> Lang {
        Lang::from_code(setting).unwrap_or_else(|| Lang::from_locale(system_locale))
    }
    /// BCP-47 tag for date / number formatting (`ru-RU` / `en-US`; pill.js uses `en-GB` for its own clock, see [`Lang::clock_locale`]).
    pub fn bcp47(self) -> &'static str {
        match self {
            Lang::Ru => "ru-RU",
            Lang::En => "en-US",
        }
    }
    /// The locale pill.js `loc()` passes to its clock / date formatting (`ru-RU` / `en-GB`).
    pub fn clock_locale(self) -> &'static str {
        match self {
            Lang::Ru => "ru-RU",
            Lang::En => "en-GB",
        }
    }
    /// The other language (for "switch language" toggles).
    pub fn other(self) -> Lang {
        match self {
            Lang::Ru => Lang::En,
            Lang::En => Lang::Ru,
        }
    }
}

const RU: u8 = 1;
const EN: u8 = 0;
static CURRENT: AtomicU8 = AtomicU8::new(EN);

/// The process-wide current language (default `En` until [`set_lang`] / [`apply_setting`] runs at start-up).
pub fn lang() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == RU {
        Lang::Ru
    } else {
        Lang::En
    }
}

/// Switch the current language; every [`GlobalTr`] and [`tr`] call sees it at once.
pub fn set_lang(l: Lang) {
    CURRENT.store(if l == Lang::Ru { RU } else { EN }, Ordering::Relaxed);
}

/// Apply the `language` setting (`auto | ru | en`) given the system locale; returns the resolved language.
pub fn apply_setting(setting: &str, system_locale: &str) -> Lang {
    let l = Lang::resolve(setting, system_locale);
    set_lang(l);
    l
}

/// `ru` or `en` by an explicit language.
pub fn pick<'a>(l: Lang, ru: &'a str, en: &'a str) -> &'a str {
    match l {
        Lang::Ru => ru,
        Lang::En => en,
    }
}

/// `ru` or `en` by the current language - the borrowed twin of `Pill.t({ ru, en })`.
pub fn tr<'a>(ru: &'a str, en: &'a str) -> &'a str {
    pick(lang(), ru, en)
}

/// pill.js `t(o)` for an object whose sides may be missing: `o[lang] ?? o.en ?? o.ru ?? ''`.
pub fn t_opt(l: Lang, ru: Option<&str>, en: Option<&str>) -> String {
    match l {
        Lang::Ru => ru.or(en).unwrap_or("").to_string(),
        Lang::En => en.or(ru).unwrap_or("").to_string(),
    }
}

// ------------------------------------------------------------------------------------------------- Tr

/// What the view-model code of every area takes to translate a label: `tr.t("Открыть", "Open")`.
/// Implemented here by [`LangTr`] and [`GlobalTr`]; tests of other areas pass `LangTr::new(Lang::Ru)` / `Lang::En` to check both
/// sides of a label.
pub trait Tr {
    /// The string for the language this translator speaks.
    fn t(&self, ru: &str, en: &str) -> String;
    /// `t` followed by [`fmt`]: `tr.tf("Шаг {0} из {1}", "Step {0} of {1}", &[&2, &5])`.
    fn tf(&self, ru: &str, en: &str, args: &[&dyn Display]) -> String {
        fmt(&self.t(ru, en), args)
    }
    /// Count-dependent text: `plural(lang, n, ru_forms, en_forms)` with this translator's language.
    fn tn(&self, n: i64, ru: [&str; 3], en: [&str; 2]) -> String;
}

/// A translator pinned to one language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LangTr {
    pub lang: Lang,
}

impl LangTr {
    pub fn new(lang: Lang) -> LangTr {
        LangTr { lang }
    }
    /// Pinned to whatever [`lang`] is right now.
    pub fn current() -> LangTr {
        LangTr { lang: lang() }
    }
}

impl Tr for LangTr {
    fn t(&self, ru: &str, en: &str) -> String {
        pick(self.lang, ru, en).to_string()
    }
    fn tn(&self, n: i64, ru: [&str; 3], en: [&str; 2]) -> String {
        plural(self.lang, n, ru, en).to_string()
    }
}

/// A translator that follows the process-wide language ([`set_lang`]) on every call: the renderer holds one of these.
#[derive(Clone, Copy, Debug, Default)]
pub struct GlobalTr;

impl Tr for GlobalTr {
    fn t(&self, ru: &str, en: &str) -> String {
        tr(ru, en).to_string()
    }
    fn tn(&self, n: i64, ru: [&str; 3], en: [&str; 2]) -> String {
        plural(lang(), n, ru, en).to_string()
    }
}

impl Tr for Lang {
    fn t(&self, ru: &str, en: &str) -> String {
        pick(*self, ru, en).to_string()
    }
    fn tn(&self, n: i64, ru: [&str; 3], en: [&str; 2]) -> String {
        plural(*self, n, ru, en).to_string()
    }
}

impl<T: Tr + ?Sized> Tr for &T {
    fn t(&self, ru: &str, en: &str) -> String {
        (**self).t(ru, en)
    }
    fn tn(&self, n: i64, ru: [&str; 3], en: [&str; 2]) -> String {
        (**self).tn(n, ru, en)
    }
}
impl<T: Tr + ?Sized> Tr for Box<T> {
    fn t(&self, ru: &str, en: &str) -> String {
        (**self).t(ru, en)
    }
    fn tn(&self, n: i64, ru: [&str; 3], en: [&str; 2]) -> String {
        (**self).tn(n, ru, en)
    }
}
impl<T: Tr + ?Sized> Tr for Arc<T> {
    fn t(&self, ru: &str, en: &str) -> String {
        (**self).t(ru, en)
    }
    fn tn(&self, n: i64, ru: [&str; 3], en: [&str; 2]) -> String {
        (**self).tn(n, ru, en)
    }
}

// ------------------------------------------------------------------------------------------------- fmt / plural

/// Replace `{0}`, `{1}`, ... with the arguments (the placeholder syntax of the extracted table). A placeholder without a
/// matching argument, or anything that is not `{<digits>}`, is left as written.
pub fn fmt(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 8 * args.len());
    let b = template.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'{' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < b.len() && b[j] == b'}' {
                if let Ok(n) = template[i + 1..j].parse::<usize>() {
                    if let Some(a) = args.get(n) {
                        out.push_str(&a.to_string());
                        i = j + 1;
                        continue;
                    }
                }
            }
        }
        // copy one UTF-8 char
        let ch = template[i..].chars().next().unwrap_or('\u{fffd}');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Russian plural form index for `n` (the rule tab-shelf.js `files()` and live-events.js use): 0 = one (1, 21, 101; not 11),
/// 1 = few (2-4, 22-24; not 12-14), 2 = many (everything else, including 0).
pub fn plural_ru_index(n: i64) -> usize {
    let (m10, m100) = (n.rem_euclid(10), n.rem_euclid(100));
    if m10 == 1 && m100 != 11 {
        0
    } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
        1
    } else {
        2
    }
}

/// `plural_ru(n, ["файл", "файла", "файлов"])`.
pub fn plural_ru<'a>(n: i64, forms: [&'a str; 3]) -> &'a str {
    forms[plural_ru_index(n)]
}

/// English: exactly 1 is singular (`n === 1 ? ' file' : ' files'`).
pub fn plural_en<'a>(n: i64, forms: [&'a str; 2]) -> &'a str {
    if n == 1 {
        forms[0]
    } else {
        forms[1]
    }
}

/// Language-dispatched plural: `plural(Lang::Ru, 3, ["файл", "файла", "файлов"], ["file", "files"]) == "файла"`.
pub fn plural<'a>(l: Lang, n: i64, ru: [&'a str; 3], en: [&'a str; 2]) -> &'a str {
    match l {
        Lang::Ru => plural_ru(n, ru),
        Lang::En => plural_en(n, en),
    }
}

// ------------------------------------------------------------------------------------------------- catalogue

/// One row of the catalogue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub ru: &'static str,
    pub en: &'static str,
}

impl Entry {
    /// The side for a language.
    pub fn text(&self, l: Lang) -> &'static str {
        pick(l, self.ru, self.en)
    }
    /// Number of distinct `{n}` placeholders (0 for plain strings).
    pub fn placeholders(&self) -> usize {
        let mut max: Option<usize> = None;
        let b = self.en.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'{' {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 && j < b.len() && b[j] == b'}' {
                    if let Ok(n) = self.en[i + 1..j].parse::<usize>() {
                        max = Some(max.map_or(n, |m| m.max(n)));
                    }
                }
            }
            i += 1;
        }
        max.map_or(0, |m| m + 1)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Content-addressed id of a pair: FNV-1a 64 over the UTF-8 bytes of `en`, a NUL, then `ru`. Matches `idOf` in extract-i18n.mjs.
pub const fn id_of(ru: &str, en: &str) -> u64 {
    let mut h = FNV_OFFSET;
    let e = en.as_bytes();
    let mut i = 0;
    while i < e.len() {
        h ^= e[i] as u64;
        h = h.wrapping_mul(FNV_PRIME);
        i += 1;
    }
    h ^= 0;
    h = h.wrapping_mul(FNV_PRIME);
    let r = ru.as_bytes();
    let mut k = 0;
    while k < r.len() {
        h ^= r[k] as u64;
        h = h.wrapping_mul(FNV_PRIME);
        k += 1;
    }
    h
}

fn entry(row: &(u64, &'static str, &'static str)) -> Entry {
    Entry { id: row.0, ru: row.1, en: row.2 }
}

/// The row with this id (binary search; the table is sorted by id).
pub fn lookup(id: u64) -> Option<Entry> {
    ENTRIES.binary_search_by_key(&id, |r| r.0).ok().map(|i| entry(&ENTRIES[i]))
}

/// The row for this exact pair, if the web UI has it.
pub fn lookup_pair(ru: &str, en: &str) -> Option<Entry> {
    lookup(id_of(ru, en)).filter(|e| e.ru == ru && e.en == en)
}

/// True when the pair is in the catalogue: `assert!(known("Открыть", "Open"))`.
pub fn known(ru: &str, en: &str) -> bool {
    lookup_pair(ru, en).is_some()
}

/// Every row whose English side is exactly `en` (the same English word can carry several Russian renderings by context).
pub fn find_by_en(en: &str) -> impl Iterator<Item = Entry> + '_ {
    ENTRIES.iter().filter(move |r| r.2 == en).map(entry)
}

/// Every row whose Russian side is exactly `ru`.
pub fn find_by_ru(ru: &str) -> impl Iterator<Item = Entry> + '_ {
    ENTRIES.iter().filter(move |r| r.1 == ru).map(entry)
}

/// The Russian rendering of an English UI string when it is unambiguous (exactly one catalogue row has that English side).
pub fn ru_of_en(en: &str) -> Option<&'static str> {
    let mut it = find_by_en(en);
    let first = it.next()?;
    it.next().is_none().then_some(first.ru)
}

/// All rows, in id order.
pub fn entries() -> impl Iterator<Item = Entry> {
    ENTRIES.iter().map(entry)
}
