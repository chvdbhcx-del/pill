//! Ctrl+F search over the settings (the field above the sidebar). Port of `settings-search.js` (+ its test cases) and of the
//! search block of `settings.js`: building the index, the result list, keyboard handling and the "go to the row" plan.
//!
//! Matching: every word of the query must occur in the title, the keywords, the page name or the description. Ranking is the
//! sum over words of: title word start 0, title elsewhere 1, keyword 2, page name 3, description 4 (lower is better; ties keep
//! index order). Case and ё/е are ignored. Keywords are new (the JS index had none): they carry extra ru+en synonyms and the
//! label of the other language, so "quiet hours" finds the row in a Russian UI and "тихие" in an English one.
use crate::settings::anim::{self, AnimSpec};
use crate::settings::pickers;
use crate::settings::schema::schema;
use crate::settings::tr::Lang;
use std::ops::Range;

/// One searchable thing, in the language on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Page id.
    pub page: String,
    pub page_name: String,
    pub title: String,
    pub desc: String,
    /// Extra words (synonyms, other-language label), space separated.
    pub keywords: String,
    /// A row or card (as opposed to the page itself).
    pub row: bool,
    /// Schema row id ("" for pages and picker cards).
    pub row_id: String,
}

impl Entry {
    pub fn new(page: &str, title: &str, desc: &str, page_name: &str) -> Entry {
        Entry { page: page.into(), page_name: page_name.into(), title: title.into(), desc: desc.into(), keywords: String::new(), row: true, row_id: String::new() }
    }
}

/// Lower-case and fold ё to е, one char to one char (so char positions of the folded text are positions in the original).
fn fold(c: char) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    if l == 'ё' {
        'е'
    } else {
        l
    }
}

/// The JS `norm`: lower case, ё -> е.
pub fn norm(s: &str) -> String {
    s.chars().map(fold).collect()
}

fn tokens(query: &str) -> Vec<String> {
    norm(query).split_whitespace().map(str::to_string).collect()
}

/// A word score against one entry, or None when the word is nowhere.
fn score_word(t: &str, title: &str, kw: &str, page: &str, desc: &str) -> Option<u32> {
    if title.starts_with(t) || title.contains(&format!(" {t}")) {
        Some(0)
    } else if title.contains(t) {
        Some(1)
    } else if kw.contains(t) {
        Some(2)
    } else if page.contains(t) {
        Some(3)
    } else if desc.contains(t) {
        Some(4)
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// Index into the entries slice that was searched.
    pub index: usize,
    pub score: u32,
    /// Matched spans of the title, in chars, sorted and merged (for bold text in the result list).
    pub title_ranges: Vec<Range<usize>>,
}

/// Ranked hits (best first), at most `max` (0 means the default 8).
pub fn search_hits(entries: &[Entry], query: &str, max: usize) -> Vec<Hit> {
    let toks = tokens(query);
    if toks.is_empty() {
        return vec![];
    }
    let mut out: Vec<Hit> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let (title, kw, page, desc) = (norm(&e.title), norm(&e.keywords), norm(&e.page_name), norm(&e.desc));
        let mut score = 0;
        let mut ok = true;
        for t in &toks {
            match score_word(t, &title, &kw, &page, &desc) {
                Some(s) => score += s,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            out.push(Hit { index: i, score, title_ranges: title_ranges(&title, &toks) });
        }
    }
    out.sort_by_key(|h| (h.score, h.index)); // stable: equal scores keep index order
    out.truncate(if max > 0 { max } else { 8 });
    out
}

/// The entries of the best hits (the JS `match`).
pub fn search<'a>(entries: &'a [Entry], query: &str, max: usize) -> Vec<&'a Entry> {
    search_hits(entries, query, max).into_iter().map(|h| &entries[h.index]).collect()
}

