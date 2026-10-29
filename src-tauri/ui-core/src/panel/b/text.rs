//! Text tab: pure text tools (exact ports of src/tab-text.js, checked against the JS output in text_js_reference.json and
//! the cases of src/tab-text.test.mjs) and the tab's view-model (tools grid, AI grid, counter line, toast, QR box).
//!
//! The text area itself is the kit's multi-line TextInput: the renderer reports its contents/selection with
//! `Action::Input` (selection in *char* indices), and edits made by tools go back as `Effect::ReplaceRange` so the input's own
//! undo restores the original (what `document.execCommand('insertText')` did). AI requests go through `ai_transform`
//! (Rust: aiactions.rs); a second AI click while one runs cancels it.
//!
//! ponytail: `sort_lines` approximates `Intl.Collator(undefined, {numeric, sensitivity:'base'})` (case/diacritics ignored,
//! digit runs compared by value, symbols < digits < Latin < Greek < Cyrillic); exotic scripts compare by code point.

use super::jsonlite;
use super::qr::{self, Qr};
use super::{clip_chars, plural, Anim, BackendCall, Effect, Flash, Fx, Lang, Reply, Tags, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

// ================================================================= pure tools

/// JS `\s` (also what `String.prototype.trim` strips).
pub fn js_ws(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}
fn js_trim(s: &str) -> &str {
    s.trim_matches(js_ws)
}
/// split on `\r\n | \r | \n`
fn lines_of(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let (mut st, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'\r' || b[i] == b'\n' {
            out.push(&s[st..i]);
            if b[i] == b'\r' && b.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            st = i + 1;
        }
        i += 1;
    }
    out.push(&s[st..]);
    out
}

const EN_LO: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,./`";
const RU_LO: &str = "йцукенгшщзхъфывапролджэячсмить бю.ё";
const EN_UP: &str = "QWERTYUIOP{}ASDFGHJKL:\"ZXCVBNM<>?~";
const RU_UP: &str = "ЙЦУКЕНГШЩЗХЪФЫВАПРОЛДЖЭЯЧСМИТЬБЮ,Ё";

fn layout_maps() -> (HashMap<char, char>, HashMap<char, char>) {
    let ru_lo: String = RU_LO.replace(' ', "");
    let en: Vec<char> = EN_LO.chars().chain(EN_UP.chars()).chain("@#$^&".chars()).collect();
    let ru: Vec<char> = ru_lo.chars().chain(RU_UP.chars()).chain("\"№;:?".chars()).collect();
    let (mut en2ru, mut ru2en) = (HashMap::new(), HashMap::new());
    for (e, r) in en.iter().zip(ru.iter()) {
        en2ru.insert(*e, *r);
        ru2en.insert(*r, *e);
    }
    (en2ru, ru2en)
}
fn is_cyr(c: char) -> bool {
    matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
}

/// RU<->EN keyboard layout swap; direction is detected (more Cyrillic letters than Latin = RU->EN).
pub fn layout_fix(s: &str) -> String {
    let cyr = s.chars().filter(|c| is_cyr(*c)).count();
    let lat = s.chars().filter(|c| c.is_ascii_alphabetic()).count();
    let (en2ru, ru2en) = layout_maps();
    let map = if cyr > lat { &ru2en } else { &en2ru };
    s.chars().map(|c| *map.get(&c).unwrap_or(&c)).collect()
}

fn t_ru(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' => "e", 'ё' => "yo", 'ж' => "zh", 'з' => "z", 'и' => "i", 'й' => "y",
        'к' => "k", 'л' => "l", 'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u", 'ф' => "f",
        'х' => "kh", 'ц' => "ts", 'ч' => "ch", 'ш' => "sh", 'щ' => "shch", 'ъ' => "", 'ы' => "y", 'ь' => "", 'э' => "e", 'ю' => "yu", 'я' => "ya",
        _ => return None,
    })
}
const T_LAT: [(&str, &str); 36] = [
    ("shch", "щ"), ("sch", "щ"), ("yo", "ё"), ("zh", "ж"), ("kh", "х"), ("ts", "ц"), ("ch", "ч"), ("sh", "ш"), ("yu", "ю"), ("ya", "я"),
    ("a", "а"), ("b", "б"), ("c", "к"), ("d", "д"), ("e", "е"), ("f", "ф"), ("g", "г"), ("h", "х"), ("i", "и"), ("j", "й"), ("k", "к"), ("l", "л"), ("m", "м"),
    ("n", "н"), ("o", "о"), ("p", "п"), ("q", "к"), ("r", "р"), ("s", "с"), ("t", "т"), ("u", "у"), ("v", "в"), ("w", "в"), ("x", "кс"), ("y", "ы"), ("z", "з"),
];
fn is_up(c: Option<char>) -> bool {
    match c {
        None => false,
        Some(c) => {
            let lo: String = c.to_lowercase().collect();
            let up: String = c.to_uppercase().collect();
            let me = c.to_string();
            lo != me && up == me
        }
    }
}
fn cap(x: &str, whole: bool) -> String {
    if whole {
        x.to_uppercase()
    } else {
        let mut it = x.chars();
        match it.next() {
            Some(f) => f.to_uppercase().collect::<String>() + it.as_str(),
            None => String::new(),
        }
    }
}
fn lower_char(c: char) -> String {
    c.to_lowercase().collect()
}

/// Transliteration: Cyrillic -> Latin, otherwise Latin -> Cyrillic (greedy digraphs, case kept).
pub fn translit(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.iter().any(|c| is_cyr(*c)) {
        let letter = |c: Option<char>| c.map_or(false, |c| c.is_ascii_alphabetic() || is_cyr(c));
        return chars
            .iter()
            .enumerate()
            .map(|(i, &c)| {
                let lo = lower_char(c);
                let v = if lo.chars().count() == 1 { t_ru(lo.chars().next().unwrap()) } else { None };
                let Some(v) = v else { return c.to_string() };
                if !is_up(Some(c)) {
                    return v.to_string();
                }
                let next = chars.get(i + 1).copied();
                let prev = if i > 0 { chars.get(i - 1).copied() } else { None };
                cap(v, is_up(next) || (is_up(prev) && !letter(next)))
            })
            .collect();
    }
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..chars.len().min(i + 4)].iter().map(|c| lower_char(*c)).collect();
        let Some((k, v)) = T_LAT.iter().find(|(k, _)| rest.starts_with(k)) else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        let kl = k.len();
        let first_up = is_up(Some(chars[i]));
        if first_up {
            let second_up = kl > 1 && is_up(chars.get(i + 1).copied());
            out.push_str(&cap(v, second_up || is_up(chars.get(i + kl).copied())));
        } else {
            out.push_str(v);
        }
        i += kl;
    }
    out
}

pub fn title_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut in_word = false;
    for c in lower.chars() {
        if in_word {
            if c.is_alphabetic() || c.is_numeric() || c == '\'' || c == '’' {
                out.push(c);
            } else {
                in_word = false;
                out.push(c);
            }
        } else if c.is_alphabetic() {
            in_word = true;
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Capital letter at the start of the text and after `. ! ? …` + spaces or a new line (everything else lowercased).
pub fn sentence_case(s: &str) -> String {
    let lower: Vec<char> = s.to_lowercase().chars().collect();
    let mut out = String::with_capacity(lower.len());
    let mut p = 0;
    while p < lower.len() {
        // try the three alternatives of /(^\s*|[.!?…]\s+|\n\s*)(\p{L})/ at p
        let mut m: Option<usize> = None; // index of the letter
        let skip_ws = |mut q: usize| {
            while q < lower.len() && js_ws(lower[q]) {
                q += 1;
            }
            q
        };
        if p == 0 {
            let q = skip_ws(0);
            if q < lower.len() && lower[q].is_alphabetic() {
                m = Some(q);
            }
        }
        if m.is_none() && matches!(lower[p], '.' | '!' | '?' | '…') {
            let q = skip_ws(p + 1);
            if q > p + 1 && q < lower.len() && lower[q].is_alphabetic() {
                m = Some(q);
            }
        }
        if m.is_none() && lower[p] == '\n' {
            let q = skip_ws(p + 1);
            if q < lower.len() && lower[q].is_alphabetic() {
                m = Some(q);
            }
        }
        match m {
            Some(q) => {
                out.extend(lower[p..q].iter());
                out.extend(lower[q].to_uppercase());
                p = q + 1;
            }
            None => {
                out.push(lower[p]);
                p += 1;
            }
        }
    }
    out
}

/// Every run of whitespace that contains a line break becomes one space; then trim.
pub fn join_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if !run.is_empty() {
            if run.contains(|c| c == '\r' || c == '\n') {
                out.push(' ');
            } else {
                out.push_str(run);
            }
            run.clear();
        }
    };
    for c in s.chars() {
        if js_ws(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    js_trim(&out).to_string()
}

fn collapse_blanks(l: &str) -> String {
    // /[ \t ]{2,}/g -> ' '
    let mut out = String::with_capacity(l.len());
    let mut run = 0usize;
    let mut first = ' ';
    let end_run = |out: &mut String, run: usize, first: char| {
        if run >= 2 {
            out.push(' ');
        } else if run == 1 {
            out.push(first);
        }
    };
    for c in l.chars() {
        if matches!(c, ' ' | '\t' | '\u{a0}') {
            if run == 0 {
                first = c;
            }
            run += 1;
        } else {
            end_run(&mut out, run, first);
            run = 0;
            out.push(c);
        }
    }
    end_run(&mut out, run, first);
    out
}
pub fn extra_spaces(s: &str) -> String {
    lines_of(s).into_iter().map(|l| js_trim(&collapse_blanks(l)).to_string()).collect::<Vec<_>>().join("\n")
}
pub fn trim_text(s: &str) -> String {
    js_trim(&lines_of(s).into_iter().map(js_trim).collect::<Vec<_>>().join("\n")).to_string()
}
pub fn json_pretty(s: &str) -> Result<String, String> {
    jsonlite::parse(s).map(|v| jsonlite::stringify(&v, 2))
}
pub fn json_min(s: &str) -> Result<String, String> {
    jsonlite::parse(s).map(|v| jsonlite::stringify(&v, 0))
}
/// encodeURIComponent
pub fn url_enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}
/// decodeURIComponent(s.trim().replace(/\+/g, ' ')); Err on a malformed escape or invalid UTF-8
pub fn url_dec(s: &str) -> Result<String, String> {
    let t = js_trim(s).replace('+', " ");
    let b = t.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = t.get(i + 1..i + 3).filter(|h| h.bytes().all(|c| c.is_ascii_hexdigit())).ok_or("URI malformed")?;
            out.push(u8::from_str_radix(h, 16).map_err(|_| "URI malformed")?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "URI malformed".to_string())
}
pub fn dedupe_lines(s: &str) -> String {
    let mut seen = std::collections::HashSet::new();
    lines_of(s).into_iter().filter(|l| seen.insert(*l)).collect::<Vec<_>>().join("\n")
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Tok {
    Sym(char),
    Num(String), // digits without leading zeros
    Word(u8, char),
}
fn fold(c: char) -> char {
    let l = c.to_lowercase().next().unwrap_or(c);
    match l {
        'ё' => 'е',
        'й' => 'и',
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => 'a',
        'ç' | 'ć' | 'č' => 'c',
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => 'e',
        'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
        'ñ' | 'ń' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' => 'o',
        'ś' | 'š' => 's',
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => 'u',
        'ý' | 'ÿ' => 'y',
        'ź' | 'ż' | 'ž' => 'z',
        x => x,
    }
}
fn coll_key(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let cs: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c.is_ascii_digit() {
            let mut j = i;
            while j < cs.len() && cs[j].is_ascii_digit() {
                j += 1;
            }
            let d: String = cs[i..j].iter().collect();
            let d = d.trim_start_matches('0');
            // fixed-width so a plain string compare is numeric compare
            out.push(Tok::Num(format!("{:>40}", d)));
            i = j;
            continue;
        }
        let f = fold(c);
        out.push(if f.is_alphabetic() {
            let class = match f {
                'a'..='z' => 3,
                'α'..='ω' => 4,
                'а'..='я' => 5,
                _ => 6,
            };
            Tok::Word(class, f)
        } else if c.is_numeric() {
            Tok::Num(format!("{:>40}", c))
        } else {
            Tok::Sym(c)
        });
        i += 1;
    }
    out
}
/// Sort lines A-Z (numbers natural); a second press on already sorted text reverses the order.
pub fn sort_lines(s: &str) -> String {
    let a = lines_of(s);
    let mut up = a.clone();
    up.sort_by_key(|l| coll_key(l));
    if a == up {
        up.reverse();
    }
    up.join("\n")
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub fn b64_enc(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = String::with_capacity((b.len() + 2) / 3 * 4);
    for ch in b.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        o.push(B64[(n >> 18) as usize & 63] as char);
        o.push(B64[(n >> 12) as usize & 63] as char);
        o.push(if ch.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if ch.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    o
}
/// Accepts the url-safe alphabet, ignores whitespace and missing padding; Err when it is not base64 or not UTF-8 text.
pub fn b64_dec(s: &str) -> Result<String, String> {
    let c: String = s.chars().filter(|c| !js_ws(*c)).map(|c| match c { '-' => '+', '_' => '/', c => c }).collect();
    let body = c.trim_end_matches('=');
    let pad = c.len() - body.len();
    if pad > 2 || !body.bytes().all(|b| B64.contains(&b)) {
        return Err("not base64".into());
    }
    if body.len() % 4 == 1 {
        return Err("bad length".into());
    }
    let mut bytes = Vec::new();
    let (mut acc, mut nb) = (0u32, 0);
    for b in body.bytes() {
        acc = acc << 6 | B64.iter().position(|x| *x == b).unwrap() as u32;
        nb += 6;
        if nb >= 8 {
            nb -= 8;
            bytes.push((acc >> nb) as u8);
            acc &= (1 << nb) - 1;
        }
    }
    String::from_utf8(bytes).map_err(|_| "not text".to_string())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    pub chars: usize,
    pub nospace: usize,
    pub words: usize,
    pub lines: usize,
    pub sentences: usize,
}
pub fn stats(s: &str) -> Stats {
    let chars = s.chars().count();
    let nospace = s.chars().filter(|c| !js_ws(*c)).count();
    let words = js_trim(s).split(js_ws).filter(|w| !w.is_empty()).count();
    let lines = if s.is_empty() { 0 } else { lines_of(s).len() };
    // /[^.!?…]+[.!?…]+|[^.!?…]+$/g, keeping the non-blank matches
    let term = |c: char| matches!(c, '.' | '!' | '?' | '…');
    let cs: Vec<char> = s.chars().collect();
    let (mut i, mut sentences) = (0, 0);
    while i < cs.len() {
        if term(cs[i]) {
            i += 1;
            continue;
        }
        let st = i;
        while i < cs.len() && !term(cs[i]) {
            i += 1;
        }
        while i < cs.len() && term(cs[i]) {
            i += 1;
        }
        let m: String = cs[st..i].iter().collect();
        if !js_trim(&m).is_empty() {
            sentences += 1;
        }
    }
    Stats { chars, nospace, words, lines, sentences }
}

// ================================================================= tool table

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Layout, Upper, Lower, Title, Sentence, Translit, Join, Spaces, JPretty, JMin, B64e, B64d, UrlE, UrlD, Trim, Sort, Dedupe, Qr, Count,
}
pub const TOOLS: [Tool; 19] = [
    Tool::Layout, Tool::Upper, Tool::Lower, Tool::Title, Tool::Sentence, Tool::Translit, Tool::Join, Tool::Spaces, Tool::JPretty, Tool::JMin,
    Tool::B64e, Tool::B64d, Tool::UrlE, Tool::UrlD, Tool::Trim, Tool::Sort, Tool::Dedupe, Tool::Qr, Tool::Count,
];
impl Tool {
    pub fn id(self) -> &'static str {
        match self {
            Tool::Layout => "layout", Tool::Upper => "upper", Tool::Lower => "lower", Tool::Title => "title", Tool::Sentence => "sentence",
            Tool::Translit => "translit", Tool::Join => "join", Tool::Spaces => "spaces", Tool::JPretty => "jpretty", Tool::JMin => "jmin",
            Tool::B64e => "b64e", Tool::B64d => "b64d", Tool::UrlE => "urle", Tool::UrlD => "urld", Tool::Trim => "trim", Tool::Sort => "sort",
            Tool::Dedupe => "dedupe", Tool::Qr => "qr", Tool::Count => "count",
        }
    }
    /// (label ru, label en, tooltip ru, tooltip en) exactly as in the JS TOOLS table
    fn texts(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Tool::Layout => ("Раскладка", "Layout", "Исправить раскладку RU <-> EN", "Fix RU <-> EN keyboard layout"),
            Tool::Upper => ("ВЕРХНИЙ", "UPPER", "", ""),
            Tool::Lower => ("нижний", "lower", "", ""),
            Tool::Title => ("Каждое Слово", "Title Case", "", ""),
            Tool::Sentence => ("Предложение", "Sentence", "Заглавная в начале предложений", "Capital at the start of sentences"),
            Tool::Translit => ("Транслит", "Translit", "Кириллица <-> латиница", "Cyrillic <-> Latin"),
            Tool::Join => ("Убрать переносы", "Join lines", "", ""),
            Tool::Spaces => ("Лишние пробелы", "Extra spaces", "", ""),
            Tool::JPretty => ("JSON красиво", "JSON pretty", "", ""),
            Tool::JMin => ("JSON в строку", "JSON minify", "", ""),
            Tool::B64e => ("→ Base64", "→ Base64", "", ""),
            Tool::B64d => ("Base64 →", "Base64 →", "", ""),
            Tool::UrlE => ("→ URL", "→ URL", "Закодировать для URL", "URL-encode"),
            Tool::UrlD => ("URL →", "URL →", "Раскодировать из URL", "URL-decode"),
            Tool::Trim => ("Обрезать", "Trim", "Пробелы по краям строк и текста", "Trim every line and the text"),
            Tool::Sort => ("Сортировка", "Sort lines", "А-Я; повторное нажатие: обратный порядок", "A-Z; press again to reverse"),
            Tool::Dedupe => ("Без дублей", "Unique lines", "", ""),
            Tool::Qr => ("QR-код", "QR code", "", ""),
            Tool::Count => ("Статистика", "Word count", "", ""),
        }
    }
    fn err(self) -> (&'static str, &'static str) {
        match self {
            Tool::JPretty | Tool::JMin => ("Это не JSON", "Not valid JSON"),
            Tool::B64d => ("Это не Base64 (или не текст)", "Not valid Base64 (or not text)"),
            Tool::UrlD => ("Неверная URL-кодировка", "Invalid URL encoding"),
            _ => ("Не получилось", "Failed"),
        }
    }
}
/// Run one text tool on `s`. `Qr` and `Count` are not text transforms (Err).
pub fn apply(tool: Tool, s: &str) -> Result<String, String> {
    Ok(match tool {
        Tool::Layout => layout_fix(s),
        Tool::Upper => s.to_uppercase(),
        Tool::Lower => s.to_lowercase(),
        Tool::Title => title_case(s),
        Tool::Sentence => sentence_case(s),
        Tool::Translit => translit(s),
        Tool::Join => join_lines(s),
        Tool::Spaces => extra_spaces(s),
        Tool::JPretty => return json_pretty(s),
        Tool::JMin => return json_min(s),
        Tool::B64e => b64_enc(s),
        Tool::B64d => return b64_dec(s),
        Tool::UrlE => url_enc(s),
        Tool::UrlD => return url_dec(s),
        Tool::Trim => trim_text(s),
        Tool::Sort => sort_lines(s),
        Tool::Dedupe => dedupe_lines(s),
        Tool::Qr | Tool::Count => return Err("not a text tool".into()),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiTool {
    Translate, Fix, Shorten, Summarize, Formal, Friendly, Bullets, Explain, Continue,
}
pub const AI_TOOLS: [AiTool; 9] = [AiTool::Translate, AiTool::Fix, AiTool::Shorten, AiTool::Summarize, AiTool::Formal, AiTool::Friendly, AiTool::Bullets, AiTool::Explain, AiTool::Continue];
impl AiTool {
    /// the `action` argument of `ai_transform`
    pub fn id(self) -> &'static str {
        match self {
            AiTool::Translate => "translate", AiTool::Fix => "fix", AiTool::Shorten => "shorten", AiTool::Summarize => "summarize", AiTool::Formal => "formal",
            AiTool::Friendly => "friendly", AiTool::Bullets => "bullets", AiTool::Explain => "explain", AiTool::Continue => "continue",
        }
    }
    fn texts(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            AiTool::Translate => ("Перевести", "Translate", "На английский, а английский на русский", "To English; English text goes to Russian"),
            AiTool::Fix => ("Исправить ошибки", "Fix errors", "Орфография и грамматика", "Spelling and grammar"),
            AiTool::Shorten => ("Сократить", "Shorten", "Примерно вдвое", "To about half"),
            AiTool::Summarize => ("Кратко", "Summarize", "Суть в нескольких предложениях", "The gist in a few sentences"),
            AiTool::Formal => ("Формально", "Formal", "Деловой тон", "A professional tone"),
            AiTool::Friendly => ("Дружелюбно", "Friendly", "Тёплый, неформальный тон", "A warm, relaxed tone"),
            AiTool::Bullets => ("Списком", "As a list", "Разбить на пункты", "Split into bullet points"),
            AiTool::Explain => ("Объяснить", "Explain", "Простыми словами", "In plain words"),
            AiTool::Continue => ("Продолжить", "Continue", "Дописать несколько предложений", "Write a few more sentences"),
        }
    }
}

// ================================================================= view-model

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Tools,
    Ai,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// the text area changed or the selection moved (char indices, start <= end)
    Input { text: String, sel: (usize, usize) },
    Run(Tool),
    SetMode(Mode),
    AiRun(AiTool),
    /// the "own request" line changed
    AskChanged(String),
    /// Enter / the run button of the own-request line
    AskGo,
    /// paste button; `None` = the clipboard could not be read
    Paste(Option<String>),
    Copy,
    CloseQr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolBtn {
    pub tool: Tool,
    pub label: String,
    pub tip: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct AiBtn {
    pub tool: AiTool,
    pub label: String,
    pub tip: String,
    /// this button started the running request (spinner / `run` class)
    pub running: bool,
}
#[derive(Clone, Debug, PartialEq)]
pub struct QrView {
    pub size: usize,
    pub quiet: usize,
    /// pixel size of one module: max(3, floor(190 / (size + 2 * quiet)))
    pub px: usize,
    pub rows: Vec<String>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextView {
    pub placeholder: String,
    pub count_line: String,
    pub mode: Mode,
    pub tools_label: String,
    pub ai_label: String,
    pub tools: Vec<ToolBtn>,
    pub ai: Vec<AiBtn>,
    pub ask_placeholder: String,
    pub ask: String,
    pub ai_busy: bool,
    pub toast: Option<(String, bool)>,
    pub qr: Option<QrView>,
}

struct AiJob {
    /// `None` = the free-form "own request" line (action `custom`)
    tool: Option<AiTool>,
    sel: (usize, usize),
    has_sel: bool,
    src: String,
}

pub struct TextModel {
    pub fx: Fx,
    lang: Lang,
    text: String,
    sel: (usize, usize),
    mode: Mode,
    ask: String,
    flash: Flash,
    qr: Option<Qr>,
    job: Option<AiJob>,
    pending: HashMap<u64, ()>,
    tags: Tags,
}

fn char_slice(s: &str, a: usize, b: usize) -> String {
    s.chars().skip(a).take(b.saturating_sub(a)).collect()
}

impl TextModel {
    pub fn new(lang: Lang) -> Self {
        Self { fx: Fx::default(), lang, text: String::new(), sel: (0, 0), mode: Mode::Tools, ask: String::new(), flash: Flash::default(), qr: None, job: None, pending: HashMap::new(), tags: Tags::new(3) }
    }
    pub fn set_lang(&mut self, l: Lang) {
        self.lang = l;
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn selection(&self) -> (usize, usize) {
        self.sel
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn is_ai_busy(&self) -> bool {
        self.job.is_some()
    }
    pub fn take_effects(&mut self) -> Vec<Effect> {
        self.fx.take_effects()
    }
    pub fn take_anims(&mut self) -> Vec<super::AnimCue> {
        self.fx.take_anims()
    }

    fn say(&mut self, ru: &str, en: &str, bad: bool) {
        let m = self.lang.t(ru, en).to_string();
        self.flash.say(&m, bad, Some(2200));
    }
    fn say_raw(&mut self, m: &str, bad: bool, sticky: bool) {
        self.flash.say(m, bad, if sticky { None } else { Some(2200) });
    }
    fn n_chars(&self) -> usize {
        self.text.chars().count()
    }
    fn source(&self) -> (String, bool) {
        let (a, b) = self.sel;
        if a != b { (char_slice(&self.text, a, b), true) } else { (self.text.clone(), false) }
    }
    /// Replace chars [start, end) (the renderer applies it through the input's edit pipeline); keeps the mirror in step.
    fn replace_range(&mut self, start: usize, end: usize, out: &str) {
        let head = char_slice(&self.text, 0, start);
        let tail = char_slice(&self.text, end, usize::MAX);
        self.text = format!("{head}{out}{tail}");
        let caret = start + out.chars().count();
        self.sel = (caret, caret);
        self.fx.eff(Effect::ReplaceRange { start, end, text: out.to_string() });
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Input { text, sel } => {
                self.text = text;
                let n = self.n_chars();
                self.sel = (sel.0.min(sel.1).min(n), sel.0.max(sel.1).min(n));
            }
            Action::SetMode(m) => self.mode = m,
            Action::AskChanged(q) => self.ask = q,
            Action::CloseQr => {
                if self.qr.take().is_some() {
                    self.fx.anim("qr", Anim::Exit { y: 0.0, scale: 0.97, ms: 160 });
                }
            }
            Action::Copy => {
                let (txt, _) = self.source();
                if !txt.is_empty() {
                    self.fx.eff(Effect::CopyText(txt));
                    self.say("Скопировано", "Copied", false);
                }
            }
            Action::Paste(clip) => match clip {
                Some(t) if !t.is_empty() => {
                    let (a, b) = self.sel;
                    self.replace_range(a, b, &t);
                }
                Some(_) => {}
                None => self.say("Нет доступа к буферу обмена", "Clipboard is not accessible", true),
            },
            Action::Run(tool) => self.run(tool),
            Action::AskGo => {
                let q = self.ask.trim().to_string();
                if !q.is_empty() {
                    return self.ai_run(None, Some(q));
                }
            }
            Action::AiRun(t) => return self.ai_run(Some(t), None),
        }
        vec![]
    }

    fn run(&mut self, tool: Tool) {
        self.fx.anim(tool.id(), Anim::Press);
        let (src, has_sel) = self.source();
        match tool {
            Tool::Count => {
                let s = stats(&src);
                let min = ((s.words as f64 / 200.0).round() as i64).max(1);
                let m = if self.lang.is_ru() {
                    format!("{} {} · {} предл. · ~{} мин чтения", s.words, plural(s.words as i64, "слово", "слова", "слов"), s.sentences, min)
                } else {
                    format!("{} {} · {} {} · ~{} min read", s.words, if s.words == 1 { "word" } else { "words" }, s.sentences, if s.sentences == 1 { "sentence" } else { "sentences" }, min)
                };
                self.say_raw(&m, false, false);
            }
            Tool::Qr => self.show_qr(&src),
            _ => {
                if src.is_empty() {
                    return;
                }
                match apply(tool, &src) {
                    Err(_) => {
                        let (ru, en) = tool.err();
                        self.say(ru, en, true);
                    }
                    Ok(out) if out == src => self.say("Без изменений", "No change", false),
                    Ok(out) => {
                        if has_sel {
                            let (a, b) = self.sel;
                            self.replace_range(a, b, &out);
                        } else {
                            let n = self.n_chars();
                            self.replace_range(0, n, &out);
                        }
                    }
                }
            }
        }
    }

    fn show_qr(&mut self, text: &str) {
        if js_trim(text).is_empty() {
            return;
        }
        match qr::qr(text) {
            Ok(q) => {
                self.qr = Some(q);
                self.fx.anim("qr", Anim::Enter { y: 0.0, ms: 180 });
            }
            Err(_) => self.say("Слишком длинно для QR (макс. 271 байт)", "Too long for a QR code (max 271 bytes)", true),
        }
    }

    /// One AI request on the selection (or the whole text); a second click while it runs cancels.
    fn ai_run(&mut self, tool: Option<AiTool>, instruction: Option<String>) -> Vec<BackendCall> {
        if self.job.is_some() {
            let tag = self.tags.next();
            return vec![BackendCall::new("ai_transform_cancel", json!({}), tag)];
        }
        let (src, has_sel) = self.source();
        if js_trim(&src).is_empty() {
            self.say("Сначала напиши или вставь текст", "Type or paste some text first", true);
            return vec![];
        }
        let action = tool.map_or("custom", |t| t.id());
        self.job = Some(AiJob { tool, sel: self.sel, has_sel, src: src.clone() });
        let m = self.lang.t("ИИ думает… нажми ещё раз, чтобы отменить", "AI is thinking… click again to cancel").to_string();
        self.say_raw(&m, false, true);
        self.fx.anim(action, Anim::Spin);
        let tag = self.tags.next();
        self.pending.insert(tag, ());
        vec![BackendCall::new("ai_transform", json!({ "text": src, "action": action, "instruction": instruction }), tag)]
    }

    pub fn on_reply(&mut self, tag: u64, r: Reply) -> Vec<BackendCall> {
        if self.pending.remove(&tag).is_none() {
            return vec![];
        }
        let Some(job) = self.job.take() else { return vec![] };
        let ok = r.ok && r.v.get("ok").and_then(Value::as_bool).unwrap_or(false);
        if !ok {
            let err = if r.ok { r.v.get("error").and_then(Value::as_str).map(str::to_string) } else { Some(r.err.clone()) };
            match err.as_deref() {
                Some("cancelled") => self.say("Отменено", "Cancelled", false),
                Some("net_off") => self.say("ИИ выключен: Настройки → Сеть → ИИ", "AI is off: Settings → Network → AI", true),
                Some("empty") => self.say("Нет текста", "No text", true),
                Some(e) if !e.is_empty() => {
                    let m = clip_chars(e, 140);
                    self.say_raw(&m, true, false);
                }
                _ => self.say("ИИ не ответил", "The AI did not answer", true),
            }
            return vec![];
        }
        let current = if job.has_sel { char_slice(&self.text, job.sel.0, job.sel.1) } else { self.text.clone() };
        if current != job.src {
            self.say("Текст изменился, пока ИИ думал", "The text changed while the AI was working", true);
            return vec![];
        }
        let out = r.v.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        let n = self.n_chars();
        if job.tool == Some(AiTool::Continue) {
            let at = if job.has_sel { job.sel.1 } else { n };
            let lead = if job.src.chars().last().map_or(false, js_ws) { "" } else { " " };
            self.replace_range(at, at, &format!("{lead}{out}"));
        } else if job.has_sel {
            self.replace_range(job.sel.0, job.sel.1, &out);
        } else {
            self.replace_range(0, n, &out);
        }
        self.say("Готово · Ctrl+Z вернёт как было", "Done · Ctrl+Z restores the original", false);
        vec![]
    }

    pub fn tick(&mut self, dt_ms: u32) {
        self.flash.tick(dt_ms);
    }

    pub fn view(&self) -> TextView {
        let l = self.lang;
        let s = stats(&self.text);
        let count_line = if l.is_ru() {
            format!("{} симв. · {} без пробелов · {} {} · {} стр.", s.chars, s.nospace, s.words, plural(s.words as i64, "слово", "слова", "слов"), s.lines)
        } else {
            format!(
                "{} {} · {} without spaces · {} {} · {} {}",
                s.chars, if s.chars == 1 { "char" } else { "chars" }, s.nospace, s.words, if s.words == 1 { "word" } else { "words" }, s.lines, if s.lines == 1 { "line" } else { "lines" }
            )
        };
        let running = self.job.as_ref().and_then(|j| j.tool);
        TextView {
            placeholder: l.t("Вставь или напиши текст…", "Type or paste text…").into(),
            count_line,
            mode: self.mode,
            tools_label: l.t("Инструменты", "Tools").into(),
            ai_label: l.t("ИИ", "AI").into(),
            tools: TOOLS.iter().map(|t| { let (a, b, c, d) = t.texts(); ToolBtn { tool: *t, label: l.t(a, b).into(), tip: l.t(c, d).into() } }).collect(),
            ai: AI_TOOLS.iter().map(|t| { let (a, b, c, d) = t.texts(); AiBtn { tool: *t, label: l.t(a, b).into(), tip: l.t(c, d).into(), running: running == Some(*t) } }).collect(),
            ask_placeholder: l.t("Свой запрос: «сделай бодрее», «на немецкий»…", "Your own request: \"make it punchier\", \"to German\"…").into(),
            ask: self.ask.clone(),
            ai_busy: self.job.is_some(),
            toast: self.flash.visible().then(|| (self.flash.text.clone(), self.flash.bad)),
            qr: self.qr.as_ref().map(|q| QrView { size: q.size, quiet: 4, px: 3.max(190 / (q.size + 8)), rows: q.rows() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reference() -> Value {
        serde_json::from_str(include_str!("text_js_reference.json")).unwrap()
    }
    fn run_named(name: &str, s: &str) -> Result<String, String> {
        match name {
            "layoutFix" => Ok(layout_fix(s)),
            "translit" => Ok(translit(s)),
            "titleCase" => Ok(title_case(s)),
            "sentenceCase" => Ok(sentence_case(s)),
            "joinLines" => Ok(join_lines(s)),
            "extraSpaces" => Ok(extra_spaces(s)),
            "trimText" => Ok(trim_text(s)),
            "jsonPretty" => json_pretty(s),
            "jsonMin" => json_min(s),
            "urlEnc" => Ok(url_enc(s)),
            "urlDec" => url_dec(s),
            "dedupeLines" => Ok(dedupe_lines(s)),
            "sortLines" => Ok(sort_lines(s)),
            "b64enc" => Ok(b64_enc(s)),
            "b64dec" => b64_dec(s),
            _ => panic!("unknown tool {name}"),
        }
    }

    /// Every tool on ~70 inputs (Latin, Cyrillic, emoji, CRLF, JSON, base64, percent-encoding) against the output of the real JS.
    #[test]
    fn every_tool_matches_the_js_output() {
        let r = reference();
        let inputs: Vec<&str> = r["inputs"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
        assert!(inputs.len() > 60);
        let mut checked = 0;
        for (name, outs) in r["tools"].as_object().unwrap() {
            for (i, want) in outs.as_array().unwrap().iter().enumerate() {
                let got = run_named(name, inputs[i]);
                match want.get("ok").and_then(Value::as_str) {
                    Some(w) => assert_eq!(got.as_deref(), Ok(w), "{name}({:?})", inputs[i]),
                    None => assert!(got.is_err(), "{name}({:?}) should fail, got {:?}", inputs[i], got),
                }
                checked += 1;
            }
        }
        assert!(checked >= 15 * 60);
        for (i, s) in inputs.iter().enumerate() {
            assert_eq!(s.to_uppercase(), r["upper"][i].as_str().unwrap(), "upper {s:?}");
            assert_eq!(s.to_lowercase(), r["lower"][i].as_str().unwrap(), "lower {s:?}");
            let st = stats(s);
            let w = &r["stats"][i];
            let got = [st.chars, st.nospace, st.words, st.lines, st.sentences];
            let want = ["chars", "nospace", "words", "lines", "sentences"].map(|k| w[k].as_u64().unwrap() as usize);
            assert_eq!(got, want, "stats {s:?}");
        }
    }

    // ---- the cases of src/tab-text.test.mjs, verbatim ----
    #[test]
    fn js_selfcheck_layout_and_translit() {
        assert_eq!(layout_fix("ghbdtn"), "привет");
        assert_eq!(layout_fix("руддщ цщкдв"), "hello world");
        assert_eq!(layout_fix("Ghbdtn vbh"), "Привет мир");
        assert_eq!(layout_fix(&layout_fix("qwerty ;'[]")), "qwerty ;'[]");
        assert_eq!(translit("Привет, мир"), "Privet, mir");
        assert_eq!(translit("Privet, mir"), "Привет, мир");
        assert_eq!(translit(&translit("Москва")), "Москва");
        assert_eq!(translit("Щука ЖУК"), "Shchuka ZHUK");
        assert_eq!(translit("Shchuka"), "Щука");
    }
    #[test]
    fn js_selfcheck_json_b64_url() {
        assert_eq!(json_pretty(r#"{"a":[1,2],"b":"x"}"#).unwrap(), "{\n  \"a\": [\n    1,\n    2\n  ],\n  \"b\": \"x\"\n}");
        assert_eq!(json_min(&json_pretty(r#"{"a":[1,2],"b":"x"}"#).unwrap()).unwrap(), r#"{"a":[1,2],"b":"x"}"#);
        assert!(json_pretty("{nope").is_err());
        for s in ["hello", "Привет, мир! 😀", "", "a\nb\tc"] {
            assert_eq!(b64_dec(&b64_enc(s)).unwrap(), s);
        }
        assert_eq!(b64_enc("hi"), "aGk=");
        assert_eq!(b64_dec("aGk").unwrap(), "hi");
        assert!(b64_dec("%%%").is_err());
        assert_eq!(url_dec(&url_enc("a b&c=д")).unwrap(), "a b&c=д");
        assert!(url_dec("%E0%A4%A").is_err());
    }
    #[test]
    fn js_selfcheck_case_lines_spaces() {
        assert_eq!(title_case("hELLO wORLD-foo don't"), "Hello World-Foo Don't");
        assert_eq!(sentence_case("hello there. HOW are you? fine\nok"), "Hello there. How are you? Fine\nOk");
        assert_eq!(join_lines("a\r\n b \n\nc"), "a b c");
        assert_eq!(extra_spaces("a   b \t c  \n  d"), "a b c\nd");
        assert_eq!(sort_lines("b\nA\nc10\nc9"), "A\nb\nc9\nc10");
        assert_eq!(sort_lines("A\nb\nc9\nc10"), "c10\nc9\nb\nA");
        assert_eq!(dedupe_lines("a\nb\na\nc\nb"), "a\nb\nc");
        let s = stats("56 chars\nx y");
        assert_eq!((s.chars, s.nospace, s.words, s.lines, s.sentences), (12, 9, 4, 2, 1));
    }

    // ---- extra edge cases (Cyrillic, emoji, CRLF) ----
    #[test]
    fn cyrillic_and_emoji_edges() {
        assert_eq!(title_case("привет ЁЖИК"), "Привет Ёжик");
        assert_eq!(sentence_case("привет. КАК дела? ХОРОШО"), "Привет. Как дела? Хорошо");
        assert_eq!(translit("Ёлка"), "Yolka");
        assert_eq!(translit("ЩУКА"), "SHCHUKA");
        assert_eq!(translit("😀 привет"), "😀 privet");
        assert_eq!(join_lines("😀\r\n😀"), "😀 😀");
        assert_eq!(stats("😀😀 a").chars, 4);
        assert_eq!(sort_lines("яблоко\nАрбуз\nбанан"), "Арбуз\nбанан\nяблоко");
        assert_eq!(url_enc("д"), "%D0%B4");
        assert_eq!(url_dec("a+b%20c").unwrap(), "a b c");
        assert_eq!(dedupe_lines("a\r\nb\r\na"), "a\nb");
        assert_eq!(trim_text("  a  \r\n  b  "), "a\nb");
        assert_eq!(extra_spaces("a\u{a0}\u{a0}b"), "a b");
    }

    // ---- model ----
    fn model(text: &str, sel: (usize, usize)) -> TextModel {
        let mut m = TextModel::new(Lang::En);
        m.on_action(Action::Input { text: text.into(), sel });
        m
    }
    #[test]
    fn tool_replaces_whole_text_or_selection() {
        let mut m = model("hello world", (0, 0));
        m.on_action(Action::Run(Tool::Upper));
        assert_eq!(m.text(), "HELLO WORLD");
        assert_eq!(m.take_effects(), vec![Effect::ReplaceRange { start: 0, end: 11, text: "HELLO WORLD".into() }]);
        let mut m = model("hello world", (6, 11));
        m.on_action(Action::Run(Tool::Upper));
        assert_eq!(m.text(), "hello WORLD");
        assert_eq!(m.selection(), (11, 11));
    }
    #[test]
    fn selection_indices_are_chars_not_bytes() {
        let mut m = model("привет мир 😀 ok", (7, 10));
        m.on_action(Action::Run(Tool::Upper));
        assert_eq!(m.text(), "привет МИР 😀 ok");
    }
    #[test]
    fn no_change_empty_and_errors_toast() {
        let mut m = model("ABC", (0, 0));
        m.on_action(Action::Run(Tool::Upper));
        assert_eq!(m.view().toast, Some(("No change".into(), false)));
        assert!(m.take_effects().is_empty());
        let mut m = model("{nope", (0, 0));
        m.on_action(Action::Run(Tool::JPretty));
        assert_eq!(m.view().toast, Some(("Not valid JSON".into(), true)));
        m.set_lang(Lang::Ru);
        m.on_action(Action::Input { text: "%%%".into(), sel: (0, 0) });
        m.on_action(Action::Run(Tool::B64d));
        assert_eq!(m.view().toast, Some(("Это не Base64 (или не текст)".into(), true)));
        let mut m = model("", (0, 0));
        m.on_action(Action::Run(Tool::Upper));
        assert_eq!(m.view().toast, None);
        m.on_action(Action::Run(Tool::Trim));
        assert!(m.take_effects().is_empty());
    }
    #[test]
    fn toast_expires_after_2200ms() {
        let mut m = model("ABC", (0, 0));
        m.on_action(Action::Run(Tool::Upper));
        m.tick(2199);
        assert!(m.view().toast.is_some());
        m.tick(1);
        assert!(m.view().toast.is_none());
    }
    #[test]
    fn count_and_qr() {
        let mut m = model("One two. Three four five.", (0, 0));
        m.on_action(Action::Run(Tool::Count));
        assert_eq!(m.view().toast.unwrap().0, "5 words · 2 sentences · ~1 min read");
        m.set_lang(Lang::Ru);
        m.on_action(Action::Run(Tool::Count));
        assert_eq!(m.view().toast.unwrap().0, "5 слов · 2 предл. · ~1 мин чтения");
        m.on_action(Action::Run(Tool::Qr));
        let q = m.view().qr.unwrap();
        assert_eq!((q.size, q.quiet), (q.rows.len(), 4));
        assert!(q.px >= 3);
        assert!(m.take_anims().iter().any(|c| c.target == "qr"));
        m.on_action(Action::CloseQr);
        assert!(m.view().qr.is_none());
        let mut m = model(&"x".repeat(272), (0, 0));
        m.on_action(Action::Run(Tool::Qr));
        assert!(m.view().qr.is_none() && m.view().toast.unwrap().1);
    }
    #[test]
    fn counter_line_both_languages() {
        let mut m = model("56 chars\nx y", (0, 0));
        assert_eq!(m.view().count_line, "12 chars · 9 without spaces · 4 words · 2 lines");
        m.set_lang(Lang::Ru);
        assert_eq!(m.view().count_line, "12 симв. · 9 без пробелов · 4 слова · 2 стр.");
    }
    #[test]
    fn copy_and_paste() {
        let mut m = model("abc def", (4, 7));
        m.on_action(Action::Copy);
        assert_eq!(m.take_effects(), vec![Effect::CopyText("def".into())]);
        m.on_action(Action::Paste(Some("XY".into())));
        assert_eq!(m.text(), "abc XY");
        m.on_action(Action::Paste(None));
        assert!(m.view().toast.unwrap().1);
    }

    fn ai_ok(text: &str) -> Reply {
        Reply::ok(json!({ "ok": true, "text": text }))
    }
    #[test]
    fn ai_translate_selection() {
        let mut m = model("привет мир", (0, 6));
        let calls = m.on_action(Action::AiRun(AiTool::Translate));
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].cmd, "ai_transform");
        assert_eq!(calls[0].args, json!({ "text": "привет", "action": "translate", "instruction": null }));
        assert!(m.is_ai_busy() && m.view().ai.iter().any(|b| b.running && b.tool == AiTool::Translate));
        m.on_reply(calls[0].tag, ai_ok("hello"));
        assert_eq!(m.text(), "hello мир");
        assert!(!m.is_ai_busy());
        assert_eq!(m.view().toast.unwrap().0, "Done · Ctrl+Z restores the original");
    }
    #[test]
    fn ai_second_click_cancels() {
        let mut m = model("some text", (0, 0));
        let first = m.on_action(Action::AiRun(AiTool::Fix));
        let second = m.on_action(Action::AiRun(AiTool::Fix));
        assert_eq!(second[0].cmd, "ai_transform_cancel");
        m.on_reply(first[0].tag, Reply::ok(json!({ "ok": false, "error": "cancelled" })));
        assert_eq!(m.view().toast, Some(("Cancelled".into(), false)));
        assert_eq!(m.text(), "some text");
    }
    #[test]
    fn ai_continue_appends_with_a_space() {
        let mut m = model("The end", (0, 0));
        let c = m.on_action(Action::AiRun(AiTool::Continue));
        m.on_reply(c[0].tag, ai_ok("is near."));
        assert_eq!(m.text(), "The end is near.");
        let mut m = model("The end ", (0, 0));
        let c = m.on_action(Action::AiRun(AiTool::Continue));
        m.on_reply(c[0].tag, ai_ok("is near."));
        assert_eq!(m.text(), "The end is near.");
    }
    #[test]
    fn ai_text_changed_meanwhile_is_not_overwritten() {
        let mut m = model("abc", (0, 0));
        let c = m.on_action(Action::AiRun(AiTool::Shorten));
        m.on_action(Action::Input { text: "abcd".into(), sel: (4, 4) });
        m.on_reply(c[0].tag, ai_ok("x"));
        assert_eq!(m.text(), "abcd");
        assert!(m.view().toast.unwrap().1);
    }
    #[test]
    fn ai_errors_map_to_messages() {
        for (err, msg) in [("net_off", "AI is off: Settings → Network → AI"), ("empty", "No text")] {
            let mut m = model("abc", (0, 0));
            let c = m.on_action(Action::AiRun(AiTool::Fix));
            m.on_reply(c[0].tag, Reply::ok(json!({ "ok": false, "error": err })));
            assert_eq!(m.view().toast, Some((msg.into(), true)));
        }
        let mut m = model("abc", (0, 0));
        let c = m.on_action(Action::AiRun(AiTool::Fix));
        m.on_reply(c[0].tag, Reply::ok(json!({ "ok": false, "error": "e".repeat(300) })));
        assert_eq!(m.view().toast.unwrap().0.chars().count(), 140);
        let mut m = model("abc", (0, 0));
        let c = m.on_action(Action::AiRun(AiTool::Fix));
        m.on_reply(c[0].tag, Reply::ok(Value::Null));
        assert_eq!(m.view().toast, Some(("The AI did not answer".into(), true)));
    }
    #[test]
    fn ai_needs_text_and_custom_request() {
        let mut m = model("  ", (0, 0));
        assert!(m.on_action(Action::AiRun(AiTool::Fix)).is_empty());
        assert_eq!(m.view().toast.unwrap().0, "Type or paste some text first");
        let mut m = model("abc", (0, 0));
        m.on_action(Action::AskChanged("  ".into()));
        assert!(m.on_action(Action::AskGo).is_empty());
        m.on_action(Action::AskChanged(" make it punchier ".into()));
        let c = m.on_action(Action::AskGo);
        assert_eq!(c[0].args, json!({ "text": "abc", "action": "custom", "instruction": "make it punchier" }));
        assert!(m.is_ai_busy() && !m.view().ai.iter().any(|b| b.running));
    }
    #[test]
    fn tools_grid_has_every_js_tool_with_ru_en_labels() {
        let v = model("", (0, 0)).view();
        assert_eq!(v.tools.len(), 19);
        assert_eq!(v.ai.len(), 9);
        assert_eq!(v.tools[0].label, "Layout");
        let mut m = model("", (0, 0));
        m.set_lang(Lang::Ru);
        assert_eq!(m.view().tools[15].label, "Сортировка");
        assert_eq!(m.view().tools[15].tip, "А-Я; повторное нажатие: обратный порядок");
    }
}
