//! Home launcher strip (programs / folders / files): drop to add, click opens, x / right-click / Delete removes,
//! pointer-drag reorders. Port of the "launcher row" half of `tab-home.js`.
//!
//! Backend: `launcher_list`, `launcher_add {paths}`, `launcher_remove {id}`, `launcher_open {id}`, `launcher_move {id, newIndex}`;
//! event `launcher:changed {count, items}`; Tauri drag events `tauri://drag-enter|over|leave|drop` (positions are physical px).
//! The renderer reports the row's geometry with [`Launcher::set_layout`]; reorder maths (6 px threshold, pitch shifting) is here.

use super::anim::Anim;
use super::{bool_of, i64_of, str_of, BackendCall, Fx, FxQueue, Tr};
use serde_json::{json, Value};
use std::collections::HashSet;

/// Millisecond the "gone" fade lasts before `launcher_remove` is sent (`dropLink`).
pub const REMOVE_DELAY_MS: f64 = 160.0;
/// Pointer travel before a press turns into a reorder drag.
pub const DRAG_THRESHOLD: f32 = 6.0;
/// Hit-test slack above and below the row for dropped files.
pub const HIT_SLACK: f32 = 10.0;

/// `launcher.rs::Link`
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: i64,
    pub title: String,
    pub target: String,
    pub missing: bool,
    pub is_dir: bool,
    pub icon: Option<String>,
}

