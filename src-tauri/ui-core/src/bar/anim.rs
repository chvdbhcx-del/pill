//! Animations as data. The JS bar animates through `PillMotion` (enter / exit / collapse / expand / flip / pop) and CSS keyframes; the native
//! bar emits the same moves as `Anim` values that the renderer plays on its own clock (Direct2D / glasskit). Nothing here ticks: an `Anim`
//! is "start this on that target", with the project's constants (durations in ms, offsets in px, easing names from theme.css).
//!
//! Reduced motion (`Settings > Reduce motion` or the OS flag) is applied at the source: `Anims::push` drops everything while it is on, and the
//! number glides jump to their value (see `Glide`), so a renderer that honours what it receives is already correct under reduced motion.
//! Frame, tick and refresh rates are never lowered: the wave and the glides run at whatever the renderer's frame clock gives them.

/// Easing curves of `theme.css`, by name; the renderer owns the actual cubic / spring tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ease {
    /// `--ease-out` cubic-bezier(.22, 1, .36, 1): opacity and colour
    Out,
    /// `--ease-out-expo`
    OutExpo,
    /// `--ease-in` cubic-bezier(.4, 0, 1, 1): exits
    In,
    /// `--spring-settle`: enter moves (transform only, never opacity)
    Settle,
    /// `--spring-pop`: presses, pops
    Pop,
    /// `1 - (1 - k)^3`: number glides
    CubicOut,
}

/// Which element the move plays on. Indices are positions in the current `BarView` (rows) or the current panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// the whole bar body (`.body.in` pop on open)
    Body,
    /// the result list as one block
    List,
    /// result row i
    Row(usize),
    /// the glide indicator under the rows (selection highlight): moves from row `from` to row `to`
    Selection { from: usize, to: usize },
    /// the pin button of row i
    PinButton(usize),
    /// the AI answer block
    Answer,
    /// the one-line message under the list (errors, pin confirmation)
    Message,
    /// the right-click menu; `item` = None is the menu itself, Some(k) its k-th line
    Context { item: Option<usize> },
    /// the uninstall box (the takeover container)
    PanelBox,
    /// the current uninstall step page
    PanelStep,
    /// the k-th block of a step page (staggered)
    PanelPart(usize),
    /// a checklist group (0 = Certain, 1 = Probable, 2 = Admin as listed in the view)
    Group(usize),
    /// a checklist row (global index in `Checklist::rows`)
    ChecklistRow(usize),
    /// the progress line shown while deleting
    DeleteProgress,
    /// the status strip under the results
    Strip,
    /// the strip's label block (name + message + count)
    StripLabel,
    /// the stage word of the scan line
    ScanStage,
    /// a gliding number: `GlideId`
    Number(GlideId),
    /// a tick box (its check draws itself)
    Tick(usize),
}

/// Which gliding number a `Target::Number` / `Glide` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlideId {
    ScanFound,
    DeleteProgress,
    SelectedCount,
    SelectedSize,
    GroupCount(usize),
    GroupSize(usize),
    StripCount,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// fade in + move from (dx, dy) to rest
    Enter,
    /// fade out + move to (dx, dy)
    Exit,
    /// height / padding to 0 (the rest slides up), optionally removing the element afterwards
    Collapse,
    /// height from 0 to natural
    Expand,
    /// spring pop from `from_scale` / `from_rotate`
    Pop,
    /// one soft colour flash over the row (opacity up then down)
    Flash,
    /// a row that survived a list rebuild slides from its old spot (FLIP)
    Slide,
    /// the selection highlight travels between rows
    Glide,
    /// a short horizontal shake (failures)
    Shake,
    /// the check mark of a tick draws itself
    DrawTick,
}

/// One animation request. Constructors below carry the JS constants.
#[derive(Clone, Debug, PartialEq)]
pub struct Anim {
    pub target: Target,
    pub kind: Kind,
    pub dur_ms: u32,
    pub delay_ms: u32,
    pub dx: f32,
    pub dy: f32,
    pub ease: Ease,
    pub from_scale: f32,
    pub from_rotate_deg: f32,
    /// Exit / Collapse: take the element out of the view when done
    pub remove: bool,
    /// Exit / Collapse: hide instead of removing
    pub hide: bool,
}

fn ms(seconds: f32) -> u32 {
    (seconds * 1000.0).round() as u32
}

