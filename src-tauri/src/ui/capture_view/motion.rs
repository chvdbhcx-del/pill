//! The motion of the capture overlay. The model (`pill_ui_core::capture`) announces animations as data (`Anim`: what started, when); the view does
//! not replay their time curves but keeps a spring per value (`crate::ui::liquid::Flow`): a pop overshoots a little, a fade glides, the
//! selection rectangle follows the pointer with a short inertia, and a move interrupted half-way keeps its speed. A spring outlives the model's
//! own timer (the model forgets an animation after its nominal duration) until it has settled, so nothing is cut off.
//!
//! The model's numbers stay the truth: the rectangle that is committed, cropped and exported is always the model's, never the eased one painted
//! here, and the eased one lands exactly on it when the pointer is released.
use crate::ui::liquid::{Feel, Flow, Flow2};
use pill_ui_core::capture::{Anim, AnimKind, AnimTarget};
use std::collections::HashMap;

/// What a paint reads for one animation (the fields `Anim::frame` used to give).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Frame {
    pub opacity: f32,
    pub scale: f32,
    pub dy: f32,
    /// Overshooting 0..1 progress of the move (handles: radius factor). For the copy flash: how much of the dim is left (1 -> 0).
    pub progress: f32,
}

/// One running animation as two springs.
struct Eff {
    id: u32,
    mv: Flow,
    fade: Flow,
    /// CopyFlash: it has peaked and is fading away
    back: bool,
    anim: Anim,
}

fn lerp(a: f64, b: f64, t: f32) -> f32 {
    (a + (b - a) * t as f64) as f32
}

impl Eff {
    /// `enter_alpha`: for a toolbar leaving, how visible the toolbar is right now (its enter may still be running).
    fn start(a: &Anim, reduce: bool, enter_alpha: Option<f32>) -> Option<Eff> {
        let (mut mv, mut fade) = (Flow::new(0.0, Feel::FLOW), Flow::new(0.0, Feel::FADE));
        let mut back = false;
        let (mv_to, fade_to);
        match a.spec.kind {
            AnimKind::HandlesPop | AnimKind::OpInsert => {
                mv.set_feel(Feel::POP);
                (mv_to, fade_to) = (1.0, 1.0);
            }
            AnimKind::ToolbarEnter | AnimKind::LoupeShow => (mv_to, fade_to) = (1.0, 1.0),
            AnimKind::ToolbarLeave => {
                fade = Flow::new(enter_alpha.unwrap_or(1.0), Feel::FADE.within(120.0));
                (mv_to, fade_to) = (0.0, 0.0);
            }
            AnimKind::CopyFlash => {
                // `mv` = how much of the dim around the confirmed area is left, `fade` = the white flash: up fast, then away
                mv = Flow::new(1.0, Feel::FADE.within(220.0));
                fade = Flow::new(0.0, Feel::FADE.within(50.0));
                (mv_to, fade_to) = (0.0, 1.0);
            }
            AnimKind::SwatchPop => {
                // a kick on a spring that rests at 1: the swatch swells and settles back
                mv = Flow::new(1.0, Feel::POP);
                mv.nudge(3.7);
                (mv_to, fade_to) = (1.0, 1.0);
            }
            AnimKind::LoupeFollow | AnimKind::HintFade | AnimKind::ButtonPress => return None, // other springs carry these
        }
        if reduce {
            mv.jump(mv_to);
            fade.jump(fade_to);
            back = true;
            if a.spec.kind == AnimKind::CopyFlash {
                fade.jump(0.0);
            }
        } else {
            mv.to(mv_to);
            fade.to(fade_to);
        }
        Some(Eff { id: a.id, mv, fade, back, anim: a.clone() })
    }

    fn step(&mut self, dt_ms: f64) {
        self.mv.step(dt_ms);
        self.fade.step(dt_ms);
        if self.anim.spec.kind == AnimKind::CopyFlash && !self.back && self.fade.at() >= 0.9 {
            self.back = true;
            self.fade.set_feel(Feel::FADE.within(150.0));
            self.fade.to(0.0);
        }
    }

    fn moving(&self) -> bool {
        self.mv.moving() || self.fade.moving() || (self.anim.spec.kind == AnimKind::CopyFlash && !self.back)
    }

