//! Animations of the capture overlay as *data*: the model starts an [`Anim`] when something appears, moves or is confirmed, and
//! the renderer samples it every frame with [`Anim::frame`]. Nothing here owns a timer or caps a frame rate - the renderer ticks
//! at its normal refresh rate and the model only needs `tick(now)` to retire finished animations.
//!
//! Durations and curves are the ones capture.css uses (`--dur-xs` 80 ms, `--dur-fast` 120 ms, `--dur-slow` 380 ms, `bar-rise` /
//! `lens-pop` 400 ms settle, `--ease-out` cubic-bezier(.22,1,.36,1), `--spring-pop` cubic-bezier(.34,1.56,.64,1)). Reduce
//! motion (`Options::reduce_motion`, settings `reduceMotion`, or the OS preference) turns every duration into 0: the end state
//! is applied at once, exactly like the `@media (prefers-reduced-motion: reduce)` block of the stylesheet.
use serde::{Deserialize, Serialize};

/// What is animating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnimKind {
    /// The 8 handle dots pop in when a selection is committed by the pointer.
    HandlesPop,
    /// Toolbar appears below / above the selection: 120 ms fade plus a 4 px rise that settles in 400 ms.
    ToolbarEnter,
    /// Toolbar fades out when the selection is dropped or a commit closes the overlay (the page hid it instantly).
    ToolbarLeave,
    /// Loupe appears (colour mode): fade plus scale from .94.
    LoupeShow,
    /// Loupe box follows the pointer: the renderer eases from the previous `box_pos` to the new one over `duration_ms`.
    LoupeFollow,
    /// A markup op was added: quick fade / settle so it does not just blink into existence.
    OpInsert,
    /// White flash over the selection when copy / done fires.
    CopyFlash,
    /// The hint pill fades away on first pointer-down.
    HintFade,
    /// Toolbar button pressed (scale .92).
    ButtonPress,
    /// Colour swatch picked / hovered (scale 1.15 spring).
    SwatchPop,
}

/// Easing curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ease {
    /// `cubic-bezier(.22, 1, .36, 1)` - `--ease-out`, also the settle curve.
    Out,
    /// `cubic-bezier(.34, 1.56, .64, 1)` - `--spring-pop` (overshoots).
    Spring,
    /// `ease-in` for fade-outs.
    In,
}

/// Static description of one animation kind.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimSpec {
    pub kind: AnimKind,
    /// Total length; 0 under Reduce motion (apply the end state at once).
    pub duration_ms: u32,
    /// Opacity animates over its own, usually shorter, window (`bar-fade` vs `bar-rise`); 0 = instant.
    pub fade_ms: u32,
    pub ease: Ease,
    pub opacity_from: f64,
    pub opacity_to: f64,
    pub scale_from: f64,
    pub scale_to: f64,
    /// Vertical offset at the start, css px (positive = starts lower), ending at 0.
    pub dy_from: f64,
}

