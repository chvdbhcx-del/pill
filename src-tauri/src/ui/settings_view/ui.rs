//! A tiny immediate-mode layer for everything in the window that is not a model-driven control: buttons, chips, cards, list items, links.
//!
//! `draw` registers a [`Hit`] for each clickable thing (window space for the chrome, content space for the page); `event` hit-tests the list of
//! the previous frame. Hover / press animation lives in a `glasskit::controls::Interact` per key, buttons are real `glasskit::controls::Button`s
//! kept by key so their press spring is the toolkit's own.
use glasskit::controls::{Button, ButtonKind, Interact, Widget};
use glasskit::geom::{Pt, Rect};
use glasskit::input::{Cursor, InputEvent, MouseButton, Response};
use std::collections::HashMap;

/// What a click does. Payloads are owned strings: the hit list is rebuilt every frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    None,
    // window chrome
    CapMin,
    CapMax,
    CapClose,
    Nav(String),
    Quit,
    SearchPick(usize),
    // rows
    RowButton(String),
    SecretSave(String),
    SecretRemove(String),
    Day(String),
    // wizard
    WizSkip,
    WizBack,
    WizNext,
    WizOpen(&'static str),
    // character page
    Reaction(&'static str),
    PetCard(String),
    StyleCard(String),
    // providers
    Provider(&'static str, String),
    OpenLink(String),
    // weather
    CityPick(usize),
    CitySearch,
    // profiles
    ProfileSave,
    ProfileApply(String),
    ProfileDelete(String),
    // clipboard
    WipeAsk,
    WipeYes,
    WipeNo,
    // Everything
    EvDownload,
    EvRun,
    JobCancel,
    // AI setup
    WhisperModel(String),
    WhisperPrimary,
    WhisperSwitch,
    WhisperDelete,
    ServerUse(String, String),
    OllamaPrimary,
    ModelPull(String),
    AiLater,
    // general / updates
    Export,
    Import,
    Update,
    Disclosure,
    // hotkeys
    Recorder(String),
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub key: u64,
    pub rect: Rect,
    pub act: Act,
    pub enabled: bool,
    /// registered in content (scrolling) space; false = window space
    pub page: bool,
    pub button: bool,
}

#[derive(Default)]
pub struct Ui {
    pub hits: Vec<Hit>,
    it: HashMap<u64, Interact>,
    btn: HashMap<u64, Button>,
    pub hover: Option<u64>,
    pub down: Option<u64>,
    /// the window origin of the page content (scroll offset subtracted), set by the view before event routing
    pub page_origin: Pt,
    /// the visible part of the page content in window space
    pub page_clip: Rect,
}

pub fn key(s: &str) -> u64 {
    // FNV-1a: stable keys from labels without allocating a registry
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl Ui {
    pub fn begin(&mut self) {
        self.hits.clear();
    }

    pub fn add(&mut self, key: u64, rect: Rect, act: Act, page: bool, button: bool, enabled: bool) {
        self.hits.push(Hit { key, rect, act, enabled, page, button });
    }

    /// The interaction state of a plain clickable (created on first use).
    pub fn it(&mut self, key: u64) -> &mut Interact {
        self.it.entry(key).or_default()
    }

    pub fn hover_of(&self, key: u64) -> f32 {
        self.it.get(&key).map(|i| i.hover()).unwrap_or(0.0)
    }
    pub fn press_of(&self, key: u64) -> f32 {
        self.it.get(&key).map(|i| i.press()).unwrap_or(0.0)
    }

    /// The `Button` for `key`, made on first use. `mk` builds it.
    pub fn button(&mut self, key: u64, mk: impl FnOnce() -> Button) -> &mut Button {
        self.btn.entry(key).or_insert_with(mk)
    }

    pub fn has_button(&self, key: u64) -> bool {
        self.btn.contains_key(&key)
    }

    pub fn set_btn_kind(&mut self, key: u64, kind: ButtonKind, label: &str, small: bool) {
        if let Some(b) = self.btn.get_mut(&key) {
            b.kind = kind;
            b.small = small;
            if b.label != label {
                b.label = label.to_string();
            }
        }
    }

    /// Step every animation; true while one moves.
    pub fn step(&mut self, dt: f32) -> bool {
        let mut busy = false;
        for i in self.it.values_mut() {
            busy |= i.step(dt);
        }
        for b in self.btn.values_mut() {
            busy |= b.step(dt);
        }
        busy
    }

    fn pos_in(&self, h: &Hit, p: Pt) -> bool {
        if h.page {
            if !self.page_clip.contains(p.0, p.1) {
                return false;
            }
            let q = (p.0 - self.page_origin.0, p.1 - self.page_origin.1);
            h.rect.contains(q.0, q.1)
        } else {
            h.rect.contains(p.0, p.1)
        }
    }

    fn rect_win(&self, h: &Hit) -> Rect {
        if h.page {
            h.rect.offset(self.page_origin.0, self.page_origin.1)
        } else {
            h.rect
        }
    }

    pub fn topmost(&self, p: Pt) -> Option<usize> {
        self.hits.iter().rposition(|h| h.enabled && self.pos_in(h, p))
    }

    /// Feed a pointer event. Returns (response, the act of a completed click).
    pub fn pointer(&mut self, ev: &InputEvent) -> (Response, Option<Act>) {
        let mut r = Response::NONE;
        let mut fired = None;
        match ev {
            InputEvent::MouseMove { pos, .. } => {
                let top = self.topmost(*pos);
                let hk = top.map(|i| self.hits[i].key);
                if hk != self.hover {
                    self.hover = hk;
                    r.redraw = true;
                }
                if hk.is_some() {
                    r.cursor = Some(Cursor::Hand);
                }
                self.sync_hover();
            }
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => {
                if let Some(i) = self.topmost(*pos) {
                    let k = self.hits[i].key;
                    self.down = Some(k);
                    if !self.hits[i].button {
                        self.it(k).set_press(true);
                    }
                    r.consumed = true;
                    r.redraw = true;
                    r.capture = true;
                }
                self.feed_buttons(ev);
            }
            InputEvent::MouseUp { pos, button: MouseButton::Left, .. } => {
                if let Some(k) = self.down.take() {
                    self.it(k).set_press(false);
                    r.redraw = true;
                    r.consumed = true;
                    if let Some(i) = self.topmost(*pos) {
                        if self.hits[i].key == k {
                            fired = Some(self.hits[i].act.clone());
                        }
                    }
                }
                self.feed_buttons(ev);
            }
            InputEvent::MouseLeave => {
                self.hover = None;
                for i in self.it.values_mut() {
                    i.set_hover(false);
                }
                self.feed_buttons(ev);
                r.redraw = true;
            }
            _ => {}
        }
        if matches!(ev, InputEvent::MouseMove { .. }) {
            self.feed_buttons(ev);
        }
        (r, fired)
    }

    /// Plain clickables: hover follows the topmost hit; buttons track their own pointer.
    fn sync_hover(&mut self) {
        let hv = self.hover;
        for (k, i) in self.it.iter_mut() {
            i.set_hover(hv == Some(*k));
        }
    }

    /// Buttons are real widgets: give each its pointer events with its current rectangle (window space).
    fn feed_buttons(&mut self, ev: &InputEvent) {
        let rects: Vec<(u64, Rect)> = self.hits.iter().filter(|h| h.button).map(|h| (h.key, self.rect_win(h))).collect();
        for (k, r) in rects {
            if let Some(b) = self.btn.get_mut(&k) {
                b.event(ev, r);
            }
        }
    }

    /// Release hover and press when the page changes under the pointer.
    pub fn reset(&mut self) {
        self.hover = None;
        self.down = None;
        for i in self.it.values_mut() {
            i.set_hover(false);
            i.set_press(false);
        }
    }
}
