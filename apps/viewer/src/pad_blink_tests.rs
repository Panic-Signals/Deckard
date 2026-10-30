//! Playback blinks a pad as each note starts, as a drum hit does, however long the note
//! is held (the sound still plays the whole note): every note that started since the
//! last look lights for [`deckard_engine::output::FLASH`], so none is missed between
//! looks and each blink is as long at any tempo.

use crate::blink::Blinks;
use deckard_engine::{Mode, NoteEvent, Timeline};
use deckard_model::PPQ;
use std::time::{Duration, Instant};

const FLASH: Duration = deckard_engine::output::FLASH;

fn note(tick: i64, sound: usize, length: i64) -> NoteEvent {
    NoteEvent { tick, group: 0, sound, pitch: 60, velocity: 100, length, probability: 100, midi: None, cue: None, expr: None, from: None }
}

fn timeline(events: Vec<NoteEvent>) -> Timeline {
    Timeline { events, length: 16 * PPQ, ..Timeline::default() }
}

fn sounds(lit: &std::collections::HashSet<(usize, usize, i64)>) -> Vec<usize> {
    let mut v: Vec<usize> = lit.iter().map(|&(_, s, _)| s).collect();
    v.sort();
    v
}

const M: Mode = Mode::Scene(0);

#[test]
fn a_held_chord_blinks_like_a_hit() {
    let t = timeline(vec![note(0, 0, 4 * PPQ), note(0, 1, 4 * PPQ), note(0, 2, 4 * PPQ)]);
    let (mut b, t0) = (Blinks::default(), Instant::now());
    assert_eq!(sounds(&b.update(&t, M, 0, t0)), [0, 1, 2], "lit as the chord starts");
    assert_eq!(sounds(&b.update(&t, M, 10, t0 + FLASH / 2)), [0, 1, 2]);
    assert!(b.update(&t, M, PPQ, t0 + FLASH).is_empty(), "out while the chord is still held");
}

#[test]
fn a_note_between_looks_still_blinks() {
    let t = timeline(vec![note(PPQ, 0, 1)]);
    let (mut b, t0) = (Blinks::default(), Instant::now());
    assert!(b.update(&t, M, PPQ - 10, t0).is_empty());
    assert_eq!(sounds(&b.update(&t, M, PPQ + PPQ / 2, t0 + Duration::from_millis(30))), [0], "started since");
    assert_eq!(sounds(&b.update(&t, M, PPQ + PPQ / 2 + 5, t0 + Duration::from_millis(60))), [0], "for a while");
    assert!(b.update(&t, M, 2 * PPQ, t0 + Duration::from_millis(30) + FLASH).is_empty());
}

#[test]
fn the_loops_first_note_blinks_as_it_comes_round() {
    let t = timeline(vec![note(0, 3, PPQ)]);
    let (mut b, t0) = (Blinks::default(), Instant::now());
    assert!(b.update(&t, M, 16 * PPQ - 10, t0).is_empty());
    assert_eq!(sounds(&b.update(&t, M, 20, t0 + Duration::from_millis(16))), [3]);
}

#[test]
fn a_jump_doesnt_blink_what_was_skipped() {
    let t = timeline(vec![note(PPQ, 0, 1), note(8 * PPQ, 1, 1)]);
    let (mut b, t0) = (Blinks::default(), Instant::now());
    b.update(&t, M, 0, t0);
    assert_eq!(sounds(&b.update(&t, M, 8 * PPQ, t0 + Duration::from_millis(16))), [1], "only what starts where it lands");
}
