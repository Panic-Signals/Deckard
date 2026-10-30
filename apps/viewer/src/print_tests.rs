//! Print to Clip from a MIDI controller (the Launchpad Pro's sequencer): into the
//! focused sound's notes of the focused group's pattern in the focused scene.

use deckard_surface::device::{Print, PrintedNote};

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

fn note(pitch: u8, start: f64, length: f64, velocity: u8) -> PrintedNote {
    PrintedNote { pitch, start, length, velocity }
}

/// The focused group's pattern in the focused scene: its length and its notes as
/// (sound, position, length, pitch, velocity), sorted.
fn printed(app: &crate::App) -> Option<(i64, Vec<(usize, i64, i64, i64, i64)>)> {
    let d = app.doc.as_ref().unwrap().borrow();
    let p = d.project();
    let pi = p.scenes.get(app.scene)?.patterns.iter().find(|(g, _)| *g == app.group)?.1;
    let pt = &p.groups[app.group].patterns[pi];
    let mut notes: Vec<_> = pt.notes.iter().map(|n| (n.sound, n.position, n.length, n.pitch, n.velocity)).collect();
    notes.sort();
    Some((pt.length, notes))
}

fn add(app: &crate::App, sound: usize, position: i64, pitch: i64) {
    let d = app.doc.as_ref().unwrap();
    let n = deckard_model::Note { sound, position, length: 240, pitch, velocity: 90, probability: 100, expr: None };
    d.borrow_mut().edit_notes("Add", app.group, app.pattern, &[], &[n]).unwrap();
}

#[test]
fn printing_replaces_the_focused_sounds_notes() {
    let mut app = app();
    app.surface_act(deckard_surface::Action::PatternEdit("create"));
    add(&app, 0, 0, 36);
    add(&app, 1, 0, 60);
    add(&app, 1, 480, 61);
    app.sound = 1;
    app.print_notes(&Print { length: 4.0, notes: vec![note(64, 0.5, 0.25, 100), note(65, 1.0, 0.5, 80)], ..Default::default() });
    let (_, notes) = printed(&app).expect("a pattern in the scene");
    assert_eq!(notes, [(0, 0, 240, 36, 90), (1, 480, 240, 64, 100), (1, 960, 480, 65, 80)]);
    // One undo takes the print back.
    app.surface_act(deckard_surface::Action::Edit("undo_step"));
    assert_eq!(printed(&app).unwrap().1, [(0, 0, 240, 36, 90), (1, 0, 240, 60, 90), (1, 480, 240, 61, 90)]);
}

#[test]
fn printing_makes_a_pattern_when_there_is_none() {
    let mut app = app();
    assert_eq!(printed(&app), None, "a new project's scene has no pattern");
    app.print_notes(&Print { length: 8.0, notes: vec![note(60, 0.0, 1.0, 100)], ..Default::default() });
    let (length, notes) = printed(&app).expect("made and put in the scene");
    assert_eq!(length, 8 * 960);
    assert_eq!(notes, [(app.sound, 0, 960, 60, 100)]);
    assert_eq!(app.pattern, app.doc.as_ref().unwrap().borrow().project().scenes[app.scene].patterns[0].1, "focused");
}

#[test]
fn printing_makes_the_pattern_the_sequences_length() {
    let mut app = app();
    app.surface_act(deckard_surface::Action::PatternEdit("create"));
    app.print_notes(&Print { length: 8.0, notes: vec![], ..Default::default() });
    assert_eq!(printed(&app).unwrap().0, 8 * 960);
    // Shorter too.
    app.print_notes(&Print { length: 1.0, notes: vec![], ..Default::default() });
    assert_eq!(printed(&app).unwrap().0, 960);
}

#[test]
fn scenes_and_patterns_have_their_colours() {
    let app = app();
    let v = app.surface_view();
    assert_ne!(v.scenes[0].rgb, [0, 0, 0]);
}

#[test]
fn a_note_past_the_end_wraps() {
    let mut app = app();
    app.print_notes(&Print { length: 4.0, notes: vec![note(60, 3.5, 1.0, 100)], ..Default::default() });
    let s = app.sound;
    assert_eq!(printed(&app).unwrap().1, [(s, 0, 480, 60, 100), (s, 3360, 480, 60, 100)]);
}

#[test]
fn a_note_as_long_as_the_pattern_stays_one_note() {
    let mut app = app();
    app.print_notes(&Print { length: 4.0, notes: vec![note(60, 0.0, 4.0, 100), note(62, 0.0, 6.0, 90)], ..Default::default() });
    let s = app.sound;
    assert_eq!(printed(&app).unwrap().1, [(s, 0, 3840, 60, 100), (s, 0, 3840, 62, 90)]);
}

