//! Behavioural tests of the pet simulation (no GPU): keyframes at known times, transitions, reduced motion, look / energy layers, the idle
//! schedule and a smoke run of every pet in every state.
use super::engine::{box_size, catalogue, eye_line_y, face_anchor, load, Place, Size};
use super::interp::Mat;
use super::model::{Pet, STATES};
use super::sim::{Act, Sim};
use std::sync::Arc;

fn sim(id: &str, state: usize) -> Sim {
    let pet = load(id).unwrap_or_else(|| panic!("{id}"));
    let mut s = Sim::new(pet, false, 1, state, None);
    s.set_idle(false);
    s
}

fn run(s: &mut Sim, ms: f32) {
    let mut left = ms;
    while left > 0.0 {
        let dt = left.min(10.0);
        s.tick(dt);
        left -= dt;
    }
}

fn node(p: &Pet, class: &str) -> usize {
    p.nodes.iter().position(|n| n.class.split_whitespace().any(|c| c == class)).unwrap_or_else(|| panic!("{} has no .{class}", p.id))
}

fn close(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() < eps
}

fn mclose(a: Mat, b: Mat, eps: f32) -> bool {
    (0..6).all(|i| close(a[i], b[i], eps))
}

#[test]
fn done_jump_hits_its_keyframe() {
    // pets.css pet-jump: 45 % = translateY(-16%) scale(0.94, 1.1) about the rig's bottom centre (50% 100%); the rig box is the 48 x 48 viewBox
    let mut s = sim("cat", 3);
    run(&mut s, 360.0);
    let m = s.local_matrix(1);
    let expect = [0.94, 0.0, 0.0, 1.1, 24.0 - 0.94 * 24.0, 48.0 - 7.68 - 1.1 * 48.0];
    assert!(mclose(m, expect, 1e-2), "{m:?} vs {expect:?}");
}

#[test]
fn breathing_loops_forever() {
    let mut s = sim("cat", 0);
    run(&mut s, 1800.0); // half of pet-breath (3.6 s, ease-in-out): scale(1.012, 1.034)
    let m = s.local_matrix(1);
    assert!(close(m[0], 1.012, 2e-3) && close(m[3], 1.034, 2e-3), "{m:?}");
    assert!(s.tick(16.0), "an infinite animation keeps the frames coming");
}

#[test]
fn reduced_motion_is_still() {
    let mut s = sim("cat", 3);
    s.set_reduced(true);
    run(&mut s, 400.0);
    let a: Vec<Mat> = (0..s.node_count()).map(|i| s.local_matrix(i)).collect();
    assert!(!s.tick(16.0), "nothing moves under reduce motion");
    run(&mut s, 900.0);
    let b: Vec<Mat> = (0..s.node_count()).map(|i| s.local_matrix(i)).collect();
    assert_eq!(a, b);
    assert!(!s.poke(), "no petting animation under reduce motion");
    assert!(s.next_wake_ms().is_none(), "no idle behaviour either");
}

#[test]
fn state_change_transitions_instead_of_cutting() {
    // pets.css: `.head { transition: transform .18s }`, thinking tilts the head by -6 deg
    let mut s = sim("cat", 0);
    let head = node(&s.pet, "head");
    assert_eq!(s.style(head).tf[2], 0.0);
    s.set_state(2, None);
    run(&mut s, 90.0);
    let mid = s.style(head).tf[2];
    assert!(mid < -0.5 && mid > -5.9, "half way through the transition: {mid}");
    run(&mut s, 400.0);
    assert!(close(s.style(head).tf[2], -6.0, 1e-3));
}

#[test]
fn a_pet_created_in_a_state_starts_there() {
    let s = sim("cat", 2);
    let head = node(&s.pet, "head");
    assert!(close(s.style(head).tf[2], -6.0, 1e-3), "no transition from calm at creation");
}

#[test]
fn look_turns_the_head_and_shifts_the_eyes() {
    let mut s = sim("cat", 0);
    let head = node(&s.pet, "head");
    let eyes = node(&s.pet, "eyes");
    s.set_look(1.0, 0.0);
    run(&mut s, 600.0);
    assert!(close(s.style(head).tf[2], 4.0, 0.05), "{}", s.style(head).tf[2]); // rotate(lx * 4deg)
    assert!(close(s.style(eyes).tf[0], 1.7, 0.05), "{}", s.style(eyes).tf[0]); // translate(lx * 1.7px)
    s.set_look(-1.0, 0.0);
    run(&mut s, 600.0);
    assert!(close(s.style(head).tf[2], -4.0, 0.05));
    s.clear_look();
    run(&mut s, 600.0);
    assert!(close(s.style(head).tf[2], 0.0, 0.05));
}

#[test]
fn asleep_ignores_the_gaze() {
    let mut s = sim("cat", 6);
    let eyes = node(&s.pet, "eyes");
    let before = s.style(eyes).tf;
    s.set_look(1.0, 1.0);
    run(&mut s, 400.0);
    assert_eq!(s.style(eyes).tf, before);
}

#[test]
fn energy_replaces_the_steady_beat() {
    let mut s = sim("slime", 1);
    run(&mut s, 400.0);
    let steady = s.local_matrix(1);
    s.set_energy(Some(1.0));
    run(&mut s, 400.0);
    let loud = s.local_matrix(1);
    s.set_energy(Some(0.0));
    run(&mut s, 400.0);
    let quiet = s.local_matrix(1);
    assert!(!mclose(loud, quiet, 1e-3), "energy moves the rig: {loud:?} {quiet:?}");
    assert!(!mclose(steady, loud, 1e-4));
    s.set_energy(None);
    run(&mut s, 400.0);
    assert!(s.tick(16.0));
    s.set_state(0, None);
    s.set_energy(Some(1.0));
    assert!(s.energy.is_none(), "energy only exists in the music state");
}

