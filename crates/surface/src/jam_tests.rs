//! The Maschine JAM's script (`lua/devices/maschine_jam/`), driven as the driver drives it. No
//! test opens real hardware.

use std::time::{Duration, Instant};

use deckard_jam::leds::{SHADE_BRIGHT, SHADE_DIM, SHADE_FAINT, colour};
use deckard_jam::map::METER_LEDS;

use crate::device::{Device, Out, devices};
use crate::tests_support::view;
use crate::{Action, GroupView, PatternView, SceneView, View};

const WHITE: i64 = 16;

fn jam() -> Device {
    let mut d = Device::new("maschine_jam", None);
    assert_eq!(d.error(), None);
    let v = jam_view();
    d.open(&v, Instant::now());
    d
}

/// Two groups, Drums with three patterns and Bass with one; two scenes, the first focused,
/// playing Drums' second pattern and Bass's first; a two-bar pattern focused.
fn jam_view() -> View {
    let mut v = view();
    let pattern = |color: usize, slot: usize| PatternView { name: "P".into(), color, bars: 1.0, slot, ..Default::default() };
    v.groups[0].color = 2;
    v.groups[1].color = 9;
    v.groups[0].patterns = vec![pattern(2, 0), pattern(3, 1), pattern(2, 2)];
    v.groups[0].scene_pattern = Some(1);
    v.groups[1].patterns = vec![pattern(9, 0)];
    v.groups[1].scene_pattern = Some(0);
    v.scenes = vec![
        SceneView { name: "1".into(), color: 5, patterns: vec![Some(1), Some(0)], ..Default::default() },
        SceneView { name: "2".into(), color: 6, patterns: vec![Some(0), None], ..Default::default() },
    ];
    v.velocity = 100;
    v.grid.step_ticks = 240;
    v.pattern_length = 2 * 3840;
    v.scale_root = 60;
    v.scale_bank = 0;
    v.scale_kind = 0;
    v
}

fn apps(out: &[Out]) -> Vec<Action> {
    out.iter().filter_map(|o| if let Out::App(a) = o { Some(*a) } else { None }).collect()
}

fn messages(out: &[Out]) -> Vec<String> {
    out.iter().filter_map(|o| if let Out::Message(m) = o { Some(m.clone()) } else { None }).collect()
}

fn press(d: &mut Device, v: &View, name: &str) -> Vec<Out> {
    let now = Instant::now();
    let mut out = d.button(name, true, v, now);
    out.extend(d.button(name, false, v, now));
    assert_eq!(d.error(), None, "{name}");
    out
}

fn hold(d: &mut Device, v: &View, name: &str) -> Vec<Out> {
    let out = d.button(name, true, v, Instant::now());
    assert_eq!(d.error(), None, "{name}");
    out
}

fn release(d: &mut Device, v: &View, name: &str) -> Vec<Out> {
    let out = d.button(name, false, v, Instant::now());
    assert_eq!(d.error(), None, "{name}");
    out
}

/// Press pad at `col`, `row`, from 1.
fn pad(d: &mut Device, v: &View, col: u8, row: u8) -> Vec<Out> {
    let now = Instant::now();
    let mut out = d.pad(col - 1, row - 1, true, v, now);
    out.extend(d.pad(col - 1, row - 1, false, v, now));
    assert_eq!(d.error(), None, "pad {col}, {row}");
    out
}

fn pad_light(d: &mut Device, v: &View, col: usize, row: usize) -> u8 {
    let l = d.leds(v, Instant::now());
    assert_eq!(d.error(), None);
    l.pads[1 + 8 + (row - 1) * 8 + (col - 1)]
}

fn button_light(d: &mut Device, v: &View, name: &str) -> u8 {
    let l = d.leds(v, Instant::now());
    assert_eq!(d.error(), None);
    let i = deckard_jam::map::BUTTON_LEDS.iter().position(|b| *b == name).unwrap();
    l.buttons[1 + i]
}

fn strip_touch(d: &mut Device, v: &View, strip: u8, kind: &str, pos: f64, now: Instant) -> Vec<Out> {
    let out = d.strip(strip - 1, 0, kind, pos, v, now);
    assert_eq!(d.error(), None, "strip {strip}");
    out
}

fn strip_lit(d: &mut Device, v: &View, strip: usize) -> Vec<u8> {
    let l = d.leds(v, Instant::now());
    l.strips[1 + (strip - 1) * 11..1 + strip * 11].to_vec()
}

#[test]
fn the_jam_is_a_shipped_hid_device_and_midi_devices_stay_midi() {
    let found = devices(None);
    let j = found.iter().find(|d| d.id == "maschine_jam").expect("shipped");
    assert_eq!(j.hid, Some((0x17cc, 0x1500)));
    assert!(j.ports.is_empty(), "no MIDI ports: the MIDI device host leaves it alone");
    let lp = found.iter().find(|d| d.id == "launchpad_pro_mk3").unwrap();
    assert_eq!(lp.hid, None);
}

/// Each column from the top down: its patterns, bright where the focused scene plays them,
/// half-lit where it doesn't, unlit where there are none (manual 7.2).
#[test]
fn project_view_shows_scenes_patterns_and_groups() {
    let mut d = jam();
    let v = jam_view();
    assert_eq!(pad_light(&mut d, &v, 1, 2), colour(3, SHADE_BRIGHT), "the scene plays it");
    assert_eq!(pad_light(&mut d, &v, 1, 1), colour(2, SHADE_FAINT), "it doesn't");
    assert_eq!(pad_light(&mut d, &v, 1, 4), 0, "no pattern there");
    assert_eq!(pad_light(&mut d, &v, 3, 1), 0, "no group");
    let l = d.leds(&v, Instant::now());
    assert_eq!(l.pads[1], colour(5, SHADE_BRIGHT), "the focused scene");
    assert_eq!(l.pads[2], colour(6, SHADE_DIM), "the other scene");
    assert_eq!(l.pads[3], colour(WHITE, SHADE_FAINT), "the place for a new one");
    assert_eq!(l.pads[4], 0);
    assert_eq!(l.pads[73], colour(2, SHADE_BRIGHT), "the focused group");
    assert_eq!(l.pads[74], colour(9, SHADE_DIM));
    assert_eq!(l.pads[75], colour(WHITE, SHADE_FAINT), "the place for a new group");
}

#[test]
fn a_pad_picks_a_pattern_deselects_it_or_makes_one() {
    let mut d = jam();
    let v = jam_view();
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::PickPattern(0)], "half-lit: the scene plays it");
    assert_eq!(apps(&pad(&mut d, &v, 1, 2)), [Action::ScenePattern { group: 0, pattern: None }], "bright: deselected");
    assert_eq!(apps(&pad(&mut d, &v, 1, 8)), [Action::NewPatternAt { group: 0, slot: 7 }], "unlit: a new one there");
    assert_eq!(
        apps(&pad(&mut d, &v, 2, 8)),
        [Action::FocusGroup(1), Action::NewPatternAt { group: 1, slot: 7 }],
        "another group's: focused too"
    );
    assert_eq!(apps(&pad(&mut d, &v, 3, 1)), [], "no group there");
}

/// Decided 2026-11-05: as the manual says. `focus_scene` is what plays next while playing, and
/// what's selected while stopped; PLAY starts it.
#[test]
fn scene_buttons_focus_scenes_and_the_next_place_makes_one() {
    let mut d = jam();
    let v = jam_view();
    assert_eq!(apps(&press(&mut d, &v, "SCENE_2")), [Action::FocusScene(1)]);
    assert_eq!(apps(&press(&mut d, &v, "SCENE_3")), [Action::SceneEdit("create")]);
    assert_eq!(apps(&press(&mut d, &v, "SCENE_4")), []);
    hold(&mut d, &v, "SHIFT");
    assert_eq!(apps(&press(&mut d, &v, "SCENE_2")), [], "bank 2 starts past the scenes there are");
    release(&mut d, &v, "SHIFT");
}

