//! Custom widgets of the Phone page: `phone.link` (QR + one-time code), `phone.devices` (paired phones, Revoke with a second click) and
//! `phone.apps` (app chips: click to mute). Data comes from `phone_status` / `phone_pair_code` / `phone_apps`; nothing here runs while the
//! page is closed. Motion: every value is a `Flow` retargeted in `step_phone` (never while painting), Reduce motion lands at once.
use super::flow::{from_to, go, tick};
use super::page::RowRt;
use super::rows::*;
use super::ui::{key, Act};
use super::SettingsView;
use crate::ui::liquid::{Feel, Flow};
use crate::ui::view::Cx;
use glasskit::color::Color;
use glasskit::controls::ButtonKind;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::calls::BackendCall as C;
use pill_ui_core::settings::tr::{pick, Lang};
use serde_json::{json, Value};

const QR_SIDE: f32 = 176.0;
const ROW_H: f32 = 56.0;
/// A pending Revoke waits this long for its second click.
const ARM_MS: f64 = 4000.0;

/// One paired phone or PC on screen. A device that is gone keeps its row (and the last data) until the leave flow has closed it.
#[derive(Clone)]
pub struct PRow {
    pub id: String,
    pub dev: Value,
    pub flow: Flow,
    /// 0 = offline, 1 = online (the PCs' dot)
    pub dot: Flow,
    pub present: bool,
}

pub struct CodeView {
    pub code: String,
    pub url: String,
    pub expires: i64,
    pub rows: Vec<String>,
}

pub struct PhoneData {
    pub status: Value,
    pub rows: Vec<PRow>,
    pub code: Option<CodeView>,
    /// the QR card popping in (0 -> 1) when a new code arrives
    pub qr_t: Flow,
    /// 0 = "link is off" panel, 1 = the code
    pub on_t: Flow,
    pub apps: Vec<(String, String)>,
    /// the Revoke that waits for its second click: (device id, deadline in `now_ms`)
    pub armed: Option<(String, f64)>,
    pub was_on: Option<bool>,
    pub ticks: u32,
}

impl Default for PhoneData {
    fn default() -> Self {
        PhoneData { status: Value::Null, rows: Vec::new(), code: None, qr_t: Flow::new(1.0, Feel::POP), on_t: Flow::new(0.0, Feel::FADE), apps: Vec::new(), armed: None, was_on: None, ticks: 0 }
    }
}

// ------------------------------------------------------------------------------------------------ pure helpers

pub fn wall_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
pub fn ago(lang: Lang, now: i64, then: i64) -> String {
    let m = ((now - then).max(0) / 60_000) as i64;
    match m {
        0 => pick(lang, "только что", "just now").to_string(),
        1..=59 => format!("{m} {}", pick(lang, "мин назад", "min ago")),
        60..=1439 => format!("{} {}", m / 60, pick(lang, "ч назад", "h ago")),
        _ => format!("{} {}", m / 1440, pick(lang, "дн назад", "d ago")),
    }
}

/// "AB3D 9XKQ"
pub fn pretty_code(c: &str) -> String {
    if c.len() == 8 {
        format!("{} {}", &c[..4], &c[4..])
    } else {
        c.to_string()
    }
}

/// Horizontal runs of dark modules `(col, row, len)`: fewer rects than modules, no seams.
pub fn runs(rows: &[String]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        let b = row.as_bytes();
        let mut x = 0;
        while x < b.len() {
            if b[x] == b'#' {
                let s = x;
                while x < b.len() && b[x] == b'#' {
                    x += 1;
                }
                out.push((s, y, x - s));
            } else {
                x += 1;
            }
        }
    }
    out
}

fn snap(v: f32, scale: f32) -> f32 {
    (v * scale).round() / scale
}