#[test]
fn poke_is_rate_limited_and_expires() {
    let mut s = sim("cat", 0);
    assert!(s.poke());
    run(&mut s, 500.0);
    assert!(!s.poke(), "at most once a second");
    run(&mut s, 600.0);
    assert!(s.poke());
    run(&mut s, 1500.0);
    // after 1.4 s the hearts are gone again
    let hearts = node(&s.pet, "fx-heart");
    assert!(s.style(hearts).dn);
}

#[test]
fn idle_schedule_replays_with_the_same_seed() {
    let log = |seed: u64| {
        let mut s = Sim::new(load("bunny").unwrap(), false, seed, 0, None);
        let mut v = Vec::new();
        for i in 0..2000 {
            s.tick(16.0);
            if let Some(a) = s.current_act() {
                if v.last() != Some(&(a, i / 100)) {
                    v.push((a, i / 100));
                }
            }
        }
        v
    };
    assert_eq!(log(42), log(42));
    assert!(!log(42).is_empty(), "something idle happens within 32 s");
    assert_ne!(log(42), log(43));
    assert!(log(42).iter().any(|(a, _)| *a == Act::Blink));
}

#[test]
fn the_idle_blink_closes_the_eyes() {
    let mut s = Sim::new(load("cat").unwrap(), false, 7, 0, None);
    let eye = node(&s.pet, "eye");
    let mut closed = false;
    for _ in 0..3000 {
        s.tick(8.0);
        let sy = s.local_matrix(eye)[3];
        if sy < 0.3 {
            closed = true;
        }
    }
    assert!(closed, "a blink squashes .eye to scaleY(0.08)");
}

#[test]
fn every_pet_survives_every_state() {
    for info in catalogue() {
        let pet = load(info.id).unwrap();
        for st in 0..STATES.len() {
            for side in [false, true] {
                let mut s = Sim::new(Arc::clone(&pet), side, 3, st, None);
                s.set_idle(true);
                for k in 0..300 {
                    s.tick(16.0);
                    if k == 40 {
                        s.set_look(0.7, -0.3);
                    }
                    if k == 80 {
                        s.poke();
                    }
                    if k == 120 {
                        s.set_energy(Some(0.6));
                    }
                    if k == 200 {
                        s.set_state((st + 1) % 7, None);
                    }
                    if k == 250 {
                        s.set_reduced(true);
                    }
                }
                for i in 0..s.node_count() {
                    let m = s.local_matrix(i);
                    assert!(m.iter().all(|v| v.is_finite()), "{} {} node {i}: {m:?}", info.id, STATES[st]);
                }
            }
        }
    }
}

#[test]
fn hooks_move_the_walkers() {
    for id in ["koi", "snail", "hamster", "swarm"] {
        let mut s = sim(id, if id == "swarm" { 1 } else { 0 });
        let hook_nodes: Vec<usize> = s.pet.nodes.iter().enumerate().filter(|(_, n)| ["k-flip", "sn-flip", "hm-wheel", "sw"].iter().any(|c| n.class.split_whitespace().any(|x| x == *c))).map(|(i, _)| i).collect();
        assert!(!hook_nodes.is_empty(), "{id}");
        let before: Vec<Mat> = hook_nodes.iter().map(|i| s.local_matrix(*i)).collect();
        run(&mut s, 1500.0);
        let after: Vec<Mat> = hook_nodes.iter().map(|i| s.local_matrix(*i)).collect();
        assert_ne!(before, after, "{id}: the JS-driven part moves");
    }
}

#[test]
fn box_sizes_follow_pets_css() {
    // capsule: S 20 / M 27 / L 30 px x --s; claude8: whole cells (round to 14 / 7 / 14)
    assert_eq!(box_size("cat", Size::M, Place::Cap, 1.0), (27.0, 27.0));
    assert_eq!(box_size("cat", Size::L, Place::Cap, 2.0), (60.0, 60.0));
    let (w, h) = box_size("kaomoji", Size::S, Place::Cap, 1.0);
    assert!(close(w / h, 2.0, 1e-4));
    assert_eq!(box_size("claude8", Size::M, Place::Cap, 1.0).1, 21.0);
    assert_eq!(box_size("claude8", Size::M, Place::Cap, 0.8).1, 14.0);
    assert_eq!(box_size("claude8", Size::L, Place::Head, 1.0).1, 42.0);
    assert_eq!(box_size("cat", Size::M, Place::Head, 1.0).1, 28.0);
}

#[test]
fn the_eyes_pet_is_lifted_onto_the_text_line() {
    // the art carries translate(0 -5.6): the eye line is above the middle of its 60 x 40 box
    let y = eye_line_y("eyes");
    assert!(y > 0.2 && y < 0.5, "{y}");
    let (x, y2) = face_anchor("cat");
    assert!(close(x, 0.5, 0.05) && y2 > 0.35 && y2 < 0.65, "{x} {y2}");
}

#[test]
fn catalogue_lists_every_group() {
    let c = catalogue();
    assert!(c.len() >= 28);
    for g in ["characters", "creatures", "things"] {
        assert!(c.iter().any(|p| p.group == g), "{g}");
    }
    assert!(c.iter().any(|p| p.id == "claude8" && !p.name_ru.is_empty()));
}
