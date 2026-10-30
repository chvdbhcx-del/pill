//! Tests for i18n: language resolution, the `Tr` implementations, fmt / plural helpers and the generated catalogue.
//! The table is validated from the Rust side so a hand edit shows up in `cargo test`.
use super::*;
use std::collections::HashSet;

#[test]
fn language_resolution_follows_pill_js() {
    assert_eq!(Lang::resolve("ru", "en-US"), Lang::Ru);
    assert_eq!(Lang::resolve("en", "ru-RU"), Lang::En);
    assert_eq!(Lang::resolve("auto", "ru-RU"), Lang::Ru);
    assert_eq!(Lang::resolve("auto", "RU"), Lang::Ru); // /^ru/i
    assert_eq!(Lang::resolve("auto", "ru_RU.UTF-8"), Lang::Ru);
    assert_eq!(Lang::resolve("", "en-GB"), Lang::En);
    assert_eq!(Lang::resolve("de", "ru-RU"), Lang::Ru, "unknown setting = auto");
    assert_eq!(Lang::resolve("RU", "en-US"), Lang::En, "the setting compare is case-sensitive, like v === 'ru'");
    assert_eq!(Lang::from_locale(""), Lang::En);
    assert_eq!(Lang::from_locale("r"), Lang::En);
    assert_eq!(Lang::from_locale("é"), Lang::En); // multi-byte first char must not panic
    assert_eq!(Lang::Ru.code(), "ru");
    assert_eq!(Lang::En.code(), "en");
    assert_eq!(Lang::from_code("ru"), Some(Lang::Ru));
    assert_eq!(Lang::from_code("fr"), None);
    assert_eq!(Lang::Ru.other(), Lang::En);
    assert_eq!(Lang::Ru.clock_locale(), "ru-RU");
    assert_eq!(Lang::En.clock_locale(), "en-GB");
    assert_eq!(Lang::En.bcp47(), "en-US");
}

#[test]
fn tr_pick_and_optional_sides() {
    assert_eq!(pick(Lang::Ru, "а", "b"), "а");
    assert_eq!(pick(Lang::En, "а", "b"), "b");
    assert_eq!(t_opt(Lang::Ru, Some("а"), Some("b")), "а");
    assert_eq!(t_opt(Lang::Ru, None, Some("b")), "b", "o.ru ?? o.en");
    assert_eq!(t_opt(Lang::En, Some("а"), None), "а", "o.en ?? o.ru");
    assert_eq!(t_opt(Lang::En, None, None), "");
    assert_eq!(t_opt(Lang::Ru, Some(""), Some("b")), "", "an empty string is a value, as in JS `??`");
}

// The global language is process-wide: one test owns it so parallel tests never race on it.
#[test]
fn global_language_switch() {
    let g = GlobalTr;
    set_lang(Lang::Ru);
    assert_eq!(lang(), Lang::Ru);
    assert_eq!(tr("Открыть", "Open"), "Открыть");
    assert_eq!(g.t("Открыть", "Open"), "Открыть");
    assert_eq!(LangTr::current().lang, Lang::Ru);
    assert_eq!(apply_setting("auto", "en-US"), Lang::En);
    assert_eq!(tr("Открыть", "Open"), "Open");
    assert_eq!(g.t("Открыть", "Open"), "Open");
    assert_eq!(g.tn(2, ["файл", "файла", "файлов"], ["file", "files"]), "files");
    assert_eq!(apply_setting("ru", "en-US"), Lang::Ru);
    assert_eq!(g.tn(2, ["файл", "файла", "файлов"], ["file", "files"]), "файла");
    set_lang(Lang::En);
}

#[test]
fn tr_trait_impls() {
    let ru = LangTr::new(Lang::Ru);
    let en = LangTr::new(Lang::En);
    assert_eq!(ru.t("Закрыть", "Close"), "Закрыть");
    assert_eq!(en.t("Закрыть", "Close"), "Close");
    assert_eq!(ru.tf("Шаг {0} из {1}", "Step {0} of {1}", &[&2, &5]), "Шаг 2 из 5");
    assert_eq!(en.tf("Шаг {0} из {1}", "Step {0} of {1}", &[&2, &5]), "Step 2 of 5");
    // usable through references, Box<dyn Tr> and Arc<dyn Tr>
    let boxed: Box<dyn Tr> = Box::new(ru);
    assert_eq!(boxed.t("Да", "Yes"), "Да");
    let arc: Arc<dyn Tr> = Arc::new(en);
    assert_eq!(arc.t("Да", "Yes"), "Yes");
    fn takes(t: &impl Tr) -> String {
        t.t("Да", "Yes")
    }
    assert_eq!(takes(&&ru), "Да");
    assert_eq!(takes(&Lang::En), "Yes");
    assert_eq!(LangTr::default().lang, Lang::En);
}

