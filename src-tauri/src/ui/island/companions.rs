//! Pets and capsule characters on the island: `ui::pets::PetEngine` (the pet standing in the capsule / the header gap) and `Characters`
//! (tail, legs + eyes). Layout follows `pets.js` / `characters.js`: the pet stands on the capsule floor 9 px from its end, the weather icon
//! moves to the side opposite the pet, the eyes stand on the text line of the live row, the gaze comes from the cursor feed, the energy from
//! `audio:bands`, the tint from the edge reaction (music = the media row's cover accent).
use crate::ui::pets::characters::Characters;
use crate::ui::pets::engine::{box_size, eye_line_y, look_from_vector, NeedsFrame, PetEngine, Place, Side, Size};
use glasskit::geom::Rect;
use glasskit::{Canvas, Color};
use pill_ui_core::shell::clock::CharSide;
use pill_ui_core::shell::settings::EdgeStyle;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct PetCfg {
    pub id: String,
    pub side: Side,
    pub size: Size,
}

impl Default for PetCfg {
    fn default() -> Self {
        PetCfg { id: String::new(), side: Side::Right, size: Size::M }
    }
}

/// `settings.pet` (`none` / unknown = no pet), `pet.side`, `pet.size`.
pub fn pet_cfg(settings: &Value) -> PetCfg {
    let id = match settings.get("pet").and_then(|v| v.as_str()) {
        Some("none") | Some("") | None => String::new(),
        Some(s) if crate::ui::pets::engine::load(s).is_some() => s.to_string(),
        _ => String::new(),
    };
    let side = if settings.get("pet.side").and_then(|v| v.as_str()) == Some("left") { Side::Left } else { Side::Right };
    let size = Size::parse(settings.get("pet.size").and_then(|v| v.as_str()).unwrap_or("m"));
    PetCfg { id, side, size }
}

/// `characters.js eyesSide()` as the shell's `CharSide`: the pet on the left sends the weather icon (and the eyes) to the right.
pub fn char_side(cfg: &PetCfg) -> CharSide {
    if !cfg.id.is_empty() && cfg.side == Side::Left {
        CharSide::Right
    } else {
        CharSide::Left
    }
}

/// `Pill.reserve(px)`: the room the capsule keeps for the pet (css px at 100 %).
pub fn reserve_px(cfg: &PetCfg) -> f64 {
    if cfg.id.is_empty() {
        return 0.0;
    }
    let (_, h) = box_size(&cfg.id, cfg.size, Place::Cap, 1.0);
    let ar = crate::ui::pets::engine::load(&cfg.id).map_or(1.0, |p| p.ar);
    let slack = if cfg.id == "eyes" { 0.2 } else { 0.0 };
    (h * ar * (1.0 - slack) + 12.0).ceil() as f64
}

