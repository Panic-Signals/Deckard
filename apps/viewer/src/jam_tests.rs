//! The Maschine JAM's script driving a real project: what it asks the app for does what it
//! should. Hardware-free, as `jam_events` takes made-up events; `App::jam` stays `None`, so
//! nothing opens a device.

use crate::jam_io::{DEVICE, coalesced};
use deckard_jam::{Event, Touch};
use deckard_surface::Action as A;
use deckard_surface::device::{Device, PadMap};
use std::time::Instant;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    // (As the app starts up: a test's default App has neither.)
    (app.grid, app.scale) = (deckard_model::PPQ / 4, (60, 0, 0));
    let view = app.surface_view();
    let mut d = Device::new(DEVICE, None);
    assert_eq!(d.error(), None);
    d.open(&view, Instant::now());
    app.jam_device = Some(d);
    app
}

fn run(app: &mut crate::App, events: Vec<Event>) -> PadMap {
    let (_, route) = app.jam_events(events, Instant::now());
    assert_eq!(app.jam_error, None);
    route
}

fn press(app: &mut crate::App, name: &'static str) {
    run(app, vec![Event::Button(name, true)]);
    run(app, vec![Event::Button(name, false)]);
}

/// Pad at column and row, from 1.
fn pad(app: &mut crate::App, col: u8, row: u8) {
    run(app, vec![Event::Pad { column: col - 1, row: row - 1, down: true }]);
    run(app, vec![Event::Pad { column: col - 1, row: row - 1, down: false }]);
}

fn project(app: &crate::App) -> deckard_model::Project {
    app.doc.as_ref().unwrap().borrow().project().clone()
}

fn strip(strip: u8, kind: Touch, pos: f64) -> Event {
    Event::Strip { strip: strip - 1, finger: 0, kind, pos }
}

#[test]
fn a_frames_moves_of_one_finger_become_the_last() {
    let got = coalesced(vec![
        strip(1, Touch::Down, 0.1),
        strip(1, Touch::Move, 0.2),
        strip(2, Touch::Move, 0.9),
        strip(1, Touch::Move, 0.3),
        Event::Button("SONG", true),
        strip(1, Touch::Up, 0.3),
        strip(1, Touch::Move, 0.4),
    ]);
    assert_eq!(
        got,
        [
            strip(1, Touch::Down, 0.1),
            strip(1, Touch::Move, 0.3),
            strip(2, Touch::Move, 0.9),
            Event::Button("SONG", true),
            strip(1, Touch::Up, 0.3),
            strip(1, Touch::Move, 0.4),
        ],
        "a touch or a release between stays"
    );
}

#[test]
fn project_view_makes_picks_and_deletes_patterns_and_scenes() {
    let mut app = app();
    assert_eq!(app.group, 0);
    let scenes = project(&app).scenes.len();
    let patterns = project(&app).groups[0].patterns.len();
    pad(&mut app, 1, 8);
    assert_eq!(project(&app).groups[0].patterns.len(), patterns + 1, "an unlit pad makes one");
    // The scene button past the last makes a scene; then its own press focuses it.
    let n = u8::try_from(scenes + 1).unwrap();
    assert!(n <= 8);
    press(&mut app, ["SCENE_1", "SCENE_2", "SCENE_3", "SCENE_4", "SCENE_5", "SCENE_6", "SCENE_7", "SCENE_8"][usize::from(n - 1)]);
    assert_eq!(project(&app).scenes.len(), scenes + 1);
    // CLEAR + that scene, CLEAR + the new pattern's pad (it's now first free row - 1).
    let at = project(&app).groups[0].patterns.len();
    run(&mut app, vec![Event::Button("CLEAR", true)]);
    pad(&mut app, 1, u8::try_from(at).unwrap());
    assert_eq!(project(&app).groups[0].patterns.len(), at - 1, "CLEAR + pad deletes it");
    press(&mut app, ["SCENE_1", "SCENE_2", "SCENE_3", "SCENE_4", "SCENE_5", "SCENE_6", "SCENE_7", "SCENE_8"][usize::from(n - 1)]);
    run(&mut app, vec![Event::Button("CLEAR", false)]);
    assert_eq!(project(&app).scenes.len(), scenes, "CLEAR + scene deletes it");
}

