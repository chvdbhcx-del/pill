//! The public face of the native pet engine: `PetEngine` (one pet on screen, or one preview card), the pet catalogue and the box geometry the
//! shell needs to place a pet. All the behaviour lives in `sim.rs` (pure) and `render.rs` (Direct2D).
use super::model::{Pet, Tag, STATES};
use super::render::Renderer;
use super::sim::Sim;
use glasskit::{Color, Gfx, Rect};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// settings `pet.size`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    S,
    M,
    L,
}

impl Size {
    pub fn parse(s: &str) -> Size {
        match s {
            "s" => Size::S,
            "l" => Size::L,
            _ => Size::M,
        }
    }
}

/// where the pet stands: the collapsed capsule, the header gap of the open panel, or the settings preview stage
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Cap,
    Head,
    Stage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeedsFrame {
    /// something moves: draw the next frame at the display refresh rate
    Yes,
    /// nothing moves until an input arrives or `PetEngine::next_wake_ms` elapses
    No,
}

#[derive(Clone, Debug)]
pub struct PetInfo {
    pub id: &'static str,
    pub group: String,
    pub name_ru: String,
    pub name_en: String,
    /// box width / height
    pub ar: f32,
}

static CACHE: OnceLock<Mutex<HashMap<String, Arc<Pet>>>> = OnceLock::new();

/// The parsed pet (cached; parsing a pet takes well under a millisecond).
pub fn load(id: &str) -> Option<Arc<Pet>> {
    let m = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut g = m.lock().ok()?;
    if let Some(p) = g.get(id) {
        return Some(Arc::clone(p));
    }
    let p = Arc::new(Pet::parse(super::model::raw(id)?).ok()?);
    g.insert(id.to_string(), Arc::clone(&p));
    Some(p)
}

/// All pets in registry order (the picker lists them by `group`).
pub fn catalogue() -> Vec<PetInfo> {
    super::model::ids()
        .filter_map(|id| {
            let p = load(id)?;
            Some(PetInfo { id, group: p.group.clone(), name_ru: p.name_ru.clone(), name_en: p.name_en.clone(), ar: p.ar })
        })
        .collect()
}

pub fn state_index(name: &str) -> usize {
    STATES.iter().position(|s| *s == name).unwrap_or(0)
}

/// Pet box (height x aspect ratio) in logical px for the island size factor `scale` (the `--s` of pets.css).
pub fn box_size(id: &str, size: Size, place: Place, scale: f32) -> (f32, f32) {
    let ar = load(id).map(|p| p.ar).unwrap_or(1.0);
    let is8 = id == "claude8";
    let h = match place {
        Place::Cap => {
            let (n, u) = if is8 {
                match size {
                    Size::S => (14.0, 14.0),
                    Size::M => (21.0, 7.0),
                    Size::L => (28.0, 14.0),
                }
            } else {
                (
                    match size {
                        Size::S => 20.0,
                        Size::M => 27.0,
                        Size::L => 30.0,
                    },
                    0.0,
                )
            };
            if is8 {
                ((n * scale / u).round() * u).max(14.0)
            } else {
                n * scale
            }
        }
        Place::Head => {
            let v = if is8 {
                match size {
                    Size::S => 14.0,
                    Size::M => 28.0,
                    Size::L => 42.0,
                }
            } else {
                match size {
                    Size::S => 22.0,
                    Size::M => 28.0,
                    Size::L => 32.0,
                }
            };
            v * scale
        }
        Place::Stage => {
            if is8 {
                match size {
                    Size::S => 28.0,
                    Size::M => 42.0,
                    Size::L => 56.0,
                }
            } else {
                (100.0 / ar).min(48.0)
            }
        }
    };
    (h * ar, h)
}

/// Where the face is inside the pet box as fractions (x, y) of (width, height); y is the eye line the shell aligns with the text line.
/// The Eyes pet is already lifted onto the text line inside its artwork, so this follows the exported art.
pub fn face_anchor(id: &str) -> (f32, f32) {
    let Some(p) = load(id) else { return (0.5, 0.5) };
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (i, n) in p.nodes.iter().enumerate() {
        let is_eye = n.class.split_whitespace().any(|c| matches!(c, "eyes" | "eye" | "ey-all" | "ey-l" | "ey-r" | "e-open"));
        let Some(b) = n.bb else { continue };
        if !is_eye || n.tag == Tag::Svg {
            continue;
        }
        // add the plain translations of the ancestors (the lift of the Eyes pet is a `translate(0 -5.6)` group)
        let (mut tx, mut ty) = (0.0, 0.0);
        let mut a = n.parent;
        while a >= 0 {
            let s = &p.base[a as usize];
            tx += s.tf[0];
            ty += s.tf[1];
            a = p.nodes[a as usize].parent;
        }
        let _ = i;
        x0 = x0.min(b[0] + tx);
        x1 = x1.max(b[0] + b[2] + tx);
        y0 = y0.min(b[1] + ty);
        y1 = y1.max(b[1] + b[3] + ty);
    }
    if x0 > x1 {
        return (0.5, 0.5);
    }
    ((x0 + x1) / 2.0 / p.w, (y0 + y1) / 2.0 / p.h)
}

