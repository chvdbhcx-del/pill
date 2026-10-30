//! Markup model of the area-capture overlay: tools, colours, annotation ops (stored in frame px), the draw list a renderer
//! executes (`DrawCmd`, a port of `drawOp` in capture.js), undo / redo history, and hit-testing / moving of ops.
//!
//! The web page has six tools (select, rect, arrow, pen, blur = pixelate, text). `Highlighter`, `Ellipse` and `Step` (numbered
//! callouts) are native extensions: fully modelled here, but only offered by the toolbar when `Options::extended_tools` is on.
//! Ops live in *frame* px (the page converts css -> frame with `fp()` when it creates them), so the flattened PNG is a plain
//! crop of the frame with the ops drawn on top at the same coordinates.
use super::geom::{IRect, Pt, Rect, Scale};
use serde::{Deserialize, Serialize};

/// Palette of the toolbar swatches, in order; the first is the default (`COLORS` in capture.js).
pub const COLORS: [&str; 5] = ["#ff3b30", "#ffcc00", "#34c759", "#0a84ff", "#ffffff"];
/// Stroke width in css px before scaling by `rx` (`lw()` = `3 * rx()`).
pub const STROKE_CSS: f64 = 3.0;
/// Minimum drag length (frame px) for a rect / arrow / blur to be kept (`> 4` in `finishDraw`).
pub const MIN_OP_LEN: f64 = 4.0;
/// Pixelate block: `max(6, round(10 * rx))` frame px.
pub const PIXELATE_MIN_BLOCK: i32 = 6;
pub const PIXELATE_CSS_BLOCK: f64 = 10.0;
/// Text op font: weight 600, `16 * rx` px.
pub const TEXT_CSS_PX: f64 = 16.0;
/// Arrow head: length `min(max(14 * rx, lw * 4), 0.6 * length)`, wings at +-0.45 rad.
pub const ARROW_HEAD_CSS: f64 = 14.0;
pub const ARROW_ANGLE: f64 = 0.45;
/// Undo depth kept (the page was unbounded; a bound keeps memory sane for a pen scribble marathon).
pub const HISTORY_CAP: usize = 200;

/// Annotation tools. The toolbar order is [`CORE_TOOLS`], then (extended) [`EXTRA_TOOLS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    Select,
    Rect,
    Arrow,
    Pen,
    Blur,
    Text,
    Highlighter,
    Ellipse,
    Step,
}

pub const CORE_TOOLS: [Tool; 6] = [Tool::Select, Tool::Rect, Tool::Arrow, Tool::Pen, Tool::Blur, Tool::Text];
pub const EXTRA_TOOLS: [Tool; 3] = [Tool::Highlighter, Tool::Ellipse, Tool::Step];

impl Tool {
    /// The `data-a` id of the web toolbar button.
    pub fn id(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Rect => "rect",
            Tool::Arrow => "arrow",
            Tool::Pen => "pen",
            Tool::Blur => "blur",
            Tool::Text => "text",
            Tool::Highlighter => "highlighter",
            Tool::Ellipse => "ellipse",
            Tool::Step => "step",
        }
    }
    pub fn from_id(s: &str) -> Option<Tool> {
        CORE_TOOLS.iter().chain(EXTRA_TOOLS.iter()).copied().find(|t| t.id() == s)
    }
    /// True for the six tools the web page ships.
    pub fn is_core(self) -> bool {
        CORE_TOOLS.contains(&self)
    }
    /// Tools that create an op by dragging (everything except select / text / step).
    pub fn drags(self) -> bool {
        matches!(self, Tool::Rect | Tool::Arrow | Tool::Pen | Tool::Blur | Tool::Highlighter | Tool::Ellipse)
    }
}

/// The geometry of one annotation, frame px.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum OpKind {
    Rect { a: Pt, b: Pt },
    Ellipse { a: Pt, b: Pt },
    Arrow { a: Pt, b: Pt },
    Pen { pts: Vec<Pt> },
    Highlighter { pts: Vec<Pt> },
    /// Pixelate the frame under the box (irreversible, unlike a soft blur).
    Blur { a: Pt, b: Pt },
    /// Text anchored at its top-left corner.
    Text { s: String, a: Pt },
    /// Numbered callout `n` centred on `c`.
    Step { n: u32, c: Pt },
}

/// One annotation. `id` is unique within a session and stable across undo / redo / move.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Op {
    pub id: u32,
    pub color: String,
    pub kind: OpKind,
}

/// Highlighter: 5x the stroke, 35 % alpha.
pub const HIGHLIGHT_WIDTH_MUL: f64 = 5.0;
pub const HIGHLIGHT_ALPHA: f64 = 0.35;
/// Step badge radius in css px and its number size.
pub const STEP_RADIUS_CSS: f64 = 12.0;