#[test]
fn fmt_fills_placeholders() {
    assert_eq!(fmt("Шаг {0} из {1}", &[&3, &"пяти"]), "Шаг 3 из пяти");
    assert_eq!(fmt("{1}-{0}-{1}", &[&"a", &"b"]), "b-a-b", "repeat and reorder");
    assert_eq!(fmt("{0}", &[]), "{0}", "missing argument stays visible");
    assert_eq!(fmt("{x} {} {0a} {", &[&1]), "{x} {} {0a} {", "only a brace-digits-brace group is a placeholder");
    assert_eq!(fmt("«{0}» · ✓", &[&"é"]), "«é» · ✓", "multi-byte text survives");
    assert_eq!(fmt("", &[&1]), "");
    assert_eq!(fmt("{12}", &[&1]), "{12}");
}

#[test]
fn plurals() {
    let f = ["файл", "файла", "файлов"];
    let expect = [(0, "файлов"), (1, "файл"), (2, "файла"), (4, "файла"), (5, "файлов"), (11, "файлов"), (12, "файлов"), (14, "файлов"), (21, "файл"), (22, "файла"), (101, "файл"), (111, "файлов"), (112, "файлов"), (1000, "файлов")];
    for (n, w) in expect {
        assert_eq!(plural_ru(n, f), w, "n={n}");
    }
    assert_eq!(plural_en(1, ["file", "files"]), "file");
    assert_eq!(plural_en(0, ["file", "files"]), "files");
    assert_eq!(plural_en(2, ["file", "files"]), "files");
    assert_eq!(plural(Lang::Ru, 3, f, ["file", "files"]), "файла");
    assert_eq!(plural(Lang::En, 3, f, ["file", "files"]), "files");
    assert_eq!(LangTr::new(Lang::Ru).tn(21, f, ["file", "files"]), "файл");
}

#[test]
fn id_function_is_fnv1a64_of_en_nul_ru() {
    // reference implementation over the concatenated bytes
    fn reference(ru: &str, en: &str) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in en.bytes().chain(std::iter::once(0)).chain(ru.bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
    for (ru, en) in [("", ""), ("Открыть", "Open"), ("Шаг {0} из {1}", "Step {0} of {1}"), ("é", "e")] {
        assert_eq!(id_of(ru, en), reference(ru, en));
    }
    // const-evaluable
    const A: u64 = id_of("Открыть", "Open");
    assert_eq!(A, id_of("Открыть", "Open"));
    assert_ne!(id_of("a", "b"), id_of("b", "a"));
}

#[test]
fn catalogue_invariants() {
    assert_eq!(ENTRIES.len(), UNIQUE, "UNIQUE constant out of sync with the table");
    assert!(UNIQUE > 1000, "the web UI has >1000 unique ru/en pairs, got {UNIQUE}");
    assert!(CALL_SITES >= UNIQUE, "dedup cannot create rows");
    assert!(DYNAMIC_SITES <= 20, "too many unflattened sites: {DYNAMIC_SITES}");
    let mut ids = HashSet::new();
    let mut pairs = HashSet::new();
    let mut prev: Option<u64> = None;
    for &(id, ru, en) in ENTRIES {
        assert!(!ru.trim().is_empty(), "empty ru for {id:#x} ({en})");
        assert!(!en.trim().is_empty(), "empty en for {id:#x} ({ru})");
        assert!(ids.insert(id), "duplicate id {id:#x}");
        assert!(pairs.insert((ru, en)), "duplicate pair {ru} / {en}");
        if let Some(p) = prev {
            assert!(p < id, "table not sorted by id at {id:#x}");
        }
        prev = Some(id);
        assert_eq!(id, id_of(ru, en), "id is not the content hash for {ru} / {en}");
    }
}

#[test]
fn placeholders_agree_on_both_sides() {
    // the extractor numbers the same expression identically in ru and en, so the highest index matches (a side may omit one,
    // e.g. "{0} файл" vs "{0} file": counts of distinct indices are compared by max)
    fn max_index(s: &str) -> Option<usize> {
        let mut m = None;
        let b = s.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'{' {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 && j < b.len() && b[j] == b'}' {
                    let n: usize = s[i + 1..j].parse().unwrap();
                    m = Some(m.map_or(n, |x: usize| x.max(n)));
                }
            }
            i += 1;
        }
        m
    }
    let mut with = 0;
    for e in entries() {
        if e.placeholders() > 0 {
            with += 1;
        }
        assert_eq!(max_index(e.ru).is_some(), max_index(e.en).is_some(), "placeholder on one side only: {} / {}", e.ru, e.en);
    }
    assert!(with >= 30, "expected dozens of templated strings, got {with}");
}

