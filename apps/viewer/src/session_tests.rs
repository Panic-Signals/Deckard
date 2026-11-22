//! Session recording as the app wires it: when one may start, what it takes the engine's
//! tap from, and what quitting leaves on disk.

use crate::App;
use crate::session_record::SessionPhase;

/// A project open and no audio device (none opens in these tests).
fn app() -> App {
    let mut app = App::default();
    app.new_project();
    app.output = Some(Err("no audio in this test".into()));
    app.settings.session_reserve_mb = 0;
    app
}

/// Without an audio output there is nothing to record: it says so, before asking for a file.
#[test]
fn no_output_refuses_before_the_file_dialog() {
    let mut app = app();
    app.toggle_session_record();
    assert_eq!(app.error.as_deref(), Some("No audio output to record from."));
    assert!(!app.session.active());
}

/// Sampling recording an internal source holds the engine's one tap: the session is
/// refused, with what to stop.
#[test]
fn an_internal_take_refuses_a_session() {
    let mut app = app();
    app.recorder.fake_recording((0, 0));
    app.recorder.fake_tap();
    assert_eq!(app.session_record_rate(), None);
    assert!(app.error.as_deref().is_some_and(|e| e.contains("Sampling is recording")), "{:?}", app.error);
    assert!(app.recorder.is_internal_active(), "the take keeps its tap");
}

/// Sampling only metering an internal source (its RECORD tab open, nothing recording) is
/// not something to stop: it gives up the tap, and the session arms.
#[test]
fn a_meter_alone_makes_way_for_a_session() {
    let mut app = app();
    app.recorder.fake_tap();
    assert!(!app.recorder.busy());
    // (Not refused for the meter: only for the missing output of these tests.)
    assert_eq!(app.session_record_rate(), None);
    assert_eq!(app.error.take().as_deref(), Some("No audio output to record from."));
    let dir = tempfile::tempdir().unwrap();
    app.arm_session_record(dir.path().join("s.wav"), 48_000);
    assert_eq!(app.error, None);
    assert_eq!(app.session.phase(), SessionPhase::Armed);
    assert!(!app.recorder.is_internal_active(), "the meter let the tap go");
    assert!(app.session_window_open);
    assert_eq!(app.settings.session_dir.as_deref(), Some(dir.path()));
}

/// A disk already down to the reserve: no session, and it says why.
#[test]
fn no_room_on_the_disk_refuses() {
    let mut app = app();
    app.settings.session_reserve_mb = u32::MAX;
    let dir = tempfile::tempdir().unwrap();
    app.arm_session_record(dir.path().join("s.wav"), 48_000);
    assert_eq!(app.error.as_deref(), Some("Not enough free disk space to start a session recording."));
    assert!(!app.session.active());
}

/// Armed before anything was played (no engine yet): an engine is made for the tap, which
/// would otherwise be dropped and leave a recording of pure silence. (Needs an audio device.)
#[test]
fn arming_before_anything_played_still_taps_the_master() {
    let Ok(out) = deckard_engine::output::Output::new() else { return };
    let mut app = app();
    app.output = Some(Ok(out));
    let Some(Ok(out)) = &app.output else { unreachable!() };
    assert!(!out.has_engine());
    let rate = app.session_record_rate().expect("an engine to record from");
    let dir = tempfile::tempdir().unwrap();
    app.arm_session_record(dir.path().join("s.wav"), rate);
    let Some(Ok(out)) = &app.output else { unreachable!() };
    assert!(out.has_engine(), "an engine was made");
    let tapped = out.call(|e| e.tap().map(|t| t.source));
    assert_eq!(tapped, Some(Some(deckard_engine::TapSource::Master)), "and it has the session's tap");
    app.stop_session_record();
}

/// Quitting while recording: the file is finalized with all that was recorded, though less
/// than a second of it (nothing flushed yet) and the app leaves without running destructors.
#[test]
fn quitting_finalizes_the_recording() {
    let mut app = app();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.wav");
    app.session.start(path.clone(), 48_000, 0, "now".into()).unwrap();
    app.session.poll_at(0.5, 48_000, &[[0.5, 0.5]; 24_000], u64::MAX);
    eframe::App::on_exit(&mut app);
    assert!(!app.session.active());
    // (Read while the app still lives: nothing was left to a destructor.)
    let r = hound::WavReader::open(&path).unwrap();
    assert_eq!(r.duration(), 24_000);
    drop(app);
}
