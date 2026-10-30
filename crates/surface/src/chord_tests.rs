//! Chords mode's pads: the Harmonizer, the chord sets and positions (docs/scales.md,
//! "Chord voicings").

use super::*;
use crate::tests_support::view;

/// Chords mode on, with `lua` run on `lib.pads` (`p`) first.
fn chords(lua: &str) -> (Surface, View) {
    let (mut s, mut v) = (Surface::new(), view());
    v.user_chords = vec![vec![Vec::new(); 12]; 16];
    let now = Instant::now();
    s.event(&Event::Button("BUTTON_CHORDS", true), &v, now);
    s.event(&Event::Button("BUTTON_CHORDS", false), &v, now);
    s.eval::<mlua::Value>(&format!("local p = require('lib.pads'); {lua}")).unwrap();
    (s, v)
}

fn pad(s: &Surface, v: &View, n: usize) -> Vec<i64> {
    let chords = s.pads(v).chords.expect("Chords mode plays chords");
    chords[n - 1].iter().copied().filter(|&x| x < 128).collect()
}

#[test]
fn the_harmonizer_plays_chords() {
    // As a project starts: the chromatic scale, Major.
    let (s, mut v) = chords("");
    assert_eq!(pad(&s, &v, 1), [60, 64, 67]);
    assert_eq!(pad(&s, &v, 2), [61, 65, 68]);
    // C major, 1-3-5: pad 2 is D minor.
    v.scale_kind = 1;
    assert_eq!(pad(&s, &v, 2), [62, 65, 69]);
    assert_eq!(pad(&s, &v, 7), [71, 74, 77], "B diminished");
}

#[test]
fn chord_sets_and_positions() {
    // Minor 3: C minor over C2, then A flat.
    let (s, v) = chords("p.chord_mode = 2; p.chord_types.set = 11");
    assert_eq!(pad(&s, &v, 1), [48, 60, 63, 67]);
    assert_eq!(pad(&s, &v, 2), [44, 60, 63, 68]);
    assert!(pad(&s, &v, 13).is_empty(), "pads 13-16 play nothing");
    // Position 1 inverts the chord over the bass.
    s.eval::<mlua::Value>("require('lib.pads').chord_position = 11").unwrap();
    assert_eq!(pad(&s, &v, 1), [48, 63, 67, 72]);
}

#[test]
fn auto_follows_the_pads_played() {
    let (mut s, mut v) = chords("p.chord_position = 19");
    v.scale_kind = 1;
    assert_eq!(pad(&s, &v, 4), [65, 69, 72], "no anchor yet: F at Root");
    let now = Instant::now();
    s.event(&Event::PadOn(1, 0.5), &v, now);
    assert_eq!(pad(&s, &v, 1), [60, 64, 67]);
    s.event(&Event::PadOff(1), &v, now);
    // C is the anchor: F leads down to C-F-A.
    assert_eq!(pad(&s, &v, 4), [60, 65, 69]);
}
