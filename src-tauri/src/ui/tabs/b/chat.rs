//! The Chat tab view: messages (streaming markdown through glasskit's `MarkdownView`), composer, error / file banners, the history and AI-settings
//! popovers (`chat_pop.rs`), the empty and "AI is off" states.
//!
//! The view owns the ui-core `ChatModel` (which owns `AiSettings`): every user gesture becomes an `Action`, every backend call the model asks
//! for runs through `kit::Calls`, replies and the `chat:*` events feed the model back. Text input is applied at the start of the next draw
//! (`kit::Field`), messages are laid out in draw (they need a `Canvas`); see `kit.rs` for the pattern.
//!
//! Selection: drag inside one message (assistant answers through the markdown view, user bubbles through their text layout); Ctrl+C copies it,
//! a click on a code block's Copy button copies the code. Selection across messages is not supported (one message at a time).
use super::chat_pop::{HistPop, SetPop, K};
use super::kit::{self, Bk, Calls, Clicks, Cues, Field, Icons};
use crate::ui::view::{Cx, DropEvent, DropPayload, DropPhase, Msg, View};
use glasskit::canvas::Canvas;
use glasskit::controls::Widget;
use glasskit::ease::{curves, dur, Tween};
use glasskit::edit::EditEvent;
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::markdown;
use glasskit::richtext::view::{Hit, MarkdownLabels, MarkdownView, Selection, TextPos};
use glasskit::richtext::{RichLayout, RichText};
use glasskit::scroll::ScrollView;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::chat::{Action, ChatModel, ChatView, ErrView, PickedFile, Role, MAX_FILES, MAX_FILE_BYTES};
use pill_ui_core::panel::b::chat_settings::SAction;
use pill_ui_core::panel::b::Tags;
use std::path::PathBuf;

const SCROLL_TOP: f32 = 28.0;
const MSG_GAP: f32 = 14.0;
const AV: f32 = 28.0;
const COL_X: f32 = AV + 12.0;
const COMPOSER_MIN: f32 = 28.0;
const COMPOSER_MAX: f32 = 96.0;

// ---------------------------------------------------------------------------------------------------------------------------------------
// Icons of tab-chat.js (inner markup)
// ---------------------------------------------------------------------------------------------------------------------------------------

