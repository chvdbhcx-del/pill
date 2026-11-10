//! The liquid state behind what the live strip shows: every entry / leave / change that used to be a time curve (`motion::*.at(age)`) is a
//! stateful [`Flow`] here, stepped once per frame by the view. A swap of the content, a card appearing, a digit rolling, a column item popping
//! in, the privacy dots, the edge pulse, the body colour: they keep their velocity when the target changes half-way and settle by themselves,
//! so `moving()` tells the view when the window may idle again. Painting only reads the resolved numbers (`Fx`).
use super::live::{CardPaint, Fx};
use crate::ui::liquid::{Feel, Flow};
use glasskit::Color;
use pill_ui_core::live::model::{Anim, RowSpec};
use pill_ui_core::shell::cards::CardPhase;
use pill_ui_core::shell::clock::DigitRoll;
use pill_ui_core::shell::types::{LiveContent, View as ShellView};
use std::collections::HashMap;

/// Feels of the strip, in one place (the presets themselves are tuned in `ui::liquid`).
pub const SWAP_IN: Feel = Feel::FADE.within(150.0);
pub const GHOST_OUT: Feel = Feel::FADE.within(80.0);
pub const HOVER: Feel = Feel::FADE.within(130.0);

#[derive(Clone, Copy, Default)]
pub struct Inputs<'a> {
    pub rows: Option<&'a HashMap<String, RowSpec>>,
    /// the card button under the pointer
    pub hover_btn: Option<usize>,
    /// (camera, microphone) in use
    pub privacy: (bool, bool),
    /// the edge pulse last asked for: (strength 0..1, when)
    pub pulse: Option<(f32, f64)>,
    /// left / right padding and the legs' eye-slot shift of the content area (css px)
    pub pad: [f32; 3],
}

pub struct LiveFx {
    seen_epoch: Option<u64>,
    /// opacity of the incoming content / of the content leaving under it
    swap: Flow,
    ghost: Flow,
    /// the row's own enter animation (`RowSpec::anim.enter`): scale, slide, opacity
    row_s: Flow,
    row_dy: Flow,
    row_a: Flow,
    card_id: Option<String>,
    /// when the card on screen began to enter: a new card with the same id draws its check mark again
    card_since: f64,
    card_a: Flow,
    card_s: Flow,
    card_check: Flow,
    card_hov: Vec<Flow>,
    /// the button that was under the pointer while the card was in: it stays lit while the card leaves (which one was pressed)
    card_last: Option<usize>,
    /// clock side-column items popping in after a reshape: key -> (scale, opacity, last age seen)
    pops: Vec<((bool, usize), Flow, Flow, f64)>,
    /// clock digits rolling: (roll, progress)
    rolls: Vec<(DigitRoll, Flow)>,
    dots: [Flow; 2],
    pulse: Flow,
    pulse_up: Option<f32>,
    pulse_t0: f64,
    pad: [Flow; 3],
    fill: [Flow; 4],
    shadow: Flow,
    inited: bool,
    busy: bool,
    // resolved for painting
    out_rolls: Vec<(DigitRoll, f32)>,
    out_enter: Vec<(bool, usize, f32, f32)>,
    out_hov: Vec<f32>,
}

fn go(f: &mut Flow, target: f32, feel: Feel) {
    if f.feel() != feel {
        f.set_feel(feel);
    }
    f.to(target);
}

impl Default for LiveFx {
    fn default() -> Self {
        LiveFx::new()
    }
}

