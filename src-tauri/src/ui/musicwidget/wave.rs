//! The waveform of the open card: the real loudness of the track, learned while it plays and remembered. Pure (no audio, no clock, no files of
//! its own): the view feeds it peak readings, `store` below keeps the result.
//!
//! * The player gives no audio file (the media session has metadata only, a stream has no file), so there is no pre-scan. The playing app's
//!   peak level is read ~8 times a second (`player_audio::PeakMeter`: the app's own audio session, else the default output device) and put in
//!   the bar of the time slice the position is in. Only peak values are kept: nothing is recorded, no audio content is stored.
//! * A bar's level is the blend of the loudest and the mean peak of its slice (about the 75th percentile, no sample buffer), on a dB scale.
//! * [`Profile::shape`] normalises the whole profile to its loudest bar with a floor, so quiet parts stay visible. Bars not measured yet are
//!   [`UNKNOWN_H`]: short and flat (the view draws them dim).
//! * A profile is remembered per track (key = title + artist + duration), one byte per bar, the 500 most recently used tracks. A track that was
//!   measured only partly is merged with the next listen: what is known stays, the new run fills the gaps (scaled to the old one where both
//!   cover the same bars, so a different volume does not make steps).
//! * Limitation: with only the default output device's meter (the player has no session of its own that can be told apart) other sounds
//!   playing at the same time are measured too, and that cannot be told from the music. A system sound makes a bar louder than it is; the
//!   next listen does not replace it (known bars stay).
use std::collections::VecDeque;

/// Bars in the waveform and the slices of the track.
pub const BARS: usize = 32;
/// Shortest bar (fraction of the full height), also where the quietest measured bar stands.
pub const FLOOR: f32 = 0.25;
/// The height of a bar that was not measured yet.
pub const UNKNOWN_H: f32 = 0.2;
/// The quietest peak that counts (dB below full scale); below it is silence.
const DB_RANGE: f32 = 50.0;
/// Tracks remembered.
pub const KEEP: usize = 500;

/// A peak (linear amplitude 0..1) as a level 1..=255 on a dB scale; 0 stays "unknown", so silence is 1.
pub fn level_of(peak: f32) -> u8 {
    let db = 20.0 * peak.clamp(1e-5, 1.0).log10();
    1 + (((db + DB_RANGE) / DB_RANGE).clamp(0.0, 1.0) * 254.0).round() as u8
}

/// The key of a track: its identity as the user knows it (the length keeps two takes of one title apart). FNV-1a, not a security hash.
pub fn key_of(title: &str, artist: &str, duration_ms: u64) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in title.trim().to_lowercase().bytes().chain([1]).chain(artist.trim().to_lowercase().bytes()).chain([1]).chain(duration_ms.to_le_bytes()) {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Acc {
    sum: f32,
    n: u32,
    max: f32,
}

impl Acc {
    fn level(&self) -> u8 {
        if self.n == 0 {
            0
        } else {
            level_of((self.max + self.sum / self.n as f32) / 2.0)
        }
    }
}

/// What is known of one track's loudness.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    /// what an earlier listen left (0 = unknown)
    base: [u8; BARS],
    /// this run's readings per slice
    acc: [Acc; BARS],
    /// base and this run merged
    lv: [u8; BARS],
}

impl Default for Profile {
    fn default() -> Profile {
        Profile { base: [0; BARS], acc: [Acc::default(); BARS], lv: [0; BARS] }
    }
}

/// The slice of the track a position falls in.
pub fn slice_of(frac: f32) -> usize {
    ((frac.clamp(0.0, 1.0) * BARS as f32) as usize).min(BARS - 1)
}

impl Profile {
    /// A profile from what was remembered.
    pub fn from_bytes(b: &[u8]) -> Profile {
        let mut p = Profile::default();
        if b.len() == BARS {
            p.base.copy_from_slice(b);
            p.lv = p.base;
        }
        p
    }

    /// What to remember: one byte per bar, 0 = unknown.
    pub fn bytes(&self) -> [u8; BARS] {
        self.lv
    }

    pub fn is_empty(&self) -> bool {
        self.lv.iter().all(|v| *v == 0)
    }

    /// Bars that have a measurement.
    pub fn known(&self) -> usize {
        self.lv.iter().filter(|v| **v > 0).count()
    }

    /// A reading `peak` (0..1) at `frac` of the track. True when the merged level of that bar changed.
    pub fn feed(&mut self, frac: f32, peak: f32) -> bool {
        let i = slice_of(frac);
        let a = &mut self.acc[i];
        let p = peak.clamp(0.0, 1.0);
        a.sum += p;
        a.n += 1;
        a.max = a.max.max(p);
        let before = self.lv[i];
        self.merge();
        self.lv[i] != before
    }

