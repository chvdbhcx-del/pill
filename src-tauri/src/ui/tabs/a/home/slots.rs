//! The animated layout of the Home tab: every part ([`hmod::PARTS`]) owns a slot whose rectangle and presence chase the target of
//! [`hmod::plan`] on liquid springs. Switch a module off and its slot fades and shrinks away while the others glide into the freed place;
//! switch one on and it grows in where the plan puts it. Targets are set when the settings change (`set_plan`), never while drawing.
use crate::ui::liquid::{Feel, Flow};
use glasskit::geom::Rect;
use pill_ui_core::panel::a::home_mods::{self as hmod, Plan, N};

/// Where the classic layout draws each section (the paint code of the player, the volume row and the strip is written against these
/// origins); a slot's offset is its current position minus its origin.
const PLAYER_ORIGIN: (f32, f32) = (0.0, 0.0);
const VOLUME_ORIGIN: (f32, f32) = (0.0, 162.0);
const STRIP_ORIGIN: (f32, f32) = (0.0, 208.0);

struct Slot {
    x: Flow,
    y: Flow,
    w: Flow,
    h: Flow,
    /// Presence 0..1: the fade and the small grow of a part that appears or goes.
    a: Flow,
    on: bool,
}

impl Slot {
    fn new() -> Slot {
        Slot { x: Flow::px(0.0, Feel::FLOW), y: Flow::px(0.0, Feel::FLOW), w: Flow::px(0.0, Feel::FLOW), h: Flow::px(0.0, Feel::FLOW), a: Flow::new(0.0, Feel::FADE.within(240.0)), on: false }
    }
    fn flows(&mut self) -> [&mut Flow; 5] {
        [&mut self.x, &mut self.y, &mut self.w, &mut self.h, &mut self.a]
    }
}

pub struct Slots {
    s: [Slot; N],
    plan: Option<Plan>,
    /// The content height as it glides (the settings schematic draws its frame with it).
    h: Flow,
}

impl Slots {
    pub fn new() -> Slots {
        Slots { s: std::array::from_fn(|_| Slot::new()), plan: None, h: Flow::px(0.0, Feel::FLOW) }
    }

    /// The plan the slots chase (`None` before the first `set_plan`).
    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    /// Retarget every slot. The first plan is taken as it is (opening the tab is not an animation); later ones glide.
    pub fn set_plan(&mut self, plan: Plan) {
        if self.plan == Some(plan) {
            return;
        }
        let first = self.plan.is_none();
        if first {
            self.h.jump(plan.h);
        }
        self.h.to(plan.h);
        for (slot, r) in self.s.iter_mut().zip(plan.rects.iter()) {
            match r {
                Some(r) => {
                    if first || !slot.on {
                        // a part that was off appears where it belongs, it does not fly in from the corner
                        slot.x.jump(r.x);
                        slot.y.jump(r.y);
                        slot.w.jump(r.w);
                        slot.h.jump(r.h);
                        if first {
                            slot.a.jump(1.0);
                        }
                    }
                    slot.x.to(r.x);
                    slot.y.to(r.y);
                    slot.w.to(r.w);
                    slot.h.to(r.h);
                    slot.a.to(1.0);
                    slot.on = true;
                }
                None => {
                    slot.a.to(0.0);
                    slot.on = false;
                }
            }
        }
        self.plan = Some(plan);
    }

    /// One frame; true while anything still moves.
    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.h.step(dt_ms);
        let mut busy = self.h.moving();
        for slot in &mut self.s {
            for f in slot.flows() {
                f.step(dt_ms);
                busy |= f.moving();
            }
        }
        busy
    }

    /// Presence of part `i`, 0 (gone) .. 1 (there); a little past 1 never happens (the fade spring is critically damped).
    pub fn alpha(&self, i: usize) -> f32 {
        self.s[i].a.at().clamp(0.0, 1.0)
    }

    /// The part takes clicks: it is on and mostly faded in.
    pub fn live(&self, i: usize) -> bool {
        self.s[i].on && self.alpha(i) > 0.5
    }

    /// Where part `i` is right now (it keeps its last place while it fades out); `None` once it is gone.
    pub fn rect(&self, i: usize) -> Option<Rect> {
        let s = &self.s[i];
        (s.a.at() > 0.004 || s.on).then(|| Rect::new(s.x.at(), s.y.at(), s.w.at(), s.h.at()))
    }

    /// How far part `i` is shifted from where the classic layout draws it (the player, the volume row and the strip).
    pub fn shift(&self, i: usize) -> (f32, f32) {
        let o = match i {
            0 => PLAYER_ORIGIN,
            2 => VOLUME_ORIGIN,
            hmod::STRIP => STRIP_ORIGIN,
            _ => (0.0, 0.0),
        };
        let s = &self.s[i];
        (s.x.at() - o.0, s.y.at() - o.1)
    }

    /// Scale of part `i` while it appears or goes: .94 -> 1.
    pub fn grow(&self, i: usize) -> f32 {
        0.94 + 0.06 * self.alpha(i)
    }

    /// The tab-local pointer as part `i`'s classic layout sees it.
    pub fn to_classic(&self, i: usize, p: (f32, f32)) -> (f32, f32) {
        let (dx, dy) = self.shift(i);
        (p.0 - dx, p.1 - dy)
    }

    /// The content height as it glides to the target.
    pub fn height(&self) -> f32 {
        self.h.at().max(0.0)
    }

    /// The target height of the content (what the panel is sized to).
    pub fn content_h(&self) -> f32 {
        self.plan.map_or(260.0, |p| p.h)
    }
}

