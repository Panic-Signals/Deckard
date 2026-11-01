//! Auto length recording (docs/superpowers/plans/2026-10-23-auto-length-recording.md):
//! REC arms when stopped, SHIFT + REC counts in, and an Auto-length pattern recorded
//! into from empty grows a bar ahead of the song until REC is off, then keeps the bars
//! played.

use deckard_engine::{Mode, Recorded};

/// A new project: group 0's empty pattern 0 in scene 0, focused.
fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "Pattern 1").unwrap();
    doc.borrow_mut().assign_pattern(0, 0, Some(0)).unwrap();
    (app.group, app.pattern, app.scene) = (0, 0, 0);
    app
}

fn length(app: &crate::App) -> i64 {
    app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[0].length
}

fn bar(app: &crate::App) -> i64 {
    app.doc.as_ref().unwrap().borrow().project().bar_length()
}

/// A note played at song position `tick` (scene 0), held `len`.
fn play(app: &mut crate::App, tick: f64, len: f64) {
    let on = Recorded { group: 0, sound: 0, pitch: 60, velocity: 100, length: i64::MAX, tick, mode: Mode::Scene(0), seq: 0, played: tick };
    app.take_in_recorded(vec![on]);
    app.take_in_recorded(vec![Recorded { velocity: 0, tick: tick + len, played: tick + len, ..on }]);
}

/// REC with the song stopped arms: nothing plays until PLAY.
#[test]
fn rec_when_stopped_arms() {
    let mut app = app();
    app.set_recording(true, false);
    assert!(app.recording, "armed");
    assert!(app.playing.is_none(), "nothing plays yet");
    assert_eq!(app.count_in_next, 0);
}

/// SHIFT + REC counts in (at least a bar), then plays.
#[test]
fn shift_rec_counts_in() {
    let mut app = app();
    app.settings.count_in_bars = 0;
    app.set_recording(true, true);
    assert!(app.recording);
    // Played at once (the count-in handed to the engine), or still to come if there's
    // no audio here.
    assert!(app.playing.is_some() || app.count_in_next == 1, "a bar's count-in");
}

/// Auto length, nothing played: it grows a bar ahead of the song all the same; REC off,
/// it's as it was.
#[test]
fn an_empty_auto_pattern_records_from_one_bar() {
    let mut app = app();
    let bar = bar(&app);
    let before = length(&app);
    app.set_recording(true, false);
    assert_eq!(length(&app), bar);
    app.auto_grow_at(Mode::Scene(0), 500.0);
    assert_eq!(length(&app), 2 * bar, "a bar ahead");
    app.auto_grow_at(Mode::Scene(0), (bar + 10) as f64);
    assert_eq!(length(&app), 3 * bar);
    app.set_recording(false, false);
    assert_eq!(length(&app), before, "nothing played: as it was");
}

/// An empty bar doesn't stop it: it grows on until REC is off, then keeps the bars
/// played (up to the last note's).
#[test]
fn an_auto_pattern_grows_until_rec_is_off() {
    let mut app = app();
    let bar = bar(&app);
    app.set_recording(true, false);
    app.auto_grow_at(Mode::Scene(0), 100.0);
    play(&mut app, 600.0, 240.0);
    assert_eq!(length(&app), 2 * bar);
    app.auto_grow_at(Mode::Scene(0), (bar + 100) as f64);
    assert_eq!(length(&app), 3 * bar, "bar 2 empty: on into bar 3");
    app.auto_grow_at(Mode::Scene(0), (2 * bar + 100) as f64);
    play(&mut app, (2 * bar + 700) as f64, 100.0);
    assert_eq!(length(&app), 4 * bar);
    app.set_recording(false, false);
    assert_eq!(length(&app), 3 * bar, "to the last note's bar");
    let doc = app.doc.clone().unwrap();
    assert_eq!(doc.borrow().project().groups[0].patterns[0].notes.len(), 2);
}

/// A note in the last bar adds the next too (before the song gets there).
#[test]
fn an_auto_pattern_grows_while_played_into() {
    let mut app = app();
    let bar = bar(&app);
    app.set_recording(true, false);
    play(&mut app, 100.0, 100.0);
    assert_eq!(length(&app), 2 * bar);
    play(&mut app, (bar + 200) as f64, 100.0);
    assert_eq!(length(&app), 3 * bar);
    app.set_recording(false, false);
    assert_eq!(length(&app), 2 * bar);
}

