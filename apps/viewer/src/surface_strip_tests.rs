//! The Smart Strip on DECKARD's own controller layer: what it plays
//! ([`deckard_surface::Action::Strip`], `StripNote`, `PerformFx`).

use deckard_engine::Live;
use deckard_model::MidiKind;
use deckard_surface::Action as A;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

#[test]
fn pitch_and_mod_go_to_the_focused_sound() {
    let mut app = app();
    (app.group, app.sound) = (0, 2);
    let bend = |app: &crate::App, v| match app.strip_live(A::Strip("pitch", v)) {
        Some(Live::ControlIn { group: 0, sound: 2, kind: MidiKind::PitchBend(b), record: true, .. }) => b,
        other => panic!("{other:?}"),
    };
    assert_eq!(bend(&app, 0.0), 8192, "the middle");
    assert_eq!(bend(&app, 1.0), 16383);
    assert_eq!(bend(&app, -1.0), 0);
    assert!(matches!(
        app.strip_live(A::Strip("mod", 0.5)),
        Some(Live::ControlIn { group: 0, sound: 2, kind: MidiKind::Controller { number: 1, value: 64 }, .. })
    ));
    assert!(matches!(
        app.strip_live(A::StripNote { sound: 3, pitch: 62, velocity: 90, on: true }),
        Some(Live::NoteOn { group: 0, sound: 3, pitch: 62, velocity: 90, length: i64::MAX })
    ));
    assert!(matches!(
        app.strip_live(A::StripNote { sound: 3, pitch: 62, velocity: 90, on: false }),
        Some(Live::NoteRelease { group: 0, sound: 3, pitch: 62 })
    ));
}

#[test]
fn perform_fx_is_loaded_shown_and_played() {
    let mut app = app();
    assert!(app.surface_view().groups[0].perform.is_none());
    app.surface_act(A::PerformFx);
    let slot = app.perform_slot(0).expect("one loaded");
    assert_eq!((app.slot_of, app.slot), (Some(deckard_doc::Chain::Group(0)), Some(slot)), "shown");
    assert_eq!(app.surface_view().groups[0].perform, Some((false, 0.5)));
    app.surface_act(A::Strip("perform_touch", 0.25));
    assert_eq!(app.surface_view().groups[0].perform, Some((true, 0.25)));
    app.surface_act(A::Strip("perform_release", 0.75));
    assert_eq!(app.surface_view().groups[0].perform, Some((false, 0.75)));
}

/// BUGS #13: the strip's bend or mod wheel with nothing playing starts an engine for
/// it (as a note does) instead of going nowhere, on either layer.
#[test]
fn a_bend_with_nothing_playing_starts_an_engine() {
    use deckard_lua_host::state::ControlRequest;
    let mut app = app();
    app.control_request(ControlRequest::StripControl { group: 0, sound: 0, pitch: true, value: 0.5 });
    let Some(Ok(out)) = &app.output else {
        eprintln!("no audio output; skipping");
        return;
    };
    assert!(out.has_engine(), "the bend has an engine to go to");
}

/// CONTROLLER-BUGS C47: a slot of DECKARD's own plug-ins carries its picture's key to
/// the controller; a hosted plug-in none.
#[test]
fn own_plugins_carry_their_picture() {
    let mut app = app();
    app.add_slot(crate::layout::AddWhat::Sampler);
    let chain = app.surface_view().control.chain;
    assert_eq!(chain.iter().map(|s| s.picture.clone()).collect::<Vec<_>>(), [Some("sampler".to_string())]);
}
