//! Animation kinds of the settings window as data. The renderer plays them; this module only says which, how long and which
//! way, and returns an instant (duration 0) spec when Reduce motion is on. Values are taken from `settings.js`,
//! `settings.css` and `motion.js` (enter/exit/expand/collapse), so the native window moves the way the web one does.
use serde::Serialize;

/// Easing curves used by the window.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub enum Easing {
    Linear,
    /// `cubic-bezier(.22, 1, .36, 1)`: the shared "ease-out" and the settle spring of motion.js.
    EaseOut,
    /// `cubic-bezier(.2, .8, .2, 1)`: the page rise.
    Rise,
    /// `cubic-bezier(.4, 0, 1, 1)`: exits.
    EaseIn,
    /// The overshooting pop of switches, segment pills, chips and the sidebar indicator (`--spring-pop`).
    SpringPop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum AnimKind {
    /// The page on screen slides 10 px left and fades before the next one rises (skipped while the wizard is up).
    PageExit,
    /// The new page rises 8 px and fades in.
    PageEnter,
    /// A row/group that becomes visible grows open (height from 0).
    RowExpand,
    /// A row/group that gets hidden collapses to 0 and is then removed.
    RowCollapse,
    /// The knob of a switch springs across, the track recolours.
    ToggleFlip,
    /// The slider fill/thumb follow the value; the number is repainted at once.
    SliderGlide,
    /// The selected pill of a segmented control slides to the chosen option (first placement without a glide).
    SegmentGlide,
    /// A switched-on chip (quiet-hours day, provider) pops once.
    ChipPop,
    /// A status / result line fades in 6 px from below.
    MessageEnter,
    /// The search results list pops in (6 px down, 3 % scale) ...
    SearchPop,
    /// ... and its entries stagger in 26 ms apart whenever the hits change (not on every arrow key).
    SearchStagger,
    /// The found row is scrolled to the centre and washes with the accent colour for 1.7 s.
    SearchHighlight,
    /// The sidebar indicator glides to the current page.
    SidebarIndicator,
    /// The progress box of a download opens / closes instead of snapping.
    PanelExpand,
    PanelCollapse,
    /// A wizard step slides in from the side it came from (+28 px forward, -28 px back).
    WizardForward,
    WizardBack,
    /// First wizard step (rises 10 px) and a re-render of the same step (4 px).
    WizardFirst,
    WizardRefresh,
    /// A wizard progress dot springs when it becomes the current one.
    WizardDot,
    /// A picker card (island style / pet) shows its selected state.
    PickerSelect,
    /// A pressed button shrinks to 97 %.
    ButtonPress,
    /// The hotkey box switches to its recording look and back.
    KeyCapture,
    /// The window itself fades in after its first paint (140 ms).
    WindowEntrance,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct AnimSpec {
    pub kind: AnimKind,
    pub dur_ms: u32,
    pub dx: i16,
    pub dy: i16,
    /// Start scale (1.0 = none).
    pub scale: f32,
    pub easing: Easing,
    /// Per-item delay for staggered lists.
    pub stagger_ms: u16,
}

impl AnimSpec {
    pub fn is_instant(&self) -> bool {
        self.dur_ms == 0
    }
}

const fn s(kind: AnimKind, dur_ms: u32, dx: i16, dy: i16, scale: f32, easing: Easing, stagger_ms: u16) -> AnimSpec {
    AnimSpec { kind, dur_ms, dx, dy, scale, easing, stagger_ms }
}

/// The animation of `kind`. `reduce` (the `reduceMotion` setting or the OS preference) gives the same spec with no duration
/// and no offset: the end state is applied at once.
pub fn spec(kind: AnimKind, reduce: bool) -> AnimSpec {
    use AnimKind::*;
    use Easing::*;
    let a = match kind {
        PageExit => s(kind, 120, -10, 0, 1.0, EaseIn, 0),
        PageEnter => s(kind, 240, 0, 8, 1.0, Rise, 0),
        RowExpand => s(kind, 340, 0, 0, 1.0, EaseOut, 0),
        RowCollapse => s(kind, 300, 0, 0, 1.0, EaseOut, 0),
        ToggleFlip => s(kind, 450, 0, 0, 1.0, SpringPop, 0),
        SliderGlide => s(kind, 300, 0, 0, 1.0, SpringPop, 0),
        SegmentGlide => s(kind, 320, 0, 0, 1.0, SpringPop, 0),
        ChipPop => s(kind, 400, 0, 0, 0.88, SpringPop, 0),
        MessageEnter => s(kind, 380, 0, 6, 1.0, EaseOut, 0),
        SearchPop => s(kind, 300, 0, -6, 0.97, SpringPop, 0),
        SearchStagger => s(kind, 300, 0, 6, 1.0, EaseOut, 26),
        SearchHighlight => s(kind, 1700, 0, 0, 1.0, EaseOut, 0),
        SidebarIndicator => s(kind, 400, 0, 0, 1.0, SpringPop, 0),
        PanelExpand => s(kind, 340, 0, 0, 1.0, EaseOut, 0),
        PanelCollapse => s(kind, 300, 0, 0, 1.0, EaseOut, 0),
        WizardForward => s(kind, 500, 28, 0, 1.0, EaseOut, 0),
        WizardBack => s(kind, 500, -28, 0, 1.0, EaseOut, 0),
        WizardFirst => s(kind, 400, 0, 10, 1.0, EaseOut, 0),
        WizardRefresh => s(kind, 400, 0, 4, 1.0, EaseOut, 0),
        WizardDot => s(kind, 400, 0, 0, 1.0, SpringPop, 0),
        PickerSelect => s(kind, 220, 0, 0, 1.0, EaseOut, 0),
        ButtonPress => s(kind, 80, 0, 0, 0.97, EaseOut, 0),
        KeyCapture => s(kind, 120, 0, 0, 1.0, EaseOut, 0),
        WindowEntrance => s(kind, 140, 0, 0, 1.0, EaseOut, 0),
    };
    if reduce {
        AnimSpec { dur_ms: 0, dx: 0, dy: 0, scale: 1.0, stagger_ms: 0, ..a }
    } else {
        a
    }
}

/// How the highlight of a found row is scrolled into view: smooth unless motion is reduced.
pub fn scroll_smooth(reduce: bool) -> bool {
    !reduce
}

/// How long the search highlight stays on the row (the wash itself lasts 1.6 s inside it).
pub const HIGHLIGHT_HOLD_MS: u32 = 1700;

/// The animation for a changed control, by control family (feedback on toggle / slider / segmented).
pub fn feedback(control: &crate::settings::schema::Control) -> Option<AnimKind> {
    use crate::settings::schema::Control as C;
    match control {
        C::Toggle | C::BackendToggle(_) => Some(AnimKind::ToggleFlip),
        C::Slider { .. } => Some(AnimKind::SliderGlide),
        C::Segmented { .. } => Some(AnimKind::SegmentGlide),
        C::Weekdays => Some(AnimKind::ChipPop),
        C::PetsPicker | C::CharacterPicker => Some(AnimKind::PickerSelect),
        C::Hotkey { .. } => Some(AnimKind::KeyCapture),
        C::Button { .. } => Some(AnimKind::ButtonPress),
        _ => None,
    }
}

/// The page-switch pair: (exit of the old page, enter of the new one). `wizard`: no exit while the first-run wizard is up.
pub fn page_switch(reduce: bool, wizard: bool) -> (Option<AnimSpec>, AnimSpec) {
    let exit = (!reduce && !wizard).then(|| spec(AnimKind::PageExit, false));
    (exit, spec(AnimKind::PageEnter, reduce))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::schema::Control;

    #[test]
    fn reduce_motion_makes_everything_instant() {
        use AnimKind::*;
        for k in [PageExit, PageEnter, RowExpand, RowCollapse, ToggleFlip, SliderGlide, SegmentGlide, ChipPop, MessageEnter, SearchPop, SearchStagger, SearchHighlight, SidebarIndicator, PanelExpand, PanelCollapse, WizardForward, WizardBack, WizardFirst, WizardRefresh, WizardDot, PickerSelect, ButtonPress, KeyCapture, WindowEntrance] {
            let on = spec(k, false);
            let off = spec(k, true);
            assert!(!on.is_instant(), "{k:?} animates by default");
            assert!(off.is_instant() && off.dx == 0 && off.dy == 0 && off.stagger_ms == 0 && off.scale == 1.0, "{k:?}");
            assert_eq!(on.kind, k);
        }
    }

    #[test]
    fn values_follow_the_web_window() {
        assert_eq!((spec(AnimKind::PageEnter, false).dy, spec(AnimKind::PageEnter, false).dur_ms), (8, 240));
        assert_eq!((spec(AnimKind::PageExit, false).dx, spec(AnimKind::PageExit, false).dur_ms), (-10, 120));
        assert_eq!((spec(AnimKind::WizardForward, false).dx, spec(AnimKind::WizardBack, false).dx), (28, -28));
        assert_eq!(spec(AnimKind::SearchStagger, false).stagger_ms, 26);
        assert_eq!(spec(AnimKind::SearchHighlight, false).dur_ms, HIGHLIGHT_HOLD_MS);
        assert!(scroll_smooth(false) && !scroll_smooth(true));
    }

    #[test]
    fn page_switch_skips_the_exit_in_the_wizard_and_when_reduced() {
        assert!(page_switch(false, false).0.is_some());
        assert!(page_switch(false, true).0.is_none());
        let (exit, enter) = page_switch(true, false);
        assert!(exit.is_none() && enter.is_instant());
    }

    #[test]
    fn feedback_is_attached_to_the_interactive_controls() {
        assert_eq!(feedback(&Control::Toggle), Some(AnimKind::ToggleFlip));
        assert_eq!(feedback(&Control::Weekdays), Some(AnimKind::ChipPop));
        assert_eq!(feedback(&Control::Time), None);
    }
}
