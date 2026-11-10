//! Timed song lyrics shared by the backend (`lyrics.rs` fetches and caches LRC text) and every UI that shows them (the pill row, the Home player).
//!
//! * [`parse_lrc`]: LRC text (`[mm:ss.xx]line`, several stamps per line, `[offset:+/-ms]`, metadata tags ignored) -> [`Lyrics`].
//! * [`Lyrics::line_at`] / [`Lyrics::window`] / [`Lyrics::next_start`]: the line being sung at a playback position, its neighbours and when the
//!   next line starts (so a ticker can sleep exactly that long).
//! * [`Track`]: the playback clock of one track (position as of the last media event, interpolated by wall time while playing).
//! * The bus payload `lyrics:changed`: `{ "track": "<key>", "lines": [[ms, "text"], ..] | null }` ([`Lyrics::to_payload`] / [`Payload::from_json`]).
//!   `lines: null` = looked up, nothing found (or the setting is off).
use serde_json::{json, Value};

/// One timed line.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub at_ms: u64,
    pub text: String,
}

/// A song's synced lyrics, sorted by time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    lines: Vec<Line>,
}

impl Lyrics {
    pub fn new(mut lines: Vec<Line>) -> Lyrics {
        lines.sort_by_key(|l| l.at_ms); // stable: equal stamps keep their file order
        Lyrics { lines }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// Index of the line being sung at `pos_ms`: the last line that started at or before it. `None` before the first line.
    pub fn line_at(&self, pos_ms: u64) -> Option<usize> {
        match self.lines.partition_point(|l| l.at_ms <= pos_ms) {
            0 => None,
            n => Some(n - 1),
        }
    }

    /// The line itself.
    pub fn current(&self, pos_ms: u64) -> Option<&str> {
        self.line_at(pos_ms).map(|i| self.lines[i].text.as_str())
    }

    /// `before` lines, the current one (an empty `current` before the first line) and `after` lines around `pos_ms`; indices out of range are `None`.
    /// Returned as `(index, text)` pairs in song order.
    pub fn window(&self, pos_ms: u64, before: usize, after: usize) -> Vec<(usize, &str)> {
        let cur = self.line_at(pos_ms).map_or(-1, |i| i as i64);
        ((cur - before as i64)..=(cur + after as i64))
            .filter(|i| *i >= 0 && (*i as usize) < self.lines.len())
            .map(|i| (i as usize, self.lines[i as usize].text.as_str()))
            .collect()
    }

    /// Start time of the first line after `pos_ms`, `None` after the last one.
    pub fn next_start(&self, pos_ms: u64) -> Option<u64> {
        self.lines.get(self.lines.partition_point(|l| l.at_ms <= pos_ms)).map(|l| l.at_ms)
    }

    pub fn to_payload(&self, track: &str) -> Value {
        json!({ "track": track, "lines": self.lines.iter().map(|l| json!([l.at_ms, l.text])).collect::<Vec<_>>() })
    }
}

/// The decoded `lyrics:changed` payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Payload {
    pub track: String,
    pub lyrics: Option<Lyrics>,
}

impl Payload {
    pub fn from_json(v: &Value) -> Option<Payload> {
        let track = v.get("track")?.as_str()?.to_string();
        let lyrics = v.get("lines").and_then(Value::as_array).map(|a| {
            Lyrics::new(
                a.iter()
                    .filter_map(|p| {
                        let p = p.as_array()?;
                        Some(Line { at_ms: p.first()?.as_u64()?, text: p.get(1)?.as_str()?.to_string() })
                    })
                    .collect(),
            )
        });
        Some(Payload { track, lyrics: lyrics.filter(|l| !l.is_empty()) })
    }
}

/// The key a track is looked up and cached under (what the backend and the UIs both use to match a payload to the playing track).
pub fn track_key(artist: &str, title: &str, album: &str, duration_ms: u64) -> String {
    format!("{}\u{1}{}\u{1}{}\u{1}{}", artist.trim().to_lowercase(), title.trim().to_lowercase(), album.trim().to_lowercase(), duration_ms / 1000)
}

/// Playback clock of the track: the position as of the last media event, advanced by wall time while playing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Clock {
    base_ms: u64,
    at_wall: i64,
    playing: bool,
    duration_ms: u64,
}

impl Clock {
    pub fn set(&mut self, position_ms: u64, duration_ms: u64, playing: bool, wall_ms: i64) {
        *self = Clock { base_ms: position_ms, at_wall: wall_ms, playing, duration_ms };
    }

    pub fn pos(&self, wall_ms: i64) -> u64 {
        let p = if self.playing { self.base_ms + (wall_ms - self.at_wall).max(0) as u64 } else { self.base_ms };
        if self.duration_ms > 0 {
            p.min(self.duration_ms)
        } else {
            p
        }
    }

    pub fn playing(&self) -> bool {
        self.playing
    }
}

/// `[mm:ss.xx]` / `[m:ss]` / `[mm:ss.xxx]` -> ms. `None` for metadata tags (`[ar:..]`) and anything else.
fn stamp(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    if m.is_empty() || m.len() > 3 || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (sec, frac) = match s.split_once(['.', ':']) {
        Some((a, b)) => (a, b),
        None => (s, ""),
    };
    if sec.is_empty() || sec.len() > 2 || !sec.bytes().all(|b| b.is_ascii_digit()) || !frac.bytes().all(|b| b.is_ascii_digit()) || frac.len() > 3 {
        return None;
    }
    let ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac.parse::<u64>().ok()?,
    };
    Some(m.parse::<u64>().ok()? * 60_000 + sec.parse::<u64>().ok()? * 1000 + ms)
}

