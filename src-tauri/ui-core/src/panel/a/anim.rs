//! Animation vocabulary of panel set A, as plain data.
//!
//! Every UI feature animates (standing rule), so view-models never draw anything themselves: they push
//! [`Anim`] descriptors (through `Fx::Anim`) naming a target and a kind, and the Direct2D renderer plays them.
//! The numbers are the ones `src/motion.js` (PillMotion) and `theme.css` use today, so the native UI feels the same.
//! Reduce motion: [`Anim::gate`] returns `None` when motion is off; the renderer then applies the end state at once.

/// Easing curves of `theme.css` (cubic-bezier control points) plus the two CSS keywords the tabs use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ease {
    /// `--ease-out` cubic-bezier(.22, 1, .36, 1)
    Out,
    /// `--ease-out-expo` cubic-bezier(.16, 1, .3, 1)
    OutExpo,
    /// `--spring-settle` (same curve as `Out`; used for transforms)
    SpringSettle,
    /// `--spring-pop` cubic-bezier(.34, 1.56, .64, 1) (small overshoot)
    SpringPop,
    /// `--ease-in` cubic-bezier(.4, 0, 1, 1)
    In,
    /// CSS `ease-out`
    CssOut,
    /// CSS `ease-in-out`
    CssInOut,
    Linear,
}