/// What a renderer has to paint for an op. Strokes use round caps and joins; coordinates are frame px.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum DrawCmd {
    /// Open path through `pts` (a single point is drawn as a 0.1 px segment, like the web page, so a click leaves a dot).
    Polyline { pts: Vec<Pt>, color: String, width: f64, alpha: f64 },
    StrokeRect { rect: Rect, color: String, width: f64 },
    StrokeEllipse { rect: Rect, color: String, width: f64 },
    /// Separate line segments sharing one stroke (arrow shaft + two wings).
    Segments { segs: Vec<(Pt, Pt)>, color: String, width: f64 },
    /// Replace the frame region with its pixelated copy; `block` frame px per cell.
    Pixelate { rect: IRect, block: i32 },
    /// Text with a dark outline under the fill: `outline_width` stroke in `outline`, then the fill in `color`.
    Text { text: String, at: Pt, color: String, font_px: f64, weight: u16, outline: &'static str, outline_width: f64 },
    /// Filled disc with a centred label (step callout).
    Disc { center: Pt, radius: f64, color: String, label: String, label_px: f64 },
}

fn corners(a: Pt, b: Pt) -> Rect {
    Rect::from_corners(a, b)
}

fn dist_to_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    if l2 == 0.0 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0);
    p.dist(Pt::new(a.x + t * dx, a.y + t * dy))
}

fn polyline_distance(p: Pt, pts: &[Pt]) -> f64 {
    match pts {
        [] => f64::INFINITY,
        [one] => p.dist(*one),
        _ => pts.windows(2).map(|w| dist_to_segment(p, w[0], w[1])).fold(f64::INFINITY, f64::min),
    }
}

impl Op {
    /// Start a draft for a dragging tool at frame point `p` (the page's `startDraw`). `None` for select / text / step.
    pub fn draft(id: u32, tool: Tool, color: &str, p: Pt) -> Option<Op> {
        let kind = match tool {
            Tool::Pen => OpKind::Pen { pts: vec![p] },
            Tool::Highlighter => OpKind::Highlighter { pts: vec![p] },
            Tool::Rect => OpKind::Rect { a: p, b: p },
            Tool::Ellipse => OpKind::Ellipse { a: p, b: p },
            Tool::Arrow => OpKind::Arrow { a: p, b: p },
            Tool::Blur => OpKind::Blur { a: p, b: p },
            Tool::Select | Tool::Text | Tool::Step => return None,
        };
        Some(Op { id, color: color.to_string(), kind })
    }

    /// Drag to `q` (frame px, already clamped into the selection): pens append, two-point shapes move their second corner.
    pub fn extend(&mut self, q: Pt) {
        match &mut self.kind {
            OpKind::Pen { pts } | OpKind::Highlighter { pts } => pts.push(q),
            OpKind::Rect { b, .. } | OpKind::Ellipse { b, .. } | OpKind::Arrow { b, .. } | OpKind::Blur { b, .. } => *b = q,
            OpKind::Text { .. } | OpKind::Step { .. } => {}
        }
    }

    /// `finishDraw` keeps pens always and shapes only when dragged further than [`MIN_OP_LEN`].
    pub fn is_substantial(&self) -> bool {
        match &self.kind {
            OpKind::Pen { .. } | OpKind::Highlighter { .. } | OpKind::Text { .. } | OpKind::Step { .. } => true,
            OpKind::Rect { a, b } | OpKind::Ellipse { a, b } | OpKind::Arrow { a, b } | OpKind::Blur { a, b } => a.dist(*b) > MIN_OP_LEN,
        }
    }

