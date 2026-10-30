//! The music row of the collapsed pill (port of `updateLive` and the state intake of tab-home.js; Rust side media.rs).
//!
//! `media:changed` (a `MediaState`: `has_session, playing, title, artist, album, app, thumb_id, thumbnail?, accent?`) drives:
//! * row `media` (priority 50, no ttl) while a track *plays*; paused/no session = the capsule goes back to clock + weather.
//!   Outline = the cover accent (theme accent when the cover has none).
//! * edge `music` baseline (`Pill.react('media', 'music', {color})`), tinted with the cover accent; removed when paused or off.
//!
//! Modes (settings `musicPill`): `standard` (cover + equaliser, 250 px, only until the settings are read), `title` (equaliser + title),
//! `mini` (cover + title + prev / pause / next buttons), `lyrics` (no widget yet: shows the title).
//! Gates: Settings > Live events > Music (`live.music`) and Modules > Media player (`module.media`).
//!
//! The cover arrives only when `thumb_id` changed; when an event omits a changed cover the provider asks once (`media_state {known: null}`).

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::model::*;
use super::util::*;

pub const MEDIA_PRIORITY: i32 = 50;
pub const STANDARD_W: f32 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicMode {
    Standard,
    Title,
    Mini,
    Lyrics,
}

impl MusicMode {
    /// `musicPill`: `mini` -> mini, `lyrics` -> lyrics (draws the title), anything else -> title.
    pub fn from_setting(s: Option<&str>) -> MusicMode {
        match s {
            Some("mini") => MusicMode::Mini,
            Some("lyrics") => MusicMode::Lyrics,
            _ => MusicMode::Title,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            MusicMode::Standard => "standard",
            MusicMode::Title => "title",
            MusicMode::Mini => "mini",
            MusicMode::Lyrics => "lyrics",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct St {
    has_session: bool,
    playing: bool,
    title: String,
    artist: String,
    app: String,
    accent: String,
}

#[derive(Debug)]
pub struct MediaLive {
    mode: MusicMode,
    st: Option<St>,
    art_id: u64,
    art_url: Option<String>,
    live_key: String,
    was_media: bool,
}

impl Default for MediaLive {
    fn default() -> Self {
        MediaLive { mode: MusicMode::Standard, st: None, art_id: 0, art_url: None, live_key: String::new(), was_media: true }
    }
}

fn app_name(app: &str) -> String {
    // `String(app).replace(/\.exe$/i, '').trim()`: the suffix is only stripped when the string *ends* with it
    let cut = if app.to_lowercase().ends_with(".exe") { &app[..app.len() - 4] } else { app };
    cut.trim().to_string()
}

impl MediaLive {
    pub fn mode(&self) -> MusicMode {
        self.mode
    }

    fn query_cover() -> LiveCmd {
        let mut q = Query::new("media_state", "media:state");
        q.args = json!({ "known": null });
        LiveCmd::Query(q)
    }

    fn clear_row(&mut self) -> Vec<LiveCmd> {
        if self.live_key.is_empty() {
            return Vec::new();
        }
        self.live_key.clear();
        vec![LiveCmd::clear("media")]
    }

    fn apply(&mut self, env: &dyn Env, s: &Value) -> Vec<LiveCmd> {
        if s.is_null() || !env.module_on("module.media") {
            return Vec::new();
        }
        let mut out = Vec::new();
        let thumb_id = num_or0(s.get("thumb_id")).max(0.0) as u64;
        if truthy(s.get("thumbnail")) {
            self.art_id = thumb_id;
            self.art_url = Some(text(s.get("thumbnail")));
        } else if truthy(s.get("has_session")) && thumb_id != self.art_id {
            // Cover changed but the event omitted it (we had missed it): fetch once.
            self.art_id = thumb_id;
            self.art_url = None;
            out.push(Self::query_cover());
        }
        self.st = Some(St {
            has_session: truthy(s.get("has_session")),
            playing: truthy(s.get("playing")),
            title: text_or_empty(s.get("title")),
            artist: text_or_empty(s.get("artist")),
            app: text_or_empty(s.get("app")),
            accent: text_or_empty(s.get("accent")),
        });
        out.extend(self.update_live(env));
        out
    }

    /// `updateLive()`.
    pub fn update_live(&mut self, env: &dyn Env) -> Vec<LiveCmd> {
        let st = match &self.st {
            Some(s) if env.live_on("music") && env.module_on("module.media") && s.has_session => s.clone(),
            _ => {
                let mut out = vec![LiveCmd::unreact("media")];
                out.extend(self.clear_row());
                return out;
            }
        };
        let accent = Rgba::from_hex(&st.accent);
        let mut out = Vec::new();
        if st.playing {
            out.push(LiveCmd::react("media", Preset::Music, None, accent)); // edge baseline while a track plays; the cover accent tints the pet too
        } else {
            out.push(LiveCmd::unreact("media"));
        }
        if !st.playing {
            out.extend(self.clear_row()); // paused: the capsule goes back to the clock + weather
            return out;
        }
        // the JS key (mode|playing|title|artist|art.id|accent) missed the cover arriving later for the same id; `cover` fixes that
        let key = [self.mode.as_str().to_string(), st.playing.to_string(), st.title.clone(), st.artist.clone(), self.art_id.to_string(), st.accent.clone(), self.art_url.is_some().to_string()].join("|");
        if key == self.live_key {
            return out;
        }
        self.live_key = key.clone();
        let eq_accent = Some(accent.map_or(Accent::Theme, Accent::Rgb));
        let eq = Elem::Eq { accent: eq_accent, playing: st.playing };
        let cover = Elem::Cover { id: self.art_id, url: self.art_url.clone(), size: 24.0 };
        let name = if st.title.is_empty() { app_name(&st.app) } else { st.title.clone() };
        let name_el = Elem::text_max(name, TextStyle::Name, 215.0);
        let btn = |icon: IconName, act: &str| Elem::Button { action: Action::invoke("media_control", json!({ "action": act })), label: None, icon: Some(icon), style: ButtonStyle::IconRound, tip: String::new() };
        let (elements, width) = match self.mode {
            MusicMode::Standard => (vec![cover, eq], STANDARD_W),
            MusicMode::Title | MusicMode::Lyrics => (vec![eq, name_el], 0.0),
            MusicMode::Mini => (vec![cover, name_el, btn(IconName::Prev, "prev"), btn(if st.playing { IconName::Pause } else { IconName::Play }, "play_pause"), btn(IconName::Next, "next")], 0.0),
        };
        out.push(LiveCmd::set(
            RowSpec::new("media", MEDIA_PRIORITY)
                .key(key)
                .width(width)
                .accent(accent.map_or(Accent::Theme, Accent::Rgb))
                .anim(Anim::Fade, Anim::Glide(250.0), Anim::Fade)
                .elems(elements),
        ));
        out
    }

    /// `loadMode`: re-read `musicPill` and the media module switch.
    fn load_mode(&mut self, env: &dyn Env) -> Vec<LiveCmd> {
        self.mode = MusicMode::from_setting(env.setting_str("musicPill").as_deref());
        let mut out = Vec::new();
        let media = env.module_on("module.media");
        if media != self.was_media {
            self.was_media = media;
            if media {
                let mut q = Query::new("media_state", "media:changed");
                q.args = json!({ "known": null });
                out.push(LiveCmd::Query(q));
            } else {
                self.st = None;
            }
        }
        out.extend(self.update_live(env));
        out
    }
}

impl Provider for MediaLive {
    fn name(&self) -> &'static str {
        "media"
    }
    fn events(&self) -> &'static [&'static str] {
        &["media:changed", "media:state", "settings:changed"]
    }
    fn boot(&self) -> Vec<Query> {
        let mut q = Query::new("media_state", "media:changed");
        q.args = json!({ "known": null });
        vec![q]
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, _now: i64) -> Vec<LiveCmd> {
        match name {
            "media:changed" => self.apply(env, p),
            // the answer to the cover query
            "media:state" => {
                if !p.is_null() && truthy(p.get("thumbnail")) {
                    self.art_id = num_or0(p.get("thumb_id")).max(0.0) as u64;
                    self.art_url = Some(text(p.get("thumbnail")));
                    return self.update_live(env);
                }
                Vec::new()
            }
            "settings:changed" => self.load_mode(env),
            _ => Vec::new(),
        }
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::env::MemEnv;

    const T0: i64 = 1_700_000_000_000;

    fn state(playing: bool, title: &str, thumb: u64, thumbnail: Option<&str>, accent: Option<&str>) -> Value {
        json!({"has_session": true, "playing": playing, "title": title, "artist": "Artist", "album": "", "session_id": "s", "app": "Spotify.exe",
               "position_ms": 1000, "duration_ms": 200000, "thumb_id": thumb, "thumbnail": thumbnail, "accent": accent})
    }
    fn row(cmds: &[LiveCmd]) -> &RowSpec {
        cmds.iter().find_map(|c| if let LiveCmd::SetRow(r) = c { Some(r) } else { None }).unwrap_or_else(|| panic!("no row in {cmds:?}"))
    }
    fn has_row(cmds: &[LiveCmd]) -> bool {
        cmds.iter().any(|c| matches!(c, LiveCmd::SetRow(_)))
    }
    fn mode_env(m: &str) -> MemEnv {
        let mut e = MemEnv::default();
        e.set("musicPill", json!(m));
        e
    }
    fn live(m: &str) -> (MediaLive, MemEnv) {
        let env = mode_env(m);
        let mut ml = MediaLive::default();
        ml.on_event(&env, "settings:changed", &Value::Null, T0);
        (ml, env)
    }

    #[test]
    fn mode_parsing() {
        assert_eq!(MusicMode::from_setting(Some("mini")), MusicMode::Mini);
        assert_eq!(MusicMode::from_setting(Some("lyrics")), MusicMode::Lyrics);
        assert_eq!(MusicMode::from_setting(Some("title")), MusicMode::Title);
        assert_eq!(MusicMode::from_setting(Some("garbage")), MusicMode::Title);
        assert_eq!(MusicMode::from_setting(None), MusicMode::Title);
        assert_eq!(MediaLive::default().mode(), MusicMode::Standard, "before the settings arrive");
    }

    #[test]
    fn standard_mode_cover_and_eq() {
        let env = MemEnv::default(); // no settings event yet: the initial mode
        let mut m = MediaLive::default();
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 7, Some("data:image/png;base64,AA"), Some("#ff8800")), T0);
        let r = row(&cmds);
        assert_eq!((r.id.as_str(), r.priority, r.ttl_ms, r.width), ("media", 50, None, 250.0));
        assert_eq!(r.accent, Some(Accent::Rgb(Rgba::rgb(255, 136, 0))));
        assert_eq!(r.elements[0], Elem::Cover { id: 7, url: Some("data:image/png;base64,AA".into()), size: 24.0 });
        assert_eq!(r.elements[1], Elem::Eq { accent: Some(Accent::Rgb(Rgba::rgb(255, 136, 0))), playing: true });
        assert_eq!(r.elements.len(), 2);
        assert!(cmds.contains(&LiveCmd::react("media", Preset::Music, None, Some(Rgba::rgb(255, 136, 0)))));
    }