impl Default for Slots {
    fn default() -> Self {
        Slots::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use pill_ui_core::panel::a::home_mods::{plan, On, PARTS};

    fn only(keys: &[&str]) -> Plan {
        plan(&On(std::array::from_fn(|i| keys.contains(&PARTS[i].key))))
    }

    fn run(s: &mut Slots, frames: usize) -> usize {
        let mut n = 0;
        while n < frames && s.step(1000.0 / 240.0) {
            n += 1;
        }
        n
    }

    #[test]
    fn the_first_plan_is_taken_as_it_is_and_idles_at_once() {
        let _m = motion_on();
        let mut s = Slots::new();
        s.set_plan(plan(&On::default()));
        assert!(!s.step(4.0), "opening the tab is not an animation");
        assert_eq!(s.rect(0).map(|r| (r.x, r.y, r.w, r.h)), Some((0.0, 0.0, 412.0, 154.0)));
        assert_eq!(s.shift(2), (0.0, 0.0));
        assert_eq!(s.shift(hmod::STRIP), (0.0, 0.0));
        assert_eq!(s.alpha(5), 1.0);
    }

    #[test]
    fn switching_modules_off_glides_the_rest_and_fades_the_gone_ones() {
        let _m = motion_on();
        let mut s = Slots::new();
        s.set_plan(plan(&On::default()));
        s.set_plan(only(&["module.media", "home.tile.area"]));
        assert!(s.step(4.0), "a relayout asks for frames");
        // the tile that stays heads for its new place (the whole side column, centred) without jumping there
        let before = s.rect(3).unwrap();
        let mut last = before.h;
        let mut peak = before.h;
        for _ in 0..60 {
            s.step(4.0);
            let r = s.rect(3).unwrap();
            assert!((r.h - last).abs() < 6.0, "no jump between two frames: {} -> {}", last, r.h);
            last = r.h;
            peak = peak.max(r.h);
        }
        assert!(peak > before.h + 10.0, "the lone tile grows into the side column: {peak}");
        // a tile that went fades but keeps its place meanwhile
        let a1 = s.alpha(5);
        assert!(a1 < 1.0 && a1 > 0.0, "fading: {a1}");
        assert!(!s.live(5), "a part that is going takes no clicks");
        assert!(s.live(0) && s.live(3));
        run(&mut s, 2000);
        assert!(!s.step(4.0), "idle once everything settled");
        assert_eq!(s.rect(5), None, "gone for good");
        assert_eq!(s.rect(0).map(|r| r.x), Some(0.0));
        assert!((s.rect(3).unwrap().h - 72.0).abs() < 0.1 && (s.rect(3).unwrap().w - 228.0).abs() < 0.1);
        assert_eq!(s.content_h(), 154.0);
    }

    #[test]
    fn a_part_switched_on_appears_in_place_and_the_player_recentres() {
        let _m = motion_on();
        let mut s = Slots::new();
        s.set_plan(only(&["module.media"]));
        assert_eq!(s.rect(0).map(|r| r.x), Some(118.0));
        s.set_plan(only(&["module.media", "module.launcher"]));
        let r = s.rect(hmod::STRIP).unwrap();
        assert_eq!((r.x, r.w), (0.0, 648.0), "the strip does not fly in from the corner");
        assert_eq!(s.alpha(hmod::STRIP), 0.0);
        s.step(30.0);
        assert!(s.alpha(hmod::STRIP) > 0.0 && s.alpha(hmod::STRIP) < 1.0);
        run(&mut s, 2000);
        assert_eq!(s.alpha(hmod::STRIP), 1.0);
        assert_eq!(s.to_classic(0, (118.0 + 10.0, 10.0)), (10.0, 10.0));
    }

    #[test]
    fn reduced_motion_lands_at_once() {
        let _g = crate::ui::host::testing::motion_lock();
        glasskit::ease::force_reduced(Some(true));
        let mut s = Slots::new();
        s.set_plan(plan(&On::default()));
        s.set_plan(only(&["module.media"]));
        s.step(1.0);
        glasskit::ease::force_reduced(None);
        assert_eq!(s.rect(0).map(|r| r.x), Some(118.0));
        assert_eq!(s.rect(5), None);
        assert!(!s.step(1.0));
    }
}