impl Link {
    pub fn from_json(v: &Value) -> Link {
        Link {
            id: i64_of(v, "id"),
            title: str_of(v, "title").to_string(),
            target: str_of(v, "target").to_string(),
            missing: bool_of(v, "missing"),
            is_dir: bool_of(v, "is_dir"),
            icon: v.get("icon").and_then(Value::as_str).map(str::to_string),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub fn from_json(v: &Value) -> Option<Pt> {
        Some(Pt { x: v.get("x")?.as_f64()? as f32, y: v.get("y")?.as_f64()? as f32 })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Click or Enter on an icon.
    Open(i64),
    /// The small x, right-click or Delete.
    Remove(i64),
    /// The "+" tile.
    AddHint,
    /// Mouse down on an icon (primary button, not on its x): may become a reorder drag.
    ReorderPress { id: i64, x: f32 },
    ReorderMove { x: f32 },
    ReorderRelease,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Reorder {
    id: i64,
    from: usize,
    to: usize,
    x0: f32,
    dx: f32,
    dragging: bool,
}

/// One icon of the strip as the renderer draws it.
#[derive(Clone, Debug, PartialEq)]
pub struct Tile {
    pub id: i64,
    /// Tooltip: the title, plus target and "(not found)" for a missing target.
    pub tooltip: String,
    pub icon: Option<String>,
    /// Glyph when there is no icon image: `folder` or `file`.
    pub glyph: &'static str,
    pub missing: bool,
    /// Being dragged (raised, 85 % opacity, no transition).
    pub dragging: bool,
    /// Fading out before the removal is sent.
    pub gone: bool,
    /// Horizontal offset in CSS px (the dragged icon follows the pointer, the others shift by one pitch).
    pub offset_x: f32,
    pub remove_title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    /// `has`: at least one link (the hint text hides).
    pub has: bool,
    /// A file drag hovers the row.
    pub over: bool,
    pub reordering: bool,
    pub visible: bool,
    pub tiles: Vec<Tile>,
    pub hint: String,
    pub add_title: String,
}

pub struct Launcher {
    links: Vec<Link>,
    seen: Option<HashSet<i64>>,
    reorder: Option<Reorder>,
    suppress_click: bool,
    gone: Vec<(i64, f64)>,
    over: bool,
    shown: bool,
    enabled: bool,
    row: Rect,
    pitch: f32,
    zoom: f32,
    dpr: f32,
    mono: f64,
    reduce: bool,
    fx: FxQueue,
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher::new()
    }
}

impl Launcher {
    pub fn new() -> Launcher {
        Launcher {
            links: vec![],
            seen: None,
            reorder: None,
            suppress_click: false,
            gone: vec![],
            over: false,
            shown: false,
            enabled: true,
            row: Rect::default(),
            pitch: 0.0,
            zoom: 1.0,
            dpr: 1.0,
            mono: 0.0,
            reduce: false,
            fx: FxQueue::default(),
        }
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }
    pub fn take_fx(&mut self) -> Vec<Fx> {
        self.fx.take()
    }
    /// Home forwards `module.launcher` (`!== false`) and the reduce-motion state.
    pub fn configure(&mut self, enabled: bool, reduce: bool) {
        self.enabled = enabled;
        self.reduce = reduce;
    }
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        if !shown {
            self.over = false;
        }
    }
    /// Geometry from the renderer: the row's rectangle in CSS px, the distance between two icon origins (0 = unknown,
    /// 52 is used: 40 px icon + 12 px gap), the panel zoom and the device pixel ratio.
    pub fn set_layout(&mut self, row: Rect, pitch: f32, zoom: f32, dpr: f32) {
        self.row = row;
        self.pitch = pitch;
        self.zoom = if zoom > 0.0 { zoom } else { 1.0 };
        self.dpr = if dpr > 0.0 { dpr } else { 1.0 };
    }

    /// `launcher_list` on show, after add / remove.
    pub fn load_calls(&self) -> Vec<BackendCall> {
        vec![BackendCall::bare("launcher_list")]
    }

    pub fn set_links(&mut self, v: &Value) {
        self.links = v.as_array().map(|a| a.iter().map(Link::from_json).collect()).unwrap_or_default();
        self.render();
    }

    fn render(&mut self) {
        let ids: HashSet<i64> = self.links.iter().map(|l| l.id).collect();
        if let Some(seen) = &self.seen {
            let removed = seen.iter().any(|id| !ids.contains(id));
            let added = ids.iter().any(|id| !seen.contains(id));
            for l in &self.links {
                if !seen.contains(&l.id) {
                    self.fx.anim(self.reduce, format!("link:{}", l.id), Anim::pop(0.6, 380));
                }
            }
            if removed || added {
                self.fx.anim(self.reduce, "launcher", Anim::launcher_flip());
            }
        }
        self.seen = Some(ids);
        self.gone.retain(|(id, _)| self.links.iter().any(|l| l.id == *id));
    }

    // ---- hit testing of dropped files ----
    /// `hitRow`: is a physical-px position over the row (with 10 px slack above and below)? False while hidden or disabled.
    pub fn hit(&self, pos: Option<Pt>) -> bool {
        let Some(p) = pos else { return false };
        if !self.shown || !self.enabled {
            return false;
        }
        let (x, y) = (p.x / self.dpr, p.y / self.dpr);
        x >= self.row.left && x <= self.row.right && y >= self.row.top - HIT_SLACK && y <= self.row.bottom + HIT_SLACK
    }

    /// Tauri drag events and `launcher:changed`. Returns follow-up calls (a drop adds the paths).
    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        match name {
            "tauri://drag-enter" | "tauri://drag-over" => {
                self.over = self.hit(payload.get("position").and_then(Pt::from_json));
                vec![]
            }
            "tauri://drag-leave" => {
                self.over = false;
                vec![]
            }
            "tauri://drag-drop" => {
                self.over = false;
                let paths = payload.get("paths").and_then(Value::as_array).cloned().unwrap_or_default();
                if paths.is_empty() || !self.hit(payload.get("position").and_then(Pt::from_json)) {
                    return vec![];
                }
                vec![BackendCall::new("launcher_add", json!({ "paths": paths }))]
            }
            "launcher:changed" => {
                self.set_links(payload.get("items").unwrap_or(&Value::Null));
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn on_action(&mut self, a: Action, tr: &dyn Tr) -> Vec<BackendCall> {
        match a {
            Action::Open(id) => {
                if self.suppress_click {
                    return vec![];
                }
                vec![BackendCall::new("launcher_open", json!({ "id": id }))]
            }
            Action::Remove(id) => {
                if !self.links.iter().any(|l| l.id == id) || self.gone.iter().any(|(g, _)| *g == id) {
                    return vec![];
                }
                self.gone.push((id, self.mono + REMOVE_DELAY_MS));
                vec![]
            }
            Action::AddHint => {
                self.fx.toast("launcher-hint", "plus", tr.t("Перетащи программу, папку или файл на эту строку", "Drag a program, folder or file onto this row"), 2600);
                vec![]
            }
            Action::ReorderPress { id, x } => {
                if let Some(from) = self.links.iter().position(|l| l.id == id) {
                    self.reorder = Some(Reorder { id, from, to: from, x0: x, dx: 0.0, dragging: false });
                }
                vec![]
            }
            Action::ReorderMove { x } => {
                let (pitch, zoom, len) = (self.eff_pitch(), self.zoom, self.links.len());
                if let Some(r) = self.reorder.as_mut() {
                    let dx = (x - r.x0) / zoom;
                    if !r.dragging {
                        if dx.abs() < DRAG_THRESHOLD {
                            return vec![];
                        }
                        r.dragging = true;
                    }
                    r.dx = dx;
                    r.to = ((r.from as f32 + dx / pitch).round().max(0.0) as usize).min(len.saturating_sub(1));
                }
                vec![]
            }
            Action::ReorderRelease => {
                let Some(r) = self.reorder.take() else { return vec![] };
                if !r.dragging {
                    return vec![];
                }
                self.suppress_click = true;
                if r.to == r.from {
                    return vec![];
                }
                let m = self.links.remove(r.from);
                self.links.insert(r.to, m); // optimistic; launcher:changed confirms
                self.fx.anim(self.reduce, "launcher", Anim::launcher_flip());
                vec![BackendCall::new("launcher_move", json!({ "id": r.id, "newIndex": r.to }))]
            }
        }
    }

    fn eff_pitch(&self) -> f32 {
        if self.pitch > 0.0 {
            self.pitch
        } else {
            52.0
        }
    }

    /// Answers of `launcher_open`, `launcher_add`, `launcher_remove`, `launcher_move`, `launcher_list`.
    pub fn on_result(&mut self, cmd: &str, args: &Value, res: &Value, tr: &dyn Tr) -> Vec<BackendCall> {
        match cmd {
            "launcher_list" => {
                self.set_links(res);
                vec![]
            }
            "launcher_open" => {
                if res.as_bool() == Some(true) {
                    self.fx.push(Fx::Collapse { via: None });
                } else {
                    let id = args.get("id").and_then(Value::as_i64).unwrap_or(-1);
                    let title = self.links.iter().find(|l| l.id == id).map(|l| l.title.clone()).unwrap_or_default();
                    self.fx.toast("launcher-open", "folder", format!("{}{}", tr.t("Не удалось открыть: ", "Could not open: "), title), 2400);
                }
                vec![]
            }
            "launcher_add" => {
                if res.as_u64().unwrap_or(0) == 0 {
                    self.fx.toast("launcher-add", "folder", tr.t("Уже в списке или не найдено", "Already there or not found"), 2200);
                }
                self.load_calls()
            }
            "launcher_remove" => self.load_calls(),
            _ => vec![],
        }
    }

    /// Timers: the removal fade, the click suppression after a drag.
    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        self.mono += dt_ms;
        self.suppress_click = false;
        let due: Vec<i64> = self.gone.iter().filter(|(_, at)| *at <= self.mono).map(|(id, _)| *id).collect();
        self.gone.retain(|(_, at)| *at > self.mono);
        due.into_iter().map(|id| BackendCall::new("launcher_remove", json!({ "id": id }))).collect()
    }

    pub fn view(&self, tr: &dyn Tr) -> View {
        let pitch = self.eff_pitch();
        let tiles = self
            .links
            .iter()
            .enumerate()
            .map(|(k, l)| {
                let mut offset_x = 0.0;
                let mut dragging = false;
                if let Some(r) = self.reorder.filter(|r| r.dragging) {
                    if r.id == l.id {
                        dragging = true;
                        offset_x = r.dx;
                    } else if r.from < r.to && k > r.from && k <= r.to {
                        offset_x = -pitch;
                    } else if r.from > r.to && k >= r.to && k < r.from {
                        offset_x = pitch;
                    }
                }
                let tooltip = if l.missing { format!("{}\n{}\n{}", l.title, l.target, tr.t("(не найдено)", "(not found)")) } else { l.title.clone() };
                Tile {
                    id: l.id,
                    tooltip,
                    icon: l.icon.clone(),
                    glyph: if l.is_dir { "folder" } else { "file" },
                    missing: l.missing,
                    dragging,
                    gone: self.gone.iter().any(|(g, _)| *g == l.id),
                    offset_x,
                    remove_title: tr.t("Убрать", "Remove"),
                }
            })
            .collect();
        View {
            has: !self.links.is_empty(),
            over: self.over,
            reordering: self.reorder.map_or(false, |r| r.dragging),
            visible: self.enabled,
            tiles,
            hint: tr.t("Перетащи сюда ярлыки программ, папки или файлы", "Drop program shortcuts, folders or files here"),
            add_title: tr.t("Перетащи сюда программу, папку или файл", "Drop a program, folder or file here"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    fn links(n: i64) -> Value {
        Value::Array((1..=n).map(|i| json!({ "id": i, "title": format!("App{i}"), "target": format!("C:/a{i}.exe"), "missing": false, "is_dir": false, "icon": null })).collect())
    }

    fn ready(n: i64) -> Launcher {
        let mut l = Launcher::new();
        l.set_shown(true);
        l.set_layout(Rect { left: 20.0, top: 200.0, right: 420.0, bottom: 260.0 }, 52.0, 1.0, 1.0);
        l.set_links(&links(n));
        l.take_fx();
        l
    }

    #[test]
    fn first_load_does_not_animate_later_additions_pop_in() {
        let mut l = Launcher::new();
        l.set_links(&links(2));
        assert!(l.take_fx().is_empty());
        l.set_links(&links(3));
        let fx = l.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "link:3")));
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "launcher" && anim.dur_ms == 280)));
        // a removal slides the rest closed without any pop
        l.set_links(&links(2));
        let fx = l.take_fx();
        assert_eq!(fx.len(), 1);
    }