#[test]
fn clear_keeps_the_last_scene() {
    let mut app = app();
    let scenes = project(&app).scenes.len();
    assert_eq!(scenes, 1, "a new project's");
    app.surface_act(A::DeleteScene(0));
    assert_eq!(project(&app).scenes.len(), 1);
}

/// Step mode's lanes write another sound's notes and leave the focus where it was.
#[test]
fn step_lanes_write_other_sounds_and_keep_the_focus() {
    let mut app = app();
    run(&mut app, vec![Event::Button("STEP", true)]);
    press(&mut app, "SCENE_4");
    run(&mut app, vec![Event::Button("STEP", false)]);
    assert_eq!(app.sound, 0);
    pad(&mut app, 2, 3);
    assert_eq!(app.sound, 0, "the focus stays");
    let p = project(&app);
    let notes = &p.groups[0].patterns[app.pattern].notes;
    assert_eq!(notes.len(), 1, "with no pattern, a new one first");
    assert_eq!((notes[0].sound, notes[0].position, notes[0].velocity), (1, app.grid, 100));
    pad(&mut app, 2, 3);
    assert!(project(&app).groups[0].patterns[app.pattern].notes.is_empty(), "again: gone");
}

#[test]
fn duplicate_copies_patterns_and_the_copy_keeps_its_scene() {
    let mut app = app();
    app.surface_step(240, 240, 100);
    let first = app.pattern;
    let notes = project(&app).groups[0].patterns[first].notes.clone();
    assert_eq!(notes.len(), 1);
    app.surface_act(A::CopyPattern { group: 0, from: first, to: first + 1 });
    let p = project(&app);
    assert_eq!(p.groups[0].patterns.len(), first + 2, "past the last: a new one");
    let copy = &p.groups[0].patterns[first + 1];
    assert_eq!(copy.notes.iter().map(|n| n.position - copy.start).collect::<Vec<_>>(), [240]);
    // Onto one with its own notes: they're replaced.
    app.surface_act(A::PickPattern(first + 1));
    app.surface_step(960, 240, 100);
    assert_eq!(project(&app).groups[0].patterns[first + 1].notes.len(), 2);
    app.surface_act(A::CopyPattern { group: 0, from: first, to: first + 1 });
    let p = project(&app);
    assert_eq!(p.groups[0].patterns[first + 1].notes.len(), 1, "replaced");
    assert_eq!(p.groups[0].patterns[first].notes, notes, "the source is as it was");
    assert!(app.doc.as_ref().unwrap().borrow_mut().undo(), "one undo step");
    assert_eq!(project(&app).groups[0].patterns[first + 1].notes.len(), 2);
}

#[test]
fn duplicate_copies_scenes() {
    let mut app = app();
    app.surface_step(240, 240, 100);
    let plays = project(&app).scenes[app.scene].patterns.clone();
    assert!(!plays.is_empty());
    app.surface_act(A::CopyScene { from: 0, to: 1 });
    let p = project(&app);
    assert_eq!(p.scenes.len(), 2, "past the last: a new one");
    assert_eq!(p.scenes[1].patterns, plays);
}

#[test]
fn duplicate_copies_sounds_and_groups() {
    let mut app = app();
    app.surface_step(240, 240, 100);
    app.surface_act(A::CopySound { from: 0, to: 3 });
    let p = project(&app);
    assert_eq!(p.groups[0].patterns[app.pattern].notes.iter().filter(|n| n.sound == 3).count(), 1, "with its notes");
    app.surface_act(A::CopySound { from: 0, to: 99 });
    let groups = project(&app).groups.len();
    app.surface_act(A::CopyGroup { from: 0, to: groups });
    let p = project(&app);
    assert_eq!(p.groups.len(), groups + 1, "past the last: a new one");
    assert_eq!(p.groups[groups].patterns.len(), p.groups[0].patterns.len(), "with its patterns");
    app.surface_act(A::CopyGroup { from: 0, to: groups + 5 });
    assert_eq!(project(&app).groups.len(), groups + 1, "not past the place after the last");
}

