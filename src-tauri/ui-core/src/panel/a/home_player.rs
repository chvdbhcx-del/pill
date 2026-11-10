//! The Home player's own settings and lyrics (the model side of `home.player` and `home.lyrics`).
//!
//! * [`PlayerStyle`]: `home.player` = `mirror` (default: a large cover with blurred reflections at its sides), `fill` (the whole card tinted with the
//!   cover's colours) or `compact` (the original card).
//! * [`HomeLyrics`]: the timed lines of the playing track (`lyrics:changed`, setting `music.lyrics`, opt-in) and the column that shows them
//!   (`home.lyrics`). The column is only there while lines exist, so with the setting off nothing about the player changes.
use crate::live::lyrics::{track_key, Lyrics, Payload};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayerStyle {
    #[default]
    Mirror,
    Fill,
    Compact,
}

impl PlayerStyle {
    pub fn from_setting(s: &str) -> PlayerStyle {
        match s {
            "fill" => PlayerStyle::Fill,
            "compact" => PlayerStyle::Compact,
            _ => PlayerStyle::Mirror,
        }
    }
}

/// How many lines around the current one the view carries (the tab scrolls over them).
pub const WINDOW: usize = 4;

/// What the lyrics column draws: a window of lines and which of them is being sung.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LyricsView {
    /// `(index in the song, text)`; an empty text is an instrumental gap
    pub lines: Vec<(usize, String)>,
    /// index of the line being sung; `None` before the first line
    pub cur: Option<usize>,
    /// identifies the song (a scroll between two songs is a jump, not a scroll over the whole list)
    pub key: u64,
}

#[derive(Debug, Default)]
pub struct HomeLyrics {
    /// `music.lyrics`
    pub enabled: bool,
    /// `home.lyrics`
    pub column: bool,
    key: String,
    last: Option<Payload>,
    lyrics: Option<Lyrics>,
}

impl HomeLyrics {
    pub fn configure(&mut self, cfg_lyrics: bool, cfg_column: bool) {
        self.enabled = cfg_lyrics;
        self.column = cfg_column;
        self.adopt();
    }

    /// The playing track changed (`None` = no track).
    pub fn track(&mut self, artist: &str, title: &str, album: &str, duration_ms: u64, has_session: bool) {
        let key = if has_session && !title.trim().is_empty() { track_key(artist, title, album, duration_ms) } else { String::new() };
        if key != self.key {
            self.key = key;
            self.adopt();
        }
    }

    /// `lyrics:changed` / the answer of `lyrics_state`.
    pub fn payload(&mut self, v: &Value) {
        if let Some(p) = Payload::from_json(v) {
            self.last = Some(p);
            self.adopt();
        }
    }

    fn adopt(&mut self) {
        let key = &self.key;
        self.lyrics = if self.enabled { self.last.as_ref().filter(|p| &p.track == key).and_then(|p| p.lyrics.clone()) } else { None };
    }

    /// True while the column is drawn (the card then makes room for it).
    pub fn shown(&self) -> bool {
        self.column && self.lyrics.is_some()
    }

    pub fn view(&self, pos_ms: f64) -> Option<LyricsView> {
        let l = self.lyrics.as_ref().filter(|_| self.column)?;
        let pos = pos_ms.max(0.0) as u64;
        let key = self.key.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
        Some(LyricsView { lines: l.window(pos, WINDOW, WINDOW).into_iter().map(|(i, t)| (i, t.to_string())).collect(), cur: l.line_at(pos), key })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::lyrics::parse_lrc;

    fn song() -> (HomeLyrics, String) {
        let mut h = HomeLyrics::default();
        h.configure(true, true);
        h.track("Band", "Song", "LP", 200_000, true);
        let key = track_key("Band", "Song", "LP", 200_000);
        (h, key)
    }

    #[test]
    fn styles_parse_and_default_to_the_mirror() {
        assert_eq!(PlayerStyle::from_setting("fill"), PlayerStyle::Fill);
        assert_eq!(PlayerStyle::from_setting("compact"), PlayerStyle::Compact);
        assert_eq!(PlayerStyle::from_setting("mirror"), PlayerStyle::Mirror);
        assert_eq!(PlayerStyle::from_setting(""), PlayerStyle::Mirror);
        assert_eq!(PlayerStyle::from_setting("nonsense"), PlayerStyle::Mirror);
    }

    #[test]
    fn the_column_appears_with_the_lines_of_the_playing_track_only() {
        let (mut h, key) = song();
        assert!(!h.shown() && h.view(0.0).is_none(), "nothing yet");
        h.payload(&parse_lrc("[00:05.00]a\n[00:10.00]b\n[00:15.00]c").to_payload("some other track"));
        assert!(!h.shown());
        h.payload(&parse_lrc("[00:05.00]a\n[00:10.00]b\n[00:15.00]c").to_payload(&key));
        assert!(h.shown());
        let v = h.view(11_000.0).unwrap();
        assert_eq!((v.cur, v.lines.len()), (Some(1), 3));
        assert_eq!(h.view(1_000.0).unwrap().cur, None, "before the first line");
        // a new track drops the lines at once
        h.track("Band", "Other", "LP", 100_000, true);
        assert!(!h.shown());
        // and the first answer, which may have come before the track was known, is used when the track matches
        h.track("Band", "Song", "LP", 200_000, true);
        assert!(h.shown());
    }

    #[test]
    fn switches_hide_the_column_and_the_lines() {
        let (mut h, key) = song();
        h.payload(&parse_lrc("[00:05.00]a").to_payload(&key));
        h.configure(true, false);
        assert!(!h.shown() && h.view(6_000.0).is_none(), "home.lyrics off");
        h.configure(false, true);
        assert!(!h.shown(), "music.lyrics off: no lines are kept visible");
        h.configure(true, true);
        assert!(h.shown(), "back on: the answer we already have is shown again");
        h.track("", "", "", 0, false);
        assert!(!h.shown(), "no session");
    }
}
