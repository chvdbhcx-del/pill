//! The markup toolbar of the area overlay as a view-model: ordered items with their rectangles, icons, translated tips, keyboard
//! shortcuts and enabled / on / hover / pressed state, plus the placement rule that puts the bar under (or over) the selection.
//!
//! Layout is the flex row of capture.css: 5 px padding + 1 px border, 2 px gap between children, 32 px buttons, 22 px swatches
//! (14 px dot), separators 1 px wide with 5 px margins each side. Default width is therefore 577 px by 44 px.
//! Order (capture.js `buildBar`): [select rect arrow pen blur text] | [5 swatches] | [undo] | [copy save record done cancel].
//! Extensions (off by default): extra tools after `text`, redo after undo, pin / shelf after record.
use super::geom::Rect;
use super::markup::{Tool, COLORS, CORE_TOOLS, EXTRA_TOOLS};
use crate::i18n::Tr;
use serde::{Deserialize, Serialize};

pub const BUTTON: f64 = 32.0;
pub const PAD: f64 = 5.0;
pub const BORDER: f64 = 1.0;
pub const GAP: f64 = 2.0;
pub const SEP_LINE: f64 = 1.0;
pub const SEP_MARGIN: f64 = 5.0;
pub const SEP_HEIGHT: f64 = 20.0;
pub const SWATCH_W: f64 = 22.0;
pub const SWATCH_DOT: f64 = 14.0;
pub const CORNER_RADIUS: f64 = 14.0;
/// Space between the selection and the bar (`g` in showBar) and the minimum distance to the viewport edge.
pub const BAR_GAP: f64 = 10.0;
pub const EDGE_MARGIN: f64 = 8.0;

/// One toolbar child.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemId {
    Tool(Tool),
    /// Index into [`COLORS`].
    Swatch(usize),
    Undo,
    Redo,
    Copy,
    Save,
    /// Record this area (`data-a="record"`, icon `video`).
    Record,
    /// Pin to screen (extension).
    Pin,
    /// Add to shelf (extension).
    Shelf,
    /// "Done: copy and save" (`data-a="both"`, icon `check`).
    Both,
    Cancel,
    Sep,
}

/// Visual variant of a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Variant {
    Normal,
    /// Accent-filled primary button (`.go`).
    Go,
    /// Turns red on hover (`.x`).
    Cancel,
    Swatch,
    Separator,
}

impl ItemId {
    /// The web `data-a` / `data-c` value.
    pub fn id(&self) -> String {
        match self {
            ItemId::Tool(t) => t.id().to_string(),
            ItemId::Swatch(i) => COLORS[*i % COLORS.len()].to_string(),
            ItemId::Undo => "undo".into(),
            ItemId::Redo => "redo".into(),
            ItemId::Copy => "copy".into(),
            ItemId::Save => "save".into(),
            ItemId::Record => "record".into(),
            ItemId::Pin => "pin".into(),
            ItemId::Shelf => "shelf".into(),
            ItemId::Both => "both".into(),
            ItemId::Cancel => "cancel".into(),
            ItemId::Sep => "sep".into(),
        }
    }
    /// Icon name (shared icon set of icons.js plus the page's own shapes).
    pub fn icon(&self) -> &'static str {
        match self {
            ItemId::Tool(t) => match t {
                Tool::Select => "select",
                Tool::Rect => "rect",
                Tool::Arrow => "arrow",
                Tool::Pen => "pen",
                Tool::Blur => "blur",
                Tool::Text => "text",
                Tool::Highlighter => "highlighter",
                Tool::Ellipse => "ellipse",
                Tool::Step => "step",
            },
            ItemId::Swatch(_) | ItemId::Sep => "",
            ItemId::Undo => "undo",
            ItemId::Redo => "redo",
            ItemId::Copy => "copy",
            ItemId::Save => "save",
            ItemId::Record => "video",
            ItemId::Pin => "pin",
            ItemId::Shelf => "shelf",
            ItemId::Both => "check",
            ItemId::Cancel => "x",
        }
    }
    pub fn variant(&self) -> Variant {
        match self {
            ItemId::Both => Variant::Go,
            ItemId::Cancel => Variant::Cancel,
            ItemId::Swatch(_) => Variant::Swatch,
            ItemId::Sep => Variant::Separator,
            _ => Variant::Normal,
        }
    }
    /// Keyboard shortcut shown in the tip and handled by the model (`None` = none).
    pub fn shortcut(&self) -> Option<&'static str> {
        match self {
            ItemId::Undo => Some("Ctrl+Z"),
            ItemId::Redo => Some("Ctrl+Y"),
            ItemId::Copy => Some("Ctrl+C"),
            ItemId::Save => Some("Ctrl+S"),
            ItemId::Both => Some("Enter"),
            ItemId::Cancel => Some("Esc"),
            _ => None,
        }
    }
    fn width(&self) -> f64 {
        match self {
            ItemId::Swatch(_) => SWATCH_W,
            ItemId::Sep => SEP_LINE + 2.0 * SEP_MARGIN,
            _ => BUTTON,
        }
    }
}

