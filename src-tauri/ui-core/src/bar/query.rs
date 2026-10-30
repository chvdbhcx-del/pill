//! Parsing of what is typed into the bar: the `? question` / `ии question` / `ai question` AI prefix, the `>` shell and `=` calc prefixes, the
//! one-letter filters (`c n f a s`), the `u <name>` uninstall trigger, the highlight words and the match highlighting of a result title.
//! Pure string work; all of it is Unicode-aware (Cyrillic queries and titles highlight the same as Latin ones).

/// Keystrokes inside this window collapse into one `cmd_query` (`setTimeout(..., 80)` in bar.js).
pub const DEBOUNCE_MS: f64 = 80.0;

/// `ASK = /^(?:\?|ии|ai)\s+/i`: returns the text after the prefix and the blanks that follow it, `None` when the line is not an AI question.
/// `?`, `ии` and `ai` need a blank after them (a bare `ai` is a normal search); the case does not matter (`ИИ`, `Ai`).
pub fn ask_prefix(text: &str) -> Option<&str> {
    let (word, rest) = split_first_blank(text)?;
    let w = lower(word);
    (w == "?" || w == "ии" || w == "ai").then(|| rest.trim_start())
}

/// Splits at the first whitespace char: (word before it, everything after it).
fn split_first_blank(text: &str) -> Option<(&str, &str)> {
    let i = text.find(char::is_whitespace)?;
    let ws = text[i..].chars().next()?;
    Some((&text[..i], &text[i + ws.len_utf8()..]))
}

/// Lowercase that never changes the number of chars (so char indexes of the lowered text are valid in the original).
fn lower_chars(s: &str) -> Vec<char> {
    s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}
fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// What the line means for the bar itself (the backend decides the rows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    /// an AI question: `question` is trimmed and may be empty (the tip about the prefix is shown then)
    Ask { question: String },
    /// a search sent to `cmd_query`
    Search,
}

/// Classifies a (left-trimmed) line.
pub fn classify(text: &str) -> Input {
    match ask_prefix(text) {
        Some(rest) => Input::Ask { question: rest.trim().to_string() },
        None => Input::Search,
    }
}

/// Words that are highlighted in titles: none for a `>` command, otherwise the lowercase words of the line without its `=` or one-letter
/// filter (`/^(=|[cnfas]\s)/i`).
pub fn highlight_words(text: &str) -> Vec<String> {
    if text.starts_with('>') {
        return Vec::new();
    }
    let rest = strip_filter(text);
    lower(rest).split_whitespace().map(str::to_string).collect()
}

/// Removes a leading `=` or a one-letter filter plus its blank (`c `, `n `, `f `, `a `, `s `), any case.
pub fn strip_filter(text: &str) -> &str {
    let mut it = text.chars();
    match it.next() {
        Some('=') => it.as_str(),
        Some(c) if "cnfas".contains(c.to_ascii_lowercase()) && c.is_ascii() && it.clone().next().is_some_and(char::is_whitespace) => {
            let mut r = it;
            r.next();
            r.as_str()
        }
        _ => text,
    }
}

/// The backend's search scopes (`command.rs: Scope`); the bar uses it to describe the active filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    Clip,
    Notes,
    Files,
    Apps,
    Settings,
    Calc,
}

/// `= 2+2` -> Calc, `c foo` -> Clip ... `parse_scope` of command.rs.
pub fn scope_of(text: &str) -> Scope {
    let q = text.trim_start();
    if q.starts_with('=') {
        return Scope::Calc;
    }
    let mut it = q.chars();
    if let (Some(c), Some(sp)) = (it.next(), it.next()) {
        if sp.is_whitespace() {
            return match c.to_ascii_lowercase() {
                'c' => Scope::Clip,
                'n' => Scope::Notes,
                'f' => Scope::Files,
                'a' => Scope::Apps,
                's' => Scope::Settings,
                _ => Scope::All,
            };
        }
    }
    Scope::All
}

/// Words that open the uninstall rows: `u foo`, `del foo`, `uninstall foo`, `удалить foo`, ... (`uninstall::trigger`). Returns the app query.
pub fn uninstall_trigger(text: &str) -> Option<&str> {
    const WORDS: &[&str] = &["u", "del", "uninstall", "remove", "удалить", "удали", "удаление", "деинсталляция", "деинсталлировать", "снести"];
    let (w, rest) = split_first_blank(text.trim_start())?;
    WORDS.contains(&lower(w).as_str()).then(|| rest.trim())
}

