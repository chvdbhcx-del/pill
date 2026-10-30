//! The JS-driven behaviour of the pets whose `tick` / `poke` hooks CSS cannot express: the hamster's wheel, the koi and the snail crossing the
//! box and turning, the swarm's music wave and poke burst. Ported line by line from src/pets-creatures.js and src/pets-chars.js (the same
//! constants and filters); the random jitter uses the engine's seeded generator.
use super::interp::{self, Mat};
use super::model::{Pet, Style};
use super::rng::Rng;
use super::sim::Sim;

pub struct Walker {
    flip: usize,
    props: usize,
    x: f32,
    dir: f32,
    sx: f32,
    sy: f32,
    kick: f32,
    d: f32,
    ox: f32,
    cx: f32,
    cy: f32,
    koi: bool,
}

pub enum Hook {
    None,
    Hamster { wheel: usize, a: f32, w: f32 },
    Walker(Walker),
    Swarm { dots: Vec<usize>, boost: f64, energy: Option<f32>, burst_until: f64, pending: bool },
}

fn find(pet: &Pet, class: &str) -> Option<usize> {
    pet.nodes.iter().position(|n| n.class.split_whitespace().any(|c| c == class))
}

impl Hook {
    pub fn for_pet(pet: &Pet) -> Hook {
        match pet.id.as_str() {
            "hamster" => find(pet, "hm-wheel").map(|wheel| Hook::Hamster { wheel, a: 0.0, w: 0.0 }).unwrap_or(Hook::None),
            "koi" => match (find(pet, "k-flip"), find(pet, "k-props")) {
                (Some(flip), Some(props)) => Hook::Walker(Walker { flip, props, x: 10.0, dir: 1.0, sx: 1.0, sy: 1.0, kick: 0.0, d: 43.0, ox: 4.5, cx: 22.0, cy: 29.0, koi: true }),
                _ => Hook::None,
            },
            "snail" => match (find(pet, "sn-flip"), find(pet, "sn-props")) {
                (Some(flip), Some(props)) => Hook::Walker(Walker { flip, props, x: 6.0, dir: 1.0, sx: 1.0, sy: 1.0, kick: 0.0, d: 40.0, ox: 4.0, cx: 22.0, cy: 0.0, koi: false }),
                _ => Hook::None,
            },
            "swarm" => {
                let dots: Vec<usize> = pet.nodes.iter().enumerate().filter(|(_, n)| n.class.split_whitespace().any(|c| c == "sw")).map(|(i, _)| i).collect();
                Hook::Swarm { dots, boost: -1e9, energy: None, burst_until: -1.0, pending: false }
            }
            _ => Hook::None,
        }
    }

    pub fn energy_changed(&mut self, e: Option<f32>) {
        if let Hook::Swarm { energy, .. } = self {
            *energy = e;
        }
    }

    pub fn poked(&mut self, now: f64, _rng: &mut Rng, state: usize) {
        match self {
            Hook::Hamster { w, .. } => *w += 320.0,
            Hook::Walker(k) if k.koi => k.kick = 0.8,
            Hook::Swarm { boost, burst_until, pending, .. } => {
                if state == 1 {
                    *boost = now;
                } else {
                    *burst_until = now + 380.0;
                    *pending = true;
                }
            }
            _ => {}
        }
    }
}

fn attr_transform(m: Mat) -> Style {
    // an SVG `transform` attribute has its origin at 0 0: a decomposed transform without pivot
    Style { tf: interp::to_dec(m), ..Style::default() }
}

fn set_attr_transform(sim: &mut Sim, i: usize, m: Mat) {
    let s = attr_transform(m);
    let c = &mut sim.cur_mut()[i];
    c.tf = s.tf;
    c.po = [0.0, 0.0];
}

pub fn state_changed(sim: &mut Sim) {
    // the hamster's wheel coasts and the swarm regroups on a state change: nothing to reset, tick() reads the new state
    if let Hook::Swarm { burst_until, pending, .. } = &mut sim.hook {
        *burst_until = -1.0;
        *pending = false;
    }
}

/// returns true while the hook keeps moving something
pub fn tick(sim: &mut Sim, dt_ms: f32) -> bool {
    if sim.reduced {
        return false;
    }
    let dt = (dt_ms / 1000.0).clamp(0.0, 0.1);
    let st = sim.state;
    let now = sim.now;
    let mut hook = std::mem::replace(&mut sim.hook, Hook::None);
    let busy = match &mut hook {
        Hook::None => false,
        Hook::Hamster { wheel, a, w } => {
            const V: [f32; 7] = [55.0, 230.0, 115.0, 0.0, 520.0, 75.0, 0.0];
            let tau = if st == 3 { 0.8 } else if st == 6 { 0.5 } else { 0.35 };
            *w += (V[st] - *w) * (1.0 - (-dt / tau).exp());
            *a = (*a + *w * dt) % 360.0;
            let m = interp::mul(interp::mul(interp::translate(24.0, 22.0), interp::rotate(*a)), interp::translate(-24.0, -22.0));
            set_attr_transform(sim, *wheel, m);
            true
        }
        Hook::Walker(k) => {
            walker(sim, k, dt, st, now);
            true
        }
        Hook::Swarm { dots, boost, energy, burst_until, pending } => swarm(sim, dots, *boost, *energy, burst_until, pending, st, now),
    };
    sim.hook = hook;
    busy
}