/// Where each word occurs in the (folded) title: every occurrence, merged where they touch or overlap.
pub fn title_ranges(folded_title: &str, toks: &[String]) -> Vec<Range<usize>> {
    let hay: Vec<char> = folded_title.chars().collect();
    let mut spans: Vec<Range<usize>> = Vec::new();
    for t in toks {
        let n: Vec<char> = t.chars().collect();
        if n.is_empty() || n.len() > hay.len() {
            continue;
        }
        for s in 0..=(hay.len() - n.len()) {
            if hay[s..s + n.len()] == n[..] {
                spans.push(s..s + n.len());
            }
        }
    }
    spans.sort_by_key(|r| (r.start, r.end));
    let mut merged: Vec<Range<usize>> = Vec::new();
    for r in spans {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

/// Ranges for an arbitrary title and query (what the renderer calls per result line).
pub fn highlight_ranges(title: &str, query: &str) -> Vec<Range<usize>> {
    title_ranges(&norm(title), &tokens(query))
}

/// The search index of the whole window in `lang`: every page, every row, the island-style cards and the pet cards.
pub fn build_index(lang: Lang) -> Vec<Entry> {
    let other = if lang == Lang::Ru { Lang::En } else { Lang::Ru };
    let mut out = Vec::new();
    for p in &schema().pages {
        let page_name = p.title.get(lang);
        out.push(Entry { page: p.id.into(), page_name: page_name.into(), title: page_name.into(), desc: String::new(), keywords: p.title.get(other).into(), row: false, row_id: String::new() });
        for r in p.rows() {
            let mut kw = vec![r.label.get(other).to_string()];
            kw.extend(r.search_keywords.iter().map(|s| s.to_string()));
            out.push(Entry { page: p.id.into(), page_name: page_name.into(), title: r.label.get(lang).into(), desc: r.hint.get(lang).into(), keywords: kw.join(" "), row: true, row_id: r.id.into() });
        }
        if p.id == "character" {
            for c in pickers::STYLES.iter() {
                out.push(Entry { page: p.id.into(), page_name: page_name.into(), title: c.name.get(lang).into(), desc: c.desc.get(lang).into(), keywords: c.name.get(other).into(), row: true, row_id: String::new() });
            }
            for c in pickers::pet_cards() {
                out.push(Entry { page: p.id.into(), page_name: page_name.into(), title: c.name.get(lang).into(), desc: c.desc.get(lang).into(), keywords: format!("{} {}", c.name.get(other), c.id), row: true, row_id: String::new() });
            }
        }
    }
    out
}

/// Where to go when a hit is picked: open `page` if it is not the current one, then (for a row) scroll to it and wash it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub page: String,
    pub row: bool,
    pub title: String,
    pub row_id: String,
}

/// The highlight of the found row: scroll it to the centre (smooth unless motion is reduced), wash it for 1.7 s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightPlan {
    pub smooth_scroll: bool,
    pub hold_ms: u32,
    pub anim: AnimSpec,
}

pub fn highlight_plan(reduce: bool) -> HighlightPlan {
    HighlightPlan { smooth_scroll: anim::scroll_smooth(reduce), hold_ms: anim::HIGHLIGHT_HOLD_MS, anim: anim::spec(anim::AnimKind::SearchHighlight, reduce) }
}

/// Keys the field reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKey {
    Escape,
    Down,
    Up,
    Enter,
}

/// What the renderer should do after an input or key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchStep {
    None,
    /// The list closed (clear the field text too when `clear`).
    Close { clear: bool },
    /// Take focus away from the field (Escape on an empty, closed field).
    Blur,
    /// Repaint the list. `fresh`: the hits changed, play the stagger (not on arrow keys).
    Paint { fresh: bool },
    Go(Target),
}

pub const MAX_HITS: usize = 8;

#[derive(Debug, Default)]
pub struct SearchBox {
    pub query: String,
    pub hits: Vec<Hit>,
    pub active: Option<usize>,
    pub open: bool,
}

impl SearchBox {
    /// Ctrl+F (without Shift/Alt) focuses and selects the field.
    pub fn is_find_shortcut(ctrl: bool, meta: bool, shift: bool, alt: bool, key: &str) -> bool {
        (ctrl || meta) && !shift && !alt && key.eq_ignore_ascii_case("f")
    }

