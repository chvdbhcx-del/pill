//! `Field`: a glasskit `TextInput` that can live inside a `View`.
//!
//! `TextInput::event` needs a `Canvas` (text measurement), but `View::event` has none. So input is QUEUED in `event` and applied at the start
//! of the next `draw`, where a canvas exists; the `EditEvent`s that produces are picked up by the view's next `step` (which keeps returning
//! true while [`Field::pending`]). Focus changes that need no measurement (click outside = blur, click inside = focus) are applied at once.
use glasskit::canvas::Canvas;
use glasskit::edit::{EditEvent, TextInput};
use glasskit::geom::Rect;
use glasskit::input::{Cursor, InputEvent, KeyCode, MouseButton, Response};
use glasskit::text::Style;
use glasskit::theme::Theme;

pub struct Field {
    pub input: TextInput,
    queue: Vec<(InputEvent, Rect)>,
    out: Vec<EditEvent>,
}

impl Field {
    pub fn new(th: &Theme, placeholder: &str) -> Field {
        let mut input = TextInput::new(false, th);
        input.style = Style::new(12.0, 400);
        input.placeholder = placeholder.to_string();
        Field { input, queue: Vec::new(), out: Vec::new() }
    }

    pub fn text(&self) -> String {
        self.input.text()
    }

    pub fn focused(&self) -> bool {
        self.input.is_focused()
    }

    /// Queue `ev` (positions in the same space as `rect`). The response covers hover cursor and clicks; the typing result shows next frame.
    pub fn queue(&mut self, ev: &InputEvent, rect: Rect) -> Response {
        let mut r = Response::NONE;
        match ev {
            InputEvent::MouseDown { pos, button: MouseButton::Left, .. } => {
                let inside = rect.contains(pos.0, pos.1);
                if inside != self.input.is_focused() {
                    self.input.set_focused(inside);
                    r.redraw = true;
                }
                if !inside {
                    return r;
                }
            }
            InputEvent::MouseMove { pos, .. } if rect.contains(pos.0, pos.1) => r.cursor = Some(Cursor::Text),
            InputEvent::MouseMove { .. } | InputEvent::MouseUp { .. } => {}
            InputEvent::Key { .. } | InputEvent::Char(_) | InputEvent::Ime(_) if !self.input.is_focused() => return r,
            InputEvent::Key { code: KeyCode::Tab, .. } => return r,
            _ => {}
        }
        self.queue.push((ev.clone(), rect));
        Response { consumed: true, redraw: true, ..r }
    }

    /// Apply the queued input (needs a canvas) and collect the events.
    pub fn flush(&mut self, cv: &Canvas) {
        for (ev, rect) in std::mem::take(&mut self.queue) {
            let r = self.input.event(&ev, rect, cv);
            if r.focus {
                self.input.set_focused(true);
            }
            self.out.extend(self.input.take_events());
        }
    }

    /// Queued input or undelivered events: the view still needs a frame.
    pub fn pending(&self) -> bool {
        !self.queue.is_empty() || !self.out.is_empty()
    }

    pub fn take_events(&mut self) -> Vec<EditEvent> {
        std::mem::take(&mut self.out)
    }

    pub fn set_text(&mut self, s: &str) {
        self.input.set_text(s);
    }
}
