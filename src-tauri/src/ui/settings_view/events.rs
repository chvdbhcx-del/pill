//! Input routing: recorder keys, shortcuts, popups, mouse (grab, hits, controls, scrollbars, wheel), keyboard focus.
use super::page::*;
use super::paint::{parts, TITLE_H};
use super::ui::Act;
use super::{Grab, SettingsView};
use crate::ui::view::{Cx, CursorShape};
use glasskit::controls::Widget;
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, KeyCode, Modifiers, MouseButton, Response};
use pill_ui_core::settings::hotkey::KeyEvent;
use serde_json::{json, Value};

/// A value a control produced: the row and the new value.
pub struct Change {
    pub row: &'static pill_ui_core::settings::schema::Row,
    pub value: Value,
    pub backend: Option<pill_ui_core::settings::schema::BackendToggle>,
}

fn dom_code(code: KeyCode) -> (String, String) {
    match code {
        KeyCode::Char(c) if c.is_ascii_alphabetic() => (String::new(), format!("Key{}", c.to_ascii_uppercase())),
        KeyCode::Char(c) if c.is_ascii_digit() => (String::new(), format!("Digit{c}")),
        KeyCode::F(n) => (String::new(), format!("F{n}")),
        KeyCode::Space => (" ".into(), "Space".into()),
        KeyCode::Enter => ("Enter".into(), "Enter".into()),
        KeyCode::Tab => ("Tab".into(), "Tab".into()),
        KeyCode::Escape => ("Escape".into(), "Escape".into()),
        KeyCode::Backspace => ("Backspace".into(), "Backspace".into()),
        KeyCode::Delete => ("Delete".into(), "Delete".into()),
        KeyCode::Insert => (String::new(), "Insert".into()),
        KeyCode::Up => (String::new(), "ArrowUp".into()),
        KeyCode::Down => (String::new(), "ArrowDown".into()),
        KeyCode::Left => (String::new(), "ArrowLeft".into()),
        KeyCode::Right => (String::new(), "ArrowRight".into()),
        KeyCode::Home => (String::new(), "Home".into()),
        KeyCode::End => (String::new(), "End".into()),
        KeyCode::PageUp => (String::new(), "PageUp".into()),
        KeyCode::PageDown => (String::new(), "PageDown".into()),
        _ => (String::new(), String::new()),
    }
}

pub fn key_event(code: KeyCode, m: Modifiers) -> KeyEvent {
    let (key, code) = dom_code(code);
    KeyEvent { key, code, ctrl: m.ctrl, alt: m.alt, shift: m.shift, meta: m.meta }
}

impl SettingsView {
    pub(super) fn route_event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let mut resp = Response::NONE;
        match ev {
            InputEvent::Focus(false) => {
                if self.model.recorder.cancel() {
                    self.rec_changed();
                }
                return Response::redraw();
            }
            InputEvent::Key { code, mods, down: true, .. } if self.model.recorder.active().is_some() => {
                self.recorder_key(*code, *mods, cx);
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
            InputEvent::Key { code: KeyCode::Char('F'), mods, down: true, .. } if mods.ctrl && !mods.shift && !mods.alt => {
                self.focus_search();
                return Response { consumed: true, redraw: true, ..Response::NONE };
            }
            _ => {}
        }
        if let Some(r) = self.select_open_event(ev) {
            return r;
        }
        match ev {
            InputEvent::MouseMove { .. } | InputEvent::MouseDown { .. } | InputEvent::MouseUp { .. } | InputEvent::MouseLeave | InputEvent::Wheel { .. } => resp = self.mouse_event(ev, cx),
            InputEvent::Key { .. } | InputEvent::Char(_) => resp = self.key_event_focus(ev, cx),
            _ => {}
        }
        resp
    }

    fn select_open_event(&mut self, ev: &InputEvent) -> Option<Response> {
        let origin = self.ui.page_origin;
        let local = ev.translated(-origin.0, -origin.1);
        for r in self.page.rows_mut() {
            if let Ctl::Select(s) = &mut r.ctl {
                if s.popup_visible() {
                    let resp = s.event(&local, r.ctl_rect);
                    let ch = if resp.changed { s.selected().and_then(|i| r.opts.get(i).map(|o| o.1.clone())) } else { None };
                    let row = r.row;
                    if let Some(v) = ch {
                        self.pending_changes.push(Change { row, value: v, backend: None });
                    }
                    return Some(Response { consumed: true, redraw: true, ..resp });
                }
            }
        }
        None
    }

