//! Share your look (Look page, row `look.share`): export the appearance as a `.pilllook` file or a `pill-look:` text, import one from a file, the
//! clipboard or the four built-in looks, SEE it in the live preview first, then Apply or Cancel, and an animated toast with Undo afterwards.
//! The rules and the schema are `pill_ui_core::settings::look` (docs/native-look-share.md); this file is the window half.
//!
//! * pending look: validated into a `Plan`, never written; the preview island is fed `model values + plan` (`LookState::overlay`) until Apply / Cancel
//! * Apply: `look::apply` through the model (one `settings_set`), the plan's previous values become the `Undo` of the toast
//! * motion: the banner opens / closes (height + fade), the toast rises and sinks, the chips pop; Reduce motion lands everything at once
//! * idle cost: nothing runs without a look in play; the toast's expiry is one sleeping thread per Apply
use super::flow::{go, tick};
use super::pal::mix;
use super::page::RowRt;
use super::rows::*;
use super::ui::{key, Act};
use super::{Local, SettingsView};
use crate::ui::liquid::{Feel, Flow};
use crate::ui::view::Cx;
use glasskit::controls::ButtonKind;
use glasskit::geom::Rect;
use glasskit::text::Style;
use pill_ui_core::settings::calls::BackendCall as C;
use pill_ui_core::settings::flows::{self, Msg as FMsg};
use pill_ui_core::settings::look::{self, LookError, Plan, Undo, SAMPLES};
use pill_ui_core::settings::tr::{pick, Lang};
use serde_json::{Map, Value};
use std::borrow::Cow;

/// How long the Undo toast stays.
const TOAST_MS: f64 = 12_000.0;
const BANNER_H: f32 = 58.0;

#[derive(Clone, Debug, PartialEq)]
pub enum LookAct {
    ExportFile,
    CopyText,
    ImportFile,
    Paste,
    Sample(usize),
    Apply,
    Cancel,
    Undo,
}

pub struct LookState {
    /// the look under preview (validated, not applied)
    plan: Option<Plan>,
    /// which built-in look it came from (its chip is lit)
    sample: Option<usize>,
    /// the banner's text while it is open or closing: (name, changes)
    info: Option<(String, usize)>,
    banner: Flow,
    /// the keys the preview was fed from the plan: they are put back when it ends
    shown: Vec<&'static str>,
    revert: Option<Map<String, Value>>,
    undo: Option<Undo>,
    toast: Flow,
    toast_at: f64,
    expiring: bool,
}

impl Default for LookState {
    fn default() -> Self {
        LookState { plan: None, sample: None, info: None, banner: Flow::new(0.0, Feel::FLOW), shown: Vec::new(), revert: None, undo: None, toast: Flow::new(0.0, Feel::FLOW), toast_at: 0.0, expiring: false }
    }
}

impl LookState {
    /// What the preview island is told: the window's values, with the pending look laid over them (or, once, what the look replaced).
    pub fn overlay<'a>(&mut self, base: &'a Map<String, Value>) -> Cow<'a, Map<String, Value>> {
        if let Some(p) = &self.plan {
            let mut m = base.clone();
            m.extend(p.values());
            self.shown = p.set.iter().map(|(k, _)| *k).collect();
            return Cow::Owned(m);
        }
        match self.revert.take() {
            Some(back) => {
                let mut m = base.clone();
                m.extend(back);
                Cow::Owned(m)
            }
            None => Cow::Borrowed(base),
        }
    }
}

impl SettingsView {
    pub(super) fn step_look(&mut self, dt: f32) -> bool {
        let (reduce, now) = (self.reduce(), self.now_ms);
        let l = &mut self.data.look;
        let mut busy = tick(&mut l.banner, dt);
        busy |= tick(&mut l.toast, dt);
        if l.undo.is_some() && !l.expiring && now - l.toast_at > TOAST_MS {
            l.expiring = true;
            go(&mut l.toast, 0.0, reduce);
            busy = true;
        }
        if l.expiring && !l.toast.moving() && l.toast.at01() < 0.01 {
            l.undo = None;
            l.expiring = false;
        }
        if l.plan.is_none() && !l.banner.moving() && l.banner.at01() < 0.01 {
            l.info = None;
        }
        busy
    }

    // ------------------------------------------------------------------------------------------------ the row