    /// The text changed. A blank query closes the list.
    pub fn input(&mut self, entries: &[Entry], text: &str) -> SearchStep {
        self.query = text.to_string();
        if text.trim().is_empty() {
            return self.close(false);
        }
        self.hits = search_hits(entries, text, MAX_HITS);
        self.active = if self.hits.is_empty() { None } else { Some(0) };
        self.open = true;
        SearchStep::Paint { fresh: true }
    }

    pub fn close(&mut self, clear: bool) -> SearchStep {
        if clear {
            self.query.clear();
        }
        self.open = false;
        self.hits.clear();
        self.active = None;
        SearchStep::Close { clear }
    }

    pub fn key(&mut self, entries: &[Entry], key: SearchKey) -> SearchStep {
        if key == SearchKey::Escape {
            return if !self.query.is_empty() || self.open { self.close(true) } else { SearchStep::Blur };
        }
        if self.hits.is_empty() {
            return SearchStep::None;
        }
        let n = self.hits.len();
        match key {
            SearchKey::Down | SearchKey::Up => {
                let cur = self.active.unwrap_or(0);
                self.active = Some(if key == SearchKey::Down { (cur + 1) % n } else { (cur + n - 1) % n });
                SearchStep::Paint { fresh: false }
            }
            SearchKey::Enter => self.pick(entries, self.active.unwrap_or(0)),
            SearchKey::Escape => SearchStep::None,
        }
    }

    /// A click (mouse down) on hit `i`, or Enter.
    pub fn pick(&mut self, entries: &[Entry], i: usize) -> SearchStep {
        let Some(h) = self.hits.get(i) else { return SearchStep::None };
        let e = &entries[h.index];
        let t = Target { page: e.page.clone(), row: e.row, title: e.title.clone(), row_id: e.row_id.clone() };
        self.close(true);
        SearchStep::Go(t)
    }

    /// The small grey text right of a result: the page name for rows, "Page" for a page entry.
    pub fn result_tag(e: &Entry, lang: Lang) -> String {
        if e.row {
            e.page_name.clone()
        } else {
            crate::settings::tr::pick(lang, "Страница", "Page").to_string()
        }
    }
}

pub const PLACEHOLDER: crate::settings::tr::Tr = crate::settings::tr::tr("Поиск (Ctrl+F)", "Search (Ctrl+F)");
pub const ARIA_LABEL: crate::settings::tr::Tr = crate::settings::tr::tr("Поиск по настройкам", "Search settings");
pub const NOTHING_FOUND: crate::settings::tr::Tr = crate::settings::tr::tr("Ничего не найдено", "Nothing found");

#[cfg(test)]
#[allow(clippy::single_range_in_vec_init)]
mod tests {
    use super::*;

    fn e(page: &str, title: &str, desc: &str, page_name: &str) -> Entry {
        Entry::new(page, title, desc, page_name)
    }

    fn rows() -> Vec<Entry> {
        vec![
            e("appearance", "Размер острова", "На большом мониторе остров можно сделать крупнее", "Внешний вид"),
            e("appearance", "Прозрачность свёрнутого острова", "Пилюля и её карточки", "Внешний вид"),
            e("behavior", "Прятать остров, пока он не нужен", "Свёрнутый остров прячется за край экрана", "Поведение"),
            e("behavior", "Тихие часы", "Пока тихо, карточки событий не показываются", "Поведение"),
            e("general", "Язык", "По умолчанию — как в Windows", "Основное"),
            e("search", "Запускать Everything сам", "поиск файлов", "Поиск файлов"),
            e("modules", "Поиск файлов в командной строке", "", "Модули"),
        ]
    }

    fn titles(rows: &[Entry], q: &str, n: usize) -> Vec<String> {
        search(rows, q, n).into_iter().map(|r| r.title.clone()).collect()
    }

    // ---- the cases of settings-search.test.mjs, one to one ----

    #[test]
    fn js_empty_and_unknown_queries_find_nothing() {
        let r = rows();
        assert!(titles(&r, "", 0).is_empty());
        assert!(titles(&r, "   ", 0).is_empty());
        assert!(titles(&r, "zzz", 0).is_empty());
        assert!(search(&[], "a", 0).is_empty(), "match(null, 'a') is empty");
    }