    fn frame(&self) -> Frame {
        let s = &self.anim.spec;
        let (m, f) = (self.mv.at(), self.fade.at01());
        let eased = |from: f64, to: f64, k: f32| lerp(from, to, k);
        match s.kind {
            AnimKind::ToolbarLeave => Frame { opacity: f, scale: 1.0, dy: 0.0, progress: 1.0 - f },
            AnimKind::CopyFlash => Frame { opacity: s.opacity_from as f32 * f, scale: 1.0, dy: 0.0, progress: self.mv.at01() },
            AnimKind::SwatchPop => Frame { opacity: 1.0, scale: m, dy: 0.0, progress: m },
            _ => Frame { opacity: eased(s.opacity_from, s.opacity_to, f), scale: eased(s.scale_from, s.scale_to, m), dy: (s.dy_from * (1.0 - m as f64)) as f32, progress: m },
        }
    }
}

/// The selection rectangle as painted: x, y, w, h each on a light spring.
struct SelRect {
    f: [Flow; 4],
    live: bool,
    was_drag: bool,
}

impl SelRect {
    fn new() -> SelRect {
        // a short settle: it is a pointer follower, it has to stay usable
        let f = || Flow::px(0.0, Feel::DRAG.within(120.0));
        SelRect { f: [f(), f(), f(), f()], live: false, was_drag: false }
    }
}

/// All the springs of one overlay.
pub(crate) struct Motion {
    /// the dim over the frozen frame, and the hint pill that follows it
    pub dim: Flow,
    hint: Flow,
    effs: HashMap<(AnimKind, AnimTarget), Eff>,
    sel: SelRect,
    /// the colour-picker lens box
    loupe: Flow2,
    loupe_live: bool,
    pub toast: Flow,
    pub toast_rise: Flow,
    /// the tooltip of a toolbar item: fades in once the pointer has rested, slides 3 px
    pub tip: Flow,
    pub tip_slide: Flow,
    /// the item the tooltip is about (kept while it fades away)
    pub tip_id: Option<String>,
    armed_ms: f64,
}

impl Motion {
    pub fn new() -> Motion {
        let mut m = Motion {
            dim: Flow::new(1.0, Feel::FADE.within(140.0)),
            hint: Flow::new(1.0, Feel::FADE.within(200.0)),
            effs: HashMap::new(),
            sel: SelRect::new(),
            loupe: Flow2::px((0.0, 0.0), Feel::DRAG.within(110.0)),
            loupe_live: false,
            toast: Flow::new(0.0, Feel::FADE),
            toast_rise: Flow::px(0.0, Feel::FLOW),
            tip: Flow::new(0.0, Feel::FADE),
            tip_slide: Flow::new(1.0, Feel::FLOW),
            tip_id: None,
            armed_ms: 0.0,
        };
        m.sel.f.iter_mut().for_each(|f| f.jump(0.0));
        m
    }

    /// The overlay is armed (again): the dim and the hint come in from nothing, everything else starts clean.
    pub fn arm(&mut self, now: f64, reduce: bool) {
        *self = Motion::new();
        self.armed_ms = now;
        let k = if reduce { 1.0 } else { 0.0 };
        self.dim.jump(k);
        self.hint.jump(k);
        if !reduce {
            self.dim.to(1.0);
        }
    }

    pub fn moving(&self) -> bool {
        self.dim.moving()
            || self.hint.moving()
            || self.effs.values().any(Eff::moving)
            || self.sel.f.iter().any(Flow::moving)
            || self.loupe.moving()
            || self.toast.moving()
            || self.toast_rise.moving()
            || self.tip.moving()
            || self.tip_slide.moving()
    }

