//! The result list as a view-model: how a backend `Res` becomes a drawn row (localised title / subtitle / label / icon), the hover buttons and the
//! context-menu lines of app / file / folder rows, the selection arithmetic (wrap-around arrows, page jumps, group jumps with Tab), the list diff
//! that drives row insert / remove / slide animations, and the shell-icon cache (`cmd_icon`, 300 entries, oldest dropped first).

use super::keys::{RunMode, PAGE_ROWS};
use super::lang::{Lang, Tr};
use super::protocol::{Kind, Res};
use super::query::{highlight, Seg};
use super::strings as s;
use std::collections::{HashMap, VecDeque};

/// `ICON`: the glyph name of a row kind (names of `icons.js`); unknown kinds draw as a file.
pub fn icon_of(kind: &Kind) -> &'static str {
    match kind {
        Kind::App => "apps",
        Kind::Uninstall => "trash",
        Kind::File => "file",
        Kind::Folder => "folder",
        Kind::Calc => "calc",
        Kind::Convert => "convert",
        Kind::Web => "globe",
        Kind::System => "power",
        Kind::Shell => "terminal",
        Kind::Hint => "sparkles",
        Kind::Ask => "sparkles",
        Kind::AskCopy => "copy",
        Kind::NewConv => "refresh",
        Kind::Clip => "clipboard",
        Kind::Note => "note",
        Kind::Shelf => "shelf",
        Kind::Reminder => "bell",
        Kind::Tab => "home",
        Kind::Setting => "settings",
        Kind::Action => "bolt",
        Kind::Other(_) => "file",
    }
}

/// `PLAIN`: titles that are not searchable text get no match highlight.
pub fn is_plain(kind: &Kind) -> bool {
    matches!(kind, Kind::Calc | Kind::Convert | Kind::Shell | Kind::Web | Kind::Ask | Kind::AskCopy | Kind::NewConv)
}

/// `GROUP`: Tab jumps between these groups of rows.
pub fn group_of(kind: &Kind) -> String {
    match kind {
        Kind::Calc | Kind::Convert => "Result".into(),
        Kind::Shell => "Command".into(),
        Kind::System => "System".into(),
        Kind::App => "Applications".into(),
        Kind::Folder => "Folders".into(),
        Kind::File => "Files".into(),
        Kind::Web => "Web".into(),
        k => k.as_str().to_string(),
    }
}

/// What the localiser needs to know about the AI setup to word the status of an `ask` row (`askStatus()`).
#[derive(Clone, Debug, Default)]
pub struct AskState {
    pub net_ai: bool,
    pub base_url: String,
    pub model: String,
}

impl AskState {
    pub fn status(&self, lang: Lang) -> &'static str {
        if !self.net_ai {
            s::ASK_OFF.tr(lang)
        } else if self.base_url.trim().is_empty() {
            s::ASK_NO_URL.tr(lang)
        } else if self.model.trim().is_empty() {
            s::ASK_NO_MODEL.tr(lang)
        } else {
            s::ASK_GO.tr(lang)
        }
    }
}

/// The localised pieces of one row (`view(it)` of bar.js).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parts {
    pub title: String,
    pub sub: String,
    pub icon: String,
    pub label: String,
}

/// `Search <engine> for "<q>"` (the backend's web row title) -> (engine, q).
fn parse_web_title(title: &str) -> Option<(&str, &str)> {
    let rest = title.strip_prefix("Search ")?;
    let body = rest.strip_suffix('"')?;
    let mut from = 1;
    while let Some(i) = body.get(from..).and_then(|b| b.find(" for \"")) {
        let at = from + i;
        if at >= 1 {
            return Some((&body[..at], &body[at + 6..]));
        }
        from += 1;
    }
    None
}

/// `/\s+-\s+Enter copies$/` removed from a calc / convert subtitle.
fn strip_enter_copies(sub: &str) -> String {
    match sub.strip_suffix("Enter copies") {
        Some(head) => {
            let h = head.trim_end();
            match h.strip_suffix('-') {
                Some(h2) if h2.ends_with(char::is_whitespace) && head.len() > h.len() => h2.trim_end().to_string(),
                _ => sub.to_string(),
            }
        }
        None => sub.to_string(),
    }
}

