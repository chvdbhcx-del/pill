//! File shelf: the drop target on the folded island (a file drag near the island opens a compact "drop to keep" card and the
//! drop lands on the shelf), the shelf tab (file tiles, native drag-out, context menu), the count badge on the folded island and
//! the transient "N files" widget. Port of `src/tab-shelf.js` (backend `shelf.rs`).
//!
//! Backend: `shelf_list`, `shelf_add {paths}` (answers how many were new), `shelf_remove {id}`, `shelf_clear`, `shelf_open {id}`,
//! `shelf_reveal {id}`, `shelf_copy_path {id}`, `shelf_drag_out {ids}` (answers `copied` after a completed drop). Event `shelf:changed
//! {count, items}`, plus the Tauri window drag events (`tauri://drag-enter|over|leave|drop`, files only, physical px positions).
//! Settings: `module.shelf` (tab on/off), `live.shelf` (badge + widget), `lastTab` (a drag opens Home instead when it was the last
//! tab, so its launcher row can take the drop), `pinned`.
//!
//! The folded-island drop card is a small state machine: *hover* -> (700 ms dwell) opens the whole island, or *drop* -> files fly
//! into the icon, the card says what happened (1.4 s / 1.7 s) and folds back.

use super::anim::{Anim, AnimKind, Ease};
use super::launcher::Pt;
use super::util::{files_word, Wall};
use super::{DropCard, BackendCall, Cfg, Fx, FxQueue, LiveBody, LiveSpec, Tr};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// A drag resting on the card opens the full island after this long.
pub const DWELL_MS: f64 = 700.0;
pub const PEEK_SAFETY_MS: f64 = 30_000.0;
pub const LANDED_SAFETY_MS: f64 = 3000.0;
pub const OK_HOLD_MS: f64 = 1400.0;
pub const DUP_HOLD_MS: f64 = 1700.0;
/// Pointer travel before a press on a tile becomes a native drag-out.
pub const DRAG_THRESHOLD: f32 = 6.0;
/// The island collapses this long after a completed drag-out.
pub const COLLAPSE_AFTER_DROP_MS: f64 = 150.0;
/// How long a freshly landed chip keeps its highlight.
pub const FRESH_MS: f64 = 1600.0;
/// The last drop point is used as the fly-in origin for this long.
pub const DROP_ORIGIN_MS: f64 = 6000.0;
pub const REMOVE_EXIT_MS: f64 = 160.0;
/// A chip hidden for its exit is shown again after this long if the backend call failed.
pub const RESTORE_MS: f64 = 900.0;

const EXT_COLORS: [(&str, &str); 30] = [
    ("pdf", "#e8494f"), ("png", "#3b9aff"), ("jpg", "#3b9aff"), ("jpeg", "#3b9aff"), ("gif", "#3b9aff"), ("webp", "#3b9aff"), ("bmp", "#3b9aff"),
    ("svg", "#3b9aff"), ("txt", "#8b8d98"), ("md", "#8b8d98"), ("log", "#8b8d98"), ("doc", "#3b6fe0"), ("docx", "#3b6fe0"), ("xls", "#2ea664"),
    ("xlsx", "#2ea664"), ("csv", "#2ea664"), ("ppt", "#e8793a"), ("pptx", "#e8793a"), ("zip", "#d9a21b"), ("rar", "#d9a21b"), ("7z", "#d9a21b"),
    ("mp3", "#a65be8"), ("wav", "#a65be8"), ("flac", "#a65be8"), ("mp4", "#e85b9a"), ("mkv", "#e85b9a"), ("mov", "#e85b9a"), ("avi", "#e85b9a"),
    ("exe", "#6b7280"), ("msi", "#6b7280"),
];
const DEFAULT_COLOR: &str = "#7c8090";

/// Tile colour of a file extension (own palette).
pub fn ext_color(ext: &str) -> &'static str {
    EXT_COLORS.iter().find(|(e, _)| *e == ext).map_or(DEFAULT_COLOR, |(_, c)| c)
}

/// `extOf`: up to 5 characters after the last dot of the file name, lower case.
pub fn ext_of(name: &str) -> String {
    let base = name.rsplit(['\\', '/']).next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if !base[i + 1..].is_empty() && base[i + 1..].chars().count() <= 5 => base[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

fn is_img_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
}

fn to_fixed(v: f64, digits: i32) -> String {
    let k = 10f64.powi(digits);
    format!("{:.*}", digits as usize, (v * k).round() / k)
}

/// `size`: `512 B`, `1.5 KB`, `34 KB`, `2.0 MB`.
pub fn size_text(b: u64) -> String {
    if b < 1024 {
        format!("{b} B")
    } else if b < 1_048_576 {
        format!("{} KB", to_fixed(b as f64 / 1024.0, if b < 10_240 { 1 } else { 0 }))
    } else {
        format!("{} MB", to_fixed(b as f64 / 1_048_576.0, 1))
    }
}

fn tag(ext: &str) -> String {
    let e = if ext.is_empty() { "file" } else { ext };
    e.to_uppercase().chars().take(4).collect()
}

/// `shelf.rs::Item`
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: i64,
    pub path: String,
    pub name: String,
    pub added_at: i64,
    pub missing: bool,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub icon: Option<String>,
}