#[test]
fn clear_takes_what_is_pressed() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "CLEAR");
    assert_eq!(apps(&pad(&mut d, &v, 2, 1)), [Action::DeletePattern { group: 1, pattern: 0 }]);
    assert_eq!(apps(&pad(&mut d, &v, 2, 2)), [], "nothing there");
    assert_eq!(apps(&press(&mut d, &v, "SCENE_2")), [Action::DeleteScene(1)]);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_B")), [Action::DeleteGroup(1)]);
    release(&mut d, &v, "CLEAR");
    press(&mut d, &v, "PAD_MODE");
    hold(&mut d, &v, "CLEAR");
    assert_eq!(apps(&pad(&mut d, &v, 5, 8)), [Action::ResetSound(0)], "pad 1");
    assert_eq!(apps(&pad(&mut d, &v, 8, 5)), [Action::ResetSound(15)], "pad 16");
}

/// The first press takes the source and it blinks; each later press copies to there, and that
/// is the source (manual 7.5).
#[test]
fn duplicate_copies_from_the_source_on_to_each_target() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "DUPLICATE");
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [], "the source");
    assert_eq!(apps(&pad(&mut d, &v, 1, 2)), [Action::CopyPattern { group: 0, from: 0, to: 1 }]);
    assert_eq!(apps(&pad(&mut d, &v, 1, 3)), [Action::CopyPattern { group: 0, from: 1, to: 2 }], "again, from the copy");
    assert_eq!(apps(&pad(&mut d, &v, 1, 8)), [Action::CopyPattern { group: 0, from: 2, to: 3 }], "an empty place: after the last");
    release(&mut d, &v, "DUPLICATE");

    hold(&mut d, &v, "DUPLICATE");
    assert_eq!(apps(&press(&mut d, &v, "SCENE_1")), []);
    assert_eq!(apps(&press(&mut d, &v, "SCENE_2")), [Action::CopyScene { from: 0, to: 1 }]);
    assert_eq!(apps(&press(&mut d, &v, "SCENE_3")), [Action::CopyScene { from: 1, to: 2 }], "the next place: a new one");
    release(&mut d, &v, "DUPLICATE");

    hold(&mut d, &v, "DUPLICATE");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_A")), []);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_B")), [Action::CopyGroup { from: 0, to: 1 }]);
    release(&mut d, &v, "DUPLICATE");

    press(&mut d, &v, "PAD_MODE");
    hold(&mut d, &v, "DUPLICATE");
    assert_eq!(apps(&pad(&mut d, &v, 5, 8)), []);
    assert_eq!(apps(&pad(&mut d, &v, 6, 8)), [Action::CopySound { from: 0, to: 1 }]);
    release(&mut d, &v, "DUPLICATE");
    assert_eq!(apps(&pad(&mut d, &v, 6, 8)), [Action::FocusSound(1)], "let go: the source is forgotten");
}

#[test]
fn the_source_blinks() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "DUPLICATE");
    pad(&mut d, &v, 1, 1);
    let start = Instant::now();
    let mut seen = std::collections::HashSet::new();
    let mut t = start;
    for _ in 0..8 {
        t += Duration::from_millis(60);
        seen.insert(d.leds(&v, t).pads[1 + 8]);
    }
    assert_eq!(seen, [0, colour(2, SHADE_BRIGHT)].into_iter().collect());
}

/// SHIFT's printed labels: the panel's own names (see the manual's top view).
#[test]
fn shift_does_what_is_printed() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "SHIFT");
    let cases: [(&str, Action); 4] = [
        ("PLAY", Action::Restart),
        ("REC", Action::Record { on: true, count_in: true }),
        ("PAGE_LEFT", Action::Metronome(true)),
        ("PAGE_RIGHT", Action::Loop(true)),
    ];
    for (name, want) in cases {
        assert_eq!(apps(&press(&mut d, &v, name)), [want], "{name}");
    }
    assert_eq!(apps(&press(&mut d, &v, "MUTE")), [Action::ChokeAll]);
    assert_eq!(apps(&press(&mut d, &v, "DUPLICATE")), [Action::PatternEdit("double")]);
    assert_eq!(apps(&press(&mut d, &v, "CLEAR")), [Action::Edit("clear_modulation")]);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_G")), [Action::File("save", 0)]);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_A")), [Action::Follow(true)]);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_E")), [Action::GroovePage(1)], "VARIATION");
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::Edit("undo_step")], "UNDO");
    assert_eq!(apps(&pad(&mut d, &v, 2, 1)), [Action::Edit("redo_step")], "REDO");
    assert_eq!(apps(&pad(&mut d, &v, 3, 1)), [Action::Events("quantize", 0)], "QUANT");
    assert_eq!(apps(&pad(&mut d, &v, 4, 1)), [Action::Events("quantize_half", 0)], "QUANT 50%");
    assert_eq!(apps(&pad(&mut d, &v, 5, 1)), [Action::Events("transpose", -1)], "SEMI-");
    assert_eq!(apps(&pad(&mut d, &v, 8, 1)), [Action::Events("transpose", 12)], "OCT+");
    assert_eq!(apps(&pad(&mut d, &v, 1, 2)), [], "only the top row");
    assert!(d.pad_map(&v, Instant::now()).pads.iter().all(Vec::is_empty), "with SHIFT held the pads don't play");
    release(&mut d, &v, "SHIFT");
}

#[test]
fn the_transport_toggles() {
    let mut d = jam();
    let mut v = jam_view();
    assert_eq!(apps(&press(&mut d, &v, "PLAY")), [Action::Play]);
    assert_eq!(apps(&press(&mut d, &v, "REC")), [Action::Record { on: true, count_in: false }]);
    v.playing = true;
    v.recording = true;
    assert_eq!(apps(&press(&mut d, &v, "PLAY")), [Action::Stop], "no STOP button: PLAY toggles");
    assert_eq!(apps(&press(&mut d, &v, "REC")), [Action::Record { on: false, count_in: false }]);
    assert_eq!(button_light(&mut d, &v, "PLAY"), deckard_jam::leds::ON);
    assert_eq!(apps(&press(&mut d, &v, "FOOT_TIP")), [Action::Stop]);
    assert_eq!(apps(&press(&mut d, &v, "FOOT_RING")), [Action::Record { on: false, count_in: false }]);
}

/// Pads 1-16 are the block at the bottom right, 1 at its bottom left; they play from the
/// driver's thread, and the script only focuses.
#[test]
fn pad_mode_plays_the_printed_block() {
    let mut d = jam();
    let v = jam_view();
    assert!(d.pad_map(&v, Instant::now()).pads.iter().all(Vec::is_empty), "Project View plays nothing");
    press(&mut d, &v, "PAD_MODE");
    let m = d.pad_map(&v, Instant::now());
    assert_eq!(m.velocity, 100);
    let at = |col: usize, row: usize| &m.pads[(col - 1) + (row - 1) * 8];
    assert_eq!(*at(5, 8), [(0, 0, 60)], "pad 1");
    assert_eq!(*at(8, 8), [(0, 3, 60)], "pad 4");
    assert_eq!(*at(5, 5), [(0, 12, 60)], "pad 13");
    assert_eq!(*at(8, 5), [(0, 15, 60)], "pad 16");
    assert!(at(1, 1).is_empty() && at(4, 8).is_empty(), "the others don't play");
    assert_eq!(apps(&pad(&mut d, &v, 6, 8)), [Action::FocusSound(1)]);
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), []);
    assert_eq!(pad_light(&mut d, &v, 5, 8), colour(0, SHADE_BRIGHT), "the focused sound");
    assert_eq!(pad_light(&mut d, &v, 6, 8), colour(0, SHADE_DIM), "a used one");
    assert_eq!(pad_light(&mut d, &v, 5, 7), 0, "an empty one");
    assert_eq!(pad_light(&mut d, &v, 1, 1), 0, "outside the block");
}

/// Held, these make the matrix the sounds in any view, and the pads stop playing.
#[test]
fn select_mute_and_solo_act_on_sounds_and_groups() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "SELECT");
    assert!(d.pad_map(&v, Instant::now()).pads.iter().all(Vec::is_empty));
    assert_eq!(apps(&pad(&mut d, &v, 7, 8)), [Action::FocusSound(2)], "silently, from Project View");
    release(&mut d, &v, "SELECT");
    hold(&mut d, &v, "MUTE");
    assert_eq!(apps(&pad(&mut d, &v, 5, 8)), [Action::MuteSound(0)]);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_B")), [Action::MuteGroup(1)]);
    assert_eq!(pad_light(&mut d, &v, 1, 2), 0, "the sounds show, not the patterns");
    release(&mut d, &v, "MUTE");
    hold(&mut d, &v, "SOLO");
    assert_eq!(apps(&pad(&mut d, &v, 5, 8)), [Action::SoloSound(0)]);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_B")), [Action::SoloGroup(1)]);
    release(&mut d, &v, "SOLO");
}

