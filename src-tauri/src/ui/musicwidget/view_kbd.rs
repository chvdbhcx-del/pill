//! Keyboard mode, UI Automation controls and high contrast of the music widget (a child module of `view`, so it reaches the view's own fields).
//! The second stop of the keyboard hotkey (`Win+Alt+J`: island, then this widget, then off, see `ui/a11y/target.rs`). The widget takes the
//! keyboard (`Cx::set_focus`: the user's window is remembered and gets the foreground back), opens its card, and the keys are:
//! Space = play / pause, Left / Right = back / forward 5 s (previous / next track when the track has no length), Shift+Left / Shift+Right =
//! previous / next track, Tab and Enter = the focus ring over the three buttons and the position bar, Esc = leave. Media keys are not touched.
//! `controls()` lists the three buttons and the position slider from the same layout the hit test uses; a screen reader invokes them or sets the
//! position (UIA RangeValue).
use super::*;
use crate::ui::a11y::contrast::{self, Mode};
use crate::ui::a11y::nav::{self, Act, Control, NavKey, NavOut, Role, Value as CV};
use crate::ui::a11y::ring::FocusRing;
use crate::ui::a11y::target::{self, Target};

const SEEK_STEP_MS: i64 = 5000;
/// Control ids: the three buttons are 1..=3 (Prev, Play, Next), the position bar is 10.
const SEEK: u32 = 10;
/// How long a key press keeps the button dipped.
const PRESS_MS: f64 = 110.0;

pub(super) struct Kbd {
    pub on: bool,
    pub focus: Option<u32>,
    pub ring: FocusRing,
    /// a button dipped by a key, and when to let it up
    dipped: Option<(usize, f64)>,
    pub ru: bool,
    pub hc: Option<contrast::Palette>,
    /// the last frame's time (the position the slider reports)
    pub now: f64,
}

impl Kbd {
    pub fn new() -> Kbd {
        Kbd { on: false, focus: None, ring: FocusRing::new(), dipped: None, ru: false, hc: None, now: 0.0 }
    }
}

impl MusicView {
    /// The controls on screen now, in window logical px (empty while the widget is not showing a track).
    pub(super) fn kbd_controls(&self) -> Vec<Control> {
        if !self.vis || self.fade.at01() < 0.2 || !self.track.has_session {
            return Vec::new();
        }
        let (p, dy) = (self.open(), self.slide());
        let (cap, card) = lg::frames(self.stand, p, dy);
        let ru = self.kbd.ru;
        let play = match (self.track.playing, ru) {
            (true, true) => "Пауза",
            (true, false) => "Pause",
            (false, true) => "Воспроизвести",
            (false, false) => "Play",
        };
        let names = [if ru { "Предыдущий трек" } else { "Previous track" }, play, if ru { "Следующий трек" } else { "Next track" }];
        let mut out = Vec::new();
        let open = p >= lg::CARD_AT;
        let btn = if open { lg::card_layout(card).btn } else { lg::capsule_layout_in(self.stand, cap).btn };
        for (i, r) in btn.iter().enumerate() {
            out.push(Control::new(1 + i as u32, names[i], Role::Button, self.stand.rect_to_real(*r)));
        }
        if open && self.track.duration_ms > 0 {
            let bar = lg::card_layout(card).bar;
            let cur = self.seek.map_or(self.clock.pos(self.kbd.now as i64), |f| (f as f64 * self.track.duration_ms as f64) as u64);
            out.push(
                Control::new(SEEK, if ru { "Позиция в треке" } else { "Track position" }, Role::Slider, self.stand.rect_to_real(bar))
                    .with(CV::Range { min: 0.0, max: self.track.duration_ms as f64, cur: cur as f64, step: SEEK_STEP_MS as f64 }),
            );
        }
        out
    }

    /// A key, a screen reader or a click on a control of the widget.
    pub(super) fn kbd_act(&mut self, id: u32, act: Act, cx: &mut Cx) -> bool {
        match (id, act) {
            (1..=3, Act::Invoke | Act::Select | Act::Toggle) => {
                let b = BTNS[id as usize - 1];
                self.command(b, cx);
                self.kbd.dipped = Some((b.index(), cx.now_ms));
                self.press[b.index()].to(1.0);
                cx.request_frame();
                true
            }
            (SEEK, Act::Step(n)) => self.seek_by(n as i64 * SEEK_STEP_MS, cx),
            (SEEK, Act::Set(ms)) if self.track.duration_ms > 0 => {
                self.commit_seek((ms / self.track.duration_ms as f64).clamp(0.0, 1.0) as f32, cx);
                true
            }
            _ => false,
        }
    }

    /// Seek `delta_ms` from where the track is now; false when the track has no length.
    fn seek_by(&mut self, delta_ms: i64, cx: &mut Cx) -> bool {
        let dur = self.track.duration_ms;
        if dur == 0 {
            return false;
        }
        let to = (self.pos_now(cx.now_ms) as i64 + delta_ms).clamp(0, dur as i64);
        self.commit_seek(to as f32 / dur as f32, cx);
        cx.request_frame();
        true
    }

    // ------------------------------------------------------------------------------------------------ the mode

