use std::collections::BTreeMap;
use std::time::Instant;

use crate::device::{Device, Out, Print, PrintedNote};
use crate::tests_support::view;
use crate::{Action, GroupView, PatternView, SceneView, View};

const HEADER: [u8; 6] = [0xF0, 0x00, 0x20, 0x29, 0x02, 0x0E];

fn sysex(body: &[u8]) -> Vec<u8> {
    [&HEADER[..], body, &[0xF7]].concat()
}

/// Two groups: Drums (red) with three patterns, the second green, and Bass (blue) with
/// two. Three scenes: 1 plays both groups' first patterns, 2 Drums' second, 3 Drums'
/// third and Bass's second; green, magenta and blue. Scene 1 is focused and plays.
fn lp_view() -> View {
    let mut v = view();
    let pattern = |rgb| PatternView { name: "P".into(), rgb, bars: 1.0, ..Default::default() };
    v.groups[0].patterns = vec![pattern([255, 66, 62]), pattern([0, 255, 0]), pattern([255, 66, 62])];
    v.groups[1].patterns = vec![pattern([77, 148, 255]), pattern([77, 148, 255])];
    v.scenes = vec![
        SceneView { name: "1".into(), rgb: [0, 255, 0], patterns: vec![Some(0), Some(0)], ..Default::default() },
        SceneView { name: "2".into(), rgb: [255, 0, 255], patterns: vec![Some(1), None], ..Default::default() },
        SceneView { name: "3".into(), rgb: [0, 0, 255], patterns: vec![Some(2), Some(1)], ..Default::default() },
    ];
    focus(&mut v, 0);
    v.playing_scene = Some(0);
    v
}

/// Scene `scene` (from 0) focused: each group's scene pattern is what it plays there.
fn focus(v: &mut View, scene: usize) {
    v.scene = scene;
    for (g, group) in v.groups.iter_mut().enumerate() {
        group.scene_pattern = v.scenes.get(scene).and_then(|s| s.patterns.get(g).copied().flatten());
    }
}

/// Group `n` (from 0) with `patterns` patterns, all red.
fn group(n: usize, patterns: usize) -> GroupView {
    GroupView {
        name: format!("G{n}"),
        rgb: [255, 66, 62],
        patterns: (0..patterns).map(|_| PatternView { rgb: [255, 66, 62], bars: 1.0, ..Default::default() }).collect(),
        ..Default::default()
    }
}

// The lights' levels for red 255 66 62 (0.15 and full), blue 77 148 255, and the
// faint white of a place for a new pattern or scene.
const RED_DIM: [u8; 3] = [19, 5, 5];
const RED: [u8; 3] = [127, 33, 31];
const BLUE: [u8; 3] = [39, 74, 127];
const NEW: [u8; 3] = [6, 6, 6];
const OFF: [u8; 3] = [0, 0, 0];

fn lp() -> Device {
    Device::new("launchpad_pro_mk3", None)
}

fn sent(out: &[Out]) -> Vec<Vec<u8>> {
    out.iter()
        .filter_map(|o| {
            if let Out::Midi { role, bytes } = o {
                assert_eq!(role, "daw");
                Some(bytes.clone())
            } else {
                None
            }
        })
        .collect()
}

fn apps(out: &[Out]) -> Vec<Action> {
    out.iter().filter_map(|o| if let Out::App(a) = o { Some(*a) } else { None }).collect()
}

/// The LEDs the RGB lighting messages in `out` set: LED → (r, g, b).
fn lights(out: &[Out]) -> BTreeMap<u8, [u8; 3]> {
    let mut m = BTreeMap::new();
    for b in sent(out) {
        if b.len() < 8 || b[..6] != HEADER || b[6] != 0x03 {
            continue;
        }
        let specs = &b[7..b.len() - 1];
        assert_eq!(specs.len() % 5, 0, "RGB specs only: {specs:?}");
        for s in specs.chunks(5) {
            assert_eq!(s[0], 3);
            m.insert(s[1], [s[2], s[3], s[4]]);
        }
    }
    m
}

#[test]
fn the_launchpad_is_a_shipped_device() {
    let d = crate::device::devices(None);
    let lp = d.iter().find(|d| d.id == "launchpad_pro_mk3").expect("shipped");
    assert_eq!(lp.exclusive, ["daw"]);
    assert_eq!(lp.drums, ["LPProMK3 MIDI"], "its notes port, for the drum channel");
    assert!(lp.ports.iter().any(|(role, names)| role == "daw" && names.iter().any(|n| n == "LPProMK3 DAW")));
}