    #[test]
    fn reduced_motion_pushes_no_cues() {
        let mut l = Launcher::new();
        l.configure(true, true);
        l.set_links(&links(1));
        l.set_links(&links(2));
        assert!(l.take_fx().is_empty());
    }

    #[test]
    fn open_collapses_or_toasts() {
        let mut l = ready(2);
        let c = l.on_action(Action::Open(2), &RuTr);
        assert_eq!(c, vec![BackendCall::new("launcher_open", json!({ "id": 2 }))]);
        l.on_result("launcher_open", &json!({ "id": 2 }), &json!(true), &RuTr);
        assert_eq!(l.take_fx(), vec![Fx::Collapse { via: None }]);
        l.on_result("launcher_open", &json!({ "id": 2 }), &json!(false), &EnTr);
        assert_eq!(l.take_fx(), vec![Fx::Toast { id: "launcher-open", icon: "folder", title: "Could not open: App2".into(), ttl_ms: 2400 }]);
    }

    #[test]
    fn remove_fades_160ms_then_sends_and_reloads() {
        let mut l = ready(3);
        assert!(l.on_action(Action::Remove(2), &RuTr).is_empty());
        assert!(l.view(&RuTr).tiles[1].gone);
        assert!(l.tick(100.0).is_empty());
        assert_eq!(l.tick(61.0), vec![BackendCall::new("launcher_remove", json!({ "id": 2 }))]);
        assert_eq!(l.on_result("launcher_remove", &json!({ "id": 2 }), &Value::Null, &RuTr), vec![BackendCall::bare("launcher_list")]);
        // double remove does not queue twice
        l.on_action(Action::Remove(1), &RuTr);
        l.on_action(Action::Remove(1), &RuTr);
        assert_eq!(l.tick(200.0).len(), 1);
    }