/// The status line under the title of the QR area: text and whether it is good news.
pub fn status_line(lang: Lang, st: &Value) -> (String, bool) {
    if st["running"] == true {
        let (ip, port) = (st["ip"].as_str().unwrap_or("?"), st["port"].as_u64().unwrap_or(0));
        return (format!("{} {ip}:{port}", pick(lang, "Слушает", "Listening on")), true);
    }
    let t = match st["error"].as_str() {
        Some("no_private_ip") => pick(lang, "Не найден адрес домашней сети: подключитесь к Wi-Fi или кабелю", "No home-network address found: connect to Wi-Fi or a cable"),
        Some("port_busy") => pick(lang, "Все порты от 41717 заняты другими программами", "Every port from 41717 up is taken by other programs"),
        _ => pick(lang, "Запускается…", "Starting…"),
    };
    (t.to_string(), false)
}

fn muted_list(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

// ------------------------------------------------------------------------------------------------ state: replies, steps, clicks

/// Brings the rows in line with the devices of a status answer: new ones flow in, missing ones flow out (also used by the PCs section).
pub fn reconcile(rows: &mut Vec<PRow>, devs: &[Value], reduce: bool) {
    let on = |d: &Value| (d["online"] == true) as u8 as f32;
    for r in rows.iter_mut() {
        let d = devs.iter().find(|d| d["id"].as_str() == Some(&r.id));
        r.present = d.is_some();
        if let Some(d) = d {
            r.dev = d.clone();
            go(&mut r.dot, on(d), reduce);
        }
    }
    for d in devs {
        let id = d["id"].as_str().unwrap_or("").to_string();
        if !id.is_empty() && !rows.iter().any(|r| r.id == id) {
            let mut f = Flow::new(0.0, Feel::FLOW);
            from_to(&mut f, 0.0, 1.0, reduce);
            rows.push(PRow { id, dev: d.clone(), flow: f, dot: Flow::new(on(d), Feel::FADE), present: true });
        }
    }
    for r in rows.iter_mut() {
        go(&mut r.flow, if r.present { 1.0 } else { 0.0 }, reduce);
    }
}

/// Advances the row flows and drops the rows that have left; true while one moves.
pub fn step_rows(rows: &mut Vec<PRow>, dt: f32) -> bool {
    let busy = rows.iter_mut().fold(false, |b, r| tick(&mut r.flow, dt) | tick(&mut r.dot, dt) | b);
    rows.retain(|r| r.present || r.flow.moving() || r.flow.at() > 0.01);
    busy
}

/// An unconfirmed two-click button (Revoke, Unpair) lapses; true on the frame it does (one more frame: the label reads as before).
pub fn lapse(armed: &mut Option<(String, f64)>, now_ms: f64) -> bool {
    armed.take_if(|(_, until)| now_ms >= *until).is_some()
}

impl PhoneData {
    /// The `phone_status` answer: reconcile the rows with the paired phones (new ones flow in, missing ones flow out).
    pub fn apply_status(&mut self, v: Value, reduce: bool) {
        reconcile(&mut self.rows, v["devices"].as_array().map_or(&[], |a| a.as_slice()), reduce);
        self.status = v;
    }

    /// The `phone_pair_code` answer. A different code pops the QR card in again.
    pub fn apply_code(&mut self, v: &Value, reduce: bool) {
        let (Some(code), Some(url)) = (v["code"].as_str(), v["url"].as_str()) else { return };
        if self.code.as_ref().is_some_and(|c| c.url == url) {
            if let Some(c) = &mut self.code {
                c.expires = v["expires"].as_i64().unwrap_or(c.expires);
            }
            return;
        }
        let Ok(q) = pill_ui_core::panel::b::qr::qr(url) else { return };
        self.code = Some(CodeView { code: code.to_string(), url: url.to_string(), expires: v["expires"].as_i64().unwrap_or(0), rows: q.rows() });
        from_to(&mut self.qr_t, 0.0, 1.0, reduce);
    }

    /// Per frame: advance the flows, drop the rows that have left, let an unconfirmed Revoke lapse. True while something moves.
    pub fn step(&mut self, dt: f32, now_ms: f64, enabled: bool, reduce: bool) -> bool {
        let mut busy = false;
        if self.was_on != Some(enabled) {
            if self.was_on.is_none() {
                self.on_t.jump(enabled as u8 as f32);
            } else {
                go(&mut self.on_t, enabled as u8 as f32, reduce);
            }
            if !enabled {
                self.status = Value::Null;
                self.code = None;
                for r in self.rows.iter_mut() {
                    r.present = false;
                    go(&mut r.flow, 0.0, reduce);
                }
            }
            self.was_on = Some(enabled);
        }
        busy |= tick(&mut self.on_t, dt);
        busy |= tick(&mut self.qr_t, dt);
        busy |= step_rows(&mut self.rows, dt);
        busy | lapse(&mut self.armed, now_ms)
    }
}

impl SettingsView {
    /// Called from `step_all`: starts the fetches when the switch flips, steps the flows.
    pub(super) fn step_phone(&mut self, dt: f32, cx: &mut Cx) -> bool {
        let enabled = self.model.get_bool("phone.enabled");
        let flipped_on = self.data.phone.was_on != Some(true) && enabled && self.data.phone.was_on.is_some();
        let (reduce, now) = (self.reduce(), self.now_ms);
        let busy = self.data.phone.step(dt, now, enabled, reduce);
        if flipped_on && self.model.current == "phone" {
            self.phone_fetch(cx, true);
        }
        busy
    }

    /// Status (+ the pairing code while the link is on); `apps` also refreshes the app list.
    pub(super) fn phone_fetch(&mut self, cx: &mut Cx, apps: bool) {
        let mut calls = vec![C::PhoneStatus];
        if self.model.get_bool("phone.enabled") {
            calls.push(C::PhonePairCode { renew: false }); // errors while the link is off, so never asked then
        }
        if apps {
            calls.push(C::PhoneApps);
        }
        self.run(cx, calls);
    }

    /// The ticker of the open Phone page (every 2 s).
    pub(super) fn phone_tick(&mut self, cx: &mut Cx) {
        self.call(cx, C::PcStatus, ""); // the PCs section on the same page
        if !self.model.get_bool("phone.enabled") && self.data.phone.rows.is_empty() {
            return; // nothing to watch while the link is off
        }
        self.data.phone.ticks += 1;
        let apps = self.data.phone.ticks % 5 == 0;
        self.phone_fetch(cx, apps);
    }

    /// Answers of the phone commands; true when `call` was one of them.
    pub(super) fn phone_reply(&mut self, call: &C, res: &Result<Value, String>, cx: &mut Cx) -> bool {
        let reduce = self.reduce();
        match call {
            C::PhoneStatus => self.data.phone.apply_status(res.clone().unwrap_or(Value::Null), reduce),
            C::PhonePairCode { .. } => {
                if let Ok(v) = res {
                    self.data.phone.apply_code(v, reduce);
                }
            }
            C::PhoneApps => {
                if let Ok(v) = res {
                    let mut apps: Vec<(String, String)> = v["apps"].as_object().map(|m| m.iter().map(|(k, n)| (k.clone(), n.as_str().unwrap_or(k).to_string())).collect()).unwrap_or_default();
                    for m in muted_list(&v["muted"]) {
                        if !apps.iter().any(|(k, _)| *k == m) {
                            apps.push((m.clone(), m));
                        }
                    }
                    apps.sort_by_key(|(k, n)| (n.to_lowercase(), k.clone()));
                    self.data.phone.apps = apps;
                }
            }
            C::PhoneRevoke { .. } => {
                self.call(cx, C::PhoneStatus, "");
            }
            _ => return self.pcs_reply(call, res),
        }
        true
    }

    pub(super) fn phone_act(&mut self, a: Act, cx: &mut Cx) {
        match a {
            Act::PhoneRenew => {
                self.call(cx, C::PhonePairCode { renew: true }, "phone.link");
            }
            Act::PhoneRevoke(id) => {
                if self.two_click(false, &id) {
                    self.call(cx, C::PhoneRevoke { id }, "phone.devices");
                }
            }
            Act::PhoneMute(pkg) => {
                let mut muted = muted_list(&self.model.get("phone.mutedApps"));
                if let Some(i) = muted.iter().position(|m| *m == pkg) {
                    muted.remove(i);
                    self.start_pop(key(&format!("chip:phoneapp:{pkg}"))); // switched back on: the chip pops
                } else {
                    muted.push(pkg);
                }
                if let Ok(calls) = self.model.try_set("phone.mutedApps", json!(muted)) {
                    self.run(cx, calls);
                }
                self.sync_page();
            }
            _ => {}
        }
        cx.request_frame();
    }

    /// A Revoke / Unpair button (`pcs`: the PCs list, else the phones): the first click arms it, the second within `ARM_MS` returns true
    /// and the row leaves at once (the next status would take it out anyway).
    pub(super) fn two_click(&mut self, pcs: bool, id: &str) -> bool {
        let (now, reduce) = (self.now_ms, self.reduce());
        let (rows, armed) = if pcs { (&mut self.data.pcs.rows, &mut self.data.pcs.armed) } else { (&mut self.data.phone.rows, &mut self.data.phone.armed) };
        if !armed.as_ref().is_some_and(|(a, until)| a == id && now < *until) {
            *armed = Some((id.to_string(), now + ARM_MS));
            return false;
        }
        *armed = None;
        if let Some(r) = rows.iter_mut().find(|r| r.id == id) {
            r.present = false;
            go(&mut r.flow, 0.0, reduce);
        }
        true
    }
}

// ------------------------------------------------------------------------------------------------ widgets

impl SettingsView {
    /// Paints the label of a custom row; returns (left edge, inner width, label height).
    pub(super) fn head(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> (f32, f32, f32) {
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        (x0, w, m.nh)
    }

    /// The device list of the phones or (`pcs`) the PCs: the label, `empty` or one row per device flowing in and out (`body` paints its text from
    /// (x0, y) over the width w), and the Revoke / Unpair button of a present device on the right.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dev_list(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool, pcs: bool, empty: &str, body: impl Fn(&mut Self, &Dc, &PRow, f32, f32, f32)) -> f32 {
        let (x0, w, nh) = self.head(d, r, full, paint);
        let mut y = full.y + nh;
        let (rows, armed) = if pcs { (&self.data.pcs.rows, &self.data.pcs.armed) } else { (&self.data.phone.rows, &self.data.phone.armed) };
        let (rows, armed) = (rows.clone(), armed.as_ref().map(|a| a.0.clone()));
        if rows.is_empty() {
            return y + self.para_at(d, paint, x0, y, w, empty, st_hint(), d.p.dim) + 12.0 - full.y;
        }
        let rh = if pcs { 84.0 } else { 56.0 };
        for r in &rows {
            let f = r.flow.at().clamp(0.0, 1.0);
            if paint && rh * f > 0.5 {
                d.cv.with_clip(Rect::new(full.x, y, full.w, rh * f), |cv| {
                    cv.with_opacity(f, |cv| {
                        cv.with_offset(12.0 * (1.0 - f), 0.0, |cv| {
                            cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), d.p.row_line);
                            body(self, d, r, x0, y, w);
                        })
                    })
                });
                if r.present {
                    let (ru, en) = match (pcs, armed.as_deref() == Some(&r.id)) {
                        (false, false) => ("Отозвать", "Revoke"),
                        (false, true) => ("Точно отозвать?", "Really revoke?"),
                        (true, false) => ("Отключить", "Unpair"),
                        (true, true) => ("Точно отключить?", "Really unpair?"),
                    };
                    let (id, act) = if pcs { ("pcs:unpair", Act::PcPeer(r.id.clone(), "unpair", true)) } else { ("phone:rev", Act::PhoneRevoke(r.id.clone())) };
                    self.btn_at(d, true, &format!("{id}:{}", r.id), pick(d.lang, ru, en), ButtonKind::Danger, 0.0, Some(full.right() - PAD_X), y + if pcs { 8.0 } else { 12.0 }, act, true);
                }
            }
            y += rh * f;
        }
        y + 8.0 - full.y
    }

    pub(super) fn w_phone_link(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w, nh) = self.head(d, r, full, paint);
        let y = full.y + nh - 11.0 + 4.0;
        let h = QR_SIDE;
        if !paint {
            return y + h + 16.0 - full.y;
        }
        let on = self.data.phone.on_t.at01();
        let area = Rect::new(x0, y, w, h);
        // the calm panel while the link is off
        if on < 0.999 {
            cv.with_opacity(1.0 - on, |cv| {
                cv.fill_rr(area, 14.0, p.ctl);
                cv.text(Rect::new(area.x, area.cy() - 20.0, area.w, 20.0), pick(d.lang, "Связь с телефоном выключена", "Phone link is off"), Style::new(14.0, 600).center(), p.text);
                cv.text(Rect::new(area.x + 20.0, area.cy() + 4.0, area.w - 40.0, 18.0), pick(d.lang, "Включите переключатель выше, и здесь появится QR-код", "Turn the switch above on and the QR code appears here"), st_hint().center(), p.dim);
            });
        }
        if on > 0.001 {
            let side = Rect::new(x0 + QR_SIDE + 20.0, y, (w - QR_SIDE - 20.0).max(80.0), h);
            cv.with_opacity(on, |cv| self.paint_link_on(d, area, side));
        }
        y + h + 16.0 - full.y
    }

    fn paint_link_on(&mut self, d: &Dc, area: Rect, side: Rect) {
        let (cv, p) = (d.cv, d.p);
        let card = Rect::new(area.x, area.y, QR_SIDE, QR_SIDE);
        let t = self.data.phone.qr_t.at();
        match &self.data.phone.code {
            Some(c) => {
                let a = self.data.phone.qr_t.at01();
                cv.with_opacity(a, |cv| {
                    cv.with_scale_about(card.center(), 0.9 + 0.1 * t, |cv| {
                        cv.fill_rr(card, 12.0, Color::rgb8(255, 255, 255));
                        let quiet = 4usize;
                        let n = c.rows.len() + 2 * quiet;
                        let px = QR_SIDE / n as f32;
                        let s = cv.scale;
                        cv.with_clip_rr(card, 12.0, |cv| {
                            for (x, y, run) in runs(&c.rows) {
                                let (x0, y0) = (card.x + (x + quiet) as f32 * px, card.y + (y + quiet) as f32 * px);
                                let (sx, sy) = (snap(x0, s), snap(y0, s));
                                cv.fill_rect(Rect::new(sx, sy, snap(x0 + run as f32 * px, s) - sx, snap(y0 + px, s) - sy), Color::rgb8(0, 0, 0));
                            }
                        });
                    });
                });
            }
            None => cv.fill_rr(card, 12.0, p.ctl),
        }
        let (line, good) = status_line(d.lang, &self.data.phone.status);
        let mut y = side.y + 2.0;
        cv.text(Rect::new(side.x, y, side.w, 18.0), &line, Style::new(12.0, 500).ellipsis(), if good { p.green } else { p.amber });
        y += 28.0;
        if let Some(c) = &self.data.phone.code {
            cv.text(Rect::new(side.x, y, side.w, 34.0), &pretty_code(&c.code), Style::new(28.0, 700).mono().track(0.08), p.text);
            y += 38.0;
            let min = ((c.expires - wall_ms()).max(0) / 60_000) as u32 + 1;
            let hint = format!("{} {} {}", pick(d.lang, "Одноразовый код, ещё", "One-time code, valid for"), min.min(5), pick(d.lang, "мин", "min"));
            cv.text(Rect::new(side.x, y, side.w, 16.0), &hint, st_hint(), p.dim);
            y += 20.0;
            cv.text(Rect::new(side.x, y, side.w, 16.0), &c.url, Style::new(11.0, 400).mono().ellipsis(), p.faint);
        }
        let by = area.bottom() - 32.0;
        let enabled = self.data.phone.code.is_some();
        self.btn_at(d, true, "phone:renew", pick(d.lang, "Новый код", "New code"), ButtonKind::Normal, side.x, None, by, Act::PhoneRenew, enabled);
    }

    pub(super) fn w_phone_devices(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let empty = pick(d.lang, "Пока ни одного телефона. Отсканируйте QR-код выше.", "No phones yet. Scan the QR code above.");
        self.dev_list(d, r, full, paint, false, empty, |_, d, r, x0, y, w| {
            let (cv, p, dev) = (d.cv, d.p, &r.dev);
            let sub = [dev["model"].as_str().unwrap_or(""), dev["os"].as_str().unwrap_or("")].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
            cv.text(Rect::new(x0, y + 8.0, w - 150.0, 18.0), dev["name"].as_str().unwrap_or("?"), st_title(), p.text);
            cv.text(Rect::new(x0, y + 26.0, w - 150.0, 14.0), &sub, st_hint().ellipsis(), p.dim);
            let conn = dev["connected"] == true;
            let mut st = if conn { pick(d.lang, "на связи", "connected now").to_string() } else { format!("{} {}", pick(d.lang, "был", "last seen"), ago(d.lang, wall_ms(), dev["lastSeen"].as_i64().unwrap_or(0))) };
            if let Some(b) = dev["battery"].as_i64().filter(|_| conn) {
                st = format!("{st} · {b} %{}", if dev["charging"] == true { " ⚡" } else { "" });
            }
            cv.text(Rect::new(x0, y + 40.0, w - 150.0, 14.0), &st, st_hint().ellipsis(), if conn { p.green } else { p.faint });
        })
    }

    pub(super) fn w_phone_apps(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let p = d.p;
        let (x0, w, nh) = self.head(d, r, full, paint);
        let mut y = full.y + nh - 11.0 + 6.0;
        if self.data.phone.apps.is_empty() {
            let msg = pick(d.lang, "Пока пусто: приложения появятся после первого уведомления с телефона.", "Nothing yet: apps appear after the first notification from the phone.");
            return y + self.para_at(d, paint, x0, y, w, msg, st_hint(), p.dim) + 12.0 - full.y;
        }
        let muted = muted_list(&self.model.get("phone.mutedApps"));
        let mut x = x0;
        for (pkg, name) in self.data.phone.apps.clone() {
            let cname = format!("phoneapp:{pkg}");
            let cw = self.chip(d, false, 0.0, 0.0, &cname, &name, &[], false, Act::None);
            if x > x0 && x + cw > x0 + w {
                x = x0;
                y += 38.0;
            }
            self.chip(d, paint, x, y, &cname, &name, &[], !muted.contains(&pkg), Act::PhoneMute(pkg));
            x += cw + 8.0;
        }
        y + 30.0 + 14.0 - full.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    #[test]
    fn relative_times() {
        let t = 1_000_000_000_000;
        assert_eq!(ago(Lang::En, t, t - 5_000), "just now");
        assert_eq!(ago(Lang::En, t, t - 5 * 60_000), "5 min ago");
        assert_eq!(ago(Lang::Ru, t, t - 3 * 3_600_000), "3 ч назад");
        assert_eq!(ago(Lang::En, t, t - 49 * 3_600_000), "2 d ago");
        assert_eq!(ago(Lang::En, t, t + 9_000), "just now", "a clock that runs behind never prints a negative");
    }

    #[test]
    fn code_and_qr_runs() {
        assert_eq!(pretty_code("AB3D9XKQ"), "AB3D 9XKQ");
        assert_eq!(runs(&["##.#".to_string(), "....".to_string(), ".###".to_string()]), vec![(0, 0, 2), (3, 0, 1), (1, 2, 3)]);
    }

    #[test]
    fn status_lines_name_the_problem() {
        let ok = status_line(Lang::En, &json!({"running": true, "ip": "192.168.1.5", "port": 41717}));
        assert_eq!(ok, ("Listening on 192.168.1.5:41717".to_string(), true));
        assert!(status_line(Lang::En, &json!({"running": false, "error": "no_private_ip"})).0.contains("home-network"));
        assert!(status_line(Lang::Ru, &json!({"running": false, "error": "port_busy"})).0.contains("заняты"));
        assert!(!status_line(Lang::En, &json!({"running": false})).1);
    }

    #[test]
    fn devices_flow_in_and_out() {
        let _m = motion_on();
        let mut d = PhoneData::default();
        d.apply_status(json!({"devices": [{"id": "a", "name": "Pixel"}, {"id": "b", "name": "Galaxy"}]}), false);
        assert_eq!(d.rows.len(), 2);
        assert!(d.rows[0].flow.at() < 0.5, "they enter softly, not at once");
        for _ in 0..60 {
            d.step(0.016, 0.0, true, false);
        }
        assert!(d.rows.iter().all(|r| r.flow.at() > 0.95));
        d.apply_status(json!({"devices": [{"id": "b", "name": "Galaxy"}]}), false);
        assert_eq!(d.rows.len(), 2, "the revoked phone keeps its row while it leaves");
        for _ in 0..90 {
            d.step(0.016, 0.0, true, false);
        }
        assert_eq!(d.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["b"]);
        assert!(!d.step(0.016, 0.0, true, false), "settled: no frames wanted");
    }

    #[test]
    fn reduce_motion_lands_at_once_and_a_new_code_pops_the_qr() {
        let _m = motion_on();
        let mut d = PhoneData::default();
        d.apply_status(json!({"devices": [{"id": "a"}]}), true);
        assert_eq!(d.rows[0].flow.at(), 1.0);
        d.apply_code(&json!({"code": "AB3D9XKQ", "url": "http://192.168.1.5:41717/p?c=AB3D9XKQ", "expires": 1}), false);
        assert!(d.code.as_ref().is_some_and(|c| c.rows.len() > 20));
        assert!(d.qr_t.at() == 0.0 && d.qr_t.target() == 1.0, "pops in from nothing");
        d.qr_t.jump(1.0);
        d.apply_code(&json!({"code": "AB3D9XKQ", "url": "http://192.168.1.5:41717/p?c=AB3D9XKQ", "expires": 99}), false);
        assert_eq!(d.qr_t.at(), 1.0, "the same code does not pop again");
        d.apply_code(&json!({"code": "ZZZZ9999", "url": "http://192.168.1.5:41717/p?c=ZZZZ9999", "expires": 1}), true);
        assert_eq!(d.qr_t.at(), 1.0, "reduce motion: no pop");
    }

    #[test]
    fn switching_off_clears_the_code_and_closes_the_rows() {
        let _m = motion_on();
        let mut d = PhoneData::default();
        d.step(0.016, 0.0, true, false);
        d.apply_status(json!({"devices": [{"id": "a"}]}), true);
        d.apply_code(&json!({"code": "AB3D9XKQ", "url": "http://192.168.1.5:41717/p?c=AB3D9XKQ", "expires": 1}), true);
        d.step(0.016, 0.0, false, true);
        assert!(d.code.is_none() && d.rows.is_empty());
        assert_eq!(d.on_t.at(), 0.0);
    }

    #[test]
    fn an_unconfirmed_revoke_lapses() {
        let mut d = PhoneData::default();
        d.armed = Some(("a".into(), 100.0));
        assert!(!d.step(0.016, 50.0, false, true) && d.armed.is_some());
        assert!(d.step(0.016, 100.0, false, true) && d.armed.is_none());
    }
}
