//! Custom widgets of the "My other PCs" section of the Phone page (docs/pc-link.md): `pcs.link` (show a token, paste a token, the pairing card with
//! Allow / Deny) and `pcs.devices` (the paired PCs: online dot, the two clipboard switches, Unpair with a second click). The data is `pc_status`,
//! polled by the page's 2 s ticker; nothing here runs while the page is closed. Motion: the pairing card fades and rises whenever its state changes,
//! the token's life drains as one flow per poll, the rows and the online dots are flows, a switched-on chip pops; Reduce motion lands it all at once.
use super::cw_phone::{ago, reconcile, step_rows, wall_ms, PRow};
use super::flow::{from_to, go, tick};
use super::page::RowRt;
use super::pal::mix;
use super::rows::*;
use super::ui::{key, Act};
use super::{Pend, SettingsView};
use crate::ui::liquid::{Feel, Flow};
use crate::ui::view::Cx;
use glasskit::color::Color;
use glasskit::controls::ButtonKind;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::calls::BackendCall as C;
use pill_ui_core::settings::tr::pick;
use serde_json::Value;
use std::collections::HashMap;

const ROW_H: f32 = 84.0;
/// A pending Unpair waits this long for its second click.
const ARM_MS: f64 = 4000.0;
/// A shown token lives this long (phone/pc.rs `SHOW_TTL_MS`).
const LIFE_MS: f64 = 600_000.0;

pub struct PcsData {
    /// the last `pc_status`
    pub status: Value,
    pub rows: Vec<PRow>,
    /// per PC: 0 = offline, 1 = online (the dot's colour flows between them)
    dots: HashMap<String, Flow>,
    /// the pairing card (0 -> 1) fading in whenever its `mode/state` changes
    card: Flow,
    shown: String,
    /// the token's remaining life (1 -> 0), re-aimed on every poll
    life: Flow,
    pub armed: Option<(String, f64)>,
    /// until when the Copy button says "Copied" (`now_ms`)
    copied: f64,
    /// the pasted text was not a token
    pub bad_token: bool,
}

impl Default for PcsData {
    fn default() -> Self {
        PcsData { status: Value::Null, rows: Vec::new(), dots: HashMap::new(), card: Flow::new(1.0, Feel::FLOW), shown: String::new(), life: Flow::new(1.0, Feel::FLOW), armed: None, copied: 0.0, bad_token: false }
    }
}

impl PcsData {
    fn pair(&self) -> (&str, &str) {
        (self.status["pair"]["mode"].as_str().unwrap_or(""), self.status["pair"]["state"].as_str().unwrap_or(""))
    }

    /// The `pc_status` answer: reconcile the rows and the dots, restart the card's entrance when the pairing moved on, re-aim the token's life.
    pub fn apply(&mut self, v: Value, wall: i64, reduce: bool) {
        let devs: Vec<Value> = v["peers"].as_array().cloned().unwrap_or_default();
        reconcile(&mut self.rows, &devs, reduce);
        for d in &devs {
            let (id, on) = (d["id"].as_str().unwrap_or("").to_string(), (d["online"] == true) as u8 as f32);
            match self.dots.get_mut(&id) {
                Some(f) => go(f, on, reduce),
                None => {
                    self.dots.insert(id, Flow::new(on, Feel::FADE));
                }
            }
        }
        let now = format!("{}/{}", v["pair"]["mode"].as_str().unwrap_or(""), v["pair"]["state"].as_str().unwrap_or(""));
        if now != self.shown {
            self.shown = now;
            from_to(&mut self.card, 0.0, 1.0, reduce);
        }
        go(&mut self.life, ((v["pair"]["expires"].as_f64().unwrap_or(0.0) - wall as f64) / LIFE_MS).clamp(0.0, 1.0) as f32, reduce);
        self.status = v;
    }