impl Anim {
    fn base(target: Target, kind: Kind, dur_s: f32, ease: Ease) -> Anim {
        Anim { target, kind, dur_ms: ms(dur_s), delay_ms: 0, dx: 0.0, dy: 0.0, ease, from_scale: 1.0, from_rotate_deg: 0.0, remove: false, hide: false }
    }
    /// `PillMotion.enter(el, { y, dur })`
    pub fn enter(target: Target, dy: f32, dur_s: f32) -> Anim {
        Anim { dy, ..Anim::base(target, Kind::Enter, dur_s, Ease::Settle) }
    }
    /// `PillMotion.exit(el, { y, x, dur })`
    pub fn exit(target: Target, dx: f32, dy: f32, dur_s: f32) -> Anim {
        Anim { dx, dy, ..Anim::base(target, Kind::Exit, dur_s, Ease::In) }
    }
    /// `PillMotion.collapse(el, { dur })`
    pub fn collapse(target: Target, dur_s: f32) -> Anim {
        Anim::base(target, Kind::Collapse, dur_s, Ease::OutExpo)
    }
    /// `PillMotion.expand(el)`
    pub fn expand(target: Target) -> Anim {
        Anim::base(target, Kind::Expand, 0.4, Ease::OutExpo)
    }
    pub fn with_delay(mut self, delay_ms: u32) -> Anim {
        self.delay_ms = delay_ms;
        self
    }
    pub fn removing(mut self) -> Anim {
        self.remove = true;
        self
    }
    pub fn hiding(mut self) -> Anim {
        self.hide = true;
        self
    }
}

/// List / step stagger: 28 ms per item, capped at the 8th (`.stag`, theme.css).
pub fn stagger_ms(i: usize) -> u32 {
    i.min(8) as u32 * 28
}

// ---- the named moves of bar.js / uninstall-ui.js, in one place ----