/// Localises a backend row. The backend speaks English; the known strings get a Russian twin.
pub fn parts(it: &Res, lang: Lang, ask: &AskState) -> Parts {
    let mut title = it.title.clone();
    let mut sub = it.subtitle.clone();
    let mut icon = icon_of(&it.kind).to_string();
    let mut label = String::new();
    let sub_tr = |sub: &str| s::known_sub(sub).map(|t| t.tr(lang).to_string()).unwrap_or_else(|| sub.to_string());
    match &it.kind {
        Kind::System => {
            if let Some((t, ic)) = s::system_row(&it.payload) {
                title = t.tr(lang).to_string();
                icon = ic.to_string();
            }
            label = "Pill".into();
            sub.clear();
        }
        Kind::Web => {
            if let Some((engine, q)) = parse_web_title(&it.title) {
                title = if lang == Lang::Ru { format!("Искать в браузере: «{q}»") } else { format!("Search in the browser: \u{201c}{q}\u{201d}") };
                label = engine.to_string();
            } else {
                label = "Web".into();
            }
            sub.clear();
        }
        Kind::Shell => {
            title = match it.title.strip_prefix("Run: ") {
                Some(r) => format!("{}{r}", if lang == Lang::Ru { "Выполнить: " } else { "Run: " }),
                None => it.title.clone(),
            };
            label = s::SHELL.tr(lang).to_string();
            sub = sub_tr(&sub);
        }
        Kind::Calc | Kind::Convert => {
            label = s::COPY.tr(lang).to_string();
            sub = strip_enter_copies(&it.subtitle);
        }
        Kind::Ask => {
            title = format!("{}: {}", s::ASK.tr(lang), it.payload);
            sub = ask.status(lang).to_string();
            label = "ИИ".into();
        }
        Kind::AskCopy => {
            title = s::COPY_ANS.tr(lang).to_string();
            sub.clear();
            label = s::COPY.tr(lang).to_string();
        }
        Kind::NewConv => {
            title = s::NEW_CONV.tr(lang).to_string();
            sub = s::NEW_CONV_SUB.tr(lang).to_string();
        }
        Kind::Uninstall => {
            label = s::UNINSTALL.tr(lang).to_string();
            if it.payload.ends_with('!') {
                sub = s::UNINSTALL_RISKY.tr(lang).to_string();
            }
        }
        Kind::App => label = s::APP.tr(lang).to_string(),
        Kind::File => label = s::FILE.tr(lang).to_string(),
        Kind::Folder => label = s::FOLDER.tr(lang).to_string(),
        k @ (Kind::Clip | Kind::Note | Kind::Shelf | Kind::Reminder | Kind::Setting | Kind::Action | Kind::Tab) => {
            let l = match k {
                Kind::Clip => s::CLIP,
                Kind::Note => s::NOTE,
                Kind::Shelf => s::SHELF,
                Kind::Reminder => s::REMINDER,
                Kind::Setting => s::SETTING,
                Kind::Action => s::ACTION,
                _ => s::TAB,
            };
            label = l.tr(lang).to_string();
            sub = sub_tr(&sub);
            if *k == Kind::Tab {
                icon = it.payload.clone();
            }
        }
        _ => sub = sub_tr(&sub),
    }
    Parts { title, sub, icon, label }
}

/// A hover button or context-menu line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub mode: RunMode,
    pub icon: &'static str,
    pub label: String,
    /// the shortcut shown next to a context-menu line ("" = none)
    pub key: &'static str,
}

/// Buttons on the selected app / file / folder row: copy path, run as administrator (not folders), pin, uninstall (apps), show in the folder.
pub fn row_buttons(kind: &Kind, lang: Lang) -> Vec<Action> {
    if !kind.is_pathy() {
        return Vec::new();
    }
    let a = |mode, icon, tip: s::S| Action { mode, icon, label: tip.tr(lang).to_string(), key: "" };
    let mut v = vec![a(RunMode::Path, "copy", s::PATH_TIP)];
    if *kind != Kind::Folder {
        v.push(a(RunMode::Admin, "shield", s::ADMIN_TIP));
    }
    if kind.pinnable() {
        v.push(a(RunMode::Pin, "pin", s::PIN_TIP));
    }
    if *kind == Kind::App {
        v.push(a(RunMode::Uninstall, "trash", s::UNINST_TIP));
    }
    v.push(a(RunMode::Folder, "folderOpen", s::FOLDER_TIP));
    v
}