#[test]
fn open_enters_daw_mode_and_session() {
    let mut d = lp();
    assert_eq!(d.error(), None);
    let out = d.open(&lp_view(), Instant::now());
    let s = sent(&out);
    assert_eq!(s[..3], [sysex(&[0x10, 1]), sysex(&[0x00, 0x00, 0x00, 0x00]), sysex(&[0x18, 1])]);
}

#[test]
fn the_grid_shows_the_ideas_view() {
    let mut d = lp();
    let v = lp_view();
    let l = lights(&d.open(&v, Instant::now()));
    let lit = |led: u8| l.get(&led).copied().unwrap_or_else(|| panic!("LED {led} not sent"));
    // Column 1 is Drums, its first pattern at the top: lit where scene 1 plays it,
    // dim in its own colour where not, then a place for a new one.
    assert_eq!(lit(81), RED);
    assert_eq!(lit(71), [0, 19, 0], "the green pattern, dim");
    assert_eq!(lit(61), RED_DIM);
    assert_eq!(lit(51), NEW);
    assert_eq!(lit(41), OFF);
    // Column 2 is Bass.
    assert_eq!(lit(82), BLUE);
    assert_ne!(lit(72), BLUE);
    assert_ne!(lit(72), OFF);
    assert_eq!(lit(62), NEW);
    assert_eq!(lit(52), OFF);
    // No more groups.
    for led in [83, 88, 63, 18] {
        assert_eq!(lit(led), OFF, "LED {led}");
    }
    // The scenes under the grid in their colours: scene 1 plays (pulsing, never off),
    // 2 and 3 dim, then a place for a new one.
    let playing = lit(101);
    assert!(playing[0] == 0 && playing[1] > 19 && playing[2] == 0, "{playing:?}");
    assert_eq!(lit(102), [19, 0, 19]);
    assert_eq!(lit(103), [0, 0, 19]);
    assert_eq!(lit(104), NEW);
    assert_eq!(lit(105), OFF);
    // Nowhere to scroll.
    for led in [80, 70, 91, 92] {
        assert_eq!(lit(led), OFF, "arrow {led}");
    }
}

#[test]
fn focusing_a_scene_lights_its_patterns() {
    let mut d = lp();
    let mut v = lp_view();
    v.playing_scene = None;
    focus(&mut v, 2);
    let l = lights(&d.open(&v, Instant::now()));
    assert_eq!((l[&81], l[&61]), (RED_DIM, RED));
    assert_eq!(l[&72], BLUE);
    assert_ne!(l[&82], BLUE);
    assert_eq!(l[&103], [0, 0, 127], "the focused scene, full in its colour");
}

#[test]
fn only_changes_are_sent() {
    let mut d = lp();
    let (v, now) = (lp_view(), Instant::now());
    d.open(&v, now);
    assert_eq!(sent(&d.tick(&v, now)), Vec::<Vec<u8>>::new());
    let mut v2 = v.clone();
    focus(&mut v2, 1);
    let l = lights(&d.tick(&v2, now));
    assert!(l.contains_key(&71) && l.contains_key(&81) && !l.contains_key(&61), "{l:?}");
}

#[test]
fn a_pad_turns_its_pattern_on_and_off_in_the_scene() {
    let mut d = lp();
    let v = lp_view();
    let now = Instant::now();
    d.open(&v, now);
    // Drums' second pattern, off in scene 1: on (and focused).
    assert_eq!(apps(&d.midi("daw", &[0x90, 71, 100], &v, now)), [Action::FocusGroup(0), Action::PickPattern(1)]);
    // Its first, on: off.
    assert_eq!(apps(&d.midi("daw", &[0x90, 81, 100], &v, now)), [Action::FocusGroup(0), Action::ScenePattern { group: 0, pattern: None }]);
    assert_eq!(apps(&d.midi("daw", &[0x90, 82, 100], &v, now)), [Action::FocusGroup(1), Action::ScenePattern { group: 1, pattern: None }]);
    // A release, a pad under the place for a new pattern and one with no group: nothing.
    for pad in [[0x90, 72, 0], [0x90, 41, 100], [0x90, 83, 100]] {
        assert_eq!(apps(&d.midi("daw", &pad, &v, now)), [], "{pad:?}");
    }
}