impl Ease {
    /// Control points (x1, y1, x2, y2) of the cubic bezier.
    pub fn bezier(self) -> [f32; 4] {
        match self {
            Ease::Out | Ease::SpringSettle => [0.22, 1.0, 0.36, 1.0],
            Ease::OutExpo => [0.16, 1.0, 0.3, 1.0],
            Ease::SpringPop => [0.34, 1.56, 0.64, 1.0],
            Ease::In => [0.4, 0.0, 1.0, 1.0],
            Ease::CssOut => [0.0, 0.0, 0.58, 1.0],
            Ease::CssInOut => [0.42, 0.0, 0.58, 1.0],
            Ease::Linear => [0.0, 0.0, 1.0, 1.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnimKind {
    /// Fade in (220 ms) while the transform settles from (dx, dy, scale) to rest. `PillMotion.enter`.
    Enter { dx: f32, dy: f32, scale: f32 },
    /// Fade out while moving to (dx, dy, scale); the target is removed / hidden afterwards. `PillMotion.exit`.
    Exit { dx: f32, dy: f32, scale: f32 },
    /// Height / padding to 0 with a fade, then removed. `PillMotion.collapse`.
    Collapse,
    /// Grow from 0 to the natural height. `PillMotion.expand`.
    Expand,
    /// Spring pop from a scale to rest. `PillMotion.pop` and the CSS "retrigger a class" pulses.
    Pop { from_scale: f32 },
    /// Siblings glide from their old spot to the new one (FLIP); `enter_new` fades new rows in.
    Flip { enter_new: bool },
    /// A tile flies from a screen point to its slot (shelf), with a rotation.
    FlyIn { from_dx: f32, from_dy: f32, rot_deg: f32 },
    /// The new content sits on top of the old one and fades in; the old one is dropped afterwards.
    Crossfade,
    /// Whole view slides in by `dx` while fading (tab switch).
    SlideIn { dx: f32 },
    /// Old view slides away by `dx` while fading out (tab switch).
    SlideAway { dx: f32 },
    /// Endless rotation for as long as the operation runs (refresh button).
    Spin,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Anim {
    pub kind: AnimKind,
    /// Duration of the main (transform / size) part.
    pub dur_ms: u32,
    pub delay_ms: u32,
    pub ease: Ease,
}

/// List stagger of `theme.css` `.stag`: 28 ms per row, capped at the 8th.
pub fn stagger_ms(i: usize) -> u32 {
    i.min(8) as u32 * 28
}

impl Anim {
    fn new(kind: AnimKind, dur_ms: u32, ease: Ease) -> Anim {
        Anim { kind, dur_ms, delay_ms: 0, ease }
    }
    pub fn delayed(mut self, ms: u32) -> Anim {
        self.delay_ms = ms;
        self
    }
    /// `None` under reduced motion: the caller applies the end state directly.
    pub fn gate(self, reduce: bool) -> Option<Anim> {
        if reduce {
            None
        } else {
            Some(self)
        }
    }

    // ---- PillMotion defaults ----
    /// `PillMotion.enter(el, {y, dur: .4})`
    pub fn enter(dy: f32) -> Anim {
        Anim::new(AnimKind::Enter { dx: 0.0, dy, scale: 1.0 }, 400, Ease::SpringSettle)
    }
    /// `PillMotion.exit(el, {y, delay})`: ease-in, .16 s
    pub fn exit(dy: f32, scale: f32) -> Anim {
        Anim::new(AnimKind::Exit { dx: 0.0, dy, scale }, 160, Ease::In)
    }
    /// `PillMotion.collapse`: .3 s on the out curve
    pub fn collapse() -> Anim {
        Anim::new(AnimKind::Collapse, 300, Ease::Out)
    }
    pub fn pop(from_scale: f32, dur_ms: u32) -> Anim {
        Anim::new(AnimKind::Pop { from_scale }, dur_ms, Ease::SpringPop)
    }
    /// `PillMotion.flip`: 400 ms settle, new rows enter with y 4 / .3 s.
    pub fn flip(enter_new: bool) -> Anim {
        Anim::new(AnimKind::Flip { enter_new }, 400, Ease::SpringSettle)
    }
    /// A list row of a freshly shown tab (CSS `.stag`): fade .22 s + rise 6 px on the settle spring, 28 ms per row.
    pub fn row_stagger(i: usize) -> Anim {
        Anim::new(AnimKind::Enter { dx: 0.0, dy: 6.0, scale: 1.0 }, 400, Ease::SpringSettle).delayed(stagger_ms(i))
    }

    // ---- named cues used by the tabs ----
    /// Home: title / artist text change: quick fade + 5 px slide, 220 ms.
    pub fn text_swap() -> Anim {
        Anim::new(AnimKind::Enter { dx: 0.0, dy: 5.0, scale: 1.0 }, 220, Ease::OutExpo)
    }
    /// Home: play / pause glyph swap: scale .55 -> 1 with a fade, 220 ms.
    pub fn glyph_swap() -> Anim {
        Anim::new(AnimKind::Enter { dx: 0.0, dy: 0.0, scale: 0.55 }, 220, Ease::OutExpo)
    }
    /// Home: new cover over the old one, 250 ms.
    pub fn cover_fade() -> Anim {
        Anim::new(AnimKind::Crossfade, 250, Ease::CssOut)
    }
    /// Home: launcher icons slide aside when one is added / removed, 280 ms.
    pub fn launcher_flip() -> Anim {
        Anim::new(AnimKind::Flip { enter_new: false }, 280, Ease::OutExpo)
    }
    /// Weather: the previous condition's gradient fades out under the new one, 0.8 s.
    pub fn gradient_fade() -> Anim {
        Anim::new(AnimKind::Crossfade, 800, Ease::CssInOut)
    }
    /// Shelf: a chip flies in from the drop point (700 ms spring-pop, 60 ms per chip up to the 6th).
    pub fn fly_in(from_dx: f32, from_dy: f32, k: usize) -> Anim {
        let rot = if k % 2 == 1 { 8.0 } else { -8.0 };
        Anim::new(AnimKind::FlyIn { from_dx, from_dy, rot_deg: rot }, 700, Ease::SpringPop).delayed(k.min(6) as u32 * 60)
    }
    /// Tab switch: the new view (CSS `view-in` .14 s + `view-slide-*` .24 s on the settle spring, 10 px).
    pub fn view_in(dir_right: bool) -> Anim {
        Anim::new(AnimKind::SlideIn { dx: if dir_right { 10.0 } else { -10.0 } }, 240, Ease::SpringSettle).delayed(30)
    }
    /// Tab switch: the old view (CSS `view-out` .08 s + `view-away-*` .12 s).
    pub fn view_away(dir_right: bool) -> Anim {
        Anim::new(AnimKind::SlideAway { dx: if dir_right { -10.0 } else { 10.0 } }, 120, Ease::In)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stagger_caps_at_eight() {
        assert_eq!(stagger_ms(0), 0);
        assert_eq!(stagger_ms(3), 84);
        assert_eq!(stagger_ms(8), 224);
        assert_eq!(stagger_ms(99), 224);
    }

    #[test]
    fn reduced_motion_drops_the_animation() {
        assert!(Anim::text_swap().gate(true).is_none());
        assert_eq!(Anim::text_swap().gate(false).unwrap().dur_ms, 220);
    }

    #[test]
    fn tab_switch_directions_mirror() {
        // going right: the new view comes from +10 px, the old one leaves to -10 px
        assert_eq!(Anim::view_in(true).kind, AnimKind::SlideIn { dx: 10.0 });
        assert_eq!(Anim::view_away(true).kind, AnimKind::SlideAway { dx: -10.0 });
        assert_eq!(Anim::view_in(false).kind, AnimKind::SlideIn { dx: -10.0 });
        assert_eq!(Anim::view_in(true).delay_ms, 30);
    }

    #[test]
    fn fly_in_alternates_tilt_and_caps_delay() {
        let a = Anim::fly_in(10.0, -5.0, 1);
        assert_eq!(a.kind, AnimKind::FlyIn { from_dx: 10.0, from_dy: -5.0, rot_deg: 8.0 });
        assert_eq!(Anim::fly_in(0.0, 0.0, 0).delay_ms, 0);
        assert_eq!(Anim::fly_in(0.0, 0.0, 20).delay_ms, 360);
        assert_eq!(Ease::SpringPop.bezier()[1], 1.56);
    }
}
