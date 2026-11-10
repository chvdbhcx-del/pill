//! Painting of the Phone tab. Layout is computed from the animated values (`open` / `reply` flows of each row), never the other way round; the
//! click targets are registered here in window coordinates (the scroll offset is subtracted) and clipped to the body, so a row scrolled out of
//! sight cannot be clicked.
use super::phone::{PhoneView, K};
use super::phone_card::LEFT_W;
use crate::ui::tabs::b::kit::{button, button_w, Bk, Clicks};
use crate::ui::view::Cx;
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::Color;
use pill_ui_core::panel::b::Tr;
use pill_ui_core::panel::c::phone::{ago, size_text, ReplyState, Sub};

const ROW_H: f32 = 80.0;
const REPLY_H: f32 = 40.0;
const ROW_GAP: f32 = 6.0;
const FILE_H: f32 = 54.0;
const ZONE_H: f32 = 72.0;
/// the list pads this much on the right for the scrollbar
const BAR: f32 = 8.0;
const BTN_H: f32 = 28.0;

/// Registers a click target given in content coordinates: window coordinates, clipped to the visible body.
pub(super) fn reg(clicks: &mut Clicks<K>, k: K, r: Rect, body: Rect, off: f32) {
    let w = Rect::new(r.x + body.x, r.y + body.y - off, r.w, r.h);
    let (top, bottom) = (w.y.max(body.y), w.bottom().min(body.bottom()));
    if bottom - top > 4.0 {
        clicks.add(k, Rect::new(w.x, top, w.w, bottom - top));
    }
}

fn letter(s: &str) -> String {
    s.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into())
}

