//! Pure simulation of one pet (no Direct2D): which context is active (state x side x idle flag x reduced motion), the static styles with CSS
//! transitions, the CSS animations, look / energy layers, the tint fade and the idle schedule of pets.js. `render.rs` only reads the result.
use super::ease::Ease;
use super::interp::{self, lerp, lerp4, Mat, TfList, DEC_IDENT, IDENT};
use super::model::*;
use super::path::{self, Path};
use super::rng::Rng;
use std::sync::Arc;

// ---------------------------------------------------------------- tint ("painting") of pets.js: --pet-accent / --pet-k, both fading in 0.35 s

/// Island-edge colours of the reactions (pets.js TINT); calm has none.
pub const TINT: [Option<[f32; 3]>; 7] = [
    None,
    Some([0xd8 as f32 / 255.0, 0x83 as f32 / 255.0, 0x6a as f32 / 255.0]),
    Some([0x9e as f32 / 255.0, 0x8f as f32 / 255.0, 0xe8 as f32 / 255.0]),
    Some([0x7f as f32 / 255.0, 0xae as f32 / 255.0, 0x45 as f32 / 255.0]),
    Some([0xe8 as f32 / 255.0, 0xa0 as f32 / 255.0, 0x40 as f32 / 255.0]),
    Some([0x5a as f32 / 255.0, 0x78 as f32 / 255.0, 0xa8 as f32 / 255.0]),
    Some([0x7d as f32 / 255.0, 0x8c as f32 / 255.0, 0xab as f32 / 255.0]),
];

#[derive(Clone, Copy, Debug)]
pub struct Tint {
    pub accent: [f32; 3],
    pub k: f32,
    from: ([f32; 3], f32),
    to: ([f32; 3], f32),
    t0: f64,
    busy: bool,
}

impl Tint {
    fn new() -> Tint {
        Tint { accent: [0.49, 0.55, 0.67], k: 0.0, from: ([0.0; 3], 0.0), to: ([0.0; 3], 0.0), t0: 0.0, busy: false }
    }
    fn set(&mut self, accent: Option<[f32; 3]>, now: f64, instant: bool) {
        let (a, k) = match accent {
            Some(c) => (c, 1.0),
            None => (self.accent, 0.0), // the accent is kept so it fades out instead of flipping
        };
        if instant {
            self.accent = a;
            self.k = k;
            self.busy = false;
            return;
        }
        self.from = (self.accent, self.k);
        self.to = (a, k);
        self.t0 = now;
        self.busy = true;
    }
    fn step(&mut self, now: f64, ease: &Ease) {
        if !self.busy {
            return;
        }
        let x = ((now - self.t0) / 350.0).clamp(0.0, 1.0) as f32;
        let e = ease.at(x);
        for i in 0..3 {
            self.accent[i] = lerp(self.from.0[i], self.to.0[i], e);
        }
        self.k = lerp(self.from.1, self.to.1, e);
        if x >= 1.0 {
            self.busy = false;
        }
    }
}

// ---------------------------------------------------------------- per node dynamic data

const NG: usize = 6; // transition groups: transform, translate, rotate, scale, opacity, fill+stroke

#[derive(Clone, Copy, Default)]
struct Slot {
    active: bool,
    t0: f64,
    dur: f32,
    delay: f32,
    ease: u16,
    from: [f32; 10],
}

fn gv(g: usize, s: &Style) -> [f32; 10] {
    let mut v = [0.0; 10];
    match g {
        0 => v[..6].copy_from_slice(&s.tf),
        1 => v[..2].copy_from_slice(&s.tl),
        2 => v[0] = s.ro,
        3 => v[..2].copy_from_slice(&s.sc),
        4 => v[0] = s.o,
        _ => {
            v[..4].copy_from_slice(&s.fi.c);
            v[4] = s.fi.p;
            v[5..9].copy_from_slice(&s.st.c);
            v[9] = s.st.p;
        }
    }
    v
}

fn set_gv(g: usize, s: &mut Style, v: &[f32; 10]) {
    match g {
        0 => s.tf.copy_from_slice(&v[..6]),
        1 => s.tl.copy_from_slice(&v[..2]),
        2 => s.ro = v[0],
        3 => s.sc.copy_from_slice(&v[..2]),
        4 => s.o = v[0],
        _ => {
            s.fi.c.copy_from_slice(&v[..4]);
            s.fi.p = v[4];
            s.st.c.copy_from_slice(&v[5..9]);
            s.st.p = v[9];
        }
    }
}

/// look / energy deltas work on these numeric fields: tf(6) tl(2) ro sc(2) o
fn lk(s: &Style) -> [f32; 12] {
    let mut v = [0.0; 12];
    v[..6].copy_from_slice(&s.tf);
    v[6..8].copy_from_slice(&s.tl);
    v[8] = s.ro;
    v[9..11].copy_from_slice(&s.sc);
    v[11] = s.o;
    v
}

fn add_lk(s: &mut Style, d: &[f32; 12], k: f32) {
    for i in 0..6 {
        s.tf[i] += d[i] * k;
    }
    s.tl[0] += d[6] * k;
    s.tl[1] += d[7] * k;
    s.ro += d[8] * k;
    s.sc[0] += d[9] * k;
    s.sc[1] += d[10] * k;
    s.o += d[11] * k;
}

