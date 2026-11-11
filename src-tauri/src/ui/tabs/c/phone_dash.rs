//! The Phone sub-tab of a paired phone: two tiles of equal weight side by side, no scrolling in the normal case.
//!
//! Left "Phone": name, model / Android / app, route pill; the big battery block; storage and memory as bars. Right "Connection": the quiet
//! "Pair another phone" button, network, the notable modes (Do Not Disturb, airplane, vibrate / silent), the session counters and the facts
//! (connected for, paired, IP, phone time, PC). With two or more phones a row of pills on top switches the shown one.
//!
//! Layout: every tile is a column of sections; the gap between them stretches (8..16 px) to fill the tile, so both tiles are as tall as the body.
//! When the body is too short for the 8 px version a tight one (smaller rows) is tried, and only then the scroll view takes over (the same
//! picture, a scrollbar's width narrower). A paired phone has no QR by default: "Pair another phone" slides it in as an overlay (`phone_pair.rs`).
//!
//! Motion (`ui::liquid`): numbers and bars are the [`Card`](super::phone_card::Card) springs, section heights follow their appearance springs
//! (a mode chip grows the row, the tile re-flows), the pill selection glides, the QR panel slides on a [`Flow`]. Targets are set in
//! `dash_sync` (called from `step`), `draw` only reads. Offline, the last known values stay, dimmed, and the pill says "Last seen ...".
use super::phone::{go, PhoneView, K};
use super::phone_card::{city, usage_color, wrap};
use super::phone_draw::reg;
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::b::kit::{Clicks, Icons, Lit};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::{Lang, Tr};
use pill_ui_core::panel::c::phone::{size_text, Device};
use pill_ui_core::panel::c::phone_info::*;
use std::cell::Cell;

pub(super) const PAD: f32 = 12.0;
pub(super) const TILE_GAP: f32 = 8.0;
const PILL_H: f32 = 24.0;
const PILL_GAP: f32 = 6.0;
const SBAR: f32 = 8.0;
const GIB: f32 = 1_073_741_824.0;
const MIB: f32 = 1_048_576.0;
const GMAX: f32 = 16.0;
const BAT: f32 = 74.0;
const BAT_DETAIL: f32 = 16.0;
const BAR_H: f32 = 26.0;
const CHIP_H: f32 = 22.0;

/// Row sizes of one density. `NORMAL` is tried first, `TIGHT` when the body is shorter.
struct Dims {
    gmin: f32,
    head_l: f32,
    head_r: f32,
    net: f32,
    stat: f32,
    stat_gap: f32,
    fact: f32,
    bar_gap: f32,
}
const NORMAL: Dims = Dims { gmin: 8.0, head_l: 36.0, head_r: 24.0, net: 42.0, stat: 24.0, stat_gap: 6.0, fact: 16.0, bar_gap: 10.0 };
const TIGHT: Dims = Dims { gmin: 6.0, head_l: 36.0, head_r: 22.0, net: 38.0, stat: 22.0, stat_gap: 4.0, fact: 15.0, bar_gap: 6.0 };

/// What the last paint measured (the fit tests read it).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Fit {
    /// height the taller tile wants at its tightest gap, the height the tiles got
    pub need: f32,
    pub tile_h: f32,
    pub tight: bool,
    /// texts that did not fit their box (cut with an ellipsis)
    pub clipped: u32,
}

/// State of the paired view that is not in the model.
pub(super) struct Dash {
    /// the phone shown (its pairing id); none = the connected one, else the first
    pub sel: Option<String>,
    pub qr_open: bool,
    /// the QR panel's slide, 0 = away, 1 = in (overshoots a little)
    pub qr: Flow,
    /// the pill selection: x / width relative to the pill row, the pill rects of the last draw
    pub pill_x: Flow,
    pub pill_w: Flow,
    pub pill_cache: Vec<Rect>,
    pill_ready: bool,
    pub paired_seen: usize,
    /// connection of the shown phone (dimming of its values)
    pub conn: Lit,
    pub fit: Fit,
}

impl Dash {
    pub(super) fn new() -> Dash {
        Dash {
            sel: None,
            qr_open: false,
            qr: Flow::px(0.0, Feel::FLOW),
            pill_x: Flow::px(0.0, Feel::FLOW),
            pill_w: Flow::px(0.0, Feel::FLOW),
            pill_cache: Vec::new(),
            pill_ready: false,
            paired_seen: 0,
            conn: Lit::new(Feel::FADE),
            fit: Fit::default(),
        }
    }

