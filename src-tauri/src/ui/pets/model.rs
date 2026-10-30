//! Data model of one exported pet (tools/dev/export-pets.mjs -> data/<id>.json) and its loader.
//!
//! Schema (all numbers are CSS px = SVG user units of the pet's viewBox, times in ms, angles in degrees):
//!   nodes[]  {t: tag (div|svg|g|path|circle|ellipse|rect), p: parent index (-1 root), a: attributes, c: class names}
//!   bb[]     per node 0 | [x, y, w, h] (getBBox, used to resolve `%` in transforms and for fill-box pivots)
//!   base[]   per node the computed style in the reference context (calm, right end, no flags), see `StylePatch`
//!   basean   {node: [[keyframes, duration, delay, iterations(-1 = infinite), direction, fill]]}
//!   basetr   per node [[property, duration, delay, easing]]
//!   ctx{}    patches keyed "<state>|<r|l>", "<flag>|<state>|<r|l>" (flags blink twitch hop pet), "rm|<state>|<r|l>" (reduced motion statics),
//!            "lx|<state>", "ly|<state>", "lxn|<state>", "lyn|<state>" (look +-1 on an axis); each {st:{node:{field:value|null}}, an:{node:[anim]}, tr:{node:[trans]}}
//!   energy[] five patches (music at energy 0, .25, .5, .75, 1) relative to "music|r"
//!   kf[]     the keyframe pool: [{o: offset, e: easing, <property>: value}]
use super::ease::Ease;
use super::interp::{Dec, TfFn, TfList, DEC_IDENT};
use super::path::{self, Path};
use serde_json::Value;
use std::collections::HashMap;

pub const STATES: [&str; 7] = ["calm", "music", "thinking", "done", "attention", "file", "sleep"];
pub const NO_PATH: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub on: bool,
    /// r g b a in 0..1
    pub c: [f32; 4],
    /// share of the tint accent mixed in at --pet-k = 1 (the colour-mix percentage of pets.css)
    pub p: f32,
    /// alpha added at --pet-k = 1 (the pet halo is transparent until it is tinted)
    pub da: f32,
}

impl Paint {
    pub const OFF: Paint = Paint { on: false, c: [0.0; 4], p: 0.0, da: 0.0 };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub c: Paint,
    pub x: f32,
    pub y: f32,
    pub b: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dash {
    pub n: u8,
    pub v: [f32; 4],
}

pub const FLD_DN: u32 = 1;
pub const FLD_VH: u32 = 1 << 1;
pub const FLD_O: u32 = 1 << 2;
pub const FLD_TF: u32 = 1 << 3;
pub const FLD_TL: u32 = 1 << 4;
pub const FLD_RO: u32 = 1 << 5;
pub const FLD_SC: u32 = 1 << 6;
pub const FLD_PO: u32 = 1 << 7;
pub const FLD_FI: u32 = 1 << 8;
pub const FLD_ST: u32 = 1 << 9;
pub const FLD_SW: u32 = 1 << 10;
pub const FLD_LC: u32 = 1 << 11;
pub const FLD_LJ: u32 = 1 << 12;
pub const FLD_DA: u32 = 1 << 13;
pub const FLD_DOF: u32 = 1 << 14;
pub const FLD_D: u32 = 1 << 15;
pub const FLD_FL: u32 = 1 << 16;

/// Computed style of a node (inherited values already resolved by the browser).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub dn: bool,
    pub vh: bool,
    pub o: f32,
    /// decomposed `transform`
    pub tf: Dec,
    pub tl: [f32; 2],
    pub ro: f32,
    pub sc: [f32; 2],
    /// transform pivot: absolute user units for SVG nodes, a fraction of the box for the rig
    pub po: [f32; 2],
    pub fi: Paint,
    pub st: Paint,
    pub sw: f32,
    pub lc: u8,
    pub lj: u8,
    pub da: Dash,
    pub dof: f32,
    pub d: u32,
    pub fl: [Shadow; 4],
    pub nfl: u8,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            dn: false,
            vh: false,
            o: 1.0,
            tf: DEC_IDENT,
            tl: [0.0; 2],
            ro: 0.0,
            sc: [1.0; 2],
            po: [0.0; 2],
            fi: Paint::OFF,
            st: Paint::OFF,
            sw: 1.0,
            lc: 0,
            lj: 0,
            da: Dash { n: 0, v: [0.0; 4] },
            dof: 0.0,
            d: NO_PATH,
            fl: [Shadow { c: Paint::OFF, x: 0.0, y: 0.0, b: 0.0 }; 4],
            nfl: 0,
        }
    }
}