    /// Per frame; true while something moves (the stripe of a pending pairing slides on its own).
    pub fn step(&mut self, dt: f32, now_ms: f64) -> bool {
        let mut busy = tick(&mut self.card, dt);
        busy |= tick(&mut self.life, dt);
        busy |= step_rows(&mut self.rows, dt);
        for f in self.dots.values_mut() {
            busy |= tick(f, dt);
        }
        if self.armed.as_ref().is_some_and(|(_, until)| now_ms >= *until) {
            self.armed = None;
            busy = true; // one more frame: the button reads "Unpair" again
        }
        busy |= self.copied > 0.0;
        if now_ms >= self.copied {
            self.copied = 0.0;
        }
        busy || matches!(self.pair(), ("enter", "waiting") | ("show", "allowed"))
    }
}

impl SettingsView {
    pub(super) fn pcs_act(&mut self, a: Act, cx: &mut Cx) {
        match a {
            Act::PcPair(op) => {
                self.data.pcs.bad_token = false;
                self.call(cx, C::PcPair { op: op.into(), token: None }, "");
            }
            Act::PcJoin => {
                let t = self.data.fields.get("token").map(|f| f.text()).unwrap_or_default();
                if !t.trim().is_empty() {
                    self.data.pcs.bad_token = false;
                    self.call(cx, C::PcPair { op: "join".into(), token: Some(t) }, "pcs.join");
                }
            }
            Act::PcCopy => {
                if let Some(t) = self.data.pcs.status["pair"]["token"].as_str() {
                    cx.set_clipboard_text(t);
                    self.data.pcs.copied = self.now_ms + 1500.0;
                }
            }
            Act::PcPeer(id, "unpair", _) => {
                if self.data.pcs.armed.as_ref().is_some_and(|(a, until)| *a == id && self.now_ms < *until) {
                    self.data.pcs.armed = None;
                    self.call(cx, C::PcPeer { id: id.clone(), op: "unpair".into(), on: true }, "");
                    let reduce = self.reduce();
                    if let Some(r) = self.data.pcs.rows.iter_mut().find(|r| r.id == id) {
                        r.present = false; // the row leaves at once, the next status would take it out anyway
                        go(&mut r.flow, 0.0, reduce);
                    }
                } else {
                    self.data.pcs.armed = Some((id, self.now_ms + ARM_MS));
                }
            }
            Act::PcPeer(id, op, on) => {
                if on {
                    self.start_pop(key(&format!("chip:pcc:{id}:{op}")));
                }
                self.call(cx, C::PcPeer { id, op: op.into(), on }, "");
            }
            _ => {}
        }
        cx.request_frame();
    }

    /// Answers of the PC commands; true when `p` was one of them.
    pub(super) fn pcs_reply(&mut self, p: &Pend, res: &Result<Value, String>) -> bool {
        if !matches!(p.call, C::PcStatus | C::PcPair { .. } | C::PcPeer { .. }) {
            return false;
        }
        let reduce = self.reduce();
        match res {
            Ok(v) => {
                if p.tag == "pcs.join" {
                    if let Some(f) = self.data.fields.get_mut("token") {
                        f.set_text("");
                    }
                }
                self.data.pcs.apply(v.clone(), wall_ms(), reduce);
            }
            Err(_) => self.data.pcs.bad_token = p.tag == "pcs.join",
        }
        true
    }

    pub(super) fn w_pcs_link(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let y = full.y + m.nh - 11.0 + 6.0;
        let mut h = 0.0;
        if paint {
            let (a, rise) = (self.data.pcs.card.at01(), (1.0 - self.data.pcs.card.at().min(1.0)) * 8.0);
            d.cv.with_opacity(a, |cv| cv.with_offset(0.0, rise, |_| h = self.pcs_card(d, x0, y, w, true)));
        } else {
            h = self.pcs_card(d, x0, y, w, false);
        }
        y + h + 12.0 - full.y
    }