#[test]
fn lookups_and_spot_checks() {
    // strings from capture.js / capture-live.js, pill tabs and settings
    for (ru, en) in [
        ("Скриншот сохранён", "Screenshot saved"),
        ("Скриншот скопирован", "Screenshot copied"),
        ("Текст не найден", "No text found"),
        ("QR-код не найден", "No QR code found"),
        ("Запись сохранена", "Recording saved"),
        ("Не получилось", "Capture failed"),
        ("Выдели область · Esc — отмена", "Drag to select an area · Esc to cancel"),
        ("Выдели QR-код · Esc — отмена", "Select a QR code · Esc to cancel"),
        ("Отменить (Ctrl+Z)", "Undo (Ctrl+Z)"),
        ("Готово: копировать и сохранить (Enter)", "Done: copy and save (Enter)"),
        ("Шаг {0} из {1}", "Step {0} of {1}"),
        ("Искать в браузере: «{0}»", "Search in the browser: “{0}”"),
        ("Небольшой снег", "Light snow"),
        ("Закрыть", "Close"),
    ] {
        let e = lookup_pair(ru, en).unwrap_or_else(|| panic!("missing {ru} / {en}"));
        assert_eq!(e.id, id_of(ru, en));
        assert_eq!(lookup(e.id), Some(e));
        assert_eq!(e.text(Lang::Ru), ru);
        assert_eq!(e.text(Lang::En), en);
        assert!(known(ru, en));
    }
    assert!(!known("Нет такой строки", "No such string"));
    assert!(lookup(0).is_none());
    assert!(lookup_pair("Открыть", "Wrong").is_none());
    assert_eq!(lookup_pair("Шаг {0} из {1}", "Step {0} of {1}").unwrap().placeholders(), 2);
    assert_eq!(lookup_pair("Закрыть", "Close").unwrap().placeholders(), 0);
    assert!(find_by_en("Close").any(|e| e.ru == "Закрыть"));
    assert!(find_by_ru("Закрыть").any(|e| e.en == "Close"));
    assert_eq!(ru_of_en("Screenshot saved"), Some("Скриншот сохранён"));
    assert_eq!(ru_of_en("Definitely not a ui string"), None);
    assert_eq!(entries().count(), UNIQUE);
}

#[test]
fn every_capture_label_of_the_web_page_is_in_the_catalogue() {
    // the labels capture/ ports from capture.js (TIPS + HINT) and capture-live.js: a regeneration that drops one fails here
    let tips = [
        ("Выбор", "Select"), ("Рамка", "Rectangle"), ("Стрелка", "Arrow"), ("Карандаш", "Pen"), ("Скрыть (пиксели)", "Hide (pixelate)"),
        ("Текст", "Text"), ("Копировать (Ctrl+C)", "Copy (Ctrl+C)"), ("Сохранить (Ctrl+S)", "Save (Ctrl+S)"),
        ("Записать эту область", "Record this area"), ("Отмена (Esc)", "Cancel (Esc)"),
        ("Выдели текст на экране · Esc — отмена", "Select the text to read · Esc to cancel"),
        ("Выдели область записи · Enter — весь экран · Esc — отмена", "Select the area to record · Enter for the whole screen · Esc to cancel"),
        ("Клик — скопировать цвет · стрелки — на 1 пиксель · Esc — отмена", "Click to copy the colour · arrows move 1 pixel · Esc to cancel"),
    ];
    for (ru, en) in tips {
        assert!(known(ru, en), "capture.js label missing from the catalogue: {ru} / {en}");
    }
}