    pub(super) fn w_look(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (cv, p) = (d.cv, d.p);
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
        }
        let mut y = full.y + m.nh - 11.0 + 6.0;
        if paint {
            cv.text(Rect::new(x0, y, w, 16.0), pick(d.lang, "Готовые оформления", "Built-in looks"), st_hint(), p.dim);
        }
        y += 20.0;
        let lit = self.data.look.sample;
        let mut x = x0;
        for (i, s) in SAMPLES.iter().enumerate() {
            let cw = self.chip(d, paint, x, y, &format!("look:{i}"), s.name, &[], lit == Some(i), Act::Look(LookAct::Sample(i)));
            x += cw + 8.0;
        }
        y += 30.0 + 14.0;
        if paint {
            cv.text(Rect::new(x0, y, w, 16.0), pick(d.lang, "Файл и текст", "File and text"), st_hint(), p.dim);
        }
        y += 20.0;
        let buttons = [
            ("look:export", pick(d.lang, "Сохранить файл", "Save file"), LookAct::ExportFile),
            ("look:copy", pick(d.lang, "Копировать как текст", "Copy as text"), LookAct::CopyText),
            ("look:import", pick(d.lang, "Открыть файл", "Open file"), LookAct::ImportFile),
            ("look:paste", pick(d.lang, "Вставить из буфера", "Paste from clipboard"), LookAct::Paste),
        ];
        let mut x = x0;
        for (id, label, act) in buttons {
            let probe = self.btn_at(d, false, id, label, ButtonKind::Normal, x, None, y, Act::None, true);
            if x > x0 && probe.right() > x0 + w {
                x = x0;
                y += 32.0 + 8.0;
            }
            let rc = self.btn_at(d, paint, id, label, ButtonKind::Normal, x, None, y, Act::Look(act), true);
            x = rc.right() + 8.0;
        }
        y += 32.0;
        // the pending look: opens under the buttons
        let k = self.data.look.banner.at01();
        let bh = BANNER_H * k;
        if let (true, Some((name, changes))) = (bh > 0.5, self.data.look.info.clone()) {
            let rc = Rect::new(x0, y + 10.0 * k, w, bh);
            if paint {
                let on = self.data.look.plan.is_some();
                let shown_in_preview = self.preview_wanted();
                cv.with_clip(rc, |cv| {
                    cv.with_opacity(k, |cv| {
                        cv.fill_rr(rc, 10.0, p.accent_bg);
                        cv.stroke_rr(rc.inset(0.5), 9.5, 1.0, p.accent_line);
                        let tw = w - 24.0 - 190.0;
                        cv.text(Rect::new(rc.x + 14.0, rc.y + 10.0, tw, 18.0), &format!("«{name}»"), st_title().ellipsis(), p.text);
                        let sub = pending_line(d.lang, changes, shown_in_preview);
                        cv.text(Rect::new(rc.x + 14.0, rc.y + 30.0, tw, 16.0), &sub, st_hint().ellipsis(), p.dim);
                        let cancel = self.btn_at(d, true, "look:cancel", pick(d.lang, "Отмена", "Cancel"), ButtonKind::Normal, 0.0, Some(rc.right() - 12.0), rc.y + (BANNER_H - 32.0) / 2.0, Act::Look(LookAct::Cancel), on);
                        self.btn_at(d, true, "look:apply", pick(d.lang, "Применить", "Apply"), ButtonKind::Primary, 0.0, Some(cancel.x - 8.0), rc.y + (BANNER_H - 32.0) / 2.0, Act::Look(LookAct::Apply), on);
                    });
                });
            }
            y += 10.0 * k + bh;
        }
        y + 14.0 - full.y
    }

    // ------------------------------------------------------------------------------------------------ the toast

    /// The Undo toast, floating at the bottom of the page area (window space, above everything of the page).
    pub(super) fn paint_look_toast(&mut self, d: &Dc, content: Rect) {
        let t = self.data.look.toast.at01();
        let Some(name) = self.data.look.undo.as_ref().map(|u| u.name.clone()).filter(|_| t > 0.01) else { return };
        let (cv, p) = (d.cv, d.p);
        let w = 400.0f32.min(content.w - 32.0).max(200.0);
        let rise = (1.0 - self.data.look.toast.at()) * 20.0;
        let rc = Rect::new(content.x + (content.w - w) / 2.0, content.bottom() - 44.0 - 18.0 + rise, w, 44.0);
        let label = pick(d.lang, "Отменить", "Undo");
        let st = Style::new(13.0, 600);
        let bw = cv.text_width(label, st) + 24.0;
        let brc = Rect::new(rc.right() - 8.0 - bw, rc.y + 7.0, bw, 30.0);
        let k = key("look:undo");
        self.ui.it(k);
        let enabled = !self.data.look.expiring && t > 0.5;
        self.ui.add(k, brc, Act::Look(LookAct::Undo), false, false, enabled);
        let hv = self.ui.hover_of(k);
        let pr = self.ui.press_of(k);
        cv.with_opacity(t, |cv| {
            cv.fill_rr(rc.offset(0.0, 4.0), 14.0, p.shadow.alpha(0.35));
            cv.fill_rr(rc, 12.0, mix(p.card, p.text, 0.07));
            cv.stroke_rr(rc.inset(0.5), 11.5, 1.0, p.card_line_hover);
            let msg = applied_line(d.lang, &name);
            cv.text(Rect::new(rc.x + 16.0, rc.y, rc.w - bw - 36.0, rc.h), &msg, Style::new(13.0, 400).ellipsis(), p.text);
            cv.with_scale_about(brc.center(), 1.0 - 0.06 * pr.max(0.0), |cv| {
                cv.fill_rr(brc, 8.0, p.accent_bg.fade(0.55 + 0.45 * hv));
                cv.text(brc, label, st.center(), p.text);
            });
        });
    }

    // ------------------------------------------------------------------------------------------------ actions

    pub(super) fn look_act(&mut self, a: LookAct, cx: &mut Cx) {
        let lang = self.lang();
        match a {
            LookAct::ExportFile => {
                let l = look::export(&self.model, pick(lang, "Моё оформление", "My look"));
                self.call(cx, C::LookExport { file_name: look::file_name(&l), text: look::to_json(&l) }, "look.share");
            }
            LookAct::CopyText => {
                let l = look::export(&self.model, pick(lang, "Моё оформление", "My look"));
                let t = look::to_text(&l);
                cx.set_clipboard_text(&t);
                self.row_message("look.share", FMsg::ok(copied_line(lang, t.chars().count())));
            }
            LookAct::ImportFile => {
                let owner = crate::ui::host::handle().and_then(|h| h.hwnd(cx.window)).unwrap_or(0);
                let w = cx.waker();
                std::thread::spawn(move || w.post_custom(Local::LookPicked(pick_look_file(owner))));
            }
            LookAct::Paste => match cx.clipboard_text() {
                Some(t) if !t.trim().is_empty() => self.look_stage(&t, None),
                _ => self.row_message("look.share", FMsg::bad(LookError::Empty.message(lang))),
            },
            LookAct::Sample(i) => {
                if let Some(s) = SAMPLES.get(i) {
                    self.look_stage(s.json, Some(i));
                }
            }
            LookAct::Cancel => self.look_end(),
            LookAct::Apply => self.look_apply(cx),
            LookAct::Undo => self.look_undo(cx),
        }
    }

    /// The file chosen by "Open file" (or why there is none).
    pub(super) fn look_picked(&mut self, r: Result<String, String>) {
        let lang = self.lang();
        match r {
            Ok(text) => self.look_stage(&text, None),
            Err(e) if e == "cancelled" => {}
            Err(e) if e == "big" => self.row_message("look.share", FMsg::bad(LookError::TooBig.message(lang))),
            Err(_) => self.row_message("look.share", flows::read_error(lang)),
        }
    }

    /// Text (a file, a paste, a built-in look) -> validated -> pending, shown in the preview. Nothing is written.
    fn look_stage(&mut self, text: &str, sample: Option<usize>) {
        let lang = self.lang();
        let reduce = self.reduce();
        match look::parse(text).and_then(|l| look::plan(&l, &self.model)) {
            Ok(plan) if plan.changes == 0 => {
                self.look_end();
                self.row_message("look.share", FMsg::plain(pick(lang, "Эти настройки уже стоят: ничего не изменится", "These settings are already in place: nothing would change")));
            }
            Ok(plan) => {
                self.row_message("look.share", FMsg::plain(""));
                let name = if plan.name.is_empty() { pick(lang, "Без названия", "Untitled").to_string() } else { plan.name.clone() };
                let l = &mut self.data.look;
                l.info = Some((name, plan.changes));
                l.plan = Some(plan);
                l.sample = sample;
                go(&mut l.banner, 1.0, reduce);
            }
            Err(e) => {
                self.look_end();
                self.row_message("look.share", FMsg::bad(e.message(lang)));
            }
        }
    }

    /// Cancel: the preview goes back to the window's values.
    fn look_end(&mut self) {
        let reduce = self.reduce();
        let l = &mut self.data.look;
        if l.plan.take().is_some() {
            let keys = std::mem::take(&mut l.shown);
            l.revert = Some(keys.iter().map(|k| (k.to_string(), self.model.get(k))).collect());
        }
        let l = &mut self.data.look;
        l.sample = None;
        go(&mut l.banner, 0.0, reduce);
    }

    fn look_apply(&mut self, cx: &mut Cx) {
        let lang = self.lang();
        let Some(plan) = self.data.look.plan.clone() else { return };
        let reduce = self.reduce();
        let (calls, undo) = look::apply(&mut self.model, &plan);
        self.run(cx, calls);
        self.sync_page();
        let l = &mut self.data.look;
        l.plan = None;
        l.shown.clear();
        l.sample = None;
        go(&mut l.banner, 0.0, reduce);
        l.undo = Some(undo);
        l.expiring = false;
        l.toast_at = cx.now_ms;
        go(&mut l.toast, 1.0, reduce);
        let w = cx.waker();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(TOAST_MS as u64 + 250));
            w.post_custom(Local::Pv);
        });
        let _ = lang;
        self.row_message("look.share", FMsg::plain(""));
    }

    fn look_undo(&mut self, cx: &mut Cx) {
        let lang = self.lang();
        let reduce = self.reduce();
        let Some(u) = self.data.look.undo.take() else { return };
        let calls = look::undo(&mut self.model, &u);
        self.run(cx, calls);
        self.sync_page();
        let l = &mut self.data.look;
        l.expiring = false;
        go(&mut l.toast, 0.0, reduce);
        self.row_message("look.share", FMsg::ok(restored_line(lang, &u.name)));
    }

    /// The answer of `look_export`.
    pub(super) fn look_export_reply(&mut self, v: &Value) {
        let lang = self.lang();
        let m = if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
            let file = v.get("path").and_then(Value::as_str).and_then(|p| p.rsplit(['\\', '/']).next()).unwrap_or("");
            FMsg::ok(format!("{}{file}", pick(lang, "Сохранено в «Загрузки»: ", "Saved to Downloads: ")))
        } else {
            FMsg::bad(format!("{}{}", pick(lang, "Не удалось сохранить: ", "Could not save: "), v.get("error").and_then(Value::as_str).unwrap_or("?")))
        };
        self.row_message("look.share", m);
    }
}

