//! Clipboard tab: history list with search, pins, click-to-paste, plain-text paste, image thumbnails, press-twice wipe, private mode.
//! Port of `src/tab-clipboard.js`.
//!
//! Backend: `clip_list {query, kind, limit, offset}`, `clip_image {id}`, `clip_paste {id, copyOnly, plain}` (answers `pasted |
//! copied_only | copied | gone`), `clip_pin {id, pinned}`, `clip_delete {id}`, `clip_wipe`, `clip_status` (`{paused, count, imageBytes}`).
//! Events: `clip:added` (list refreshes after 120 ms), `clip:open` (hotkey: open the tab, caret into the search box after 320 ms).
//! Settings: `clipClickPaste` (false = a plain click only copies).
//! Click / Enter pastes into the window that had focus and closes the island; Shift+click copies only and stays open; the "T"
//! button pastes as plain text; the sparkles button hands the item to the AI-actions panel (set B draws it).

use super::anim::Anim;
use super::util::files_word;
use super::{bool_of, i64_of, str_of, BackendCall, Cfg, Fx, FxQueue, Tr};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub const SEARCH_DEBOUNCE_MS: f64 = 150.0;
pub const REFRESH_SOON_MS: f64 = 120.0;
pub const WIPE_ARM_MS: f64 = 3000.0;
/// Time the search box needs to hand focus back before a paste is sent.
pub const BLUR_WAIT_MS: f64 = 120.0;
pub const OPEN_FOCUS_MS: f64 = 320.0;
pub const LIMIT: i64 = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub id: i64,
    /// `text` | `image` | `files`
    pub kind: String,
    pub preview: String,
    pub truncated: bool,
    pub file_count: i64,
    pub pinned: bool,
    pub source: String,
    pub used_at: i64,
}

