//! Measured layout of the bar (bar.css / bar.html), in logical px at 100 % scale, so the Direct2D renderer lays the same geometry out without
//! reading CSS: one grid for the whole bar (icon column at x = 26, text column at x = 62 for the search row and every result row), heights
//! that are even so every centre lands on a whole pixel, the corner radii, and the periods of the looping animations (spinner, progress bar,
//! recording pulse). Under reduced motion the loops are drawn still (progress bar at 100 % width, 50 % opacity).

/// Window size (command.rs): the transparent window; only the body is drawn.
pub const WINDOW_W: f32 = 560.0;
pub const WINDOW_H: f32 = 350.0;
/// The body: black, big round corners, a wavy outline at the bottom edge. `max-height` includes the search row.
pub const BODY_MAX_H: f32 = 326.0;
pub const BODY_RADIUS: f32 = 24.0;
/// `#bar-root` padding below the body (room for the wave).
pub const ROOT_PAD_BOTTOM: f32 = 12.0;
/// Search row.
pub const SEARCH_H: f32 = 50.0;
pub const SEARCH_GAP: f32 = 14.0;
pub const SEARCH_PAD: (f32, f32) = (20.0, 18.0); // left, right
pub const SEARCH_FONT: f32 = 15.0;
pub const MIC_SIZE: f32 = 28.0;
/// Result list.
pub const LIST_PAD: (f32, f32) = (6.0, 6.0); // sides, bottom
pub const ROW_H: f32 = 38.0;
pub const ROW_RADIUS: f32 = 13.0;
pub const ROW_PAD: (f32, f32) = (14.0, 10.0); // left, right
pub const ROW_GAP: f32 = 12.0;
pub const ROW_ICON: f32 = 18.0;
/// The real shell icon of app / file / folder rows is a bit larger than the glyph.
pub const ROW_SHELL_ICON: f32 = 22.0;
pub const HOVER_BUTTON: f32 = 26.0;
pub const HOVER_BUTTON_RADIUS: f32 = 8.0;
/// Answer block (`? question`), the dialogue hint line, the message line.
pub const ANSWER_MAX_H: f32 = 252.0;
/// A formatted (markdown) answer is capped lower.
pub const ANSWER_MD_MAX_H: f32 = 176.0;
pub const ANSWER_MARGIN: (f32, f32) = (8.0, 8.0);
pub const ANSWER_PAD: (f32, f32) = (12.0, 18.0); // vertical, horizontal
pub const ANSWER_RADIUS: f32 = 16.0;
pub const MESSAGE_PAD: (f32, f32, f32) = (6.0, 26.0, 12.0); // top, sides, bottom
/// Uninstall box and strip.
pub const PANEL_MAX_H: f32 = 262.0;
pub const PANEL_MARGIN: (f32, f32) = (8.0, 8.0);
pub const PANEL_PAD: (f32, f32) = (6.0, 16.0);
pub const PANEL_RADIUS: f32 = 16.0;
pub const STRIP_RADIUS: f32 = 13.0;
pub const STRIP_PAD: (f32, f32, f32) = (8.0, 12.0, 10.0);
pub const TICK_SIZE: f32 = 16.0;
pub const BUTTON_RADIUS: f32 = 10.0;
/// Context menu.
pub const MENU_MIN_W: f32 = 250.0;
pub const MENU_PAD: f32 = 6.0;
pub const MENU_LINE_H: f32 = 34.0;
pub const MENU_RADIUS: f32 = 16.0;

/// Rows that fit in the list when nothing else (answer, message, panel) takes room.
pub fn visible_rows() -> usize {
    ((BODY_MAX_H - SEARCH_H - LIST_PAD.1) / ROW_H).floor() as usize
}

/// Looping animations: one full turn / sweep in ms. They run while the thing they decorate is on screen and never slow down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loop {
    /// the spinner of «Думаю…» (`bar-rot .8s linear`)
    ThinkSpinner,
    /// the progress bars of the uninstall steps and the strip (`u-slide 1.3s ease-in-out`)
    ProgressSlide,
    /// the strip's spinner icon (`u-rot .9s linear`)
    StripSpinner,
    /// the dot on the recording microphone (`mic-pulse 1.1s ease-in-out`)
    MicPulse,
}

impl Loop {
    pub fn period_ms(self) -> u32 {
        match self {
            Loop::ThinkSpinner => 800,
            Loop::ProgressSlide => 1300,
            Loop::StripSpinner => 900,
            Loop::MicPulse => 1100,
        }
    }
}

/// The sliding part of an indeterminate progress bar is this share of its width.
pub const PROGRESS_SLIDE_SHARE: f32 = 0.38;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_is_even_and_the_list_fits_seven_rows() {
        assert_eq!(visible_rows(), 7);
        assert_eq!(SEARCH_H % 2.0, 0.0);
        assert_eq!(ROW_H % 2.0, 0.0);
        const { assert!(BODY_MAX_H + ROOT_PAD_BOTTOM <= WINDOW_H) } // the body and the wave room fit the window
    }

    #[test]
    fn loop_periods_are_the_css_ones() {
        assert_eq!(Loop::ThinkSpinner.period_ms(), 800);
        assert_eq!(Loop::ProgressSlide.period_ms(), 1300);
        assert_eq!(Loop::StripSpinner.period_ms(), 900);
        assert_eq!(Loop::MicPulse.period_ms(), 1100);
    }

    #[test]
    fn the_context_menu_metrics_match_the_model() {
        // model.rs keeps the same numbers to place the menu without asking the renderer
        assert_eq!((MENU_MIN_W, MENU_PAD, MENU_LINE_H), (250.0, 6.0, 34.0));
        assert_eq!((WINDOW_W, WINDOW_H), (crate::bar::model::WIN_W, crate::bar::model::WIN_H));
    }
}
