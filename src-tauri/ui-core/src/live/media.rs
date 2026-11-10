//! The music row of the collapsed pill (port of `updateLive` and the state intake of tab-home.js; Rust side media.rs).
//!
//! `media:changed` (a `MediaState`: `has_session, playing, title, artist, album, app, thumb_id, thumbnail?, accent?`) drives:
//! * row `media` (priority 50, no ttl) while a track *plays*; paused/no session = the capsule goes back to clock + weather.
//!   Outline = the cover accent (theme accent when the cover has none).
//! * edge `music` baseline (`Pill.react('media', 'music', {color})`), tinted with the cover accent; removed when paused or off.
//!
//! Modes (settings `musicPill`): `standard` (cover + equaliser, 250 px, only until the settings are read), `title` (equaliser + title),
//! `mini` (cover + title + prev / pause / next buttons), `lyrics` (the line being sung, see below; the title when there are no lyrics).
//! Lyrics (setting `music.lyrics`, opt-in, [`super::lyrics`]): the backend's `lyrics:changed` brings the timed lines of the playing track; the row
//! shows `Elem::Lyric` (the renderer slides old and new line on a spring) and `tick_ms` asks for a check every 200 ms only while that line is on screen.
//! The mini player gets a quote button (setting `music.miniLyric`) that swaps its title for the sung line.
//! Gates: Settings > Live events > Music (`live.music`) and Modules > Media player (`module.media`).
//!
//! The cover arrives only when `thumb_id` changed; when an event omits a changed cover the provider asks once (`media_state {known: null}`).

use std::any::Any;

use serde_json::{json, Value};

use super::env::Env;
use super::hub::Provider;
use super::lyrics::{track_key, Clock, Lyrics, Payload};
use super::model::*;
use super::util::*;

pub const MEDIA_PRIORITY: i32 = 50;
pub const STANDARD_W: f32 = 250.0;
/// How often the row checks whether the sung line changed (only while a lyric line is shown).
pub const LYRIC_TICK_MS: u32 = 200;

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
    lyr: LyricState,
}

/// Song lyrics for the row (`music.lyrics`, opt-in): the backend sends `lyrics:changed`; the position is interpolated from the last media event.
#[derive(Debug, Default)]
struct LyricState {
    /// `music.lyrics`
    on: bool,
    /// `music.miniLyric`: the mini player shows the line instead of the title (the quote button)
    mini: bool,
    key: String,
    clock: Clock,
    now: i64,
    last: Option<Payload>,
    lyrics: Option<Lyrics>,
    /// the line on the row, the one before it, and a counter of changes (the renderer slides on `seq`)
    text: String,
    prev: String,
    seq: u64,
}

impl Default for MediaLive {
    fn default() -> Self {
        MediaLive { mode: MusicMode::Standard, st: None, art_id: 0, art_url: None, live_key: String::new(), was_media: true, lyr: LyricState::default() }
    }
}

impl LyricState {
    /// The row shows the sung line (Lyrics mode, or the mini player with the quote button on) instead of the title.
    fn wanted(&self, mode: MusicMode) -> bool {
        self.on && self.lyrics.is_some() && (mode == MusicMode::Lyrics || (mode == MusicMode::Mini && self.mini))
    }

    /// The line at `now`: the title before the first line, "♪" for an instrumental gap.
    fn line(&self, title: &str) -> String {
        let Some(l) = &self.lyrics else { return title.to_string() };
        match l.line_at(self.clock.pos(self.now)) {
            None => title.to_string(),
            Some(i) if l.lines()[i].text.is_empty() => "\u{266a}".to_string(),
            Some(i) => l.lines()[i].text.clone(),
        }
    }