impl LiveFx {
    pub fn new() -> LiveFx {
        LiveFx {
            seen_epoch: None,
            swap: Flow::new(1.0, SWAP_IN),
            ghost: Flow::new(0.0, GHOST_OUT),
            row_s: Flow::new(1.0, Feel::POP.within(400.0)),
            row_dy: Flow::px(0.0, Feel::FLOW),
            row_a: Flow::new(1.0, Feel::FADE.within(220.0)),
            card_id: None,
            card_since: f64::NEG_INFINITY,
            card_a: Flow::new(0.0, Feel::FADE.within(240.0)),
            card_s: Flow::new(0.96, Feel::POP.within(360.0)),
            card_check: Flow::new(0.0, Feel::FADE.within(320.0)),
            card_hov: Vec::new(),
            card_last: None,
            pops: Vec::new(),
            rolls: Vec::new(),
            dots: [Flow::new(0.0, Feel::POP), Flow::new(0.0, Feel::POP)],
            pulse: Flow::new(0.0, Feel::FADE.within(110.0)),
            pulse_up: None,
            pulse_t0: f64::NEG_INFINITY,
            pad: [Flow::px(0.0, Feel::FLOW), Flow::px(0.0, Feel::FLOW), Flow::px(0.0, Feel::FLOW)],
            fill: [Flow::new(0.0, Feel::FADE), Flow::new(0.0, Feel::FADE), Flow::new(0.0, Feel::FADE), Flow::new(0.0, Feel::FADE)],
            shadow: Flow::new(0.0, Feel::FADE),
            inited: false,
            busy: false,
            out_rolls: Vec::new(),
            out_enter: Vec::new(),
            out_hov: Vec::new(),
        }
    }

    /// True while anything here has not settled: the window keeps asking for frames.
    pub fn moving(&self) -> bool {
        self.busy
    }

    fn all(&mut self) -> Vec<&mut Flow> {
        let mut v: Vec<&mut Flow> = vec![
            &mut self.swap, &mut self.ghost, &mut self.row_s, &mut self.row_dy, &mut self.row_a, &mut self.card_a, &mut self.card_s, &mut self.card_check,
            &mut self.pulse, &mut self.shadow,
        ];
        v.extend(self.card_hov.iter_mut());
        v.extend(self.dots.iter_mut());
        v.extend(self.pad.iter_mut());
        v.extend(self.fill.iter_mut());
        for p in self.pops.iter_mut() {
            v.push(&mut p.1);
            v.push(&mut p.2);
        }
        v.extend(self.rolls.iter_mut().map(|r| &mut r.1));
        v
    }

    fn start_row(&mut self, anim: Anim) {
        let ms = (super::core::anim_ms(anim) as f32).max(200.0);
        let (s0, dy0, a0, fade) = match anim {
            Anim::Pop => (0.92, 0.0, 0.0, 220.0),
            Anim::Slide => (1.0, -8.0, 0.0, ms),
            Anim::Fade => (1.0, 0.0, 0.0, ms),
            _ => (1.0, 0.0, 1.0, 220.0),
        };
        // `jump` drops the target too: set it again afterwards
        self.row_s.jump(s0);
        self.row_dy.jump(dy0);
        self.row_a.jump(a0);
        go(&mut self.row_a, 1.0, Feel::FADE.within(fade));
        go(&mut self.row_s, 1.0, Feel::POP.within(ms));
        go(&mut self.row_dy, 0.0, Feel::FLOW.within(ms));
    }

    fn on_swap(&mut self, v: &ShellView, rows: Option<&HashMap<String, RowSpec>>) {
        // what was shown fades out from the opacity it had reached, the new content flows in from nothing
        self.ghost.jump(self.swap.at01());
        self.ghost.to(0.0);
        self.swap.jump(0.0);
        self.swap.to(1.0);
        let anim = match &v.live.content {
            LiveContent::Slot { id, .. } => rows.and_then(|r| r.get(id)).map_or(Anim::None, |r| r.anim.enter),
            _ => Anim::None,
        };
        self.start_row(anim);
    }