fn clamp(v: f32, a: f32, b: f32) -> f32 {
    v.max(a).min(b)
}

fn walker(sim: &mut Sim, k: &mut Walker, dt: f32, st: usize, now: f64) {
    let t = now as f32;
    let (flip, props) = (k.flip, k.props);
    if k.koi {
        const KO_V: [f32; 7] = [5.0, 11.0, 3.0, 11.0, 0.0, 6.0, 0.0];
        let mut v = KO_V[st];
        if st == 4 {
            v = if ((t / 1000.0) % 1.3) < 0.3 { 72.0 } else { 3.0 }; // quick darts with short pauses
        }
        if k.kick > 0.0 {
            v = v.max(5.0) * 3.0;
            k.kick -= dt;
        }
        if st == 6 {
            let d = k.d / 2.0 - k.x;
            k.x += d.signum() * d.abs().min(8.0 * dt); // drifts to the middle, then floats
        } else {
            k.x += k.dir * v * dt;
            if k.x >= k.d {
                k.x = k.d;
                k.dir = -1.0;
            } else if k.x <= 0.0 {
                k.x = 0.0;
                k.dir = 1.0;
            }
        }
        k.sx += clamp(k.dir - k.sx, -6.0 * dt, 6.0 * dt); // the turn: the body narrows through zero and widens the other way
        k.sy += clamp((if st == 6 { -1.0 } else { 1.0 }) - k.sy, -4.0 * dt, 4.0 * dt); // asleep: belly up
        let sx = if k.sx.abs() < 0.06 { 0.06 * if k.dir != 0.0 { k.dir } else { 1.0 } } else { k.sx };
        let sy = if k.sy.abs() < 0.06 { 0.06 } else { k.sy };
        let (per, amp) = if st == 1 { (220.0, 1.7) } else if st == 6 { (800.0, 0.9) } else { (800.0, 1.1) };
        let bob = (t / per).sin() * amp;
        let m = interp::mul(interp::mul(interp::translate(k.x + k.ox + k.cx, k.cy + bob), interp::scale(sx, sy)), interp::translate(-k.cx, -k.cy));
        set_attr_transform(sim, flip, m);
        set_attr_transform(sim, props, interp::translate(k.x + k.ox, bob));
    } else {
        const SN_V: [f32; 7] = [2.2, 1.6, 1.0, 5.0, 0.0, 2.2, 0.0];
        k.x += k.dir * SN_V[st] * dt;
        if k.x >= k.d {
            k.x = k.d;
            k.dir = -1.0;
        } else if k.x <= 0.0 {
            k.x = 0.0;
            k.dir = 1.0;
        }
        k.sx += clamp(k.dir - k.sx, -4.0 * dt, 4.0 * dt);
        let sx = if k.sx.abs() < 0.06 { 0.06 * if k.dir != 0.0 { k.dir } else { 1.0 } } else { k.sx };
        let m = interp::mul(interp::mul(interp::translate(k.x + k.ox + k.cx, 0.0), interp::scale(sx, 1.0)), interp::translate(-k.cx, 0.0));
        set_attr_transform(sim, flip, m);
        set_attr_transform(sim, props, interp::translate(k.x + k.ox, 0.0));
    }
}

#[allow(clippy::too_many_arguments)]
fn swarm(sim: &mut Sim, dots: &[usize], boost: f64, energy: Option<f32>, burst_until: &mut f64, pending: &mut bool, st: usize, now: f64) -> bool {
    const CY: f32 = 26.0;
    let n = dots.len();
    if n < 2 {
        return false;
    }
    if st == 1 {
        // music: the wave runs on JS; its amplitude follows the audio energy
        let amp = 3.5 + energy.unwrap_or(0.35) * 11.0 + ((1.0 - (now - boost) / 500.0).max(0.0) as f32) * 5.0;
        let t = now as f32;
        for (i, d) in dots.iter().enumerate() {
            let c = &mut sim.cur_mut()[*d];
            c.tl = [5.0 + i as f32 * 38.0 / (n - 1) as f32, CY + (t / 170.0 - i as f32 * 0.7).sin() * amp];
        }
        return true;
    }
    if *burst_until > 0.0 {
        // poke: burst outwards (jitter +-9 px, eased by the dots' own translate transition), then regroup after 380 ms
        if *pending {
            *pending = false;
            let mut r = std::mem::replace(&mut sim.rng, Rng::new(1));
            for d in dots {
                let t = sim.target_style(*d);
                let mut s = t;
                s.tl = [t.tl[0] + (r.f32() - 0.5) * 18.0, t.tl[1] + (r.f32() - 0.5) * 18.0];
                sim.override_target(*d, s);
            }
            sim.rng = r;
        } else if now >= *burst_until {
            *burst_until = -1.0;
            sim.restore_targets(dots);
        }
        return true;
    }
    false
}