    pub(super) fn rec_changed(&mut self) {
        self.rec_live.clear();
        let on = self.model.recorder.active().is_some();
        let to = if on { 1.0 } else { 0.0 };
        if self.reduce() {
            self.rec_t = glasskit::ease::Tween::at_rest(to);
        } else {
            self.rec_t.retarget(to, 0.12, glasskit::ease::curves::EASE_OUT);
        }
    }

    pub(super) fn focus_search(&mut self) {
        self.set_focus_id(Some(super::chrome::FID_SEARCH), true);
        self.search.field.input.select_all();
    }
}

impl SettingsView {
    pub(super) fn content_rect(&self) -> Rect {
        let wiz = self.wiz.active() || self.page.wizard;
        parts(Rect::new(0.0, 0.0, self.size.0, self.size.1), self.maximized, !wiz).content
    }

    fn mouse_event(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let content = self.content_rect();
        let mut out = Response::NONE;
        // a drag in progress keeps its target
        match self.grab.clone() {
            Some(Grab::Scrollbar) => {
                let r = self.page.bar.event(ev, content, &mut self.page.scroll);
                if matches!(ev, InputEvent::MouseUp { .. }) {
                    self.grab = None;
                }
                return Response { redraw: true, ..r };
            }
            Some(Grab::NavScroll) => {
                let list = self.side.list;
                let r = self.side.bar.event(ev, list, &mut self.side.scroll);
                if matches!(ev, InputEvent::MouseUp { .. }) {
                    self.grab = None;
                }
                return Response { redraw: true, ..r };
            }
            Some(Grab::Row(fid)) => {
                let origin = self.ui.page_origin;
                let local = ev.translated(-origin.0, -origin.1);
                let r = self.dispatch_rows(&local, Some(fid), cx);
                if matches!(ev, InputEvent::MouseUp { .. }) {
                    self.grab = None;
                }
                return Response { redraw: true, ..r };
            }
            None => {}
        }
        let pos = ev.pos();
        let in_content = pos.is_some_and(|p| content.contains(p.0, p.1));
        // blur a recorder on a click anywhere else
        if let InputEvent::MouseDown { .. } = ev {
            if self.model.recorder.active().is_some() && !self.over_recorder(pos) {
                self.model.recorder.cancel();
                self.rec_changed();
            }
        }
        if let InputEvent::Wheel { pos, delta, precise, .. } = ev {
            return self.wheel(*pos, *delta, *precise, content);
        }
        let (r, fired) = self.ui.pointer(ev);
        if let Some(a) = fired {
            self.act(a, cx);
            return Response { consumed: true, redraw: true, cursor: Some(Cursor::Hand), ..Response::NONE };
        }
        out = out.merge(r);
        if r.consumed {
            self.blur_fields(cx);
            return out;
        }
        // scrollbars first: they overlay the right edge
        let br = self.page.bar.event(ev, content, &mut self.page.scroll);
        if br.capture {
            self.grab = Some(Grab::Scrollbar);
            return br;
        }
        if br.consumed {
            return br;
        }
        out = out.merge(br);
        let sb = self.side.bar.event(ev, self.side.list, &mut self.side.scroll);
        if sb.capture {
            self.grab = Some(Grab::NavScroll);
            return sb;
        }
        let origin = self.ui.page_origin;
        let local = match ev {
            InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. } if !in_content => InputEvent::MouseLeave,
            InputEvent::MouseDown { .. } if !in_content => {
                self.blur_fields(cx);
                return out;
            }
            _ => ev.translated(-origin.0, -origin.1),
        };
        let r = self.dispatch_rows(&local, None, cx);
        if matches!(ev, InputEvent::MouseDown { .. }) && !r.consumed {
            self.blur_fields(cx);
        }
        out.merge(r)
    }

    fn over_recorder(&self, pos: Option<(f32, f32)>) -> bool {
        let Some(p) = pos else { return false };
        self.ui.topmost(p).is_some_and(|i| matches!(self.ui.hits[i].act, Act::Recorder(_)))
    }

    fn wheel(&mut self, pos: (f32, f32), delta: (f32, f32), precise: bool, content: Rect) -> Response {
        if self.side.list.contains(pos.0, pos.1) {
            self.side.scroll.wheel(delta.1, precise);
            return Response { consumed: true, redraw: true, ..Response::NONE };
        }
        if content.contains(pos.0, pos.1) {
            self.page.scroll.wheel(delta.1, precise);
            self.page.bar.show();
            return Response { consumed: true, redraw: true, ..Response::NONE };
        }
        Response::NONE
    }

    pub(super) fn blur_fields(&mut self, cx: &mut Cx) {
        let cur = self.focus.focused();
        if cur.is_some_and(|f| f != super::chrome::FID_SEARCH && f < 1000) || cur == Some(super::chrome::FID_SEARCH) {
            // a click on nothing in particular: the field loses focus and commits
            self.set_focus_id(None, false);
        }
        let _ = cx;
    }
}