#[test]
fn the_place_after_a_groups_patterns_makes_one() {
    let mut d = lp();
    let v = lp_view();
    let now = Instant::now();
    d.open(&v, now);
    assert_eq!(apps(&d.midi("daw", &[0x90, 51, 100], &v, now)), [Action::FocusGroup(0), Action::PatternEdit("create")]);
}

#[test]
fn the_bottom_row_launches() {
    let mut d = lp();
    let v = lp_view();
    d.open(&v, Instant::now());
    assert_eq!(apps(&d.midi("daw", &[0xB0, 102, 127], &v, Instant::now())), [Action::LaunchScene(1)]);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 102, 0], &v, Instant::now())), []);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 105, 127], &v, Instant::now())), []);
}

#[test]
fn the_place_after_the_last_scene_makes_one() {
    let mut d = lp();
    let v = lp_view();
    d.open(&v, Instant::now());
    assert_eq!(apps(&d.midi("daw", &[0xB0, 104, 127], &v, Instant::now())), [Action::SceneEdit("create")]);
}

#[test]
fn left_and_right_scroll_the_scenes_to_a_place_for_a_new_one() {
    let mut d = lp();
    let mut v = lp_view();
    v.scenes = (0..10)
        .map(|i| SceneView { name: format!("{i}"), rgb: [255, 255, 255], patterns: vec![Some(0), Some(0)], ..Default::default() })
        .collect();
    let now = Instant::now();
    let l = lights(&d.open(&v, now));
    assert_ne!(l[&92], OFF);
    let l = lights(&d.midi("daw", &[0xB0, 92, 127], &v, now));
    // Scenes 9 and 10, then the new one's place: right off, left on.
    assert_eq!((l[&103], l[&104]), (NEW, OFF));
    assert_eq!(l[&92], OFF);
    assert_ne!(l[&91], OFF);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 101, 127], &v, now)), [Action::LaunchScene(8)]);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 103, 127], &v, now)), [Action::SceneEdit("create")]);

    // Eight scenes: the ninth place is on the next eight.
    let mut d = lp();
    v.scenes.truncate(8);
    let l = lights(&d.open(&v, now));
    assert_ne!(l[&92], OFF);
    let l = lights(&d.midi("daw", &[0xB0, 92, 127], &v, now));
    assert_eq!((l[&101], l[&102]), (NEW, OFF));
}

#[test]
fn up_and_down_scroll_the_patterns_seven_at_a_time() {
    let mut d = lp();
    let mut v = lp_view();
    v.groups[0] = group(0, 10);
    focus(&mut v, 0);
    let now = Instant::now();
    let mut l = lights(&d.open(&v, now));
    assert_eq!((l[&80], l[&70] != OFF), (OFF, true));
    // (Only the LEDs that change are sent.)
    l.extend(lights(&d.midi("daw", &[0xB0, 70, 127], &v, now)));
    // (Seven rows: the bottom one is the playhead's.) Patterns 8 to 10, then the new
    // one's place.
    assert_eq!((l[&81], l[&71], l[&61], l[&51], l[&41]), (RED_DIM, RED_DIM, RED_DIM, NEW, OFF));
    assert_eq!((l[&70], l[&80] != OFF), (OFF, true));
    assert_eq!(apps(&d.midi("daw", &[0x90, 81, 100], &v, now)), [Action::FocusGroup(0), Action::PickPattern(7)]);
}

#[test]
fn the_groups_shown_are_the_focused_groups_eight() {
    let mut d = lp();
    let mut v = lp_view();
    v.groups = (0..10).map(|g| group(g, 1)).collect();
    v.scenes = vec![SceneView { name: "1".into(), patterns: vec![None; 10], ..Default::default() }];
    v.group = 8;
    focus(&mut v, 0);
    let now = Instant::now();
    let l = lights(&d.open(&v, now));
    assert_eq!((l[&81], l[&82], l[&83]), (RED_DIM, RED_DIM, OFF));
    assert_eq!(apps(&d.midi("daw", &[0x90, 82, 100], &v, now)), [Action::FocusGroup(9), Action::PickPattern(0)]);
}