    /// `lv` from `base` and `acc`: a known bar stays; an unknown one takes this run's reading, scaled by how this run compares with the old one
    /// on the bars both have (at least 2; otherwise as it is).
    fn merge(&mut self) {
        let (mut old, mut new, mut n) = (0.0f32, 0.0f32, 0);
        for i in 0..BARS {
            let (b, a) = (self.base[i], self.acc[i].level());
            if b > 0 && a > 0 {
                old += b as f32;
                new += a as f32;
                n += 1;
            }
        }
        let k = if n >= 2 && new > 0.0 { (old / new).clamp(0.25, 4.0) } else { 1.0 };
        for i in 0..BARS {
            self.lv[i] = match (self.base[i], self.acc[i].level()) {
                (0, 0) => 0,
                (0, a) => ((a as f32 * k).round().clamp(1.0, 255.0)) as u8,
                (b, _) => b,
            };
        }
    }

    /// The bars to draw: `(height 0..1, measured)`. Measured ones are normalised to the loudest with [`FLOOR`] under them.
    pub fn shape(&self) -> [(f32, bool); BARS] {
        let top = self.lv.iter().copied().max().unwrap_or(0).max(1) as f32;
        let low = self.lv.iter().copied().filter(|v| *v > 0).min().unwrap_or(0) as f32;
        // quiet parts stay visible: the quietest measured bar stands on the floor, the loudest at full height
        std::array::from_fn(|i| {
            let v = self.lv[i];
            if v == 0 {
                (UNKNOWN_H, false)
            } else if top <= low {
                (1.0, true)
            } else {
                (FLOOR + (1.0 - FLOOR) * (v as f32 - low) / (top - low), true)
            }
        })
    }
}

/// Throws away the reading right after the position jumped (a seek): the audio that is audible then still belongs to the old place.
#[derive(Debug, Default)]
pub struct Sampler {
    last: Option<(u64, f64)>,
}

/// How far the position may be from where steady playback puts it before it counts as a jump.
const JUMP_MS: f64 = 1500.0;

impl Sampler {
    /// Is a reading taken at host time `now` (ms) while the position is `pos_ms` to be kept?
    pub fn accept(&mut self, pos_ms: u64, now: f64) -> bool {
        let ok = match self.last {
            Some((p, t)) => (pos_ms as f64 - (p as f64 + (now - t))).abs() <= JUMP_MS,
            None => true,
        };
        self.last = Some((pos_ms, now));
        ok
    }

    pub fn reset(&mut self) {
        self.last = None;
    }
}

// ------------------------------------------------------------------------------------------------------------------------------ the memory

/// The remembered profiles, the most recently used last (the file's order): at most [`KEEP`].
#[derive(Debug, Default)]
pub struct Memory {
    rows: VecDeque<(u64, [u8; BARS])>,
}

const ROW: usize = 8 + BARS;
const MAGIC: &[u8; 4] = b"PWF1";

impl Memory {
    pub fn get(&self, key: u64) -> Option<[u8; BARS]> {
        self.rows.iter().find(|r| r.0 == key).map(|r| r.1)
    }

    /// Remember `bytes` for `key` (the most recent now); an empty profile is not worth a row. The oldest go beyond [`KEEP`].
    pub fn put(&mut self, key: u64, bytes: [u8; BARS]) {
        if bytes.iter().all(|v| *v == 0) {
            return;
        }
        self.rows.retain(|r| r.0 != key);
        self.rows.push_back((key, bytes));
        while self.rows.len() > KEEP {
            self.rows.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.rows.len() * ROW);
        out.extend_from_slice(MAGIC);
        for (k, b) in &self.rows {
            out.extend_from_slice(&k.to_le_bytes());
            out.extend_from_slice(b);
        }
        out
    }

    /// A damaged or foreign file is an empty memory (never an error: it is a cache).
    pub fn from_bytes(b: &[u8]) -> Memory {
        let mut m = Memory::default();
        let Some(body) = b.strip_prefix(MAGIC.as_slice()) else { return m };
        for row in body.chunks_exact(ROW).rev().take(KEEP).collect::<Vec<_>>().into_iter().rev() {
            let key = u64::from_le_bytes(row[..8].try_into().unwrap_or([0; 8]));
            let mut bars = [0u8; BARS];
            bars.copy_from_slice(&row[8..]);
            m.rows.push_back((key, bars));
        }
        m
    }
}

/// The file the memory lives in (next to the settings). Names and paths never go to a log.
fn path() -> std::path::PathBuf {
    crate::store::dir().join("waveforms.bin")
}

pub fn load() -> Memory {
    if cfg!(test) {
        return Memory::default(); // a test run never touches the user's file
    }
    std::fs::read(path()).map(|b| Memory::from_bytes(&b)).unwrap_or_default()
}

/// Writes the memory (temp file and rename, so a crash never leaves half a file). Best effort.
pub fn save(m: &Memory) {
    if cfg!(test) {
        return;
    }
    let p = path();
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, m.to_bytes()).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(test)]
#[path = "wave_tests.rs"]
mod tests;