impl Clip {
    pub fn from_json(v: &Value) -> Clip {
        Clip {
            id: i64_of(v, "id"),
            kind: str_of(v, "kind").to_string(),
            preview: str_of(v, "preview").to_string(),
            truncated: bool_of(v, "truncated"),
            file_count: i64_of(v, "fileCount"),
            pinned: bool_of(v, "pinned"),
            source: str_of(v, "source").to_string(),
            used_at: i64_of(v, "usedAt"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    /// `shift` = copy only
    Enter { shift: bool },
    Escape,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// The search box text changed (debounced 150 ms).
    Search(String),
    /// Click on a row (`shift` = copy only).
    Activate { id: i64, shift: bool },
    /// The "T" button.
    PastePlain(i64),
    /// The sparkles button (text items only).
    Ai(i64),
    Pin(i64),
    Delete(i64),
    /// The trash button in the bar: first click arms, second within 3 s wipes.
    Wipe,
    /// Pointer moved over row `index`: selects it without scrolling.
    Hover(usize),
    Key(Key),
    /// A printable key pressed outside a text field: hands the keyboard to the search box (`/` is consumed).
    Typed(char),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Tile {
    /// Thumbnail data URL once `clip_image` answered.
    Image(Option<String>),
    Files,
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub id: i64,
    pub index: usize,
    pub selected: bool,
    pub pinned: bool,
    pub tile: Tile,
    pub title: String,
    /// `3 файла` for file items.
    pub file_count: Option<String>,
    pub source: String,
    pub ago: String,
    pub plain_button: Option<String>,
    pub ai_button: Option<String>,
    pub pin_title: String,
    pub delete_title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub search_placeholder: String,
    pub wipe_title: String,
    pub wipe_armed: bool,
    pub rows: Vec<Row>,
    /// Shown when there are no rows.
    pub empty: Option<String>,
    pub hint: String,
    pub private_chip: Option<String>,
    pub private_mode: bool,
}

pub struct Clipboard {
    cfg: Cfg,
    mono: f64,
    items: Vec<Clip>,
    query: String,
    sel: usize,
    shown: bool,
    paused: bool,
    wipe_arm: Option<f64>,
    load_at: Option<f64>,
    enter: bool,
    fresh: bool,
    had_rows: bool,
    img_cache: HashMap<i64, Option<String>>,
    img_inflight: HashSet<i64>,
    search_focused: bool,
    ai_open: bool,
    pending_paste: Option<(f64, i64, bool)>,
    pending_plain: bool,
    wipe_toast: bool,
    focus_at: Option<f64>,
    fx: FxQueue,
}

impl Default for Clipboard {
    fn default() -> Self {
        Clipboard::new()
    }
}

/// `ago`: `сейчас`, `5 мин`, `3 ч`, `2 д`.
pub fn ago(tr: &dyn Tr, now_ms: i64, ts: i64) -> String {
    let m = (now_ms - ts).div_euclid(60_000);
    if m < 1 {
        tr.t("сейчас", "now")
    } else if m < 60 {
        format!("{}{}", m, tr.t(" мин", " min"))
    } else if m < 1440 {
        format!("{}{}", m / 60, tr.t(" ч", " h"))
    } else {
        format!("{}{}", m / 1440, tr.t(" д", " d"))
    }
}

fn base_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

impl Clipboard {
    pub fn new() -> Clipboard {
        Clipboard {
            cfg: Cfg::new(),
            mono: 0.0,
            items: vec![],
            query: String::new(),
            sel: 0,
            shown: false,
            paused: false,
            wipe_arm: None,
            load_at: None,
            enter: false,
            fresh: false,
            had_rows: false,
            img_cache: HashMap::new(),
            img_inflight: HashSet::new(),
            search_focused: false,
            ai_open: false,
            pending_paste: None,
            pending_plain: false,
            wipe_toast: false,
            focus_at: None,
            fx: FxQueue::default(),
        }
    }

    pub fn set_settings(&mut self, v: &Value) {
        self.cfg.apply(v);
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.cfg.os_reduce = on;
    }
    /// The Settings tab's private-mode switch (`P.clipPaused`).
    pub fn set_paused(&mut self, v: bool) {
        self.paused = v;
    }
    /// The renderer reports where the keyboard is.
    pub fn set_search_focused(&mut self, v: bool) {
        self.search_focused = v;
    }
    /// The AI-actions panel owns the keys while it is open.
    pub fn set_ai_open(&mut self, v: bool) {
        self.ai_open = v;
    }
    pub fn take_fx(&mut self) -> Vec<Fx> {
        self.fx.take()
    }
    pub fn items(&self) -> &[Clip] {
        &self.items
    }
    pub fn selected(&self) -> usize {
        self.sel
    }
    pub fn query(&self) -> &str {
        &self.query
    }
    /// The first Esc clears the query, the next one closes the island.
    pub fn wants_escape(&self) -> bool {
        self.shown && !self.query.is_empty()
    }
    fn reduce(&self) -> bool {
        self.cfg.reduce_motion()
    }

    // ---------------------------------------------------------------- loading
    fn load(&mut self) -> Vec<BackendCall> {
        self.load_at = None;
        let q = self.query.trim();
        vec![BackendCall::new("clip_list", json!({ "query": if q.is_empty() { Value::Null } else { json!(q) }, "kind": null, "limit": LIMIT, "offset": 0 }))]
    }

    fn set_sel(&mut self, i: isize, scroll: bool) {
        let max = self.items.len() as isize - 1;
        self.sel = i.clamp(0, max.max(0)) as usize;
        if scroll {
            if let Some(it) = self.items.get(self.sel) {
                self.fx.push(Fx::ScrollTo { target: format!("row:{}", it.id) });
            }
        }
    }

    fn render(&mut self) -> Vec<BackendCall> {
        let reduce = self.reduce();
        if self.items.is_empty() {
            if self.had_rows || self.enter {
                self.fx.anim(reduce, "empty", Anim::enter(6.0)); // rows -> empty state (not on every keystroke inside "nothing found")
            }
            self.had_rows = false;
            self.enter = false;
            self.fresh = false;
            return vec![];
        }
        // first render of a visit: the stagger; later renders: rows that stay slide to their new spot, a copied item drops in
        if self.enter {
            for (i, it) in self.items.iter().enumerate() {
                self.fx.anim(reduce, format!("row:{}", it.id), Anim::row_stagger(i));
            }
        } else {
            self.fx.anim(reduce, "list", Anim::flip(self.fresh));
        }
        self.enter = false;
        self.fresh = false;
        self.had_rows = true;
        let want: Vec<i64> = self.items.iter().filter(|i| i.kind == "image" && !self.img_cache.contains_key(&i.id) && !self.img_inflight.contains(&i.id)).map(|i| i.id).collect();
        want.into_iter()
            .map(|id| {
                self.img_inflight.insert(id);
                BackendCall::new("clip_image", json!({ "id": id }))
            })
            .collect()
    }

    // ---------------------------------------------------------------- lifecycle and events
    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.shown = true;
        self.sel = 0;
        self.enter = true;
        vec![BackendCall::bare("clip_status")]
    }

    pub fn on_hide(&mut self) {
        self.shown = false;
        self.wipe_arm = None;
        self.fx.push(Fx::CloseClipAi);
    }

    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        match name {
            "clip:added" => {
                if self.shown {
                    self.fresh = true;
                    self.load_at = Some(self.mono + REFRESH_SOON_MS);
                }
                vec![]
            }
            "clip:open" => {
                self.fx.push(Fx::Expand { tab: "clipboard" });
                self.focus_at = Some(self.mono + OPEN_FOCUS_MS);
                vec![]
            }
            "settings:changed" => {
                self.cfg.apply(payload);
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        self.mono += dt_ms;
        let mut calls = vec![];
        if self.wipe_arm.map_or(false, |at| at <= self.mono) {
            self.wipe_arm = None;
        }
        if self.load_at.map_or(false, |at| at <= self.mono) {
            calls.extend(self.load());
        }
        if let Some((at, id, copy_only)) = self.pending_paste {
            if at <= self.mono {
                self.pending_paste = None;
                let plain = std::mem::take(&mut self.pending_plain);
                calls.push(BackendCall::new("clip_paste", json!({ "id": id, "copyOnly": copy_only, "plain": plain })));
            }
        }
        if self.focus_at.map_or(false, |at| at <= self.mono) {
            self.focus_at = None;
            if self.shown {
                self.fx.push(Fx::FocusSearch);
            }
        }
        calls
    }

    // ---------------------------------------------------------------- actions
    fn activate(&mut self, id: i64, shift: bool, plain: bool) -> Vec<BackendCall> {
        if !self.items.iter().any(|i| i.id == id) {
            return vec![];
        }
        let copy_only = shift || self.cfg.get("clipClickPaste") == Some(&Value::Bool(false));
        if self.search_focused {
            // let pill_activate(false) hand focus back first
            self.search_focused = false;
            self.fx.push(Fx::BlurSearch);
            self.pending_paste = Some((self.mono + BLUR_WAIT_MS, id, copy_only));
            self.pending_plain = plain;
            return vec![];
        }
        vec![BackendCall::new("clip_paste", json!({ "id": id, "copyOnly": copy_only, "plain": plain }))]
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Search(q) => {
                self.query = q;
                self.load_at = Some(self.mono + SEARCH_DEBOUNCE_MS);
                vec![]
            }
            Action::Activate { id, shift } => self.activate(id, shift, false),
            Action::PastePlain(id) => self.activate(id, false, true),
            Action::Ai(id) => {
                self.fx.push(Fx::OpenClipAi { id });
                vec![]
            }
            Action::Pin(id) => {
                let pinned = !self.items.iter().find(|i| i.id == id).map_or(false, |i| i.pinned);
                vec![BackendCall::new("clip_pin", json!({ "id": id, "pinned": pinned }))]
            }
            Action::Delete(id) => {
                self.img_cache.remove(&id);
                self.fx.anim(self.reduce(), format!("row:{id}"), Anim::collapse());
                vec![BackendCall::new("clip_delete", json!({ "id": id }))]
            }
            Action::Wipe => {
                if self.wipe_arm.is_none() {
                    self.wipe_arm = Some(self.mono + WIPE_ARM_MS); // confirm step: second click within 3 s
                    return vec![];
                }
                self.wipe_arm = None;
                let reduce = self.reduce();
                for (i, it) in self.items.iter().enumerate() {
                    self.fx.anim(reduce, format!("row:{}", it.id), Anim::exit(-4.0, 1.0).delayed(i.min(8) as u32 * 20));
                }
                vec![BackendCall::bare("clip_wipe")]
            }
            Action::Hover(i) => {
                if i != self.sel && i < self.items.len() {
                    self.set_sel(i as isize, false);
                }
                vec![]
            }
            Action::Key(k) => {
                if !self.shown || self.ai_open {
                    return vec![];
                }
                if k == Key::Escape {
                    if !self.query.is_empty() {
                        self.query.clear();
                        return self.load();
                    }
                    return vec![];
                }
                if self.items.is_empty() {
                    return vec![];
                }
                match k {
                    Key::Down => self.set_sel(self.sel as isize + 1, true),
                    Key::Up => self.set_sel(self.sel as isize - 1, true),
                    Key::Enter { shift } => {
                        let id = self.items[self.sel.min(self.items.len() - 1)].id;
                        return self.activate(id, shift, false);
                    }
                    Key::Escape => {}
                }
                vec![]
            }
            Action::Typed(_) => {
                if self.shown && !self.ai_open && !self.search_focused {
                    self.search_focused = true;
                    self.fx.push(Fx::FocusSearch);
                }
                vec![]
            }
        }
    }

    pub fn on_result(&mut self, cmd: &str, args: &Value, res: &Value, tr: &dyn Tr) -> Vec<BackendCall> {
        match cmd {
            "clip_status" => {
                if res.is_object() {
                    self.paused = bool_of(res, "paused");
                }
                self.load()
            }
            "clip_list" => {
                self.items = res.as_array().map(|a| a.iter().map(Clip::from_json).collect()).unwrap_or_default();
                self.sel = self.sel.min(self.items.len().saturating_sub(1));
                let calls = self.render();
                if self.wipe_toast {
                    self.wipe_toast = false;
                    self.fx.toast("clip-wipe", "trash", tr.t("Буфер очищен", "Clipboard cleared"), 1800);
                }
                calls
            }
            "clip_image" => {
                let id = i64_of(args, "id");
                self.img_inflight.remove(&id);
                self.img_cache.insert(id, res.as_str().map(str::to_string));
                vec![]
            }
            "clip_paste" => {
                let copy_only = bool_of(args, "copyOnly");
                let r = res.as_str().unwrap_or("");
                let title = match r {
                    "pasted" => tr.t("Вставлено", "Pasted"),
                    "copied" => tr.t("Скопировано", "Copied"),
                    "copied_only" => tr.t("Скопировано (цель с правами админа)", "Copied (target is elevated)"),
                    _ => tr.t("Записи уже нет", "Item is gone"),
                };
                self.fx.toast("clip-paste", if copy_only { "copy" } else { "clipboard" }, title, 1800);
                if r == "pasted" || r == "copied_only" {
                    self.fx.push(Fx::Collapse { via: Some("click") }); // a click that closes the island squashes deeper (measured)
                }
                self.load()
            }
            "clip_pin" | "clip_delete" => self.load(),
            "clip_wipe" => {
                self.img_cache.clear();
                self.query.clear();
                self.sel = 0;
                self.wipe_toast = true;
                self.load()
            }
            _ => vec![],
        }
    }

    // ---------------------------------------------------------------- view
    fn title(&self, tr: &dyn Tr, it: &Clip) -> String {
        match it.kind.as_str() {
            "image" => tr.t("Изображение", "Image"),
            "files" => {
                let f: Vec<&str> = it.preview.split('\n').filter(|s| !s.is_empty()).map(base_name).collect();
                let mut s = f.iter().take(2).cloned().collect::<Vec<_>>().join(", ");
                if f.len() > 2 || it.file_count > 2 {
                    let n = if it.file_count != 0 { it.file_count } else { f.len() as i64 };
                    s.push_str(&format!(" +{}", n - 2));
                }
                s
            }
            _ => format!("{}{}", it.preview.split_whitespace().collect::<Vec<_>>().join(" "), if it.truncated { "…" } else { "" }),
        }
    }

    fn files_text(&self, tr: &dyn Tr, it: &Clip) -> String {
        let n = if it.file_count != 0 { it.file_count } else { (it.preview.split('\n').filter(|s| !s.is_empty()).count() as i64).max(1) };
        format!("{} {}", n, files_word(tr.is_ru(), n as u64))
    }

    pub fn view(&self, tr: &dyn Tr, now_ms: i64) -> View {
        let rows: Vec<Row> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| {
                let src = {
                    let s = it.source.as_str();
                    if s.to_ascii_lowercase().ends_with(".exe") { s[..s.len() - 4].to_string() } else { s.to_string() }
                };
                Row {
                    id: it.id,
                    index: i,
                    selected: i == self.sel,
                    pinned: it.pinned,
                    tile: match it.kind.as_str() {
                        "image" => Tile::Image(self.img_cache.get(&it.id).cloned().flatten()),
                        "files" => Tile::Files,
                        _ => Tile::Text,
                    },
                    title: self.title(tr, it),
                    file_count: if it.kind == "files" { Some(self.files_text(tr, it)) } else { None },
                    source: src,
                    ago: ago(tr, now_ms, it.used_at),
                    plain_button: if it.kind == "image" { None } else { Some(tr.t("Вставить как простой текст", "Paste as plain text")) },
                    ai_button: if it.kind == "text" { Some(tr.t("ИИ-действия", "AI actions")) } else { None },
                    pin_title: if it.pinned { tr.t("Открепить", "Unpin") } else { tr.t("Закрепить", "Pin") },
                    delete_title: tr.t("Удалить", "Delete"),
                }
            })
            .collect();
        let empty = if rows.is_empty() {
            Some(if !self.query.trim().is_empty() {
                tr.t("Ничего не найдено", "Nothing found")
            } else if self.paused {
                tr.t("Приватный режим: ничего не записывается", "Private mode is on: nothing is recorded")
            } else {
                tr.t("Скопированный текст, картинки и файлы появятся здесь", "Copied text, images and files will show up here")
            })
        } else {
            None
        };
        View {
            search_placeholder: tr.t("Поиск по истории", "Search history"),
            wipe_title: if self.wipe_arm.is_some() { tr.t("Нажми ещё раз: удалить всю историю", "Click again to wipe everything") } else { tr.t("Очистить историю", "Clear history") },
            wipe_armed: self.wipe_arm.is_some(),
            rows,
            empty,
            hint: tr.t("Клик — вставить · Shift+клик — только копировать", "Click to paste · Shift+click to copy only"),
            private_chip: if self.paused { Some(tr.t("Приватный режим: история не пишется", "Private mode: nothing is recorded")) } else { None },
            private_mode: self.paused,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    const NOW: i64 = 1_793_369_109_000;

    fn item(id: i64, kind: &str, preview: &str) -> Value {
        json!({ "id": id, "kind": kind, "preview": preview, "truncated": false, "fileCount": 0, "size": 10, "pinned": false, "source": "Code.exe", "createdAt": NOW - 60_000, "usedAt": NOW - 3 * 60_000 })
    }

    /// Open the tab and serve `clip_status` + `clip_list`.
    fn shown_with(list: Value) -> Clipboard {
        let mut c = Clipboard::new();
        assert_eq!(c.on_show(), vec![BackendCall::bare("clip_status")]);
        let calls = c.on_result("clip_status", &Value::Null, &json!({ "paused": false, "count": 1 }), &EnTr);
        assert_eq!(calls, vec![BackendCall::new("clip_list", json!({ "query": null, "kind": null, "limit": 100, "offset": 0 }))]);
        c.on_result("clip_list", &Value::Null, &list, &EnTr);
        c.take_fx();
        c
    }

    #[test]
    fn ago_labels() {
        let a = |ms: i64| ago(&EnTr, NOW, NOW - ms);
        assert_eq!(a(30_000), "now");
        assert_eq!(a(60_000), "1 min");
        assert_eq!(a(59 * 60_000), "59 min");
        assert_eq!(a(60 * 60_000), "1 h");
        assert_eq!(a(25 * 3_600_000), "1 d");
        assert_eq!(ago(&RuTr, NOW, NOW - 2 * 3_600_000), "2 ч");
        assert_eq!(ago(&RuTr, NOW, NOW - 5 * 60_000), "5 мин");
        assert_eq!(ago(&RuTr, NOW, NOW), "сейчас");
    }

    #[test]
    fn rows_for_text_image_and_files() {
        let mut files = item(3, "files", "C:\\docs\\a.txt\nD:/x/b.png\nE:/c.zip");
        files["fileCount"] = json!(5);
        let mut text = item(1, "text", "  hello \n  world  ");
        text["truncated"] = json!(true);
        text["pinned"] = json!(true);
        let c = shown_with(json!([text, item(2, "image", ""), files]));
        let v = c.view(&EnTr, NOW);
        assert_eq!(v.rows[0].title, "hello world…");
        assert!(v.rows[0].pinned && v.rows[0].ai_button.is_some() && v.rows[0].plain_button.is_some());
        assert_eq!(v.rows[0].pin_title, "Unpin");
        assert_eq!(v.rows[0].source, "Code");
        assert_eq!(v.rows[0].ago, "3 min");
        assert_eq!(v.rows[1].title, "Image");
        assert_eq!(v.rows[1].tile, Tile::Image(None));
        assert!(v.rows[1].plain_button.is_none() && v.rows[1].ai_button.is_none());
        assert_eq!(v.rows[2].title, "a.txt, b.png +3");
        assert_eq!(v.rows[2].file_count.as_deref(), Some("5 files"));
        assert!(v.rows[2].ai_button.is_none() && v.rows[2].plain_button.is_some());
        assert_eq!(c.view(&RuTr, NOW).rows[2].file_count.as_deref(), Some("5 файлов"));
    }

    #[test]
    fn file_count_falls_back_to_preview_lines_and_plural_forms() {
        let c = shown_with(json!([item(1, "files", "C:/one.txt"), item(2, "files", "a\nb"), item(3, "files", "")]));
        let v = c.view(&RuTr, NOW);
        assert_eq!(v.rows[0].file_count.as_deref(), Some("1 файл"));
        assert_eq!(v.rows[1].file_count.as_deref(), Some("2 файла"));
        assert_eq!(v.rows[2].file_count.as_deref(), Some("1 файл")); // never 0
        assert_eq!(v.rows[1].title, "a, b");
    }

    #[test]
    fn image_thumbnails_load_once() {
        let mut c = Clipboard::new();
        c.on_show();
        let calls = c.on_result("clip_list", &Value::Null, &json!([item(2, "image", ""), item(3, "text", "x")]), &EnTr);
        assert_eq!(calls, vec![BackendCall::new("clip_image", json!({ "id": 2 }))]);
        // another render while the fetch is in flight does not duplicate it
        assert!(c.on_result("clip_list", &Value::Null, &json!([item(2, "image", "")]), &EnTr).is_empty());
        c.on_result("clip_image", &json!({ "id": 2 }), &json!("data:image/png;base64,xx"), &EnTr);
        assert_eq!(c.view(&EnTr, NOW).rows[0].tile, Tile::Image(Some("data:image/png;base64,xx".into())));
        assert!(c.on_result("clip_list", &Value::Null, &json!([item(2, "image", "")]), &EnTr).is_empty());
        // deleting forgets the cache
        c.on_action(Action::Delete(2));
        assert_eq!(c.on_result("clip_list", &Value::Null, &json!([item(2, "image", "")]), &EnTr).len(), 1);
    }

    #[test]
    fn click_pastes_shift_copies_and_clip_click_paste_false_copies() {
        let mut c = shown_with(json!([item(1, "text", "a")]));
        assert_eq!(c.on_action(Action::Activate { id: 1, shift: false }), vec![BackendCall::new("clip_paste", json!({ "id": 1, "copyOnly": false, "plain": false }))]);
        assert_eq!(c.on_action(Action::Activate { id: 1, shift: true })[0].args["copyOnly"], true);
        assert_eq!(c.on_action(Action::PastePlain(1))[0].args, json!({ "id": 1, "copyOnly": false, "plain": true }));
        c.set_settings(&json!({ "clipClickPaste": false }));
        assert_eq!(c.on_action(Action::Activate { id: 1, shift: false })[0].args["copyOnly"], true);
        assert!(c.on_action(Action::Activate { id: 99, shift: false }).is_empty());
    }

    #[test]
    fn paste_results_toast_and_collapse() {
        let mut c = shown_with(json!([item(1, "text", "a")]));
        let args = json!({ "id": 1, "copyOnly": false, "plain": false });
        let next = c.on_result("clip_paste", &args, &json!("pasted"), &RuTr);
        assert_eq!(next[0].cmd, "clip_list"); // reload after every paste
        assert_eq!(c.take_fx(), vec![Fx::Toast { id: "clip-paste", icon: "clipboard", title: "Вставлено".into(), ttl_ms: 1800 }, Fx::Collapse { via: Some("click") }]);
        c.on_result("clip_paste", &args, &json!("copied_only"), &EnTr);
        let fx = c.take_fx();
        assert!(matches!(&fx[0], Fx::Toast { title, .. } if title == "Copied (target is elevated)"));
        assert!(fx.contains(&Fx::Collapse { via: Some("click") }));
        // copy-only keeps the island open and uses the copy icon
        c.on_result("clip_paste", &json!({ "id": 1, "copyOnly": true, "plain": false }), &json!("copied"), &EnTr);
        assert_eq!(c.take_fx(), vec![Fx::Toast { id: "clip-paste", icon: "copy", title: "Copied".into(), ttl_ms: 1800 }]);
        c.on_result("clip_paste", &args, &json!("gone"), &EnTr);
        assert_eq!(c.take_fx(), vec![Fx::Toast { id: "clip-paste", icon: "clipboard", title: "Item is gone".into(), ttl_ms: 1800 }]);
    }

    #[test]
    fn pasting_from_the_search_box_waits_for_focus_to_return() {
        let mut c = shown_with(json!([item(1, "text", "a")]));
        c.set_search_focused(true);
        assert!(c.on_action(Action::Activate { id: 1, shift: false }).is_empty());
        assert_eq!(c.take_fx(), vec![Fx::BlurSearch]);
        assert!(c.tick(119.0).is_empty());
        assert_eq!(c.tick(2.0), vec![BackendCall::new("clip_paste", json!({ "id": 1, "copyOnly": false, "plain": false }))]);
    }

    #[test]
    fn pin_toggles_and_reloads() {
        let mut p = item(1, "text", "a");
        p["pinned"] = json!(true);
        let mut c = shown_with(json!([p, item(2, "text", "b")]));
        assert_eq!(c.on_action(Action::Pin(1)), vec![BackendCall::new("clip_pin", json!({ "id": 1, "pinned": false }))]);
        assert_eq!(c.on_action(Action::Pin(2))[0].args, json!({ "id": 2, "pinned": true }));
        assert_eq!(c.on_result("clip_pin", &Value::Null, &Value::Null, &EnTr)[0].cmd, "clip_list");
    }

    #[test]
    fn delete_collapses_the_row_and_reloads() {
        let mut c = shown_with(json!([item(1, "text", "a")]));
        assert_eq!(c.on_action(Action::Delete(1)), vec![BackendCall::new("clip_delete", json!({ "id": 1 }))]);
        assert!(c.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "row:1" && anim.dur_ms == 300)));
        assert_eq!(c.on_result("clip_delete", &Value::Null, &Value::Null, &EnTr)[0].cmd, "clip_list");
    }

    #[test]
    fn search_is_debounced_and_trimmed_and_null_when_empty() {
        let mut c = shown_with(json!([]));
        c.on_action(Action::Search("fo".into()));
        c.tick(100.0);
        c.on_action(Action::Search("  foo ".into())); // typing again restarts the wait
        assert!(c.tick(149.0).is_empty());
        let calls = c.tick(2.0);
        assert_eq!(calls, vec![BackendCall::new("clip_list", json!({ "query": "foo", "kind": null, "limit": 100, "offset": 0 }))]);
        c.on_action(Action::Search("   ".into()));
        assert_eq!(c.tick(200.0)[0].args["query"], Value::Null);
        // Esc: first clears a non-empty query, then lets go
        c.on_action(Action::Search("x".into()));
        assert!(c.wants_escape());
        assert_eq!(c.on_action(Action::Key(Key::Escape))[0].args["query"], Value::Null);
        assert!(!c.wants_escape());
        assert!(c.on_action(Action::Key(Key::Escape)).is_empty());
    }

    #[test]
    fn empty_states() {
        let mut c = shown_with(json!([]));
        assert_eq!(c.view(&EnTr, NOW).empty.as_deref(), Some("Copied text, images and files will show up here"));
        c.set_paused(true);
        assert_eq!(c.view(&RuTr, NOW).empty.as_deref(), Some("Приватный режим: ничего не записывается"));
        assert_eq!(c.view(&EnTr, NOW).private_chip.as_deref(), Some("Private mode: nothing is recorded"));
        c.on_action(Action::Search("zz".into()));
        assert_eq!(c.view(&EnTr, NOW).empty.as_deref(), Some("Nothing found"));
        c.on_result("clip_status", &Value::Null, &json!({ "paused": false }), &EnTr);
        assert!(c.view(&EnTr, NOW).private_chip.is_none());
    }

    #[test]
    fn keyboard_selection_clamps_scrolls_and_enter_pastes() {
        let mut c = shown_with(json!([item(1, "text", "a"), item(2, "text", "b"), item(3, "text", "c")]));
        assert_eq!(c.selected(), 0);
        c.on_action(Action::Key(Key::Up));
        assert_eq!(c.selected(), 0);
        c.on_action(Action::Key(Key::Down));
        c.on_action(Action::Key(Key::Down));
        c.on_action(Action::Key(Key::Down));
        assert_eq!(c.selected(), 2);
        assert!(c.take_fx().contains(&Fx::ScrollTo { target: "row:3".into() }));
        let calls = c.on_action(Action::Key(Key::Enter { shift: true }));
        assert_eq!(calls[0].args, json!({ "id": 3, "copyOnly": true, "plain": false }));
        // hover selects without scrolling, only on change
        c.on_action(Action::Hover(1));
        assert_eq!(c.selected(), 1);
        assert!(c.take_fx().iter().all(|f| !matches!(f, Fx::ScrollTo { .. })));
        // a shorter list pulls the selection back
        c.on_action(Action::Hover(2));
        c.on_result("clip_list", &Value::Null, &json!([item(1, "text", "a")]), &EnTr);
        assert_eq!(c.selected(), 0);
    }

    #[test]
    fn ai_panel_owns_the_keys_while_open() {
        let mut c = shown_with(json!([item(1, "text", "a"), item(2, "text", "b")]));
        c.set_ai_open(true);
        c.on_action(Action::Key(Key::Down));
        assert_eq!(c.selected(), 0);
        assert!(c.on_action(Action::Key(Key::Enter { shift: false })).is_empty());
        c.on_action(Action::Typed('a'));
        assert!(c.take_fx().is_empty());
        c.set_ai_open(false);
        c.on_action(Action::Ai(2));
        assert_eq!(c.take_fx(), vec![Fx::OpenClipAi { id: 2 }]);
        c.on_hide();
        assert_eq!(c.take_fx(), vec![Fx::CloseClipAi]);
    }

    #[test]
    fn typing_hands_the_keyboard_to_the_search_box_once() {
        let mut c = shown_with(json!([]));
        c.on_action(Action::Typed('/'));
        assert_eq!(c.take_fx(), vec![Fx::FocusSearch]);
        c.on_action(Action::Typed('a')); // already focused: nothing
        assert!(c.take_fx().is_empty());
    }

    #[test]
    fn wipe_needs_two_clicks_then_clears_everything_and_toasts_after_the_reload() {
        let mut c = shown_with(json!([item(1, "text", "a"), item(2, "image", "")]));
        assert!(c.on_action(Action::Wipe).is_empty());
        let v = c.view(&EnTr, NOW);
        assert!(v.wipe_armed);
        assert_eq!(v.wipe_title, "Click again to wipe everything");
        c.tick(3001.0);
        assert!(!c.view(&EnTr, NOW).wipe_armed);
        c.on_action(Action::Wipe);
        c.tick(500.0);
        assert_eq!(c.on_action(Action::Wipe), vec![BackendCall::bare("clip_wipe")]);
        assert_eq!(c.take_fx().iter().filter(|f| matches!(f, Fx::Anim { anim, .. } if anim.dur_ms == 160)).count(), 2);
        c.on_action(Action::Search("q".into()));
        let next = c.on_result("clip_wipe", &Value::Null, &Value::Null, &RuTr);
        assert_eq!(next[0].args["query"], Value::Null); // search box cleared
        assert!(c.take_fx().is_empty()); // the toast waits for the reload
        c.on_result("clip_list", &Value::Null, &json!([]), &RuTr);
        assert_eq!(c.take_fx().iter().filter(|f| matches!(f, Fx::Toast { id: "clip-wipe", title, .. } if title == "Буфер очищен")).count(), 1);
    }

    #[test]
    fn new_copies_reload_after_120ms_only_while_shown_and_drop_in() {
        let mut c = shown_with(json!([item(1, "text", "a")]));
        c.on_event("clip:added", &json!({}));
        assert!(c.tick(119.0).is_empty());
        assert_eq!(c.tick(2.0)[0].cmd, "clip_list");
        c.on_result("clip_list", &Value::Null, &json!([item(2, "text", "b"), item(1, "text", "a")]), &EnTr);
        let fx = c.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "list" && anim.kind == crate::panel::a::anim::AnimKind::Flip { enter_new: true })));
        // hidden tab: nothing
        c.on_hide();
        c.on_event("clip:added", &json!({}));
        assert!(c.tick(500.0).is_empty());
    }

    #[test]
    fn first_render_of_a_visit_staggers_then_flips() {
        let mut c = Clipboard::new();
        c.on_show();
        c.on_result("clip_list", &Value::Null, &json!([item(1, "text", "a"), item(2, "text", "b")]), &EnTr);
        let fx = c.take_fx();
        let delays: Vec<u32> = fx.iter().filter_map(|f| if let Fx::Anim { anim, .. } = f { Some(anim.delay_ms) } else { None }).collect();
        assert_eq!(delays, [0, 28]);
        c.on_result("clip_list", &Value::Null, &json!([item(2, "text", "b")]), &EnTr);
        assert!(c.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "list")));
    }

    #[test]
    fn rows_to_empty_state_animates_once() {
        let mut c = shown_with(json!([item(1, "text", "a")]));
        c.on_result("clip_list", &Value::Null, &json!([]), &EnTr);
        assert!(c.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "empty")));
        c.on_result("clip_list", &Value::Null, &json!([]), &EnTr); // typing inside "nothing found": no replay
        assert!(c.take_fx().is_empty());
    }

    #[test]
    fn hotkey_opens_the_tab_and_focuses_search_after_320ms() {
        let mut c = shown_with(json!([]));
        c.on_event("clip:open", &Value::Null);
        assert_eq!(c.take_fx(), vec![Fx::Expand { tab: "clipboard" }]);
        c.tick(319.0);
        assert!(c.take_fx().is_empty());
        c.tick(2.0);
        assert_eq!(c.take_fx(), vec![Fx::FocusSearch]);
        // if the tab was left meanwhile, no focus
        c.on_event("clip:open", &Value::Null);
        c.take_fx();
        c.on_hide();
        c.take_fx();
        c.tick(400.0);
        assert!(c.take_fx().is_empty());
    }

    #[test]
    fn reduced_motion_pushes_no_cues() {
        let mut c = Clipboard::new();
        c.set_settings(&json!({ "reduceMotion": true }));
        c.on_show();
        c.on_result("clip_list", &Value::Null, &json!([item(1, "text", "a")]), &EnTr);
        c.on_action(Action::Delete(1));
        assert!(c.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
    }
}