    /// Bounding box in frame px (stroke width excluded). Text width is estimated at 0.6 em per char - the renderer owns real
    /// font metrics; this is only good enough for hit-testing.
    pub fn bounds(&self, scale: Scale) -> Rect {
        match &self.kind {
            OpKind::Rect { a, b } | OpKind::Ellipse { a, b } | OpKind::Arrow { a, b } | OpKind::Blur { a, b } => corners(*a, *b),
            OpKind::Pen { pts } | OpKind::Highlighter { pts } => {
                let (mut l, mut t, mut r, mut b) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for p in pts {
                    l = l.min(p.x);
                    t = t.min(p.y);
                    r = r.max(p.x);
                    b = b.max(p.y);
                }
                if pts.is_empty() { Rect::default() } else { Rect::new(l, t, r - l, b - t) }
            }
            OpKind::Text { s, a } => {
                let px = TEXT_CSS_PX * scale.rx;
                Rect::new(a.x, a.y, s.chars().count() as f64 * px * 0.6, px * 1.25)
            }
            OpKind::Step { c, .. } => {
                let r = STEP_RADIUS_CSS * scale.rx;
                Rect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r)
            }
        }
    }

    /// Does frame point `p` hit this op? Strokes hit within half a line width plus `tol`; filled things (blur, text, step) hit
    /// anywhere inside their box.
    pub fn hit(&self, p: Pt, tol: f64, scale: Scale) -> bool {
        let half = STROKE_CSS * scale.rx / 2.0 + tol;
        match &self.kind {
            OpKind::Rect { a, b } => {
                let r = corners(*a, *b);
                let (tl, tr, br, bl) = (Pt::new(r.x, r.y), Pt::new(r.right(), r.y), Pt::new(r.right(), r.bottom()), Pt::new(r.x, r.bottom()));
                polyline_distance(p, &[tl, tr, br, bl, tl]) <= half
            }
            OpKind::Ellipse { a, b } => {
                let r = corners(*a, *b);
                let (rx, ry) = (r.w / 2.0, r.h / 2.0);
                if rx <= 0.0 || ry <= 0.0 {
                    return r.contains(p);
                }
                let c = r.center();
                // normalised radius: 1.0 on the outline; compare in px using the smaller semi-axis as the unit
                let k = (((p.x - c.x) / rx).powi(2) + ((p.y - c.y) / ry).powi(2)).sqrt();
                (k - 1.0).abs() * rx.min(ry) <= half
            }
            OpKind::Arrow { a, b } => dist_to_segment(p, *a, *b) <= half,
            OpKind::Pen { pts } => polyline_distance(p, pts) <= half,
            OpKind::Highlighter { pts } => polyline_distance(p, pts) <= STROKE_CSS * HIGHLIGHT_WIDTH_MUL * scale.rx / 2.0 + tol,
            OpKind::Blur { .. } | OpKind::Text { .. } => {
                let b = self.bounds(scale);
                p.x >= b.x - tol && p.x <= b.right() + tol && p.y >= b.y - tol && p.y <= b.bottom() + tol
            }
            OpKind::Step { c, .. } => p.dist(*c) <= STEP_RADIUS_CSS * scale.rx + tol,
        }
    }

    /// Translate by a frame-px delta.
    pub fn move_by(&mut self, dx: f64, dy: f64) {
        let mv = |p: &mut Pt| {
            p.x += dx;
            p.y += dy;
        };
        match &mut self.kind {
            OpKind::Rect { a, b } | OpKind::Ellipse { a, b } | OpKind::Arrow { a, b } | OpKind::Blur { a, b } => {
                mv(a);
                mv(b);
            }
            OpKind::Pen { pts } | OpKind::Highlighter { pts } => pts.iter_mut().for_each(mv),
            OpKind::Text { a, .. } => mv(a),
            OpKind::Step { c, .. } => mv(c),
        }
    }

    /// The draw list for this op at the given scale (port of `drawOp`).
    pub fn draw_cmds(&self, frame_w: i32, frame_h: i32, scale: Scale) -> Vec<DrawCmd> {
        let lw = STROKE_CSS * scale.rx;
        let c = self.color.clone();
        match &self.kind {
            OpKind::Rect { a, b } => vec![DrawCmd::StrokeRect { rect: corners(*a, *b), color: c, width: lw }],
            OpKind::Ellipse { a, b } => vec![DrawCmd::StrokeEllipse { rect: corners(*a, *b), color: c, width: lw }],
            OpKind::Arrow { a, b } => {
                let ang = (b.y - a.y).atan2(b.x - a.x);
                let n = (ARROW_HEAD_CSS * scale.rx).max(lw * 4.0).min(a.dist(*b) * 0.6);
                let wing = |d: f64| Pt::new(b.x - n * (ang + d).cos(), b.y - n * (ang + d).sin());
                vec![DrawCmd::Segments { segs: vec![(*a, *b), (*b, wing(-ARROW_ANGLE)), (*b, wing(ARROW_ANGLE))], color: c, width: lw }]
            }
            OpKind::Pen { pts } => vec![DrawCmd::Polyline { pts: pts.clone(), color: c, width: lw, alpha: 1.0 }],
            OpKind::Highlighter { pts } => vec![DrawCmd::Polyline { pts: pts.clone(), color: c, width: lw * HIGHLIGHT_WIDTH_MUL, alpha: HIGHLIGHT_ALPHA }],
            OpKind::Blur { a, b } => {
                let x = a.x.min(b.x).round() as i32;
                let y = a.y.min(b.y).round() as i32;
                let w = (b.x - a.x).abs().round() as i32;
                let h = (b.y - a.y).abs().round() as i32;
                if w > 1 && h > 1 {
                    let block = PIXELATE_MIN_BLOCK.max((PIXELATE_CSS_BLOCK * scale.rx).round() as i32);
                    // keep the region inside the frame so the renderer never samples out of range
                    let (x0, y0) = (x.clamp(0, frame_w), y.clamp(0, frame_h));
                    let (x1, y1) = ((x + w).clamp(0, frame_w), (y + h).clamp(0, frame_h));
                    if x1 - x0 > 1 && y1 - y0 > 1 {
                        return vec![DrawCmd::Pixelate { rect: IRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }, block }];
                    }
                }
                Vec::new()
            }
            OpKind::Text { s, a } => vec![DrawCmd::Text {
                text: s.clone(),
                at: *a,
                color: c,
                font_px: (TEXT_CSS_PX * scale.rx).round(),
                weight: 600,
                outline: "rgba(0,0,0,.85)",
                outline_width: 3.0f64.max(3.0 * scale.rx),
            }],
            OpKind::Step { n, c: center } => vec![DrawCmd::Disc {
                center: *center,
                radius: STEP_RADIUS_CSS * scale.rx,
                color: c,
                label: n.to_string(),
                label_px: (14.0 * scale.rx).round(),
            }],
        }
    }
}

