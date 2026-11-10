//! The parts of the Home tab as the user arranged them (Settings > Modules > What is on Home): which are on, where they are right now,
//! how they are painted and where a click lands. The player, the volume row and the launcher strip are still drawn by their classic
//! code (`paint.rs`) in their own coordinates; the slots only say how far each one is shifted, and a pointer is shifted back the same way.
use super::*;

impl HomeView {
    /// Retarget the slots to the parts that are on. Called after every model input, never from `draw`.
    pub(super) fn sync_slots(&mut self, cx: &mut Cx) {
        if !self.cfg_seen {
            return;
        }
        let on = self.model.parts_on();
        self.slots.set_plan(hmod::plan(&on));
        let t = tr(cx);
        self.recent.set_texts(t.t("Недавно скопировано", "Recently copied"), t.t("Пока ничего не скопировано", "Nothing copied yet"), t.t("Скопировано", "Copied"));
        // the output-device list belongs to the volume row: it closes when the row goes
        if self.menu_open && !on.0[2] {
            self.act(cx, Action::DevClose);
        }
        if on.0[1] && self.base.is_shown() && self.recent.take_stale() {
            self.run(cx, vec![home_recent::list_call()]);
        }
    }

    /// Paint `f` as part `i`: faded and grown by how far it has appeared (a part on its way out keeps its last place). Nothing when gone.
    fn scoped(&self, cv: &Canvas, i: usize, ox: f32, oy: f32, f: impl FnOnce(&Canvas)) {
        let a = self.slots.alpha(i);
        let Some(r) = self.slots.rect(i).filter(|_| a > 0.004) else { return };
        let g = self.slots.grow(i);
        let c = (r.x + r.w / 2.0 + ox, r.y + r.h / 2.0 + oy);
        let grown = |cv: &Canvas| {
            if g < 0.999 {
                cv.with_scale_about(c, g, f)
            } else {
                f(cv)
            }
        };
        if a < 0.999 {
            cv.with_opacity(a, grown)
        } else {
            grown(cv)
        }
    }

    pub(super) fn paint_parts(&self, cv: &Canvas, th: &Theme, ox: f32, oy: f32) {
        if self.slots.plan().is_none() {
            return;
        }
        let (px, py) = self.slots.shift(0);
        self.scoped(cv, 0, ox, oy, |cv| self.paint_player(cv, th, ox + px, oy + py));
        if let Some(r) = self.slots.rect(1) {
            self.scoped(cv, 1, ox, oy, |cv| self.recent.paint(cv, th, Rect::new(r.x + ox, r.y + oy, r.w, r.h)));
        }
        let (vx, vy) = self.slots.shift(2);
        self.scoped(cv, 2, ox, oy, |cv| self.paint_volume(cv, th, ox + vx, oy + vy));
        for (i, t) in self.v.tiles.iter().enumerate().take(8) {
            if let Some(r) = self.slots.rect(tile_part(i)) {
                self.scoped(cv, tile_part(i), ox, oy, |cv| self.paint_tile(cv, th, Rect::new(r.x + ox, r.y + oy, r.w, r.h), Hit::Tile(i), t));
            }
        }
        let (sx, sy) = self.slots.shift(hmod::STRIP);
        self.scoped(cv, hmod::STRIP, ox, oy, |cv| self.paint_launcher(cv, th, ox + sx, oy + sy));
        let (mx, my) = self.menu_shift();
        self.paint_menu(cv, th, ox + mx, oy + my);
    }

    /// Where the output-device list sits: above the volume row wherever that is now, pushed down when the row is too near the top for it.
    pub(super) fn menu_shift(&self) -> (f32, f32) {
        let (dx, dy) = self.slots.shift(2);
        let top = layout::menu(self.v.devices.items.len()).y + dy;
        (dx, dy + (-top).max(0.0))
    }

    pub(super) fn menu_rect(&self) -> Rect {
        let (dx, dy) = self.menu_shift();
        layout::menu(self.v.devices.items.len()).offset(dx, dy)
    }