impl SettingsView {
    /// Feeds one (page-space) event to the controls of the rows; `only` restricts it to the row with that focus id.
    pub(super) fn dispatch_rows(&mut self, ev: &InputEvent, only: Option<u32>, cx: &mut Cx) -> Response {
        let th = self.th;
        let mut resp = Response::NONE;
        let mut grab = None;
        let mut focus_req = None;
        let mut changes: Vec<Change> = Vec::new();
        let left_down = matches!(ev, InputEvent::MouseDown { button: MouseButton::Left, .. });
        for r in self.page.rows_mut() {
            if only.is_some_and(|f| f != r.fid) || r.vis.value() < 0.5 {
                continue;
            }
            let rect = r.ctl_rect;
            let row = r.row;
            let rr = match &mut r.ctl {
                Ctl::Toggle(t) => t.event(ev, rect),
                Ctl::Backend(t, _) => t.event(ev, rect),
                Ctl::Slider(s) => s.event_themed(ev, Rect::new(rect.x, rect.y, 140.0, 16.0), &th),
                Ctl::Seg(s) => s.event(ev, rect),
                Ctl::Select(s) => s.event(ev, rect),
                Ctl::Field(f) => {
                    let fr = f.tf.rect;
                    match ev {
                        InputEvent::MouseDown { pos, button: MouseButton::Left, .. } if fr.contains(pos.0, pos.1) => {
                            f.tf.queue(ev, fr);
                            focus_req = Some((r.fid, true));
                            Response { consumed: true, capture: true, redraw: true, cursor: Some(Cursor::Text), ..Response::NONE }
                        }
                        InputEvent::MouseMove { pos, .. } if f.tf.focused() || fr.contains(pos.0, pos.1) => {
                            if f.tf.focused() {
                                f.tf.queue(ev, fr);
                            }
                            Response { cursor: fr.contains(pos.0, pos.1).then_some(Cursor::Text), ..Response::NONE }
                        }
                        InputEvent::MouseUp { .. } if f.tf.focused() => {
                            f.tf.queue(ev, fr);
                            Response::redraw()
                        }
                        _ => Response::NONE,
                    }
                }
                _ => Response::NONE,
            };
            if rr.capture {
                grab = Some(r.fid);
            }
            if rr.focus && !matches!(r.ctl, Ctl::Field(_)) {
                focus_req = Some((r.fid, false));
            }
            if rr.changed {
                let (value, backend) = match &r.ctl {
                    Ctl::Toggle(t) => (json!(t.on()), None),
                    Ctl::Backend(t, b) => (json!(t.on()), Some(*b)),
                    Ctl::Slider(s) => (json!(s.value() as f64), None),
                    Ctl::Seg(s) => (r.opts.get(s.selected()).map(|o| o.1.clone()).unwrap_or(Value::Null), None),
                    Ctl::Select(s) => (s.selected().and_then(|i| r.opts.get(i)).map(|o| o.1.clone()).unwrap_or(Value::Null), None),
                    _ => (Value::Null, None),
                };
                if !value.is_null() {
                    changes.push(Change { row, value, backend });
                }
            }
            resp = resp.merge(rr);
            if left_down && rr.consumed {
                break;
            }
        }
        if let Some(f) = grab {
            self.grab = Some(Grab::Row(f));
        }
        if let Some((f, _field)) = focus_req {
            self.set_focus_id(Some(f), false);
        }
        self.pending_changes.extend(changes);
        self.flush_changes(cx);
        resp
    }

