//! The Home tab's controls for UI Automation and keyboard mode (`View::controls`), built from the rectangles `hit_parts` tests the pointer
//! against: the player's three buttons and its seek bar, the volume row (mute, slider, device), the quick tiles and the launcher icons.
//! A press on one of them is a click at its centre, a slider moves by a click at its fraction (`island/view_kbd.rs`), so nothing here
//! acts: it only says what is where.
use super::*;
use crate::ui::a11y::nav::{Control, Role, Value as CV};

const PREV: u32 = 1;
const MAIN: u32 = 2;
const NEXT: u32 = 3;
const SEEK: u32 = 4;
const SRC: u32 = 5;
const MUTE: u32 = 6;
const VOL: u32 = 7;
const DEV: u32 = 8;
const TILE0: u32 = 20;
const APP0: u32 = 100;

impl HomeView {
    pub(super) fn a11y_controls(&self) -> Vec<Control> {
        let mut out = Vec::new();
        if self.menu_open {
            return out; // the device menu is open over everything: the pointer sees only it
        }
        let pl = &self.v.player;
        if self.slots.live(0) && !pl.none && pl.visible {
            let (dx, dy) = self.slots.shift(0);
            let g = &self.pl.geo;
            let at = |r: Rect| r.offset(dx, dy);
            out.push(Control::new(PREV, pl.prev_title.as_str(), Role::Button, at(g.prev)));
            out.push(Control::new(MAIN, pl.play_title.as_str(), Role::Button, at(g.main)));
            out.push(Control::new(NEXT, pl.next_title.as_str(), Role::Button, at(g.next)));
            if pl.source_multi {
                out.push(Control::new(SRC, pl.source_title.as_str(), Role::Button, at(g.src)));
            }
            if pl.seek.enabled {
                let name = if pl.title.is_empty() { pl.seek_label.clone() } else { format!("{} {}", pl.seek_label, pl.title).trim().to_string() };
                out.push(Control::new(SEEK, name, Role::Slider, at(g.seek)).with(CV::Range { min: 0.0, max: pl.seek.max_ms as f64, cur: pl.seek.value_ms, step: 5000.0 }));
            }
        }
        if self.slots.live(2) {
            let (dx, dy) = self.slots.shift(2);
            let v = &self.v.volume;
            out.push(Control::new(MUTE, v.mute_title.as_str(), Role::Button, layout::MUTE.offset(dx, dy)).with(CV::Toggled(v.muted)));
            let vr = layout::VOL;
            out.push(Control::new(VOL, v.label.as_str(), Role::Slider, Rect::new(vr.x, vr.y - 4.0, vr.w, vr.h + 8.0).offset(dx, dy)).with(CV::Range { min: 0.0, max: 100.0, cur: v.shown, step: 5.0 }));
            out.push(Control::new(DEV, self.v.devices.button_title.as_str(), Role::Button, layout::DEV.offset(dx, dy)));
        }
        for (i, t) in self.v.tiles.iter().enumerate().take(8) {
            if let (true, Some(r)) = (self.slots.live(tile_part(i)) && !t.soon, self.slots.rect(tile_part(i))) {
                out.push(Control::new(TILE0 + i as u32, t.label.as_str(), Role::Button, r));
            }
        }
        if self.slots.live(hmod::STRIP) && self.v.launcher.visible {
            let (dx, dy) = self.slots.shift(hmod::STRIP);
            for (i, a) in self.apps.iter().enumerate() {
                let name = self.v.launcher.tiles.iter().find(|t| t.id == a.id).map(|t| t.tooltip.lines().next().unwrap_or("").to_string()).unwrap_or_default();
                out.push(Control::new(APP0 + i as u32, name, Role::ListItem, a.rect.offset(dx, dy)).with(CV::Selected(false)));
            }
        }
        out
    }
}