fn icons() -> Icons {
    let mut ic = Icons::new();
    ic.add("c-sparkles", r##"<path d="M9.5 3.5 11 8.2l4.700 1.500L11 11.200 9.500 16 8 11.200 3.300 9.700 8 8.200z"/><path d="M18 14l.8 2.200 2.200.8-2.200.8L18 20l-.8-2.200-2.200-.8 2.200-.8zM18.500 3.500l.5 1.300 1.300.5-1.300.5-.5 1.300-.5-1.300-1.300-.5 1.300-.5z"/>"##);
    ic.add("c-attach", r##"<path d="M20.500 11.500 12 20a5.500 5.500 0 0 1-7.800-7.800l8.800-8.800a3.700 3.700 0 0 1 5.200 5.200L9.700 17.300a1.800 1.800 0 0 1-2.600-2.600l8-8"/>"##);
    ic.add("c-mic", r##"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.500 11.500a6.500 6.500 0 0 0 13 0M12 18v3"/>"##);
    ic.add("c-send", r##"<path d="M12 19V5M6 11l6-6 6 6"/>"##);
    ic.add("c-stop", r##"<rect x="7" y="7" width="10" height="10" rx="2.200" fill="currentColor" stroke="none"/>"##);
    ic.add("c-compose", r##"<path d="M12 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-6"/><path d="M18.400 2.600a1.900 1.900 0 0 1 2.700 2.700L12 14.400 8 15.500l1.100-4z"/>"##);
    ic.add("c-copy", r##"<rect x="9" y="9" width="11" height="11" rx="2.500"/><path d="M5 15V6a2 2 0 0 1 2-2h8"/>"##);
    ic.add("c-check", r##"<path d="m5 12.500 4.500 4.500L19 7.500"/>"##);
    ic.add("c-retry", r##"<path d="M3.500 12a8.500 8.500 0 1 0 2.800-6.300L3.500 8.500"/><path d="M3.500 3.500v5h5"/>"##);
    ic.add("c-history", r##"<path d="M3.500 12a8.500 8.500 0 1 0 2.800-6.300L3.500 8.500"/><path d="M3.500 3.500v5h5M12 7.500V12l3 2"/>"##);
    ic.add("c-gear", r##"<rect x="7" y="7" width="10" height="10" rx="2.500"/><path d="M9.500 3v2.500M14.500 3v2.500M9.500 18.500V21M14.500 18.500V21M3 9.500h2.500M3 14.500h2.500M18.500 9.500H21M18.500 14.500H21"/>"##);
    ic
}

/// The minutes east of UTC of the local time zone (what `wall(ms, tz)` of the model wants).
#[cfg(windows)]
fn local_tz_min() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let dst = if id == 2 { tz.DaylightBias } else { tz.StandardBias };
    -(tz.Bias + dst)
}
#[cfg(not(windows))]
fn local_tz_min() -> i32 {
    0
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------------------------------------------------

struct Bubble {
    lay: RichLayout,
    w: f32,
}

struct Item {
    id: i64,
    role: Role,
    text: String,
    idx: usize,
    streaming: bool,
    typing: bool,
    actions: bool,
    copied: bool,
    md: Option<MarkdownView>,
    md_dark: bool,
    bubble: Option<Bubble>,
    y: f32,
    h: f32,
}

/// The selection of a user bubble (UTF-16 range).
#[derive(Clone, Copy)]
struct USel {
    item: usize,
    a: usize,
    b: usize,
}

enum Drag {
    Md { item: usize, anchor: TextPos, link: Option<String> },
    User { item: usize, anchor: usize },
}

#[derive(Default, Clone, Copy)]
struct Geo {
    area: Rect,
    viewport: Rect,
    row: Rect,
    box_: Rect,
    ta: Rect,
    err: Rect,
    files: Rect,
}

/// True for the answer of the file picker (the island routes it to this tab).
pub(crate) fn is_picked(b: &(dyn std::any::Any + Send)) -> bool {
    b.is::<Picked>()
}

struct Picked(Vec<PathBuf>);

pub struct ChatScreen {
    m: ChatModel,
    calls: Calls,
    cues: Cues,
    clicks: Clicks<K>,
    ic: Icons,
    comp: Field,
    url: Field,
    mdl: Field,
    key: Field,
    hist: HistPop,
    set: SetPop,
    view: Option<ChatView>,
    dirty: bool,
    items: Vec<Item>,
    ghost: Option<(MarkdownView, f32)>,
    sv: ScrollView,
    geo: Geo,
    mounted: bool,
    carry: f64,
    scroll_bottom: bool,
    scroll_jump: bool,
    usel: Option<USel>,
    drag: Option<Drag>,
    last_pushed: String,
    recalling: bool,
    comp_enter: bool,
    ime: Option<Rect>,
    err_k: Tween<f32>,
    err_last: Option<ErrView>,
    files_k: Tween<f32>,
    files_last: Vec<String>,
    off_k: Tween<f32>,
    tools_k: Tween<f32>,
    send_k: Tween<f32>,
    now_ms: f64,
    scale: f32,
    pick_busy: bool,
    sent_cfg_fields: bool,
    focus_next: bool,
    seen_urls: (String, String),
}

impl Default for ChatScreen {
    fn default() -> Self {
        ChatScreen::new()
    }
}

impl ChatScreen {
    pub fn new() -> ChatScreen {
        let th = Theme::pill_dark();
        let mut comp = Field::new(true, &th);
        comp.ti.pad = (4.0, 5.5);
        let single = || Field::new(false, &th);
        ChatScreen {
            m: ChatModel::new(pill_ui_core::panel::b::Lang::En),
            calls: Calls::new(),
            cues: Cues::new(),
            clicks: Clicks::new(),
            ic: icons(),
            comp,
            url: single(),
            mdl: single(),
            key: single(),
            hist: HistPop::new(),
            set: SetPop::new(),
            view: None,
            dirty: true,
            items: Vec::new(),
            ghost: None,
            sv: ScrollView::new(),
            geo: Geo::default(),
            mounted: false,
            carry: 0.0,
            scroll_bottom: false,
            scroll_jump: false,
            usel: None,
            drag: None,
            last_pushed: String::new(),
            recalling: false,
            comp_enter: false,
            ime: None,
            err_k: Tween::at_rest(0.0),
            err_last: None,
            files_k: Tween::at_rest(0.0),
            files_last: Vec::new(),
            off_k: Tween::at_rest(0.0),
            tools_k: Tween::at_rest(0.7),
            send_k: Tween::at_rest(0.0),
            now_ms: 0.0,
            scale: 1.0,
            pick_busy: false,
            sent_cfg_fields: false,
            focus_next: false,
            seen_urls: (String::new(), String::new()),
        }
    }

    // ------------------------------------------------------------------------------------------------------------------ model plumbing

    fn run(&mut self, a: Action, cx: &mut Cx) {
        let calls = self.m.on_action(a);
        self.calls.run(cx, calls);
        self.after_model(cx);
    }

    fn after_model(&mut self, cx: &mut Cx) {
        self.drain(cx);
        self.dirty = true;
        cx.request_frame();
    }

    /// Effects and cues the model queued (also after a tick: the reveal asks for `ScrollToBottom`).
    fn drain(&mut self, cx: &mut Cx) {
        for e in kit::route_effects(self.m.take_effects(), cx) {
            use pill_ui_core::panel::b::Effect as E;
            match e {
                E::ScrollToBottom => {
                    self.scroll_bottom = true;
                    self.scroll_jump |= !self.m.is_streaming();
                }
                E::Focus(_) => self.focus_next = true,
                _ => {}
            }
        }
        self.cues.reduced = cx.reduced_motion;
        self.clicks.reduced = cx.reduced_motion;
        self.cues.play_all(self.m.take_anims());
    }

    fn refresh_view(&mut self, cx: &Cx) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let v = self.m.view();
        // the model changed the input itself (history recall, a dictation, cleared after a send): mirror it into the field
        if v.input != self.last_pushed {
            self.comp.ti.set_text(&v.input);
            self.last_pushed = v.input.clone();
        }
        self.hist.sync(&v.history, cx.reduced_motion);
        self.set.sync(&v.settings, cx.reduced_motion);
        let target = |on: bool| on as u8 as f32;
        if let Some(e) = &v.error {
            self.err_last = Some(e.clone());
        }
        self.retarget(0, target(v.error.is_some()), cx.reduced_motion);
        if !v.files.is_empty() {
            self.files_last = v.files.clone();
        }
        self.retarget(1, target(!v.files.is_empty()), cx.reduced_motion);
        self.retarget(2, target(v.off.is_some()), cx.reduced_motion);
        self.retarget(4, target(v.busy), cx.reduced_motion);
        // settings fields follow the saved values unless being edited
        if !self.url.is_focused() && self.url.ti.text() != v.settings.url {
            self.url.ti.set_text(&v.settings.url);
        }
        if !self.mdl.is_focused() && self.mdl.ti.text() != v.settings.model {
            self.mdl.ti.set_text(&v.settings.model);
        }
        self.view = Some(v);
    }

    fn retarget(&mut self, which: u8, to: f32, reduced: bool) {
        let t = match which {
            0 => &mut self.err_k,
            1 => &mut self.files_k,
            2 => &mut self.off_k,
            _ => &mut self.send_k,
        };
        if t.target() != to {
            if reduced {
                t.jump(to);
            } else {
                t.retarget(to, dur::MED, curves::EASE_OUT);
            }
        }
    }

    fn field_mut(&mut self, i: usize) -> &mut Field {
        match i {
            0 => &mut self.comp,
            1 => &mut self.url,
            2 => &mut self.mdl,
            _ => &mut self.key,
        }
    }

    fn focus_field(&mut self, which: Option<usize>) {
        for i in 0..4 {
            let on = Some(i) == which;
            let f = self.field_mut(i);
            if f.ti.is_focused() != on {
                f.ti.set_focused(on);
            }
        }
    }

    fn focused_field(&self) -> Option<usize> {
        [&self.comp, &self.url, &self.mdl, &self.key].iter().position(|f| f.is_focused())
    }

    fn open_picker(&mut self, cx: &mut Cx) {
        if self.pick_busy {
            return;
        }
        self.pick_busy = true;
        let w = cx.waker();
        std::thread::spawn(move || w.post_custom(Picked(pick_files())));
    }

    fn attach(&mut self, paths: Vec<PathBuf>, cx: &mut Cx) {
        let files: Vec<PickedFile> = paths.into_iter().take(MAX_FILES).filter_map(|p| read_picked(&p)).collect();
        if !files.is_empty() {
            self.run(Action::Attach(files), cx);
        }
    }

    // ------------------------------------------------------------------------------------------------------------------ click routing

    fn clicked(&mut self, k: K, cx: &mut Cx) {
        let v = match &self.view {
            Some(v) => v,
            None => return,
        };
        let idea = |i: usize| v.empty.as_ref().and_then(|e| e.ideas.get(i).cloned());
        match k {
            K::Compose | K::HNew => self.run(Action::NewChat, cx),
            K::Attach => self.open_picker(cx),
            K::Mic => self.run(Action::Mic, cx),
            K::Send => {
                // the model sends what it was last told is in the composer: flush ours first
                let t = self.comp.ti.text();
                self.push_input(t);
                self.run(Action::SendButton, cx);
            }
            K::Hist => self.run(Action::OpenHistory, cx),
            K::Gear => self.run(Action::OpenSettings, cx),
            K::Idea(i) => {
                if let Some(t) = idea(i) {
                    self.run(Action::SendText(t), cx);
                }
            }
            K::CopyMsg(i) => self.run(Action::CopyMessage(i), cx),
            K::Retry | K::ErrRetry => self.run(Action::Retry, cx),
            K::ErrSettings => self.run(Action::OpenSettings, cx),
            K::ErrX => self.run(Action::DismissError, cx),
            K::FileX(i) => self.run(Action::RemoveFile(i), cx),
            K::Enable => self.run(Action::EnableAi, cx),
            K::HRow(id) => self.run(Action::PickThread(id), cx),
            K::HDel(id) => self.run(Action::DeleteThread(id), cx),
            K::SClose => self.run(Action::Settings(SAction::Close), cx),
            K::SPreset(i) => {
                if let Some(p) = v.settings.presets.get(i) {
                    let id = p.id.clone();
                    self.run(Action::Settings(SAction::Preset(id)), cx);
                }
            }
            K::SScan => self.run(Action::Settings(SAction::Scan), cx),
            K::SSaveKey => {
                let t = self.key.ti.text();
                self.key.ti.set_text("");
                self.run(Action::Settings(SAction::SaveKey(t)), cx);
            }
            K::SRmKey => self.run(Action::Settings(SAction::RemoveKey), cx),
            K::STest => {
                let (url, model) = (self.url.ti.text(), self.mdl_text());
                self.run(Action::Settings(SAction::Test { url, model }), cx);
            }
            K::SOff => self.run(Action::Settings(SAction::TurnOff), cx),
            K::HBody | K::SBody | K::SModel => {}
        }
    }

    fn mdl_text(&self) -> String {
        match (&self.view, self.set.show_models()) {
            (Some(v), true) => v.settings.model.clone(),
            _ => self.mdl.ti.text(),
        }
    }

    fn push_input(&mut self, text: String) {
        if text != self.last_pushed {
            self.last_pushed = text.clone();
            let calls = self.m.on_action(Action::Input(text));
            debug_assert!(calls.is_empty());
        }
    }

    fn settings_edit_events(&mut self, cx: &mut Cx) {
        for e in self.url.take_events() {
            match e {
                EditEvent::Submit | EditEvent::FocusChanged(false) => {
                    let t = self.url.ti.text();
                    if self.view.as_ref().is_some_and(|v| v.settings.url != t.trim()) {
                        self.run(Action::Settings(SAction::UrlChanged(t)), cx);
                    }
                }
                _ => {}
            }
        }
        for e in self.mdl.take_events() {
            match e {
                EditEvent::Submit | EditEvent::FocusChanged(false) => {
                    let t = self.mdl.ti.text();
                    if self.view.as_ref().is_some_and(|v| v.settings.model != t.trim()) {
                        self.run(Action::Settings(SAction::ModelChanged(t)), cx);
                    }
                }
                _ => {}
            }
        }
        for e in self.key.take_events() {
            if e == EditEvent::Submit {
                let t = self.key.ti.text();
                if !t.trim().is_empty() {
                    self.key.ti.set_text("");
                    self.run(Action::Settings(SAction::SaveKey(t)), cx);
                }
            }
        }
    }

    // ------------------------------------------------------------------------------------------------------------------ messages area

    fn content_pos(&self, p: Pt) -> Pt {
        (p.0 - self.geo.viewport.x, p.1 - self.geo.viewport.y + self.sv.offset())
    }

    fn item_at(&self, y: f32) -> Option<usize> {
        self.items.iter().position(|it| y >= it.y && y < it.y + it.h + MSG_GAP * 0.5)
    }

    fn clear_selection(&mut self) {
        self.usel = None;
        for it in &mut self.items {
            if let Some(md) = &mut it.md {
                md.set_selection(None);
            }
        }
    }

    fn selection_text(&self) -> String {
        if let Some(u) = self.usel {
            if let Some(b) = self.items.get(u.item).and_then(|i| i.bubble.as_ref()) {
                return b.lay.text_range(u.a.min(u.b)..u.a.max(u.b));
            }
        }
        self.items.iter().filter_map(|i| i.md.as_ref()).map(|m| m.selection_text()).find(|t| !t.is_empty()).unwrap_or_default()
    }

    fn msg_mouse(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let vp = self.geo.viewport;
        match ev {
            InputEvent::MouseDown { pos, button: MouseButton::Left, clicks, .. } if vp.contains(pos.0, pos.1) => {
                self.focus_field(None);
                let cp = self.content_pos(*pos);
                self.clear_selection();
                let Some(i) = self.item_at(cp.1) else { return Response::consumed() };
                let sv_w = self.sv_width();
                let it = &mut self.items[i];
                if let (Some(md), true) = (it.md.as_mut(), cp.0 >= COL_X) {
                    let local = (cp.0 - COL_X, cp.1 - it.y);
                    match md.hit(local) {
                        Hit::CopyButton(leaf) => {
                            let code = md.leaf_text(leaf);
                            md.mark_copied(leaf);
                            cx.set_clipboard_text(&code);
                            return Response { consumed: true, redraw: true, ..Response::NONE };
                        }
                        Hit::Link(u) => {
                            self.drag = Some(Drag::Md { item: i, anchor: TextPos { leaf: 0, index: 0 }, link: Some(u) });
                            return Response { consumed: true, capture: true, ..Response::NONE };
                        }
                        Hit::Text(p) => {
                            if *clicks >= 3 {
                                md.select_all();
                            } else {
                                md.set_selection(Some(Selection { anchor: p, focus: p }));
                                self.drag = Some(Drag::Md { item: i, anchor: p, link: None });
                            }
                            return Response { consumed: true, redraw: true, capture: true, cursor: Some(Cursor::Text), ..Response::NONE };
                        }
                        Hit::None => {}
                    }
                }
                if let Some(b) = it.bubble.as_ref() {
                    let bx = sv_w - b.w;
                    if cp.0 >= bx && cp.0 <= bx + b.w {
                        let idx = b.lay.index_at((cp.0 - bx - 16.0, cp.1 - it.y - 9.0));
                        if *clicks >= 3 {
                            self.usel = Some(USel { item: i, a: 0, b: b.lay.len() });
                        } else {
                            self.usel = Some(USel { item: i, a: idx, b: idx });
                            self.drag = Some(Drag::User { item: i, anchor: idx });
                        }
                        return Response { consumed: true, redraw: true, capture: true, cursor: Some(Cursor::Text), ..Response::NONE };
                    }
                }
                Response::consumed()
            }
            InputEvent::MouseMove { pos, .. } => {
                let cp = self.content_pos(*pos);
                match &self.drag {
                    Some(Drag::Md { item, anchor, link: None }) => {
                        let (item, anchor) = (*item, *anchor);
                        if let Some(it) = self.items.get_mut(item) {
                            if let Some(md) = it.md.as_mut() {
                                if let Some(f) = md.text_pos_at((cp.0 - COL_X, cp.1 - it.y)) {
                                    md.set_selection(Some(Selection { anchor, focus: f }));
                                }
                            }
                        }
                        Response { consumed: true, redraw: true, capture: true, cursor: Some(Cursor::Text), ..Response::NONE }
                    }
                    Some(Drag::Md { .. }) => Response { consumed: true, capture: true, ..Response::NONE },
                    Some(Drag::User { item, anchor }) => {
                        let (item, anchor) = (*item, *anchor);
                        let bw = self.items.get(item).and_then(|i| i.bubble.as_ref()).map(|b| b.w).unwrap_or(0.0);
                        let bx = self.sv_width() - bw;
                        if let Some(it) = self.items.get(item) {
                            if let Some(b) = &it.bubble {
                                let idx = b.lay.index_at((cp.0 - bx - 16.0, cp.1 - it.y - 9.0));
                                self.usel = Some(USel { item, a: anchor, b: idx });
                            }
                        }
                        Response { consumed: true, redraw: true, capture: true, cursor: Some(Cursor::Text), ..Response::NONE }
                    }
                    None => {
                        if !vp.contains(pos.0, pos.1) {
                            return self.set_md_hover(&Hit::None);
                        }
                        let mut hit = Hit::None;
                        if let Some(i) = self.item_at(cp.1) {
                            let it = &self.items[i];
                            if let (Some(md), true) = (it.md.as_ref(), cp.0 >= COL_X) {
                                hit = md.hit((cp.0 - COL_X, cp.1 - it.y));
                            }
                            if let Some(b) = &it.bubble {
                                if cp.0 >= self.sv_width() - b.w && cp.1 >= it.y && cp.1 <= it.y + it.h {
                                    return Response { cursor: Some(Cursor::Text), ..Response::NONE };
                                }
                            }
                        }
                        let mut r = self.set_md_hover(&hit);
                        r.cursor = match hit {
                            Hit::CopyButton(_) | Hit::Link(_) => Some(Cursor::Hand),
                            Hit::Text(_) => Some(Cursor::Text),
                            Hit::None => None,
                        };
                        r
                    }
                }
            }
            InputEvent::MouseUp { pos, button: MouseButton::Left, .. } => {
                let Some(d) = self.drag.take() else { return Response::NONE };
                if let Drag::Md { item, link: Some(u), .. } = d {
                    // a click on a link (not a drag away from it) opens it; the markdown view only exposes safe urls
                    let cp = self.content_pos(*pos);
                    if let Some(it) = self.items.get(item) {
                        if let Some(md) = &it.md {
                            if md.hit((cp.0 - COL_X, cp.1 - it.y)) == Hit::Link(u.clone()) && markdown::is_safe_url(&u) {
                                self.run(Action::OpenLink(u), cx);
                            }
                        }
                    }
                }
                Response { consumed: true, redraw: true, ..Response::NONE }
            }
            _ => Response::NONE,
        }
    }

    fn set_md_hover(&mut self, hit: &Hit) -> Response {
        let mut ch = false;
        for it in &mut self.items {
            if let Some(md) = &mut it.md {
                ch |= md.set_hover(hit);
            }
        }
        Response { redraw: ch, ..Response::NONE }
    }

    fn sv_width(&self) -> f32 {
        (self.geo.viewport.w - 10.0).max(0.0)
    }

    fn report_scroll(&mut self) {
        let s = &self.sv.state;
        let a = Action::Scrolled { offset: s.target(), viewport: s.viewport(), content: s.content() };
        let _ = self.m.on_action(a);
    }

    fn geo_of(&self, area: Rect) -> (f32, f32) {
        // (bottom area height, composer text height); the row itself is laid out in `layout`
        let _ = area;
        (0.0, COMPOSER_MIN)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// File helpers
// ---------------------------------------------------------------------------------------------------------------------------------------

fn read_picked(p: &std::path::Path) -> Option<PickedFile> {
    let name = p.file_name()?.to_string_lossy().to_string();
    let meta = std::fs::metadata(p).ok()?;
    let size = meta.len() as usize;
    if size > MAX_FILE_BYTES {
        // the model shows the "over 200 KB" banner from the size alone
        return Some(PickedFile { name, size, text: String::new() });
    }
    let bytes = std::fs::read(p).ok()?;
    // a NUL byte makes the model refuse the file ("text files only")
    let text = String::from_utf8_lossy(&bytes).to_string();
    Some(PickedFile { name, size, text })
}

/// The Windows "Open" dialog (multi-select) on the calling worker thread; empty when cancelled.
#[cfg(windows)]
fn pick_files() -> Vec<PathBuf> {
    use windows::core::PWSTR;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_ALLOWMULTISELECT, FOS_FORCEFILESYSTEM, SIGDN_FILESYSPATH};
    let mut out = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if let Ok(dlg) = CoCreateInstance::<_, IFileOpenDialog>(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) {
            let _ = dlg.SetOptions(FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM);
            if dlg.Show(None).is_ok() {
                if let Ok(items) = dlg.GetResults() {
                    for i in 0..items.GetCount().unwrap_or(0) {
                        if let Ok(it) = items.GetItemAt(i) {
                            if let Ok(name) = it.GetDisplayName(SIGDN_FILESYSPATH) {
                                let s: PWSTR = name;
                                if let Ok(p) = s.to_string() {
                                    out.push(PathBuf::from(p));
                                }
                                CoTaskMemFree(Some(s.0 as *const _));
                            }
                        }
                    }
                }
            }
        }
        CoUninitialize();
    }
    out
}
#[cfg(not(windows))]
fn pick_files() -> Vec<PathBuf> {
    Vec::new()
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------------------------------------------------------------------

impl View for ChatScreen {
    fn on_show(&mut self, cx: &mut Cx) {
        self.m.set_lang(kit::lang_of(cx));
        self.m.set_tz_offset_min(local_tz_min());
        self.m.set_reduce_motion(cx.reduced_motion);
        if !self.mounted {
            self.mounted = true;
            for ev in ["chat:delta", "chat:done", "settings:changed", "voice:state", "voice:done"] {
                cx.subscribe(ev);
            }
        }
        let calls = self.m.on_show();
        self.calls.run(cx, calls);
        self.after_model(cx);
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        match msg {
            Msg::Event { name, payload, .. } => {
                let calls = self.m.on_event(&name, &payload);
                self.calls.run(cx, calls);
                self.after_model(cx);
            }
            Msg::Reply { ticket, result } => {
                if let Some((tag, reply)) = self.calls.take(ticket, result) {
                    let calls = self.m.on_reply(tag, reply);
                    self.calls.run(cx, calls);
                    self.after_model(cx);
                    // keep the owner byte honest for tests: tags of the two models differ
                    let _ = Tags::owner_of(tag);
                }
            }
            Msg::Open(_) => {
                self.run(Action::OpenSettings, cx);
            }
            Msg::Custom(b) => {
                if let Ok(p) = b.downcast::<Picked>() {
                    self.pick_busy = false;
                    self.attach(p.0, cx);
                }
            }
        }
        Response::redraw()
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.run(Action::ClickOutside, cx);
        self.focus_field(None);
    }

    fn on_drop(&mut self, ev: &DropEvent, cx: &mut Cx) -> Response {
        match (&ev.phase, &ev.payload) {
            (DropPhase::Enter | DropPhase::Over, _) => Response::consumed(),
            (DropPhase::Drop, Some(DropPayload::Files(f))) => {
                self.attach(f.clone(), cx);
                Response::consumed()
            }
            _ => Response::NONE,
        }
    }

    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.refresh_view(cx);
        let Some(v) = self.view.clone() else { return Response::NONE };
        let set_open = self.set.is_open();
        let hist_open = v.history.open;
        let off = v.off.is_some();
        // ---- keyboard
        if let InputEvent::Key { code, mods, down: true, .. } = ev {
            if *code == KeyCode::Escape {
                if self.set.select.is_open() {
                    return self.set.select.event(ev, Rect::default());
                }
                if set_open {
                    self.run(Action::Settings(SAction::Close), cx);
                } else if hist_open {
                    self.run(Action::ClickOutside, cx);
                } else if v.busy {
                    self.run(Action::Stop, cx);
                } else {
                    return Response::NONE;
                }
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
            if mods.ctrl && matches!(code, KeyCode::Char('C')) {
                let comp_sel = self.comp.is_focused() && self.comp.ti.selection().0 != self.comp.ti.selection().1;
                let sel = self.selection_text();
                if !sel.is_empty() && !comp_sel && self.focused_field().is_none() {
                    cx.set_clipboard_text(&sel);
                    return Response { consumed: true, ..Response::NONE };
                }
            }
            if self.comp.is_focused() && !set_open {
                match code {
                    KeyCode::Enter if !mods.shift => {
                        self.comp_enter = true;
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                    KeyCode::Up if self.comp.ti.text().is_empty() || self.recalling => {
                        self.recalling = true;
                        self.run(Action::HistoryUp, cx);
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                    KeyCode::Down if self.recalling => {
                        self.run(Action::HistoryDown, cx);
                        return Response { consumed: true, redraw: true, ..Response::NONE };
                    }
                    _ => {}
                }
            }
        }
        if matches!(ev, InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_)) {
            if let Some(i) = self.focused_field() {
                if matches!(ev, InputEvent::Char(_)) || matches!(ev, InputEvent::Key { code, mods, .. } if !(matches!(code, KeyCode::Char(_)) && mods.ctrl && matches!(code, KeyCode::Char('V') | KeyCode::Char('C') | KeyCode::Char('X') | KeyCode::Char('A') | KeyCode::Char('Z') | KeyCode::Char('Y')) && false)) {
                    if matches!(ev, InputEvent::Char(_) | InputEvent::Key { .. }) && i == 0 {
                        self.recalling &= matches!(ev, InputEvent::Key { .. });
                    }
                    return self.field_mut(i).queue(ev);
                }
            }
            if let (false, InputEvent::Key { .. }) = (self.set.select.is_open(), ev) {
                // arrows / page keys scroll the messages when no field has the keyboard
                let r = self.sv.event(ev, self.geo.viewport);
                if r.consumed {
                    self.report_scroll();
                    return r;
                }
            }
            return Response::NONE;
        }
        // ---- the model dropdown owns the pointer while open
        if self.set.select.is_open() || self.set.select.popup_visible() {
            if matches!(ev, InputEvent::MouseMove { .. } | InputEvent::MouseDown { .. } | InputEvent::MouseUp { .. }) {
                let r = self.set.select.event(ev, self.set.model_rect);
                if r.changed {
                    if let Some(l) = self.set.select.selected_label().map(str::to_string) {
                        self.run(Action::Settings(SAction::ModelChanged(l)), cx);
                    }
                }
                if r.consumed || r.redraw {
                    return Response { redraw: true, ..r };
                }
            }
        }
        // ---- pointer
        let pos = ev.pos();
        if let (Some(p), InputEvent::MouseDown { .. }) = (pos, ev) {
            // a press outside the history popover and the tools row closes it (and still reaches what is under it)
            let on_tools = self.clicks.hit(p).is_some_and(|k| matches!(k, K::Hist | K::Gear | K::HBody | K::HNew | K::HRow(_) | K::HDel(_)));
            if hist_open && !on_tools {
                self.run(Action::ClickOutside, cx);
            }
            if set_open || self.set.visible() {
                // fields of the settings panel
            }
        }
        if let InputEvent::Wheel { pos, .. } = ev {
            if hist_open && self.hist.rect().contains(pos.0, pos.1) {
                return self.hist.wheel(ev);
            }
            if set_open {
                return self.set.scroll.event(ev, self.set.rect().inset_xy(0.0, 1.0));
            }
            if self.comp.rect.contains(pos.0, pos.1) && self.comp.wants(ev) {
                let r = self.comp.queue(ev);
                if r.consumed {
                    return r;
                }
            }
            let r = self.sv.event(ev, self.geo.viewport);
            if r.consumed {
                self.report_scroll();
            }
            return r;
        }
        // scrollbar of the messages (drag) before everything else in its zone
        if !set_open && !off {
            let r = self.sv.bar.event(ev, self.geo.viewport, &mut self.sv.state);
            if r.consumed || r.capture {
                self.report_scroll();
                return r;
            }
        }
        if set_open {
            if let Some(p) = pos {
                // settings fields take the pointer first
                for (i, rect) in [(1usize, self.url.rect), (2, self.mdl.rect), (3, self.key.rect)] {
                    let f = self.field_mut(i);
                    let busy = f.wants(ev) || (matches!(ev, InputEvent::MouseDown { .. }) && rect.contains(p.0, p.1));
                    if busy && !(i == 2 && self.set.show_models()) {
                        if matches!(ev, InputEvent::MouseDown { .. }) {
                            self.focus_field(Some(i));
                        }
                        return self.field_mut(i).queue(ev);
                    }
                }
                if matches!(ev, InputEvent::MouseDown { .. }) {
                    self.focus_field(None);
                }
            }
        } else if let Some(p) = pos {
            if !off && self.comp.wants(ev) && (self.comp.rect.contains(p.0, p.1) || matches!(ev, InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. })) {
                if matches!(ev, InputEvent::MouseDown { .. }) {
                    self.focus_field(Some(0));
                }
                return self.comp.queue(ev);
            }
        }
        let (mut r, clicked) = self.clicks.event(ev);
        if let Some(k) = clicked {
            self.clicked(k, cx);
            r.redraw = true;
        }
        if r.consumed || r.capture {
            if matches!(ev, InputEvent::MouseDown { .. }) {
                // a button press takes the keyboard from the fields
                if !matches!(self.clicks.hovered(), Some(K::SBody | K::HBody)) {
                    self.focus_field(None);
                }
            }
            return r;
        }
        if !set_open && !off {
            let m = self.msg_mouse(ev, cx);
            if m.consumed || m.redraw || m.cursor.is_some() || m.capture {
                return m.merge(r);
            }
        }
        // swallow presses on the popovers' bodies / the off card
        if matches!(ev, InputEvent::MouseDown { .. }) && (set_open || off) {
            return Response::consumed();
        }
        r
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.refresh_view(cx);
        self.scale = cx.scale;
        let ms = kit::ms_of(dt_ms, &mut self.carry);
        self.m.set_reduce_motion(cx.reduced_motion);
        self.m.tick(ms);
        self.drain(cx);
        // the model's tick reveals streamed text and runs timers: look at the result next frame
        let streaming = self.m.is_streaming();
        if streaming || self.view.as_ref().is_some_and(|v| !v.ghosts.is_empty() || v.messages.iter().any(|m| m.copied)) {
            self.dirty = true;
        }
        if self.focus_next {
            self.focus_next = false;
            self.focus_field(Some(0));
        }
        // composer edits -> model; Enter once the typed text has been applied
        if !self.comp.has_input() {
            for e in self.comp.take_events() {
                if e == EditEvent::Changed {
                    self.recalling = false;
                }
            }
            let t = self.comp.ti.text();
            self.push_input(t);
            if std::mem::take(&mut self.comp_enter) {
                self.run(Action::Send, cx);
            }
        }
        self.settings_edit_events(cx);
        for e in self.set.select.is_open().then_some(()).iter() {
            let _ = e;
        }
        if let Some(r) = self.ime {
            cx.set_ime_rect(r);
        }
        self.cues.reduced = cx.reduced_motion;
        self.clicks.reduced = cx.reduced_motion;
        for (i, f) in [&mut self.comp, &mut self.url, &mut self.mdl, &mut self.key].into_iter().enumerate() {
            f.ti.reduced = Some(cx.reduced_motion);
            let _ = i;
        }
        self.sv.state.reduced = Some(cx.reduced_motion);
        self.sv.bar.reduced = Some(cx.reduced_motion);
        let dt = dt_ms as f32 / 1000.0;
        let mut busy = streaming;
        busy |= self.cues.step(dt_ms as f32);
        busy |= self.clicks.step(dt);
        busy |= self.sv.step(dt);
        busy |= self.hist.step(dt_ms as f32);
        busy |= self.set.step(dt_ms as f32);
        for t in [&mut self.err_k, &mut self.files_k, &mut self.off_k, &mut self.send_k] {
            busy |= t.step(dt);
        }
        for it in &mut self.items {
            if let Some(md) = &mut it.md {
                busy |= md.step(dt);
            }
        }
        for f in [&mut self.comp, &mut self.url, &mut self.mdl, &mut self.key] {
            busy |= f.ti.step(dt) || f.has_input();
        }
        busy |= self.comp_enter || self.dirty;
        if let Some(g) = &mut self.ghost {
            g.1 -= dt_ms as f32;
            if g.1 <= 0.0 {
                self.ghost = None;
            } else {
                busy = true;
            }
        }
        busy
    }

    fn draw(&mut self, cv: &Canvas, th: &Theme, rect: Rect, cx: &Cx) {
        self.now_ms = cx.now_ms;
        self.scale = cx.scale;
        // the model may have changed since the last step (a tick, an event): lay out what it shows now, so a scroll request
        // from the same tick lands on the new content
        self.refresh_view(cx);
        if self.view.is_none() {
            self.view = Some(self.m.view());
        }
        let v = self.view.clone().unwrap();
        self.clicks.begin();
        let reduced = cx.reduced_motion;
        let ic = std::mem::take(&mut self.ic);

        // ---------------- layout
        let comp_w = rect.w - 40.0 - 8.0 - 28.0 - 4.0 - 4.0 - 28.0 - 4.0 - 28.0 - 5.0;
        let ta_h = if self.comp.ti.text().is_empty() { COMPOSER_MIN } else { self.comp.ti.content_height(comp_w + 6.0, cv).clamp(COMPOSER_MIN, COMPOSER_MAX) };
        let box_h = (ta_h + 10.0).max(38.0);
        let err_h = self.err_h(cv, th, rect.w);
        let err_vis = err_h * self.err_k.value().clamp(0.0, 1.0);
        let files_vis = (24.0 + 8.0) * self.files_k.value().clamp(0.0, 1.0);
        let bottom_h = 4.0 + if err_vis > 0.5 { err_vis + 8.0 * self.err_k.value().clamp(0.0, 1.0) } else { 0.0 } + files_vis + box_h;
        let bottom = Rect::new(rect.x, rect.bottom() - bottom_h, rect.w, bottom_h);
        let viewport = Rect::new(rect.x, rect.y + SCROLL_TOP, rect.w, (bottom.y - rect.y - SCROLL_TOP).max(0.0));
        let row = Rect::new(rect.x, rect.bottom() - box_h, rect.w, box_h);
        let box_ = Rect::new(rect.x + 40.0, row.y, rect.w - 40.0, box_h);
        let ta = Rect::new(box_.x + 8.0 + 28.0 + 4.0, box_.bottom() - 5.0 - ta_h, box_.w - 8.0 - 28.0 - 4.0 - 4.0 - 28.0 - 4.0 - 28.0 - 5.0, ta_h);
        self.geo = Geo { area: rect, viewport, row, box_, ta, err: Rect::new(rect.x, bottom.y + 4.0, rect.w, err_vis), files: Rect::new(rect.x + 40.0, row.y - files_vis, rect.w - 40.0, 24.0) };

        self.layout_items(cv, th, &v);
        let chat_has_content = !v.messages.is_empty();
        let content_h = if chat_has_content { self.items.last().map_or(0.0, |i| i.y + i.h) + 12.0 } else { 0.0 };
        self.sv.set_content_height(content_h, viewport.h);
        if self.scroll_bottom {
            self.scroll_bottom = false;
            let m = self.sv.state.max_offset();
            let animate = !std::mem::take(&mut self.scroll_jump) && !reduced;
            self.sv.state.scroll_to(m, animate);
        }

        // ---------------- messages / empty state
        let inert = v.off.is_some();
        if chat_has_content {
            cv.with_clip(viewport, |cv| {
                let oy = viewport.y - self.sv.offset();
                for i in 0..self.items.len() {
                    self.draw_item(cv, th, &ic, i, viewport.x, oy, cx, &v);
                }
            });
            self.sv.bar.draw(cv, viewport, &self.sv.state, th);
        } else if v.empty.is_some() {
            self.draw_empty(cv, th, &ic, viewport, &v, !inert);
        }
        if let Some((g, left)) = &self.ghost {
            let k = (left / 180.0).clamp(0.0, 1.0);
            let y = viewport.y + self.items.last().map_or(6.0, |i| i.y) - self.sv.offset() + (1.0 - k) * -6.0;
            cv.with_clip(viewport, |cv| cv.with_opacity(k, |cv| g.draw(cv, (viewport.x + COL_X, y), th)));
        }

        // ---------------- bottom: error, files, composer
        self.draw_bottom(cv, th, &ic, &v, cx, err_vis);

        // ---------------- tools (history, settings)
        if v.off.is_none() {
            let tb = [(K::Hist, "c-history", rect.right() - 28.0 * 2.0 - 2.0), (K::Gear, "c-gear", rect.right() - 28.0)];
            let any_hover = tb.iter().any(|(k, _, _)| self.clicks.hover_amt(k) > 0.0);
            let _ = any_hover;
            let op = 0.7 + 0.3 * tb.iter().map(|(k, _, _)| self.clicks.hover_amt(k)).fold(0.0, f32::max);
            cv.with_opacity(op, |cv| {
                for (k, icon, x) in &tb {
                    let r = Rect::new(*x, rect.y, 28.0, 28.0);
                    self.clicks.add(k.clone(), r);
                    kit::button(cv, th, &ic, r, "", Some(icon), Bk::Icon, self.clicks.hover_amt(k), self.clicks.press_scale(k, 0.92), true);
                }
            });
        }

        // ---------------- popovers
        self.hist.draw(cv, th, &ic, &mut self.clicks, &self.cues, rect, reduced);
        let focus_req = self.set.draw(cv, th, &ic, &mut self.clicks, &self.cues, rect, [&mut self.url, &mut self.mdl, &mut self.key], reduced);
        if let Some(i) = focus_req {
            self.focus_field(Some(i + 1));
        }
        let (mr, _) = (self.set.model_rect, ());
        let _ = mr;

        // ---------------- off card (covers everything)
        if self.off_k.value() > 0.003 {
            if let Some(off) = &v.off {
                self.draw_off(cv, th, &ic, rect, off.title.as_str(), off.body.as_str(), off.enable_label.as_str());
            } else {
                self.draw_off(cv, th, &ic, rect, "", "", "");
            }
        }
        self.ic = ic;
    }
}

impl ChatScreen {
    fn err_h(&self, cv: &Canvas, th: &Theme, w: f32) -> f32 {
        let Some(e) = self.err_last.as_ref() else { return 0.0 };
        let (tw, _) = self.err_text_w(cv, th, e, w);
        let (_, h) = cv.measure(&e.text, th.small(400).wrap(), tw);
        (h.max(16.0) + 12.0).max(36.0)
    }

    /// (width available to the message text, x where the buttons start)
    fn err_text_w(&self, cv: &Canvas, th: &Theme, e: &ErrView, w: f32) -> (f32, f32) {
        let bw = |s: &str| cv.text_width(s, th.small(500)) + 16.0;
        let mut btn = bw(&e.settings_label) + 24.0 + 12.0;
        if e.retry {
            btn += bw(&e.retry_label) + 6.0;
        }
        ((w - 12.0 - 6.0 - btn - 6.0).max(60.0), w - btn)
    }

    fn layout_items(&mut self, cv: &Canvas, th: &Theme, v: &ChatView) {
        let w = self.sv_width();
        let labels = MarkdownLabels { copy: self.copy_label(), copied: self.copied_label() };
        let md_w = (w - COL_X).max(40.0);
        let mut y = 6.0;
        self.items.truncate(v.messages.len());
        for (i, m) in v.messages.iter().enumerate() {
            let reuse = self.items.get(i).is_some_and(|it| it.id == m.id && it.role == m.role);
            if !reuse {
                if self.items.len() > i {
                    self.items.truncate(i);
                    self.usel = None;
                }
                self.items.push(Item { id: m.id, role: m.role, text: String::new(), idx: i, streaming: false, typing: false, actions: false, copied: false, md: None, md_dark: th.dark, bubble: None, y: 0.0, h: 0.0 });
            }
            let it = &mut self.items[i];
            it.idx = i;
            it.streaming = m.streaming;
            it.typing = m.typing;
            it.actions = m.actions;
            it.copied = m.copied;
            let dark = th.dark;
            match m.role {
                Role::Assistant => {
                    if !m.typing {
                        let blocks = markdown::parse(&m.text);
                        match it.md.as_mut() {
                            None => it.md = MarkdownView::new(cv, blocks, md_w, th, &labels).ok(),
                            Some(md) => {
                                if (md.width() - md_w).abs() > 0.5 || it.md_dark != dark {
                                    let _ = md.relayout(cv, md_w, th);
                                    it.md_dark = dark;
                                }
                                if it.text != m.text {
                                    let _ = md.update(cv, blocks, th);
                                }
                            }
                        }
                    }
                    it.text = m.text.clone();
                    let col_h = if m.typing { 22.0 } else { it.md.as_ref().map_or(22.0, |md| md.height().max(22.0)) };
                    let act = if m.actions { 4.0 + 26.0 } else { 0.0 };
                    it.h = (col_h + act).max(AV - 3.0);
                }
                Role::User => {
                    let maxw = (w * 0.78).floor();
                    let inner = (maxw - 32.0).max(20.0);
                    let rebuild = it.bubble.as_ref().map_or(true, |b| it.text != m.text || (b.lay.width() - inner).abs() > 0.5);
                    if rebuild {
                        let rt = RichText::new().with_base(Style::new(14.0, 400)).with_line_height(20.0).text(&m.text);
                        if let Ok(lay) = RichLayout::new(cv, &rt, inner) {
                            let bw = (lay.size().0.ceil() + 32.0).min(maxw).max(40.0);
                            it.bubble = Some(Bubble { lay, w: bw });
                        }
                    }
                    it.text = m.text.clone();
                    it.h = it.bubble.as_ref().map_or(20.0, |b| b.lay.height() + 18.0);
                }
            }
            it.y = y;
            y += it.h + MSG_GAP;
        }
        // the ghost of an answer that is being replaced (retry)
        if !v.ghosts.is_empty() && self.ghost.is_none() {
            let g = &v.ghosts[0];
            if let Ok(md) = MarkdownView::new(cv, markdown::parse(&g.text), md_w, th, &labels) {
                self.ghost = Some((md, g.left_ms as f32));
            }
        }
    }

    fn copy_label(&self) -> &'static str {
        if self.m.cfg().on && false {
            ""
        } else {
            self.lang_label(true)
        }
    }

    fn copied_label(&self) -> &'static str {
        self.lang_label(false)
    }

    fn lang_label(&self, copy: bool) -> &'static str {
        let ru = self.view.as_ref().is_some_and(|v| v.placeholder.starts_with("Сообщение"));
        match (ru, copy) {
            (true, true) => "Копировать",
            (true, false) => "Скопировано",
            (false, true) => "Copy",
            (false, false) => "Copied",
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_item(&mut self, cv: &Canvas, th: &Theme, ic: &Icons, i: usize, x0: f32, oy: f32, cx: &Cx, v: &ChatView) {
        let it = &self.items[i];
        let w = self.sv_width();
        let y = oy + it.y;
        if y > self.geo.viewport.bottom() || y + it.h < self.geo.viewport.y - 40.0 {
            // off-screen rows still need their click targets out of the way: they register none
            if it.actions || !matches!(it.role, Role::User) {
                return;
            }
            return;
        }
        let cue = match (it.role, it.id) {
            (Role::User, -1) => "msg:user",
            (Role::Assistant, -2) => "msg:ai",
            _ => "",
        };
        let about = (x0 + w / 2.0, y + it.h / 2.0);
        let mut draw = |cv: &Canvas| match it.role {
            Role::User => {
                let Some(b) = &it.bubble else { return };
                let br = Rect::new(x0 + w - b.w, y, b.w, b.lay.height() + 18.0);
                cv.fill_rr(br, 19.0, th.raised);
                if let Some(u) = self.usel.filter(|u| u.item == i) {
                    b.lay.draw_selection(cv, (br.x + 16.0, br.y + 9.0), u.a.min(u.b)..u.a.max(u.b), th.accent.alpha(0.35));
                }
                b.lay.draw(cv, (br.x + 16.0, br.y + 9.0), th.text);
            }
            Role::Assistant => {
                // avatar: centre on the first 22 px line
                let av = Rect::new(x0, y - 3.0, AV, AV);
                cv.fill_circle(av.center(), AV / 2.0, th.accent_soft);
                ic.draw(cv, "c-sparkles", av.centered(16.0, 16.0), th.accent);
                if it.typing {
                    let t = cx.now_ms / 1000.0;
                    for d in 0..3 {
                        let ph = if cx.reduced_motion { 0.5 } else { (((t - d as f64 * 0.15) % 1.0 + 1.0) % 1.0) as f32 };
                        let k = if ph < 0.4 { ph / 0.4 } else if ph < 0.8 { 1.0 - (ph - 0.4) / 0.4 } else { 0.0 };
                        let c = (x0 + COL_X + 3.0 + d as f32 * 10.0, y + 8.0 + 3.0 + 2.0);
                        cv.fill_circle(c, 3.0 * (0.8 + 0.2 * k), th.dim.fade(0.25 + 0.75 * k));
                    }
                } else if let Some(md) = &it.md {
                    md.draw(cv, (x0 + COL_X, y), th);
                    if it.streaming {
                        let n = md.leaf_count();
                        if n > 0 {
                            if let (Some(lr), Some(lay)) = (md.leaf_rect(n - 1), md.leaf_layout(n - 1)) {
                                let c = lay.caret_rect(lay.len());
                                let pulse = if cx.reduced_motion { 0.8 } else { 0.55 + 0.45 * ((cx.now_ms / 1100.0 * std::f64::consts::TAU).sin() as f32) };
                                cv.fill_rr(Rect::new(x0 + COL_X + lr.x + c.x + 1.0, y + lr.y + c.y + 3.0, 2.0, (c.h - 6.0).max(6.0)), 1.0, th.accent.fade(pulse));
                            }
                        }
                    }
                }
                if it.actions {
                    let ay = y + it.h - 26.0;
                    let items = [(K::CopyMsg(it.idx), if it.copied { "c-check" } else { "c-copy" }), (K::Retry, "c-retry")];
                    for (n, (k, icon)) in items.into_iter().enumerate() {
                        let r = Rect::new(x0 + COL_X - 6.0 + n as f32 * 28.0, ay, 26.0, 26.0);
                        self.clicks.add(k.clone(), r);
                        kit::button(cv, th, ic, r, "", Some(icon), Bk::Icon, self.clicks.hover_amt(&k), self.clicks.press_scale(&k, 0.92), true);
                    }
                }
            }
        };
        let _ = v;
        if cue.is_empty() {
            draw(cv)
        } else {
            self.cues.apply(cv, cue, about, draw);
        }
    }

    fn draw_empty(&mut self, cv: &Canvas, th: &Theme, ic: &Icons, vp: Rect, v: &ChatView, live: bool) {
        let Some(e) = &v.empty else { return };
        let w = vp.w - 40.0;
        let sub_st = Style::new(12.0, 400).center().wrap();
        let (_, sh) = cv.measure(&e.sub, sub_st, w.min(520.0));
        let sh = sh.max(16.0).round();
        let chips = e.ideas.len() as f32 * 28.0 + (e.ideas.len() as f32 - 1.0).max(0.0) * 8.0;
        let total = 40.0 + 6.0 + 6.0 + 20.0 + 6.0 + sh + 12.0 + chips;
        let mut y = vp.y + ((vp.h - 8.0 - total) / 2.0).max(0.0);
        let cxm = vp.cx();
        let sp = Rect::new(cxm - 20.0, y, 40.0, 40.0);
        cv.fill_circle(sp.center(), 20.0, th.accent_soft);
        ic.draw(cv, "c-sparkles", sp.centered(20.0, 20.0), th.accent);
        y += 40.0 + 6.0 + 6.0;
        cv.text(Rect::new(vp.x, y, vp.w, 20.0), &e.title, Style::new(16.0, 600).center(), th.text);
        y += 20.0 + 6.0;
        cv.text(Rect::new(cxm - w.min(520.0) / 2.0, y, w.min(520.0), sh), &e.sub, sub_st, th.dim);
        y += sh + 12.0;
        for (i, idea) in e.ideas.iter().enumerate() {
            let st = Style::new(12.0, 500);
            let tw = cv.text_width(idea, st).ceil() + 32.0;
            let r = Rect::new(cxm - tw / 2.0, y, tw, 28.0);
            if live {
                self.clicks.add(K::Idea(i), r);
            }
            let (hv, ps) = (self.clicks.hover_amt(&K::Idea(i)), self.clicks.press_scale(&K::Idea(i), 0.96));
            cv.with_scale_about(r.center(), ps, |cv| {
                cv.fill_rr(r, 14.0, th.card.mix(th.raised, hv));
                cv.text(r, idea, st.center(), th.text2.mix(th.text, hv));
            });
            y += 28.0 + 8.0;
        }
    }

    fn draw_off(&mut self, cv: &Canvas, th: &Theme, ic: &Icons, rect: Rect, title: &str, body: &str, enable: &str) {
        let k = self.off_k.value().clamp(0.0, 1.0);
        let live = !title.is_empty();
        if live {
            self.clicks.add(K::SBody, rect);
        }
        cv.with_opacity(k, |cv| {
            cv.fill_rect(rect, th.bg);
            let bw = (rect.w - 140.0).min(460.0);
            let st = Style::new(12.0, 400).center().wrap();
            let line_st = st;
            let (_, bh) = cv.measure(body, line_st, bw);
            let bh = (bh / 18.0).ceil().max(1.0) * 18.0;
            let total = 40.0 + 8.0 + 20.0 + 8.0 + bh + 8.0 + 6.0 + 32.0;
            let mut y = rect.cy() - total / 2.0;
            let rise = (1.0 - k) * 8.0;
            let sp = Rect::new(rect.cx() - 20.0, y + rise, 40.0, 40.0);
            cv.fill_circle(sp.center(), 20.0, th.accent_soft);
            ic.draw(cv, "c-sparkles", sp.centered(20.0, 20.0), th.accent);
            y += 40.0 + 8.0;
            cv.text(Rect::new(rect.x, y + rise, rect.w, 20.0), title, Style::new(16.0, 600).center(), th.text);
            y += 20.0 + 8.0;
            cv.text(Rect::new(rect.cx() - bw / 2.0, y + rise, bw, bh), body, line_st, th.dim);
            y += bh + 8.0 + 6.0;
            let r = Rect::new(rect.cx() - 55.0, y + rise, 110.0, 32.0);
            if live {
                self.clicks.add(K::Enable, r);
            }
            kit::button(cv, th, ic, r, enable, None, Bk::Primary, self.clicks.hover_amt(&K::Enable), self.clicks.press_scale(&K::Enable, 0.96), true);
        });
    }

    fn draw_bottom(&mut self, cv: &Canvas, th: &Theme, ic: &Icons, v: &ChatView, cx: &Cx, err_vis: f32) {
        let g = self.geo;
        let live = v.off.is_none() && !self.set.is_open();
        // error banner
        if err_vis > 0.5 {
            if let Some(e) = self.err_last.clone() {
                let k = self.err_k.value().clamp(0.0, 1.0);
                let r = Rect::new(g.err.x, g.err.y, g.err.w, err_vis);
                cv.with_clip(r, |cv| {
                    cv.with_opacity(k, |cv| {
                        cv.fill_rr(r, 12.0, th.red_bg);
                        let (tw, bx) = self.err_text_w(cv, th, &e, r.w);
                        cv.text(Rect::new(r.x + 12.0, r.y + 6.0, tw, r.h - 12.0), &e.text, th.small(400).wrap(), th.red);
                        let mut x = bx;
                        let mut btn = |cv: &Canvas, clicks: &mut Clicks<K>, key: K, label: &str| {
                            let bw = cv.text_width(label, th.small(500)) + 16.0;
                            let br = Rect::new(x, r.cy() - 12.0, bw, 24.0);
                            if live {
                                clicks.add(key.clone(), br);
                            }
                            let hv = clicks.hover_amt(&key);
                            cv.with_scale_about(br.center(), clicks.press_scale(&key, 0.96), |cv| {
                                cv.fill_rr(br, th.m.btn_r, th.hover.fade(hv));
                                cv.text(br, label, th.small(500).center(), th.text2.mix(th.text, hv));
                            });
                            x += bw + 6.0;
                        };
                        if e.retry {
                            btn(cv, &mut self.clicks, K::ErrRetry, &e.retry_label);
                        }
                        btn(cv, &mut self.clicks, K::ErrSettings, &e.settings_label);
                        let xr = Rect::new(r.right() - 6.0 - 24.0, r.cy() - 12.0, 24.0, 24.0);
                        if live {
                            self.clicks.add(K::ErrX, xr);
                        }
                        kit::button(cv, th, ic, xr, "", Some("x"), Bk::Icon, self.clicks.hover_amt(&K::ErrX), self.clicks.press_scale(&K::ErrX, 0.92), true);
                    });
                });
            }
        }
        // files
        let fk = self.files_k.value().clamp(0.0, 1.0);
        if fk > 0.02 {
            let files = if v.files.is_empty() { self.files_last.clone() } else { v.files.clone() };
            cv.with_opacity(fk, |cv| {
                let mut x = g.files.x;
                let y = g.row.y - (24.0 + 8.0) * fk;
                for (i, f) in files.iter().enumerate() {
                    let st = Style::new(12.0, 400);
                    let tw = cv.text_width(f, st).ceil();
                    let r = Rect::new(x, y, 12.0 + tw + 4.0 + 20.0 + 2.0, 24.0);
                    cv.fill_rr(r, 12.0, th.raised);
                    cv.text(Rect::new(r.x + 12.0, r.y, tw + 2.0, 24.0), f, st, th.text2);
                    let xr = Rect::new(r.right() - 22.0, r.cy() - 10.0, 20.0, 20.0);
                    if live {
                        self.clicks.add(K::FileX(i), xr);
                    }
                    kit::button(cv, th, ic, xr, "", Some("x"), Bk::Icon, self.clicks.hover_amt(&K::FileX(i)), self.clicks.press_scale(&K::FileX(i), 0.92), true);
                    x = r.right() + 6.0;
                }
            });
        }
        // composer row
        let (row, bx, ta) = (g.row, g.box_, g.ta);
        let comp = Rect::new(row.x, row.bottom() - 5.0 - 28.0, 28.0, 28.0);
        let attach = Rect::new(bx.x + 8.0, bx.bottom() - 5.0 - 28.0, 28.0, 28.0);
        let mic = Rect::new(ta.right() + 4.0, attach.y, 28.0, 28.0);
        let send = Rect::new(mic.right() + 4.0, attach.y, 28.0, 28.0);
        if live {
            for (k, r) in [(K::Compose, comp), (K::Attach, attach), (K::Mic, mic), (K::Send, send)] {
                self.clicks.add(k, r);
            }
        }
        cv.fill_rr(bx, 19.0, th.card);
        let btn = |cv: &Canvas, clicks: &Clicks<K>, k: K, r: Rect, icon: &str, tint: Option<Color>| {
            let (hv, ps) = (clicks.hover_amt(&k), clicks.press_scale(&k, 0.92));
            cv.with_scale_about(r.center(), ps, |cv| {
                cv.fill_rr(r, 14.0, th.hover.fade(hv));
                ic.draw(cv, icon, r.centered(16.0, 16.0), tint.unwrap_or(th.icon.mix(th.text, hv)));
            });
        };
        btn(cv, &self.clicks, K::Compose, comp, "c-compose", None);
        btn(cv, &self.clicks, K::Attach, attach, "c-attach", None);
        btn(cv, &self.clicks, K::Mic, mic, "c-mic", if v.mic_recording { Some(Color::rgb8(0xff, 0x5a, 0x4e)) } else { None });
        // send / stop
        let sk = self.send_k.value().clamp(0.0, 1.0);
        {
            let (hv, ps) = (self.clicks.hover_amt(&K::Send), self.clicks.press_scale(&K::Send, 0.94));
            cv.with_scale_about(send.center(), ps, |cv| {
                let stop_bg = if th.dark { Color::rgb8(255, 255, 255) } else { Color::rgb8(0x16, 0x16, 0x16) };
                let bg = th.raised2.mix(th.accent, hv).mix(stop_bg, sk);
                cv.fill_circle(send.center(), 14.0, bg);
                let fg = th.text2.mix(th.on_accent, hv).mix(if th.dark { Color::rgb8(0, 0, 0) } else { Color::rgb8(255, 255, 255) }, sk);
                let r = send.centered(15.0, 15.0);
                cv.with_opacity(1.0 - sk, |cv| ic.draw_sw(cv, "c-send", r, fg, 2.0));
                cv.with_opacity(sk, |cv| ic.draw(cv, "c-stop", r, fg));
            });
        }
        // the composer text
        let visible = Rect::new(ta.x - 2.0, ta.y, ta.w + 4.0, ta.h);
        let big = visible.inset_xy(-2.0, -2.0);
        let mut t = *th;
        t.field = Color::rgba(0.0, 0.0, 0.0, 0.0);
        self.comp.ti.style = Style::new(13.0, 400);
        self.comp.ti.radius = 0.0;
        self.comp.ti.pad = (6.0, 7.5);
        self.comp.ti.placeholder = v.placeholder.clone();
        if self.comp.pump(cv, big) {
            self.focus_field(Some(0));
        }
        cv.with_clip(visible, |cv| self.comp.ti.draw(cv, big, &t));
        self.ime = self.comp.ime_rect(cv).filter(|_| self.comp.is_focused());
        let _ = cx;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use crate::ui::tabs::b::kit::snap;
    use crate::ui::view::{Fakes, MsgQueue, Services};
    use glasskit::input::{Buttons, Modifiers};
    use std::sync::Arc;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/tabs-b/chat.json");
    const SIZE: (f32, f32) = snap::BODY_TALL;

    /// A view plus the host pieces a test needs (the same `Cx` the real host builds, a queue to pump).
    struct Rig {
        v: ChatScreen,
        cx: Cx,
        q: Arc<MsgQueue>,
        fakes: Fakes,
        events: Vec<(String, Value, u64)>,
        th: Theme,
    }

    fn scenario(name: &str) -> Value {
        let j: Value = serde_json::from_str(FIXTURE).expect("chat.json");
        j["scenarios"][name].clone()
    }

    impl Rig {
        fn new(name: &str) -> Rig {
            let sc = scenario(name);
            let (svc, fakes): (Services, Fakes) = Services::fake();
            for (cmd, v) in sc["replies"].as_object().unwrap() {
                fakes.api.reply(cmd, Ok(v.clone()));
            }
            let (mut cx, q) = Cx::headless_queue(svc);
            cx.lang = glasskit::lang::Lang::Ru;
            cx.reduced_motion = true;
            let events = sc["events"].as_array().map(|a| a.iter().map(|e| (e["name"].as_str().unwrap().to_string(), e["payload"].clone(), e["after_ms"].as_u64().unwrap_or(100))).collect()).unwrap_or_default();
            let mut r = Rig { v: ChatScreen::new(), cx, q, fakes, events, th: Theme::pill_dark() };
            r.v.on_show(&mut r.cx);
            r.settle();
            r
        }

        fn pump(&mut self) {
            for _ in 0..50 {
                let msgs = self.q.drain();
                if msgs.is_empty() {
                    break;
                }
                for (_, m) in msgs {
                    self.v.message(m, &mut self.cx);
                }
            }
        }

        /// One frame: messages, step, draw.
        fn frame(&mut self, dt: f64) {
            self.pump();
            self.cx.now_ms += dt;
            self.v.step(dt, &mut self.cx);
            self.pump();
            crate::ui::host::testing::render_view_cx(&mut self.v, &self.cx, SIZE, 1.0, &self.th);
        }

        fn settle(&mut self) {
            for _ in 0..6 {
                self.frame(16.0);
            }
        }

        fn send_ev(&mut self, ev: InputEvent) -> Response {
            let r = self.v.event(&ev, &mut self.cx);
            self.frame(16.0);
            r
        }

        fn click(&mut self, p: Pt) {
            let mv = InputEvent::MouseMove { pos: p, mods: Modifiers::NONE, buttons: Buttons::NONE };
            self.send_ev(mv);
            self.send_ev(InputEvent::MouseDown { pos: p, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
            self.send_ev(InputEvent::MouseUp { pos: p, button: MouseButton::Left, mods: Modifiers::NONE });
            self.settle();
        }

        fn click_key(&mut self, k: K) {
            let r = self.v.clicks.rect_of(&k).unwrap_or_else(|| panic!("no click target {k:?}"));
            self.click(r.center());
        }

        fn type_text(&mut self, s: &str) {
            for c in s.chars() {
                self.v.event(&InputEvent::Char(c), &mut self.cx);
            }
            self.settle();
        }

        fn key(&mut self, code: KeyCode) {
            self.send_ev(InputEvent::Key { code, mods: Modifiers::NONE, down: true, repeat: false });
            self.settle();
        }

        fn calls(&self, name: &str) -> Vec<Value> {
            self.fakes.api.calls().into_iter().filter(|c| c.name == name).map(|c| c.args).collect()
        }

        /// Focus the composer by clicking it, type, press Enter.
        fn send_message(&mut self, text: &str) {
            let ta = self.v.geo.ta;
            self.click(ta.center());
            assert!(self.v.comp.is_focused(), "a click focuses the composer");
            self.type_text(text);
            self.key(KeyCode::Enter);
            self.settle();
        }

        fn emit_scripted_events(&mut self) {
            for (name, payload, _) in self.events.clone() {
                self.fakes.bus.emit(&name, payload);
                self.frame(16.0);
            }
        }

        fn run_frames(&mut self, n: usize) {
            for _ in 0..n {
                self.frame(16.7);
            }
        }

        fn shoot(&mut self, name: &str, dark: bool, scale: f32) {
            self.th = snap::theme(dark);
            self.frame(16.0);
            snap::shoot(&mut self.v, &self.cx, name, SIZE, scale, dark);
        }
    }

    fn md_of(r: &Rig, i: usize) -> &MarkdownView {
        r.v.items[i].md.as_ref().expect("markdown view")
    }

    #[test]
    fn mount_loads_the_config_then_the_last_thread() {
        let r = Rig::new("chat-answer");
        let names: Vec<String> = r.fakes.api.calls().into_iter().map(|c| c.name).collect();
        assert_eq!(&names[..3], ["ai_config", "chat_threads", "chat_messages"], "{names:?}");
        assert_eq!(r.calls("chat_messages")[0], json!({ "thread": 1 }));
        assert_eq!(r.v.items.len(), 2);
        assert!(r.v.items[1].actions, "the last answer carries the copy / answer-again buttons");
        for ev in ["chat:delta", "chat:done", "settings:changed", "voice:state", "voice:done"] {
            assert_eq!(r.fakes.bus.subscribers(ev), 1, "{ev}");
        }
    }

    #[test]
    fn enter_sends_the_text_and_shift_enter_does_not() {
        let mut r = Rig::new("chat-streaming");
        let ta = r.v.geo.ta;
        r.click(ta.center());
        r.type_text("Привет");
        r.send_ev(InputEvent::Key { code: KeyCode::Enter, mods: Modifiers { shift: true, ..Modifiers::NONE }, down: true, repeat: false });
        r.settle();
        assert!(r.calls("chat_send").is_empty(), "Shift+Enter is a newline");
        assert!(r.v.comp.ti.text().contains('\n'));
        r.v.comp.ti.set_text("Привет");
        r.key(KeyCode::Enter);
        r.settle();
        let sent = r.calls("chat_send");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0], json!({ "thread": null, "text": "Привет", "model": null }));
        assert!(r.v.comp.ti.text().is_empty(), "the composer is cleared after a send");
    }

    #[test]
    fn typing_dots_then_streamed_markdown_grows() {
        let mut r = Rig::new("chat-streaming");
        r.send_message("Объясни простыми словами, как работает VPN");
        assert!(r.v.items.last().is_some_and(|i| i.typing), "typing dots before the first token");
        assert!(r.v.m.is_streaming());
        let mut sizes = vec![];
        r.emit_scripted_events();
        for _ in 0..40 {
            r.run_frames(1);
            sizes.push(r.v.items.last().map_or(0, |i| i.text.len()));
        }
        assert!(sizes.windows(2).all(|w| w[0] <= w[1]), "the revealed text only grows: {sizes:?}");
        let last = r.v.items.last().unwrap();
        assert!(!last.typing && last.streaming);
        assert!(md_of(&r, r.v.items.len() - 1).leaf_count() >= 4, "heading, list items, paragraph and the code block are leaves");
    }

    #[test]
    fn two_hundred_deltas_keep_the_markdown_incremental() {
        let mut r = Rig::new("chat-streaming");
        r.send_message("long");
        let paragraph = |i: usize| format!("Абзац номер {i} с **жирным** словом и `кодом`, ссылка [сайт](https://example.com/{i}) и ещё немного текста, чтобы строка переносилась по ширине.\n\n");
        let started = std::time::Instant::now();
        let mut draws = 0u32;
        for i in 0..200 {
            let chunk = if i % 4 == 0 { paragraph(i / 4) } else { "и ещё слово ".to_string() };
            r.fakes.bus.emit("chat:delta", json!({ "thread": 1, "run": 7, "text": chunk }));
            r.frame(8.0);
            draws += 1;
        }
        r.run_frames(300);
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let idx = r.v.items.len() - 1;
        let md = md_of(&r, idx);
        let leaves = md.leaf_count();
        assert!(leaves >= 40, "fifty paragraphs: {leaves}");
        // a full re-layout on every frame would build leaves x frames layouts (> 10 000); incremental rebuilds ~ one tail block per frame
        assert!(md.rebuilds() < (leaves as u32) * 6 + draws * 4, "rebuilds {} for {} leaves / {} frames", md.rebuilds(), leaves, draws);
        let per = ms / (draws as f64 + 300.0);
        eprintln!("streaming: {} rebuilds, {} leaves, {:.2} ms per step+draw frame (headless WARP, 648x346)", md.rebuilds(), leaves, per);
        assert!(per < 60.0, "a frame costs {per:.1} ms");
        assert!(r.v.sv.state.at_end() || r.v.sv.state.max_offset() < 1.0, "the view follows the stream");
    }

    #[test]
    fn the_view_follows_the_stream_until_the_user_scrolls_up() {
        let mut r = Rig::new("chat-streaming");
        r.send_message("go");
        let para = "Строка ответа, достаточно длинная, чтобы занять место в списке сообщений.\n\n".repeat(30);
        r.fakes.bus.emit("chat:delta", json!({ "thread": 1, "run": 7, "text": para }));
        r.run_frames(120);
        let st = &r.v.sv.state;
        assert!(st.max_offset() > 100.0 && st.at_end(), "pinned to the bottom while streaming: max {} target {} off {} content {} vp {} streaming {} shown {} near {}", st.max_offset(), st.target(), st.offset(), st.content(), st.viewport(), r.v.m.is_streaming(), r.v.m.streamed().0.len(), r.v.m.view().near_bottom);
        let vp = r.v.geo.viewport;
        r.send_ev(InputEvent::Wheel { pos: vp.center(), delta: (0.0, -300.0), mods: Modifiers::NONE, precise: true });
        r.settle();
        let before = r.v.sv.offset();
        r.fakes.bus.emit("chat:delta", json!({ "thread": 1, "run": 7, "text": para }));
        r.run_frames(120);
        assert!((r.v.sv.offset() - before).abs() < 1.0, "scrolled-up view stays where the user put it ({} vs {})", r.v.sv.offset(), before);
        assert!(!r.v.sv.state.at_end());
    }

    #[test]
    fn escape_stops_the_stream() {
        let mut r = Rig::new("chat-streaming");
        r.send_message("stop me");
        assert!(r.v.m.is_streaming());
        r.key(KeyCode::Escape);
        assert_eq!(r.calls("chat_stop"), vec![json!({ "thread": 1 })]);
    }

    #[test]
    fn the_send_button_turns_into_stop() {
        let mut r = Rig::new("chat-streaming");
        r.send_message("hi");
        r.run_frames(5);
        assert!(r.v.send_k.target() > 0.5, "stop shape while streaming");
        r.click_key(K::Send);
        assert_eq!(r.calls("chat_stop").len(), 1);
    }

    #[test]
    fn retry_calls_chat_retry_and_a_fresh_typing_answer_appears() {
        let mut r = Rig::new("chat-answer");
        r.click_key(K::Retry);
        assert_eq!(r.calls("chat_retry"), vec![json!({ "thread": 1, "model": null })]);
        assert!(r.v.m.is_streaming());
        assert!(r.v.items.last().is_some_and(|i| i.typing), "a fresh typing answer replaces the old one");
    }

    #[test]
    fn history_popover_lists_threads_and_picks_one() {
        let mut r = Rig::new("chat-history-popover");
        r.click_key(K::Hist);
        assert_eq!(r.calls("chat_threads").len(), 2, "mount + the popover");
        assert!(r.v.hist.visible());
        assert_eq!(r.v.view.as_ref().unwrap().history.rows.len(), 3);
        r.click_key(K::HRow(2));
        assert_eq!(r.calls("chat_messages").last().unwrap(), &json!({ "thread": 2 }));
        r.run_frames(30);
        assert!(!r.v.hist.visible(), "the popover folds away after a pick");
    }

    #[test]
    fn a_press_outside_the_history_popover_closes_it() {
        let mut r = Rig::new("chat-history-popover");
        r.click_key(K::Hist);
        assert!(r.v.view.as_ref().unwrap().history.open);
        r.click((100.0, 150.0));
        assert!(!r.v.view.as_ref().unwrap().history.open);
    }

    #[test]
    fn settings_popover_opens_probes_and_tests_with_the_typed_values() {
        let mut r = Rig::new("chat-settings-popover");
        r.click_key(K::Gear);
        assert!(r.v.set.is_open());
        assert_eq!(r.calls("ai_probe").len(), 1);
        assert_eq!(r.calls("net_features_status").len(), 1);
        r.click_key(K::STest);
        let sets = r.calls("settings_set");
        assert_eq!(sets[0], json!({ "patch": { "ai.baseUrl": "http://127.0.0.1:11434/v1", "ai.model": "qwen3:8b" } }));
        r.key(KeyCode::Escape);
        assert!(!r.v.view.as_ref().unwrap().settings.open, "Esc closes the panel");
    }

    #[test]
    fn the_api_key_travels_only_in_ai_set_key() {
        let mut r = Rig::new("chat-settings-popover");
        r.click_key(K::Gear);
        let kc = r.v.key.rect.center();
        r.click(kc);
        r.type_text("sk-test-secret");
        r.click_key(K::SSaveKey);
        let all = r.fakes.api.calls();
        let with_key: Vec<&str> = all.iter().filter(|c| c.args.to_string().contains("sk-test-secret")).map(|c| c.name.as_str()).collect();
        assert_eq!(with_key, ["ai_set_key"]);
        assert!(r.v.key.ti.text().is_empty(), "the field is cleared");
    }

    #[test]
    fn offline_card_enables_ai() {
        let mut r = Rig::new("chat-offline");
        assert!(r.v.view.as_ref().unwrap().off.is_some());
        r.click_key(K::Enable);
        assert_eq!(r.calls("settings_set")[0], json!({ "patch": { "net.ai": true } }));
    }

    #[test]
    fn empty_state_idea_sends_it() {
        let mut r = Rig::new("chat-empty");
        r.click_key(K::Idea(0));
        let sent = r.calls("chat_send");
        assert_eq!(sent[0]["text"], "Объясни простыми словами, как работает VPN");
    }

    /// Window position of the origin of item `i`'s markdown.
    fn md_origin(r: &Rig, i: usize) -> Pt {
        (r.v.geo.viewport.x + COL_X, r.v.geo.viewport.y - r.v.sv.offset() + r.v.items[i].y)
    }

    #[test]
    fn code_block_copy_button_copies_the_code() {
        let mut r = Rig::new("chat-answer");
        let i = r.v.items.len() - 1;
        let btn = md_of(&r, i).copy_button_rects()[0].1;
        let o = md_origin(&r, i);
        r.click((o.0 + btn.cx(), o.1 + btn.cy()));
        let clip = r.cx.clipboard_text().expect("copied");
        assert!(clip.starts_with("fn main() {") && clip.contains("derive_key"), "{clip}");
        assert!(!clip.contains("```"), "the code, not the fence");
    }

    #[test]
    fn a_safe_link_opens_through_the_backend_and_javascript_urls_are_inert() {
        let mut r = Rig::new("chat-answer");
        let i = r.v.items.len() - 1;
        let o = md_origin(&r, i);
        let md = md_of(&r, i);
        let mut found = None;
        'o: for leaf in 0..md.leaf_count() {
            let lr = md.leaf_rect(leaf).unwrap();
            let mut y = lr.y + 4.0;
            while y < lr.bottom() {
                let mut x = lr.x + 2.0;
                while x < lr.right() {
                    if let Hit::Link(u) = md.hit((x, y)) {
                        found = Some(((x, y), u));
                        break 'o;
                    }
                    x += 3.0;
                }
                y += 6.0;
            }
        }
        let ((x, y), url) = found.expect("the answer has a link");
        assert_eq!(url, "https://www.wireguard.com/");
        r.click((o.0 + x, o.1 + y));
        assert_eq!(r.calls("chat_open_link"), vec![json!({ "url": "https://www.wireguard.com/" })]);
        let h = glasskit::testkit::Harness::new();
        let th = Theme::pill_dark();
        h.render(300, 80, 1.0, th.bg, |cv| {
            let md = MarkdownView::new(cv, markdown::parse("[click me](javascript:alert(1)) and [ok](mailto:a@b.c)"), 280.0, &th, &MarkdownLabels { copy: "Copy", copied: "Copied" }).unwrap();
            let lr = md.leaf_rect(0).unwrap();
            let mut links: Vec<String> = vec![];
            let mut x = lr.x;
            while x < lr.right() {
                if let Hit::Link(u) = md.hit((x, lr.y + 8.0)) {
                    if !links.contains(&u) {
                        links.push(u);
                    }
                }
                x += 2.0;
            }
            assert_eq!(links, vec!["mailto:a@b.c".to_string()], "javascript: is never exposed");
        });
        assert!(!markdown::is_safe_url("javascript:alert(1)") && !markdown::is_safe_url("file:///c:/x"));
    }

    #[test]
    fn dragging_selects_message_text_and_ctrl_c_copies_it() {
        let mut r = Rig::new("chat-answer");
        let i = r.v.items.len() - 1;
        let o = md_origin(&r, i);
        let last = md_of(&r, i).leaf_count() - 1;
        let lr = md_of(&r, i).leaf_rect(last).unwrap();
        let a = (o.0 + lr.x + 2.0, o.1 + lr.y + 6.0);
        let b = (o.0 + lr.x + 120.0, o.1 + lr.y + 6.0);
        r.send_ev(InputEvent::MouseMove { pos: a, mods: Modifiers::NONE, buttons: Buttons::NONE });
        r.send_ev(InputEvent::MouseDown { pos: a, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 });
        r.send_ev(InputEvent::MouseMove { pos: b, mods: Modifiers::NONE, buttons: Buttons::NONE.with(MouseButton::Left) });
        r.send_ev(InputEvent::MouseUp { pos: b, button: MouseButton::Left, mods: Modifiers::NONE });
        assert!(!r.v.selection_text().is_empty(), "something is selected");
        r.send_ev(InputEvent::Key { code: KeyCode::Char('C'), mods: Modifiers { ctrl: true, ..Modifiers::NONE }, down: true, repeat: false });
        let clip = r.cx.clipboard_text().expect("copied");
        assert!("Подробнее читайте в документации WireGuard.".contains(clip.trim()) && !clip.trim().is_empty(), "{clip}");
    }

    #[test]
    fn the_last_answer_copy_button_copies_the_whole_answer() {
        let mut r = Rig::new("chat-answer");
        r.click_key(K::CopyMsg(1));
        let clip = r.cx.clipboard_text().unwrap();
        assert!(clip.contains("Как работает VPN") && clip.contains("```rust"), "the raw markdown of the answer");
    }

    #[test]
    fn dropped_text_files_become_attachment_chips() {
        let dir = std::env::temp_dir().join("pill-chat-drop-test");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("notes.txt");
        std::fs::write(&f, "hello").unwrap();
        let mut r = Rig::new("chat-empty");
        let ev = DropEvent { phase: DropPhase::Drop, pos: (10.0, 10.0), payload: Some(DropPayload::Files(vec![f.clone()])) };
        r.v.on_drop(&ev, &mut r.cx);
        r.settle();
        assert_eq!(r.v.view.as_ref().unwrap().files, vec!["notes.txt".to_string()]);
        let _ = std::fs::remove_file(f);
    }

    #[test]
    fn reduced_motion_plays_no_cues() {
        let mut r = Rig::new("chat-streaming");
        r.send_message("x");
        assert!(r.v.cues.is_empty(), "no enter cue under reduced motion");
    }

    #[test]
    fn animated_enter_plays_when_motion_is_on() {
        let mut r = Rig::new("chat-streaming");
        r.cx.reduced_motion = false;
        r.v.on_show(&mut r.cx);
        let ta = r.v.geo.ta;
        r.click(ta.center());
        r.type_text("x");
        r.v.event(&InputEvent::Key { code: KeyCode::Enter, mods: Modifiers::NONE, down: true, repeat: false }, &mut r.cx);
        r.frame(8.0);
        r.frame(8.0);
        r.frame(8.0);
        assert!(r.v.cues.xf("msg:user").alpha < 1.0, "the bubble enters with a fade");
    }

    // ------------------------------------------------------------------------------------------------ snapshots

    fn shoot_all(r: &mut Rig, name: &str) {
        for dark in [true, false] {
            for scale in [1.0, 2.0] {
                r.shoot(name, dark, scale);
            }
        }
    }

    #[test]
    fn snapshots() {
        let mut r = Rig::new("chat-empty");
        shoot_all(&mut r, "chat-empty");
        let mut r = Rig::new("chat-offline");
        shoot_all(&mut r, "chat-offline");
        let mut r = Rig::new("chat-answer");
        shoot_all(&mut r, "chat-answer");
        let mut r = Rig::new("chat-history-popover");
        r.click_key(K::Hist);
        r.run_frames(10);
        shoot_all(&mut r, "chat-history-popover");
        let mut r = Rig::new("chat-settings-popover");
        r.click_key(K::Gear);
        r.run_frames(10);
        shoot_all(&mut r, "chat-settings-popover");
        let mut r = Rig::new("chat-typing-dots");
        r.send_message("Объясни простыми словами, как работает VPN");
        r.run_frames(3);
        shoot_all(&mut r, "chat-typing-dots");
        let mut r = Rig::new("chat-streaming");
        r.send_message("Объясни простыми словами, как работает VPN");
        r.emit_scripted_events();
        r.run_frames(60);
        shoot_all(&mut r, "chat-streaming");
    }
}