/// Translated tooltip (`TIPS` in capture.js; extension items have their own strings).
pub fn tip(item: &ItemId, tr: &dyn Tr) -> String {
    match item {
        ItemId::Tool(Tool::Select) => tr.t("Выбор", "Select"),
        ItemId::Tool(Tool::Rect) => tr.t("Рамка", "Rectangle"),
        ItemId::Tool(Tool::Arrow) => tr.t("Стрелка", "Arrow"),
        ItemId::Tool(Tool::Pen) => tr.t("Карандаш", "Pen"),
        ItemId::Tool(Tool::Blur) => tr.t("Скрыть (пиксели)", "Hide (pixelate)"),
        ItemId::Tool(Tool::Text) => tr.t("Текст", "Text"),
        ItemId::Tool(Tool::Highlighter) => tr.t("Маркер", "Highlighter"),
        ItemId::Tool(Tool::Ellipse) => tr.t("Овал", "Ellipse"),
        ItemId::Tool(Tool::Step) => tr.t("Нумерация", "Numbered step"),
        ItemId::Undo => tr.t("Отменить (Ctrl+Z)", "Undo (Ctrl+Z)"),
        ItemId::Redo => tr.t("Повторить (Ctrl+Y)", "Redo (Ctrl+Y)"),
        ItemId::Copy => tr.t("Копировать (Ctrl+C)", "Copy (Ctrl+C)"),
        ItemId::Save => tr.t("Сохранить (Ctrl+S)", "Save (Ctrl+S)"),
        ItemId::Record => tr.t("Записать эту область", "Record this area"),
        ItemId::Pin => tr.t("Закрепить на экране", "Pin to screen"),
        ItemId::Shelf => tr.t("На полку", "Add to shelf"),
        ItemId::Both => tr.t("Готово: копировать и сохранить (Enter)", "Done: copy and save (Enter)"),
        ItemId::Cancel => tr.t("Отмена (Esc)", "Cancel (Esc)"),
        ItemId::Swatch(i) => COLORS[*i % COLORS.len()].to_string(),
        ItemId::Sep => String::new(),
    }
}

/// Dynamic state the view needs from the model.
#[derive(Clone, Debug, Default)]
pub struct ToolbarState {
    pub tool: Option<Tool>,
    pub color: usize,
    pub can_undo: bool,
    pub can_redo: bool,
    /// A commit is in flight: nothing is clickable.
    pub busy: bool,
    /// Id (as [`ItemId::id`]) under the pointer / being pressed.
    pub hover: Option<String>,
    pub pressed: Option<String>,
}

