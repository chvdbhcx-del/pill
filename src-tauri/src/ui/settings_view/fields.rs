//! `TextField`: a glasskit `TextInput` whose events are replayed at the next `draw`.
//!
//! `TextInput::event` needs a `Canvas` (DirectWrite hit testing) but `View::event` has none. So the view queues the events of a field with the
//! rectangle they were meant for, and `pump` runs them against the canvas of the frame that follows; `View::step` of the NEXT frame collects the
//! resulting `EditEvent`s (`take`). One frame of latency (4 ms at 240 Hz) is the price of keeping the real edit control.
use glasskit::canvas::Canvas;
use glasskit::edit::{EditEvent, TextInput};
use glasskit::geom::Rect;
use glasskit::input::InputEvent;
use glasskit::theme::Theme;

pub struct TextField {
    pub input: TextInput,
    pending: Vec<(InputEvent, Rect)>,
    out: Vec<EditEvent>,
    /// the text changed since the last commit
    pub dirty: bool,
    /// where it was drawn last (the space the view hit-tests in)
    pub rect: Rect,
}

impl TextField {
    pub fn new(multiline: bool, th: &Theme) -> TextField {
        TextField { input: TextInput::new(multiline, th), pending: Vec::new(), out: Vec::new(), dirty: false, rect: Rect::default() }
    }

    pub fn with_placeholder(mut self, p: &str) -> TextField {
        self.input.placeholder = p.to_string();
        self
    }

    pub fn secret(mut self) -> TextField {
        self.input.password = true;
        self
    }

    pub fn text(&self) -> String {
        self.input.text()
    }

    /// Reset the text without an event (the value came from the model).
    pub fn set_text(&mut self, s: &str) {
        if self.input.text() != s {
            self.input.set_text(s);
        }
        self.dirty = false;
    }

    pub fn queue(&mut self, ev: &InputEvent, rect: Rect) {
        self.pending.push((ev.clone(), rect));
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty() || !self.out.is_empty()
    }

    /// Replay the queued events (call from `draw`, where a canvas exists).
    pub fn pump(&mut self, cv: &Canvas) {
        for (ev, r) in std::mem::take(&mut self.pending) {
            self.input.event(&ev, r, cv);
            for e in self.input.take_events() {
                if e == EditEvent::Changed {
                    self.dirty = true;
                }
                self.out.push(e);
            }
        }
    }

    pub fn take(&mut self) -> Vec<EditEvent> {
        // events produced by `set_focused` and friends outside `pump`
        for e in self.input.take_events() {
            self.out.push(e);
        }
        std::mem::take(&mut self.out)
    }

    pub fn focused(&self) -> bool {
        self.input.is_focused()
    }

    pub fn focus(&mut self, on: bool) {
        self.input.set_focused(on);
    }

    pub fn step(&mut self, dt: f32) -> bool {
        self.input.step(dt)
    }

    pub fn draw(&mut self, cv: &Canvas, rect: Rect, th: &Theme) {
        self.rect = rect;
        self.input.update(rect, cv);
        self.input.draw(cv, rect, th);
    }
}
