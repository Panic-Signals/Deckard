//! What a pad hit plays, with no device and no window in the way: the pad core's
//! own tests (see `apps/viewer/src/pad_play.rs`).

use crate::mk3_io::{PadRoute, pad_notes};
use crate::pad_play::Pads;
use deckard_engine::Live;
use deckard_model::pad_touch::{PadTouch, TouchMode};

/// Pad mode: group 0's sounds, played by pad, every hit at its own velocity.
fn pad_route_pad_mode() -> PadRoute {
    PadRoute { play: true, group: 0, keys: None, chords: None, velocity: None, scaling: 0, fixed: 100, touches: [PadTouch::default(); 16] }
}

/// A sound's base key: Maschine's default, C3 (`mk3_io::PAD_PITCH`).
const C3: i64 = 60;

fn note_on(sound: usize, pitch: i64, velocity: i64) -> Live {
    Live::NoteOn { group: 0, sound, pitch, velocity, length: i64::MAX }
}

fn note_release(sound: usize, pitch: i64) -> Live {
    Live::NoteRelease { group: 0, sound, pitch }
}

#[test]
fn a_pad_hit_plays_the_pad_s_sound_once() {
    let route = pad_route_pad_mode();
    let mut pads = Pads::new();
    assert_eq!(pads.hit(&route, 1, 100), vec![note_on(0, C3, 100)]);
    assert_eq!(pads.release(1), vec![note_release(0, C3)]);
}

/// A pad held on one sound while the route moves on stops the note it started, not
/// the one the route would play now.
#[test]
fn a_pad_releases_the_note_it_started_when_the_route_changed() {
    let mut pads = Pads::new();
    // Pad mode: pad 3 plays sound 2 at C3.
    assert_eq!(pads.hit(&pad_route_pad_mode(), 3, 100), vec![note_on(2, C3, 100)]);
    // Chords mode now: pad 3 would play sound 0's D#3, G3 and A#3.
    let mut chords = [[128; 16]; 16];
    chords[2][..3].copy_from_slice(&[67, 71, 74]);
    let moved_on = PadRoute { keys: Some((0, [128; 16])), chords: Some(chords), ..pad_route_pad_mode() };
    assert_eq!(pad_notes(&moved_on, 3), [(0, 67), (0, 71), (0, 74)], "the route would play the chord now");
    assert_eq!(pads.release(3), vec![note_release(2, C3)], "the note it started, not the chord");
}

/// A hit with no engine running waits for one: taken once, and never again.
#[test]
fn a_hit_with_no_engine_is_pending_once() {
    let route = pad_route_pad_mode();
    let mut pads = Pads::new();
    pads.set_engine(false);
    assert_eq!(pads.hit(&route, 1, 100), vec![], "no engine: nothing plays now");
    assert_eq!(pads.hit(&route, 2, 90), vec![]);
    assert_eq!(pads.pending(), vec![(0, 0, C3, 100), (0, 1, C3, 90)]);
    assert_eq!(pads.pending(), vec![], "taken once");
}

/// A pad held still for a long time sends only the changes: 100 reports of the same
/// pressure send once.
#[test]
fn a_long_pressure_hold_sends_no_duplicate_live_events() {
    let mut route = pad_route_pad_mode();
    route.touches[0] = PadTouch { mode: TouchMode::Aftertouch, ..PadTouch::default() };
    let mut pads = Pads::new();
    assert_eq!(pads.hit(&route, 1, 100), vec![note_on(0, C3, 100)]);
    let slot = pads.sounding(1);
    assert_eq!(slot, Some((0, 0, C3)), "what the pad is sounding");
    assert_eq!(pads.pressure(&route, 1, 0.5, slot).len(), 1, "the first report sends");
    for _ in 0..99 {
        assert_eq!(pads.pressure(&route, 1, 0.5, slot), vec![], "the same value sends nothing");
    }
}