    /// The part under the tab-local point `p`, the way `hit` used to answer for the classic layout.
    pub(super) fn hit_parts(&self, p: Pt) -> Option<Hit> {
        let (x, y) = p;
        if self.menu_open {
            let m = self.menu_rect();
            if m.contains(x, y) {
                let i = ((y - m.y - 5.0 + self.menu_scroll.offset()) / layout::MENU_ROW).floor();
                return (i >= 0.0 && (i as usize) < self.v.devices.items.len() && y >= m.y + 5.0 && y < m.bottom() - 5.0).then_some(Hit::MenuItem(i as usize));
            }
        }
        let sp = self.slots.to_classic(hmod::STRIP, p);
        if self.slots.live(hmod::STRIP) && (layout::STRIP.contains(sp.0, sp.1) || self.apps.iter().any(|a| a.rect.contains(sp.0, sp.1))) {
            for a in self.apps.iter().rev() {
                let badge = layout::app_x(a.rect);
                if self.hover_amount(Hit::App(a.id)) > 0.3 && !self.v.launcher.reordering && badge.contains(sp.0, sp.1) {
                    return Some(Hit::AppX(a.id));
                }
            }
            for a in self.apps.iter().rev() {
                if a.rect.contains(sp.0, sp.1) && layout::STRIP.contains(sp.0, sp.1) {
                    return Some(Hit::App(a.id));
                }
            }
            let add = Rect::new(self.add_x, layout::add(0).y, layout::ADD, layout::ADD);
            if add.contains(sp.0, sp.1) && layout::STRIP.contains(sp.0, sp.1) {
                return Some(Hit::Add);
            }
        }
        let pp = self.slots.to_classic(0, p);
        if self.slots.live(0) {
            if self.v.player.none {
                if layout::PLAYER.contains(pp.0, pp.1) {
                    return None;
                }
            } else {
                if hit_rounded(layout::MAIN, 18.0, pp) {
                    return Some(Hit::Main);
                }
                if hit_rounded(layout::PREV, 14.0, pp) {
                    return Some(Hit::Prev);
                }
                if hit_rounded(layout::NEXT, 14.0, pp) {
                    return Some(Hit::Next);
                }
                if self.v.player.source_multi && layout::SRC.contains(pp.0, pp.1) {
                    return Some(Hit::Src);
                }
                if self.v.player.seek.enabled && layout::SEEK.contains(pp.0, pp.1) {
                    return Some(Hit::Seek);
                }
            }
        }
        let vp = self.slots.to_classic(2, p);
        if self.slots.live(2) {
            if layout::MUTE.contains(vp.0, vp.1) {
                return Some(Hit::Mute);
            }
            if Rect::new(layout::VOL.x, layout::VOL.y - 4.0, layout::VOL.w, layout::VOL.h + 8.0).contains(vp.0, vp.1) {
                return Some(Hit::Vol);
            }
            if layout::DEV.contains(vp.0, vp.1) {
                return Some(Hit::Dev);
            }
        }
        if let (true, Some(r)) = (self.slots.live(1), self.slots.rect(1)) {
            if let Some(i) = self.recent.hit(Rect::new(r.x, r.y, r.w, r.h), p) {
                return Some(Hit::Recent(i));
            }
        }
        (0..self.v.tiles.len().min(8)).find(|&i| self.slots.live(tile_part(i)) && self.slots.rect(tile_part(i)).is_some_and(|r| r.contains(x, y))).map(Hit::Tile)
    }

    /// A tab-local x as the player's classic layout sees it (the seek bar), and as the volume row's does (the slider).
    pub(super) fn player_x(&self, x: f32) -> f32 {
        x - self.slots.shift(0).0
    }
    pub(super) fn volume_x(&self, x: f32) -> f32 {
        x - self.slots.shift(2).0
    }

    /// The answers and events of the "recently copied" card; true when the message was its own.
    pub(super) fn recent_message(&mut self, cx: &mut Cx, inc: &Incoming) -> bool {
        match inc {
            Incoming::Event { name, .. } if name == "clip:added" => {
                self.recent.mark_stale();
                true
            }
            Incoming::Result { cmd: "clip_list", result, .. } => {
                if let Ok(v) = result {
                    self.recent.set_items(home_recent::items(v, tr(cx)));
                }
                true
            }
            Incoming::Result { cmd: "clip_paste", .. } => true,
            _ => false,
        }
    }
}
