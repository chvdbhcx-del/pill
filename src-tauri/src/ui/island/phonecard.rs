//! The phone notification card on the folded island (the Telegram card of the product video): avatar, "<sender> · <app>", the text, "now".
//! Hovering it (or typing in it) grows the island by a second row with the card's two buttons (Hide / Mark read: ordinary card actions, so the
//! shell's own hit test and dismissal run them) and, when the notification can be answered, a Reply field with a send arrow.
//!
//! `layout` is pure (used by `live::layout_card_open` / `measure`). `PhoneUi` owns what outlives a frame: the hover state, the second row's flow,
//! the reply field and the send state. It calls nothing itself: `step` / `event` queue `PhoneOut`s and the island view applies them (keep the
//! card alive, take / give back the keyboard, send the reply).
//! Motion: the lower row, the arrow, the spinner and the hover fills are `Flow`s whose targets are set in `step`; `paint` only reads them.
//! Reduce motion jumps to the end state.
use super::gpu::Gpu;
use super::rows::Meter;
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::b::kit::Field;
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::text::Style;
use glasskit::theme::Theme;
use glasskit::{Canvas, Color};
use pill_ui_core::live::model::{CardBody, CardSpec, PhoneCard};

pub const W: f32 = 440.0;
const H0: f32 = 52.0;
const ROW2: f32 = 46.0;
const PAD: f32 = 14.0;
const TITLE: Style = Style::new(13.0, 600).ellipsis();
const TEXT: Style = Style::new(12.0, 400).ellipsis();
const WHEN: Style = Style::new(11.0, 500);
const BTN: Style = Style::new(12.0, 600);
/// How long the card stays after the pointer left it or the text was cleared; and while it is being worked with (hovered, typing, sending).
pub const HOLD_AWAY_MS: f64 = 2600.0;
pub const HOLD_WORKING_MS: f64 = 90_000.0;
/// A reply the phone never answered: the field works again after this long.
const SEND_TIMEOUT_MS: f64 = 12_000.0;

#[derive(Clone, Debug)]
pub struct Lay {
    pub w: f32,
    pub h: f32,
    pub avatar: Rect,
    pub title: Rect,
    pub text: Rect,
    pub when: Rect,
    pub buttons: [Rect; 2],
    pub field: Rect,
    pub send: Rect,
}

/// Card-local geometry (origin = the card's top-left). `open`: the second row is part of the card.
pub fn layout(p: &PhoneCard, m: &dyn Meter, open: bool) -> Lay {
    let h = if open { H0 + ROW2 } else { H0 };
    let avatar = Rect::new(PAD, (H0 - 32.0) / 2.0, 32.0, 32.0);
    let when_w = m.w(&p.when, WHEN).ceil().max(24.0);
    let when = Rect::new(W - PAD - when_w, 10.0, when_w, 16.0);
    let tx = avatar.right() + 10.0;
    let title = Rect::new(tx, 9.0, (when.x - 8.0 - tx).max(40.0), 18.0);
    let text = Rect::new(tx, 28.0, W - PAD - tx, 16.0);
    let by = H0 + (ROW2 - 30.0) / 2.0 - 2.0;
    let (w0, w1) = ((m.w(&p.hide, BTN) + 28.0).max(56.0), (m.w(&p.mark_read, BTN) + 28.0).max(56.0));
    let b0 = Rect::new(PAD, by, w0, 30.0);
    let b1 = Rect::new(b0.right() + 8.0, by, w1, 30.0);
    let send = Rect::new(W - PAD - 30.0, by, 30.0, 30.0);
    let fx = b1.right() + 10.0;
    let field = Rect::new(fx, by, (send.x - 6.0 - fx).max(60.0), 30.0);
    Lay { w: W, h, avatar, title, text, when, buttons: [b0, b1], field, send }
}

/// What the island view must do for the card.
#[derive(Clone, Debug, PartialEq)]
pub enum PhoneOut {
    /// Keep the card on screen for this long from now (it is being worked with), or let it go soon.
    Ttl(f64),
    /// Take / give back the keyboard (the shell's `TextFocus`).
    Focus(bool),
    /// Send this reply (`phone_notif_action`).
    Reply { key: String, text: String },
}