/// A new pattern after a 4-bar one (chosen, with notes): recorded into, it grows past 4
/// bars, not held to the other's length.
#[test]
fn a_new_pattern_after_a_long_one_grows_past_it() {
    use crate::layout::{Act, apply_acts};
    let mut app = app();
    let bar = bar(&app);
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().set_pattern_length(0, 0, 4 * bar).unwrap();
    doc.borrow_mut().set_length_auto(0, 0, false);
    let n = deckard_model::Note { expr: None, sound: 1, position: 0, length: 100, pitch: 60, velocity: 100, probability: 100 };
    doc.borrow_mut().edit_notes("t", 0, 0, &[], &[n]).unwrap();
    apply_acts(&mut app, vec![Act::NewPattern(0, 0)]);
    assert_eq!(app.pattern, 1);
    assert_eq!(doc.borrow().project().scenes[0].patterns, [(0, 1)]);
    app.set_recording(true, false);
    assert!(app.grow.is_some(), "auto length: {}", app.status);
    let len = |app: &crate::App| app.doc.as_ref().unwrap().borrow().project().groups[0].patterns[1].length;
    assert_eq!(len(&app), bar);
    for b in 0..6 {
        app.auto_grow_at(Mode::Scene(0), (b * bar + 10) as f64);
        play(&mut app, (b * bar + 200) as f64, 100.0);
    }
    assert_eq!(len(&app), 7 * bar, "a bar past the last played");
    app.set_recording(false, false);
    assert_eq!(len(&app), 6 * bar);
}

/// A chosen length (not Auto) records at that length.
#[test]
fn a_chosen_length_doesnt_grow() {
    let mut app = app();
    let bar = bar(&app);
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().set_pattern_length(0, 0, 4 * bar).unwrap();
    doc.borrow_mut().set_length_auto(0, 0, false);
    app.set_recording(true, false);
    assert_eq!(length(&app), 4 * bar);
    play(&mut app, 100.0, 100.0);
    app.set_recording(false, false);
    assert_eq!(length(&app), 4 * bar);
}

/// A pattern with notes records at its length.
#[test]
fn a_pattern_with_notes_doesnt_grow() {
    let mut app = app();
    let bar = bar(&app);
    let doc = app.doc.clone().unwrap();
    doc.borrow_mut().set_pattern_length(0, 0, 2 * bar).unwrap();
    let n = deckard_model::Note { expr: None, sound: 1, position: 0, length: 100, pitch: 60, velocity: 100, probability: 100 };
    doc.borrow_mut().edit_notes("t", 0, 0, &[], &[n]).unwrap();
    app.set_recording(true, false);
    assert_eq!(length(&app), 2 * bar);
}

/// The software: SHIFT + REC (a shift-click on ●) counts in; choosing a pattern length
/// turns Auto off, choosing Auto turns it back on.
#[test]
fn the_softwares_rec_and_length_menu() {
    use crate::layout::{Act, apply_acts};
    let mut app = app();
    let bar = bar(&app);
    apply_acts(&mut app, vec![Act::RecordCountIn]);
    assert!(app.recording);
    assert!(app.playing.is_some() || app.count_in_next == 1);
    apply_acts(&mut app, vec![Act::Record]);
    assert!(!app.recording);
    let doc = app.doc.clone().unwrap();
    apply_acts(&mut app, vec![Act::PatternLength(0, 0, 2 * bar)]);
    assert!(!doc.borrow().length_is_auto(0, 0), "a length chosen");
    apply_acts(&mut app, vec![Act::PatternLengthAuto(0, 0)]);
    assert!(doc.borrow().length_is_auto(0, 0));
}

/// Armed, SHIFT + REC still counts in and starts.
#[test]
fn shift_rec_when_armed_counts_in() {
    let mut app = app();
    app.settings.count_in_bars = 0;
    app.set_recording(true, false);
    assert!(app.playing.is_none());
    app.set_recording(true, true);
    assert!(app.recording);
    assert!(app.playing.is_some() || app.count_in_next == 1, "counted in and started");
}
