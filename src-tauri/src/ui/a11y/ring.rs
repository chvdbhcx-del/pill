//! The keyboard focus ring: ONE ring per window that glides (spring) from the control that had the focus to the one that has it now,
//! growing or shrinking to its size on the way, and fades in / out when keyboard mode starts / ends. Targets are set from `step`
//! (`aim`), `paint` only reads. Reduce motion: the springs land at once (see `liquid.rs`), so the ring simply jumps.
use crate::ui::liquid::{Feel, Flow};
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::Color;

/// The glide between two controls; a hair slower than a pop, so the eye can follow it.
const GLIDE: Feel = Feel::FLOW.within(300.0);
const SHOW: Feel = Feel::FADE.within(140.0);

pub struct FocusRing {
    x: Flow,
    y: Flow,
    w: Flow,
    h: Flow,
    a: Flow,
    /// the control the ring stands on (None = hidden / fading out)
    at: Option<u32>,
}

impl Default for FocusRing {
    fn default() -> Self {
        FocusRing::new()
    }
}

impl FocusRing {
    pub fn new() -> FocusRing {
        FocusRing { x: Flow::px(0.0, GLIDE), y: Flow::px(0.0, GLIDE), w: Flow::px(0.0, GLIDE), h: Flow::px(0.0, GLIDE), a: Flow::new(0.0, SHOW), at: None }
    }

    /// Stand on control `id` (its rect now), or hide (`None`). Call from `step` / an event, every time: a control that moves (a tab that
    /// opens) drags the ring with it, a new control makes it glide, from hidden it appears in place.
    pub fn aim(&mut self, target: Option<(u32, Rect)>) {
        match target {
            Some((id, r)) => {
                if self.a.at() < 0.02 && self.a.target() < 0.5 {
                    // coming from nothing: appear in place, do not fly in from wherever it last stood
                    for (f, v) in [(&mut self.x, r.x), (&mut self.y, r.y), (&mut self.w, r.w), (&mut self.h, r.h)] {
                        f.jump(v);
                    }
                } else {
                    self.x.to(r.x);
                    self.y.to(r.y);
                    self.w.to(r.w);
                    self.h.to(r.h);
                }
                self.a.to(1.0);
                self.at = Some(id);
            }
            None => {
                self.a.to(0.0);
                self.at = None;
            }
        }
    }

    /// Advance the springs; true while any moves (frames are needed).
    pub fn step(&mut self, dt_ms: f64) -> bool {
        for f in [&mut self.x, &mut self.y, &mut self.w, &mut self.h, &mut self.a] {
            f.step(dt_ms);
        }
        self.moving()
    }

    pub fn moving(&self) -> bool {
        [&self.x, &self.y, &self.w, &self.h, &self.a].iter().any(|f| f.moving())
    }

    /// The control the ring stands on.
    pub fn at(&self) -> Option<u32> {
        self.at
    }

    pub fn rect(&self) -> Rect {
        Rect::new(self.x.at(), self.y.at(), self.w.at().max(0.0), self.h.at().max(0.0))
    }

    pub fn alpha(&self) -> f32 {
        self.a.at01()
    }

    /// Paint the ring (a 2 px outline with a 1 px gap, glasskit's own). `hc`: a second outer line for high contrast.
    pub fn paint(&self, cv: &Canvas, accent: Color, hc: bool) {
        let k = self.alpha();
        if k <= 0.0 {
            return;
        }
        let r = self.rect();
        let rad = (r.h / 2.0).min(14.0);
        cv.focus_ring(r, rad, accent, k);
        if hc {
            // a second, outer line: the ring must read against any colour behind it
            cv.focus_ring(r.inset(-3.0), rad + 3.0, accent, k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(r: &mut FocusRing) {
        for _ in 0..200 {
            r.step(16.0);
        }
    }

    #[test]
    fn appears_in_place_then_glides_between_controls() {
        let mut r = FocusRing::new();
        assert!(!r.moving() && r.alpha() == 0.0);
        r.aim(Some((1, Rect::new(100.0, 10.0, 40.0, 24.0))));
        assert_eq!(r.rect().x, 100.0, "no flight from the origin when it first appears");
        assert!(r.moving(), "the fade-in runs");
        settle(&mut r);
        assert!(!r.moving() && (r.alpha() - 1.0).abs() < 0.01);
        r.aim(Some((2, Rect::new(200.0, 10.0, 60.0, 24.0))));
        r.step(16.0);
        let mid = r.rect();
        assert!(mid.x > 100.0 && mid.x < 200.0, "gliding, not jumping: {}", mid.x);
        assert!(r.moving());
        settle(&mut r);
        assert!((r.rect().x - 200.0).abs() < 0.1 && (r.rect().w - 60.0).abs() < 0.1, "it takes the new control's size too");
        assert_eq!(r.at(), Some(2));
    }

    #[test]
    fn hides_and_returns_without_flying() {
        let mut r = FocusRing::new();
        r.aim(Some((1, Rect::new(0.0, 0.0, 10.0, 10.0))));
        settle(&mut r);
        r.aim(None);
        settle(&mut r);
        assert_eq!((r.alpha(), r.at()), (0.0, None));
        r.aim(Some((9, Rect::new(300.0, 50.0, 10.0, 10.0))));
        assert_eq!((r.rect().x, r.rect().y), (300.0, 50.0));
    }
}