pub struct PhoneUi {
    open: Flow,
    arrow: Flow,
    spin: Flow,
    send_hov: Flow,
    field: Field,
    /// id of the card this state belongs to
    card: Option<String>,
    key: String,
    can_reply: bool,
    hovering: bool,
    sending: Option<f64>,
    want_send: bool,
    focused: bool,
    holding: Option<bool>,
    outs: Vec<PhoneOut>,
    now: f64,
    /// geometry of the last paint: the layout and the card's top-left in the live strip's px
    lay: Option<(Lay, (f32, f32))>,
    /// the stored avatar key (so the decoded picture is cached per card)
    avatar_id: u64,
}

impl PhoneUi {
    pub fn new(th: &Theme) -> PhoneUi {
        let mut field = Field::new(false, th);
        field.ti.radius = 15.0;
        field.ti.pad = (12.0, 0.0);
        let fl = |f: Feel| Flow::new(0.0, f);
        PhoneUi {
            open: fl(Feel::FLOW),
            arrow: fl(Feel::FADE),
            spin: fl(Feel::FADE),
            send_hov: fl(Feel::FADE),
            field,
            card: None,
            key: String::new(),
            can_reply: false,
            hovering: false,
            sending: None,
            want_send: false,
            focused: false,
            holding: None,
            outs: Vec::new(),
            now: 0.0,
            lay: None,
            avatar_id: 0,
        }
    }

    #[cfg(test)]
    pub fn moving(&self) -> bool {
        self.open.moving() || self.arrow.moving() || self.spin.moving() || self.send_hov.moving() || self.sending.is_some() || self.focused
    }

    #[cfg(test)]
    pub fn dbg_lay(&self) -> Option<(Lay, (f32, f32))> {
        self.lay.clone()
    }

    /// A phone card is the current card.
    pub fn active(&self) -> bool {
        self.card.is_some()
    }

    /// The second row is (becoming) part of the card: the capsule is measured with it.
    pub fn open_target(&self) -> bool {
        self.card.is_some() && (self.hovering || self.focused || self.sending.is_some() || !self.field.ti.text().is_empty())
    }

    pub fn take_outs(&mut self) -> Vec<PhoneOut> {
        std::mem::take(&mut self.outs)
    }

    /// The pointer left the island window.
    pub fn leave(&mut self) {
        self.hovering = false;
    }

    /// The reply command failed: the field is usable again.
    pub fn reply_failed(&mut self) {
        self.sending = None;
    }

    /// Per frame: follow the current card, set the flows' targets. `card`: the typed card on screen (anything but a phone card releases the state).
    pub fn step(&mut self, card: Option<&CardSpec>, dt_ms: f64, now: f64, reduced: bool) -> bool {
        self.now = now;
        let want = card.and_then(|c| if let CardBody::Phone(p) = &c.body { Some((c.id.clone(), p.clone())) } else { None });
        match want {
            Some((id, p)) => {
                if self.card.as_deref() != Some(id.as_str()) {
                    self.reset();
                    self.card = Some(id);
                    self.key = p.key.clone();
                    self.avatar_id = fnv(&p.key);
                    self.field.ti.placeholder = p.reply_hint.clone();
                }
                self.can_reply = p.can_reply;
            }
            None => {
                if self.card.is_some() {
                    self.reset();
                }
            }
        }
        if std::mem::take(&mut self.want_send) {
            self.send();
        }
        if self.sending.is_some_and(|t| now - t > SEND_TIMEOUT_MS) {
            self.sending = None;
        }
        let open = self.open_target();
        if self.card.is_some() && self.holding != Some(open) {
            self.holding = Some(open);
            self.outs.push(PhoneOut::Ttl(if open { HOLD_WORKING_MS } else { HOLD_AWAY_MS }));
        }
        let has_text = !self.field.ti.text().trim().is_empty();
        let mut busy = false;
        for (f, to) in [(&mut self.open, open as u8), (&mut self.arrow, (has_text && self.sending.is_none()) as u8), (&mut self.spin, self.sending.is_some() as u8)] {
            if reduced {
                f.jump(to as f32);
            } else {
                f.to(to as f32);
            }
            f.step(dt_ms);
            busy |= f.moving();
        }
        self.send_hov.step(dt_ms);
        busy |= self.send_hov.moving();
        if self.focused {
            busy |= self.field.ti.step(dt_ms as f32 / 1000.0) | self.field.has_input();
        }
        busy | self.sending.is_some()
    }