    /// Picks up the animations the model announced (called from every paint and step, it is idempotent) and aims the rest at the model's state.
    #[allow(clippy::too_many_arguments)]
    pub fn sync(&mut self, anims: &[Anim], sel: Option<[f32; 4]>, dragging: bool, loupe: Option<(f32, f32)>, hint_visible: bool, now: f64, reduce: bool) {
        for a in anims {
            let key = (a.spec.kind, a.target.clone());
            if self.effs.get(&key).is_some_and(|e| e.id == a.id) {
                continue;
            }
            let enter_alpha = self.effs.get(&(AnimKind::ToolbarEnter, AnimTarget::Toolbar)).map(|e| e.frame().opacity);
            match Eff::start(a, reduce, enter_alpha) {
                Some(e) => {
                    self.effs.insert(key, e);
                }
                None => continue,
            }
            if a.spec.kind == AnimKind::ToolbarEnter {
                self.effs.remove(&(AnimKind::ToolbarLeave, AnimTarget::Toolbar));
            }
        }
        // the selection rectangle
        match sel {
            None => self.sel.live = false,
            Some(r) => {
                if !self.sel.live || reduce || (self.sel.was_drag && !dragging) {
                    // first sight of it, or the pointer was just released: exactly the model's rectangle, at once
                    self.sel.f.iter_mut().zip(r).for_each(|(f, v)| f.jump(v));
                    self.sel.live = true;
                } else {
                    self.sel.f.iter_mut().zip(r).for_each(|(f, v)| f.to(v));
                }
            }
        }
        self.sel.was_drag = dragging;
        // the lens
        match loupe {
            None => self.loupe_live = false,
            Some(p) if !self.loupe_live || reduce => {
                self.loupe.jump(p);
                self.loupe_live = true;
            }
            Some(p) => self.loupe.to(p),
        }
        // the hint pill follows the dim by a beat, and leaves when the model hides it
        let hint_on = hint_visible && now - self.armed_ms >= 60.0;
        if reduce {
            self.hint.jump(if hint_visible { 1.0 } else { 0.0 });
        } else {
            self.hint.to(if hint_on { 1.0 } else { 0.0 });
        }
    }

    pub fn step(&mut self, dt_ms: f64) {
        self.dim.step(dt_ms);
        self.hint.step(dt_ms);
        for e in self.effs.values_mut() {
            e.step(dt_ms);
        }
        self.sel.f.iter_mut().for_each(|f| {
            f.step(dt_ms);
        });
        self.loupe.step(dt_ms);
        for f in [&mut self.toast, &mut self.toast_rise, &mut self.tip, &mut self.tip_slide] {
            f.step(dt_ms);
        }
    }

    /// Forgets finished animations the model no longer lists.
    pub fn sweep(&mut self, anims: &[Anim]) {
        self.effs.retain(|k, e| e.moving() || anims.iter().any(|a| (a.spec.kind, &a.target) == (k.0, &k.1)));
    }

    pub fn frame(&self, kind: AnimKind, target: &AnimTarget) -> Option<Frame> {
        self.effs.get(&(kind, target.clone())).map(Eff::frame)
    }

    /// How much of the dim around a confirmed area is left while the overlay leaves (1 -> 0).
    pub fn leave_k(&self) -> f32 {
        let t = AnimTarget::Toolbar;
        self.frame(AnimKind::CopyFlash, &AnimTarget::Selection).map(|f| f.progress).or_else(|| self.frame(AnimKind::ToolbarLeave, &t).map(|f| f.opacity)).unwrap_or(0.0)
    }

    pub fn dim_k(&self) -> f32 {
        self.dim.at01()
    }

    pub fn hint_k(&self) -> f32 {
        self.hint.at01()
    }

    /// The selection rectangle to paint (x, y, w, h), when there is one.
    pub fn sel_rect(&self) -> Option<[f32; 4]> {
        self.sel.live.then(|| [self.sel.f[0].at(), self.sel.f[1].at(), self.sel.f[2].at().max(0.0), self.sel.f[3].at().max(0.0)])
    }

    pub fn loupe_pos(&self) -> (f32, f32) {
        self.loupe.at()
    }

    /// Starts a toast: it fades up and rises 8 px.
    pub fn toast_in(&mut self, reduce: bool) {
        self.toast.jump(0.0);
        self.toast_rise.jump(if reduce { 0.0 } else { 8.0 });
        if reduce {
            self.toast.jump(1.0);
        } else {
            self.toast.to(1.0);
            self.toast_rise.to(0.0);
        }
    }

    pub fn toast_out(&mut self, reduce: bool) {
        if reduce {
            self.toast.jump(0.0);
        } else {
            self.toast.to(0.0);
        }
    }