/// The right-click menu of an app / file / folder row (other kinds have none).
pub fn context_lines(kind: &Kind, lang: Lang) -> Vec<Action> {
    if !kind.is_pathy() {
        return Vec::new();
    }
    let a = |mode, icon, l: s::S, key| Action { mode, icon, label: l.tr(lang).to_string(), key };
    let mut v = vec![a(RunMode::Open, "enter", s::CTX_OPEN, "Enter"), a(RunMode::Folder, "folderOpen", s::CTX_FOLDER, "Ctrl+Enter"), a(RunMode::Path, "copy", s::CTX_PATH, "Ctrl+Shift+C")];
    if *kind != Kind::Folder {
        v.push(a(RunMode::Admin, "shield", s::CTX_ADMIN, "Ctrl+Shift+Enter"));
    }
    if kind.pinnable() {
        v.push(a(RunMode::Pin, "pin", s::CTX_PIN, "Ctrl+P"));
    }
    if *kind == Kind::App {
        v.push(a(RunMode::Uninstall, "trash", s::CTX_UNINSTALL, ""));
    }
    v
}

/// Where the context menu goes: kept 6 px inside the window, the animation origin is the cursor relative to the menu's corner.
/// Returns `(left, top, origin_x, origin_y)`.
pub fn context_position(x: f32, y: f32, menu_w: f32, menu_h: f32, win_w: f32, win_h: f32) -> (f32, f32, f32, f32) {
    let lx = 6.0f32.max(x.min(win_w - menu_w - 6.0));
    let ly = 6.0f32.max(y.min(win_h - menu_h - 6.0));
    (lx, ly, x - lx, y - ly)
}

/// One drawn row.
#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    pub index: usize,
    pub id: String,
    pub kind: Kind,
    /// the title cut into plain / `<mark>` pieces (a single plain piece for kinds without highlight)
    pub title: Vec<Seg>,
    pub sub: String,
    /// glyph name (`icons.js`); `shell_icon` replaces it once loaded
    pub icon: String,
    pub shell_icon: Option<String>,
    /// the right-hand label ("Application", "Copy", ...); `Press Enter again` while a confirm is pending
    pub label: String,
    pub selected: bool,
    /// a dangerous row waits for the second Enter (red background)
    pub confirm: bool,
    /// calc / convert results draw big
    pub big: bool,
    /// the tooltip on app / file / folder rows
    pub tooltip: Option<String>,
    /// hover buttons, shown on the selected row
    pub buttons: Vec<Action>,
    /// the subtitle is hidden for app rows
    pub show_sub: bool,
}

/// Builds the row for result `it` at `index`.
pub fn row_view(it: &Res, index: usize, lang: Lang, ask: &AskState, words: &[String]) -> RowView {
    let p = parts(it, lang, ask);
    let title = if !is_plain(&it.kind) && it.kind != Kind::System { highlight(&p.title, words) } else { vec![Seg { text: p.title.clone(), mark: false }] };
    RowView {
        index,
        id: it.id.clone(),
        kind: it.kind.clone(),
        title,
        show_sub: !p.sub.is_empty() && it.kind != Kind::App,
        sub: p.sub,
        icon: p.icon,
        shell_icon: None,
        label: p.label,
        selected: false,
        confirm: false,
        big: matches!(it.kind, Kind::Calc | Kind::Convert),
        tooltip: it.kind.is_pathy().then(|| s::OPEN_TIP.tr(lang).to_string()),
        buttons: row_buttons(&it.kind, lang),
    }
}

// ---- selection ----

/// `select(i)`: wrap-around (`(i + n) % n`); `None` for an empty list.
pub fn wrap(i: i64, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    Some(i.rem_euclid(n as i64) as usize)
}

/// PageUp / PageDown: `PAGE_ROWS` rows, clamped to the list (no wrap).
pub fn page(sel: usize, dir: i32, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let target = sel as i64 + dir as i64 * PAGE_ROWS as i64;
    Some(target.clamp(0, n as i64 - 1) as usize)
}