/// One rendered child.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemVm {
    pub item: ItemId,
    pub id: String,
    pub icon: &'static str,
    pub tip: String,
    pub shortcut: Option<&'static str>,
    /// Relative to the bar's top-left corner.
    pub rect: Rect,
    pub variant: Variant,
    /// The active tool / selected swatch (`.on`).
    pub on: bool,
    pub disabled: bool,
    pub hover: bool,
    pub pressed: bool,
    /// Swatch colour (`--c`), empty for other items.
    pub color: String,
}

/// The whole bar, ready to draw.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolbarVm {
    pub visible: bool,
    /// Absolute css rectangle in the overlay.
    pub rect: Rect,
    pub corner_radius: f64,
    pub items: Vec<ItemVm>,
}

/// Static layout of the bar for one feature set.
#[derive(Clone, Debug, PartialEq)]
pub struct Toolbar {
    items: Vec<(ItemId, Rect)>,
    pub width: f64,
    pub height: f64,
}

impl Toolbar {
    /// Build the item list: `extended_tools` adds highlighter / ellipse / step and redo, `ext_actions` adds pin and shelf.
    pub fn new(extended_tools: bool, ext_actions: bool) -> Toolbar {
        let mut ids: Vec<ItemId> = CORE_TOOLS.iter().map(|t| ItemId::Tool(*t)).collect();
        if extended_tools {
            ids.extend(EXTRA_TOOLS.iter().map(|t| ItemId::Tool(*t)));
        }
        ids.push(ItemId::Sep);
        ids.extend((0..COLORS.len()).map(ItemId::Swatch));
        ids.push(ItemId::Sep);
        ids.push(ItemId::Undo);
        if extended_tools {
            ids.push(ItemId::Redo);
        }
        ids.push(ItemId::Sep);
        ids.push(ItemId::Copy);
        ids.push(ItemId::Save);
        ids.push(ItemId::Record);
        if ext_actions {
            ids.push(ItemId::Pin);
            ids.push(ItemId::Shelf);
        }
        ids.push(ItemId::Both);
        ids.push(ItemId::Cancel);
        let mut x = BORDER + PAD;
        let mut items = Vec::with_capacity(ids.len());
        for (k, id) in ids.into_iter().enumerate() {
            if k > 0 {
                x += GAP;
            }
            let w = id.width();
            let (y, h) = if id == ItemId::Sep { (BORDER + PAD + (BUTTON - SEP_HEIGHT) / 2.0, SEP_HEIGHT) } else { (BORDER + PAD, BUTTON) };
            let r = if id == ItemId::Sep { Rect::new(x + SEP_MARGIN, y, SEP_LINE, h) } else { Rect::new(x, y, w, h) };
            items.push((id, r));
            x += w;
        }
        Toolbar { items, width: x + PAD + BORDER, height: BUTTON + 2.0 * (PAD + BORDER) }
    }

    pub fn items(&self) -> impl Iterator<Item = &ItemId> {
        self.items.iter().map(|(i, _)| i)
    }

    /// The child under a point given relative to the bar origin (`None` over padding, gaps and separators).
    pub fn item_at(&self, rel: super::geom::Pt) -> Option<&ItemId> {
        self.items.iter().find(|(id, r)| *id != ItemId::Sep && r.contains(rel)).map(|(id, _)| id)
    }