/// The spec of an animation, with Reduce motion folded in.
pub fn spec(kind: AnimKind, reduce_motion: bool) -> AnimSpec {
    let s = match kind {
        AnimKind::HandlesPop => AnimSpec { kind, duration_ms: 380, fade_ms: 120, ease: Ease::Spring, opacity_from: 0.0, opacity_to: 1.0, scale_from: 0.0, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::ToolbarEnter => AnimSpec { kind, duration_ms: 400, fade_ms: 120, ease: Ease::Out, opacity_from: 0.0, opacity_to: 1.0, scale_from: 1.0, scale_to: 1.0, dy_from: 4.0 },
        AnimKind::ToolbarLeave => AnimSpec { kind, duration_ms: 120, fade_ms: 120, ease: Ease::In, opacity_from: 1.0, opacity_to: 0.0, scale_from: 1.0, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::LoupeShow => AnimSpec { kind, duration_ms: 400, fade_ms: 120, ease: Ease::Out, opacity_from: 0.0, opacity_to: 1.0, scale_from: 0.94, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::LoupeFollow => AnimSpec { kind, duration_ms: 90, fade_ms: 0, ease: Ease::Out, opacity_from: 1.0, opacity_to: 1.0, scale_from: 1.0, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::OpInsert => AnimSpec { kind, duration_ms: 380, fade_ms: 120, ease: Ease::Spring, opacity_from: 0.0, opacity_to: 1.0, scale_from: 0.98, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::CopyFlash => AnimSpec { kind, duration_ms: 220, fade_ms: 220, ease: Ease::Out, opacity_from: 0.55, opacity_to: 0.0, scale_from: 1.0, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::HintFade => AnimSpec { kind, duration_ms: 180, fade_ms: 180, ease: Ease::In, opacity_from: 1.0, opacity_to: 0.0, scale_from: 1.0, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::ButtonPress => AnimSpec { kind, duration_ms: 80, fade_ms: 0, ease: Ease::Out, opacity_from: 1.0, opacity_to: 1.0, scale_from: 0.92, scale_to: 1.0, dy_from: 0.0 },
        AnimKind::SwatchPop => AnimSpec { kind, duration_ms: 380, fade_ms: 0, ease: Ease::Spring, opacity_from: 1.0, opacity_to: 1.0, scale_from: 1.15, scale_to: 1.0, dy_from: 0.0 },
    };
    if reduce_motion {
        AnimSpec { duration_ms: 0, fade_ms: 0, ..s }
    } else {
        s
    }
}

/// What an animation is attached to; the renderer looks the element up by this.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnimTarget {
    Selection,
    Handles,
    Toolbar,
    /// A toolbar button / swatch by its id (`rect`, `undo`, `#ff3b30`, ...).
    ToolbarItem(String),
    Loupe,
    Op(u32),
    Hint,
}

/// A running animation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anim {
    pub id: u32,
    pub spec: AnimSpec,
    pub target: AnimTarget,
    pub start_ms: u64,
}

/// Interpolated values at one instant.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnimFrame {
    pub opacity: f64,
    pub scale: f64,
    pub dy: f64,
    /// 0..1 of the main duration (after easing).
    pub progress: f64,
    pub done: bool,
}

/// CSS `cubic-bezier(x1, y1, x2, y2)` evaluated at time fraction `t` (0..1): solve x(s) = t by Newton with a bisection fallback.
pub fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    let bez = |a: f64, b: f64, s: f64| 3.0 * (1.0 - s).powi(2) * s * a + 3.0 * (1.0 - s) * s * s * b + s.powi(3);
    let dbez = |a: f64, b: f64, s: f64| 3.0 * (1.0 - s).powi(2) * a + 6.0 * (1.0 - s) * s * (b - a) + 3.0 * s * s * (1.0 - b);
    let mut s = t;
    for _ in 0..8 {
        let err = bez(x1, x2, s) - t;
        if err.abs() < 1e-7 {
            return bez(y1, y2, s);
        }
        let d = dbez(x1, x2, s);
        if d.abs() < 1e-6 {
            break;
        }
        s -= err / d;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    s = t;
    for _ in 0..40 {
        let x = bez(x1, x2, s);
        if (x - t).abs() < 1e-7 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) / 2.0;
    }
    bez(y1, y2, s)
}

impl Ease {
    pub fn apply(self, t: f64) -> f64 {
        match self {
            Ease::Out => cubic_bezier(0.22, 1.0, 0.36, 1.0, t),
            Ease::Spring => cubic_bezier(0.34, 1.56, 0.64, 1.0, t),
            Ease::In => cubic_bezier(0.42, 0.0, 1.0, 1.0, t),
        }
    }
}

impl Anim {
    /// Sample at `now_ms` (clock of `CaptureIo::now_ms`). A zero duration is already at its end state.
    pub fn frame(&self, now_ms: u64) -> AnimFrame {
        let s = &self.spec;
        let el = now_ms.saturating_sub(self.start_ms) as f64;
        let main = if s.duration_ms == 0 { 1.0 } else { (el / s.duration_ms as f64).min(1.0) };
        let fade = if s.fade_ms == 0 { 1.0 } else { (el / s.fade_ms as f64).min(1.0) };
        let e = s.ease.apply(main);
        let ef = if s.fade_ms == 0 { 1.0 } else { s.ease.apply(fade) };
        AnimFrame {
            opacity: s.opacity_from + (s.opacity_to - s.opacity_from) * ef,
            scale: s.scale_from + (s.scale_to - s.scale_from) * e,
            dy: s.dy_from * (1.0 - e),
            progress: e,
            done: main >= 1.0 && fade >= 1.0,
        }
    }
    pub fn is_done(&self, now_ms: u64) -> bool {
        self.frame(now_ms).done
    }
}

/// The running animations of one overlay.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimSet {
    items: Vec<Anim>,
    next_id: u32,
}

impl AnimSet {
    pub fn new() -> AnimSet {
        AnimSet::default()
    }
    /// Start (or restart) an animation. An animation of the same kind on the same target is replaced, so mashing a button
    /// restarts its press instead of stacking.
    pub fn start(&mut self, kind: AnimKind, target: AnimTarget, now_ms: u64, reduce_motion: bool) -> u32 {
        self.items.retain(|a| !(a.spec.kind == kind && a.target == target));
        self.next_id += 1;
        self.items.push(Anim { id: self.next_id, spec: spec(kind, reduce_motion), target, start_ms: now_ms });
        self.next_id
    }
    /// Retire finished animations; returns true while something is still running (the host keeps rendering at full rate then,
    /// and may idle when it is false).
    pub fn tick(&mut self, now_ms: u64) -> bool {
        self.items.retain(|a| !a.is_done(now_ms));
        !self.items.is_empty()
    }
    pub fn active(&self) -> &[Anim] {
        &self.items
    }
    pub fn is_running(&self) -> bool {
        !self.items.is_empty()
    }
    pub fn has(&self, kind: AnimKind) -> bool {
        self.items.iter().any(|a| a.spec.kind == kind)
    }
    pub fn find(&self, kind: AnimKind, target: &AnimTarget) -> Option<&Anim> {
        self.items.iter().find(|a| a.spec.kind == kind && &a.target == target)
    }
    /// Cancel animations of a kind (a new selection cancels the previous toolbar-leave).
    pub fn stop(&mut self, kind: AnimKind) {
        self.items.retain(|a| a.spec.kind != kind);
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
}