struct LookNode {
    node: u32,
    /// (+x, -x, +y, -y) deltas of the numeric fields for a unit look
    d: [[f32; 12]; 4],
}

struct Run {
    def: AnimDef,
    t0: f64,
    /// var()-driven keyframes (energy): the keyframe set for each of the five energy samples
    alt: Option<[u32; 5]>,
}

#[derive(Default)]
pub struct Ov {
    pub tf: Option<TfList>,
    pub ro: Option<f32>,
    pub o: Option<f32>,
    pub fi: Option<[f32; 4]>,
    pub st: Option<[f32; 4]>,
    pub vis: Option<bool>,
    pub dof: Option<f32>,
    pub da: Option<Dash>,
    pub sw: Option<f32>,
    pub has_d: bool,
    pub d: Path,
}

impl Ov {
    fn clear(&mut self) {
        self.tf = None;
        self.ro = None;
        self.o = None;
        self.fi = None;
        self.st = None;
        self.vis = None;
        self.dof = None;
        self.da = None;
        self.sw = None;
        self.has_d = false;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Blink,
    Twitch,
    Hop,
}

impl Act {
    fn key(self) -> &'static str {
        match self {
            Act::Blink => "blink",
            Act::Twitch => "twitch",
            Act::Hop => "hop",
        }
    }
}

/// pets whose look / energy the web version quantised in JS (whole cells): (x threshold, y threshold, energy steps)
fn quant(id: &str) -> Option<(f32, f32, f32)> {
    match id {
        "pixel" => Some((0.3, 0.5, 2.0)),
        "claude8" => Some((0.35, f32::INFINITY, 2.0)),
        _ => None,
    }
}

/// idle twitch choices of pets.js TWITCH (default: twitch only)
fn twitch_list(id: &str) -> &'static [Act] {
    match id {
        "bunny" => &[Act::Twitch, Act::Hop],
        _ => &[Act::Twitch],
    }
}

pub struct Sim {
    pub pet: Arc<Pet>,
    pub left: bool,
    pub state: usize,
    pub reduced: bool,
    pub energy: Option<f32>,
    pub look: [f32; 2],
    aimed: bool,
    pub now: f64,
    pub tint: Tint,
    tint_ease: Ease,
    tbase: Vec<Style>,
    target: Vec<Style>,
    pub(crate) cur: Vec<Style>,
    trs: Vec<Vec<TransDef>>,
    slots: Vec<[Slot; NG]>,
    tr_active: Vec<u32>,
    runs: Vec<Vec<Run>>,
    pub(crate) ov: Vec<Ov>,
    animated: Vec<u32>,
    looks: Vec<LookNode>,
    en_nodes: Vec<(u32, [Style; 5])>,
    /// energy-driven `d` (lens iris): per node, rebuilt when the energy changes
    pub(crate) en_path: Vec<(u32, Path)>,
    act: Option<(Act, f64)>,
    pet_until: f64,
    last_poke: f64,
    glance_until: f64,
    idle_at: f64,
    pub idle: bool,
    /// run the JS-ported hooks (wheel, swimming, swarm wave); the comparison tools switch them off to compare the initial pose
    pub hooks_on: bool,
    pub rng: Rng,
    pub hook: super::hooks::Hook,
    tmp: Ov,
    tmp2: Ov,
    pub(crate) tick_dirty: bool,
}

const POKE_COOLDOWN: f64 = 1000.0;
const POKE_MS: f64 = 1400.0;

impl Sim {
    pub fn new(pet: Arc<Pet>, left: bool, seed: u64) -> Sim {
        let n = pet.nodes.len();
        let hook = super::hooks::Hook::for_pet(&pet);
        let mut s = Sim {
            left,
            state: 0,
            reduced: false,
            energy: None,
            look: [0.0; 2],
            aimed: false,
            now: 0.0,
            tint: Tint::new(),
            tint_ease: Ease::Bezier([0.22, 1.0, 0.36, 1.0]),
            tbase: Vec::new(),
            target: Vec::new(),
            cur: Vec::new(),
            trs: vec![Vec::new(); n],
            slots: vec![[Slot::default(); NG]; n],
            tr_active: Vec::new(),
            runs: (0..n).map(|_| Vec::new()).collect(),
            ov: (0..n).map(|_| Ov::default()).collect(),
            animated: Vec::new(),
            looks: Vec::new(),
            en_nodes: Vec::new(),
            en_path: Vec::new(),
            act: None,
            pet_until: -1e9,
            last_poke: -1e9,
            glance_until: -1.0,
            idle_at: 0.0,
            idle: true,
            hooks_on: true,
            rng: Rng::new(seed),
            hook,
            tmp: Ov::default(),
            tmp2: Ov::default(),
            tick_dirty: false,
            pet,
        };
        s.idle_at = 1200.0 + (s.rng.f32() * 2500.0) as f64;
        s.recompose(true, true);
        s.tint.set(TINT[0], 0.0, true);
        s.cur.copy_from_slice(&s.target.clone());
        s.hook_state_changed();
        s
    }

    // ------------------------------------------------------------ public controls