    /// Full view at `origin` (top-left, css px).
    pub fn view(&self, origin: super::geom::Pt, visible: bool, st: &ToolbarState, tr: &dyn Tr) -> ToolbarVm {
        let items = self
            .items
            .iter()
            .map(|(item, rect)| {
                let id = item.id();
                let on = match item {
                    ItemId::Tool(t) => st.tool == Some(*t),
                    ItemId::Swatch(i) => *i == st.color,
                    _ => false,
                };
                let disabled = match item {
                    ItemId::Sep => false,
                    ItemId::Undo => !st.can_undo || st.busy,
                    ItemId::Redo => !st.can_redo || st.busy,
                    ItemId::Swatch(_) | ItemId::Tool(_) => false,
                    _ => st.busy,
                };
                ItemVm {
                    icon: item.icon(),
                    tip: tip(item, tr),
                    shortcut: item.shortcut(),
                    rect: *rect,
                    variant: item.variant(),
                    on,
                    disabled,
                    hover: !disabled && st.hover.as_deref() == Some(id.as_str()),
                    pressed: !disabled && st.pressed.as_deref() == Some(id.as_str()),
                    color: if let ItemId::Swatch(i) = item { COLORS[*i % COLORS.len()].to_string() } else { String::new() },
                    id,
                    item: item.clone(),
                }
            })
            .collect();
        ToolbarVm { visible, rect: Rect::new(origin.x, origin.y, self.width, self.height), corner_radius: CORNER_RADIUS, items }
    }
}

/// Where the bar goes (`showBar`): bottom-right aligned under the selection, flipped above when it would not fit, and as a last
/// resort tucked inside the selection's bottom edge. Always at least [`EDGE_MARGIN`] from the viewport.
pub fn position(sel: &Rect, bar_w: f64, bar_h: f64, vw: f64, vh: f64) -> super::geom::Pt {
    let x = (vw - bar_w - EDGE_MARGIN).min(sel.x + sel.w - bar_w).max(EDGE_MARGIN);
    let mut y = sel.y + sel.h + BAR_GAP;
    if y + bar_h > vh - EDGE_MARGIN {
        y = sel.y - bar_h - BAR_GAP;
    }
    if y < EDGE_MARGIN {
        y = EDGE_MARGIN.max(sel.y + sel.h - bar_h - BAR_GAP);
    }
    super::geom::Pt::new(x, y)
}

/// One keyboard shortcut of the overlay, for help text / docs / tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Shortcut {
    pub keys: &'static str,
    /// The mode(s) it applies in: `area`, `all` (every mode) or `color`.
    pub scope: &'static str,
    pub what: &'static str,
    /// True when the web page has it; false for native extensions.
    pub web: bool,
}

/// Every shortcut the model handles.
pub fn shortcuts() -> &'static [Shortcut] {
    const S: &[Shortcut] = &[
        Shortcut { keys: "Esc", scope: "all", what: "cancel", web: true },
        Shortcut { keys: "Right click", scope: "all", what: "cancel", web: true },
        Shortcut { keys: "Enter", scope: "area", what: "done (copy and save)", web: true },
        Shortcut { keys: "Enter", scope: "text/qr/record", what: "run on the selection (record: whole screen without one)", web: true },
        Shortcut { keys: "Enter", scope: "color", what: "pick the colour", web: true },
        Shortcut { keys: "Ctrl+Z", scope: "area", what: "undo", web: true },
        Shortcut { keys: "Ctrl+C", scope: "area", what: "copy (needs a selection)", web: true },
        Shortcut { keys: "Ctrl+S", scope: "area", what: "save (needs a selection)", web: true },
        Shortcut { keys: "Double click", scope: "area", what: "done, select tool only", web: true },
        Shortcut { keys: "Arrows", scope: "color", what: "move the sample 1 px (Shift: 10 px)", web: true },
        Shortcut { keys: "Click", scope: "color", what: "pick the colour", web: true },
        Shortcut { keys: "Arrows", scope: "area", what: "nudge the selection 1 px (Shift: 10 px)", web: false },
        Shortcut { keys: "Ctrl+Arrows", scope: "area", what: "resize the selection 1 px (Shift: 10 px)", web: false },
        Shortcut { keys: "Shift (while dragging)", scope: "area", what: "square selection", web: false },
        Shortcut { keys: "Ctrl+Y / Ctrl+Shift+Z", scope: "area", what: "redo", web: false },
        Shortcut { keys: "Delete", scope: "area", what: "delete the selected markup op", web: false },
    ];
    S
}
