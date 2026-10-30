//! `Shell::view()`: the deterministic description of one frame for the renderer (body rect, radii, opacities, the live row, the liquid
//! edge, the tab/header state). Pure read of the model; call after `tick`.

use crate::shell::io::{Backend, Clock};
use crate::shell::liquid::{SPAN, STRIP, UP};
use crate::shell::motion::{GHOST_LIFETIME_MS, PANEL_IN};
use crate::shell::types::{BodyView, ChromeView, Ghost, LiveView, PanelView, Phase, StripRect, TabView, View};

use super::capsule::{PillState, Shell, CARD_FRAME_H, COL_FRAME_H, COL_FRAME_W, EXP_FRAME_H, EXP_FRAME_W};

impl<B: Backend, C: Clock> Shell<B, C> {
    /// The capsule's high-level state.
    pub fn phase(&self) -> Phase {
        if self.hidden {
            return Phase::Hidden;
        }
        if self.away {
            return Phase::AutoHidden;
        }
        if self.state == PillState::Expanded {
            return if self.sp.busy || self.exp_wait.is_some() { Phase::Expanding } else { Phase::Expanded };
        }
        if self.cards.has_current() {
            return Phase::Cards;
        }
        if self.sp.busy && self.frame_exp {
            return Phase::Collapsing;
        }
        if self.hovering {
            Phase::Hover
        } else {
            Phase::Collapsed
        }
    }

    /// The interactive region last reported to the backend with `pill_rect`.
    pub fn hit_rect(&self) -> crate::shell::types::Rect {
        self.last_rect.unwrap_or_default()
    }

    pub fn view(&self) -> View {
        let s = self.scale();
        let pal = self.palette();
        let theme = self.s.resolved_theme(self.env.os_prefers_light);
        let expanded = self.state == PillState::Expanded;
        let ty = self.current_ty();
        let p = self.paint;
        let body = BodyView {
            left: p.left,
            top: ty,
            width: p.w,
            height: p.h,
            radius_top: p.t,
            radius_bottom: p.r,
            fill: pal.bg.with_alpha(self.body_alpha() as f32),
            flare_radius: 14.0 * s,
            flare_visible: !expanded && !self.capsule(),
            shadow: theme == crate::shell::color::Theme::Light,
            away: self.away,
            panel_left: p.panel_left,
        };
        let content = self.current_content();
        let ghost = self.ghost.as_ref().and_then(|(c, t0)| {
            let age = self.now - t0;
            ((0.0..GHOST_LIFETIME_MS).contains(&age) && !self.reduce()).then(|| Ghost { content: c.clone(), age_ms: age })
        });
        let live = LiveView {
            visible: !expanded,
            content,
            tall: self.root_tall,
            swap_epoch: self.swap_epoch,
            swap_age_ms: self.now - self.swap_at,
            ghost,
            rolls: self.rolls.iter().filter(|(_, t0)| self.now - t0 < crate::shell::motion::DIGIT_ROLL_CLEANUP_MS).map(|(r, t0)| (r.clone(), self.now - t0)).collect(),
            entering: self.entering.iter().filter(|(_, _, t0)| self.now - t0 < crate::shell::motion::RESHAPE_POP.dur_ms).map(|(r, i, t0)| (*r, *i, self.now - t0)).collect(),
        };
        let mut tabs_clone = self.tabs.clone();
        let panel = PanelView { visible: expanded, height: tabs_clone.body_height(EXP_FRAME_H, super::capsule::EXP_TOP, super::capsule::WAVE_PAD).0, shown_age_ms: self.now - self.panel_shown_at };
        let ctx = self.liq_ctx();
        let edge = self.liq.edge_view(&ctx);
        let strip = StripRect { x: p.left + p.w / 2.0 - SPAN * s / 2.0, y: ty + p.h - UP * s, w: SPAN * s, h: (STRIP + UP) * s };
        let tabs = self
            .tabs
            .list
            .iter()
            .map(|t| TabView { id: t.spec.id.clone(), title: t.spec.title.clone(), order: t.spec.order, hidden: t.spec.hidden, off: t.off, active: self.tabs.cur.as_deref() == Some(&t.spec.id), badge: t.badge })
            .collect();
        let wave_color = if self.wave_c.is_empty() { None } else { crate::shell::color::parse_css(&self.wave_c, &pal) };
        View {
            phase: self.phase(),
            scale: s,
            theme,
            palette: pal,
            shape: self.s.shape,
            reduced: self.reduce(),
            lang: self.lang,
            frame: if self.frame_exp {
                (EXP_FRAME_W, EXP_FRAME_H)
            } else if self.carding {
                (COL_FRAME_W, CARD_FRAME_H)
            } else {
                (COL_FRAME_W, COL_FRAME_H)
            },
            body,
            live,
            panel,
            edge,
            strip,
            tabs,
            tab_dir: self.tabs.dir,
            current_tab: self.tabs.cur.clone(),
            chrome: ChromeView { vpn: self.s.vpn_country.clone(), pinned: self.s.pinned, settings_current: self.tabs.cur.as_deref() == Some("settings") },
            wave_color,
            hit_rect: self.hit_rect(),
            panel_anim: PANEL_IN,
            fps_cap: self.s.fps_cap,
            wants_frames: self.wants_frames(),
        }
    }
}