/// `eye_line_y` of the shell contract: the fraction of the box height at which the eyes sit.
pub fn eye_line_y(id: &str) -> f32 {
    face_anchor(id).1
}

pub struct PetEngine {
    rend: Renderer,
    sim: Option<Sim>,
    seed: u64,
    reduced: bool,
    side: Side,
    size: Size,
    state: usize,
    tint: Option<Color>,
    idle: bool,
}

impl PetEngine {
    pub fn new(gfx: &Gfx) -> Result<PetEngine> {
        Ok(PetEngine { rend: Renderer::new(&gfx.dc)?, sim: None, seed: 0x5EED, reduced: false, side: Side::Right, size: Size::M, state: 0, tint: None, idle: true })
    }

    /// Same, on an existing Direct2D context.
    pub fn with_context(dc: &ID2D1DeviceContext) -> Result<PetEngine> {
        Ok(PetEngine { rend: Renderer::new(dc)?, sim: None, seed: 0x5EED, reduced: false, side: Side::Right, size: Size::M, state: 0, tint: None, idle: true })
    }

    /// Seed of the idle schedule (blink / glance / twitch); the same seed and calls replay the same pet.
    pub fn set_seed(&mut self, seed: u64) {
        self.seed = seed;
    }

    /// Switch to pet `id` (a new pet starts in its current state). False for an unknown id ("none" removes the pet).
    pub fn set_pet(&mut self, id: &str, side: Side, size: Size) -> bool {
        self.side = side;
        self.size = size;
        if let Some(s) = &mut self.sim {
            if s.pet.id == id {
                s.set_side(side == Side::Left);
                return true;
            }
        }
        match load(id) {
            Some(p) => {
                let mut sim = Sim::new(p, side == Side::Left, self.seed, self.state, self.tint.map(|c| [c.r, c.g, c.b]));
                sim.set_idle(self.idle);
                sim.set_reduced(self.reduced);
                self.sim = Some(sim);
                true
            }
            None => {
                self.sim = None;
                false
            }
        }
    }

    pub fn pet_id(&self) -> Option<&str> {
        self.sim.as_ref().map(|s| s.pet.id.as_str())
    }

    pub fn side(&self) -> Side {
        self.side
    }
    pub fn size(&self) -> Size {
        self.size
    }

    /// `name` = calm | music | thinking | done | attention | file | sleep; `tint` = the reaction's own colour (None: the edge colour of that state).
    pub fn set_state(&mut self, name: &str, tint: Option<Color>) {
        self.state = state_index(name);
        self.tint = tint;
        if let Some(s) = &mut self.sim {
            s.set_state(self.state, tint.map(|c| [c.r, c.g, c.b]));
        }
    }

    pub fn state(&self) -> &'static str {
        STATES[self.state]
    }

    /// Where the pet looks, -1..1 each (cursor direction relative to the pet). `clear_look` = straight ahead.
    pub fn set_look(&mut self, dx: f32, dy: f32) {
        if let Some(s) = &mut self.sim {
            s.set_look(dx, dy);
        }
    }

    pub fn clear_look(&mut self) {
        if let Some(s) = &mut self.sim {
            s.clear_look();
        }
    }

    /// Click petting (hearts and a squeeze); false when ignored (at most once a second, never under reduced motion).
    pub fn poke(&mut self) -> bool {
        self.sim.as_mut().is_some_and(|s| s.poke())
    }

    /// Music loudness 0..1 (None = the steady beat).
    pub fn set_energy(&mut self, e: Option<f32>) {
        if let Some(s) = &mut self.sim {
            s.set_energy(e);
        }
    }

    /// Reduce motion: static poses, no animation, no transition, no idle behaviour.
    pub fn set_reduced(&mut self, on: bool) {
        self.reduced = on;
        if let Some(s) = &mut self.sim {
            s.set_reduced(on);
        }
    }

    pub fn set_idle(&mut self, on: bool) {
        self.idle = on;
        if let Some(s) = &mut self.sim {
            s.set_idle(on);
        }
    }

    /// Advance by `dt_ms`. Call once per frame; keep drawing frames while it says `Yes`.
    pub fn tick(&mut self, dt_ms: f32) -> NeedsFrame {
        match self.sim.as_mut() {
            Some(s) => {
                if s.tick(dt_ms) {
                    NeedsFrame::Yes
                } else {
                    NeedsFrame::No
                }
            }
            None => NeedsFrame::No,
        }
    }

    /// Milliseconds until the next idle behaviour starts (relevant when `tick` returned `No`).
    pub fn next_wake_ms(&self) -> Option<f32> {
        self.sim.as_ref().and_then(|s| s.next_wake_ms())
    }

    /// Draw into `rect` (logical px, the pet box of `box_size`); `scale` = device px per logical px.
    pub fn draw(&mut self, dc: &ID2D1DeviceContext, rect: Rect, scale: f32) -> Result<()> {
        match &self.sim {
            Some(s) => self.rend.draw(dc, s, rect, scale),
            None => Ok(()),
        }
    }

    pub fn sim(&self) -> Option<&Sim> {
        self.sim.as_ref()
    }

    pub fn sim_mut(&mut self) -> Option<&mut Sim> {
        self.sim.as_mut()
    }
}
