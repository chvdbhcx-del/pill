//! Provider plumbing: the [`Provider`] trait, the [`Hub`] that routes backend events to the
//! providers that subscribe to them, runs the quiet-hours filter and feeds the row [`Board`].
//!
//! Usage by the host (the future native shell):
//! 1. `let mut hub = Hub::standard(Box::new(my_agents_io));` then run every `hub.boot()` [`Query`]
//!    against the backend and feed the answer back with [`Hub::dispatch`] under `query.as_event`
//!    (after `query.delay_ms`).
//! 2. For every backend event (`bus::subscribe`) call [`Hub::dispatch`] with the event name and JSON
//!    payload; the broadcast events `settings:changed`, `pill:hide` and `pill:show` go to every
//!    subscriber too. Execute the returned [`LiveCmd`]s.
//! 3. While `hub.tick_ms(now)` is `Some(ms)` call [`Hub::tick`] at least that often (never slower; never throttle it).
//! 4. Every 20 s call [`Hub::sweep_quiet`] (drops muted chips/pulses when quiet hours begin).

use std::any::Any;

use serde_json::Value;

use super::agents::{AgentsIo, AgentsLive};
use super::board::{filter_quiet, Board, QUIET_SRC};
use super::capture::{CaptureCards, RecLive};
use super::env::Env;
use super::events::{CopyLive, KbdLive};
use super::ext::{NotificationLive, ObsLive, SteamLive, UninstallLive};
use super::media::MediaLive;
use super::model::{LiveCmd, Query};
use super::plus::{BatteryLive, CalendarLive, FileDoneLive, MailLive, UsbLive};
use super::shelf::ShelfLive;
use super::sysmon::SysmonLive;
use super::system::{DownloadLive, PrivacyLive, UpdateLive, VolumeLive};
use super::timers::TimerLive;
use super::voice::VoiceLive;
use super::whatsnew::WhatsNewLive;

/// One live-event producer's UI side.
pub trait Provider: Any {
    fn name(&self) -> &'static str;
    /// Backend event names this provider consumes (including `settings:changed`, `pill:hide`, `pill:show` when it cares).
    fn events(&self) -> &'static [&'static str];
    /// Questions to ask the backend once at start (events sent before the UI existed are gone).
    fn boot(&self) -> Vec<Query> {
        Vec::new()
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, payload: &Value, now: i64) -> Vec<LiveCmd>;
    /// Clock-driven work (timers, delayed cards). Called by [`Hub::tick`].
    fn on_tick(&mut self, _env: &dyn Env, _now: i64) -> Vec<LiveCmd> {
        Vec::new()
    }
    /// How often `on_tick` must run right now, `None` = nothing to tick.
    fn tick_ms(&self, _now: i64) -> Option<u32> {
        None
    }
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub struct Hub {
    providers: Vec<Box<dyn Provider>>,
    board: Board,
}

impl Hub {
    pub fn new(providers: Vec<Box<dyn Provider>>) -> Hub {
        Hub { providers, board: Board::new() }
    }

    /// Every provider of the app.
    pub fn standard(agents_io: Box<dyn AgentsIo>) -> Hub {
        Hub::new(vec![
            Box::new(VolumeLive::default()),
            Box::new(PrivacyLive::default()),
            Box::new(DownloadLive::default()),
            Box::new(UpdateLive),
            Box::new(KbdLive),
            Box::new(CopyLive),
            Box::new(ObsLive::default()),
            Box::new(SteamLive),
            Box::new(NotificationLive),
            Box::new(UninstallLive),
            Box::new(FileDoneLive::default()),
            Box::new(UsbLive),
            Box::new(BatteryLive),
            Box::new(CalendarLive),
            Box::new(MailLive),
            Box::new(AgentsLive::new(agents_io)),
            Box::new(RecLive::default()),
            Box::new(CaptureCards),
            Box::new(VoiceLive::default()),
            Box::new(SysmonLive::default()),
            Box::new(WhatsNewLive),
            Box::new(MediaLive::default()),
            Box::new(TimerLive::default()),
            Box::new(ShelfLive::default()),
        ])
    }