    #[test]
    fn js_case_and_yo_are_ignored_and_title_beats_description() {
        let r = rows();
        assert_eq!(titles(&r, "свернутого", 0), ["Прозрачность свёрнутого острова"]);
        assert_eq!(titles(&r, "ОСТРОВ", 0)[0], "Размер острова");
        assert_eq!(titles(&r, "язык", 0)[0], "Язык");
    }

    #[test]
    fn js_every_word_must_match_in_title_page_or_description() {
        let r = rows();
        assert_eq!(titles(&r, "размер остров", 0), ["Размер острова"]);
        assert_eq!(titles(&r, "поведение тихие", 0), ["Тихие часы"]);
        assert_eq!(titles(&r, "карточки пилюля", 0), ["Прозрачность свёрнутого острова"]);
        assert_eq!(titles(&r, "карточки тихо", 0), ["Тихие часы"]);
    }

    #[test]
    fn js_title_hits_rank_before_page_name_before_description() {
        let r = rows();
        assert_eq!(titles(&r, "поиск файлов", 0), ["Поиск файлов в командной строке", "Запускать Everything сам"]);
        assert_eq!(titles(&r, "windows", 0)[0], "Язык");
    }

    #[test]
    fn js_the_cap_and_the_default_cap() {
        let many: Vec<Entry> = (0..30).map(|i| e("p", &format!("Row {i}"), "", "p")).collect();
        assert_eq!(search(&many, "row", 5).len(), 5);
        assert_eq!(search(&many, "row", 0).len(), 8);
        assert_eq!(search(&many, "row 7", 0)[0].title, "Row 7");
    }

    // ---- additions: keywords, English, highlight ranges, index, box ----

    #[test]
    fn keywords_rank_between_title_and_page_name() {
        let mut a = e("p", "Alpha", "", "Page");
        a.keywords = "quiet".into();
        let b = e("p", "Beta", "quiet text", "Page");
        let c = e("p", "Quiet row", "", "Page");
        let r = vec![a, b, c];
        assert_eq!(titles(&r, "quiet", 0), ["Quiet row", "Alpha", "Beta"]);
        assert_eq!(search_hits(&r, "quiet", 0).iter().map(|h| h.score).collect::<Vec<_>>(), [0, 2, 4]);
    }

    #[test]
    fn english_matches_like_russian() {
        let r = vec![e("appearance", "Island size", "On a large monitor the pill can be made bigger", "Appearance"), e("behavior", "Quiet hours", "While it is quiet, event cards are not shown", "Behavior")];
        assert_eq!(titles(&r, "SIZE", 0), ["Island size"]);
        assert_eq!(titles(&r, "behavior quiet", 0), ["Quiet hours"]);
        assert_eq!(titles(&r, "cards", 0), ["Quiet hours"]);
        assert!(titles(&r, "nothing like it", 0).is_empty());
    }

    #[test]
    fn highlight_ranges_cover_every_word_occurrence() {
        assert_eq!(highlight_ranges("Прозрачность свёрнутого острова", "свернутого"), vec![13..23]);
        assert_eq!(highlight_ranges("Размер острова", "остров размер"), vec![0..6, 7..13]);
        assert_eq!(highlight_ranges("Quiet quiet", "quiet"), vec![0..5, 6..11]);
        assert_eq!(highlight_ranges("aaa", "aa"), vec![0..3], "overlapping hits merge");
        assert!(highlight_ranges("Язык", "zzz").is_empty());
        assert!(highlight_ranges("Язык", "").is_empty());
    }

    #[test]
    fn hits_carry_their_ranges() {
        let r = rows();
        let h = search_hits(&r, "тихие", 0);
        assert_eq!((h.len(), r[h[0].index].title.as_str(), h[0].title_ranges.clone()), (1, "Тихие часы", vec![0..5]));
    }