/// Horizontal runs of dark modules `(col, row, length)` of a QR given as rows of `#` / `.` (fewer rects, no seams).
fn qr_runs(rows: &[String]) -> Vec<(usize, usize, usize)> {
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

impl PhoneView {
    pub(super) fn draw_all(&mut self, cv: &Canvas, th: &Theme, _cx: &Cx) {
        self.clicks.begin();
        self.field.rect = Rect::default();
        self.draw_header(cv, th);
        if let Some(msg) = self.model.empty_text() {
            self.draw_off(cv, th, &msg);
            return;
        }
        match self.model.sub() {
            Sub::Notifs => self.draw_notifs(cv, th),
            Sub::Files => self.draw_files(cv, th),
            Sub::Phone => self.draw_phone(cv, th),
        }
    }

    // ------------------------------------------------------------------------------------------------------------ header
    fn draw_header(&mut self, cv: &Canvas, th: &Theme) {
        let r = self.rect;
        let l = self.model.lang();
        let st = th.seg_text();
        let mut x = r.x + 2.0;
        let mut cache = [Rect::default(); 3];
        for (i, s) in Sub::ALL.iter().enumerate() {
            let w = cv.text_width(s.title(l), st).ceil() + 28.0;
            cache[i] = Rect::new(x - r.x, r.y + 3.0, w, 28.0);
            x += w + 4.0;
        }
        self.note_segments(cache);
        let ind = Rect::new(r.x + self.seg_x.at(), r.y + 3.0, self.seg_w.at().max(0.0), 28.0);
        cv.fill_rr(ind, 14.0, th.raised2);
        let ind_c = ind.cx();
        let unread = self.model.unread();
        for (i, s) in Sub::ALL.iter().enumerate() {
            let c = Rect::new(r.x + cache[i].x, cache[i].y, cache[i].w, 28.0);
            self.clicks.add(K::Sub(i), c);
            let on = (1.0 - (c.cx() - ind_c).abs() / (c.w * 0.9 + 20.0)).clamp(0.0, 1.0);
            let h = self.clicks.hover_amt(&K::Sub(i));
            let col = th.dim.mix(th.text, on.max(h * 0.6));
            let sc = self.clicks.press_scale(&K::Sub(i), 0.96);
            cv.with_scale_about(c.center(), sc, |cv| cv.text(c, s.title(l), st.center(), col));
            if *s == Sub::Notifs && unread > 0 && !self.link_off() {
                let t = unread.min(99).to_string();
                let bw = (cv.text_width(&t, Style::new(10.0, 700)) + 10.0).max(16.0);
                let b = Rect::new(c.right() - 4.0, c.y - 1.0, bw, 16.0);
                cv.fill_rr(b, 8.0, th.accent);
                cv.text(b, &t, Style::new(10.0, 700).center(), th.on_accent);
            }
        }
        // the link state, right aligned
        let k = self.conn_k.at();
        let link = self.model.link();
        let text = if self.link_off() {
            l.t("Связь выключена", "Link off").to_string()
        } else if let Some(p) = link.phone().filter(|p| p.connected) {
            p.name.clone()
        } else if link.paired > 0 {
            l.t("Не в сети", "Offline").to_string()
        } else {
            l.t("Нет телефонов", "No phone yet").to_string()
        };
        let sst = Style::new(12.0, 500);
        let tw = cv.text_width(&text, sst).ceil().min(r.w * 0.4);
        let tr = Rect::new(r.right() - 4.0 - tw, r.y, tw, 34.0);
        cv.text(tr, &text, sst.ellipsis(), th.dim.mix(th.text, k));
        cv.fill_circle((tr.x - 12.0, r.y + 17.0), 4.0, th.dim.mix(th.green, k));
    }

    // ------------------------------------------------------------------------------------------------------------ link off / error
    fn draw_off(&mut self, cv: &Canvas, th: &Theme, msg: &str) {
        let body = self.body_rect();
        let l = self.model.lang();
        let cx = body.cx();
        let mut y = body.y + body.h * 0.5 - 70.0;
        self.ic.draw(cv, "p-phone", Rect::new(cx - 20.0, y, 40.0, 40.0), th.dim);
        y += 50.0;
        cv.text(Rect::new(body.x, y, body.w, 20.0), msg, Style::new(14.0, 600).center(), th.text);
        y += 26.0;
        let hint = l.t(
            "Уведомления телефона появятся здесь и на острове, а файлы можно отправлять в обе стороны. Работает только по домашней сети, без облака.",
            "Your phone's notifications show up here and on the island, and files go both ways. It works over your home network only, with no cloud.",
        );
        let hw = body.w.min(440.0);
        cv.text(Rect::new(cx - hw / 2.0, y, hw, 34.0), hint, Style::new(12.0, 400).wrap().center(), th.dim);
        y += 46.0;
        let label = l.t("Связь с телефоном", "Phone link");
        let lw = cv.text_width(label, Style::new(13.0, 500)).ceil();
        let sw = th.m.sw_w + 8.0;
        let total = lw + 12.0 + sw;
        cv.text(Rect::new(cx - total / 2.0, y, lw + 2.0, 28.0), label, Style::new(13.0, 500), th.text);
        self.switch_rect = Rect::new(cx - total / 2.0 + lw + 12.0, y, sw, 28.0);
        self.switch.draw(cv, th, self.switch_rect);
    }

    // ------------------------------------------------------------------------------------------------------------ notifications
    fn draw_notifs(&mut self, cv: &Canvas, th: &Theme) {
        let body = self.body_rect();
        let l = self.model.lang();
        // heights from the flows
        let mut ys = Vec::new();
        let mut y = 2.0f32;
        for r in self.model.rows() {
            let a = self.rows.get(&r.key);
            let open = a.map_or(0.0, |a| a.open.at().max(0.0));
            let rep = a.map_or(0.0, |a| a.reply.at().max(0.0));
            let card = (ROW_H + REPLY_H * rep) * open;
            ys.push((y, card, open.clamp(0.0, 1.0), rep));
            y += card + ROW_GAP * open;
        }
        let content_h = y + 4.0;
        if self.model.rows().iter().all(|r| r.leaving) && ys.iter().all(|t| t.1 < 1.0) {
            cv.text(Rect::new(body.x, body.y + body.h * 0.5 - 30.0, body.w, 20.0), l.t("Пока нет уведомлений с телефона", "No notifications from your phone yet"), Style::new(13.0, 600).center(), th.text);
            cv.text(Rect::new(body.x + 40.0, body.y + body.h * 0.5 - 6.0, body.w - 80.0, 34.0), l.t("Они появятся здесь, пока телефон подключён.", "They appear here while the phone is connected."), Style::new(12.0, 400).wrap().center(), th.dim);
            return;
        }
        let mut sv = std::mem::take(&mut self.scroll[0]);
        sv.set_content_height(content_h, body.h);
        let off = sv.offset();
        {
            let PhoneView { model, clicks, ic, img, field, rows, reply_key, .. } = self;
            let reply_key = reply_key.clone();
            let mut want_focus = false;
            sv.draw(cv, body, th, |cv, _c| {
                let w = body.w - BAR;
                for (r, &(y, h, open, rep)) in model.rows().iter().zip(&ys) {
                    if h < 1.0 || y > off + body.h || y + h < off {
                        continue;
                    }
                    let card = Rect::new(0.0, y, w, h);
                    let dim = if r.ongoing || r.replay { 0.62 } else { 1.0 };
                    cv.with_clip_rr(card, 14.0, |cv| {
                        cv.with_opacity(open * dim, |cv| {
                            cv.fill_rr(Rect::new(0.0, y, w, ROW_H + REPLY_H), 14.0, th.card);
                            // avatar: the sender's picture, else the app icon, else a letter
                            let av = Rect::new(12.0, y + (ROW_H - 38.0) / 2.0, 38.0, 38.0);
                            let shown = img.circle(cv, &format!("{}\0av", r.key), r.avatar.as_deref(), av) || img.circle(cv, &format!("{}\0ic", r.pkg), r.icon.as_deref(), av);
                            if !shown {
                                cv.fill_circle(av.center(), 19.0, th.raised2);
                                cv.text(av, &letter(if r.app.is_empty() { &r.title } else { &r.app }), Style::new(15.0, 600).center(), th.text2);
                            }
                            // buttons on the right
                            let mut bx = w - 12.0;
                            let by = y + (ROW_H - BTN_H) / 2.0;
                            let mut specs: Vec<(K, &str, &str)> = vec![(K::Hide(r.key.clone()), l.t("Скрыть", "Hide"), "p-x"), (K::Read(r.key.clone()), l.t("Прочитано", "Mark read"), "p-check")];
                            if r.can_reply && !r.ongoing {
                                specs.push((K::Reply(r.key.clone()), l.t("Ответить", "Reply"), "p-reply"));
                            }
                            let mut left = w - 12.0;
                            for (k, label, icon) in specs {
                                let bw = button_w(cv, th, label, true, Bk::Normal);
                                bx -= bw;
                                let br = Rect::new(bx, by, bw, BTN_H);
                                let active = matches!(&k, K::Reply(_)) && reply_key.as_deref() == Some(r.key.as_str());
                                button(cv, th, ic, br, label, Some(icon), if active { Bk::Primary } else { Bk::Normal }, clicks.hover_amt(&k), clicks.press_scale(&k, 0.96), true);
                                reg(clicks, k, br, body, off);
                                left = bx;
                                bx -= 6.0;
                            }
                            let tx = 62.0;
                            let tw = (left - 12.0 - tx).max(40.0);
                            let head = if r.app.is_empty() { ago(0, l) } else { format!("{} · {}", r.app, model.time_label(r.when)) };
                            cv.text(Rect::new(tx, y + 11.0, tw, 14.0), &head, Style::new(11.0, 500).ellipsis(), th.dim);
                            cv.text(Rect::new(tx, y + 26.0, tw, 18.0), &r.title, Style::new(13.0, 600).ellipsis(), th.text);
                            if let ReplyState::Failed(e) = &r.reply {
                                let msg = format!("{}: {}", l.t("Не отправлено", "Not sent"), e);
                                cv.text(Rect::new(tx, y + 45.0, tw, 16.0), &msg, Style::new(12.0, 500).ellipsis(), th.red);
                            } else {
                                let body_r = Rect::new(tx, y + 45.0, tw, 32.0);
                                cv.with_clip(body_r, |cv| cv.text(body_r, &r.text, Style::new(12.0, 400).wrap(), th.dim));
                            }
                            // the reply strip
                            if rep > 0.02 {
                                let sy = y + ROW_H - 2.0;
                                let send = Rect::new(w - 12.0 - 30.0, sy + 3.0, 30.0, 30.0);
                                let fr = Rect::new(tx, sy + 3.0, send.x - 8.0 - tx, 30.0);
                                cv.with_opacity(rep.clamp(0.0, 1.0), |cv| {
                                    let sent = rows.get(&r.key).map_or(0.0, |a| a.sent.raw());
                                    if r.reply == ReplyState::Sent {
                                        // "Sent": a check that pops, then the row leaves
                                        let c = Rect::new(tx, sy + 3.0, 30.0, 30.0);
                                        cv.fill_circle(c.center(), 15.0 * sent.clamp(0.0, 1.2), th.accent_soft);
                                        cv.with_scale_about(c.center(), sent.clamp(0.0, 1.25), |cv| ic.draw_sw(cv, "p-check", c.centered(16.0, 16.0), th.accent, 2.0));
                                        cv.text(Rect::new(tx + 40.0, sy + 3.0, 200.0, 30.0), l.t("Отправлено", "Sent"), Style::new(13.0, 600), th.accent);
                                    } else {
                                        // the field lives in window coordinates for input and in content coordinates for painting
                                        let win = Rect::new(fr.x + body.x, fr.y + body.y - off, fr.w, fr.h);
                                        if reply_key.as_deref() == Some(r.key.as_str()) {
                                            field.ti.placeholder = l.t("Ответ…", "Reply…").to_string();
                                            want_focus |= field.pump(cv, win);
                                            field.ti.draw(cv, fr, th);
                                        }
                                        let k = K::Send(r.key.clone());
                                        let busy = r.reply == ReplyState::Sending;
                                        let hv = clicks.hover_amt(&k);
                                        let ps = clicks.press_scale(&k, 0.92);
                                        cv.with_scale_about(send.center(), ps, |cv| {
                                            cv.fill_circle(send.center(), 15.0, th.accent.mix(th.accent_hover, hv).fade(if busy { 0.45 } else { 1.0 }));
                                            ic.draw_sw(cv, "p-send", send.centered(16.0, 16.0), th.on_accent, 2.0);
                                        });
                                        if !busy {
                                            reg(clicks, k, send, body, off);
                                        }
                                    }
                                });
                            }
                        });
                    });
                }
            });
            if want_focus {
                field.ti.set_focused(true);
            }
        }
        self.scroll[0] = sv;
    }

    // ------------------------------------------------------------------------------------------------------------ files
    fn draw_files(&mut self, cv: &Canvas, th: &Theme) {
        let body = self.body_rect();
        let l = self.model.lang();
        let zone_k = self.drop_k.at();
        let zone_pop = self.drop_k.raw();
        let err_k = self.drop_err_k.at();
        let err = self.drop_err.clone().unwrap_or_default();
        let mut ys = Vec::new();
        let mut y = ZONE_H + 8.0;
        for f in self.model.files() {
            let open = self.files.get(&f.id).map_or(0.0, |a| a.open.at().max(0.0));
            ys.push((y, FILE_H * open, open.clamp(0.0, 1.0)));
            y += (FILE_H + ROW_GAP) * open;
        }
        let content_h = y + 4.0;
        let mut sv = std::mem::take(&mut self.scroll[1]);
        sv.set_content_height(content_h, body.h);
        let off = sv.offset();
        {
            let PhoneView { model, clicks, ic, files, .. } = self;
            sv.draw(cv, body, th, |cv, _c| {
                let w = body.w - BAR;
                // the drop zone
                let z = Rect::new(0.0, 2.0, w, ZONE_H - 2.0);
                cv.with_scale_about(z.center(), 1.0 + 0.012 * zone_pop, |cv| {
                    cv.fill_rr(z, 16.0, th.card.mix(th.accent_soft, zone_k));
                    cv.stroke_rr(z.inset(0.75), 16.0, 1.5, th.line.mix(th.accent, zone_k));
                    ic.draw_sw(cv, "p-upload", Rect::new(22.0, z.cy() - 12.0, 24.0, 24.0), th.dim.mix(th.accent, zone_k), 1.8);
                    let t = l.t("Перетащи файлы на остров: они уйдут на телефон", "Drop files on the island: they go to the phone");
                    cv.text(Rect::new(62.0, z.y + 14.0, w - 80.0, 20.0), t, Style::new(13.0, 600).ellipsis(), th.text);
                    let sub = if !err.is_empty() { err.as_str() } else { l.t("Принятые файлы лежат в папке загрузок Pill", "Files from the phone land in the Pill downloads folder") };
                    let col = if err.is_empty() { th.dim } else { th.dim.mix(th.red, err_k) };
                    cv.text(Rect::new(62.0, z.y + 36.0, w - 80.0, 18.0), sub, Style::new(12.0, 400).ellipsis(), col);
                });
                if model.files().is_empty() {
                    cv.text(Rect::new(0.0, ZONE_H + 40.0, w, 20.0), l.t("Файлов пока нет", "No files yet"), Style::new(12.0, 400).center(), th.dim);
                }
                for (f, &(y, h, open)) in model.files().iter().zip(&ys) {
                    if h < 1.0 || y > off + body.h || y + h < off {
                        continue;
                    }
                    let card = Rect::new(0.0, y, w, h);
                    cv.with_clip_rr(card, 14.0, |cv| {
                        cv.with_opacity(open, |cv| {
                            cv.fill_rr(Rect::new(0.0, y, w, FILE_H), 14.0, th.card);
                            let ic_r = Rect::new(12.0, y + 11.0, 32.0, 32.0);
                            cv.fill_circle(ic_r.center(), 16.0, if f.incoming { th.accent_soft } else { th.raised2 });
                            ic.draw_sw(cv, if f.incoming { "p-down" } else { "p-up" }, ic_r.centered(16.0, 16.0), if f.incoming { th.accent } else { th.text2 }, 1.9);
                            // buttons
                            let mut specs: Vec<(K, &str, Bk)> = Vec::new();
                            if f.pending() {
                                specs.push((K::Decline(f.id.clone()), l.t("Отклонить", "Decline"), Bk::Normal));
                                specs.push((K::Accept(f.id.clone()), l.t("Принять", "Accept"), Bk::Primary));
                            } else if f.has_file() {
                                specs.push((K::Folder(f.id.clone()), l.t("В папке", "Show in folder"), Bk::Normal));
                                specs.push((K::Open(f.id.clone()), l.t("Открыть", "Open"), Bk::Normal));
                            }
                            let mut bx = w - 12.0;
                            let mut left = bx;
                            for (k, label, kind) in specs {
                                let bw = button_w(cv, th, label, false, kind);
                                bx -= bw;
                                let br = Rect::new(bx, y + (FILE_H - BTN_H) / 2.0, bw, BTN_H);
                                button(cv, th, ic, br, label, None, kind, clicks.hover_amt(&k), clicks.press_scale(&k, 0.96), true);
                                reg(clicks, k, br, body, off);
                                left = bx;
                                bx -= 6.0;
                            }
                            let tx = 56.0;
                            let tw = (left - 12.0 - tx).max(40.0);
                            cv.text(Rect::new(tx, y + 9.0, tw, 18.0), &f.name, Style::new(13.0, 500).ellipsis(), th.text);
                            let status = match (f.status.as_str(), f.incoming) {
                                ("pending", _) => l.t("Ждёт твоего решения", "Waiting for you"),
                                ("accepted", true) => l.t("Принято, ждём файл", "Accepted, waiting for the file"),
                                ("accepted", false) => l.t("Ждёт телефон", "Waiting for the phone"),
                                ("active", _) => l.t("Передаётся", "Transferring"),
                                ("done", true) => l.t("Получено", "Received"),
                                ("done", false) => l.t("Отправлено", "Sent"),
                                ("rejected", _) => l.t("Отклонено", "Declined"),
                                _ => l.t("Не удалось", "Failed"),
                            };
                            let line = format!("{} · {}", size_text(f.size, l), status);
                            cv.text(Rect::new(tx, y + 29.0, tw, 16.0), &line, Style::new(11.0, 400).ellipsis(), if f.status == "failed" { th.red } else { th.dim });
                            if let Some(a) = files.get(&f.id) {
                                if f.status == "active" {
                                    let bar = Rect::new(tx, y + FILE_H - 7.0, tw, 3.0);
                                    cv.fill_rr(bar, 1.5, th.track);
                                    cv.fill_rr(Rect::new(bar.x, bar.y, (bar.w * a.progress.at01()).max(3.0), 3.0), 1.5, th.accent);
                                }
                            }
                        });
                    });
                }
            });
        }
        self.scroll[1] = sv;
    }

    // ------------------------------------------------------------------------------------------------------------ phone
    fn draw_phone(&mut self, cv: &Canvas, th: &Theme) {
        let body = self.body_rect();
        let l = self.model.lang();
        let left = Rect::new(body.x, body.y, LEFT_W, body.h);
        let right = Rect::new(left.right() + 8.0, body.y, body.w - LEFT_W - 8.0, body.h);
        self.draw_phone_left(cv, th, left);
        cv.fill_rr(right, 16.0, th.card);
        let link = self.model.link().clone();
        // the QR card
        let cap = if link.paired > 0 { l.t("Подключить ещё телефон", "Pair another phone") } else { l.t("Подключить телефон", "Pair your phone") };
        cv.text(Rect::new(right.x, right.y + 14.0, right.w, 18.0), cap, Style::new(13.0, 600).center(), th.text);
        let side = 150.0;
        let qr = Rect::new(right.cx() - side / 2.0, right.y + 40.0, side, side);
        let pop = self.qr_k.raw();
        let url = self.model.code().map(|c| c.url.clone());
        let code = self.model.code().map(|c| c.code.clone());
        if let Some(url) = &url {
            if self.qr_cache.as_ref().map(|c| c.0.as_str()) != Some(url.as_str()) {
                let rows = pill_ui_core::panel::b::qr::qr(url).map(|q| q.rows()).unwrap_or_default();
                self.qr_cache = Some((url.clone(), rows));
            }
        }
        let a = self.qr_k.at();
        cv.with_opacity(a.max(0.15), |cv| {
            cv.with_scale_about(qr.center(), 0.9 + 0.1 * pop.clamp(0.0, 1.2), |cv| {
                cv.with_clip_rr(qr, 12.0, |cv| {
                    cv.fill_rect(qr, if url.is_some() { Color::rgb8(255, 255, 255) } else { th.raised2 });
                    if let Some((_, rows)) = self.qr_cache.as_ref().filter(|_| url.is_some()) {
                        let quiet = 4usize;
                        let px = side / (rows.len() + 2 * quiet) as f32;
                        let sc = cv.scale;
                        let snap = |v: f32| (v * sc).round() / sc;
                        for (cx, cy, run) in qr_runs(rows) {
                            let x0 = qr.x + (cx + quiet) as f32 * px;
                            let y0 = qr.y + (cy + quiet) as f32 * px;
                            let (sx, sy) = (snap(x0), snap(y0));
                            cv.fill_rect(Rect::new(sx, sy, snap(x0 + run as f32 * px) - sx, snap(y0 + px) - sy), Color::rgb8(0, 0, 0));
                        }
                    }
                });
            });
        });
        if let Some(code) = code {
            let spaced = format!("{} {}", &code[..4.min(code.len())], &code[4.min(code.len())..]);
            cv.text(Rect::new(right.x, qr.bottom() + 8.0, right.w, 18.0), &spaced, Style::new(13.0, 600).mono().center(), th.text2);
        }
        let label = l.t("Новый код", "New code");
        let bw = button_w(cv, th, label, false, Bk::Normal);
        let br = Rect::new(right.cx() - bw / 2.0, right.bottom() - 14.0 - BTN_H, bw, BTN_H);
        button(cv, th, &self.ic, br, label, None, Bk::Normal, self.clicks.hover_amt(&K::NewCode), self.clicks.press_scale(&K::NewCode, 0.96), true);
        self.clicks.add(K::NewCode, br);
    }
}