    pub(super) fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = self.conn.step(dt_ms);
        for f in [&mut self.qr, &mut self.pill_x, &mut self.pill_w] {
            f.step(dt_ms);
            busy |= f.moving();
        }
        busy
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Route {
    Home,
    Relay,
    Off,
}

fn route_of(d: &Device) -> Route {
    match (d.route.as_str(), d.connected) {
        ("lan", _) | ("", true) => Route::Home,
        ("relay", _) => Route::Relay,
        _ => Route::Off,
    }
}

fn route_word(r: Route, l: Lang) -> &'static str {
    match r {
        Route::Home => l.t("Домашняя сеть", "Home network"),
        Route::Relay => l.t("Интернет", "Internet"),
        Route::Off => l.t("Не в сети", "Offline"),
    }
}

struct BarRow {
    label: &'static str,
    used: f32,
    text: String,
}

struct ModeChip {
    icon: &'static str,
    label: String,
    k: f32,
    fg: Color,
    bg: Color,
}

struct StatChip {
    icon: &'static str,
    n: f32,
    target: u64,
    tail: String,
}

struct Fact {
    label: &'static str,
    value: String,
    /// a shorter value for a tight column
    short: Option<String>,
    mono: bool,
}

struct PillInfo {
    name: String,
    route: Route,
    word: &'static str,
    sel: bool,
}

/// Everything the painters need, owned (the paint closure borrows other parts of the view).
struct Content {
    conn: f32,
    dimk: f32,
    route: Route,
    name: String,
    sub: String,
    pill: String,
    pct: i32,
    bat_v: f32,
    ck: f32,
    dk: f32,
    charge: String,
    detail: Option<String>,
    sk: f32,
    bars: Vec<BarRow>,
    net_title: String,
    net_detail: Option<String>,
    net_icon: &'static str,
    net_col: Color,
    bars_k: f32,
    sig: f32,
    modes: Vec<ModeChip>,
    stats: Vec<StatChip>,
    facts: Vec<Fact>,
    pc: Option<String>,
    pc_label: &'static str,
    title_r: &'static str,
    pair: &'static str,
    pills: Vec<PillInfo>,
}

/// Sections of a tile from top to bottom: `hs` are their heights (0 = absent), `caps` how much each may grow. Returns each one's y inside the
/// tile, the heights they end up with and the height the column wants at the smallest gap. Spare room goes first to the growing sections, then
/// to the gap between sections (up to [`GMAX`]), and what is still left is split above and below.
fn lay(hs: &[f32], caps: &[f32], avail: f32, gmin: f32) -> (Vec<f32>, Vec<f32>, f32) {
    let first = hs.iter().position(|h| *h >= 0.5);
    let w: Vec<f32> = hs.iter().enumerate().map(|(i, h)| if Some(i) == first || *h < 0.5 { 0.0 } else { (h / 16.0).min(1.0) }).collect();
    let (sh, sw) = (hs.iter().sum::<f32>(), w.iter().sum::<f32>());
    let need = 2.0 * PAD + sh + gmin * sw;
    let cap = caps.iter().zip(hs).filter(|(_, h)| **h >= 0.5).map(|(c, _)| *c).sum::<f32>();
    let grow = (avail - need).max(0.0).min(cap);
    let hs: Vec<f32> = hs.iter().zip(caps).map(|(h, c)| if *h >= 0.5 && cap > 0.0 { h + grow * c / cap } else { *h }).collect();
    let sh = sh + grow;
    let g = if sw > 0.0 { ((avail - 2.0 * PAD - sh) / sw).clamp(gmin, GMAX) } else { gmin };
    let extra = ((avail - (2.0 * PAD + sh + g * sw)) / 2.0).max(0.0);
    let mut y = PAD + extra;
    let mut ys = Vec::new();
    for (h, w) in hs.iter().zip(&w) {
        y += g * w;
        ys.push(y);
        y += h;
    }
    (ys, hs, need)
}

struct Plan {
    tw: f32,
    dm: &'static Dims,
    ys_l: Vec<f32>,
    ys_r: Vec<f32>,
    hs_l: Vec<f32>,
    hs_r: Vec<f32>,
    mpos: Vec<(f32, f32)>,
    mfull: Vec<f32>,
    need: f32,
    /// the height both tiles get
    tile_h: f32,
    scroll: bool,
}

/// Paint helpers shared by the tile painters; `t` counts a text that is wider than its box.
struct Ink<'a, 'c> {
    cv: &'a Canvas<'c>,
    th: &'a Theme,
    ic: &'a Icons,
    clipped: &'a Cell<u32>,
}

impl Ink<'_, '_> {
    fn t(&self, r: Rect, s: &str, st: Style, c: Color) {
        if self.cv.text_width(s, st) > r.w + 0.5 {
            self.clipped.set(self.clipped.get() + 1);
        }
        self.cv.text(r, s, st.ellipsis(), c);
    }
}

fn st11() -> Style {
    Style::new(11.0, 500)
}