impl Item {
    pub fn from_json(v: &Value) -> Item {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        Item {
            id: v.get("id").and_then(Value::as_i64).unwrap_or(0),
            path: s("path"),
            name: s("name"),
            added_at: v.get("added_at").and_then(Value::as_i64).unwrap_or(0),
            missing: v.get("missing").and_then(Value::as_bool).unwrap_or(false),
            is_dir: v.get("is_dir").and_then(Value::as_bool).unwrap_or(false),
            size: v.get("size").and_then(Value::as_u64),
            icon: v.get("icon").and_then(Value::as_str).map(str::to_string),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Open,
    Reveal,
    CopyPath,
    Remove,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Mouse down on a chip (`button0` = primary button). Ctrl toggles the selection, otherwise it may become a drag-out.
    ChipPress { id: i64, x: f32, y: f32, ctrl: bool, button0: bool },
    /// Mouse down on "Drag all".
    DragAllPress { x: f32, y: f32 },
    PointerMove { x: f32, y: f32 },
    PointerRelease,
    DoubleClick(i64),
    /// The x on a chip.
    RemoveChip(i64),
    /// "Clear".
    Clear,
    ContextMenu { id: i64, x: f32, y: f32 },
    MenuPick(usize),
    /// Mouse down outside the menu.
    MenuClose,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChipTile {
    Folder,
    /// Shell icon / thumbnail as a data URL.
    Image(String),
    Doc { label: String, color: &'static str },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub id: i64,
    pub tile: ChipTile,
    pub name: String,
    pub sub: String,
    pub tooltip: String,
    pub missing: bool,
    pub selected: bool,
    /// Being dragged out: dimmed, dashed ghost until the drag ends.
    pub ghost: bool,
    /// Landed a moment ago: highlighted for 1.6 s.
    pub fresh: bool,
    /// Faded out for its removal; shown again if the backend call fails.
    pub hidden: bool,
    pub remove_title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    pub label: String,
    pub action: MenuAction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuView {
    pub x: f32,
    pub y: f32,
    pub entries: Vec<MenuEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BadgeView {
    pub on: bool,
    pub text: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmptyView {
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// A file drag hovers the tab.
    pub over: bool,
    pub empty: bool,
    /// The zone's "landed" ring is up (800 ms).
    pub landed: bool,
    pub head_visible: bool,
    pub drag_all_label: String,
    pub drag_all_tooltip: String,
    pub clear_label: String,
    pub chips: Vec<Chip>,
    pub empty_state: EmptyView,
    pub drop_title: String,
    pub menu: Option<MenuView>,
    pub badge: BadgeView,
    /// The folded-island drop card, while one is up (also pushed as `Fx::Live`).
    pub peek: Option<DropCard>,
    /// Document tiles flying into the drop card's icon after a drop.
    pub peek_fly: Vec<(String, &'static str)>,
}

#[derive(Clone, Debug)]
struct Peek {
    dwell_at: Option<f64>,
    safety_at: f64,
    landed: bool,
    card: DropCard,
    fly: Vec<(String, &'static str)>,
}

#[derive(Clone, Debug)]
struct Press {
    x: f32,
    y: f32,
    /// None = "Drag all"
    id: Option<i64>,
}

pub struct Shelf {
    cfg: Cfg,
    mono: f64,
    items: Vec<Item>,
    seen: i64,
    sel: Vec<i64>,
    drag_over: bool,
    out_drag: bool,
    peek: Option<Peek>,
    last_drop: Option<(Pt, f64)>,
    known: HashSet<i64>,
    enter: bool,
    tab_on: bool,
    expanded: bool,
    press: Option<Press>,
    ghosts: Vec<i64>,
    was_pinned: bool,
    menu: Option<(f32, f32, i64)>,
    leaving: HashMap<i64, f64>,
    removal: Option<(f64, &'static str, Value)>,
    collapse_at: Option<f64>,
    fresh_until: HashMap<i64, f64>,
    landed_until: f64,
    dpr: f32,
    viewport: (f32, f32),
    last_label: String,
    fx: FxQueue,
}

impl Default for Shelf {
    fn default() -> Self {
        Shelf::new()
    }
}

impl Shelf {
    pub fn new() -> Shelf {
        Shelf {
            cfg: Cfg::new(),
            mono: 0.0,
            items: vec![],
            seen: -1,
            sel: vec![],
            drag_over: false,
            out_drag: false,
            peek: None,
            last_drop: None,
            known: HashSet::new(),
            enter: false,
            tab_on: false,
            expanded: false,
            press: None,
            ghosts: vec![],
            was_pinned: false,
            menu: None,
            leaving: HashMap::new(),
            removal: None,
            collapse_at: None,
            fresh_until: HashMap::new(),
            landed_until: 0.0,
            dpr: 1.0,
            viewport: (1000.0, 700.0),
            last_label: String::new(),
            fx: FxQueue::default(),
        }
    }

    pub fn init_calls(&self) -> Vec<BackendCall> {
        vec![BackendCall::bare("shelf_list")]
    }
    pub fn set_settings(&mut self, v: &Value) {
        self.cfg.apply(v);
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.cfg.os_reduce = on;
    }
    /// Device pixel ratio (drop positions arrive in physical px) and the window size in CSS px (menu placement).
    pub fn set_metrics(&mut self, dpr: f32, viewport: (f32, f32)) {
        self.dpr = if dpr > 0.0 { dpr } else { 1.0 };
        self.viewport = viewport;
    }
    /// `Pill.state === 'expanded'`
    pub fn set_expanded(&mut self, v: bool) {
        self.expanded = v;
    }
    pub fn take_fx(&mut self) -> Vec<Fx> {
        self.fx.take()
    }
    pub fn items(&self) -> &[Item] {
        &self.items
    }
    pub fn selection(&self) -> &[i64] {
        &self.sel
    }
    pub fn peeking(&self) -> bool {
        self.peek.is_some()
    }
    /// The island must not collapse on Esc while the context menu is open.
    pub fn wants_escape(&self) -> bool {
        self.menu.is_some()
    }
    fn shelf_on(&self) -> bool {
        self.cfg.module_on("module.shelf")
    }
    fn live_on(&self) -> bool {
        self.cfg.live_on("shelf")
    }
    fn reduce(&self) -> bool {
        self.cfg.reduce_motion()
    }
    fn on_home(&self) -> bool {
        self.cfg.str("lastTab") == "home"
    }

    /// Fly-in vector of a chip (`flyIn`): from the last drop point (if recent) or from above the chip, in the panel's CSS px.
    pub fn fly_vector(&self, chip_center: Pt, zoom: f32) -> (f32, f32) {
        let d = match self.last_drop {
            Some((p, t)) if self.mono - t < DROP_ORIGIN_MS => p,
            _ => Pt { x: chip_center.x, y: -30.0 },
        };
        let z = if zoom > 0.0 { zoom } else { 1.0 };
        ((d.x - chip_center.x) / z, (d.y - chip_center.y) / z)
    }

    // ---------------------------------------------------------------- items
    fn set_items(&mut self, v: &Value) {
        let was = self.seen;
        self.items = v.as_array().map(|a| a.iter().map(Item::from_json).collect()).unwrap_or_default();
        let ids: HashSet<i64> = self.items.iter().map(|i| i.id).collect();
        self.sel.retain(|id| ids.contains(id));
        self.leaving.clear();
        self.removal = None;
        let n = self.items.len() as i64;
        self.seen = n;
        if was >= 0 && n > was {
            self.fx.push(Fx::React { key: "shelf", kind: "file", color: None, ttl_ms: Some(3000) }); // item added (not on first load, not on removal)
        }
        self.fx.push(Fx::Badge { tab: "shelf", text: if n == 0 { String::new() } else { n.to_string() } });
        let on = n > 0 && self.shelf_on() && self.live_on();
        if was >= 0 && n != was && on {
            self.fx.anim(self.reduce(), "badge", Anim::pop(0.35, 500));
        }
        // Transient: a permanent widget would hide the clock for good (the badge is the permanent part). 5 s when the count changes.
        let widget = n > 0 && was >= 0 && n != was && self.shelf_on() && self.live_on() && !self.peek.as_ref().map_or(false, |p| p.landed);
        self.fx.push(Fx::Live {
            key: "shelf",
            spec: if widget { Some(LiveSpec { priority: 30, ttl_ms: Some(5000), tall: false, accent: String::new(), body: LiveBody::ShelfCount { count: self.items.len() } }) } else { None },
        });
        self.render(was >= 0 && n > was);
    }

    fn label(&self, tr: &dyn Tr) -> String {
        format!("{} ({})", tr.t("Перетащить все", "Drag all"), self.items.len())
    }

    fn render(&mut self, added: bool) {
        let reduce = self.reduce();
        // the label "bump" needs the language: compare by count instead
        let key = self.items.len().to_string();
        if self.last_label != key {
            self.last_label = key;
            if added {
                self.fx.anim(reduce, "dragall", Anim::pop(1.16, 500));
            }
        }
        if self.enter {
            for (i, it) in self.items.iter().enumerate() {
                self.fx.anim(reduce, format!("chip:{}", it.id), Anim::row_stagger(i));
            }
        } else {
            self.fx.anim(reduce, "list", Anim::flip(false)); // chips that stay slide into the freed spot, no stagger replay
        }
        // a chip that was not on the grid before flies in from the drop point and the zone gives a "landed" pulse
        let fresh: Vec<i64> = if self.enter || self.known.is_empty() || !self.tab_on || !self.expanded {
            vec![]
        } else {
            self.items.iter().filter(|i| !self.known.contains(&i.id)).map(|i| i.id).collect()
        };
        self.known = self.items.iter().map(|i| i.id).collect();
        self.enter = false;
        for (k, id) in fresh.iter().enumerate() {
            self.fresh_until.insert(*id, self.mono + FRESH_MS);
            self.fx.anim(reduce, format!("chip:{id}"), Anim::fly_in(0.0, 0.0, k)); // the renderer fills in `fly_vector`
        }
        if !fresh.is_empty() && !reduce {
            self.landed_until = self.mono + 800.0;
            self.fx.anim(false, "zone", Anim::ring());
        }
    }

    // ---------------------------------------------------------------- lifecycle and events
    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.tab_on = true;
        self.enter = true;
        vec![BackendCall::bare("shelf_list")]
    }

    pub fn on_hide(&mut self) {
        self.tab_on = false;
        self.sel.clear();
        self.menu = None;
    }

    fn pin(&mut self, on: bool) {
        self.fx.push(Fx::SetPinned(on));
    }

    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        match name {
            "shelf:changed" => {
                self.set_items(payload.get("items").unwrap_or(&Value::Null));
                vec![]
            }
            "settings:changed" => {
                self.cfg.apply(payload);
                if !self.shelf_on() {
                    self.end_peek();
                }
                self.badge_fx();
                vec![]
            }
            "pill:hide" => {
                self.end_peek();
                vec![]
            }
            "tauri://drag-enter" => {
                if self.out_drag {
                    return vec![];
                }
                if !self.shelf_on() {
                    if self.on_home() {
                        self.fx.push(Fx::Expand { tab: "home" });
                    }
                    return vec![];
                }
                if !self.expanded {
                    self.start_peek();
                    return vec![];
                }
                if self.on_home() {
                    self.fx.push(Fx::Expand { tab: "home" });
                    return vec![];
                }
                self.drag_over = true;
                self.fx.push(Fx::Expand { tab: "shelf" });
                vec![]
            }
            "tauri://drag-leave" => {
                self.drag_over = false;
                if self.peek.as_ref().map_or(false, |p| !p.landed) {
                    self.end_peek();
                }
                vec![]
            }
            "tauri://drag-drop" => self.on_drag_drop(payload, false),
            _ => vec![],
        }
    }

    fn badge_fx(&mut self) {
        let n = self.items.len();
        self.fx.push(Fx::Badge { tab: "shelf", text: if n == 0 { String::new() } else { n.to_string() } });
    }

    /// A file drop. `on_row` = the drop point is over Home's launcher row (`Launcher::hit`): then the launcher takes it, not the shelf.
    pub fn on_drag_drop(&mut self, payload: &Value, on_row: bool) -> Vec<BackendCall> {
        self.drag_over = false;
        let paths: Vec<Value> = payload.get("paths").and_then(Value::as_array).cloned().unwrap_or_default();
        if !self.out_drag && self.shelf_on() && !on_row && !paths.is_empty() {
            let pos = payload.get("position").and_then(Pt::from_json).map(|p| Pt { x: p.x / self.dpr, y: p.y / self.dpr });
            self.last_drop = pos.map(|p| (p, self.mono));
            let card = self.peek.is_some();
            if card {
                self.land(&paths);
            }
            if !card && self.on_home() {
                self.fx.push(Fx::Expand { tab: "shelf" }); // dropped beside the launcher row: show where the files went
            }
            return vec![BackendCall::new("shelf_add", json!({ "paths": paths }))];
        }
        self.end_peek();
        vec![]
    }

    // ---------------------------------------------------------------- the compact drop card
    fn start_peek(&mut self) {
        if self.peek.is_some() {
            return;
        }
        self.peek = Some(Peek { dwell_at: Some(self.mono + DWELL_MS), safety_at: self.mono + PEEK_SAFETY_MS, landed: false, card: DropCard { title: String::new(), sub: String::new(), phase: "idle" }, fly: vec![] });
        self.push_card_live(30_000);
        self.fx.push(Fx::DragPull(true));
        self.fx.push(Fx::EdgeColor(Some("var(--accent)".into())));
    }

    fn push_card_live(&mut self, ttl: u32) {
        if let Some(p) = &self.peek {
            self.fx.push(Fx::Live {
                key: "shelf-drop",
                spec: Some(LiveSpec { priority: 95, ttl_ms: Some(ttl), tall: true, accent: String::new(), body: LiveBody::ShelfDrop(p.card.clone()) }),
            });
        }
    }

    fn end_peek(&mut self) {
        if self.peek.take().is_none() {
            return;
        }
        self.fx.push(Fx::Live { key: "shelf-drop", spec: None });
        self.fx.push(Fx::DragPull(false));
        self.fx.push(Fx::EdgeColor(None));
    }

    /// The drag rests on the card: open the whole island under it.
    fn open_full(&mut self) {
        if !self.peek.as_ref().map_or(false, |p| !p.landed) {
            return;
        }
        self.end_peek();
        if self.on_home() {
            self.fx.push(Fx::Expand { tab: "home" });
            return;
        }
        self.drag_over = true;
        self.fx.push(Fx::Expand { tab: "shelf" });
    }

    /// Drop on the card: the file tiles fly from the drop point into the icon, the icon pops.
    fn land(&mut self, paths: &[Value]) {
        let reduce = self.reduce();
        let Some(p) = self.peek.as_mut() else { return };
        p.dwell_at = None;
        p.landed = true;
        p.safety_at = self.mono + LANDED_SAFETY_MS; // the answer from shelf_add normally ends it sooner
        p.card.phase = "go";
        p.fly = paths
            .iter()
            .take(3)
            .map(|v| {
                let name = v.as_str().unwrap_or("").rsplit(['\\', '/']).next().unwrap_or("").to_string();
                let e = ext_of(&name);
                (tag(&e), ext_color(&e))
            })
            .collect();
        let n = p.fly.len();
        if !reduce {
            for k in 0..n {
                self.fx.push(Fx::Anim {
                    target: format!("drop-fly:{k}"),
                    anim: Anim { kind: AnimKind::FlyIn { from_dx: 0.0, from_dy: 0.0, rot_deg: (k as f32 - 1.0) * 12.0 }, dur_ms: 560, delay_ms: k as u32 * 70, ease: Ease::SpringSettle },
                });
            }
        }
    }

    /// `shelf_add` answered: `added` of `total` paths were new.
    fn landed(&mut self, added: u64, total: usize, tr: &dyn Tr) {
        let reduce = self.reduce();
        let ru = tr.is_ru();
        let Some(p) = self.peek.as_mut() else { return };
        if !p.landed {
            return;
        }
        let dup = added == 0;
        p.card.phase = if dup { "dup" } else { "ok" };
        p.card.title = if dup { tr.t("Уже лежит на полке", "Already on the shelf") } else { tr.t("Лежит на полке", "On the shelf") };
        p.card.sub = if dup {
            tr.t("Ничего нового не добавлено", "Nothing new was added")
        } else {
            format!("+{} {}{}", added, files_word(ru, added), if (added as usize) < total { tr.t(", остальные уже были", ", the rest were already there") } else { String::new() })
        };
        p.safety_at = self.mono + if added > 0 { OK_HOLD_MS } else { DUP_HOLD_MS };
        self.push_card_live(30_000);
        self.fx.anim(reduce, "drop-ico", if dup { Anim::shake() } else { Anim::pop(0.82, 600) });
    }

    pub fn on_result(&mut self, cmd: &str, args: &Value, res: &Value, tr: &dyn Tr) -> Vec<BackendCall> {
        match cmd {
            "shelf_list" => {
                self.set_items(res);
                vec![]
            }
            "shelf_add" => {
                let total = args.get("paths").and_then(Value::as_array).map_or(0, |a| a.len());
                self.landed(res.as_u64().unwrap_or(0), total, tr);
                vec![]
            }
            "shelf_drag_out" => {
                self.out_drag = false;
                self.ghosts.clear();
                if res.as_str() == Some("copied") {
                    self.collapse_at = Some(self.mono + COLLAPSE_AFTER_DROP_MS);
                }
                if !self.was_pinned {
                    self.pin(false);
                }
                vec![]
            }
            "shelf_remove" | "shelf_clear" => {
                // shelf:changed rebuilds the grid; a failed call must never leave an invisible chip
                let at = self.mono + RESTORE_MS;
                for v in self.leaving.values_mut() {
                    *v = at;
                }
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        self.mono += dt_ms;
        let mut calls = vec![];
        let mut dwell_fired = false;
        let mut safety_fired = false;
        if let Some(p) = &mut self.peek {
            if p.dwell_at.map_or(false, |at| at <= self.mono) {
                p.dwell_at = None;
                dwell_fired = true;
            }
            if p.safety_at <= self.mono {
                safety_fired = true;
            }
        }
        if safety_fired {
            self.end_peek();
        } else if dwell_fired {
            self.open_full();
        }
        if let Some((at, cmd, args)) = self.removal.clone() {
            if at <= self.mono {
                self.removal = None;
                calls.push(BackendCall { cmd, args });
                let restore = self.mono + RESTORE_MS;
                for v in self.leaving.values_mut() {
                    *v = restore;
                }
            }
        }
        let mono = self.mono;
        if self.removal.is_none() {
            self.leaving.retain(|_, until| *until > mono);
        }
        self.fresh_until.retain(|_, until| *until > mono);
        if self.collapse_at.map_or(false, |at| at <= self.mono) {
            self.collapse_at = None;
            self.fx.push(Fx::Collapse { via: None });
        }
        calls
    }

    // ---------------------------------------------------------------- actions
    fn remove_chips(&mut self, ids: Vec<i64>, cmd: &'static str, args: Value) -> Vec<BackendCall> {
        let reduce = self.reduce();
        let mut longest = 0.0f64;
        for (i, id) in ids.iter().enumerate() {
            if !self.items.iter().any(|x| x.id == *id) {
                continue;
            }
            let delay = i.min(8) as u32 * 20;
            if !reduce {
                self.fx.anim(false, format!("chip:{id}"), Anim::exit(0.0, 0.8).delayed(delay));
            }
            self.leaving.insert(*id, f64::INFINITY);
            longest = longest.max(delay as f64 + REMOVE_EXIT_MS);
        }
        if reduce {
            self.leaving.clear();
            return vec![BackendCall { cmd, args }];
        }
        self.removal = Some((self.mono + longest, cmd, args));
        vec![]
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::ChipPress { id, x, y, ctrl, button0 } => {
                let Some(it) = self.items.iter().find(|i| i.id == id) else { return vec![] };
                if !button0 || it.missing {
                    return vec![];
                }
                if ctrl {
                    if let Some(p) = self.sel.iter().position(|s| *s == id) {
                        self.sel.remove(p);
                    } else {
                        self.sel.push(id);
                    }
                    return vec![];
                }
                self.press = Some(Press { x, y, id: Some(id) });
                vec![]
            }
            Action::DragAllPress { x, y } => {
                self.press = Some(Press { x, y, id: None });
                vec![]
            }
            Action::PointerMove { x, y } => {
                let Some(p) = self.press.clone() else { return vec![] };
                if ((x - p.x).powi(2) + (y - p.y).powi(2)).sqrt() < DRAG_THRESHOLD {
                    return vec![];
                }
                self.press = None;
                let ids: Vec<i64> = match p.id {
                    None => self.items.iter().filter(|i| !i.missing).map(|i| i.id).collect(),
                    Some(id) if self.sel.contains(&id) => self.sel.clone(),
                    Some(id) => vec![id],
                };
                if ids.is_empty() {
                    return vec![];
                }
                self.out_drag = true;
                self.ghosts = ids.iter().copied().filter(|id| self.items.iter().any(|i| i.id == *id)).collect();
                // the cursor leaves the island during a native drag: keep it open (pin) until the drop; a completed drop closes it
                self.was_pinned = self.cfg.truthy("pinned");
                if !self.was_pinned {
                    self.pin(true);
                }
                vec![BackendCall::new("shelf_drag_out", json!({ "ids": ids }))]
            }
            Action::PointerRelease => {
                self.press = None;
                vec![]
            }
            Action::DoubleClick(id) => vec![BackendCall::new("shelf_open", json!({ "id": id }))],
            Action::RemoveChip(id) => self.remove_chips(vec![id], "shelf_remove", json!({ "id": id })),
            Action::Clear => {
                let ids = self.items.iter().map(|i| i.id).collect();
                self.remove_chips(ids, "shelf_clear", Value::Null)
            }
            Action::ContextMenu { id, x, y } => {
                if !self.items.iter().any(|i| i.id == id) {
                    return vec![];
                }
                self.menu = Some((x.min(self.viewport.0 - 150.0), y.min(self.viewport.1 - 130.0), id));
                self.fx.anim(self.reduce(), "menu", Anim::popover_in());
                vec![]
            }
            Action::MenuPick(i) => {
                let Some((_, _, id)) = self.menu else { return vec![] };
                let Some(it) = self.items.iter().find(|x| x.id == id).cloned() else { return vec![] };
                let entry = self.menu_entries(&it).get(i).map(|e| e.1);
                self.menu = None;
                self.fx.anim(self.reduce(), "menu", Anim::exit(-4.0, 0.96));
                match entry {
                    Some(MenuAction::Open) => vec![BackendCall::new("shelf_open", json!({ "id": id }))],
                    Some(MenuAction::Reveal) => vec![BackendCall::new("shelf_reveal", json!({ "id": id }))],
                    Some(MenuAction::CopyPath) => vec![BackendCall::new("shelf_copy_path", json!({ "id": id }))],
                    Some(MenuAction::Remove) => self.remove_chips(vec![id], "shelf_remove", json!({ "id": id })),
                    None => vec![],
                }
            }
            Action::MenuClose => {
                if self.menu.take().is_some() {
                    self.fx.anim(self.reduce(), "menu", Anim::exit(-4.0, 0.96));
                }
                vec![]
            }
        }
    }

    fn menu_entries(&self, it: &Item) -> Vec<(&'static str, MenuAction)> {
        let mut v = vec![];
        if !it.missing {
            v.push(("open", MenuAction::Open));
            v.push(("reveal", MenuAction::Reveal));
        }
        v.push(("copy", MenuAction::CopyPath));
        v.push(("remove", MenuAction::Remove));
        v
    }

    // ---------------------------------------------------------------- view
    pub fn view(&self, tr: &dyn Tr, _now: Wall) -> View {
        let chips = self
            .items
            .iter()
            .map(|it| {
                let ext = ext_of(&it.name);
                let tile = if it.is_dir {
                    ChipTile::Folder
                } else if let (Some(icon), true) = (it.icon.as_ref(), is_img_ext(&ext) || !EXT_COLORS.iter().any(|(e, _)| *e == ext)) {
                    ChipTile::Image(icon.clone())
                } else {
                    ChipTile::Doc { label: tag(&ext), color: ext_color(&ext) }
                };
                let sub = if it.missing {
                    tr.t("не найден", "missing")
                } else if let Some(sz) = it.size {
                    format!("{} · {}", tag(&ext), size_text(sz))
                } else if it.is_dir {
                    tr.t("Папка", "Folder")
                } else {
                    tag(&ext)
                };
                Chip {
                    id: it.id,
                    tile,
                    name: it.name.clone(),
                    sub,
                    tooltip: if it.missing { format!("{}\n{}", it.path, tr.t("(файл не найден)", "(file not found)")) } else { it.path.clone() },
                    missing: it.missing,
                    selected: self.sel.contains(&it.id),
                    ghost: self.ghosts.contains(&it.id),
                    fresh: self.fresh_until.get(&it.id).map_or(false, |u| *u > self.mono),
                    hidden: self.leaving.contains_key(&it.id),
                    remove_title: tr.t("Убрать с полки", "Remove from shelf"),
                }
            })
            .collect();
        let n = self.items.len();
        let on = n > 0 && self.shelf_on() && self.live_on();
        View {
            over: self.drag_over,
            empty: self.items.is_empty(),
            landed: self.landed_until > self.mono,
            head_visible: !self.items.is_empty(),
            drag_all_label: self.label(tr),
            drag_all_tooltip: tr.t("Зажми и перетащи все файлы в нужное окно", "Press and drag every file into another window"),
            clear_label: tr.t("Очистить", "Clear"),
            chips,
            empty_state: EmptyView {
                title: tr.t("Полка пуста", "The shelf is empty"),
                text: tr.t(
                    "Тащи файлы к острову: он раскроется, отпусти — они полежат здесь, пока не понадобятся. Потом вытащи их в нужное окно",
                    "Drag files toward the pill: it opens, let go and they wait here until you need them. Then drag them into any window",
                ),
            },
            drop_title: tr.t("Отпусти файлы здесь", "Drop files here"),
            menu: self.menu.and_then(|(x, y, id)| {
                self.items.iter().find(|i| i.id == id).map(|it| MenuView {
                    x,
                    y,
                    entries: self
                        .menu_entries(it)
                        .into_iter()
                        .map(|(k, a)| MenuEntry {
                            label: match k {
                                "open" => tr.t("Открыть", "Open"),
                                "reveal" => tr.t("Показать в папке", "Show in folder"),
                                "copy" => tr.t("Копировать путь", "Copy path"),
                                _ => tr.t("Убрать", "Remove"),
                            },
                            action: a,
                        })
                        .collect(),
                })
            }),
            badge: BadgeView { on, text: if n > 99 { "99+".into() } else { n.to_string() }, title: format!("{}{}", tr.t("На полке: ", "On the shelf: "), n) },
            peek: self.peek.as_ref().map(|p| {
                let mut c = p.card.clone();
                if c.phase == "idle" {
                    c.title = tr.t("Отпустите, чтобы положить на полку", "Drop to keep on the shelf");
                    c.sub = if n > 0 { format!("{}{}", tr.t("Уже на полке: ", "On the shelf now: "), n) } else { tr.t("Файлы останутся на своих местах", "Files stay where they are") };
                }
                c
            }),
            peek_fly: self.peek.as_ref().map(|p| p.fly.clone()).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    const W: Wall = Wall { ms: 0, tz_min: 0 };

    fn item(id: i64, name: &str) -> Value {
        json!({ "id": id, "path": format!("C:/x/{name}"), "name": name, "added_at": 1, "missing": false, "is_dir": false, "size": 2048, "icon": null })
    }

    fn items(list: &[Value]) -> Value {
        json!({ "count": list.len(), "items": list })
    }

    fn open_shelf(list: &[Value]) -> Shelf {
        let mut s = Shelf::new();
        s.set_expanded(true);
        s.on_show();
        s.on_result("shelf_list", &Value::Null, &Value::Array(list.to_vec()), &EnTr);
        s.take_fx();
        s
    }

    #[test]
    fn extension_and_size_helpers() {
        assert_eq!(ext_of("Report.PDF"), "pdf");
        assert_eq!(ext_of("archive.tar.gz"), "gz");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(ext_of("trailing."), "");
        assert_eq!(ext_of("a.toolongext"), "");
        assert_eq!(ext_of("C:\\dir.d\\file"), "");
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(1536), "1.5 KB");
        assert_eq!(size_text(10_240), "10 KB");
        assert_eq!(size_text(34_000), "33 KB");
        assert_eq!(size_text(2_097_152), "2.0 MB");
        assert_eq!(ext_color("pdf"), "#e8494f");
        assert_eq!(ext_color("weird"), "#7c8090");
    }

    #[test]
    fn tiles_and_sub_lines() {
        let mut img = item(2, "pic.png");
        img["icon"] = json!("data:thumb");
        let mut unknown = item(3, "data.xyz");
        unknown["icon"] = json!("data:shellicon");
        let mut doc_with_icon = item(4, "doc.pdf");
        doc_with_icon["icon"] = json!("data:pdficon");
        let mut folder = item(5, "Stuff");
        folder["is_dir"] = json!(true);
        folder["size"] = Value::Null;
        let mut gone = item(6, "gone.txt");
        gone["missing"] = json!(true);
        let mut nosize = item(7, "x.mp3");
        nosize["size"] = Value::Null;
        let s = open_shelf(&[item(1, "a.zip"), img, unknown, doc_with_icon, folder, gone, nosize]);
        let v = s.view(&EnTr, W);
        assert_eq!(v.chips[0].tile, ChipTile::Doc { label: "ZIP".into(), color: "#d9a21b" });
        assert_eq!(v.chips[0].sub, "ZIP · 2.0 KB");
        assert_eq!(v.chips[1].tile, ChipTile::Image("data:thumb".into()));
        assert_eq!(v.chips[2].tile, ChipTile::Image("data:shellicon".into())); // unknown type: the shell icon
        assert_eq!(v.chips[3].tile, ChipTile::Doc { label: "PDF".into(), color: "#e8494f" }); // known type keeps the coloured tile
        assert_eq!(v.chips[4].tile, ChipTile::Folder);
        assert_eq!(v.chips[4].sub, "Folder");
        assert_eq!(s.view(&RuTr, W).chips[4].sub, "Папка");
        assert_eq!(v.chips[5].sub, "missing");
        assert_eq!(v.chips[5].tooltip, "C:/x/gone.txt\n(file not found)");
        assert_eq!(v.chips[6].sub, "MP3");
        assert_eq!(v.drag_all_label, "Drag all (7)");
        assert!(v.head_visible && !v.empty);
    }

    #[test]
    fn changed_event_drives_badge_reaction_and_transient_widget() {
        let mut s = Shelf::new();
        s.on_event("shelf:changed", &items(&[item(1, "a.txt")]));
        let fx = s.take_fx();
        // first load: badge only, no reaction, no widget
        assert!(fx.contains(&Fx::Badge { tab: "shelf", text: "1".into() }));
        assert!(!fx.iter().any(|f| matches!(f, Fx::React { .. })));
        assert!(fx.contains(&Fx::Live { key: "shelf", spec: None }));
        s.on_event("shelf:changed", &items(&[item(1, "a.txt"), item(2, "b.txt")]));
        let fx = s.take_fx();
        assert!(fx.contains(&Fx::React { key: "shelf", kind: "file", color: None, ttl_ms: Some(3000) }));
        let widget = fx.iter().find_map(|f| if let Fx::Live { key: "shelf", spec: Some(w) } = f { Some(w.clone()) } else { None }).unwrap();
        assert_eq!((widget.priority, widget.ttl_ms, widget.body), (30, Some(5000), LiveBody::ShelfCount { count: 2 }));
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "badge" && anim.dur_ms == 500)));
        // a removal: no reaction, but the widget still shows the new count for 5 s
        s.on_event("shelf:changed", &items(&[item(1, "a.txt")]));
        let fx = s.take_fx();
        assert!(!fx.iter().any(|f| matches!(f, Fx::React { .. })));
        assert!(fx.iter().any(|f| matches!(f, Fx::Live { spec: Some(_), .. })));
        // emptied: badge hides
        s.on_event("shelf:changed", &items(&[]));
        assert!(s.take_fx().contains(&Fx::Badge { tab: "shelf", text: String::new() }));
    }

    #[test]
    fn module_or_live_switch_hides_badge_and_widget() {
        let mut s = Shelf::new();
        s.set_settings(&json!({ "live.shelf": false }));
        s.on_event("shelf:changed", &items(&[item(1, "a")]));
        s.take_fx();
        s.on_event("shelf:changed", &items(&[item(1, "a"), item(2, "b")]));
        let fx = s.take_fx();
        assert!(fx.contains(&Fx::Live { key: "shelf", spec: None }));
        assert!(!s.view(&EnTr, W).badge.on);
        s.set_settings(&json!({}));
        assert!(s.view(&EnTr, W).badge.on);
        assert_eq!(s.view(&EnTr, W).badge.title, "On the shelf: 2");
    }

    #[test]
    fn badge_text_caps_at_99_plus() {
        let list: Vec<Value> = (1..=120).map(|i| item(i, "f")).collect();
        let mut s = Shelf::new();
        s.on_event("shelf:changed", &items(&list));
        assert_eq!(s.view(&EnTr, W).badge.text, "99+");
    }

    #[test]
    fn selection_survives_only_for_existing_items() {
        let mut s = open_shelf(&[item(1, "a"), item(2, "b")]);
        s.on_action(Action::ChipPress { id: 1, x: 0.0, y: 0.0, ctrl: true, button0: true });
        s.on_action(Action::ChipPress { id: 2, x: 0.0, y: 0.0, ctrl: true, button0: true });
        assert_eq!(s.selection(), [1, 2]);
        s.on_action(Action::ChipPress { id: 1, x: 0.0, y: 0.0, ctrl: true, button0: true });
        assert_eq!(s.selection(), [2]); // ctrl again deselects
        s.on_event("shelf:changed", &items(&[item(1, "a")]));
        assert!(s.selection().is_empty());
        s.on_hide();
        assert!(s.selection().is_empty());
    }

    #[test]
    fn dragging_one_chip_out_pins_the_island_and_closes_after_a_copy() {
        let mut s = open_shelf(&[item(1, "a"), item(2, "b")]);
        s.on_action(Action::ChipPress { id: 1, x: 100.0, y: 100.0, ctrl: false, button0: true });
        assert!(s.on_action(Action::PointerMove { x: 103.0, y: 102.0 }).is_empty()); // under 6 px
        let c = s.on_action(Action::PointerMove { x: 110.0, y: 100.0 });
        assert_eq!(c, vec![BackendCall::new("shelf_drag_out", json!({ "ids": [1] }))]);
        assert!(s.take_fx().contains(&Fx::SetPinned(true)));
        assert!(s.view(&EnTr, W).chips[0].ghost);
        // while the native drag runs, our own drag events are ignored
        s.on_event("tauri://drag-enter", &Value::Null);
        assert!(s.take_fx().is_empty());
        s.on_result("shelf_drag_out", &json!({}), &json!("copied"), &EnTr);
        assert!(s.take_fx().contains(&Fx::SetPinned(false)));
        assert!(!s.view(&EnTr, W).chips[0].ghost);
        s.tick(149.0);
        assert!(s.take_fx().is_empty());
        s.tick(2.0);
        assert_eq!(s.take_fx(), vec![Fx::Collapse { via: None }]);
    }

    #[test]
    fn already_pinned_island_stays_pinned_and_a_cancelled_drag_does_not_close() {
        let mut s = open_shelf(&[item(1, "a")]);
        s.set_settings(&json!({ "pinned": true }));
        s.on_action(Action::ChipPress { id: 1, x: 0.0, y: 0.0, ctrl: false, button0: true });
        s.on_action(Action::PointerMove { x: 50.0, y: 0.0 });
        assert!(s.take_fx().is_empty());
        s.on_result("shelf_drag_out", &json!({}), &json!("cancelled"), &EnTr);
        assert!(s.take_fx().is_empty());
        s.tick(1000.0);
        assert!(s.take_fx().is_empty());
    }

    #[test]
    fn selected_chips_drag_together_and_drag_all_skips_missing() {
        let mut gone = item(3, "gone");
        gone["missing"] = json!(true);
        let mut s = open_shelf(&[item(1, "a"), item(2, "b"), gone]);
        s.on_action(Action::ChipPress { id: 1, x: 0.0, y: 0.0, ctrl: true, button0: true });
        s.on_action(Action::ChipPress { id: 2, x: 0.0, y: 0.0, ctrl: true, button0: true });
        s.on_action(Action::ChipPress { id: 2, x: 0.0, y: 0.0, ctrl: false, button0: true });
        assert_eq!(s.on_action(Action::PointerMove { x: 20.0, y: 0.0 })[0].args, json!({ "ids": [1, 2] }));
        s.on_result("shelf_drag_out", &json!({}), &Value::Null, &EnTr);
        // a chip outside the selection drags alone
        s.on_action(Action::ChipPress { id: 3, x: 0.0, y: 0.0, ctrl: false, button0: true }); // missing: no press at all
        assert!(s.on_action(Action::PointerMove { x: 20.0, y: 0.0 }).is_empty());
        s.on_action(Action::DragAllPress { x: 0.0, y: 0.0 });
        assert_eq!(s.on_action(Action::PointerMove { x: 20.0, y: 0.0 })[0].args, json!({ "ids": [1, 2] }));
        // releasing before the threshold cancels the press
        s.on_result("shelf_drag_out", &json!({}), &Value::Null, &EnTr);
        s.on_action(Action::DragAllPress { x: 0.0, y: 0.0 });
        s.on_action(Action::PointerRelease);
        assert!(s.on_action(Action::PointerMove { x: 99.0, y: 0.0 }).is_empty());
        // non-primary buttons never start anything
        assert!(s.on_action(Action::ChipPress { id: 1, x: 0.0, y: 0.0, ctrl: false, button0: false }).is_empty());
    }

    #[test]
    fn removing_a_chip_exits_first_then_calls_the_backend() {
        let mut s = open_shelf(&[item(1, "a"), item(2, "b")]);
        assert!(s.on_action(Action::RemoveChip(1)).is_empty());
        let fx = s.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "chip:1" && anim.kind == AnimKind::Exit { dx: 0.0, dy: 0.0, scale: 0.8 })));
        assert!(s.view(&EnTr, W).chips[0].hidden);
        assert!(s.tick(100.0).is_empty());
        assert_eq!(s.tick(70.0), vec![BackendCall::new("shelf_remove", json!({ "id": 1 }))]);
        // the call failed: the chip comes back after 900 ms
        s.on_result("shelf_remove", &json!({ "id": 1 }), &Value::Null, &EnTr);
        assert!(s.view(&EnTr, W).chips[0].hidden);
        s.tick(901.0);
        assert!(!s.view(&EnTr, W).chips[0].hidden);
    }

    #[test]
    fn clear_staggers_the_exits_and_waits_for_the_last() {
        let list: Vec<Value> = (1..=12).map(|i| item(i, "f")).collect();
        let mut s = open_shelf(&list);
        s.on_action(Action::Clear);
        let fx = s.take_fx();
        let delays: Vec<u32> = fx.iter().filter_map(|f| if let Fx::Anim { anim, .. } = f { Some(anim.delay_ms) } else { None }).collect();
        assert_eq!(delays.len(), 12);
        assert_eq!(delays[8], 160);
        assert_eq!(delays[11], 160); // capped at the 8th
        assert!(s.tick(300.0).is_empty());
        let c = s.tick(30.0);
        assert_eq!(c, vec![BackendCall { cmd: "shelf_clear", args: Value::Null }]);
        // shelf:changed then rebuilds the grid and un-hides everything
        s.on_event("shelf:changed", &items(&[]));
        assert!(s.view(&EnTr, W).chips.is_empty());
    }

    #[test]
    fn reduced_motion_removes_without_waiting() {
        let mut s = open_shelf(&[item(1, "a")]);
        s.set_settings(&json!({ "reduceMotion": true }));
        let c = s.on_action(Action::RemoveChip(1));
        assert_eq!(c, vec![BackendCall::new("shelf_remove", json!({ "id": 1 }))]);
        assert!(s.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
    }

    #[test]
    fn context_menu_entries_and_placement() {
        let mut gone = item(2, "gone");
        gone["missing"] = json!(true);
        let mut s = open_shelf(&[item(1, "a"), gone]);
        s.set_metrics(1.0, (600.0, 400.0));
        s.on_action(Action::ContextMenu { id: 1, x: 580.0, y: 390.0 });
        let m = s.view(&EnTr, W).menu.unwrap();
        assert_eq!((m.x, m.y), (450.0, 270.0)); // min(x, width - 150), min(y, height - 130)
        assert_eq!(m.entries.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["Open", "Show in folder", "Copy path", "Remove"]);
        assert!(s.wants_escape());
        assert_eq!(s.on_action(Action::MenuPick(1)), vec![BackendCall::new("shelf_reveal", json!({ "id": 1 }))]);
        assert!(s.view(&EnTr, W).menu.is_none());
        s.on_action(Action::ContextMenu { id: 2, x: 10.0, y: 10.0 }); // missing file: no open / reveal
        let m = s.view(&RuTr, W).menu.unwrap();
        assert_eq!(m.entries.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["Копировать путь", "Убрать"]);
        assert_eq!(s.on_action(Action::MenuPick(0))[0].cmd, "shelf_copy_path");
        s.on_action(Action::ContextMenu { id: 1, x: 10.0, y: 10.0 });
        s.take_fx();
        s.on_action(Action::MenuClose);
        assert!(s.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "menu")));
        s.on_action(Action::ContextMenu { id: 1, x: 10.0, y: 10.0 });
        assert!(s.on_action(Action::MenuPick(3)).is_empty()); // "Remove" starts the exit first
        assert!(s.view(&EnTr, W).chips[0].hidden);
        assert_eq!(s.on_action(Action::DoubleClick(1)), vec![BackendCall::new("shelf_open", json!({ "id": 1 }))]);
    }

    // ---------------- drag events ----------------

    fn drop_payload(paths: &[&str], x: f64, y: f64) -> Value {
        json!({ "paths": paths, "position": { "x": x, "y": y } })
    }

    #[test]
    fn open_island_drag_switches_to_the_shelf_or_home() {
        let mut s = open_shelf(&[]);
        s.on_event("tauri://drag-enter", &Value::Null);
        assert_eq!(s.take_fx(), vec![Fx::Expand { tab: "shelf" }]);
        assert!(s.view(&EnTr, W).over);
        s.on_event("tauri://drag-leave", &Value::Null);
        assert!(!s.view(&EnTr, W).over);
        // Home was the last tab: its launcher row takes drops, so Home opens instead
        s.set_settings(&json!({ "lastTab": "home" }));
        s.on_event("tauri://drag-enter", &Value::Null);
        assert_eq!(s.take_fx(), vec![Fx::Expand { tab: "home" }]);
        assert!(!s.view(&EnTr, W).over);
        // shelf module off: only the Home case reacts
        s.set_settings(&json!({ "module.shelf": false }));
        s.on_event("tauri://drag-enter", &Value::Null);
        assert_eq!(s.take_fx(), vec![]);
    }

    #[test]
    fn dropping_adds_the_paths_and_remembers_the_point() {
        let mut s = open_shelf(&[]);
        s.set_metrics(2.0, (900.0, 600.0));
        let c = s.on_event("tauri://drag-drop", &drop_payload(&["C:/a.pdf", "C:/b.txt"], 400.0, 200.0));
        assert_eq!(c, vec![BackendCall::new("shelf_add", json!({ "paths": ["C:/a.pdf", "C:/b.txt"] }))]);
        s.tick(10.0);
        // chips fly in from the drop point (css px 200,100): chip centred at (300, 300), zoom 1
        assert_eq!(s.fly_vector(Pt { x: 300.0, y: 300.0 }, 1.0), (-100.0, -200.0));
        assert_eq!(s.fly_vector(Pt { x: 300.0, y: 300.0 }, 2.0), (-50.0, -100.0));
        s.tick(6000.0);
        assert_eq!(s.fly_vector(Pt { x: 300.0, y: 300.0 }, 1.0), (0.0, -330.0)); // stale drop: from above the chip
    }

    #[test]
    fn drop_on_the_launcher_row_or_with_no_files_is_not_ours() {
        let mut s = open_shelf(&[]);
        assert!(s.on_drag_drop(&drop_payload(&["C:/a"], 1.0, 1.0), true).is_empty());
        assert!(s.on_drag_drop(&drop_payload(&[], 1.0, 1.0), false).is_empty());
        s.set_settings(&json!({ "module.shelf": false }));
        assert!(s.on_drag_drop(&drop_payload(&["C:/a"], 1.0, 1.0), false).is_empty());
    }

    #[test]
    fn drop_beside_the_launcher_row_shows_where_the_files_went() {
        let mut s = open_shelf(&[]);
        s.set_settings(&json!({ "lastTab": "home" }));
        s.on_drag_drop(&drop_payload(&["C:/a"], 1.0, 1.0), false);
        assert!(s.take_fx().contains(&Fx::Expand { tab: "shelf" }));
    }

    #[test]
    fn new_chips_fly_in_only_while_the_tab_is_on_screen() {
        let mut s = open_shelf(&[item(1, "a")]);
        s.on_show(); // enter = true for the next list
        s.on_result("shelf_list", &Value::Null, &json!([item(1, "a")]), &EnTr);
        s.take_fx();
        s.on_event("shelf:changed", &items(&[item(1, "a"), item(2, "b"), item(3, "c")]));
        let fx = s.take_fx();
        let fly: Vec<u32> = fx.iter().filter_map(|f| if let Fx::Anim { target, anim } = f { (anim.dur_ms == 700 && target.starts_with("chip:")).then_some(anim.delay_ms) } else { None }).collect();
        assert_eq!(fly, [0, 60]);
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "zone" && anim.dur_ms == 750)));
        assert!(s.view(&EnTr, W).landed);
        assert!(s.view(&EnTr, W).chips[1].fresh);
        s.tick(801.0);
        assert!(!s.view(&EnTr, W).landed);
        s.tick(1000.0);
        assert!(!s.view(&EnTr, W).chips[1].fresh);
        // tab not on screen: nothing flies
        s.on_hide();
        s.on_event("shelf:changed", &items(&[item(1, "a"), item(2, "b"), item(3, "c"), item(4, "d")]));
        assert!(s.take_fx().iter().all(|f| !matches!(f, Fx::Anim { anim, .. } if anim.dur_ms == 700)));
    }

