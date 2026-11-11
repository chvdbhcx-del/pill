//! The left card of the Phone sub-tab: everything the PC knows about the phone, as tiles inside a scroll view.
//!
//! Order: header (name, model / Android / app, connection pill) > battery (percent, bar, charge line, temperature and health) > storage and
//! RAM > network (name, speed or carrier, signal bars) > mode chips (DND, airplane, ringer, screen) > session counters > facts (connected for,
//! paired, IP, the phone's clock). Unknown values are simply not drawn. Offline, the last known values stay, dimmed, and the pill says
//! "Last seen ...".
//!
//! Motion (`ui::liquid`): every number is a [`Num`] spring (percent, bars, GB, counters, signal), chips and tiles appear on a 0..1 [`Num`]; targets
//! are set in `Card::sync` (called from `step`), `draw` only reads. Nothing polls: the "connected for" text is recomputed when the card is
//! painted for another reason, like the notification times.
use super::phone::PhoneView;
use crate::ui::liquid::{Feel, Flow};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::{Lang, Tr};
use pill_ui_core::panel::c::phone::{size_text, Device};
use pill_ui_core::panel::c::phone_info::*;

/// Width of the left card; the QR card takes the rest.
pub(super) const LEFT_W: f32 = 388.0;
/// The footer line (PC name and address) is pinned under the scroll area.
const FOOT_H: f32 = 30.0;
const PAD: f32 = 12.0;
const GAP: f32 = 6.0;
const SBAR: f32 = 8.0;
const GIB: f32 = 1_073_741_824.0;
const MIB: f32 = 1_048_576.0;

/// One animated number. The first value lands at once (an opening tab is not news); reduced motion lands every value at once.
pub(super) struct Num {
    f: Flow,
    seen: bool,
}

impl Num {
    fn new(px: bool) -> Num {
        Num { f: if px { Flow::px(0.0, Feel::DRAG) } else { Flow::new(0.0, Feel::DRAG) }, seen: false }
    }
    /// A 0..1 switch (a chip, a tile that appears) with its own feel.
    fn lit(feel: Feel) -> Num {
        Num { f: Flow::new(0.0, feel), seen: false }
    }
    fn on(&mut self, on: bool, reduced: bool) {
        self.set(on as u8 as f32, reduced);
    }
    pub(super) fn at01(&self) -> f32 {
        self.f.at01()
    }
    fn set(&mut self, v: f32, reduced: bool) {
        if !self.seen || reduced {
            self.seen = true;
            self.f.jump(v);
        }
        self.f.to(v);
    }
    pub(super) fn at(&self) -> f32 {
        self.f.at()
    }
    pub(super) fn target(&self) -> f32 {
        self.f.target()
    }
    fn step(&mut self, dt_ms: f64) -> bool {
        self.f.step(dt_ms);
        self.f.moving()
    }
}

/// All the motion of the card.
pub(super) struct Card {
    /// battery temperature, deg C
    pub temp: Num,
    /// used fractions of storage and RAM, free amounts in GiB
    pub st_used: Num,
    pub st_free: Num,
    pub ram_used: Num,
    pub ram_free: Num,
    /// signal level 0..4, link speed Mbps
    pub sig: Num,
    pub mbps: Num,
    /// counters; bytes in MiB
    pub n_notifs: Num,
    pub n_replies: Num,
    pub n_in: Num,
    pub n_out: Num,
    pub b_in: Num,
    pub b_out: Num,
    /// mode chips
    pub dnd: Num,
    pub air: Num,
    pub ringer: Num,
    pub screen: Num,
    pub screen_on: Num,
    /// parts that exist only when the phone told: the storage row, the temperature line, the signal bars; and the bolt
    pub store_k: Num,
    pub detail_k: Num,
    pub bars_k: Num,
    pub charge_k: Num,
}

impl Card {
    pub(super) fn new() -> Card {
        let n = || Num::new(true);
        Card {
            temp: n(),
            st_used: Num::new(false),
            st_free: n(),
            ram_used: Num::new(false),
            ram_free: n(),
            sig: Num::new(false),
            mbps: n(),
            n_notifs: n(),
            n_replies: n(),
            n_in: n(),
            n_out: n(),
            b_in: n(),
            b_out: n(),
            dnd: Num::lit(Feel::POP),
            air: Num::lit(Feel::POP),
            ringer: Num::lit(Feel::POP),
            screen: Num::lit(Feel::POP),
            screen_on: Num::lit(Feel::FADE),
            store_k: Num::lit(Feel::FLOW),
            detail_k: Num::lit(Feel::FLOW),
            bars_k: Num::lit(Feel::FADE),
            charge_k: Num::lit(Feel::FADE),
        }
    }

