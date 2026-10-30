//! Note body: the stored (sanitized) HTML of `note_html::sanitize` <-> the document the native editor edits. Pure, no GPU.
//!
//! The web editor is a contenteditable `div` whose whitelist is `b i s mark ul ol li div br a input[checkbox]`. The model here:
//! a [`Doc`] is a list of [`Line`]s (one per visual paragraph), a line has a [`Kind`] (plain, bullet, numbered, checklist), a nesting
//! `level` (lists only) and per-character [`Attr`]s (bold, italic, strike, mark, link). A soft line break (`<br>` inside a paragraph,
//! Shift+Enter) is a `'\n'` char of the line. A checkbox that is not the first thing of a plain line (or sits in a list item) is kept as an
//! inline object char (`☐` / `☑` with the [`CHK`] flag) so nothing of the markup is lost.
//!
//! Chromium shapes the HTML like this, and [`Doc::to_html`] writes exactly that: the first line is bare text, every later plain line is
//! `<div>..</div>` (`<div><br></div>` when empty), consecutive list lines share one `<ul>`/`<ol>`, a checklist line is
//! `<input type="checkbox">&nbsp;text` (`checked=""` when ticked), formatting nests as `<a><b><i><s><mark>` with the minimal number of
//! tag changes between runs (`<b>bold <i>both</i></b>`). Samples written by Chromium round-trip byte for byte; anything else round-trips
//! SEMANTICALLY (`from_html(to_html(d)) == d`), which is the rule the tests pin. Deliberate normalisations, all invisible in the web view:
//! a literal `'\n'` in a text node becomes a soft break (`<br>`), a `<div>` nested in a `<div>` becomes a sibling line, the nbsp after a
//! checklist box is always written, empty inline elements (`<b></b>`) vanish, tag nesting order is the fixed one above.
use pill_ui_core::panel::b::note_html::sanitize;
use pill_ui_core::panel::b::text::js_ws;

pub const B: u8 = 1;
pub const I: u8 = 2;
pub const S: u8 = 4;
pub const M: u8 = 8;
/// the char is an inline checkbox object (`☐` / `☑`)
pub const CHK: u8 = 16;
/// that inline checkbox is ticked
pub const TICK: u8 = 32;
pub const FMT_MASK: u8 = B | I | S | M;

pub const BOX_OFF: char = '\u{2610}';
pub const BOX_ON: char = '\u{2611}';

/// Formatting of one char: flag bits and a link (0 = none, n = `Doc::links[n - 1]`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attr {
    pub f: u8,
    pub link: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    #[default]
    Plain,
    Bullet,
    Number,
    Check(bool),
}

