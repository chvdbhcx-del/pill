//! What comes in when the selection lands on a row: the "enter" icon, the uninstall tint, and the hover buttons one after another. Every
//! piece is a spring that is released a stagger apart (never a clock-driven curve), so a second keypress mid-way simply starts again.
use crate::ui::liquid::{Feel, Flow};

/// Hover buttons per row that get their own spring (a row never has more).
pub(crate) const MAX_BTN: usize = 6;
/// Seconds between two buttons coming in.
const STAGGER: f32 = 0.024;

pub(crate) struct SelFx {
    age: f32,
    /// the Enter icon at the row's right edge
    pub enter: Flow,
    /// the uninstall rows turn red as they are selected
    pub tint: Flow,
    alpha: [Flow; MAX_BTN],
    pop: [Flow; MAX_BTN],
}

impl SelFx {
    /// Everything shown (nothing animates for the row that is selected when the bar opens).
    pub fn new() -> SelFx {
        SelFx {
            age: 10.0,
            enter: Flow::new(1.0, Feel::FADE.within(120.0)),
            tint: Flow::new(1.0, Feel::FADE.within(380.0)),
            alpha: [Flow::new(1.0, Feel::FADE); MAX_BTN],
            pop: [Flow::new(1.0, Feel::POP); MAX_BTN],
        }
    }

    /// The selection moved: everything starts from nothing again.
    pub fn restart(&mut self) {
        self.age = 0.0;
        self.enter.jump(0.0);
        self.enter.to(1.0);
        self.tint.jump(0.0);
        self.tint.to(1.0);
        for f in self.alpha.iter_mut().chain(self.pop.iter_mut()) {
            f.jump(0.0);
        }
    }

    /// Opacity (0..1) and scale (0.5 .. 1, plus the bounce) of hover button `b`.
    pub fn button(&self, b: usize) -> (f32, f32) {
        let b = b.min(MAX_BTN - 1);
        (self.alpha[b].at01(), 0.5 + 0.5 * self.pop[b].at())
    }

    /// Advance; true while anything is still coming in.
    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.age += (dt_ms / 1000.0) as f32;
        let mut busy = self.age < STAGGER * MAX_BTN as f32;
        for b in 0..MAX_BTN {
            if self.age >= STAGGER * b as f32 {
                self.alpha[b].to(1.0);
                self.pop[b].to(1.0);
            }
        }
        for f in [&mut self.enter, &mut self.tint].into_iter().chain(self.alpha.iter_mut()).chain(self.pop.iter_mut()) {
            f.step(dt_ms);
            busy |= f.moving();
        }
        busy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    #[test]
    fn buttons_come_in_one_after_another_and_it_idles() {
        let _m = motion_on();
        let mut s = SelFx::new();
        assert!(!s.step(8.0), "a fresh bar shows the selected row as is");
        s.restart();
        assert_eq!(s.button(2), (0.0, 0.5));
        let mut t = 0.0;
        for _ in 0..4 {
            s.step(8.0);
            t += 8.0;
        }
        assert!(s.button(0).0 > s.button(2).0, "button 0 leads by its stagger ({t} ms)");
        let mut ms = 0.0;
        let mut last_off = 0.0;
        while s.step(8.0) && ms < 3000.0 {
            ms += 8.0;
            if (0..MAX_BTN).any(|b| (s.button(b).0 - 1.0).abs() > 0.05 || (s.button(b).1 - 1.0).abs() > 0.05) {
                last_off = ms;
            }
        }
        assert!(last_off < 420.0 * 1.5 + 6.0 * 24.0, "visually settled by {last_off} ms");
        assert!(ms < 2000.0, "idle after {ms} ms");
        assert_eq!((s.button(0).0, s.enter.at()), (1.0, 1.0));
    }
}