/// Fields of `s` selected by `mask`, applied onto a style.
#[derive(Clone, Copy, Debug)]
pub struct StylePatch {
    pub mask: u32,
    pub s: Style,
}

impl StylePatch {
    pub fn apply(&self, dst: &mut Style) {
        let (m, s) = (self.mask, &self.s);
        if m & FLD_DN != 0 {
            dst.dn = s.dn;
        }
        if m & FLD_VH != 0 {
            dst.vh = s.vh;
        }
        if m & FLD_O != 0 {
            dst.o = s.o;
        }
        if m & FLD_TF != 0 {
            dst.tf = s.tf;
        }
        if m & FLD_TL != 0 {
            dst.tl = s.tl;
        }
        if m & FLD_RO != 0 {
            dst.ro = s.ro;
        }
        if m & FLD_SC != 0 {
            dst.sc = s.sc;
        }
        if m & FLD_PO != 0 {
            dst.po = s.po;
        }
        if m & FLD_FI != 0 {
            dst.fi = s.fi;
        }
        if m & FLD_ST != 0 {
            dst.st = s.st;
        }
        if m & FLD_SW != 0 {
            dst.sw = s.sw;
        }
        if m & FLD_LC != 0 {
            dst.lc = s.lc;
        }
        if m & FLD_LJ != 0 {
            dst.lj = s.lj;
        }
        if m & FLD_DA != 0 {
            dst.da = s.da;
        }
        if m & FLD_DOF != 0 {
            dst.dof = s.dof;
        }
        if m & FLD_D != 0 {
            dst.d = s.d;
        }
        if m & FLD_FL != 0 {
            dst.fl = s.fl;
            dst.nfl = s.nfl;
        }
    }
}