    fn step_card(&mut self, v: &ShellView, hover: Option<usize>) {
        if let LiveContent::Card(cv) = &v.live.content {
            if cv.spec.id != self.card_id {
                self.card_id = cv.spec.id.clone();
                self.card_a.jump(0.0);
                self.card_s.jump(0.96);
                self.card_check.jump(0.0);
                self.card_hov.clear();
                self.card_last = None;
            }
            match cv.phase {
                CardPhase::In { since_ms } => {
                    if since_ms != self.card_since {
                        self.card_since = since_ms;
                        self.card_check.jump(0.0);
                    }
                    go(&mut self.card_a, 1.0, Feel::FADE.within(240.0));
                    go(&mut self.card_s, 1.0, Feel::POP.within(360.0));
                    self.card_check.to(1.0);
                }
                CardPhase::Out { .. } => {
                    go(&mut self.card_a, 0.0, Feel::FADE.within(150.0));
                    go(&mut self.card_s, 0.97, Feel::FLOW.within(180.0));
                }
            }
            let hover = if matches!(cv.phase, CardPhase::Out { .. }) { hover.or(self.card_last) } else { hover };
            self.card_last = if matches!(cv.phase, CardPhase::Out { .. }) { self.card_last } else { hover };
            if let Some(i) = hover {
                while self.card_hov.len() <= i {
                    self.card_hov.push(Flow::new(0.0, HOVER));
                }
            }
            for (i, f) in self.card_hov.iter_mut().enumerate() {
                go(f, if hover == Some(i) { 1.0 } else { 0.0 }, HOVER);
            }
        }
    }

    fn step_clock(&mut self, v: &ShellView) {
        // side-column items popping in: scale on the pop spring, opacity on a fade
        for &(right, idx, age) in &v.live.entering {
            match self.pops.iter_mut().find(|p| p.0 == (right, idx)) {
                Some(p) => {
                    if age < p.3 && !p.1.moving() {
                        p.1.jump(0.6);
                        p.2.jump(0.0);
                    }
                    p.3 = age;
                    p.1.to(1.0);
                    p.2.to(1.0);
                }
                None if age < 200.0 => {
                    let (mut s, mut a) = (Flow::new(0.6, Feel::POP), Flow::new(0.0, Feel::FADE.within(220.0)));
                    s.to(1.0);
                    a.to(1.0);
                    self.pops.push(((right, idx), s, a, age));
                }
                None => {}
            }
        }
        // a rolling digit: the old one leaves upwards, the new one comes in from below
        for (roll, age) in &v.live.rolls {
            if !self.rolls.iter().any(|r| r.0 == *roll) && *age < 150.0 {
                self.rolls.retain(|r| r.0.index != roll.index);
                let mut f = Flow::new(0.0, Feel::FLOW.within(190.0));
                f.to(1.0);
                self.rolls.push((roll.clone(), f));
            }
        }
    }

    fn step_pulse(&mut self, pulse: Option<(f32, f64)>) {
        if let Some((s, t0)) = pulse {
            if t0 != self.pulse_t0 {
                self.pulse_t0 = t0;
                self.pulse_up = Some(s);
                go(&mut self.pulse, s, Feel::FADE.within(110.0));
            }
        }
        if let Some(peak) = self.pulse_up {
            if self.pulse.at() >= peak * 0.9 {
                self.pulse_up = None;
                go(&mut self.pulse, 0.0, Feel::FADE.within(520.0));
            }
        }
    }