    #[test]
    fn reorder_threshold_and_shifts() {
        let mut l = ready(4);
        l.on_action(Action::ReorderPress { id: 1, x: 100.0 }, &RuTr);
        l.on_action(Action::ReorderMove { x: 103.0 }, &RuTr); // under 6 px: still a click
        assert!(!l.view(&RuTr).reordering);
        l.on_action(Action::ReorderMove { x: 100.0 + 110.0 }, &RuTr); // ~2.1 pitches: lands on index 2
        let v = l.view(&RuTr);
        assert!(v.reordering);
        assert_eq!(v.tiles[0].offset_x, 110.0);
        assert!(v.tiles[0].dragging);
        assert_eq!(v.tiles[1].offset_x, -52.0);
        assert_eq!(v.tiles[2].offset_x, -52.0);
        assert_eq!(v.tiles[3].offset_x, 0.0);
        let c = l.on_action(Action::ReorderRelease, &RuTr);
        assert_eq!(c, vec![BackendCall::new("launcher_move", json!({ "id": 1, "newIndex": 2 }))]);
        assert_eq!(l.links().iter().map(|x| x.id).collect::<Vec<_>>(), vec![2, 3, 1, 4]);
        // the click that ends the drag is swallowed once, then clicks work again after a tick
        assert!(l.on_action(Action::Open(1), &RuTr).is_empty());
        l.tick(1.0);
        assert_eq!(l.on_action(Action::Open(1), &RuTr).len(), 1);
    }

