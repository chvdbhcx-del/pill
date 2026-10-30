//! The native Settings window (wave "settings" of the WebView2 -> native rewrite): `SettingsView: View` over the pure model of
//! `pill_ui_core::settings` (declarative schema, `SettingsModel`, search, wizard, flows) and glasskit controls. See `docs/native-settings.md`.
//!
//! * `mod.rs`      the view struct and the `View` impl;  `open.rs` window creation, geometry memory, the `settingswin.rs` gate, the backend facade
//! * `nav.rs`      building / switching pages;  `paint.rs` the frame;  `events.rs` input routing;  `acts.rs` clicks, calls and their replies
//! * `page.rs`     page / row / control types;  `rows.rs` `cardpaint.rs` `ctlpaint.rs` `keys.rs` rows and their controls
//! * `chrome.rs`   title bar, sidebar, search;  `wizard.rs` first-run wizard;  `picker.rs` pet and island cards;  `custom.rs` the 12 `Custom` widgets
#![allow(dead_code, unused_imports, unused_variables, unused_assignments, unused_mut)] // consumers (the settingswin.rs gate) arrive in a later commit; tidy-up pass at the end

mod acts;
mod cardpaint;
mod chrome;
mod ctlpaint;
mod custom;
mod events;
mod fields;
mod icons;
mod keys;
mod nav;
mod open;
mod pal;
mod pagepaint;
mod paint;
mod page;
mod picker;
mod picker2;
mod picker3;
mod cw1;
mod cw2;
mod cw3;
mod cw3b;
mod customacts;
mod prim;
mod replies;
mod rows;
mod step;
mod txt;
mod ui;
mod wizard;

#[cfg(test)]
mod tests;

pub use open::{close_native, native_enabled, open_native};
pub use pal::settings_light;

use crate::ui::view::{Cx, Msg, Ticket, View};
use glasskit::canvas::Canvas;
use glasskit::ease::Tween;
use glasskit::geom::Rect;
use glasskit::input::{FocusChain, InputEvent, Response};
use glasskit::theme::Theme;
use pill_ui_core::settings::calls::BackendCall;
use pill_ui_core::settings::SettingsModel;
use std::collections::HashMap;

use custom::Data;
use icons::Icons;
use page::{Dyn, Page};
use pal::Pal;
use ui::Ui;

/// Window label (`Cx::caller`, the singleton key).
pub const LABEL: &str = "settings";

/// What a pending `call_async` was.
struct Pend {
    call: BackendCall,
    /// a row id / widget tag the answer belongs to
    tag: String,
}

/// The mouse is held on something that wants the rest of the gesture.
#[derive(Clone, Debug, PartialEq)]
enum Grab {
    Row(u32),
    Scrollbar,
    NavScroll,
}

/// A request to bring a row (or card) of the page into the middle of the window and flash it (search result, `?page=`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Goto {
    row_id: String,
    title: String,
}

/// Messages from the window hook / own workers, delivered as `Msg::Custom`.
pub(crate) enum Local {
    /// the window moved / resized / maximized (from `WinEvent::Geometry`)
    Geometry { maximized: bool },
    /// a periodic wake (status polling, caret blink)
    Tick,
    /// the file the user picked for the settings import
    Picked(Result<String, String>),
}

pub struct SettingsView {
    pub(crate) model: SettingsModel,
    th: Theme,
    pal: Pal,
    icons: Icons,
    loaded: bool,
    version: String,
    dir: String,
    size: (f32, f32),
    maximized: bool,
    caption_dirty: bool,
    now_ms: f64,
    reduced: bool,
    page: Page,
    leaving: Option<Page>,
    leave_t: Tween<f32>,
    enter_t: Tween<f32>,
    win_t: Tween<f32>,
    /// page requested while the old one slides out
    want_page: Option<String>,
    ui: Ui,
    dy: Dyn,
    data: Data,
    focus: FocusChain,
    fid_next: u32,
    pend: HashMap<u64, Pend>,
    rec_live: Vec<&'static str>,
    rec_t: Tween<f32>,
    side: chrome::Side,
    search: chrome::SearchUi,
    grab: Option<Grab>,
    goto: Option<Goto>,
    hl_card: Option<(String, f32)>,
    wiz: wizard::WizState,
    open_select: Option<u32>,
    ticker: Option<open::Ticker>,
    edits_due: bool,
    last_fx_size: (f32, f32),
    pets: picker::Pickers,
    caption: Rect,
    pending_changes: Vec<events::Change>,
}

