//! The "computer / player" switch of the Home volume strip: a small chip between the bar and the number. "Computer" (default) = the bar, the
//! wheel, the keys and the mute button drive the Windows master volume; "player" = they drive only the audio session of the player that is
//! shown (ISimpleAudioVolume, `player_audio.rs`). It is the setting `audio.volumeTarget`, so Settings > Sound shows the same switch.
//! The Home model is untouched: in player mode the view adds `"target": "player"` to its `media_volume_get|set` / `media_mute` calls
//! and ignores `live:volume` (that event reports the master volume).
use super::{layout as L, Hit, HomeView};
use crate::ui::liquid::{Feel, Flow};
use crate::ui::tabs::a::icons;
use glasskit::canvas::Canvas;
use glasskit::color::Color;
use glasskit::geom::Rect;
use glasskit::theme::Theme;
use pill_ui_core::panel::a::BackendCall;
use serde_json::{json, Value};

pub const KEY: &str = "audio.volumeTarget";

/// The chip sits right of the bar (the bar was shortened to make room, see `layout::VOL`).
pub fn rect() -> Rect {
    Rect::new(L::VOL.right() + 8.0, L::VOL_ROW.y + 7.0, 26.0, 24.0)
}

pub struct Target {
    player: bool,
    /// 0 = computer, 1 = player: the two glyphs cross-fade
    pub(super) f: Flow,
}

impl Target {
    pub fn new() -> Target {
        Target { player: false, f: Flow::new(0.0, Feel::SNAP) }
    }

    pub fn is_player(&self) -> bool {
        self.player
    }

    /// Reads the setting out of a `settings_get` answer; true when the mode changed.
    pub fn load(&mut self, settings: &Value) -> bool {
        let player = settings.get(KEY).and_then(Value::as_str) == Some("player");
        std::mem::replace(&mut self.player, player) != player
    }

    /// The click: flips the mode at once (the glyph does not wait for the round trip) and returns the setting write.
    pub fn toggle(&mut self) -> BackendCall {
        self.player = !self.player;
        BackendCall::new("settings_set", json!({ "patch": { KEY: if self.player { "player" } else { "master" } } }))
    }

    /// Per frame: the glyph flow. True while it moves.
    pub fn step(&mut self, dt_ms: f64) -> bool {
        self.f.to(self.player as u8 as f32);
        self.f.step(dt_ms);
        self.f.moving()
    }

    /// In player mode the volume calls carry `"target": "player"`, so the backend drives the player's session.
    pub fn retarget(&self, calls: &mut [BackendCall]) {
        if !self.player {
            return;
        }
        for c in calls.iter_mut().filter(|c| matches!(c.cmd, "media_volume_get" | "media_volume_set" | "media_mute")) {
            match &mut c.args {
                Value::Object(m) => {
                    m.insert("target".into(), json!("player"));
                }
                a => *a = json!({ "target": "player" }),
            }
        }
    }

    /// `live:volume` reports the master volume: meaningless for the player's bar.
    pub fn ignores_event(&self, name: &str) -> bool {
        self.player && name == "live:volume"
    }
}

impl HomeView {
    pub(super) fn paint_target(&self, cv: &Canvas, th: &Theme, ox: f32, oy: f32) {
        let r = rect().offset(ox, oy);
        let h = self.hover_amount(Hit::Target);
        // the same fill as the output-device chip
        let (c0, c1) = if th.dark { (Color::rgb8(0x1d, 0x1d, 0x1d), Color::rgb8(0x26, 0x26, 0x26)) } else { (Color::rgb8(0xd6, 0xd6, 0xd6), Color::rgb8(0xd6, 0xd6, 0xd6)) };
        let ink = th.dim.mix(th.text, h);
        let k = self.target.f.at01();
        let press = self.hov.get(&Hit::Target).map_or(1.0, |i| i.press_scale(0.94));
        cv.with_scale_about(r.center(), press, |cv| {
            cv.fill_rr(r, 9.0, c0.mix(c1, h));
            let ic = r.centered(16.0, 16.0);
            cv.with_opacity(1.0 - k, |cv| icons::draw(cv, "monitor", ic, ink, 1.6));
            cv.with_opacity(k, |cv| icons::draw(cv, "note2", ic, ink, 1.6));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_fits_between_the_bar_and_the_number() {
        let r = rect();
        assert!(r.x >= L::VOL.right() + 6.0 && r.right() + 6.0 <= L::VOL_NUM.x, "{r:?}");
        assert!(r.y >= L::VOL_ROW.y && r.bottom() <= L::VOL_ROW.bottom());
    }

    #[test]
    fn load_reads_the_setting_and_reports_a_change() {
        let mut t = Target::new();
        assert!(!t.load(&json!({})), "default = computer, nothing changed");
        assert!(t.load(&json!({ KEY: "player" })) && t.is_player());
        assert!(!t.load(&json!({ KEY: "player" })));
        assert!(t.load(&json!({ KEY: "master" })) && !t.is_player());
    }

    #[test]
    fn toggle_flips_and_writes_the_setting() {
        let mut t = Target::new();
        let c = t.toggle();
        assert!(t.is_player());
        assert_eq!((c.cmd, c.args), ("settings_set", json!({ "patch": { KEY: "player" } })));
        assert_eq!(t.toggle().args, json!({ "patch": { KEY: "master" } }));
    }

    #[test]
    fn player_mode_tags_only_the_volume_calls() {
        let mut calls = vec![
            BackendCall::bare("media_volume_get"),
            BackendCall::new("media_volume_set", json!({ "v": 30 })),
            BackendCall::new("media_mute", json!({ "muted": true })),
            BackendCall::new("media_select", json!({ "id": "x" })),
        ];
        let mut t = Target::new();
        t.retarget(&mut calls);
        assert_eq!(calls[1].args, json!({ "v": 30 }), "computer mode leaves the calls alone");
        t.player = true;
        t.retarget(&mut calls);
        assert_eq!(calls[0].args, json!({ "target": "player" }));
        assert_eq!(calls[1].args, json!({ "v": 30, "target": "player" }));
        assert_eq!(calls[2].args, json!({ "muted": true, "target": "player" }));
        assert_eq!(calls[3].args, json!({ "id": "x" }));
        assert!(t.ignores_event("live:volume") && !t.ignores_event("media:changed"));
        t.player = false;
        assert!(!t.ignores_event("live:volume"));
    }
}