#[test]
fn a_print_with_no_length_takes_whole_bars() {
    let mut app = app();
    app.print_notes(&Print { length: 0.0, notes: vec![note(60, 4.5, 0.25, 100)], ..Default::default() });
    assert_eq!(printed(&app).unwrap().0, 2 * 4 * 960);
}

#[test]
fn clock_isnt_sent_to_a_controllers_own_port() {
    let mut app = app();
    app.settings.midi_sync = crate::settings::MidiSync::Send;
    app.midi_devices = crate::midi_devices::MidiDevices::only(None, &["launchpad_pro_mk3"]);
    let (daw, midi) = ("Launchpad Pro MK3 LPProMK3 DAW".to_string(), "Launchpad Pro MK3 LPProMK3 MIDI".to_string());
    app.midi_ports = (vec![], vec![daw, midi.clone()]);
    app.midi_ports_at = Some(std::time::Instant::now());
    let ports: Vec<String> = app.clock_out().unwrap().ports.iter().map(|p| p.to_string()).collect();
    assert_eq!(ports, [midi]);
}

#[test]
fn launch_scene_focuses_it() {
    let mut app = app();
    app.surface_act(deckard_surface::Action::SceneEdit("create"));
    app.surface_act(deckard_surface::Action::LaunchScene(0));
    assert_eq!(app.scene, 0);
}

/// The focused group taking MIDI as pads (Key Mode Manual from C1, or Drumkit from
/// C2): a print plays its pads, as the Launchpad's drum tracks do, and stands for the
/// whole group's notes.
#[test]
fn a_print_into_a_group_taking_pads_plays_them() {
    use crate::layout::{Act, apply_acts};
    let mut app = app();
    apply_acts(&mut app, vec![Act::SetParam(deckard_doc::Target::Group(0), "key_mode", 2.0)]);
    app.surface_act(deckard_surface::Action::PatternEdit("create"));
    add(&app, 5, 0, 60);
    app.sound = 1;
    app.print_notes(&Print {
        length: 4.0,
        notes: vec![note(36, 0.0, 0.25, 100), note(38, 1.0, 0.25, 90), note(52, 2.0, 0.25, 80)],
        ..Default::default()
    });
    // Pad 1 and pad 3; past pad 16 nothing; sound 6's note gone.
    assert_eq!(printed(&app).unwrap().1, [(0, 0, 240, 60, 100), (2, 960, 240, 60, 90)]);

    apply_acts(&mut app, vec![Act::SetParam(deckard_doc::Target::Group(0), "key_mode", 1.0)]);
    app.print_notes(&Print { length: 4.0, notes: vec![note(48, 0.0, 0.25, 100)], ..Default::default() });
    assert_eq!(printed(&app).unwrap().1, [(0, 0, 240, 60, 100)]);
}

#[test]
fn a_drum_channel_reaches_the_midi_routing() {
    let mut app = app();
    app.midi_devices = crate::midi_devices::MidiDevices::only(None, &["launchpad_pro_mk3"]);
    assert_eq!(app.drum_ports(), vec![], "off");
    app.settings.drum_channels.insert("launchpad_pro_mk3".into(), 10);
    assert_eq!(app.drum_ports(), vec![("LPProMK3 MIDI".to_string(), 10)]);
}

/// The Launchpad's drum track (Preferences ▸ MIDI, beside the drum channel): its prints
/// land on the focused group's pads from C1, as its notes do; other tracks' as before.
#[test]
fn a_print_from_the_drum_track_plays_the_pads() {
    let mut app = app();
    app.settings.drum_tracks.insert("launchpad_pro_mk3".into(), 1);
    app.surface_act(deckard_surface::Action::PatternEdit("create"));
    add(&app, 5, 0, 60);
    let drums = |track| Print {
        length: 4.0,
        notes: vec![note(36, 0.0, 0.25, 100), note(42, 1.0, 0.25, 90)],
        track: Some(track),
        device: "launchpad_pro_mk3".into(),
    };
    app.print_notes(&drums(1));
    assert_eq!(printed(&app).unwrap().1, [(0, 0, 240, 60, 100), (6, 960, 240, 60, 90)]);
    // Track 2: notes on the focused sound, as before.
    app.sound = 3;
    app.print_notes(&drums(2));
    assert_eq!(printed(&app).unwrap().1, [(0, 0, 240, 60, 100), (3, 0, 240, 36, 100), (3, 960, 240, 42, 90), (6, 960, 240, 60, 90)]);
}