    #[test]
    fn first_render_of_a_visit_staggers_the_chips() {
        let mut s = Shelf::new();
        s.set_expanded(true);
        s.on_show();
        s.on_result("shelf_list", &Value::Null, &json!([item(1, "a"), item(2, "b")]), &EnTr);
        let delays: Vec<u32> = s.take_fx().iter().filter_map(|f| if let Fx::Anim { target, anim } = f { target.starts_with("chip:").then_some(anim.delay_ms) } else { None }).collect();
        assert_eq!(delays, [0, 28]);
    }

    // ---------------- the folded-island drop card ----------------

    fn folded() -> Shelf {
        let mut s = Shelf::new();
        s.set_expanded(false);
        s.on_event("shelf:changed", &items(&[item(1, "a")]));
        s.take_fx();
        s
    }

    #[test]
    fn hovering_the_folded_island_shows_the_card() {
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        let fx = s.take_fx();
        let spec = fx.iter().find_map(|f| if let Fx::Live { key: "shelf-drop", spec: Some(w) } = f { Some(w.clone()) } else { None }).unwrap();
        assert_eq!((spec.priority, spec.tall, spec.ttl_ms), (95, true, Some(30_000)));
        assert!(fx.contains(&Fx::DragPull(true)));
        assert!(fx.contains(&Fx::EdgeColor(Some("var(--accent)".into()))));
        let card = s.view(&EnTr, W).peek.unwrap();
        assert_eq!((card.title.as_str(), card.sub.as_str(), card.phase), ("Drop to keep on the shelf", "On the shelf now: 1", "idle"));
        assert_eq!(s.view(&RuTr, W).peek.unwrap().title, "Отпустите, чтобы положить на полку");
        // a second enter does not restart it
        s.on_event("tauri://drag-enter", &Value::Null);
        assert!(s.take_fx().is_empty());
        // leaving without a drop ends it
        s.on_event("tauri://drag-leave", &Value::Null);
        let fx = s.take_fx();
        assert!(fx.contains(&Fx::Live { key: "shelf-drop", spec: None }) && fx.contains(&Fx::DragPull(false)) && fx.contains(&Fx::EdgeColor(None)));
        assert!(!s.peeking());
    }