/// Mode + SONG pins it: it stays until pressed again, and next time it's pressed it pins by
/// itself; pinned + SONG again takes that off (manual 4.3).
#[test]
fn song_pins_a_temporary_mode() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "MUTE");
    press(&mut d, &v, "SONG");
    release(&mut d, &v, "MUTE");
    assert_eq!(apps(&pad(&mut d, &v, 5, 8)), [Action::MuteSound(0)], "pinned on");
    press(&mut d, &v, "MUTE");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_A")), [Action::FocusGroup(0)], "pressed: off");
    press(&mut d, &v, "MUTE");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_B")), [Action::MuteGroup(1)], "pressed: on again");
    press(&mut d, &v, "MUTE");
    hold(&mut d, &v, "MUTE");
    press(&mut d, &v, "SONG");
    release(&mut d, &v, "MUTE");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_A")), [Action::FocusGroup(0)], "unpinned");
}

/// The 64 pads play the focused sound chromatically from C1, the root white.
#[test]
fn keyboard_mode_plays_all_64_pads() {
    let mut d = jam();
    let mut v = jam_view();
    v.sound = 2;
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "PAD_MODE");
    release(&mut d, &v, "SHIFT");
    let m = d.pad_map(&v, Instant::now());
    let at = |col: usize, row: usize| &m.pads[(col - 1) + (row - 1) * 8];
    assert_eq!(*at(1, 8), [(0, 2, 36)], "the bottom left: C1");
    assert_eq!(*at(2, 8), [(0, 2, 37)]);
    assert_eq!(*at(1, 7), [(0, 2, 44)], "the next row on");
    assert_eq!(*at(8, 1), [(0, 2, 99)], "the top right");
    assert_eq!(pad_light(&mut d, &v, 1, 8), colour(WHITE, SHADE_BRIGHT), "the root");
    assert_eq!(pad_light(&mut d, &v, 2, 8), colour(0, SHADE_DIM));
    press(&mut d, &v, "UP");
    let m = d.pad_map(&v, Instant::now());
    assert_eq!(m.pads[56], [(0, 2, 48)], "D-pad up: an octave on");
}

#[test]
fn keyboard_mode_follows_the_scale() {
    let mut d = jam();
    let mut v = jam_view();
    // (The first bank's second kind: whatever it is, it's not every semitone.)
    v.scale_kind = 1;
    let steps = deckard_model::scale_tables::SNAP[1].iter().enumerate().filter(|(_, s)| **s == 0).count();
    assert!(steps < 12);
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "PAD_MODE");
    release(&mut d, &v, "SHIFT");
    let m = d.pad_map(&v, Instant::now());
    let mut last = 0;
    for pad in [56usize, 57, 58, 59, 60, 61, 62, 63] {
        let p = m.pads[pad][0].2;
        assert!(p > last, "ascending: {p} after {last}");
        last = p;
    }
    assert_eq!(m.pads[56][0].2, 36, "from the root");
    assert_eq!(m.pads[56 + steps.min(7)][0].2 % 12 == 0, steps <= 7, "the octave comes round");
}

#[test]
fn step_mode_toggles_steps_with_a_chasing_light() {
    let mut d = jam();
    let mut v = jam_view();
    press(&mut d, &v, "STEP");
    assert_eq!(button_light(&mut d, &v, "STEP"), deckard_jam::leds::ON);
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::StepToggle { at: 0, length: 240, velocity: 100 }]);
    assert_eq!(apps(&pad(&mut d, &v, 3, 2)), [Action::StepToggle { at: 10 * 240, length: 240, velocity: 100 }]);
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "SELECT");
    release(&mut d, &v, "SHIFT");
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::StepToggle { at: 0, length: 240, velocity: 127 }], "accent");
    assert!(d.pad_map(&v, Instant::now()).pads.iter().all(Vec::is_empty), "steps don't play");

    v.notes = vec![
        crate::NoteView { sound: 0, position: 240, length: 240, pitch: 60, velocity: 100, ..Default::default() },
        crate::NoteView { sound: 0, position: 480, length: 240, pitch: 60, velocity: 127, ..Default::default() },
        crate::NoteView { sound: 1, position: 720, length: 240, pitch: 60, velocity: 100, ..Default::default() },
    ];
    v.pattern_position = Some(960);
    assert_eq!(pad_light(&mut d, &v, 2, 1), colour(0, SHADE_BRIGHT), "a note");
    assert_eq!(pad_light(&mut d, &v, 3, 1), colour(0, 3), "an accent is brighter");
    assert_eq!(pad_light(&mut d, &v, 4, 1), colour(0, SHADE_FAINT), "another sound's note isn't this one's");
    assert_eq!(pad_light(&mut d, &v, 5, 1), colour(WHITE, SHADE_BRIGHT), "the light chases");
    // (32 steps is a two-bar pattern: past it, nothing.)
    assert_eq!(pad_light(&mut d, &v, 1, 5), 0);
    assert_eq!(apps(&pad(&mut d, &v, 1, 5)), [], "past the pattern");
    press(&mut d, &v, "STEP");
    assert_eq!(button_light(&mut d, &v, "STEP"), deckard_jam::leds::BACKLIGHT, "STEP again leaves");
}

/// STEP + 1, 4 or 8 is how many sounds show: two lanes each with 4, one lane each with 8. A
/// lane's step is that sound's and the focus stays (manual 6.2.3).
#[test]
fn step_mode_shows_four_or_eight_sounds() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "STEP");
    press(&mut d, &v, "SCENE_4");
    release(&mut d, &v, "STEP");
    let step = |at, velocity| Action::StepToggle { at, length: 240, velocity };
    let lane = |s| [Action::FocusSound(s), step(240, 100), Action::FocusSound(0)];
    assert_eq!(apps(&pad(&mut d, &v, 2, 3)), lane(1), "the second sound's first lane");
    assert_eq!(apps(&pad(&mut d, &v, 2, 7)), lane(3));
    assert_eq!(apps(&pad(&mut d, &v, 1, 2)), [step(8 * 240, 100)], "the first sound's second lane");
    hold(&mut d, &v, "STEP");
    press(&mut d, &v, "SCENE_8");
    release(&mut d, &v, "STEP");
    assert_eq!(button_light(&mut d, &v, "STEP"), deckard_jam::leds::ON, "using it doesn't leave Step mode");
    assert_eq!(apps(&pad(&mut d, &v, 2, 8)), lane(7), "one lane each");
}

/// The numbered buttons pick the part shown; Follow moves it with the song.
#[test]
fn scene_buttons_pick_the_part_of_a_long_pattern() {
    let mut d = jam();
    let mut v = jam_view();
    v.pattern_length = 16 * 3840;
    press(&mut d, &v, "STEP");
    assert_eq!(apps(&press(&mut d, &v, "SCENE_2")), []);
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::StepToggle { at: 64 * 240, length: 240, velocity: 100 }]);
    let l = d.leds(&v, Instant::now());
    assert_eq!((l.pads[1], l.pads[2]), (colour(WHITE, SHADE_DIM), colour(WHITE, SHADE_BRIGHT)));
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "GROUP_A");
    release(&mut d, &v, "SHIFT");
    v.pattern_position = Some(200 * 240);
    d.tick(&v, Instant::now());
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::StepToggle { at: 192 * 240, length: 240, velocity: 100 }], "part 4, followed");
}

