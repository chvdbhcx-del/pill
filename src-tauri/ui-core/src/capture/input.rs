//! Vocabulary of the capture overlay model: modes, phases, input events, semantic actions and options.
use super::geom::{Pt, Snap};
use super::io::{BackendCall, CommitKind};
use super::markup::Tool;
use crate::i18n::Tr;
use serde::{Deserialize, Serialize};

/// What the overlay was started for (`capture_session.mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    /// Area screenshot with markup.
    Area,
    /// Whole-screen shot (never reaches an overlay; the backend handles it).
    Screen,
    /// OCR: select the text to read.
    Text,
    /// QR: select a code.
    Qr,
    /// Colour picker with the loupe.
    Color,
    /// Record an area (Enter = whole screen).
    Record,
    RecordScreen,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "area" => Mode::Area,
            "screen" => Mode::Screen,
            "text" => Mode::Text,
            "qr" => Mode::Qr,
            "color" | "picker" => Mode::Color,
            "record" => Mode::Record,
            "record_screen" => Mode::RecordScreen,
            _ => return None,
        })
    }
    /// The `data-mode` / session name.
    pub fn id(self) -> &'static str {
        match self {
            Mode::Area => "area",
            Mode::Screen => "screen",
            Mode::Text => "text",
            Mode::Qr => "qr",
            Mode::Color => "color",
            Mode::Record => "record",
            Mode::RecordScreen => "record_screen",
        }
    }
    /// True for the modes that show a hint pill.
    pub fn has_hint(self) -> bool {
        !matches!(self, Mode::Screen | Mode::RecordScreen)
    }
    /// The hint pill text (`HINT` in capture.js); `None` for modes without one.
    pub fn hint(self, tr: &dyn Tr) -> Option<String> {
        Some(match self {
            Mode::Area => tr.t("Выдели область · Esc — отмена", "Drag to select an area · Esc to cancel"),
            Mode::Text => tr.t("Выдели текст на экране · Esc — отмена", "Select the text to read · Esc to cancel"),
            Mode::Qr => tr.t("Выдели QR-код · Esc — отмена", "Select a QR code · Esc to cancel"),
            Mode::Record => tr.t("Выдели область записи · Enter — весь экран · Esc — отмена", "Select the area to record · Enter for the whole screen · Esc to cancel"),
            Mode::Color => tr.t("Клик — скопировать цвет · стрелки — на 1 пиксель · Esc — отмена", "Click to copy the colour · arrows move 1 pixel · Esc to cancel"),
            Mode::Screen | Mode::RecordScreen => return None,
        })
    }
}

/// Where the selection state machine is. Derived, never stored: `Idle` -> `Dragging` -> `Adjusting` -> `Committed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Armed (or between sessions) with no selection.
    Idle,
    /// A pointer drag is in progress: new selection, move, resize, or drawing a markup op.
    Dragging,
    /// A selection exists and nothing is being dragged; handles, toolbar and markup are live.
    Adjusting,
    /// A commit was sent; the overlay is closing.
    Committed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Button {
    Left,
    Middle,
    Right,
}

/// Keys the overlay cares about; everything else arrives as [`Key::Char`] or is ignored by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    Escape,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Char(char),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub meta: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { shift: false, ctrl: false, alt: false, meta: false };
    pub const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false, meta: false };
    pub const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false, meta: false };
    /// Ctrl or Cmd (`e.ctrlKey || e.metaKey`).
    pub fn command(&self) -> bool {
        self.ctrl || self.meta
    }
}

/// Events the host feeds in. Pointer positions are css px inside this overlay's viewport (`clientX / clientY`).
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// `capture:begin {id}`: fetch the session and the frame.
    Begin,
    /// `capture:end`: drop everything.
    End,
    PointerDown { pos: Pt, button: Button, mods: Mods },
    PointerMove { pos: Pt, mods: Mods },
    PointerUp { pos: Pt, button: Button, mods: Mods },
    DoubleClick { pos: Pt },
    ContextMenu,
    Key { key: Key, mods: Mods },
    /// Committed text from the host's text input (IME / paste) while the text tool is open.
    Text(String),
    /// The overlay window was resized.
    Resize { vw: f64, vh: f64 },
}

/// Semantic things the model does; `on_input` returns the ones it performed, `apply` performs one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    SetTool(Tool),
    /// Index into the palette.
    SetColor(usize),
    Undo,
    Redo,
    /// Delete the markup op selected with the select tool (extension).
    DeleteOp,
    /// Send a commit (Copy, Save, Both, Record, Run, Pick).
    Commit(CommitKind),
    /// Esc, right click, Cancel button.
    Cancel,
    /// Arrow-key move of the selection by whole frame pixels.
    Nudge { dx: i32, dy: i32, big: bool },
    /// Ctrl+Arrow resize.
    NudgeSize { dw: i32, dh: i32, big: bool },
    /// Save, then pin the file to the screen (extension).
    PinToScreen,
    /// Save, then put the file on the shelf (extension).
    AddToShelf,
}

/// Follow-up work after a save, for the host to do when `capture:done` brings the path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfterSave {
    Pin,
    Shelf,
}

impl AfterSave {
    /// The call to make once the saved file's path is known.
    pub fn call(self, path: &str) -> BackendCall {
        match self {
            AfterSave::Pin => BackendCall::pin(path),
            AfterSave::Shelf => BackendCall::shelf_add(&[path.to_string()]),
        }
    }
}

/// What `on_input` reports back to the host.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reaction {
    pub actions: Vec<Action>,
    /// Something visible changed; redraw.
    pub redraw: bool,
    /// The host should swallow the event (`e.preventDefault()` in the page).
    pub prevent_default: bool,
}

/// Behaviour switches. [`Options::default`] keeps the web page's behaviour and adds only the harmless native extensions
/// (nudge, Shift = square); [`Options::web_parity`] turns every extension off.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// Reduce motion: animations are instant.
    pub reduce_motion: bool,
    /// Highlighter / ellipse / numbered steps in the toolbar, redo button.
    pub extended_tools: bool,
    /// Pin and shelf buttons (they save first, then the host acts on the path).
    pub ext_actions: bool,
    /// Select tool picks and moves markup ops instead of only moving the selection.
    pub edit_ops: bool,
    /// Arrow keys nudge the selection, Ctrl+Arrows resize it.
    pub nudge: bool,
    /// Holding Shift while dragging a selection keeps it square.
    pub shift_square: bool,
    /// Edge snapping (off by default).
    pub snap: Option<Snap>,
    /// Lock the selection to `w / h` while dragging (`None` = free).
    pub aspect: Option<f64>,
}

impl Default for Options {
    fn default() -> Options {
        Options { reduce_motion: false, extended_tools: false, ext_actions: false, edit_ops: false, nudge: true, shift_square: true, snap: None, aspect: None }
    }
}

impl Options {
    /// Exactly the web page: no extension.
    pub fn web_parity() -> Options {
        Options { nudge: false, shift_square: false, ..Options::default() }
    }
}