    /// Targets from the device. Called from `step` only. A value the phone did not send keeps its last target.
    pub(super) fn sync(&mut self, d: &Device, reduced: bool) {
        let i = &d.info;
        if let Some(t) = i.temp_c {
            self.temp.set(t, reduced);
        }
        if let (Some(u), Some((f, _))) = (i.storage_used(), i.storage) {
            self.st_used.set(u, reduced);
            self.st_free.set(f as f32 / GIB, reduced);
        }
        if let (Some(u), Some((f, _))) = (i.ram_used(), i.ram) {
            self.ram_used.set(u, reduced);
            self.ram_free.set(f as f32 * MIB / GIB, reduced);
        }
        let net = net_view(&d.network, d.ssid.as_deref(), i, Lang::En);
        if let Some(b) = net.bars {
            self.sig.set(b as f32, reduced);
        }
        if let Some(m) = i.mbps {
            self.mbps.set(m as f32, reduced);
        }
        let s = &d.stats;
        for (n, v) in [(&mut self.n_notifs, s.notifs), (&mut self.n_replies, s.replies), (&mut self.n_in, s.files_in), (&mut self.n_out, s.files_out)] {
            n.set(v as f32, reduced);
        }
        self.b_in.set(s.bytes_in as f32 / MIB, reduced);
        self.b_out.set(s.bytes_out as f32 / MIB, reduced);
        self.dnd.on(d.dnd, reduced);
        self.air.on(i.airplane == Some(true), reduced);
        self.ringer.on(matches!(i.ringer, Some(Ringer::Vibrate | Ringer::Silent)), reduced);
        self.screen.on(i.screen.is_some(), reduced);
        self.screen_on.on(i.screen == Some(true), reduced);
        self.store_k.on(i.storage.is_some() || i.ram.is_some(), reduced);
        self.detail_k.on(battery_detail(i, Lang::En).is_some(), reduced);
        self.bars_k.on(net.bars.is_some(), reduced);
        self.charge_k.on(
            match i.charge {
                Some(c) => c == Charge::Charging,
                None => d.charging,
            },
            reduced,
        );
    }

    /// Advances every spring; true while one is still moving.
    pub(super) fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = false;
        for n in [
            &mut self.temp,
            &mut self.st_used,
            &mut self.st_free,
            &mut self.ram_used,
            &mut self.ram_free,
            &mut self.sig,
            &mut self.mbps,
            &mut self.n_notifs,
            &mut self.n_replies,
            &mut self.n_in,
            &mut self.n_out,
            &mut self.b_in,
            &mut self.b_out,
            &mut self.dnd,
            &mut self.air,
            &mut self.ringer,
            &mut self.screen,
            &mut self.screen_on,
            &mut self.store_k,
            &mut self.detail_k,
            &mut self.bars_k,
            &mut self.charge_k,
        ] {
            busy |= n.step(dt_ms);
        }
        busy
    }
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A usage bar colour that slides from the accent to amber to red as the bar fills.
fn usage_color(th: &Theme, v: f32) -> Color {
    th.accent.mix(th.amber, smooth(0.78, 0.9, v)).mix(th.red, smooth(0.93, 0.98, v))
}

/// Flow-wraps chips of the given (width, appearance) into rows of `avail` px; returns each chip's (x, y) and the total height.
/// A chip of width 0 takes no room.
fn wrap(items: &[(f32, f32)], avail: f32, row_h: f32) -> (Vec<(f32, f32)>, f32) {
    let (mut x, mut y, mut any) = (0.0f32, 0.0f32, false);
    let mut out = Vec::new();
    for &(w, k) in items {
        if w < 0.5 {
            out.push((x, y));
            continue;
        }
        if x > 0.0 && x + w > avail {
            x = 0.0;
            y += row_h + 5.0;
        }
        any = true;
        out.push((x, y));
        x += w + GAP * k;
    }
    (out, if any { y + row_h } else { 0.0 })
}

fn city(tz: &str) -> String {
    tz.rsplit('/').next().unwrap_or(tz).replace('_', " ")
}