/// Tab / Shift+Tab: the first row of the next group, or the first row of the previous group (`nextGroup`).
pub fn next_group(items: &[Res], sel: usize, dir: i32) -> Option<usize> {
    let n = items.len();
    if n == 0 {
        return None;
    }
    let sel = sel.min(n - 1);
    let g = |i: usize| group_of(&items[i].kind);
    let here = g(sel);
    let mut i = sel as i64;
    for _ in 0..n {
        i = (i + dir as i64).rem_euclid(n as i64);
        if g(i as usize) != here {
            break;
        }
    }
    let mut i = i as usize;
    if dir < 0 {
        while i > 0 && g(i - 1) == g(i) {
            i -= 1;
        }
    }
    Some(i)
}

// ---- list diff (row insert / remove / slide) ----

/// What changed between two renders, keyed by result id: the FLIP animation inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListDiff {
    /// new indexes of rows that did not exist before (drop in)
    pub inserted: Vec<usize>,
    /// (new index, old index) of survivors whose position changed (slide)
    pub moved: Vec<(usize, usize)>,
    /// old indexes of rows that are gone (fade out as ghosts)
    pub removed: Vec<usize>,
}

pub fn diff(old: &[String], new: &[String]) -> ListDiff {
    let old_at: HashMap<&str, usize> = old.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let new_at: HashMap<&str, usize> = new.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
    let mut d = ListDiff::default();
    for (i, id) in new.iter().enumerate() {
        match old_at.get(id.as_str()) {
            None => d.inserted.push(i),
            Some(&o) if o != i => d.moved.push((i, o)),
            _ => {}
        }
    }
    for (o, id) in old.iter().enumerate() {
        if !new_at.contains_key(id.as_str()) {
            d.removed.push(o);
        }
    }
    d
}

// ---- shell icons ----

/// Max kept shell icons per session (`ICONS_MAX`).
pub const ICONS_MAX: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconState {
    /// requested, not answered yet
    Pending,
    /// answered; empty string = the backend has none
    Loaded(String),
}

/// result id -> data URL, bounded; the oldest answered entry drops out beyond `ICONS_MAX`.
#[derive(Clone, Debug, Default)]
pub struct IconCache {
    map: HashMap<String, IconState>,
    order: VecDeque<String>,
}