    pub fn boot(&self) -> Vec<Query> {
        self.providers.iter().flat_map(|p| p.boot()).collect()
    }

    /// Route one event; the commands come back quiet-filtered and the row commands are already applied to the board.
    pub fn dispatch(&mut self, env: &dyn Env, name: &str, payload: &Value, now: i64) -> Vec<LiveCmd> {
        let mut out = Vec::new();
        for p in self.providers.iter_mut() {
            if p.events().contains(&name) {
                out.extend(p.on_event(env, name, payload, now));
            }
        }
        self.finish(env, out, now)
    }

    pub fn tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        let mut out = Vec::new();
        for p in self.providers.iter_mut() {
            if p.tick_ms(now).is_some() {
                out.extend(p.on_tick(env, now));
            }
        }
        self.finish(env, out, now)
    }

    /// The smallest tick period any provider wants right now.
    pub fn tick_ms(&self, now: i64) -> Option<u32> {
        self.providers.iter().filter_map(|p| p.tick_ms(now)).min()
    }

    /// Quiet hours began: clear the muted rows and reactions (`QUIET_SRC.forEach(delete + unreact)`).
    pub fn sweep_quiet(&mut self, env: &dyn Env) -> Vec<LiveCmd> {
        if !env.quiet() {
            return Vec::new();
        }
        self.board.sweep_quiet();
        QUIET_SRC.iter().map(|s| LiveCmd::unreact(s)).collect()
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    pub fn board_mut(&mut self) -> &mut Board {
        &mut self.board
    }

    /// A provider by type (the volume wheel, the shelf drop card, the Note tab's visibility, ...).
    pub fn get_mut<T: Provider>(&mut self) -> Option<&mut T> {
        self.providers.iter_mut().find_map(|p| p.as_any_mut().downcast_mut::<T>())
    }

    fn finish(&mut self, env: &dyn Env, cmds: Vec<LiveCmd>, now: i64) -> Vec<LiveCmd> {
        let cmds = filter_quiet(cmds, env.quiet());
        for c in &cmds {
            self.board.apply(c, now);
        }
        cmds
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::agents::NoAgentTools;
    use crate::live::env::MemEnv;
    use crate::live::model::{Elem, RowSpec};
    use serde_json::json;

    const T0: i64 = 1_700_000_000_000;

    #[test]
    fn standard_hub_has_unique_names_and_boot_queries() {
        let h = Hub::standard(Box::new(NoAgentTools));
        let mut names: Vec<&str> = h.providers.iter().map(|p| p.name()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n, "duplicate provider name");
        let boot: Vec<&str> = h.boot().iter().map(|q| q.cmd).collect();
        for c in ["livesys_privacy", "agents_list", "sysmon_get", "capture_settings", "voice_state", "whatsnew_check", "timer_state", "media_state"] {
            assert!(boot.contains(&c), "missing boot query {c}: {boot:?}");
        }
        let wn = h.boot().into_iter().find(|q| q.cmd == "whatsnew_check").unwrap();
        assert_eq!(wn.delay_ms, 4000);
    }

    #[test]
    fn dispatch_routes_and_feeds_the_board() {
        let mut h = Hub::standard(Box::new(NoAgentTools));
        let env = MemEnv::default();
        let cmds = h.dispatch(&env, "kbd:layout", &json!({"lang": "EN", "name": "English"}), T0);
        assert!(matches!(&cmds[0], LiveCmd::SetRow(r) if r.id == "kbd"));
        assert_eq!(h.board_mut().top(T0 + 100).unwrap().id, "kbd");
        assert!(h.board_mut().top(T0 + 1401).is_none());
        // an event nobody consumes
        assert!(h.dispatch(&env, "no:such", &Value::Null, T0).is_empty());
    }

    #[test]
    fn quiet_hours_filter_runs_in_dispatch() {
        let mut h = Hub::standard(Box::new(NoAgentTools));
        let mut env = MemEnv::default();
        env.quiet = true;
        let cmds = h.dispatch(&env, "kbd:layout", &json!({"lang": "EN"}), T0);
        assert_eq!(cmds, vec![LiveCmd::clear("kbd")]);
        assert!(h.board().is_empty());
        // a download card is dropped, a timer card gets through
        let cmds = h.dispatch(&env, "live:download", &json!({"state": "done", "id": "a", "name": "f.zip", "bytes": 10, "path": "C:\\d\\f.zip"}), T0);
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::Toast(_))), "{cmds:?}");
        let cmds = h.dispatch(&env, "remind:due", &json!({"id": 3, "text": "call", "kind": "reminder", "late": false, "repeat": "none"}), T0);
        assert!(cmds.iter().any(|c| matches!(c, LiveCmd::Toast(t) if t.id == "remind-3")), "{cmds:?}");
    }

    #[test]
    fn sweep_only_while_quiet() {
        let mut h = Hub::standard(Box::new(NoAgentTools));
        let mut env = MemEnv::default();
        h.board_mut().set(RowSpec::new("kbd", 90).with(Elem::Flex), T0);
        assert!(h.sweep_quiet(&env).is_empty());
        assert_eq!(h.board().len(), 1);
        env.quiet = true;
        let cmds = h.sweep_quiet(&env);
        assert_eq!(cmds.len(), QUIET_SRC.len());
        assert!(h.board().is_empty());
    }

    #[test]
    fn tick_ms_is_the_minimum_and_get_mut_finds_providers() {
        let mut h = Hub::standard(Box::new(NoAgentTools));
        let env = MemEnv::default();
        assert_eq!(h.tick_ms(T0), None);
        h.dispatch(&env, "rec:state", &json!({"recording": true, "startedAt": T0}), T0);
        assert_eq!(h.tick_ms(T0 + 10), Some(250)); // recording capsule: 4 Hz
        h.dispatch(&env, "live:obs", &json!({"recording": true, "streaming": false, "recMs": 0, "streamMs": 0}), T0);
        // obs is off by default: still 250
        assert_eq!(h.tick_ms(T0 + 10), Some(250));
        assert!(h.get_mut::<VolumeLive>().is_some());
        assert!(h.get_mut::<ShelfLive>().is_some());
    }

    #[test]
    fn every_contract_event_of_the_live_layer_has_a_consumer() {
        let h = Hub::standard(Box::new(NoAgentTools));
        let subscribed: Vec<&str> = h.providers.iter().flat_map(|p| p.events().iter().copied()).collect();
        for e in [
            "agents:changed", "agents:done", "capture:done", "clip:added", "kbd:layout", "live:battery", "live:calendar", "live:download", "live:file-done",
            "live:mail", "live:notification", "live:obs", "live:privacy", "live:steam", "live:usb", "live:volume", "media:changed", "rec:state",
            "remind:changed", "remind:due", "settings:changed", "shelf:changed", "sysmon:tick", "timer:tick", "uninstall:ready", "update:available",
            "voice:level", "voice:state", "pill:hide", "pill:show",
        ] {
            assert!(subscribed.contains(&e), "no provider consumes {e}");
        }
    }

    #[test]
    fn capsule_arbitration_by_priority_and_expiry() {
        let mut h = Hub::standard(Box::new(NoAgentTools));
        let env = MemEnv::default();
        let media = json!({"has_session": true, "playing": true, "title": "Song", "artist": "A", "app": "x.exe", "thumb_id": 1, "thumbnail": "data:u", "accent": "#336699"});
        h.dispatch(&env, "settings:changed", &Value::Null, T0);
        h.dispatch(&env, "media:changed", &media, T0);
        assert_eq!(h.board_mut().top(T0).unwrap().id, "media");
        h.dispatch(&env, "agents:changed", &json!({"working": 0, "waiting": 1, "sessions": [{"state": "waiting", "project": "p"}]}), T0 + 10);
        assert_eq!(h.board_mut().top(T0 + 10).unwrap().id, "agents", "70 beats the music 50");
        h.dispatch(&env, "live:volume", &json!({"level": 30, "muted": false}), T0 + 20);
        assert_eq!(h.board_mut().top(T0 + 20).unwrap().id, "volume");
        h.dispatch(&env, "kbd:layout", &json!({"lang": "EN"}), T0 + 30);
        assert_eq!(h.board_mut().top(T0 + 30).unwrap().id, "kbd", "90 beats the volume 88");
        assert_eq!(h.board_mut().top(T0 + 30 + 1400).unwrap().id, "volume", "kbd expired after 1.4 s");
        assert_eq!(h.board_mut().top(T0 + 20 + 1600).unwrap().id, "agents", "volume expired too");
        h.dispatch(&env, "agents:changed", &json!({"working": 0, "waiting": 0, "sessions": []}), T0 + 3000);
        assert_eq!(h.board_mut().top(T0 + 3000).unwrap().id, "media");
        h.dispatch(&env, "rec:state", &json!({"recording": true, "startedAt": T0}), T0 + 4000);
        assert_eq!(h.board_mut().top(T0 + 4000).unwrap().id, "rec", "recording (100) outranks everything");
        h.dispatch(&env, "rec:state", &json!({"recording": false}), T0 + 5000);
        assert_eq!(h.board_mut().top(T0 + 5000).unwrap().id, "media");
    }

    #[test]
    fn quiet_hours_mute_exactly_the_quiet_sources() {
        let mut env = MemEnv::default();
        env.quiet = true;
        let mut h = Hub::standard(Box::new(NoAgentTools));
        let media = json!({"has_session": true, "playing": true, "title": "Song", "thumb_id": 1, "thumbnail": "u"});
        h.dispatch(&env, "media:changed", &media, T0);
        h.dispatch(&env, "live:volume", &json!({"level": 5}), T0);
        h.dispatch(&env, "kbd:layout", &json!({"lang": "RU"}), T0);
        h.dispatch(&env, "clip:added", &json!({"kind": "text"}), T0);
        h.dispatch(&env, "sysmon:tick", &json!({"cpu": 1}), T0);
        h.dispatch(&env, "agents:changed", &json!({"working": 2, "waiting": 0, "sessions": []}), T0);
        h.dispatch(&env, "live:steam", &json!({"state": "downloading", "percent": 5}), T0);
        h.dispatch(&env, "live:download", &json!({"state": "progress", "id": "a", "name": "f", "bytes": 1}), T0);
        let ids: Vec<String> = ["media", "volume", "kbd", "copy", "agents", "steam", "download"].iter().filter(|i| h.board().get(i).is_some()).map(|s| s.to_string()).collect();
        assert_eq!(ids, ["media", "volume"], "only non-quiet rows survive");
        // the reaction of a quiet source is turned into an unreact
        let cmds = h.dispatch(&env, "agents:changed", &json!({"working": 1, "waiting": 0, "sessions": []}), T0 + 1);
        assert!(cmds.contains(&LiveCmd::unreact("agents")));
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::React { source, .. } if source == "agents")));
        // media keeps its edge reaction
        let cmds = h.dispatch(&env, "media:changed", &media, T0 + 2);
        assert!(cmds.iter().any(|c| matches!(c, LiveCmd::React { source, .. } if source == "media")));
    }

    #[test]
    fn ticks_flow_through_the_hub() {
        let mut h = Hub::standard(Box::new(NoAgentTools));
        let mut env = MemEnv::default();
        env.set("live.obs", json!(true));
        h.dispatch(&env, "live:obs", &json!({"recording": true, "streaming": false, "recMs": 0}), T0);
        h.dispatch(&env, "timer:tick", &json!({"id": 1, "remaining_ms": 61_000, "total_s": 120}), T0);
        let ms = h.tick_ms(T0).unwrap();
        assert!(ms <= 500, "obs ticks at 2 Hz: {ms}");
        let cmds = h.tick(&env, T0 + 1000);
        let ids: Vec<&str> = cmds.iter().filter_map(|c| if let LiveCmd::SetRow(r) = c { Some(r.id.as_str()) } else { None }).collect();
        assert!(ids.contains(&"obs") && ids.contains(&"timer"), "{ids:?}");
        // the board holds the freshest copy of each row
        let obs = h.board().get("obs").unwrap();
        assert!(obs.elements.iter().any(|e| matches!(e, Elem::Clock { text, .. } if text == "0:01")));
    }

    #[test]
    fn garbage_payloads_never_panic() {
        let garbage = vec![
            Value::Null,
            json!({}),
            json!([]),
            json!("x"),
            json!(7),
            json!(-1.5e300),
            json!(true),
            serde_json::from_str::<Value>(r#"{"state": 5, "level": "x", "kind": [], "title": {}, "name": null, "percent": "NaN", "bytes": -1, "total": 1e300, "recMs": 1e300, "streamMs": -1e300,
                   "remaining_ms": 1e300, "startedAt": -1e300, "recStartedAt": 1e300, "recording": 1, "id": {}, "sessions": 5, "gpus": "x", "items": 3, "text": 4,
                   "file": 5, "letter": 6, "lang": [1], "app": 1, "thumb_id": -5, "ts": 1e300, "seconds": -1e300, "history": 5, "rgb": "x", "found": 1, "isUrl": 1,
                   "version": [], "notes": 5, "ram": 5, "cpu": "x", "thumbnail": 3, "has_session": "yes", "playing": [], "minutes": 1e300, "more": 1e300, "fileCount": -1e300,
                   "active": 1, "due_at": 1e300, "total_s": -1e300, "remaining": null, "waiting": 1e300, "working": -1e300}"#).unwrap(),
            json!({"sessions": [1, null, "x", {"state": 3, "project": {}}], "gpus": [null, 1, {"index": "z", "tempC": 1e300, "util": "a"}], "items": [1, 2],
                   "history": [1, null], "rgb": [null, "a"], "notes": [null, 1], "apps": [1, {}], "state": "progress", "id": 1, "recording": true, "waiting": 2}),
            json!({"state": "done", "id": 1, "name": 5, "bytes": "x", "path": []}),
            json!({"state": "recording", "startedAt": "soon"}),
            json!({"state": "error", "code": 5, "message": {}}),
            json!({"kind": "color", "hex": 5, "history": [{}, 1]}),
            json!({"kind": "screenshot", "path": 1}),
            json!({"kind": "record", "path": 1, "seconds": "x"}),
        ];
        let mut envs = vec![MemEnv::default(), MemEnv::en()];
        let mut quiet = MemEnv::default();
        quiet.quiet = true;
        quiet.state = crate::live::env::PillState::Expanded;
        envs.push(quiet);
        let mut loud = MemEnv::default();
        for k in ["obs", "sysmon", "calendar", "mail"] {
            loud.set(&format!("live.{k}"), json!(true));
        }
        envs.push(loud);
        for env in &envs {
            let mut h = Hub::standard(Box::new(NoAgentTools));
            let names: Vec<&'static str> = h.providers.iter().flat_map(|p| p.events().iter().copied()).collect();
            for n in &names {
                for (i, p) in garbage.iter().enumerate() {
                    let now = T0 + (i as i64) * 700;
                    h.dispatch(env, n, p, now);
                    h.tick(env, now + 5000);
                    h.tick_ms(now);
                    h.board_mut().top(now);
                }
            }
            h.sweep_quiet(env);
        }
    }
}