    pub fn state_name(&self) -> &'static str {
        STATES[self.state]
    }

    /// pets.js setState: restarts every animation (even for the same state) and tints with `accent` or the state's edge colour.
    pub fn set_state(&mut self, state: usize, accent: Option<[f32; 3]>) {
        self.state = state.min(STATES.len() - 1);
        if self.state != 1 {
            self.energy = None;
        }
        if self.state == 6 {
            self.aimed = false;
            self.look = [0.0; 2];
            self.glance_until = -1.0;
        }
        self.act = None;
        self.tint.set(accent.or(TINT[self.state]), self.now, self.reduced);
        self.recompose(true, false);
        self.hook_state_changed();
    }

    pub fn set_side(&mut self, left: bool) {
        if self.left != left {
            self.left = left;
            self.recompose(false, false);
        }
    }

    pub fn set_reduced(&mut self, r: bool) {
        if self.reduced != r {
            self.reduced = r;
            self.act = None;
            self.pet_until = -1e9;
            if r {
                self.tint.accent = self.tint.to.0;
                self.tint.k = self.tint.to.1;
                self.tint.busy = false;
            }
            self.recompose(true, true);
            let cur = self.target.clone();
            self.cur.copy_from_slice(&cur);
            self.clear_transitions();
        }
    }

    /// Aim the eyes / head; -1..1 each (the cursor direction). Ignored while asleep.
    pub fn set_look(&mut self, dx: f32, dy: f32) {
        if self.state == 6 {
            return;
        }
        self.aimed = true;
        self.glance_until = -1.0;
        self.apply_look([dx.clamp(-1.0, 1.0), dy.clamp(-1.0, 1.0)]);
    }

    /// `look(null)`: straight ahead again, idle glances allowed.
    pub fn clear_look(&mut self) {
        self.aimed = false;
        self.apply_look([0.0, 0.0]);
    }

    /// Audio energy 0..1 while music plays; `None` returns to the steady beat. Only the music state reacts.
    pub fn set_energy(&mut self, e: Option<f32>) {
        let e = if self.state == 1 { e.map(|v| v.clamp(0.0, 1.0)) } else { None };
        let was = self.energy.is_some();
        self.energy = e;
        if was != e.is_some() {
            self.recompose(false, false);
            self.hook_state_changed();
        } else if e.is_some() {
            self.update_energy();
        }
        self.hook.energy_changed(e);
    }

    /// Click petting: squeeze, happy eyes, hearts. At most once a second; false when it was ignored.
    pub fn poke(&mut self) -> bool {
        if self.reduced || self.now - self.last_poke < POKE_COOLDOWN {
            return false;
        }
        self.last_poke = self.now;
        self.pet_until = self.now + POKE_MS;
        self.recompose(false, false);
        self.hook.poked(self.now, &mut self.rng, self.state);
        true
    }

    /// Switch the idle behaviour (blink / glance / twitch) on or off (pets.js `idle: false`).
    pub fn set_idle(&mut self, on: bool) {
        self.idle = on;
    }

    // ------------------------------------------------------------ composition

    fn side_key(&self) -> char {
        if self.left {
            'l'
        } else {
            'r'
        }
    }

    fn active_flag(&self) -> Option<String> {
        let sk = self.side_key();
        let st = STATES[self.state];
        if self.reduced {
            return None;
        }
        if self.now < self.pet_until {
            return Some(format!("pet|{st}|{sk}"));
        }
        self.act.map(|(a, _)| format!("{}|{st}|r", a.key()))
    }

    /// Rebuilds styles, transition specs and animation lists for the current context. `restart` = every animation starts again (a state set).
    fn recompose(&mut self, restart: bool, snap: bool) {
        let pet = Arc::clone(&self.pet);
        let n = pet.nodes.len();
        let st_key = format!("{}|{}", STATES[self.state], self.side_key());
        let mut base: Vec<Style> = pet.base.clone();
        let mut lists: Vec<Vec<AnimDef>> = pet.base_an.clone();
        let mut trs: Vec<Vec<TransDef>> = pet.base_tr.clone();
        let apply = |p: &Patch, base: &mut Vec<Style>, lists: &mut Vec<Vec<AnimDef>>, trs: &mut Vec<Vec<TransDef>>, with_an: bool| {
            for (i, sp) in &p.st {
                sp.apply(&mut base[*i as usize]);
            }
            if with_an {
                for (i, l) in &p.an {
                    lists[*i as usize] = l.clone();
                }
                for (i, l) in &p.tr {
                    trs[*i as usize] = l.clone();
                }
            }
        };
        if let Some(p) = pet.ctx.get(&st_key) {
            apply(p, &mut base, &mut lists, &mut trs, true);
        }
        let mut alts: Vec<(u32, usize, [u32; 5])> = Vec::new();
        if self.reduced {
            if let Some(p) = pet.ctx.get(&format!("rm|{st_key}")) {
                apply(p, &mut base, &mut lists, &mut trs, false);
            }
            for l in lists.iter_mut() {
                l.clear();
            }
            for t in trs.iter_mut() {
                t.clear();
            }
        } else {
            if let Some(fk) = self.active_flag() {
                if let Some(p) = pet.ctx.get(&fk) {
                    apply(p, &mut base, &mut lists, &mut trs, true);
                }
            }
            self.en_nodes.clear();
            if self.energy.is_some() && self.state == 1 && pet.energy.len() == 5 {
                // energy: the music state plus `data-energy`; styles come from five samples, animations from sample 0 (+ var()-driven keyframes)
                let mut base_en: Vec<Style> = base.clone();
                for (i, sp) in &pet.energy[0].st {
                    sp.apply(&mut base_en[*i as usize]);
                }
                let mut ids: Vec<u32> = pet.energy.iter().flat_map(|p| p.st.iter().map(|(i, _)| *i)).collect();
                ids.sort_unstable();
                ids.dedup();
                for i in ids {
                    let mut arr = [base[i as usize]; 5];
                    for (k, p) in pet.energy.iter().enumerate() {
                        for (j, sp) in &p.st {
                            if *j == i {
                                sp.apply(&mut arr[k]);
                            }
                        }
                    }
                    self.en_nodes.push((i, arr));
                }
                for (i, l) in &pet.energy[0].an {
                    lists[*i as usize] = l.clone();
                    for (j, def) in l.iter().enumerate() {
                        let mut alt = [def.kf; 5];
                        for (k, p) in pet.energy.iter().enumerate() {
                            if let Some((_, lk)) = p.an.iter().find(|(x, _)| x == i) {
                                if let Some(d) = lk.get(j) {
                                    alt[k] = d.kf;
                                }
                            }
                        }
                        if alt.iter().any(|x| *x != alt[0]) {
                            alts.push((*i, j, alt));
                        }
                    }
                }
                for (i, l) in &pet.energy[0].tr {
                    trs[*i as usize] = l.clone();
                }
                base = base_en;
            }
        }
        // static styles
        let old_target = std::mem::take(&mut self.target);
        self.tbase = base;
        self.target = self.tbase.clone();
        self.trs = trs;
        self.rebuild_looks();
        self.update_energy_styles();
        self.apply_look_to_target();
        // animations: keep a running one when the node still lists it
        let now = self.now;
        for i in 0..n {
            let old = std::mem::take(&mut self.runs[i]);
            let mut v: Vec<Run> = Vec::with_capacity(lists[i].len());
            for (j, def) in lists[i].iter().enumerate() {
                let keep = if restart { None } else { old.iter().find(|r| r.def == *def) };
                let alt = alts.iter().find(|(a, b, _)| *a as usize == i && *b == j).map(|x| x.2);
                v.push(Run { def: *def, t0: keep.map(|r| r.t0).unwrap_or(now), alt });
            }
            self.runs[i] = v;
        }
        self.animated = (0..n as u32).filter(|i| !self.runs[*i as usize].is_empty()).collect();
        if old_target.len() != n || snap {
            self.cur = self.target.clone();
            self.clear_transitions();
        } else {
            for i in 0..n {
                if old_target[i] != self.target[i] {
                    self.retarget(i, &old_target[i]);
                }
            }
        }
    }

    fn clear_transitions(&mut self) {
        for s in self.slots.iter_mut() {
            *s = [Slot::default(); NG];
        }
        self.tr_active.clear();
    }

    /// target[i] changed from `old`: start the declared transitions, snap everything else.
    fn retarget(&mut self, i: usize, old: &Style) {
        let new = self.target[i];
        let mut cur = self.cur[i];
        let ntr = self.trs[i].clone();
        // fields that never transition
        cur.dn = new.dn;
        cur.vh = new.vh;
        cur.po = new.po;
        cur.sw = new.sw;
        cur.lc = new.lc;
        cur.lj = new.lj;
        cur.da = new.da;
        cur.dof = new.dof;
        cur.d = new.d;
        cur.fl = new.fl;
        cur.nfl = new.nfl;
        let mut any = false;
        for g in 0..NG {
            let (a, b) = (gv(g, old), gv(g, &new));
            let flags_differ = g == 5 && (old.fi.on != new.fi.on || old.st.on != new.st.on);
            if g == 5 {
                cur.fi.on = new.fi.on;
                cur.st.on = new.st.on;
            }
            if a == b && !flags_differ {
                if self.slots[i][g].active {
                    any = true; // keep going towards the (unchanged) target
                }
                continue;
            }
            let bit = 1u8 << g;
            let spec = if self.reduced || flags_differ { None } else { ntr.iter().find(|t| t.mask & bit != 0 && (t.dur > 0.0)) };
            match spec {
                Some(t) => {
                    let from = gv(g, &cur);
                    self.slots[i][g] = Slot { active: true, t0: self.now, dur: t.dur, delay: t.delay, ease: t.ease, from };
                    any = true;
                }
                None => {
                    self.slots[i][g].active = false;
                    set_gv(g, &mut cur, &b);
                }
            }
        }
        self.cur[i] = cur;
        if any && !self.tr_active.contains(&(i as u32)) {
            self.tr_active.push(i as u32);
        }
    }

    // ------------------------------------------------------------ look and energy layers

    fn rebuild_looks(&mut self) {
        self.looks.clear();
        let pet = Arc::clone(&self.pet);
        let st = STATES[self.state];
        let keys = ["lx", "lxn", "ly", "lyn"];
        let patches: Vec<Option<&Patch>> = keys.iter().map(|k| pet.ctx.get(&format!("{k}|{st}"))).collect();
        let mut ids: Vec<u32> = patches.iter().flatten().flat_map(|p| p.st.iter().map(|(i, _)| *i)).collect();
        ids.sort_unstable();
        ids.dedup();
        for i in ids {
            let s0 = self.tbase[i as usize];
            let l0 = lk(&s0);
            let mut d = [[0.0f32; 12]; 4];
            for (k, p) in patches.iter().enumerate() {
                if let Some(p) = p {
                    let mut s1 = s0;
                    for (j, sp) in &p.st {
                        if *j == i {
                            sp.apply(&mut s1);
                        }
                    }
                    let l1 = lk(&s1);
                    for q in 0..12 {
                        d[k][q] = l1[q] - l0[q];
                    }
                }
            }
            self.looks.push(LookNode { node: i, d });
        }
    }

    fn look_axes(&self) -> [f32; 2] {
        match quant(&self.pet.id) {
            Some((tx, ty, _)) => {
                let q = |v: f32, th: f32| if v.abs() < th { 0.0 } else { v.signum() };
                [q(self.look[0], tx), q(self.look[1], ty)]
            }
            None => self.look,
        }
    }

    fn look_style(&self, ln: &LookNode, mut s: Style) -> Style {
        let [lx, ly] = self.look_axes();
        if lx != 0.0 {
            add_lk(&mut s, &ln.d[if lx > 0.0 { 0 } else { 1 }], lx.abs());
        }
        if ly != 0.0 {
            add_lk(&mut s, &ln.d[if ly > 0.0 { 2 } else { 3 }], ly.abs());
        }
        s
    }

    fn apply_look_to_target(&mut self) {
        for k in 0..self.looks.len() {
            let n = self.looks[k].node as usize;
            let s = self.look_style(&self.looks[k], self.tbase[n]);
            self.target[n] = s;
        }
    }

    fn apply_look(&mut self, v: [f32; 2]) {
        if self.look == v {
            return;
        }
        self.look = v;
        if self.reduced {
            return;
        }
        for k in 0..self.looks.len() {
            let n = self.looks[k].node as usize;
            let new = self.look_style(&self.looks[k], self.tbase[n]);
            if new != self.target[n] {
                let old = self.target[n];
                self.target[n] = new;
                self.retarget(n, &old);
            }
        }
    }

    /// energy samples -> tbase / target of the energy nodes (and the lerped `d` paths)
    fn update_energy_styles(&mut self) {
        let Some(mut e) = self.energy else { return };
        if let Some((_, _, steps)) = quant(&self.pet.id) {
            e = (e * steps).round() / steps; // the 8-bit pets lift in whole cells
        }
        let x = e * 4.0;
        let i0 = (x.floor() as usize).min(3);
        let t = x - i0 as f32;
        self.en_path.clear();
        for k in 0..self.en_nodes.len() {
            let (node, arr) = (self.en_nodes[k].0 as usize, self.en_nodes[k].1);
            let (a, b) = (arr[i0], arr[i0 + 1]);
            let mut s = a;
            s.o = lerp(a.o, b.o, t);
            s.tf = interp::lerp_dec(a.tf, b.tf, t);
            for q in 0..2 {
                s.tl[q] = lerp(a.tl[q], b.tl[q], t);
                s.sc[q] = lerp(a.sc[q], b.sc[q], t);
            }
            s.ro = lerp(a.ro, b.ro, t);
            s.fi.c = lerp4(a.fi.c, b.fi.c, t);
            s.st.c = lerp4(a.st.c, b.st.c, t);
            if a.d != b.d && a.d != NO_PATH && b.d != NO_PATH {
                let mut out = Path::default();
                if path::lerp_into(&self.pet.paths[a.d as usize], &self.pet.paths[b.d as usize], t, &mut out) {
                    self.en_path.push((node as u32, out));
                }
            }
            self.tbase[node] = s;
            self.target[node] = s;
        }
    }

    fn update_energy(&mut self) {
        let old = self.target.clone();
        self.update_energy_styles();
        self.apply_look_to_target();
        for k in 0..self.en_nodes.len() {
            let n = self.en_nodes[k].0 as usize;
            if old[n] != self.target[n] {
                self.retarget(n, &old[n]);
            }
        }
    }

    // ------------------------------------------------------------ the clock

    /// Advance by `dt_ms`. Returns true while something moves (animations, transitions, tint fade, hooks) so the caller keeps drawing frames.
    pub fn tick(&mut self, dt_ms: f32) -> bool {
        let dt = dt_ms.max(0.0) as f64;
        self.now += dt;
        let now = self.now;
        // flags expire
        let mut changed = false;
        if let Some((_, until)) = self.act {
            if now >= until {
                self.act = None;
                changed = true;
            }
        }
        if self.pet_until > 0.0 && now >= self.pet_until {
            self.pet_until = -1e9;
            changed = true;
        }
        if self.glance_until > 0.0 && now >= self.glance_until {
            self.glance_until = -1.0;
            if !self.aimed {
                self.apply_look([0.0, 0.0]);
            }
        }
        if !self.reduced && self.idle && now >= self.idle_at {
            self.idle_behaviour();
            changed = true;
        }
        if changed {
            self.recompose(false, false);
        }
        self.tint.step(now, &self.tint_ease);
        self.step_transitions();
        let hook_busy = self.hooks_on && super::hooks::tick(self, dt as f32);
        let anim_busy = self.eval_animations();
        anim_busy || hook_busy || !self.tr_active.is_empty() || self.tint.busy
    }

    /// ms until the next idle behaviour (for a caller that sleeps while nothing moves)
    pub fn next_wake_ms(&self) -> Option<f32> {
        (!self.reduced && self.idle).then(|| (self.idle_at - self.now).max(0.0) as f32)
    }

    /// pets.js tick(): blink 60 %, else glance 20 % (calm / done), else a twitch; next in 2.4 - 7 s.
    fn idle_behaviour(&mut self) {
        let calmish = self.state == 0 || self.state == 3;
        let r = self.rng.f32();
        if self.state == 6 {
            // asleep: nothing
        } else if r < 0.6 || !calmish {
            self.act = Some((Act::Blink, self.now + 240.0));
        } else if r < 0.8 {
            let x = if self.rng.f32() < 0.5 { -1.0 } else { 1.0 };
            let y = if self.rng.f32() < 0.5 { 0.0 } else { -0.5 };
            if !self.aimed {
                self.glance_until = self.now + 1300.0;
                self.apply_look([x, y]);
            }
        } else {
            let list = twitch_list(&self.pet.id);
            let a = list[(self.rng.f32() * list.len() as f32) as usize % list.len()];
            self.act = Some((a, self.now + if a == Act::Hop { 700.0 } else { 650.0 }));
        }
        self.idle_at = self.now + 2400.0 + (self.rng.f32() * 4600.0) as f64;
    }

    fn step_transitions(&mut self) {
        if self.tr_active.is_empty() {
            return;
        }
        let now = self.now;
        let mut k = 0;
        while k < self.tr_active.len() {
            let i = self.tr_active[k] as usize;
            let mut live = false;
            for g in 0..NG {
                let sl = self.slots[i][g];
                if !sl.active {
                    continue;
                }
                let target = gv(g, &self.target[i]);
                let x = ((now - sl.t0 - sl.delay as f64) / sl.dur as f64).clamp(-1.0, 1.0);
                if x >= 1.0 {
                    set_gv(g, &mut self.cur[i], &target);
                    self.slots[i][g].active = false;
                } else {
                    let e = if x <= 0.0 { 0.0 } else { self.pet.eases[sl.ease as usize].at(x as f32) };
                    let mut v = [0.0; 10];
                    for q in 0..10 {
                        v[q] = lerp(sl.from[q], target[q], e);
                    }
                    set_gv(g, &mut self.cur[i], &v);
                    live = true;
                }
            }
            if live {
                k += 1;
            } else {
                self.tr_active.swap_remove(k);
            }
        }
    }

    // ------------------------------------------------------------ animations

    /// progress 0..1 of one running animation, None while it contributes nothing
    fn progress(def: &AnimDef, t_ms: f64) -> Option<f32> {
        let t = t_ms - def.delay as f64;
        let dur = def.dur as f64;
        let total = if def.iter < 0.0 { f64::INFINITY } else { dur * def.iter as f64 };
        let (i, frac);
        if t < 0.0 {
            if def.fill & 2 == 0 {
                return None;
            }
            i = 0.0;
            frac = 0.0;
        } else if t >= total {
            if def.fill & 1 == 0 {
                return None;
            }
            let it = def.iter as f64;
            i = if it.fract() == 0.0 { it - 1.0 } else { it.floor() };
            frac = if it.fract() == 0.0 { 1.0 } else { it.fract() };
        } else {
            let q = t / dur;
            i = q.floor();
            frac = q - i;
        }
        let odd = (i as i64) % 2 == 1;
        let p = match def.dir {
            0 => frac,
            1 => 1.0 - frac,
            2 => {
                if odd {
                    1.0 - frac
                } else {
                    frac
                }
            }
            _ => {
                if odd {
                    frac
                } else {
                    1.0 - frac
                }
            }
        };
        Some(p as f32)
    }

    /// Evaluates every running animation into `ov`. Returns true while any of them is still going.
    fn eval_animations(&mut self) -> bool {
        let pet = Arc::clone(&self.pet);
        let now = self.now;
        let mut busy = false;
        let (rw, rh) = (pet.w, pet.h);
        let e = self.energy;
        for a in 0..self.animated.len() {
            let i = self.animated[a] as usize;
            self.ov[i].clear();
            let under = self.cur[i];
            let (w, h) = match pet.nodes[i].bb {
                Some(b) if i > 2 => (b[2], b[3]),
                _ => (rw, rh),
            };
            for r in 0..self.runs[i].len() {
                let (def, t0, alt) = (self.runs[i][r].def, self.runs[i][r].t0, self.runs[i][r].alt);
                let finite_done = def.iter >= 0.0 && now - t0 >= (def.delay + def.dur * def.iter) as f64;
                if !finite_done || def.fill & 1 != 0 {
                    // an animation that finished without fill-forwards is silent, one with it holds its end value without needing frames
                }
                if def.iter < 0.0 || !finite_done {
                    busy = true;
                }
                let Some(p) = Self::progress(&def, now - t0) else { continue };
                match (alt, e) {
                    (Some(alt), Some(e)) => {
                        let x = e * 4.0;
                        let i0 = (x.floor() as usize).min(3);
                        let t = x - i0 as f32;
                        self.tmp.clear();
                        self.tmp2.clear();
                        let (mut ta, mut tb) = (std::mem::take(&mut self.tmp), std::mem::take(&mut self.tmp2));
                        Self::eval_set(&pet, &pet.kf[alt[i0] as usize], p, &under, w, h, &mut ta);
                        Self::eval_set(&pet, &pet.kf[alt[i0 + 1] as usize], p, &under, w, h, &mut tb);
                        let ov = &mut self.ov[i];
                        merge_lerp(ov, &ta, &tb, t, w, h);
                        self.tmp = ta;
                        self.tmp2 = tb;
                    }
                    _ => Self::eval_set(&pet, &pet.kf[def.kf as usize], p, &under, w, h, &mut self.ov[i]),
                }
            }
        }
        busy
    }

    fn eval_set(pet: &Pet, set: &KfSet, p: f32, under: &Style, w: f32, h: f32, ov: &mut Ov) {
        for tr in set {
            let fr = &tr.frames;
            if fr.is_empty() {
                continue;
            }
            // segment [a, b] around p; implicit 0% / 100% frames carry the underlying value
            let und = under_val(tr.prop, under);
            let (ao, av, bo, bv, e);
            if p < fr[0].o {
                match &und {
                    Some(u) => {
                        (ao, av, bo, bv, e) = (0.0, u, fr[0].o, &fr[0].v, fr[0].e);
                    }
                    None => {
                        write_val(tr.prop, &fr[0].v, &fr[0].v, 0.0, ov, w, h);
                        continue;
                    }
                }
            } else if p >= fr[fr.len() - 1].o {
                let l = &fr[fr.len() - 1];
                if l.o >= 1.0 || und.is_none() {
                    write_val(tr.prop, &l.v, &l.v, 0.0, ov, w, h);
                    continue;
                }
                let u = und.as_ref().unwrap();
                (ao, av, bo, bv, e) = (l.o, &l.v, 1.0, u, l.e);
            } else {
                let k = fr.windows(2).position(|x| p >= x[0].o && p < x[1].o).unwrap_or(0);
                (ao, av, bo, bv, e) = (fr[k].o, &fr[k].v, fr[k + 1].o, &fr[k + 1].v, fr[k].e);
            }
            let x = if bo > ao { (p - ao) / (bo - ao) } else { 1.0 };
            let t = pet.eases[e as usize].at(x);
            write_val(tr.prop, av, bv, t, ov, w, h);
        }
    }

    // ------------------------------------------------------------ results for the renderer

    pub fn node_count(&self) -> usize {
        self.pet.nodes.len()
    }

    pub fn style(&self, i: usize) -> &Style {
        &self.cur[i]
    }

    pub fn overrides(&self, i: usize) -> &Ov {
        &self.ov[i]
    }

    /// the `d` path a node draws now (animation / energy / static), None for non-paths
    pub fn path_of(&self, i: usize) -> Option<&Path> {
        let ov = &self.ov[i];
        if ov.has_d {
            return Some(&ov.d);
        }
        if let Some((_, p)) = self.en_path.iter().find(|(n, _)| *n as usize == i) {
            return Some(p);
        }
        let s = &self.cur[i];
        let idx = if s.d != NO_PATH {
            s.d
        } else if let Shape::Path(p) = self.pet.nodes[i].shape {
            p
        } else {
            return None;
        };
        self.pet.paths.get(idx as usize)
    }

    /// local transform of node `i` (about its pivot), the box `(w, h)` is the viewBox
    pub fn local_matrix(&self, i: usize) -> Mat {
        let s = &self.cur[i];
        let ov = &self.ov[i];
        let tl = s.tl;
        let ro = ov.ro.unwrap_or(s.ro);
        let sc = s.sc;
        let (bw, bh) = match self.pet.nodes[i].bb {
            Some(b) if i > 2 => (b[2], b[3]),
            _ => (self.pet.w, self.pet.h),
        };
        let m_tf = match &ov.tf {
            Some(l) => l.to_mat(bw, bh),
            None => interp::from_dec(s.tf),
        };
        if ov.tf.is_none() && s.tf == DEC_IDENT && tl == [0.0; 2] && ro == 0.0 && sc == [1.0; 2] {
            return IDENT;
        }
        let po = if self.pet.nodes[i].tag == Tag::Div { [s.po[0] * self.pet.w, s.po[1] * self.pet.h] } else { s.po };
        let mut m = interp::translate(po[0], po[1]);
        m = interp::mul(m, interp::translate(tl[0], tl[1]));
        if ro != 0.0 {
            m = interp::mul(m, interp::rotate(ro));
        }
        if sc != [1.0; 2] {
            m = interp::mul(m, interp::scale(sc[0], sc[1]));
        }
        m = interp::mul(m, m_tf);
        interp::mul(m, interp::translate(-po[0], -po[1]))
    }

    /// Does anything in the pet move on its own (an infinite animation, a hook)? Used by static renders.
    pub fn is_static(&self) -> bool {
        self.animated.is_empty() && self.tr_active.is_empty() && !self.tint.busy
    }

    fn hook_state_changed(&mut self) {
        super::hooks::state_changed(self);
    }

    /// reset all dynamic state to "just created" at `now` (used by the dump tools to replay a state from t = 0)
    pub fn restart(&mut self) {
        let s = self.state;
        self.set_state(s, None);
    }

    pub(crate) fn cur_mut(&mut self) -> &mut Vec<Style> {
        &mut self.cur
    }

    pub(crate) fn target_style(&self, i: usize) -> Style {
        self.target[i]
    }

    /// a hook moves the target of a node (the transition declared on it plays)
    pub(crate) fn override_target(&mut self, i: usize, s: Style) {
        let old = self.target[i];
        self.target[i] = s;
        self.retarget(i, &old);
    }

    pub(crate) fn restore_targets(&mut self, nodes: &[usize]) {
        for &i in nodes {
            let base = self.tbase[i];
            let s = match self.looks.iter().find(|l| l.node as usize == i) {
                Some(l) => self.look_style(l, base),
                None => base,
            };
            self.override_target(i, s);
        }
    }

    pub fn dynamic_path(&self, i: usize) -> bool {
        self.en_path.iter().any(|(n, _)| *n as usize == i)
    }
}