    /// The body under the label: the form, or the card of the pairing under way. Returns its height.
    fn pcs_card(&mut self, d: &Dc, x0: f32, y0: f32, w: f32, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let st = self.data.pcs.status["pair"].clone();
        let (mode, state) = (st["mode"].as_str().unwrap_or(""), st["state"].as_str().unwrap_or(""));
        let peer = st["peer"]["name"].as_str().unwrap_or("").to_string();
        let mut y = y0;
        let right = x0 + w;
        match (mode, state) {
            ("show", "waiting") => {
                let label = if self.data.pcs.copied > 0.0 { pick(d.lang, "Скопировано", "Copied") } else { pick(d.lang, "Копировать", "Copy") };
                let b = self.btn_at(d, paint, "pcs:copy", label, ButtonKind::Normal, 0.0, Some(right), y, Act::PcCopy, true);
                if paint {
                    cv.text(Rect::new(x0, y, b.x - 8.0 - x0, 32.0), st["token"].as_str().unwrap_or(""), Style::new(15.0, 700).mono().ellipsis(), p.text);
                }
                y += 42.0;
                let mins = ((st["expires"].as_i64().unwrap_or(0) - wall_ms()).max(0) / 60_000 + 1).min(10);
                let hint = format!("{} {mins} {}", pick(d.lang, "Вставьте его на другом компьютере (Настройки, Телефон, Мои другие компьютеры). Код одноразовый, ещё", "Paste it on your other PC (Settings, Phone, My other PCs). One use, valid for"), pick(d.lang, "мин", "min"));
                y += self.para_at(d, paint, x0, y, w, &hint, st_hint(), p.dim) + 8.0;
                if paint {
                    let bar = Rect::new(x0, y, w, 6.0);
                    cv.fill_rr(bar, 3.0, p.ctl);
                    cv.fill_rr(Rect::new(x0, y, (w * self.data.pcs.life.at().clamp(0.0, 1.0)).max(6.0), 6.0), 3.0, p.accent);
                }
                y += 18.0;
                self.btn_at(d, paint, "pcs:cancel", pick(d.lang, "Отмена", "Cancel"), ButtonKind::Normal, x0, None, y, Act::PcPair("cancel"), true);
                y += 32.0;
            }
            ("show", "request") => {
                let line = format!("{peer} ({}) {}", st["peer"]["os"].as_str().unwrap_or("Windows"), pick(d.lang, "хочет подключиться", "wants to pair"));
                if paint {
                    cv.text(Rect::new(x0, y, w, 18.0), &line, st_title(), p.text);
                }
                y += 22.0;
                y += self.para_at(d, paint, x0, y, w, pick(d.lang, "Разрешайте, только если вы сами ввели код на том компьютере. Общий буфер обмена заработает сразу.", "Allow it only if you entered the token on that PC yourself. The shared clipboard starts at once."), st_hint(), p.dim) + 8.0;
                let a = self.btn_at(d, paint, "pcs:allow", pick(d.lang, "Разрешить", "Allow"), ButtonKind::Primary, x0, None, y, Act::PcPair("allow"), true);
                self.btn_at(d, paint, "pcs:deny", pick(d.lang, "Отклонить", "Deny"), ButtonKind::Normal, a.right() + 8.0, None, y, Act::PcPair("deny"), true);
                y += 32.0;
            }
            ("show", "allowed") | ("enter", "waiting") => {
                let (line, cancel) = if state == "allowed" { (pick(d.lang, "Соединяю…", "Connecting…"), false) } else { (pick(d.lang, "Жду, пока другой компьютер разрешит…", "Waiting for the other PC to allow it…"), true) };
                if paint {
                    cv.text(Rect::new(x0, y, w, 18.0), line, st_title(), p.text);
                }
                y += 26.0;
                if paint {
                    self.bar(d, Rect::new(x0, y, w, 6.0), 0, true);
                }
                y += 6.0;
                if cancel {
                    y += 12.0;
                    self.btn_at(d, paint, "pcs:cancel", pick(d.lang, "Отмена", "Cancel"), ButtonKind::Normal, x0, None, y, Act::PcPair("cancel"), true);
                    y += 32.0;
                }
            }
            _ => {
                let b = self.btn_at(d, paint, "pcs:start", pick(d.lang, "Показать код", "Show a token"), ButtonKind::Primary, x0, None, y, Act::PcPair("start"), true);
                if paint {
                    cv.text(Rect::new(b.right() + 12.0, y, w - b.w - 12.0, 32.0), pick(d.lang, "здесь; вставьте его на другом компьютере", "here; paste it on the other PC"), st_hint().ellipsis(), p.dim);
                }
                y += 44.0;
                let pasted = self.data.fields.get("token").is_some_and(|f| !f.text().trim().is_empty());
                let b = self.btn_at(d, paint, "pcs:join", pick(d.lang, "Подключить", "Pair"), ButtonKind::Normal, 0.0, Some(right), y, Act::PcJoin, pasted);
                if paint {
                    self.paint_cfield(d, "token", Rect::new(x0, y, b.x - 8.0 - x0, 32.0), pick(d.lang, "Вставьте код с другого компьютера", "Paste the token from the other PC"));
                }
                y += 32.0;
                let msg: Option<(String, Color)> = match (mode, state) {
                    _ if self.data.pcs.bad_token => Some((pick(d.lang, "Это не код Pill: проверьте, что он вставлен целиком.", "That is not a Pill token: check that you pasted all of it.").to_string(), p.red)),
                    (_, "done") => Some((format!("{} {peer}", pick(d.lang, "Компьютер подключён:", "Paired with")), p.green)),
                    (_, "denied") => Some((pick(d.lang, "Другой компьютер отклонил подключение.", "The other PC said no.").to_string(), p.red)),
                    ("show", "failed") => Some((pick(d.lang, "Код истёк. Покажите новый.", "The token expired. Show a new one.").to_string(), p.amber)),
                    (_, "failed") => Some((pick(d.lang, "Нет ответа. Проверьте код и что другой компьютер его показывает.", "No answer. Check the token and that the other PC is showing it.").to_string(), p.amber)),
                    _ => None,
                };
                if let Some((t, c)) = msg {
                    y += 8.0;
                    y += self.para_at(d, paint, x0, y, w, &t, st_hint(), c);
                }
            }
        }
        y - y0
    }

