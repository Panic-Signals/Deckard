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
    let on = Recorded {
        midi: false,
        group: 0,
        sound: 0,
        pitch: 60,
        velocity: 100,
        length: i64::MAX,
        tick,
        mode: Mode::Scene(0),
        seq: 0,
        played: tick,
    };
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
    assert!(app.surface_view().pattern_auto, "still empty: still Auto");
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
    assert!(!doc.borrow().length_is_auto(0, 0), "settled length is no longer Auto");
    assert!(!app.surface_view().pattern_auto, "controller Pattern page shows settled length, not Auto");
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
    let n = deckard_model::Note {
        ratchet: Default::default(),
        expr: None,
        sound: 1,
        position: 0,
        length: 100,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
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
    let n = deckard_model::Note {
        ratchet: Default::default(),
        expr: None,
        sound: 1,
        position: 0,
        length: 100,
        pitch: 60,
        velocity: 100,
        probability: 100,
    };
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

/// During recording into an empty Auto-length pattern, the MK3 touchstrip LED dots reset
/// every bar when nothing was recorded during that bar, and keep spanning bars when
/// notes or controls (such as pitch bend) are recorded.
#[test]
fn mk3_strip_resets_every_bar_during_recording_when_nothing_was_recorded() {
    let mut app = app();
    app.controller = None;
    let bar = bar(&app);
    let at = |app: &mut crate::App, pos: i64| {
        app.playing = Some(Mode::Scene(0));
        app.parked = None;
        app.auto_grow_at(Mode::Scene(0), pos as f64);
        let head = Some(crate::Playhead { mode: Mode::Scene(0), queued: None, position: pos as f64, played: pos as f64 });
        let p = app.strip_progress(head).unwrap();
        let out = {
            let mut v = app.surface_view();
            v.progress = Some(p);
            app.surface.leds(&v)
        };
        let lit = (1..=25)
            .filter(|i| {
                let name = format!("LED_TS_{i:02}");
                let idx = deckard_mk3::leds::PAD_LEDS.iter().position(|n| *n == name).unwrap();
                out.pads[1 + idx] != 0
            })
            .count();
        (p, lit)
    };

    app.set_recording(true, false);

    // Bar 0: empty, fills 1..25 across the bar despite grow extending pattern length.
    let (p, lit) = at(&mut app, 0);
    assert!((p - 0.0).abs() < 1e-6 && lit == 1, "bar 0 start: {p} / {lit}");
    let (p, lit) = at(&mut app, bar / 2);
    assert!((p - 0.5).abs() < 1e-6 && lit == 13, "bar 0 mid: {p} / {lit}");

    // Bar 1: nothing recorded in bar 0 -> resets back to dot 1.
    let (p, lit) = at(&mut app, bar);
    assert!((p - 0.0).abs() < 1e-6 && lit == 1, "bar 1 reset: {p} / {lit}");
    let (p, lit) = at(&mut app, bar + bar / 2);
    assert!((p - 0.5).abs() < 1e-6 && lit == 13, "bar 1 mid: {p} / {lit}");

    // Record pitch bend in bar 1 -> bar 2 continues without resetting.
    app.take_in_all(
        Vec::new(),
        Vec::new(),
        vec![deckard_engine::RecordedControl {
            group: 0,
            sound: 0,
            kind: deckard_model::MidiKind::PitchBend(12000),
            tick: (bar + bar / 2) as f64,
            mode: Mode::Scene(0),
            seq: 1,
        }],
        std::time::Instant::now(),
    );
    let (p, lit) = at(&mut app, 2 * bar);
    assert!((p - 0.5).abs() < 1e-6 && lit == 13, "bar 2 continues after pitch bend in bar 1: {p} / {lit}");

    // Nothing recorded in bar 2 -> bar 3 resets back to dot 1.
    let (p, lit) = at(&mut app, 3 * bar);
    assert!((p - 0.0).abs() < 1e-6 && lit == 1, "bar 3 reset after empty bar 2: {p} / {lit}");

    // Record a note in bar 3 -> bar 4 continues without resetting.
    play(&mut app, (3 * bar + bar / 4) as f64, 120.0);
    let (p, lit) = at(&mut app, 4 * bar);
    assert!((p - 0.5).abs() < 1e-6 && lit == 13, "bar 4 continues after note in bar 3: {p} / {lit}");
}

/// Deleting or clearing the growing pattern, deleting an earlier pattern in the same
/// group, or deleting/moving groups during auto-length recording does not panic when
/// playback advances or REC turns off.
#[test]
fn deleting_or_moving_patterns_and_groups_while_growing_does_not_panic() {
    // 1. Deleting the growing pattern itself clears `grow` cleanly.
    let mut a = app();
    a.set_recording(true, false);
    assert!(a.grow.is_some());
    a.menu_act(crate::layout::Act::DeletePattern(0, 0));
    assert!(a.grow.is_none());
    a.auto_grow_at(Mode::Scene(0), 200.0);
    a.set_recording(false, false);

    // 2. Direct document pattern removal (e.g. undo / clear group) also settles safely.
    let mut a = app();
    a.set_recording(true, false);
    a.doc.as_ref().unwrap().borrow_mut().delete_pattern(0, 0).unwrap();
    a.auto_grow_at(Mode::Scene(0), 200.0);
    assert!(a.grow.is_none());
    a.set_recording(true, false);
    let doc = a.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "Pattern 1").unwrap();
    doc.borrow_mut().assign_pattern(0, 0, Some(0)).unwrap();
    (a.group, a.pattern) = (0, 0);
    a.set_recording(true, false);
    doc.borrow_mut().delete_pattern(0, 0).unwrap();
    a.set_recording(false, false);

    // 3. Deleting an earlier pattern in the same group shifts `grow.pattern` down.
    let mut a = app();
    let bar = bar(&a);
    let doc = a.doc.clone().unwrap();
    doc.borrow_mut().add_pattern(0, 0, "Pattern 2").unwrap();
    doc.borrow_mut().assign_pattern(0, 0, Some(1)).unwrap();
    (a.group, a.pattern) = (0, 1);
    a.set_recording(true, false);
    assert_eq!(a.grow.as_ref().map(|g| g.pattern), Some(1));
    a.menu_act(crate::layout::Act::DeletePattern(0, 0));
    assert_eq!(a.grow.as_ref().map(|g| g.pattern), Some(0));
    play(&mut a, 120.0, 240.0);
    a.auto_grow_at(Mode::Scene(0), 100.0);
    play(&mut a, (bar + 120) as f64, 240.0);
    a.set_recording(false, false);
    assert_eq!(length(&a), 2 * bar);

    // 4. Moving or deleting a group while growing updates or drops `grow.group` cleanly.
    let mut a = app();
    a.doc.as_ref().unwrap().borrow_mut().add_group().unwrap();
    a.follow_group_moves();
    a.set_recording(true, false);
    a.move_group(0, 1).unwrap();
    assert_eq!(a.grow.as_ref().map(|g| g.group), Some(1));
    a.delete_group(1).unwrap();
    assert!(a.grow.is_none());
    a.auto_grow_at(Mode::Scene(0), 200.0);
    a.set_recording(false, false);
}

