//! A page of the window as live objects: blocks (headings, captions, cards of rows, picker grids, wizard parts) built from the declarative
//! schema, with one retained glasskit control per row. Building is cheap; the view builds a page on every switch.
use super::fields::TextField;
use glasskit::controls::{Segmented, Select, Slider, Toggle};
use glasskit::ease::Tween;
use glasskit::geom::Rect;
use glasskit::scroll::{Axis, ScrollState, Scrollbar};
use glasskit::theme::Theme;
use pill_ui_core::settings::flows::Msg;
use pill_ui_core::settings::schema::{self, BackendToggle, Control, Dynamic, Row, TextKind};
use pill_ui_core::settings::tr::Lang;
use pill_ui_core::settings::SettingsModel;
use serde_json::Value;

/// How a text-like row edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FK {
    Text,
    Secret,
    Lines { rows: u8, exe: bool },
    Number,
    Time,
}

pub struct FieldCtl {
    pub tf: TextField,
    pub kind: FK,
}

/// The seven chips of Quiet hours: pop animation per chip.
pub struct Week {
    pub pop: [Tween<f32>; 7],
    pub cur: usize,
}

pub enum Ctl {
    None,
    Toggle(Toggle),
    Backend(Toggle, BackendToggle),
    Slider(Slider),
    Seg(Segmented),
    Select(Select),
    Hotkey,
    Field(Box<FieldCtl>),
    Week(Week),
    Button,
    /// Wizard step 3: the stored combo as read-only key caps
    Keys(String),
    Custom(&'static str),
}

pub struct RowRt {
    pub row: &'static Row,
    pub ctl: Ctl,
    /// 0..1 how open the row is (expand / collapse)
    pub vis: Tween<f32>,
    pub vis_on: bool,
    /// 1 enabled .. 0.5 disabled
    pub en: Tween<f32>,
    pub en_on: bool,
    pub msg: Option<(Msg, Tween<f32>)>,
    pub status: Option<Msg>,
    /// seconds since the search highlight started, None = no highlight
    pub hl: Option<f32>,
    /// laid out at the last draw: the row body (content space, un-collapsed height) and its control
    pub rect: Rect,
    pub ctl_rect: Rect,
    pub fid: u32,
    /// labels / values of a segmented control or select, parallel
    pub opts: Vec<(String, Value)>,
}

pub enum Block {
    H1(String),
    Cap(String),
    Foot(String),
    Note(String),
    Card(Vec<RowRt>),
    Pets(u32),
    Chars(u32),
    WizTop,
    WizNav,
    Spacer,
}

pub struct Page {
    pub id: String,
    pub blocks: Vec<Block>,
    pub rects: Vec<Rect>,
    pub scroll: ScrollState,
    pub bar: Scrollbar,
    pub content_h: f32,
    pub fids: Vec<u32>,
    pub wizard: bool,
}

impl Page {
    pub fn empty() -> Page {
        Page { id: String::new(), blocks: Vec::new(), rects: Vec::new(), scroll: ScrollState::new(), bar: Scrollbar::new(Axis::Vertical), content_h: 0.0, fids: Vec::new(), wizard: false }
    }

    pub fn rows(&self) -> impl Iterator<Item = &RowRt> {
        self.blocks.iter().flat_map(|b| match b {
            Block::Card(rows) => rows.iter(),
            _ => [].iter(),
        })
    }

    pub fn rows_mut(&mut self) -> impl Iterator<Item = &mut RowRt> {
        self.blocks.iter_mut().flat_map(|b| match b {
            Block::Card(rows) => rows.iter_mut(),
            _ => [].iter_mut(),
        })
    }