#[test]
fn piano_roll_is_time_across_and_pitch_up() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "STEP");
    release(&mut d, &v, "SHIFT");
    let got = apps(&pad(&mut d, &v, 3, 8));
    let Action::StepNotes { at, length, velocity, pitches } = got[0] else { panic!("{got:?}") };
    assert_eq!((at, length, velocity, pitches[0], pitches[1]), (2 * 240, 240, 100, 36, 128));
    let got = apps(&pad(&mut d, &v, 3, 1));
    let Action::StepNotes { pitches, .. } = got[0] else { panic!("{got:?}") };
    assert_eq!(pitches[0], 36 + 7, "the top row is higher");
    assert!(d.pad_map(&v, Instant::now()).pads.iter().all(Vec::is_empty));
    press(&mut d, &v, "UP");
    let got = apps(&pad(&mut d, &v, 3, 8));
    let Action::StepNotes { pitches, .. } = got[0] else { panic!("{got:?}") };
    assert_eq!(pitches[0], 37, "D-pad up: a row on");
}

/// SHIFT + SOLO: a pad is a bar (manual 6.1.3).
#[test]
fn pattern_length_mode_counts_bars() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "SOLO");
    release(&mut d, &v, "SHIFT");
    assert_eq!(pad_light(&mut d, &v, 1, 1), colour(2, SHADE_DIM), "bar 1");
    assert_eq!(pad_light(&mut d, &v, 2, 1), colour(2, SHADE_BRIGHT), "bar 2: its end");
    assert_eq!(pad_light(&mut d, &v, 3, 1), 0);
    assert_eq!(apps(&pad(&mut d, &v, 4, 1)), [Action::PatternLength(4 * 3840)]);
    assert_eq!(apps(&pad(&mut d, &v, 1, 2)), [Action::PatternLength(9 * 3840)]);
    press(&mut d, &v, "SONG");
    assert_eq!(pad_light(&mut d, &v, 1, 2), colour(3, SHADE_BRIGHT), "back in Project View");
}

#[test]
fn lock_view_has_all_64_snapshots() {
    let mut d = jam();
    let mut v = jam_view();
    assert_eq!(apps(&press(&mut d, &v, "LOCK")), [Action::Lock("button", 0)]);
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "LOCK");
    release(&mut d, &v, "SHIFT");
    v.lock.slots = vec![0, 63];
    v.lock.focus = Some(63);
    assert_eq!(pad_light(&mut d, &v, 1, 1), colour(WHITE, SHADE_DIM));
    assert_eq!(pad_light(&mut d, &v, 8, 8), colour(WHITE, SHADE_BRIGHT), "in focus");
    assert_eq!(pad_light(&mut d, &v, 2, 1), 0);
    assert_eq!(apps(&pad(&mut d, &v, 2, 2)), [Action::Lock("slot", 9)]);
    assert_eq!(apps(&press(&mut d, &v, "LOCK")), [Action::Lock("update", 0)], "LOCK updates the one in focus");
    hold(&mut d, &v, "CLEAR");
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::Lock("delete_slot", 0)]);
    assert_eq!(apps(&pad(&mut d, &v, 2, 1)), [], "empty");
    release(&mut d, &v, "CLEAR");
}

/// Strips are absolute: LEVEL is groups by default, each with its travel's level.
#[test]
fn level_strips_set_group_levels_and_show_them() {
    let mut d = jam();
    let mut v = jam_view();
    v.groups[0].mix.level = 0.0;
    v.groups[1].mix.level = f64::NEG_INFINITY;
    let lit = strip_lit(&mut d, &v, 1);
    assert!(lit[7] != 0 && lit[9] == 0, "0 dB is three-quarters of the way up: {lit:?}");
    assert!(strip_lit(&mut d, &v, 2).iter().all(|b| *b == 0), "silent");
    assert!(strip_lit(&mut d, &v, 3).iter().all(|b| *b == 0), "no group");
    let out = strip_touch(&mut d, &v, 2, "down", 0.75, Instant::now());
    let [Action::ChannelLevel { group: 1, sound: None, db }] = apps(&out)[..] else { panic!("{out:?}") };
    assert!(db.abs() < 0.1, "{db}");
    assert_eq!(apps(&strip_touch(&mut d, &v, 3, "down", 0.5, Instant::now())), [], "no group");
}

/// LEVEL + a pad: the strips are its eight sounds; the page buttons, the other eight.
#[test]
fn a_mode_button_and_a_pad_pick_sounds() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "LEVEL");
    assert_eq!(pad_light(&mut d, &v, 5, 8), colour(0, SHADE_BRIGHT), "the sounds show");
    assert_eq!(apps(&pad(&mut d, &v, 5, 6)), [Action::FocusSound(8)], "pad 9");
    release(&mut d, &v, "LEVEL");
    let out = strip_touch(&mut d, &v, 1, "down", 0.75, Instant::now());
    assert!(matches!(apps(&out)[..], [Action::ChannelLevel { group: 0, sound: Some(8), .. }]), "{out:?}");
    press(&mut d, &v, "PAGE_LEFT");
    let out = strip_touch(&mut d, &v, 2, "down", 0.75, Instant::now());
    assert!(matches!(apps(&out)[..], [Action::ChannelLevel { group: 0, sound: Some(1), .. }]), "{out:?}");
    hold(&mut d, &v, "LEVEL");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_B")), [Action::FocusGroup(1)]);
    release(&mut d, &v, "LEVEL");
    let out = strip_touch(&mut d, &v, 2, "down", 0.75, Instant::now() + Duration::from_secs(1));
    assert!(matches!(apps(&out)[..], [Action::ChannelLevel { group: 1, sound: None, .. }]), "groups again: {out:?}");
}

#[test]
fn a_double_tap_resets_a_strip() {
    let mut d = jam();
    let v = jam_view();
    let t = Instant::now();
    strip_touch(&mut d, &v, 1, "down", 0.2, t);
    strip_touch(&mut d, &v, 1, "up", 0.2, t + Duration::from_millis(50));
    let out = strip_touch(&mut d, &v, 1, "down", 0.2, t + Duration::from_millis(150));
    assert_eq!(apps(&out), [Action::ChannelLevel { group: 0, sound: None, db: 0.0 }], "unity");
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "move", 0.5, t + Duration::from_millis(170))), [], "the rest of the touch does nothing");
    strip_touch(&mut d, &v, 1, "up", 0.5, t + Duration::from_millis(200));
    let out = strip_touch(&mut d, &v, 1, "down", 0.5, t + Duration::from_secs(2));
    assert!(matches!(apps(&out)[..], [Action::ChannelLevel { db, .. }] if db < 0.0), "a slow second touch sets it: {out:?}");
}

#[test]
fn pan_aux_tune_and_swing_strips() {
    let mut d = jam();
    let mut v = jam_view();
    let t = Instant::now();
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "LEVEL");
    release(&mut d, &v, "SHIFT");
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 1.0, t)), [Action::ChannelPan { group: 0, sound: None, pan: 1.0 }]);
    assert_eq!(strip_lit(&mut d, &v, 1)[5], colour(2, SHADE_BRIGHT), "from the middle");
    assert_eq!(strip_lit(&mut d, &v, 1)[0], 0);
    press(&mut d, &v, "AUX");
    assert_eq!(apps(&strip_touch(&mut d, &v, 2, "down", 0.5, t)), [Action::SetSend { group: 1, sound: None, aux: 0, level: 0.5 }]);
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "AUX");
    release(&mut d, &v, "SHIFT");
    assert_eq!(apps(&strip_touch(&mut d, &v, 3, "down", 0.5, t)), [], "no group");
    assert_eq!(apps(&strip_touch(&mut d, &v, 2, "move", 0.5, t)), [Action::SetSend { group: 1, sound: None, aux: 1, level: 0.5 }], "AUX 2");
    press(&mut d, &v, "TUNE");
    assert!(strip_lit(&mut d, &v, 1).iter().all(|b| *b == 0), "no Sampler: nothing to tune");
    v.groups[0].mix.tune = Some(0.0);
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "move", 1.0, t)), [Action::SetTune { group: 0, sound: None, semitones: 36.0 }]);
    press(&mut d, &v, "SWING");
    assert_eq!(apps(&strip_touch(&mut d, &v, 2, "move", 0.25, t)), [Action::SetChannelSwing { group: Some(1), sound: None, amount: 25.0 }]);
}