    #[test]
    fn title_mode() {
        let (mut m, env) = live("title");
        let cmds = m.on_event(&env, "media:changed", &state(true, "Bohemian Rhapsody", 1, Some("u"), None), T0);
        let r = row(&cmds);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.elements, vec![Elem::Eq { accent: Some(Accent::Theme), playing: true }, Elem::text_max("Bohemian Rhapsody", TextStyle::Name, 215.0)]);
        assert_eq!(r.accent, Some(Accent::Theme), "no cover colour: the theme accent outlines the capsule");
        assert!(cmds.contains(&LiveCmd::react("media", Preset::Music, None, None)));
    }

    #[test]
    fn lyrics_mode_shows_the_title_widget() {
        let (mut m, env) = live("lyrics");
        assert_eq!(m.mode(), MusicMode::Lyrics);
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 1, Some("u"), None), T0);
        assert_eq!(row(&cmds).elements.len(), 2);
        assert!(matches!(row(&cmds).elements[0], Elem::Eq { .. }));
    }

    #[test]
    fn mini_mode_buttons() {
        let (mut m, env) = live("mini");
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 3, Some("u"), None), T0);
        let r = row(&cmds);
        assert_eq!(r.elements.len(), 5);
        assert!(matches!(r.elements[0], Elem::Cover { id: 3, .. }));
        let acts: Vec<(Option<IconName>, Action)> = r.elements[2..]
            .iter()
            .map(|e| match e {
                Elem::Button { icon, action, style, .. } => {
                    assert_eq!(*style, ButtonStyle::IconRound);
                    (*icon, action.clone())
                }
                o => panic!("{o:?}"),
            })
            .collect();
        assert_eq!(acts[0], (Some(IconName::Prev), Action::invoke("media_control", json!({"action": "prev"}))));
        assert_eq!(acts[1], (Some(IconName::Pause), Action::invoke("media_control", json!({"action": "play_pause"}))));
        assert_eq!(acts[2], (Some(IconName::Next), Action::invoke("media_control", json!({"action": "next"}))));
    }

    #[test]
    fn title_falls_back_to_the_app_name() {
        let (mut m, env) = live("title");
        let cmds = m.on_event(&env, "media:changed", &state(true, "", 1, Some("u"), None), T0);
        assert_eq!(row(&cmds).elements[1], Elem::text_max("Spotify", TextStyle::Name, 215.0), ".exe stripped");
        let mut s = state(true, "", 1, Some("u"), None);
        s["app"] = json!("  Chrome.EXE");
        let (mut m, env) = live("title");
        assert_eq!(row(&m.on_event(&env, "media:changed", &s, T0)).elements[1], Elem::text_max("Chrome", TextStyle::Name, 215.0));
        s["app"] = json!("Chrome.EXE "); // trailing space: the JS regex does not match, the suffix stays
        let (mut m, env) = live("title");
        assert_eq!(row(&m.on_event(&env, "media:changed", &s, T0)).elements[1], Elem::text_max("Chrome.EXE", TextStyle::Name, 215.0));
        s["app"] = json!("");
        let (mut m, env) = live("title");
        assert_eq!(row(&m.on_event(&env, "media:changed", &s, T0)).elements[1], Elem::text_max("", TextStyle::Name, 215.0));
        assert_eq!(app_name("exe"), "exe");
        assert_eq!(app_name(".exe"), "");
    }

    #[test]
    fn paused_removes_row_and_edge() {
        let (mut m, env) = live("title");
        m.on_event(&env, "media:changed", &state(true, "Song", 1, Some("u"), None), T0);
        let cmds = m.on_event(&env, "media:changed", &state(false, "Song", 1, None, None), T0 + 1000);
        assert_eq!(cmds, vec![LiveCmd::unreact("media"), LiveCmd::clear("media")]);
        // playing again re-creates it
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 1, None, None), T0 + 2000);
        assert!(has_row(&cmds));
        // paused again with nothing shown: only the unreact
        m.on_event(&env, "media:changed", &state(false, "Song", 1, None, None), T0 + 3000);
        let cmds = m.on_event(&env, "media:changed", &state(false, "Song", 1, None, None), T0 + 4000);
        assert_eq!(cmds, vec![LiveCmd::unreact("media")]);
    }

    #[test]
    fn same_state_does_not_rebuild_the_row() {
        let (mut m, env) = live("title");
        let s = state(true, "Song", 1, Some("u"), Some("#112233"));
        let first = m.on_event(&env, "media:changed", &s, T0);
        assert!(has_row(&first));
        let again = m.on_event(&env, "media:changed", &s, T0 + 1000);
        assert!(!has_row(&again), "same key: nothing to redraw");
        assert!(again.contains(&LiveCmd::react("media", Preset::Music, None, Some(Rgba::rgb(0x11, 0x22, 0x33)))), "the standing edge state is re-asserted (the shell dedupes it)");
        // a new title, cover or accent changes the key
        let t2 = m.on_event(&env, "media:changed", &state(true, "Other", 1, Some("u"), Some("#112233")), T0 + 2000);
        assert!(has_row(&t2) && row(&t2).key != row(&first).key);
        let c2 = m.on_event(&env, "media:changed", &state(true, "Other", 2, Some("v"), Some("#112233")), T0 + 3000);
        assert!(has_row(&c2));
        let a2 = m.on_event(&env, "media:changed", &state(true, "Other", 2, Some("v"), Some("#445566")), T0 + 4000);
        assert!(has_row(&a2));
    }

    #[test]
    fn missing_cover_is_fetched_once() {
        let (mut m, env) = live("mini");
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 9, None, None), T0);
        let q = cmds.iter().find_map(|c| if let LiveCmd::Query(q) = c { Some(q.clone()) } else { None }).expect("query");
        assert_eq!((q.cmd, q.as_event, q.args), ("media_state", "media:state", json!({"known": null})));
        assert!(matches!(row(&cmds).elements[0], Elem::Cover { id: 9, url: None, .. }), "placeholder meanwhile");
        // same id again: no second query
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 9, None, None), T0 + 100);
        assert!(!cmds.iter().any(|c| matches!(c, LiveCmd::Query(_))));
        // the answer brings the picture
        let cmds = m.on_event(&env, "media:state", &state(true, "Song", 9, Some("data:x"), None), T0 + 200);
        assert!(matches!(&row(&cmds).elements[0], Elem::Cover { id: 9, url: Some(u), .. } if u == "data:x"));
        // an answer without a picture changes nothing
        assert!(m.on_event(&env, "media:state", &state(true, "Song", 9, None, None), T0 + 300).is_empty());
        assert!(m.on_event(&env, "media:state", &Value::Null, T0).is_empty());
    }

    #[test]
    fn no_session_clears() {
        let (mut m, env) = live("title");
        m.on_event(&env, "media:changed", &state(true, "Song", 1, Some("u"), None), T0);
        let cmds = m.on_event(&env, "media:changed", &json!({"has_session": false, "playing": false, "thumb_id": 0}), T0 + 10);
        assert_eq!(cmds, vec![LiveCmd::unreact("media"), LiveCmd::clear("media")]);
        assert!(m.on_event(&env, "media:changed", &Value::Null, T0).is_empty());
    }

    #[test]
    fn switches_gate_the_row() {
        let (mut m, mut env) = live("title");
        env.set("live.music", json!(false));
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 1, Some("u"), None), T0);
        assert!(!has_row(&cmds) && cmds.contains(&LiveCmd::unreact("media")));
        env.unset("live.music");
        env.set("module.media", json!(false));
        assert!(m.on_event(&env, "media:changed", &state(true, "Song", 1, Some("u"), None), T0).is_empty(), "module off: the event is dropped");
    }

    #[test]
    fn settings_changes_redraw_and_module_toggle_refetches() {
        let (mut m, mut env) = live("title");
        let s = state(true, "Song", 1, Some("u"), None);
        let t = m.on_event(&env, "media:changed", &s, T0);
        assert_eq!(row(&t).elements.len(), 2);
        env.set("musicPill", json!("mini"));
        let cmds = m.on_event(&env, "settings:changed", &Value::Null, T0 + 10);
        assert_eq!(row(&cmds).elements.len(), 5, "the mode switch redraws at once");
        env.set("module.media", json!(false));
        let cmds = m.on_event(&env, "settings:changed", &Value::Null, T0 + 20);
        assert!(!has_row(&cmds));
        assert!(cmds.contains(&LiveCmd::clear("media")) || cmds.contains(&LiveCmd::unreact("media")));
        env.unset("module.media");
        let cmds = m.on_event(&env, "settings:changed", &Value::Null, T0 + 30);
        assert!(cmds.iter().any(|c| matches!(c, LiveCmd::Query(q) if q.cmd == "media_state" && q.as_event == "media:changed")), "module back on: ask for the state");
        assert_eq!(m.boot().len(), 1);
    }

    #[test]
    fn bad_accent_is_ignored() {
        let (mut m, env) = live("title");
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 1, Some("u"), Some("orange")), T0);
        assert!(cmds.contains(&LiveCmd::react("media", Preset::Music, None, None)));
        assert_eq!(row(&cmds).accent, Some(Accent::Theme));
    }

    #[test]
    fn unicode_title_is_kept() {
        let (mut m, env) = live("title");
        let cmds = m.on_event(&env, "media:changed", &state(true, "Кино — Группа крови 🎸", 1, Some("u"), None), T0);
        assert_eq!(row(&cmds).elements[1], Elem::text_max("Кино — Группа крови 🎸", TextStyle::Name, 215.0));
    }
}