#[test]
fn shift_and_left_or_right_focus_the_next_eight_groups() {
    let mut d = lp();
    let mut v = lp_view();
    v.groups = (0..10).map(|g| group(g, 1)).collect();
    let now = Instant::now();
    d.open(&v, now);
    d.midi("daw", &[0xB0, 90, 127], &v, now);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 92, 127], &v, now)), [Action::FocusGroup(8)]);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 91, 127], &v, now)), [], "no eight before");
    v.group = 8;
    assert_eq!(apps(&d.midi("daw", &[0xB0, 91, 127], &v, now)), [Action::FocusGroup(0)]);
    // SHIFT let go: the scenes again.
    d.midi("daw", &[0xB0, 90, 0], &v, now);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 92, 127], &v, now)), []);
}

#[test]
fn not_in_session_draws_nothing() {
    let mut d = lp();
    let mut v = lp_view();
    let now = Instant::now();
    d.open(&v, now);
    assert_eq!(sent(&d.midi("daw", &sysex(&[0x00, 0x04, 0x00, 0x00]), &v, now)), Vec::<Vec<u8>>::new());
    v.scene = 2;
    assert_eq!(sent(&d.tick(&v, now)), Vec::<Vec<u8>>::new());
    // Back in Session: everything again.
    let l = lights(&d.midi("daw", &sysex(&[0x00, 0x00, 0x00, 0x00]), &v, now));
    assert!(l.len() >= 64 + 8, "{}", l.len());
}

#[test]
fn the_layouts_echo_redraws_nothing() {
    // (The Launchpad answers a layout select with a report of it.)
    let mut d = lp();
    let (v, now) = (lp_view(), Instant::now());
    d.open(&v, now);
    assert_eq!(sent(&d.midi("daw", &sysex(&[0x00, 0x00, 0x00, 0x00]), &v, now)), Vec::<Vec<u8>>::new());
}

/// A Print to Clip packet: kind, id (8 nibbles), two unused bytes, the payload.
fn packet(kind: u8, id: u32, payload: &[u8]) -> Vec<u8> {
    packet_of(0, kind, id, payload)
}

/// A packet of sequencer track `track` (from 0): the byte after the id's `04` (as the
/// hardware sends it, 2026-10-28).
fn packet_of(track: u8, kind: u8, id: u32, payload: &[u8]) -> Vec<u8> {
    let mut d = vec![0x03, kind];
    d.extend((0..8).rev().map(|k| ((id >> (4 * k)) & 15) as u8));
    d.extend([4, track]);
    d.extend(payload);
    sysex(&d)
}

fn print(d: &mut Device, packets: &[Vec<u8>]) -> Vec<Out> {
    let v = lp_view();
    packets.iter().flat_map(|p| d.midi("daw", p, &v, Instant::now())).collect()
}

#[test]
fn print_to_clip_decodes() {
    let mut d = lp();
    let out = print(
        &mut d,
        &[
            packet(1, 0, &[]),
            packet(2, 1, &[0x00, 0x0F, 0x50]),
            packet(2, 2, &[0, 0, 0, 0x00, 0x00, 0x00, 0x7D, 0x3C, 0x64, 0x03, 0x74, 0x00, 0x7D, 0x3E, 0x50]),
            packet(3, 3, &[]),
        ],
    );
    let note = |pitch, start, length, velocity| PrintedNote { pitch, start, length, velocity };
    let want = Print {
        length: 4.0,
        notes: vec![note(60, 0.0, 0.25, 100), note(62, 1.0, 0.25, 80)],
        track: Some(1),
        device: "launchpad_pro_mk3".into(),
    };
    assert!(out.contains(&Out::Print(want)), "{out:?}");
}

#[test]
fn a_print_says_its_track() {
    let mut d = lp();
    let out = print(&mut d, &[packet_of(1, 1, 0, &[1]), packet_of(1, 2, 1, &[0x00, 0x0F, 0x50]), packet_of(1, 3, 2, &[])]);
    assert!(out.iter().any(|o| matches!(o, Out::Print(p) if p.track == Some(2))), "{out:?}");
}

#[test]
fn a_group_offset_moves_its_notes() {
    let mut d = lp();
    // Offset 2000 (4 beats): a note at 0 in it starts at beat 4.
    let out = print(&mut d, &[packet(1, 0, &[]), packet(2, 1, &[0x00, 0x0F, 0x50, 0, 0, 0, 0x7D, 0x3C, 0x64]), packet(3, 2, &[])]);
    assert!(
        out.iter().any(|o| matches!(o, Out::Print(p) if p.notes == [PrintedNote { pitch: 60, start: 4.0, length: 0.25, velocity: 100 }])),
        "{out:?}"
    );
}