    pub(super) fn w_pcs_devices(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let mut y = full.y + m.nh;
        if self.data.pcs.rows.is_empty() {
            let msg = pick(d.lang, "Пока ни одного компьютера. Покажите код на одном и вставьте на другом.", "No PCs yet. Show a token on one and paste it on the other.");
            y += self.para_at(d, paint, x0, y, w, msg, st_hint(), p.dim) + 12.0;
            return y - full.y;
        }
        let now = wall_ms();
        let armed = self.data.pcs.armed.as_ref().map(|(i, _)| i.clone());
        for i in 0..self.data.pcs.rows.len() {
            let (id, dev, f) = {
                let r = &self.data.pcs.rows[i];
                (r.id.clone(), r.dev.clone(), r.flow.at().clamp(0.0, 1.0))
            };
            let h = ROW_H * f;
            if paint && h > 0.5 {
                let dot = self.data.pcs.dots.get(&id).map_or(0.0, |f| f.at());
                let (to, from) = (dev["clipTo"] != false, dev["clipFrom"] != false);
                let rr = Rect::new(full.x, y, full.w, h);
                cv.with_clip(rr, |cv| {
                    cv.with_opacity(f, |cv| {
                        cv.with_offset(12.0 * (1.0 - f), 0.0, |cv| {
                            cv.fill_rect(Rect::new(full.x, y, full.w, 1.0), p.row_line);
                            cv.fill_circle((x0 + 5.0, y + 17.0), 4.5, mix(p.faint, p.green, dot));
                            cv.text(Rect::new(x0 + 18.0, y + 8.0, w - 170.0, 18.0), dev["name"].as_str().unwrap_or("?"), st_title(), p.text);
                            let state = if dev["online"] == true {
                                pick(d.lang, if dev["route"] == "lan" { "на связи · по домашней сети" } else { "на связи · через интернет" }, if dev["route"] == "lan" { "online · on the home network" } else { "online · over the internet" }).to_string()
                            } else {
                                format!("{} {}", pick(d.lang, "был на связи", "last seen"), ago(d.lang, now, dev["lastSeen"].as_i64().unwrap_or(0)))
                            };
                            let sub = [dev["os"].as_str().unwrap_or(""), state.as_str()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" · ");
                            cv.text(Rect::new(x0 + 18.0, y + 26.0, w - 170.0, 14.0), &sub, st_hint().ellipsis(), if dev["online"] == true { p.green } else { p.faint });
                            let (to_l, from_l) = (pick(d.lang, "Мой буфер идёт сюда", "My clipboard goes here"), pick(d.lang, "Буфер отсюда приходит", "Its clipboard comes here"));
                            let wd = self.chip(d, true, x0, y + 46.0, &format!("pcc:{id}:clipTo"), to_l, &[], to, Act::PcPeer(id.clone(), "clipTo", !to));
                            self.chip(d, true, x0 + wd + 8.0, y + 46.0, &format!("pcc:{id}:clipFrom"), from_l, &[], from, Act::PcPeer(id.clone(), "clipFrom", !from));
                        });
                    });
                });
                if self.data.pcs.rows[i].present {
                    let label = if armed.as_deref() == Some(id.as_str()) { pick(d.lang, "Точно отключить?", "Really unpair?") } else { pick(d.lang, "Отключить", "Unpair") };
                    self.btn_at(d, true, &format!("pcs:unpair:{id}"), label, ButtonKind::Danger, 0.0, Some(full.right() - PAD_X), y + 8.0, Act::PcPeer(id.clone(), "unpair", true), true);
                }
            }
            y += h;
        }
        y + 8.0 - full.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use serde_json::json;

    fn waiting() -> Value {
        json!({ "peers": [{ "id": "a", "name": "A", "online": true }], "pair": { "mode": "show", "state": "waiting", "token": "PILL-X", "expires": 600_000 } })
    }

    #[test]
    fn the_card_the_rows_and_the_dots_move_softly_and_then_stop_asking_for_frames() {
        let _m = motion_on();
        let mut d = PcsData::default();
        d.apply(waiting(), 0, false);
        assert!(d.card.at() < 0.5 && d.rows[0].flow.at() < 0.5, "they enter softly, not at once");
        for _ in 0..120 {
            d.step(0.016, 0.0);
        }
        assert!(d.card.at() > 0.95 && d.rows[0].flow.at() > 0.95);
        assert!(!d.step(0.016, 0.0), "settled: no frames wanted");
        // the same state again does not restart the card; a new state does
        d.apply(waiting(), 0, false);
        assert!(d.card.at() > 0.95);
        d.apply(json!({ "peers": [], "pair": { "mode": "show", "state": "request" } }), 0, false);
        assert!(d.card.at() < 0.5, "the card enters again");
        // a pending pairing keeps the stripe sliding, so frames keep coming only then
        d.apply(json!({ "peers": [], "pair": { "mode": "enter", "state": "waiting" } }), 0, false);
        for _ in 0..120 {
            d.step(0.016, 0.0);
        }
        assert!(d.step(0.016, 0.0));
    }

    #[test]
    fn reduce_motion_lands_at_once() {
        let _m = motion_on();
        let mut d = PcsData::default();
        d.apply(waiting(), 0, true);
        assert_eq!((d.card.at(), d.rows[0].flow.at(), d.dots["a"].at()), (1.0, 1.0, 1.0));
        assert_eq!(d.life.at(), 1.0, "a fresh token is full");
    }

    #[test]
    fn an_unconfirmed_unpair_lapses() {
        let mut d = PcsData::default();
        d.armed = Some(("a".into(), 100.0));
        assert!(!d.step(0.016, 50.0) && d.armed.is_some());
        assert!(d.step(0.016, 100.0) && d.armed.is_none());
    }
}
