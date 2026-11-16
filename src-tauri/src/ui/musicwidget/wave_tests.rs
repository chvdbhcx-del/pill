//! The loudness profile with synthetic peaks (no audio hardware): slices, normalisation, floor, merge of partial runs, the LRU cap, seeks.
use super::*;

/// Feeds `peak` at the middle of every slice in `from..to`, a few readings each.
fn run(p: &mut Profile, from: usize, to: usize, peak: impl Fn(usize) -> f32) {
    for i in from..to {
        for _ in 0..4 {
            p.feed((i as f32 + 0.5) / BARS as f32, peak(i));
        }
    }
}

#[test]
fn a_reading_lands_in_the_slice_of_its_position() {
    assert_eq!((slice_of(0.0), slice_of(0.5), slice_of(0.999), slice_of(1.0), slice_of(7.0), slice_of(-1.0)), (0, 16, 31, 31, 31, 0));
    let mut p = Profile::default();
    assert!(p.is_empty());
    assert!(p.feed(0.51, 0.5));
    assert_eq!((p.known(), p.bytes()[16] > 0, p.bytes()[15]), (1, true, 0));
}

#[test]
fn levels_follow_decibels_and_silence_is_a_measurement() {
    assert_eq!((level_of(1.0), level_of(0.0), level_of(0.0001)), (255, 1, 1), "full scale, silence, below the range");
    assert!(level_of(0.5) > level_of(0.25) && level_of(0.25) > level_of(0.1));
    // 6 dB (half the amplitude) is the same step everywhere on the scale
    let d = |a: f32| level_of(a) as i32 - level_of(a / 2.0) as i32;
    assert!((d(1.0) - d(0.4)).abs() <= 1);
}

#[test]
fn a_slice_is_between_its_mean_and_its_loudest_peak() {
    let mut p = Profile::default();
    for v in [0.1, 0.1, 0.1, 0.9] {
        p.feed(0.01, v);
    }
    let lv = p.bytes()[0];
    assert!(lv > level_of(0.1) && lv < level_of(0.9), "{lv}");
}

#[test]
fn the_shape_is_normalised_to_the_loudest_bar_with_a_floor_and_unknown_bars_are_short_and_flat() {
    let mut p = Profile::default();
    run(&mut p, 0, 8, |i| if i == 3 { 0.8 } else { 0.05 });
    let s = p.shape();
    assert_eq!(s[3], (1.0, true), "the loudest bar is full height");
    for b in &s[..8] {
        assert!(b.1 && b.0 >= FLOOR - 1e-4 && b.0 <= 1.0, "{b:?}");
    }
    assert!((s[0].0 - FLOOR).abs() < 1e-3, "the quietest measured bar stands on the floor: {}", s[0].0);
    assert!(s[8..].iter().all(|b| *b == (UNKNOWN_H, false)), "the rest is unknown, short and the same");
    // a track that is the same loudness all through is not a row of floor bars
    let mut flat = Profile::default();
    run(&mut flat, 0, BARS, |_| 0.3);
    assert!(flat.shape().iter().all(|b| b.0 == 1.0 && b.1));
    assert!(Profile::default().shape().iter().all(|b| *b == (UNKNOWN_H, false)));
}

#[test]
fn a_louder_part_arriving_later_rescales_what_was_already_shown() {
    let mut p = Profile::default();
    run(&mut p, 0, 4, |_| 0.1);
    run(&mut p, 4, 5, |_| 0.1);
    let before = p.shape()[0].0;
    run(&mut p, 5, 6, |_| 0.9);
    assert!(p.shape()[5].0 == 1.0 && before == 1.0 && p.shape()[0].0 < 1.0, "the first bar is no longer the loudest");
}

#[test]
fn a_partly_measured_track_is_merged_with_the_next_run_and_known_bars_stay() {
    let mut first = Profile::default();
    run(&mut first, 0, 12, |i| 0.1 + 0.05 * i as f32);
    let saved = first.bytes();
    assert_eq!(first.known(), 12);
    // the next listen: starts from what was remembered, and plays the second half, a little quieter (another volume)
    let mut next = Profile::from_bytes(&saved);
    assert_eq!(next.bytes(), saved);
    run(&mut next, 8, 20, |i| (0.1 + 0.05 * i as f32) * 0.5);
    let m = next.bytes();
    assert_eq!(&m[..12], &saved[..12], "what was known stays, also where the new run measured it again");
    assert!(m[12..20].iter().all(|v| *v > 0), "the gap is filled");
    assert!(m[20..].iter().all(|v| *v == 0), "and what nobody heard is still unknown");
    // the new bars are scaled to the old on the overlap (bars 8..12): no step where the two runs meet
    let step = m[12] as i32 - m[11] as i32;
    assert!(step.abs() < 30, "no cliff at the seam: {step}");
    assert_eq!(next.known(), 20);
}