impl SettingsView {
    pub fn new(cx: &mut Cx, args: &serde_json::Value) -> SettingsView {
        let th = Theme::settings_dark();
        let mut v = SettingsView {
            model: SettingsModel::new(&open::system_lang()),
            th,
            pal: pal::pal(&th),
            icons: Icons::new(),
            loaded: false,
            version: String::new(),
            dir: String::new(),
            size: (840.0, 760.0),
            maximized: false,
            caption_dirty: true,
            now_ms: 0.0,
            reduced: false,
            page: Page::empty(),
            leaving: None,
            leave_t: Tween::at_rest(1.0),
            enter_t: Tween::at_rest(1.0),
            win_t: Tween::at_rest(1.0),
            want_page: None,
            ui: Ui::default(),
            dy: Dyn::default(),
            data: Data::default(),
            focus: FocusChain::default(),
            fid_next: 100,
            pend: HashMap::new(),
            rec_live: Vec::new(),
            rec_t: Tween::at_rest(0.0),
            side: chrome::Side::new(),
            search: chrome::SearchUi::new(&th),
            grab: None,
            goto: None,
            hl_card: None,
            wiz: wizard::WizState::default(),
            open_select: None,
            ticker: None,
            edits_due: false,
            last_fx_size: (0.0, 0.0),
            pets: picker::Pickers::default(),
            caption: Rect::default(),
            pending_changes: Vec::new(),
        };
        v.start(cx, args);
        v
    }

    /// Subscribes and asks the backend for the first data; the page appears when `settings_get` answers.
    fn start(&mut self, cx: &mut Cx, args: &serde_json::Value) {
        cx.subscribe("settings:changed");
        cx.subscribe("settings:page");
        cx.subscribe("aisetup:progress");
        self.reduced = cx.reduced_motion;
        self.data.open_page = args.get("page").and_then(|p| p.as_str()).map(str::to_string);
        for c in [BackendCall::SettingsGet, BackendCall::SettingsInfo, BackendCall::AutostartGet, BackendCall::PillMonitors, BackendCall::MediaDevices, BackendCall::MediaCaptureDevices] {
            self.call(cx, c, "");
        }
    }

    /// Runs one backend call (or a `Ui*` effect) now. `tag` travels to the reply handler.
    pub(crate) fn call(&mut self, cx: &mut Cx, c: BackendCall, tag: &str) -> Option<Ticket> {
        if c.is_ui_effect() {
            self.ui_effect(cx, c);
            return None;
        }
        let t = cx.call_async(c.command(), c.args());
        self.pend.insert(t.0, Pend { call: c, tag: tag.to_string() });
        Some(t)
    }

    pub(crate) fn run(&mut self, cx: &mut Cx, calls: Vec<BackendCall>) {
        for c in calls {
            self.call(cx, c, "");
        }
    }

    pub(crate) fn lang(&self) -> pill_ui_core::settings::tr::Lang {
        self.model.lang()
    }

    pub(crate) fn reduce(&self) -> bool {
        self.reduced || self.model.reduce_motion()
    }
}

impl View for SettingsView {
    fn event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        self.now_ms = cx.now_ms;
        self.reduced = cx.reduced_motion;
        self.route_event(ev, cx)
    }

    fn message(&mut self, msg: Msg, cx: &mut Cx) -> Response {
        self.now_ms = cx.now_ms;
        self.on_msg(msg, cx);
        Response::redraw()
    }

    fn step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.now_ms = cx.now_ms;
        self.reduced = cx.reduced_motion;
        self.step_all(dt_ms as f32 / 1000.0, cx)
    }

    fn draw(&mut self, cv: &Canvas, theme: &Theme, rect: Rect, cx: &crate::ui::view::Cx) {
        self.paint(cv, theme, rect, cx);
    }

    fn on_show(&mut self, cx: &mut Cx) {
        self.win_t = Tween::new(0.0, 1.0, 0.14, glasskit::ease::curves::EASE_OUT);
        cx.request_frame();
    }

    fn on_hide(&mut self, cx: &mut Cx) {
        self.model.recorder.stop();
        self.stop_ticker();
        let _ = cx;
    }

    fn on_dpi(&mut self, _scale: f32, cx: &mut Cx) {
        cx.request_frame();
    }
}

#[cfg(test)]
mod tests2;