    pub fn row_mut(&mut self, id: &str) -> Option<&mut RowRt> {
        self.rows_mut().find(|r| r.row.id == id)
    }
}

/// Dynamic option lists the backend fills in (monitors, audio devices).
#[derive(Default, Clone)]
pub struct Dyn {
    pub monitors: Vec<String>,
    pub outputs: Vec<(String, String)>,
    pub captures: Vec<(String, String)>,
}

/// Labels and values of a segmented control / select row in `lang`, the dynamic tail included.
pub fn options_of(row: &Row, d: &Dyn, lang: Lang) -> Vec<(String, Value)> {
    let (opts, dynamic) = match &row.control {
        Control::Segmented { options, dynamic } | Control::Select { options, dynamic } => (options, *dynamic),
        _ => return Vec::new(),
    };
    let mut v: Vec<(String, Value)> = opts.iter().map(|o| (o.label.get(lang).to_string(), o.value.clone())).collect();
    match dynamic {
        Dynamic::None => {}
        Dynamic::Monitors => {
            for i in 0..d.monitors.len() {
                v.push(((i + 1).to_string(), Value::String((i + 1).to_string())));
            }
        }
        Dynamic::OutputDevices => v.extend(d.outputs.iter().map(|(id, n)| (n.clone(), Value::String(id.clone())))),
        Dynamic::CaptureDevices => v.extend(d.captures.iter().map(|(id, n)| (n.clone(), Value::String(id.clone())))),
    }
    v
}

fn same_value(a: &Value, b: &Value) -> bool {
    let r = |v: &Value| match v {
        Value::String(s) => s.clone(),
        o => o.to_string(),
    };
    r(a) == r(b)
}

pub fn selected_index(opts: &[(String, Value)], cur: &Value) -> Option<usize> {
    opts.iter().position(|(_, v)| same_value(v, cur))
}

pub fn fmt_pct(v: f32) -> String {
    format!("{} %", v.round() as i64)
}
pub fn fmt_plain(v: f32) -> String {
    format!("{}", v.round() as i64)
}

pub fn field_kind(row: &Row) -> Option<FK> {
    Some(match &row.control {
        Control::Text { secret: true, .. } => FK::Secret,
        Control::Text { kind: TextKind::Secret, .. } => FK::Secret,
        Control::Text { .. } => FK::Text,
        Control::Lines { rows, exe_suffix, .. } => FK::Lines { rows: *rows, exe: *exe_suffix },
        Control::Number { .. } => FK::Number,
        Control::Time => FK::Time,
        _ => return None,
    })
}

/// The text a field shows for the stored value.
pub fn field_text(row: &Row, m: &SettingsModel) -> String {
    match &row.control {
        Control::Lines { exe_suffix, .. } => {
            let list: Vec<String> = match m.get(row.key) {
                Value::Array(a) => a.iter().map(|x| x.as_str().unwrap_or("").to_string()).collect(),
                _ => vec![],
            };
            pill_ui_core::settings::model::lines_text(&list, *exe_suffix)
        }
        Control::Text { secret: true, .. } => String::new(),
        _ => m.get_str(row.key),
    }
}

/// A fresh control for `row` with the model's current value.
pub fn make_ctl(row: &'static Row, m: &SettingsModel, th: &Theme, dy: &Dyn, lang: Lang) -> Ctl {
    let spring = th.m.sw_spring;
    match &row.control {
        Control::Toggle => Ctl::Toggle(Toggle::new(m.get_bool(row.key)).spring(spring)),
        Control::BackendToggle(b) => {
            let on = match b {
                BackendToggle::Autostart => m.autostart(),
                BackendToggle::ClipboardPrivate => false,
            };
            Ctl::Backend(Toggle::new(on).spring(spring), *b)
        }
        Control::Slider { min, max, step, unit } => {
            let f: fn(f32) -> String = match unit {
                schema::Unit::Percent => fmt_pct,
                schema::Unit::None => fmt_plain,
            };
            let v = m.get(row.key).as_f64().unwrap_or(*min) as f32;
            Ctl::Slider(Slider::new(*min as f32, *max as f32, v).step_by(*step as f32).with_format(f))
        }
        Control::Segmented { .. } => {
            let opts = options_of(row, dy, lang);
            let labels: Vec<&str> = opts.iter().map(|o| o.0.as_str()).collect();
            let sel = selected_index(&opts, &m.get(row.key)).unwrap_or(0);
            Ctl::Seg(Segmented::new(&labels, sel))
        }
        Control::Select { .. } => {
            let opts = options_of(row, dy, lang);
            let labels: Vec<&str> = opts.iter().map(|o| o.0.as_str()).collect();
            let sel = selected_index(&opts, &m.get(row.key));
            Ctl::Select(Select::new(&labels, sel))
        }
        Control::Hotkey { .. } => Ctl::Hotkey,
        Control::Text { .. } | Control::Lines { .. } | Control::Number { .. } | Control::Time => {
            let kind = field_kind(row).unwrap_or(FK::Text);
            let multi = matches!(kind, FK::Lines { .. });
            let mut tf = TextField::new(multi, th);
            if let Control::Text { placeholder, mono, .. } = &row.control {
                tf.input.placeholder = placeholder.get(lang).to_string();
                if *mono {
                    tf.input.style = glasskit::text::Style::new(12.0, 400);
                }
            }
            if let Control::Lines { mono: true, .. } = &row.control {
                tf.input.style = glasskit::text::Style::new(12.0, 400);
            }
            if kind == FK::Secret {
                tf = tf.secret();
            }
            tf.input.pad = (th.m.field_pad_x, if multi { 8.0 } else { 0.0 });
            tf.input.radius = if multi { 10.0 } else { th.m.field_r };
            tf.set_text(&field_text(row, m));
            Ctl::Field(Box::new(FieldCtl { tf, kind }))
        }
        Control::Weekdays => Ctl::Week(Week { pop: std::array::from_fn(|_| Tween::at_rest(1.0)), cur: 0 }),
        Control::Button { .. } => Ctl::Button,
        Control::Custom(name) => Ctl::Custom(name),
        Control::PetsPicker | Control::CharacterPicker | Control::Color => Ctl::None,
    }
}

pub fn new_row(row: &'static Row, m: &SettingsModel, th: &Theme, dy: &Dyn, lang: Lang, fid: u32) -> RowRt {
    let st = m.row_state(row);
    let ctl = make_ctl(row, m, th, dy, lang);
    let opts = options_of(row, dy, lang);
    RowRt {
        row,
        ctl,
        vis: Tween::at_rest(st.visible as u8 as f32),
        vis_on: st.visible,
        en: Tween::at_rest(if st.enabled { 1.0 } else { 0.5 }),
        en_on: st.enabled,
        msg: None,
        status: None,
        hl: None,
        rect: Rect::default(),
        ctl_rect: Rect::default(),
        fid,
        opts,
    }
}

/// Is a Tab stop wanted on this row?
pub fn focusable(ctl: &Ctl) -> bool {
    !matches!(ctl, Ctl::None | Ctl::Keys(_))
}