    /// One frame. Returns true while anything still moves.
    pub fn step(&mut self, v: &ShellView, inp: Inputs, dt_ms: f64) -> bool {
        let b = &v.body;
        let fill = [b.fill.r as f32 / 255.0, b.fill.g as f32 / 255.0, b.fill.b as f32 / 255.0, b.fill.a];
        let first = !self.inited;
        if first {
            self.inited = true;
            for (f, x) in self.fill.iter_mut().zip(fill) {
                f.jump(x);
            }
            self.shadow.jump(if b.shadow { 1.0 } else { 0.0 });
            for (f, x) in self.pad.iter_mut().zip(inp.pad) {
                f.jump(x);
            }
        }
        match self.seen_epoch {
            None => self.seen_epoch = Some(v.live.swap_epoch),
            Some(e) if e != v.live.swap_epoch => {
                self.seen_epoch = Some(v.live.swap_epoch);
                self.on_swap(v, inp.rows);
            }
            _ => {}
        }
        self.step_card(v, inp.hover_btn);
        self.step_clock(v);
        self.step_pulse(inp.pulse);
        for (f, x) in self.fill.iter_mut().zip(fill) {
            go(f, x, Feel::FADE.within(220.0));
        }
        go(&mut self.shadow, if b.shadow { 1.0 } else { 0.0 }, Feel::FADE.within(220.0));
        for (f, x) in self.pad.iter_mut().zip(inp.pad) {
            go(f, x, Feel::FLOW);
        }
        go(&mut self.dots[0], if inp.privacy.1 { 1.0 } else { 0.0 }, Feel::POP);
        go(&mut self.dots[1], if inp.privacy.0 { 1.0 } else { 0.0 }, Feel::POP);
        if v.reduced {
            // no animation: every flow lands where it is heading
            for f in self.all() {
                let t = f.target();
                f.jump(t);
            }
            self.pulse_up = None;
        }
        for f in self.all() {
            f.step(dt_ms);
        }
        // finished entries leave the lists once they have settled and the shell no longer lists them
        let (entering, rolls) = (&v.live.entering, &v.live.rolls);
        self.pops.retain(|p| p.1.moving() || p.2.moving() || entering.iter().any(|e| (e.0, e.1) == p.0));
        self.rolls.retain(|r| r.1.moving() || rolls.iter().any(|x| x.0 == r.0));
        self.out_enter = self.pops.iter().map(|p| (p.0 .0, p.0 .1, p.1.at(), p.2.at01())).collect();
        self.out_rolls = self.rolls.iter().map(|r| (r.0.clone(), r.1.at())).collect();
        self.out_hov = self.card_hov.iter().map(|f| f.at01()).collect();
        self.busy = self.all().iter().any(|f| f.moving()) || self.pulse_up.is_some();
        self.busy
    }

    // ---------------------------------------------------------------------------------------------- what painting reads

    /// Opacity of the incoming content.
    pub fn swap(&self) -> f32 {
        self.swap.at01()
    }

    /// Opacity of the content leaving (0 once it is gone).
    pub fn ghost(&self) -> f32 {
        self.ghost.at01()
    }