#[test]
fn a_run_that_overlaps_nothing_is_taken_as_it_is() {
    let mut a = Profile::from_bytes(&{
        let mut b = [0u8; BARS];
        b[0] = 200;
        b
    });
    run(&mut a, 10, 12, |_| 0.5);
    assert_eq!(a.bytes()[10], level_of(0.5));
    assert_eq!(a.bytes()[0], 200);
    assert!(Profile::from_bytes(&[1, 2, 3]).is_empty(), "a wrong length is no profile");
}

#[test]
fn a_seek_forward_shows_placeholders_and_the_part_fills_in_as_it_plays() {
    let mut p = Profile::default();
    run(&mut p, 0, 4, |_| 0.4);
    // jump to the middle: nothing between is invented
    run(&mut p, 16, 18, |_| 0.4);
    let s = p.shape();
    assert!(s[4..16].iter().all(|b| !b.1) && s[16].1 && s[17].1 && !s[18].1);
    run(&mut p, 4, 16, |_| 0.4);
    assert_eq!(p.known(), 18);
}

#[test]
fn the_reading_right_after_a_seek_is_thrown_away() {
    let mut s = Sampler::default();
    assert!(s.accept(10_000, 0.0), "the first reading");
    assert!(s.accept(10_125, 125.0), "steady playback");
    assert!(s.accept(11_000, 1000.0));
    assert!(!s.accept(90_000, 1125.0), "the position jumped: the audio is still the old place");
    assert!(s.accept(90_125, 1250.0), "and the next one is the new place");
    assert!(!s.accept(2_000, 1375.0), "back too");
    s.reset();
    assert!(s.accept(5_000, 9_000.0), "a new track starts clean");
    assert!(s.accept(5_130, 9_125.0));
}

#[test]
fn the_key_is_the_track_not_its_case_or_the_spaces_and_a_different_length_is_another_take() {
    assert_eq!(key_of("Song", "Band", 200_000), key_of("  song ", "BAND", 200_000));
    assert_ne!(key_of("Song", "Band", 200_000), key_of("Song", "Band", 201_000));
    assert_ne!(key_of("Song", "Band", 1), key_of("Band", "Song", 1));
    assert_ne!(key_of("ab", "c", 1), key_of("a", "bc", 1), "the fields do not run into each other");
}

#[test]
fn the_memory_keeps_the_most_recent_tracks_only_and_a_use_makes_a_track_recent() {
    let mut m = Memory::default();
    let row = |v: u8| [v; BARS];
    for k in 0..(KEEP as u64 + 20) {
        m.put(k, row(1 + (k % 200) as u8));
    }
    assert_eq!(m.len(), KEEP);
    assert!(m.get(0).is_none() && m.get(19).is_none(), "the oldest went");
    assert!(m.get(20).is_some() && m.get(KEEP as u64 + 19).is_some());
    // used again: it moves to the end and outlives the ones that were before it
    m.put(20, row(7));
    for k in 1000..1010u64 {
        m.put(k, row(9));
    }
    assert_eq!(m.get(20), Some(row(7)));
    assert!(m.get(21).is_none() && m.len() == KEEP);
    m.put(5000, [0; BARS]);
    assert!(m.get(5000).is_none(), "an empty profile is not kept");
}

#[test]
fn the_file_round_trips_and_a_damaged_one_is_just_empty() {
    let mut m = Memory::default();
    m.put(42, [3; BARS]);
    m.put(7, [9; BARS]);
    let b = m.to_bytes();
    assert_eq!(b.len(), 4 + 2 * (8 + BARS), "one byte per bar and eight for the key");
    let back = Memory::from_bytes(&b);
    assert_eq!((back.get(42), back.get(7), back.len()), (Some([3; BARS]), Some([9; BARS]), 2));
    assert_eq!(Memory::from_bytes(&b[1..]).len(), 0, "no header");
    assert_eq!(Memory::from_bytes(&b[..b.len() - 5]).len(), 1, "a cut row is dropped, the rest is kept");
    assert_eq!(Memory::from_bytes(b"").len(), 0);
    // a file from a bigger cap keeps the most recent rows
    let mut big = MAGIC.to_vec();
    for k in 0..(KEEP as u64 + 5) {
        big.extend_from_slice(&k.to_le_bytes());
        big.extend_from_slice(&[1; BARS]);
    }
    let m = Memory::from_bytes(&big);
    assert!(m.len() == KEEP && m.get(4).is_none() && m.get(KEEP as u64 + 4).is_some());
}
