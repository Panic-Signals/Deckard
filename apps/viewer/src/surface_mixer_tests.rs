//! MIX on the controller opens the software Mixer, and closes it pressed again.

use deckard_mk3::Event;
use std::time::Instant;

fn app() -> crate::App {
    let mut app = crate::App::default();
    app.new_project();
    app
}

fn press_mix(app: &mut crate::App) {
    let now = Instant::now();
    let view = app.surface_view();
    let actions = app.surface.event(&Event::Button("BUTTON_MIX", true), &view, now);
    for action in actions {
        app.surface_act(action);
    }
}

#[test]
fn the_mixer_button_opens_and_closes_the_mixer_view() {
    let mut app = app();
    app.lfos = true;
    press_mix(&mut app);
    assert!(app.mixer, "MIX opens the Mixer");
    assert!(!app.lfos, "the LFO panel gives the Mixer its place");
    assert_eq!(app.surface.page(), "mixer");
    press_mix(&mut app);
    assert!(!app.mixer, "MIX again closes it");
    assert_eq!(app.surface.page(), "control");
}