/// Control is relative underneath: the strip turns by the difference from the knob's value.
#[test]
fn control_strips_turn_the_knobs_and_turn_pages() {
    let mut d = jam();
    let v = jam_view();
    let was = v.control.knobs[0].as_ref().map(|k| k.value).unwrap_or(0.0);
    let got = apps(&press(&mut d, &v, "CONTROL"));
    assert_eq!(got, [Action::ModulesVisible(true)]);
    let out = strip_touch(&mut d, &v, 1, "down", 0.75, Instant::now());
    if v.control.knobs[0].is_some() {
        assert_eq!(apps(&out), [Action::Turn { knob: 0, inc: 0.75 - was, fine: false }]);
    }
    assert_eq!(apps(&press(&mut d, &v, "PAGE_RIGHT")), [Action::Page(true)]);
    assert_eq!(apps(&press(&mut d, &v, "PAGE_LEFT")), [Action::Page(false)]);
    hold(&mut d, &v, "CONTROL");
    assert_eq!(apps(&press(&mut d, &v, "DOWN")), [Action::Slot(1)], "the next plug-in");
    assert_eq!(apps(&press(&mut d, &v, "UP")), [Action::Slot(-1)]);
    assert_eq!(apps(&press(&mut d, &v, "GROUP_A")), [Action::FocusGroup(0), Action::Level(1)]);
    assert_eq!(apps(&press(&mut d, &v, "MST")), [Action::Level(0)]);
    assert_eq!(apps(&pad(&mut d, &v, 5, 8)), [Action::FocusSound(0), Action::Level(2)]);
    release(&mut d, &v, "CONTROL");
    assert_eq!(apps(&press(&mut d, &v, "MACRO")), [Action::ModulesVisible(false), Action::PageGroup(3)]);
}

/// The same movement on any strip touches, moves and lets go of the focused group's FX.
#[test]
fn perform_strips_drive_the_focused_groups_effect() {
    let mut d = jam();
    let mut v = jam_view();
    let t = Instant::now();
    press(&mut d, &v, "PERFORM");
    assert_eq!(apps(&strip_touch(&mut d, &v, 4, "down", 0.5, t)), [], "no effect: nothing to do");
    v.groups[0].perform = Some((false, 0.5));
    strip_touch(&mut d, &v, 4, "up", 0.5, t);
    assert_eq!(apps(&strip_touch(&mut d, &v, 4, "down", 0.3, t)), [Action::Strip("perform_touch", 0.3)]);
    assert_eq!(apps(&strip_touch(&mut d, &v, 4, "move", 0.6, t)), [Action::Strip("perform", 0.6)]);
    assert_eq!(apps(&strip_touch(&mut d, &v, 4, "up", 0.6, t)), [Action::Strip("perform_release", 0.6)]);
    // SHIFT + PERFORM: a list on the encoder, the effect going in if there's none.
    hold(&mut d, &v, "SHIFT");
    let out = press(&mut d, &v, "PERFORM");
    release(&mut d, &v, "SHIFT");
    assert_eq!(apps(&out), [Action::PerformFx]);
    assert_eq!(messages(&out), ["PERFORM FX ▸ Effect: [Filter]"]);
    let out = d.encoder(1, &v, Instant::now());
    assert_eq!(apps(&out), [Action::PerformMode { group: 0, mode: 1 }]);
}

/// AUTO is held to write (manual 8.5).
#[test]
fn auto_writes_while_held() {
    let mut d = jam();
    let v = jam_view();
    assert_eq!(apps(&hold(&mut d, &v, "AUTO")), [Action::AutoWrite(true)]);
    assert_eq!(apps(&release(&mut d, &v, "AUTO")), [Action::AutoWrite(false)]);
}

/// NOTES + 1, 2 or 3 picks a set; a strip strums its column's notes, each finger its own.
#[test]
fn notes_mode_strums_each_columns_chord() {
    let mut d = jam();
    let v = jam_view();
    let t = Instant::now();
    hold(&mut d, &v, "NOTES");
    let out = press(&mut d, &v, "SCENE_2");
    assert_eq!(messages(&out), ["Notes: Chords"]);
    release(&mut d, &v, "NOTES");
    let note = |pitch, on| Action::StripNote { sound: 0, pitch, velocity: 100, on };
    // The chromatic scale from C2: column 1's degrees are 0, 2, 4 and 7.
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.1, t)), [note(48, true)]);
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "move", 0.15, t)), [], "the same note");
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "move", 0.3, t)), [note(48, false), note(50, true)]);
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "up", 0.3, t)), [note(50, false)]);
    assert!(d.leds(&v, t).pads[1 + 8 + 7 * 8] != 0, "the column's notes show on the matrix");
    // A pad adds to a column, or takes away.
    assert_eq!(apps(&pad(&mut d, &v, 1, 8)), []);
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.1, t)), [note(50, true)], "degree 0 gone");
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "up", 0.1, t)), [note(50, false)]);
    hold(&mut d, &v, "NOTES");
    press(&mut d, &v, "SCENE_3");
    release(&mut d, &v, "NOTES");
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.1, t)), [], "User starts empty");
}

#[test]
fn meters_show_what_mst_grp_and_in_choose() {
    let mut d = jam();
    let mut v = jam_view();
    v.master_meter = (1.0, 0.0);
    v.groups[0].mix.meter = (0.0, 1.0);
    let column = |l: &deckard_jam::Leds, c: usize| {
        l.buttons[1 + METER_LEDS + c * 8..1 + METER_LEDS + (c + 1) * 8].iter().filter(|b| **b != 0).count()
    };
    let l = d.leds(&v, Instant::now());
    assert_eq!((column(&l, 0), column(&l, 1)), (8, 0), "the master by default");
    press(&mut d, &v, "GRP");
    let l = d.leds(&v, Instant::now());
    assert_eq!((column(&l, 0), column(&l, 1)), (0, 8));
    assert_eq!(button_light(&mut d, &v, "GRP"), deckard_jam::leds::ON);
    assert_eq!(button_light(&mut d, &v, "MST"), deckard_jam::leds::BACKLIGHT);
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "IN1");
    release(&mut d, &v, "SHIFT");
    assert_eq!(button_light(&mut d, &v, "IN2"), deckard_jam::leds::ON);
}

#[test]
fn in1_in2_and_cue_meter_their_own() {
    let mut d = jam();
    let mut v = jam_view();
    v.input_meters = [(1.0, 0.0), (0.0, 1.0)];
    v.cue_meter = (1.0, 1.0);
    let column = |l: &deckard_jam::Leds, c: usize| {
        l.buttons[1 + METER_LEDS + c * 8..1 + METER_LEDS + (c + 1) * 8].iter().filter(|b| **b != 0).count()
    };
    assert_eq!(apps(&press(&mut d, &v, "IN1")), [Action::WatchInput(true)], "held open for the meter");
    let l = d.leds(&v, Instant::now());
    assert_eq!((column(&l, 0), column(&l, 1)), (8, 0), "In 1");
    hold(&mut d, &v, "SHIFT");
    assert_eq!(apps(&press(&mut d, &v, "IN1")), [], "already open");
    release(&mut d, &v, "SHIFT");
    let l = d.leds(&v, Instant::now());
    assert_eq!((column(&l, 0), column(&l, 1)), (0, 8), "In 2");
    assert_eq!(apps(&press(&mut d, &v, "CUE")), [Action::WatchInput(false)], "let go");
    let l = d.leds(&v, Instant::now());
    assert_eq!((column(&l, 0), column(&l, 1)), (8, 8), "the cue");
}

#[test]
fn the_encoder_sets_an_inputs_level() {
    let mut d = jam();
    let mut v = jam_view();
    v.input_levels = [0.75, 0.75];
    press(&mut d, &v, "IN1");
    let out = d.encoder(2, &v, Instant::now());
    assert!(matches!(apps(&out)[..], [Action::InputDb(0, db)] if (db - 1.0).abs() < 1e-3), "{:?}", apps(&out));
    assert_eq!(messages(&out), ["In 1 1.0 dB"]);
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "IN1");
    release(&mut d, &v, "SHIFT");
    let out = d.encoder(-1, &v, Instant::now());
    assert!(matches!(apps(&out)[..], [Action::InputDb(1, db)] if (db + 0.5).abs() < 1e-3), "{:?}", apps(&out));
    assert_eq!(messages(&out), ["In 2 -0.5 dB"]);
}