#[cfg(windows)]
fn local_tz_min() -> i64 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let dst = if id == 2 { tz.DaylightBias } else { tz.StandardBias };
    -(tz.Bias + dst) as i64
}
#[cfg(not(windows))]
fn local_tz_min() -> i64 {
    0
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

impl PhoneView {
    /// The scroll area of the left card (window coordinates).
    pub(super) fn card_area(&self) -> Rect {
        let b = self.body_rect();
        Rect::new(b.x, b.y + 4.0, LEFT_W, (b.h - 4.0 - FOOT_H).max(0.0))
    }

    /// The tz offset used for dates: the PC's, 0 when the wall clock is pinned (tests).
    fn tz_min(&self) -> i64 {
        if self.wall_pinned() {
            0
        } else {
            local_tz_min()
        }
    }

    /// Paints the whole left card: background, the scrolling tiles, the footer.
    pub(super) fn draw_phone_left(&mut self, cv: &Canvas, th: &Theme, left: Rect) {
        cv.fill_rr(left, 16.0, th.card);
        let l = self.model.lang();
        let link = self.model.link().clone();
        match link.phone() {
            Some(p) => self.draw_tiles(cv, th, p),
            None => {
                let t = l.t("Наведи камеру телефона на код справа, установи Pill Phone и открой его по кнопке на странице.", "Point your phone's camera at the code on the right, install Pill Phone and open it with the button on the page.");
                cv.text(Rect::new(left.x + 18.0, left.y + 18.0, left.w - 36.0, 70.0), t, Style::new(12.0, 400).wrap(), th.dim);
            }
        }
        let foot = format!("{} · {}:{}", link.pc_name, link.ip, link.port);
        cv.fill_rect(Rect::new(left.x + 14.0, left.bottom() - FOOT_H, left.w - 28.0, 1.0), th.line.fade(0.6));
        cv.text(Rect::new(left.x + 18.0, left.bottom() - FOOT_H + 4.0, left.w - 36.0, FOOT_H - 8.0), &foot, Style::new(11.0, 400).mono().ellipsis(), th.faint);
    }

    fn draw_tiles(&mut self, cv: &Canvas, th: &Theme, d: &Device) {
        let area = self.card_area();
        let l = self.model.lang();
        let wall = self.wall();
        let tz = self.tz_min();
        let w = area.w - SBAR;
        let cw = w - 2.0 * PAD;
        let conn = self.conn_k.at();
        let dimk = 0.55 + 0.45 * conn;
        let i = &d.info;
        let tile = th.card.mix(th.raised2, 0.7);
        let st12 = Style::new(11.0, 500);

        // ---- content that decides the layout
        let c = &self.card;
        let dk = c.detail_k.at01();
        let sk = c.store_k.at01();
        let modes = [
            ModeChip { icon: "p-moon", label: l.t("Не беспокоить", "Do not disturb").into(), k: c.dnd.at01(), fg: th.amber, bg: th.amber_bg },
            ModeChip { icon: "p-plane", label: l.t("Режим полёта", "Airplane").into(), k: c.air.at01(), fg: th.accent, bg: th.accent_soft },
            ModeChip { icon: "bell", label: i.ringer.map(|r| ringer_text(r, l)).unwrap_or("").into(), k: c.ringer.at01(), fg: th.amber, bg: th.amber_bg },
            ModeChip {
                icon: "p-phone",
                label: if i.screen == Some(false) { l.t("Экран выкл.", "Screen off") } else { l.t("Экран вкл.", "Screen on") }.into(),
                k: c.screen.at01(),
                fg: th.dim.mix(th.green, c.screen_on.at01()),
                bg: th.track.mix(th.green_bg, c.screen_on.at01()),
            },
        ];
        let mfull = |m: &ModeChip| cv.text_width(&m.label, st12).ceil() + 38.0;
        let mw: Vec<(f32, f32)> = modes.iter().map(|m| (if m.k > 0.01 && !m.label.is_empty() { mfull(m) * m.k.min(1.0) } else { 0.0 }, m.k.min(1.0))).collect();
        let (mpos, mh) = wrap(&mw, cw, 22.0);
        let s = &d.stats;
        let stats = [
            StatChip { icon: "bell", n: c.n_notifs.at(), target: c.n_notifs.target().round() as u64, tail: notifs_word(s.notifs, l).into() },
            StatChip { icon: "p-reply", n: c.n_replies.at(), target: c.n_replies.target().round() as u64, tail: replies_word(s.replies, l).into() },
            StatChip { icon: "p-down", n: c.n_in.at(), target: c.n_in.target().round() as u64, tail: files_tail(s.files_in, (c.b_in.at().max(0.0) * MIB) as u64, l) },
            StatChip { icon: "p-up", n: c.n_out.at(), target: c.n_out.target().round() as u64, tail: files_tail(s.files_out, (c.b_out.at().max(0.0) * MIB) as u64, l) },
        ];
        let num_style = Style::new(11.0, 700);
        let num_w = |s: &StatChip| cv.text_width(&s.target.to_string(), num_style).ceil();
        let sfull = |s: &StatChip| 28.0 + num_w(s) + 4.0 + cv.text_width(&s.tail, st12).ceil() + 10.0;
        let sw: Vec<(f32, f32)> = stats.iter().map(|s| (sfull(s), 1.0)).collect();
        let (spos, sh) = wrap(&sw, cw, 22.0);
        // facts: two columns
        let mut facts: Vec<(String, String, bool)> = Vec::new();
        facts.push((l.t("Связь", "Route").into(), d.route_text(l).into(), false));
        if d.connected && d.connected_at > 0 {
            facts.push((l.t("На связи", "Connected for").into(), duration_text(wall - d.connected_at, l), false));
        } else if d.last_seen > 0 {
            let when = format!("{} {}", date_text(d.last_seen, tz, l).unwrap_or_default(), phone_clock(d.last_seen, tz as i32));
            facts.push((l.t("Был в сети", "Last seen").into(), when, false));
        }
        if let Some(t) = date_text(d.first_seen, tz, l) {
            facts.push((l.t("Подключён", "Paired").into(), t, false));
        }
        if !d.ip.is_empty() {
            facts.push((l.t("Адрес", "Phone IP").into(), d.ip.clone(), true));
        }
        if let Some(off) = i.utc_min {
            let place = i.tz.as_deref().map(|t| format!(" · {}", city(t))).unwrap_or_default();
            facts.push((l.t("Время", "Phone time").into(), format!("{}{}", phone_clock(wall, off), place), false));
        }
        let fh = facts.len().div_ceil(2) as f32 * 18.0;

        // ---- vertical layout
        let gap = |h: f32| GAP * (h / 22.0).clamp(0.0, 1.0);
        let (head_h, net_h) = (42.0, 42.0);
        let bat_h = 54.0 + 14.0 * dk;
        let store_h = 50.0 * sk;
        let mut y = 8.0;
        let y_head = y;
        y += head_h + GAP;
        let y_bat = y;
        y += bat_h + GAP;
        let y_store = y;
        y += store_h + gap(store_h);
        let y_net = y;
        y += net_h + GAP;
        let y_modes = y;
        y += mh + gap(mh);
        let y_stats = y;
        y += sh + gap(sh) + 4.0;
        let y_facts = y;
        y += fh + 10.0;
        let content_h = y;

        let net = net_view(&d.network, d.ssid.as_deref(), i, l);
        let pct = (self.bat.at01() * 100.0).round() as i32;
        let bat_v = self.bat.at01();
        let charge_line = charge_text(i.charge, d.charging, i, l);
        let detail = battery_detail(&Info { temp_c: i.temp_c.map(|_| c.temp.at()), ..i.clone() }, l);
        let pill = if d.connected {
            l.t("Подключён", "Connected").to_string()
        } else {
            ago_text(wall, d.last_seen, l).map(|a| format!("{} {}", l.t("Был в сети", "Last seen"), a)).unwrap_or_else(|| l.t("Не в сети", "Offline").to_string())
        };
        let name_line = {
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

        let mut sv = std::mem::take(&mut self.scroll[2]);
        sv.set_content_height(content_h, area.h);
        {
            let PhoneView { ic, card: c, .. } = self;
            sv.draw(cv, area, th, |cv, _c| {
                // header
                cv.fill_circle((PAD + 5.0, y_head + 11.0), 5.0, th.dim.mix(th.green, conn));
                let pw = (cv.text_width(&pill, st12).ceil() + 18.0).min(w * 0.5);
                let pr = Rect::new(w - PAD - pw, y_head + 1.0, pw, 20.0);
                cv.fill_rr(pr, 10.0, th.track.mix(th.green_bg, conn));
                cv.text(pr, &pill, st12.center().ellipsis(), th.dim.mix(th.green, conn));
                cv.text(Rect::new(PAD + 18.0, y_head, (pr.x - PAD - 26.0).max(30.0), 22.0), &d.name, Style::new(15.0, 600).ellipsis(), th.text);
                cv.text(Rect::new(PAD + 18.0, y_head + 22.0, w - 2.0 * PAD - 18.0, 16.0), &name_line, Style::new(11.0, 400).ellipsis(), th.dim);
                cv.with_opacity(dimk, |cv| {
                    // battery
                    let b = Rect::new(PAD, y_bat, cw, bat_h);
                    cv.fill_rr(b, 12.0, tile);
                    let ck = c.charge_k.at01();
                    ic.draw_sw(cv, "p-phone", Rect::new(b.x + 10.0, b.y + 10.0, 20.0, 20.0), th.dim.fade(1.0 - ck), 1.7);
                    ic.draw_sw(cv, "p-bolt", Rect::new(b.x + 10.0, b.y + 10.0, 20.0, 20.0), th.green.fade(ck), 1.7);
                    cv.text(Rect::new(b.x + 36.0, b.y + 6.0, 46.0, 28.0), &format!("{pct}%"), Style::new(17.0, 700), th.text);
                    let bar = Rect::new(b.x + 88.0, b.y + 16.0, b.w - 88.0 - 14.0, 8.0);
                    cv.fill_rr(bar, 4.0, th.track);
                    if bat_v > 0.0 {
                        let col = th.green.mix(th.red, smooth(0.28, 0.12, bat_v) * (1.0 - ck));
                        cv.fill_rr(Rect::new(bar.x, bar.y, (bar.w * bat_v).max(8.0), 8.0), 4.0, col);
                    }
                    cv.text(Rect::new(b.x + 12.0, b.y + 34.0, b.w - 24.0, 16.0), &charge_line, st12.ellipsis(), th.dim.mix(th.green, ck));
                    if let Some(t) = &detail {
                        cv.with_opacity(dk, |cv| cv.text(Rect::new(b.x + 12.0, b.y + 50.0, b.w - 24.0, 16.0), t, Style::new(11.0, 400).ellipsis(), th.dim));
                    }
                    // storage and RAM
                    if store_h > 0.5 {
                        let mut tiles: Vec<(&str, f32, f32, u64)> = Vec::new();
                        if let Some((_, t)) = i.storage {
                            tiles.push((l.t("Хранилище", "Storage"), c.st_used.at(), c.st_free.at(), t));
                        }
                        if let Some((_, t)) = i.ram {
                            tiles.push((l.t("Память", "Memory"), c.ram_used.at(), c.ram_free.at(), t * 1_048_576));
                        }
                        let n = tiles.len().max(1) as f32;
                        let tw = (cw - GAP * (n - 1.0)) / n;
                        cv.with_clip(Rect::new(PAD - 1.0, y_store, cw + 2.0, store_h), |cv| {
                            cv.with_opacity(sk, |cv| {
                                for (j, (label, used, free_gib, total)) in tiles.iter().enumerate() {
                                    let r = Rect::new(PAD + j as f32 * (tw + GAP), y_store, tw, 50.0);
                                    cv.fill_rr(r, 12.0, tile);
                                    cv.text(Rect::new(r.x + 12.0, r.y + 6.0, tw - 24.0, 14.0), label, Style::new(10.0, 600), th.dim);
                                    let of = format!("{} {}", l.t("из", "of"), size_text(*total, l));
                                    cv.text(Rect::new(r.x + 12.0, r.y + 6.0, tw - 24.0, 14.0), &of, Style::new(10.0, 400).right().ellipsis(), th.faint);
                                    let free = format!("{} {}", size_text((free_gib.max(0.0) * GIB) as u64, l), l.t("свободно", "free"));
                                    cv.text(Rect::new(r.x + 12.0, r.y + 20.0, tw - 24.0, 16.0), &free, Style::new(12.0, 600).ellipsis(), th.text);
                                    let tr = Rect::new(r.x + 12.0, r.y + 39.0, tw - 24.0, 5.0);
                                    cv.fill_rr(tr, 2.5, th.track);
                                    let u = used.clamp(0.0, 1.0);
                                    if u > 0.0 {
                                        cv.fill_rr(Rect::new(tr.x, tr.y, (tr.w * u).max(5.0), 5.0), 2.5, usage_color(th, u));
                                    }
                                }
                            });
                        });
                    }
                    // network
                    let nr = Rect::new(PAD, y_net, cw, net_h);
                    cv.fill_rr(nr, 12.0, tile);
                    let (icon, icol) = match (net.kind, i.airplane == Some(true)) {
                        (_, true) => ("p-plane", th.accent),
                        (NetKind::Wifi, _) => ("p-wifi", th.text2),
                        (NetKind::Mobile, _) => ("p-mobile", th.text2),
                        (NetKind::None, _) => ("p-mobile", th.dim),
                    };
                    ic.draw_sw(cv, icon, Rect::new(nr.x + 10.0, nr.y + 11.0, 20.0, 20.0), icol, 1.7);
                    let detail_net = match (net.kind, i.mbps) {
                        (NetKind::Wifi, Some(_)) => Some(format!("{} {}", c.mbps.at().round() as i64, l.t("Мбит/с", "Mbps"))),
                        _ => net.detail.clone(),
                    };
                    let bars_k = c.bars_k.at01();
                    let tw = nr.w - 40.0 - 12.0 - 24.0 * bars_k;
                    match detail_net {
                        Some(dn) => {
                            cv.text(Rect::new(nr.x + 40.0, nr.y + 5.0, tw, 18.0), &net.title, Style::new(13.0, 600).ellipsis(), th.text);
                            cv.text(Rect::new(nr.x + 40.0, nr.y + 22.0, tw, 14.0), &dn, Style::new(11.0, 400).ellipsis(), th.dim);
                        }
                        None => cv.text(Rect::new(nr.x + 40.0, nr.y, tw, nr.h), &net.title, Style::new(13.0, 600).ellipsis(), th.text),
                    }
                    cv.with_opacity(bars_k, |cv| {
                        let sig = c.sig.at();
                        let x0 = nr.right() - 12.0 - 22.0;
                        for k in 0..4 {
                            let h = 6.0 + 4.0 * k as f32;
                            let a = (sig - k as f32).clamp(0.0, 1.0);
                            cv.fill_rr(Rect::new(x0 + k as f32 * 6.0, nr.bottom() - 11.0 - h, 4.0, h), 1.5, th.track.mix(th.text, a * 0.9));
                        }
                    });
                    // modes
                    for (m, &(x, yy)) in modes.iter().zip(&mpos) {
                        let full = mfull(m);
                        let k = m.k.min(1.0);
                        if k * full < 1.0 || m.label.is_empty() {
                            continue;
                        }
                        let r = Rect::new(PAD + x, y_modes + yy, full * k, 22.0);
                        cv.with_clip_rr(r, 11.0, |cv| {
                            cv.with_opacity(k, |cv| {
                                cv.fill_rr(Rect::new(r.x, r.y, full, 22.0), 11.0, m.bg);
                                ic.draw_sw(cv, m.icon, Rect::new(r.x + 9.0, r.y + 4.0, 14.0, 14.0), m.fg, 1.8);
                                cv.text(Rect::new(r.x + 28.0, r.y, full, 22.0), &m.label, st12, m.fg);
                            });
                        });
                    }
                    // session counters
                    for (s, &(x, yy)) in stats.iter().zip(&spos) {
                        let nw = num_w(s);
                        let r = Rect::new(PAD + x, y_stats + yy, sfull(s), 22.0);
                        cv.fill_rr(r, 11.0, tile);
                        let live = if s.target == 0 { 0.0 } else { 1.0 };
                        ic.draw_sw(cv, s.icon, Rect::new(r.x + 9.0, r.y + 4.0, 14.0, 14.0), th.dim.mix(th.accent, 0.45 * live), 1.8);
                        cv.text(Rect::new(r.x + 28.0, r.y, nw + 8.0, 22.0), &format!("{}", s.n.round().max(0.0) as u64), num_style, th.text.mix(th.dim, 0.45 * (1.0 - live)));
                        cv.text(Rect::new(r.x + 28.0 + nw + 4.0, r.y, r.w, 22.0), &s.tail, st12, th.dim);
                    }
                    // facts
                    let colw = (cw - 16.0) / 2.0;
                    for (j, (label, value, mono)) in facts.iter().enumerate() {
                        let (col, row) = ((j % 2) as f32, (j / 2) as f32);
                        let x = PAD + 2.0 + col * (colw + 16.0);
                        let yy = y_facts + row * 18.0;
                        let lw = cv.text_width(label, Style::new(11.0, 400)).ceil() + 8.0;
                        cv.text(Rect::new(x, yy, lw, 18.0), label, Style::new(11.0, 400), th.dim);
                        let vs = if *mono { Style::new(11.0, 500).mono() } else { Style::new(11.0, 500) };
                        cv.text(Rect::new(x + lw, yy, colw - lw, 18.0), value, vs.right().ellipsis(), th.text2);
                    }
                });
            });
        }
        self.scroll[2] = sv;
    }
}