    #[test]
    fn index_covers_pages_rows_and_cards_in_both_languages() {
        for lang in Lang::ALL {
            let idx = build_index(lang);
            let pages = idx.iter().filter(|e| !e.row).count();
            assert_eq!(pages, schema().pages.len());
            assert!(idx.iter().all(|e| !e.title.is_empty()));
            assert!(idx.iter().any(|e| e.row_id == "quiet.on"));
            assert!(idx.iter().any(|e| e.page == "character" && e.row_id.is_empty() && e.row), "picker cards are indexed");
        }
        // a Russian UI finds rows by their English label and the other way round
        let ru = build_index(Lang::Ru);
        assert!(search(&ru, "quiet hours", 3).iter().any(|e| e.row_id == "quiet.on"));
        let en = build_index(Lang::En);
        assert!(search(&en, "тихие часы", 3).iter().any(|e| e.row_id == "quiet.on"));
        assert!(search(&ru, "размер острова", 1).first().is_some_and(|e| e.row_id == "size"));
        assert!(search(&en, "frame rate", 3).iter().any(|e| e.row_id == "fps"));
        assert!(search(&ru, "ollama", 8).iter().any(|e| e.page == "ai" || e.page == "aisetup"));
    }

    #[test]
    fn search_box_state_machine() {
        let r = rows();
        let mut b = SearchBox::default();
        assert_eq!(b.key(&r, SearchKey::Down), SearchStep::None);
        assert_eq!(b.key(&r, SearchKey::Escape), SearchStep::Blur, "Escape on an idle field blurs it");
        assert_eq!(b.input(&r, "остров"), SearchStep::Paint { fresh: true });
        assert!(b.open && b.active == Some(0) && b.hits.len() == 3);
        assert_eq!(b.key(&r, SearchKey::Down), SearchStep::Paint { fresh: false });
        assert_eq!(b.active, Some(1));
        b.key(&r, SearchKey::Down);
        assert_eq!(b.key(&r, SearchKey::Down), SearchStep::Paint { fresh: false });
        assert_eq!(b.active, Some(0), "arrows wrap down");
        b.key(&r, SearchKey::Up);
        assert_eq!(b.active, Some(2), "and up");
        b.active = Some(0);
        match b.key(&r, SearchKey::Enter) {
            SearchStep::Go(t) => assert_eq!((t.page.as_str(), t.title.as_str(), t.row), ("appearance", "Размер острова", true)),
            s => panic!("{s:?}"),
        }
        assert!(!b.open && b.query.is_empty() && b.hits.is_empty(), "going clears the field and closes");
        b.input(&r, "zzz");
        assert!(b.hits.is_empty() && b.active.is_none() && b.open, "no hits: the list shows 'Nothing found'");
        assert_eq!(b.key(&r, SearchKey::Enter), SearchStep::None);
        assert_eq!(b.key(&r, SearchKey::Escape), SearchStep::Close { clear: true });
        assert_eq!(b.input(&r, "   "), SearchStep::Close { clear: false }, "a blank query closes without clearing");
    }

    #[test]
    fn find_shortcut_and_tags() {
        assert!(SearchBox::is_find_shortcut(true, false, false, false, "F"));
        assert!(SearchBox::is_find_shortcut(false, true, false, false, "f"));
        assert!(!SearchBox::is_find_shortcut(true, false, true, false, "f"));
        assert!(!SearchBox::is_find_shortcut(true, false, false, true, "f"));
        assert!(!SearchBox::is_find_shortcut(false, false, false, false, "f"));
        let mut page = e("p", "Page", "", "Page");
        page.row = false;
        assert_eq!(SearchBox::result_tag(&page, Lang::Ru), "Страница");
        assert_eq!(SearchBox::result_tag(&e("p", "Row", "", "Behavior"), Lang::En), "Behavior");
    }

    #[test]
    fn highlight_follows_reduce_motion() {
        assert!(highlight_plan(false).smooth_scroll && !highlight_plan(true).smooth_scroll);
        assert_eq!(highlight_plan(false).hold_ms, 1700);
        assert!(highlight_plan(true).anim.is_instant());
    }
}