fn under_val(p: Prop, s: &Style) -> Option<Val> {
    Some(match p {
        Prop::Tf => Val::Tf(TfList::from_dec(s.tf)),
        Prop::Rotate => Val::F(s.ro),
        Prop::Opacity => Val::F(s.o),
        Prop::Fill => Val::Col(s.fi.c),
        Prop::Stroke => Val::Col(s.st.c),
        Prop::Visibility => Val::Vis(!s.vh),
        Prop::DashOffset => Val::F(s.dof),
        Prop::StrokeWidth => Val::F(s.sw),
        Prop::DashArray => Val::Dash(s.da),
        Prop::D => return None,
    })
}

fn write_val(prop: Prop, a: &Val, b: &Val, t: f32, ov: &mut Ov, w: f32, h: f32) {
    match (prop, a, b) {
        (Prop::Tf, Val::Tf(x), Val::Tf(y)) => ov.tf = Some(if t == 0.0 { *x } else if t == 1.0 { *y } else { x.lerp(y, t, w, h) }),
        (Prop::Rotate, Val::F(x), Val::F(y)) => ov.ro = Some(lerp(*x, *y, t)),
        (Prop::Opacity, Val::F(x), Val::F(y)) => ov.o = Some(lerp(*x, *y, t)),
        (Prop::DashOffset, Val::F(x), Val::F(y)) => ov.dof = Some(lerp(*x, *y, t)),
        (Prop::StrokeWidth, Val::F(x), Val::F(y)) => ov.sw = Some(lerp(*x, *y, t)),
        (Prop::Fill, Val::Col(x), Val::Col(y)) => ov.fi = Some(lerp4(*x, *y, t)),
        (Prop::Stroke, Val::Col(x), Val::Col(y)) => ov.st = Some(lerp4(*x, *y, t)),
        (Prop::Visibility, Val::Vis(x), Val::Vis(y)) => {
            // CSS: visible during the whole transition when either end is visible, discrete otherwise
            ov.vis = Some(if x == y { *x } else if t <= 0.0 { *x } else if t >= 1.0 { *y } else { *x || *y });
        }
        (Prop::D, Val::D(x), Val::D(y)) => {
            ov.has_d = if t == 0.0 {
                ov.d.segs.clear();
                ov.d.segs.extend_from_slice(&x.segs);
                true
            } else if path::lerp_into(x, y, t, &mut ov.d) {
                true
            } else {
                ov.d.segs.clear();
                ov.d.segs.extend_from_slice(if t < 0.5 { &x.segs } else { &y.segs });
                true
            };
        }
        (Prop::DashArray, Val::Dash(x), Val::Dash(y)) => {
            ov.da = Some(if x.n == y.n {
                let mut d = *x;
                for q in 0..x.n as usize {
                    d.v[q] = lerp(x.v[q], y.v[q], t);
                }
                d
            } else if t < 0.5 {
                *x
            } else {
                *y
            });
        }
        _ => {}
    }
}

/// result = lerp(a, b, t) for the animated numeric fields (energy-driven keyframes), everything else from `a`
fn merge_lerp(out: &mut Ov, a: &Ov, b: &Ov, t: f32, w: f32, h: f32) {
    out.tf = match (&a.tf, &b.tf) {
        (Some(x), Some(y)) => Some(x.lerp(y, t, w, h)),
        (x, _) => *x,
    };
    out.ro = match (a.ro, b.ro) {
        (Some(x), Some(y)) => Some(lerp(x, y, t)),
        (x, _) => x,
    };
    out.o = match (a.o, b.o) {
        (Some(x), Some(y)) => Some(lerp(x, y, t)),
        (x, _) => x,
    };
    out.fi = a.fi;
    out.st = a.st;
    out.vis = a.vis;
    out.dof = a.dof;
    out.da = a.da;
    out.sw = a.sw;
    if a.has_d {
        out.has_d = true;
        out.d.segs.clear();
        out.d.segs.extend_from_slice(&a.d.segs);
    }
}