/// the bar pops in from above when it opens (`.body.in`): fade + `translateY(-18px) scale(.96, .9)` on the pop spring, 450 ms.
pub fn body_pop() -> Anim {
    Anim { from_scale: 0.96, ..Anim { ease: Ease::Pop, dur_ms: 450, ..Anim::enter(Target::Body, -18.0, 0.45) } }
}
/// the first render after opening staggers the rows; later renders only slide survivors and drop new rows in (`PillMotion.flip`).
pub fn row_insert(i: usize, first_render: bool) -> Anim {
    Anim::enter(Target::Row(i), 6.0, 0.4).with_delay(if first_render { stagger_ms(i) } else { 0 })
}
pub fn row_slide(i: usize) -> Anim {
    Anim { ease: Ease::Settle, dur_ms: 400, ..Anim::base(Target::Row(i), Kind::Slide, 0.4, Ease::Settle) }
}
pub fn row_remove(i: usize) -> Anim {
    Anim::exit(Target::Row(i), 0.0, 0.0, 0.16).removing()
}
/// the highlight under the rows travels from `from` to `to` (`.selind.mv`: transform on the settle spring, `--dur-slow` 380 ms)
pub fn selection_glide(from: usize, to: usize) -> Anim {
    Anim { dur_ms: 380, ..Anim::base(Target::Selection { from, to }, Kind::Glide, 0.38, Ease::Settle) }
}
/// pin confirmation: the icon pops from `scale(.4) rotate(-40deg)` in 600 ms, the row flashes for 700 ms
pub fn pin_pop(row: usize) -> Anim {
    Anim { from_scale: 0.4, from_rotate_deg: -40.0, ..Anim::base(Target::PinButton(row), Kind::Pop, 0.6, Ease::Pop) }
}
pub fn row_flash(row: usize) -> Anim {
    Anim { dur_ms: 700, ..Anim::base(Target::Row(row), Kind::Flash, 0.6, Ease::Out) }
}
/// message under the list slides in (y 8, 400 ms) and later out (y -4, 220 ms)
pub fn message_enter() -> Anim {
    Anim::enter(Target::Message, 8.0, 0.4)
}
pub fn message_exit() -> Anim {
    Anim::exit(Target::Message, 0.0, -4.0, 0.22)
}
/// the right-click menu opens from the cursor (scale .88 -> 1 on the settle spring, 340 ms); its lines follow 24 ms apart after 40 ms.
pub fn ctx_open() -> Anim {
    Anim { from_scale: 0.88, ..Anim::base(Target::Context { item: None }, Kind::Enter, 0.34, Ease::Settle) }
}
pub fn ctx_item(k: usize) -> Anim {
    Anim::enter(Target::Context { item: Some(k) }, 5.0, 0.34).with_delay(k as u32 * 24 + 40)
}
/// the uninstall box: appears (y -8, 400 ms), the previous step leaves first (y -6, 140 ms), the next one settles in (y 10, 400 ms)
pub fn panel_box_enter() -> Anim {
    Anim::enter(Target::PanelBox, -8.0, 0.4)
}
pub fn step_exit() -> Anim {
    Anim::exit(Target::PanelStep, 0.0, -6.0, 0.14)
}
pub fn step_enter() -> Anim {
    Anim::enter(Target::PanelStep, 10.0, 0.4)
}
pub fn part_enter(i: usize) -> Anim {
    Anim::enter(Target::PanelPart(i), 6.0, 0.4).with_delay(stagger_ms(i))
}
/// the result list steps aside when the panel takes over (y -4, 120 ms) ...
pub fn list_step_aside() -> Anim {
    Anim::exit(Target::List, 0.0, -4.0, 0.12).hiding()
}
/// ... and comes back when the panel leaves (y 4, 300 ms); the panel itself collapses in 260 ms
pub fn list_back() -> Anim {
    Anim::enter(Target::List, 4.0, 0.3)
}
pub fn panel_collapse() -> Anim {
    Anim::collapse(Target::PanelBox, 0.26)
}
/// the status strip: enter y 8 / 400 ms, exit y 6 / 180 ms, label slide y 4 / 300 ms, finished-step pop
pub fn strip_enter() -> Anim {
    Anim::enter(Target::Strip, 8.0, 0.4)
}
pub fn strip_exit() -> Anim {
    Anim::exit(Target::Strip, 0.0, 6.0, 0.18).hiding()
}
pub fn strip_label_slide() -> Anim {
    Anim::enter(Target::StripLabel, 4.0, 0.3)
}
pub fn strip_pop() -> Anim {
    Anim { from_scale: 0.94, ..Anim::base(Target::Strip, Kind::Pop, 0.5, Ease::Pop) }
}
/// the stage word of the scan line leaves (y -3, 100 ms) and the new one comes in (y 3, 250 ms)
pub fn stage_exit() -> Anim {
    Anim::exit(Target::ScanStage, 0.0, -3.0, 0.1)
}
pub fn stage_enter() -> Anim {
    Anim::enter(Target::ScanStage, 3.0, 0.25)
}
/// the deleting progress line drops in (y -6, 300 ms)
pub fn delete_progress_enter() -> Anim {
    Anim::enter(Target::DeleteProgress, -6.0, 0.3)
}
/// a deleted checklist row slides right (x 18, 180 ms) and then collapses (240 ms, removed)
pub fn checklist_row_exit(i: usize) -> Anim {
    Anim::exit(Target::ChecklistRow(i), 18.0, 0.0, 0.18)
}
pub fn checklist_row_collapse(i: usize) -> Anim {
    Anim::collapse(Target::ChecklistRow(i), 0.24).removing()
}
pub fn group_expand(g: usize) -> Anim {
    Anim::expand(Target::Group(g))
}
pub fn group_collapse(g: usize) -> Anim {
    Anim::collapse(Target::Group(g), 0.26).hiding()
}
/// a group whose last row is gone collapses away (260 ms, hidden)
pub fn group_vanish(g: usize) -> Anim {
    group_collapse(g)
}
pub fn shake(target: Target) -> Anim {
    Anim { dx: 6.0, ..Anim::base(target, Kind::Shake, 0.42, Ease::Out).with_delay(120) }
}
pub fn draw_tick(i: usize) -> Anim {
    Anim::base(Target::Tick(i), Kind::DrawTick, 0.22, Ease::Out)
}

/// Collects animations while reduced motion is off; the owner drains them once per event / tick.
#[derive(Clone, Debug, Default)]
pub struct Anims {
    reduce: bool,
    out: Vec<Anim>,
}

impl Anims {
    pub fn new(reduce: bool) -> Anims {
        Anims { reduce, out: Vec::new() }
    }
    pub fn set_reduce(&mut self, reduce: bool) {
        self.reduce = reduce;
        if reduce {
            self.out.clear();
        }
    }
    pub fn reduced(&self) -> bool {
        self.reduce
    }
    /// Queues an animation; dropped under reduced motion (`PillMotion.off()`).
    pub fn push(&mut self, a: Anim) {
        if !self.reduce {
            self.out.push(a);
        }
    }
    pub fn drain(&mut self) -> Vec<Anim> {
        std::mem::take(&mut self.out)
    }
    pub fn len(&self) -> usize {
        self.out.len()
    }
    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }
}