#[test]
fn the_encoder_sets_what_the_meter_shows_and_says_so() {
    let mut d = jam();
    let mut v = jam_view();
    v.master_db = -6.0;
    let out = d.encoder(2, &v, Instant::now());
    assert_eq!(apps(&out), [Action::VolumeDb(-5.0)]);
    assert_eq!(messages(&out), ["Master -5.0 dB"]);
    press(&mut d, &v, "GRP");
    v.groups[0].mix.level = -3.0;
    assert_eq!(apps(&d.encoder(1, &v, Instant::now())), [Action::ChannelLevel { group: 0, sound: None, db: -2.5 }]);
    press(&mut d, &v, "CUE");
    v.cue_level = 0.75;
    assert!(matches!(apps(&d.encoder(1, &v, Instant::now()))[..], [Action::CueDb(_)]));
    press(&mut d, &v, "TEMPO");
    v.tempo = 120.0;
    assert_eq!(apps(&d.encoder(3, &v, Instant::now())), [Action::Tempo(123.0)]);
}

#[test]
fn shift_tempo_taps() {
    let mut d = jam();
    let v = jam_view();
    let t = Instant::now();
    d.button("SHIFT", true, &v, t);
    d.button("TEMPO", true, &v, t);
    d.button("TEMPO", false, &v, t);
    d.button("TEMPO", true, &v, t + Duration::from_millis(500));
    let got = apps(&d.button("TEMPO", false, &v, t + Duration::from_millis(500)));
    assert_eq!(got, [], "acted on the press");
    let out = d.button("TEMPO", true, &v, t + Duration::from_millis(1000));
    assert_eq!(apps(&out), [Action::Tempo(120.0)]);
}

/// GRID + 1-4: the common step grids, as a press (by the numbered buttons or the pads).
#[test]
fn grid_held_picks_the_step_grid() {
    let mut d = jam();
    let v = jam_view();
    hold(&mut d, &v, "GRID");
    assert_eq!(apps(&press(&mut d, &v, "SCENE_3")), [Action::Grid("step", 4)], "1/16");
    assert_eq!(apps(&pad(&mut d, &v, 8, 8)), [Action::Grid("step", 5)], "pad 4: 1/32");
    release(&mut d, &v, "GRID");
    // Its menu stays, with the rest of the grids on the encoder.
    let out = press(&mut d, &v, "ENCODER");
    assert_eq!(messages(&out), ["GRID ▸ Step: [1 Bar]"]);
    let out = d.encoder(1, &v, Instant::now());
    assert_eq!(apps(&out), [Action::Grid("step", 1)]);
    press(&mut d, &v, "ENCODER");
    let out = d.encoder(1, &v, Instant::now());
    assert_eq!(messages(&out), ["GRID ▸ Nudge: Step"]);
    assert_eq!(button_light(&mut d, &v, "GRID"), deckard_jam::leds::ON);
    press(&mut d, &v, "GRID");
    assert_eq!(button_light(&mut d, &v, "GRID"), deckard_jam::leds::BACKLIGHT);
}

#[test]
fn rec_mode_wants_the_settings_and_lets_them_go() {
    let mut d = jam();
    let mut v = jam_view();
    v.settings.values.insert("metronome_volume".into(), "70 %".into());
    hold(&mut d, &v, "SHIFT");
    let out = press(&mut d, &v, "GRID");
    release(&mut d, &v, "SHIFT");
    assert_eq!(apps(&out), [Action::SettingsWanted(true)]);
    assert_eq!(messages(&out), ["REC MODE ▸ Metronome Level: 70 %"]);
    press(&mut d, &v, "ENCODER");
    assert_eq!(apps(&d.encoder(-1, &v, Instant::now())), [Action::Setting("metronome_volume", 0, -1)]);
    assert_eq!(apps(&press(&mut d, &v, "GRID")), [Action::SettingsWanted(false)]);
}

/// Touching the encoder opens what the view has to set; it goes when left alone.
#[test]
fn touching_the_encoder_opens_the_scale_and_it_closes_by_itself() {
    let mut d = jam();
    let v = jam_view();
    let t = Instant::now();
    assert!(messages(&d.button("ENCODER_TOUCH", true, &v, t)).is_empty(), "Project View has none");
    d.button("ENCODER_TOUCH", false, &v, t);
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "PAD_MODE");
    release(&mut d, &v, "SHIFT");
    let out = d.button("ENCODER_TOUCH", true, &v, t);
    assert_eq!(messages(&out), ["SCALE ▸ Root Note: C"]);
    d.button("ENCODER_TOUCH", false, &v, t);
    press(&mut d, &v, "ENCODER");
    assert_eq!(apps(&d.encoder(2, &v, t)), [Action::Scale { root: 62, bank: 0, kind: 0 }]);
    d.tick(&v, t + Duration::from_secs(10));
    assert_eq!(apps(&d.encoder(2, &v, t + Duration::from_secs(10))), [Action::VolumeDb(1.0)], "closed: the encoder's the master's again");
}

/// Chord modes: each pad plays its chord, on the same thread as a note.
#[test]
fn chords_come_through_the_pads_route() {
    let mut d = jam();
    let v = jam_view();
    let t = Instant::now();
    hold(&mut d, &v, "SHIFT");
    press(&mut d, &v, "PAD_MODE");
    release(&mut d, &v, "SHIFT");
    d.button("ENCODER_TOUCH", true, &v, t);
    d.encoder(3, &v, t); // To Chord Mode.
    press(&mut d, &v, "ENCODER");
    d.encoder(1, &v, t); // Harmonizer.
    assert_eq!(d.error(), None);
    let m = d.pad_map(&v, t);
    assert_eq!(d.error(), None);
    let bottom_left: Vec<i64> = m.pads[56].iter().map(|n| n.2).collect();
    assert!(bottom_left.len() >= 3 && bottom_left.contains(&36), "{bottom_left:?}");
}

/// NOTE REPEAT goes on and off and takes the pads to Pad mode; SHIFT is the Arp, in Keyboard.
#[test]
fn note_repeat_and_the_arp() {
    let mut d = jam();
    let v = jam_view();
    let out = press(&mut d, &v, "NOTE_REPEAT");
    assert_eq!(apps(&out), [Action::KeyboardMode(false), Action::Repeat("on", 1.0)]);
    assert!(!d.pad_map(&v, Instant::now()).pads[60].is_empty(), "Pad mode");
    assert_eq!(apps(&press(&mut d, &v, "NOTE_REPEAT")), [Action::Repeat("on", 0.0)]);
    hold(&mut d, &v, "SHIFT");
    let out = press(&mut d, &v, "NOTE_REPEAT");
    release(&mut d, &v, "SHIFT");
    assert_eq!(apps(&out), [Action::KeyboardMode(true), Action::Repeat("on", 1.0)]);
    assert_eq!(d.pad_map(&v, Instant::now()).pads[0].len(), 1, "Keyboard mode: every pad");
    assert_eq!(messages(&d.button("ENCODER_TOUCH", true, &v, Instant::now())), ["ARP ▸ Type: -"]);
}

/// The library opens as the main window's panel, and the encoder and D-pad walk it.
#[test]
fn browse_opens_the_panel_and_loads() {
    let mut d = jam();
    let mut v = jam_view();
    assert_eq!(apps(&press(&mut d, &v, "BROWSE")), [Action::Library(true)]);
    v.browser.kind = 2;
    v.browser.results = vec![("Kick".into(), false), ("Snare".into(), true)];
    v.browser.attributes[0] = (vec!["Drums".into(), "Synth".into()], None);
    assert_eq!(apps(&d.encoder(1, &v, Instant::now())), [Action::Browse(crate::Browse::Kind(3))], "the type of file");
    press(&mut d, &v, "DOWN");
    assert_eq!(apps(&d.encoder(1, &v, Instant::now())), [Action::Browse(crate::Browse::Attribute(0, Some(0)))], "TYPES");
    press(&mut d, &v, "RIGHT");
    assert_eq!(apps(&d.encoder(1, &v, Instant::now())), [Action::Browse(crate::Browse::Focus(1))], "the results");
    assert_eq!(apps(&press(&mut d, &v, "ENCODER")), [Action::Browse(crate::Browse::Load)]);
    hold(&mut d, &v, "SHIFT");
    assert_eq!(apps(&press(&mut d, &v, "ENCODER")), [Action::Browse(crate::Browse::SetFavorite)]);
    assert_eq!(apps(&d.encoder(1, &v, Instant::now())), [Action::Browse(crate::Browse::Focus(10))], "ten at a time");
    assert_eq!(
        apps(&press(&mut d, &v, "DOWN")),
        [Action::Browse(crate::Browse::Focus(1)), Action::Browse(crate::Browse::Load)],
        "the next one, loaded"
    );
    release(&mut d, &v, "SHIFT");
    press(&mut d, &v, "UP");
    assert_eq!(
        apps(&press(&mut d, &v, "ENCODER")),
        [Action::Browse(crate::Browse::Favorites)],
        "up from the results: the favourites switch"
    );
    assert_eq!(apps(&press(&mut d, &v, "BROWSE")), [Action::Library(false)]);
    assert_eq!(apps(&d.encoder(1, &v, Instant::now())).len(), 1, "the master's again");
}

