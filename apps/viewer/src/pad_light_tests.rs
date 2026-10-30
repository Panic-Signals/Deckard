//! Pad mode's pad lights on the MK3 (BUGS.md #36): a sound with anything in it lights
//! its pad, in any group.

use crate::layout::{Act, apply_acts};
use deckard_model::Note;

fn used(app: &crate::App, g: usize) -> Vec<bool> {
    app.surface_view().groups[g].sounds.iter().map(|s| s.used).collect()
}

#[test]
fn a_new_groups_sound_with_a_sampler_lights() {
    let mut app = crate::App::default();
    app.new_project();
    apply_acts(&mut app, vec![Act::AddGroup]);
    let g = app.group;
    app.doc.as_ref().unwrap().borrow_mut().add_slot(g, 2, 0, &deckard_doc::NewModule::Sampler).unwrap();
    assert!(used(&app, g)[2], "{:?}", used(&app, g));
}

#[test]
fn a_sound_with_notes_lights() {
    let mut app = crate::App::default();
    app.new_project();
    apply_acts(&mut app, vec![Act::AddGroup, Act::NewPattern(0, 1)]);
    let (g, pi) = (app.group, app.pattern);
    let note = Note { expr: None, sound: 3, position: 0, length: 960, pitch: 60, velocity: 100, probability: 100 };
    app.doc.as_ref().unwrap().borrow_mut().add_note(g, pi, note).unwrap();
    assert!(used(&app, g)[3], "{:?}", used(&app, g));
}

/// A sound that only plays a MIDI instrument: no plug-ins, its MIDI output set.
#[test]
fn a_midi_only_sound_lights() {
    let mut app = crate::App::default();
    app.new_project();
    let route = deckard_doc::SoundRoute::MidiOut(deckard_model::MidiDest::Port("Synth".into()));
    app.doc.as_ref().unwrap().borrow_mut().set_sound_route(0, 5, route).unwrap();
    assert!(used(&app, 0)[5], "{:?}", used(&app, 0));
}

/// Palenight's chosen tab's border runs round as the app drives the MK3 (on the page it
/// starts on).
#[test]
fn palenights_border_moves_in_the_app() {
    let mut app = crate::App::default();
    app.new_project();
    app.surface.set_theme("palenight");
    let mut requests = Vec::new();
    let mut frame = || {
        let out = app.mk3_events(Vec::new(), &mut requests);
        out.screens.map(|(_, px)| (4..357).map(|x| px[4 * 960 + x]).collect::<Vec<_>>())
    };
    let a = frame().expect("a picture");
    std::thread::sleep(std::time::Duration::from_millis(400));
    let b = frame().expect("a new picture");
    assert_ne!(a, b);
    assert_eq!(app.surface.error(), None);
}