/// A number that glides to its new value in 380 ms on a cubic ease-out (`glide()` in uninstall-ui.js). `value(now)` is what to draw.
/// Instant under reduced motion or when the target does not change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glide {
    from: f64,
    to: f64,
    t0: f64,
    /// last value handed out (so a retarget in the middle starts from where the eye is)
    shown: f64,
}

pub const GLIDE_MS: f64 = 380.0;

impl Default for Glide {
    fn default() -> Glide {
        Glide { from: 0.0, to: 0.0, t0: 0.0, shown: 0.0 }
    }
}

impl Glide {
    pub fn new(v: f64) -> Glide {
        Glide { from: v, to: v, t0: 0.0, shown: v }
    }
    /// Aim at `to`. The JS restarts from the previous *target* (`node.dataset.v`), not from the half-way text; the same here.
    pub fn set(&mut self, to: f64, now_ms: f64, reduce: bool) {
        let from = self.to;
        self.to = to;
        if reduce || from == to {
            self.from = to;
            self.shown = to;
            return;
        }
        self.from = from;
        self.t0 = now_ms;
        self.shown = from;
    }
    pub fn value(&mut self, now_ms: f64) -> f64 {
        if self.from == self.to {
            self.shown = self.to;
            return self.to;
        }
        let k = ((now_ms - self.t0) / GLIDE_MS).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - k).powi(3);
        self.shown = self.from + (self.to - self.from) * e;
        if k >= 1.0 {
            self.from = self.to;
            self.shown = self.to;
        }
        self.shown
    }
    /// Read without advancing the clock (the last drawn value).
    pub fn shown(&self) -> f64 {
        self.shown
    }
    pub fn target(&self) -> f64 {
        self.to
    }
    pub fn running(&self) -> bool {
        self.from != self.to
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduced_motion_drops_every_animation() {
        let mut a = Anims::new(true);
        a.push(body_pop());
        a.push(strip_enter());
        assert!(a.is_empty());
        a.set_reduce(false);
        a.push(strip_enter());
        assert_eq!(a.len(), 1);
        a.set_reduce(true);
        assert!(a.is_empty(), "switching reduce on drops what was queued");
    }

    #[test]
    fn project_constants() {
        let e = step_enter();
        assert_eq!((e.dy, e.dur_ms, e.ease), (10.0, 400, Ease::Settle));
        let x = step_exit();
        assert_eq!((x.dy, x.dur_ms, x.ease), (-6.0, 140, Ease::In));
        assert_eq!(strip_exit().dur_ms, 180);
        assert!(strip_exit().hide);
        assert_eq!(checklist_row_exit(2).dx, 18.0);
        assert_eq!(pin_pop(0).from_rotate_deg, -40.0);
        assert_eq!(ctx_item(3).delay_ms, 3 * 24 + 40);
        assert_eq!(stagger_ms(0), 0);
        assert_eq!(stagger_ms(3), 84);
        assert_eq!(stagger_ms(40), 8 * 28, "stagger is capped at the 8th item");
        assert!(row_insert(2, true).delay_ms > 0 && row_insert(2, false).delay_ms == 0);
    }

    #[test]
    fn glide_eases_out_and_lands() {
        let mut g = Glide::new(0.0);
        g.set(100.0, 1000.0, false);
        assert_eq!(g.value(1000.0), 0.0);
        let mid = g.value(1190.0);
        assert!(mid > 50.0 && mid < 100.0, "ease-out is past half-way at half time: {mid}");
        assert_eq!(g.value(1380.0), 100.0);
        assert!(!g.running());
        // reduced motion and unchanged targets are instant
        g.set(7.0, 2000.0, true);
        assert_eq!(g.value(2000.0), 7.0);
        g.set(7.0, 2100.0, false);
        assert!(!g.running());
    }

    #[test]
    fn glide_retarget_restarts_from_previous_target() {
        let mut g = Glide::new(0.0);
        g.set(10.0, 0.0, false);
        g.set(20.0, 100.0, false);
        assert_eq!(g.value(100.0), 10.0, "like node.dataset.v: restart from the last target");
        assert_eq!(g.value(480.0), 20.0);
    }
}