    #[test]
    fn empty_shelf_card_text() {
        let mut s = Shelf::new();
        s.on_event("tauri://drag-enter", &Value::Null);
        assert_eq!(s.view(&EnTr, W).peek.unwrap().sub, "Files stay where they are");
    }

    #[test]
    fn dwelling_700ms_opens_the_island_on_the_shelf_or_home() {
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        s.take_fx();
        s.tick(699.0);
        assert!(s.peeking());
        s.tick(2.0);
        let fx = s.take_fx();
        assert!(fx.contains(&Fx::Expand { tab: "shelf" }));
        assert!(!s.peeking());
        assert!(s.view(&EnTr, W).over);
        s.set_settings(&json!({ "lastTab": "home" }));
        s.on_event("tauri://drag-leave", &Value::Null);
        s.on_event("tauri://drag-enter", &Value::Null);
        s.take_fx();
        s.tick(701.0);
        assert!(s.take_fx().contains(&Fx::Expand { tab: "home" }));
    }

    #[test]
    fn a_landed_card_gives_up_after_3_seconds_without_an_answer() {
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        s.on_event("tauri://drag-drop", &drop_payload(&["C:/a.txt"], 5.0, 5.0));
        s.tick(2999.0);
        assert!(s.peeking());
        s.tick(2.0);
        assert!(!s.peeking());
    }

