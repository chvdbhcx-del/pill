//! The loudness of the playing track, as the music widget learns it (a child module of `view`): while a track plays and the widget is on
//! screen the player's peak level is read ~8 times a second (`player_audio::PeakMeter`), put in the bar of the time slice the position is in
//! (`wave::Profile`) and remembered per track (`wave::Memory`, one byte per bar, a small file next to the settings). Nothing is recorded: only
//! peak numbers exist here. The bars' heights are springs (`Loud::bars`), so a bar that gets measured grows to its height; they draw nothing
//! by themselves (the layout decides if and where the waveform is drawn).
//!
//! Idle cost: the 8 Hz tick exists only while a track with a length plays on a visible widget and Pill is not paused by the energy rules
//! (locked, display off, remote desktop hidden); otherwise the view's ticker is the 1 Hz one. A tick that changes no bar asks for no frame.
use crate::ui::musicwidget::wave::{self, Memory, Profile, Sampler, BARS};
use super::*;
use crate::player_audio::PeakSource;

/// The period of the tick while measuring, and of the ordinary one.
pub(super) const MEASURE_MS: u64 = 125;
pub(super) const IDLE_MS: u64 = 1000;
/// A profile that changed is written to disk at most this often while it plays (also when the track changes, it pauses or the widget hides).
const SAVE_EVERY_MS: f64 = 30_000.0;

/// What `PeakSource` is in a test run: nothing (no audio hardware is touched).
#[cfg(test)]
struct Silent;
#[cfg(test)]
impl PeakSource for Silent {
    fn peak(&mut self) -> Option<f32> {
        None
    }
}

fn default_source() -> Box<dyn PeakSource> {
    #[cfg(test)]
    {
        Box::new(Silent)
    }
    #[cfg(not(test))]
    {
        Box::new(crate::player_audio::PeakMeter::default())
    }
}

pub(super) struct Loud {
    prof: Profile,
    key: Option<u64>,
    mem: Option<Memory>,
    sampler: Sampler,
    src: Box<dyn PeakSource>,
    dirty: bool,
    saved_at: f64,
    /// the bars' heights now (0..1 of the waveform's height): measured ones normalised to the loudest, the rest short
    pub bars: [Flow; BARS],
}

impl Loud {
    pub fn new() -> Loud {
        Loud {
            prof: Profile::default(),
            key: None,
            mem: None,
            sampler: Sampler::default(),
            src: default_source(),
            dirty: false,
            saved_at: 0.0,
            bars: std::array::from_fn(|_| Flow::new(wave::UNKNOWN_H, Feel::FADE.within(300.0))),
        }
    }

    pub fn moving(&self) -> bool {
        self.bars.iter().any(Flow::moving)
    }

    pub fn step(&mut self, dt: f64) {
        for b in self.bars.iter_mut() {
            b.step(dt);
        }
    }

    /// Which bars are measured (the rest are drawn dim and flat).
    pub fn known(&self) -> [bool; BARS] {
        let s = self.prof.shape();
        std::array::from_fn(|i| s[i].1)
    }

    /// The bars head for the profile's shape; `at_once` lands them there (nobody is looking).
    fn aim(&mut self, at_once: bool) {
        for (b, (h, _)) in self.bars.iter_mut().zip(self.prof.shape()) {
            if (b.target() - h).abs() > 1e-4 || at_once {
                b.to(h);
                if at_once {
                    b.jump(h);
                }
            }
        }
    }

    /// Writes what is learned (when anything changed).
    fn flush(&mut self, now: f64) {
        if let (Some(k), true) = (self.key, self.dirty) {
            let mem = self.mem.get_or_insert_with(wave::load);
            mem.put(k, self.prof.bytes());
            wave::save(mem);
        }
        self.dirty = false;
        self.saved_at = now;
    }

    /// Another track (or none): remember the old profile, take the new one from memory.
    fn switch(&mut self, key: Option<u64>, now: f64, at_once: bool) {
        self.flush(now);
        self.key = key;
        self.sampler.reset();
        self.prof = match key {
            Some(k) => Profile::from_bytes(&self.mem.get_or_insert_with(wave::load).get(k).unwrap_or([0; BARS])),
            None => Profile::default(),
        };
        self.aim(at_once);
    }

    #[cfg(test)]
    pub fn set_source(&mut self, s: Box<dyn PeakSource>) {
        self.src = s;
    }
}

#[cfg(test)]
impl MusicView {
    /// Test hook: a track that was heard before (what the memory would hold), for the contact sheets.
    pub(crate) fn seed_loudness(&mut self, key: u64, bytes: [u8; BARS]) {
        self.loud.mem.get_or_insert_with(wave::load).put(key, bytes);
    }
}