/// Transition property groups (what `transition-property` can name here).
pub const TP_TF: u8 = 1;
pub const TP_TL: u8 = 2;
pub const TP_RO: u8 = 4;
pub const TP_SC: u8 = 8;
pub const TP_O: u8 = 16;
pub const TP_FILL: u8 = 32;
pub const TP_D: u8 = 64;
pub const TP_ALL: u8 = 0x7f;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransDef {
    pub mask: u8,
    pub dur: f32,
    pub delay: f32,
    pub ease: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimDef {
    pub kf: u32,
    pub dur: f32,
    pub delay: f32,
    /// negative = infinite
    pub iter: f32,
    pub dir: u8,
    pub fill: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Patch {
    pub st: Vec<(u32, StylePatch)>,
    pub an: Vec<(u32, Vec<AnimDef>)>,
    pub tr: Vec<(u32, Vec<TransDef>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Div,
    Svg,
    G,
    Path,
    Circle,
    Ellipse,
    Rect,
}

#[derive(Clone, Debug)]
pub enum Shape {
    None,
    Path(u32),
    Circle { cx: f32, cy: f32, r: f32 },
    Ellipse { cx: f32, cy: f32, rx: f32, ry: f32 },
    Rect { x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32 },
}

#[derive(Clone, Debug)]
pub struct Node {
    pub tag: Tag,
    pub parent: i32,
    pub shape: Shape,
    pub evenodd: bool,
    pub path_length: f32,
    pub crisp: bool,
    pub class: String,
    pub bb: Option<[f32; 4]>,
}

/// One animated property of a keyframe set: its own keyframes (CSS animates every property over the keyframes that name it).
#[derive(Clone, Debug)]
pub enum Val {
    Tf(TfList),
    F(f32),
    Col([f32; 4]),
    Vis(bool),
    D(Path),
    Dash(Dash),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prop {
    Tf,
    Rotate,
    Opacity,
    Fill,
    Stroke,
    Visibility,
    D,
    DashOffset,
    DashArray,
    StrokeWidth,
}

#[derive(Clone, Debug)]
pub struct Frame {
    pub o: f32,
    pub v: Val,
    pub e: u16,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub prop: Prop,
    pub frames: Vec<Frame>,
}

pub type KfSet = Vec<Track>;

#[derive(Debug)]
pub struct Pet {
    pub id: String,
    pub w: f32,
    pub h: f32,
    pub ar: f32,
    pub group: String,
    pub name_ru: String,
    pub name_en: String,
    pub has_fx: bool,
    pub nodes: Vec<Node>,
    pub children: Vec<Vec<u32>>,
    pub base: Vec<Style>,
    pub base_an: Vec<Vec<AnimDef>>,
    pub base_tr: Vec<Vec<TransDef>>,
    pub ctx: HashMap<String, Patch>,
    pub energy: Vec<Patch>,
    pub kf: Vec<KfSet>,
    pub paths: Vec<Path>,
    pub eases: Vec<Ease>,
}

fn f(v: &Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

pub fn paint(v: &Value) -> Paint {
    let a = arr(v);
    if a.len() < 4 {
        return Paint::OFF;
    }
    Paint { on: true, c: [f(&a[0]), f(&a[1]), f(&a[2]), f(&a[3])], p: a.get(4).map(f).unwrap_or(0.0), da: a.get(5).map(f).unwrap_or(0.0) }
}

fn dash(v: &Value) -> Dash {
    let a = arr(v);
    let mut d = Dash { n: 0, v: [0.0; 4] };
    for (i, x) in a.iter().take(4).enumerate() {
        d.v[i] = f(x);
        d.n = i as u8 + 1;
    }
    d
}

impl Pet {
    fn path_idx(&mut self, d: &str) -> u32 {
        match path::parse(d) {
            Some(p) => {
                self.paths.push(p);
                (self.paths.len() - 1) as u32
            }
            None => NO_PATH,
        }
    }

    fn ease_idx(&mut self, s: &str) -> u16 {
        let e = Ease::parse(s);
        if let Some(i) = self.eases.iter().position(|x| *x == e) {
            return i as u16;
        }
        self.eases.push(e);
        (self.eases.len() - 1) as u16
    }

    /// Style fields named in a JSON object -> a patch (a `null` value resets that field to its default).
    fn style_patch(&mut self, o: &serde_json::Map<String, Value>) -> StylePatch {
        let mut p = StylePatch { mask: 0, s: Style::default() };
        for (k, v) in o {
            let s = &mut p.s;
            let bit = match k.as_str() {
                "dn" => {
                    s.dn = v.as_i64() == Some(1);
                    FLD_DN
                }
                "vh" => {
                    s.vh = v.as_i64() == Some(1);
                    FLD_VH
                }
                "o" => {
                    s.o = if v.is_null() { 1.0 } else { f(v) };
                    FLD_O
                }
                "tf" => {
                    let a = arr(v);
                    if a.len() == 6 {
                        for i in 0..6 {
                            s.tf[i] = f(&a[i]);
                        }
                    }
                    FLD_TF
                }
                "tl" => {
                    let a = arr(v);
                    if a.len() >= 2 {
                        s.tl = [f(&a[0]), f(&a[1])];
                    }
                    FLD_TL
                }
                "ro" => {
                    s.ro = f(v);
                    FLD_RO
                }
                "sc" => {
                    let a = arr(v);
                    if a.len() >= 2 {
                        s.sc = [f(&a[0]), f(&a[1])];
                    }
                    FLD_SC
                }
                "po" => {
                    let a = arr(v);
                    if a.len() >= 2 {
                        s.po = [f(&a[0]), f(&a[1])];
                    }
                    FLD_PO
                }
                "fi" => {
                    s.fi = paint(v);
                    FLD_FI
                }
                "st" => {
                    s.st = paint(v);
                    FLD_ST
                }
                "sw" => {
                    s.sw = if v.is_null() { 1.0 } else { f(v) };
                    FLD_SW
                }
                "lc" => {
                    s.lc = v.as_u64().unwrap_or(0) as u8;
                    FLD_LC
                }
                "lj" => {
                    s.lj = v.as_u64().unwrap_or(0) as u8;
                    FLD_LJ
                }
                "da" => {
                    s.da = dash(v);
                    FLD_DA
                }
                "dof" => {
                    s.dof = f(v);
                    FLD_DOF
                }
                "d" => {
                    s.d = v.as_str().map(|d| self.path_idx(d)).unwrap_or(NO_PATH);
                    FLD_D
                }
                "fl" => {
                    let a = arr(v);
                    let mut n = 0;
                    for sh in a.iter().take(4) {
                        p.s.fl[n] = Shadow { c: paint(&sh["c"]), x: f(&sh["x"]), y: f(&sh["y"]), b: f(&sh["b"]) };
                        n += 1;
                    }
                    p.s.nfl = n as u8;
                    FLD_FL
                }
                _ => 0, // "tb" etc.: nothing to keep
            };
            p.mask |= bit;
            // the closure borrow above ends here; `s` is re-taken each iteration
        }
        p
    }

    fn anims(&self, v: &Value) -> Vec<AnimDef> {
        arr(v)
            .iter()
            .map(|a| {
                let a = arr(a);
                AnimDef { kf: a[0].as_u64().unwrap_or(0) as u32, dur: f(&a[1]), delay: f(&a[2]), iter: f(&a[3]), dir: a[4].as_u64().unwrap_or(0) as u8, fill: a[5].as_u64().unwrap_or(0) as u8 }
            })
            .collect()
    }

    fn trans(&mut self, v: &Value) -> Vec<TransDef> {
        let mut out = Vec::new();
        for t in arr(v) {
            let a = arr(t);
            let mask = match a[0].as_str().unwrap_or("") {
                "transform" => TP_TF,
                "translate" => TP_TL,
                "rotate" => TP_RO,
                "scale" => TP_SC,
                "opacity" => TP_O,
                "fill" | "stroke" => TP_FILL,
                "d" => TP_D,
                "all" => TP_ALL,
                _ => 0, // --pet-accent / --pet-k fade (the engine owns the tint fade)
            };
            if mask != 0 {
                let ease = self.ease_idx(a[3].as_str().unwrap_or("l"));
                out.push(TransDef { mask, dur: f(&a[1]), delay: f(&a[2]), ease });
            }
        }
        out
    }

    fn patch(&mut self, v: &Value) -> Patch {
        let mut p = Patch::default();
        if let Some(st) = v.get("st").and_then(|x| x.as_object()) {
            for (k, o) in st {
                if let (Ok(i), Some(o)) = (k.parse::<u32>(), o.as_object()) {
                    let sp = self.style_patch(o);
                    p.st.push((i, sp));
                }
            }
        }
        if let Some(an) = v.get("an").and_then(|x| x.as_object()) {
            for (k, l) in an {
                if let Ok(i) = k.parse::<u32>() {
                    p.an.push((i, self.anims(l)));
                }
            }
        }
        if let Some(tr) = v.get("tr").and_then(|x| x.as_object()) {
            for (k, l) in tr {
                if let Ok(i) = k.parse::<u32>() {
                    let t = self.trans(l);
                    p.tr.push((i, t));
                }
            }
        }
        p
    }

    fn keyframes(&mut self, frames: &Value) -> KfSet {
        let mut tracks: Vec<Track> = Vec::new();
        for fr in arr(frames) {
            let o = f(&fr["o"]);
            let e = self.ease_idx(fr["e"].as_str().unwrap_or("l"));
            let Some(obj) = fr.as_object() else { continue };
            for (k, v) in obj {
                let (prop, val) = match k.as_str() {
                    "o" | "e" => continue,
                    "transform" => {
                        let list: Vec<TfFn> = arr(v).iter().map(tf_fn).collect();
                        (Prop::Tf, Val::Tf(TfList::from_slice(&list).expect("more than 4 transform functions in a keyframe")))
                    }
                    "rotate" => (Prop::Rotate, Val::F(f(v))),
                    "opacity" => (Prop::Opacity, Val::F(f(v))),
                    "fill" => (Prop::Fill, Val::Col(col4(v))),
                    "stroke" => (Prop::Stroke, Val::Col(col4(v))),
                    "visibility" => (Prop::Visibility, Val::Vis(v.as_i64() == Some(1))),
                    "d" => (Prop::D, Val::D(path::parse(v.as_str().unwrap_or("")).unwrap_or_default())),
                    "strokeDashoffset" => (Prop::DashOffset, Val::F(f(v))),
                    "strokeDasharray" => (Prop::DashArray, Val::Dash(dash(v))),
                    "strokeWidth" => (Prop::StrokeWidth, Val::F(f(v))),
                    other => panic!("unknown keyframe property {other}"),
                };
                let i = match tracks.iter().position(|t| t.prop == prop) {
                    Some(i) => i,
                    None => {
                        tracks.push(Track { prop, frames: Vec::new() });
                        tracks.len() - 1
                    }
                };
                tracks[i].frames.push(Frame { o, v: val, e });
            }
        }
        tracks
    }

    pub fn parse(bytes: &[u8]) -> Result<Pet, String> {
        let j: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let mut pet = Pet {
            id: j["id"].as_str().ok_or("id")?.to_string(),
            w: f(&j["w"]),
            h: f(&j["h"]),
            ar: f(&j["ar"]),
            group: j["group"].as_str().unwrap_or("characters").to_string(),
            name_ru: j["name"]["ru"].as_str().unwrap_or("").to_string(),
            name_en: j["name"]["en"].as_str().unwrap_or("").to_string(),
            has_fx: j["fx"].as_bool().unwrap_or(true),
            nodes: Vec::new(),
            children: Vec::new(),
            base: Vec::new(),
            base_an: Vec::new(),
            base_tr: Vec::new(),
            ctx: HashMap::new(),
            energy: Vec::new(),
            kf: Vec::new(),
            paths: Vec::new(),
            eases: vec![Ease::Linear],
        };
        let nodes = arr(&j["nodes"]);
        let bbs = arr(&j["bb"]);
        for (i, n) in nodes.iter().enumerate() {
            let a = &n["a"];
            let g = |k: &str| a.get(k).and_then(|x| x.as_str()).and_then(|x| x.parse::<f32>().ok()).unwrap_or(0.0);
            let tag = match n["t"].as_str().unwrap_or("") {
                "div" => Tag::Div,
                "svg" => Tag::Svg,
                "g" => Tag::G,
                "path" => Tag::Path,
                "circle" => Tag::Circle,
                "ellipse" => Tag::Ellipse,
                "rect" => Tag::Rect,
                t => return Err(format!("unknown tag {t}")),
            };
            let shape = match tag {
                Tag::Path => Shape::Path(pet.path_idx(a["d"].as_str().unwrap_or(""))),
                Tag::Circle => Shape::Circle { cx: g("cx"), cy: g("cy"), r: g("r") },
                Tag::Ellipse => Shape::Ellipse { cx: g("cx"), cy: g("cy"), rx: g("rx"), ry: g("ry") },
                Tag::Rect => {
                    let (rx, ry) = (g("rx"), g("ry"));
                    Shape::Rect { x: g("x"), y: g("y"), w: g("width"), h: g("height"), rx: if rx > 0.0 { rx } else { ry }, ry: if ry > 0.0 { ry } else { rx } }
                }
                _ => Shape::None,
            };
            let bb = bbs.get(i).and_then(|b| b.as_array()).filter(|b| b.len() == 4).map(|b| [f(&b[0]), f(&b[1]), f(&b[2]), f(&b[3])]);
            pet.nodes.push(Node {
                tag,
                parent: n["p"].as_i64().unwrap_or(-1) as i32,
                shape,
                evenodd: a.get("fill-rule").and_then(|x| x.as_str()) == Some("evenodd"),
                path_length: g("pathLength"),
                crisp: a.get("shape-rendering").and_then(|x| x.as_str()) == Some("crispEdges"),
                class: n["c"].as_str().unwrap_or("").to_string(),
                bb,
            });
        }
        // the svg's own viewBox is the coordinate system of the art (claude8: 14 x 14 although the registry says 48 x 48)
        if let Some(vb) = j["nodes"][2]["a"]["viewBox"].as_str() {
            let v: Vec<f32> = vb.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() == 4 && v[2] > 0.0 && v[3] > 0.0 {
                pet.w = v[2];
                pet.h = v[3];
            }
        }
        pet.children = vec![Vec::new(); pet.nodes.len()];
        for i in 0..pet.nodes.len() {
            let p = pet.nodes[i].parent;
            if p >= 0 {
                pet.children[p as usize].push(i as u32);
            }
        }
        for s in arr(&j["base"]) {
            let sp = pet.style_patch(s.as_object().ok_or("base style")?);
            let mut st = Style::default();
            sp.apply(&mut st);
            pet.base.push(st);
        }
        pet.base_an = vec![Vec::new(); pet.nodes.len()];
        if let Some(o) = j["basean"].as_object() {
            for (k, l) in o {
                if let Ok(i) = k.parse::<usize>() {
                    pet.base_an[i] = pet.anims(l);
                }
            }
        }
        for t in arr(&j["basetr"]) {
            let d = pet.trans(t);
            pet.base_tr.push(d);
        }
        if let Some(o) = j["ctx"].as_object() {
            for (k, v) in o {
                let p = pet.patch(v);
                pet.ctx.insert(k.clone(), p);
            }
        }
        for v in arr(&j["energy"]) {
            let p = pet.patch(v);
            pet.energy.push(p);
        }
        for k in arr(&j["kf"]) {
            let set = pet.keyframes(k);
            pet.kf.push(set);
        }
        if pet.base.len() != pet.nodes.len() || pet.base_tr.len() != pet.nodes.len() {
            return Err("node / style count mismatch".into());
        }
        Ok(pet)
    }
}

fn col4(v: &Value) -> [f32; 4] {
    let a = arr(v);
    if a.len() < 4 {
        return [0.0; 4];
    }
    [f(&a[0]), f(&a[1]), f(&a[2]), f(&a[3])]
}

fn tf_fn(v: &Value) -> TfFn {
    let a = arr(v);
    match a[0].as_str().unwrap_or("") {
        "t" => TfFn::T(f(&a[1]), f(&a[2]), f(&a[3]), f(&a[4])),
        "s" => TfFn::S(f(&a[1]), f(&a[2])),
        "r" => TfFn::R(f(&a[1])),
        "k" => TfFn::K(f(&a[1]), f(&a[2])),
        "m" => TfFn::M([f(&a[1]), f(&a[2]), f(&a[3]), f(&a[4]), f(&a[5]), f(&a[6])]),
        k => panic!("unknown transform function {k}"),
    }
}

/// Embedded pet data (generated).
pub fn raw(id: &str) -> Option<&'static [u8]> {
    super::data::PETS.iter().find(|(k, _)| *k == id).map(|(_, b)| *b)
}

pub fn ids() -> impl Iterator<Item = &'static str> {
    super::data::PETS.iter().map(|(k, _)| *k)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_pet_parses_and_is_consistent() {
        let mut n = 0;
        for id in ids() {
            let p = Pet::parse(raw(id).unwrap()).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(p.id, id);
            assert!(p.w > 0.0 && p.h > 0.0 && p.ar >= 0.5 && p.ar <= 2.5, "{id}: size");
            assert_eq!(p.nodes[0].tag, Tag::Div);
            assert_eq!(p.nodes[1].tag, Tag::Div);
            assert_eq!(p.nodes[2].tag, Tag::Svg);
            for st in STATES {
                for side in ["r", "l"] {
                    // every state of every side exists (an empty patch is allowed only for the reference context)
                    let k = format!("{st}|{side}");
                    assert!(p.ctx.contains_key(&k) || k == "calm|r" || k == "calm|l" || k == "music|l" || k == "file|l" || p.ctx.contains_key(&format!("rm|{k}")), "{id}: missing {k}");
                    assert!(p.ctx.contains_key(&format!("rm|{k}")), "{id}: missing reduced-motion context for {k}");
                }
            }
            assert_eq!(p.energy.len(), 5, "{id}");
            for patch in p.ctx.values().chain(p.energy.iter()) {
                for (i, _) in &patch.st {
                    assert!((*i as usize) < p.nodes.len());
                }
                for (_, l) in &patch.an {
                    for a in l {
                        assert!((a.kf as usize) < p.kf.len(), "{id}: keyframe index");
                    }
                }
            }
            for (i, l) in p.base_an.iter().enumerate() {
                for a in l {
                    assert!((a.kf as usize) < p.kf.len(), "{id}: node {i}");
                    assert!(a.dur > 0.0);
                }
            }
            for set in &p.kf {
                for t in set {
                    assert!(t.frames.windows(2).all(|w| w[0].o <= w[1].o), "{id}: keyframe offsets must be sorted");
                }
            }
            for s in &p.base {
                if s.d != NO_PATH {
                    assert!((s.d as usize) < p.paths.len());
                }
            }
            n += 1;
        }
        assert!(n >= 28, "expected all 28 pets, got {n}");
    }

    #[test]
    fn style_patch_null_resets() {
        let mut pet = Pet::parse(raw("cat").unwrap()).unwrap();
        let v: Value = serde_json::json!({"dn": 1, "fi": null, "o": 0.5});
        let sp = pet.style_patch(v.as_object().unwrap());
        let mut s = Style::default();
        s.fi = Paint { on: true, c: [1.0; 4], p: 0.0, da: 0.0 };
        sp.apply(&mut s);
        assert!(s.dn && !s.fi.on && s.o == 0.5);
    }
}