    fn reset(&mut self) {
        if self.focused {
            self.outs.push(PhoneOut::Focus(false));
        }
        self.card = None;
        self.key.clear();
        self.hovering = false;
        self.focused = false;
        self.sending = None;
        self.holding = None;
        self.want_send = false;
        self.field.ti.set_text("");
        self.field.ti.set_focused(false);
        for f in [&mut self.open, &mut self.arrow, &mut self.spin, &mut self.send_hov] {
            f.jump(0.0);
        }
        self.lay = None;
    }

    fn send(&mut self) {
        let text = self.field.ti.text();
        let text = text.trim();
        if self.sending.is_some() || text.is_empty() || !self.can_reply {
            return;
        }
        self.outs.push(PhoneOut::Reply { key: self.key.clone(), text: text.to_string() });
        self.sending = Some(self.now);
    }

    fn blur(&mut self) {
        if self.focused {
            self.focused = false;
            self.field.ti.set_focused(false);
            self.outs.push(PhoneOut::Focus(false));
        }
    }

    /// Input for the card. `local`: the pointer in the live strip's px (None for keys and for events without a position).
    pub fn event(&mut self, ev: &InputEvent, local: Option<(f32, f32)>) -> Response {
        let none = Response::NONE;
        let (Some(_), Some((lay, o))) = (&self.card, self.lay.clone()) else { return none };
        let at = |p: (f32, f32), r: Rect| r.offset(o.0, o.1).contains(p.0, p.1);
        let row_open = self.open.at01() > 0.5;
        match (ev, local) {
            (InputEvent::MouseMove { .. }, Some(p)) => {
                let was = self.hovering;
                self.hovering = Rect::new(o.0, o.1, lay.w, lay.h).contains(p.0, p.1);
                let over_send = row_open && self.can_reply && at(p, lay.send);
                self.send_hov.to(over_send as u8 as f32);
                let cursor = if row_open && self.can_reply && at(p, lay.field) {
                    Some(Cursor::Text)
                } else if over_send {
                    Some(Cursor::Hand)
                } else {
                    None
                };
                Response { cursor, redraw: was != self.hovering || over_send, ..none }
            }
            (InputEvent::MouseDown { button: MouseButton::Left, .. }, Some(p)) if row_open => {
                if self.can_reply && at(p, lay.send) {
                    self.want_send = true;
                    return Response { consumed: true, redraw: true, ..none };
                }
                if self.can_reply && at(p, lay.field) {
                    if !self.focused {
                        self.focused = true;
                        self.field.ti.set_focused(true);
                        self.outs.push(PhoneOut::Focus(true));
                    }
                    let mut e = ev.clone();
                    if let InputEvent::MouseDown { pos, .. } = &mut e {
                        *pos = p; // the field is painted in the same space (the live strip), the click is replayed there
                    }
                    self.field.queue(&e);
                    return Response { consumed: true, redraw: true, cursor: Some(Cursor::Text), ..none };
                }
                if !Rect::new(o.0, o.1, lay.w, lay.h).contains(p.0, p.1) {
                    self.blur();
                }
                none
            }
            (InputEvent::MouseUp { .. } | InputEvent::Wheel { .. }, _) if self.focused => {
                let mut e = ev.clone();
                if let (Some(p), InputEvent::MouseUp { pos, .. } | InputEvent::Wheel { pos, .. }) = (local, &mut e) {
                    *pos = p;
                }
                self.field.queue(&e);
                Response { consumed: true, redraw: true, ..none }
            }
            (InputEvent::Key { code: KeyCode::Escape, down: true, .. }, _) if self.focused => {
                self.field.ti.set_text("");
                self.blur();
                Response { consumed: true, redraw: true, ..none }
            }
            (InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_), _) if self.focused => {
                let r = self.field.queue(ev);
                Response { consumed: true, redraw: true, ..r }
            }
            (InputEvent::Focus(false), _) => {
                self.focused = false;
                self.field.ti.set_focused(false);
                none
            }
            _ => none,
        }
    }

    /// Paints the card with its top-left at `o` (the live strip's px). Returns the rects of the card's two actions (the island hit-tests them;
    /// out of reach while the lower row is closed).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, cv: &Canvas, th: &Theme, gpu: &mut Gpu, p: &PhoneCard, o: (f32, f32), t_ms: f64, hover: &[f32]) -> Vec<Rect> {
        struct M<'a, 'b>(&'a Canvas<'b>);
        impl Meter for M<'_, '_> {
            fn w(&self, s: &str, st: Style) -> f32 {
                self.0.text_width(s, st)
            }
        }
        let lay = layout(p, &M(cv), self.open_target());
        self.lay = Some((lay.clone(), o));
        let at = |r: Rect| r.offset(o.0, o.1);
        // avatar: the picture in a circle, else the first letter
        let av = at(lay.avatar);
        match p.avatar.as_deref() {
            Some(b64) => {
                let url = format!("data:image/png;base64,{b64}");
                cv.with_clip_rr(av, 16.0, |cv| gpu.cover(cv, th, self.avatar_id, Some(&url), av, t_ms));
            }
            None => {
                cv.fill_circle(av.center(), 16.0, th.raised);
                let letter: String = p.sender.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                cv.text(av, &letter, Style::new(14.0, 600).center(), th.text);
            }
        }
        let head = if p.app.is_empty() { p.sender.clone() } else { format!("{} · {}", p.sender, p.app) };
        cv.text(at(lay.title), &head, TITLE, th.text);
        cv.text(at(lay.text), &p.text, TEXT, th.dim);
        cv.text(at(lay.when), &p.when, WHEN, th.faint);
        let a = self.open.at01();
        let mut rects = vec![Rect::new(-1e4, -1e4, 0.0, 0.0); 2];
        if a > 0.01 {
            cv.with_opacity(a, |cv| {
                cv.with_offset(0.0, (1.0 - a) * -6.0, |cv| {
                    for (i, label) in [&p.hide, &p.mark_read].into_iter().enumerate() {
                        let r = at(lay.buttons[i]);
                        let h = hover.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                        cv.fill_rr(r, 15.0, th.raised.mix(Color::rgb8(255, 255, 255), 0.12 * h));
                        cv.text(r, label, BTN.center(), th.text2);
                        if a > 0.5 {
                            rects[i] = r;
                        }
                    }
                    if p.can_reply {
                        let fr = at(lay.field);
                        let _ = self.field.pump(cv, fr);
                        self.field.ti.draw(cv, fr, th);
                        for e in self.field.take_events() {
                            match e {
                                glasskit::edit::EditEvent::Submit => self.want_send = true,
                                glasskit::edit::EditEvent::Cancel => {
                                    self.field.ti.set_text("");
                                    self.blur();
                                }
                                _ => {}
                            }
                        }
                        self.paint_send(cv, th, at(lay.send), t_ms);
                    }
                });
            });
        }
        rects
    }

    fn paint_send(&self, cv: &Canvas, th: &Theme, r: Rect, t_ms: f64) {
        let (arrow, spin, hov) = (self.arrow.at01(), self.spin.at01(), self.send_hov.at01());
        let c = r.center();
        cv.fill_circle(c, 15.0, th.raised.mix(th.accent, arrow).mix(Color::rgb8(255, 255, 255), 0.12 * hov * arrow));
        let ink = th.dim.mix(th.on_accent, arrow);
        cv.with_opacity((1.0 - spin).max(0.0), |cv| {
            cv.line((c.0 - 5.5, c.1), (c.0 + 5.5, c.1), 2.0, ink);
            cv.line((c.0 + 1.0, c.1 - 5.0), (c.0 + 5.5, c.1), 2.0, ink);
            cv.line((c.0 + 1.0, c.1 + 5.0), (c.0 + 5.5, c.1), 2.0, ink);
        });
        if spin > 0.01 {
            // ten dots, the bright one walks round
            let lead = ((t_ms / 80.0) as usize) % 10;
            cv.with_opacity(spin, |cv| {
                for i in 0..10usize {
                    let ang = i as f32 / 10.0 * std::f32::consts::TAU;
                    let age = (lead + 10 - i) % 10;
                    cv.fill_circle((c.0 + 7.0 * ang.cos(), c.1 + 7.0 * ang.sin()), 1.4, th.dim.fade(1.0 - age as f32 * 0.09));
                }
            });
        }
    }
}