// ------------------------------------------------------------------------------------------------ texts

fn pending_line(lang: Lang, changes: usize, in_preview: bool) -> String {
    let n = match lang {
        Lang::Ru => format!("Изменится настроек: {changes}"),
        Lang::En => format!("{changes} setting{} would change", if changes == 1 { "" } else { "s" }),
    };
    let tail = if in_preview { pick(lang, " · смотрите в предпросмотре", " · see the preview") } else { "" };
    format!("{n}{tail}")
}

fn applied_line(lang: Lang, name: &str) -> String {
    match lang {
        Lang::Ru => format!("Оформление «{name}» применено"),
        Lang::En => format!("Look “{name}” applied"),
    }
}

fn restored_line(lang: Lang, name: &str) -> String {
    match lang {
        Lang::Ru => format!("Вернул прежний вид (было «{name}»)"),
        Lang::En => format!("Back to the previous look (“{name}” undone)"),
    }
}

fn copied_line(lang: Lang, chars: usize) -> String {
    match lang {
        Lang::Ru => format!("Скопировано ({chars} зн.): вставьте в чат, там это одна строка"),
        Lang::En => format!("Copied ({chars} characters): paste it in a chat, it is one line"),
    }
}

// ------------------------------------------------------------------------------------------------ the file dialog

/// The system "open" dialog for a `.pilllook` (or `.json`) file; runs on a worker. Err: "cancelled", "big" or the reason.
fn pick_look_file(owner: isize) -> Result<String, String> {
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let spec = [COMDLG_FILTERSPEC { pszName: w!("Pill look"), pszSpec: w!("*.pilllook;*.json") }];
        dlg.SetFileTypes(&spec).map_err(|e| e.to_string())?;
        dlg.Show(HWND(owner as *mut _)).map_err(|_| "cancelled".to_string())?;
        let item = dlg.GetResult().map_err(|e| e.to_string())?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| e.to_string())?;
        let path = name.to_string().map_err(|e| e.to_string())?;
        let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if meta.len() as usize > look::MAX_FILE_BYTES {
            return Err("big".into());
        }
        std::fs::read_to_string(&path).map_err(|e| e.to_string())
    }
}