    /// The tooltip: `item` = the hovered toolbar item once the pointer has rested on it, else None.
    pub fn tip_to(&mut self, item: Option<&str>, reduce: bool) {
        match item {
            Some(id) => {
                if self.tip_id.as_deref() != Some(id) && self.tip.at() < 0.01 {
                    self.tip_id = Some(id.to_string());
                    self.tip_slide.jump(0.0);
                }
                if self.tip_id.as_deref() == Some(id) {
                    if reduce {
                        self.tip.jump(1.0);
                        self.tip_slide.jump(1.0);
                    } else {
                        self.tip.to(1.0);
                        self.tip_slide.to(1.0);
                    }
                } else {
                    self.tip.to(0.0); // another item's tip is still on its way out
                }
            }
            None => {
                if reduce {
                    self.tip.jump(0.0);
                } else {
                    self.tip.to(0.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use pill_ui_core::capture::anim::AnimSet;

    fn run(m: &mut Motion, ms: f64) {
        let mut t = 0.0;
        while t < ms {
            m.step(8.0);
            t += 8.0;
        }
    }

    #[test]
    fn the_painted_selection_trails_the_pointer_a_little_and_snaps_exactly_on_release() {
        let _g = motion_on();
        let mut m = Motion::new();
        m.sync(&[], Some([100.0, 100.0, 0.0, 0.0]), true, None, true, 100.0, false);
        assert_eq!(m.sel_rect(), Some([100.0, 100.0, 0.0, 0.0]), "a new selection starts exactly where the pointer went down");
        m.sync(&[], Some([100.0, 100.0, 200.0, 150.0]), true, None, true, 110.0, false);
        m.step(8.0);
        let r = m.sel_rect().unwrap();
        assert!(r[2] > 0.0 && r[2] < 200.0 * 0.3, "it follows with inertia: {r:?}");
        // keep dragging: it never overshoots the pointer by much and catches up within ~1.5x of its short settle
        let (mut peak, mut last_off) = (0.0f32, 0.0);
        for i in 0..40 {
            m.step(8.0);
            let w = m.sel_rect().unwrap()[2];
            peak = peak.max(w);
            if (w - 200.0).abs() > 10.0 {
                last_off = ((i + 2) * 8) as f32;
            }
        }
        assert!(peak < 200.0 * 1.05, "no bounce to speak of: {peak}");
        assert!(last_off <= 120.0 * 1.5, "caught up after {last_off} ms");
        // a fast move, then the button goes up while it is still behind: the painted rectangle lands exactly on the model's
        m.sync(&[], Some([100.0, 100.0, 400.0, 300.0]), true, None, true, 500.0, false);
        m.step(8.0);
        assert!(m.sel_rect().unwrap()[2] < 399.0);
        m.sync(&[], Some([100.0, 100.0, 400.0, 300.0]), false, None, true, 510.0, false);
        assert_eq!(m.sel_rect(), Some([100.0, 100.0, 400.0, 300.0]));
        assert!(!m.moving(), "and it idles");
    }

    #[test]
    fn reduced_motion_paints_the_model_rectangle_at_once() {
        let _g = motion_on();
        let mut m = Motion::new();
        m.sync(&[], Some([10.0, 10.0, 50.0, 50.0]), true, None, true, 0.0, true);
        m.sync(&[], Some([10.0, 10.0, 90.0, 80.0]), true, None, true, 10.0, true);
        assert_eq!(m.sel_rect(), Some([10.0, 10.0, 90.0, 80.0]));
        assert!(!m.moving());
    }

    #[test]
    fn announced_animations_become_springs_that_outlive_the_models_timer() {
        let _g = motion_on();
        let mut set = AnimSet::new();
        set.start(AnimKind::HandlesPop, AnimTarget::Handles, 0, false);
        set.start(AnimKind::ToolbarEnter, AnimTarget::Toolbar, 0, false);
        let mut m = Motion::new();
        m.sync(set.active(), None, false, None, true, 0.0, false);
        let h = m.frame(AnimKind::HandlesPop, &AnimTarget::Handles).unwrap();
        let t = m.frame(AnimKind::ToolbarEnter, &AnimTarget::Toolbar).unwrap();
        assert_eq!((h.progress, h.opacity), (0.0, 0.0), "the first frame is the start state");
        assert!(t.opacity == 0.0 && (t.dy - 4.0).abs() < 1e-4);
        let (mut peak, mut op_peak) = (0.0f32, 0.0f32);
        let mut last_off = 0.0;
        let mut ms = 0.0;
        while m.moving() && ms < 3000.0 {
            m.step(8.0);
            ms += 8.0;
            let h = m.frame(AnimKind::HandlesPop, &AnimTarget::Handles).unwrap();
            peak = peak.max(h.progress);
            op_peak = op_peak.max(h.opacity);
            if (h.progress - 1.0).abs() > 0.05 {
                last_off = ms;
            }
        }
        assert!(peak > 1.0 && peak < 1.5, "a lively pop: {peak}");
        assert!(op_peak <= 1.0, "opacity never overshoots");
        assert!(last_off < 420.0 * 1.5, "popped by {last_off} ms");
        assert!(!m.moving(), "idle afterwards");
        // the model has forgotten both long ago: the view lets go of them
        m.sweep(&[]);
        assert!(m.frame(AnimKind::HandlesPop, &AnimTarget::Handles).is_none());
    }

    #[test]
    fn a_leaving_toolbar_continues_from_how_visible_it_was() {
        let _g = motion_on();
        let mut set = AnimSet::new();
        set.start(AnimKind::ToolbarEnter, AnimTarget::Toolbar, 0, false);
        let mut m = Motion::new();
        m.sync(set.active(), None, false, None, true, 0.0, false);
        run(&mut m, 40.0);
        let seen = m.frame(AnimKind::ToolbarEnter, &AnimTarget::Toolbar).unwrap().opacity;
        assert!(seen > 0.0 && seen < 0.95);
        set.start(AnimKind::ToolbarLeave, AnimTarget::Toolbar, 40, false);
        m.sync(set.active(), None, false, None, true, 40.0, false);
        let left = m.frame(AnimKind::ToolbarLeave, &AnimTarget::Toolbar).unwrap().opacity;
        assert!((left - seen).abs() < 0.02, "no jump back to full opacity: {seen} -> {left}");
        run(&mut m, 1500.0);
        assert!(m.frame(AnimKind::ToolbarLeave, &AnimTarget::Toolbar).unwrap().opacity < 0.01);
    }

    #[test]
    fn the_copy_flash_rises_fast_fades_away_and_idles() {
        let _g = motion_on();
        let mut set = AnimSet::new();
        set.start(AnimKind::CopyFlash, AnimTarget::Selection, 0, false);
        let mut m = Motion::new();
        m.sync(set.active(), None, false, None, true, 0.0, false);
        assert_eq!(m.frame(AnimKind::CopyFlash, &AnimTarget::Selection).unwrap().opacity, 0.0, "it does not start at full white");
        assert_eq!(m.leave_k(), 1.0, "the dim is still all there");
        let mut peak = 0.0f32;
        let mut ms = 0.0;
        while m.moving() && ms < 3000.0 {
            m.step(8.0);
            ms += 8.0;
            peak = peak.max(m.frame(AnimKind::CopyFlash, &AnimTarget::Selection).unwrap().opacity);
        }
        assert!(peak > 0.45 && peak <= 0.55 + 1e-4, "peak {peak}");
        assert!(m.leave_k() < 0.01);
    }

    #[test]
    fn the_tooltip_waits_fades_out_with_its_own_text_and_the_next_one_comes_after() {
        let _g = motion_on();
        let mut m = Motion::new();
        m.tip_to(Some("rect"), false);
        run(&mut m, 600.0);
        assert_eq!(m.tip_id.as_deref(), Some("rect"));
        assert!(m.tip.at() > 0.99);
        m.tip_to(Some("undo"), false);
        assert_eq!(m.tip_id.as_deref(), Some("rect"), "the old text fades out, it is not swapped under the eye");
        run(&mut m, 1500.0);
        m.tip_to(Some("undo"), false);
        assert_eq!(m.tip_id.as_deref(), Some("undo"));
        run(&mut m, 1500.0);
        m.tip_to(None, false);
        run(&mut m, 1500.0);
        assert!(m.tip.at() < 0.01 && !m.moving());
    }
}