/// Pet colours of the edge reactions (`pets.js TINT`) used when the reaction carries none.
pub fn parse_tint(css: &str) -> Option<Color> {
    let h = css.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some(Color::rgb8((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

#[derive(Default)]
pub struct Companions {
    pub cfg: PetCfg,
    pub style: EdgeStyle,
    pet: Option<PetEngine>,
    chars: Option<Characters>,
    state: String,
    tint: Option<Color>,
    reduced: bool,
    light: bool,
    energy_pk: f32,
    last_gaze: Option<(f32, f32)>,
    /// where the face is (window px) for the gaze: set while drawing
    face: Option<(f32, f32)>,
    pub eyes_at: Option<(f32, f32)>,
    /// host clock of the last `tick`: the engines get the REAL time since then (the host hands a nominal frame after an idle gap)
    last_tick: Option<f64>,
    created: bool,
}

impl Companions {
    pub fn new() -> Companions {
        Companions { energy_pk: 0.2, state: "calm".into(), ..Default::default() }
    }

    /// Applies the settings; true when the pet or the character style changed (the capsule must re-measure).
    pub fn apply(&mut self, settings: &Value, style: EdgeStyle) -> bool {
        let cfg = pet_cfg(settings);
        let changed = cfg != self.cfg || style != self.style;
        if changed {
            if let Some(p) = &mut self.pet {
                p.set_pet(if cfg.id.is_empty() { "none" } else { &cfg.id }, cfg.side, cfg.size);
            }
            self.cfg = cfg;
            self.style = style;
        }
        changed
    }

    pub fn wants_gaze(&self) -> bool {
        (!self.cfg.id.is_empty() || self.style == EdgeStyle::Legs) && !self.reduced && self.state != "sleep"
    }

    pub fn set_theme(&mut self, light: bool) {
        self.light = light;
        if let Some(c) = &mut self.chars {
            c.set_light_island(light);
        }
    }

    pub fn set_reduced(&mut self, on: bool) {
        self.reduced = on;
        if let Some(p) = &mut self.pet {
            p.set_reduced(on);
        }
        if let Some(c) = &mut self.chars {
            c.set_reduced(on);
        }
    }

    /// The effective edge reaction (`calm`, `music`, ...) and its colour (css hex or empty).
    pub fn set_reaction(&mut self, name: &str, color: &str) {
        let tint = parse_tint(color);
        if self.state == name && self.tint == tint {
            return; // the view calls this every frame: re-applying the same state would restart the pet (cancels blinks, resets the breath)
        }
        self.state = name.to_string();
        self.tint = tint;
        if let Some(p) = &mut self.pet {
            p.set_state(name, self.tint);
        }
        if let Some(c) = &mut self.chars {
            c.set_state(name, self.tint);
        }
    }

    /// `audio:bands`: average of the first eight bands drives the characters, the whole spectrum the pet's bob.
    pub fn feed_audio(&mut self, bands: &[f64], now_ms: f64) {
        if bands.is_empty() {
            return;
        }
        let avg8 = bands.iter().take(8).sum::<f64>() as f32 / 8.0;
        if let Some(c) = &mut self.chars {
            c.feed_audio(avg8, now_ms);
        }
        let avg = bands.iter().sum::<f64>() as f32 / bands.len() as f32;
        self.energy_pk = avg.max(self.energy_pk * 0.995).max(0.05);
        if let Some(p) = &mut self.pet {
            p.set_energy(Some((avg / self.energy_pk).min(1.0)));
        }
    }

    /// Cursor feed in window px (`None` = unknown): the look of the pet and the eyes.
    pub fn gaze(&mut self, cursor: Option<(f32, f32)>) {
        self.last_gaze = cursor;
        self.apply_gaze();
    }

    fn apply_gaze(&mut self) {
        let from = self.eyes_at.or(self.face);
        match (self.last_gaze, from) {
            (Some(c), Some(f)) => {
                let (dx, dy) = (c.0 - f.0, c.1 - f.1);
                if let Some(p) = &mut self.pet {
                    let (lx, ly) = look_from_vector(dx, dy);
                    p.set_look(lx, ly);
                }
                if let Some(ch) = &mut self.chars {
                    ch.set_gaze(Some((dx, dy)));
                }
            }
            _ => {
                if let Some(p) = &mut self.pet {
                    p.clear_look();
                }
                if let Some(ch) = &mut self.chars {
                    ch.set_gaze(None);
                }
            }
        }
    }

    pub fn poke(&mut self) {
        if let Some(p) = &mut self.pet {
            p.poke();
        }
    }

    /// Advances the pet and the characters; true while something moves (the view keeps asking for frames).
    pub fn tick(&mut self, now_ms: f64) -> bool {
        let dt_ms = match self.last_tick {
            Some(t) => (now_ms - t).clamp(0.0, 5000.0) as f32,
            None => 16.0,
        };
        self.last_tick = Some(now_ms);
        let mut busy = false;
        if let Some(p) = &mut self.pet {
            busy |= p.tick(dt_ms) == NeedsFrame::Yes;
        }
        if let Some(c) = &mut self.chars {
            if !self.reduced && matches!(self.style, EdgeStyle::Tail | EdgeStyle::Legs) {
                c.tick(dt_ms);
                busy = true;
            }
        }
        busy
    }

    /// Ms until the pet's next idle behaviour (it sleeps in between).
    pub fn next_wake_ms(&self) -> Option<f32> {
        self.pet.as_ref().and_then(|p| p.next_wake_ms())
    }

    /// Ms (host clock) until the pet has something to do again, counted from `now_ms`; `None` = no pet / reduced motion.
    pub fn wake_in(&self, now_ms: f64) -> Option<f64> {
        let behind = self.last_tick.map_or(0.0, |t| (now_ms - t).max(0.0));
        self.next_wake_ms().map(|w| (w as f64 - behind).max(0.0))
    }

    /// True once after an engine was created (in `draw`): the view must tick and draw again to start it.
    pub fn take_created(&mut self) -> bool {
        std::mem::take(&mut self.created)
    }

    fn ensure(&mut self, cv: &Canvas) {
        if self.pet.is_none() {
            if let Ok(mut p) = PetEngine::with_context(cv.dc) {
                p.set_reduced(self.reduced);
                if !self.cfg.id.is_empty() {
                    p.set_pet(&self.cfg.id, self.cfg.side, self.cfg.size);
                }
                p.set_state(&self.state, self.tint);
                self.pet = Some(p);
                self.created = true;
            }
        }
        if self.chars.is_none() && matches!(self.style, EdgeStyle::Tail | EdgeStyle::Legs) {
            if let Ok(mut c) = Characters::new(cv.dc) {
                c.set_reduced(self.reduced);
                c.set_light_island(self.light);
                c.set_state(&self.state, self.tint);
                self.chars = Some(c);
                self.created = true;
            }
        }
    }

    /// The device was lost: engines hold Direct2D resources of the old one.
    pub fn drop_device_resources(&mut self) {
        self.pet = None;
        self.chars = None;
    }

    /// Where the pet box is in the capsule: on the floor, 9 px from the end (`pets.css`), for island size `s`.
    pub fn cap_box(&self, body: Rect, s: f32) -> Option<Rect> {
        if self.cfg.id.is_empty() {
            return None;
        }
        let (w, h) = box_size(&self.cfg.id, self.cfg.size, Place::Cap, s);
        let h = h.min(body.h);
        let x = if self.cfg.side == Side::Left { body.x + 9.0 * s } else { body.right() - 9.0 * s - w };
        Some(Rect::new(x, body.bottom() - 1.5 * s - h, w, h))
    }

    /// Draws the pet into `r` at opacity `alpha`.
    pub fn draw_pet(&mut self, cv: &Canvas, r: Rect, alpha: f32) {
        self.ensure(cv);
        let (cx, cy) = (r.cx(), r.y + r.h * eye_line_y(&self.cfg.id));
        self.face = Some((cx, cy));
        if let Some(p) = &mut self.pet {
            if alpha < 0.999 {
                cv.with_opacity(alpha, |cv| {
                    let _ = p.draw(cv.dc, r, cv.scale);
                });
            } else {
                let _ = p.draw(cv.dc, r, cv.scale);
            }
        }
        self.apply_gaze();
    }

    /// Tail at the bottom-right corner arc (`characters.js place`), `body` = the island body in window px, `rb` its bottom radius.
    pub fn draw_tail(&mut self, cv: &Canvas, body: Rect, rb: f32, s: f32) {
        self.ensure(cv);
        let a = 35.0f32.to_radians();
        let r = rb.clamp(1.0, body.w.min(body.h) / 2.0);
        let (x, y) = (body.right() - r + (r - 0.6) * a.cos(), body.bottom() - r + (r - 0.6) * a.sin());
        if let Some(c) = &self.chars {
            c.draw_tail(cv.dc, x, y, s);
        }
    }

    /// Legs under the eyes and the eyes on the text line. `eyes` = (x, y) of the middle between the eyes in window px.
    pub fn draw_legs_eyes(&mut self, cv: &Canvas, body: Rect, eyes: (f32, f32), s: f32) {
        self.ensure(cv);
        self.eyes_at = Some(eyes);
        let lx = eyes.0.clamp(body.x + 14.0 * s, (body.right() - 14.0 * s).max(body.x + 14.0 * s));
        if let Some(c) = &mut self.chars {
            c.draw_legs(cv.dc, lx, body.bottom() - 0.2, s);
            c.draw_eyes(cv.dc, eyes.0, eyes.1, s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn settings_pick_the_pet_side_and_size() {
        let c = pet_cfg(&json!({"pet": "cat", "pet.side": "left", "pet.size": "l"}));
        assert_eq!((c.id.as_str(), c.side, c.size), ("cat", Side::Left, Size::L));
        assert!(pet_cfg(&json!({"pet": "none"})).id.is_empty());
        assert!(pet_cfg(&json!({"pet": "no-such-pet"})).id.is_empty());
        assert!(pet_cfg(&json!({})).id.is_empty());
    }

    #[test]
    fn the_weather_icon_goes_opposite_the_pet() {
        let left = pet_cfg(&json!({"pet": "cat", "pet.side": "left"}));
        let right = pet_cfg(&json!({"pet": "cat", "pet.side": "right"}));
        assert_eq!(char_side(&left), CharSide::Right);
        assert_eq!(char_side(&right), CharSide::Left);
        assert_eq!(char_side(&PetCfg::default()), CharSide::Left);
    }

    #[test]
    fn the_capsule_reserves_room_for_the_pet_only_when_there_is_one() {
        assert_eq!(reserve_px(&PetCfg::default()), 0.0);
        let cat = pet_cfg(&json!({"pet": "cat"}));
        let px = reserve_px(&cat);
        assert!(px > 12.0 && px < 120.0, "{px}");
        let mut l = cat.clone();
        l.size = Size::L;
        assert!(reserve_px(&l) > px);
    }

    #[test]
    fn the_pet_stands_on_the_floor_nine_px_from_its_end() {
        let mut c = Companions::new();
        c.apply(&json!({"pet": "cat", "pet.side": "right", "pet.size": "m"}), EdgeStyle::Liquid);
        let body = Rect::new(285.0, 8.0, 150.0, 30.0);
        let r = c.cap_box(body, 1.0).unwrap();
        assert!((r.right() - (435.0 - 9.0)).abs() < 1e-3);
        assert!((r.bottom() - (38.0 - 1.5)).abs() < 1e-3);
        c.apply(&json!({"pet": "cat", "pet.side": "left"}), EdgeStyle::Liquid);
        assert!((c.cap_box(body, 1.0).unwrap().x - 294.0).abs() < 1e-3);
        assert_eq!(parse_tint("#d8836a").map(|c| (c.r * 255.0).round() as u8), Some(0xd8));
        assert!(parse_tint("var(--accent)").is_none());
    }

    fn shot(c: &mut Companions, h: &glasskit::testkit::Harness) -> glasskit::testkit::Image {
        h.render(60, 40, 1.0, Color::rgba(0.0, 0.0, 0.0, 1.0), |cv| c.draw_pet(cv, Rect::new(5.0, 5.0, 50.0, 30.0), 1.0))
    }

    fn eyes() -> (Companions, glasskit::testkit::Harness) {
        let h = glasskit::testkit::Harness::new();
        let mut c = Companions::new();
        c.apply(&json!({"pet": "eyes"}), EdgeStyle::Liquid);
        let _ = shot(&mut c, &h); // creates the engine
        assert!(c.take_created(), "the first draw reports the new engine so the view can start it");
        (c, h)
    }

    #[test]
    fn the_pet_gets_real_elapsed_time_after_an_idle_gap_and_wakes_on_schedule() {
        let (mut c, h) = eyes();
        c.tick(0.0);
        let first = shot(&mut c, &h);
        let wake = c.wake_in(0.0).expect("an idle pet announces its next behaviour (the view arms its alarm with it)");
        assert!(wake > 0.0 && wake < 7500.0, "{wake}");
        // the host sleeps; the alarm fires 8 s later and the next frame carries a nominal 16 ms: the engine must still be 8 s older
        let mut moved = false;
        for k in 0..40 {
            c.tick(8000.0 + k as f64 * 16.0);
            if shot(&mut c, &h).diff(&first, 2) > 0 {
                moved = true;
                break;
            }
        }
        assert!(moved, "after 8 s of real time the pet blinked / glanced");
        assert!(c.wake_in(8000.0 + 16.0 * 40.0).is_some());
    }

    #[test]
    fn the_gaze_moves_the_eyes_and_clears() {
        let (mut c, h) = eyes();
        c.tick(0.0);
        c.gaze(Some((-500.0, 20.0)));
        for k in 1..30 {
            c.tick(k as f64 * 16.0);
        }
        let left = shot(&mut c, &h);
        c.gaze(Some((900.0, 20.0)));
        for k in 30..60 {
            c.tick(k as f64 * 16.0);
        }
        let right = shot(&mut c, &h);
        assert!(left.diff(&right, 2) > 0);
    }

    #[test]
    fn a_calm_pet_asks_for_frames_while_it_breathes() {
        let h = glasskit::testkit::Harness::new();
        for id in crate::ui::pets::engine::catalogue().iter().map(|p| p.id) {
            let mut c = Companions::new();
            c.apply(&json!({ "pet": id }), EdgeStyle::Liquid);
            let _ = shot(&mut c, &h);
            let busy = (0..120).filter(|k| c.tick(*k as f64 * 16.0)).count();
            assert!(busy > 100, "{id}: the breathing rig must keep frames coming ({busy}/120)");
        }
    }

    #[test]
    fn re_applying_the_same_reaction_every_frame_does_not_freeze_the_pet() {
        // the view feeds the effective reaction on every step; the engine must only see a CHANGE
        let (mut c, h) = eyes();
        c.set_reaction("calm", "");
        c.tick(0.0);
        let first = shot(&mut c, &h);
        let mut moved = false;
        for k in 1..800 {
            c.set_reaction("calm", "");
            c.tick(k as f64 * 16.0);
            if shot(&mut c, &h).diff(&first, 2) > 0 {
                moved = true;
                break;
            }
        }
        assert!(moved, "12 s of frames with the reaction re-applied each frame: the pet never moved");
        c.set_reaction("music", "#d8836a");
        assert_eq!(c.state, "music", "a real change still goes through");
    }
}