    fn adopt(&mut self) {
        let key = self.key.clone();
        self.lyrics = self.last.as_ref().filter(|p| p.track == key).and_then(|p| p.lyrics.clone());
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
        // lyrics: the playback clock re-bases on every event; a new track drops the old lines until the backend answers
        let (playing, dur) = (truthy(s.get("playing")), num_or0(s.get("duration_ms")).max(0.0) as u64);
        self.lyr.clock.set(num_or0(s.get("position_ms")).max(0.0) as u64, dur, playing, self.lyr.now);
        let key = if truthy(s.get("has_session")) && !text_or_empty(s.get("title")).trim().is_empty() {
            track_key(&text_or_empty(s.get("artist")), &text_or_empty(s.get("title")), &text_or_empty(s.get("album")), dur)
        } else {
            String::new()
        };
        if key != self.lyr.key {
            self.lyr.key = key;
            self.lyr.adopt();
        }
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
        let name = if st.title.is_empty() { app_name(&st.app) } else { st.title.clone() };
        let lyric = self.lyr.wanted(self.mode);
        let line = if lyric { self.lyr.line(&name) } else { String::new() };
        // the JS key (mode|playing|title|artist|art.id|accent) missed the cover arriving later for the same id; `cover` fixes that
        let key = [self.mode.as_str().to_string(), st.playing.to_string(), st.title.clone(), st.artist.clone(), self.art_id.to_string(), st.accent.clone(), self.art_url.is_some().to_string(), format!("q{}{}", self.mode == MusicMode::Mini && self.lyr.on, self.lyr.mini), lyric.to_string()].join("|");
        // the sung line is not part of the row key: a new line edits the row in place (the renderer slides it), it only has to redraw
        let live_key = format!("{key}|{line}");
        if live_key == self.live_key {
            return out;
        }
        self.live_key = live_key;
        let eq_accent = Some(accent.map_or(Accent::Theme, Accent::Rgb));
        let eq = Elem::Eq { accent: eq_accent, playing: st.playing };
        let cover = Elem::Cover { id: self.art_id, url: self.art_url.clone(), size: 24.0 };
        let quote = self.mode == MusicMode::Mini && self.lyr.on; // the mini player gets the quote button while lyrics are switched on
        let max_w = if quote { 190.0 } else { 215.0 };
        let name_el = if lyric {
            if line != self.lyr.text {
                self.lyr.prev = std::mem::replace(&mut self.lyr.text, line);
                self.lyr.seq += 1;
            }
            Elem::Lyric { text: self.lyr.text.clone(), prev: self.lyr.prev.clone(), seq: self.lyr.seq, max_w }
        } else {
            self.lyr.text.clear();
            Elem::text_max(name, TextStyle::Name, max_w)
        };
        let btn = |icon: IconName, act: &str| Elem::Button { action: Action::invoke("media_control", json!({ "action": act })), label: None, icon: Some(icon), style: ButtonStyle::IconRound, tip: String::new() };
        let (elements, width) = match self.mode {
            MusicMode::Standard => (vec![cover, eq], STANDARD_W),
            MusicMode::Title | MusicMode::Lyrics => (vec![eq, name_el], 0.0),
            MusicMode::Mini => {
                let mut v = vec![cover, name_el, btn(IconName::Prev, "prev"), btn(if st.playing { IconName::Pause } else { IconName::Play }, "play_pause"), btn(IconName::Next, "next")];
                if quote {
                    let on = self.lyr.mini;
                    v.push(Elem::Button { action: Action::invoke("settings_set", json!({ "patch": { "music.miniLyric": !on } })), label: None, icon: Some(if on { IconName::Quote } else { IconName::QuoteOff }), style: ButtonStyle::IconRound, tip: String::new() });
                }
                (v, 0.0)
            }
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
        self.lyr.on = env.setting_true("music.lyrics");
        self.lyr.mini = env.setting_true("music.miniLyric");
        if !self.lyr.on {
            self.lyr.lyrics = None;
        } else if self.lyr.lyrics.is_none() {
            self.lyr.adopt();
        }
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
        &["media:changed", "media:state", "settings:changed", "lyrics:changed"]
    }
    fn boot(&self) -> Vec<Query> {
        let mut q = Query::new("media_state", "media:changed");
        q.args = json!({ "known": null });
        vec![q, Query::new("lyrics_state", "lyrics:changed")] // `null` while the setting is off or nothing is known
    }
    fn on_event(&mut self, env: &dyn Env, name: &str, p: &Value, now: i64) -> Vec<LiveCmd> {
        self.lyr.now = now;
        match name {
            "lyrics:changed" => match Payload::from_json(p) {
                Some(pl) => {
                    self.lyr.last = Some(pl);
                    self.lyr.adopt();
                    self.update_live(env)
                }
                None => Vec::new(),
            },
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
    fn on_tick(&mut self, env: &dyn Env, now: i64) -> Vec<LiveCmd> {
        self.lyr.now = now;
        self.update_live(env) // redraws only when the sung line changed (the live key carries it)
    }
    /// Only while a sung line is on the row and a later line exists: 5 checks a second (a line may change up to 200 ms late; the host runs
    /// `on_tick` only when `wall - last >= tick_ms`, so a period that jumps with the position, like "time to the next line", would skip ticks).
    fn tick_ms(&self, now: i64) -> Option<u32> {
        if self.live_key.is_empty() || !self.lyr.wanted(self.mode) || !self.lyr.clock.playing() {
            return None;
        }
        self.lyr.lyrics.as_ref()?.next_start(self.lyr.clock.pos(now))?;
        Some(LYRIC_TICK_MS)
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

    const LRC: &str = "[00:10.00]First\n[00:20.00]Second\n[00:30.00]\n[00:40.00]Last";

    fn lyric_payload(state: &Value, lrc: &str) -> Value {
        let key = track_key(state["artist"].as_str().unwrap(), state["title"].as_str().unwrap(), state["album"].as_str().unwrap(), state["duration_ms"].as_u64().unwrap());
        crate::live::lyrics::parse_lrc(lrc).to_payload(&key)
    }

    /// A lyrics-mode provider playing "Song" at `pos` ms with the lyrics loaded.
    fn singing(mode: &str, pos: i64) -> (MediaLive, MemEnv, Value) {
        let (mut m, mut env) = live(mode);
        env.set("music.lyrics", json!(true));
        m.on_event(&env, "settings:changed", &Value::Null, T0);
        let mut s = state(true, "Song", 1, Some("u"), None);
        s["position_ms"] = json!(pos);
        m.on_event(&env, "media:changed", &s, T0);
        m.on_event(&env, "lyrics:changed", &lyric_payload(&s, LRC), T0);
        (m, env, s)
    }

    /// The row commands of the event that brought the lyrics of `s`.
    fn lyrics_arrive(m: &mut MediaLive, env: &MemEnv, s: &Value) -> Vec<LiveCmd> {
        m.on_event(env, "lyrics:changed", &lyric_payload(s, LRC), T0)
    }

    fn lyric_of(cmds: &[LiveCmd]) -> (String, String, u64) {
        match row(cmds).elements.iter().find(|e| matches!(e, Elem::Lyric { .. })) {
            Some(Elem::Lyric { text, prev, seq, .. }) => (text.clone(), prev.clone(), *seq),
            _ => panic!("no lyric element in {:?}", row(cmds).elements),
        }
    }

    #[test]
    fn lyrics_mode_shows_the_sung_line_and_slides_on_a_change() {
        let (mut m, env, s) = singing("lyrics", 12_000);
        let cmds = m.on_event(&env, "media:changed", &s, T0); // same state again: nothing new
        assert!(!has_row(&cmds));
        let k0 = m.live_key.clone();
        assert!(k0.ends_with("|First"));
        // 9 s later (wall clock) the second line is on
        let cmds = m.on_tick(&env, T0 + 9_000);
        let (text, prev, seq) = lyric_of(&cmds);
        assert_eq!((text.as_str(), prev.as_str()), ("Second", "First"));
        assert!(seq >= 2);
        assert_eq!(row(&cmds).key, k0.rsplit_once('|').unwrap().0, "the row key does not carry the line: edited in place");
        // nothing changes within a line
        assert!(!has_row(&m.on_tick(&env, T0 + 9_500)));
        // an instrumental gap shows a note, the next line comes after it
        assert_eq!(lyric_of(&m.on_tick(&env, T0 + 19_000)).0, "\u{266a}");
        assert_eq!(lyric_of(&m.on_tick(&env, T0 + 29_000)).0, "Last");
    }

    #[test]
    fn before_the_first_line_and_without_lyrics_the_row_shows_the_title() {
        let (mut m, env) = live("lyrics");
        let mut s = state(true, "Song", 1, Some("u"), None);
        s["position_ms"] = json!(2_000);
        let mut env = env;
        env.set("music.lyrics", json!(true));
        m.on_event(&env, "settings:changed", &Value::Null, T0);
        m.on_event(&env, "media:changed", &s, T0);
        let cmds = lyrics_arrive(&mut m, &env, &s);
        assert_eq!(lyric_of(&cmds).0, "Song", "intro: the title sits in the same element, so the first line slides in");
        assert_eq!(lyric_of(&m.on_tick(&env, T0 + 8_500)).0, "First");
        let (mut m, env) = live("lyrics");
        let cmds = m.on_event(&env, "media:changed", &state(true, "Song", 1, Some("u"), None), T0);
        assert!(matches!(row(&cmds).elements[1], Elem::Text { .. }), "setting off, nothing loaded: the title");
    }

    #[test]
    fn lyrics_of_another_track_or_a_switch_off_are_ignored() {
        let (mut m, mut env, s) = singing("lyrics", 12_000);
        let mut other = s.clone();
        other["title"] = json!("Other");
        let cmds = m.on_event(&env, "media:changed", &other, T0 + 10);
        assert!(matches!(row(&cmds).elements[1], Elem::Text { .. }), "new track: the old lines are dropped until the backend answers");
        // a late answer for the old track changes nothing
        assert!(m.on_event(&env, "lyrics:changed", &lyric_payload(&s, LRC), T0 + 20).iter().all(|c| !matches!(c, LiveCmd::SetRow(_))));
        // the right answer arrives
        let cmds = m.on_event(&env, "lyrics:changed", &lyric_payload(&other, LRC), T0 + 30);
        assert!(has_row(&cmds));
        env.set("music.lyrics", json!(false));
        let cmds = m.on_event(&env, "settings:changed", &Value::Null, T0 + 40);
        assert!(matches!(row(&cmds).elements[1], Elem::Text { .. }), "switched off: the title again");
    }

    #[test]
    fn the_lyrics_tick_runs_only_while_a_line_is_shown() {
        let (mut m, env, mut s) = singing("lyrics", 12_000);
        assert_eq!(m.tick_ms(T0), Some(LYRIC_TICK_MS));
        assert_eq!(m.tick_ms(T0 + 100_000), None, "after the last line there is nothing to wait for");
        s["playing"] = json!(false);
        m.on_event(&env, "media:changed", &s, T0 + 10);
        assert_eq!(m.tick_ms(T0 + 10), None, "paused: no ticks");
        let (m, _) = live("title");
        assert_eq!(m.tick_ms(T0), None);
    }

    #[test]
    fn the_mini_player_has_a_quote_button_that_swaps_the_title_for_the_line() {
        let (mut m, mut env) = live("mini");
        env.set("music.lyrics", json!(true));
        m.on_event(&env, "settings:changed", &Value::Null, T0);
        let mut s = state(true, "Song", 1, Some("u"), None);
        s["position_ms"] = json!(12_000);
        let first = m.on_event(&env, "media:changed", &s, T0);
        lyrics_arrive(&mut m, &env, &s);
        let r = row(&first);
        assert!(matches!(r.elements[1], Elem::Text { .. }), "hidden by default: the title");
        let Some(Elem::Button { icon, action, .. }) = r.elements.last() else { panic!() };
        assert_eq!((*icon, action.clone()), (Some(IconName::QuoteOff), Action::invoke("settings_set", json!({ "patch": { "music.miniLyric": true } }))));
        env.set("music.miniLyric", json!(true));
        let cmds = m.on_event(&env, "settings:changed", &Value::Null, T0 + 2);
        assert_eq!(lyric_of(&cmds).0, "First");
        let Some(Elem::Button { icon, action, .. }) = row(&cmds).elements.last() else { panic!() };
        assert_eq!((*icon, action.clone()), (Some(IconName::Quote), Action::invoke("settings_set", json!({ "patch": { "music.miniLyric": false } }))));
        // lyrics off: no button
        env.set("music.lyrics", json!(false));
        let cmds = m.on_event(&env, "settings:changed", &Value::Null, T0 + 3);
        assert_eq!(row(&cmds).elements.len(), 5);
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
        assert_eq!(m.boot().len(), 2);
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
