//! The "Frame cost" line of Look > Power: what a frame costs now (from the `PILL_PERF` meter, switched on only while the Look page is open,
//! see `nav::page_shown`) and the fps cap with its reason (`host::energy`). Read-only; the page's 1 s ticker repaints it.
use super::flow::{from_to, tick};
use super::page::RowRt;
use super::txt;
use super::rows::*;
use super::SettingsView;
use crate::ui::host::{energy, perf};
use crate::ui::liquid::{Feel, Flow};
use glasskit::geom::Rect;
use std::time::Duration;

/// A sample older than this means the meter saw no wake-up at all: the island is idle.
const STALE: Duration = Duration::from_secs(6);

pub struct EnergyData {
    /// the text fades in (from 0.35) whenever the cap or its reason changes
    pub fade: Flow,
    seen: String,
}

impl Default for EnergyData {
    fn default() -> Self {
        EnergyData { fade: Flow::new(1.0, Feel::FADE), seen: String::new() }
    }
}

impl EnergyData {
    pub fn step(&mut self, dt: f32) -> bool {
        tick(&mut self.fade, dt)
    }
}

impl SettingsView {
    pub(super) fn w_energy_cost(&mut self, d: &Dc, r: &RowRt, full: Rect, paint: bool) -> f32 {
        let (x0, w) = (full.x + PAD_X, full.w - 2.0 * PAD_X);
        let m = self.row_metrics_plain(d, r, w);
        let sample = perf::last().map(|s| if s.at.elapsed() > STALE { perf::Sample { island_fps: 0.0, ..s } } else { s });
        let (measured, cap) = energy::cost_lines(d.lang == pill_ui_core::settings::tr::Lang::Ru, &energy::verdict(), sample.as_ref());
        if paint && cap != self.data.energy.seen {
            let first = self.data.energy.seen.is_empty();
            self.data.energy.seen = cap.clone();
            if !first {
                from_to(&mut self.data.energy.fade, 0.35, 1.0, d.reduced);
            }
        }
        let st = st_hint();
        let top = full.y + 11.0 + m.title_h + 2.0;
        let hm = txt::wrapped_h(d.cv, &measured, st, w, 16.0);
        let hc = txt::wrapped_h(d.cv, &cap, st, w, 16.0);
        if paint {
            self.paint_label(d, r, &m, x0, full.y);
            let (dim, text) = (d.p.dim, d.p.text);
            d.cv.with_opacity(self.data.energy.fade.at01(), |cv| {
                txt::para(cv, Rect::new(x0, top, w, hm), &measured, st, dim);
                txt::para(cv, Rect::new(x0, top + hm + 2.0, w, hc), &cap, st, text);
            });
        }
        11.0 + m.title_h + 2.0 + hm + 2.0 + hc + 11.0
    }
}