#[test]
fn group_banks_follow_the_d_pad_and_the_focus() {
    let mut d = jam();
    let mut v = jam_view();
    let spare = GroupView { name: "x".into(), color: 4, sounds: v.groups[0].sounds.clone(), ..Default::default() };
    v.groups = (0..10).map(|_| spare.clone()).collect();
    v.groups[8].color = 7;
    d.leds(&v, Instant::now());
    press(&mut d, &v, "RIGHT");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_A")), [Action::FocusGroup(8)], "the second bank");
    assert_eq!(d.leds(&v, Instant::now()).pads[73], colour(7, SHADE_DIM));
    press(&mut d, &v, "LEFT");
    v.group = 9;
    d.tick(&v, Instant::now());
    assert_eq!(apps(&press(&mut d, &v, "GROUP_A")), [Action::FocusGroup(8)], "focus from the MK3 brings its bank");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_C")), [Action::AddGroup], "the place after the last");
    assert_eq!(apps(&press(&mut d, &v, "GROUP_D")), [], "past it");
}

/// By default another group's pattern takes the focus with it; with the setting off, the
/// pattern goes on and the focus stays.
#[test]
fn picking_another_groups_pattern_can_leave_the_focus() {
    let mut d = jam();
    let mut v = jam_view();
    let second = PatternView { slot: 1, ..v.groups[1].patterns[0].clone() };
    v.groups[1].patterns.push(second);
    assert_eq!(apps(&pad(&mut d, &v, 2, 2)), [Action::FocusGroup(1), Action::PickPattern(1)]);
    assert_eq!(apps(&pad(&mut d, &v, 2, 3)), [Action::FocusGroup(1), Action::NewPatternAt { group: 1, slot: 2 }], "an empty place");
    v.keep_focus = true;
    assert_eq!(apps(&pad(&mut d, &v, 2, 2)), [Action::ScenePattern { group: 1, pattern: Some(1) }]);
    assert_eq!(apps(&pad(&mut d, &v, 2, 3)), [Action::NewPatternAt { group: 1, slot: 2 }]);
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::PickPattern(0)], "its own group's: as ever");
}

/// The pads are places: a pattern lights where it is, gaps and all, and an unlit pad makes
/// one there.
#[test]
fn project_view_pads_are_places() {
    let mut d = jam();
    let mut v = jam_view();
    let p = v.groups[0].patterns[0].clone();
    v.groups[0].patterns = vec![PatternView { slot: 0, ..p.clone() }, PatternView { slot: 7, ..p }];
    v.groups[0].scene_pattern = Some(0);
    assert_ne!(pad_light(&mut d, &v, 1, 8), 0, "the second of the list, on pad 8");
    assert_eq!(pad_light(&mut d, &v, 1, 2), 0, "nothing between");
    assert_eq!(apps(&pad(&mut d, &v, 1, 8)), [Action::PickPattern(1)]);
    assert_eq!(apps(&pad(&mut d, &v, 1, 4)), [Action::NewPatternAt { group: 0, slot: 3 }]);
}

#[test]
fn up_and_down_scroll_patterns_in_project_view() {
    let mut d = jam();
    let mut v = jam_view();
    let p = v.groups[0].patterns[0].clone();
    v.groups[0].patterns = (0..12).map(|slot| PatternView { slot, ..p.clone() }).collect();
    v.groups[0].scene_pattern = None;
    press(&mut d, &v, "DOWN");
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::PickPattern(1)], "scrolled a row");
    press(&mut d, &v, "UP");
    press(&mut d, &v, "UP");
    assert_eq!(apps(&pad(&mut d, &v, 1, 1)), [Action::PickPattern(0)], "stops at the top");
}

#[test]
fn closing_ends_what_the_script_started() {
    let mut d = jam();
    let v = jam_view();
    press(&mut d, &v, "NOTE_REPEAT");
    press(&mut d, &v, "BROWSE");
    let out = d.close();
    assert_eq!(apps(&out), [Action::Repeat("on", 0.0), Action::Library(false)]);
}

/// Every view and every strip mode draws without a script error, whatever the project has.
#[test]
fn every_mode_draws_on_an_empty_project_too() {
    let mut d = jam();
    let empty = View { groups: Vec::new(), scenes: Vec::new(), ..jam_view() };
    let full = jam_view();
    for v in [&empty, &full] {
        for name in [
            "SONG", "PAD_MODE", "STEP", "STEP", "LOCK", "BROWSE", "BROWSE", "LEVEL", "AUX", "TUNE", "SWING", "CONTROL", "MACRO", "PERFORM",
            "NOTES", "NOTES", "GRID", "GRID", "SONG",
        ] {
            press(&mut d, v, name);
            let _ = d.leds(v, Instant::now());
            let _ = d.pad_map(v, Instant::now());
            let _ = d.encoder(1, v, Instant::now());
            for kind in ["down", "move", "up"] {
                d.strip(0, 0, kind, 0.5, v, Instant::now());
            }
            d.tick(v, Instant::now());
            assert_eq!(d.error(), None, "after {name}");
        }
    }
}

#[test]
fn a_broken_lights_call_reports_rather_than_panics() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::create_dir_all(dir.join("devices/maschine_jam")).unwrap();
    std::fs::write(dir.join("devices/maschine_jam/main.lua"), "return { leds = function() deckard.led('NOPE', 'on') end }").unwrap();
    let mut d = Device::new("maschine_jam", Some(dir.to_path_buf()));
    let _ = d.leds(&view(), Instant::now());
    assert!(d.error().is_some_and(|e| e.contains("no light NOPE")), "{:?}", d.error());
}

/// Group 1's Swing strip (0..100 is the whole strip), with Relative touchstrips as asked.
fn swing_view(relative: bool, swing: f64) -> (Device, View) {
    let mut d = jam();
    let mut v = jam_view();
    v.relative_strips = relative;
    v.groups[0].mix.swing = swing;
    press(&mut d, &v, "SWING");
    (d, v)
}

fn swing_of(out: &[Out]) -> f64 {
    let [Action::SetChannelSwing { group: Some(0), sound: None, amount }] = apps(out)[..] else { panic!("{out:?}") };
    amount
}

/// Relative touchstrips: a touch picks the value up, the travel moves it by 0.35 of itself.
#[test]
fn relative_strips_move_by_the_travel_and_do_not_jump() {
    let (mut d, v) = swing_view(true, 50.0);
    let t = Instant::now();
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.9, t)), [], "a touch sets nothing");
    let amount = swing_of(&strip_touch(&mut d, &v, 1, "move", 0.7, t));
    assert!((amount - (50.0 - 0.2 * 0.35 * 100.0)).abs() < 1e-6, "{amount}");
    // The next move goes on from there, not from where the touch began.
    let amount = swing_of(&strip_touch(&mut d, &v, 1, "move", 0.9, t));
    assert!((amount - 50.0).abs() < 1e-6, "{amount}");
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "up", 0.9, t)), [], "lifting does nothing");
    // A new touch somewhere else picks up the value as it is then, not the finger's place.
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.1, t + Duration::from_secs(2))), []);
    let amount = swing_of(&strip_touch(&mut d, &v, 1, "move", 0.2, t + Duration::from_secs(2)));
    assert!((amount - (50.0 + 0.1 * 0.35 * 100.0)).abs() < 1e-6, "{amount}");
}

