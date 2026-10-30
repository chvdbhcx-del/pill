//! The Note tab view: note chips, the rich editor, the toolbar, and (on the right) the timer ring + reminders card.
//!
//! Model: `pill_ui_core::panel::b::note::NoteTab` (owns the timer card model too). This file is the glue the module docs of `kit.rs` describe:
//! backend calls through `kit::Calls`, events from the bus, the editor ([`Editor`]) and the card ([`TimerCardView`]) as child painters, the
//! toolbar / chips as `kit::Clicks` targets. Look and layout follow `src/tab-note.css` (grid `1fr 232px`, gap 12; left column chips 24 /
//! editor / toolbar 28 with 8 px gaps).
//!
//! The editor reports `EdOut::Changed` ONLY after a user edit, so opening, scrolling, selecting and leaving a note never rewrites it: the
//! stored HTML stays byte for byte what the web UI saved (`NoteTab` keeps the loaded HTML until an `EditorChanged` arrives).
use super::editor::{EdOut, Editor};
use super::kit::{lang_of, ms_of, route_effects, Calls, Clicks, Cues, Icons};
use super::timer_card::{TimerCardView, CARD_W};
use crate::ui::view::{Cx, Msg, Ticket, View};
use glasskit::canvas::Canvas;
use glasskit::ease::{curves, Tween};
use glasskit::geom::Rect;
use glasskit::input::{InputEvent, MouseButton, Response};
use glasskit::scroll::ScrollState;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::note::{Action, Fmt, NoteTab, NoteView, SaveState};
use pill_ui_core::panel::b::{Effect, Kv};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use windows::Foundation::Numerics::Matrix3x2;

/// Width of the right column (`grid-template-columns: minmax(0, 1fr) 232px`) and the column gap.
const GAP: f32 = 12.0;
const CHIP_H: f32 = 24.0;
const BAR_H: f32 = 28.0;
const VIEW_REFRESH_MS: f64 = 100.0;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum K {
    Chip(i64),
    Add,
    Tool(usize),
    Remind,
    Copy,
    Del,
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Persistent UI prefs (`localStorage` of the web page): `pill.note.cur`, `pill.timer.sel`
// ---------------------------------------------------------------------------------------------------------------------------------------

/// A tiny string map persisted as JSON in `%APPDATA%\pill\ui-prefs.json` (shared by both models through [`SharedMap`]); in memory when `path`
/// is `None`. Reads happen once at construction, a write rewrites the whole (small) file; a failed write is ignored like blocked storage.
#[derive(Default)]
pub struct PrefsFile {
    path: Option<PathBuf>,
    map: HashMap<String, String>,
}

impl PrefsFile {
    pub fn memory() -> PrefsFile {
        PrefsFile::default()
    }

    /// The app's prefs file (`%APPDATA%\pill\ui-prefs.json`).
    pub fn app() -> PrefsFile {
        let path = std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("pill").join("ui-prefs.json"));
        PrefsFile::at(path)
    }

    pub fn at(path: Option<PathBuf>) -> PrefsFile {
        let map = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()))
            .unwrap_or_default();
        PrefsFile { path, map }
    }

    fn save(&self) {
        let Some(p) = &self.path else { return };
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let obj: serde_json::Map<String, Value> = self.map.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        let _ = std::fs::write(p, Value::Object(obj).to_string());
    }
}

impl Kv for PrefsFile {
    fn get(&self, k: &str) -> Option<String> {
        self.map.get(k).cloned()
    }
    fn set(&mut self, k: &str, v: &str) {
        if self.map.get(k).map(String::as_str) != Some(v) {
            self.map.insert(k.to_string(), v.to_string());
            self.save();
        }
    }
}

/// One [`PrefsFile`] used by the note model and the timer model.
#[derive(Clone)]
pub struct SharedMap(Rc<RefCell<PrefsFile>>);

impl SharedMap {
    pub fn new(p: PrefsFile) -> SharedMap {
        SharedMap(Rc::new(RefCell::new(p)))
    }
}

impl Kv for SharedMap {
    fn get(&self, k: &str) -> Option<String> {
        self.0.borrow().get(k)
    }
    fn set(&mut self, k: &str, v: &str) {
        self.0.borrow_mut().set(k, v)
    }
}

/// The PC's UTC offset in minutes (reminder times are shown in local time).
fn tz_offset_min() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let extra = if id == 2 { tz.DaylightBias } else { tz.StandardBias };
    -(tz.Bias + extra)
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------------------------------------------------------------------

pub struct NoteScreen {
    m: NoteTab,
    view: Option<NoteView>,
    view_age: f64,
    calls: Calls,
    ed: Editor,
    card: TimerCardView,
    clicks: Clicks<K>,
    cues: Cues,
    ic: Icons,
    rect: Rect,
    ed_rect: Rect,
    bar_rect: Rect,
    chips_rect: Rect,
    chip_sv: ScrollState,
    chips_w: f32,
    chip_to_show: bool,
    mounted: bool,
    visible: bool,
    carry: f64,
    settings_t: Option<Ticket>,
    toast_t: Tween<f32>,
    toast: (String, bool),
    saved_t: Tween<f32>,
    saved_prev: SaveState,
    saved_now: SaveState,
    dirty_view: bool,
    last_lang: Option<pill_ui_core::panel::b::Lang>,
    /// tests: (unix ms, utc offset min) instead of the system clock
    fixed_clock: Option<(i64, i32)>,
}