    pub(super) fn flush_changes(&mut self, cx: &mut Cx) {
        for c in std::mem::take(&mut self.pending_changes) {
            self.apply_change(c, cx);
        }
    }

    pub(super) fn focus_ring_for(&self, fid: u32) -> (bool, bool) {
        (self.focus.is_focused(fid), self.focus.ring_visible(fid))
    }

    /// Moves the keyboard focus (None = nobody). Text fields take / give their caret; keyboard moves scroll the target into view.
    pub(super) fn set_focus_id(&mut self, id: Option<u32>, keyboard: bool) {
        let old = self.focus.focused();
        match id {
            Some(i) => {
                self.focus.focus(i, keyboard);
            }
            None => self.focus.blur(),
        }
        self.focus_applied(old, id, keyboard);
    }

    /// The focus chain already moved from `old` to `new`: tell the text fields and scroll the target into view.
    pub(super) fn focus_applied(&mut self, old: Option<u32>, new: Option<u32>, keyboard: bool) {
        if old == new {
            return;
        }
        for r in self.page.rows_mut() {
            if let Ctl::Field(f) = &mut r.ctl {
                let on = new == Some(r.fid);
                if f.tf.focused() != on {
                    f.tf.focus(on);
                }
            }
        }
        let on = new == Some(super::chrome::FID_SEARCH);
        if self.search.field.focused() != on {
            self.search.field.focus(on);
        }
        self.edits_due = true;
        if keyboard {
            if let Some(i) = new {
                self.scroll_to_fid(i);
            }
        }
    }

    pub(super) fn scroll_to_fid(&mut self, fid: u32) {
        let mut target = None;
        for r in self.page.rows() {
            if r.fid == fid {
                target = Some(r.rect);
            }
        }
        if let Some(r) = target {
            let view = self.content_rect().h;
            let top = self.page.scroll.offset();
            if r.y < top + 8.0 {
                self.page.scroll.scroll_to((r.y - 16.0).max(0.0), !self.reduce());
            } else if r.bottom() > top + view - 8.0 {
                self.page.scroll.scroll_to(r.bottom() - view + 16.0, !self.reduce());
            }
        }
    }
}

impl SettingsView {
    pub(super) fn key_event_focus(&mut self, ev: &InputEvent, cx: &mut Cx) -> Response {
        let old = self.focus.focused();
        if self.focus.handle(ev) {
            let new = self.focus.focused();
            self.focus_applied(old, new, true);
            return Response { consumed: true, redraw: true, ..Response::NONE };
        }
        let fid = self.focus.focused();
        let down = matches!(ev, InputEvent::Key { down: true, .. });
        let code = match ev {
            InputEvent::Key { code, .. } => Some(*code),
            _ => None,
        };
        let activate = down && matches!(code, Some(KeyCode::Enter) | Some(KeyCode::Space));
        let rc = Response { consumed: true, redraw: true, ..Response::NONE };
        match fid {
            None => {
                if let InputEvent::Key { code, mods, down: true, .. } = ev {
                    if self.page.scroll.key(*code, *mods, glasskit::scroll::LINE_PX) {
                        return rc;
                    }
                }
                Response::NONE
            }
            Some(super::chrome::FID_SEARCH) => self.search_key(ev, cx),
            Some(n) if n >= 1000 => {
                if activate {
                    let page = pill_ui_core::settings::nav::NAV.iter().filter_map(|x| if let pill_ui_core::settings::nav::NavItem::Page(p) = x { Some(*p) } else { None }).nth((n - 1000) as usize);
                    match page {
                        Some(p) if n < super::chrome::FID_QUIT => self.act(Act::Nav(p.to_string()), cx),
                        _ => self.act(Act::Quit, cx),
                    }
                    return rc;
                }
                Response::NONE
            }
            Some(n) => self.row_key(n, ev, activate, cx),
        }
    }