/// FNV-1a: the id of a card's picture in the cover cache.
fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use glasskit::input::{Buttons, Modifiers};
    use pill_ui_core::live::model::CardSpec;

    struct Fixed;
    impl Meter for Fixed {
        fn w(&self, s: &str, st: Style) -> f32 {
            s.chars().count() as f32 * st.size * 0.55
        }
    }

    fn card(can_reply: bool) -> CardSpec {
        let p = PhoneCard { key: "k1".into(), sender: "Вадим".into(), app: "Telegram".into(), text: "привет".into(), when: "now".into(), avatar: None, can_reply, hide: "Hide".into(), mark_read: "Mark read".into(), reply_hint: "Reply…".into() };
        CardSpec::new("phone:k1", 7000, CardBody::Phone(p))
    }

    fn mv(x: f32, y: f32) -> InputEvent {
        InputEvent::MouseMove { pos: (x, y), mods: Modifiers::NONE, buttons: Buttons::NONE }
    }

    /// The state the painter would leave behind (tests have no canvas).
    fn painted(ui: &mut PhoneUi, c: &CardSpec) {
        let CardBody::Phone(p) = &c.body else { panic!() };
        ui.lay = Some((layout(p, &Fixed, ui.open_target()), (0.0, 0.0)));
    }

    #[test]
    fn the_lower_row_adds_height_and_everything_fits_the_card() {
        let CardBody::Phone(p) = card(true).body else { panic!() };
        let (c, o) = (layout(&p, &Fixed, false), layout(&p, &Fixed, true));
        assert_eq!((c.w, c.h, o.h), (W, H0, H0 + ROW2));
        for r in [o.avatar, o.title, o.text, o.when, o.buttons[0], o.buttons[1], o.field, o.send] {
            assert!(r.x >= 0.0 && r.right() <= W + 0.01 && r.y >= 0.0 && r.bottom() <= o.h + 0.01, "{r:?}");
        }
        assert!(o.buttons[1].right() < o.field.x && o.field.right() < o.send.x, "buttons | field | arrow, in a row");
        assert!(o.title.right() <= o.when.x, "the time never overlaps the title");
        // long Russian labels still leave a usable field
        let mut ru = p.clone();
        ru.hide = "Скрыть".into();
        ru.mark_read = "Прочитано".into();
        assert!(layout(&ru, &Fixed, true).field.w >= 120.0);
    }

    #[test]
    fn hover_opens_the_card_and_holds_it_then_lets_it_go() {
        let c = card(true);
        let mut ui = PhoneUi::new(&Theme::pill_dark());
        assert!(!ui.step(Some(&c), 16.0, 1000.0, true), "nothing moves while nobody is near");
        assert_eq!(ui.take_outs(), vec![PhoneOut::Ttl(HOLD_AWAY_MS)]);
        painted(&mut ui, &c);
        ui.event(&mv(100.0, 20.0), Some((100.0, 20.0)));
        assert!(ui.open_target());
        ui.step(Some(&c), 16.0, 1016.0, true);
        assert_eq!(ui.take_outs(), vec![PhoneOut::Ttl(HOLD_WORKING_MS)]);
        assert!((ui.open.at01() - 1.0).abs() < 1e-3, "reduce motion: end state at once");
        // no repeated Ttl while nothing changes
        ui.step(Some(&c), 16.0, 1032.0, true);
        assert!(ui.take_outs().is_empty());
        // the pointer leaves
        ui.leave();
        ui.step(Some(&c), 16.0, 1050.0, true);
        assert_eq!(ui.take_outs(), vec![PhoneOut::Ttl(HOLD_AWAY_MS)]);
        assert!(!ui.open_target());
    }

    #[test]
    fn the_lower_row_is_a_spring_not_a_jump() {
        let _m = crate::ui::host::testing::motion_on();
        let c = card(true);
        let mut ui = PhoneUi::new(&Theme::pill_dark());
        ui.step(Some(&c), 16.0, 0.0, false);
        painted(&mut ui, &c);
        ui.event(&mv(100.0, 20.0), Some((100.0, 20.0)));
        let mut seen = Vec::new();
        for i in 1..40 {
            ui.step(Some(&c), 16.0, i as f64 * 16.0, false);
            seen.push(ui.open.at());
        }
        assert!(seen.iter().any(|v| *v > 0.05 && *v < 0.95), "{seen:?}");
        assert!((seen.last().unwrap() - 1.0).abs() < 0.05);
    }

    #[test]
    fn clicking_the_field_takes_the_keyboard_and_enter_sends() {
        let c = card(true);
        let mut ui = PhoneUi::new(&Theme::pill_dark());
        ui.step(Some(&c), 16.0, 0.0, true);
        painted(&mut ui, &c);
        ui.event(&mv(100.0, 20.0), Some((100.0, 20.0)));
        ui.step(Some(&c), 16.0, 16.0, true);
        painted(&mut ui, &c);
        let (lay, _) = ui.lay.clone().unwrap();
        let f = lay.field.center();
        let down = InputEvent::MouseDown { pos: f, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 };
        let r = ui.event(&down, Some(f));
        assert!(r.consumed);
        assert!(ui.take_outs().contains(&PhoneOut::Focus(true)));
        // typing goes to the field
        assert!(ui.event(&InputEvent::Char('o'), None).consumed);
        // Esc gives the keyboard back and clears
        let esc = InputEvent::Key { code: KeyCode::Escape, down: true, mods: Modifiers::NONE, repeat: false };
        assert!(ui.event(&esc, None).consumed);
        assert!(ui.take_outs().contains(&PhoneOut::Focus(false)));
        // a reply: text in the field, the arrow is clicked
        ui.field.ti.set_text("хорошо");
        let a = lay.send.center();
        ui.event(&mv(a.0, a.1), Some(a));
        let down = InputEvent::MouseDown { pos: a, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 };
        assert!(ui.event(&down, Some(a)).consumed);
        ui.step(Some(&c), 16.0, 100.0, true);
        assert!(ui.take_outs().contains(&PhoneOut::Reply { key: "k1".into(), text: "хорошо".into() }));
        assert!(ui.sending.is_some() && ui.open_target(), "stays open while sending");
        // a second press does not send twice; a failed command frees the field
        ui.want_send = true;
        ui.step(Some(&c), 16.0, 120.0, true);
        assert!(!ui.take_outs().iter().any(|o| matches!(o, PhoneOut::Reply { .. })));
        ui.reply_failed();
        assert!(ui.sending.is_none());
    }

    #[test]
    fn a_card_that_cannot_be_answered_has_no_field_and_a_new_card_resets_everything() {
        let c = card(false);
        let mut ui = PhoneUi::new(&Theme::pill_dark());
        ui.step(Some(&c), 16.0, 0.0, true);
        painted(&mut ui, &c);
        let (lay, _) = ui.lay.clone().unwrap();
        ui.event(&mv(50.0, 20.0), Some((50.0, 20.0)));
        ui.step(Some(&c), 16.0, 16.0, true);
        painted(&mut ui, &c);
        let f = lay.field.center();
        let down = InputEvent::MouseDown { pos: f, button: MouseButton::Left, mods: Modifiers::NONE, clicks: 1 };
        assert!(!ui.event(&down, Some(f)).consumed, "no reply field on this one");
        ui.take_outs();
        // the card goes away (replaced by a normal one): state released, keyboard handed back if it was taken
        ui.focused = true;
        ui.step(None, 16.0, 40.0, true);
        assert!(!ui.active() && ui.take_outs().contains(&PhoneOut::Focus(false)));
        ui.step(Some(&card(true)), 16.0, 60.0, true);
        assert!(ui.active() && !ui.open_target());
    }

    #[test]
    fn idle_when_nothing_happens() {
        let _m = crate::ui::host::testing::motion_on();
        let c = card(true);
        let mut ui = PhoneUi::new(&Theme::pill_dark());
        let mut moving = true;
        for i in 0..60 {
            moving = ui.step(Some(&c), 16.0, i as f64 * 16.0, false);
        }
        assert!(!moving, "a resting card asks for no frames");
    }
}