#[test]
fn a_skipped_packet_drops_the_print() {
    let mut d = lp();
    let out = print(&mut d, &[packet(1, 0, &[]), packet(2, 2, &[0x00, 0x0F, 0x50]), packet(3, 3, &[])]);
    assert!(!out.iter().any(|o| matches!(o, Out::Print(_))), "{out:?}");
    assert!(out.iter().any(|o| matches!(o, Out::Message(m) if m.contains("missing"))), "{out:?}");
    // The next print works.
    let out = print(&mut d, &[packet(1, 0, &[]), packet(2, 1, &[0x00, 0x0F, 0x50]), packet(3, 2, &[])]);
    assert!(out.iter().any(|o| matches!(o, Out::Print(p) if p.length == 4.0)), "{out:?}");
}

#[test]
fn a_print_missing_its_start_or_end_says_so() {
    let mut d = lp();
    let said = |out: &[Out]| out.iter().any(|o| matches!(o, Out::Message(m) if m.contains("missing")));
    // No begin: data then an end.
    let out = print(&mut d, &[packet(2, 1, &[0x00, 0x0F, 0x50]), packet(3, 2, &[])]);
    assert!(said(&out) && !out.iter().any(|o| matches!(o, Out::Print(_))), "{out:?}");
    // No end: a begin, data, then the next print's begin; that one prints.
    let out = print(&mut d, &[packet(1, 0, &[]), packet(2, 1, &[0x00, 0x0F, 0x50]), packet(1, 0, &[])]);
    assert!(said(&out), "{out:?}");
    let out = print(&mut d, &[packet(2, 1, &[0x00, 0x0F, 0x50]), packet(3, 2, &[])]);
    assert!(out.iter().any(|o| matches!(o, Out::Print(p) if p.length == 4.0)), "{out:?}");
}

#[test]
fn close_leaves_daw_mode() {
    let mut d = lp();
    d.open(&lp_view(), Instant::now());
    assert_eq!(sent(&d.close()), [sysex(&[0x10, 0])]);
}

#[test]
fn play_starts_and_stops() {
    let mut d = lp();
    let mut v = lp_view();
    d.open(&v, Instant::now());
    assert_eq!(apps(&d.midi("daw", &[0xB0, 20, 127], &v, Instant::now())), [Action::Play]);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 20, 0], &v, Instant::now())), []);
    v.playing = true;
    assert_eq!(apps(&d.midi("daw", &[0xB0, 20, 127], &v, Instant::now())), [Action::Stop]);
}

#[test]
fn record_starts_and_stops() {
    let mut d = lp();
    let mut v = lp_view();
    d.open(&v, Instant::now());
    assert_eq!(apps(&d.midi("daw", &[0xB0, 10, 127], &v, Instant::now())), [Action::Record { on: true, count_in: false }]);
    v.recording = true;
    assert_eq!(apps(&d.midi("daw", &[0xB0, 10, 127], &v, Instant::now())), [Action::Record { on: false, count_in: false }]);
}

#[test]
fn play_and_record_light_up() {
    let mut d = lp();
    let mut v = lp_view();
    let off = lights(&d.open(&v, Instant::now()));
    assert_eq!(off[&20], [19, 19, 19], "dim while stopped");
    assert_eq!(off[&10], [19, 0, 0], "dim while not recording");
    v.playing = true;
    v.recording = true;
    let on = lights(&d.tick(&v, Instant::now()));
    assert_eq!(on[&20], [0, 127, 0]);
    assert_eq!(on[&10], [127, 0, 0]);
}