impl Kind {
    pub fn is_list(self) -> bool {
        matches!(self, Kind::Bullet | Kind::Number)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    /// nesting depth of a list line (0 = outermost)
    pub level: u8,
    pub text: Vec<char>,
    pub at: Vec<Attr>,
}

impl Line {
    pub fn len(&self) -> usize {
        self.text.len()
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn string(&self) -> String {
        self.text.iter().collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl Pos {
    pub fn new(line: usize, col: usize) -> Pos {
        Pos { line, col }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub lines: Vec<Line>,
    pub links: Vec<String>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc::empty()
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// HTML -> Doc
// ---------------------------------------------------------------------------------------------------------------------------------------

enum Tok {
    Text(String),
    Open(String, Vec<(String, String)>),
    Close(String),
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.char_indices().take(12).find(|(_, c)| *c == ';').map(|(i, _)| i);
        let ch = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                n if n.starts_with("#x") || n.starts_with("#X") => u32::from_str_radix(&n[2..], 16).ok().and_then(char::from_u32),
                n if n.starts_with('#') => n[1..].parse::<u32>().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e))
        });
        match ch {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A tolerant tokenizer for the sanitized whitelist (the sanitizer already produced well-formed, lower-case markup).
fn tokenize(html: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < html.len() {
        let Some(lt) = html[i..].find('<').map(|p| i + p) else {
            out.push(Tok::Text(decode(&html[i..])));
            break;
        };
        if lt > i {
            out.push(Tok::Text(decode(&html[i..lt])));
        }
        let Some(gt) = html[lt..].find('>').map(|p| lt + p) else {
            out.push(Tok::Text(decode(&html[lt..])));
            break;
        };
        let inner = &html[lt + 1..gt];
        i = gt + 1;
        if let Some(name) = inner.strip_prefix('/') {
            out.push(Tok::Close(name.trim().to_ascii_lowercase()));
            continue;
        }
        let inner = inner.trim_end_matches('/').trim();
        let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
        let name = inner[..name_end].to_ascii_lowercase();
        let mut attrs = Vec::new();
        let mut rest = inner[name_end..].trim_start();
        while !rest.is_empty() {
            let ke = rest.find(|c: char| c == '=' || c.is_whitespace()).unwrap_or(rest.len());
            let key = rest[..ke].to_ascii_lowercase();
            rest = rest[ke..].trim_start();
            let mut val = String::new();
            if let Some(r) = rest.strip_prefix('=') {
                let r = r.trim_start();
                if let Some(q) = r.chars().next().filter(|c| *c == '"' || *c == '\'') {
                    let body = &r[1..];
                    let ve = body.find(q).unwrap_or(body.len());
                    val = decode(&body[..ve]);
                    rest = body[(ve + 1).min(body.len())..].trim_start();
                } else {
                    let ve = r.find(|c: char| c.is_whitespace()).unwrap_or(r.len());
                    val = decode(&r[..ve]);
                    rest = r[ve..].trim_start();
                }
            }
            if !key.is_empty() {
                attrs.push((key, val));
            }
        }
        out.push(Tok::Open(name, attrs));
    }
    out
}

struct Builder {
    doc: Doc,
    cur: Line,
    open: bool,
    /// open inline elements: (tag, flag bit, link)
    inl: Vec<(String, u8, u32)>,
    lists: Vec<Kind>,
    in_li: usize,
    /// the leading nbsp after a line-start checkbox is still to be swallowed
    eat_nbsp: bool,
}

impl Builder {
    fn attr(&self) -> Attr {
        let mut a = Attr::default();
        for (_, f, l) in &self.inl {
            a.f |= f;
            if *l != 0 {
                a.link = *l;
            }
        }
        a
    }

    fn push(&mut self, c: char, extra: u8) {
        let mut a = self.attr();
        a.f |= extra;
        self.cur.text.push(c);
        self.cur.at.push(a);
        self.open = true;
    }

    /// End the current line (if anything opened it). A single trailing soft break is the block's placeholder `<br>`, not content.
    fn flush(&mut self) {
        if !self.open {
            return;
        }
        if self.cur.text.last() == Some(&'\n') {
            self.cur.text.pop();
            self.cur.at.pop();
        }
        self.doc.lines.push(std::mem::take(&mut self.cur));
        self.open = false;
        self.eat_nbsp = false;
    }

    fn text(&mut self, t: &str) {
        if !self.open && self.cur.text.is_empty() && t.contains('\n') && t.chars().all(char::is_whitespace) {
            return; // indentation between blocks
        }
        if !self.lists.is_empty() && self.in_li == 0 && t.chars().all(char::is_whitespace) {
            return; // whitespace between <ul> and <li>
        }
        for c in t.chars() {
            if c == '\r' {
                continue;
            }
            if self.eat_nbsp {
                self.eat_nbsp = false;
                if c == '\u{a0}' {
                    continue;
                }
            }
            self.push(c, 0);
        }
    }
}

impl Doc {
    pub fn empty() -> Doc {
        Doc { lines: vec![Line::default()], links: Vec::new() }
    }

    /// Parse stored note HTML (sanitized first, exactly like the web editor does when it loads a note).
    pub fn from_html(html: &str) -> Doc {
        let html = sanitize(html);
        let mut b = Builder { doc: Doc { lines: Vec::new(), links: Vec::new() }, cur: Line::default(), open: false, inl: Vec::new(), lists: Vec::new(), in_li: 0, eat_nbsp: false };
        for t in tokenize(&html) {
            match t {
                Tok::Text(t) => b.text(&t),
                Tok::Open(name, attrs) => match name.as_str() {
                    "b" => b.inl.push((name, B, 0)),
                    "i" => b.inl.push((name, I, 0)),
                    "s" => b.inl.push((name, S, 0)),
                    "mark" => b.inl.push((name, M, 0)),
                    "a" => {
                        let href = attrs.iter().find(|(k, _)| k == "href").map(|(_, v)| v.clone()).filter(|h| !h.is_empty());
                        let l = match href {
                            Some(h) => match b.doc.links.iter().position(|x| *x == h) {
                                Some(p) => p as u32 + 1,
                                None => {
                                    b.doc.links.push(h);
                                    b.doc.links.len() as u32
                                }
                            },
                            None => 0,
                        };
                        b.inl.push((name, 0, l));
                    }
                    "br" => b.push('\n', 0),
                    "div" => {
                        if b.in_li > 0 {
                            if !b.cur.text.is_empty() && b.cur.text.last() != Some(&'\n') {
                                b.push('\n', 0);
                            }
                        } else {
                            b.flush();
                        }
                    }
                    "ul" | "ol" => {
                        if b.in_li > 0 && b.open {
                            if b.cur.text.is_empty() {
                                b.open = false; // an empty parent item that only holds the nested list
                                b.cur = Line::default();
                            } else {
                                b.flush();
                            }
                        } else {
                            b.flush();
                        }
                        b.lists.push(if name == "ul" { Kind::Bullet } else { Kind::Number });
                    }
                    "li" => {
                        b.flush();
                        b.cur.kind = b.lists.last().copied().unwrap_or(Kind::Bullet);
                        b.cur.level = b.lists.len().saturating_sub(1).min(8) as u8;
                        b.in_li += 1;
                        b.open = true;
                    }
                    "input" => {
                        let checkbox = attrs.iter().any(|(k, v)| k == "type" && v.eq_ignore_ascii_case("checkbox"));
                        if checkbox {
                            let ticked = attrs.iter().any(|(k, _)| k == "checked");
                            if b.cur.text.is_empty() && b.cur.kind == Kind::Plain && b.in_li == 0 {
                                b.cur.kind = Kind::Check(ticked);
                                b.open = true;
                                b.eat_nbsp = true;
                            } else {
                                b.push(if ticked { BOX_ON } else { BOX_OFF }, CHK | if ticked { TICK } else { 0 });
                            }
                        }
                    }
                    _ => {}
                },
                Tok::Close(name) => match name.as_str() {
                    "b" | "i" | "s" | "mark" | "a" => {
                        if let Some(p) = b.inl.iter().rposition(|(t, _, _)| *t == name) {
                            b.inl.remove(p);
                        }
                    }
                    "div" => {
                        if b.in_li == 0 {
                            b.flush();
                        }
                    }
                    "li" => {
                        b.flush();
                        b.in_li = b.in_li.saturating_sub(1);
                    }
                    "ul" | "ol" => {
                        b.flush();
                        b.lists.pop();
                    }
                    _ => {}
                },
            }
        }
        b.flush();
        if b.doc.lines.is_empty() {
            b.doc.lines.push(Line::default());
        }
        b.doc
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // Doc -> HTML
    // -----------------------------------------------------------------------------------------------------------------------------------

    /// The canonical Chromium-shaped HTML of the document (see the module docs).
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        let mut stack: Vec<Kind> = Vec::new();
        for (n, l) in self.lines.iter().enumerate() {
            if l.kind.is_list() {
                let lv = l.level as usize;
                while stack.len() > lv + 1 {
                    out.push_str(close_list(stack.pop().unwrap_or(Kind::Bullet)));
                }
                if stack.len() == lv + 1 {
                    if stack[lv] == l.kind {
                        out.push_str("</li><li>");
                    } else {
                        out.push_str(close_list(stack.pop().unwrap_or(Kind::Bullet)));
                        out.push_str(open_list(l.kind));
                        stack.push(l.kind);
                    }
                } else {
                    while stack.len() <= lv {
                        out.push_str(open_list(l.kind));
                        stack.push(l.kind);
                    }
                }
                self.emit_content(l, &mut out);
                continue;
            }
            while let Some(k) = stack.pop() {
                out.push_str(close_list(k));
            }
            let bare = n == 0 && (matches!(l.kind, Kind::Check(_)) || !l.text.is_empty() || self.lines.len() == 1);
            if !bare {
                out.push_str("<div>");
            }
            if let Kind::Check(t) = l.kind {
                out.push_str(if t { "<input type=\"checkbox\" checked=\"\">&nbsp;" } else { "<input type=\"checkbox\">&nbsp;" });
            }
            self.emit_content(l, &mut out);
            let needs_br = (l.text.is_empty() && !matches!(l.kind, Kind::Check(_)) && !bare) || l.text.last() == Some(&'\n');
            if needs_br {
                out.push_str("<br>");
            }
            if !bare {
                out.push_str("</div>");
            }
        }
        while let Some(k) = stack.pop() {
            out.push_str(close_list(k));
        }
        out
    }

    fn emit_content(&self, l: &Line, out: &mut String) {
        let mut open: Vec<Tag> = Vec::new();
        let mut i = 0;
        while i < l.text.len() {
            let a = l.at[i];
            if a.f & CHK != 0 {
                close_tags(out, &mut open, 0);
                out.push_str(if a.f & TICK != 0 { "<input type=\"checkbox\" checked=\"\">" } else { "<input type=\"checkbox\">" });
                i += 1;
                continue;
            }
            let want = tags_of(a);
            let common = open.iter().zip(&want).take_while(|(x, y)| x == y).count();
            close_tags(out, &mut open, common);
            for t in &want[common..] {
                match t {
                    Tag::A(n) => {
                        out.push_str("<a href=\"");
                        out.push_str(&esc_attr(&self.links[*n as usize - 1]));
                        out.push_str("\">");
                    }
                    other => out.push_str(other.open()),
                }
                open.push(*t);
            }
            let mut j = i;
            while j < l.text.len() && l.at[j] == a {
                j += 1;
            }
            for k in i..j {
                match l.text[k] {
                    '\n' => out.push_str("<br>"),
                    c => push_esc(out, c),
                }
            }
            i = j;
        }
        close_tags(out, &mut open, 0);
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // reading
    // -----------------------------------------------------------------------------------------------------------------------------------

    /// `note_html::is_empty`: no visible text and no list / checklist line.
    pub fn is_blank(&self) -> bool {
        self.lines.iter().all(|l| l.kind == Kind::Plain && l.text.iter().all(|c| js_ws(*c) && *c != BOX_OFF) && l.at.iter().all(|a| a.f & CHK == 0))
    }

    pub fn link(&self, a: Attr) -> Option<&str> {
        (a.link != 0).then(|| self.links[a.link as usize - 1].as_str())
    }

    pub fn end(&self) -> Pos {
        let l = self.lines.len() - 1;
        Pos::new(l, self.lines[l].len())
    }

    pub fn clamp(&self, p: Pos) -> Pos {
        let line = p.line.min(self.lines.len() - 1);
        Pos::new(line, p.col.min(self.lines[line].len()))
    }

    /// The text of a range, lines joined by `\n` (what Ctrl+C puts on the clipboard).
    pub fn text_in(&self, a: Pos, b: Pos) -> String {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        let mut s = String::new();
        for li in a.line..=b.line {
            let l = &self.lines[li];
            let (c0, c1) = (if li == a.line { a.col } else { 0 }, if li == b.line { b.col } else { l.len() });
            s.extend(l.text[c0..c1].iter().map(|c| if *c == BOX_OFF || *c == BOX_ON { ' ' } else { *c }));
            if li != b.line {
                s.push('\n');
            }
        }
        s
    }

    /// The whole text, one line per paragraph.
    pub fn plain_text(&self) -> String {
        self.text_in(Pos::new(0, 0), self.end())
    }

    /// The number shown for a numbered line (1-based position in its run of numbered lines of the same level).
    pub fn number_of(&self, line: usize) -> usize {
        let lv = self.lines[line].level;
        let mut n = 1;
        let mut i = line;
        while i > 0 {
            let p = &self.lines[i - 1];
            if p.kind == Kind::Number && p.level == lv {
                n += 1;
            } else if !(p.kind.is_list() && p.level > lv) {
                break;
            }
            i -= 1;
        }
        n
    }

    /// Word start / end around `p` (letters and digits form a word; a run of spaces or of other signs is one unit).
    pub fn word_at(&self, p: Pos) -> (Pos, Pos) {
        let l = &self.lines[p.line.min(self.lines.len() - 1)];
        if l.is_empty() {
            return (Pos::new(p.line, 0), Pos::new(p.line, 0));
        }
        let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
        let i = p.col.min(l.len() - 1);
        let k = class(l.text[i]);
        let (mut s, mut e) = (i, i + 1);
        while s > 0 && class(l.text[s - 1]) == k {
            s -= 1;
        }
        while e < l.len() && class(l.text[e]) == k {
            e += 1;
        }
        (Pos::new(p.line, s), Pos::new(p.line, e))
    }

    // -----------------------------------------------------------------------------------------------------------------------------------
    // editing
    // -----------------------------------------------------------------------------------------------------------------------------------

    /// Insert `s` (may hold `\n`: a soft break) at `at` with attribute `attr`; returns the position after it.
    pub fn insert(&mut self, at: Pos, s: &str, attr: Attr) -> Pos {
        let at = self.clamp(at);
        let l = &mut self.lines[at.line];
        let mut col = at.col;
        for c in s.chars() {
            if c == '\r' {
                continue;
            }
            l.text.insert(col, c);
            l.at.insert(col, attr);
            col += 1;
        }
        Pos::new(at.line, col)
    }

    /// Insert a pasted block: its lines become lines of the document (kinds of the surroundings are kept for the first line).
    pub fn insert_multiline(&mut self, at: Pos, s: &str, attr: Attr) -> Pos {
        let norm = s.replace("\r\n", "\n").replace('\r', "\n");
        let mut p = at;
        for (i, part) in norm.split('\n').enumerate() {
            if i > 0 {
                p = self.split(p);
            }
            p = self.insert(p, part, attr);
        }
        p
    }

    /// Enter: split the line at `at`. A list or checklist line continues in the same kind (a new checkbox is unticked).
    pub fn split(&mut self, at: Pos) -> Pos {
        let at = self.clamp(at);
        let l = &mut self.lines[at.line];
        let tail_t = l.text.split_off(at.col);
        let tail_a = l.at.split_off(at.col);
        let kind = match l.kind {
            Kind::Check(_) => Kind::Check(false),
            k => k,
        };
        let nl = Line { kind, level: l.level, text: tail_t, at: tail_a };
        self.lines.insert(at.line + 1, nl);
        Pos::new(at.line + 1, 0)
    }

    /// Remove `[a, b)`; lines join (the first keeps its kind). Returns the start.
    pub fn delete(&mut self, a: Pos, b: Pos) -> Pos {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        if a == b {
            return a;
        }
        if a.line == b.line {
            let l = &mut self.lines[a.line];
            l.text.drain(a.col..b.col);
            l.at.drain(a.col..b.col);
            return a;
        }
        let tail = self.lines[b.line].clone();
        self.lines.drain(a.line + 1..=b.line);
        let l = &mut self.lines[a.line];
        l.text.truncate(a.col);
        l.at.truncate(a.col);
        l.text.extend_from_slice(&tail.text[b.col..]);
        l.at.extend_from_slice(&tail.at[b.col..]);
        a
    }

    /// Do all chars of `[a, b)` carry `bit`? (Empty range: false.)
    pub fn all_have(&self, a: Pos, b: Pos, bit: u8) -> bool {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        let mut any = false;
        for li in a.line..=b.line {
            let l = &self.lines[li];
            let (c0, c1) = (if li == a.line { a.col } else { 0 }, if li == b.line { b.col } else { l.len() });
            for k in c0..c1 {
                any = true;
                if l.at[k].f & bit == 0 {
                    return false;
                }
            }
        }
        any
    }

    pub fn set_flag(&mut self, a: Pos, b: Pos, bit: u8, on: bool) {
        let (a, b) = (self.clamp(a.min(b)), self.clamp(a.max(b)));
        for li in a.line..=b.line {
            let l = &mut self.lines[li];
            let (c0, c1) = (if li == a.line { a.col } else { 0 }, if li == b.line { b.col } else { l.len() });
            for k in c0..c1 {
                if l.at[k].f & CHK != 0 {
                    continue;
                }
                if on {
                    l.at[k].f |= bit;
                } else {
                    l.at[k].f &= !bit;
                }
            }
        }
    }

    /// Attribute of the char before `p` (typing continues its formatting), the plain one at a line start.
    pub fn attr_before(&self, p: Pos) -> Attr {
        let l = &self.lines[p.line.min(self.lines.len() - 1)];
        if p.col == 0 || l.is_empty() {
            return Attr::default();
        }
        let mut a = l.at[(p.col - 1).min(l.len() - 1)];
        a.f &= FMT_MASK;
        a
    }

    /// The link at `p` (the char under / before the caret).
    pub fn link_at(&self, p: Pos) -> Option<&str> {
        let l = &self.lines[p.line.min(self.lines.len() - 1)];
        let i = p.col.min(l.len().saturating_sub(1));
        l.at.get(i).and_then(|a| self.link(*a))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tag {
    A(u32),
    B,
    I,
    S,
    M,
}

impl Tag {
    fn open(self) -> &'static str {
        match self {
            Tag::B => "<b>",
            Tag::I => "<i>",
            Tag::S => "<s>",
            Tag::M => "<mark>",
            Tag::A(_) => "",
        }
    }
    fn close(self) -> &'static str {
        match self {
            Tag::B => "</b>",
            Tag::I => "</i>",
            Tag::S => "</s>",
            Tag::M => "</mark>",
            Tag::A(_) => "</a>",
        }
    }
}

fn tags_of(a: Attr) -> Vec<Tag> {
    let mut v = Vec::new();
    if a.link != 0 {
        v.push(Tag::A(a.link));
    }
    for (bit, t) in [(B, Tag::B), (I, Tag::I), (S, Tag::S), (M, Tag::M)] {
        if a.f & bit != 0 {
            v.push(t);
        }
    }
    v
}

fn close_tags(out: &mut String, open: &mut Vec<Tag>, keep: usize) {
    while open.len() > keep {
        out.push_str(open.pop().map_or("", Tag::close));
    }
}

fn open_list(k: Kind) -> &'static str {
    if k == Kind::Number {
        "<ol><li>"
    } else {
        "<ul><li>"
    }
}
fn close_list(k: Kind) -> &'static str {
    if k == Kind::Number {
        "</li></ol>"
    } else {
        "</li></ul>"
    }
}

fn push_esc(out: &mut String, c: char) {
    match c {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '\u{a0}' => out.push_str("&nbsp;"),
        c => out.push(c),
    }
}
fn esc_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('\u{a0}', "&nbsp;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written by Chromium's contenteditable (shape: bare first line, `<div>` lines, `<ul>`/`<ol>`, checkbox lines): byte-exact round trip.
    const CANON: &[&str] = &[
        "",
        "hello",
        "Список покупок<div>молоко</div><div>хлеб</div>",
        "first<div><br></div><div>third</div>",
        "<div><br></div><div>after an empty first line</div>",
        "a &amp; b &lt;tag&gt; &nbsp;x",
        "<b>bold</b> and <i>italic</i> and <s>gone</s> and <mark>marked</mark>",
        "<b>bold <i>both</i></b>",
        "<b>x</b><i>y</i>",
        "plain<ul><li>one</li><li><b>two</b></li></ul>",
        "<ol><li>a</li><li>b</li><li>c</li></ol><div>after</div>",
        "<ul><li>top<ul><li>nested</li></ul></li><li>again</li></ul>",
        "<input type=\"checkbox\">&nbsp;todo<div><input type=\"checkbox\" checked=\"\">&nbsp;done</div><div><input type=\"checkbox\">&nbsp;</div>",
        "see <a href=\"https://example.com/a?x=1&amp;y=2\">the <b>site</b></a> now",
        "line one<br>line two<div>next</div>",
        "x<ul><li>y</li></ul><ol><li>z</li></ol><div>w</div>",
        "emoji 😀 и ёлка<div>tab\there</div>",
        "a<input type=\"checkbox\"> inline box",
    ];

    #[test]
    fn canonical_samples_round_trip_byte_for_byte() {
        for html in CANON {
            let d = Doc::from_html(html);
            assert_eq!(&d.to_html(), html, "sample {html:?}");
        }
    }

    #[test]
    fn every_sample_round_trips_semantically() {
        let extra = [
            "<div>a</div><div>b</div>",
            "<div>nested<div>inner</div></div>",
            "<strong>s</strong><em>e</em><del>d</del><h1>head</h1><p>para</p>",
            "<i><b>swapped</b></i>",
            "a\nb\n\nc",
            "<ul>\n<li>one</li>\n<li>two</li>\n</ul>",
            "<ul><li><ul><li>only nested</li></ul></li></ul>",
            "<ul><li>a</li></ul><ol><li>b</li></ol><ul><li>c</li></ul>",
            "<b><i><s><mark>all four</mark></s></i></b>",
            "<script>x</script>kept<style>p{}</style>",
            "<div><input type=\"checkbox\">no nbsp</div>",
            "<ul><li><input type=\"checkbox\">&nbsp;in a list</li></ul>",
            "<a>no href</a> <a href=\"mailto:me@x.org\">mail</a>",
            "x<br><br>y<br>",
            "   leading and trailing   ",
        ];
        for html in CANON.iter().chain(extra.iter()) {
            let d = Doc::from_html(html);
            let again = Doc::from_html(&d.to_html());
            assert_eq!(again, d, "sample {html:?} -> {:?}", d.to_html());
        }
    }

    #[test]
    fn structure_is_read_right() {
        let d = Doc::from_html("Title<div>x</div><ul><li>a</li><li>b</li></ul><input type=\"checkbox\" checked=\"\">&nbsp;t");
        let kinds: Vec<Kind> = d.lines.iter().map(|l| l.kind).collect();
        assert_eq!(kinds, vec![Kind::Plain, Kind::Plain, Kind::Bullet, Kind::Bullet, Kind::Check(true)]);
        assert_eq!(d.lines[4].string(), "t", "the nbsp after the box is not content");
        let d = Doc::from_html("x<br>y");
        assert_eq!(d.lines.len(), 1);
        assert_eq!(d.lines[0].string(), "x\ny");
        let d = Doc::from_html("<div><br></div>");
        assert_eq!((d.lines.len(), d.lines[0].len()), (1, 0));
        let d = Doc::from_html("<b>a</b>b");
        assert_eq!(d.lines[0].at[0].f, B);
        assert_eq!(d.lines[0].at[1].f, 0);
    }

    #[test]
    fn blank_matches_the_web_rule() {
        assert!(Doc::from_html("").is_blank());
        assert!(Doc::from_html("<div><br></div><div> </div>").is_blank());
        assert!(!Doc::from_html("<ul><li></li></ul>").is_blank());
        assert!(!Doc::from_html("<input type=\"checkbox\">&nbsp;").is_blank());
        assert!(!Doc::from_html("x").is_blank());
    }

    #[test]
    fn plain_text_has_one_line_per_paragraph() {
        let d = Doc::from_html("Список<div>молоко</div><ul><li>а</li></ul>");
        assert_eq!(d.plain_text(), "Список\nмолоко\nа");
        assert_eq!(d.text_in(Pos::new(0, 2), Pos::new(1, 3)), "исок\nмол");
    }

    #[test]
    fn editing_ops() {
        let mut d = Doc::from_html("hello<div>world</div>");
        let p = d.insert(Pos::new(0, 5), "!", Attr { f: B, link: 0 });
        assert_eq!(p, Pos::new(0, 6));
        assert_eq!(d.to_html(), "hello<b>!</b><div>world</div>");
        let p = d.split(Pos::new(1, 2));
        assert_eq!(p, Pos::new(2, 0));
        assert_eq!(d.to_html(), "hello<b>!</b><div>wo</div><div>rld</div>");
        let p = d.delete(Pos::new(0, 3), Pos::new(2, 1));
        assert_eq!(p, Pos::new(0, 3));
        assert_eq!(d.to_html(), "helld");
        d.set_flag(Pos::new(0, 0), Pos::new(0, 2), I, true);
        assert!(d.all_have(Pos::new(0, 0), Pos::new(0, 2), I) && !d.all_have(Pos::new(0, 0), Pos::new(0, 3), I));
        assert_eq!(d.to_html(), "<i>he</i>lld");
    }

    #[test]
    fn enter_continues_lists_and_unticks() {
        let mut d = Doc::from_html("<input type=\"checkbox\" checked=\"\">&nbsp;ab");
        d.split(Pos::new(0, 1));
        assert_eq!(d.lines[1].kind, Kind::Check(false));
        assert_eq!(d.to_html(), "<input type=\"checkbox\" checked=\"\">&nbsp;a<div><input type=\"checkbox\">&nbsp;b</div>");
        let mut d = Doc::from_html("<ol><li>ab</li></ol>");
        d.split(Pos::new(0, 2));
        assert_eq!(d.to_html(), "<ol><li>ab</li><li></li></ol>");
    }

    #[test]
    fn numbering_counts_runs_and_levels() {
        let d = Doc::from_html("<ol><li>a</li><li>b<ol><li>x</li><li>y</li></ol></li><li>c</li></ol><div>t</div><ol><li>again</li></ol>");
        let nums: Vec<usize> = (0..d.lines.len()).filter(|i| d.lines[*i].kind == Kind::Number).map(|i| d.number_of(i)).collect();
        assert_eq!(nums, vec![1, 2, 1, 2, 3, 1]);
    }

    #[test]
    fn word_bounds() {
        let d = Doc::from_html("foo, bar_baz");
        assert_eq!(d.word_at(Pos::new(0, 1)), (Pos::new(0, 0), Pos::new(0, 3)));
        assert_eq!(d.word_at(Pos::new(0, 3)), (Pos::new(0, 3), Pos::new(0, 4)));
        assert_eq!(d.word_at(Pos::new(0, 8)), (Pos::new(0, 5), Pos::new(0, 12)));
    }

    #[test]
    fn untouched_html_is_what_the_model_keeps() {
        // The Note tab never sends EditorChanged without an edit, so the stored html stays as is; this pins that parsing does not need to mutate.
        let html = "<b>x</b><div>y</div>";
        assert_eq!(sanitize(html), html);
    }
}