/// Parse LRC text. Empty lines are kept (an instrumental gap shows nothing); a stamp-less line is dropped. At most 2000 lines.
pub fn parse_lrc(text: &str) -> Lyrics {
    let mut offset: i64 = 0;
    let mut out: Vec<Line> = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim_start_matches('\u{feff}').trim();
        let mut stamps: Vec<u64> = Vec::new();
        while let Some(r) = rest.strip_prefix('[') {
            let Some((tag, after)) = r.split_once(']') else { break };
            if let Some(t) = stamp(tag) {
                stamps.push(t);
            } else if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().parse().unwrap_or(0);
            } else if !tag.contains(':') {
                break;
            }
            rest = after;
        }
        let text = rest.trim();
        // enhanced LRC word stamps `<mm:ss.xx>` are dropped
        let text = if text.contains('<') { strip_word_stamps(text) } else { text.to_string() };
        for t in stamps {
            let at = (t as i64 - offset).max(0) as u64; // [offset:+500] makes the lyrics appear 500 ms earlier
            out.push(Line { at_ms: at, text: text.clone() });
        }
        if out.len() >= 2000 {
            break;
        }
    }
    Lyrics::new(out)
}

fn strip_word_stamps(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '<' {
            let tag: String = it.clone().take_while(|x| *x != '>').collect();
            if stamp(&tag).is_some() && it.clone().any(|x| x == '>') {
                for _ in 0..=tag.chars().count() {
                    it.next();
                }
                continue;
            }
        }
        o.push(c);
    }
    o.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LRC: &str = "[ar:Artist]\n[ti:Song]\n[00:12.00]First\n[00:17.5]Second\n[00:20.123]Third\n[01:00.00][01:30.00]Chorus\n[02:00.00]\n";

    #[test]
    fn parses_stamps_metadata_and_repeats() {
        let l = parse_lrc(LRC);
        let v: Vec<(u64, &str)> = l.lines().iter().map(|x| (x.at_ms, x.text.as_str())).collect();
        assert_eq!(v, vec![(12_000, "First"), (17_500, "Second"), (20_123, "Third"), (60_000, "Chorus"), (90_000, "Chorus"), (120_000, "")]);
    }

    #[test]
    fn offset_shifts_and_garbage_is_ignored() {
        let l = parse_lrc("[offset:+500]\n[00:10.00]a\nnot a line\n[xx:yy]b\n[00:05.00]<00:05.10>wo<00:05.40>rd");
        assert_eq!(l.lines()[0], Line { at_ms: 4_500, text: "word".into() });
        assert_eq!(l.lines()[1], Line { at_ms: 9_500, text: "a".into() });
        assert_eq!(l.len(), 2);
        assert!(parse_lrc("").is_empty() && parse_lrc("[ar:x]").is_empty());
    }

    #[test]
    fn the_current_line_neighbours_and_the_next_start() {
        let l = parse_lrc(LRC);
        assert_eq!(l.line_at(0), None);
        assert_eq!(l.line_at(11_999), None);
        assert_eq!(l.line_at(12_000), Some(0));
        assert_eq!(l.current(19_999), Some("Second"));
        assert_eq!(l.current(5_000_000), Some(""));
        assert_eq!(l.next_start(0), Some(12_000));
        assert_eq!(l.next_start(12_000), Some(17_500));
        assert_eq!(l.next_start(120_000), None);
        let w: Vec<usize> = l.window(18_000, 1, 2).iter().map(|x| x.0).collect();
        assert_eq!(w, vec![0, 1, 2, 3]);
        let w: Vec<usize> = l.window(0, 1, 2).iter().map(|x| x.0).collect();
        assert_eq!(w, vec![0, 1], "before the first line only the upcoming ones");
        assert_eq!(l.window(999_999, 2, 3).last().map(|x| x.0), Some(5));
    }

    #[test]
    fn payload_round_trips_and_empty_means_none() {
        let l = parse_lrc(LRC);
        let p = Payload::from_json(&l.to_payload("k")).unwrap();
        assert_eq!((p.track.as_str(), p.lyrics.as_ref()), ("k", Some(&l)));
        assert_eq!(Payload::from_json(&json!({"track": "k", "lines": null})).unwrap().lyrics, None);
        assert_eq!(Payload::from_json(&json!({"track": "k", "lines": []})).unwrap().lyrics, None);
        assert!(Payload::from_json(&json!({"lines": []})).is_none());
    }

    #[test]
    fn clock_interpolates_only_while_playing_and_clamps() {
        let mut c = Clock::default();
        c.set(10_000, 200_000, true, 1_000);
        assert_eq!(c.pos(1_000), 10_000);
        assert_eq!(c.pos(3_500), 12_500);
        assert_eq!(c.pos(0), 10_000, "a wall clock that went back never rewinds");
        assert_eq!(c.pos(10_000_000), 200_000);
        c.set(10_000, 0, false, 1_000);
        assert_eq!(c.pos(9_000), 10_000);
    }

    #[test]
    fn track_keys_ignore_case_and_sub_second_duration() {
        assert_eq!(track_key(" Queen", "Bohemian Rhapsody", "", 354_999), track_key("queen", "bohemian rhapsody ", "", 354_100));
        assert_ne!(track_key("a", "b", "", 1000), track_key("a", "c", "", 1000));
    }
}