#[test]
fn mute_is_a_mode_where_pads_mute_groups() {
    let mut d = lp();
    let mut v = lp_view();
    let now = Instant::now();
    assert_eq!(lights(&d.open(&v, now))[&2], [19, 19, 0], "dim while off");
    assert_eq!(lights(&d.midi("daw", &[0xB0, 2, 127], &v, now))[&2], [127, 127, 0]);
    // Any pad in a group's column mutes the group; one with no group nothing.
    assert_eq!(apps(&d.midi("daw", &[0x90, 71, 100], &v, now)), [Action::MuteGroup(0)]);
    assert_eq!(apps(&d.midi("daw", &[0x90, 42, 100], &v, now)), [Action::MuteGroup(1)]);
    assert_eq!(apps(&d.midi("daw", &[0x90, 83, 100], &v, now)), []);
    // A scene's button mutes the groups that play in it; when they all are, unmutes them.
    assert_eq!(apps(&d.midi("daw", &[0xB0, 103, 127], &v, now)), [Action::MuteGroup(0), Action::MuteGroup(1)]);
    v.groups[0].muted = true;
    assert_eq!(apps(&d.midi("daw", &[0xB0, 103, 127], &v, now)), [Action::MuteGroup(1)]);
    assert_eq!(apps(&d.midi("daw", &[0xB0, 102, 127], &v, now)), [Action::MuteGroup(0)], "only Drums plays in scene 2");
    v.groups[1].muted = true;
    assert_eq!(apps(&d.midi("daw", &[0xB0, 103, 127], &v, now)), [Action::MuteGroup(0), Action::MuteGroup(1)]);
    // (The place for a new scene makes none.)
    assert_eq!(apps(&d.midi("daw", &[0xB0, 104, 127], &v, now)), []);
    // Mute again: off, and the pads are the patterns again.
    assert_eq!(lights(&d.midi("daw", &[0xB0, 2, 127], &v, now))[&2], [19, 19, 0]);
    assert_eq!(apps(&d.midi("daw", &[0x90, 71, 100], &v, now)), [Action::FocusGroup(0), Action::PickPattern(1)]);
}

#[test]
fn a_muted_groups_patterns_are_dimmer() {
    let mut d = lp();
    let mut v = lp_view();
    v.groups[0].muted = true;
    let l = lights(&d.open(&v, Instant::now()));
    assert_eq!(l[&81], [45, 12, 11], "in the scene, but muted");
    assert_eq!(l[&71], [0, 19, 0]);
    assert_eq!(l[&82], BLUE);
}

/// The grid's bottom row while playing: the longest pattern in the scene spread over
/// the eight pads however long it is (an eighth each), the part played dim, the pad
/// playing full, those to come faint. (It used to be a pad a bar, so a one-bar
/// pattern's never moved: BUGS #46.)
#[test]
fn the_grids_bottom_row_spreads_the_longest_pattern_over_eight_pads() {
    let mut d = lp();
    let mut v = lp_view();
    v.playing = true;
    v.groups[0].patterns[0].bars = 4.0;
    v.groups[0].progress = Some(0.5);
    v.groups[1].patterns[0].bars = 2.0;
    v.groups[1].progress = Some(0.9);
    let now = Instant::now();
    let l = lights(&d.open(&v, now));
    let row: Vec<[u8; 3]> = (11..=18).map(|b| l[&b]).collect();
    let (dim, full) = ([19, 19, 19], [127, 127, 127]);
    assert_eq!(row, [dim, dim, dim, dim, full, NEW, NEW, NEW], "the four-bar pattern, halfway");
    // Its pads do nothing.
    assert_eq!(apps(&d.midi("daw", &[0x90, 11, 100], &v, now)), []);
    assert_eq!(apps(&d.midi("daw", &[0x90, 12, 100], &v, now)), []);
    // A one-bar pattern moves across the row too. (A tick sends only what changed.)
    v.groups[0].patterns[0].bars = 1.0;
    v.groups[1].patterns[0].bars = 1.0;
    let mut row = l.clone();
    for (progress, pad) in [(0.0, 0), (0.3, 2), (0.99, 7)] {
        v.groups[0].progress = Some(progress);
        v.groups[1].progress = Some(progress);
        row.extend(lights(&d.tick(&v, now)));
        let lit: Vec<u8> = (11..=18).filter(|b| row[b] == full).map(|b| b - 11).collect();
        assert_eq!(lit, [pad], "at {progress}");
    }
}

#[test]
fn stopped_the_bottom_rows_first_pad_strobes_with_the_tempo() {
    let mut d = lp();
    let v = lp_view();
    let t0 = Instant::now();
    let l = lights(&d.open(&v, t0));
    assert_eq!(l[&11], [127, 127, 127], "on the beat");
    for b in 12..=18 {
        assert_eq!(l[&b], OFF, "pad {b}");
    }
    // 120 BPM: a beat each half second, lit for its first quarter.
    use std::time::Duration;
    assert_eq!(lights(&d.tick(&v, t0 + Duration::from_millis(300)))[&11], OFF);
    assert_eq!(lights(&d.tick(&v, t0 + Duration::from_millis(510)))[&11], [127, 127, 127]);
}