    #[test]
    fn reorder_left_clamps_and_shifts_the_other_way() {
        let mut l = ready(4);
        l.on_action(Action::ReorderPress { id: 4, x: 300.0 }, &RuTr);
        l.on_action(Action::ReorderMove { x: -500.0 }, &RuTr);
        let v = l.view(&RuTr);
        assert_eq!(v.tiles[0].offset_x, 52.0);
        assert_eq!(v.tiles[2].offset_x, 52.0);
        assert_eq!(v.tiles[3].offset_x, -800.0);
        let c = l.on_action(Action::ReorderRelease, &RuTr);
        assert_eq!(c[0].args, json!({ "id": 4, "newIndex": 0 }));
    }

    #[test]
    fn drag_back_to_start_sends_nothing() {
        let mut l = ready(3);
        l.on_action(Action::ReorderPress { id: 2, x: 50.0 }, &RuTr);
        l.on_action(Action::ReorderMove { x: 120.0 }, &RuTr);
        l.on_action(Action::ReorderMove { x: 52.0 + 4.0 }, &RuTr);
        assert!(l.on_action(Action::ReorderRelease, &RuTr).is_empty());
        assert_eq!(l.links().iter().map(|x| x.id).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn panel_zoom_scales_pointer_travel() {
        let mut l = ready(3);
        l.set_layout(Rect::default(), 52.0, 2.0, 1.0);
        l.on_action(Action::ReorderPress { id: 1, x: 0.0 }, &RuTr);
        l.on_action(Action::ReorderMove { x: 208.0 }, &RuTr); // 104 css px = 2 pitches
        assert_eq!(l.view(&RuTr).tiles[0].offset_x, 104.0);
        assert_eq!(l.on_action(Action::ReorderRelease, &RuTr)[0].args["newIndex"], 2);
    }

    #[test]
    fn file_drop_hits_only_the_row_with_slack() {
        let mut l = Launcher::new();
        l.set_layout(Rect { left: 20.0, top: 200.0, right: 420.0, bottom: 260.0 }, 52.0, 1.0, 1.5);
        let drop = |x: f64, y: f64| json!({ "paths": ["C:/x.exe"], "position": { "x": x, "y": y } });
        // hidden: never a hit
        assert!(l.on_event("tauri://drag-drop", &drop(300.0, 350.0)).is_empty());
        l.set_shown(true);
        // physical (300, 350) = css (200, 233.3): inside
        assert_eq!(l.on_event("tauri://drag-drop", &drop(300.0, 350.0)), vec![BackendCall::new("launcher_add", json!({ "paths": ["C:/x.exe"] }))]);
        // css y 189 is within the 10 px slack above (200 - 10 = 190)? 189 is not
        assert!(l.on_event("tauri://drag-drop", &drop(300.0, 283.5)).is_empty());
        assert!(!l.on_event("tauri://drag-drop", &drop(300.0, 285.0)).is_empty()); // css 190
        // empty path list never adds
        assert!(l.on_event("tauri://drag-drop", &json!({ "paths": [], "position": { "x": 300.0, "y": 350.0 } })).is_empty());
        // module off: no hit
        l.configure(false, false);
        assert!(l.on_event("tauri://drag-drop", &drop(300.0, 350.0)).is_empty());
    }

    #[test]
    fn hover_highlight_follows_drag_events() {
        let mut l = ready(1);
        l.on_event("tauri://drag-enter", &json!({ "position": { "x": 100.0, "y": 230.0 } }));
        assert!(l.view(&EnTr).over);
        l.on_event("tauri://drag-over", &json!({ "position": { "x": 100.0, "y": 600.0 } }));
        assert!(!l.view(&EnTr).over);
        l.on_event("tauri://drag-enter", &json!({ "position": { "x": 100.0, "y": 230.0 } }));
        l.on_event("tauri://drag-leave", &Value::Null);
        assert!(!l.view(&EnTr).over);
    }

    #[test]
    fn add_result_toasts_when_nothing_was_added_and_reloads() {
        let mut l = ready(1);
        let c = l.on_result("launcher_add", &json!({ "paths": [] }), &json!(0), &RuTr);
        assert_eq!(c, vec![BackendCall::bare("launcher_list")]);
        assert_eq!(l.take_fx(), vec![Fx::Toast { id: "launcher-add", icon: "folder", title: "Уже в списке или не найдено".into(), ttl_ms: 2200 }]);
        l.on_result("launcher_add", &json!({}), &json!(2), &RuTr);
        assert!(l.take_fx().is_empty());
    }

    #[test]
    fn missing_links_show_target_and_not_found() {
        let mut l = Launcher::new();
        l.set_links(&json!([{ "id": 7, "title": "Old", "target": "D:/old.exe", "missing": true, "is_dir": true, "icon": null }]));
        let t = &l.view(&EnTr).tiles[0];
        assert_eq!(t.tooltip, "Old\nD:/old.exe\n(not found)");
        assert_eq!(t.glyph, "folder");
        assert!(t.missing);
        assert!(l.view(&RuTr).has);
    }

    #[test]
    fn changed_event_replaces_the_list() {
        let mut l = ready(1);
        l.on_event("launcher:changed", &json!({ "count": 2, "items": links(2) }));
        assert_eq!(l.links().len(), 2);
        l.on_event("launcher:changed", &json!({ "count": 0 }));
        assert!(l.links().is_empty() && !l.view(&EnTr).has);
    }

    #[test]
    fn add_tile_shows_a_hint() {
        let mut l = ready(0);
        l.on_action(Action::AddHint, &EnTr);
        assert_eq!(l.take_fx(), vec![Fx::Toast { id: "launcher-hint", icon: "plus", title: "Drag a program, folder or file onto this row".into(), ttl_ms: 2600 }]);
    }
}