impl PhoneView {
    /// The phone shown: the chosen one, else the connected one, else the first paired.
    pub(super) fn shown(&self) -> Option<&Device> {
        let l = self.model.link();
        self.dash.sel.as_deref().and_then(|id| l.devices.iter().find(|d| d.id == id)).or_else(|| l.phone())
    }

    fn sel_index(&self) -> usize {
        let l = self.model.link();
        self.shown().and_then(|s| l.devices.iter().position(|d| std::ptr::eq(d, s))).unwrap_or(0)
    }

    /// Choose the phone at `i` of the list.
    pub(super) fn pick(&mut self, i: usize) {
        if let Some(d) = self.model.link().devices.get(i) {
            self.dash.sel = Some(d.id.clone());
        }
    }

    /// Targets of the paired view. Called from `step` only.
    pub(super) fn dash_sync(&mut self) {
        let reduced = self.reduced;
        let n = self.model.link().paired;
        if n > self.dash.paired_seen && self.dash.qr_open {
            self.dash.qr_open = false; // a phone just paired with the code: the panel has done its job
        }
        self.dash.paired_seen = n;
        let conn = self.shown().is_some_and(|d| d.connected);
        self.dash.conn.set(conn);
        go(&mut self.dash.qr, self.dash.qr_open as u8 as f32, reduced);
        let i = self.sel_index();
        if let Some(r) = self.dash.pill_cache.get(i).copied() {
            if !self.dash.pill_ready {
                self.dash.pill_ready = true;
                self.dash.pill_x.jump(r.x);
                self.dash.pill_w.jump(r.w);
            }
            go(&mut self.dash.pill_x, r.x, reduced);
            go(&mut self.dash.pill_w, r.w, reduced);
        }
    }

