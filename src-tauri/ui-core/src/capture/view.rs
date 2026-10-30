//! The view-model of one capture overlay: everything a renderer needs to draw a frame, as plain data derived from the model.
//! One-to-one with the DOM of capture.html: `#frame` (the renderer's own bitmap), `#dim`, `#sel` + handles + `#size`, `#ann`
//! (ops + draft, clipped to the selection), `#hint`, `#bar`, the text input `#txt`, the loupe `#lens` and marker `#pix`.
use super::anim::Anim;
use super::geom::{Cursor, Handle, Pt, Rect, Scale};
use super::input::{Mode, Phase};
use super::loupe::LoupeView;
use super::markup::{DrawCmd, Tool};
use super::toolbar::ToolbarVm;
use serde::{Deserialize, Serialize};

/// One selection handle dot.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandleVm {
    pub handle: Handle,
    /// Centre of the 9 px dot, css px.
    pub at: Pt,
    pub cursor: Cursor,
}

/// `clip-path: inset(top right bottom left)` that confines the markup layer to the selection.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipInset {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

/// The selection box. Outside of it the frame is dimmed by 46 % black (`box-shadow: 0 0 0 100vmax rgba(0,0,0,.46)`), the box
/// has a 1 px `rgba(255,255,255,.92)` outline, and handles are white 9 px dots with a 1.5 px accent border.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionVm {
    pub rect: Rect,
    /// `1920 × 1080` in frame px (U+00D7).
    pub size_label: String,
    /// The label sits inside the box (top-left, 6 px in) when the selection is within 30 px of the top.
    pub size_inside: bool,
    /// Empty outside area mode (text / qr / record show no handles).
    pub handles: Vec<HandleVm>,
    pub clip: ClipInset,
}

/// One op ready to draw.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OpVm {
    pub id: u32,
    pub cmds: Vec<DrawCmd>,
    /// Picked with the select tool (extension): the renderer outlines it.
    pub selected: bool,
}

/// The text tool's input box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextVm {
    /// Top-left of the box, css px.
    pub pos: Pt,
    pub color: String,
    pub text: String,
    /// Caret as a char index into `text`.
    pub caret: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HintVm {
    pub text: String,
    /// False after the first pointer-down (it fades out: see the `HintFade` animation).
    pub visible: bool,
}

/// What stays on screen for the exit animations after a commit / cancel cleared the selection.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Ghost {
    pub selection: Rect,
    pub toolbar: Option<ToolbarVm>,
}

/// Everything for one frame.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CaptureView {
    pub mode: Mode,
    pub phase: Phase,
    /// Overlay size in css px and the css -> frame scale.
    pub viewport: (f64, f64),
    pub scale: Scale,
    pub hint: Option<HintVm>,
    /// Flat 38 % black over the whole frame (`#dim`): shown until a selection exists, never in colour mode.
    pub dim_full: bool,
    pub selection: Option<SelectionVm>,
    pub ops: Vec<OpVm>,
    pub draft: Option<OpVm>,
    pub text_edit: Option<TextVm>,
    pub toolbar: Option<ToolbarVm>,
    pub loupe: Option<LoupeView>,
    /// Where the lens box was before the last sample move: ease `box_pos` from here (see `AnimKind::LoupeFollow`).
    pub loupe_from: Option<Pt>,
    pub cursor: Cursor,
    /// Colour mode hides the system cursor (`cursor: none`); the marker + lens replace it.
    pub cursor_hidden: bool,
    pub tool: Tool,
    pub color: String,
    pub busy: bool,
    /// Running animations; sample each with `Anim::frame(now)`.
    pub anims: Vec<Anim>,
    pub ghost: Option<Ghost>,
}