/// Font stack of the text op (`g.font` in capture.js).
pub const TEXT_FONT_STACK: &str = "\"Medeo Sans\",\"Segoe UI Variable Text\",\"Segoe UI\",system-ui,sans-serif";

/// Ops plus undo / redo. Every mutation takes a snapshot, so add, move and delete all undo the same way; undoing an add is the
/// page's `S.ops.pop()`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    ops: Vec<Op>,
    undo: Vec<Vec<Op>>,
    redo: Vec<Vec<Op>>,
    next_id: u32,
}

impl History {
    pub fn new() -> History {
        History::default()
    }
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
    pub fn len(&self) -> usize {
        self.ops.len()
    }
    /// Reserve an id for a new op / draft.
    pub fn fresh_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }
    fn snapshot(&mut self) {
        self.undo.push(self.ops.clone());
        if self.undo.len() > HISTORY_CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    /// Add a finished op.
    pub fn push(&mut self, op: Op) {
        self.snapshot();
        self.ops.push(op);
    }
    /// The page's undo button is enabled iff there is something to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self) -> bool {
        match self.undo.pop() {
            Some(prev) => {
                self.redo.push(std::mem::replace(&mut self.ops, prev));
                true
            }
            None => false,
        }
    }
    pub fn redo(&mut self) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(&mut self.ops, next));
                true
            }
            None => false,
        }
    }
    /// Drop everything (new selection, new session).
    pub fn clear(&mut self) {
        self.ops.clear();
        self.undo.clear();
        self.redo.clear();
    }
    /// Topmost op under frame point `p` (later ops are drawn on top).
    pub fn hit_test(&self, p: Pt, tol: f64, scale: Scale) -> Option<u32> {
        self.ops.iter().rev().find(|o| o.hit(p, tol, scale)).map(|o| o.id)
    }
    pub fn get(&self, id: u32) -> Option<&Op> {
        self.ops.iter().find(|o| o.id == id)
    }
    /// Move an op by a frame-px delta as one undoable step.
    pub fn move_op(&mut self, id: u32, dx: f64, dy: f64) -> bool {
        if self.get(id).is_none() || (dx == 0.0 && dy == 0.0) {
            return false;
        }
        self.snapshot();
        if let Some(o) = self.ops.iter_mut().find(|o| o.id == id) {
            o.move_by(dx, dy);
        }
        true
    }
    /// Move without a snapshot (live drag); call [`History::begin_edit`] once before the drag starts.
    pub fn move_op_live(&mut self, id: u32, dx: f64, dy: f64) {
        if let Some(o) = self.ops.iter_mut().find(|o| o.id == id) {
            o.move_by(dx, dy);
        }
    }
    /// Take the undo snapshot for a multi-event edit (a drag).
    pub fn begin_edit(&mut self) {
        self.snapshot();
    }
    /// A drag that moved nothing must not leave an undo step behind.
    pub fn cancel_edit(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.ops = prev;
        }
    }
    pub fn remove(&mut self, id: u32) -> bool {
        if self.get(id).is_none() {
            return false;
        }
        self.snapshot();
        self.ops.retain(|o| o.id != id);
        true
    }
    /// Number the next step callout: 1 + the step ops present.
    pub fn next_step(&self) -> u32 {
        self.ops.iter().filter(|o| matches!(o.kind, OpKind::Step { .. })).count() as u32 + 1
    }
}