impl MusicView {
    /// Whether the player's level is being measured now: a track with a length is playing on a visible widget and Pill is not paused.
    pub(super) fn measuring(&self) -> bool {
        self.vis && self.want && self.track.playing && self.track.duration_ms > 0 && self.loud.key.is_some() && !crate::ui::host::energy::verdict().paused
    }

    /// The tick's period: fast while measuring.
    pub(super) fn pace_ms(&self) -> u64 {
        if self.measuring() {
            MEASURE_MS
        } else {
            IDLE_MS
        }
    }

    /// After a reading of the player: the track (and with it the profile) may have changed, and a pause is a good time to write.
    pub(super) fn loud_track(&mut self, now: f64) {
        let t = &self.track;
        let key = (t.has_session && t.duration_ms > 0).then(|| wave::key_of(&t.heading(), &t.sub(), t.duration_ms));
        if key != self.loud.key {
            self.loud.switch(key, now, !self.vis);
        } else if !t.playing {
            self.loud.flush(now);
            self.loud.sampler.reset();
        }
    }

    /// One reading of the level at the position now. True when a bar changed and the card is open enough to show it (a frame is wanted).
    pub(super) fn loud_sample(&mut self, now: f64) -> bool {
        if !self.measuring() {
            return false;
        }
        let pos = self.pos_now(now);
        if !self.loud.sampler.accept(pos, now) {
            return false;
        }
        let Some(peak) = self.loud.src.peak() else { return false };
        let frac = lg::fraction(pos, self.track.duration_ms);
        if !self.loud.prof.feed(frac, peak) {
            return false;
        }
        if !self.loud.dirty {
            self.loud.saved_at = now; // the wait for the next write starts with the first unsaved change
        }
        self.loud.dirty = true;
        let seen = self.open() > 0.05;
        self.loud.aim(!seen); // a closed card is not looking: no animation, no frames
        seen
    }

    /// Once in a while: write a profile that keeps changing (a crash or a kill loses at most that much).
    pub(super) fn loud_housekeeping(&mut self, now: f64) {
        if self.loud.dirty && now - self.loud.saved_at >= SAVE_EVERY_MS {
            self.loud.flush(now);
        }
    }

