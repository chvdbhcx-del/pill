//! View-model of the uninstall panel and its status strip: plain data the renderer draws with glasskit controls. The renderer sends back
//! `Control` presses (`UninstallUi::press`), never paths or indexes of its own: items are addressed by their position in `Checklist::picks`.

use super::state::{Phase, StripKind};
use crate::bar::anim::Target;

/// A control the user can press in the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// the red «Удалить» of the confirm card (starts the uninstaller)
    Start,
    /// the «Silent» tick of the confirm card
    ToggleSilent,
    Cancel,
    Close,
    CheckAgain,
    StopWait,
    Hide,
    StopScan,
    SelectAllCertain,
    DeleteSelected,
    AdminScan,
    AdminDelete,
    OpenBackups,
    /// a checklist group header (expand / collapse)
    ToggleGroup(usize),
    /// a checklist row's tick (index into `Checklist::picks`)
    ToggleItem(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    Plain,
    Danger,
    Ghost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub control: Control,
    pub label: String,
    pub tooltip: String,
    pub style: ButtonStyle,
    /// takes the keyboard when the step waits for a decision (`.dflt`)
    pub default_focus: bool,
    pub enabled: bool,
    /// a leading glyph (the shield of the administrator button)
    pub icon: Option<&'static str>,
}

impl Button {
    pub fn new(control: Control, label: &str) -> Button {
        Button { control, label: label.to_string(), tooltip: String::new(), style: ButtonStyle::Plain, default_focus: false, enabled: true, icon: None }
    }
    pub fn danger(mut self) -> Button {
        self.style = ButtonStyle::Danger;
        self
    }
    pub fn ghost(mut self) -> Button {
        self.style = ButtonStyle::Ghost;
        self
    }
    pub fn dflt(mut self) -> Button {
        self.default_focus = true;
        self
    }
    pub fn tip(mut self, t: &str) -> Button {
        self.tooltip = t.to_string();
        self
    }
    pub fn disabled(mut self, d: bool) -> Button {
        self.enabled = !d;
        self
    }
    pub fn icon(mut self, i: &'static str) -> Button {
        self.icon = Some(i);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressKind {
    /// the running bar (`.uprog`)
    Indeterminate,
    /// waiting for the Windows permission prompt (`.uprog.elev`)
    Elevating,
}

/// One checklist row.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemRowView {
    /// `Control::ToggleItem(pick)`
    pub pick: usize,
    pub checked: bool,
    pub path: String,
    /// kind · size · note, or the deletion error
    pub meta: String,
    pub failed: bool,
    /// deleted: kept in the view only so its leave animation can finish
    pub gone: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupView {
    pub index: usize,
    pub title: String,
    pub tip: String,
    pub open: bool,
    pub hidden: bool,
    /// gliding value, already rounded for display
    pub count: u64,
    /// ` · 12 MB` or empty
    pub size_text: String,
    pub rows: Vec<ItemRowView>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FooterView {
    /// «Selected: N · size» (only when there are user items)
    pub selected: Option<String>,
    pub buttons: Vec<Button>,
}

/// One block of a step page, top to bottom. Each block enters with a 28 ms stagger (`stagger_ms`).
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    Heading(String),
    Sub(String),
    SubHead(String),
    Warn { text: String, shake: bool },
    Note(String),
    Busy(String),
    Path { text: String, fail: bool },
    Check { control: Control, label: String, checked: bool },
    Progress(ProgressKind),
    /// the shield above the elevation step
    Shield,
    /// «found: N · stage»
    ScanLine { label: String, found: u64, stage: String },
    /// the summary head of a finished job: tick, title, counts line, optional administrator / failed notes
    DoneHead { title: String, line: String, need_admin: Option<String>, failed_head: Option<String>, failed: Vec<String> },
    /// «Registry backup: …» / «Log: …»
    Extra { text: String },
    /// «Open backups folder»
    Link(Button),
    Group(GroupView),
    Buttons(Vec<Button>),
    /// the deleting overlay: «Deleting the selection… 2 / 5»
    DeleteProgress { text: String },
    Footer(FooterView),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PanelView {
    /// the drawn phase word (`confirm`, `running`, `scanning`, `deleting`, `elevating`, `still`, `review`, `error`, `done`)
    pub phase: String,
    pub parts: Vec<Part>,
    /// all controls of the panel rest (deleting)
    pub busy: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripIcon {
    Spinner,
    Check,
    Warn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StripView {
    pub name: String,
    /// «: searching for leftovers, found » — appended after the name
    pub message: String,
    /// the gliding number after the message (scanning: found, checklist: leftovers)
    pub count: Option<u64>,
    pub icon: StripIcon,
    pub kind: StripKind,
    pub phase: Phase,
    pub tooltip: String,
    /// the keycap on the strip
    pub shortcut: &'static str,
    /// busy strips carry a running bar
    pub bar: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UninstallView {
    pub takeover: bool,
    /// the box is on screen
    pub panel: Option<PanelView>,
    /// the normal result list is hidden behind the panel
    pub list_hidden: bool,
    pub strip: Option<StripView>,
    /// a Redraw is needed on the next frame (a number is still gliding)
    pub animating: bool,
}

/// What the renderer should do besides drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    /// run a backend command; `reply` says how the answer must be fed back (`UninstallUi::on_reply`)
    Call { call: crate::bar::protocol::BackendCall, reply: Option<ReplyTag> },
    /// give the keyboard to the step's default button
    FocusDefault,
    /// give the keyboard to the search box
    FocusInput,
    /// re-run the search for the box's current text (`q.dispatchEvent(new Event('input'))`)
    SearchAgain,
    /// play an animation
    Anim(crate::bar::anim::Anim),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ReplyTag {
    /// `uninstall_info` answer for the confirm card
    Info,
    /// `uninstall_state` answer after `bar:shown`
    State,
}

/// Target helper for tests and renderers: which animation target a panel part has.
pub fn part_target(i: usize) -> Target {
    Target::PanelPart(i)
}