/// Each channel value the strips set comes back in the view they light from.
#[test]
fn channel_values_are_set_and_shown() {
    let mut app = app();
    app.surface_act(A::SetSend { group: 0, sound: None, aux: 1, level: 0.5 });
    assert_eq!(app.surface_view().groups[0].mix.aux, [0.0, 0.5]);
    app.surface_act(A::SetSend { group: 0, sound: Some(2), aux: 0, level: 0.25 });
    assert_eq!(app.surface_view().groups[0].sounds[2].mix.aux, [0.25, 0.0]);
    app.surface_act(A::SetChannelSwing { group: None, sound: None, amount: 10.0 });
    assert_eq!(app.surface_view().swing, 10.0);
    app.surface_act(A::SetChannelSwing { group: Some(0), sound: None, amount: 20.0 });
    assert_eq!(app.surface_view().groups[0].mix.swing, 20.0);
    app.surface_act(A::SetChannelSwing { group: Some(0), sound: Some(1), amount: 30.0 });
    assert_eq!(app.surface_view().groups[0].sounds[1].mix.swing, 30.0);
    assert!(!app.surface_view().keep_focus, "the focus follows by default");
    app.settings.focus_follows = false;
    assert!(app.surface_view().keep_focus);
    app.settings.focus_follows = true;
    app.surface_act(A::CueDb(0.0));
    assert!((app.surface_view().cue_level - 0.75).abs() < 1e-3, "unity");
    assert_eq!(app.surface_view().input_levels, [0.75, 0.75], "unity to start with");
    app.surface_act(A::InputDb(1, -6.0));
    let levels = app.surface_view().input_levels;
    assert_eq!(levels[0], 0.75);
    assert!((levels[1] - deckard_model::level_from_db(-6.0)).abs() < 1e-9);
    assert!((app.input_bus.gain(2) - deckard_model::level_gain(levels[1]) as f32).abs() < 1e-6, "on the way in");
    assert_eq!(app.input_bus.gain(0), 1.0);
    app.settings.input_levels = [0.75, 0.75];
    app.input_bus.set_levels(&app.settings.input_levels);
    assert_eq!(app.surface_view().scale_root, 60);
}

/// A sound with no Sampler has no tune; one with a Sampler does, and a group's tune moves its
/// Samplers together.
#[test]
fn tune_is_a_samplers() {
    use deckard_doc::NewModule;
    let mut app = app();
    assert_eq!(app.surface_view().groups[0].mix.tune, None);
    app.surface_act(A::SetTune { group: 0, sound: Some(0), semitones: 5.0 });
    app.surface_act(A::SetTune { group: 0, sound: None, semitones: 5.0 });
    {
        let mut d = app.doc.as_ref().unwrap().borrow_mut();
        d.add_slot(0, 0, 0, &NewModule::Sampler).unwrap();
        d.add_slot(0, 1, 0, &NewModule::Sampler).unwrap();
    }
    app.surface_act(A::SetTune { group: 0, sound: Some(1), semitones: 2.0 });
    let v = app.surface_view();
    assert_eq!((v.groups[0].sounds[0].mix.tune, v.groups[0].sounds[1].mix.tune), (Some(0.0), Some(2.0)));
    app.surface_act(A::SetTune { group: 0, sound: None, semitones: 3.0 });
    let v = app.surface_view();
    assert_eq!(v.groups[0].mix.tune, Some(3.0), "the group's reads as its first");
    assert_eq!(v.groups[0].sounds[1].mix.tune, Some(5.0), "the offset stays");
}

#[test]
fn the_perform_effect_is_chosen_and_added_if_there_is_none() {
    let mut app = app();
    assert_eq!(app.surface_view().groups[0].perform_mode, None);
    app.surface_act(A::PerformMode { group: 0, mode: 2 });
    assert_eq!(app.surface_view().groups[0].perform_mode, Some((2, "Burst Echo".to_string())));
    app.surface_act(A::PerformMode { group: 0, mode: 7 });
    assert_eq!(app.surface_view().groups[0].perform_mode, Some((7, "Scratcher".to_string())));
}