    fn content(&self, cv: &Canvas, th: &Theme, d: &Device) -> Content {
        let l = self.model.lang();
        let c = &self.card;
        let i = &d.info;
        let wall = self.wall();
        let tz = self.tz_min();
        let conn = self.dash.conn.at();
        let route = route_of(d);
        let pill = if d.connected {
            route_word(route, l).to_string()
        } else {
            ago_text(wall, d.last_seen, l).map(|a| format!("{} {}", l.t("Был в сети", "Last seen"), a)).unwrap_or_else(|| l.t("Не в сети", "Offline").to_string())
        };
        let sub = {
            let mut parts: Vec<String> = Vec::new();
            if !d.model.is_empty() {
                parts.push(d.model.clone());
            }
            if !d.os.is_empty() {
                parts.push(d.os.clone());
            }
            if !d.app_version.is_empty() {
                parts.push(format!("Pill Phone {}", d.app_version));
            }
            parts.join(" · ")
        };
        let mut bars = Vec::new();
        if let Some((_, t)) = i.storage {
            let of = format!("{} {}", l.t("из", "of"), size_text(t, l));
            bars.push(BarRow { label: l.t("Хранилище", "Storage"), used: c.st_used.at(), text: format!("{} {} {of}", size_text((c.st_free.at().max(0.0) * GIB) as u64, l), l.t("свободно", "free")) });
        }
        if let Some((_, t)) = i.ram {
            let of = format!("{} {}", l.t("из", "of"), size_text(t * 1_048_576, l));
            bars.push(BarRow { label: l.t("Память", "Memory"), used: c.ram_used.at(), text: format!("{} {} {of}", size_text((c.ram_free.at().max(0.0) * GIB) as u64, l), l.t("свободно", "free")) });
        }
        let net = net_view(&d.network, d.ssid.as_deref(), i, l);
        let (net_icon, net_col) = match (net.kind, i.airplane == Some(true)) {
            (_, true) => ("p-plane", th.accent),
            (NetKind::Wifi, _) => ("p-wifi", th.text2),
            (NetKind::Mobile, _) => ("p-mobile", th.text2),
            (NetKind::None, _) => ("p-mobile", th.dim),
        };
        let net_detail = match (net.kind, i.mbps) {
            (NetKind::Wifi, Some(_)) => Some(format!("{} {}", c.mbps.at().round() as i64, l.t("Мбит/с", "Mbps"))),
            _ => net.detail.clone(),
        };
        // only the notable modes: the "screen on / off" chip is gone
        let modes = vec![
            ModeChip { icon: "p-moon", label: l.t("Не беспокоить", "Do not disturb").into(), k: c.dnd.at01(), fg: th.amber, bg: th.amber_bg },
            ModeChip { icon: "p-plane", label: l.t("Режим полёта", "Airplane").into(), k: c.air.at01(), fg: th.accent, bg: th.accent_soft },
            ModeChip { icon: "bell", label: i.ringer.map(|r| ringer_text(r, l)).unwrap_or("").into(), k: c.ringer.at01(), fg: th.amber, bg: th.amber_bg },
        ];
        let s = &d.stats;
        let stats = vec![
            StatChip { icon: "bell", n: c.n_notifs.at(), target: c.n_notifs.target().round() as u64, tail: notifs_word(s.notifs, l).into() },
            StatChip { icon: "p-reply", n: c.n_replies.at(), target: c.n_replies.target().round() as u64, tail: replies_word(s.replies, l).into() },
            StatChip { icon: "p-down", n: c.n_in.at(), target: c.n_in.target().round() as u64, tail: files_tail(s.files_in, (c.b_in.at().max(0.0) * MIB) as u64, l) },
            StatChip { icon: "p-up", n: c.n_out.at(), target: c.n_out.target().round() as u64, tail: files_tail(s.files_out, (c.b_out.at().max(0.0) * MIB) as u64, l) },
        ];
        let mut facts = Vec::new();
        if d.connected && d.connected_at > 0 {
            facts.push(Fact { label: l.t("На связи", "Connected for"), value: duration_text(wall - d.connected_at, l), short: None, mono: false });
        } else if d.last_seen > 0 {
            let when = format!("{} {}", date_text(d.last_seen, tz, l).unwrap_or_default(), phone_clock(d.last_seen, tz as i32));
            facts.push(Fact { label: l.t("Был в сети", "Last seen"), value: when, short: date_text(d.last_seen, tz, l), mono: false });
        }
        if let Some(t) = date_text(d.first_seen, tz, l) {
            facts.push(Fact { label: l.t("Подключён", "Paired"), value: t, short: None, mono: false });
        }
        if !d.ip.is_empty() {
            facts.push(Fact { label: l.t("Адрес", "Phone IP"), value: d.ip.clone(), short: None, mono: true });
        }
        if let Some(off) = i.utc_min {
            let clock = phone_clock(wall, off);
            let place = i.tz.as_deref().map(|t| format!(" · {}", city(t))).unwrap_or_default();
            facts.push(Fact { label: l.t("Время", "Phone time"), value: format!("{clock}{place}"), short: Some(clock), mono: false });
        }
        let link = self.model.link();
        let pc = (!link.pc_name.is_empty() || !link.ip.is_empty()).then(|| format!("{} · {}:{}", link.pc_name, link.ip, link.port));
        let sel = self.sel_index();
        let pills = link.devices.iter().enumerate().map(|(n, p)| PillInfo { name: p.name.clone(), route: route_of(p), word: route_word(route_of(p), l), sel: n == sel }).collect();
        let _ = cv;
        Content {
            conn,
            dimk: 0.55 + 0.45 * conn,
            route,
            name: d.name.clone(),
            sub,
            pill,
            pct: (self.bat.at01() * 100.0).round() as i32,
            bat_v: self.bat.at01(),
            ck: c.charge_k.at01(),
            dk: c.detail_k.at01(),
            charge: charge_text(i.charge, d.charging, i, l),
            detail: battery_detail(&Info { temp_c: i.temp_c.map(|_| c.temp.at()), ..i.clone() }, l),
            sk: c.store_k.at01(),
            bars,
            net_title: net.title,
            net_detail,
            net_icon,
            net_col,
            bars_k: c.bars_k.at01(),
            sig: c.sig.at(),
            modes,
            stats,
            facts,
            pc,
            pc_label: l.t("ПК", "PC"),
            title_r: l.t("Связь", "Connection"),
            pair: l.t("Подключить ещё телефон", "Pair another phone"),
            pills,
        }
    }