impl Default for NoteScreen {
    fn default() -> Self {
        NoteScreen::new()
    }
}

impl NoteScreen {
    pub fn new() -> NoteScreen {
        NoteScreen::with_prefs(PrefsFile::app())
    }

    pub fn with_prefs(p: PrefsFile) -> NoteScreen {
        let kv = SharedMap::new(p);
        let th = Theme::pill_dark();
        let mut ic = Icons::new();
        ic.add("n-mark", r#"<path d="M9 14l6-6 3 3-6 6H9z"/><path d="M4 20h9"/>"#);
        ic.add("n-ul", r#"<path d="M9.5 7H20M9.5 12H20M9.5 17H20"/><circle cx="5" cy="7" r="1" fill="currentColor"/><circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="5" cy="17" r="1" fill="currentColor"/>"#);
        ic.add("n-ol", r#"<path d="M10 7h10M10 12h10M10 17h10M4 5.5l1.5-.7V10M4 15.2c0-1.2 2.5-1.2 2.5 0S4 17 4 18.2h2.6"/>"#);
        ic.add("n-todo", r#"<path d="M3.5 7l1.5 1.5L8 5.5M3.5 16.5L5 18l3-3M12 7h8M12 16.5h8"/>"#);
        ic.add("n-plus", r#"<path d="M12 6v12M6 12h12"/>"#);
        ic.add("n-trash", r#"<path d="M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13"/>"#);
        ic.add("n-check", r#"<path d="M5 12.5l4.5 4.5L19 7.5"/>"#);
        ic.add("n-copy", r#"<rect x="9" y="9" width="11" height="11" rx="2.5"/><path d="M15 9V6.5A2.5 2.5 0 0 0 12.5 4h-6A2.5 2.5 0 0 0 4 6.5v6A2.5 2.5 0 0 0 6.5 15H9"/>"#);
        ic.add("n-bell", r#"<path d="M6 16v-5a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>"#);
        NoteScreen {
            m: NoteTab::new(pill_ui_core::panel::b::Lang::En, Box::new(kv.clone()), Box::new(kv)),
            view: None,
            view_age: 1.0e9,
            calls: Calls::new(),
            ed: Editor::new(),
            card: TimerCardView::new(&th),
            clicks: Clicks::new(),
            cues: Cues::new(),
            ic,
            rect: Rect::default(),
            ed_rect: Rect::default(),
            bar_rect: Rect::default(),
            chips_rect: Rect::default(),
            chip_sv: ScrollState::new(),
            chips_w: 0.0,
            chip_to_show: false,
            mounted: false,
            visible: false,
            carry: 0.0,
            settings_t: None,
            toast_t: Tween::at_rest(0.0),
            toast: (String::new(), false),
            saved_t: Tween::at_rest(1.0),
            saved_prev: SaveState::Saved,
            saved_now: SaveState::Saved,
            dirty_view: true,
            last_lang: None,
            fixed_clock: None,
        }
    }

    /// Use a fixed clock (unix ms, UTC offset in minutes) instead of the system's: snapshots and tests.
    pub fn set_fixed_clock(&mut self, now_ms: i64, tz_min: i32) {
        self.fixed_clock = Some((now_ms, tz_min));
    }

    /// The model (tests inspect it).
    pub fn model(&self) -> &NoteTab {
        &self.m
    }

    pub fn editor(&mut self) -> &mut Editor {
        &mut self.ed
    }

    fn touch(&mut self) {
        self.dirty_view = true;
    }

    fn sync_cx(&mut self, cx: &Cx) {
        let lang = lang_of(cx);
        if self.last_lang != Some(lang) {
            self.last_lang = Some(lang);
            self.m.set_lang(lang);
            self.touch();
        }
        self.m.set_reduce_motion(cx.reduced_motion);
        self.cues.reduced = cx.reduced_motion;
        self.clicks.reduced = cx.reduced_motion;
        self.ed.reduced = cx.reduced_motion;
        self.card.set_reduced(cx.reduced_motion);
        self.chip_sv.reduced = Some(cx.reduced_motion);
    }

    fn act(&mut self, cx: &mut Cx, a: Action) {
        let calls = self.m.on_action(a);
        self.calls.run(cx, calls);
        self.touch();
    }

    fn apply_settings(&mut self, v: &Value) {
        if let Some(b) = v.get("clock24").and_then(Value::as_bool) {
            self.m.card.set_clock24(b);
        }
        if let Some(b) = v.get("live.reminders").and_then(Value::as_bool) {
            self.m.card.set_live_on(b);
        } else if let Some(b) = v.get("live").and_then(|l| l.get("reminders")).and_then(Value::as_bool) {
            self.m.card.set_live_on(b);
        }
        self.touch();
    }

    fn mount(&mut self, cx: &mut Cx) {
        self.mounted = true;
        let (now, tz) = self.fixed_clock.unwrap_or_else(|| (unix_ms(), tz_offset_min()));
        self.m.card.set_now(now);
        self.m.card.set_tz_offset_min(tz);
        for ev in ["pill:hide", "pill:show", "pill:open-tab", "remind:due", "remind:changed", "timer:tick", "timer:done", "settings:changed"] {
            cx.subscribe(ev);
        }
        self.settings_t = Some(cx.call_async("settings_get", json!({})));
        let calls = self.m.mount();
        self.calls.run(cx, calls);
    }

    /// Everything the model asked for outside the backend, plus what the editor and the card produced since the last step.
    fn pump(&mut self, cx: &mut Cx) {
        for a in self.card.drain() {
            self.act(cx, Action::Timer(a));
        }
        let rest = route_effects(self.m.take_effects(), cx);
        for e in rest {
            match e {
                Effect::SetEditorHtml(h) => self.ed.set_html(&h),
                Effect::EditorExec(c) => self.ed.exec(c),
                Effect::Focus("note-editor") => self.ed.set_focused(true),
                Effect::ScrollIntoView("note-chip") => self.chip_to_show = true,
                _ => {}
            }
        }
        for o in self.ed.take_out() {
            match o {
                EdOut::Changed => {
                    let html = self.ed.html();
                    self.act(cx, Action::EditorChanged(html));
                }
                EdOut::Blur => self.act(cx, Action::Blur),
                EdOut::OpenLink(url) => {
                    let _ = cx.call_async("chat_open_link", json!({ "url": url }));
                }
                EdOut::Caret([bold, italic, strike, mark, ul, ol]) => self.act(cx, Action::CaretState { bold, italic, strike, mark, ul, ol }),
            }
        }
        for c in self.m.take_anims() {
            self.card.play(c.clone());
            self.cues.play(c);
        }
    }

    fn click(&mut self, cx: &mut Cx, k: K) {
        match k {
            K::Chip(id) => self.act(cx, Action::Open(id)),
            K::Add => self.act(cx, Action::New),
            K::Tool(i) => {
                let f = [Fmt::Bold, Fmt::Italic, Fmt::Strike, Fmt::Mark, Fmt::BulletList, Fmt::NumberList, Fmt::Checklist][i.min(6)];
                self.act(cx, Action::Format(f));
            }
            K::Remind => {
                let sel = self.ed.selected_text();
                let selection = (!sel.is_empty()).then_some(sel);
                self.act(cx, Action::RemindFromNote { selection });
                self.card.focus_reminder_input();
            }
            K::Copy => self.act(cx, Action::Copy),
            K::Del => self.act(cx, Action::Delete),
        }
    }
}

impl View for NoteScreen {
    fn on_show(&mut self, cx: &mut Cx) {
        self.sync_cx(cx);
        self.visible = true;
        if !self.mounted {
            self.mount(cx);
            let _ = self.m.on_show(); // marks the tab visible; mount() already asked for the card's data
        } else {
            self.m.card.set_now(self.fixed_clock.map_or_else(unix_ms, |c| c.0));
            let calls = self.m.on_show();
            self.calls.run(cx, calls);
        }
        cx.request_frame();
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.visible = false;
        self.ed.set_focused(false);
        let calls = self.m.on_hide();
        self.calls.run(cx, calls);
        self.pump(cx);
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Event { name, payload, .. } => {
                if name == "settings:changed" {
                    self.apply_settings(&payload);
                } else {
                    let calls = self.m.on_event(&name, &payload);
                    self.calls.run(cx, calls);
                    self.touch();
                }
            }
            Msg::Reply { ticket, result } => {
                if Some(ticket) == self.settings_t {
                    self.settings_t = None;
                    if let Ok(v) = result {
                        self.apply_settings(&v);
                    }
                } else if let Some((tag, reply)) = self.calls.take(ticket, result) {
                    let calls = self.m.on_reply(tag, reply);
                    self.calls.run(cx, calls);
                    self.touch();
                }
            }
            Msg::Open(v) => {
                let calls = self.m.on_event("pill:open-tab", &v);
                self.calls.run(cx, calls);
                self.touch();
            }
            Msg::Custom(_) => {}
        }
        self.pump(cx);
        Response::redraw()
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let mut r = Response::NONE;
        if let InputEvent::MouseDown { pos, button: MouseButton::Left, .. } = ev {
            // a click anywhere but the editor and the toolbar (those keep the editor's focus) takes the focus away: that saves, like `blur`
            if !self.ed_rect.contains(pos.0, pos.1) && !self.bar_rect.contains(pos.0, pos.1) && self.ed.is_focused() {
                self.ed.set_focused(false);
                r.redraw = true;
            }
        }
        let on_chips = matches!(ev, InputEvent::Wheel { pos, .. } if self.chips_rect.contains(pos.0, pos.1));
        if on_chips {
            if let InputEvent::Wheel { delta, precise, .. } = ev {
                self.chip_sv.wheel(if delta.0 != 0.0 { delta.0 } else { delta.1 }, *precise);
                return Response { consumed: true, redraw: true, ..r };
            }
        }
        if self.card.wants(ev) {
            r = r.merge(self.card.event(ev));
        } else if self.ed.wants(ev) && !matches!(ev, InputEvent::MouseLeave) {
            r = r.merge(self.ed.queue(ev));
        }
        if !matches!(ev, InputEvent::Key { .. } | InputEvent::Char(_)) && !self.card.wants(ev) {
            let over_editor = matches!(ev, InputEvent::MouseDown { pos, .. } if self.ed_rect.contains(pos.0, pos.1));
            if !over_editor {
                let (cr, click) = self.clicks.event(ev);
                r = r.merge(cr);
                if let Some(k) = click {
                    self.click(cx, k);
                }
            }
        }
        if matches!(ev, InputEvent::MouseLeave) {
            self.card.event(ev);
            self.ed.queue(ev);
        }
        if r.focus {
            // keyboard focus lives in the children (editor / card fields); the window keeps routing keys to them
            r.focus = false;
        }
        r
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.sync_cx(cx);
        let ms = ms_of(dt_ms, &mut self.carry);
        if self.mounted && ms > 0 {
            let calls = self.m.tick(ms);
            self.calls.run(cx, calls);
        }
        self.view_age += dt_ms;
        self.pump(cx);
        let dt = (dt_ms / 1000.0) as f32;
        let mut busy = self.clicks.step(dt);
        busy |= self.cues.step(dt_ms as f32);
        busy |= self.ed.step(dt_ms);
        busy |= self.card.step(dt_ms);
        busy |= self.chip_sv.step(dt);
        busy |= self.toast_t.step(dt);
        busy |= self.saved_t.step(dt);
        if let Some(r) = self.ed.ime_rect().filter(|_| self.ed.is_focused()) {
            cx.set_ime_rect(r);
        }
        // a running timer repaints the ring / clock a few times a second even when nothing else moves
        busy || (self.mounted && self.m.card.is_running() && self.visible)
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.rect = rect;
        if self.dirty_view || self.view_age >= VIEW_REFRESH_MS || self.view.is_none() {
            self.view = Some(self.m.view());
            self.view_age = 0.0;
            self.dirty_view = false;
        }
        let v = self.view.clone().unwrap_or_else(|| self.m.view());
        self.clicks.begin();
        let (left, right) = rect.cut_right(CARD_W);
        let left = Rect::new(left.x, left.y, (left.w - GAP).max(100.0), left.h);

        // ---- right: timer + reminders ----
        self.card.draw(cv, th, right, &v.timer, &self.ic, cx.now_ms);

        // ---- left: chips ----
        self.chips_rect = Rect::new(left.x, left.y, left.w, CHIP_H);
        self.draw_chips(cv, th, &v);

        // ---- left: toolbar ----
        self.bar_rect = Rect::new(left.x, left.bottom() - BAR_H, left.w, BAR_H);
        self.ed_rect = Rect::new(left.x, left.y + CHIP_H + 8.0, left.w, (self.bar_rect.y - 8.0 - (left.y + CHIP_H + 8.0)).max(40.0));

        // ---- left: editor ----
        self.ed.draw(cv, th, self.ed_rect, Some(&v.placeholder), cx);
        self.draw_toast(cv, th, &v);
        self.draw_bar(cv, th, &v);
    }
}

impl NoteScreen {
    fn draw_chips(&mut self, cv: &Canvas, th: &Theme, v: &NoteView) {
        let st = Style::new(11.0, 600);
        let r = self.chips_rect;
        // measure
        let mut items: Vec<(i64, String, f32, bool)> = Vec::new();
        let mut total = 0.0;
        for c in &v.chips {
            let w = (cv.text_width(&c.title, st).ceil() + 24.0).min(130.0);
            items.push((c.id, c.title.clone(), w, c.on));
            total += w + 6.0;
        }
        total += CHIP_H; // the + button
        self.chips_w = total;
        self.chip_sv.set_metrics(total, r.w);
        if std::mem::take(&mut self.chip_to_show) {
            let mut x = 0.0;
            for (_, _, w, on) in &items {
                if *on {
                    let off = self.chip_sv.target();
                    if x < off {
                        self.chip_sv.scroll_to(x, true);
                    } else if x + w > off + r.w {
                        self.chip_sv.scroll_to(x + w - r.w, true);
                    }
                }
                x += w + 6.0;
            }
        }
        let off = self.chip_sv.offset();
        cv.with_clip(r, |cv| {
            let mut x = r.x - off;
            for (id, title, w, on) in &items {
                let cr = Rect::new(x, r.y, *w, CHIP_H);
                let k = K::Chip(*id);
                if cr.right() > r.x && cr.x < r.right() {
                    // only the visible part is clickable
                    let vis = Rect::new(cr.x.max(r.x), cr.y, (cr.right().min(r.right()) - cr.x.max(r.x)).max(0.0), cr.h);
                    self.clicks.add(k.clone(), vis);
                }
                let hov = self.clicks.hover_amt(&k);
                let ps = self.clicks.press_scale(&k, 0.96);
                self.cues.apply(cv, &format!("note-chip:{id}"), cr.center(), |cv| {
                    cv.with_scale_about(cr.center(), ps, |cv| {
                        cv.fill_rr(cr, 12.0, if *on { th.raised2 } else { th.card.mix(th.raised, hov * 0.0) });
                        let fg = if *on { th.text } else { th.dim.mix(th.text, hov) };
                        cv.text(cr.inset_xy(12.0, 0.0), title, st.center().ellipsis(), fg);
                    });
                });
                x += w + 6.0;
            }
            let ar = Rect::new(x, r.y, CHIP_H, CHIP_H);
            if ar.right() > r.x && ar.x < r.right() {
                self.clicks.add(K::Add, Rect::new(ar.x.max(r.x), ar.y, (ar.right().min(r.right()) - ar.x.max(r.x)).max(0.0), ar.h));
            }
            let hov = self.clicks.hover_amt(&K::Add);
            let ps = self.clicks.press_scale(&K::Add, 0.92);
            cv.with_scale_about(ar.center(), ps, |cv| {
                cv.fill_circle(ar.center(), 12.0, th.card.mix(th.raised, hov));
                self.ic.draw(cv, "n-plus", ar.centered(14.0, 14.0), th.dim.mix(th.text, hov));
            });
        });
    }

    fn draw_toast(&mut self, cv: &Canvas, th: &Theme, v: &NoteView) {
        let want = v.toast.is_some();
        if let Some(t) = &v.toast {
            self.toast = t.clone();
        }
        let tgt = want as u8 as f32;
        if (self.toast_t.target() - tgt).abs() > 0.5 {
            if !self.cues.reduced {
                self.toast_t.retarget(tgt, 0.18, curves::EASE_OUT);
            } else {
                self.toast_t.jump(tgt);
            }
        }
        let k = self.toast_t.value().clamp(0.0, 1.0);
        if k <= 0.01 {
            return;
        }
        let st = Style::new(12.0, 600);
        let w = cv.text_width(&self.toast.0, st).ceil() + 24.0;
        let r = Rect::new(self.ed_rect.cx() - w / 2.0, self.ed_rect.bottom() - 12.0 - 30.0 + (1.0 - k) * 8.0, w, 30.0);
        cv.with_opacity(k, |cv| {
            cv.shadow_rr(r, 12.0, 10.0, 4.0, Color::rgba(0.0, 0.0, 0.0, 0.4));
            let (bg, fg) = if self.toast.1 { (Color::rgb8(0x5a, 0x1f, 0x24), Color::rgb8(0xff, 0xd9, 0xdc)) } else { (th.raised2, th.text) };
            cv.fill_rr(r, 12.0, bg);
            cv.text(r, &self.toast.0, st.center(), fg);
        });
    }

    fn draw_bar(&mut self, cv: &Canvas, th: &Theme, v: &NoteView) {
        let b = self.bar_rect;
        let tool = |i: usize| -> &str { ["", "", "", "n-mark", "n-ul", "n-ol", "n-todo"][i] };
        let mut x = b.x;
        for (i, t) in v.tools.iter().enumerate() {
            if i == 4 {
                // separator: 1 x 16, margin 0 6
                x += 6.0;
                cv.fill_rect(Rect::new(x, b.cy() - 8.0, 1.0, 16.0), th.line);
                x += 7.0;
            }
            let r = Rect::new(x, b.y, 28.0, 28.0);
            let k = K::Tool(i);
            self.clicks.add(k.clone(), r);
            let hov = self.clicks.hover_amt(&k);
            let ps = self.clicks.press_scale(&k, 0.94);
            let bg = if t.on { th.raised } else { th.hover.alpha(th.hover.a * hov) };
            let fg = if t.on { th.text } else { th.dim.mix(th.text, hov) };
            cv.with_scale_about(r.center(), ps, |cv| {
                cv.fill_rr(r, 8.0, bg);
                self.tool_glyph(cv, th, i, tool(i), r, fg);
            });
            x += 28.0;
        }
        // right side, laid out from the right edge: trash, copy, remind, saved
        let mut rx = b.right();
        let del = Rect::new(rx - 28.0, b.y, 28.0, 28.0);
        rx -= 28.0;
        let cpy = Rect::new(rx - 28.0, b.y, 28.0, 28.0);
        rx -= 28.0;
        let rst = Style::new(11.0, 600);
        let rw = 8.0 + 14.0 + 6.0 + cv.text_width(&v.remind_label, rst).ceil() + 8.0;
        let rem = Rect::new(rx - 2.0 - rw, b.y, rw, 28.0);
        rx -= rw + 4.0;
        let saved = Rect::new(rx - 24.0, b.y, 24.0, 28.0);

        // remind
        self.clicks.add(K::Remind, rem);
        let hov = self.clicks.hover_amt(&K::Remind);
        let ps = self.clicks.press_scale(&K::Remind, 0.96);
        cv.with_scale_about(rem.center(), ps, |cv| {
            cv.fill_rr(rem, 8.0, th.hover.alpha(th.hover.a * hov));
            let fg = th.text2.mix(th.text, hov);
            self.ic.draw(cv, "n-bell", Rect::new(rem.x + 8.0, rem.cy() - 7.0, 14.0, 14.0), fg);
            cv.text(Rect::new(rem.x + 8.0 + 20.0, rem.y, rem.w - 28.0, rem.h), &v.remind_label, rst, fg);
        });
        // copy
        self.draw_icon_tool(cv, th, K::Copy, cpy, "n-copy", false);
        // delete (armed = red wash)
        let dx = self.cues.xf("note-delete");
        cv.with_scale_about(del.center(), dx.scale, |cv| self.draw_icon_tool(cv, th, K::Del, del, "n-trash", v.delete_armed));
        // saved check: colour follows the state with a short cross-fade
        if v.saved != self.saved_now {
            self.saved_prev = self.saved_now;
            self.saved_now = v.saved;
            if self.cues.reduced {
                self.saved_t.jump(1.0);
            } else {
                self.saved_t = Tween::new(0.0, 1.0, 0.18, curves::EASE_OUT);
            }
        }
        let col = |s: SaveState| match s {
            SaveState::Saved => th.green,
            SaveState::Saving => th.dim.fade(0.6),
            SaveState::Error => th.red,
        };
        let c = col(self.saved_prev).mix(col(self.saved_now), self.saved_t.value().clamp(0.0, 1.0));
        self.ic.draw(cv, "n-check", saved.centered(16.0, 16.0), c);
    }

    fn draw_icon_tool(&mut self, cv: &Canvas, th: &Theme, k: K, r: Rect, icon: &str, armed: bool) {
        self.clicks.add(k.clone(), r);
        let hov = self.clicks.hover_amt(&k);
        let ps = self.clicks.press_scale(&k, 0.94);
        cv.with_scale_about(r.center(), ps, |cv| {
            let (bg, fg) = if armed { (th.red_bg, th.red) } else { (th.hover.alpha(th.hover.a * hov), th.dim.mix(th.text, hov)) };
            cv.fill_rr(r, 8.0, bg);
            self.ic.draw(cv, icon, r.centered(16.0, 16.0), fg);
        });
    }

    /// B / I / S are letters (like the web's `<b>B</b>`), the rest are icons.
    fn tool_glyph(&self, cv: &Canvas, _th: &Theme, i: usize, icon: &str, r: Rect, fg: Color) {
        let st = Style::new(13.0, 600).center();
        match i {
            0 => cv.text(r, "B", Style::new(13.0, 800).center(), fg),
            1 => {
                let k = 0.2;
                let m = Matrix3x2 { M11: 1.0, M12: 0.0, M21: -k, M22: 1.0, M31: k * r.cy(), M32: 0.0 };
                cv.with_transform(m, |cv| cv.text(r, "I", st, fg));
            }
            2 => {
                cv.text(r, "S", st, fg);
                cv.line((r.cx() - 4.5, r.cy()), (r.cx() + 4.5, r.cy()), 1.2, fg);
            }
            _ => self.ic.draw(cv, icon, r.centered(16.0, 16.0), fg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::Sim;
    use crate::ui::view::Services;
    use glasskit::input::{Buttons, KeyCode, Modifiers};

    pub const NOTES: &str = r#"[{"id":1,"title":"Список покупок"},{"id":2,"title":"Идеи для проекта"},{"id":3,"title":"Звонки"},{"id":4,"title":"Рецепт борща"}]"#;
    pub const BODY1: &str = "<b>Список покупок</b><div>молоко <i>2 литра</i></div><div><s>хлеб</s> <mark>купить свежий</mark></div><ul><li>яблоки</li><li>сыр &amp; масло</li></ul><ol><li>позвонить маме</li><li>оплатить счёт</li></ol><div><input type=\"checkbox\" checked=\"\">&nbsp;сделано</div><div><input type=\"checkbox\">&nbsp;ещё нет</div><div>сайт <a href=\"https://example.com\">example.com</a></div>";

    fn calls_named(f: &crate::ui::view::Fakes, n: &str) -> Vec<crate::ui::view::ApiCall> {
        f.api.calls().into_iter().filter(|c| c.name == n).collect()
    }

    fn sim() -> (Sim, crate::ui::view::Fakes) {
        let (svc, fakes) = Services::fake();
        fakes.api.reply("note_list", Ok(serde_json::from_str(NOTES).unwrap()));
        fakes.api.reply("note_get", Ok(json!({ "id": 1, "title": "Список покупок", "body_html": BODY1 })));
        fakes.api.reply("note_save", Ok(json!({ "id": 1, "title": "Список покупок" })));
        fakes.api.reply("timer_state", Ok(json!({ "active": false })));
        fakes.api.reply("remind_list", Ok(json!([])));
        fakes.api.reply("settings_get", Ok(json!({ "clock24": true })));
        let mut s = Sim::with_services(Box::new(NoteScreen::with_prefs(PrefsFile::memory())), svc);
        s.cx.lang = glasskit::lang::Lang::Ru;
        s.show();
        s.pump();
        s.pump();
        s.run(30);
        (s, fakes)
    }

    fn screen(s: &mut Sim) -> &mut NoteScreen {
        // the Sim owns the view as a trait object; the tests below keep a typed handle through a raw downcast-free helper
        let p: *mut dyn View = &mut *s.view;
        unsafe { &mut *(p as *mut NoteScreen) }
    }

    fn mv(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
    }
    fn down(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseDown { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 }
    }
    fn up(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseUp { pos: (x, y), button: MouseButton::Left, mods: Modifiers::NONE }
    }

    fn draw_frame(s: &mut Sim) {
        let th = Theme::pill_dark();
        let _ = s.render((648.0, 346.0), &th);
    }

    fn click(s: &mut Sim, x: f32, y: f32) {
        s.send(&mv(x, y));
        s.send(&down(x, y));
        draw_frame(s);
        s.send(&up(x, y));
        s.run(20);
        s.pump();
        s.run(20);
    }

    fn rect_of(s: &mut Sim, k: K) -> Rect {
        screen(s).clicks.rect_of(&k).expect("target drawn")
    }

    #[test]
    fn mount_loads_notes_timer_reminders_and_opens_the_first_note_without_saving() {
        let (mut s, f) = sim();
        draw_frame(&mut s);
        assert_eq!(calls_named(&f, "note_list").len(), 1);
        assert!(!calls_named(&f, "timer_state").is_empty(), "the timer is synced");
        assert_eq!(calls_named(&f, "remind_list").len(), 1);
        assert_eq!(calls_named(&f, "note_get").len(), 1);
        assert!(calls_named(&f, "note_save").is_empty(), "opening a note never saves");
        assert_eq!(screen(&mut s).editor().doc.lines[0].string(), "Список покупок");
        assert_eq!(screen(&mut s).model().current(), Some(1));
        // wander about without editing, then tick past the autosave debounce: still nothing is written
        s.send(&mv(100.0, 120.0));
        draw_frame(&mut s);
        s.send(&down(100.0, 120.0));
        draw_frame(&mut s);
        s.send(&up(100.0, 120.0));
        for _ in 0..600 {
            s.step_frame();
        }
        assert!(calls_named(&f, "note_save").is_empty());
    }

    #[test]
    fn typing_then_the_debounce_saves_the_html() {
        let (mut s, f) = sim();
        draw_frame(&mut s);
        s.send(&down(60.0, 60.0));
        draw_frame(&mut s);
        s.send(&up(60.0, 60.0));
        screen(&mut s).editor().set_selection(super::super::note_doc::Pos::new(0, 0), super::super::note_doc::Pos::new(0, 0));
        s.send(&InputEvent::Char('!'));
        draw_frame(&mut s);
        s.run(5);
        s.cx.now_ms += 1.0;
        // 400 ms of frames at 240 Hz
        for _ in 0..140 {
            s.step_frame();
            s.pump();
        }
        let saves = calls_named(&f, "note_save");
        assert_eq!(saves.len(), 1, "{saves:?}");
        let html = saves[0].args["body_html"].as_str().unwrap();
        assert!(html.starts_with("<b>!Список покупок</b>"), "{html}");
        assert_eq!(saves[0].args["title"], json!("!Список покупок"));
        assert_eq!(saves[0].args["id"], json!(1));
    }

    #[test]
    fn switching_notes_saves_the_dirty_one_first() {
        let (mut s, f) = sim();
        draw_frame(&mut s);
        screen(&mut s).editor().set_focused(true);
        s.send(&InputEvent::Char('x'));
        draw_frame(&mut s);
        s.run(3);
        draw_frame(&mut s);
        let r = rect_of(&mut s, K::Chip(2));
        click(&mut s, r.cx(), r.cy());
        let names: Vec<String> = f.api.calls().into_iter().map(|c| c.name).collect();
        let save = names.iter().position(|n| n == "note_save").expect("saved");
        let get = names.iter().rposition(|n| n == "note_get").unwrap();
        assert!(save < get, "{names:?}");
    }

    #[test]
    fn delete_needs_two_presses_and_the_toolbar_buttons_are_clickable() {
        let (mut s, f) = sim();
        draw_frame(&mut s);
        let r = rect_of(&mut s, K::Del);
        click(&mut s, r.cx(), r.cy());
        assert!(calls_named(&f, "note_delete").is_empty(), "first press only arms");
        draw_frame(&mut s);
        let r = rect_of(&mut s, K::Del);
        click(&mut s, r.cx(), r.cy());
        assert_eq!(calls_named(&f, "note_delete").len(), 1);
        // bold from the toolbar formats the selection and arms the autosave
        let (mut s, f) = sim();
        draw_frame(&mut s);
        screen(&mut s).editor().set_selection(super::super::note_doc::Pos::new(1, 0), super::super::note_doc::Pos::new(1, 5));
        let r = rect_of(&mut s, K::Tool(0));
        click(&mut s, r.cx(), r.cy());
        for _ in 0..140 {
            s.step_frame();
            s.pump();
        }
        let saves = calls_named(&f, "note_save");
        assert_eq!(saves.len(), 1);
        assert!(saves[0].args["body_html"].as_str().unwrap().contains("<div><b>молок</b>о <i>2 литра</i></div>"), "{}", saves[0].args["body_html"]);
    }

    #[test]
    fn prefs_file_round_trips_and_survives_a_missing_directory() {
        let dir = std::env::temp_dir().join(format!("pill-prefs-{}", std::process::id()));
        let path = dir.join("sub").join("ui-prefs.json");
        let mut p = PrefsFile::at(Some(path.clone()));
        assert_eq!(p.get("pill.note.cur"), None);
        p.set("pill.note.cur", "42");
        p.set("pill.timer.sel", "900");
        let q = PrefsFile::at(Some(path));
        assert_eq!(q.get("pill.note.cur").as_deref(), Some("42"));
        assert_eq!(q.get("pill.timer.sel").as_deref(), Some("900"));
        let _ = std::fs::remove_dir_all(dir);
        // an unwritable location is ignored
        let mut bad = PrefsFile::at(Some(PathBuf::from("Z:/definitely/not/here/x.json")));
        bad.set("a", "b");
        assert_eq!(bad.get("a").as_deref(), Some("b"));
    }

    #[test]
    fn idle_screen_asks_for_no_frames() {
        let (mut s, _f) = sim();
        draw_frame(&mut s);
        for _ in 0..400 {
            if !s.step_frame() {
                break;
            }
        }
        assert!(!s.frame.wants_frame(), "a settled note tab with no running timer idles");
    }

    #[test]
    fn keys_reach_the_focused_editor() {
        let (mut s, _f) = sim();
        draw_frame(&mut s);
        screen(&mut s).editor().set_focused(true);
        let r = s.send(&InputEvent::Key { code: KeyCode::Char('A'), mods: Modifiers::CTRL, down: true, repeat: false });
        assert!(r.consumed);
        draw_frame(&mut s);
        assert!(screen(&mut s).editor().selected_text().starts_with("Список покупок"));
    }

    // ---- snapshots over the same scenarios the web harness renders (tools/dev/fixtures/tabs-b) ----

    const DEFAULTS: &str = include_str!("../../../../../tools/dev/fixtures/tabs-b/_defaults.json");
    const TIMER_FIX: &str = include_str!("../../../../../tools/dev/fixtures/tabs-b/timer.json");

    /// The replies of the named scenarios (later names override earlier ones; `common` of the defaults applies first) and the fixture clock.
    fn scenario(names: &[&str]) -> (Vec<(String, Value)>, i64) {
        let d: Value = serde_json::from_str(DEFAULTS).unwrap();
        let t: Value = serde_json::from_str(TIMER_FIX).unwrap();
        let mut replies: Vec<(String, Value)> = Vec::new();
        let mut put = |m: &Value| {
            if let Some(o) = m.as_object() {
                for (k, v) in o {
                    replies.retain(|(n, _)| n != k);
                    replies.push((k.clone(), v.clone()));
                }
            }
        };
        put(&d["common"]["replies"]);
        for n in names {
            let sc = if d["scenarios"].get(*n).is_some() { &d["scenarios"][*n] } else { &t["scenarios"][*n] };
            assert!(!sc.is_null(), "no scenario {n}");
            put(&sc["replies"]);
        }
        (replies, d["now"].as_i64().unwrap())
    }

    fn shot_sim(names: &[&str], reduced: bool) -> Sim {
        let (replies, now) = scenario(names);
        let (svc, f) = Services::fake();
        for (k, v) in replies {
            f.api.reply(&k, Ok(v));
        }
        let mut sc = NoteScreen::with_prefs(PrefsFile::memory());
        sc.set_fixed_clock(now, 180);
        let mut s = Sim::with_services(Box::new(sc), svc);
        s.cx.lang = glasskit::lang::Lang::Ru;
        s.cx.reduced_motion = reduced;
        s.show();
        for _ in 0..4 {
            s.pump();
            s.run(60);
        }
        s
    }

    fn shoot(s: &mut Sim, name: &str, dark: bool, scale: f32) -> glasskit::testkit::Image {
        s.set_scale(scale);
        super::super::kit::snap::shoot(&mut *s.view, &s.cx, name, super::super::kit::snap::BODY_TALL, scale, dark)
    }

    #[test]
    fn snapshots_of_the_note_states() {
        for (name, names) in [("note-formatted", vec!["note-formatted"]), ("note-empty", vec!["note-empty"]), ("note-checklist", vec!["note-checklist"]), ("note-timer-running", vec!["note-formatted", "running-reminders"])] {
            let mut s = shot_sim(&names, true);
            for dark in [true, false] {
                for scale in [1.0, 2.0] {
                    let img = shoot(&mut s, name, dark, scale);
                    // the editor card, the chips and the right card all painted something
                    assert!(img.count(None, |p| p[0] != p[2] || p[0] > 20) > 2000, "{name} {dark} {scale}");
                }
            }
        }
    }

    #[test]
    fn snapshot_with_selection_and_a_focused_caret() {
        let mut s = shot_sim(&["note-formatted"], true);
        {
            let sc = screen(&mut s);
            sc.editor().set_focused(true);
            sc.editor().set_selection(super::super::note_doc::Pos::new(1, 0), super::super::note_doc::Pos::new(1, 11));
        }
        let _ = shoot(&mut s, "note-selection", true, 1.0);
        let _ = shoot(&mut s, "note-selection", false, 1.0);
    }

    #[test]
    fn animated_state_settles_and_idles() {
        let mut s = shot_sim(&["note-formatted"], false);
        let _ = shoot(&mut s, "note-animated-settled", true, 1.0);
        s.run(2000);
        let dt = 1000.0 / 240.0;
        let sc = screen(&mut s);
        let flags = (sc.clicks.step(0.004), sc.cues.step(dt as f32), sc.ed.step(dt), sc.card.step(dt), sc.chip_sv.step(0.004), sc.toast_t.step(0.004), sc.saved_t.step(0.004));
        assert!(!s.frame.wants_frame(), "settled: clicks, cues, editor, card, chips, toast, saved = {flags:?}");
    }
}