/// The box content the row's trash button / context-menu «Удалить…» puts into the input: `u ` + the title with runs of blanks collapsed.
pub fn uninstall_query_for(title: &str) -> String {
    format!("u {}", title.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// While a dialogue is open, a leading `>` or `=` ends it and searches as usual (`dlgInput`).
pub fn leaves_dialogue(raw: &str) -> bool {
    raw.starts_with('>') || raw.starts_with('=')
}

/// The follow-up question typed in a dialogue: the line without an optional `? ` / `ии ` / `ai ` prefix, trimmed.
pub fn follow_up(raw: &str) -> String {
    match ask_prefix(raw) {
        Some(rest) => rest.trim().to_string(),
        None => raw.trim().to_string(),
    }
}

/// One piece of a highlighted title: plain text or a match (`<mark>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub mark: bool,
}

/// Case-insensitive match ranges (char indexes) of every word in `text`, merged exactly like bar.js `hl()`: sorted by start, overlaps trimmed.
pub fn match_ranges(text: &str, words: &[String]) -> Vec<(usize, usize)> {
    let low = lower_chars(text);
    let mut r: Vec<(usize, usize)> = Vec::new();
    for w in words {
        let wl = lower_chars(w);
        if wl.is_empty() || wl.len() > low.len() {
            continue;
        }
        let mut i = 0;
        while i + wl.len() <= low.len() {
            if low[i..i + wl.len()] == wl[..] {
                r.push((i, i + wl.len()));
                i += wl.len();
            } else {
                i += 1;
            }
        }
    }
    r.sort_by_key(|x| x.0);
    r
}

/// Splits a title into plain / marked segments for the words (text nodes only, never markup).
pub fn highlight(text: &str, words: &[String]) -> Vec<Seg> {
    let chars: Vec<char> = text.chars().collect();
    let take = |a: usize, b: usize| chars[a.min(chars.len())..b.min(chars.len())].iter().collect::<String>();
    let mut out = Vec::new();
    let mut pos = 0;
    for (s, e) in match_ranges(text, words) {
        if e <= pos {
            continue;
        }
        let s2 = s.max(pos);
        if s2 > pos {
            out.push(Seg { text: take(pos, s2), mark: false });
        }
        out.push(Seg { text: take(s2, e), mark: true });
        pos = e;
    }
    if pos < chars.len() || out.is_empty() {
        out.push(Seg { text: take(pos, chars.len()), mark: false });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ask_prefixes() {
        assert_eq!(ask_prefix("? why is the sky blue"), Some("why is the sky blue"));
        assert_eq!(ask_prefix("ai   hello"), Some("hello"));
        assert_eq!(ask_prefix("AI hello"), Some("hello"));
        assert_eq!(ask_prefix("ии что такое rust"), Some("что такое rust"));
        assert_eq!(ask_prefix("ИИ что"), Some("что"));
        assert_eq!(ask_prefix("Ии что"), Some("что"));
        assert_eq!(ask_prefix("ai"), None, "a bare prefix without a blank is a normal search");
        assert_eq!(ask_prefix("?why"), None);
        assert_eq!(ask_prefix("airport"), None);
        assert_eq!(ask_prefix("ai "), Some(""), "a trailing blank is enough: the tip about the prefix shows");
        assert_eq!(ask_prefix("hello ai x"), None);
    }

    #[test]
    fn classify_questions() {
        assert_eq!(classify("? hi there "), Input::Ask { question: "hi there".into() });
        assert_eq!(classify("ии "), Input::Ask { question: String::new() });
        assert_eq!(classify("5*8"), Input::Search);
        assert_eq!(classify("> ipconfig"), Input::Search);
    }

    #[test]
    fn filters_strip_from_the_words() {
        assert_eq!(highlight_words("c буфер текст"), vec!["буфер", "текст"]);
        assert_eq!(highlight_words("C Foo"), vec!["foo"]);
        assert_eq!(highlight_words("= 2 + 2"), vec!["2", "+", "2"]);
        assert_eq!(highlight_words("> dir /w"), Vec::<String>::new());
        assert_eq!(highlight_words("cnn"), vec!["cnn"], "a filter letter needs a blank after it");
        assert_eq!(highlight_words("a"), vec!["a"]);
        assert_eq!(highlight_words("Блокнот ЗАМЕТКИ"), vec!["блокнот", "заметки"]);
        assert_eq!(highlight_words("   "), Vec::<String>::new());
        // a trailing blank is kept by the caller: "c " selects the clipboard filter and leaves no words
        assert_eq!(highlight_words("c "), Vec::<String>::new());
        assert_eq!(strip_filter("s dark"), "dark");
        assert_eq!(strip_filter("x dark"), "x dark");
        assert_eq!(strip_filter("с dark"), "с dark", "Cyrillic с is not the Latin filter letter");
    }

    #[test]
    fn scopes() {
        assert_eq!(scope_of("= 1+1"), Scope::Calc);
        assert_eq!(scope_of("c hello"), Scope::Clip);
        assert_eq!(scope_of("N todo"), Scope::Notes);
        assert_eq!(scope_of("f report"), Scope::Files);
        assert_eq!(scope_of("a code"), Scope::Apps);
        assert_eq!(scope_of("s theme"), Scope::Settings);
        assert_eq!(scope_of("cc hello"), Scope::All);
        assert_eq!(scope_of("hello"), Scope::All);
        assert_eq!(scope_of("  c x"), Scope::Clip);
    }

    #[test]
    fn uninstall_triggers_including_cyrillic() {
        assert_eq!(uninstall_trigger("u qbittorrent"), Some("qbittorrent"));
        assert_eq!(uninstall_trigger("U  Foo Bar "), Some("Foo Bar"));
        assert_eq!(uninstall_trigger("uninstall foo"), Some("foo"));
        assert_eq!(uninstall_trigger("удалить телеграм"), Some("телеграм"));
        assert_eq!(uninstall_trigger("Удалить Телеграм"), Some("Телеграм"));
        assert_eq!(uninstall_trigger("снести хром"), Some("хром"));
        assert_eq!(uninstall_trigger("u"), None, "the word alone is not a trigger");
        assert_eq!(uninstall_trigger("ubuntu foo"), None);
        assert_eq!(uninstall_trigger("u "), Some(""), "`u ` lists installed apps");
        assert_eq!(uninstall_query_for("Visual   Studio\tCode "), "u Visual Studio Code");
        assert_eq!(uninstall_query_for("Тест  программа"), "u Тест программа");
    }

    #[test]
    fn dialogue_inputs() {
        assert!(leaves_dialogue("> ls"));
        assert!(leaves_dialogue("=2+2"));
        assert!(!leaves_dialogue("and then?"));
        assert_eq!(follow_up("? and then"), "and then");
        assert_eq!(follow_up("  more detail  "), "more detail");
        assert_eq!(follow_up("ии подробнее"), "подробнее");
        assert_eq!(follow_up(""), "");
    }

    fn marks(text: &str, words: &[&str]) -> Vec<(String, bool)> {
        let w: Vec<String> = words.iter().map(|s| s.to_string()).collect();
        highlight(text, &w).into_iter().map(|s| (s.text, s.mark)).collect()
    }

    #[test]
    fn highlight_latin_and_cyrillic() {
        assert_eq!(marks("Visual Studio Code", &["studio"]), vec![("Visual ".into(), false), ("Studio".into(), true), (" Code".into(), false)]);
        assert_eq!(marks("Блокнот заметки", &["зам"]), vec![("Блокнот ".into(), false), ("зам".into(), true), ("етки".into(), false)]);
        assert_eq!(marks("ЗАМЕТКИ", &["замет"]), vec![("ЗАМЕТ".into(), true), ("КИ".into(), false)]);
        assert_eq!(marks("Привет", &["xyz"]), vec![("Привет".into(), false)]);
        assert_eq!(marks("abc", &[]), vec![("abc".into(), false)]);
        assert_eq!(marks("", &["a"]), vec![("".into(), false)]);
    }

    #[test]
    fn highlight_merges_overlaps_and_repeats() {
        // overlapping words: the second match only contributes the part after the first
        assert_eq!(marks("foobar", &["foob", "obar"]), vec![("foob".into(), true), ("ar".into(), true)]);
        // every non-overlapping occurrence of a word
        assert_eq!(marks("a-b-a", &["a"]), vec![("a".into(), true), ("-b-".into(), false), ("a".into(), true)]);
        // a word longer than the title never matches
        assert_eq!(marks("ab", &["abc"]), vec![("ab".into(), false)]);
    }
}