    /// Sizes for a total width: the chips wrap by the tile width, so this runs again when the scrollbar takes its share.
    fn plan(&self, cv: &Canvas, ct: &Content, w_total: f32, tile_h: f32) -> Plan {
        let tw = ((w_total - TILE_GAP) / 2.0).floor();
        let cw = tw - 2.0 * PAD;
        let mfull: Vec<f32> = ct.modes.iter().map(|m| cv.text_width(&m.label, st11()).ceil() + 38.0).collect();
        let mw: Vec<(f32, f32)> = ct.modes.iter().zip(&mfull).map(|(m, f)| (if m.k > 0.01 && !m.label.is_empty() { f * m.k.min(1.0) } else { 0.0 }, m.k.min(1.0))).collect();
        let (mpos, mh) = wrap(&mw, cw, CHIP_H);
        let mk = ct.modes.iter().filter(|m| !m.label.is_empty()).map(|m| m.k.min(1.0)).fold(0.0f32, f32::max);
        let nb = ct.bars.len() as f32;
        let bars_h = |dm: &Dims| if nb > 0.0 { (nb * BAR_H + (nb - 1.0) * dm.bar_gap) * ct.sk } else { 0.0 };
        let facts_h = |dm: &Dims| (ct.facts.len().div_ceil(2) as f32 + ct.pc.is_some() as u8 as f32) * dm.fact;
        let hs_l = |dm: &Dims| vec![dm.head_l, BAT + BAT_DETAIL * ct.dk, bars_h(dm)];
        let hs_r = |dm: &Dims| vec![dm.head_r, dm.net, mh * mk, 2.0 * dm.stat + dm.stat_gap, facts_h(dm)];
        let try_dm = |dm: &'static Dims, avail: f32| {
            let (yl, hl, nl) = lay(&hs_l(dm), &[0.0, 24.0, 0.0], avail, dm.gmin);
            let (yr, hr, nr) = lay(&hs_r(dm), &[0.0, 6.0, 0.0, 0.0, 0.0], avail, dm.gmin);
            (yl, yr, hl, hr, nl.max(nr))
        };
        for dm in [&NORMAL, &TIGHT] {
            let (ys_l, ys_r, hs_l, hs_r, need) = try_dm(dm, tile_h);
            if need <= tile_h + 0.5 {
                return Plan { tw, dm, ys_l, ys_r, hs_l, hs_r, mpos, mfull, need, tile_h, scroll: false };
            }
        }
        let (_, _, _, _, need) = try_dm(&TIGHT, tile_h);
        let (ys_l, ys_r, hs_l, hs_r, _) = try_dm(&TIGHT, need);
        Plan { tw, dm: &TIGHT, ys_l, ys_r, hs_l, hs_r, mpos, mfull, need, tile_h: need, scroll: true }
    }

    /// The whole paired view: pills (2+ phones), the two tiles, inside the scroll view that only scrolls when the body is too short.
    pub(super) fn draw_dash(&mut self, cv: &Canvas, th: &Theme, body: Rect, d: &Device) {
        let ct = self.content(cv, th, d);
        let multi = ct.pills.len() > 1;
        let top = if multi { PILL_H + PILL_GAP } else { 0.0 };
        let avail = (body.h - top).max(0.0);
        let mut p = self.plan(cv, &ct, body.w, avail);
        if p.scroll {
            p = self.plan(cv, &ct, body.w - SBAR, avail);
        }
        let content_h = top + p.tile_h;
        let clipped = Cell::new(0u32);
        let mut sv = std::mem::take(&mut self.scroll[2]);
        sv.set_content_height(content_h, body.h);
        let off = sv.offset();
        let slide = self.dash.qr.at();
        let qr_shut = !self.dash.qr_open && slide < 0.02;
        let pill_cache = {
            let PhoneView { ic, clicks, dash, .. } = self;
            let ink = Ink { cv, th, ic, clipped: &clipped };
            let mut cache = Vec::new();
            sv.draw(cv, body, th, |cv, _c| {
                let ink = Ink { cv, ..ink };
                if multi {
                    cache = paint_pills(&ink, &ct, body.w, dash, clicks, body, off, qr_shut);
                }
                let left = Rect::new(0.0, top, p.tw, p.tile_h);
                let right = Rect::new(p.tw + TILE_GAP, top, p.tw, p.tile_h);
                paint_left(&ink, &ct, &p, left);
                paint_right(&ink, &ct, &p, right, clicks, body, off, qr_shut);
            });
            cache
        };
        self.scroll[2] = sv;
        if multi {
            self.dash.pill_cache = pill_cache;
        }
        self.dash.fit = Fit { need: p.need, tile_h: p.tile_h, tight: std::ptr::eq(p.dm, &TIGHT), clipped: clipped.get() };
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------- pills

/// The phone switcher. Returns the pill rects (relative to the row) for the indicator targets.
#[allow(clippy::too_many_arguments)]
fn paint_pills(x: &Ink, ct: &Content, w: f32, dash: &Dash, clicks: &mut Clicks<K>, body: Rect, off: f32, live: bool) -> Vec<Rect> {
    let (cv, th) = (x.cv, x.th);
    let ns = Style::new(12.0, 600);
    let rs = Style::new(11.0, 400);
    let full: Vec<f32> = ct.pills.iter().map(|p| cv.text_width(&p.name, ns).ceil() + cv.text_width(p.word, rs).ceil() + 10.0 + 6.0 + 24.0).collect();
    let words = full.iter().sum::<f32>() + 4.0 * ct.pills.len() as f32 <= w;
    let mut rects = Vec::new();
    let mut px = 2.0;
    for (p, f) in ct.pills.iter().zip(&full) {
        let pw = (if words { *f } else { f - cv.text_width(p.word, rs).ceil() - 6.0 }).min((w - 4.0) / ct.pills.len() as f32).ceil();
        rects.push(Rect::new(px, 0.0, pw, PILL_H));
        px += pw + 4.0;
    }
    let ind = Rect::new(dash.pill_x.at(), 0.0, dash.pill_w.at().max(0.0), PILL_H);
    cv.fill_rr(ind, PILL_H / 2.0, th.accent_soft);
    for (i, (p, r)) in ct.pills.iter().zip(&rects).enumerate() {
        let hv = clicks.hover_amt(&K::Pick(i));
        let on = (1.0 - (r.cx() - ind.cx()).abs() / (r.w * 0.9 + 20.0)).clamp(0.0, 1.0);
        let col = th.dim.mix(th.text, on.max(hv * 0.6));
        let dot = match p.route {
            Route::Home => th.green,
            Route::Relay => th.accent,
            Route::Off => th.faint,
        };
        cv.with_scale_about(r.center(), clicks.press_scale(&K::Pick(i), 0.97), |cv| {
            cv.fill_circle((r.x + 12.0, r.cy()), 3.5, dot);
            let nw = cv.text_width(&p.name, ns).ceil();
            let ink = Ink { cv, ..*x };
            ink.t(Rect::new(r.x + 22.0, r.y, (r.w - 22.0 - 8.0).min(nw + 1.0), r.h), &p.name, ns, col);
            if words {
                ink.t(Rect::new(r.x + 22.0 + nw + 6.0, r.y, r.w - 22.0 - nw - 6.0 - 8.0, r.h), p.word, rs, col.fade(0.75));
            }
        });
        if live {
            reg(clicks, K::Pick(i), *r, body, off);
        }
    }
    rects
}

// ---------------------------------------------------------------------------------------------------------------------------------- tiles

fn inner(th: &Theme) -> Color {
    th.card.mix(th.raised2, 0.7)
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn paint_left(x: &Ink, ct: &Content, p: &Plan, tile: Rect) {
    let (cv, th, ic) = (x.cv, x.th, x.ic);
    cv.fill_rr(tile, 16.0, th.card);
    let (lx, cw) = (tile.x + PAD, tile.w - 2.0 * PAD);
    // header: connection dot, name, route pill; the model line under it
    let ys = &p.ys_l;
    let y = tile.y + ys[0];
    cv.fill_circle((lx + 4.0, y + 10.0), 4.0, th.dim.mix(if ct.route == Route::Relay { th.accent } else { th.green }, ct.conn));
    let ps = Style::new(11.0, 600);
    let pw = (cv.text_width(&ct.pill, ps).ceil() + 18.0).min(tile.w * 0.6);
    let pr = Rect::new(tile.right() - PAD - pw, y, pw, 20.0);
    let (fg, bg) = if ct.route == Route::Relay { (th.accent, th.accent_soft) } else { (th.green, th.green_bg) };
    cv.fill_rr(pr, 10.0, th.track.mix(bg, ct.conn));
    x.t(pr, &ct.pill, ps.center(), th.dim.mix(fg, ct.conn));
    x.t(Rect::new(lx + 16.0, y, (pr.x - 8.0 - lx - 16.0).max(30.0), 20.0), &ct.name, Style::new(15.0, 600), th.text);
    x.t(Rect::new(lx + 16.0, y + 20.0, cw - 16.0, 16.0), &ct.sub, Style::new(11.0, 400), th.dim);
    cv.with_opacity(ct.dimk, |cv| {
        let x = Ink { cv, ..*x };
        // battery: the one big number
        let b = Rect::new(lx, tile.y + ys[1], cw, p.hs_l[1]);
        // spare height (the tile is taller than it needs) opens the block up
        let e = (b.h - (BAT + BAT_DETAIL * ct.dk)).max(0.0);
        let (o1, o2) = (e * 0.3, e * 0.65);
        cv.fill_rr(b, 12.0, inner(th));
        ic.draw_sw(cv, "p-phone", Rect::new(b.x + 12.0, b.y + 16.0 + o1, 20.0, 20.0), th.dim.fade(1.0 - ct.ck), 1.7);
        ic.draw_sw(cv, "p-bolt", Rect::new(b.x + 12.0, b.y + 16.0 + o1, 20.0, 20.0), th.green.fade(ct.ck), 1.7);
        let num = ct.pct.to_string();
        let ns = Style::new(32.0, 700);
        cv.text(Rect::new(b.x + 42.0, b.y + 6.0 + o1, 80.0, 40.0), &num, ns, th.text);
        let nw = cv.text_width(&num, ns).ceil();
        cv.text(Rect::new(b.x + 42.0 + nw + 2.0, b.y + 12.0 + o1, 24.0, 40.0), "%", Style::new(15.0, 600), th.dim);
        let bar = Rect::new(b.x + 132.0, b.y + 22.0 + o1, b.w - 132.0 - 12.0, 8.0);
        cv.fill_rr(bar, 4.0, th.track);
        if ct.bat_v > 0.0 {
            let col = th.green.mix(th.red, smooth(0.28, 0.12, ct.bat_v) * (1.0 - ct.ck));
            cv.fill_rr(Rect::new(bar.x, bar.y, (bar.w * ct.bat_v).max(8.0), 8.0), 4.0, col);
        }
        x.t(Rect::new(b.x + 12.0, b.y + 46.0 + o2, b.w - 24.0, 16.0), &ct.charge, st11(), th.dim.mix(th.green, ct.ck));
        if let Some(t) = &ct.detail {
            cv.with_opacity(ct.dk, |cv| Ink { cv, ..x }.t(Rect::new(b.x + 12.0, b.y + 62.0 + o2, b.w - 24.0, 16.0), t, Style::new(11.0, 400), th.dim));
        }
        // storage and memory: two bars
        let n = ct.bars.len() as f32;
        if n > 0.0 && ct.sk > 0.01 {
            let y0 = tile.y + ys[2];
            cv.with_clip(Rect::new(lx - 1.0, y0, cw + 2.0, (n * BAR_H + (n - 1.0) * p.dm.bar_gap) * ct.sk), |cv| {
                cv.with_opacity(ct.sk, |cv| {
                    let x = Ink { cv, ..x };
                    for (j, r) in ct.bars.iter().enumerate() {
                        let ry = y0 + j as f32 * (BAR_H + p.dm.bar_gap);
                        let ls = Style::new(11.0, 600);
                        let lw = cv.text_width(r.label, ls).ceil();
                        x.t(Rect::new(lx, ry, lw + 2.0, 14.0), r.label, ls, th.text2);
                        x.t(Rect::new(lx + lw + 10.0, ry, cw - lw - 10.0, 14.0), &r.text, Style::new(11.0, 400).right(), th.dim);
                        let tr = Rect::new(lx, ry + 18.0, cw, 6.0);
                        cv.fill_rr(tr, 3.0, th.track);
                        let u = r.used.clamp(0.0, 1.0);
                        if u > 0.0 {
                            cv.fill_rr(Rect::new(tr.x, tr.y, (tr.w * u).max(6.0), 6.0), 3.0, usage_color(th, u));
                        }
                    }
                });
            });
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn paint_right(x: &Ink, ct: &Content, p: &Plan, tile: Rect, clicks: &mut Clicks<K>, body: Rect, off: f32, live: bool) {
    let (cv, th, ic) = (x.cv, x.th, x.ic);
    let dm = p.dm;
    cv.fill_rr(tile, 16.0, th.card);
    let (lx, cw) = (tile.x + PAD, tile.w - 2.0 * PAD);
    let ys = &p.ys_r;
    let hs = &p.hs_r;
    // header: title and the quiet pair button
    let y = tile.y + ys[0];
    let bs = st11();
    let title_w = cv.text_width(ct.title_r, Style::new(13.0, 600)).ceil();
    let bw = (cv.text_width(ct.pair, bs).ceil() + 42.0).min(cw - title_w - 12.0);
    let br = Rect::new(tile.right() - PAD - bw, y, bw, dm.head_r);
    let hv = clicks.hover_amt(&K::Pair);
    cv.with_scale_about(br.center(), clicks.press_scale(&K::Pair, 0.96), |cv| {
        cv.fill_rr(br, br.h / 2.0, th.hover.alpha(0.0).mix(th.hover, hv));
        cv.stroke_rr(br.inset(0.5), br.h / 2.0 - 0.5, 1.0, th.line.fade(0.9));
        let fg = th.dim.mix(th.text, hv);
        ic.draw_sw(cv, "qr", Rect::new(br.x + 10.0, br.cy() - 7.0, 14.0, 14.0), fg, 1.8);
        Ink { cv, ..*x }.t(Rect::new(br.x + 30.0, br.y, br.w - 30.0 - 10.0, br.h), ct.pair, bs, fg);
    });
    if live {
        reg(clicks, K::Pair, br, body, off);
    }
    x.t(Rect::new(lx, y, (br.x - 8.0 - lx).max(30.0), dm.head_r), ct.title_r, Style::new(13.0, 600), th.text);
    cv.with_opacity(ct.dimk, |cv| {
        let x = Ink { cv, ..*x };
        // network
        let nr = Rect::new(lx, tile.y + ys[1], cw, hs[1]);
        cv.fill_rr(nr, 12.0, inner(th));
        ic.draw_sw(cv, ct.net_icon, Rect::new(nr.x + 10.0, nr.cy() - 10.0, 20.0, 20.0), ct.net_col, 1.7);
        let tw = nr.w - 40.0 - 12.0 - 24.0 * ct.bars_k;
        match &ct.net_detail {
            Some(dn) => {
                let top = nr.y + (nr.h - 32.0) / 2.0;
                x.t(Rect::new(nr.x + 40.0, top, tw, 18.0), &ct.net_title, Style::new(13.0, 600), th.text);
                x.t(Rect::new(nr.x + 40.0, top + 18.0, tw, 14.0), dn, Style::new(11.0, 400), th.dim);
            }
            None => x.t(Rect::new(nr.x + 40.0, nr.y, tw, nr.h), &ct.net_title, Style::new(13.0, 600), th.text),
        }
        cv.with_opacity(ct.bars_k, |cv| {
            let x0 = nr.right() - 12.0 - 22.0;
            for k in 0..4 {
                let h = 6.0 + 4.0 * k as f32;
                let a = (ct.sig - k as f32).clamp(0.0, 1.0);
                cv.fill_rr(Rect::new(x0 + k as f32 * 6.0, nr.cy() + 10.0 - h, 4.0, h), 1.5, th.track.mix(th.text, a * 0.9));
            }
        });
        // modes
        let my = tile.y + ys[2];
        for ((m, &(mx, myy)), &full) in ct.modes.iter().zip(&p.mpos).zip(&p.mfull) {
            let k = m.k.min(1.0);
            if k * full < 1.0 || m.label.is_empty() {
                continue;
            }
            let r = Rect::new(lx + mx, my + myy, full * k, CHIP_H);
            cv.with_clip_rr(r, 11.0, |cv| {
                cv.with_opacity(k, |cv| {
                    cv.fill_rr(Rect::new(r.x, r.y, full, CHIP_H), 11.0, m.bg);
                    ic.draw_sw(cv, m.icon, Rect::new(r.x + 9.0, r.y + 4.0, 14.0, 14.0), m.fg, 1.8);
                    cv.text(Rect::new(r.x + 28.0, r.y, full, CHIP_H), &m.label, st11(), m.fg);
                });
            });
        }
        // session counters: 2 x 2
        let sy = tile.y + ys[3];
        let cell_w = ((cw - 6.0) / 2.0).floor();
        let num_style = Style::new(11.0, 700);
        for (j, s) in ct.stats.iter().enumerate() {
            let r = Rect::new(lx + (j % 2) as f32 * (cell_w + 6.0), sy + (j / 2) as f32 * (dm.stat + dm.stat_gap), cell_w, dm.stat);
            cv.fill_rr(r, dm.stat / 2.0, inner(th));
            let live = if s.target == 0 { 0.0 } else { 1.0 };
            ic.draw_sw(cv, s.icon, Rect::new(r.x + 9.0, r.cy() - 7.0, 14.0, 14.0), th.dim.mix(th.accent, 0.45 * live), 1.8);
            let nw = cv.text_width(&s.target.to_string(), num_style).ceil();
            cv.text(Rect::new(r.x + 28.0, r.y, nw + 8.0, r.h), &s.n.round().max(0.0).to_string(), num_style, th.text.mix(th.dim, 0.45 * (1.0 - live)));
            x.t(Rect::new(r.x + 28.0 + nw + 4.0, r.y, (r.right() - 8.0 - (r.x + 28.0 + nw + 4.0)).max(10.0), r.h), &s.tail, st11(), th.dim);
        }
        // facts
        let fy = tile.y + ys[4];
        if !ct.facts.is_empty() || ct.pc.is_some() {
            cv.fill_rect(Rect::new(lx + 2.0, fy - 5.0, cw - 4.0, 1.0), th.line.fade(0.6));
        }
        let colw = ((cw - 16.0) / 2.0).floor();
        let (ls, vs) = (Style::new(11.0, 400), Style::new(11.0, 500));
        let fact = |cv: &Canvas, label: &str, value: &str, short: Option<&String>, mono: bool, r: Rect| {
            let lw = cv.text_width(label, ls).ceil() + 8.0;
            let vstyle = if mono { vs.mono() } else { vs };
            let v = match short {
                Some(s) if cv.text_width(value, vstyle) > r.w - lw => s.as_str(),
                _ => value,
            };
            let x = Ink { cv, ..x };
            x.t(Rect::new(r.x, r.y, lw, r.h), label, ls, th.dim);
            x.t(Rect::new(r.x + lw, r.y, r.w - lw, r.h), v, vstyle.right(), th.text2);
        };
        for (j, f) in ct.facts.iter().enumerate() {
            let r = Rect::new(lx + 2.0 + (j % 2) as f32 * (colw + 16.0), fy + (j / 2) as f32 * dm.fact, colw - 2.0, dm.fact);
            fact(cv, f.label, &f.value, f.short.as_ref(), f.mono, r);
        }
        if let Some(pc) = &ct.pc {
            let r = Rect::new(lx + 2.0, fy + ct.facts.len().div_ceil(2) as f32 * dm.fact, cw - 4.0, dm.fact);
            let lbl = ct.pc_label;
            let lw = cv.text_width(lbl, ls).ceil() + 8.0;
            x.t(Rect::new(r.x, r.y, lw, r.h), lbl, ls, th.dim);
            x.t(Rect::new(r.x + lw, r.y, r.w - lw, r.h), pc, Style::new(11.0, 400).mono().right(), th.faint);
        }
    });
}