/// Recording from the third bar into an empty Auto-length pattern (whether REC was
/// started in bar 3 or earlier, or armed when a note arrives on an unfocused empty Auto
/// group, or when a new pattern is created while REC is already on) never wraps or jumps
/// the playhead back to bar 3 at the end of bar 4; it continues into bar 5 and settles
/// on `[2 * bar, 2 * bar + 4 * bar)` when REC turns off after bar 6.
#[test]
fn recording_from_third_bar_continues_past_fourth_bar_without_jumping_back() {
    use crate::layout::{Act, apply_acts};

    // 1. Pressing REC while focused on a populated group, then creating a new Auto-length
    // pattern and recording from bar 3 through bar 6.
    let mut a = app();
    let bar = bar(&a);
    let doc = a.doc.clone().unwrap();
    doc.borrow_mut().set_pattern_length(0, 0, 4 * bar).unwrap();
    doc.borrow_mut().set_length_auto(0, 0, false);
    let seed = deckard_model::Note {
        ratchet: Default::default(),
        expr: None,
        sound: 0,
        position: 0,
        length: 120,
        pitch: 36,
        velocity: 100,
        probability: 100,
    };
    doc.borrow_mut().edit_notes("seed", 0, 0, &[], &[seed]).unwrap();
    doc.borrow_mut().add_group().unwrap();
    a.follow_group_moves();

    a.set_recording(true, false);
    assert!(a.grow.is_none());

    apply_acts(&mut a, vec![Act::NewPattern(0, 1)]);
    assert_eq!(a.grow.as_ref().map(|g| (g.group, g.pattern)), Some((1, 0)));
    assert_eq!(doc.borrow().project().groups[1].patterns[0].start, 0);

    let play_in_g1 = |app: &mut crate::App, tick: i64| {
        let on = Recorded {
            midi: true,
            group: 1,
            sound: 0,
            pitch: 60,
            velocity: 100,
            length: i64::MAX,
            tick: tick as f64,
            mode: Mode::Scene(0),
            seq: 0,
            played: tick as f64,
        };
        app.take_in_recorded(vec![on, Recorded { velocity: 0, tick: (tick + 240) as f64, played: (tick + 240) as f64, ..on }]);
    };

    for b in 0..6 {
        let tick = b * bar + 120;
        a.auto_grow_at(Mode::Scene(0), tick as f64);
        let head = Some(crate::Playhead { mode: Mode::Scene(0), queued: None, position: tick as f64, played: tick as f64 });
        let (pos, len) = a.pattern_at(1, head).unwrap();
        assert_eq!(pos / bar, b, "at bar {}: playhead stayed in bar {} (pos={pos}, len={len})", b + 1, b + 1);
        if b >= 2 {
            play_in_g1(&mut a, b * bar + 240);
        }
    }
    a.set_recording(false, false);
    let pat = doc.borrow().project().groups[1].patterns[0].clone();
    assert_eq!((pat.start, pat.length), (2 * bar, 4 * bar));
    assert_eq!(pat.notes.len(), 4);

    // 2. Playing MIDI directly into an unfocused empty Auto-length group starting in bar 3
    // (`focus_follows = false`), after a previous pattern in that group had `start = 2 * bar`.
    apply_acts(&mut a, vec![Act::NewPattern(0, 1)]);
    assert_eq!(doc.borrow().project().groups[1].patterns[1].start, 0, "new pattern after bar-3 pattern still starts at 0");
    a.focus_group(0);
    a.settings.focus_follows = false;
    a.set_recording(true, false);
    assert!(a.grow.is_none(), "Group 0 has notes");

    play_in_g1(&mut a, 2 * bar + 240);
    assert_eq!(a.grow.as_ref().map(|g| (g.group, g.pattern)), Some((1, 1)));
    assert_eq!(doc.borrow().project().groups[1].patterns[1].length, 4 * bar, "grown ahead of bar 3 immediately");

    a.auto_grow_at(Mode::Scene(0), (3 * bar + 120) as f64);
    play_in_g1(&mut a, 3 * bar + 240);
    assert_eq!(doc.borrow().project().scene_length(0), 5 * bar, "scene extended past 4 bars before end of bar 4");
    a.auto_grow_at(Mode::Scene(0), (4 * bar + 120) as f64);
    let head_bar5 =
        Some(crate::Playhead { mode: Mode::Scene(0), queued: None, position: (4 * bar + 120) as f64, played: (4 * bar + 120) as f64 });
    assert_eq!(a.pattern_at(1, head_bar5).unwrap().0 / bar, 4, "at end of 4th bar, playhead is in 5th bar (not back at 3rd)");
    play_in_g1(&mut a, 4 * bar + 240);
    a.set_recording(false, false);
    let pat2 = doc.borrow().project().groups[1].patterns[1].clone();
    assert_eq!((pat2.start, pat2.length), (2 * bar, 3 * bar));
}