    /// The numbers the content being shown is painted with.
    pub fn fx(&self) -> Fx<'_> {
        Fx { rolls: &self.out_rolls, entering: &self.out_enter, row: (self.row_s.at(), self.row_dy.at(), self.row_a.at01()), card: self.card(), ring: (0.0, 0.0) }
    }

    /// The numbers the content that is leaving is painted with: no entry motion, the card as it last was.
    pub fn ghost_fx(&self) -> Fx<'_> {
        Fx { rolls: &[], entering: &[], row: (1.0, 0.0, 1.0), card: self.card(), ring: (0.0, 0.0) }
    }

    fn card(&self) -> CardPaint<'_> {
        CardPaint { alpha: self.card_a.at01(), scale: self.card_s.at(), check: self.card_check.at01(), hover: &self.out_hov, phone_open: false }
    }

    /// Presence of the microphone (0) and camera (1) dot, with a little overshoot: they pop.
    pub fn dot(&self, i: usize) -> f32 {
        self.dots[i].at().max(0.0)
    }

    /// Strength of the edge pulse right now.
    pub fn pulse(&self) -> f32 {
        self.pulse.at01()
    }

    /// (left, right, slot shift) of the content area.
    pub fn pad(&self) -> [f32; 3] {
        [self.pad[0].at(), self.pad[1].at(), self.pad[2].at()]
    }

    /// The body colour, eased between the folded and the open transparency and between themes.
    pub fn fill(&self) -> Color {
        Color::rgba(self.fill[0].at01(), self.fill[1].at01(), self.fill[2].at01(), self.fill[3].at01())
    }

    pub fn shadow(&self) -> f32 {
        self.shadow.at01()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use crate::ui::island::tests_support::blank_view;
    use pill_ui_core::shell::cards::CardView;

    fn run(fx: &mut LiveFx, v: &ShellView, inp: Inputs, ms: f64) -> Vec<f32> {
        (0..(ms / 8.0) as usize)
            .map(|_| {
                fx.step(v, inp, 8.0);
                fx.swap()
            })
            .collect()
    }

    #[test]
    fn a_swap_flows_in_from_nothing_while_the_old_content_fades_from_where_it_was() {
        let _m = motion_on();
        let mut fx = LiveFx::new();
        let mut v = blank_view();
        assert!(!fx.step(&v, Inputs::default(), 8.0), "the first frame animates nothing");
        v.live.swap_epoch += 1;
        let up = run(&mut fx, &v, Inputs::default(), 40.0);
        assert!(up[0] < 0.2 && up.windows(2).all(|w| w[1] >= w[0]), "starts softly and only rises: {up:?}");
        assert!(fx.ghost() > 0.0 && fx.ghost() < 1.0 + 1e-3);
        // a second swap in the middle of the first: the old content leaves from the opacity the first one had reached
        let mid = fx.swap();
        v.live.swap_epoch += 1;
        fx.step(&v, Inputs::default(), 0.0);
        assert!((fx.ghost() - mid).abs() < 0.05, "ghost {} vs {mid}", fx.ghost());
        run(&mut fx, &v, Inputs::default(), 500.0);
        assert!(!fx.moving() && fx.swap() == 1.0 && fx.ghost() == 0.0, "settled and idle afterwards");
    }

    #[test]
    fn a_card_enters_flows_out_and_turns_around_without_a_jump() {
        let _m = motion_on();
        let mut fx = LiveFx::new();
        let mut v = blank_view();
        fx.step(&v, Inputs::default(), 8.0);
        let spec = pill_ui_core::shell::cards::CardSpec { id: Some("c1".into()), ..Default::default() };
        v.live.content = LiveContent::Card(CardView { spec: spec.clone(), phase: CardPhase::In { since_ms: 0.0 } });
        let mut last = 0.0;
        for _ in 0..12 {
            fx.step(&v, Inputs::default(), 8.0);
            let a = fx.fx().card.alpha;
            assert!(a >= last && a < 1.0, "{a}");
            last = a;
        }
        assert!(last > 0.0 && fx.fx().card.scale < 1.02);
        // the card is dismissed while it is still coming in: it turns around from where it is
        v.live.content = LiveContent::Card(CardView { spec, phase: CardPhase::Out { since_ms: 100.0 } });
        let mut prev = last;
        let mut peak = last;
        for _ in 0..80 {
            fx.step(&v, Inputs::default(), 8.0);
            let a = fx.fx().card.alpha;
            assert!((a - prev).abs() < 0.25, "no jump: {prev} -> {a}");
            peak = peak.max(a);
            prev = a;
        }
        assert!(peak < last + 0.3, "it only carried on a little before turning");
        assert!(prev < 0.02 && !fx.moving(), "gone and idle: {prev}");
    }

    #[test]
    fn the_privacy_dots_pop_in_and_the_pulse_rises_then_falls_back_to_idle() {
        let _m = motion_on();
        let mut fx = LiveFx::new();
        let v = blank_view();
        fx.step(&v, Inputs::default(), 8.0);
        let on = Inputs { privacy: (false, true), ..Default::default() };
        let seen: Vec<f32> = (0..60).map(|_| {
            fx.step(&v, on, 8.0);
            fx.dot(0)
        }).collect();
        assert!(seen[0] < 0.2 && seen.iter().any(|x| *x > 1.0), "a pop with a little overshoot: {seen:?}");
        assert_eq!(fx.dot(1), 0.0);
        let p = Inputs { privacy: (false, true), pulse: Some((0.8, 100.0)), ..Default::default() };
        let strengths: Vec<f32> = (0..200).map(|_| {
            fx.step(&v, p, 8.0);
            fx.pulse()
        }).collect();
        let top = strengths.iter().cloned().fold(0.0, f32::max);
        assert!(top > 0.5 && top < 0.9, "{top}");
        assert!(*strengths.last().unwrap() < 0.01, "the pulse faded away");
        assert!(!fx.moving(), "idle afterwards");
    }

    #[test]
    fn reduced_motion_lands_everything_at_once() {
        let _lock = crate::ui::host::testing::motion_lock();
        let mut fx = LiveFx::new();
        let mut v = blank_view();
        v.reduced = true;
        fx.step(&v, Inputs::default(), 8.0);
        v.live.swap_epoch += 1;
        fx.step(&v, Inputs::default(), 8.0);
        assert!(fx.swap() == 1.0 && !fx.moving());
    }
}