    #[test]
    fn dropping_on_the_card_flies_the_files_in_and_reports_the_answer() {
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        s.take_fx();
        let c = s.on_event("tauri://drag-drop", &drop_payload(&["C:/a.pdf", "C:/b", "D:/c.mp3", "E:/d.txt"], 50.0, 20.0));
        assert_eq!(c[0].cmd, "shelf_add");
        let fx = s.take_fx();
        let flies: Vec<_> = fx.iter().filter_map(|f| if let Fx::Anim { target, anim } = f { target.starts_with("drop-fly:").then_some((target.clone(), anim.delay_ms, anim.dur_ms)) } else { None }).collect();
        assert_eq!(flies, [("drop-fly:0".to_string(), 0, 560), ("drop-fly:1".to_string(), 70, 560), ("drop-fly:2".to_string(), 140, 560)]); // first three only
        let v = s.view(&EnTr, W);
        assert_eq!(v.peek.as_ref().unwrap().phase, "go");
        assert_eq!(v.peek_fly, [("PDF".to_string(), "#e8494f"), ("FILE".to_string(), "#7c8090"), ("MP3".to_string(), "#a65be8")]);
        // the dwell no longer opens the island
        s.tick(800.0);
        assert!(s.peeking());
        // answer: 3 of 4 were new
        s.on_result("shelf_add", &json!({ "paths": ["a", "b", "c", "d"] }), &json!(3), &EnTr);
        let card = s.view(&EnTr, W).peek.unwrap();
        assert_eq!((card.phase, card.title.as_str(), card.sub.as_str()), ("ok", "On the shelf", "+3 files, the rest were already there"));
        assert!(s.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "drop-ico" && anim.dur_ms == 600)));
        s.tick(1399.0);
        assert!(s.peeking());
        s.tick(2.0);
        assert!(!s.peeking());
    }

    #[test]
    fn all_new_and_duplicate_answers() {
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        s.on_event("tauri://drag-drop", &drop_payload(&["C:/a"], 5.0, 5.0));
        s.on_result("shelf_add", &json!({ "paths": ["C:/a"] }), &json!(1), &RuTr);
        assert_eq!(s.view(&RuTr, W).peek.unwrap().sub, "+1 файл");
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        s.on_event("tauri://drag-drop", &drop_payload(&["C:/a", "C:/b"], 5.0, 5.0));
        s.on_result("shelf_add", &json!({ "paths": ["C:/a", "C:/b"] }), &json!(0), &EnTr);
        let card = s.view(&EnTr, W).peek.unwrap();
        assert_eq!((card.phase, card.title.as_str(), card.sub.as_str()), ("dup", "Already on the shelf", "Nothing new was added"));
        assert!(s.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "drop-ico" && anim.dur_ms == 450)));
        s.tick(1699.0);
        assert!(s.peeking());
        s.tick(2.0);
        assert!(!s.peeking());
    }

    #[test]
    fn while_the_card_reports_a_landing_the_count_widget_stays_quiet() {
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        s.on_event("tauri://drag-drop", &drop_payload(&["C:/b"], 5.0, 5.0));
        s.take_fx();
        s.on_event("shelf:changed", &items(&[item(1, "a"), item(2, "b")]));
        assert!(s.take_fx().contains(&Fx::Live { key: "shelf", spec: None }));
    }

    #[test]
    fn hiding_the_pill_or_turning_the_module_off_ends_the_card() {
        let mut s = folded();
        s.on_event("tauri://drag-enter", &Value::Null);
        s.on_event("pill:hide", &Value::Null);
        assert!(!s.peeking());
        s.on_event("tauri://drag-enter", &Value::Null);
        s.on_event("settings:changed", &json!({ "module.shelf": false }));
        assert!(!s.peeking());
    }

    #[test]
    fn reduced_motion_skips_card_and_chip_cues() {
        let mut s = folded();
        s.set_settings(&json!({ "reduceMotion": true }));
        s.on_event("tauri://drag-enter", &Value::Null);
        s.on_event("tauri://drag-drop", &drop_payload(&["C:/a"], 5.0, 5.0));
        s.on_result("shelf_add", &json!({ "paths": ["C:/a"] }), &json!(0), &EnTr);
        assert!(s.take_fx().iter().all(|f| !matches!(f, Fx::Anim { .. })));
    }

    #[test]
    fn empty_state_texts_in_both_languages() {
        let s = Shelf::new();
        let v = s.view(&RuTr, W);
        assert!(v.empty && !v.head_visible);
        assert_eq!(v.empty_state.title, "Полка пуста");
        assert_eq!(s.view(&EnTr, W).drop_title, "Drop files here");
        assert_eq!(s.view(&EnTr, W).clear_label, "Clear");
    }
}