    /// The widget goes away: write, and stop measuring.
    pub(super) fn loud_hide(&mut self, now: f64) {
        self.loud.flush(now);
        self.loud.sampler.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::host::testing::motion_on;
    use crate::ui::view::Services;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Fake(Rc<Cell<Option<f32>>>);
    impl PeakSource for Fake {
        fn peak(&mut self) -> Option<f32> {
            self.0.get()
        }
    }

    fn view(playing: bool, dur: u64) -> (MusicView, Cx, Rc<Cell<Option<f32>>>) {
        let (svc, f) = Services::fake();
        f.api.reply("settings_get", Ok(json!({ "media.widget": true, "module.media": true, "theme": "dark", "language": "en" })));
        let (mut cx, _q) = Cx::headless_queue(svc);
        let mut v = MusicView::new(&mut cx, Dock::Bottom);
        let level = Rc::new(Cell::new(Some(0.5)));
        v.loud.set_source(Box::new(Fake(level.clone())));
        cx.now_ms = 1000.0;
        v.apply(&json!({ "has_session": true, "playing": playing, "title": "Song", "artist": "Band", "app": "Spotify.exe", "position_ms": 0, "duration_ms": dur, "thumb_id": 0 }), &mut cx);
        v.on_show(&mut cx);
        (v, cx, level)
    }

    #[test]
    fn it_measures_only_while_a_track_with_a_length_plays_on_a_visible_widget_and_ticks_fast_only_then() {
        let _m = motion_on();
        let (mut v, _cx, _l) = view(true, 200_000);
        assert!(v.measuring() && v.pace_ms() == MEASURE_MS);
        v.track.playing = false;
        assert!(!v.measuring() && v.pace_ms() == IDLE_MS, "paused: the slow tick");
        v.track.playing = true;
        v.track.duration_ms = 0;
        assert!(!v.measuring(), "a stream has no length to put a bar in");
        v.track.duration_ms = 200_000;
        v.vis = false;
        assert!(!v.measuring(), "hidden: nothing runs");
    }

    #[test]
    fn readings_become_bars_that_grow_and_a_quiet_gap_in_the_source_changes_nothing() {
        let _m = motion_on();
        let (mut v, mut cx, level) = view(true, 200_000);
        // the card is closed: a reading lands the bar at once, no frame
        let mut now = 1000.0;
        let mut feed = |v: &mut MusicView, cx: &mut Cx, ms: f64, peak: Option<f32>| {
            now += ms;
            cx.now_ms = now;
            level.set(peak);
            v.loud_sample(now)
        };
        assert!(!feed(&mut v, &mut cx, 125.0, Some(0.5)), "closed card: no frame wanted");
        assert!(v.loud.known().iter().filter(|k| **k).count() == 1 && !v.loud.moving(), "but the bar is there");
        assert!(!feed(&mut v, &mut cx, 125.0, None), "a source that cannot tell is no reading");
        v.expand.jump(1.0);
        assert!(feed(&mut v, &mut cx, 20_000.0, Some(0.9)), "an open card shows the new bar: a frame");
        assert_eq!(v.loud.known().iter().filter(|k| **k).count(), 2);
    }

    #[test]
    fn an_open_card_grows_the_new_bar_and_a_seek_throws_one_reading_away() {
        let _m = motion_on();
        let (mut v, mut cx, level) = view(true, 200_000);
        v.expand.jump(1.0);
        let mut now = 1000.0;
        let mut step = |v: &mut MusicView, cx: &mut Cx, ms: f64| {
            now += ms;
            cx.now_ms = now;
            v.loud_sample(now)
        };
        level.set(Some(0.3));
        assert!(step(&mut v, &mut cx, 125.0), "a bar was measured and the card shows it: a frame");
        assert!(v.loud.moving(), "and it grows (a spring), it does not jump");
        for _ in 0..100 {
            v.loud.step(8.0);
        }
        assert!(!v.loud.moving());
        // a seek forward by a minute: the first reading after it is dropped, the next one lands in the new place
        v.commit_seek(0.5, &mut cx);
        level.set(Some(0.8));
        assert!(!step(&mut v, &mut cx, 125.0), "the audio is still the old place");
        assert!(step(&mut v, &mut cx, 125.0), "now it is the new place");
        let k = v.loud.known();
        assert!(k[0] && k[16] && !k[8], "slice 0 and slice 16 are measured, what lies between is not invented");
    }

    #[test]
    fn a_track_is_remembered_across_the_change_and_a_new_one_starts_unknown() {
        let _m = motion_on();
        let (mut v, mut cx, level) = view(true, 200_000);
        level.set(Some(0.4));
        v.loud_sample(1125.0);
        let key = v.loud.key;
        assert!(key.is_some() && v.loud.dirty);
        let bytes = v.loud.prof.bytes();
        // another track: the old profile goes to the memory, the new one is empty
        cx.now_ms = 5000.0;
        v.apply(&json!({ "has_session": true, "playing": true, "title": "Other", "artist": "Band", "app": "Spotify.exe", "position_ms": 0, "duration_ms": 180_000, "thumb_id": 0 }), &mut cx);
        assert!(v.loud.key != key && v.loud.prof.is_empty() && v.loud.known().iter().all(|k| !k));
        assert_eq!(v.loud.mem.as_ref().and_then(|m| m.get(key.unwrap())), Some(bytes), "the first track is remembered");
        // and back: from the first second the whole profile is there
        cx.now_ms = 9000.0;
        v.apply(&json!({ "has_session": true, "playing": true, "title": "Song", "artist": "Band", "app": "Spotify.exe", "position_ms": 0, "duration_ms": 200_000, "thumb_id": 0 }), &mut cx);
        assert_eq!(v.loud.prof.bytes(), bytes);
    }

    #[test]
    fn a_pause_writes_and_hiding_writes_and_a_playing_profile_is_written_every_half_minute() {
        let _m = motion_on();
        let (mut v, mut cx, level) = view(true, 200_000);
        level.set(Some(0.4));
        v.loud_sample(1125.0);
        assert!(v.loud.dirty);
        v.loud_housekeeping(1125.0 + SAVE_EVERY_MS - 1.0);
        assert!(v.loud.dirty, "not yet");
        v.loud_housekeeping(1125.0 + SAVE_EVERY_MS + 1.0);
        assert!(!v.loud.dirty, "now");
        level.set(Some(0.9));
        v.loud_sample(40_000.0);
        assert!(v.loud.dirty);
        cx.now_ms = 41_000.0;
        v.apply(&json!({ "has_session": true, "playing": false, "title": "Song", "artist": "Band", "app": "Spotify.exe", "position_ms": 5000, "duration_ms": 200_000, "thumb_id": 0 }), &mut cx);
        assert!(!v.loud.dirty, "a pause writes");
        v.loud_sample(42_000.0);
        assert!(!v.loud.dirty, "and a paused track is not measured");
        let k = v.loud.key.unwrap();
        v.loud.dirty = true;
        v.on_hide(&mut cx);
        assert!(!v.loud.dirty && v.loud.mem.as_ref().is_some_and(|m| m.get(k).is_some()), "hiding writes");
    }
}