    pub(super) fn kbd_event(&mut self, name: &str, payload: &Value, cx: &mut Cx) {
        if name == target::EVENT {
            if payload.get("target").and_then(Value::as_str).and_then(Target::parse) == Some(Target::Widget) {
                self.kbd_enter(cx);
            } else if self.kbd.on {
                self.kbd_leave(cx);
            }
        }
    }

    fn kbd_enter(&mut self, cx: &mut Cx) {
        if !self.vis || !self.want {
            return; // nothing on screen to control
        }
        self.kbd.on = true;
        self.kbd.focus = Some(2); // the play button
        target::set_active(Some(Target::Widget));
        cx.set_focus(true);
        self.retarget();
        cx.request_frame();
    }

    /// Esc / the hotkey / the widget going away: the keyboard goes back to the window the user was in.
    pub(super) fn kbd_leave(&mut self, cx: &mut Cx) {
        self.kbd_stop();
        cx.set_focus(false);
        self.retarget();
        cx.request_frame();
    }

    fn kbd_stop(&mut self) {
        self.kbd.on = false;
        self.kbd.focus = None;
        target::release(Target::Widget);
    }

    /// Input before the normal handling. `None` = not ours.
    pub(super) fn kbd_input(&mut self, ev: &InputEvent, cx: &mut Cx) -> Option<Response> {
        if !self.kbd.on {
            return None;
        }
        match ev {
            InputEvent::MouseDown { .. } | InputEvent::Focus(false) => {
                self.kbd_stop(); // the user took the mouse, or the window lost the keyboard: the ring fades, the click goes on
                self.retarget();
                None
            }
            InputEvent::Key { code, mods, down: true, repeat } => {
                let key = nav::nav_key(*code, *mods)?;
                let done = Some(Response::consumed().merge(Response::redraw()));
                match key {
                    NavKey::Space => {
                        if !repeat {
                            self.kbd_act(2, Act::Invoke, cx);
                        }
                        done
                    }
                    NavKey::Arrow(d @ (nav::Dir::Left | nav::Dir::Right)) => {
                        let right = d == nav::Dir::Right;
                        if mods.shift || self.track.duration_ms == 0 {
                            self.kbd_act(if right { 3 } else { 1 }, Act::Invoke, cx);
                        } else {
                            self.seek_by(if right { SEEK_STEP_MS } else { -SEEK_STEP_MS }, cx);
                        }
                        done
                    }
                    _ => match nav::route(&self.kbd_controls(), self.kbd.focus, key) {
                        NavOut::Focus(id) => {
                            self.kbd.focus = Some(id);
                            done
                        }
                        NavOut::Act(id, act) => {
                            self.kbd.focus = Some(id);
                            self.kbd_act(id, act, cx);
                            done
                        }
                        NavOut::Close => {
                            self.kbd_leave(cx);
                            done
                        }
                        NavOut::Pass => None,
                    },
                }
            }
            _ => None,
        }
    }

    /// Per frame: the ring follows the focused control, a dipped button comes up. True while anything moves.
    pub(super) fn kbd_step(&mut self, dt_ms: f64, cx: &mut Cx) -> bool {
        self.kbd.now = cx.now_ms;
        if self.kbd.on && !self.want {
            self.kbd_leave(cx); // the widget is leaving (a pause went on too long, the guard): give the keyboard back
        }
        if let Some((i, at)) = self.kbd.dipped {
            if cx.now_ms - at >= PRESS_MS {
                self.press[i].to(0.0);
                self.kbd.dipped = None;
            }
        }
        let aim = if self.kbd.on {
            let cs = self.kbd_controls();
            if self.kbd.focus.is_some_and(|f| !cs.iter().any(|c| c.id == f)) {
                self.kbd.focus = cs.iter().find(|c| c.id == 2).map(|c| c.id); // the bar folded away: back to the play button
            }
            self.kbd.focus.and_then(|f| cs.iter().find(|c| c.id == f)).map(|c| (c.id, c.rect))
        } else {
            None
        };
        self.kbd.ring.aim(aim);
        self.kbd.ring.step(dt_ms) | self.kbd.dipped.is_some()
    }

    /// The ring, in window logical px (outside the size transform: the controls are in real px).
    pub(super) fn kbd_paint(&self, cv: &Canvas, th: &Theme, alpha: f32) {
        cv.with_opacity(alpha, |cv| self.kbd.ring.paint(cv, th.accent, self.kbd.hc.is_some()));
    }

    // ------------------------------------------------------------------------------------------------ high contrast

    pub(super) fn contrast_refresh(&mut self) {
        let mode = self.settings.raw.get(contrast::KEY).and_then(Value::as_str).map(Mode::parse).unwrap_or_default();
        let sys = if mode == Mode::Off { None } else { contrast::read_system() };
        self.kbd.hc = contrast::pick(mode, sys);
    }

    pub(super) fn refresh_lang(&mut self) {
        self.kbd.ru = self.settings.resolved_lang(&crate::ui::island::sys::os_lang()) == pill_ui_core::shell::settings::Lang::Ru;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_ids_match_the_button_order() {
        assert_eq!((BTNS[0], BTNS[1], BTNS[2]), (Btn::Prev, Btn::Play, Btn::Next), "ids 1..=3 are indexes + 1");
        assert!(SEEK > 3);
    }
}
