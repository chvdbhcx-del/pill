//! Command bar bookmarks and prefix templates (`command.shortcuts` in Settings > General > Command bar search): one entry per line,
//! `prefix = address`. An address with `{query}` is a template: `g = https://www.google.com/search?q={query}` turns `g rust lifetimes` into
//! that page with the words percent-encoded in. An address without it is a bookmark: `gh = https://github.com` opens when the bar says just `gh`.
//! At most [`MAX`] entries are read. Everything is local: nothing is sent anywhere until the user presses Enter on the row, and then it is
//! only opened in the default browser.
//!
//! Pure: no settings, no window (`configured` is the one reader of settings).
use super::pct_encode;

pub const MAX: usize = 20;
/// The place the typed words go.
pub const SLOT: &str = "{query}";
const MAX_PREFIX: usize = 16;
const MAX_URL: usize = 2000;

#[derive(Clone, Debug, PartialEq)]
pub struct Shortcut {
    /// lower-case, no spaces
    pub prefix: String,
    /// an http(s) address, with `{query}` or without
    pub address: String,
}

impl Shortcut {
    pub fn is_template(&self) -> bool {
        self.address.contains(SLOT)
    }

    /// The address to open for `query` (percent-encoded into every `{query}`).
    pub fn expand(&self, query: &str) -> String {
        self.address.replace(SLOT, &pct_encode(query))
    }

    /// The site shown in the row: the host of the address.
    pub fn site(&self) -> &str {
        self.address.split("//").nth(1).and_then(|r| r.split(['/', '?', '#']).next()).filter(|h| !h.is_empty()).unwrap_or(&self.address)
    }
}

/// `g = https://...{query}` -> the shortcut; `None` for anything that is not a valid entry (no `=`, a prefix with spaces or too long, an address
/// that is not http(s) or is huge).
pub fn parse_line(line: &str) -> Option<Shortcut> {
    let (p, a) = line.split_once('=')?;
    let (p, a) = (p.trim().to_lowercase(), a.trim());
    let ok_prefix = !p.is_empty() && p.chars().count() <= MAX_PREFIX && p.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'));
    let lower = a.to_ascii_lowercase();
    let ok_addr = a.len() < MAX_URL && (lower.starts_with("https://") || lower.starts_with("http://")) && a.len() > "https://".len() && !a.contains(char::is_whitespace);
    (ok_prefix && ok_addr).then(|| Shortcut { prefix: p, address: a.to_string() })
}

/// The valid entries of the list, in order, the first [`MAX`]; a prefix that is already taken by an earlier entry is dropped.
pub fn parse(lines: &[String]) -> Vec<Shortcut> {
    let mut out: Vec<Shortcut> = Vec::new();
    for l in lines {
        if out.len() >= MAX {
            break;
        }
        if let Some(s) = parse_line(l) {
            if !out.iter().any(|o| o.prefix == s.prefix) {
                out.push(s);
            }
        }
    }
    out
}

/// What the text typed in the bar means: the shortcut and the words after its prefix (`""` for a bookmark). A template needs words
/// (`g` alone is still a plain search); a bookmark answers only to its bare prefix.
pub fn find<'a>(list: &'a [Shortcut], typed: &str) -> Option<(&'a Shortcut, String)> {
    let t = typed.trim();
    let (head, rest) = match t.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (t, ""),
    };
    let s = list.iter().find(|s| s.prefix == head.to_lowercase())?;
    match (s.is_template(), rest.is_empty()) {
        (true, false) => Some((s, rest.to_string())),
        (false, true) => Some((s, String::new())),
        _ => None,
    }
}

/// `command.shortcuts` from settings.json.
pub fn configured() -> Vec<Shortcut> {
    let lines: Vec<String> = crate::settings::settings_get()
        .get("command.shortcuts")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    parse(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(x: &[&str]) -> Vec<String> {
        x.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_line_is_a_prefix_and_an_http_address() {
        let s = parse_line(" G = https://www.google.com/search?q={query} ").unwrap();
        assert_eq!((s.prefix.as_str(), s.address.as_str()), ("g", "https://www.google.com/search?q={query}"), "the first = splits, the address keeps its own");
        assert!(s.is_template() && s.site() == "www.google.com");
        let long = format!("{} = https://x.com", "a".repeat(17));
        for bad in ["", "g", "g = ", "g = ftp://x/{query}", "g = javascript:alert(1)", "g h = https://x.com", "= https://x.com", "g = https://", "g = https://a b.com", long.as_str()] {
            assert_eq!(parse_line(bad), None, "{bad:?}");
        }
        assert!(parse_line("yt.music-1_x = http://x.com").is_some());
    }

    #[test]
    fn the_query_is_percent_encoded_into_the_slot() {
        let s = parse_line("w = https://en.wikipedia.org/wiki/Special:Search?search={query}&x={query}").unwrap();
        assert_eq!(s.expand("rust & c++ é"), "https://en.wikipedia.org/wiki/Special:Search?search=rust+%26+c%2B%2B+%C3%A9&x=rust+%26+c%2B%2B+%C3%A9");
        let b = parse_line("gh = https://github.com").unwrap();
        assert_eq!(b.expand("ignored"), "https://github.com", "a bookmark has no slot");
    }

    #[test]
    fn the_list_is_capped_deduplicated_and_ordered() {
        let mut lines = l(&["g = https://a.com/{query}", "g = https://other.com/{query}", "junk"]);
        for i in 0..30 {
            lines.push(format!("p{i} = https://x.com/{i}"));
        }
        let list = parse(&lines);
        assert_eq!(list.len(), MAX, "no more than 20");
        assert_eq!(list[0].address, "https://a.com/{query}", "the first entry of a prefix wins");
        assert_eq!(list[1].prefix, "p0");
    }

    #[test]
    fn typing_picks_templates_with_words_and_bookmarks_alone() {
        let list = parse(&l(&["g = https://www.google.com/search?q={query}", "gh = https://github.com"]));
        let (s, q) = find(&list, "g  rust lifetimes ").unwrap();
        assert_eq!((s.prefix.as_str(), q.as_str()), ("g", "rust lifetimes"));
        assert_eq!(find(&list, "G Rust").unwrap().1, "Rust", "the prefix is case-insensitive, the words keep theirs");
        assert!(find(&list, "g").is_none(), "a template without words is a plain search");
        let (b, q) = find(&list, "gh").unwrap();
        assert_eq!((b.prefix.as_str(), q.as_str()), ("gh", ""));
        assert!(find(&list, "gh issues").is_none(), "a bookmark takes no words");
        assert!(find(&list, "google").is_none(), "only the whole first word counts");
        assert!(find(&[], "g x").is_none());
    }
}
