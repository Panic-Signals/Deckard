//! The Events page of DECKARD's own controller layer: its edits of the selected notes
//! ([`deckard_surface::Action::Events`]).

use deckard_model::Note;
use deckard_surface::Action as A;

fn note(sound: usize, position: i64, pitch: i64) -> Note {
    Note { expr: None, sound, position, length: 240, pitch, velocity: 100, probability: 100 }
}

/// A new project whose focused pattern (a bar) has these notes.
fn app(notes: &[Note]) -> crate::App {
    let mut app = crate::App { grid: 240, ..Default::default() };
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "Pattern 1").unwrap();
    doc.borrow_mut().assign_pattern(0, 0, Some(0)).unwrap();
    doc.borrow_mut().set_pattern_length(0, 0, 3840).unwrap();
    doc.borrow_mut().edit_notes("t", 0, 0, &[], notes).unwrap();
    (app.group, app.pattern, app.scene) = (0, 0, 0);
    app
}

fn notes(app: &crate::App) -> Vec<Note> {
    let mut n = app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[0].notes.clone();
    n.sort_by_key(|n| (n.sound, n.position));
    n
}

fn selected(app: &mut crate::App) -> Vec<(usize, i64)> {
    let mut v: Vec<(usize, i64)> = app.surface_view().notes.iter().filter(|n| n.selected).map(|n| (n.sound, n.position)).collect();
    v.sort();
    v
}

#[test]
fn the_pads_and_knobs_select_notes() {
    let mut app = app(&[note(0, 0, 60), note(0, 960, 62), note(1, 480, 60)]);
    app.surface_act(A::Events("select_sound", 0));
    assert_eq!(selected(&mut app), [(0, 0), (0, 960)], "sound 1's notes");
    assert_eq!(app.sound, 0);
    app.surface_act(A::Events("select_sound", 0));
    assert!(selected(&mut app).is_empty(), "again: out of the selection");
    app.surface_act(A::Events("select_all", 0));
    assert_eq!(selected(&mut app).len(), 3);
    app.surface_act(A::Events("select_none", 0));
    assert!(selected(&mut app).is_empty());
    app.surface_act(A::Events("select_step", 1));
    assert_eq!(selected(&mut app), [(0, 0)], "the focused sound's first note");
    app.surface_act(A::Events("select_step", 1));
    assert_eq!(selected(&mut app), [(0, 960)], "then the next");
    app.surface_act(A::EventsRange { start: 0, end: 3840, low: 61, high: 127 });
    assert_eq!(selected(&mut app), [(0, 960)], "pitches from 61");
    app.surface_act(A::Events("select_pitch", 60));
    assert_eq!(selected(&mut app), [(0, 0), (0, 960)], "the focused sound's C3s too");
    app.surface_act(A::Events("erase_pitch", 62));
    assert_eq!(notes(&app), [note(0, 0, 60), note(1, 480, 60)]);
}

#[test]
fn the_knobs_edit_the_selected_notes() {
    let mut app = app(&[note(0, 0, 60), note(0, 960, 62), note(1, 500, 60)]);
    app.surface_act(A::Events("select_sound", 0));
    app.surface_act(A::Events("pitch", 1));
    app.surface_act(A::Events("velocity", -1));
    app.surface_act(A::Events("probability", -1));
    app.surface_act(A::Events("length", 1));
    app.surface_act(A::Events("length_fine", -1));
    let n = notes(&app);
    assert_eq!((n[0].pitch, n[0].velocity, n[0].probability, n[0].length), (61, 99, 99, 240 + 240 - 30));
    assert_eq!(n[1].pitch, 63);
    assert_eq!(n[2], note(1, 500, 60), "not selected: as it was");
    assert_eq!(selected(&mut app).len(), 2, "still selected");
    app.surface_act(A::Events("nudge", 1));
    assert_eq!((notes(&app)[0].position, notes(&app)[1].position), (120, 1080), "by the Nudge Grid (as new: half a step)");
    app.surface_act(A::Events("nudge_fine", -1));
    assert_eq!(notes(&app)[0].position, 90);
    // Nothing selected: the knobs leave the notes; the 4-D encoder's nudge takes them all.
    app.surface_act(A::Events("select_none", 0));
    app.surface_act(A::Events("pitch", 1));
    assert_eq!(notes(&app)[2].pitch, 60);
    app.surface_act(A::Events("nudge_shown", 1));
    assert_eq!(notes(&app)[2].position, 620);
    app.surface_act(A::Events("quantize", 0));
    assert_eq!(notes(&app).iter().map(|n| n.position).collect::<Vec<_>>(), [240, 1200, 720]);
    app.surface_act(A::Events("clear_all", 1));
    assert_eq!(notes(&app).len(), 1, "the focused sound's");
    app.surface_act(A::Events("clear_all", 0));
    assert!(notes(&app).is_empty(), "the group's");
}

#[test]
fn step_mode_puts_steps_in_and_takes_them_out() {
    let mut app = app(&[note(0, 0, 60)]);
    app.surface_act(A::StepToggle { at: 240, length: 240, velocity: 90 });
    let added = Note { position: 240, velocity: 90, ..note(0, 240, 60) };
    assert_eq!(notes(&app), [note(0, 0, 60), added], "at the sound's base key, a step long");
    assert_eq!(selected(&mut app), [(0, 240)], "the new step selected");
    app.surface_act(A::StepToggle { at: 0, length: 240, velocity: 90 });
    assert_eq!(notes(&app), [added], "a step with a note: out");
}

#[test]
fn a_step_makes_a_pattern_when_theres_none() {
    let mut app = crate::App::default();
    app.new_project();
    (app.group, app.scene) = (0, 0);
    app.surface_act(A::StepToggle { at: 0, length: 240, velocity: 100 });
    let p = app.doc.as_ref().unwrap().borrow().project().clone();
    assert_eq!(p.groups[0].patterns.len(), 1, "{}", app.status);
    assert_eq!(p.groups[0].patterns[0].notes.len(), 1);
}

/// #7: quantizing while recording is its own undo step: undo takes back the notes
/// recorded after it, not the quantize too.
#[test]
fn quantizing_while_recording_is_its_own_undo_step() {
    let mut app = app(&[]);
    let doc = app.doc.clone().unwrap();
    let positions = |app: &crate::App| notes(app).iter().map(|n| n.position).collect::<Vec<_>>();
    doc.borrow_mut().begin_take("Record");
    doc.borrow_mut().edit_notes("Record", 0, 0, &[], &[note(0, 10, 60)]).unwrap();
    app.surface_act(A::Events("quantize", 0));
    crate::layout::apply_acts(&mut app, vec![crate::layout::Act::Quantize(100)]);
    doc.borrow_mut().edit_notes("Record", 0, 0, &[], &[note(0, 970, 60)]).unwrap();
    doc.borrow_mut().end_take();
    assert_eq!(positions(&app), [0, 970]);
    assert!(doc.borrow_mut().undo());
    assert_eq!(positions(&app), [0], "only the later notes");
}
