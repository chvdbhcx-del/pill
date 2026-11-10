//! The "Recently copied" card of the Home tab: state, motion and painting of up to three clipboard chips (the data model is
//! `pill_ui_core::panel::a::home_recent`). A chip rises in when its entry is new, lights up while it is copied back.
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::a::touch::Touch;
use glasskit::canvas::Canvas;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::home_recent::{Item, SHOWN};

const PAD: f32 = 12.0;
const CHIP_H: f32 = 28.0;
const CHIP_Y: f32 = 26.0;

pub struct Recent {
    items: Vec<Item>,
    /// the history has to be asked for again (the tab was shown, an entry was added, the card was switched on)
    stale: bool,
    /// the first answer has arrived (before it the card shows nothing rather than "nothing copied yet")
    loaded: bool,
    /// per chip: how far it has risen in (0 -> 1)
    rise: [Flow; SHOWN],
    /// per chip: the lit-up amount after a click, and the chip it belongs to
    lit: [Flow; SHOWN],
    touch: [Touch; SHOWN],
    /// card title, "nothing copied yet", "copied" (set from the model sync, in the current language)
    txt: (String, String, String),
}

impl Recent {
    pub fn new() -> Recent {
        Recent {
            items: vec![],
            stale: true,
            loaded: false,
            rise: std::array::from_fn(|_| Flow::new(1.0, Feel::POP)),
            lit: std::array::from_fn(|_| Flow::new(0.0, Feel::FADE.within(520.0))),
            touch: std::array::from_fn(|_| Touch::new()),
            txt: Default::default(),
        }
    }

    pub fn set_texts(&mut self, title: String, empty: String, copied: String) {
        self.txt = (title, empty, copied);
    }

    pub fn mark_stale(&mut self) {
        self.stale = true;
    }

    /// True once (and only once per `mark_stale`): the caller sends `clip_list`.
    pub fn take_stale(&mut self) -> bool {
        std::mem::replace(&mut self.stale, false)
    }