    fn row_key(&mut self, fid: u32, ev: &InputEvent, activate: bool, cx: &mut Cx) -> Response {
        let rc = Response { consumed: true, redraw: true, ..Response::NONE };
        enum K {
            Hot(String),
            Btn(String),
            Week,
            Field(Rect),
            Custom(&'static str),
            Widget,
        }
        let mut k = K::Widget;
        for r in self.page.rows() {
            if r.fid != fid {
                continue;
            }
            k = match &r.ctl {
                Ctl::Hotkey => K::Hot(match &r.row.control {
                    pill_ui_core::settings::schema::Control::Hotkey { action } => action.to_string(),
                    _ => String::new(),
                }),
                Ctl::Button => K::Btn(r.row.id.to_string()),
                Ctl::Week(_) => K::Week,
                Ctl::Field(f) => K::Field(f.tf.rect),
                Ctl::Custom(n) => K::Custom(n),
                _ => K::Widget,
            };
        }
        match k {
            K::Hot(a) if activate => {
                self.act(Act::Recorder(a), cx);
                rc
            }
            K::Btn(id) if activate => {
                self.act(Act::RowButton(id), cx);
                rc
            }
            K::Week => {
                if let InputEvent::Key { code, down: true, .. } = ev {
                    return self.week_key(fid, *code, cx);
                }
                Response::NONE
            }
            K::Field(rect) => {
                for r in self.page.rows_mut() {
                    if r.fid == fid {
                        if let Ctl::Field(f) = &mut r.ctl {
                            f.tf.queue(ev, rect);
                        }
                    }
                }
                self.edits_due = true;
                rc
            }
            K::Custom(n) => self.custom_key(n, ev, activate, cx),
            K::Widget => self.dispatch_rows(ev, Some(fid), cx),
            _ => Response::NONE,
        }
    }

    fn week_key(&mut self, fid: u32, code: KeyCode, cx: &mut Cx) -> Response {
        let mut day = None;
        for r in self.page.rows_mut() {
            if r.fid == fid {
                if let Ctl::Week(w) = &mut r.ctl {
                    match code {
                        KeyCode::Left => w.cur = w.cur.saturating_sub(1),
                        KeyCode::Right => w.cur = (w.cur + 1).min(6),
                        KeyCode::Space | KeyCode::Enter => day = pill_ui_core::settings::model::WEEKDAYS.get(w.cur).map(|d| d.0.to_string()),
                        _ => {}
                    }
                }
            }
        }
        if let Some(d) = day {
            self.act(Act::Day(d), cx);
        }
        Response { consumed: true, redraw: true, ..Response::NONE }
    }

    pub(super) fn recorder_key(&mut self, code: KeyCode, mods: Modifiers, cx: &mut Cx) {
        use pill_ui_core::settings::hotkey::RecStep;
        let Some(action) = self.model.recorder.active().map(str::to_string) else { return };
        let e = key_event(code, mods);
        match self.model.recorder.key_down(&e) {
            Some(RecStep::Partial(m)) => self.rec_live = m,
            Some(RecStep::Cancel) => self.rec_changed(),
            Some(RecStep::Clear) => {
                self.rec_changed();
                self.commit_hotkey(&action, "", cx);
            }
            Some(RecStep::Commit(combo)) => {
                self.rec_changed();
                self.commit_hotkey(&action, &combo, cx);
            }
            None => {}
        }
    }

    pub(super) fn commit_hotkey(&mut self, action: &str, combo: &str, cx: &mut Cx) {
        match self.model.try_set(&format!("hotkey.{action}"), json!(combo)) {
            Ok(calls) => {
                for c in calls {
                    self.call(cx, c, action);
                }
            }
            Err(pill_ui_core::settings::SetError::Hotkey(h)) => self.row_message(action, pill_ui_core::settings::flows::Msg::bad(h.message().get(self.lang()))),
            Err(_) => {}
        }
    }
}

#[cfg(test)]
impl SettingsView {
    /// Window-space rectangle of the hit registered for `act` in the last frame (tests click through it).
    pub(super) fn hit_rect(&self, act: &Act) -> Option<Rect> {
        let o = self.ui.page_origin;
        self.ui.hits.iter().rev().find(|h| &h.act == act).map(|h| if h.page { h.rect.offset(o.0, o.1) } else { h.rect })
    }

    /// Window-space rectangle of the control of row `id` (last frame).
    pub(super) fn ctl_rect_of(&self, id: &str) -> Option<Rect> {
        let o = self.ui.page_origin;
        self.page.rows().find(|r| r.row.id == id).map(|r| r.ctl_rect.offset(o.0, o.1))
    }
}