/// A Perform FX's knobs have defaults; a double tap on a Control strip goes back to them.
#[test]
fn a_knob_goes_back_to_its_default() {
    let mut app = app();
    app.surface_act(A::PerformMode { group: 0, mode: 0 });
    app.focus_group(0);
    app.control_request(deckard_lua_host::state::ControlRequest::ShowPerformFx(0));
    app.update_slot_view();
    let was = app.surface_view().control.knobs.iter().flatten().map(|k| k.value).collect::<Vec<_>>();
    assert!(!was.is_empty());
    let turned = (0..8).find(|&k| {
        let before = app.surface_view().control.knobs[k].as_ref().map(|x| x.value);
        app.surface_act(A::Turn { knob: k, inc: 0.2, fine: false });
        app.update_slot_view();
        before.is_some() && app.surface_view().control.knobs[k].as_ref().map(|x| x.value) != before
    });
    let Some(k) = turned else { panic!("a knob that turns") };
    app.surface_act(A::ResetKnob(k));
    app.update_slot_view();
    let now = app.surface_view().control.knobs.iter().flatten().map(|x| x.value).collect::<Vec<_>>();
    assert!((now[k.min(now.len() - 1)] - was[k.min(was.len() - 1)]).abs() < 1e-3, "{now:?} vs {was:?}");
}

#[test]
fn lock_view_reaches_every_slot() {
    let mut app = app();
    app.surface_act(A::Lock("slot", 40));
    let l = app.surface_view().lock;
    assert!(l.slots.contains(&40), "a free slot takes one, whatever bank shows");
    let before = l.bank;
    app.surface_act(A::Lock("slot", 40));
    let l = app.surface_view().lock;
    assert_eq!((l.focus, l.bank), (Some(40), before), "taken: recalled and in focus");
    // The MK3's pads still count in the bank shown.
    app.surface_act(A::Lock("pad", 3));
    assert!(app.surface_view().lock.slots.contains(&3));
}

#[test]
fn note_repeat_goes_on_and_off_from_a_controller_with_no_page() {
    let mut app = app();
    press(&mut app, "NOTE_REPEAT");
    app.sync_arp();
    assert!(app.repeat_sent.is_some());
    assert!(app.surface_view().note_repeat.active);
    press(&mut app, "NOTE_REPEAT");
    app.sync_arp();
    assert!(app.repeat_sent.is_none());
}

/// Without an engine, choke asks nothing of anything.
#[test]
fn choke_needs_no_engine() {
    let mut app = app();
    app.surface_act(A::ChokeAll);
}

/// The pads' route, from the app's real view: Pad mode's block, and Keyboard's 64 in the scale.
#[test]
fn the_route_is_the_focused_groups() {
    let mut app = app();
    let route = run(&mut app, Vec::new());
    assert!(route.pads.iter().all(Vec::is_empty), "Project View");
    press(&mut app, "PAD_MODE");
    let route = run(&mut app, Vec::new());
    assert_eq!(route.pads[60], [(0, 0, 60)], "pad 13: the bottom-right block's top row is 13-16, its bottom row 1-4");
    assert_eq!(route.pads[56 + 4], [(0, 0, 60)], "pad 1 at E8");
    assert_eq!(route.velocity, 100);
}

/// Every button on the panel, with and without SHIFT, in every view, on the app's real view:
/// no script error, and the app survives what it asked.
#[test]
fn every_control_runs() {
    let mut app = app();
    let names: Vec<&'static str> = (0..120)
        .filter_map(|n| match deckard_jam::map::control(n) {
            deckard_jam::map::Control::Button(b) if !matches!(b, "BROWSE" | "GROUP_G" | "SHIFT") => Some(b),
            _ => None,
        })
        .collect();
    for shift in [false, true] {
        if shift {
            run(&mut app, vec![Event::Button("SHIFT", true)]);
        }
        for name in &names {
            run(&mut app, vec![Event::Button(name, true)]);
            run(&mut app, vec![Event::Encoder(1)]);
            run(&mut app, vec![Event::Pad { column: 2, row: 2, down: true }]);
            run(&mut app, vec![Event::Pad { column: 2, row: 2, down: false }]);
            run(&mut app, vec![strip(1, Touch::Down, 0.5), strip(1, Touch::Move, 0.6), strip(1, Touch::Up, 0.6)]);
            run(&mut app, vec![Event::Button(name, false)]);
        }
        if shift {
            run(&mut app, vec![Event::Button("SHIFT", false)]);
        }
    }
    assert_eq!(app.jam_error, None);
    let _ = project(&app);
}