#[test]
fn relative_strips_stop_at_both_ends() {
    let (mut d, v) = swing_view(true, 95.0);
    let t = Instant::now();
    strip_touch(&mut d, &v, 1, "down", 0.0, t);
    assert_eq!(swing_of(&strip_touch(&mut d, &v, 1, "move", 1.0, t)), 100.0);
    // Travel past the end is lost: coming back moves it at once.
    let amount = swing_of(&strip_touch(&mut d, &v, 1, "move", 0.9, t));
    assert!((amount - 96.5).abs() < 1e-6, "{amount}");
    let (mut d, v) = swing_view(true, 5.0);
    strip_touch(&mut d, &v, 1, "down", 1.0, t);
    assert_eq!(swing_of(&strip_touch(&mut d, &v, 1, "move", 0.0, t)), 0.0);
}

#[test]
fn relative_strips_take_a_tenth_with_shift() {
    let (mut d, v) = swing_view(true, 50.0);
    let t = Instant::now();
    strip_touch(&mut d, &v, 1, "down", 0.5, t);
    hold(&mut d, &v, "SHIFT");
    let amount = swing_of(&strip_touch(&mut d, &v, 1, "move", 0.7, t));
    release(&mut d, &v, "SHIFT");
    assert!((amount - (50.0 + 0.2 * 0.035 * 100.0)).abs() < 1e-6, "{amount}");
    // SHIFT down at the touch doesn't set it either.
    hold(&mut d, &v, "SHIFT");
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.9, t + Duration::from_secs(2))), []);
    release(&mut d, &v, "SHIFT");
}

#[test]
fn relative_strips_never_reset_on_a_double_tap_and_ignore_a_second_finger() {
    let (mut d, v) = swing_view(true, 50.0);
    let t = Instant::now();
    strip_touch(&mut d, &v, 1, "down", 0.2, t);
    strip_touch(&mut d, &v, 1, "up", 0.2, t + Duration::from_millis(50));
    // A second stroke straight after the first: it moves on, as any other touch.
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.2, t + Duration::from_millis(150))), []);
    let amount = swing_of(&strip_touch(&mut d, &v, 1, "move", 0.3, t + Duration::from_millis(170)));
    assert!((amount - (50.0 + 0.1 * 0.35 * 100.0)).abs() < 1e-6, "{amount}");
    strip_touch(&mut d, &v, 1, "up", 0.3, t + Duration::from_millis(200));
    // A second finger neither moves nor jumps the value.
    let t = t + Duration::from_secs(2);
    strip_touch(&mut d, &v, 1, "down", 0.2, t);
    for kind in ["down", "move", "up"] {
        assert_eq!(apps(&d.strip(0, 1, kind, 0.9, &v, t)), [], "finger 2 {kind}");
    }
    let amount = swing_of(&strip_touch(&mut d, &v, 1, "move", 0.3, t));
    assert!((amount - (50.0 + 0.1 * 0.35 * 100.0)).abs() < 1e-6, "{amount}");
}

/// Faders move in dB: a whole strip is 0.35 of a turn of the Mixer's fader knob.
#[test]
fn relative_strips_move_faders_in_db() {
    let (mut d, mut v) = (jam(), jam_view());
    let t = Instant::now();
    v.relative_strips = true;
    v.groups[0].mix.level = -10.0;
    v.groups[1].mix.level = f64::NEG_INFINITY;
    v.groups[0].mix.aux = [deckard_model::level_from_db(-10.0), deckard_model::level_from_db(-10.0)];
    assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.5, t)), []);
    let out = strip_touch(&mut d, &v, 1, "move", 0.7, t);
    let [Action::ChannelLevel { group: 0, sound: None, db }] = apps(&out)[..] else { panic!("{out:?}") };
    assert!((db - (-10.0 + 0.2 * 0.35 * 70.0)).abs() < 1e-6, "{db}");
    strip_touch(&mut d, &v, 1, "up", 0.7, t);
    // Silence comes back from the foot of the scale, and going under it is silence again.
    strip_touch(&mut d, &v, 2, "down", 0.1, t);
    let out = strip_touch(&mut d, &v, 2, "move", 0.2, t);
    let [Action::ChannelLevel { group: 1, sound: None, db }] = apps(&out)[..] else { panic!("{out:?}") };
    assert!((db - (-60.0 + 0.1 * 0.35 * 70.0)).abs() < 1e-6, "{db}");
    let out = strip_touch(&mut d, &v, 2, "move", 0.0, t);
    assert_eq!(apps(&out), [Action::ChannelLevel { group: 1, sound: None, db: f64::NEG_INFINITY }]);
    strip_touch(&mut d, &v, 2, "up", 0.0, t);
    press(&mut d, &v, "AUX");
    strip_touch(&mut d, &v, 1, "down", 0.5, t);
    let out = strip_touch(&mut d, &v, 1, "move", 0.6, t);
    let [Action::SetSend { group: 0, sound: None, aux: 0, level }] = apps(&out)[..] else { panic!("{out:?}") };
    let want = deckard_model::level_from_db(-10.0 + 0.1 * 0.35 * 70.0);
    assert!((level - want).abs() < 1e-6, "{level} {want}");
}

/// The other value strips: Tune takes 0.25 of the travel, Control and Macro are deltas on the knob.
#[test]
fn relative_strips_in_tune_and_control() {
    let (mut d, mut v) = swing_view(true, 0.0);
    let t = Instant::now();
    v.groups[0].mix.tune = Some(0.0);
    press(&mut d, &v, "TUNE");
    strip_touch(&mut d, &v, 1, "down", 0.5, t);
    let out = strip_touch(&mut d, &v, 1, "move", 0.7, t);
    let [Action::SetTune { group: 0, sound: None, semitones }] = apps(&out)[..] else { panic!("{out:?}") };
    assert_eq!(semitones, 4.0, "0.2 x 0.25 x 72 = 3.6, to a semitone");
    for (i, mode) in ["CONTROL", "MACRO"].into_iter().enumerate() {
        let t = t + Duration::from_secs(2 * (i as u64 + 1));
        press(&mut d, &v, mode);
        v.control.knobs[0] = Some(crate::KnobView { value: 0.5, ..Default::default() });
        assert_eq!(apps(&strip_touch(&mut d, &v, 1, "down", 0.9, t)), [], "{mode}");
        let out = strip_touch(&mut d, &v, 1, "move", 0.7, t);
        let [Action::Turn { knob: 0, inc, fine: false }] = apps(&out)[..] else { panic!("{out:?}") };
        assert!((inc - -0.2).abs() < 1e-9, "{mode}: {inc}");
        strip_touch(&mut d, &v, 1, "up", 0.7, t);
    }
}

/// AUTO held writes the move as ever: the same Turn, and the touch is written from the timer.
#[test]
fn relative_strips_still_write_modulation_with_auto() {
    let (mut d, mut v) = swing_view(true, 0.0);
    let t = Instant::now();
    v.playing = true;
    v.auto_write = true;
    v.control.knobs[0] = Some(crate::KnobView { value: 0.5, modulatable: true, ..Default::default() });
    hold(&mut d, &v, "AUTO");
    press(&mut d, &v, "CONTROL");
    strip_touch(&mut d, &v, 1, "down", 0.5, t);
    assert!(apps(&d.tick(&v, t)).contains(&Action::TouchWrite(0)), "written while touched");
    let out = strip_touch(&mut d, &v, 1, "move", 0.9, t);
    assert!(matches!(apps(&out)[..], [Action::Turn { knob: 0, .. }]), "{out:?}");
    strip_touch(&mut d, &v, 1, "up", 0.9, t);
    assert!(apps(&d.tick(&v, t)).is_empty(), "let go");
}

/// Off (the default), a touch still sets the value where the finger lands.
#[test]
fn strips_are_absolute_unless_relative_is_on() {
    let (mut d, v) = swing_view(false, 50.0);
    let t = Instant::now();
    assert!((swing_of(&strip_touch(&mut d, &v, 1, "down", 0.9, t)) - 90.0).abs() < 1e-6);
    assert!((swing_of(&strip_touch(&mut d, &v, 1, "move", 0.2, t)) - 20.0).abs() < 1e-6);
    assert!(!View::default().relative_strips);
}
