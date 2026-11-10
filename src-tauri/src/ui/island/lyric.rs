//! The lyric line of the music row: when the sung line changes the old one rises out and the new one rises in from below, both on one spring
//! (`Feel::FLOW`, so it lands with a small drop-of-water overshoot). The spring follows the line counter `seq` of `Elem::Lyric`; it starts AT
//! the current counter (a row that appears does not animate), reduced motion lands at once (`Gpu::follow`).
use super::gpu::Gpu;
use crate::ui::liquid::Feel;
use glasskit::geom::Rect;
use glasskit::text::Style;
use glasskit::{Canvas, Color};

/// How far (css px) a line travels while it fades.
pub const RISE: f32 = 9.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slide {
    pub prev_alpha: f32,
    pub prev_dy: f32,
    pub cur_alpha: f32,
    pub cur_dy: f32,
}

/// `f` = how far the change has come: 0 = the old line, 1 = the new one (the spring may pass 1 a little).
pub fn slide(f: f32) -> Slide {
    let k = f.clamp(0.0, 1.0);
    Slide { prev_alpha: 1.0 - k, prev_dy: -RISE * f.max(0.0), cur_alpha: k, cur_dy: RISE * (1.0 - f).max(-0.4) }
}

#[allow(clippy::too_many_arguments)]
pub fn paint(cv: &Canvas, gpu: &mut Gpu, key: &str, (prev, text, seq): (&str, &str, u64), r: Rect, (st, c): (Style, Color), t: f64, reduced: bool) {
    let p = gpu.follow(key, seq as f32, Feel::FLOW, t, reduced);
    let s = slide(p - (seq as f32 - 1.0));
    cv.with_clip(r, |cv| {
        if s.prev_alpha > 0.01 && !prev.is_empty() {
            cv.with_opacity(s.prev_alpha, |cv| cv.text(r.offset(0.0, s.prev_dy), prev, st, c));
        }
        cv.with_opacity(s.cur_alpha, |cv| cv.text(r.offset(0.0, s.cur_dy), text, st, c));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;

    #[test]
    fn the_old_line_leaves_as_the_new_one_arrives() {
        let (a, b) = (slide(0.0), slide(1.0));
        assert_eq!((a.prev_alpha, a.cur_alpha, a.prev_dy, a.cur_dy), (1.0, 0.0, 0.0, RISE));
        assert_eq!((b.prev_alpha, b.cur_alpha, b.cur_dy), (0.0, 1.0, 0.0));
        let m = slide(0.5);
        assert!((m.prev_alpha + m.cur_alpha - 1.0).abs() < 1e-6 && m.prev_dy < 0.0 && m.cur_dy > 0.0);
        let over = slide(1.15);
        assert_eq!(over.cur_alpha, 1.0);
        assert!(over.cur_dy < 0.0, "the spring overshoots a little and settles back");
    }

    #[test]
    fn a_new_line_flows_in_on_the_spring_and_a_fresh_row_does_not_animate() {
        let _m = motion_on();
        let mut g = Gpu::new();
        let at = |g: &mut Gpu, seq: u64, t: f64| g.follow("r#0:ly", seq as f32, Feel::FLOW, t, false) - (seq as f32 - 1.0);
        assert_eq!(at(&mut g, 4, 0.0), 1.0, "first sight: already there");
        let f0 = at(&mut g, 5, 16.0);
        assert!(f0 > 0.0 && f0 < 0.2, "starts softly: {f0}");
        assert!(g.gliding());
        for i in 2..80 {
            at(&mut g, 5, i as f64 * 16.0);
        }
        assert_eq!(at(&mut g, 5, 1400.0), 1.0);
        assert!(!g.gliding(), "settled: the window idles");
    }
}