impl IconCache {
    pub fn new() -> IconCache {
        IconCache::default()
    }
    /// True when `cmd_icon` has to be called for this id (neither known nor already in flight); marks it pending.
    pub fn want(&mut self, id: &str) -> bool {
        if self.map.contains_key(id) {
            return false;
        }
        self.map.insert(id.to_string(), IconState::Pending);
        true
    }
    /// The answer of `cmd_icon` (`None` = failed / none). Returns the data URL to draw, if any.
    pub fn put(&mut self, id: &str, url: Option<String>) -> Option<String> {
        let u = url.unwrap_or_default();
        if !matches!(self.map.get(id), Some(IconState::Loaded(_))) {
            self.order.push_back(id.to_string());
        }
        self.map.insert(id.to_string(), IconState::Loaded(u.clone()));
        while self.order.len() > ICONS_MAX {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
        (!u.is_empty()).then_some(u)
    }
    pub fn get(&self, id: &str) -> Option<&str> {
        match self.map.get(id) {
            Some(IconState::Loaded(u)) if !u.is_empty() => Some(u),
            _ => None,
        }
    }
    pub fn known(&self, id: &str) -> bool {
        self.map.contains_key(id)
    }
    pub fn len(&self) -> usize {
        self.order.len()
    }
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    /// A pending request whose reply never came (the bar closed) is forgotten so the row asks again next time.
    pub fn forget_pending(&mut self) {
        self.map.retain(|_, v| !matches!(v, IconState::Pending));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(kind: Kind, id: &str, title: &str, sub: &str, payload: &str) -> Res {
        Res::new(kind, id, title, sub, payload)
    }
    fn p(it: &Res, lang: Lang) -> Parts {
        parts(it, lang, &AskState::default())
    }

    #[test]
    fn system_rows_localise_and_take_the_pill_label() {
        let it = r(Kind::System, "sys:lock", "Lock", "Press Enter twice to confirm", "lock");
        let ru = p(&it, Lang::Ru);
        assert_eq!((ru.title.as_str(), ru.icon.as_str(), ru.label.as_str(), ru.sub.as_str()), ("Заблокировать компьютер", "lock", "Pill", ""));
        assert_eq!(p(&it, Lang::En).title, "Lock the computer");
        let odd = r(Kind::System, "sys:x", "Mystery", "", "x");
        assert_eq!(p(&odd, Lang::En).title, "Mystery");
        assert_eq!(p(&odd, Lang::En).icon, "power");
    }

    #[test]
    fn web_rows() {
        let it = r(Kind::Web, "web:cats", "Search Google for \"cats\"", "Opens your default browser", "https://x");
        let ru = p(&it, Lang::Ru);
        assert_eq!((ru.title.as_str(), ru.label.as_str(), ru.sub.as_str()), ("Искать в браузере: «cats»", "Google", ""));
        assert_eq!(p(&it, Lang::En).title, "Search in the browser: \u{201c}cats\u{201d}");
        let cyr = r(Kind::Web, "web:q", "Search Bing for \"привет мир\"", "", "u");
        assert_eq!(p(&cyr, Lang::Ru).title, "Искать в браузере: «привет мир»");
        assert_eq!(p(&cyr, Lang::Ru).label, "Bing");
        let nomatch = r(Kind::Web, "w", "Something else", "", "");
        assert_eq!(p(&nomatch, Lang::En).label, "Web");
        assert_eq!(parse_web_title("Search A for \"b for \"c\""), Some(("A", "b for \"c")));
    }

    #[test]
    fn shell_calc_and_convert() {
        let sh = r(Kind::Shell, "shell:dir", "Run: dir /w", "cmd /c, hidden window", "dir /w");
        let ru = p(&sh, Lang::Ru);
        assert_eq!((ru.title.as_str(), ru.label.as_str(), ru.sub.as_str()), ("Выполнить: dir /w", "Команда", "cmd /c, окно скрыто"));
        assert_eq!(p(&sh, Lang::En).title, "Run: dir /w");
        let calc = r(Kind::Calc, "calc:40", "= 40", "5*8  -  Enter copies", "40");
        let c = p(&calc, Lang::En);
        assert_eq!((c.sub.as_str(), c.label.as_str(), c.icon.as_str()), ("5*8", "Copy", "calc"));
        assert_eq!(strip_enter_copies("a - Enter copies"), "a");
        assert_eq!(strip_enter_copies("a-Enter copies"), "a-Enter copies", "needs blanks around the dash");
        assert_eq!(strip_enter_copies("plain"), "plain");
        let conv = r(Kind::Convert, "convert:3.1", "3.1 mi", "5 km  -  Enter copies", "3.1 mi");
        assert_eq!(p(&conv, Lang::Ru).label, "Скопировать");
    }

    #[test]
    fn ask_rows_report_the_ai_setup() {
        let it = r(Kind::Ask, "ask:q", "", "", "why?");
        assert_eq!(parts(&it, Lang::En, &AskState::default()).sub, s::ASK_OFF.1);
        let on = AskState { net_ai: true, base_url: "http://x".into(), model: String::new() };
        assert_eq!(parts(&it, Lang::En, &on).sub, s::ASK_NO_MODEL.1);
        let no_url = AskState { net_ai: true, base_url: " ".into(), model: "m".into() };
        assert_eq!(parts(&it, Lang::Ru, &no_url).sub, s::ASK_NO_URL.0);
        let ready = AskState { net_ai: true, base_url: "u".into(), model: "m".into() };
        let q = parts(&it, Lang::Ru, &ready);
        assert_eq!((q.title.as_str(), q.sub.as_str(), q.label.as_str()), ("Спросить ИИ: why?", "Enter — отправить", "ИИ"));
        let cp = parts(&r(Kind::AskCopy, "ask:copy", "", "", "text"), Lang::En, &ready);
        assert_eq!((cp.title.as_str(), cp.label.as_str()), ("Copy the answer", "Copy"));
        let nc = parts(&r(Kind::NewConv, "ask:new", "", "", ""), Lang::En, &ready);
        assert_eq!((nc.title.as_str(), nc.sub.as_str(), nc.label.as_str()), ("New conversation", "forget the history and search as usual", ""));
    }

    #[test]
    fn uninstall_app_and_in_app_rows() {
        let risky = r(Kind::Uninstall, "uninstall:3", "Microsoft Visual C++", "x", "3!");
        let u = p(&risky, Lang::En);
        assert_eq!((u.label.as_str(), u.sub.as_str()), ("Uninstall", s::UNINSTALL_RISKY.1));
        let safe = r(Kind::Uninstall, "uninstall:4", "qBittorrent", "qBittorrent · 4.6", "4");
        assert_eq!(p(&safe, Lang::En).sub, "qBittorrent · 4.6");
        assert_eq!(p(&r(Kind::App, "a", "Notepad", "Application", "x"), Lang::Ru).label, "Приложение");
        assert_eq!(p(&r(Kind::Folder, "f", "Docs", "C:\\Docs", "C:\\Docs"), Lang::Ru).label, "Папка");
        let tab = r(Kind::Tab, "tab:note", "Notes", "Pill tab, Notes", "note");
        let t = p(&tab, Lang::En);
        assert_eq!((t.icon.as_str(), t.label.as_str()), ("note", "Tab"), "a tab row draws its own tab's glyph");
        assert_eq!(p(&r(Kind::Clip, "clip:1", "text", "x", "1"), Lang::Ru).label, "Буфер");
        let other = r(Kind::Other("zz".into()), "z", "T", "Opens your default browser", "");
        assert_eq!(p(&other, Lang::Ru).sub, "Откроется в браузере по умолчанию");
        assert_eq!(p(&other, Lang::Ru).icon, "file");
    }

    #[test]
    fn buttons_and_context_lines() {
        let modes = |v: Vec<Action>| v.into_iter().map(|a| a.mode).collect::<Vec<_>>();
        assert_eq!(modes(row_buttons(&Kind::App, Lang::En)), vec![RunMode::Path, RunMode::Admin, RunMode::Pin, RunMode::Uninstall, RunMode::Folder]);
        assert_eq!(modes(row_buttons(&Kind::File, Lang::En)), vec![RunMode::Path, RunMode::Admin, RunMode::Pin, RunMode::Folder]);
        assert_eq!(modes(row_buttons(&Kind::Folder, Lang::En)), vec![RunMode::Path, RunMode::Pin, RunMode::Folder]);
        assert!(row_buttons(&Kind::Calc, Lang::En).is_empty());
        assert_eq!(modes(context_lines(&Kind::App, Lang::En)), vec![RunMode::Open, RunMode::Folder, RunMode::Path, RunMode::Admin, RunMode::Pin, RunMode::Uninstall]);
        assert_eq!(modes(context_lines(&Kind::Folder, Lang::En)), vec![RunMode::Open, RunMode::Folder, RunMode::Path, RunMode::Pin]);
        let app = context_lines(&Kind::App, Lang::Ru);
        assert_eq!(app.last().unwrap().label, "Удалить…");
        assert_eq!(app.last().unwrap().key, "");
        assert_eq!(app[1].key, "Ctrl+Enter");
        assert!(context_lines(&Kind::Clip, Lang::En).is_empty(), "other kinds have no menu");
    }

    #[test]
    fn context_menu_stays_inside_the_window() {
        assert_eq!(context_position(100.0, 50.0, 250.0, 200.0, 560.0, 350.0), (100.0, 50.0, 0.0, 0.0));
        let (lx, ly, ox, oy) = context_position(500.0, 300.0, 250.0, 200.0, 560.0, 350.0);
        assert_eq!((lx, ly), (304.0, 144.0));
        assert_eq!((ox, oy), (196.0, 156.0), "the menu opens from the cursor");
        assert_eq!(context_position(0.0, 0.0, 250.0, 200.0, 560.0, 350.0).0, 6.0);
    }

    #[test]
    fn row_view_highlights_searchable_titles_only() {
        let words = vec!["note".to_string()];
        let app = row_view(&r(Kind::App, "app:n", "Notepad Notes", "Application", "x"), 0, Lang::En, &AskState::default(), &words);
        assert!(app.title.iter().any(|s| s.mark));
        assert!(!app.show_sub, "app rows hide the subtitle");
        assert!(app.tooltip.is_some() && app.buttons.len() == 5);
        let calc = row_view(&r(Kind::Calc, "calc:1", "= note", "a  -  Enter copies", "1"), 1, Lang::En, &AskState::default(), &words);
        assert!(calc.title.iter().all(|s| !s.mark) && calc.big && calc.show_sub);
        let cyr = row_view(&r(Kind::Note, "note:1", "Заметка про лето", "…", "1"), 2, Lang::Ru, &AskState::default(), &["про".to_string()]);
        assert_eq!(cyr.title.iter().filter(|s| s.mark).map(|s| s.text.as_str()).collect::<Vec<_>>(), vec!["про"]);
        let sys = row_view(&r(Kind::System, "sys:lock", "Lock", "x", "lock"), 3, Lang::En, &AskState::default(), &["lock".to_string()]);
        assert!(sys.title.iter().all(|s| !s.mark), "system titles are not highlighted");
    }

    #[test]
    fn selection_wraps_and_pages() {
        assert_eq!(wrap(5, 5), Some(0));
        assert_eq!(wrap(-1, 5), Some(4));
        assert_eq!(wrap(2, 5), Some(2));
        assert_eq!(wrap(1, 0), None);
        assert_eq!(page(0, 1, 20), Some(PAGE_ROWS));
        assert_eq!(page(3, -1, 20), Some(0));
        assert_eq!(page(18, 1, 20), Some(19));
        assert_eq!(page(0, 1, 0), None);
    }

    fn rows(kinds: &[Kind]) -> Vec<Res> {
        kinds.iter().enumerate().map(|(i, k)| r(k.clone(), &format!("id{i}"), "t", "", "")).collect()
    }

    #[test]
    fn tab_jumps_between_groups() {
        // Result, Applications x3, Folders, Files x2, Web
        let items = rows(&[Kind::Calc, Kind::App, Kind::App, Kind::App, Kind::Folder, Kind::File, Kind::File, Kind::Web]);
        assert_eq!(next_group(&items, 0, 1), Some(1));
        assert_eq!(next_group(&items, 1, 1), Some(4));
        assert_eq!(next_group(&items, 2, 1), Some(4));
        assert_eq!(next_group(&items, 5, 1), Some(7));
        assert_eq!(next_group(&items, 7, 1), Some(0), "wraps around");
        assert_eq!(next_group(&items, 4, -1), Some(1), "lands on the first row of the previous group");
        assert_eq!(next_group(&items, 6, -1), Some(4), "Shift+Tab walks back to the previous group and lands on its head");
        assert_eq!(next_group(&items, 3, -1), Some(0));
        assert_eq!(next_group(&items, 0, -1), Some(7));
        assert_eq!(next_group(&[], 0, 1), None);
        // calc and convert are one group
        let cc = rows(&[Kind::Calc, Kind::Convert, Kind::App]);
        assert_eq!(next_group(&cc, 0, 1), Some(2));
        // a single group: Tab stays, Shift+Tab goes to the head
        let one = rows(&[Kind::App, Kind::App, Kind::App]);
        assert_eq!(next_group(&one, 1, 1), Some(1));
        assert_eq!(next_group(&one, 2, -1), Some(0));
    }

    #[test]
    fn diff_finds_inserts_moves_and_removals() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let d = diff(&s(&["a", "b", "c"]), &s(&["b", "x", "a"]));
        assert_eq!(d.inserted, vec![1]);
        assert_eq!(d.moved, vec![(0, 1), (2, 0)]);
        assert_eq!(d.removed, vec![2]);
        assert_eq!(diff(&s(&["a"]), &s(&["a"])), ListDiff::default());
        assert_eq!(diff(&[], &s(&["a", "b"])).inserted, vec![0, 1]);
    }

    #[test]
    fn icon_cache_dedupes_requests_and_drops_the_oldest() {
        let mut c = IconCache::new();
        assert!(c.want("a"));
        assert!(!c.want("a"), "in flight: no second request");
        assert_eq!(c.put("a", Some("data:x".into())), Some("data:x".into()));
        assert_eq!(c.get("a"), Some("data:x"));
        assert!(!c.want("a"));
        assert!(c.want("b"));
        assert_eq!(c.put("b", None), None, "no icon: remembered as empty, glyph stays");
        assert!(!c.want("b") && c.get("b").is_none());
        for i in 0..(ICONS_MAX + 5) {
            let id = format!("k{i}");
            c.want(&id);
            c.put(&id, Some("u".into()));
        }
        assert_eq!(c.len(), ICONS_MAX);
        assert!(!c.known("a"), "oldest entries dropped");
        assert!(c.known(&format!("k{}", ICONS_MAX + 4)));
        c.want("pend");
        c.forget_pending();
        assert!(c.want("pend"));
    }
}
