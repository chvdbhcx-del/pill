//! Small extras of the pets in the capsule: the room a big pet needs, the pet stepping aside while music plays (`pet.keepDuringMusic` off),
//! and the height of the liquid edge under a point (the legs stand on it).
use super::companions::PetCfg;
use crate::ui::liquid::{Feel, Flow};
use crate::ui::pets::engine::{box_size, Place, Size};

/// css px the collapsed capsule must be at least tall for this pet (0 = the normal height): only the big size outgrows the 30 px capsule.
pub fn capsule_min_h(cfg: &PetCfg) -> f64 {
    if cfg.id.is_empty() || cfg.size != Size::Xl {
        return 0.0;
    }
    let (_, h) = box_size(&cfg.id, cfg.size, Place::Cap, 1.0);
    (h + 6.0).ceil() as f64 // 1.5 px floor under the feet, a little air above the head
}

/// How far (css px, >= 0) the liquid edge hangs below the straight bottom edge at `x` css px from the island's middle.
pub fn edge_offset_at(points: &[(f64, f64)], x: f64) -> f64 {
    let Some(i) = points.windows(2).position(|w| x >= w[0].0 && x <= w[1].0) else { return 0.0 };
    let (a, b) = (points[i], points[i + 1]);
    let t = if b.0 > a.0 { (x - a.0) / (b.0 - a.0) } else { 0.0 };
    (a.1 + (b.1 - a.1) * t).max(0.0)
}

/// The pet fading out while music plays and back in when it stops (a spring, so a quick pause does not flicker it).
pub struct MusicHide(Flow);

impl Default for MusicHide {
    fn default() -> Self {
        MusicHide(Flow::new(0.0, Feel::FADE))
    }
}

impl MusicHide {
    /// Advance towards `hide`; true while it still moves.
    pub fn step(&mut self, hide: bool, dt_ms: f64) -> bool {
        self.0.chase(if hide { 1.0 } else { 0.0 }, dt_ms);
        self.0.moving()
    }
    /// Opacity factor of the pet, 1 = fully there.
    pub fn alpha(&self) -> f32 {
        1.0 - self.0.at01()
    }
    pub fn gone(&self) -> bool {
        self.0.at01() > 0.99
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use serde_json::json;

    #[test]
    fn only_the_big_pet_makes_the_capsule_taller() {
        let cfg = |size: &str| super::super::companions::pet_cfg(&json!({"pet": "cat", "pet.size": size}));
        assert_eq!((capsule_min_h(&cfg("m")), capsule_min_h(&cfg("l"))), (0.0, 0.0));
        assert!(capsule_min_h(&cfg("xl")) > 30.0, "{}", capsule_min_h(&cfg("xl")));
        assert_eq!(capsule_min_h(&PetCfg::default()), 0.0);
    }

    #[test]
    fn the_edge_offset_is_interpolated_and_zero_outside() {
        let pts = [(-10.0, 0.0), (0.0, 4.0), (10.0, 0.0)];
        assert_eq!(edge_offset_at(&pts, 0.0), 4.0);
        assert_eq!(edge_offset_at(&pts, 5.0), 2.0);
        assert_eq!(edge_offset_at(&pts, 50.0), 0.0);
        assert_eq!(edge_offset_at(&[], 1.0), 0.0);
    }

    #[test]
    fn the_pet_fades_out_with_inertia_and_settles() {
        let _m = motion_on();
        let mut h = MusicHide::default();
        assert!(!h.step(false, 16.0) && h.alpha() == 1.0);
        assert!(h.step(true, 16.0), "moving");
        assert!(h.alpha() < 1.0 && !h.gone(), "not a snap");
        for _ in 0..200 {
            h.step(true, 16.0);
        }
        assert!(h.gone() && !h.step(true, 16.0), "settled: no more frames");
        for _ in 0..200 {
            h.step(false, 16.0);
        }
        assert_eq!(h.alpha(), 1.0);
    }
}