    /// A new `clip_list` answer. Chips whose entry changed rise in again; unchanged ones stay put.
    pub fn set_items(&mut self, items: Vec<Item>) {
        for (i, f) in self.rise.iter_mut().enumerate() {
            if self.loaded && items.get(i).map(|n| n.id) != self.items.get(i).map(|n| n.id) && i < items.len() {
                f.jump(0.0);
                f.to(1.0);
            }
        }
        self.loaded = true;
        self.items = items;
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn id_at(&self, i: usize) -> Option<i64> {
        self.items.get(i).map(|n| n.id)
    }

    /// A chip was clicked: it lights up (the caller sends the copy).
    pub fn flash(&mut self, i: usize) {
        if let Some(f) = self.lit.get_mut(i) {
            f.jump(1.0);
            f.to(0.0);
        }
    }

    pub fn hover(&mut self, i: Option<usize>) {
        for (k, t) in self.touch.iter_mut().enumerate() {
            t.set_hover(i == Some(k));
        }
    }

    pub fn press(&mut self, i: usize, on: bool) {
        if let Some(t) = self.touch.get_mut(i) {
            t.set_press(on);
        }
    }

    /// The chip `i` inside the card `card`.
    pub fn chip_rect(card: Rect, i: usize) -> Rect {
        let w = (card.w - 2.0 * PAD - (SHOWN - 1) as f32 * 8.0) / SHOWN as f32;
        Rect::new(card.x + PAD + i as f32 * (w + 8.0), card.y + CHIP_Y, w, CHIP_H)
    }

    /// The chip under `p` (card-relative hit test).
    pub fn hit(&self, card: Rect, p: (f32, f32)) -> Option<usize> {
        (0..self.items.len().min(SHOWN)).find(|i| Self::chip_rect(card, *i).contains(p.0, p.1))
    }

    pub fn step(&mut self, dt_ms: f64) -> bool {
        let mut busy = false;
        for f in self.rise.iter_mut().chain(self.lit.iter_mut()) {
            f.step(dt_ms);
            busy |= f.moving();
        }
        for t in &mut self.touch {
            busy |= t.step(dt_ms);
        }
        busy
    }

    pub fn paint(&self, cv: &Canvas, th: &Theme, card: Rect) {
        let (title, empty, copied) = (self.txt.0.as_str(), self.txt.1.as_str(), self.txt.2.as_str());
        cv.fill_rr(card, th.m.radius, th.card);
        cv.text(Rect::new(card.x + PAD, card.y + 8.0, card.w - 2.0 * PAD, 14.0), title, Style::new(11.0, 600).ellipsis(), th.dim);
        if self.loaded && self.items.is_empty() {
            cv.text(Rect::new(card.x + PAD, card.y + CHIP_Y, card.w - 2.0 * PAD, CHIP_H), empty, Style::new(12.0, 400), th.faint);
            return;
        }
        for (i, it) in self.items.iter().take(SHOWN).enumerate() {
            let r = Self::chip_rect(card, i);
            let (rise, lit) = (self.rise[i].at().clamp(0.0, 1.2), self.lit[i].at01());
            let t = &self.touch[i];
            // rises 8 px into place while it fades in, overshooting a little like everything else
            let r = r.offset(0.0, (1.0 - rise) * 8.0);
            cv.with_opacity(rise.clamp(0.0, 1.0), |cv| {
                cv.with_scale_about(r.center(), t.press_scale(0.96) * (1.0 + 0.03 * lit), |cv| {
                    cv.fill_rr(r, 8.0, th.raised.mix(th.raised2, t.hover()).mix(th.accent_soft, lit));
                    if lit > 0.01 {
                        cv.stroke_rr(r.inset(0.5), 7.5, 1.0, th.accent.fade(lit));
                    }
                    let label = if lit > 0.35 { copied } else { it.label.as_str() };
                    cv.text(Rect::new(r.x + 10.0, r.y, r.w - 20.0, r.h), label, Style::new(12.0, 500).ellipsis(), th.text);
                });
            });
        }
    }
}

impl Default for Recent {
    fn default() -> Self {
        Recent::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    fn item(id: i64) -> Item {
        Item { id, label: format!("clip {id}"), kind: "text".into() }
    }

    #[test]
    fn the_history_is_asked_for_once_per_reason() {
        let mut r = Recent::new();
        assert!(r.take_stale() && !r.take_stale());
        r.mark_stale();
        assert!(r.take_stale());
    }

    #[test]
    fn a_new_entry_rises_in_and_the_old_ones_stay() {
        let _m = motion_on();
        let mut r = Recent::new();
        r.set_items(vec![item(1), item(2)]);
        assert!(!r.step(4.0), "the first answer is not an animation");
        r.set_items(vec![item(3), item(1), item(2)]);
        assert!(r.step(4.0), "new entries move");
        for _ in 0..600 {
            if !r.step(1000.0 / 240.0) {
                break;
            }
        }
        assert!(!r.step(4.0) && (r.rise[0].at() - 1.0).abs() < 0.01);
        assert_eq!((r.id_at(0), r.len()), (Some(3), 3));
    }

    #[test]
    fn a_click_lights_the_chip_and_it_fades_back() {
        let _m = motion_on();
        let mut r = Recent::new();
        r.set_items(vec![item(1)]);
        r.flash(0);
        assert!(r.lit[0].at() > 0.9 && r.step(4.0));
        for _ in 0..600 {
            if !r.step(1000.0 / 240.0) {
                break;
            }
        }
        assert!(r.lit[0].at() < 0.02);
    }

    #[test]
    fn chips_sit_in_a_row_inside_the_card() {
        let card = Rect::new(0.0, 100.0, 412.0, 62.0);
        let (a, b, c) = (Recent::chip_rect(card, 0), Recent::chip_rect(card, 1), Recent::chip_rect(card, 2));
        assert_eq!((a.x, a.y), (12.0, 126.0));
        assert!((b.x - a.right() - 8.0).abs() < 1e-3 && (c.right() - (412.0 - 12.0)).abs() < 1e-3);
        let mut r = Recent::new();
        r.set_items(vec![item(1), item(2)]);
        assert_eq!(r.hit(card, (a.x + 3.0, a.y + 3.0)), Some(0));
        assert_eq!(r.hit(card, (c.x + 3.0, c.y + 3.0)), None, "no third chip yet");
    }
}
